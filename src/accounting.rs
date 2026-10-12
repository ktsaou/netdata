// SPDX-License-Identifier: GPL-3.0-or-later
//! Observations remain active without admitting, throttling or limiting queries.
use crate::shared::Strong;
use crate::statistics::{PHASES, Phase, Statistics};
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
    iter::FusedIterator,
    ops::{Deref, DerefMut, Range, RangeBounds},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum MemoryKind {
    Compiled,
    Workspace,
    Result,
    ProviderHeld,
    BorrowedView,
    RegexEstimate,
    SharedCacheReference,
    /// Engine/query ledgers and explicitly engine-owned cancellation allocations.
    Accounting,
}
pub const MEMORY_KINDS: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoryObservation {
    pub current: u64,
    pub peak: u64,
    pub acquired: u64,
    pub released: u64,
    pub overflowed: bool,
}

fn add(value: &mut u64, amount: u64, overflowed: &mut bool) {
    match value.checked_add(amount) {
        Some(next) => *value = next,
        None => {
            *value = u64::MAX;
            *overflowed = true;
        }
    }
}
impl MemoryObservation {
    fn acquire(&mut self, bytes: u64) {
        add(&mut self.current, bytes, &mut self.overflowed);
        add(&mut self.acquired, bytes, &mut self.overflowed);
        self.peak = self.peak.max(self.current);
    }
    fn release(&mut self, bytes: u64) {
        self.current = self.current.saturating_sub(bytes);
        add(&mut self.released, bytes, &mut self.overflowed);
    }
}

/// Counts scoped aliases, independently of physical allocation bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReferenceObservation {
    pub current: u64,
    pub peak: u64,
    pub acquired: u64,
    pub released: u64,
    pub overflowed: bool,
}
impl ReferenceObservation {
    fn acquire(&mut self) {
        add(&mut self.current, 1, &mut self.overflowed);
        add(&mut self.acquired, 1, &mut self.overflowed);
        self.peak = self.peak.max(self.current);
    }
    fn release(&mut self) {
        self.current = self.current.saturating_sub(1);
        add(&mut self.released, 1, &mut self.overflowed);
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CancellationReferences {
    /// Engine-held guard aliases of tokens created by any engine.
    pub engine_owned: ReferenceObservation,
    /// Engine-held guard aliases of caller/default tokens.
    pub external: ReferenceObservation,
}

/// Logical provider backing is separate from RAM and raw source pages.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TemporaryStorageObservations {
    pub extent: MemoryObservation,
    pub stores: ReferenceObservation,
    pub read_calls: u64,
    pub write_calls: u64,
    pub read_bytes: u64,
    pub written_bytes: u64,
    pub counter_overflowed: bool,
}
impl TemporaryStorageObservations {
    fn transfer(&mut self, write: bool, bytes: Option<u64>) {
        let calls = if write {
            &mut self.write_calls
        } else {
            &mut self.read_calls
        };
        if let Some(bytes) = bytes {
            let counter = if write {
                &mut self.written_bytes
            } else {
                &mut self.read_bytes
            };
            add(counter, bytes, &mut self.counter_overflowed);
        } else {
            add(calls, 1, &mut self.counter_overflowed);
        }
    }
}
impl CancellationReferences {
    fn entry(&mut self, engine_owned: bool) -> &mut ReferenceObservation {
        if engine_owned {
            &mut self.engine_owned
        } else {
            &mut self.external
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct QueryObservations {
    pub statistics: Statistics,
    pub memory: [MemoryObservation; MEMORY_KINDS],
    pub cancellation_references: CancellationReferences,
    pub temporary_storage: TemporaryStorageObservations,
    pub provider_calls: u64,
    pub pages: u64,
    /// Samples received in accepted pages, including a page discarded on cancellation.
    pub samples_fetched: u64,
    pub work: u64,
    pub output_samples: u64,
    pub elapsed: Duration,
    pub counter_overflowed: bool,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EngineObservations {
    pub memory: [MemoryObservation; MEMORY_KINDS],
    pub cancellation_references: CancellationReferences,
    pub temporary_storage: TemporaryStorageObservations,
    pub active_executions: u64,
    pub peak_executions: u64,
    pub completed_executions: u64,
    pub elapsed: Duration,
    pub counter_overflowed: bool,
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
#[derive(Clone, Debug)]
pub struct EngineAccounting(Strong<Mutex<EngineObservations>>);
impl Default for EngineAccounting {
    fn default() -> Self {
        let mut observations = EngineObservations::default();
        // The terminal root cannot retain a lease to itself.
        observations.memory[MemoryKind::Accounting as usize].acquire(Self::allocation_bytes());
        Self(Strong::new_in(Mutex::new(observations), None))
    }
}
impl EngineAccounting {
    pub(crate) fn allocation_bytes() -> u64 {
        Strong::<Mutex<EngineObservations>>::allocation_bytes()
    }
    pub fn snapshot(&self) -> EngineObservations {
        lock(&self.0).clone()
    }
    pub fn query(&self) -> QueryAccounting {
        let mut observations = QueryObservations::default();
        observations.memory[MemoryKind::Accounting as usize]
            .acquire(QueryAccounting::allocation_bytes());
        QueryAccounting(Strong::new_for_engine(
            QueryState {
                engine: self.clone(),
                observations: Mutex::new(observations),
                current_samples: AtomicU64::new(0),
                timers: Mutex::new([TimerState::default(); PHASES]),
            },
            self,
        ))
    }
    pub(crate) fn hold_accounting(&self, bytes: u64) -> EngineLease {
        let mut lease = EngineLease {
            engine: self.clone(),
            bytes: 0,
        };
        lease.resize(bytes);
        lease
    }
}
#[derive(Debug)]
struct QueryState {
    engine: EngineAccounting,
    observations: Mutex<QueryObservations>,
    current_samples: AtomicU64,
    timers: Mutex<[TimerState; PHASES]>,
}
#[derive(Clone, Copy, Debug, Default)]
struct TimerState {
    start: Option<Instant>,
    active: u64,
}
#[derive(Clone, Debug)]
pub struct QueryAccounting(Strong<QueryState>);
impl QueryAccounting {
    pub(crate) fn allocation_bytes() -> u64 {
        Strong::<QueryState>::allocation_bytes()
    }
    pub fn snapshot(&self) -> QueryObservations {
        lock(&self.0.observations).clone()
    }
    pub(crate) fn phase(&self, phase: Phase) -> PhaseObservation {
        let mut timers = lock(&self.0.timers);
        let timer = &mut timers[phase as usize];
        if timer.active == 0 {
            timer.start = Some(Instant::now());
        }
        timer.active += 1;
        PhaseObservation {
            query: self.clone(),
            phase,
        }
    }
    pub(crate) fn samples_read(&self, units: u64) {
        self.count(|value| &mut value.statistics.samples.samples_read, units);
    }
    pub(crate) fn queryable_samples(&self, units: u64) {
        lock(&self.0.observations)
            .statistics
            .samples
            .total_queryable_samples = units;
    }
    pub(crate) fn statistics_overflow(&self) {
        lock(&self.0.observations).counter_overflowed = true;
    }
    pub(crate) fn resize_samples(&self, previous: u64, next: u64) {
        if previous == next {
            return;
        }
        let mut observed = lock(&self.0.observations);
        let mut current = self.0.current_samples.load(Ordering::Relaxed);
        if next > previous {
            add(
                &mut current,
                next - previous,
                &mut observed.counter_overflowed,
            );
            observed.statistics.samples.peak_samples =
                observed.statistics.samples.peak_samples.max(current);
        } else {
            current = current.saturating_sub(previous - next);
        }
        self.0.current_samples.store(current, Ordering::Relaxed);
    }
    #[cfg(test)]
    pub(crate) fn current_samples(&self) -> u64 {
        self.0.current_samples.load(Ordering::Relaxed)
    }
    pub fn hold(&self, kind: MemoryKind, bytes: u64) -> MemoryLease {
        let mut lease = MemoryLease {
            query: self.clone(),
            kind,
            bytes: 0,
        };
        lease.resize(bytes);
        lease
    }
    pub fn execution(&self) -> ExecutionObservation {
        let mut engine = lock(&self.0.engine.0);
        engine.active_executions += 1;
        engine.peak_executions = engine.peak_executions.max(engine.active_executions);
        ExecutionObservation {
            query: self.clone(),
            start: Instant::now(),
        }
    }
    pub(crate) fn compilation(&self) -> CompilationObservation {
        CompilationObservation {
            query: self.clone(),
            start: Instant::now(),
        }
    }
    pub fn provider_call(&self) {
        self.count(|s| &mut s.provider_calls, 1);
    }
    pub fn page(&self, samples: usize) {
        self.count(|s| &mut s.pages, 1);
        self.count(|s| &mut s.samples_fetched, samples as u64);
    }
    #[cfg(test)]
    pub(crate) fn test_identity(&self) -> usize {
        (&*self.0 as *const QueryState) as usize
    }
    pub fn work(&self, amount: u64) {
        self.count(|s| &mut s.work, amount);
        #[cfg(test)]
        crate::test_storage::work_observed(self, amount);
    }
    pub fn output(&self, amount: u64) {
        self.count(|s| &mut s.output_samples, amount);
    }
    fn count(&self, field: impl FnOnce(&mut QueryObservations) -> &mut u64, amount: u64) {
        let mut observations = lock(&self.0.observations);
        let mut overflowed = observations.counter_overflowed;
        add(field(&mut observations), amount, &mut overflowed);
        observations.counter_overflowed = overflowed;
    }
    pub(crate) fn hold_cancellation_reference(&self, engine_owned: bool) -> ReferenceLease {
        let lease = ReferenceLease {
            query: self.clone(),
            engine_owned,
        };
        let mut engine = lock(&self.0.engine.0);
        let mut query = lock(&self.0.observations);
        engine.cancellation_references.entry(engine_owned).acquire();
        query.cancellation_references.entry(engine_owned).acquire();
        lease
    }
    pub(crate) fn hold_temporary_storage(&self, bytes: u64) -> TemporaryStorageLease {
        let mut engine = lock(&self.0.engine.0);
        let mut query = lock(&self.0.observations);
        for observation in [&mut engine.temporary_storage, &mut query.temporary_storage] {
            observation.extent.acquire(bytes);
            observation.stores.acquire();
        }
        TemporaryStorageLease {
            query: self.clone(),
            bytes,
        }
    }
    pub(crate) fn temporary_transfer(&self, write: bool, bytes: Option<u64>) {
        let mut engine = lock(&self.0.engine.0);
        let mut query = lock(&self.0.observations);
        engine.temporary_storage.transfer(write, bytes);
        query.temporary_storage.transfer(write, bytes);
    }
}

#[derive(Debug)]
pub(crate) struct TemporaryStorageLease {
    query: QueryAccounting,
    bytes: u64,
}
impl Drop for TemporaryStorageLease {
    fn drop(&mut self) {
        let mut engine = lock(&self.query.0.engine.0);
        let mut query = lock(&self.query.0.observations);
        for observation in [&mut engine.temporary_storage, &mut query.temporary_storage] {
            observation.extent.release(self.bytes);
            observation.stores.release();
        }
    }
}

/// A child charge retains only its ancestor, never the allocation being charged.
#[derive(Debug)]
pub(crate) struct EngineLease {
    engine: EngineAccounting,
    bytes: u64,
}
impl EngineLease {
    fn resize(&mut self, bytes: u64) {
        if bytes == self.bytes {
            return;
        }
        let mut engine = lock(&self.engine.0);
        let observation = &mut engine.memory[MemoryKind::Accounting as usize];
        if bytes >= self.bytes {
            observation.acquire(bytes - self.bytes);
        } else {
            observation.release(self.bytes - bytes);
        }
        self.bytes = bytes;
    }
}
impl Drop for EngineLease {
    fn drop(&mut self) {
        self.resize(0);
    }
}
#[derive(Debug)]
pub(crate) enum SharedCharge {
    Query(MemoryLease),
    Engine(EngineLease),
}
impl SharedCharge {
    pub(crate) fn bytes(&self) -> u64 {
        match self {
            Self::Query(lease) => lease.bytes(),
            Self::Engine(lease) => lease.bytes,
        }
    }
    pub(crate) fn owns(&self, context: &AllocationContext) -> bool {
        matches!(self, Self::Query(lease) if context.owns(lease))
    }
    pub(crate) fn engine_owned(&self) -> bool {
        matches!(self, Self::Engine(_))
    }
}
impl From<MemoryLease> for SharedCharge {
    fn from(value: MemoryLease) -> Self {
        Self::Query(value)
    }
}

#[derive(Debug)]
pub(crate) struct ReferenceLease {
    query: QueryAccounting,
    engine_owned: bool,
}
impl Drop for ReferenceLease {
    fn drop(&mut self) {
        let mut engine = lock(&self.query.0.engine.0);
        let mut query = lock(&self.query.0.observations);
        engine
            .cancellation_references
            .entry(self.engine_owned)
            .release();
        query
            .cancellation_references
            .entry(self.engine_owned)
            .release();
    }
}
#[derive(Debug)]
pub struct MemoryLease {
    query: QueryAccounting,
    kind: MemoryKind,
    bytes: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct AllocationContext {
    query: QueryAccounting,
    kind: MemoryKind,
}
impl AllocationContext {
    pub(crate) fn accounting(&self) -> &QueryAccounting {
        &self.query
    }
    pub(crate) fn new(query: &QueryAccounting, kind: MemoryKind) -> Self {
        Self {
            query: query.clone(),
            kind,
        }
    }
    pub(crate) fn hold(&self, bytes: u64) -> MemoryLease {
        self.query.hold(self.kind, bytes)
    }
    pub(crate) fn owns(&self, lease: &MemoryLease) -> bool {
        self.kind == lease.kind && self.query.0.same_allocation(&lease.query.0)
    }
    pub(crate) fn with_kind(&self, kind: MemoryKind) -> Self {
        Self::new(&self.query, kind)
    }
}
impl MemoryLease {
    pub(crate) fn reconcile<const N: usize>(mut targets: [(&mut Self, u64); N]) {
        if N == 0 {
            return;
        }
        let owner = targets[0].0.query.clone();
        assert!(
            targets
                .iter()
                .all(|(lease, _)| owner.0.same_allocation(&lease.query.0)),
            "snapshot owners must share a query"
        );
        let mut engine = lock(&owner.0.engine.0);
        let mut query = lock(&owner.0.observations);
        // Ownership transfer is one observation, not overlapping allocations.
        for (lease, bytes) in &mut targets {
            if *bytes < lease.bytes {
                let released = lease.bytes - *bytes;
                engine.memory[lease.kind as usize].release(released);
                query.memory[lease.kind as usize].release(released);
            }
        }
        for (lease, bytes) in targets {
            if bytes >= lease.bytes {
                let acquired = bytes - lease.bytes;
                engine.memory[lease.kind as usize].acquire(acquired);
                query.memory[lease.kind as usize].acquire(acquired);
            }
            lease.bytes = bytes;
        }
    }
    pub fn resize(&mut self, bytes: u64) {
        if bytes == self.bytes {
            return;
        }
        let mut engine = lock(&self.query.0.engine.0);
        let mut query = lock(&self.query.0.observations);
        let index = self.kind as usize;
        if bytes >= self.bytes {
            engine.memory[index].acquire(bytes - self.bytes);
            query.memory[index].acquire(bytes - self.bytes);
        } else {
            engine.memory[index].release(self.bytes - bytes);
            query.memory[index].release(self.bytes - bytes);
        }
        self.bytes = bytes;
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}
impl Drop for MemoryLease {
    fn drop(&mut self) {
        self.resize(0);
    }
}

#[derive(Debug)]
pub struct ExecutionObservation {
    query: QueryAccounting,
    start: Instant,
}
pub(crate) struct PhaseObservation {
    query: QueryAccounting,
    phase: Phase,
}
impl Drop for PhaseObservation {
    fn drop(&mut self) {
        let elapsed = {
            let mut timers = lock(&self.query.0.timers);
            let timer = &mut timers[self.phase as usize];
            timer.active -= 1;
            if timer.active != 0 {
                return;
            }
            timer.start.take().expect("active phase start").elapsed()
        };
        lock(&self.query.0.observations)
            .statistics
            .timings
            .add(self.phase, elapsed);
    }
}

pub(crate) struct CompilationObservation {
    query: QueryAccounting,
    start: Instant,
}
impl Drop for CompilationObservation {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        let mut query = lock(&self.query.0.observations);
        query.elapsed = query.elapsed.saturating_add(elapsed);
    }
}

impl ExecutionObservation {
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}
impl Drop for ExecutionObservation {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        let mut engine = lock(&self.query.0.engine.0);
        let mut query = lock(&self.query.0.observations);
        engine.active_executions -= 1;
        let mut overflowed = engine.counter_overflowed;
        add(&mut engine.completed_executions, 1, &mut overflowed);
        engine.counter_overflowed = overflowed;
        engine.elapsed = engine.elapsed.saturating_add(elapsed);
        query.elapsed = query.elapsed.saturating_add(elapsed);
    }
}

#[cfg(test)]
fn assert_released_memory(
    memory: &[MemoryObservation; MEMORY_KINDS],
    references: CancellationReferences,
    accounting_bytes: u64,
    workspace_bytes: u64,
) {
    for (kind, observed) in memory.iter().enumerate() {
        let expected = if kind == MemoryKind::Accounting as usize {
            accounting_bytes
        } else if kind == MemoryKind::Workspace as usize {
            workspace_bytes
        } else {
            0
        };
        assert_eq!(observed.current, expected, "memory category {kind}");
        assert!(!observed.overflowed, "memory category {kind}");
        assert_eq!(observed.acquired - observed.released, expected);
    }
    for observed in [references.engine_owned, references.external] {
        assert_eq!(observed.current, 0);
        assert_eq!(observed.acquired, observed.released);
        assert!(!observed.overflowed);
    }
}
#[cfg(test)]
impl QueryObservations {
    pub(crate) fn assert_memory_released(&self) {
        assert_temporary_released(self.temporary_storage);
        assert_released_memory(
            &self.memory,
            self.cancellation_references,
            QueryAccounting::allocation_bytes(),
            0,
        );
    }
    pub(crate) fn assert_retained_error_memory(&self, bytes: u64) {
        assert_temporary_released(self.temporary_storage);
        assert_released_memory(
            &self.memory,
            self.cancellation_references,
            QueryAccounting::allocation_bytes(),
            bytes,
        );
    }
}
#[cfg(test)]
impl EngineObservations {
    pub(crate) fn assert_memory_released(&self) {
        self.assert_memory_released_with_queries(0);
    }
    pub(crate) fn assert_memory_released_with_queries(&self, queries: u64) {
        assert_temporary_released(self.temporary_storage);
        assert_released_memory(
            &self.memory,
            self.cancellation_references,
            EngineAccounting::allocation_bytes() + queries * QueryAccounting::allocation_bytes(),
            0,
        );
    }
}

#[cfg(test)]
fn assert_temporary_released(observation: TemporaryStorageObservations) {
    assert_eq!(observation.extent.current, 0);
    assert_eq!(observation.extent.acquired, observation.extent.released);
    assert_eq!(observation.stores.current, 0);
    assert_eq!(observation.stores.acquired, observation.stores.released);
}

#[derive(Debug)]
struct BoxAllocation<T> {
    value: T,
    _lease: MemoryLease,
}

/// The allocation token lives inside the box so enum variants retain a thin pointer.
#[derive(Debug)]
pub(crate) struct AccountedBox<T>(Box<BoxAllocation<T>>);
impl<T> AccountedBox<T> {
    pub(crate) fn new(value: T, context: &AllocationContext) -> Self {
        let lease = context.hold(Self::allocation_bytes() as u64);
        Self(Box::new(BoxAllocation {
            value,
            _lease: lease,
        }))
    }
    pub(crate) fn allocation_bytes() -> usize {
        size_of::<BoxAllocation<T>>()
    }
    pub(crate) fn into_inner(self) -> T {
        self.0.value
    }
}
impl<T> Deref for AccountedBox<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0.value
    }
}
impl<T> DerefMut for AccountedBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.value
    }
}

/// Include both allocations in the observed peak while replacing a buffer.
#[derive(Debug)]
pub struct AccountedVec<T> {
    values: Vec<T>,
    lease: Option<MemoryLease>,
}
impl<T> AccountedVec<T> {
    pub fn new(query: &QueryAccounting, kind: MemoryKind) -> Self {
        Self::new_in(Some(&AllocationContext::new(query, kind)))
    }
    pub(crate) fn new_in(context: Option<&AllocationContext>) -> Self {
        Self {
            values: Vec::new(),
            lease: context.map(|context| context.hold(0)),
        }
    }
    pub(crate) fn with_capacity_in(capacity: usize, context: Option<&AllocationContext>) -> Self {
        let values = Vec::with_capacity(capacity);
        let bytes = values
            .capacity()
            .checked_mul(size_of::<T>())
            .expect("allocation size overflow");
        Self {
            values,
            lease: context.map(|context| context.hold(bytes as u64)),
        }
    }
    /// Transfer an existing owned allocation without copying or losing its capacity charge.
    pub(crate) fn from_vec_in(values: Vec<T>, context: Option<&AllocationContext>) -> Self {
        let bytes = values
            .capacity()
            .checked_mul(size_of::<T>())
            .expect("allocation size overflow");
        Self {
            values,
            lease: context.map(|context| context.hold(bytes as u64)),
        }
    }
    pub(crate) fn context(&self) -> Option<AllocationContext> {
        self.lease
            .as_ref()
            .map(|lease| AllocationContext::new(&lease.query, lease.kind))
    }
    pub(crate) fn account_if_unobserved(&mut self, context: &AllocationContext) {
        if self.lease.is_none() {
            let bytes = self
                .values
                .capacity()
                .checked_mul(size_of::<T>())
                .expect("allocation size overflow");
            self.lease = Some(context.hold(bytes as u64));
        }
    }
    pub fn capacity(&self) -> usize {
        self.values.capacity()
    }
    pub fn reserve(&mut self, additional: usize) {
        let required = self
            .values
            .len()
            .checked_add(additional)
            .expect("allocation size overflow");
        if required <= self.values.capacity() {
            return;
        }
        let Some(previous) = &self.lease else {
            self.values.reserve(additional);
            return;
        };
        let capacity = required
            .max(self.values.capacity().saturating_mul(2))
            .max(4);
        let mut values = Vec::with_capacity(capacity);
        let bytes = values
            .capacity()
            .checked_mul(size_of::<T>())
            .expect("allocation size overflow");
        let lease = previous.query.hold(previous.kind, bytes as u64);
        values.append(&mut self.values);
        self.values = values;
        self.lease = Some(lease);
    }
    pub fn push(&mut self, value: T) {
        self.reserve(1);
        self.values.push(value);
    }
    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }
    pub fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
    }
    pub fn remove(&mut self, index: usize) -> T {
        self.values.remove(index)
    }
    pub fn append(&mut self, other: &mut Self) {
        self.reserve(other.len());
        self.values.append(&mut other.values);
    }
    pub fn split_off(&mut self, at: usize) -> Self {
        assert!(at <= self.len(), "split index out of bounds");
        let mut result = Self::with_capacity_in(self.len() - at, self.context().as_ref());
        result.values.extend(self.values.drain(at..));
        result
    }
    pub fn drain<R: RangeBounds<usize>>(&mut self, range: R) -> std::vec::Drain<'_, T> {
        self.values.drain(range)
    }
    pub fn insert(&mut self, index: usize, value: T) {
        assert!(index <= self.values.len(), "insertion index out of bounds");
        self.reserve(1);
        self.values.insert(index, value);
    }
    pub fn resize(&mut self, len: usize, value: T)
    where
        T: Clone,
    {
        self.reserve(len.saturating_sub(self.values.len()));
        self.values.resize(len, value);
    }
    pub(crate) fn from_slice_in(values: &[T], context: Option<&AllocationContext>) -> Self
    where
        T: Clone,
    {
        let mut result = Self::with_capacity_in(values.len(), context);
        result.values.extend_from_slice(values);
        result
    }
    pub(crate) fn from_iter_in<I: IntoIterator<Item = T>>(
        values: I,
        context: Option<&AllocationContext>,
    ) -> Self {
        let values = values.into_iter();
        let mut result = Self::with_capacity_in(values.size_hint().0, context);
        result.extend(values);
        result
    }
    pub(crate) fn into_unobserved(self) -> Vec<T> {
        assert!(
            self.lease.is_none(),
            "observed buffers must retain their owner"
        );
        self.values
    }
    pub(crate) fn transfer_to_dependency<R>(self, consumer: impl FnOnce(Vec<T>) -> R) -> R {
        let Self { values, lease } = self;
        // The dependency's separate estimate takes over at this ownership boundary.
        drop(lease);
        consumer(values)
    }
    pub fn extend_from_slice(&mut self, values: &[T])
    where
        T: Clone,
    {
        self.reserve(values.len());
        self.values.extend_from_slice(values);
    }
}
impl<T> Default for AccountedVec<T> {
    fn default() -> Self {
        Self::new_in(None)
    }
}
impl<T> From<Vec<T>> for AccountedVec<T> {
    fn from(values: Vec<T>) -> Self {
        Self {
            values,
            lease: None,
        }
    }
}
impl<T> FromIterator<T> for AccountedVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from(Vec::from_iter(iter))
    }
}
impl<T> Extend<T> for AccountedVec<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, values: I) {
        let values = values.into_iter();
        self.reserve(values.size_hint().0);
        for value in values {
            self.push(value);
        }
    }
}
pub struct AccountedIntoIter<T> {
    values: std::vec::IntoIter<T>,
    _lease: Option<MemoryLease>,
}
impl<T> Iterator for AccountedIntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.values.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}
impl<T> DoubleEndedIterator for AccountedIntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        self.values.next_back()
    }
}
impl<T> ExactSizeIterator for AccountedIntoIter<T> {}
impl<T> FusedIterator for AccountedIntoIter<T> {}
impl<T> IntoIterator for AccountedVec<T> {
    type Item = T;
    type IntoIter = AccountedIntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        AccountedIntoIter {
            values: self.values.into_iter(),
            _lease: self.lease,
        }
    }
}
impl<T: Copy> AccountedVec<T> {
    pub(crate) fn reserve_copy_with<E>(
        &mut self,
        additional: usize,
        mut poll: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<(), E> {
        poll(1)?;
        let required = self
            .len()
            .checked_add(additional)
            .expect("allocation size overflow");
        if required > self.capacity() {
            let capacity = required.max(self.capacity().saturating_mul(2)).max(4);
            let mut replacement = Self::with_capacity_in(capacity, self.context().as_ref());
            // Keep the source and both charges until the complete replacement is ready.
            for chunk in self.values.chunks(1024) {
                poll(chunk.len())?;
                replacement.values.extend_from_slice(chunk);
            }
            poll(1)?;
            *self = replacement;
        }
        poll(1)
    }

    pub(crate) fn push_copy_with<E>(
        &mut self,
        value: T,
        mut poll: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<(), E> {
        self.reserve_copy_with(1, &mut poll)?;
        self.values.push(value);
        poll(1)
    }

    pub(crate) fn copy_within_with<E>(
        &mut self,
        source: Range<usize>,
        destination: usize,
        mut poll: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<(), E> {
        assert!(
            source.start <= source.end && source.end <= self.len(),
            "copy range out of bounds"
        );
        let length = source.end - source.start;
        assert!(
            destination <= self.len() && length <= self.len() - destination,
            "copy target out of bounds"
        );
        poll(1)?;
        if destination <= source.start {
            let mut copied = 0;
            while copied < length {
                let count = (length - copied).min(1024);
                poll(count)?;
                self.values.copy_within(
                    source.start + copied..source.start + copied + count,
                    destination + copied,
                );
                copied += count;
            }
        } else {
            let mut remaining = length;
            while remaining > 0 {
                let count = remaining.min(1024);
                let start = remaining - count;
                poll(count)?;
                self.values.copy_within(
                    source.start + start..source.start + remaining,
                    destination + start,
                );
                remaining = start;
            }
        }
        poll(1)
    }

    pub(crate) fn insert_copy_with<E>(
        &mut self,
        index: usize,
        value: T,
        mut poll: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<(), E> {
        assert!(index <= self.len(), "insertion index out of bounds");
        self.reserve_copy_with(1, &mut poll)?;
        let length = self.len();
        self.values.push(value);
        self.copy_within_with(index..length, index + 1, &mut poll)?;
        self.values[index] = value;
        poll(1)
    }

    pub(crate) fn remove_copy_range_with<E>(
        &mut self,
        start: usize,
        length: usize,
        mut poll: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<(), E> {
        assert!(
            start <= self.len() && length <= self.len() - start,
            "removal range out of bounds"
        );
        self.copy_within_with(start + length..self.len(), start, &mut poll)?;
        self.values.truncate(self.len() - length);
        poll(1)
    }

    pub(crate) fn resize_copy_with<E>(
        &mut self,
        length: usize,
        value: T,
        mut poll: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<(), E> {
        self.reserve_copy_with(length.saturating_sub(self.len()), &mut poll)?;
        if length < self.len() {
            self.values.truncate(length);
        }
        while self.len() < length {
            let count = (length - self.len()).min(1024);
            poll(count)?;
            self.values.resize(self.len() + count, value);
        }
        poll(1)
    }
}
impl<T: Clone> Clone for AccountedVec<T> {
    fn clone(&self) -> Self {
        Self::from_slice_in(&self.values, self.context().as_ref())
    }
    fn clone_from(&mut self, source: &Self) {
        self.clear();
        self.extend_from_slice(&source.values);
    }
}
impl<'a, T> IntoIterator for &'a AccountedVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}
impl<'a, T> IntoIterator for &'a mut AccountedVec<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}
impl<T> Deref for AccountedVec<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl<T> DerefMut for AccountedVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}
impl<T> AsRef<[T]> for AccountedVec<T> {
    fn as_ref(&self) -> &[T] {
        &self.values
    }
}
impl<T> Borrow<[T]> for AccountedVec<T> {
    fn borrow(&self) -> &[T] {
        &self.values
    }
}
impl<T: Hash> Hash for AccountedVec<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.values.hash(state);
    }
}
impl<T: PartialEq, R: AsRef<[T]> + ?Sized> PartialEq<R> for AccountedVec<T> {
    fn eq(&self, other: &R) -> bool {
        self.values == other.as_ref()
    }
}
impl<T: Eq> Eq for AccountedVec<T> {}
impl std::fmt::Write for AccountedVec<u8> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.extend_from_slice(value.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn checked_growth_keeps_the_source_and_releases_interrupted_replacement() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let control = crate::storage::Control::default();
        let original: Vec<_> = (0..16_384u64).collect();
        let mut values = AccountedVec::from_slice_in(
            &original,
            Some(&AllocationContext::new(&query, MemoryKind::Workspace)),
        );
        let pointer = values.as_ptr();
        let before = query.snapshot().memory[MemoryKind::Workspace as usize];
        let result = crate::test_storage::interrupt_during(&query, &control, || {
            values.reserve_copy_with(1, |units| {
                assert!(units <= 1024);
                control.check()?;
                query.work(units as u64);
                Ok::<_, crate::storage::Interrupted>(())
            })
        });
        assert_eq!(result, Err(crate::storage::Interrupted::Cancelled));
        assert_eq!(values.as_ptr(), pointer);
        assert_eq!(&values[..], &original);
        let after = query.snapshot().memory[MemoryKind::Workspace as usize];
        assert_eq!(after.current, before.current);
        assert!(after.peak >= 3 * before.current);
        assert_eq!(
            after.acquired - before.acquired,
            after.released - before.released
        );
        drop(values);
        query.snapshot().assert_memory_released();
        drop(query);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn checked_overlapping_moves_preserve_values_across_chunk_boundaries() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let original: Vec<_> = (0..8193u64).collect();
        for distance in [1, 1023, 1024, 1025] {
            for (source, destination) in [
                (0..original.len() - distance, distance),
                (distance..original.len(), 0),
            ] {
                let mut values = AccountedVec::from_slice_in(
                    &original,
                    Some(&AllocationContext::new(&query, MemoryKind::Workspace)),
                );
                values
                    .copy_within_with(source.clone(), destination, |units| {
                        assert!(units <= 1024);
                        Ok::<_, std::convert::Infallible>(())
                    })
                    .unwrap();
                let mut expected = original.clone();
                expected[destination..destination + source.len()]
                    .copy_from_slice(&original[source]);
                assert_eq!(&values[..], &expected);
            }
        }
        query.snapshot().assert_memory_released();
        drop(query);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn idle_roots_remain_observable_without_any_data_buffers() {
        let engine = EngineAccounting::default();
        let engine_bytes = std::alloc::Layout::new::<
            crate::shared::Allocation<Mutex<EngineObservations>>,
        >()
        .size() as u64;
        let query_bytes =
            std::alloc::Layout::new::<crate::shared::Allocation<QueryState>>().size() as u64;
        let current = |memory: [MemoryObservation; MEMORY_KINDS]| {
            memory.iter().map(|entry| entry.current).sum::<u64>()
        };
        assert_eq!(current(engine.snapshot().memory), engine_bytes);
        assert_eq!(
            engine.snapshot().memory[MemoryKind::Accounting as usize].current,
            engine_bytes
        );
        println!(
            "bookkeeping_layout engine={engine_bytes} query={query_bytes} shared_charge={} reference={} memory_kinds={MEMORY_KINDS}",
            size_of::<SharedCharge>(),
            size_of::<ReferenceObservation>()
        );
        let query = engine.query();
        assert_eq!(current(query.snapshot().memory), query_bytes);
        assert_eq!(
            current(engine.snapshot().memory),
            engine_bytes + query_bytes
        );
        let alias = query.clone();
        drop(query);
        assert_eq!(
            current(engine.snapshot().memory),
            engine_bytes + query_bytes
        );
        drop(alias);
        assert_eq!(current(engine.snapshot().memory), engine_bytes);
        engine.snapshot().assert_memory_released();
    }
    #[test]
    fn empty_owners_retain_one_query_root_and_do_not_reacquire_on_clone() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let baseline = engine.snapshot();
        let engine_alias = engine.clone();
        let query_alias = query.clone();
        assert_eq!(engine.snapshot(), baseline);
        let lease = query.hold(MemoryKind::Result, 0);
        let buffer = AccountedVec::<u64>::new(&query, MemoryKind::Result);
        drop((engine, query, query_alias));
        engine_alias
            .snapshot()
            .assert_memory_released_with_queries(1);
        drop(lease);
        engine_alias
            .snapshot()
            .assert_memory_released_with_queries(1);
        drop(buffer);
        engine_alias.snapshot().assert_memory_released();
    }
    #[test]
    fn root_identity_and_reconciliation_do_not_merge_independent_queries() {
        let engine = EngineAccounting::default();
        let left = engine.query();
        let right = engine.query();
        let mut a = left.hold(MemoryKind::ProviderHeld, 3);
        let mut b = right.hold(MemoryKind::ProviderHeld, 5);
        let context = AllocationContext::new(&left, MemoryKind::ProviderHeld);
        assert!(context.owns(&a));
        assert!(!context.owns(&b));
        let observed = engine.snapshot();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                MemoryLease::reconcile([(&mut a, 0), (&mut b, 0)])
            }))
            .is_err()
        );
        assert_eq!(engine.snapshot(), observed);
        drop((a, b, context, left, right));
        engine.snapshot().assert_memory_released();
    }
    #[test]
    fn poisoned_root_ledgers_recover_snapshots_and_balanced_release() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let baseline = engine.snapshot();
        let control = crate::storage::Control::default();
        let observation = control.observe(&query);
        let lease = query.hold(MemoryKind::Workspace, 19);
        for engine_mutex in [false, true] {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if engine_mutex {
                        let _guard = lock(&engine.0);
                        panic!("synthetic engine ledger poison");
                    } else {
                        let _guard = lock(&query.0.observations);
                        panic!("synthetic query ledger poison");
                    }
                }))
                .is_err()
            );
        }
        assert_eq!(
            query.snapshot().memory[MemoryKind::Workspace as usize].current,
            19
        );
        assert_eq!(query.snapshot().cancellation_references.external.current, 1);
        drop((observation, lease));
        query.snapshot().assert_memory_released();
        assert_eq!(
            engine.snapshot().memory[MemoryKind::Accounting as usize],
            baseline.memory[MemoryKind::Accounting as usize]
        );
        drop(query);
        engine.snapshot().assert_memory_released();
    }
    #[test]
    fn capacity_growth_accounts_overlap_and_survives_query_owner() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let mut result = AccountedVec::<u64>::new(&query, MemoryKind::Result);
        for value in 0..5 {
            result.push(value);
        }
        assert_eq!(&*result, &[0, 1, 2, 3, 4]);
        let observation = query.snapshot().memory[MemoryKind::Result as usize];
        assert_eq!(observation.current, 64);
        assert_eq!(observation.peak, 96);
        assert_eq!(observation.acquired, 96);
        assert_eq!(observation.released, 32);
        result.clear();
        assert_eq!(query.snapshot().memory[2].current, 64);
        drop(query);
        assert_eq!(engine.snapshot().memory[2].current, 64);
        drop(result);
        assert_eq!(engine.snapshot().memory[2].current, 0);
    }
    #[test]
    fn cloned_buffers_account_growth_and_retain_cleared_capacity() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let context = AllocationContext::new(&query, MemoryKind::Workspace);
        let source = AccountedVec::from_slice_in(&[1.0f64], Some(&context));
        let mut copy = source.clone();
        assert_eq!(query.snapshot().memory[1].current, 16);
        copy.insert(1, 2.0);
        let observation = query.snapshot().memory[1];
        assert_eq!((observation.current, observation.peak), (40, 48));
        copy.resize(6, 3.0);
        let observation = query.snapshot().memory[1];
        assert_eq!((observation.current, observation.peak), (72, 104));
        copy.clear();
        copy.clone_from(&source);
        assert_eq!(&*copy, &[1.0]);
        assert_eq!(query.snapshot().memory[1].current, 72);
        drop(context);
        drop(query);
        assert_eq!(engine.snapshot().memory[1].current, 72);
        drop(copy);
        assert_eq!(engine.snapshot().memory[1].current, 8);
        drop(source);
        assert_eq!(engine.snapshot().memory[1].current, 0);
    }
    #[test]
    fn invalid_insert_unwinds_without_growing_or_leaking_the_buffer() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let context = AllocationContext::new(&query, MemoryKind::Workspace);
            let mut buffer = AccountedVec::from_slice_in(&[1.0f64], Some(&context));
            buffer.insert(2, 2.0);
        }));
        assert!(panic.is_err());
        let observation = query.snapshot().memory[1];
        assert_eq!((observation.current, observation.peak), (0, 8));
        assert_eq!(observation.acquired, observation.released);
    }
    #[test]
    fn split_append_drain_and_consuming_iteration_retain_actual_buffer_owners() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let context = AllocationContext::new(&query, MemoryKind::Workspace);
        let mut source = AccountedVec::from_slice_in(&[0u64, 1, 2, 3, 4], Some(&context));
        let mut target = AccountedVec::from_slice_in(&[9u64], Some(&context));
        let mut tail = source.split_off(2);
        assert_eq!(&*source, &[0, 1]);
        assert_eq!(&*tail, &[2, 3, 4]);
        assert_eq!(query.snapshot().memory[1].current, 72);
        target.append(&mut tail);
        assert!(tail.is_empty());
        assert_eq!(tail.capacity(), 3);
        assert_eq!(query.snapshot().memory[1].current, 96);
        assert_eq!(query.snapshot().memory[1].peak, 104);
        drop(tail);
        assert_eq!(source.drain(..).next(), Some(0));
        assert!(source.is_empty());
        let mut values = target.into_iter();
        assert_eq!(values.len(), 4);
        assert_eq!(values.next(), Some(9));
        assert_eq!(values.next_back(), Some(4));
        assert_eq!(values.next(), Some(2));
        assert_eq!(values.next(), Some(3));
        assert_eq!(values.next(), None);
        assert_eq!(query.snapshot().memory[1].current, 72);
        drop(source);
        drop(context);
        drop(query);
        assert_eq!(engine.snapshot().memory[1].current, 32);
        drop(values);
        assert_eq!(engine.snapshot().memory[1].current, 0);
    }
    #[test]
    // Hashing uses immutable bytes; the accounting mutex is outside the key's equality.
    #[allow(clippy::mutable_key_type)]
    fn byte_keys_match_borrowed_slices_and_duplicate_keys_release_their_buffers() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let context = AllocationContext::new(&query, MemoryKind::Compiled);
        let mut values = std::collections::HashSet::new();
        assert!(values.insert(AccountedVec::from_slice_in(b"abc", Some(&context))));
        assert!(!values.insert(AccountedVec::from_slice_in(b"abc", Some(&context))));
        assert!(values.contains(&b"abc"[..]));
        assert_eq!(query.snapshot().memory[0].current, 3);
        drop(values);
        assert_eq!(query.snapshot().memory[0].current, 0);
    }
    #[test]
    fn an_observed_buffer_cannot_escape_as_an_unowned_vector() {
        let engine = EngineAccounting::default();
        let query = engine.query();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let buffer = AccountedVec::from_slice_in(
                b"abc",
                Some(&AllocationContext::new(&query, MemoryKind::Workspace)),
            );
            buffer.into_unobserved();
        }));
        assert!(panic.is_err());
        let observation = query.snapshot().memory[1];
        assert_eq!(observation.current, 0);
        assert_eq!(observation.acquired, observation.released);
    }
    #[test]
    fn independent_concurrent_owners_release_after_execution() {
        let engine = EngineAccounting::default();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let engine = engine.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    let query = engine.query();
                    let execution = query.execution();
                    let lease = query.hold(MemoryKind::ProviderHeld, 1024);
                    barrier.wait();
                    barrier.wait();
                    drop(execution);
                    assert_eq!(query.snapshot().memory[3].current, 1024);
                    drop(lease);
                });
            }
            barrier.wait();
            let snapshot = engine.snapshot();
            assert_eq!(snapshot.active_executions, 2);
            assert_eq!(snapshot.peak_executions, 2);
            assert_eq!(snapshot.memory[3].current, 2048);
            barrier.wait();
        });
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.active_executions, 0);
        assert_eq!(snapshot.completed_executions, 2);
        assert_eq!(snapshot.memory[3].current, 0);
        assert!(snapshot.elapsed > Duration::ZERO);
    }
}
