// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

unsafe fn sort(
    indices: *mut u64,
    count: u64,
    raw_control: *const types::CallbackControl,
    callback: Option<types::SortCompareCallback>,
    context: *mut std::ffi::c_void,
    out_report: *mut *mut types::Report,
) -> Result<u32, Invalid> {
    unsafe { checked::output(out_report) }?;
    let (report_begin, report_end) = checked::region(out_report, 1)?;
    let (begin, end) = match checked::region(indices, count) {
        Ok(region) => region,
        Err(error) => {
            unsafe { out_report.write(std::ptr::null_mut()) };
            return Err(error);
        }
    };
    if begin < report_end && report_begin < end {
        return Err(Invalid("overlapping sort array and report slot"));
    }
    unsafe { out_report.write(std::ptr::null_mut()) };
    let callback = callback.ok_or(Invalid("missing sort comparator"))?;
    let control = unsafe { checked::callback(raw_control) }?;
    let mut report = Report::new(control.account, false);
    let value = (|| {
        control.control.check()?;
        let _borrowed = control
            .account
            .hold(MemoryKind::BorrowedView, (end - begin) as u64);
        let data = if count == 0 {
            &mut []
        } else {
            // The caller grants exclusive access, including absence of access from comparator aliases.
            unsafe { std::slice::from_raw_parts_mut(indices, count as usize) }
        };
        crate::go_sort::try_sort_by_with(
            data,
            |left, right| {
                let mut ordering = 0;
                let mut error = types::Bytes::default();
                let status = unsafe {
                    callback(
                        context,
                        *left,
                        *right,
                        raw_control,
                        &mut ordering,
                        &mut error,
                    )
                };
                unsafe { output::callback_status(status, error, control.account) }
                    .map_err(|error| ExecutionError::output(error, control.account))?;
                Ok(ordering.cmp(&0))
            },
            || {
                control.control.check()?;
                control.account.work(1);
                Ok::<_, ExecutionError>(())
            },
        )?;
        control.control.check()?;
        Ok(())
    })();
    let status = execution_status(&value);
    let slot = report.get_mut().expect("unpublished sorting report");
    slot.outcome = Outcome::Execute(value);
    slot.observations = control.account.snapshot();
    unsafe { out_report.write(report.into_raw().cast()) };
    Ok(status)
}

export! {
    fn netdata_promql_callback_sort_indices(indices: *mut u64, count: u64, control: *const types::CallbackControl, callback: Option<types::SortCompareCallback>, context: *mut std::ffi::c_void, out_report: *mut *mut types::Report) {
        unsafe { sort(indices, count, control, callback, context, out_report) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::EngineAccounting;
    use std::cmp::Ordering;

    struct Comparator {
        keys: Vec<u64>,
        calls: Vec<(u64, u64)>,
        status: u32,
        cancel: bool,
    }
    unsafe extern "C" fn compare(
        context: *mut std::ffi::c_void,
        left: u64,
        right: u64,
        raw_control: *const types::CallbackControl,
        ordering: *mut i32,
        error: *mut types::Bytes,
    ) -> u32 {
        let receiver = unsafe { &mut *context.cast::<Comparator>() };
        receiver.calls.push((left, right));
        if receiver.cancel {
            unsafe { checked::callback(raw_control) }
                .unwrap()
                .control
                .cancellation
                .cancel();
        }
        unsafe {
            ordering.write(
                match receiver.keys[left as usize].cmp(&receiver.keys[right as usize]) {
                    Ordering::Less => -17,
                    Ordering::Equal => 0,
                    Ordering::Greater => 23,
                },
            );
            error.write(types::Bytes::of(b"compare\0error"));
        }
        receiver.status
    }
    fn call(
        data: *mut u64,
        count: u64,
        account: &QueryAccounting,
        control: &CoreControl,
        receiver: &mut Comparator,
    ) -> (u32, *mut types::Report) {
        let callback = checked::Callback::new(control, account);
        let mut report = std::ptr::null_mut();
        let status = unsafe {
            netdata_promql_callback_sort_indices(
                data,
                count,
                callback.as_ptr(),
                Some(compare),
                (receiver as *mut Comparator).cast(),
                &mut report,
            )
        };
        (status, report)
    }
    fn receiver(keys: Vec<u64>) -> Comparator {
        Comparator {
            keys,
            calls: Vec::new(),
            status: types::OK,
            cancel: false,
        }
    }

    #[test]
    fn ties_and_comparator_trace_reuse_existing_go_program() {
        let keys: Vec<_> = (0..193).map(|i| ((i * 37) % 13) as u64).collect();
        let mut expected: Vec<_> = (0..keys.len() as u64).collect();
        let trace = std::cell::RefCell::new(Vec::new());
        crate::go_sort::sort_by(&mut expected, |left, right| {
            trace.borrow_mut().push((*left, *right));
            keys[*left as usize].cmp(&keys[*right as usize])
        });
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut actual: Vec<_> = (0..keys.len() as u64).collect();
        let mut receiver = receiver(keys);
        let (status, report) = call(
            actual.as_mut_ptr(),
            actual.len() as u64,
            &account,
            &CoreControl::default(),
            &mut receiver,
        );
        assert_eq!(status, types::OK);
        assert_eq!(actual, expected);
        assert_eq!(receiver.calls, trace.into_inner());
        assert!(account.snapshot().work > receiver.calls.len() as u64);
        assert_eq!(
            account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
            0
        );
        assert_eq!(
            account.snapshot().memory[MemoryKind::BorrowedView as usize].peak,
            (actual.len() * 8) as u64
        );
        assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            0
        );
    }

    #[test]
    fn callback_error_owns_bytes_and_precedes_its_cancellation() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = CoreControl::default();
        let mut indices = [0, 1];
        let mut receiver = receiver(vec![2, 1]);
        receiver.status = types::OUTPUT_ERROR;
        receiver.cancel = true;
        let (status, report) = call(indices.as_mut_ptr(), 2, &account, &control, &mut receiver);
        assert_eq!(status, types::OUTPUT_ERROR);
        assert_eq!(receiver.calls.len(), 1);
        let owned = unsafe { borrow::<Report, _>(report) }.unwrap();
        let Outcome::Execute(Err(ExecutionError::Sink(message))) = &owned.outcome else {
            panic!("owned comparator error")
        };
        assert_eq!(&**message, b"compare\0error");
        assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
        assert_eq!(
            account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
            0
        );
    }

    #[test]
    fn cancelled_and_empty_arrays_do_not_call_comparator() {
        for empty in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = CoreControl::default();
            if !empty {
                control.cancellation.cancel();
            }
            let mut indices = [0, 1];
            let mut receiver = receiver(vec![2, 1]);
            let (status, report) = call(
                if empty {
                    std::ptr::null_mut()
                } else {
                    indices.as_mut_ptr()
                },
                if empty { 0 } else { 2 },
                &account,
                &control,
                &mut receiver,
            );
            assert_eq!(status, if empty { types::OK } else { types::CANCELLED });
            assert!(receiver.calls.is_empty());
            assert_eq!(indices, [0, 1]);
            assert_eq!(
                account.snapshot().memory[MemoryKind::BorrowedView as usize].peak,
                0
            );
            assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
        }
    }

    #[test]
    fn shapes_and_overlapping_report_slot_are_rejected_before_mutation() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut receiver = receiver(vec![0]);
        for count in [1, u64::MAX] {
            let (status, report) = call(
                std::ptr::null_mut(),
                count,
                &account,
                &CoreControl::default(),
                &mut receiver,
            );
            assert_eq!(status, types::INVALID_ARGUMENT);
            assert!(report.is_null());
        }
        let mut data = [17u64];
        let control = CoreControl::default();
        let callback = checked::Callback::new(&control, &account);
        assert_eq!(
            unsafe {
                netdata_promql_callback_sort_indices(
                    data.as_mut_ptr(),
                    1,
                    callback.as_ptr(),
                    Some(compare),
                    (&mut receiver as *mut Comparator).cast(),
                    data.as_mut_ptr().cast(),
                )
            },
            types::INVALID_ARGUMENT
        );
        assert_eq!(data, [17]);
        assert!(receiver.calls.is_empty());
    }
}
