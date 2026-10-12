// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{
    fixture_temporary_storage::FileStore,
    storage::{self, TemporaryFloatBits, TemporaryFloatBitsRequest},
    storage_observed::ObservedQuery,
};

struct Token {
    file: FileStore,
    allocation: *mut types::AllocationHandle,
}
#[repr(C)]
struct Probe {
    fixture: Fixture,
    token: Cell<*mut Token>,
    opened: Cell<u64>,
    closed: Cell<u64>,
    phases: Cell<u32>,
    null_token: bool,
    fail: u8,
    status: u32,
    wrong_length: i8,
    cancel: bool,
}
impl Probe {
    fn new() -> Self {
        Self {
            fixture: Fixture::new(7),
            token: Cell::new(ptr::null_mut()),
            opened: Cell::new(0),
            closed: Cell::new(0),
            phases: Cell::new(0),
            null_token: false,
            fail: 0,
            status: types::PROVIDER_ERROR,
            wrong_length: 0,
            cancel: false,
        }
    }
    fn table(&mut self) -> types::Provider {
        let mut table = self.fixture.table();
        table.context = (self as *mut Self).cast();
        table.temporary_create = Some(create);
        table.temporary_read = Some(read);
        table.temporary_write = Some(write);
        table.temporary_resources = Some(resources);
        table.temporary_close = Some(close);
        table
    }
    unsafe fn phase(&self, context: *const types::AccountingContext, phase: u8) {
        self.phases.set(self.phases.get() | (1 << phase));
        let account = unsafe { checked::accounting(context) }.unwrap();
        let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
        let allocation = unsafe { allocations::acquire(account, MemoryKind::Workspace, 257) };
        let scratch = vec![0u8; 257];
        std::hint::black_box(&scratch);
        drop(scratch);
        assert_eq!(
            unsafe { netdata_promql_allocation_release(allocation) },
            types::OK
        );
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].current,
            before
        );
    }
    unsafe fn fault(
        &self,
        phase: u8,
        control: *const types::CallbackControl,
        error: *mut types::Bytes,
    ) -> u32 {
        if self.fail != phase {
            return types::OK;
        }
        if self.cancel {
            assert_eq!(
                unsafe { netdata_promql_control_cancel(self.fixture.cancellation) },
                types::OK
            );
        }
        let mut bytes = self.fixture.error_bytes.borrow_mut();
        bytes.clear();
        bytes.extend_from_slice(b"temporary\0\xfffailure");
        unsafe { error.write(types::Bytes::of(&bytes)) };
        assert!(unsafe { checked::callback(control) }.is_ok());
        self.status
    }
}
unsafe fn probe<'a>(context: *mut c_void) -> &'a Probe {
    unsafe { &*context.cast::<Probe>() }
}
unsafe fn token<'a>(context: *mut c_void, handle: *mut c_void) -> &'a mut Token {
    let state = unsafe { probe(context) };
    assert_eq!(
        handle,
        if state.null_token {
            ptr::null_mut()
        } else {
            state.token.get().cast()
        }
    );
    unsafe { &mut *state.token.get() }
}
unsafe extern "C" fn create(
    context: *mut c_void,
    _: *mut c_void,
    count: u64,
    control: *const types::CallbackControl,
    out: *mut *mut c_void,
    error: *mut types::Bytes,
) -> u32 {
    let state = unsafe { probe(context) };
    let callback = unsafe { checked::callback(control) }.unwrap();
    unsafe { state.phase(checked::accounting_context(callback.account), 1) };
    let status = unsafe { state.fault(1, control, error) };
    if status != types::OK {
        return status;
    }
    let file = FileStore::create(
        TemporaryFloatBitsRequest {
            value_count: count,
            accounting: callback.account,
        },
        callback.control,
    )
    .unwrap();
    let allocation = unsafe {
        allocations::acquire(
            callback.account,
            MemoryKind::ProviderHeld,
            size_of::<Token>() as u64,
        )
    };
    let value = Box::into_raw(Box::new(Token { file, allocation }));
    state.token.set(value);
    state.opened.set(state.opened.get() + 1);
    unsafe {
        out.write(if state.null_token {
            ptr::null_mut()
        } else {
            value.cast()
        })
    };
    types::OK
}
unsafe extern "C" fn read(
    context: *mut c_void,
    handle: *mut c_void,
    offset: u64,
    values: *mut u64,
    count: u64,
    control: *const types::CallbackControl,
    transferred: *mut u64,
    error: *mut types::Bytes,
) -> u32 {
    let state = unsafe { probe(context) };
    let callback = unsafe { checked::callback(control) }.unwrap();
    unsafe { state.phase(checked::accounting_context(callback.account), 2) };
    let status = unsafe { state.fault(2, control, error) };
    if status != types::OK {
        return status;
    }
    let slice = unsafe { std::slice::from_raw_parts_mut(values, count as usize) };
    unsafe { token(context, handle) }
        .file
        .read_into(offset, slice, callback.control)
        .unwrap();
    unsafe {
        transferred.write(if state.fail == 2 {
            count.saturating_add_signed(i64::from(state.wrong_length))
        } else {
            count
        })
    };
    types::OK
}
unsafe extern "C" fn write(
    context: *mut c_void,
    handle: *mut c_void,
    offset: u64,
    values: *const u64,
    count: u64,
    control: *const types::CallbackControl,
    transferred: *mut u64,
    error: *mut types::Bytes,
) -> u32 {
    let state = unsafe { probe(context) };
    let callback = unsafe { checked::callback(control) }.unwrap();
    unsafe { state.phase(checked::accounting_context(callback.account), 3) };
    let status = unsafe { state.fault(3, control, error) };
    if status != types::OK {
        return status;
    }
    let slice = unsafe { checked::slice(values, count) }.unwrap();
    unsafe { token(context, handle) }
        .file
        .write_at(offset, slice, callback.control)
        .unwrap();
    unsafe {
        transferred.write(if state.fail == 3 {
            count.saturating_add_signed(i64::from(state.wrong_length))
        } else {
            count
        })
    };
    types::OK
}
unsafe extern "C" fn resources(
    context: *mut c_void,
    _: *mut c_void,
    account: *const types::AccountingContext,
) -> types::Resources {
    let state = unsafe { probe(context) };
    unsafe { state.phase(account, 4) };
    state.fixture.invalidate_error();
    types::Resources::default()
}
unsafe extern "C" fn close(
    context: *mut c_void,
    handle: *mut c_void,
    account: *const types::AccountingContext,
) {
    let state = unsafe { probe(context) };
    unsafe { state.phase(account, 5) };
    let account = unsafe { checked::accounting(account) }.unwrap();
    assert!(account.snapshot().temporary_storage.stores.current > 0);
    let pointer = state.token.get();
    assert_eq!(
        handle,
        if state.null_token {
            ptr::null_mut()
        } else {
            pointer.cast()
        }
    );
    state.token.set(ptr::null_mut());
    let Token { file, allocation } = *unsafe { Box::from_raw(pointer) };
    drop(file);
    assert_eq!(
        unsafe { netdata_promql_allocation_release(allocation) },
        types::OK
    );
    state.closed.set(state.closed.get() + 1);
    state.fixture.invalidate_error();
}

#[test]
fn abi4_rejects_legacy_short_and_every_partial_temporary_callback_group() {
    let engine = crate::accounting::EngineAccounting::default();
    let account = engine.query();
    #[repr(align(16))]
    struct Prefix(types::Header);
    for (version, size) in [
        (1, u32::MAX),
        (2, u32::MAX),
        (3, u32::MAX),
        (
            types::ABI_VERSION,
            (size_of::<types::Provider>() - 5 * size_of::<usize>()) as u32,
        ),
    ] {
        let prefix = Prefix(types::Header {
            abi_version: version,
            struct_size: size,
        });
        assert!(
            unsafe {
                provider::Provider::new((&prefix.0 as *const types::Header).cast(), &account)
            }
            .is_err()
        );
    }
    let mut state = Probe::new();
    let complete = state.table();
    for mask in 0..32 {
        let mut table = complete;
        if mask & 1 == 0 {
            table.temporary_create = None;
        }
        if mask & 2 == 0 {
            table.temporary_read = None;
        }
        if mask & 4 == 0 {
            table.temporary_write = None;
        }
        if mask & 8 == 0 {
            table.temporary_resources = None;
        }
        if mask & 16 == 0 {
            table.temporary_close = None;
        }
        assert_eq!(
            unsafe { provider::Provider::new(&table, &account) }.is_ok(),
            mask == 0 || mask == 31
        );
    }
    assert_eq!(state.fixture.opened[0].get(), 0);
    account.snapshot().assert_memory_released();
    drop(account);
    engine.snapshot().assert_memory_released();
}

#[test]
fn temporary_callbacks_preserve_bits_wide_offsets_errors_lengths_and_null_ownership() {
    for (fail, status, wrong_length, null_token, cancel) in [
        (0, types::OK, 0, false, false),
        (0, types::OK, 0, true, false),
        (1, types::PROVIDER_ERROR, 0, false, false),
        (2, types::PROVIDER_ERROR, 0, false, true),
        (3, types::PROVIDER_ERROR, 0, false, true),
        (2, types::OK, -1, false, false),
        (2, types::OK, 1, false, false),
        (3, types::OK, -1, false, false),
        (3, types::OK, 1, false, false),
        (2, types::END, 0, false, false),
        (3, u32::MAX, 0, false, false),
    ] {
        let engine = unsafe { engine() };
        let accounting = unsafe { borrow::<Engine, _>(engine) }
            .unwrap()
            .core
            .accounting
            .clone();
        let account = accounting.query();
        let mut options = types::ControlOptions::default();
        unsafe { netdata_promql_control_options_default(&mut options) };
        let mut cancellation = ptr::null_mut();
        assert_eq!(
            unsafe { netdata_promql_control_create(engine, &options, &mut cancellation) },
            types::OK
        );
        let mut state = Probe {
            fail,
            status,
            wrong_length,
            null_token,
            cancel,
            ..Probe::new()
        };
        state.fixture.cancellation = cancellation;
        let table = state.table();
        let foreign = unsafe { provider::Provider::new(&table, &account) }.unwrap();
        let control = unsafe { super::super::control(cancellation, &accounting) }.unwrap();
        let query = storage::Provider::open(
            &foreign,
            storage::TimeBounds {
                start_ms: 0,
                end_ms: 1,
            },
            &control,
        )
        .unwrap();
        let query = ObservedQuery::new(query, &account);
        let count = (1u64 << 32) + 17;
        let request = TemporaryFloatBitsRequest {
            value_count: count,
            accounting: &account,
        };
        let result =
            storage::QueryProvider::with_temporary_float_bits(&query, request, &control, |store| {
                let expected = [
                    0x8000_0000_0000_0000,
                    0x7ff0_0000_0000_0044,
                    0xfff8_0000_0000_0045,
                ];
                store.write_at(count - 3, &expected, &control)?;
                let mut bits = [0; 3];
                store.read_into(count - 3, &mut bits, &control)?;
                assert_eq!(bits, expected);
                assert!(matches!(
                    store.read_into(count, &mut bits, &control),
                    Err(StorageError::Invalid("temporary transfer out of bounds"))
                ));
                Ok::<_, StorageError>(())
            })
            .and_then(|result| result);
        if fail == 0 {
            assert!(result.is_ok());
            assert_eq!(state.phases.get(), 0b111110);
        } else if status == types::PROVIDER_ERROR {
            let Err(StorageError::Provider(bytes)) = &result else {
                panic!("specific storage failure");
            };
            assert_eq!(bytes.as_ref(), b"temporary\0\xfffailure");
        } else {
            assert!(matches!(result, Err(StorageError::Invalid(_))));
        }
        assert!(state.token.get().is_null());
        assert_eq!(state.opened.get(), state.closed.get());
        drop(result);
        drop(query);
        account.snapshot().assert_memory_released();
        drop(account);
        assert_eq!(
            unsafe { netdata_promql_control_release(cancellation) },
            types::OK
        );
        assert_eq!(unsafe { netdata_promql_engine_release(engine) }, types::OK);
        drop(control);
        accounting.snapshot().assert_memory_released();
    }
}
