// SPDX-License-Identifier: GPL-3.0-or-later
//! AST-site selections and retained metadata; raw values remain cursor/page scoped.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    annotations::AnnotationBuilder,
    evaluator::{EvaluationError, EvaluationOptions},
    frontend::ast::{Expr, Matcher, NodeId, ParsedExpression},
    query_bounds,
    query_preparation::Prepared,
    storage::{
        Control, Interrupted, QueryProvider, ResourceOwner, SelectionRequest, Series,
        SeriesSelection, StorageError, matches_checked, validate_labels_checked,
    },
};
use std::cell::RefCell;

type SelectedSeries<'q, Q> = <<Q as QueryProvider>::Selection<'q> as SeriesSelection>::Series;
struct Site<'q, Q: QueryProvider + 'q> {
    // Retained descendants release before the provider selection and query.
    series: Option<AccountedVec<ResourceOwner<SelectedSeries<'q, Q>>>>,
    selection: Option<ResourceOwner<Q::Selection<'q>>>,
    failure: Option<AccountedVec<u8>>,
}
pub(crate) struct Catalog<'q, Q: QueryProvider + 'q> {
    expression: &'q ParsedExpression,
    query: &'q Q,
    sites: RefCell<AccountedVec<Option<Site<'q, Q>>>>,
    info_sites: RefCell<AccountedVec<Site<'q, Q>>>,
    pub(crate) count_windows: crate::count_windows::Registry<'q>,
    pub(crate) derived_progress: crate::derived_progress::Registry<'q>,
    computed_groups: RefCell<AccountedVec<(NodeId, Option<crate::computed_groups::Store<'q>>)>>,
}
#[derive(Clone, Copy)]
pub(crate) struct InfoSite(usize);
#[derive(Clone, Copy)]
enum ExpansionMode {
    Ordinary,
    Info,
}
pub(crate) struct ExpansionFailure {
    pub(crate) error: EvaluationError,
    pub(crate) annotations: AnnotationBuilder,
}

impl<'q, Q: QueryProvider + 'q> Catalog<'q, Q> {
    pub(crate) fn new(
        query: &'q Q,
        expression: &'q ParsedExpression,
        prepared: &Prepared,
        options: EvaluationOptions,
        no_step_interval_ms: i64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, EvaluationError> {
        let requests = query_bounds::selection_sites(
            expression,
            options,
            prepared,
            no_step_interval_ms,
            control,
            account,
        )?;
        let mut sites = AccountedVec::new(account, MemoryKind::Workspace);
        for _ in &expression.arena.nodes {
            control.check()?;
            account.work(1);
            sites.push(None);
        }
        for request in requests {
            control.check()?;
            account.provider_call();
            let Expr::Selector(selector) = &expression.arena[request.node].expression else {
                unreachable!("selection site")
            };
            let selection = query
                .select(
                    SelectionRequest {
                        matchers: &selector.matchers,
                        bounds: request.bounds,
                        hints: request.hints,
                        accounting: Some(account),
                    },
                    control,
                )
                .map_err(|error| error.accounted(account));
            let (selection, failure) = match selection {
                Ok(value) => {
                    let resources = value.resources();
                    (Some(ResourceOwner::new(value, resources, account)), None)
                }
                Err(StorageError::Provider(bytes)) => (None, Some(bytes)),
                Err(error) => return Err(error.into()),
            };
            sites[request.node.0] = Some(Site {
                series: None,
                selection,
                failure,
            });
        }
        Ok(Self {
            expression,
            query,
            sites: RefCell::new(sites),
            info_sites: RefCell::new(AccountedVec::new(account, MemoryKind::Workspace)),
            derived_progress: crate::derived_progress::Registry::new(account),
            count_windows: crate::count_windows::Registry::new(account),
            computed_groups: RefCell::new(AccountedVec::new(account, MemoryKind::Workspace)),
        })
    }

    pub(crate) fn add_computed_group(
        &self,
        node: NodeId,
        store: crate::computed_groups::Store<'q>,
    ) {
        self.computed_groups.borrow_mut().push((node, Some(store)));
    }

    pub(crate) fn with_computed_group<R>(
        &self,
        node: NodeId,
        control: &Control,
        account: &QueryAccounting,
        operation: impl FnOnce(&mut crate::computed_groups::Store<'q>) -> Result<R, StorageError>,
    ) -> Result<Option<R>, StorageError> {
        let (position, mut store) = {
            let mut entries = self.computed_groups.borrow_mut();
            let mut found = None;
            for (position, (id, store)) in entries.iter_mut().enumerate() {
                control.check()?;
                account.work(1);
                if *id == node {
                    found = store.take().map(|store| (position, store));
                    break;
                }
            }
            let Some(found) = found else { return Ok(None) };
            found
        };
        // Neither recursive evaluation nor provider callbacks hold the registry borrow.
        let result = operation(&mut store);
        self.computed_groups.borrow_mut()[position].1 = Some(store);
        result.map(Some)
    }

    pub(crate) fn discard_computed_group(&self, node: NodeId) {
        let value = {
            let mut entries = self.computed_groups.borrow_mut();
            entries
                .iter()
                .position(|(id, _)| *id == node)
                .map(|position| entries.remove(position))
        };
        drop(value);
    }

    pub(crate) fn start_final_groups(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        #[cfg(test)]
        crate::evaluator::aggregate_final_phase();
        for (_, store) in self.computed_groups.borrow_mut().iter_mut() {
            control.check()?;
            account.work(1);
            if let Some(store) = store {
                store.start_final();
            }
        }
        self.count_windows.start_final();
        self.derived_progress.start_final(control, account)
    }

    pub(crate) fn expand(
        &self,
        node: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<AnnotationBuilder, ExpansionFailure> {
        let mut sites = self.sites.borrow_mut();
        let site = sites[node.0].as_mut().expect("upfront selector site");
        let Expr::Selector(selector) = &self.expression.arena[node].expression else {
            unreachable!("upfront selector site")
        };
        self.expand_site(
            site,
            ExpansionMode::Ordinary,
            Some(&selector.matchers),
            control,
            account,
        )
    }

    pub(crate) fn select_info(
        &self,
        request: SelectionRequest<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(InfoSite, AnnotationBuilder), EvaluationError> {
        control.check()?;
        account.provider_call();
        let selection = self
            .query
            .select(request, control)
            .map_err(|error| error.accounted(account))?;
        let resources = selection.resources();
        let mut site = Site {
            series: None,
            selection: Some(ResourceOwner::new(selection, resources, account)),
            failure: None,
        };
        let annotations = self
            .expand_site(
                &mut site,
                ExpansionMode::Info,
                Some(request.matchers),
                control,
                account,
            )
            .map_err(|failure| failure.error)?;
        let mut sites = self.info_sites.borrow_mut();
        let id = InfoSite(sites.len());
        sites.push(site);
        Ok((id, annotations))
    }

    fn expand_site(
        &self,
        site: &mut Site<'q, Q>,
        mode: ExpansionMode,
        matchers: Option<&[Option<Matcher>]>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<AnnotationBuilder, ExpansionFailure> {
        let mut annotations = AnnotationBuilder::new(account);
        if site
            .series
            .as_ref()
            .is_some_and(|series| !series.is_empty())
        {
            return Ok(annotations);
        }
        if let Some(message) = site.failure.take() {
            return Err(ExpansionFailure {
                error: expansion_error(message, mode, account),
                annotations,
            });
        }
        let selection = site.selection.as_mut().expect("live selector site");
        let mut series = AccountedVec::new(account, MemoryKind::Workspace);
        let mut failure = None;
        loop {
            if let Err(error) = control.check() {
                return Err(ExpansionFailure {
                    error: error.into(),
                    annotations,
                });
            }
            account.provider_call();
            let result = selection
                .value
                .next(control)
                .map_err(|error| error.accounted(account));
            selection.lease.update(selection.value.resources());
            match result {
                Ok(Some(value)) => {
                    let resources = value.resources();
                    let value = ResourceOwner::new(value, resources, account);
                    if let Err(error) = control.check() {
                        return Err(ExpansionFailure {
                            error: error.into(),
                            annotations,
                        });
                    }
                    let result = value
                        .value
                        .label_views_checked(account, control)
                        .map_err(StorageError::from)
                        .map_err(EvaluationError::from)
                        .and_then(|labels| {
                            validate_labels_checked(&labels, account, control)?;
                            if let Some(matchers) = matchers {
                                matches_checked(&labels, matchers, Some(account), control)
                                    .map_err(EvaluationError::from)
                            } else {
                                Ok(true)
                            }
                        });
                    match result {
                        Ok(false) => continue,
                        Ok(true) => (),
                        Err(error) => return Err(ExpansionFailure { error, annotations }),
                    }
                    series.push(value);
                }
                Ok(None) => break,
                Err(StorageError::Provider(bytes)) => {
                    failure = Some(bytes);
                    break;
                }
                Err(error) => {
                    return Err(ExpansionFailure {
                        error: error.into(),
                        annotations,
                    });
                }
            }
        }
        if failure.is_none()
            && let Err(error) = control.check()
        {
            return Err(ExpansionFailure {
                error: error.into(),
                annotations,
            });
        }
        account.provider_call();
        let count = selection.value.diagnostic_count();
        selection.lease.update(selection.value.resources());
        for index in 0..count {
            if failure.is_none()
                && let Err(error) = control.check()
            {
                return Err(ExpansionFailure {
                    error: error.into(),
                    annotations,
                });
            }
            account.work(1);
            account.provider_call();
            let result = selection.value.diagnostic(index, |diagnostic| {
                let _view = account.hold(
                    MemoryKind::BorrowedView,
                    (size_of_val(&diagnostic) + diagnostic.message.len()) as u64,
                );
                annotations.add_provider(
                    diagnostic,
                    &self.expression.source,
                    failure.is_none().then_some(control),
                    account,
                )
            });
            selection.lease.update(selection.value.resources());
            if let Err(error) = result {
                return Err(ExpansionFailure {
                    error: error.into(),
                    annotations: AnnotationBuilder::new(account),
                });
            }
        }
        if let Some(message) = failure {
            return Err(ExpansionFailure {
                error: expansion_error(message, mode, account),
                annotations,
            });
        }
        site.series = Some(series);
        Ok(annotations)
    }

    pub(crate) fn len(&self, node: NodeId) -> usize {
        self.sites.borrow()[node.0]
            .as_ref()
            .expect("selector site")
            .series
            .as_ref()
            .expect("expanded selector site")
            .len()
    }
    /// Backing factories do not borrow selector tables across a nested replay.
    pub(crate) fn query(&self) -> &'q Q {
        self.query
    }
    pub(crate) fn with_series<R>(
        &self,
        node: NodeId,
        ordinal: usize,
        visit: impl for<'s> FnOnce(&'s SelectedSeries<'q, Q>) -> R,
    ) -> Option<R> {
        let sites = self.sites.borrow();
        let series = sites[node.0]
            .as_ref()
            .expect("selector site")
            .series
            .as_ref()
            .expect("expanded selector site");
        series.get(ordinal).map(|series| visit(&series.value))
    }

    pub(crate) fn info_len(&self, site: InfoSite) -> usize {
        self.info_sites.borrow()[site.0]
            .series
            .as_ref()
            .expect("expanded info site")
            .len()
    }

    pub(crate) fn with_info_series<R>(
        &self,
        site: InfoSite,
        ordinal: usize,
        visit: impl for<'s> FnOnce(&'s SelectedSeries<'q, Q>) -> R,
    ) -> Option<R> {
        let sites = self.info_sites.borrow();
        sites[site.0]
            .series
            .as_ref()
            .expect("expanded info site")
            .get(ordinal)
            .map(|series| visit(&series.value))
    }
}

fn expansion_error(
    message: AccountedVec<u8>,
    mode: ExpansionMode,
    account: &QueryAccounting,
) -> EvaluationError {
    if matches!(mode, ExpansionMode::Info) {
        return StorageError::Provider(message).into();
    }
    let mut output = AccountedVec::new(account, MemoryKind::Result);
    output.extend_from_slice(b"expanding series: ");
    for bytes in message.chunks(1024) {
        // Formatting an already returned failure cannot replace it with later cancellation.
        account.work(bytes.len() as u64);
        output.extend_from_slice(bytes);
    }
    EvaluationError::Expansion(output)
}

#[cfg(test)]
mod tests {
    use crate::{
        accounting::MemoryKind,
        evaluator::EvaluationError,
        execution::{Engine, EngineOptions, ExecutionError, QueryRequest},
        result::OwnedValue,
        storage::{Control, Interrupted, StorageError},
        test_storage::{FixtureProvider, observe_label_reads},
    };
    use std::sync::atomic::Ordering;

    fn provider(count: usize, width: usize, page: usize) -> FixtureProvider {
        let mut provider = FixtureProvider::new(3, page, count);
        for series in &mut provider.series {
            for index in series.labels.len()..width {
                series
                    .labels
                    .push((format!("z{index:03}").into_bytes(), b"padding".to_vec()));
            }
            assert_eq!(series.labels.len(), width);
        }
        provider
    }

    #[test]
    fn validated_views_serve_selected_and_rejected_series_without_a_second_walk() {
        let mut guards = Vec::new();
        let mut complete = 0;
        for page in [1, 17, 1024] {
            for delayed in [false, true] {
                for count in [1, 64] {
                    for width in [3, 32] {
                        for range in [false, true] {
                            for (source, selected) in [
                                ("counter_total{instance=\"absent\"}", 0),
                                ("counter_total{instance=\"0\"}", 1),
                                ("counter_total{instance=~\"0|1\"}", count.min(2)),
                            ] {
                                let provider = provider(count, width, page);
                                let engine = Engine::default();
                                let control = Control::default();
                                let query = engine
                                    .compile(
                                        source.as_bytes(),
                                        EngineOptions {
                                            delayed_name_removal: delayed,
                                            ..EngineOptions::default()
                                        },
                                        &control,
                                    )
                                    .unwrap();
                                let request = if range {
                                    QueryRequest::Range {
                                        start_ms: 1000,
                                        end_ms: 3000,
                                        step_ms: 1000,
                                    }
                                } else {
                                    QueryRequest::Instant { time_ms: 3000 }
                                };
                                let (report, reads) = observe_label_reads(&control, None, || {
                                    query.collect_reported(&provider, request, &control)
                                });
                                let result = report.result.as_ref().unwrap();
                                assert!(report.annotations.is_empty());
                                assert!(result.scalar.is_none());
                                assert_eq!(result.series.len(), selected);
                                let times = if range {
                                    &[1000_i64, 2000, 3000][..]
                                } else {
                                    &[3000_i64][..]
                                };
                                for (index, row) in result.series.iter().enumerate() {
                                    let expected = &provider.series[index].labels;
                                    assert_eq!(row.labels.0.len(), expected.len());
                                    for (actual, (name, value)) in row.labels.0.iter().zip(expected)
                                    {
                                        assert_eq!(&*actual.name, name.as_slice());
                                        assert_eq!(&*actual.value, value.as_slice());
                                    }
                                    assert_eq!(row.points.len(), times.len());
                                    for (point, &time_ms) in row.points.iter().zip(times) {
                                        assert_eq!(point.time_ms, time_ms);
                                        let OwnedValue::Float(value) = &point.value else {
                                            panic!("expected complete float result");
                                        };
                                        assert_eq!(
                                            value.to_bits(),
                                            (time_ms as f64 / 1000.0).to_bits()
                                        );
                                    }
                                }
                                assert_eq!(
                                    report.observations.output_samples,
                                    (selected * times.len()) as u64
                                );
                                assert_eq!(
                                    provider.counts.series_opened.load(Ordering::Relaxed),
                                    count
                                );
                                if selected == 0 {
                                    assert_eq!(report.observations.samples_fetched, 0);
                                    assert_eq!(
                                        provider.series[0]
                                            .source
                                            .counters
                                            .opened
                                            .load(Ordering::Relaxed),
                                        0
                                    );
                                    guards.push((page, delayed, count, width, range, reads));
                                }
                                let workspace =
                                    report.observations.memory[MemoryKind::Workspace as usize];
                                println!(
                                    "catalog outcome page={page} delayed={delayed} count={count} width={width} range={range} selected={selected} reads={reads} work={} acquired={} released={} peak={}",
                                    report.observations.work,
                                    workspace.acquired,
                                    workspace.released,
                                    workspace.peak
                                );
                                provider.assert_closed();
                                drop((report, query));
                                engine.accounting.snapshot().assert_memory_released();
                                complete += 1;
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(complete, 144);
        assert_eq!(guards.len(), 48);
        println!("catalog complete144-values-labels-times-annotations-physical-reads-release");
        for (page, delayed, count, width, range, reads) in guards {
            assert_eq!(
                reads,
                count * width,
                "duplicate catalog label walk: page={page} delayed={delayed} count={count} width={width} range={range}"
            );
        }
    }

    #[test]
    fn validation_errors_and_label_cancellation_precede_filtering_and_release() {
        for page in [1, 17, 1024] {
            for (malformed, expected) in [
                (0, "empty label name"),
                (1, "label names are not unique and sorted"),
                (2, "label names are not unique and sorted"),
            ] {
                let mut provider = provider(1, 32, page);
                provider.series[0].source.fail_page = Some(0);
                match malformed {
                    0 => provider.series[0].labels[0].0.clear(),
                    1 => {
                        let duplicate = provider.series[0].labels[0].clone();
                        provider.series[0].labels.push(duplicate);
                    }
                    _ => provider.series[0].labels.swap(0, 1),
                }
                let engine = Engine::default();
                let control = Control::default();
                let query = engine
                    .compile(
                        b"counter_total{instance=\"absent\"}",
                        EngineOptions::default(),
                        &control,
                    )
                    .unwrap();
                let report = query.collect_reported(
                    &provider,
                    QueryRequest::Instant { time_ms: 3000 },
                    &control,
                );
                assert!(matches!(
                    &report.result,
                    Err(ExecutionError::Evaluation(EvaluationError::Storage(StorageError::Invalid(message))))
                        if *message == expected
                ));
                assert_eq!(
                    provider.series[0]
                        .source
                        .counters
                        .opened
                        .load(Ordering::Relaxed),
                    0
                );
                assert_eq!(report.observations.samples_fetched, 0);
                provider.assert_closed();
                drop((report, query));
                engine.accounting.snapshot().assert_memory_released();
            }
            for cancel_at in [1, 16, 32] {
                let provider = provider(1, 32, page);
                let engine = Engine::default();
                let control = Control::default();
                let query = engine
                    .compile(
                        b"counter_total{instance=\"absent\"}",
                        EngineOptions::default(),
                        &control,
                    )
                    .unwrap();
                let (report, reads) = observe_label_reads(&control, Some(cancel_at), || {
                    query.collect_reported(
                        &provider,
                        QueryRequest::Instant { time_ms: 3000 },
                        &control,
                    )
                });
                assert!(matches!(
                    &report.result,
                    Err(ExecutionError::Evaluation(EvaluationError::Storage(
                        StorageError::Interrupted(Interrupted::Cancelled)
                    )))
                ));
                assert_eq!(reads, cancel_at);
                assert!(report.annotations.is_empty());
                assert_eq!(report.observations.samples_fetched, 0);
                assert_eq!(
                    provider.series[0]
                        .source
                        .counters
                        .opened
                        .load(Ordering::Relaxed),
                    0
                );
                provider.assert_closed();
                drop((report, query));
                engine.accounting.snapshot().assert_memory_released();
            }
        }
        println!("catalog validation complete9-invalid-nonmatches-and9-cancelled-walks");
        for page in [1, 17, 1024] {
            let mut provider = provider(1, 3, page);
            provider.series[0].labels[1].1 = vec![b'a'; 65_536];
            provider.series[0].source.fail_page = Some(0);
            let source = format!("counter_total{{instance=\"{}\"}}", "a".repeat(65_536));
            let engine = Engine::default();
            let control = Control::default();
            let query = engine
                .compile(source.as_bytes(), EngineOptions::default(), &control)
                .unwrap();
            let account = query.execution_accounting();
            let report =
                crate::test_storage::interrupt_after_work(&account, &control, 8192, || {
                    query.collect_reported_in(
                        &provider,
                        QueryRequest::Instant { time_ms: 3000 },
                        &control,
                        &account,
                    )
                });
            assert!(matches!(
                &report.result,
                Err(ExecutionError::Evaluation(EvaluationError::Interrupted(
                    Interrupted::Cancelled
                )))
            ));
            assert!(control.cancellation.cancelled());
            assert!((8192..9216).contains(&report.observations.work));
            assert_eq!(report.observations.samples_fetched, 0);
            assert_eq!(
                provider.series[0]
                    .source
                    .counters
                    .opened
                    .load(Ordering::Relaxed),
                0
            );
            provider.assert_closed();
            drop((report, query));
            account.snapshot().assert_memory_released();
            drop(account);
            engine.accounting.snapshot().assert_memory_released();
        }
        println!("catalog matcher interruption complete3-direct-errors-after-validation");
    }
}
