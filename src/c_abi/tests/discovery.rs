// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

const BOUNDS: types::Bounds = types::Bounds {
    start_ms: -123,
    end_ms: 4567,
};

#[derive(Default)]
struct Rows {
    calls: [Cell<usize>; 4],
    rows: RefCell<Vec<LabelRow>>,
    stop: bool,
    fail: usize,
    cancel: usize,
    token: *const types::Control,
    lease: Cell<*mut types::AllocationHandle>,
    body: RefCell<Option<Box<[u8]>>>,
    committed: Cell<Option<types::QueryObservations>>,
}
type LabelRow = Vec<(Vec<u8>, Vec<u8>)>;
impl Rows {
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
    fn release_body(&self) {
        drop(self.body.borrow_mut().take());
        let lease = self.lease.replace(ptr::null_mut());
        if !lease.is_null() {
            assert_eq!(
                unsafe { netdata_promql_allocation_release(lease) },
                types::OK
            );
        }
    }
    unsafe fn status(
        &self,
        stage: usize,
        control: *const types::CallbackControl,
        error: *mut types::Bytes,
    ) -> u32 {
        self.calls[stage - 1].set(self.calls[stage - 1].get() + 1);
        let mut state = types::ControlState::default();
        assert_eq!(
            unsafe { netdata_promql_callback_control_state(control, &mut state) },
            types::OK
        );
        if self.cancel == stage {
            assert_eq!(
                unsafe { netdata_promql_control_cancel(self.token) },
                types::OK
            );
        }
        if self.fail == stage {
            unsafe { error.write(types::Bytes::of(b"metadata output failed")) };
            types::OUTPUT_ERROR
        } else {
            types::OK
        }
    }
}
unsafe fn rows<'a>(context: *mut c_void) -> &'a Rows {
    unsafe { &*context.cast::<Rows>() }
}
unsafe extern "C" fn begin(
    context: *mut c_void,
    header: *const types::OutputHeader,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    let header = unsafe { &*header };
    assert_eq!(
        (
            header.value_type,
            header.start_ms,
            header.end_ms,
            header.step_ms
        ),
        (types::TYPE_VECTOR, BOUNDS.start_ms, BOUNDS.end_ms, 0)
    );
    unsafe { rows(context).status(1, control, error) }
}
unsafe extern "C" fn chunk(
    context: *mut c_void,
    value: *const types::Chunk,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    let rows = unsafe { rows(context) };
    let value = unsafe { &*value };
    if value.kind == types::CHUNK_SERIES {
        let row = unsafe { value.payload.series };
        assert_eq!(row.series_id, rows.rows.borrow().len() as u64);
        assert_eq!(row.points.len, 0);
        let labels = unsafe { checked::slice(row.labels.data, row.labels.len) }.unwrap();
        rows.rows.borrow_mut().push(
            labels
                .iter()
                .map(|value| {
                    (
                        unsafe { checked::bytes(value.name) }.unwrap().to_vec(),
                        unsafe { checked::bytes(value.value) }.unwrap().to_vec(),
                    )
                })
                .collect(),
        );
        if rows.lease.get().is_null() {
            let mut account = ptr::null();
            assert_eq!(
                unsafe { netdata_promql_callback_accounting(control, &mut account) },
                types::OK
            );
            let mut lease = ptr::null_mut();
            assert_eq!(
                unsafe {
                    netdata_promql_allocation_acquire(
                        account,
                        MemoryKind::Result as u32,
                        8192,
                        &mut lease,
                    )
                },
                types::OK
            );
            rows.lease.set(lease);
            *rows.body.borrow_mut() = Some(vec![0_u8; 8192].into_boxed_slice());
        }
    }
    let status = unsafe { rows.status(2, control, error) };
    if status == types::OK && rows.stop && value.kind == types::CHUNK_SERIES {
        types::END
    } else {
        status
    }
}
unsafe extern "C" fn commit(
    context: *mut c_void,
    value: *const types::QueryObservations,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    let rows = unsafe { rows(context) };
    rows.committed.set(Some(unsafe { value.read() }));
    let value = unsafe { &*value };
    assert_eq!(
        (value.pages, value.samples_fetched, value.output_samples),
        (0, 0, 0)
    );
    assert_eq!(value.memory[MemoryKind::BorrowedView as usize].current, 0);
    unsafe { rows.status(3, control, error) }
}
unsafe extern "C" fn abort(context: *mut c_void, account: *const types::AccountingContext) {
    assert!(!account.is_null());
    let rows = unsafe { rows(context) };
    rows.calls[3].set(rows.calls[3].get() + 1);
    rows.release_body();
}
unsafe extern "C" fn open(
    context: *mut c_void,
    bounds: types::Bounds,
    control: *const types::CallbackControl,
    handle: *mut *mut c_void,
    error: *mut types::Bytes,
) -> u32 {
    assert_eq!(
        (bounds.start_ms, bounds.end_ms),
        (BOUNDS.start_ms, BOUNDS.end_ms)
    );
    unsafe { query_open(context, bounds, control, handle, error) }
}
unsafe extern "C" fn select(
    context: *mut c_void,
    query: *mut c_void,
    request: *const types::SelectionRequest,
    control: *const types::CallbackControl,
    handle: *mut *mut c_void,
    error: *mut types::Bytes,
) -> u32 {
    let request_value = unsafe { &*request };
    assert_eq!(
        (request_value.bounds.start_ms, request_value.bounds.end_ms),
        (BOUNDS.start_ms, BOUNDS.end_ms)
    );
    assert_eq!((request_value.step_ms, request_value.range_ms), (0, 0));
    unsafe { selection_open(context, query, request, control, handle, error) }
}
unsafe extern "C" fn next_with_late_cancel(
    context: *mut c_void,
    handle: *mut c_void,
    control: *const types::CallbackControl,
    out: *mut types::SeriesOpen,
    diagnostics: *mut types::ProviderDiagnostics,
    error: *mut types::Bytes,
) -> u32 {
    let status = unsafe { selection_next(context, handle, control, out, diagnostics, error) };
    if status == types::PROVIDER_ERROR {
        unsafe { fixture(context) }.cancel(3);
    }
    status
}
unsafe fn selector(engine: *const types::Engine, source: &[u8]) -> *mut types::Selector {
    let mut value = ptr::null_mut();
    let mut report = ptr::null_mut();
    assert_eq!(
        unsafe {
            netdata_promql_selector_compile(
                engine,
                types::Bytes::of(source),
                ptr::null(),
                &mut value,
                &mut report,
            )
        },
        types::OK
    );
    assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
    value
}
fn provider(fixture: &mut Fixture) -> types::Provider {
    let mut table = fixture.table();
    table.query_open = Some(open);
    table.selection_open = Some(select);
    table.selection_next = Some(next_with_late_cancel);
    table
}

#[test]
fn metadata_discovery_streams_selector_union_without_cursor_calls_and_retains_body_credit() {
    for sources in [vec![], vec![b"m".as_slice(), br#"{zone=~"value.*"}"#, b"n"]] {
        unsafe {
            let engine = engine();
            let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let selectors: Vec<_> = sources
                .iter()
                .map(|source| selector(engine, source))
                .collect();
            let raw: Vec<_> = selectors.iter().map(|value| value.cast_const()).collect();
            let mut fixture = Fixture::new(1);
            let table = provider(&mut fixture);
            let mut output = Rows::default();
            let sink = output.table();
            let mut report = ptr::null_mut();
            assert_eq!(
                netdata_promql_discover(
                    engine,
                    &table,
                    BOUNDS,
                    raw.as_ptr(),
                    raw.len() as u64,
                    ptr::null(),
                    &sink,
                    &mut report
                ),
                types::OK
            );
            assert_eq!(
                output.rows.borrow().len(),
                if sources.is_empty() { 1 } else { 2 }
            );
            assert_eq!(fixture.opened[3].get(), 0);
            assert_eq!(fixture.samples_returned.get(), 0);
            assert_eq!(output.calls[3].get(), 0);
            for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                assert_eq!(opened.get(), closed.get());
            }
            for value in selectors {
                assert_eq!(netdata_promql_selector_release(value), types::OK);
            }
            assert_eq!(netdata_promql_report_release(report), types::OK);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            assert!(account.snapshot().memory[MemoryKind::Result as usize].current >= 8192);
            output.release_body();
            account.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn metadata_early_stop_closes_current_series_and_does_not_turn_into_cancellation() {
    unsafe {
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let mut fixture = Fixture::new(1);
        fixture.cancel_stage = 7;
        let table = provider(&mut fixture);
        let mut output = Rows {
            stop: true,
            ..Rows::default()
        };
        let sink = output.table();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_discover(
                engine,
                &table,
                BOUNDS,
                ptr::null(),
                0,
                ptr::null(),
                &sink,
                &mut report
            ),
            types::OK
        );
        assert_eq!(
            output.calls.each_ref().map(|value| value.get()),
            [1, 1, 1, 0]
        );
        assert_eq!(fixture.cancelled_status.get(), types::OK);
        assert!(fixture.cancellation.is_null());
        for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
            assert_eq!(opened.get(), closed.get());
        }
        output.release_body();
        assert_eq!(netdata_promql_report_release(report), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        account.snapshot().assert_memory_released();
    }
}

#[test]
fn metadata_validates_malformed_nonmatches_and_preserves_provider_error_before_late_cancel() {
    for malformed in [false, true] {
        unsafe {
            let engine = engine();
            let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let selector = selector(engine, b"n");
            let raw = [selector.cast_const()];
            let mut token = ptr::null_mut();
            assert_eq!(
                netdata_promql_control_create(engine, ptr::null(), &mut token),
                types::OK
            );
            let mut fixture = Fixture::new(1);
            if malformed {
                fixture.labels[1].name = types::Bytes::of(b"__name__");
            } else {
                fixture.failure = 3;
                fixture.cancel_stage = 3;
                fixture.cancellation = token;
            }
            let table = provider(&mut fixture);
            let mut output = Rows::default();
            let sink = output.table();
            let mut report = ptr::null_mut();
            assert_eq!(
                netdata_promql_discover(
                    engine,
                    &table,
                    BOUNDS,
                    raw.as_ptr(),
                    1,
                    token,
                    &sink,
                    &mut report
                ),
                if malformed {
                    types::EVALUATION_ERROR
                } else {
                    types::PROVIDER_ERROR
                }
            );
            assert_eq!(
                output.calls.each_ref().map(|value| value.get()),
                [1, 0, 0, 1]
            );
            assert_eq!(fixture.opened[3].get(), 0);
            for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                assert_eq!(opened.get(), closed.get());
            }
            assert_eq!(netdata_promql_selector_release(selector), types::OK);
            assert_eq!(netdata_promql_control_release(token), types::OK);
            assert_eq!(netdata_promql_report_release(report), types::OK);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            account.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn metadata_output_failures_and_cancellation_abort_once_including_before_begin() {
    for (failure, cancellation) in [(1, 1), (2, 2), (3, 3), (0, 0), (0, 1), (0, 2)] {
        unsafe {
            let engine = engine();
            let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let mut token = ptr::null_mut();
            assert_eq!(
                netdata_promql_control_create(engine, ptr::null(), &mut token),
                types::OK
            );
            if failure == 0 && cancellation == 0 {
                assert_eq!(netdata_promql_control_cancel(token), types::OK);
            }
            let mut fixture = Fixture::new(1);
            let table = provider(&mut fixture);
            let mut output = Rows {
                fail: failure,
                cancel: cancellation,
                token,
                ..Rows::default()
            };
            let sink = output.table();
            let mut report = ptr::null_mut();
            assert_eq!(
                netdata_promql_discover(
                    engine,
                    &table,
                    BOUNDS,
                    ptr::null(),
                    0,
                    token,
                    &sink,
                    &mut report
                ),
                if failure > 0 {
                    types::OUTPUT_ERROR
                } else {
                    types::CANCELLED
                }
            );
            assert_eq!(output.calls[3].get(), 1);
            assert!(output.lease.get().is_null());
            assert_eq!(fixture.opened[3].get(), 0);
            for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                assert_eq!(opened.get(), closed.get());
            }
            assert_eq!(netdata_promql_control_release(token), types::OK);
            assert_eq!(netdata_promql_report_release(report), types::OK);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            account.snapshot().assert_memory_released();
        }
    }
}
