// SPDX-License-Identifier: GPL-3.0-or-later
use super::{getters::*, *};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    ptr,
};

mod allocations;
mod delivery;
mod ownership;
mod pages;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")
))]
mod temporary;

struct Fixture {
    points: Vec<types::Sample>,
    labels: [types::Label; 2],
    page_size: usize,
    samples_returned: Cell<usize>,
    opened: [Cell<usize>; 4],
    closed: [Cell<usize>; 4],
    failure: usize,
    cancel_stage: usize,
    cancellation: *const types::Control,
    cancelled_status: Cell<u32>,
    error_bytes: RefCell<Vec<u8>>,
    diagnostics: Vec<types::ProviderDiagnostic>,
    gates: Option<std::sync::Arc<(std::sync::Barrier, std::sync::Barrier)>>,
}
impl Fixture {
    fn new(page_size: usize) -> Self {
        Self {
            points: (0..17)
                .map(|i| types::Sample {
                    time_ms: i * 1000,
                    start_time_ms: 0,
                    value: types::Value {
                        kind: types::VALUE_FLOAT,
                        reserved: 0,
                        payload: types::ValuePayload {
                            float_bits: (i as f64).to_bits(),
                        },
                    },
                })
                .collect(),
            labels: [
                types::Label {
                    name: types::Bytes::of(b"__name__"),
                    value: types::Bytes::of(b"m"),
                },
                types::Label {
                    name: types::Bytes::of(b"zone"),
                    value: types::Bytes::of(b"value\0\xff"),
                },
            ],
            page_size,
            samples_returned: Cell::new(0),
            opened: std::array::from_fn(|_| Cell::new(0)),
            closed: std::array::from_fn(|_| Cell::new(0)),
            failure: 0,
            cancel_stage: 0,
            cancellation: ptr::null(),
            cancelled_status: Cell::new(types::OK),
            error_bytes: RefCell::new(Vec::new()),
            diagnostics: Vec::new(),
            gates: None,
        }
    }
    fn table(&mut self) -> types::Provider {
        types::Provider {
            header: types::Header::of::<types::Provider>(),
            context: (self as *mut Self).cast(),
            query_open: Some(query_open),
            query_close: Some(query_close),
            query_resources: Some(empty_resources),
            selection_open: Some(selection_open),
            selection_close: Some(selection_close),
            selection_resources: Some(selection_resources),
            selection_next: Some(selection_next),
            series_close: Some(series_close),
            series_resources: Some(empty_resources),
            cursor_open: Some(cursor_open),
            cursor_close: Some(cursor_close),
            cursor_resources: Some(cursor_resources),
            cursor_seek: Some(cursor_seek),
            cursor_next: Some(cursor_next),
            temporary_create: None,
            temporary_read: None,
            temporary_write: None,
            temporary_resources: None,
            temporary_close: None,
        }
    }
    fn opened(&self, index: usize) {
        self.opened[index].set(self.opened[index].get() + 1);
    }
    fn closed(&self, index: usize) {
        self.closed[index].set(self.closed[index].get() + 1);
    }
    fn cancel(&self, stage: usize) {
        if self.cancel_stage == stage {
            self.cancelled_status
                .set(unsafe { netdata_promql_control_cancel(self.cancellation) });
        }
    }
    fn invalidate_error(&self) {
        self.error_bytes.borrow_mut().fill(b'X');
    }
}
struct Cursor {
    index: usize,
    bounds: types::Bounds,
    page: Vec<types::Sample>,
}
unsafe fn fixture<'a>(context: *mut c_void) -> &'a Fixture {
    unsafe { &*context.cast::<Fixture>() }
}
unsafe fn failure(context: *mut c_void, stage: usize, error: *mut types::Bytes) -> bool {
    if unsafe { fixture(context) }.failure == stage {
        unsafe { fixture(context) }.cancel(stage);
        let fixture = unsafe { fixture(context) };
        let mut bytes = fixture.error_bytes.borrow_mut();
        bytes.clear();
        bytes.extend_from_slice(b"provider\0\xfffailure");
        unsafe { error.write(types::Bytes::of(&bytes)) };
        true
    } else {
        false
    }
}
unsafe extern "C" fn query_open(
    context: *mut c_void,
    _: types::Bounds,
    _: *const types::CallbackControl,
    out: *mut *mut c_void,
    error: *mut types::Bytes,
) -> u32 {
    unsafe { fixture(context) }.opened(0);
    if unsafe { failure(context, 1, error) } {
        return types::PROVIDER_ERROR;
    }
    if let Some(gates) = &unsafe { fixture(context) }.gates {
        gates.0.wait();
        gates.1.wait();
    }
    unsafe { out.write(ptr::null_mut()) };
    unsafe { fixture(context) }.cancel(1);
    types::OK
}
unsafe extern "C" fn query_close(
    context: *mut c_void,
    _: *mut c_void,
    _: *const types::AccountingContext,
) {
    unsafe { fixture(context) }.closed(0);
}
unsafe extern "C" fn empty_resources(
    context: *mut c_void,
    _: *mut c_void,
    _: *const types::AccountingContext,
) -> types::Resources {
    unsafe { fixture(context) }.invalidate_error();
    types::Resources::default()
}
unsafe extern "C" fn selection_open(
    context: *mut c_void,
    _: *mut c_void,
    _: *const types::SelectionRequest,
    _: *const types::CallbackControl,
    out: *mut *mut c_void,
    error: *mut types::Bytes,
) -> u32 {
    unsafe { fixture(context) }.opened(1);
    if unsafe { failure(context, 2, error) } {
        return types::PROVIDER_ERROR;
    }
    unsafe { out.write(Box::into_raw(Box::new(false)).cast()) };
    unsafe { fixture(context) }.cancel(2);
    types::OK
}
unsafe extern "C" fn selection_close(
    context: *mut c_void,
    value: *mut c_void,
    _: *const types::AccountingContext,
) {
    unsafe { fixture(context) }.closed(1);
    drop(unsafe { Box::from_raw(value.cast::<bool>()) });
}
unsafe extern "C" fn selection_resources(
    context: *mut c_void,
    _: *mut c_void,
    _: *const types::AccountingContext,
) -> types::Resources {
    unsafe { fixture(context) }.invalidate_error();
    types::Resources {
        held_bytes: size_of::<bool>() as u64,
        shared_cache_bytes: 0,
    }
}
unsafe extern "C" fn selection_next(
    context: *mut c_void,
    value: *mut c_void,
    _: *const types::CallbackControl,
    out: *mut types::SeriesOpen,
    diagnostics: *mut types::ProviderDiagnostics,
    error: *mut types::Bytes,
) -> u32 {
    if unsafe { failure(context, 3, error) } {
        let fixture = unsafe { fixture(context) };
        unsafe {
            diagnostics.write(types::ProviderDiagnostics {
                data: fixture.diagnostics.as_ptr(),
                len: fixture.diagnostics.len() as u64,
            })
        };
        return types::PROVIDER_ERROR;
    }
    let sent = unsafe { &mut *value.cast::<bool>() };
    if *sent {
        let fixture = unsafe { fixture(context) };
        unsafe {
            diagnostics.write(types::ProviderDiagnostics {
                data: fixture.diagnostics.as_ptr(),
                len: fixture.diagnostics.len() as u64,
            })
        };
        fixture.cancel(7);
        return types::END;
    }
    *sent = true;
    let fixture = unsafe { fixture(context) };
    fixture.opened(2);
    unsafe {
        out.write(types::SeriesOpen {
            handle: ptr::null_mut(),
            labels: types::Labels {
                data: fixture.labels.as_ptr(),
                len: fixture.labels.len() as u64,
            },
        })
    };
    fixture.cancel(3);
    types::OK
}
unsafe extern "C" fn series_close(
    context: *mut c_void,
    _: *mut c_void,
    _: *const types::AccountingContext,
) {
    unsafe { fixture(context) }.closed(2);
}
unsafe extern "C" fn cursor_open(
    context: *mut c_void,
    _: *mut c_void,
    bounds: types::Bounds,
    _: *const types::CallbackControl,
    out: *mut types::CursorOpen,
    error: *mut types::Bytes,
) -> u32 {
    let fixture = unsafe { fixture(context) };
    fixture.opened(3);
    if unsafe { failure(context, 4, error) } {
        return types::PROVIDER_ERROR;
    }
    let cursor = Box::new(Cursor {
        index: 0,
        bounds,
        page: Vec::new(),
    });
    unsafe {
        out.write(types::CursorOpen {
            handle: Box::into_raw(cursor).cast(),
            max_page_samples: fixture.page_size as u64,
        })
    };
    fixture.cancel(4);
    types::OK
}
unsafe extern "C" fn cursor_close(
    context: *mut c_void,
    value: *mut c_void,
    _: *const types::AccountingContext,
) {
    unsafe { fixture(context) }.closed(3);
    drop(unsafe { Box::from_raw(value.cast::<Cursor>()) });
}
unsafe extern "C" fn cursor_resources(
    context: *mut c_void,
    value: *mut c_void,
    _: *const types::AccountingContext,
) -> types::Resources {
    unsafe { fixture(context) }.invalidate_error();
    let value = unsafe { &*value.cast::<Cursor>() };
    cursor_memory(value)
}
fn cursor_memory(value: &Cursor) -> types::Resources {
    types::Resources {
        held_bytes: (size_of::<Cursor>() + value.page.capacity() * size_of::<types::Sample>())
            as u64,
        shared_cache_bytes: 7,
    }
}
unsafe extern "C" fn cursor_seek(
    context: *mut c_void,
    value: *mut c_void,
    time_ms: i64,
    _: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    if unsafe { failure(context, 5, error) } {
        return types::PROVIDER_ERROR;
    }
    let value = unsafe { &mut *value.cast::<Cursor>() };
    value.page.clear();
    value.index = unsafe { fixture(context) }
        .points
        .partition_point(|point| point.time_ms < time_ms);
    unsafe { fixture(context) }.cancel(5);
    types::OK
}
unsafe extern "C" fn cursor_next(
    context: *mut c_void,
    value: *mut c_void,
    _: *const types::CallbackControl,
    out: *mut types::Page,
    resources: *mut types::Resources,
    error: *mut types::Bytes,
) -> u32 {
    let cursor = unsafe { &mut *value.cast::<Cursor>() };
    cursor.page.clear();
    let fixture = unsafe { fixture(context) };
    if unsafe { failure(context, 6, error) } {
        unsafe { resources.write(cursor_memory(cursor)) };
        return types::PROVIDER_ERROR;
    }
    while cursor.page.len() < fixture.page_size {
        let Some(point) = fixture.points.get(cursor.index) else {
            break;
        };
        if point.time_ms > cursor.bounds.end_ms {
            break;
        }
        cursor.index += 1;
        if point.time_ms >= cursor.bounds.start_ms {
            cursor.page.push(*point);
        }
    }
    let observed = cursor_memory(cursor);
    unsafe { resources.write(observed) };
    if cursor.page.is_empty() {
        return types::END;
    }
    fixture
        .samples_returned
        .set(fixture.samples_returned.get() + cursor.page.len());
    unsafe {
        out.write(types::Page {
            samples: types::Samples {
                data: cursor.page.as_ptr(),
                len: cursor.page.len() as u64,
            },
            resources: observed,
        })
    };
    fixture.cancel(6);
    types::OK
}
unsafe fn engine() -> *mut types::Engine {
    let mut value = ptr::null_mut();
    assert_eq!(
        unsafe { netdata_promql_engine_create(ptr::null(), &mut value) },
        types::OK
    );
    value
}
unsafe fn compiled(engine: *mut types::Engine, source: &[u8]) -> *mut types::Query {
    let mut query = ptr::null_mut();
    let mut report = ptr::null_mut();
    assert_eq!(
        unsafe {
            netdata_promql_compile(
                engine,
                types::Bytes::of(source),
                ptr::null(),
                &mut query,
                &mut report,
            )
        },
        types::OK
    );
    assert_eq!(unsafe { netdata_promql_report_release(report) }, types::OK);
    query
}
fn instant(time_ms: i64) -> types::Request {
    types::Request {
        header: types::Header::of::<types::Request>(),
        kind: types::REQUEST_INSTANT,
        start_ms: time_ms,
        ..types::Request::default()
    }
}

#[test]
fn paged_c_collection_outlives_all_foreign_and_plan_handles() {
    for page_size in [1, 7, 1024] {
        unsafe {
            let engine = engine();
            let accounting = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let query = compiled(engine, b"m[5s]");
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            let mut fixture = Fixture::new(page_size);
            let table = fixture.table();
            let mut result = ptr::null_mut();
            let mut report = ptr::null_mut();
            assert_eq!(
                netdata_promql_collect(
                    query,
                    &table,
                    &instant(10_000),
                    ptr::null(),
                    &mut result,
                    &mut report
                ),
                types::OK
            );
            for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                assert!(opened.get() > 0);
                assert_eq!(opened.get(), closed.get());
            }
            let mut observations = types::QueryObservations::default();
            assert_eq!(
                netdata_promql_report_observations(report, &mut observations),
                types::OK
            );
            assert_eq!(
                observations.samples_fetched,
                fixture.samples_returned.get() as u64
            );
            assert_eq!(
                observations.memory[MemoryKind::BorrowedView as usize].current,
                0
            );
            assert_eq!(
                observations.memory[MemoryKind::BorrowedView as usize].peak,
                (page_size.min(5) * size_of::<types::Sample>()) as u64
            );
            assert_eq!(netdata_promql_report_release(report), types::OK);
            assert_eq!(netdata_promql_query_release(query), types::OK);
            drop(fixture);
            let mut info = types::ResultInfo::default();
            assert_eq!(netdata_promql_result_info(result, &mut info), types::OK);
            assert_eq!(
                (info.header.value_type, info.series_count),
                (types::TYPE_MATRIX, 1)
            );
            let mut series = types::SeriesInfo::default();
            assert_eq!(
                netdata_promql_result_series_info(result, 0, &mut series),
                types::OK
            );
            assert_eq!((series.label_count, series.point_count), (2, 5));
            let mut label = types::Label::default();
            assert_eq!(
                netdata_promql_result_label(result, 0, 1, &mut label),
                types::OK
            );
            assert_eq!(checked::bytes(label.value).unwrap(), b"value\0\xff");
            for offset in 0..5 {
                let mut point = types::PointInfo::default();
                assert_eq!(
                    netdata_promql_result_point(result, 0, offset, &mut point),
                    types::OK
                );
                assert_eq!(
                    (point.time_ms, point.value_kind, point.value_bits),
                    (
                        (offset as i64 + 6) * 1000,
                        types::VALUE_FLOAT,
                        (offset as f64 + 6.0).to_bits()
                    )
                );
            }
            assert_eq!(netdata_promql_result_release(result), types::OK);
            accounting.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn foreign_failures_keep_bytes_and_close_only_successful_opens() {
    for stage in 1..=6 {
        unsafe {
            let engine = engine();
            let accounting = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            // A matrix requires a physical seek; this instant lookup's seek is a no-op.
            let query = compiled(engine, if stage == 5 { b"m[1s]" } else { b"m" });
            let mut fixture = Fixture::new(1);
            fixture.failure = stage;
            let table = fixture.table();
            let mut result = ptr::null_mut();
            let mut report = ptr::null_mut();
            let status = netdata_promql_collect(
                query,
                &table,
                &instant(10_000),
                ptr::null(),
                &mut result,
                &mut report,
            );
            assert_ne!(status, types::OK, "stage {stage}");
            assert!(result.is_null());
            assert!(!report.is_null());
            let mut error = types::Error::default();
            assert_eq!(
                netdata_promql_report_error(report, 0, &mut error),
                types::OK
            );
            assert!(
                checked::bytes(error.message)
                    .unwrap()
                    .ends_with(b"provider\0\xfffailure")
            );
            for index in 0..4 {
                let failed = (stage == 1 && index == 0)
                    || (stage == 2 && index == 1)
                    || (stage == 4 && index == 3);
                assert_eq!(
                    fixture.opened[index].get(),
                    fixture.closed[index].get() + usize::from(failed),
                    "stage {stage}, owner {index}"
                );
            }
            assert_eq!(netdata_promql_query_release(query), types::OK);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            drop(fixture);
            assert!(
                checked::bytes(error.message)
                    .unwrap()
                    .ends_with(b"provider\0\xfffailure")
            );
            assert_eq!(netdata_promql_report_release(report), types::OK);
            accounting.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn legacy_callback_prefixes_reject_before_table_access() {
    #[repr(align(16))]
    struct Prefix(types::Header);
    let prefix = Prefix(types::Header {
        abi_version: 1,
        struct_size: u32::MAX,
    });
    unsafe {
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_execute(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                (&prefix.0 as *const types::Header).cast(),
                &mut report,
            ),
            types::INVALID_ARGUMENT
        );
        assert!(report.is_null());
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let query = compiled(engine, b"m");
        let mut result = ptr::null_mut();
        assert_eq!(
            netdata_promql_collect(
                query,
                (&prefix.0 as *const types::Header).cast(),
                &instant(3000),
                ptr::null(),
                &mut result,
                &mut report,
            ),
            types::INVALID_ARGUMENT
        );
        assert!(result.is_null() && report.is_null());
        assert_eq!(netdata_promql_query_release(query), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        account.snapshot().assert_memory_released();
    }
}

#[test]
fn short_prefixes_and_invalid_deadlines_reject_before_full_access() {
    #[repr(align(16))]
    struct Prefix(types::Header);
    unsafe {
        let prefix = Prefix(types::Header {
            abi_version: types::ABI_VERSION,
            struct_size: size_of::<types::Header>() as u32,
        });
        let mut engine_value = ptr::null_mut();
        assert_eq!(
            netdata_promql_engine_create(
                (&prefix.0 as *const types::Header).cast(),
                &mut engine_value
            ),
            types::INVALID_ARGUMENT
        );
        assert!(engine_value.is_null());
        let engine_value = engine();
        let mut token = ptr::null_mut();
        let mut options = types::ControlOptions {
            header: types::Header::of::<types::ControlOptions>(),
            deadline_present: 1,
            nanoseconds: 1_000_000_000,
            seconds: 0,
        };
        assert_eq!(
            netdata_promql_control_create(engine_value, &options, &mut token),
            types::INVALID_ARGUMENT
        );
        assert!(token.is_null());
        options.nanoseconds = 0;
        assert_eq!(
            netdata_promql_control_create(engine_value, &options, &mut token),
            types::OK
        );
        let mut state = types::ControlState::default();
        assert_eq!(netdata_promql_control_state(token, &mut state), types::OK);
        assert_eq!((state.status, state.deadline_present), (types::DEADLINE, 1));
        assert_eq!(netdata_promql_control_cancel(token), types::OK);
        assert_eq!(netdata_promql_control_state(token, &mut state), types::OK);
        assert_eq!(state.status, types::CANCELLED);
        assert_eq!(netdata_promql_control_release(token), types::OK);
        assert_eq!(netdata_promql_engine_release(engine_value), types::OK);
    }
}

export! {
fn netdata_promql_test_unwind(cleaned: *const Cell<u32>, entered: *mut u32) {
    struct Guard<'a>(&'a Cell<u32>);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    struct Payload;
    impl Drop for Payload {
        fn drop(&mut self) {
            panic!("secondary payload drop");
        }
    }
    let _guard = Guard(unsafe { &*cleaned });
    unsafe { *entered = 1 };
    std::panic::panic_any(Payload)
}
}

#[test]
fn whole_export_containment_cleans_a_panicking_panic_payload() {
    let cleaned = Cell::new(0);
    let mut entered = 0;
    assert_eq!(
        unsafe { netdata_promql_test_unwind(&cleaned, &mut entered) },
        types::INTERNAL_ERROR
    );
    assert_eq!((entered, cleaned.get()), (1, 1));
    assert_eq!(unsafe { netdata_promql_abi_version() }, types::ABI_VERSION);
}
