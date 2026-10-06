// SPDX-License-Identifier: GPL-3.0-or-later
//! Reconcile owner snapshots at callbacks and releases without inventing transfer peaks.
use crate::{
    accounting::{AllocationContext, MemoryKind, QueryAccounting},
    shared::Strong,
    storage::*,
};
use std::cell::RefCell;

pub(crate) struct ObservedQuery<Q: QueryProvider> {
    value: Q,
    lease: RefCell<ResourceLease>,
    account: QueryAccounting,
}
impl<Q: QueryProvider> ObservedQuery<Q> {
    pub(crate) fn new(value: Q, account: &QueryAccounting) -> Self {
        let resources = value.resources();
        Self {
            value,
            lease: RefCell::new(ResourceLease::new(account, resources)),
            account: account.clone(),
        }
    }
    fn pair(&self, lease: &RefCell<ResourceLease>, resources: ProviderResources) {
        ResourceLease::pair(
            &mut self.lease.borrow_mut(),
            self.value.resources(),
            &mut lease.borrow_mut(),
            resources,
        );
    }
    fn triple(
        &self,
        a: &RefCell<ResourceLease>,
        ar: ProviderResources,
        b: &RefCell<ResourceLease>,
        br: ProviderResources,
    ) {
        ResourceLease::triple(
            &mut self.lease.borrow_mut(),
            self.value.resources(),
            &mut a.borrow_mut(),
            ar,
            &mut b.borrow_mut(),
            br,
        );
    }
    fn four(
        &self,
        a: (&RefCell<ResourceLease>, ProviderResources),
        b: (&RefCell<ResourceLease>, ProviderResources),
        c: (&RefCell<ResourceLease>, ProviderResources),
    ) {
        ResourceLease::quartet(
            (&mut self.lease.borrow_mut(), self.value.resources()),
            (&mut a.0.borrow_mut(), a.1),
            (&mut b.0.borrow_mut(), b.1),
            (&mut c.0.borrow_mut(), c.1),
        );
    }
}
trait SelectionObservation {
    fn snapshot(&self) -> (&RefCell<ResourceLease>, ProviderResources);
}
trait SeriesObservation {
    fn reconcile(&self, cursor: &RefCell<ResourceLease>, resources: ProviderResources);
}
struct SelectionOwner<S> {
    value: RefCell<Option<S>>,
    lease: RefCell<ResourceLease>,
}
impl<S: SeriesSelection> SelectionObservation for SelectionOwner<S> {
    fn snapshot(&self) -> (&RefCell<ResourceLease>, ProviderResources) {
        (
            &self.lease,
            self.value
                .borrow()
                .as_ref()
                .map_or_else(ProviderResources::default, SeriesSelection::resources),
        )
    }
}
pub(crate) struct ObservedSelection<'a, Q: QueryProvider, S> {
    owner: Strong<SelectionOwner<S>>,
    query: &'a ObservedQuery<Q>,
}
pub(crate) struct ObservedSeries<'a, Q: QueryProvider, S, P: SeriesSelection> {
    value: Option<S>,
    lease: RefCell<ResourceLease>,
    query: &'a ObservedQuery<Q>,
    parent: Strong<SelectionOwner<P>>,
}
pub(crate) struct ObservedCursor<'a, C> {
    value: Option<C>,
    lease: RefCell<ResourceLease>,
    parent: &'a dyn SeriesObservation,
}
impl<Q: QueryProvider> QueryProvider for ObservedQuery<Q> {
    type Selection<'a>
        = ObservedSelection<'a, Q, Q::Selection<'a>>
    where
        Self: 'a;
    // These adapters already own their charges; external scan owners must not duplicate them.
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn select(
        &self,
        request: SelectionRequest<'_>,
        control: &Control,
    ) -> Result<Self::Selection<'_>, StorageError> {
        let result = self.value.select(
            SelectionRequest {
                accounting: Some(&self.account),
                ..request
            },
            control,
        );
        match result {
            Ok(value) => {
                let resources = value.resources();
                let selection = ObservedSelection {
                    owner: Strong::new_in(
                        SelectionOwner {
                            value: RefCell::new(Some(value)),
                            lease: RefCell::new(ResourceLease::new(
                                &self.account,
                                ProviderResources::default(),
                            )),
                        },
                        Some(&AllocationContext::new(
                            &self.account,
                            MemoryKind::Workspace,
                        )),
                    ),
                    query: self,
                };
                self.pair(&selection.owner.lease, resources);
                Ok(selection)
            }
            Err(error) => {
                self.lease.borrow_mut().update(self.value.resources());
                Err(error)
            }
        }
    }
}
impl<'a, Q: QueryProvider, S: SeriesSelection + 'a> SeriesSelection
    for ObservedSelection<'a, Q, S>
{
    type Series = ObservedSeries<'a, Q, S::Series, S>;
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn next(&mut self, control: &Control) -> Result<Option<Self::Series>, StorageError> {
        let result = self
            .owner
            .value
            .borrow_mut()
            .as_mut()
            .expect("live selection")
            .next(control);
        let (_, selection_resources) = self.owner.snapshot();
        match result {
            Ok(Some(value)) => {
                let resources = value.resources();
                let series = ObservedSeries {
                    value: Some(value),
                    lease: RefCell::new(ResourceLease::new(
                        &self.query.account,
                        ProviderResources::default(),
                    )),
                    query: self.query,
                    parent: self.owner.clone(),
                };
                self.query.triple(
                    &self.owner.lease,
                    selection_resources,
                    &series.lease,
                    resources,
                );
                Ok(Some(series))
            }
            other => {
                self.query.pair(&self.owner.lease, selection_resources);
                other.map(|_| None)
            }
        }
    }
    fn diagnostic_count(&self) -> usize {
        let result = self
            .owner
            .value
            .borrow()
            .as_ref()
            .expect("live selection")
            .diagnostic_count();
        let (_, resources) = self.owner.snapshot();
        self.query.pair(&self.owner.lease, resources);
        result
    }
    fn diagnostic<R>(
        &self,
        index: usize,
        visit: impl for<'b> FnOnce(ProviderDiagnosticRef<'b>) -> R,
    ) -> R {
        let result = self
            .owner
            .value
            .borrow()
            .as_ref()
            .expect("live selection")
            .diagnostic(index, |diagnostic| {
                let (_, resources) = self.owner.snapshot();
                self.query.pair(&self.owner.lease, resources);
                visit(diagnostic)
            });
        let (_, resources) = self.owner.snapshot();
        self.query.pair(&self.owner.lease, resources);
        result
    }
}
impl<Q: QueryProvider, S: Series, P: SeriesSelection> ObservedSeries<'_, Q, S, P> {
    fn observe(&self) {
        let (parent, resources) = self.parent.snapshot();
        self.query.triple(
            parent,
            resources,
            &self.lease,
            self.value.as_ref().expect("live series").resources(),
        );
    }
}
impl<Q: QueryProvider, S: Series, P: SeriesSelection> SeriesObservation
    for ObservedSeries<'_, Q, S, P>
{
    fn reconcile(&self, cursor: &RefCell<ResourceLease>, resources: ProviderResources) {
        let selection = self.parent.snapshot();
        self.query.four(
            selection,
            (
                &self.lease,
                self.value.as_ref().expect("live series").resources(),
            ),
            (cursor, resources),
        );
    }
}
impl<Q: QueryProvider, S: Series, P: SeriesSelection> Series for ObservedSeries<'_, Q, S, P> {
    type Cursor<'a>
        = ObservedCursor<'a, S::Cursor<'a>>
    where
        Self: 'a;
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn label_count(&self) -> usize {
        let count = self.value.as_ref().expect("live series").label_count();
        self.observe();
        count
    }
    fn label(&self, index: usize) -> LabelRef<'_> {
        let label = self.value.as_ref().expect("live series").label(index);
        self.observe();
        label
    }
    fn cursor(
        &self,
        bounds: TimeBounds,
        control: &Control,
    ) -> Result<Self::Cursor<'_>, StorageError> {
        let result = self
            .value
            .as_ref()
            .expect("live series")
            .cursor(bounds, control);
        match result {
            Ok(value) => {
                let resources = value.resources();
                let cursor = ObservedCursor {
                    value: Some(value),
                    lease: RefCell::new(ResourceLease::new(
                        &self.query.account,
                        ProviderResources::default(),
                    )),
                    parent: self,
                };
                self.reconcile(&cursor.lease, resources);
                Ok(cursor)
            }
            Err(error) => {
                self.observe();
                Err(error)
            }
        }
    }
}
impl<C: SampleCursor> ObservedCursor<'_, C> {
    fn observe(&self) {
        self.parent.reconcile(
            &self.lease,
            self.value.as_ref().expect("live cursor").resources(),
        );
    }
}
impl<C: SampleCursor> SampleCursor for ObservedCursor<'_, C> {
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn max_page_samples(&self) -> usize {
        let count = self.value.as_ref().expect("live cursor").max_page_samples();
        self.observe();
        count
    }
    fn seek(&mut self, time_ms: i64, control: &Control) -> Result<(), StorageError> {
        let result = self
            .value
            .as_mut()
            .expect("live cursor")
            .seek(time_ms, control);
        self.observe();
        result
    }
    fn next(
        &mut self,
        control: &Control,
        resources: &mut ProviderResources,
    ) -> Result<Option<SamplePage<'_>>, StorageError> {
        let mut snapshot = self.value.as_ref().expect("live cursor").resources();
        let result = self
            .value
            .as_mut()
            .expect("live cursor")
            .next(control, &mut snapshot);
        self.parent.reconcile(&self.lease, snapshot);
        *resources = ProviderResources::default();
        match result {
            Ok(Some(mut page)) => {
                if page.resources != snapshot {
                    return Err(StorageError::Invalid(
                        "page resource snapshot differs from cursor",
                    ));
                }
                page.resources = ProviderResources::default();
                Ok(Some(page))
            }
            other => other,
        }
    }
}
impl<Q: QueryProvider, S> Drop for ObservedSelection<'_, Q, S> {
    fn drop(&mut self) {
        let value = self.owner.value.borrow_mut().take();
        drop(value);
        if !std::thread::panicking() {
            self.query
                .pair(&self.owner.lease, ProviderResources::default());
        }
    }
}
impl<Q: QueryProvider, S, P: SeriesSelection> Drop for ObservedSeries<'_, Q, S, P> {
    fn drop(&mut self) {
        drop(self.value.take());
        if !std::thread::panicking() {
            let (parent, resources) = self.parent.snapshot();
            self.query
                .triple(parent, resources, &self.lease, ProviderResources::default());
        }
    }
}
impl<C> Drop for ObservedCursor<'_, C> {
    fn drop(&mut self) {
        drop(self.value.take());
        if !std::thread::panicking() {
            self.parent
                .reconcile(&self.lease, ProviderResources::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        evaluator::EvaluationError,
        execution::{Engine, EngineOptions, ExecutionError, QueryRequest},
        storage::Provider,
        value::{Sample, SampleValue},
    };
    use std::{cell::Cell, rc::Rc, sync::OnceLock};
    #[derive(Clone, Copy, Default)]
    struct Mode {
        lazy: bool,
        transfer_next: bool,
        transfer_drop: bool,
        fail_seek: bool,
        fail_next: bool,
        mismatched_page: bool,
        named: bool,
        cancel_page: bool,
        pool: bool,
    }
    struct Source {
        held: RefCell<Vec<u8>>,
        mode: Mode,
    }
    struct SourceProvider(Mode);
    impl Provider for SourceProvider {
        type Query<'a> = Source;
        fn open(&self, _: TimeBounds, _: &Control) -> Result<Source, StorageError> {
            Ok(Source {
                held: RefCell::new(vec![]),
                mode: self.0,
            })
        }
    }
    struct Selection<'a> {
        query: &'a Source,
        returned: bool,
        pool: Rc<Pool>,
    }
    struct Pool {
        held: RefCell<Vec<u8>>,
        closed: Cell<bool>,
        calls: Cell<usize>,
        closes: Cell<usize>,
    }
    struct Handle<'a> {
        query: &'a Source,
        metadata: OnceLock<Vec<u8>>,
        pool: Rc<Pool>,
        held: Vec<u8>,
    }
    struct Cursor<'a> {
        query: &'a Source,
        held: Vec<u8>,
        returned: bool,
        samples: [Sample; 1],
    }
    impl QueryProvider for Source {
        type Selection<'a> = Selection<'a>;
        fn resources(&self) -> ProviderResources {
            ProviderResources {
                held_bytes: self.held.borrow().capacity() as u64,
                ..ProviderResources::default()
            }
        }
        fn select(
            &self,
            _: SelectionRequest<'_>,
            _: &Control,
        ) -> Result<Self::Selection<'_>, StorageError> {
            Ok(Selection {
                query: self,
                returned: false,
                pool: Rc::new(Pool {
                    held: RefCell::new(if self.mode.pool { vec![0; 16] } else { vec![] }),
                    closed: Cell::new(false),
                    calls: Cell::new(0),
                    closes: Cell::new(0),
                }),
            })
        }
    }
    impl<'a> SeriesSelection for Selection<'a> {
        type Series = Handle<'a>;
        fn resources(&self) -> ProviderResources {
            assert!(!self.pool.closed.get());
            self.pool.calls.set(self.pool.calls.get() + 1);
            ProviderResources {
                held_bytes: self.pool.held.borrow().capacity() as u64,
                ..ProviderResources::default()
            }
        }
        fn next(&mut self, _: &Control) -> Result<Option<Self::Series>, StorageError> {
            if self.returned {
                return Ok(None);
            }
            self.returned = true;
            Ok(Some(Handle {
                query: self.query,
                metadata: OnceLock::new(),
                pool: self.pool.clone(),
                held: std::mem::take(&mut *self.pool.held.borrow_mut()),
            }))
        }
    }
    impl Series for Handle<'_> {
        type Cursor<'a>
            = Cursor<'a>
        where
            Self: 'a;
        fn label_count(&self) -> usize {
            if self.query.mode.lazy {
                self.metadata.get_or_init(|| vec![0; 37]);
            }
            usize::from(self.query.mode.named)
        }
        fn label(&self, _: usize) -> LabelRef<'_> {
            assert!(self.query.mode.named);
            LabelRef {
                name: b"__name__",
                value: b"m",
            }
        }
        fn resources(&self) -> ProviderResources {
            ProviderResources {
                held_bytes: (self.metadata.get().map_or(0, |v| v.capacity()) + self.held.capacity())
                    as u64,
                ..ProviderResources::default()
            }
        }
        fn cursor(&self, _: TimeBounds, _: &Control) -> Result<Self::Cursor<'_>, StorageError> {
            Ok(Cursor {
                query: self.query,
                held: vec![0; 16],
                returned: false,
                samples: [Sample {
                    time_ms: 0,
                    start_time_ms: 0,
                    value: SampleValue::Float(1.0),
                }],
            })
        }
    }
    impl Drop for Selection<'_> {
        fn drop(&mut self) {
            assert!(!self.pool.closed.replace(true));
            self.pool.closes.set(self.pool.closes.get() + 1);
            *self.pool.held.borrow_mut() = vec![];
        }
    }
    impl Drop for Handle<'_> {
        fn drop(&mut self) {
            if !self.pool.closed.get() {
                *self.pool.held.borrow_mut() = std::mem::take(&mut self.held);
            }
        }
    }
    impl Cursor<'_> {
        fn transfer(&mut self) {
            if !self.held.is_empty() {
                *self.query.held.borrow_mut() = std::mem::take(&mut self.held);
            }
        }
    }
    impl SampleCursor for Cursor<'_> {
        fn resources(&self) -> ProviderResources {
            ProviderResources {
                held_bytes: self.held.capacity() as u64,
                ..ProviderResources::default()
            }
        }
        fn max_page_samples(&self) -> usize {
            1
        }
        fn seek(&mut self, _: i64, _: &Control) -> Result<(), StorageError> {
            if self.query.mode.fail_seek {
                self.query.held.borrow_mut().resize(128, 0);
                return Err(StorageError::Provider(vec![1]));
            }
            Ok(())
        }
        fn next(
            &mut self,
            control: &Control,
            resources: &mut ProviderResources,
        ) -> Result<Option<SamplePage<'_>>, StorageError> {
            if self.query.mode.transfer_next {
                self.transfer();
            }
            *resources = self.resources();
            if self.query.mode.fail_next {
                return Err(StorageError::Provider(vec![2]));
            }
            if self.returned {
                self.query.held.borrow_mut().resize(128, 0);
                *resources = self.resources();
                return Ok(None);
            }
            self.returned = true;
            if self.query.mode.cancel_page {
                control.cancellation.cancel();
            }
            Ok(Some(SamplePage {
                samples: PageSamples::Samples(&self.samples),
                resources: if self.query.mode.mismatched_page {
                    ProviderResources::default()
                } else {
                    *resources
                },
            }))
        }
    }
    impl Drop for Cursor<'_> {
        fn drop(&mut self) {
            if self.query.mode.transfer_drop {
                self.transfer();
            }
        }
    }
    fn held(account: &QueryAccounting) -> u64 {
        account.snapshot().memory[MemoryKind::ProviderHeld as usize].current
    }
    #[test]
    fn selection_pool_transfers_and_release_with_live_descendants_keep_owner_timing() {
        for close_first in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let query = ObservedQuery::new(
                Source {
                    held: RefCell::new(vec![]),
                    mode: Mode {
                        pool: true,
                        ..Mode::default()
                    },
                },
                &account,
            );
            let control = Control::default();
            let bounds = TimeBounds {
                start_ms: 0,
                end_ms: 0,
            };
            let mut selection = query
                .select(
                    SelectionRequest {
                        accounting: None,
                        matchers: &[],
                        bounds,
                        hints: Default::default(),
                    },
                    &control,
                )
                .unwrap();
            let wrapper =
                size_of::<crate::shared::Allocation<SelectionOwner<Selection<'_>>>>() as u64;
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].current,
                wrapper
            );
            assert_eq!(held(&account), 16);
            let series = selection.next(&control).unwrap().unwrap();
            let pool = series.value.as_ref().unwrap().pool.clone();
            assert_eq!(held(&account), 16);
            if close_first {
                let cursor = series.cursor(bounds, &control).unwrap();
                assert_eq!(held(&account), 32);
                drop(selection);
                assert_eq!(pool.closes.get(), 1);
                assert_eq!(
                    account.snapshot().memory[MemoryKind::Workspace as usize].current,
                    wrapper
                );
                let calls = pool.calls.get();
                assert_eq!(cursor.max_page_samples(), 1);
                drop(cursor);
                drop(series);
                assert_eq!(pool.calls.get(), calls);
            } else {
                drop(series);
                assert_eq!(pool.held.borrow().capacity(), 16);
                assert_eq!(held(&account), 16);
                assert_eq!(
                    account.snapshot().memory[MemoryKind::ProviderHeld as usize].peak,
                    16
                );
                assert!(selection.next(&control).unwrap().is_none());
                drop(selection);
            }
            assert_eq!(pool.closes.get(), 1);
            assert_eq!(held(&account), 0);
            let memory = account.snapshot().memory[MemoryKind::Workspace as usize];
            assert_eq!(memory.current, 0);
            assert_eq!(memory.acquired, wrapper);
            assert_eq!(memory.released, wrapper);
            drop(query);
            assert_eq!(held(&account), 0);
        }
    }
    #[test]
    fn observed_execution_rejects_inconsistent_page_snapshots_and_releases_owners() {
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(b"m[1s]", EngineOptions::default(), &control)
            .unwrap();
        let report = query.collect_reported(
            &SourceProvider(Mode {
                mismatched_page: true,
                named: true,
                ..Mode::default()
            }),
            QueryRequest::Instant { time_ms: 0 },
            &control,
        );
        assert!(matches!(
            report.result,
            Err(ExecutionError::Evaluation(EvaluationError::Storage(
                StorageError::Invalid("page resource snapshot differs from cursor")
            )))
        ));
        assert_eq!(
            report.observations.memory[MemoryKind::ProviderHeld as usize].peak,
            16
        );
        assert!(
            report
                .observations
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
        assert_eq!(engine.accounting.snapshot().active_executions, 0);
    }
    #[test]
    fn cancellation_after_a_returned_page_keeps_exposure_and_received_counters() {
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(b"m[1s]", EngineOptions::default(), &control)
            .unwrap();
        let report = query.collect_reported(
            &SourceProvider(Mode {
                cancel_page: true,
                named: true,
                ..Mode::default()
            }),
            QueryRequest::Instant { time_ms: 0 },
            &control,
        );
        assert!(matches!(
            report.result,
            Err(ExecutionError::Evaluation(EvaluationError::Interrupted(_)))
        ));
        assert_eq!(report.observations.pages, 1);
        assert_eq!(report.observations.samples_fetched, 1);
        assert_eq!(
            report.observations.memory[MemoryKind::BorrowedView as usize].peak,
            size_of::<Sample>() as u64
        );
        assert_eq!(report.observations.output_samples, 0);
        assert!(
            report
                .observations
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }
    #[test]
    fn lazy_metadata_and_callback_or_release_transfers_are_one_observation() {
        for mode in [
            Mode {
                transfer_next: true,
                ..Mode::default()
            },
            Mode {
                transfer_drop: true,
                ..Mode::default()
            },
            Mode {
                transfer_next: true,
                fail_next: true,
                ..Mode::default()
            },
            Mode {
                lazy: true,
                transfer_drop: true,
                ..Mode::default()
            },
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let query = ObservedQuery::new(
                Source {
                    held: RefCell::new(vec![]),
                    mode,
                },
                &account,
            );
            let mut selection = query
                .select(
                    SelectionRequest {
                        accounting: None,
                        matchers: &[],
                        hints: Default::default(),
                        bounds: TimeBounds {
                            start_ms: 0,
                            end_ms: 0,
                        },
                    },
                    &control,
                )
                .unwrap();
            let series = selection.next(&control).unwrap().unwrap();
            series.label_count();
            let metadata = if mode.lazy { 37 } else { 0 };
            assert_eq!(held(&account), metadata);
            let mut cursor = series
                .cursor(
                    TimeBounds {
                        start_ms: 0,
                        end_ms: 0,
                    },
                    &control,
                )
                .unwrap();
            assert_eq!(held(&account), metadata + 16);
            if mode.transfer_next {
                let mut snapshot = ProviderResources::default();
                let result = cursor.next(&control, &mut snapshot);
                assert_eq!(result.is_err(), mode.fail_next);
                assert_eq!(held(&account), metadata + 16);
            }
            drop(cursor);
            assert_eq!(held(&account), metadata + 16);
            assert_eq!(
                account.snapshot().memory[MemoryKind::ProviderHeld as usize].peak,
                metadata + 16
            );
            drop(series);
            assert_eq!(held(&account), 16);
            drop(selection);
            drop(query);
            assert_eq!(held(&account), 0);
        }
    }
    #[test]
    fn seek_error_and_eof_publish_query_growth_before_cleanup() {
        for fail_seek in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let query = ObservedQuery::new(
                Source {
                    held: RefCell::new(vec![]),
                    mode: Mode {
                        fail_seek,
                        ..Mode::default()
                    },
                },
                &account,
            );
            let mut selection = query
                .select(
                    SelectionRequest {
                        accounting: None,
                        matchers: &[],
                        hints: Default::default(),
                        bounds: TimeBounds {
                            start_ms: 0,
                            end_ms: 0,
                        },
                    },
                    &control,
                )
                .unwrap();
            let series = selection.next(&control).unwrap().unwrap();
            let mut cursor = series
                .cursor(
                    TimeBounds {
                        start_ms: 0,
                        end_ms: 0,
                    },
                    &control,
                )
                .unwrap();
            if fail_seek {
                assert!(cursor.seek(0, &control).is_err());
            } else {
                let mut resources = ProviderResources::default();
                assert!(cursor.next(&control, &mut resources).unwrap().is_some());
                assert!(cursor.next(&control, &mut resources).unwrap().is_none());
            }
            assert_eq!(held(&account), 144);
            drop(cursor);
            assert_eq!(held(&account), 128);
            drop(series);
            drop(selection);
            drop(query);
            assert_eq!(held(&account), 0);
        }
    }
}
