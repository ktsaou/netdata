// SPDX-License-Identifier: GPL-3.0-or-later
//! Literal pinned sorting over provider backing with two bounded dirty pages.
use crate::{
    accounting::{AccountedBox, AllocationContext, MemoryKind, QueryAccounting},
    go_sort_program::{Operation, Program},
    range_quantile_rank::Plan,
    storage::{
        Control, QueryProvider, StorageError, TemporaryFloatBits, TemporaryFloatBitsRequest,
    },
};

const PAGE_VALUES: usize = 1024;
type Buffer = AccountedBox<[u64; PAGE_VALUES]>;

pub(crate) struct Writer<'a> {
    store: &'a mut dyn TemporaryFloatBits,
    buffer: Buffer,
    count: u64,
    written: u64,
    len: usize,
    control: &'a Control,
    account: &'a QueryAccounting,
}
impl Writer<'_> {
    pub(crate) fn push(&mut self, bits: u64) -> Result<(), StorageError> {
        self.control.check()?;
        self.account.work(1);
        if self.written + self.len as u64 >= self.count {
            return Err(StorageError::Invalid(
                "temporary float replay membership changed",
            ));
        }
        self.buffer[self.len] = bits;
        self.len += 1;
        if self.len == PAGE_VALUES {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), StorageError> {
        if self.len != 0 {
            write(
                self.store,
                self.written,
                &self.buffer[..self.len],
                self.control,
                self.account,
            )?;
            self.written += self.len as u64;
            self.len = 0;
        }
        Ok(())
    }
}

fn write(
    store: &mut dyn TemporaryFloatBits,
    offset: u64,
    values: &[u64],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), StorageError> {
    control.check()?;
    account.provider_call();
    account.temporary_transfer(true, None);
    store
        .write_at(offset, values, control)
        .map_err(|error| error.accounted(account))?;
    account.temporary_transfer(true, Some(values.len() as u64 * 8));
    control.check()?;
    Ok(())
}

struct Page {
    values: Buffer,
    base: Option<u64>,
    len: usize,
    dirty: bool,
}
struct Cache<'a> {
    store: &'a mut dyn TemporaryFloatBits,
    pages: [Page; 2],
    recent: usize,
    count: u64,
    control: &'a Control,
    account: &'a QueryAccounting,
}
impl Cache<'_> {
    fn page(&mut self, index: u64) -> Result<(usize, usize), StorageError> {
        self.control.check()?;
        if index >= self.count {
            return Err(StorageError::Invalid("temporary sort index out of bounds"));
        }
        let base = index - index % PAGE_VALUES as u64;
        let slot = self.pages.iter().position(|page| page.base == Some(base));
        let slot = match slot {
            Some(slot) => slot,
            None => {
                let slot = self
                    .pages
                    .iter()
                    .position(|page| page.base.is_none())
                    .unwrap_or(1 - self.recent);
                let page = &mut self.pages[slot];
                if page.dirty {
                    write(
                        self.store,
                        page.base.expect("dirty page"),
                        &page.values[..page.len],
                        self.control,
                        self.account,
                    )?;
                    page.dirty = false;
                }
                page.base = None;
                page.len = (self.count - base).min(PAGE_VALUES as u64) as usize;
                self.account.provider_call();
                self.account.temporary_transfer(false, None);
                self.store
                    .read_into(base, &mut page.values[..page.len], self.control)
                    .map_err(|error| error.accounted(self.account))?;
                self.account
                    .temporary_transfer(false, Some(page.len as u64 * 8));
                self.control.check()?;
                page.base = Some(base);
                slot
            }
        };
        self.recent = slot;
        Ok((slot, (index - base) as usize))
    }
    fn get(&mut self, index: u64) -> Result<u64, StorageError> {
        let (page, offset) = self.page(index)?;
        Ok(self.pages[page].values[offset])
    }
    fn set(&mut self, index: u64, bits: u64) -> Result<(), StorageError> {
        let (page, offset) = self.page(index)?;
        self.pages[page].values[offset] = bits;
        self.pages[page].dirty = true;
        Ok(())
    }
}

pub(crate) fn quantile<Q: QueryProvider, E: From<StorageError>>(
    query: &Q,
    count: u64,
    plan: Plan,
    control: &Control,
    account: &QueryAccounting,
    materialize: impl FnOnce(&mut Writer<'_>) -> Result<(), E>,
) -> Result<f64, E> {
    let request = TemporaryFloatBitsRequest {
        value_count: count,
        accounting: account,
    };
    request.extent_bytes()?;
    control.check().map_err(StorageError::from)?;
    account.provider_call();
    query
        .with_temporary_float_bits(request, control, |store| {
            control.check().map_err(StorageError::from)?;
            let context = AllocationContext::new(account, MemoryKind::Workspace);
            let mut writer = Writer {
                store,
                buffer: AccountedBox::new([0; PAGE_VALUES], &context),
                count,
                written: 0,
                len: 0,
                control,
                account,
            };
            materialize(&mut writer)?;
            if writer.written + writer.len as u64 != count {
                return Err(
                    StorageError::Invalid("temporary float replay membership changed").into(),
                );
            }
            writer.flush()?;
            let Writer { store, buffer, .. } = writer;
            // Materialization has closed its raw cursor before the second value page is allocated.
            let mut cache = Cache {
                store,
                pages: [
                    Page {
                        values: buffer,
                        base: None,
                        len: 0,
                        dirty: false,
                    },
                    Page {
                        values: AccountedBox::new([0; PAGE_VALUES], &context),
                        base: None,
                        len: 0,
                        dirty: false,
                    },
                ],
                recent: 0,
                count,
                control,
                account,
            };
            let mut program = AccountedBox::new(Program::sort(count), &context);
            let mut reply = None;
            loop {
                control.check().map_err(StorageError::from)?;
                account.work(1);
                match program.next(reply.take()) {
                    Operation::Step => (),
                    Operation::Less(a, b) => {
                        let left = f64::from_bits(cache.get(a)?);
                        let right = f64::from_bits(cache.get(b)?);
                        // Pinned vectorByValueHeap.Less returns true even for two NaNs.
                        reply = Some(left.is_nan() || left < right);
                    }
                    Operation::Swap(a, b) => {
                        let left = cache.get(a)?;
                        let right = cache.get(b)?;
                        cache.set(a, right)?;
                        cache.set(b, left)?;
                    }
                    Operation::Done => break,
                }
            }
            let values = [
                f64::from_bits(cache.get(plan.ordinals[0])?),
                f64::from_bits(cache.get(plan.ordinals[1])?),
            ];
            // Unused dirty pages belong to an ephemeral store; cleanup cannot replace a query error.
            Ok(plan.interpolate(values))
        })
        .map_err(|error| E::from(error.accounted(account)))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        storage::{Interrupted, ProviderResources, SelectionRequest},
        storage_observed::ObservedQuery,
    };
    use std::cell::Cell;

    #[derive(Default)]
    struct Query {
        opened: Cell<u64>,
        closed: Cell<u64>,
        reads: Cell<u64>,
        writes: Cell<u64>,
        fail: u8,
        cancel: bool,
    }
    struct Closed<'a> {
        query: &'a Query,
        request: TemporaryFloatBitsRequest<'a>,
        entered: Cell<bool>,
    }
    impl Drop for Closed<'_> {
        fn drop(&mut self) {
            if self.entered.get() {
                // The disk factory has already closed its file; outer accounting is still live.
                assert!(
                    self.request
                        .accounting
                        .snapshot()
                        .temporary_storage
                        .extent
                        .current
                        >= self.request.extent_bytes().unwrap()
                );
                self.query.closed.set(self.query.closed.get() + 1);
            }
        }
    }
    struct Store<'a, 's> {
        value: &'s mut dyn TemporaryFloatBits,
        query: &'a Query,
        account: &'a QueryAccounting,
    }
    impl Store<'_, '_> {
        fn fault(&self, write: bool, control: &Control) -> Result<(), StorageError> {
            let calls = if write {
                &self.query.writes
            } else {
                &self.query.reads
            };
            calls.set(calls.get() + 1);
            if self.query.fail == if write { 3 } else { 2 } {
                if self.query.cancel {
                    control.cancellation.cancel();
                }
                return Err(crate::storage::temporary_storage_error(
                    self.account,
                    b"injected temporary I/O failure",
                ));
            }
            Ok(())
        }
    }
    impl TemporaryFloatBits for Store<'_, '_> {
        fn resources(&self) -> ProviderResources {
            self.value.resources()
        }
        fn read_into(
            &mut self,
            offset: u64,
            values: &mut [u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            self.fault(false, control)?;
            self.value.read_into(offset, values, control)
        }
        fn write_at(
            &mut self,
            offset: u64,
            values: &[u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            self.fault(true, control)?;
            self.value.write_at(offset, values, control)
        }
    }
    impl QueryProvider for &Query {
        type Selection<'a>
            = crate::test_storage::FixtureSelection<'a>
        where
            Self: 'a;
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn select(
            &self,
            _: SelectionRequest<'_>,
            _: &Control,
        ) -> Result<Self::Selection<'_>, StorageError> {
            Err(StorageError::Invalid(
                "temporary test query has no selectors",
            ))
        }
        fn with_temporary_float_bits<R>(
            &self,
            request: TemporaryFloatBitsRequest<'_>,
            control: &Control,
            visit: impl for<'a> FnOnce(&'a mut dyn TemporaryFloatBits) -> R,
        ) -> Result<R, StorageError> {
            if self.fail == 1 {
                return Err(crate::storage::temporary_storage_error(
                    request.accounting,
                    b"injected temporary create failure",
                ));
            }
            let guard = Closed {
                query: self,
                request,
                entered: Cell::new(false),
            };
            crate::fixture_temporary_storage::with_store(request, control, |value| {
                guard.entered.set(true);
                self.opened.set(self.opened.get() + 1);
                if self.cancel && self.fail == 0 {
                    control.cancellation.cancel();
                }
                visit(&mut Store {
                    value,
                    query: self,
                    account: request.accounting,
                })
            })
        }
    }

    #[test]
    fn unavailable_backing_and_extent_overflow_do_not_materialize() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let visited = Cell::new(false);
        for count in [13, u64::MAX] {
            let result: Result<f64, StorageError> = quantile(
                &crate::test_storage::NoTemporary,
                count,
                Plan::new(0.5, count),
                &Control::default(),
                &account,
                |_| {
                    visited.set(true);
                    Ok(())
                },
            );
            if count == 13 {
                assert!(matches!(result, Err(StorageError::Provider(_))));
            } else {
                assert!(matches!(
                    result,
                    Err(StorageError::Invalid("temporary float extent overflow"))
                ));
            }
            drop(result);
        }
        assert!(!visited.get());
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn disk_sort_crosses_pages_and_releases_backing_after_success_failure_and_cancel() {
        for (fail, cancel) in [
            (0, false),
            (1, false),
            (2, false),
            (3, false),
            (0, true),
            (2, true),
            (3, true),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let query = Query {
                fail,
                cancel,
                ..Query::default()
            };
            let observed = ObservedQuery::new(&query, &account);
            let result: Result<f64, StorageError> = quantile(
                &observed,
                2051,
                Plan::new(0.5, 2051),
                &control,
                &account,
                |writer| {
                    for index in 0..2051 {
                        writer.push(((2050 - index) as f64).to_bits())?;
                    }
                    Ok(())
                },
            );
            if fail != 0 {
                let Err(StorageError::Provider(message)) = &result else {
                    panic!("expected original storage failure: {result:?}");
                };
                assert!(message.starts_with(b"injected temporary"));
            } else if cancel {
                assert_eq!(
                    result,
                    Err(StorageError::Interrupted(Interrupted::Cancelled))
                );
            } else {
                assert_eq!(*result.as_ref().unwrap(), 1025.0);
            }
            let snapshot = account.snapshot();
            assert_eq!(query.opened.get(), query.closed.get());
            assert_eq!(
                snapshot.temporary_storage.extent.peak,
                if fail == 1 { 0 } else { 2051 * 8 }
            );
            assert_eq!(snapshot.temporary_storage.stores.current, 0);
            if fail == 2 {
                assert_eq!(snapshot.temporary_storage.read_calls, 1);
                assert_eq!(snapshot.temporary_storage.read_bytes, 0);
            }
            if fail == 3 {
                assert_eq!(snapshot.temporary_storage.write_calls, 1);
                assert_eq!(snapshot.temporary_storage.written_bytes, 0);
            }
            drop(observed);
            // Failed results retain only their precisely charged error bytes.
            if let Err(StorageError::Provider(bytes)) = &result {
                account
                    .snapshot()
                    .assert_retained_error_memory(bytes.capacity() as u64);
            } else {
                account.snapshot().assert_memory_released();
            }
            drop(result);
            account.snapshot().assert_memory_released();
            assert_eq!(
                engine.snapshot().temporary_storage,
                snapshot.temporary_storage
            );
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn factory_unwind_closes_physical_backing_before_releasing_its_account() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let query = Query::default();
        let observed = ObservedQuery::new(&query, &account);
        let control = Control::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<f64, StorageError> = quantile(
                &observed,
                13,
                Plan::new(0.5, 13),
                &control,
                &account,
                |_| panic!("injected materialization panic"),
            );
        }));
        assert!(result.is_err());
        assert_eq!((query.opened.get(), query.closed.get()), (1, 1));
        drop(observed);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn dirty_evictions_and_short_tail_preserve_nan_payloads_and_signed_zero_bits() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let query = Query::default();
        let observed = ObservedQuery::new(&query, &account);
        let control = Control::default();
        let context = AllocationContext::new(&account, MemoryKind::Workspace);
        let original = |index: u64| {
            [
                0x8000_0000_0000_0000,
                0,
                0x7ff0_0000_0000_0044,
                0xfff8_0000_0000_0045,
            ][index as usize % 4]
        };
        observed
            .with_temporary_float_bits(
                TemporaryFloatBitsRequest {
                    value_count: 3069,
                    accounting: &account,
                },
                &control,
                |store| {
                    let mut writer = Writer {
                        store,
                        buffer: AccountedBox::new([0; PAGE_VALUES], &context),
                        count: 3069,
                        written: 0,
                        len: 0,
                        control: &control,
                        account: &account,
                    };
                    for index in 0..3069 {
                        writer.push(original(index)).unwrap();
                    }
                    writer.flush().unwrap();
                    let Writer { store, buffer, .. } = writer;
                    let page_bytes =
                        account.snapshot().memory[MemoryKind::Workspace as usize].current;
                    let mut cache = Cache {
                        store,
                        pages: [
                            Page {
                                values: buffer,
                                base: None,
                                len: 0,
                                dirty: false,
                            },
                            Page {
                                values: AccountedBox::new([0; PAGE_VALUES], &context),
                                base: None,
                                len: 0,
                                dirty: false,
                            },
                        ],
                        recent: 0,
                        count: 3069,
                        control: &control,
                        account: &account,
                    };
                    let replacements = [
                        (1023, 0xfff0_0000_0000_0077),
                        (1024, 0x8000_0000_0000_0000),
                        (2048, 0x7ff8_0000_0000_0066),
                        (3068, 0),
                    ];
                    for (index, bits) in replacements {
                        cache.set(index, bits).unwrap();
                    }
                    for index in [0, 1023, 1024, 1025, 2047, 2048, 3068, 1024, 1023] {
                        let expected = replacements
                            .iter()
                            .find(|(at, _)| *at == index)
                            .map_or_else(|| original(index), |(_, bits)| *bits);
                        assert_eq!(cache.get(index).unwrap(), expected);
                    }
                    assert_eq!(
                        account.snapshot().memory[MemoryKind::Workspace as usize].peak,
                        2 * page_bytes
                    );
                },
            )
            .unwrap();
        assert_eq!((query.opened.get(), query.closed.get()), (1, 1));
        assert_eq!(account.snapshot().temporary_storage.extent.peak, 3069 * 8);
        drop(observed);
        drop(context);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn nested_temporary_sorts_conserve_extent_and_traffic_until_each_physical_close() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let query = Query::default();
        let observed = ObservedQuery::new(&query, &account);
        let control = Control::default();
        let result: Result<f64, StorageError> = quantile(
            &observed,
            2051,
            Plan::new(0.5, 2051),
            &control,
            &account,
            |writer| {
                let inner: Result<f64, StorageError> = quantile(
                    &observed,
                    2097,
                    Plan::new(0.5, 2097),
                    &control,
                    &account,
                    |inner| {
                        assert_eq!(
                            account.snapshot().temporary_storage.extent.current,
                            (2051 + 2097) * 8
                        );
                        assert_eq!(account.snapshot().temporary_storage.stores.current, 2);
                        for index in 0..2097 {
                            inner.push(((2096 - index) as f64).to_bits())?;
                        }
                        Ok(())
                    },
                );
                assert_eq!(inner?, 1048.0);
                assert_eq!((query.opened.get(), query.closed.get()), (2, 1));
                assert_eq!(
                    account.snapshot().temporary_storage.extent.current,
                    2051 * 8
                );
                for index in 0..2051 {
                    writer.push(((2050 - index) as f64).to_bits())?;
                }
                Ok(())
            },
        );
        assert_eq!(result.unwrap(), 1025.0);
        let observation = account.snapshot().temporary_storage;
        assert_eq!(
            (
                observation.extent.peak,
                observation.extent.acquired,
                observation.extent.released
            ),
            ((2051 + 2097) * 8, (2051 + 2097) * 8, (2051 + 2097) * 8)
        );
        assert_eq!(
            (
                observation.stores.peak,
                observation.stores.acquired,
                observation.stores.released
            ),
            (2, 2, 2)
        );
        assert!(observation.read_calls > 0 && observation.written_bytes >= (2051 + 2097) * 8);
        assert_eq!(engine.snapshot().temporary_storage, observation);
        assert_eq!((query.opened.get(), query.closed.get()), (2, 2));
        drop(observed);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn concurrent_temporary_sorts_measure_actual_overlap_and_distinct_queries() {
        let engine = EngineAccounting::default();
        let entered = std::sync::Barrier::new(3);
        let proceed = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            let handles: Vec<_> = [2051, 2097]
                .into_iter()
                .map(|count| {
                    let engine = &engine;
                    let entered = &entered;
                    let proceed = &proceed;
                    scope.spawn(move || {
                        let account = engine.query();
                        let query = Query::default();
                        let observed = ObservedQuery::new(&query, &account);
                        let control = Control::default();
                        let result: Result<f64, StorageError> = quantile(
                            &observed,
                            count,
                            Plan::new(0.5, count),
                            &control,
                            &account,
                            |writer| {
                                entered.wait();
                                proceed.wait();
                                for index in 0..count {
                                    writer.push(((count - index - 1) as f64).to_bits())?;
                                }
                                Ok(())
                            },
                        );
                        assert_eq!(result.unwrap(), (count / 2) as f64);
                        let snapshot = account.snapshot().temporary_storage;
                        assert_eq!(snapshot.extent.peak, count * 8);
                        assert_eq!(snapshot.stores.peak, 1);
                        assert_eq!((query.opened.get(), query.closed.get()), (1, 1));
                        drop(observed);
                        account.snapshot().assert_memory_released();
                        snapshot
                    })
                })
                .collect();
            entered.wait();
            let snapshot = engine.snapshot().temporary_storage;
            assert_eq!(snapshot.extent.current, (2051 + 2097) * 8);
            assert_eq!(snapshot.stores.current, 2);
            proceed.wait();
            let a = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>();
            let combined = engine.snapshot().temporary_storage;
            assert_eq!(combined.extent.peak, (2051 + 2097) * 8);
            assert_eq!((combined.extent.current, combined.stores.current), (0, 0));
            assert_eq!(
                combined.read_calls,
                a.iter().map(|s| s.read_calls).sum::<u64>()
            );
            assert_eq!(
                combined.write_calls,
                a.iter().map(|s| s.write_calls).sum::<u64>()
            );
            assert_eq!(
                combined.read_bytes,
                a.iter().map(|s| s.read_bytes).sum::<u64>()
            );
            assert_eq!(
                combined.written_bytes,
                a.iter().map(|s| s.written_bytes).sum::<u64>()
            );
        });
        engine.snapshot().assert_memory_released();
    }
}
