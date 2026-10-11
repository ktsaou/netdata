// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

const TRANSIENT_BYTES: usize = 1024 * 1024;

unsafe extern "C" fn transient_open(
    context: *mut c_void,
    bounds: types::Bounds,
    control: *const types::CallbackControl,
    out: *mut *mut c_void,
    error: *mut types::Bytes,
) -> u32 {
    let mut scratch = vec![0u8; TRANSIENT_BYTES];
    let mut account = ptr::null();
    assert_eq!(
        unsafe { netdata_promql_callback_accounting(control, &mut account) },
        types::OK
    );
    let mut allocation = ptr::null_mut();
    assert_eq!(
        unsafe {
            netdata_promql_allocation_acquire(
                account,
                MemoryKind::ProviderHeld as u32,
                scratch.capacity() as u64,
                &mut allocation,
            )
        },
        types::OK
    );
    std::hint::black_box(&mut scratch);
    drop(scratch);
    assert_eq!(
        unsafe { netdata_promql_allocation_release(allocation) },
        types::OK
    );
    unsafe { query_open(context, bounds, control, out, error) }
}

#[test]
fn temporary_foreign_allocation_is_observed_before_a_zero_resource_snapshot() {
    unsafe {
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let query = compiled(engine, b"m");
        let mut fixture = Fixture::new(1);
        let mut provider = fixture.table();
        provider.query_open = Some(transient_open);
        let mut result = ptr::null_mut();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_collect(
                query,
                &provider,
                &instant(1000),
                ptr::null(),
                &mut result,
                &mut report
            ),
            types::OK
        );
        let mut observed = types::QueryObservations::default();
        assert_eq!(
            netdata_promql_report_observations(report, &mut observed),
            types::OK
        );
        assert_eq!(netdata_promql_result_release(result), types::OK);
        assert_eq!(netdata_promql_report_release(report), types::OK);
        assert_eq!(netdata_promql_query_release(query), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        account.snapshot().assert_memory_released();
        assert_eq!(
            observed.memory[MemoryKind::ProviderHeld as usize].peak,
            TRANSIENT_BYTES as u64
        );
        assert_eq!(
            observed.memory[MemoryKind::ProviderHeld as usize].current,
            0
        );
    }
}

pub(super) unsafe fn acquire(
    account: &QueryAccounting,
    kind: MemoryKind,
    bytes: u64,
) -> *mut types::AllocationHandle {
    let mut handle = ptr::null_mut();
    assert_eq!(
        unsafe {
            netdata_promql_allocation_acquire(
                checked::accounting_context(account),
                kind as u32,
                bytes,
                &mut handle,
            )
        },
        types::OK
    );
    assert!(!handle.is_null());
    handle
}

#[test]
fn foreign_overlap_resize_and_wrapper_bytes_are_precise_and_keep_the_creator() {
    let creator = crate::accounting::EngineAccounting::default();
    let other = crate::accounting::EngineAccounting::default();
    let account = creator.query();
    let other_before = other.snapshot();
    let before = account.snapshot();
    let wrapper = Strong::<ForeignAllocation>::allocation_bytes();
    let accounting = MemoryKind::Accounting as usize;
    let kind = MemoryKind::ProviderHeld as usize;
    unsafe {
        let first = acquire(&account, MemoryKind::ProviderHeld, 1024);
        let second = acquire(&account, MemoryKind::ProviderHeld, 2048);
        assert_eq!(account.snapshot().memory[kind].current, 3072);
        assert_eq!(
            account.snapshot().memory[accounting].current,
            before.memory[accounting].current + 2 * wrapper
        );
        assert_eq!(netdata_promql_allocation_resize(first, 4096), types::OK);
        assert_eq!(account.snapshot().memory[kind].peak, 6144);
        assert_eq!(netdata_promql_allocation_resize(first, 0), types::OK);
        assert_eq!(netdata_promql_allocation_resize(second, 3072), types::OK);
        assert_eq!(account.snapshot().memory[kind].current, 3072);
        assert_eq!(netdata_promql_allocation_release(first), types::OK);
        let observed = account.snapshot();
        assert_eq!(
            observed.memory[accounting].current,
            before.memory[accounting].current + wrapper
        );
        assert_eq!(observed.memory[kind].acquired, 7168);
        assert_eq!(observed.memory[kind].released, 4096);
        drop(account);
        assert_eq!(creator.snapshot().memory[kind].current, 3072);
        assert_eq!(netdata_promql_allocation_resize(second, 0), types::OK);
        assert_eq!(creator.snapshot().memory[kind].current, 0);
        assert_eq!(
            creator.snapshot().memory[accounting].current,
            crate::accounting::EngineAccounting::allocation_bytes()
                + QueryAccounting::allocation_bytes()
                + wrapper
        );
        assert_eq!(netdata_promql_allocation_release(second), types::OK);
    }
    creator.snapshot().assert_memory_released();
    assert_eq!(other.snapshot(), other_before);
}

#[test]
fn allocation_shape_and_category_failures_do_not_acquire_a_lease() {
    let engine = crate::accounting::EngineAccounting::default();
    let account = engine.query();
    let before = account.snapshot();
    unsafe {
        let live = acquire(&account, MemoryKind::Workspace, 0);
        for kind in [4, 5, 6, 7, u32::MAX] {
            let mut handle = live;
            assert_eq!(
                netdata_promql_allocation_acquire(
                    checked::accounting_context(&account),
                    kind,
                    100,
                    &mut handle
                ),
                types::INVALID_ARGUMENT
            );
            assert!(handle.is_null());
        }
        let mut handle = live;
        assert_eq!(
            netdata_promql_allocation_acquire(
                ptr::null(),
                MemoryKind::Workspace as u32,
                100,
                &mut handle
            ),
            types::INVALID_ARGUMENT
        );
        assert!(handle.is_null());
        assert_eq!(
            netdata_promql_allocation_acquire(
                checked::accounting_context(&account),
                MemoryKind::Workspace as u32,
                100,
                ptr::null_mut()
            ),
            types::INVALID_ARGUMENT
        );
        assert_eq!(
            netdata_promql_allocation_resize(ptr::null(), 0),
            types::INVALID_ARGUMENT
        );
        assert_eq!(
            netdata_promql_allocation_release(ptr::null_mut()),
            types::INVALID_ARGUMENT
        );
        let mut context = checked::accounting_context(&account);
        assert_eq!(
            netdata_promql_callback_accounting(ptr::null(), &mut context),
            types::INVALID_ARGUMENT
        );
        assert!(context.is_null());
        for kind in [
            MemoryKind::Compiled,
            MemoryKind::Workspace,
            MemoryKind::Result,
            MemoryKind::ProviderHeld,
        ] {
            let handle = acquire(&account, kind, 31);
            assert_eq!(account.snapshot().memory[kind as usize].current, 31);
            assert_eq!(netdata_promql_allocation_release(handle), types::OK);
        }
        assert_eq!(netdata_promql_allocation_release(live), types::OK);
    }
    let observed = account.snapshot();
    assert_eq!(
        observed.memory[MemoryKind::Accounting as usize].current,
        before.memory[MemoryKind::Accounting as usize].current
    );
    account.snapshot().assert_memory_released();
    drop(account);
    engine.snapshot().assert_memory_released();
}

#[repr(C)]
struct Metered {
    fixture: Fixture,
    account: RefCell<Option<QueryAccounting>>,
    phases: Cell<u32>,
    held: RefCell<Vec<u8>>,
    lease: Cell<*mut types::AllocationHandle>,
}
impl Metered {
    fn new() -> Self {
        Self {
            fixture: Fixture::new(1),
            account: RefCell::new(None),
            phases: Cell::new(0),
            held: RefCell::new(Vec::new()),
            lease: Cell::new(ptr::null_mut()),
        }
    }
    fn table(&mut self) -> types::Provider {
        let mut table = self.fixture.table();
        table.context = (self as *mut Self).cast();
        table.query_open = Some(metered_open);
        table.query_resources = Some(metered_query_resources);
        table.query_close = Some(metered_query_close);
        table.selection_resources = Some(metered_selection_resources);
        table.selection_close = Some(metered_selection_close);
        table.series_resources = Some(metered_series_resources);
        table.series_close = Some(metered_series_close);
        table.cursor_resources = Some(metered_cursor_resources);
        table.cursor_close = Some(metered_cursor_close);
        table
    }
    unsafe fn probe(&self, context: *const types::AccountingContext, phase: u32) {
        let account = unsafe { checked::accounting(context) }.unwrap();
        let mut original = self.account.borrow_mut();
        if let Some(original) = &*original {
            assert_eq!(original.test_identity(), account.test_identity());
        } else {
            *original = Some(account.clone());
        }
        drop(original);
        self.phases.set(self.phases.get() | (1 << phase));
        let before = account.snapshot();
        let scratch = vec![0u8; 256];
        let lease = unsafe { acquire(account, MemoryKind::Workspace, scratch.capacity() as u64) };
        assert_eq!(
            account.snapshot().memory[1].current,
            before.memory[1].current + 256
        );
        assert_eq!(
            account.snapshot().memory[7].current,
            before.memory[7].current + Strong::<ForeignAllocation>::allocation_bytes()
        );
        std::hint::black_box(&scratch);
        drop(scratch);
        assert_eq!(
            unsafe { netdata_promql_allocation_release(lease) },
            types::OK
        );
        assert_eq!(
            account.snapshot().memory[1].current,
            before.memory[1].current
        );
        assert_eq!(
            account.snapshot().memory[7].current,
            before.memory[7].current
        );
    }
}
unsafe fn metered<'a>(context: *mut c_void) -> &'a Metered {
    unsafe { &*context.cast::<Metered>() }
}
unsafe extern "C" fn metered_open(
    context: *mut c_void,
    bounds: types::Bounds,
    control: *const types::CallbackControl,
    out: *mut *mut c_void,
    error: *mut types::Bytes,
) -> u32 {
    let mut account = ptr::null();
    assert_eq!(
        unsafe { netdata_promql_callback_accounting(control, &mut account) },
        types::OK
    );
    let state = unsafe { metered(context) };
    unsafe { state.probe(account, 0) };
    let status = unsafe { query_open(context, bounds, control, out, error) };
    if status == types::OK {
        let mut held = state.held.borrow_mut();
        held.resize(4096, 0);
        state.lease.set(unsafe {
            acquire(
                checked::accounting(account).unwrap(),
                MemoryKind::ProviderHeld,
                held.capacity() as u64,
            )
        });
    }
    status
}
unsafe extern "C" fn metered_query_resources(
    context: *mut c_void,
    _: *mut c_void,
    account: *const types::AccountingContext,
) -> types::Resources {
    let state = unsafe { metered(context) };
    unsafe { state.probe(account, 1) };
    // The retained query buffer has its own lease and is excluded from this snapshot.
    types::Resources::default()
}
unsafe extern "C" fn metered_query_close(
    context: *mut c_void,
    value: *mut c_void,
    account: *const types::AccountingContext,
) {
    let state = unsafe { metered(context) };
    unsafe { state.probe(account, 2) };
    drop(std::mem::take(&mut *state.held.borrow_mut()));
    assert_eq!(
        unsafe { netdata_promql_allocation_release(state.lease.replace(ptr::null_mut())) },
        types::OK
    );
    unsafe { query_close(context, value, account) };
}
macro_rules! cleanup_probes {
    ($(($resource:ident, $close:ident, $original_resource:ident, $original_close:ident, $phase:literal)),*) => { $(
        unsafe extern "C" fn $resource(context: *mut c_void, value: *mut c_void, account: *const types::AccountingContext) -> types::Resources {
            unsafe { metered(context).probe(account, $phase) };
            unsafe { $original_resource(context, value, account) }
        }
        unsafe extern "C" fn $close(context: *mut c_void, value: *mut c_void, account: *const types::AccountingContext) {
            unsafe { metered(context).probe(account, $phase + 1) };
            unsafe { $original_close(context, value, account) };
        }
    )* };
}
cleanup_probes!(
    (
        metered_selection_resources,
        metered_selection_close,
        selection_resources,
        selection_close,
        3
    ),
    (
        metered_series_resources,
        metered_series_close,
        empty_resources,
        series_close,
        5
    ),
    (
        metered_cursor_resources,
        metered_cursor_close,
        cursor_resources,
        cursor_close,
        7
    )
);

#[test]
fn resources_and_all_provider_closes_share_the_actual_account_and_exclude_leased_storage() {
    unsafe {
        let engine = engine();
        let root = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let query = compiled(engine, b"m[1s]");
        let mut state = Metered::new();
        let provider = state.table();
        let mut result = ptr::null_mut();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_collect(
                query,
                &provider,
                &instant(1000),
                ptr::null(),
                &mut result,
                &mut report
            ),
            types::OK
        );
        assert_eq!(state.phases.get(), (1 << 9) - 1);
        assert_eq!(
            state.account.borrow().as_ref().unwrap().test_identity(),
            borrow::<Report, _>(report).unwrap().account.test_identity()
        );
        assert!(state.held.borrow().is_empty());
        assert!(state.lease.get().is_null());
        let observed = state.account.borrow().as_ref().unwrap().snapshot();
        assert_eq!(
            observed.memory[MemoryKind::ProviderHeld as usize].current,
            0
        );
        // Descendant snapshots add only their own small cursor/selection state.
        assert!(observed.memory[MemoryKind::ProviderHeld as usize].peak < 2 * 4096);
        assert!(observed.memory[MemoryKind::ProviderHeld as usize].peak >= 4096);
        assert_eq!(netdata_promql_result_release(result), types::OK);
        assert_eq!(netdata_promql_report_release(report), types::OK);
        assert_eq!(netdata_promql_query_release(query), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        drop(state);
        root.snapshot().assert_memory_released();
    }
}

#[derive(Default)]
struct AbortProbe {
    calls: [Cell<usize>; 4],
    account: RefCell<Option<QueryAccounting>>,
    failed: usize,
    retain_output: bool,
    held: RefCell<Vec<u8>>,
    lease: Cell<*mut types::AllocationHandle>,
}
impl AbortProbe {
    fn table(&mut self) -> types::Output {
        types::Output {
            header: types::Header::of::<types::Output>(),
            context: (self as *mut Self).cast(),
            begin: Some(probe_begin),
            chunk: Some(probe_chunk),
            commit: Some(probe_commit),
            abort: Some(probe_abort),
        }
    }
    unsafe fn probe(&self, context: *const types::AccountingContext) {
        let account = unsafe { checked::accounting(context) }.unwrap();
        let mut original = self.account.borrow_mut();
        if let Some(original) = &*original {
            assert_eq!(original.test_identity(), account.test_identity());
        } else {
            *original = Some(account.clone());
        }
        drop(original);
        let before = account.snapshot();
        let scratch = vec![0u8; 512];
        let handle = unsafe { acquire(account, MemoryKind::Workspace, scratch.capacity() as u64) };
        assert_eq!(
            account.snapshot().memory[1].current,
            before.memory[1].current + 512
        );
        std::hint::black_box(&scratch);
        drop(scratch);
        assert_eq!(
            unsafe { netdata_promql_allocation_release(handle) },
            types::OK
        );
        assert_eq!(
            account.snapshot().memory[1].current,
            before.memory[1].current
        );
    }
    unsafe fn called(
        &self,
        index: usize,
        control: *const types::CallbackControl,
        error: *mut types::Bytes,
    ) -> u32 {
        self.calls[index].set(self.calls[index].get() + 1);
        let mut context = ptr::null();
        assert_eq!(
            unsafe { netdata_promql_callback_accounting(control, &mut context) },
            types::OK
        );
        unsafe { self.probe(context) };
        if self.retain_output && self.lease.get().is_null() {
            let mut held = self.held.borrow_mut();
            held.resize(4096, 0);
            self.lease.set(unsafe {
                acquire(
                    checked::accounting(context).unwrap(),
                    MemoryKind::Result,
                    held.capacity() as u64,
                )
            });
        }
        if self.failed == index + 1 {
            unsafe { error.write(types::Bytes::of(b"metered output failure")) };
            types::OUTPUT_ERROR
        } else {
            types::OK
        }
    }
}
unsafe fn abort_probe<'a>(context: *mut c_void) -> &'a AbortProbe {
    unsafe { &*context.cast::<AbortProbe>() }
}
unsafe extern "C" fn probe_begin(
    context: *mut c_void,
    _: *const types::OutputHeader,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    unsafe { abort_probe(context).called(0, control, error) }
}
unsafe extern "C" fn probe_chunk(
    context: *mut c_void,
    _: *const types::Chunk,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    unsafe { abort_probe(context).called(1, control, error) }
}
unsafe extern "C" fn probe_commit(
    context: *mut c_void,
    _: *const types::QueryObservations,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    unsafe { abort_probe(context).called(2, control, error) }
}
unsafe extern "C" fn probe_abort(context: *mut c_void, account: *const types::AccountingContext) {
    let state = unsafe { abort_probe(context) };
    state.calls[3].set(state.calls[3].get() + 1);
    if !account.is_null() {
        unsafe { state.probe(account) };
    }
}

#[test]
fn valid_query_aborts_before_begin_keep_accounting_and_invalid_queries_do_not_invent_it() {
    for case in 0..8 {
        unsafe {
            let engine = engine();
            let root = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let query = compiled(engine, if case == 7 { b"m[step()]" } else { b"m" });
            let mut fixture = Fixture::new(1);
            let mut provider = fixture.table();
            let mut state = AbortProbe::default();
            let output = state.table();
            let mut report = ptr::null_mut();
            let mut request = instant(1000);
            let mut token = ptr::null_mut();
            let mut control = ptr::null();
            let aligned = [0u64; 2];
            if case == 2 {
                request.kind = u32::MAX;
            }
            if case == 3 {
                control = aligned
                    .as_ptr()
                    .cast::<u8>()
                    .add(1)
                    .cast::<types::Control>();
            }
            if case == 5 {
                assert_eq!(
                    netdata_promql_control_create(engine, ptr::null(), &mut token),
                    types::OK
                );
                assert_eq!(netdata_promql_control_cancel(token), types::OK);
                control = token;
            }
            if case == 6 {
                request.kind = types::REQUEST_RANGE;
                request.start_ms = 0;
                request.end_ms = 1;
                request.step_ms = 0;
            }
            if case == 4 {
                provider.header.abi_version = 1;
            }
            let status = netdata_promql_execute(
                if case == 0 { ptr::null() } else { query },
                &provider,
                if case == 1 { ptr::null() } else { &request },
                control,
                &output,
                &mut report,
            );
            assert_eq!(
                status,
                match case {
                    0..=4 => types::INVALID_ARGUMENT,
                    5 => types::CANCELLED,
                    _ => types::EVALUATION_ERROR,
                },
                "case {case}"
            );
            assert_eq!(
                state.calls.iter().map(Cell::get).collect::<Vec<_>>(),
                [0, 0, 0, 1]
            );
            assert_eq!(state.account.borrow().is_some(), case != 0);
            if !report.is_null() {
                assert_eq!(
                    state.account.borrow().as_ref().unwrap().test_identity(),
                    borrow::<Report, _>(report).unwrap().account.test_identity()
                );
                let observed = state.account.borrow().as_ref().unwrap().snapshot();
                assert!(observed.memory[1].peak >= 512);
                assert_eq!(observed.memory[1].current, 0);
                assert_eq!(netdata_promql_report_release(report), types::OK);
            }
            if !token.is_null() {
                assert_eq!(netdata_promql_control_release(token), types::OK);
            }
            assert_eq!(netdata_promql_query_release(query), types::OK);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            drop(state);
            root.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn output_callbacks_and_abort_share_the_same_account_and_committed_handles_can_outlive_it() {
    for failed in 0..=3 {
        unsafe {
            let engine = engine();
            let root = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let query = compiled(engine, b"m");
            let mut fixture = Fixture::new(1);
            let provider = fixture.table();
            let mut state = AbortProbe {
                failed,
                retain_output: failed == 0,
                ..AbortProbe::default()
            };
            let output = state.table();
            let mut report = ptr::null_mut();
            assert_eq!(
                netdata_promql_execute(
                    query,
                    &provider,
                    &instant(1000),
                    ptr::null(),
                    &output,
                    &mut report
                ),
                if failed == 0 {
                    types::OK
                } else {
                    types::OUTPUT_ERROR
                }
            );
            assert_eq!(state.calls[3].get(), usize::from(failed != 0));
            assert_eq!(
                state.account.borrow().as_ref().unwrap().test_identity(),
                borrow::<Report, _>(report).unwrap().account.test_identity()
            );
            let held = state.lease.get();
            let mut buffer = std::mem::take(&mut *state.held.borrow_mut());
            let mut observed = types::QueryObservations::default();
            assert_eq!(
                netdata_promql_report_live_observations(report, &mut observed),
                types::OK
            );
            assert!(held.is_null() || observed.memory[2].current >= 4096);
            assert_eq!(netdata_promql_report_release(report), types::OK);
            assert_eq!(netdata_promql_query_release(query), types::OK);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            drop(state);
            if !held.is_null() {
                assert_eq!(root.snapshot().memory[2].current, 4096);
                buffer.resize(8192, 0);
                assert_eq!(
                    netdata_promql_allocation_resize(held, buffer.capacity() as u64),
                    types::OK
                );
                assert_eq!(root.snapshot().memory[2].current, buffer.capacity() as u64);
                drop(buffer);
                assert_eq!(netdata_promql_allocation_release(held), types::OK);
            }
            root.snapshot().assert_memory_released();
        }
    }
}

unsafe extern "C" fn overlapping_open(
    context: *mut c_void,
    bounds: types::Bounds,
    control: *const types::CallbackControl,
    out: *mut *mut c_void,
    error: *mut types::Bytes,
) -> u32 {
    let mut account = ptr::null();
    assert_eq!(
        unsafe { netdata_promql_callback_accounting(control, &mut account) },
        types::OK
    );
    let scratch = vec![0u8; TRANSIENT_BYTES];
    let lease = unsafe {
        acquire(
            checked::accounting(account).unwrap(),
            MemoryKind::ProviderHeld,
            scratch.capacity() as u64,
        )
    };
    let result = unsafe { query_open(context, bounds, control, out, error) };
    std::hint::black_box(&scratch);
    drop(scratch);
    assert_eq!(
        unsafe { netdata_promql_allocation_release(lease) },
        types::OK
    );
    result
}

#[test]
fn concurrent_execution_leases_have_distinct_queries_and_an_exact_shared_peak() {
    struct SharedPlan(*const types::Query);
    // Immutable plans stay live until all workers join and all paired releases finish.
    unsafe impl Sync for SharedPlan {}
    impl SharedPlan {
        fn get(&self) -> *const types::Query {
            self.0
        }
    }
    unsafe {
        let engine = engine();
        let root = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let query = SharedPlan(compiled(engine, b"m"));
        let gates = std::sync::Arc::new((std::sync::Barrier::new(3), std::sync::Barrier::new(3)));
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for _ in 0..2 {
                let (query, gates) = (&query, gates.clone());
                workers.push(scope.spawn(move || {
                    let mut fixture = Fixture::new(1);
                    fixture.gates = Some(gates);
                    let mut provider = fixture.table();
                    provider.query_open = Some(overlapping_open);
                    let mut result = ptr::null_mut();
                    let mut report = ptr::null_mut();
                    assert_eq!(
                        netdata_promql_collect(
                            query.get(),
                            &provider,
                            &instant(1000),
                            ptr::null(),
                            &mut result,
                            &mut report
                        ),
                        types::OK
                    );
                    let report_value = borrow::<Report, _>(report).unwrap();
                    let identity = report_value.account.test_identity();
                    let observed = report_value.account.snapshot();
                    assert_eq!(observed.memory[3].peak, TRANSIENT_BYTES as u64);
                    assert_eq!(observed.memory[3].current, 0);
                    assert_eq!(netdata_promql_result_release(result), types::OK);
                    assert_eq!(netdata_promql_report_release(report), types::OK);
                    identity
                }));
            }
            gates.0.wait();
            let observed = root.snapshot();
            assert_eq!(observed.active_executions, 2);
            assert_eq!(observed.memory[3].current, (2 * TRANSIENT_BYTES) as u64);
            assert_eq!(observed.memory[3].peak, (2 * TRANSIENT_BYTES) as u64);
            gates.1.wait();
            let identities = workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>();
            assert_ne!(identities[0], identities[1]);
        });
        assert_eq!(netdata_promql_query_release(query.0.cast_mut()), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        root.snapshot().assert_memory_released();
    }
}
