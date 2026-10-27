// SPDX-License-Identifier: GPL-3.0-or-later
//! Completed dry-reader positions; one fixed provider-backed record per existing row.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    derived_matrix::{CHECKPOINT_WORDS, Checkpoint, Window},
    frontend::ast::NodeId,
    query_scopes::Grid,
    statistics::OwnedSamples,
    storage::{
        Control, Interrupted, OwnedTemporary, QueryProvider, StorageError, TemporaryFloatBits,
        TemporaryFloatBitsRequest,
    },
    vector_matching::ReplayKey,
};
use std::cell::{Cell, RefCell};

#[cfg(test)]
thread_local! {
    static OBSERVATION: Cell<Option<(bool, usize, usize, u64)>> = const { Cell::new(None) };
}
#[cfg(test)]
pub(crate) fn observe<R>(disabled: bool, run: impl FnOnce() -> R) -> (R, usize, usize) {
    let (result, loads, saves, _) = observe_with_reads(disabled, run);
    (result, loads, saves)
}
#[cfg(test)]
pub(crate) fn observe_with_reads<R>(
    disabled: bool,
    run: impl FnOnce() -> R,
) -> (R, usize, usize, u64) {
    struct Restore(Option<(bool, usize, usize, u64)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OBSERVATION.set(self.0);
        }
    }
    let _restore = Restore(OBSERVATION.replace(Some((disabled, 0, 0, 0))));
    let result = run();
    let (_, loads, saves, bytes) = OBSERVATION.get().unwrap();
    (result, loads, saves, bytes)
}
#[cfg(test)]
fn note(save: bool) {
    if let Some((disabled, loads, saves, bytes)) = OBSERVATION.get() {
        OBSERVATION.set(Some((
            disabled,
            loads + usize::from(!save),
            saves + usize::from(save),
            bytes,
        )));
    }
}

const FINAL: u64 = 1 << 63;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scope {
    key: ReplayKey,
    grid: Grid,
    window: Window,
    rows: usize,
}
impl Scope {
    pub(crate) fn new(key: ReplayKey, grid: Grid, mut window: Window, rows: usize) -> Self {
        window.end_ms = window.first_end_ms;
        Self {
            key: key.at(0, 0),
            grid,
            window,
            rows,
        }
    }
}

struct Store<'q> {
    // Close provider backing while its sample/accounting owners remain live.
    backing: OwnedTemporary<'q>,
    scope: Scope,
    buffer: Checkpoint,
    samples: OwnedSamples,
}
impl<'q> Store<'q> {
    fn create<Q: QueryProvider>(
        query: &'q Q,
        scope: Scope,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<Self>, StorageError> {
        #[cfg(test)]
        if OBSERVATION.get().is_some_and(|value| value.0) {
            return Ok(None);
        }
        let words = u64::try_from(scope.rows)
            .ok()
            .and_then(|rows| rows.checked_mul(CHECKPOINT_WORDS as u64))
            .ok_or(StorageError::Invalid("derived progress extent overflow"))?;
        let request = TemporaryFloatBitsRequest {
            value_count: words,
            accounting: account,
        };
        request.extent_bytes()?;
        control.check()?;
        account.provider_call();
        let Some(backing) = query.open_temporary_float_bits(request, control)? else {
            return Ok(None);
        };
        let mut store = Self {
            backing,
            scope,
            buffer: Checkpoint([0; CHECKPOINT_WORDS]),
            samples: OwnedSamples::new(account, 0),
        };
        let mut offset = 0;
        while offset < words {
            let length = (words - offset).min(1024) as usize;
            crate::temporary_float_bits::write(
                &mut store.backing,
                offset,
                &[0; 1024][..length],
                control,
                account,
            )?;
            account.work(length as u64);
            offset += length as u64;
        }
        Ok(Some(store))
    }

    fn offset(&self, row: usize) -> Result<u64, StorageError> {
        if row >= self.scope.rows {
            return Err(StorageError::Invalid("derived progress row out of bounds"));
        }
        Ok(row as u64 * CHECKPOINT_WORDS as u64)
    }

    fn load(
        &mut self,
        row: usize,
        final_phase: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<Checkpoint>, StorageError> {
        control.check()?;
        let offset = self.offset(row)?;
        self.samples.resize(0);
        account.provider_call();
        account.temporary_transfer(false, None);
        self.backing
            .read_into(offset, &mut self.buffer.0, control)
            .map_err(|error| error.accounted(account))?;
        account.temporary_transfer(false, Some(CHECKPOINT_WORDS as u64 * 8));
        #[cfg(test)]
        if let Some((disabled, loads, saves, bytes)) = OBSERVATION.get() {
            OBSERVATION.set(Some((
                disabled,
                loads,
                saves,
                bytes + CHECKPOINT_WORDS as u64 * 8,
            )));
        }
        account.work(CHECKPOINT_WORDS as u64);
        let current = (self.buffer.0[0] & FINAL != 0) == final_phase;
        self.samples.resize(if current {
            self.buffer.sample_units()
        } else {
            0
        });
        control.check()?;
        if !current || self.buffer.0[0] & 1 == 0 {
            return Ok(None);
        }
        let mut checkpoint = self.buffer;
        checkpoint.0[0] &= !FINAL;
        #[cfg(test)]
        note(false);
        Ok(Some(checkpoint))
    }

    fn save(
        &mut self,
        row: usize,
        mut checkpoint: Checkpoint,
        final_phase: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        control.check()?;
        let offset = self.offset(row)?;
        if final_phase {
            checkpoint.0[0] |= FINAL;
        }
        self.buffer = checkpoint;
        self.samples.resize(checkpoint.sample_units());
        account.work(CHECKPOINT_WORDS as u64);
        crate::temporary_float_bits::write(
            &mut self.backing,
            offset,
            &self.buffer.0,
            control,
            account,
        )?;
        #[cfg(test)]
        note(true);
        Ok(())
    }
}

pub(crate) struct Registry<'q> {
    entries: RefCell<AccountedVec<(NodeId, Option<Store<'q>>)>>,
    final_phase: Cell<bool>,
}
impl<'q> Registry<'q> {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            entries: RefCell::new(AccountedVec::new(account, MemoryKind::Workspace)),
            final_phase: Cell::new(false),
        }
    }

    fn take(
        &self,
        node: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<(usize, Option<Store<'q>>)>, Interrupted> {
        let mut entries = self.entries.borrow_mut();
        for (position, (id, store)) in entries.iter_mut().enumerate() {
            control.check()?;
            account.work(1);
            if *id == node {
                return Ok(Some((position, store.take())));
            }
        }
        control.check()?;
        Ok(None)
    }

    pub(crate) fn load<Q: QueryProvider>(
        &self,
        query: &'q Q,
        node: NodeId,
        scope: Scope,
        row: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<Checkpoint>, StorageError> {
        let (position, mut store) = match self.take(node, control, account)? {
            Some(entry) => entry,
            None => {
                let store = Store::create(query, scope, control, account)?;
                let mut entries = self.entries.borrow_mut();
                let position = entries.len();
                // An absent capability is remembered without retrying it for each row.
                entries.push((node, None));
                (position, store)
            }
        };
        let result = match store.as_mut().filter(|store| store.scope == scope) {
            Some(store) => store.load(row, self.final_phase.get(), control, account),
            None => Ok(None),
        };
        self.entries.borrow_mut()[position].1 = store;
        result
    }

    pub(crate) fn save(
        &self,
        node: NodeId,
        scope: Scope,
        row: usize,
        checkpoint: Checkpoint,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        let Some((position, mut store)) = self.take(node, control, account)? else {
            return Ok(());
        };
        let result = match store.as_mut().filter(|store| store.scope == scope) {
            Some(store) => store.save(row, checkpoint, self.final_phase.get(), control, account),
            None => Ok(()),
        };
        self.entries.borrow_mut()[position].1 = store;
        result
    }

    pub(crate) fn start_final(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        self.final_phase.set(true);
        for (_, store) in self.entries.borrow_mut().iter_mut() {
            control.check()?;
            account.work(1);
            if let Some(store) = store {
                store.buffer.0.fill(0);
                store.samples.resize(0);
            }
        }
        control.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        evaluator::EvaluationOptions,
        storage::{ProviderResources, SelectionRequest, TimeBounds},
        storage_observed::ObservedQuery,
    };

    #[derive(Default)]
    struct Provider {
        unavailable: bool,
        failure: Cell<u8>,
        opened: Cell<usize>,
        closed: Cell<usize>,
    }
    struct Backing<'a> {
        words: AccountedVec<u64>,
        initialized: usize,
        provider: &'a Provider,
    }
    impl Drop for Backing<'_> {
        fn drop(&mut self) {
            self.provider.closed.set(self.provider.closed.get() + 1);
        }
    }
    impl TemporaryFloatBits for Backing<'_> {
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn read_into(
            &mut self,
            offset: u64,
            values: &mut [u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            control.check()?;
            if self.provider.failure.get() == 2 {
                return Err(StorageError::Invalid("progress read"));
            }
            let start = offset as usize;
            assert!(start + values.len() <= self.initialized);
            values.copy_from_slice(&self.words[start..start + values.len()]);
            if self.provider.failure.get() == 4 {
                control.cancellation.cancel();
            }
            Ok(())
        }
        fn write_at(
            &mut self,
            offset: u64,
            values: &[u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            control.check()?;
            if self.provider.failure.get() == 1 {
                return Err(StorageError::Invalid("progress write"));
            }
            let start = offset as usize;
            assert!(start <= self.initialized);
            self.words[start..start + values.len()].copy_from_slice(values);
            self.initialized = self.initialized.max(start + values.len());
            if self.provider.failure.get() == 3 {
                control.cancellation.cancel();
            }
            Ok(())
        }
    }
    impl QueryProvider for &Provider {
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
            unreachable!()
        }
        fn open_temporary_float_bits(
            &self,
            request: TemporaryFloatBitsRequest<'_>,
            _: &Control,
        ) -> Result<Option<OwnedTemporary<'_>>, StorageError> {
            self.opened.set(self.opened.get() + 1);
            if self.unavailable {
                return Ok(None);
            }
            let mut words = AccountedVec::new(request.accounting, MemoryKind::ProviderHeld);
            words.resize(request.value_count as usize, u64::MAX);
            Ok(Some(OwnedTemporary::new(
                Backing {
                    words,
                    initialized: 0,
                    provider: self,
                },
                request.accounting,
            )))
        }
    }
    fn scope() -> Scope {
        let grid = Grid {
            start_ms: -10,
            end_ms: 100,
            interval_ms: 1,
        };
        let options = EvaluationOptions {
            time_ms: 4,
            start_ms: -10,
            end_ms: 100,
            step_ms: 1,
            lookback_ms: 10,
            start_timestamps: false,
            delayed_name_removal: false,
        };
        Scope::new(
            ReplayKey::new(
                options,
                0,
                10_000_000,
                TimeBounds {
                    start_ms: -20,
                    end_ms: 100,
                },
            ),
            grid,
            Window {
                first_end_ms: 4,
                end_ms: 5,
                range_ms: 4,
                step_ms: 1,
            },
            100,
        )
    }
    fn checkpoint() -> Checkpoint {
        let mut words = [0; CHECKPOINT_WORDS];
        words[0] = 7;
        words[1] = 4;
        words[16] = (-0.0_f64).to_bits();
        Checkpoint(words)
    }
    #[test]
    fn progress_scope_generation_dirty_backing_and_optional_capability() {
        for unavailable in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let provider = Provider {
                unavailable,
                ..Provider::default()
            };
            let query = ObservedQuery::new(&provider, &account);
            let registry = Registry::new(&account);
            let control = Control::default();
            let scope = scope();
            let node = NodeId(0);
            for row in [0, 60, 99] {
                assert_eq!(
                    registry
                        .load(&query, node, scope, row, &control, &account)
                        .unwrap(),
                    None
                );
                registry
                    .save(node, scope, row, checkpoint(), &control, &account)
                    .unwrap();
                assert_eq!(
                    registry
                        .load(&query, node, scope, row, &control, &account)
                        .unwrap(),
                    (!unavailable).then_some(checkpoint())
                );
            }
            assert_eq!(provider.opened.get(), 1);
            if !unavailable {
                assert!(
                    registry
                        .load(&query, node, scope, 100, &control, &account)
                        .is_err()
                );
                let mut mismatch = scope;
                mismatch.window.range_ms += 1;
                assert_eq!(
                    registry
                        .load(&query, node, mismatch, 0, &control, &account)
                        .unwrap(),
                    None
                );
                registry.start_final(&control, &account).unwrap();
                assert_eq!(
                    registry
                        .load(&query, node, scope, 0, &control, &account)
                        .unwrap(),
                    None
                );
                registry
                    .save(node, scope, 0, checkpoint(), &control, &account)
                    .unwrap();
                assert_eq!(
                    registry
                        .load(&query, node, scope, 0, &control, &account)
                        .unwrap(),
                    Some(checkpoint())
                );
                assert_eq!(
                    registry
                        .load(&query, node, scope, 99, &control, &account)
                        .unwrap(),
                    None
                );
            }
            drop(registry);
            drop(query);
            assert_eq!(provider.closed.get(), usize::from(!unavailable));
            account.snapshot().assert_memory_released();
            assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
        }
    }
    #[test]
    fn progress_callback_failures_and_cancellation_release_backing() {
        for initialization in [false, true] {
            for failure in 1..=4 {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let provider = Provider::default();
                let query = ObservedQuery::new(&provider, &account);
                let registry = Registry::new(&account);
                let control = Control::default();
                if !initialization {
                    registry
                        .load(&query, NodeId(0), scope(), 0, &control, &account)
                        .unwrap();
                }
                provider.failure.set(failure);
                let result = if initialization || failure == 2 || failure == 4 {
                    registry
                        .load(&query, NodeId(0), scope(), 0, &control, &account)
                        .map(|_| ())
                } else {
                    registry.save(NodeId(0), scope(), 0, checkpoint(), &control, &account)
                };
                assert!(
                    result.is_err(),
                    "initialization={initialization}, failure={failure}"
                );
                drop(registry);
                drop(query);
                assert_eq!(provider.closed.get(), 1);
                account.snapshot().assert_memory_released();
                assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
            }
        }
    }
}
