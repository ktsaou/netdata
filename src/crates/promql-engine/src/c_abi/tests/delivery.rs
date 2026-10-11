// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[derive(Default)]
struct Delivery {
    calls: [Cell<usize>; 4],
    failed: usize,
    cancelled: usize,
    control: *const types::Control,
    cancel_status: Cell<u32>,
    borrowed_status: Cell<u32>,
    values: RefCell<Vec<(i64, u32, u64)>>,
    commit_observations: Cell<Option<types::QueryObservations>>,
    errors: RefCell<Vec<u8>>,
}
impl Delivery {
    fn table(&mut self) -> types::Output {
        types::Output {
            header: types::Header::of::<types::Output>(),
            context: (self as *mut Self).cast(),
            begin: Some(begin),
            chunk: Some(chunk),
            commit: Some(commit),
            abort: Some(abort),
        }
    }
    unsafe fn called(
        &self,
        index: usize,
        control: *const types::CallbackControl,
        error: *mut types::Bytes,
    ) -> u32 {
        self.calls[index - 1].set(self.calls[index - 1].get() + 1);
        let mut state = types::ControlState::default();
        self.borrowed_status
            .set(unsafe { netdata_promql_callback_control_state(control, &mut state) });
        if self.cancelled == index {
            self.cancel_status
                .set(unsafe { netdata_promql_control_cancel(self.control) });
        }
        if self.failed == index {
            let mut bytes = self.errors.borrow_mut();
            bytes.extend_from_slice(b"sink\0\xfffailure");
            unsafe { error.write(types::Bytes::of(&bytes)) };
            types::OUTPUT_ERROR
        } else {
            types::OK
        }
    }
}
unsafe fn delivery<'a>(context: *mut c_void) -> &'a Delivery {
    unsafe { &*context.cast::<Delivery>() }
}
unsafe extern "C" fn begin(
    context: *mut c_void,
    _: *const types::OutputHeader,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    unsafe { delivery(context).called(1, control, error) }
}
unsafe extern "C" fn chunk(
    context: *mut c_void,
    value: *const types::Chunk,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    let delivery = unsafe { delivery(context) };
    let value = unsafe { &*value };
    match value.kind {
        types::CHUNK_SCALAR => {
            let value = unsafe { value.payload.scalar };
            delivery.values.borrow_mut().push((
                value.time_ms,
                types::VALUE_FLOAT,
                value.value_bits,
            ));
        }
        types::CHUNK_SERIES => {
            let series = unsafe { value.payload.series };
            for value in unsafe {
                std::slice::from_raw_parts(series.points.data, series.points.len as usize)
            } {
                delivery.values.borrow_mut().push((
                    value.time_ms,
                    value.value.kind,
                    if value.value.kind == types::VALUE_FLOAT {
                        unsafe { value.value.payload.float_bits }
                    } else {
                        0
                    },
                ));
            }
        }
        _ => (),
    }
    unsafe { delivery.called(2, control, error) }
}
unsafe extern "C" fn commit(
    context: *mut c_void,
    observations: *const types::QueryObservations,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    let delivery = unsafe { delivery(context) };
    delivery
        .commit_observations
        .set(Some(unsafe { observations.read() }));
    unsafe { delivery.called(3, control, error) }
}
unsafe extern "C" fn abort(context: *mut c_void, _: *const types::AccountingContext) {
    let delivery = unsafe { delivery(context) };
    delivery.calls[3].set(delivery.calls[3].get() + 1);
    // Returned error bytes expire at this callback, after the engine has retained them.
    delivery.errors.borrow_mut().fill(b'X');
    delivery.values.borrow_mut().clear();
}

#[test]
fn foreign_output_preserves_errors_and_the_successful_commit_boundary() {
    for failure in 0..=3 {
        for cancellation in [false, true] {
            unsafe {
                let engine = engine();
                let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
                let query = compiled(engine, b"-0.0");
                let mut token = ptr::null_mut();
                assert_eq!(
                    netdata_promql_control_create(engine, ptr::null(), &mut token),
                    types::OK
                );
                let mut fixture = Fixture::new(1);
                let provider = fixture.table();
                let mut delivery = Delivery {
                    failed: failure,
                    cancelled: if cancellation {
                        if failure == 0 { 3 } else { failure }
                    } else {
                        0
                    },
                    control: token,
                    ..Delivery::default()
                };
                let output = delivery.table();
                let mut report = ptr::null_mut();
                assert_eq!(
                    netdata_promql_execute(
                        query,
                        &provider,
                        &instant(42),
                        token,
                        &output,
                        &mut report
                    ),
                    if failure == 0 {
                        types::OK
                    } else {
                        types::OUTPUT_ERROR
                    }
                );
                assert_eq!(delivery.borrowed_status.get(), types::OK);
                if cancellation {
                    assert_eq!(delivery.cancel_status.get(), types::OK);
                }
                assert_eq!(delivery.calls[0].get(), 1);
                assert_eq!(delivery.calls[1].get(), usize::from(failure != 1));
                assert_eq!(
                    delivery.calls[2].get(),
                    usize::from(failure == 0 || failure == 3)
                );
                assert_eq!(delivery.calls[3].get(), usize::from(failure != 0));
                let mut observations = types::QueryObservations::default();
                assert_eq!(
                    netdata_promql_report_observations(report, &mut observations),
                    types::OK
                );
                assert_eq!(
                    observations.memory[MemoryKind::BorrowedView as usize].current,
                    0
                );
                if failure == 0 {
                    assert_eq!(
                        *delivery.values.borrow(),
                        [(42, types::VALUE_FLOAT, (-0.0f64).to_bits())]
                    );
                    let committed = delivery.commit_observations.get().unwrap();
                    assert_eq!(committed.output_samples, 1);
                    assert!(committed.memory[MemoryKind::Result as usize].current > 0);
                } else {
                    let mut error = types::Error::default();
                    assert_eq!(
                        netdata_promql_report_error(report, 0, &mut error),
                        types::OK
                    );
                    assert_eq!(error.kind, 3);
                    assert_eq!(checked::bytes(error.message).unwrap(), b"sink\0\xfffailure");
                    assert!(
                        observations.memory[MemoryKind::Workspace as usize].current
                            >= error.message.len
                    );
                }
                for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                    assert_eq!(opened.get(), closed.get());
                }
                assert_eq!(netdata_promql_control_release(token), types::OK);
                assert_eq!(netdata_promql_query_release(query), types::OK);
                assert_eq!(netdata_promql_engine_release(engine), types::OK);
                drop(fixture);
                drop(delivery);
                assert_eq!(netdata_promql_report_release(report), types::OK);
                account.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn cancellation_from_provisional_callbacks_aborts_exactly_once() {
    for stage in 1..=2 {
        unsafe {
            let engine = engine();
            let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let query = compiled(engine, b"m");
            let mut token = ptr::null_mut();
            assert_eq!(
                netdata_promql_control_create(engine, ptr::null(), &mut token),
                types::OK
            );
            let mut fixture = Fixture::new(1);
            let provider = fixture.table();
            let mut delivery = Delivery {
                cancelled: stage,
                control: token,
                ..Delivery::default()
            };
            let output = delivery.table();
            let mut report = ptr::null_mut();
            assert_eq!(
                netdata_promql_execute(
                    query,
                    &provider,
                    &instant(1000),
                    token,
                    &output,
                    &mut report
                ),
                types::CANCELLED
            );
            assert_eq!(delivery.calls[3].get(), 1);
            assert_eq!(delivery.calls[2].get(), 0);
            assert!(delivery.values.borrow().is_empty());
            for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                assert_eq!(opened.get(), closed.get());
            }
            assert_eq!(netdata_promql_report_release(report), types::OK);
            assert_eq!(netdata_promql_control_release(token), types::OK);
            assert_eq!(netdata_promql_query_release(query), types::OK);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            account.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn valid_foreign_sink_aborts_even_when_later_shape_validation_fails() {
    unsafe {
        let mut delivery = Delivery::default();
        let output = delivery.table();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_execute(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                &output,
                &mut report
            ),
            types::INVALID_ARGUMENT
        );
        assert!(report.is_null());
        assert_eq!(
            delivery.calls.iter().map(Cell::get).collect::<Vec<_>>(),
            [0, 0, 0, 1]
        );
        let mut invalid = output;
        invalid.commit = None;
        assert_eq!(
            netdata_promql_execute(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                &invalid,
                &mut report
            ),
            types::INVALID_ARGUMENT
        );
        assert_eq!(delivery.calls[3].get(), 1);
    }
}

#[test]
fn provider_errors_preserve_upstream_phase_precedence_and_scratch_lifetimes() {
    for stage in 1..=6 {
        for notes in [false, true] {
            unsafe {
                let engine = engine();
                let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
                // A matrix requires a physical seek; this instant lookup's seek is a no-op.
                let query = compiled(engine, if stage == 5 { b"m[1s]" } else { b"m" });
                let mut token = ptr::null_mut();
                assert_eq!(
                    netdata_promql_control_create(engine, ptr::null(), &mut token),
                    types::OK
                );
                let mut fixture = Fixture::new(1);
                fixture.failure = stage;
                fixture.cancel_stage = stage;
                fixture.cancellation = token;
                if notes && stage == 3 {
                    fixture.diagnostics.push(types::ProviderDiagnostic {
                        severity: 1,
                        message: types::Bytes::of(b"opaque\0\xffnote"),
                        ..types::ProviderDiagnostic::default()
                    });
                }
                let provider = fixture.table();
                let mut result = ptr::null_mut();
                let mut report = ptr::null_mut();
                let status = netdata_promql_collect(
                    query,
                    &provider,
                    &instant(1000),
                    token,
                    &mut result,
                    &mut report,
                );
                if stage == 2 {
                    assert_eq!(status, types::CANCELLED);
                } else {
                    assert_ne!(status, types::CANCELLED, "stage {stage}");
                }
                assert_ne!(status, types::OK, "stage {stage}");
                assert!(result.is_null());
                let mut error = types::Error::default();
                assert_eq!(
                    netdata_promql_report_error(report, 0, &mut error),
                    types::OK
                );
                if stage != 2 {
                    assert!(
                        checked::bytes(error.message)
                            .unwrap()
                            .ends_with(b"provider\0\xfffailure")
                    );
                }
                let mut count = 0;
                assert_eq!(
                    netdata_promql_report_annotation_count(report, &mut count),
                    types::OK
                );
                assert_eq!(count, u64::from(notes && stage == 3));
                if count != 0 {
                    let mut annotation = types::Annotation::default();
                    assert_eq!(
                        netdata_promql_report_annotation(report, 0, &mut annotation),
                        types::OK
                    );
                    assert_eq!((annotation.severity, annotation.position_present), (1, 0));
                    assert_eq!(
                        checked::bytes(annotation.raw_message).unwrap(),
                        b"opaque\0\xffnote"
                    );
                    assert_eq!(
                        checked::bytes(annotation.message).unwrap(),
                        b"opaque\0\xffnote"
                    );
                }
                assert_eq!(fixture.cancelled_status.get(), types::OK);
                assert_eq!(netdata_promql_query_release(query), types::OK);
                assert_eq!(netdata_promql_control_release(token), types::OK);
                assert_eq!(netdata_promql_engine_release(engine), types::OK);
                drop(fixture);
                if stage != 2 {
                    assert!(
                        checked::bytes(error.message)
                            .unwrap()
                            .ends_with(b"provider\0\xfffailure")
                    );
                }
                assert_eq!(netdata_promql_report_release(report), types::OK);
                account.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn cancellation_at_selection_end_skips_the_terminal_diagnostic_roster() {
    let mut work = Vec::new();
    for count in [0, 16_384] {
        unsafe {
            let engine = engine();
            let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let query = compiled(engine, b"m");
            let mut token = ptr::null_mut();
            assert_eq!(
                netdata_promql_control_create(engine, ptr::null(), &mut token),
                types::OK
            );
            let mut fixture = Fixture::new(1);
            fixture.cancel_stage = 7;
            fixture.cancellation = token;
            fixture.diagnostics = (0..count)
                .map(|_| types::ProviderDiagnostic {
                    message: types::Bytes::of(b""),
                    severity: 0,
                    position_present: 0,
                    position: types::Position::default(),
                })
                .collect();
            let provider = fixture.table();
            let mut result = ptr::null_mut();
            let mut report = ptr::null_mut();
            assert_eq!(
                netdata_promql_collect(
                    query,
                    &provider,
                    &instant(1000),
                    token,
                    &mut result,
                    &mut report
                ),
                types::CANCELLED
            );
            assert!(result.is_null() && !report.is_null());
            assert_eq!(fixture.cancelled_status.get(), types::OK);
            let mut observations = types::QueryObservations::default();
            assert_eq!(
                netdata_promql_report_observations(report, &mut observations),
                types::OK
            );
            work.push(observations.work);
            for index in 0..4 {
                assert_eq!(fixture.opened[index].get(), fixture.closed[index].get());
            }
            assert_eq!(netdata_promql_report_release(report), types::OK);
            assert_eq!(netdata_promql_query_release(query), types::OK);
            assert_eq!(netdata_promql_control_release(token), types::OK);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            account.snapshot().assert_memory_released();
        }
    }
    assert_eq!(
        work[0], work[1],
        "a successful terminal callback cancellation must stop before diagnostic conversion"
    );
}
