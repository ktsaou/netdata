// SPDX-License-Identifier: GPL-3.0-or-later
//! Storage phase witnesses grounded in pinned populateSeries/checkAndExpandSeriesSet.
use crate::{
    accounting::MemoryKind,
    annotations::Severity,
    evaluator::EvaluationError,
    execution::{Engine, EngineOptions, ExecutionError, QueryRequest},
    storage::{
        Control, LabelRef, Provider, ProviderDiagnosticRef, ProviderResources, QueryProvider,
        SelectionRequest, Series, SeriesSelection, StorageError, TimeBounds,
    },
    test_storage::{
        FixtureHandle, FixtureProvider, FixtureQuery, FixtureSelection, GeneratedCursor,
    },
};
use std::cell::RefCell;

#[derive(Debug)]
enum Event {
    Open {
        start_ms: i64,
        end_ms: i64,
    },
    Select(Vec<u8>),
    Metadata {
        site: usize,
        index: usize,
    },
    Cursor {
        site: usize,
        index: usize,
        end_ms: i64,
    },
    SeriesClosed {
        site: usize,
        index: usize,
    },
    SelectionClosed(usize),
    QueryClosed,
}
struct Source {
    data: FixtureProvider,
    events: RefCell<Vec<Event>>,
    fail_select: Option<usize>,
    fail_metadata: Option<(usize, usize)>,
    fail_cursor: Option<(usize, usize)>,
    cancel_metadata: Option<(usize, usize)>,
    notes: Vec<(usize, Severity, Vec<u8>)>,
}
impl Source {
    fn new() -> Self {
        let mut data = FixtureProvider::new(4, 1, 2);
        data.filter_selectors = true;
        for (series, name) in data.series.iter_mut().zip([b"m", b"n"]) {
            series.labels[0].1 = name.to_vec();
        }
        Self {
            data,
            events: RefCell::new(Vec::new()),
            fail_select: None,
            fail_metadata: None,
            fail_cursor: None,
            cancel_metadata: None,
            notes: Vec::new(),
        }
    }
    fn select_names(&self) -> Vec<Vec<u8>> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Select(name) => Some(name.clone()),
                _ => None,
            })
            .collect()
    }
    fn assert_closed(&self) {
        self.data.assert_closed();
        let events = self.events.borrow();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::Open { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::QueryClosed))
                .count(),
            1
        );
        let selected = self.select_names().len();
        for site in 0..selected {
            let closed = events
                .iter()
                .filter(|event| matches!(event, Event::SelectionClosed(id) if *id == site))
                .count();
            assert_eq!(closed, usize::from(self.fail_select != Some(site)));
        }
    }
}
struct Query<'a> {
    source: &'a Source,
    inner: FixtureQuery<'a>,
}
struct Selection<'a> {
    source: &'a Source,
    inner: FixtureSelection<'a>,
    site: usize,
    index: usize,
}
struct Handle<'a> {
    source: &'a Source,
    inner: FixtureHandle<'a>,
    site: usize,
    index: usize,
}
impl Provider for Source {
    type Query<'a> = Query<'a>;
    fn open(&self, bounds: TimeBounds, control: &Control) -> Result<Self::Query<'_>, StorageError> {
        self.events.borrow_mut().push(Event::Open {
            start_ms: bounds.start_ms,
            end_ms: bounds.end_ms,
        });
        Ok(Query {
            source: self,
            inner: self.data.open(bounds, control)?,
        })
    }
}
impl QueryProvider for Query<'_> {
    type Selection<'a>
        = Selection<'a>
    where
        Self: 'a;
    fn resources(&self) -> ProviderResources {
        self.inner.resources()
    }
    fn select(
        &self,
        request: SelectionRequest<'_>,
        control: &Control,
    ) -> Result<Self::Selection<'_>, StorageError> {
        let name = request
            .matchers
            .iter()
            .flatten()
            .find(|matcher| matcher.name == b"__name__")
            .map_or(&b""[..], |matcher| &matcher.value);
        let site = self.source.select_names().len();
        self.source
            .events
            .borrow_mut()
            .push(Event::Select(name.to_vec()));
        if self.source.fail_select == Some(site) {
            let mut message = Vec::with_capacity(8192);
            message.extend_from_slice(b"source-a");
            return Err(StorageError::Provider(message));
        }
        Ok(Selection {
            source: self.source,
            inner: self.inner.select(request, control)?,
            site,
            index: 0,
        })
    }
}
impl<'a> SeriesSelection for Selection<'a> {
    type Series = Handle<'a>;
    fn resources(&self) -> ProviderResources {
        self.inner.resources()
    }
    fn next(&mut self, control: &Control) -> Result<Option<Self::Series>, StorageError> {
        self.source.events.borrow_mut().push(Event::Metadata {
            site: self.site,
            index: self.index,
        });
        if self.source.cancel_metadata == Some((self.site, self.index)) {
            control.cancellation.cancel();
        }
        if self.source.fail_metadata == Some((self.site, self.index)) {
            return Err(StorageError::Provider(b"metadata-failed".to_vec()));
        }
        let result = self.inner.next(control)?;
        let index = self.index;
        self.index += usize::from(result.is_some());
        Ok(result.map(|inner| Handle {
            source: self.source,
            inner,
            site: self.site,
            index,
        }))
    }
    fn diagnostic_count(&self) -> usize {
        self.source
            .notes
            .iter()
            .filter(|(site, _, _)| *site == self.site)
            .count()
    }
    fn diagnostic<R>(
        &self,
        index: usize,
        visit: impl for<'b> FnOnce(ProviderDiagnosticRef<'b>) -> R,
    ) -> R {
        let (_, severity, message) = self
            .source
            .notes
            .iter()
            .filter(|(site, _, _)| *site == self.site)
            .nth(index)
            .unwrap();
        visit(ProviderDiagnosticRef {
            message,
            severity: *severity,
            position: None,
        })
    }
}
impl Series for Handle<'_> {
    type Cursor<'a>
        = GeneratedCursor<'a>
    where
        Self: 'a;
    fn label_count(&self) -> usize {
        self.inner.label_count()
    }
    fn label(&self, index: usize) -> LabelRef<'_> {
        self.inner.label(index)
    }
    fn resources(&self) -> ProviderResources {
        self.inner.resources()
    }
    fn cursor(
        &self,
        bounds: TimeBounds,
        control: &Control,
    ) -> Result<Self::Cursor<'_>, StorageError> {
        self.source.events.borrow_mut().push(Event::Cursor {
            site: self.site,
            index: self.index,
            end_ms: bounds.end_ms,
        });
        if self.source.fail_cursor == Some((self.site, self.index)) {
            return Err(StorageError::Provider(b"raw-failed".to_vec()));
        }
        self.inner.cursor(bounds, control)
    }
}
impl Drop for Handle<'_> {
    fn drop(&mut self) {
        self.source.events.borrow_mut().push(Event::SeriesClosed {
            site: self.site,
            index: self.index,
        });
    }
}
impl Drop for Selection<'_> {
    fn drop(&mut self) {
        self.source
            .events
            .borrow_mut()
            .push(Event::SelectionClosed(self.site));
    }
}
impl Drop for Query<'_> {
    fn drop(&mut self) {
        self.source.events.borrow_mut().push(Event::QueryClosed);
    }
}

fn request() -> QueryRequest {
    QueryRequest::Range {
        start_ms: 1000,
        end_ms: 2000,
        step_ms: 1000,
    }
}

#[test]
fn label_runtime_validation_precedes_child_expansion_but_follows_all_selects() {
    // Source functions.go:2483-2499 validates before ev.eval; populateSeries is upfront.
    let cases: &[(&[u8], bool)] = &[
        (b"label_replace(m+n,\"\",\"x\",\"id\",\"(\")", false),
        (b"label_replace(m,\"dst\",\"x\",\"id\",\"(\")+n", false),
        (b"m+label_replace(n,\"dst\",\"x\",\"id\",\"(\")", true),
    ];
    for (expression, left_expanded) in cases {
        let mut source = Source::new();
        source.fail_select = Some(1);
        source.notes = vec![(0, Severity::Warning, b"unfinished-left".to_vec())];
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(expression, EngineOptions::default(), &control)
            .unwrap();
        let report = query.collect_reported(&source, request(), &control);
        let Err(ExecutionError::Evaluation(EvaluationError::Runtime(message))) = &report.result
        else {
            panic!("expected label regex failure: {:?}", report.result)
        };
        assert_eq!(
            &message[..],
            b"invalid regular expression in label_replace(): ("
        );
        assert_eq!(source.select_names(), [b"m".to_vec(), b"n".to_vec()]);
        assert_eq!(
            source
                .events
                .borrow()
                .iter()
                .any(|event| matches!(event, Event::Metadata { site: 0, .. })),
            *left_expanded
        );
        assert!(
            !source
                .events
                .borrow()
                .iter()
                .any(|event| matches!(event, Event::Metadata { site: 1, .. }))
        );
        assert!(report.annotations.is_empty());
        assert_eq!(
            report.observations.memory[MemoryKind::Workspace as usize].current,
            message.capacity() as u64
        );
        assert_eq!(
            report.observations.memory[MemoryKind::RegexEstimate as usize].current,
            0
        );
        source.assert_closed();
        drop(report);
        assert_eq!(
            engine.accounting.snapshot().memory[MemoryKind::Workspace as usize].current,
            0
        );
    }
}

#[test]
fn label_projection_waits_for_late_child_failure_and_drops_pending_regex_on_cancel() {
    for cancelled in [false, true] {
        let mut source = Source::new();
        source.data.series[1].labels[0].1 = b"m".to_vec();
        source.notes = vec![(0, Severity::Warning, b"unfinished-child".to_vec())];
        if cancelled {
            source.cancel_metadata = Some((0, 1));
        } else {
            source.fail_cursor = Some((0, 1));
        }
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(
                b"label_replace(m,\"instance\",\"\",\"instance\",\".*\")",
                EngineOptions::default(),
                &control,
            )
            .unwrap();
        let report = query.collect_reported(&source, request(), &control);
        if cancelled {
            assert!(matches!(
                &report.result,
                Err(ExecutionError::Evaluation(
                    EvaluationError::Interrupted(_)
                        | EvaluationError::Storage(StorageError::Interrupted(_))
                ))
            ));
        } else {
            assert!(matches!(&report.result, Err(ExecutionError::Evaluation(
                EvaluationError::Storage(StorageError::Provider(message))
            )) if message == b"raw-failed"));
            assert!(source.events.borrow().iter().any(|event| matches!(
                event,
                Event::Cursor {
                    site: 0,
                    index: 0,
                    end_ms: 2000
                }
            )));
        }
        assert!(report.annotations.is_empty());
        assert_eq!(
            report.observations.memory[MemoryKind::Workspace as usize].current,
            0
        );
        assert_eq!(
            report.observations.memory[MemoryKind::RegexEstimate as usize].current,
            0
        );
        source.assert_closed();
    }
}

#[test]
fn selection_sites_open_in_syntax_order_before_metadata_and_only_once() {
    let source = Source::new();
    let engine = Engine::default();
    let control = Control::default();
    let query = engine
        .compile(
            b"sum(m)+scalar(last_over_time(n[1s]))",
            EngineOptions::default(),
            &control,
        )
        .unwrap();
    let result = query.collect(&source, request(), &control).unwrap();
    drop(result);
    source.assert_closed();
    assert_eq!(source.select_names(), [b"m".to_vec(), b"n".to_vec()]);
    let events = source.events.borrow();
    let first_metadata = events
        .iter()
        .position(|event| matches!(event, Event::Metadata { .. }))
        .unwrap();
    assert_eq!(
        events[..first_metadata]
            .iter()
            .filter(|event| matches!(event, Event::Select(_)))
            .count(),
        2
    );
}

#[test]
fn identical_syntactic_selectors_keep_distinct_query_owned_sites() {
    let source = Source::new();
    let engine = Engine::default();
    let control = Control::default();
    let query = engine
        .compile(
            b"scalar(sum(m))+scalar(sum(m))",
            EngineOptions::default(),
            &control,
        )
        .unwrap();
    let result = query.collect(&source, request(), &control).unwrap();
    drop(result);
    source.assert_closed();
    assert_eq!(source.select_names(), [b"m".to_vec(), b"m".to_vec()]);
}

#[test]
fn whole_selector_metadata_expands_before_any_raw_cursor() {
    let source = Source::new();
    let engine = Engine::default();
    let control = Control::default();
    let query = engine
        .compile(
            b"sum({__name__=~\"m|n\"})",
            EngineOptions::default(),
            &control,
        )
        .unwrap();
    let result = query
        .collect(&source, QueryRequest::Instant { time_ms: 1000 }, &control)
        .unwrap();
    drop(result);
    source.assert_closed();
    let events = source.events.borrow();
    let first_cursor = events
        .iter()
        .position(|event| {
            matches!(
                event,
                Event::Cursor {
                    site: 0,
                    index: 0,
                    ..
                }
            )
        })
        .unwrap();
    assert!(
        events[..first_cursor]
            .iter()
            .any(|event| matches!(event, Event::Metadata { site: 0, index: 2 }))
    );
}

#[test]
fn deferred_select_failure_still_opens_later_ast_sites() {
    let mut source = Source::new();
    source.fail_select = Some(0);
    let engine = Engine::default();
    let control = Control::default();
    let query = engine
        .compile(b"sum(m)+scalar(sum(n))", EngineOptions::default(), &control)
        .unwrap();
    let result = query.collect(&source, request(), &control);
    assert!(result.is_err());
    drop(result);
    source.assert_closed();
    assert_eq!(source.select_names(), [b"m".to_vec(), b"n".to_vec()]);
}

#[test]
fn ordinary_raw_series_finish_steps_before_the_next_series_and_direct_clocks_keep_steps_first() {
    for expression in [
        "{__name__=~\"m|n\"}",
        "timestamp({__name__=~\"m|n\"})",
        "timestamp(({__name__=~\"m|n\"}))",
    ] {
        let source = Source::new();
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(expression.as_bytes(), EngineOptions::default(), &control)
            .unwrap();
        drop(query.collect(&source, request(), &control).unwrap());
        source.assert_closed();
        let reads: Vec<_> = source
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Cursor {
                    site: 0,
                    index,
                    end_ms,
                } => Some((*index, *end_ms)),
                _ => None,
            })
            .collect();
        let expected = if expression.starts_with("timestamp") {
            [(0, 1000), (1, 1000), (0, 2000), (1, 2000)]
        } else {
            [(0, 1000), (0, 2000), (1, 1000), (1, 2000)]
        };
        assert_eq!(&reads[..4], &expected, "{expression}");
        let events = source.events.borrow();
        let close = events
            .iter()
            .position(|event| matches!(event, Event::SelectionClosed(0)))
            .unwrap();
        for index in 0..2 {
            assert!(events[..close].iter().any(|event| matches!(event, Event::SeriesClosed { site: 0, index: value } if *value == index)));
        }
        assert!(matches!(events.last(), Some(Event::QueryClosed)));
    }
}

#[test]
fn original_query_bounds_reach_open_before_adjusted_lookup_clocks() {
    let source = Source::new();
    let engine = Engine::default();
    let control = Control::default();
    let query = engine
        .compile(
            b"last_over_time(m[2s] offset 1s)",
            EngineOptions::default(),
            &control,
        )
        .unwrap();
    drop(query.collect(&source, request(), &control).unwrap());
    assert!(matches!(
        source.events.borrow()[0],
        Event::Open {
            start_ms: -1999,
            end_ms: 1000
        }
    ));
    source.assert_closed();
}

#[test]
fn opaque_terminal_annotations_keep_bytes_and_explicit_severity_after_scope_release() {
    let mut source = Source::new();
    source.notes = vec![
        (0, Severity::Warning, b"bytes\0\xff\n".to_vec()),
        (0, Severity::Warning, b"bytes\0\xff\n".to_vec()),
        (0, Severity::Info, b"provider-information".to_vec()),
    ];
    let engine = Engine::default();
    let control = Control::default();
    let query = engine
        .compile(b"m", EngineOptions::default(), &control)
        .unwrap();
    let report = query.collect_reported(&source, request(), &control);
    assert!(report.result.is_ok());
    source.assert_closed();
    drop(query);
    drop(source);
    let notes: Vec<_> = report.annotations.iter().collect();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].message, b"bytes\0\xff\n");
    assert_eq!(notes[0].severity, Severity::Warning);
    assert_eq!(notes[0].span, None);
    assert_eq!(notes[1].severity, Severity::Info);
    drop(report);
    assert!(
        engine
            .accounting
            .snapshot()
            .memory
            .iter()
            .all(|kind| kind.current == 0)
    );
}

#[test]
fn expansion_failure_notes_are_local_and_plain_raw_failure_discards_unfinished_notes() {
    for (expression, fail_site, expected) in [
        ("m", 0, vec![b"own".to_vec()]),
        ("m[1s]", 0, vec![b"own".to_vec()]),
        ("timestamp(m)", 0, vec![b"own".to_vec()]),
        ("scalar(m)+scalar(n)", 1, vec![b"later".to_vec()]),
        (
            "predict_linear(n[1s], scalar(m))",
            0,
            vec![b"later".to_vec(), b"own".to_vec()],
        ),
        ("quantile(scalar(m), n)", 0, vec![b"own".to_vec()]),
    ] {
        let mut source = Source::new();
        source.fail_metadata = Some((fail_site, 0));
        source.notes = vec![
            (0, Severity::Warning, b"own".to_vec()),
            (1, Severity::Warning, b"later".to_vec()),
        ];
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(expression.as_bytes(), EngineOptions::default(), &control)
            .unwrap();
        let report =
            query.collect_reported(&source, QueryRequest::Instant { time_ms: 1000 }, &control);
        assert!(
            matches!(&report.result, Err(ExecutionError::Evaluation(EvaluationError::Expansion(message))) if &**message == b"expanding series: metadata-failed"),
            "{expression}: {:?}",
            report.result
        );
        let actual: Vec<_> = report
            .annotations
            .iter()
            .map(|note| note.message.to_vec())
            .collect();
        assert_eq!(actual, expected, "{expression}");
        source.assert_closed();
        drop(report);
        assert_eq!(
            engine.accounting.snapshot().memory[MemoryKind::Result as usize].current,
            0
        );
    }
    let mut source = Source::new();
    source.notes = vec![(0, Severity::Warning, b"unfinished".to_vec())];
    source.fail_cursor = Some((0, 0));
    let engine = Engine::default();
    let control = Control::default();
    let query = engine
        .compile(b"m", EngineOptions::default(), &control)
        .unwrap();
    let report = query.collect_reported(&source, request(), &control);
    assert!(report.result.is_err());
    assert!(report.annotations.is_empty());
    source.assert_closed();
}

#[test]
fn opaque_and_generated_notes_use_the_same_key_with_the_incoming_owner() {
    let raw = b"PromQL info: metric might not be a counter, name does not end in _total/_sum/_count/_bucket: \"m\"";
    for (expression, incoming_opaque) in [
        ("scalar(rate(m[2s]))+scalar(n)", true),
        ("scalar(n)+scalar(rate(m[2s]))", false),
    ] {
        let mut source = Source::new();
        source.notes = vec![(
            usize::from(incoming_opaque),
            Severity::Warning,
            raw.to_vec(),
        )];
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(expression.as_bytes(), EngineOptions::default(), &control)
            .unwrap();
        let report =
            query.collect_reported(&source, QueryRequest::Instant { time_ms: 2000 }, &control);
        assert!(report.result.is_ok(), "{expression}: {:?}", report.result);
        let notes: Vec<_> = report.annotations.iter().collect();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].raw_message, raw);
        assert_eq!(
            notes[0].severity,
            if incoming_opaque {
                Severity::Warning
            } else {
                Severity::Info
            }
        );
        assert_eq!(notes[0].span.is_none(), incoming_opaque);
        source.assert_closed();
    }
}

#[test]
fn deferred_error_capacity_and_cancelled_partial_catalog_owners_release() {
    let mut source = Source::new();
    source.fail_select = Some(0);
    let engine = Engine::default();
    let control = Control::default();
    let query = engine
        .compile(b"scalar(m)+scalar(n)", EngineOptions::default(), &control)
        .unwrap();
    let report = query.collect_reported(&source, request(), &control);
    assert!(report.result.is_err());
    assert!(report.observations.memory[MemoryKind::Workspace as usize].peak >= 8192);
    assert_eq!(
        report.observations.memory[MemoryKind::Workspace as usize].current,
        0
    );
    assert!(report.observations.memory[MemoryKind::Result as usize].current > 0);
    source.assert_closed();
    drop(report);
    assert_eq!(
        engine.accounting.snapshot().memory[MemoryKind::Result as usize].current,
        0
    );

    let mut source = Source::new();
    source.cancel_metadata = Some((0, 1));
    source.notes = vec![(0, Severity::Warning, b"unfinished".to_vec())];
    let control = Control::default();
    let query = engine
        .compile(b"{__name__=~\"m|n\"}", EngineOptions::default(), &control)
        .unwrap();
    let report = query.collect_reported(&source, request(), &control);
    assert!(matches!(
        report.result,
        Err(ExecutionError::Evaluation(
            EvaluationError::Interrupted(_)
                | EvaluationError::Storage(StorageError::Interrupted(_))
        ))
    ));
    assert!(report.annotations.is_empty());
    assert!(
        report
            .observations
            .memory
            .iter()
            .all(|kind| kind.current == 0)
    );
    source.assert_closed();
}
