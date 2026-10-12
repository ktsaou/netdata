// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{histogram_iterator::AllFloatBucketIterator, histogram_work::HistogramWork};

unsafe fn visit(
    raw_histogram: *const types::FloatHistogram,
    raw_control: *const types::CallbackControl,
    callback: Option<types::HistogramBucketCallback>,
    context: *mut std::ffi::c_void,
    out_report: *mut *mut types::Report,
) -> Result<u32, Invalid> {
    unsafe { checked::output(out_report) }?;
    unsafe { out_report.write(std::ptr::null_mut()) };
    let callback = callback.ok_or(Invalid("missing histogram bucket callback"))?;
    let control = unsafe { checked::callback(raw_control) }?;
    let mut footprint = checked::Footprint::new(control.account);
    let histogram = unsafe { provider::float_histogram(raw_histogram, Some(&mut footprint)) }?;
    let borrowed = control
        .account
        .hold(MemoryKind::BorrowedView, footprint.bytes()?);
    let mut report = Report::new(control.account, false);
    let work = HistogramWork::new(control.control, control.account);
    let value = (|| {
        let mut iterator = AllFloatBucketIterator::new_with(histogram, false, work)
            .map_err(|error| ExecutionError::from(EvaluationError::from(error)))?;
        while iterator
            .advance_with(work)
            .map_err(|error| ExecutionError::from(EvaluationError::from(error)))?
        {
            let bucket = iterator.at();
            let value = types::HistogramBucket {
                lower_bits: bucket.lower.to_bits(),
                upper_bits: bucket.upper.to_bits(),
                count_bits: bucket.count.to_bits(),
                lower_inclusive: u32::from(bucket.lower_inclusive),
                upper_inclusive: u32::from(bucket.upper_inclusive),
            };
            let mut error = types::Bytes::default();
            let status = unsafe { callback(context, &value, raw_control, &mut error) };
            unsafe { output::callback_status(status, error, control.account) }
                .map_err(|error| ExecutionError::output(error, control.account))?;
        }
        control.control.check()?;
        Ok(())
    })();
    drop(borrowed);
    let status = execution_status(&value);
    let slot = report.get_mut().expect("unpublished histogram report");
    slot.outcome = Outcome::Execute(value);
    slot.observations = control.account.snapshot();
    unsafe { out_report.write(report.into_raw().cast()) };
    Ok(status)
}

export! {
    fn netdata_promql_callback_histogram_buckets(histogram: *const types::FloatHistogram, control: *const types::CallbackControl, callback: Option<types::HistogramBucketCallback>, context: *mut std::ffi::c_void, out_report: *mut *mut types::Report) {
        unsafe { visit(histogram, control, callback, context, out_report) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{FloatHistogram, Span},
    };

    #[cfg(feature = "fixtures")]
    use crate::fixture_codec;

    #[derive(Default)]
    struct Receiver {
        buckets: Vec<types::HistogramBucket>,
        status: u32,
        cancel: bool,
    }
    unsafe extern "C" fn receive(
        context: *mut std::ffi::c_void,
        bucket: *const types::HistogramBucket,
        control: *const types::CallbackControl,
        error: *mut types::Bytes,
    ) -> u32 {
        let receiver = unsafe { &mut *context.cast::<Receiver>() };
        receiver.buckets.push(unsafe { *bucket });
        if receiver.cancel {
            unsafe { checked::callback(control) }
                .unwrap()
                .control
                .cancellation
                .cancel();
        }
        unsafe {
            error.write(types::Bytes {
                data: b"bucket\0error".as_ptr(),
                len: 12,
            })
        };
        receiver.status
    }
    fn call(
        histogram: *const types::FloatHistogram,
        control: &CoreControl,
        account: &QueryAccounting,
        receiver: &mut Receiver,
    ) -> (u32, *mut types::Report) {
        let control = checked::Callback::new(control, account);
        let mut report = std::ptr::null_mut();
        let status = unsafe {
            netdata_promql_callback_histogram_buckets(
                histogram,
                control.as_ptr(),
                Some(receive),
                (receiver as *mut Receiver).cast(),
                &mut report,
            )
        };
        (status, report)
    }
    fn simple_histogram() -> FloatHistogram {
        FloatHistogram {
            schema: 0,
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: 2,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![0.0, 4.0].into()),
            zero_count: 2.0,
            zero_threshold: 0.25,
            ..FloatHistogram::default()
        }
    }
    #[test]
    fn buckets_keep_zero_counts_and_borrowed_payload_is_released() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let histogram = simple_histogram();
        let descriptor = output::float_histogram(histogram.as_ref());
        let mut receiver = Receiver::default();
        let (status, report) = call(
            &descriptor,
            &CoreControl::default(),
            &account,
            &mut receiver,
        );
        assert_eq!(status, types::OK);
        let expected = [
            (-0.25, 0.25, 2.0, 1, 1),
            (0.5, 1.0, 0.0, 0, 1),
            (1.0, 2.0, 4.0, 0, 1),
        ];
        assert_eq!(receiver.buckets.len(), expected.len());
        for (actual, (lower, upper, count, lower_inclusive, upper_inclusive)) in
            receiver.buckets.iter().zip(expected)
        {
            assert_eq!(actual.lower_bits, f64::to_bits(lower));
            assert_eq!(actual.upper_bits, f64::to_bits(upper));
            assert_eq!(actual.count_bits, f64::to_bits(count));
            assert_eq!(actual.lower_inclusive, lower_inclusive);
            assert_eq!(actual.upper_inclusive, upper_inclusive);
        }
        assert_eq!(
            account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
            0
        );
        assert!(
            account.snapshot().memory[MemoryKind::BorrowedView as usize].peak
                >= size_of::<types::FloatHistogram>() as u64
        );
        assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].current,
            0
        );
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            0
        );
    }
    #[test]
    fn callback_error_precedes_cancellation_and_owns_its_bytes() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let histogram = simple_histogram();
        for status in [types::OUTPUT_ERROR, types::PROVIDER_ERROR] {
            let mut receiver = Receiver {
                status,
                cancel: true,
                ..Receiver::default()
            };
            let (status, report) = call(
                &output::float_histogram(histogram.as_ref()),
                &CoreControl::default(),
                &account,
                &mut receiver,
            );
            assert_eq!(status, types::OUTPUT_ERROR);
            assert_eq!(receiver.buckets.len(), 1);
            let owned = unsafe { borrow::<Report, _>(report) }.unwrap();
            let Outcome::Execute(Err(ExecutionError::Sink(error))) = &owned.outcome else {
                panic!("owned callback error")
            };
            assert_eq!(
                &**error,
                if receiver.status == types::OUTPUT_ERROR {
                    b"bucket\0error".as_slice()
                } else {
                    b"invalid output callback status".as_slice()
                }
            );
            assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
        }
    }
    #[test]
    fn invalid_array_shapes_leave_no_report_or_borrowed_lease() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        for kind in 0..4 {
            let mut histogram = types::FloatHistogram::default();
            match kind {
                0 => histogram.positive_buckets.len = 1,
                1 => histogram.positive_buckets.reserved = 1,
                2 => histogram.negative_spans.present = 2,
                _ => histogram.reset_hint = 256,
            }
            let (status, report) = call(
                &histogram,
                &CoreControl::default(),
                &account,
                &mut Receiver::default(),
            );
            assert_eq!(status, types::INVALID_ARGUMENT);
            assert!(report.is_null());
            assert_eq!(
                account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
                0
            );
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].current,
                0
            );
        }
    }
    #[test]
    fn large_payload_keeps_constant_adapter_workspace() {
        let mut peaks = Vec::new();
        for length in [17, 100_000] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let histogram = FloatHistogram {
                schema: 0,
                positive_spans: Some(vec![Span { offset: 0, length }].into()),
                positive_buckets: Some(vec![1.0; length as usize].into()),
                ..FloatHistogram::default()
            };
            let (status, report) = call(
                &output::float_histogram(histogram.as_ref()),
                &CoreControl::default(),
                &account,
                &mut Receiver::default(),
            );
            assert_eq!(status, types::OK);
            assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
            peaks.push(account.snapshot().memory[MemoryKind::Workspace as usize].peak);
            assert_eq!(
                account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
                0
            );
        }
        assert_eq!(peaks[0], peaks[1]);
        assert!(peaks[0] < 4096);
    }
    #[test]
    fn construction_and_traversal_poll_the_enclosing_control() {
        for many_spans in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let histogram = FloatHistogram {
                schema: 0,
                negative_spans: Some(
                    vec![
                        Span {
                            offset: 0,
                            length: 1
                        };
                        if many_spans { 100_000 } else { 1 }
                    ]
                    .into(),
                ),
                positive_spans: Some(
                    vec![Span {
                        offset: 0,
                        length: 100_000,
                    }]
                    .into(),
                ),
                positive_buckets: Some(vec![1.0; 100_000].into()),
                ..FloatHistogram::default()
            };
            let control = CoreControl::default();
            let mut receiver = Receiver::default();
            let (status, report) =
                crate::test_storage::interrupt_during(&account, &control, || {
                    call(
                        &output::float_histogram(histogram.as_ref()),
                        &control,
                        &account,
                        &mut receiver,
                    )
                });
            assert_eq!(status, types::CANCELLED);
            if many_spans {
                assert!(receiver.buckets.is_empty());
            } else {
                assert!(!receiver.buckets.is_empty());
            }
            assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
            assert_eq!(
                account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
                0
            );
        }
        let engine = EngineAccounting::default();
        let account = engine.query();
        let histogram = simple_histogram();
        let control = CoreControl {
            deadline: Some(std::time::Instant::now()),
            ..CoreControl::default()
        };
        let mut receiver = Receiver::default();
        let (status, report) = call(
            &output::float_histogram(histogram.as_ref()),
            &control,
            &account,
            &mut receiver,
        );
        assert_eq!(status, types::DEADLINE);
        assert!(receiver.buckets.is_empty());
        assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
    }

    #[cfg(feature = "fixtures")]
    #[test]
    fn original_upstream_all_bucket_expectations_match_exact_bits() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/promql-conformance/histogram-original-inputs.json");
        let ledger: serde_json::Value =
            serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
        assert_eq!(ledger["commit"], "5241a27fe3c6983549fccc32f6e65917408c63cd");
        let mut cases = 0;
        for case in ledger["cases"].as_array().unwrap() {
            if case["mode"] != "float-iterator-set"
                || case["declared"].is_null()
                || !case["request"]["routes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|route| route == "all")
            {
                continue;
            }
            let histogram = fixture_codec::float(&case["request"]["input"]).unwrap();
            let engine = EngineAccounting::default();
            let account = engine.query();
            let mut receiver = Receiver::default();
            let (status, report) = call(
                &output::float_histogram(histogram.as_ref()),
                &CoreControl::default(),
                &account,
                &mut receiver,
            );
            assert_eq!(status, types::OK, "{}", case["id"]);
            let expected = &case["declared"]["buckets"]["all"];
            let expected = if expected.is_null() {
                &[][..]
            } else {
                expected.as_array().unwrap().as_slice()
            };
            assert_eq!(receiver.buckets.len(), expected.len(), "{}", case["id"]);
            for (actual, expected) in receiver.buckets.iter().zip(expected) {
                assert_eq!(
                    actual.lower_bits,
                    fixture_codec::number(&expected["lower"]).unwrap().to_bits(),
                    "{}",
                    case["id"]
                );
                assert_eq!(
                    actual.upper_bits,
                    fixture_codec::number(&expected["upper"]).unwrap().to_bits(),
                    "{}",
                    case["id"]
                );
                assert_eq!(
                    actual.count_bits,
                    fixture_codec::number(&expected["count"]).unwrap().to_bits(),
                    "{}",
                    case["id"]
                );
                assert_eq!(
                    actual.lower_inclusive,
                    u32::from(expected["lower_inclusive"].as_bool().unwrap()),
                    "{}",
                    case["id"]
                );
                assert_eq!(
                    actual.upper_inclusive,
                    u32::from(expected["upper_inclusive"].as_bool().unwrap()),
                    "{}",
                    case["id"]
                );
            }
            assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
            assert_eq!(
                account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
                0
            );
            cases += 1;
        }
        assert_eq!(cases, 9, "original declared all-bucket fixture roster");
    }
}
