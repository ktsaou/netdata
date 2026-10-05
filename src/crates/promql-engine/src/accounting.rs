// SPDX-License-Identifier: GPL-3.0-or-later
//! Observations remain active without admitting, throttling or limiting queries.
use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, MutexGuard},
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
}
pub const MEMORY_KINDS: usize = 7;

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct QueryObservations {
    pub memory: [MemoryObservation; MEMORY_KINDS],
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
    pub active_executions: u64,
    pub peak_executions: u64,
    pub completed_executions: u64,
    pub elapsed: Duration,
    pub counter_overflowed: bool,
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
#[derive(Clone, Debug, Default)]
pub struct EngineAccounting(Arc<Mutex<EngineObservations>>);
impl EngineAccounting {
    pub fn snapshot(&self) -> EngineObservations {
        lock(&self.0).clone()
    }
    pub fn query(&self) -> QueryAccounting {
        QueryAccounting(Arc::new(QueryState {
            engine: self.clone(),
            observations: Mutex::new(QueryObservations::default()),
        }))
    }
}
#[derive(Debug)]
struct QueryState {
    engine: EngineAccounting,
    observations: Mutex<QueryObservations>,
}
#[derive(Clone, Debug)]
pub struct QueryAccounting(Arc<QueryState>);
impl QueryAccounting {
    pub fn snapshot(&self) -> QueryObservations {
        lock(&self.0.observations).clone()
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
    pub fn provider_call(&self) {
        self.count(|s| &mut s.provider_calls, 1);
    }
    pub fn page(&self, samples: usize) {
        self.count(|s| &mut s.pages, 1);
        self.count(|s| &mut s.samples_fetched, samples as u64);
    }
    pub fn work(&self, amount: u64) {
        self.count(|s| &mut s.work, amount);
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
        self.kind == lease.kind && Arc::ptr_eq(&self.query.0, &lease.query.0)
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
                .all(|(lease, _)| Arc::ptr_eq(&owner.0, &lease.query.0)),
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
    pub(crate) fn context(&self) -> Option<AllocationContext> {
        self.lease
            .as_ref()
            .map(|lease| AllocationContext::new(&lease.query, lease.kind))
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
    pub(crate) fn filled_in(len: usize, value: T, context: Option<&AllocationContext>) -> Self
    where
        T: Clone,
    {
        let mut result = Self::with_capacity_in(len, context);
        result.values.resize(len, value);
        result
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
impl std::fmt::Write for AccountedVec<u8> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.extend_from_slice(value.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
