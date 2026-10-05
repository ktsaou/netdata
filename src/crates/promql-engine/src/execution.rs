// SPDX-License-Identifier: GPL-3.0-or-later
//! Synchronous query ownership, incremental output and ordinary-panic containment.
use crate::{
    accounting::{
        AccountedVec, AllocationContext, EngineAccounting, MemoryKind, MemoryLease,
        QueryAccounting, QueryObservations,
    },
    annotations::{AnnotationBuilder, Annotations},
    evaluator::{
        EvaluationError, EvaluationMetadata, EvaluationOptions, Machine, metadata, remember_unique,
    },
    evaluator_roster::{Descriptor, Rosters},
    frontend::{
        ast::{
            Expr, NodeId, ParseError, ParsedExpression, ParserOptions, ValueType, VectorMatching,
        },
        parser,
        regex::Selector,
        token as tokens,
    },
    range_reduce::RangeFunction,
    result::{CollectedResult, Collector, OutputChunk, OutputHeader, OutputSink},
    storage::{
        Control, Interrupted, Provider, QueryProvider, ResourceOwner, SelectionRequest, Series,
        SeriesSelection, StorageError, TimeBounds, matches_in, scan_with, validate_labels,
    },
    storage_observed::ObservedQuery,
    value::SampleRef,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use tokens::TokenKind;

#[derive(Clone, Copy, Debug)]
pub struct EngineOptions {
    pub parser: ParserOptions,
    pub lookback_ms: i64,
    pub start_timestamps: bool,
    pub delayed_name_removal: bool,
}
impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            parser: ParserOptions::default(),
            lookback_ms: 300_000,
            start_timestamps: false,
            delayed_name_removal: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Engine {
    pub accounting: EngineAccounting,
}

#[derive(Debug)]
pub enum CompileError {
    Interrupted(crate::storage::Interrupted),
    Parse(Vec<ParseError>),
    Internal,
}

#[derive(Debug)]
pub enum ExecutionError {
    Evaluation(EvaluationError),
    Sink(Vec<u8>),
    Internal,
}
impl From<EvaluationError> for ExecutionError {
    fn from(value: EvaluationError) -> Self {
        Self::Evaluation(value)
    }
}
impl From<StorageError> for ExecutionError {
    fn from(value: StorageError) -> Self {
        EvaluationError::from(value).into()
    }
}
impl From<Interrupted> for ExecutionError {
    fn from(value: Interrupted) -> Self {
        EvaluationError::from(value).into()
    }
}

#[derive(Debug)]
pub struct ExecutionReport<T> {
    pub result: Result<T, ExecutionError>,
    pub observations: QueryObservations,
    pub annotations: Annotations,
}

#[derive(Clone, Copy, Debug)]
pub enum QueryRequest {
    Instant {
        time_ms: i64,
    },
    Range {
        start_ms: i64,
        end_ms: i64,
        step_ms: i64,
    },
}

pub struct CompiledQuery {
    expression: ParsedExpression,
    options: EngineOptions,
    engine: EngineAccounting,
    _allocation: MemoryLease,
    accounting: QueryAccounting,
}

impl Engine {
    pub fn compile(
        &self,
        source: &[u8],
        mut options: EngineOptions,
        control: &Control,
    ) -> Result<CompiledQuery, CompileError> {
        control.check().map_err(CompileError::Interrupted)?;
        if options.lookback_ms == 0 {
            options.lookback_ms = EngineOptions::default().lookback_ms;
        }
        catch_unwind(AssertUnwindSafe(|| {
            let account = self.accounting.query();
            let context = AllocationContext::new(&account, MemoryKind::Compiled);
            let output = parser::parse_in(
                source,
                options.parser,
                |pattern| Selector::compile_in(pattern, Some(&context)),
                Some(&account),
            );
            control.check().map_err(CompileError::Interrupted)?;
            if !output.errors.is_empty() {
                return Err(CompileError::Parse(output.errors));
            }
            let expression = ParsedExpression {
                source: source.into(),
                arena: output.arena,
                root: output.root.expect("valid expression has a root"),
            };
            let allocation = account.hold(MemoryKind::Compiled, compiled_bytes(&expression) as u64);
            Ok(CompiledQuery {
                expression,
                options,
                engine: self.accounting.clone(),
                _allocation: allocation,
                accounting: account,
            })
        }))
        .unwrap_or(Err(CompileError::Internal))
    }
}

impl CompiledQuery {
    pub fn source(&self) -> &[u8] {
        &self.expression.source
    }
    pub fn value_type(&self) -> ValueType {
        self.expression.arena[self.expression.root].value_type
    }
    pub fn compiled_buffer_bytes(&self) -> u64 {
        self.accounting.snapshot().memory[MemoryKind::Compiled as usize].current
    }
    pub fn execute<P: Provider, S: OutputSink>(
        &self,
        provider: &P,
        request: QueryRequest,
        control: &Control,
        sink: &mut S,
    ) -> Result<QueryObservations, ExecutionError> {
        let report = self.execute_reported(provider, request, control, sink);
        report.result.map(|()| report.observations)
    }
    pub fn execute_reported<P: Provider, S: OutputSink>(
        &self,
        provider: &P,
        request: QueryRequest,
        control: &Control,
        sink: &mut S,
    ) -> ExecutionReport<()> {
        let account = self.engine.query();
        let mut annotations = Annotations::default();
        let result = self
            .execute_accounted(provider, request, control, sink, &account, &mut annotations)
            .map(|_| ());
        if matches!(&result, Err(ExecutionError::Internal)) {
            annotations = Annotations::default();
        }
        ExecutionReport {
            result,
            observations: account.snapshot(),
            annotations,
        }
    }
    pub fn collect<P: Provider>(
        &self,
        provider: &P,
        request: QueryRequest,
        control: &Control,
    ) -> Result<(CollectedResult, QueryObservations), ExecutionError> {
        let report = self.collect_reported(provider, request, control);
        report.result.map(|result| (result, report.observations))
    }
    pub fn collect_reported<P: Provider>(
        &self,
        provider: &P,
        request: QueryRequest,
        control: &Control,
    ) -> ExecutionReport<CollectedResult> {
        let account = self.engine.query();
        let mut collector = Collector::new(&account);
        let mut annotations = Annotations::default();
        let result = self
            .execute_accounted(
                provider,
                request,
                control,
                &mut collector,
                &account,
                &mut annotations,
            )
            .map(|_| {
                collector
                    .take()
                    .expect("successful collector commits its result")
            });
        if matches!(&result, Err(ExecutionError::Internal)) {
            annotations = Annotations::default();
        }
        ExecutionReport {
            result,
            observations: account.snapshot(),
            annotations,
        }
    }
    fn execute_accounted<P: Provider, S: OutputSink>(
        &self,
        provider: &P,
        request: QueryRequest,
        control: &Control,
        sink: &mut S,
        account: &QueryAccounting,
        annotations: &mut Annotations,
    ) -> Result<QueryObservations, ExecutionError> {
        let observation = account.execution();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let header = self.header(request)?;
            control.check().map_err(EvaluationError::from)?;
            sink.begin(header).map_err(ExecutionError::Sink)?;
            {
                account.provider_call();
                let query = provider.open(control).map_err(EvaluationError::from)?;
                let query = ObservedQuery::new(query, account);
                self.evaluate(&query, header, control, account, sink, annotations)?;
            }
            control.check().map_err(EvaluationError::from)?;
            if !annotations.is_empty() {
                sink.chunk(OutputChunk::Annotations(annotations))
                    .map_err(ExecutionError::Sink)?;
            }
            let mut observations = account.snapshot();
            observations.elapsed = observations.elapsed.saturating_add(observation.elapsed());
            sink.commit(&observations).map_err(ExecutionError::Sink)?;
            Ok(())
        }))
        .unwrap_or(Err(ExecutionError::Internal));
        if outcome.is_err() && catch_unwind(AssertUnwindSafe(|| sink.abort())).is_err() {
            return Err(ExecutionError::Internal);
        }
        drop(observation);
        outcome.map(|()| account.snapshot())
    }
    fn header(&self, request: QueryRequest) -> Result<OutputHeader, EvaluationError> {
        let value_type = self.expression.arena[self.expression.root].value_type;
        match request {
            QueryRequest::Instant { time_ms } => Ok(OutputHeader {
                value_type,
                start_ms: time_ms,
                end_ms: time_ms,
                step_ms: 0,
            }),
            QueryRequest::Range {
                start_ms,
                end_ms,
                step_ms,
            } => {
                if step_ms <= 0 || end_ms < start_ms {
                    return Err(EvaluationError::Invalid("invalid range query times"));
                }
                if !matches!(value_type, ValueType::Vector | ValueType::Scalar) {
                    return Err(EvaluationError::Invalid(
                        "range expression must be a scalar or instant vector",
                    ));
                }
                Ok(OutputHeader {
                    value_type: ValueType::Matrix,
                    start_ms,
                    end_ms,
                    step_ms,
                })
            }
        }
    }
    fn evaluate<Q: QueryProvider, S: OutputSink>(
        &self,
        query: &Q,
        header: OutputHeader,
        control: &Control,
        account: &QueryAccounting,
        sink: &mut S,
        annotations: &mut Annotations,
    ) -> Result<(), ExecutionError> {
        let mut root = self.expression.root;
        while let Expr::Paren(child) = self.expression.arena[root].expression {
            root = child;
        }
        if let Expr::String(value) = &self.expression.arena[root].expression {
            sink.chunk(OutputChunk::String {
                time_ms: header.start_ms,
                value,
            })
            .map_err(ExecutionError::Sink)?;
            account.output(1);
            return Ok(());
        }
        let root_type = self.expression.arena[self.expression.root].value_type;
        if root_type == ValueType::Matrix {
            return self.evaluate_matrix(query, header, control, account, sink);
        }
        let mut rosters = self.prepare_rosters(query, header, control, account)?;
        *annotations = rosters
            .take_annotations(self.expression.root)
            .expect("completed root diagnostics")
            .finish();
        let merge_root = self.options.delayed_name_removal
            || matches!(
                self.expression.arena[root].expression,
                Expr::Unary {
                    operator: TokenKind::SUB,
                    ..
                }
            );
        let mut identities = AccountedVec::<(u64, usize)>::new(account, MemoryKind::Workspace);
        let mut time_ms = header.start_ms;
        loop {
            let options = EvaluationOptions {
                time_ms,
                start_ms: header.start_ms,
                end_ms: header.end_ms,
                step_ms: header.step_ms,
                lookback_ms: self.options.lookback_ms,
                start_timestamps: self.options.start_timestamps,
                delayed_name_removal: self.options.delayed_name_removal,
            };
            let mut machine = Machine::new(
                query,
                &self.expression,
                self.expression.root,
                options,
                control,
                account,
                EvaluationMetadata {
                    rosters: &rosters,
                    annotations: None,
                },
            );
            let mut seen = AccountedVec::<u64>::new(account, MemoryKind::Workspace);
            while let Some(mut sample) = machine.next()? {
                control.check().map_err(EvaluationError::from)?;
                if sample.drop_name && self.options.delayed_name_removal {
                    sample.labels.retain(|name| !metadata(name));
                }
                if self.options.delayed_name_removal {
                    remember_unique(&mut seen, &sample.labels, account, control)?;
                }
                let labels = sample.labels.views(account);
                let series_id = if merge_root {
                    let hash = crate::label_hash::full(&sample.labels, control, account)?;
                    if let Some((_, id)) = identities.iter().find(|(key, _)| *key == hash) {
                        *id
                    } else {
                        let id = identities.len();
                        identities.push((hash, id));
                        id
                    }
                } else {
                    sample.ordinal
                };
                let point = SampleRef {
                    time_ms,
                    start_time_ms: 0,
                    value: sample.value.as_ref(),
                };
                if root_type == ValueType::Scalar && header.step_ms == 0 {
                    let crate::result::OwnedValue::Float(value) = sample.value else {
                        unreachable!("scalar output")
                    };
                    sink.chunk(OutputChunk::Scalar { time_ms, value })
                        .map_err(ExecutionError::Sink)?;
                } else {
                    sink.chunk(OutputChunk::Series {
                        series_id,
                        labels: &labels,
                        points: &[point],
                    })
                    .map_err(ExecutionError::Sink)?;
                }
                account.output(1);
            }
            if header.step_ms == 0 {
                break;
            }
            let Some(next) = time_ms.checked_add(header.step_ms) else {
                break;
            };
            if next > header.end_ms {
                break;
            }
            time_ms = next;
        }
        Ok(())
    }
    fn evaluation_options(&self, header: OutputHeader, time_ms: i64) -> EvaluationOptions {
        EvaluationOptions {
            time_ms,
            start_ms: header.start_ms,
            end_ms: header.end_ms,
            step_ms: header.step_ms,
            lookback_ms: self.options.lookback_ms,
            start_timestamps: self.options.start_timestamps,
            delayed_name_removal: self.options.delayed_name_removal,
        }
    }

    fn fixed_range_call(&self, expression: &Expr) -> bool {
        let Expr::Call {
            function: Some(function),
            arguments,
        } = expression
        else {
            return false;
        };
        if RangeFunction::parse(function.name).is_none() {
            return false;
        }
        let argument = self.expression.arena.unparenthesized(arguments[0]);
        let Expr::Matrix { selector, .. } = &self.expression.arena[argument].expression else {
            return false;
        };
        let Expr::Selector(selector) = &self.expression.arena[*selector].expression else {
            return false;
        };
        selector.modifiers.timestamp.is_some() || selector.modifiers.start_or_end.is_some()
    }

    fn prepare_rosters<Q: QueryProvider>(
        &self,
        query: &Q,
        header: OutputHeader,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Rosters, EvaluationError> {
        enum Task {
            Visit(NodeId),
            Finish(NodeId),
        }
        let mut rosters = Rosters::new(account, self.expression.arena.nodes.len());
        let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
        tasks.push(Task::Visit(self.expression.root));
        while let Some(task) = tasks.pop() {
            control.check()?;
            account.work(1);
            let node = match task {
                Task::Visit(node) | Task::Finish(node) => node,
            };
            let expression = &self.expression.arena[node].expression;
            if matches!(task, Task::Visit(_)) {
                tasks.push(Task::Finish(node));
                for child in children(expression).rev() {
                    tasks.push(Task::Visit(child));
                }
                continue;
            }
            if let Expr::Aggregate {
                expression: Some(child),
                grouping,
                without,
                ..
            } = expression
            {
                rosters.prepare_groups(
                    node,
                    *child,
                    grouping.as_deref(),
                    *without,
                    account,
                    control,
                )?;
            }
            // The upstream multi-step map ordering contract is awaiting decision 9.
            if header.start_ms != header.end_ms
                && self.expression.arena[node].value_type == ValueType::Vector
                && matches!(expression, Expr::Binary { .. })
            {
                return Err(EvaluationError::Unsupported(
                    "unordered intermediate matrix policy pending",
                ));
            }
            let mut annotations = AnnotationBuilder::new(account);
            for child in children(expression) {
                if let Some(child_annotations) = rosters.take_annotations(child) {
                    annotations.merge(child_annotations, &self.expression.source);
                }
            }
            let mut descriptors = AccountedVec::<Descriptor>::new(account, MemoryKind::Workspace);
            // Fixed safe range calls have one logical evaluation, irrespective of output steps.
            let single_step = header.step_ms == 0 || self.fixed_range_call(expression);
            let mut time_ms = header.start_ms;
            loop {
                let mut machine = Machine::new(
                    query,
                    &self.expression,
                    node,
                    self.evaluation_options(header, time_ms),
                    control,
                    account,
                    EvaluationMetadata {
                        rosters: &rosters,
                        annotations: Some(&mut annotations),
                    },
                );
                while let Some(sample) = machine.next()? {
                    if descriptors.iter().any(|descriptor| {
                        descriptor.ordinal == sample.ordinal
                            && descriptor.labels.same(&sample.labels)
                    }) {
                        continue;
                    }
                    descriptors.push(Descriptor {
                        ordinal: sample.ordinal,
                        labels: sample.labels,
                    });
                }
                if single_step {
                    break;
                }
                let Some(next) = time_ms.checked_add(header.step_ms) else {
                    break;
                };
                if next > header.end_ms {
                    break;
                }
                time_ms = next;
            }
            if !self.options.delayed_name_removal
                && matches!(expression,
                Expr::Call { function: Some(function), .. }
                if RangeFunction::parse(function.name).is_some())
            {
                let mut seen = AccountedVec::new(account, MemoryKind::Workspace);
                for descriptor in descriptors.iter() {
                    remember_unique(&mut seen, &descriptor.labels, account, control)?;
                }
            }
            rosters.output(node, descriptors);
            rosters.annotations(node, annotations);
            for child in children(expression) {
                rosters.release_output(child);
            }
        }
        Ok(rosters)
    }

    fn evaluate_matrix<Q: QueryProvider, S: OutputSink>(
        &self,
        query: &Q,
        header: OutputHeader,
        control: &Control,
        account: &QueryAccounting,
        sink: &mut S,
    ) -> Result<(), ExecutionError> {
        let mut root = self.expression.root;
        while let Expr::Paren(child) = self.expression.arena[root].expression {
            root = child;
        }
        let Expr::Matrix {
            selector,
            range,
            range_expression: None,
        } = self.expression.arena[root].expression
        else {
            return Err(EvaluationError::Unsupported("top-level subquery output").into());
        };
        let Expr::Selector(selector) = &self.expression.arena[selector].expression else {
            unreachable!("typed selector")
        };
        if selector.anchored
            || selector.smoothed
            || selector.modifiers.original_offset_expression.is_some()
        {
            return Err(EvaluationError::Unsupported("extended top-level matrix").into());
        }
        let timestamp =
            selector
                .modifiers
                .timestamp
                .unwrap_or(match selector.modifiers.start_or_end {
                    Some(TokenKind::START) => header.start_ms,
                    Some(TokenKind::END) => header.end_ms,
                    _ => header.start_ms,
                });
        let end_ms = timestamp.wrapping_sub(selector.modifiers.original_offset / 1_000_000);
        let bounds = TimeBounds {
            start_ms: end_ms.wrapping_sub(range / 1_000_000).wrapping_add(1),
            end_ms,
        };
        account.provider_call();
        let selection = query.select(
            SelectionRequest {
                matchers: &selector.matchers,
                bounds,
                accounting: Some(account),
            },
            control,
        )?;
        let resources = selection.resources();
        let mut selection = ResourceOwner::new(selection, resources, account);
        let mut series_id = 0;
        loop {
            control.check()?;
            account.provider_call();
            let series = selection.value.next(control);
            selection.lease.update(selection.value.resources());
            let series = series?;
            let Some(series) = series else {
                break;
            };
            let resources = series.resources();
            let current_series_id = series_id;
            series_id += 1;
            let series = ResourceOwner::new(series, resources, account);
            let labels = series.value.label_views(account);
            validate_labels(&labels)?;
            if !matches_in(&labels, &selector.matchers, Some(account)) {
                continue;
            }
            scan_with(
                &series.value,
                bounds,
                control,
                account,
                |sample| -> Result<(), ExecutionError> {
                    if sample.value.is_stale() {
                        return Ok(());
                    }
                    let value = crate::result::OwnedValue::copy_from(
                        sample.value,
                        account,
                        MemoryKind::Workspace,
                    );
                    let point = SampleRef {
                        time_ms: sample.time_ms,
                        start_time_ms: 0,
                        value: value.as_ref(),
                    };
                    sink.chunk(OutputChunk::Series {
                        series_id: current_series_id,
                        labels: &labels,
                        points: &[point],
                    })
                    .map_err(ExecutionError::Sink)?;
                    account.output(1);
                    Ok(())
                },
            )?;
        }
        Ok(())
    }
}

fn children(expression: &Expr) -> impl DoubleEndedIterator<Item = NodeId> + '_ {
    let (fixed, slice): ([Option<NodeId>; 2], &[NodeId]) = match expression {
        Expr::Paren(child)
        | Expr::Unary {
            expression: child, ..
        } => ([Some(*child), None], &[]),
        Expr::Binary { left, right, .. } => ([Some(*left), Some(*right)], &[]),
        Expr::Aggregate {
            parameter,
            expression,
            ..
        } => ([*parameter, *expression], &[]),
        Expr::Call {
            function: Some(function),
            ..
        } if RangeFunction::parse(function.name).is_some() => ([None, None], &[]),
        Expr::Call { arguments, .. } => ([None, None], arguments),
        _ => ([None, None], &[]),
    };
    fixed.into_iter().flatten().chain(slice.iter().copied())
}

fn vector_bytes(values: &Option<Vec<Vec<u8>>>) -> usize {
    values.as_ref().map_or(0, |values| {
        values.capacity() * size_of::<Vec<u8>>() + values.iter().map(Vec::capacity).sum::<usize>()
    })
}
fn matching_bytes(value: &Option<VectorMatching>) -> usize {
    value.as_ref().map_or(0, |value| {
        vector_bytes(&value.labels) + vector_bytes(&value.include)
    })
}
fn compiled_bytes(expression: &ParsedExpression) -> usize {
    let mut bytes = expression.source.len()
        + expression.arena.nodes.capacity() * size_of::<crate::frontend::ast::Node>();
    for node in &expression.arena.nodes {
        bytes += match &node.expression {
            Expr::String(value) => value.capacity(),
            Expr::Selector(value) => {
                value.name.capacity()
                    + value.matchers.capacity() * size_of::<Option<crate::frontend::ast::Matcher>>()
                    + value
                        .matchers
                        .iter()
                        .flatten()
                        .map(|matcher| matcher.name.capacity() + matcher.value.capacity())
                        .sum::<usize>()
            }
            Expr::Aggregate { grouping, .. } => vector_bytes(grouping),
            Expr::Binary { matching, .. } => matching_bytes(matching),
            Expr::Call { arguments, .. } => {
                arguments.capacity() * size_of::<crate::frontend::ast::NodeId>()
            }
            _ => 0,
        };
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{result::OwnedValue, test_storage::FixtureProvider};
    use std::sync::atomic::Ordering;

    #[test]
    fn failed_compilation_releases_prior_matchers_and_retains_regex_error_bytes() {
        let engine = Engine::default();
        let error = engine
            .compile(
                b"counter_total{good=~\"a[^0-9]+c\",bad=~\"(\"}",
                EngineOptions::default(),
                &Control::default(),
            )
            .err()
            .unwrap();
        let CompileError::Parse(errors) = &error else {
            panic!("expected parse error")
        };
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.starts_with(b"error parsing regexp:"));
        let observed = engine.accounting.snapshot();
        let compiled = observed.memory[MemoryKind::Compiled as usize];
        assert_eq!(compiled.current, errors[0].message.capacity() as u64);
        assert!(compiled.peak > compiled.current);
        let estimated = observed.memory[MemoryKind::RegexEstimate as usize];
        assert!(estimated.peak > 0);
        assert_eq!(estimated.current, 0);
        drop(error);
        assert!(
            engine
                .accounting
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }

    #[test]
    fn empty_selector_validation_observes_matcher_search_and_releases_the_plan() {
        let engine = Engine::default();
        assert!(matches!(
            engine.compile(
                b"{l=~\"a*\"}",
                EngineOptions::default(),
                &Control::default()
            ),
            Err(CompileError::Parse(_))
        ));
        let observed = engine.accounting.snapshot();
        assert!(observed.memory[MemoryKind::Workspace as usize].peak > 0);
        assert!(observed.memory[MemoryKind::RegexEstimate as usize].peak > 0);
        assert!(observed.memory.iter().all(|memory| memory.current == 0));
    }

    #[test]
    fn provider_and_core_regex_filtering_use_the_execution_owner() {
        for filter in [false, true] {
            let engine = Engine::default();
            let mut provider = FixtureProvider::new(1, 1, 4);
            provider.filter_selectors = filter;
            for (series, instance) in provider.series.iter_mut().zip([
                b"a\xffbc".as_slice(),
                b"a\xff9c",
                b"unrelated",
                b"abc",
            ]) {
                series.labels[1].1 = instance.to_vec();
            }
            let control = Control::default();
            let query = engine
                .compile(
                    b"counter_total{instance=~\"a[^0-9]+c\"}",
                    EngineOptions::default(),
                    &control,
                )
                .unwrap();
            let compiled = query.accounting.snapshot();
            let (result, observed) = query
                .collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control)
                .unwrap();
            assert_eq!(result.series.len(), 2);
            for series in &*result.series {
                assert_eq!(series.points.len(), 1);
                assert_eq!(float_bits(&series.points[0]), 1.0f64.to_bits());
            }
            assert_eq!(query.accounting.snapshot(), compiled);
            for kind in [
                MemoryKind::Workspace,
                MemoryKind::RegexEstimate,
                MemoryKind::ProviderHeld,
            ] {
                assert!(
                    observed.memory[kind as usize].peak > 0,
                    "{kind:?} filter {filter}"
                );
                assert_eq!(observed.memory[kind as usize].current, 0);
            }
            assert_eq!(observed.memory[MemoryKind::Compiled as usize].acquired, 0);
            provider.assert_closed();
            drop(query);
            drop(provider);
            let retained = engine.accounting.snapshot();
            assert_eq!(retained.memory[MemoryKind::Compiled as usize].current, 0);
            assert_eq!(
                retained.memory[MemoryKind::RegexEstimate as usize].current,
                0
            );
            assert!(retained.memory[MemoryKind::Result as usize].current > 0);
            drop(result);
            assert!(
                engine
                    .accounting
                    .snapshot()
                    .memory
                    .iter()
                    .all(|memory| memory.current == 0)
            );
        }
    }

    #[test]
    fn complete_grouped_rate_threshold_pages_and_owned_release() {
        for page_size in [1, 17, 1024] {
            let provider = FixtureProvider::new(10_000, page_size, 2);
            let engine = Engine::default();
            let control = Control::default();
            let query = engine
                .compile(
                    b"sum by(job)(rate(counter_total[10000s])) >= bool 2",
                    EngineOptions::default(),
                    &control,
                )
                .unwrap();
            let (result, observations) = query
                .collect(
                    &provider,
                    QueryRequest::Instant {
                        time_ms: 10_000_000,
                    },
                    &control,
                )
                .unwrap();
            provider.assert_closed();
            assert_eq!(
                provider.series[0]
                    .source
                    .counters
                    .peak
                    .load(Ordering::Relaxed),
                1
            );
            assert_eq!(observations.samples_fetched, 160_000);
            assert_eq!(
                observations.memory[MemoryKind::Workspace as usize].current,
                0
            );
            assert_eq!(
                observations.memory[MemoryKind::ProviderHeld as usize].current,
                0
            );
            assert_eq!(result.series.len(), 1);
            assert_eq!(result.series[0].labels.0.len(), 1);
            assert_eq!(&*result.series[0].labels.0[0].name, b"job");
            assert_eq!(&*result.series[0].labels.0[0].value, b"worker");
            assert!(matches!(
                result.series[0].points[0].value,
                OwnedValue::Float(0.0)
            ));
            drop(query);
            drop(provider);
            assert_eq!(
                engine.accounting.snapshot().memory[MemoryKind::Compiled as usize].current,
                0
            );
            assert!(engine.accounting.snapshot().memory[MemoryKind::Result as usize].current > 0);
            assert_eq!(&*result.series[0].labels.0[0].value, b"worker");
            drop(result);
            assert!(
                engine
                    .accounting
                    .snapshot()
                    .memory
                    .iter()
                    .all(|memory| memory.current == 0)
            );
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        chunks: usize,
        committed: bool,
        aborted: bool,
        fail_after: Option<usize>,
        cancel: Option<crate::storage::Cancellation>,
        panic_after: Option<usize>,
    }
    impl OutputSink for RecordingSink {
        fn begin(&mut self, _: OutputHeader) -> Result<(), Vec<u8>> {
            Ok(())
        }
        fn chunk(&mut self, _: OutputChunk<'_>) -> Result<(), Vec<u8>> {
            self.chunks += 1;
            assert_ne!(self.panic_after, Some(self.chunks), "injected output panic");
            if let Some(cancel) = &self.cancel
                && self.chunks == 2
            {
                cancel.cancel();
            }
            if self.fail_after == Some(self.chunks) {
                return Err(b"sink failure".to_vec());
            }
            Ok(())
        }
        fn commit(&mut self, _: &QueryObservations) -> Result<(), Vec<u8>> {
            self.committed = true;
            Ok(())
        }
        fn abort(&mut self) {
            self.committed = false;
            self.aborted = true;
        }
    }

    #[test]
    fn provisional_output_aborts_after_sink_failure_or_cancellation() {
        for cancel in [false, true] {
            let engine = Engine::default();
            let provider = FixtureProvider::new(10, 1, 1);
            let control = Control::default();
            let query = engine
                .compile(b"counter_total", EngineOptions::default(), &control)
                .unwrap();
            let mut sink = RecordingSink {
                fail_after: if cancel { None } else { Some(2) },
                cancel: cancel.then(|| control.cancellation.clone()),
                ..RecordingSink::default()
            };
            assert!(
                query
                    .execute(
                        &provider,
                        QueryRequest::Range {
                            start_ms: 1000,
                            end_ms: 10000,
                            step_ms: 1000
                        },
                        &control,
                        &mut sink
                    )
                    .is_err()
            );
            assert_eq!(sink.chunks, 2);
            assert!(!sink.committed);
            assert!(sink.aborted);
            provider.assert_closed();
            assert_eq!(engine.accounting.snapshot().active_executions, 0);
            assert_eq!(
                engine.accounting.snapshot().memory[MemoryKind::ProviderHeld as usize].current,
                0
            );
        }
    }

    #[test]
    fn panic_discards_context_and_later_execution_still_works() {
        fn sample(index: usize) -> crate::value::Sample {
            assert_ne!(index, 4, "injected query panic");
            crate::test_storage::counter(index)
        }
        let engine = Engine::default();
        let mut provider = FixtureProvider::new(10, 1, 1);
        provider.series[0].source.sample = sample;
        let control = Control::default();
        let query = engine
            .compile(b"counter_total", EngineOptions::default(), &control)
            .unwrap();
        let mut sink = RecordingSink::default();
        assert!(matches!(
            query.execute(
                &provider,
                QueryRequest::Range {
                    start_ms: 1000,
                    end_ms: 10000,
                    step_ms: 1000
                },
                &control,
                &mut sink
            ),
            Err(ExecutionError::Internal)
        ));
        assert!(sink.aborted && !sink.committed);
        provider.assert_closed();
        let (result, _) = query
            .collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control)
            .unwrap();
        assert_eq!(result.series.len(), 1);
        provider.assert_closed();
    }

    #[test]
    fn nested_evaluator_frames_use_a_small_caller_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut source = b"sum(".repeat(1000);
                source.extend_from_slice(b"counter_total");
                source.extend(std::iter::repeat_n(b')', 1000));
                let engine = Engine::default();
                let provider = FixtureProvider::new(1, 1, 1);
                let control = Control::default();
                let query = engine
                    .compile(&source, EngineOptions::default(), &control)
                    .unwrap();
                let (result, _) = query
                    .collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control)
                    .unwrap();
                assert!(matches!(
                    result.series[0].points[0].value,
                    OwnedValue::Float(1.0)
                ));
                provider.assert_closed();
            })
            .unwrap()
            .join()
            .unwrap();
    }

    fn second_value(index: usize) -> crate::value::Sample {
        let mut sample = crate::test_storage::counter(index);
        sample.value = crate::value::SampleValue::Float(2.0);
        sample
    }
    fn options(delayed: bool) -> EngineOptions {
        EngineOptions {
            delayed_name_removal: delayed,
            start_timestamps: delayed,
            ..EngineOptions::default()
        }
    }
    fn float_bits(point: &crate::result::OutputPoint) -> u64 {
        let OwnedValue::Float(value) = point.value else {
            panic!("expected float")
        };
        value.to_bits()
    }
    fn is_duplicate(error: &ExecutionError) -> bool {
        matches!(
            error,
            ExecutionError::Evaluation(EvaluationError::Invalid(
                "vector cannot contain metrics with the same labelset"
            ))
        )
    }
    #[test]
    fn empty_group_keys_remain_distinct_until_source_label_projection() {
        let mut provider = FixtureProvider::new(1, 1, 2);
        provider.series[0].labels = vec![
            (b"__name__".to_vec(), b"m".to_vec()),
            (b"job".to_vec(), vec![]),
        ];
        provider.series[1].labels = vec![(b"__name__".to_vec(), b"m".to_vec())];
        provider.series[1].source.sample = second_value;
        let engine = Engine::default();
        let control = Control::default();
        for delayed in [false, true] {
            for (expression, duplicate, bits) in [
                (
                    &b"sum by(job)(m)"[..],
                    delayed,
                    vec![1.0f64.to_bits(), 2.0f64.to_bits()],
                ),
                (&b"sum(sum by(job)(m))"[..], false, vec![3.0f64.to_bits()]),
                (&b"sum by(job)(m)+0"[..], true, vec![]),
                (&b"-sum by(job)(m)"[..], true, vec![]),
            ] {
                let query = engine
                    .compile(expression, options(delayed), &control)
                    .unwrap();
                let result =
                    query.collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control);
                if duplicate {
                    assert!(is_duplicate(&result.unwrap_err()));
                } else {
                    let (result, _) = result.unwrap();
                    assert_eq!(result.series.len(), bits.len());
                    assert_eq!(
                        result
                            .series
                            .iter()
                            .map(|row| float_bits(&row.points[0]))
                            .collect::<Vec<_>>(),
                        bits
                    );
                    assert!(result.series.iter().all(|row| row.labels.0.is_empty()));
                }
            }
        }
        provider.assert_closed();
    }
    #[test]
    fn child_duplicate_errors_cannot_be_hidden_by_a_fold_or_filter() {
        let mut provider = FixtureProvider::new(1, 1, 2);
        for (series, name) in provider.series.iter_mut().zip([b"a", b"b"]) {
            series.labels = vec![
                (b"__name__".to_vec(), name.to_vec()),
                (b"job".to_vec(), b"x".to_vec()),
            ];
        }
        provider.series[1].source.sample = second_value;
        let engine = Engine::default();
        let control = Control::default();
        for delayed in [false, true] {
            for (expression, output) in [
                (&b"sum({__name__=~\"a|b\",job=\"x\"}+0)"[..], Some(3.0)),
                (&b"sum(({__name__=~\"a|b\",job=\"x\"}+0)>100)"[..], None),
            ] {
                let query = engine
                    .compile(expression, options(delayed), &control)
                    .unwrap();
                let result =
                    query.collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control);
                if !delayed {
                    assert!(is_duplicate(&result.unwrap_err()));
                } else {
                    let (result, _) = result.unwrap();
                    assert_eq!(result.series.len(), usize::from(output.is_some()));
                    if let Some(value) = output {
                        assert_eq!(float_bits(&result.series[0].points[0]), f64::to_bits(value));
                    }
                }
            }
        }
        provider.assert_closed();
    }
    fn early_rate(index: usize) -> crate::value::Sample {
        crate::value::Sample {
            time_ms: if index == 0 { 100 } else { 900 },
            start_time_ms: 0,
            value: crate::value::SampleValue::Float(index as f64),
        }
    }
    fn late_rate(index: usize) -> crate::value::Sample {
        let mut sample = early_rate(index);
        sample.time_ms += 1000;
        sample
    }
    fn diagnostic_sample(index: usize) -> crate::value::Sample {
        let mut sample = crate::test_storage::counter(index);
        sample.value = crate::value::SampleValue::Float(index as f64);
        sample
    }
    fn mixed_range_sample(index: usize) -> crate::value::Sample {
        use crate::{
            histogram::{FloatHistogram, IntegerHistogram},
            value::{STALE_NAN_BITS, SampleValue},
        };
        let mut sample = diagnostic_sample(index);
        sample.value = match index {
            0 | 3 => SampleValue::Float(5.0),
            1 => SampleValue::IntegerHistogram(IntegerHistogram {
                zero_threshold: 0.001,
                zero_count: 2,
                count: 2,
                sum: 0.0001,
                ..IntegerHistogram::default()
            }),
            2 => SampleValue::FloatHistogram(FloatHistogram {
                zero_threshold: 0.001,
                zero_count: 1.0,
                count: 1.0,
                sum: 0.00001,
                ..FloatHistogram::default()
            }),
            4 => SampleValue::Float(f64::from_bits(0x7ff8_0000_0000_0042)),
            5 => SampleValue::Float(f64::from_bits(0x7ff8_0000_0000_0043)),
            6 => SampleValue::Float(f64::from_bits(STALE_NAN_BITS)),
            7 => SampleValue::Float(-0.0),
            8 => SampleValue::Float(0.0),
            _ => unreachable!(),
        };
        sample
    }
    #[test]
    fn ordinary_range_reducers_keep_typed_membership_extrema_metadata_and_negative_time_sentinels()
    {
        fn negative(index: usize) -> crate::value::Sample {
            let mut sample = diagnostic_sample(index);
            sample.time_ms = -4000 + index as i64 * 1000;
            sample.value = crate::value::SampleValue::Float(if index == 0 { -0.0 } else { 0.0 });
            sample
        }
        let cases = [
            ("count_over_time", 8.0f64, 2.0f64),
            ("present_over_time", 1.0, 1.0),
            ("first_over_time", 5.0, -0.0),
            ("last_over_time", 0.0, 0.0),
            ("min_over_time", -0.0, -0.0),
            ("max_over_time", 5.0, -0.0),
            ("ts_of_first_over_time", 1.0, -4.0),
            ("ts_of_last_over_time", 9.0, 0.0),
            ("ts_of_min_over_time", 9.0, -3.0),
            ("ts_of_max_over_time", 4.0, -3.0),
            ("changes", 5.0, 0.0),
            ("resets", 3.0, 0.0),
        ];
        for page_size in [1, 17, 1024] {
            let mut provider = FixtureProvider::new(9, page_size, 1);
            provider.series[0].labels = vec![
                (b"__name__".to_vec(), b"m".to_vec()),
                (b"__type__".to_vec(), b"gauge".to_vec()),
                (b"__unit__".to_vec(), b"seconds".to_vec()),
                (b"job".to_vec(), b"x".to_vec()),
            ];
            let engine = Engine::default();
            let control = Control::default();
            for is_negative in [false, true] {
                provider.series[0].source.sample = if is_negative {
                    negative
                } else {
                    mixed_range_sample
                };
                provider.series[0].source.count = if is_negative { 2 } else { 9 };
                for delayed in [false, true] {
                    let mut options = options(delayed);
                    options.parser.experimental_functions = delayed;
                    for (name, mixed_value, negative_value) in cases {
                        let source = format!("{name}(m[{}s])", if is_negative { 5 } else { 10 });
                        let query = engine.compile(source.as_bytes(), options, &control);
                        if name.starts_with("ts_") && !delayed {
                            assert!(query.is_err());
                            continue;
                        }
                        let query = query.unwrap();
                        let (result, observations) = query
                            .collect(
                                &provider,
                                QueryRequest::Instant {
                                    time_ms: if is_negative { -1000 } else { 9000 },
                                },
                                &control,
                            )
                            .unwrap();
                        assert_eq!(result.series.len(), 1, "{source}");
                        assert_eq!(
                            float_bits(&result.series[0].points[0]),
                            if is_negative {
                                negative_value
                            } else {
                                mixed_value
                            }
                            .to_bits(),
                            "{source}"
                        );
                        let preserves_name = matches!(name, "first_over_time" | "last_over_time");
                        assert_eq!(
                            result.series[0].labels.0.len(),
                            if preserves_name { 4 } else { 1 }
                        );
                        let ignored = !is_negative
                            && matches!(
                                name,
                                "min_over_time"
                                    | "max_over_time"
                                    | "ts_of_min_over_time"
                                    | "ts_of_max_over_time"
                            );
                        assert_eq!(result.annotations.len(), usize::from(ignored));
                        if ignored {
                            let info = result.annotations.iter().next().unwrap();
                            assert_eq!(
                                info.kind,
                                crate::annotations::AnnotationKind::HistogramIgnoredInMixedRange
                            );
                            assert_eq!(info.span.start, name.len() + 1);
                            assert_eq!(info.message, format!("PromQL info: ignored histograms in a range containing both floats and histograms for metric name \"m\" (1:{})", name.len()+2).as_bytes());
                        }
                        assert_eq!(
                            observations.memory[MemoryKind::ProviderHeld as usize].current,
                            0
                        );
                    }
                }
            }
            provider.series[0].source.sample = mixed_range_sample;
            provider.series[0].source.count = 9;
            for (name, count, sum_bits) in [
                ("first_over_time", 2.0f64, 0x3f1a36e2eb1c432du64),
                ("last_over_time", 1.0, 0x3ee4f8b588e368f1),
            ] {
                let query = engine
                    .compile(
                        format!("{name}(m[2s])").as_bytes(),
                        EngineOptions::default(),
                        &control,
                    )
                    .unwrap();
                let (result, _) = query
                    .collect(&provider, QueryRequest::Instant { time_ms: 3000 }, &control)
                    .unwrap();
                let crate::value::SampleValueRef::FloatHistogram(value) =
                    result.series[0].points[0].value.as_ref()
                else {
                    panic!("expected histogram");
                };
                assert_eq!(value.count.to_bits(), count.to_bits());
                assert_eq!(value.zero_count.to_bits(), count.to_bits());
                assert_eq!(value.sum.to_bits(), sum_bits);
                assert_eq!(value.zero_threshold.to_bits(), 0x3f50624dd2f1a9fc);
                assert_eq!(value.schema, 0);
                assert_eq!(value.reset_hint, 0);
                assert!(value.positive_spans.is_none() && value.negative_spans.is_none());
                assert!(
                    value.positive_buckets.is_none()
                        && value.negative_buckets.is_none()
                        && value.custom_values.is_none()
                );
                assert_eq!(result.series[0].labels.0.len(), 4);
                assert!(result.annotations.is_empty());
            }
            let query = engine
                .compile(b"rate(m[10s])", EngineOptions::default(), &control)
                .unwrap();
            let (result, _) = query
                .collect(&provider, QueryRequest::Instant { time_ms: 9000 }, &control)
                .unwrap();
            assert!(result.series.is_empty());
            assert_eq!(result.annotations.len(), 1);
            let warning = result.annotations.iter().next().unwrap();
            assert_eq!(
                warning.kind,
                crate::annotations::AnnotationKind::MixedFloatsHistograms
            );
            assert_eq!(warning.message, b"PromQL warning: encountered a mix of histograms and floats for metric name \"m\" (1:6)");
            provider.assert_closed();
        }
    }
    #[test]
    fn parenthesized_range_arguments_keep_values_and_source_positions() {
        fn ordinary(index: usize) -> crate::value::Sample {
            let mut sample = diagnostic_sample(index);
            sample.value = crate::value::SampleValue::Float((index + 1) as f64);
            sample
        }
        // Pinned Go ordinary-selector capture also checks the extrapolated rate values.
        let cases: [(&str, f64); 19] = [
            ("rate", 0.625),
            ("increase", 2.5),
            ("delta", 2.5),
            ("count_over_time", 3.0),
            ("present_over_time", 1.0),
            ("first_over_time", 1.0),
            ("last_over_time", 3.0),
            ("min_over_time", 1.0),
            ("max_over_time", 3.0),
            ("sum_over_time", 6.0),
            ("avg_over_time", 2.0),
            ("stdvar_over_time", 2.0 / 3.0),
            ("stddev_over_time", 0.816496580927726),
            ("ts_of_first_over_time", 1.0),
            ("ts_of_last_over_time", 3.0),
            ("ts_of_min_over_time", 1.0),
            ("ts_of_max_over_time", 3.0),
            ("changes", 2.0),
            ("resets", 0.0),
        ];
        for page_size in [1, 17, 1024] {
            let mut provider = FixtureProvider::new(3, page_size, 1);
            provider.series[0].labels = vec![(b"__name__".to_vec(), b"m".to_vec())];
            provider.series[0].source.sample = ordinary;
            let engine = Engine::default();
            let control = Control::default();
            for delayed in [false, true] {
                let mut options = options(delayed);
                options.parser.experimental_functions = true;
                for (name, expected) in cases {
                    for depth in 0..=2 {
                        let source =
                            format!("{name}({}m[4s]{})", "(".repeat(depth), ")".repeat(depth));
                        let query = engine
                            .compile(source.as_bytes(), options, &control)
                            .unwrap();
                        let (result, _) = query
                            .collect(&provider, QueryRequest::Instant { time_ms: 3000 }, &control)
                            .unwrap();
                        assert_eq!(result.series.len(), 1, "{source}");
                        assert_eq!(result.series[0].points.len(), 1);
                        assert_eq!(result.series[0].points[0].time_ms, 3000);
                        assert_eq!(
                            float_bits(&result.series[0].points[0]),
                            expected.to_bits(),
                            "{source}"
                        );
                        let retains_name = matches!(name, "first_over_time" | "last_over_time");
                        assert_eq!(result.series[0].labels.0.len(), usize::from(retains_name));
                        let warns = matches!(name, "rate" | "increase");
                        assert_eq!(result.annotations.len(), usize::from(warns));
                        if warns {
                            let note = result.annotations.iter().next().unwrap();
                            assert_eq!(
                                note.kind,
                                crate::annotations::AnnotationKind::PossibleNonCounter
                            );
                            assert_eq!(note.span.start, name.len() + 1 + depth);
                            assert_eq!(note.occurrences, 1);
                        }
                        drop(query);
                        drop(result);
                        provider.assert_closed();
                        assert!(
                            engine
                                .accounting
                                .snapshot()
                                .memory
                                .iter()
                                .all(|m| m.current == 0)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn numerical_range_folds_preserve_overflow_compensation_and_mixed_diagnostics() {
        fn ordinary(index: usize) -> crate::value::Sample {
            let mut sample = diagnostic_sample(index);
            sample.value = crate::value::SampleValue::Float((index + 1) as f64);
            sample
        }
        fn precision(index: usize) -> crate::value::Sample {
            let mut sample = diagnostic_sample(index);
            sample.value = crate::value::SampleValue::Float([1e100, 1.0, 1.0, -1e100][index]);
            sample
        }
        fn overflow(index: usize) -> crate::value::Sample {
            let mut sample = diagnostic_sample(index);
            sample.value = crate::value::SampleValue::Float([1e308, 1e308, -1e308][index]);
            sample
        }
        fn mixed(index: usize) -> crate::value::Sample {
            let mut sample = if index == 1 {
                mixed_range_sample(1)
            } else {
                ordinary(index)
            };
            sample.time_ms = (index + 1) as i64 * 1000;
            if index != 1 {
                sample.value = crate::value::SampleValue::Float(if index == 0 { 2.0 } else { 4.0 });
            }
            sample
        }
        type NumericCase = (fn(usize) -> crate::value::Sample, usize, [u64; 4]);
        let cases: [NumericCase; 4] = [
            (
                ordinary,
                3,
                [
                    6.0f64.to_bits(),
                    2.0f64.to_bits(),
                    0x3fe5_5555_5555_5555,
                    0x3fea_20bd_700c_2c3e,
                ],
            ),
            (
                precision,
                4,
                [
                    2.0f64.to_bits(),
                    0.5f64.to_bits(),
                    0x6964_e718_d7d7_625b,
                    0x54a9_dce3_e3d9_4c84,
                ],
            ),
            (
                overflow,
                3,
                [
                    f64::INFINITY.to_bits(),
                    0x7fc7_bbef_5d3a_60d5,
                    f64::NEG_INFINITY.to_bits(),
                    f64::NAN.to_bits(),
                ],
            ),
            (mixed, 3, [0, 0, 1.0f64.to_bits(), 1.0f64.to_bits()]),
        ];
        for page_size in [1, 17, 1024] {
            for delayed in [false, true] {
                for (case, (sample, count, expected)) in cases.iter().enumerate() {
                    let mut provider = FixtureProvider::new(*count, page_size, 1);
                    provider.series[0].labels[0].1 = b"m".to_vec();
                    provider.series[0].source.sample = *sample;
                    let engine = Engine::default();
                    let control = Control::default();
                    for (index, name) in [
                        "sum_over_time",
                        "avg_over_time",
                        "stdvar_over_time",
                        "stddev_over_time",
                    ]
                    .iter()
                    .enumerate()
                    {
                        let query = engine
                            .compile(
                                format!("{name}(m[5s])").as_bytes(),
                                options(delayed),
                                &control,
                            )
                            .unwrap();
                        let (result, _) = query
                            .collect(
                                &provider,
                                QueryRequest::Instant {
                                    time_ms: *count as i64 * 1000,
                                },
                                &control,
                            )
                            .unwrap();
                        if case == 3 && index < 2 {
                            assert!(result.series.is_empty());
                            let notes: Vec<_> = result.annotations.iter().collect();
                            assert_eq!(notes.len(), 1);
                            assert_eq!(
                                notes[0].kind,
                                crate::annotations::AnnotationKind::MixedFloatsHistograms
                            );
                            assert_eq!(notes[0].span.start, name.len() + 1);
                        } else {
                            assert_eq!(result.series.len(), 1);
                            let actual = f64::from_bits(float_bits(&result.series[0].points[0]));
                            if f64::from_bits(expected[index]).is_nan() {
                                assert!(actual.is_nan());
                            } else {
                                assert_eq!(actual.to_bits(), expected[index], "{name} case {case}");
                            }
                            assert_eq!(result.annotations.len(), usize::from(case == 3));
                            if case == 3 {
                                assert_eq!(result.annotations.iter().next().unwrap().kind, crate::annotations::AnnotationKind::HistogramIgnoredInMixedRange);
                            }
                        }
                        provider.assert_closed();
                        drop(query);
                        drop(result);
                        assert!(
                            engine
                                .accounting
                                .snapshot()
                                .memory
                                .iter()
                                .all(|m| m.current == 0)
                        );
                    }
                }
            }
        }
    }

    fn native_range_sample(index: usize) -> crate::value::Sample {
        use crate::{
            histogram::{FloatHistogram, Span},
            value::{Sample, SampleValue},
        };
        Sample {
            time_ms: (index as i64 + 1) * 1000,
            start_time_ms: 0,
            value: SampleValue::FloatHistogram(FloatHistogram {
                schema: 0,
                zero_threshold: 0.001,
                zero_count: 2.0,
                count: 8.0,
                sum: (index + 1) as f64,
                positive_spans: Some(
                    vec![Span {
                        offset: 0,
                        length: 3,
                    }]
                    .into(),
                ),
                positive_buckets: Some(vec![1.0, 2.0, 3.0].into()),
                ..FloatHistogram::default()
            }),
        }
    }

    #[test]
    fn native_range_folds_keep_integer_stale_compensated_and_overflow_outcomes() {
        use crate::{
            histogram::{IntegerHistogram, Span},
            value::{STALE_NAN_BITS, Sample, SampleValue},
        };
        fn integer(index: usize) -> Sample {
            let mut sample = native_range_sample(index);
            sample.value = SampleValue::IntegerHistogram(IntegerHistogram {
                schema: 0,
                zero_threshold: 0.001,
                zero_count: 2,
                count: 8,
                sum: (index + 1) as f64,
                positive_spans: Some(vec![Span {
                    offset: 0,
                    length: 3,
                }]),
                positive_buckets: Some(vec![1, 1, 1]),
                ..IntegerHistogram::default()
            });
            sample
        }
        fn stale(index: usize) -> Sample {
            let mut sample = native_range_sample(index);
            if index == 1 {
                let SampleValue::FloatHistogram(histogram) = &mut sample.value else {
                    unreachable!()
                };
                histogram.sum = f64::from_bits(STALE_NAN_BITS);
            }
            sample
        }
        fn precision(index: usize) -> Sample {
            let mut sample = native_range_sample(index);
            let SampleValue::FloatHistogram(histogram) = &mut sample.value else {
                unreachable!()
            };
            histogram.sum = [1e100, 1.0, 1.0, -1e100][index];
            sample
        }
        fn overflow(index: usize) -> Sample {
            let mut sample = native_range_sample(index);
            let SampleValue::FloatHistogram(histogram) = &mut sample.value else {
                unreachable!()
            };
            histogram.sum = [1e308, 1e308, -1e308][index];
            sample
        }
        type NativeCase = (fn(usize) -> Sample, usize, usize, [u64; 2]);
        // Compensated and overflow sum fields are also independently captured from pinned Go.
        let cases: [NativeCase; 5] = [
            (
                native_range_sample,
                3,
                3,
                [6.0f64.to_bits(), 2.0f64.to_bits()],
            ),
            (integer, 3, 3, [6.0f64.to_bits(), 2.0f64.to_bits()]),
            (stale, 3, 2, [4.0f64.to_bits(), 2.0f64.to_bits()]),
            (precision, 4, 4, [2.0f64.to_bits(), 0.5f64.to_bits()]),
            (
                overflow,
                3,
                3,
                [f64::INFINITY.to_bits(), 0x7fc7_bbef_5d3a_60d5],
            ),
        ];
        for page_size in [1, 17, 1024] {
            for delayed in [false, true] {
                for (sample, count, valid, expected) in cases {
                    let mut provider = FixtureProvider::new(count, page_size, 1);
                    provider.series[0].labels[0].1 = b"m".to_vec();
                    provider.series[0].source.sample = sample;
                    let engine = Engine::default();
                    let control = Control::default();
                    for (function, name) in ["sum_over_time", "avg_over_time"].iter().enumerate() {
                        for fixed in [false, true] {
                            let query = engine
                                .compile(
                                    format!(
                                        "{name}((m[10s]{}))",
                                        if fixed {
                                            format!(" @ {count}")
                                        } else {
                                            String::new()
                                        }
                                    )
                                    .as_bytes(),
                                    options(delayed),
                                    &control,
                                )
                                .unwrap();
                            let request = if fixed {
                                QueryRequest::Range {
                                    start_ms: 1000,
                                    end_ms: 3000,
                                    step_ms: 1000,
                                }
                            } else {
                                QueryRequest::Instant {
                                    time_ms: count as i64 * 1000,
                                }
                            };
                            let (result, observations) =
                                query.collect(&provider, request, &control).unwrap();
                            assert_eq!(result.series.len(), 1);
                            assert!(result.annotations.is_empty());
                            assert_eq!(result.series[0].points.len(), if fixed { 3 } else { 1 });
                            // Fixed calls emit diagnostics once and replay each provisional output step.
                            assert_eq!(
                                observations.samples_fetched,
                                count as u64 * if fixed { 8 } else { 4 }
                            );
                            let factor = if function == 0 { valid as f64 } else { 1.0 };
                            for point in result.series[0].points.iter() {
                                let OwnedValue::Histogram(histogram) = &point.value else {
                                    panic!("expected native result")
                                };
                                let value = histogram.as_ref();
                                assert_eq!(
                                    value.sum.to_bits(),
                                    expected[function],
                                    "{name} fixed {fixed}"
                                );
                                assert_eq!(value.count, factor * 8.0);
                                assert_eq!(value.zero_count, factor * 2.0);
                                assert_eq!(value.reset_hint, 0);
                                assert_eq!(value.schema, 0);
                                assert_eq!(
                                    value.positive_spans,
                                    Some(
                                        &[Span {
                                            offset: 0,
                                            length: 3
                                        }][..]
                                    )
                                );
                                assert_eq!(
                                    value.positive_buckets,
                                    Some(&[factor, factor * 2.0, factor * 3.0][..])
                                );
                            }
                            provider.assert_closed();
                            drop(query);
                            drop(result);
                            assert!(
                                engine
                                    .accounting
                                    .snapshot()
                                    .memory
                                    .iter()
                                    .all(|m| m.current == 0)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn native_range_history_length_does_not_grow_retained_raw_memory() {
        for page_size in [1, 17, 1024] {
            for name in ["sum_over_time", "avg_over_time"] {
                let mut peaks = vec![];
                // Both histories cross a full page while an accumulator is already retained.
                for count in [2048, 100_000] {
                    let engine = Engine::default();
                    let mut provider = FixtureProvider::new(count, page_size, 1);
                    provider.series[0].source.sample = native_range_sample;
                    let control = Control::default();
                    let query = engine
                        .compile(
                            format!("{name}(counter_total[{count}s])").as_bytes(),
                            EngineOptions::default(),
                            &control,
                        )
                        .unwrap();
                    let (result, observations) = query
                        .collect(
                            &provider,
                            QueryRequest::Instant {
                                time_ms: count as i64 * 1000,
                            },
                            &control,
                        )
                        .unwrap();
                    let OwnedValue::Histogram(histogram) = &result.series[0].points[0].value else {
                        panic!("expected native result")
                    };
                    let value = histogram.as_ref();
                    let n = count as f64;
                    assert_eq!(
                        value.sum,
                        if name == "sum_over_time" {
                            n * (n + 1.0) / 2.0
                        } else {
                            (n + 1.0) / 2.0
                        }
                    );
                    let factor = if name == "sum_over_time" { n } else { 1.0 };
                    assert_eq!(value.count, factor * 8.0);
                    assert_eq!(
                        value.positive_buckets,
                        Some(&[factor, factor * 2.0, factor * 3.0][..])
                    );
                    assert_eq!(observations.samples_fetched, count as u64 * 4);
                    assert_eq!(
                        provider.series[0]
                            .source
                            .counters
                            .peak
                            .load(Ordering::Relaxed),
                        1
                    );
                    peaks.push(observations.memory[MemoryKind::Workspace as usize].peak);
                    provider.assert_closed();
                    drop(query);
                    drop(result);
                    assert!(
                        engine
                            .accounting
                            .snapshot()
                            .memory
                            .iter()
                            .all(|m| m.current == 0)
                    );
                }
                assert_eq!(peaks[0], peaks[1], "{name} page {page_size}");
            }
        }
    }

    #[test]
    fn native_range_diagnostics_keep_original_hints_and_source_precedence() {
        use crate::{
            annotations::{AnnotationKind, HistogramOperation},
            value::{STALE_NAN_BITS, Sample, SampleValue},
        };
        fn original_hints(index: usize) -> Sample {
            let mut sample = native_range_sample(index);
            let SampleValue::FloatHistogram(histogram) = &mut sample.value else {
                unreachable!()
            };
            histogram.reset_hint = [1, 0, 2][index];
            sample
        }
        fn custom_collision(index: usize) -> Sample {
            let mut sample = native_range_sample(index);
            let SampleValue::FloatHistogram(histogram) = &mut sample.value else {
                unreachable!()
            };
            histogram.schema = -53;
            histogram.zero_threshold = 0.0;
            histogram.zero_count = 0.0;
            histogram.count = 6.0;
            histogram.sum = [7.0, 11.0][index];
            histogram.reset_hint = [1, 2][index];
            histogram.custom_values = Some(
                if index == 0 {
                    vec![1.0, 2.0]
                } else {
                    vec![2.0, 3.0]
                }
                .into(),
            );
            sample
        }
        fn schema_after_notes(index: usize) -> Sample {
            if index < 2 {
                custom_collision(index)
            } else {
                native_range_sample(index)
            }
        }
        fn float_after_schema(index: usize) -> Sample {
            if index < 3 {
                schema_after_notes(index)
            } else {
                diagnostic_sample(index)
            }
        }
        fn stale_float_after_notes(index: usize) -> Sample {
            if index < 2 {
                custom_collision(index)
            } else {
                let mut sample = diagnostic_sample(index);
                sample.value = SampleValue::Float(f64::from_bits(STALE_NAN_BITS));
                sample
            }
        }
        let collision =
            AnnotationKind::HistogramCounterResetCollision(HistogramOperation::Aggregation);
        let reconcile = AnnotationKind::MismatchedCustomBuckets(HistogramOperation::Aggregation);
        type DiagnosticCase = (fn(usize) -> Sample, usize, bool, Vec<AnnotationKind>);
        let cases: [DiagnosticCase; 5] = [
            (original_hints, 3, true, vec![collision]),
            (custom_collision, 2, true, vec![collision, reconcile]),
            (
                schema_after_notes,
                3,
                false,
                vec![AnnotationKind::MixedExponentialCustomHistograms],
            ),
            (
                float_after_schema,
                4,
                false,
                vec![AnnotationKind::MixedFloatsHistograms],
            ),
            (stale_float_after_notes, 3, true, vec![collision, reconcile]),
        ];
        for page_size in [1, 17, 1024] {
            for delayed in [false, true] {
                for (case, (sample, count, productive, expected)) in cases.iter().enumerate() {
                    let mut provider = FixtureProvider::new(*count, page_size, 1);
                    provider.series[0].labels[0].1 = b"m".to_vec();
                    provider.series[0].source.sample = *sample;
                    let engine = Engine::default();
                    let control = Control::default();
                    for name in ["sum_over_time", "avg_over_time"] {
                        let query = engine
                            .compile(
                                format!("{name}((m[10s] @ {count}))").as_bytes(),
                                options(delayed),
                                &control,
                            )
                            .unwrap();
                        let (result, _) = query
                            .collect(
                                &provider,
                                QueryRequest::Range {
                                    start_ms: 1000,
                                    end_ms: 3000,
                                    step_ms: 1000,
                                },
                                &control,
                            )
                            .unwrap();
                        assert_eq!(result.series.len(), usize::from(*productive));
                        let notes: Vec<_> = result.annotations.iter().collect();
                        assert_eq!(
                            notes.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
                            *expected
                        );
                        for note in notes {
                            assert_eq!(note.span.start, name.len() + 2);
                            assert_eq!(note.occurrences, 1);
                            assert!(
                                note.message
                                    .ends_with(format!("(1:{})", name.len() + 3).as_bytes())
                            );
                        }
                        if *productive {
                            assert_eq!(result.series[0].points.len(), 3);
                            if case != 0 {
                                let factor = if name == "sum_over_time" { 1.0 } else { 0.5 };
                                for point in result.series[0].points.iter() {
                                    let OwnedValue::Histogram(histogram) = &point.value else {
                                        panic!("expected native result")
                                    };
                                    let value = histogram.as_ref();
                                    assert_eq!(value.reset_hint, 0);
                                    assert_eq!(value.count, factor * 12.0);
                                    assert_eq!(value.sum, factor * 18.0);
                                    assert_eq!(value.custom_values, Some(&[2.0][..]));
                                    assert_eq!(
                                        value.positive_buckets,
                                        Some(&[factor * 4.0, factor * 8.0][..])
                                    );
                                }
                            }
                        }
                        provider.assert_closed();
                        drop(query);
                        drop(result);
                        assert!(
                            engine
                                .accounting
                                .snapshot()
                                .memory
                                .iter()
                                .all(|m| m.current == 0)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn numerical_range_history_length_does_not_grow_retained_raw_memory() {
        for page_size in [1, 17, 1024] {
            for name in [
                "sum_over_time",
                "avg_over_time",
                "stdvar_over_time",
                "stddev_over_time",
            ] {
                let mut peaks = vec![];
                for count in [1024, 100_000] {
                    let engine = Engine::default();
                    let provider = FixtureProvider::new(count, page_size, 1);
                    let control = Control::default();
                    let query = engine
                        .compile(
                            format!("{name}(counter_total[{count}s])").as_bytes(),
                            EngineOptions::default(),
                            &control,
                        )
                        .unwrap();
                    let (result, observations) = query
                        .collect(
                            &provider,
                            QueryRequest::Instant {
                                time_ms: count as i64 * 1000,
                            },
                            &control,
                        )
                        .unwrap();
                    let n = count as f64;
                    let variance = (n * n - 1.0) / 12.0;
                    let expected = match name {
                        "sum_over_time" => n * (n + 1.0) / 2.0,
                        "avg_over_time" => (n + 1.0) / 2.0,
                        "stdvar_over_time" => variance,
                        _ => crate::numerical::sqrt(variance),
                    };
                    assert_eq!(float_bits(&result.series[0].points[0]), expected.to_bits());
                    assert_eq!(observations.samples_fetched, count as u64 * 2);
                    assert_eq!(
                        provider.series[0]
                            .source
                            .counters
                            .peak
                            .load(Ordering::Relaxed),
                        1
                    );
                    peaks.push(observations.memory[MemoryKind::Workspace as usize].peak);
                    provider.assert_closed();
                    drop(query);
                    drop(result);
                    assert!(
                        engine
                            .accounting
                            .snapshot()
                            .memory
                            .iter()
                            .all(|m| m.current == 0)
                    );
                }
                assert_eq!(peaks[0], peaks[1], "{name}");
            }
        }
    }

    #[test]
    fn count_range_history_length_does_not_grow_retained_raw_memory() {
        for page_size in [1, 17, 1024] {
            let mut peaks = vec![];
            for count in [1024, 100_000] {
                let engine = Engine::default();
                let provider = FixtureProvider::new(count, page_size, 1);
                let control = Control::default();
                let query = engine
                    .compile(
                        format!("count_over_time(counter_total[{count}s])").as_bytes(),
                        EngineOptions::default(),
                        &control,
                    )
                    .unwrap();
                let (result, observations) = query
                    .collect(
                        &provider,
                        QueryRequest::Instant {
                            time_ms: count as i64 * 1000,
                        },
                        &control,
                    )
                    .unwrap();
                assert_eq!(
                    float_bits(&result.series[0].points[0]),
                    (count as f64).to_bits()
                );
                assert_eq!(observations.samples_fetched, count as u64 * 2);
                assert_eq!(
                    provider.series[0]
                        .source
                        .counters
                        .peak
                        .load(Ordering::Relaxed),
                    1
                );
                peaks.push(observations.memory[MemoryKind::Workspace as usize].peak);
                provider.assert_closed();
                drop(query);
                drop(result);
                assert!(
                    engine
                        .accounting
                        .snapshot()
                        .memory
                        .iter()
                        .all(|memory| memory.current == 0)
                );
            }
            assert_eq!(peaks[0], peaks[1]);
        }
    }
    #[test]
    fn rate_diagnostics_merge_logical_positions_and_windows_without_replay_counts() {
        use crate::annotations::AnnotationKind;
        fn overlap(index: usize) -> crate::value::Sample {
            let mut sample = diagnostic_sample(index);
            sample.start_time_ms = [100, 500, 1500, 2500][index];
            sample
        }
        for page_size in [1, 17, 1024] {
            let mut provider = FixtureProvider::new(2, page_size, 1);
            provider.series[0].labels = vec![(b"__name__".to_vec(), b"m".to_vec())];
            provider.series[0].source.sample = diagnostic_sample;
            let engine = Engine::default();
            let control = Control::default();
            for delayed in [false, true] {
                let query = engine
                    .compile(
                        b"sum(rate(m[2s])) + scalar(sum(rate(m[2s])))",
                        options(delayed),
                        &control,
                    )
                    .unwrap();
                let (result, _) = query
                    .collect(&provider, QueryRequest::Instant { time_ms: 2000 }, &control)
                    .unwrap();
                assert_eq!(float_bits(&result.series[0].points[0]), 1.0f64.to_bits());
                assert_eq!(result.annotations.len(), 1);
                let info = result.annotations.iter().next().unwrap();
                assert_eq!(info.kind, AnnotationKind::PossibleNonCounter);
                assert_eq!(info.span.start, 35);
                assert!(info.message.ends_with(b"\"m\" (1:36)"));
            }
            provider.series[0].source.sample = overlap;
            provider.series[0].source.count = 4;
            for delayed in [false, true] {
                for expression in [&b"sum(rate(m[4s]))"[..], &b"delta(m[4s])"[..]] {
                    let query = engine
                        .compile(expression, options(delayed), &control)
                        .unwrap();
                    let (result, _) = query
                        .collect(
                            &provider,
                            QueryRequest::Range {
                                start_ms: 2000,
                                end_ms: 4000,
                                step_ms: 1000,
                            },
                            &control,
                        )
                        .unwrap();
                    let is_rate = expression[0] == b's';
                    assert_eq!(
                        result.series[0]
                            .points
                            .iter()
                            .map(float_bits)
                            .collect::<Vec<_>>(),
                        if is_rate {
                            vec![0.25f64.to_bits(), 0.5f64.to_bits(), 0.75f64.to_bits()]
                        } else {
                            vec![1.5f64.to_bits(), 2.5f64.to_bits(), 4.0f64.to_bits()]
                        }
                    );
                    if is_rate {
                        assert_eq!(result.annotations.len(), if delayed { 2 } else { 1 });
                        if delayed {
                            let warning = result
                                .annotations
                                .iter()
                                .find(|entry| entry.kind == AnnotationKind::StartTimeOverlap)
                                .unwrap();
                            assert_eq!(warning.occurrences, 3);
                            assert!(warning.message.ends_with(b"\"m\" (3 occurrences) (1:10)"));
                        }
                    } else {
                        assert!(result.annotations.is_empty());
                    }
                }
            }
            provider.assert_closed();
        }
    }

    #[test]
    fn fixed_range_calls_emit_one_logical_overlap_and_every_output_step() {
        fn overlap(index: usize) -> crate::value::Sample {
            let mut sample = diagnostic_sample(index);
            sample.start_time_ms = [100, 500, 1500, 2500][index];
            sample
        }
        for page_size in [1, 17, 1024] {
            let mut provider = FixtureProvider::new(4, page_size, 1);
            provider.series[0].labels = vec![(b"__name__".to_vec(), b"m".to_vec())];
            provider.series[0].source.sample = overlap;
            let engine = Engine::default();
            let control = Control::default();
            for delayed in [false, true] {
                for expression in [
                    &b"sum(rate(m[4s] @ 4))"[..],
                    &b"sum(rate(m[4s] @ end()))"[..],
                    &b"rate(m[4s] @ 4)"[..],
                    &b"sum(rate(((m[4s] @ 4))))"[..],
                    &b"sum(rate(((m[4s] @ end()))))"[..],
                    &b"rate(((m[4s] @ 4)))"[..],
                ] {
                    let query = engine
                        .compile(expression, options(delayed), &control)
                        .unwrap();
                    let (result, _) = query
                        .collect(
                            &provider,
                            QueryRequest::Range {
                                start_ms: 2000,
                                end_ms: 4000,
                                step_ms: 1000,
                            },
                            &control,
                        )
                        .unwrap();
                    assert_eq!(
                        result.series[0]
                            .points
                            .iter()
                            .map(float_bits)
                            .collect::<Vec<_>>(),
                        vec![0.75f64.to_bits(); 3]
                    );
                    assert_eq!(result.annotations.len(), if delayed { 2 } else { 1 });
                    if delayed {
                        let warning = result
                            .annotations
                            .iter()
                            .find(|entry| {
                                entry.kind == crate::annotations::AnnotationKind::StartTimeOverlap
                            })
                            .unwrap();
                        assert_eq!(warning.occurrences, 1);
                        assert!(
                            !warning
                                .message
                                .windows(11)
                                .any(|bytes| bytes == b"occurrences")
                        );
                        let position = expression
                            .windows(2)
                            .position(|bytes| bytes == b"m[")
                            .unwrap();
                        assert_eq!(warning.span.start, position);
                        assert!(
                            warning
                                .message
                                .ends_with(format!("(1:{})", position + 1).as_bytes())
                        );
                    }
                }
            }
            provider.assert_closed();
        }
    }

    #[test]
    fn resets_observes_enabled_start_timestamps_without_changing_changes() {
        fn sample(index: usize) -> crate::value::Sample {
            let mut sample = diagnostic_sample(index);
            sample.start_time_ms = [100, 1500][index];
            sample.value = crate::value::SampleValue::Float(5.0);
            sample
        }
        for page_size in [1, 17, 1024] {
            let mut provider = FixtureProvider::new(2, page_size, 1);
            provider.series[0].labels = vec![(b"__name__".to_vec(), b"m".to_vec())];
            provider.series[0].source.sample = sample;
            let engine = Engine::default();
            let control = Control::default();
            for delayed in [false, true] {
                for expression in [&b"resets(m[2s])"[..], &b"changes(m[2s])"[..]] {
                    let query = engine
                        .compile(expression, options(delayed), &control)
                        .unwrap();
                    let (result, _) = query
                        .collect(&provider, QueryRequest::Instant { time_ms: 2000 }, &control)
                        .unwrap();
                    assert_eq!(
                        float_bits(&result.series[0].points[0]),
                        if delayed && expression[0] == b'r' {
                            1.0f64.to_bits()
                        } else {
                            0.0f64.to_bits()
                        }
                    );
                    assert!(result.annotations.is_empty());
                }
            }
            provider.assert_closed();
        }
    }

    #[test]
    fn scalar_requires_one_float_and_ignores_histograms_in_either_encounter_order() {
        fn histogram(index: usize) -> crate::value::Sample {
            let mut sample = crate::test_storage::counter(index);
            sample.value =
                crate::value::SampleValue::FloatHistogram(crate::histogram::FloatHistogram {
                    count: 1.0,
                    sum: 1.0,
                    ..crate::histogram::FloatHistogram::default()
                });
            sample
        }
        for page_size in [1, 17, 1024] {
            let mut provider = FixtureProvider::new(1, page_size, 3);
            provider.series[0].labels = vec![
                (b"__name__".to_vec(), b"metric".to_vec()),
                (b"l".to_vec(), b"x".to_vec()),
                (b"type".to_vec(), b"float".to_vec()),
            ];
            provider.series[1].labels = vec![
                (b"__name__".to_vec(), b"metric".to_vec()),
                (b"l".to_vec(), b"y".to_vec()),
                (b"type".to_vec(), b"float".to_vec()),
            ];
            provider.series[1].source.sample = second_value;
            provider.series[2].labels = vec![
                (b"__name__".to_vec(), b"metric".to_vec()),
                (b"l".to_vec(), b"x".to_vec()),
                (b"type".to_vec(), b"histogram".to_vec()),
            ];
            provider.series[2].source.sample = histogram;
            let engine = Engine::default();
            let control = Control::default();
            for reverse in [false, true] {
                if reverse {
                    provider.series.swap(0, 2);
                }
                for delayed in [false, true] {
                    for (expression, expected) in [
                        (&b"scalar({l=\"x\"})"[..], Some(1.0)),
                        (&b"scalar({type=\"histogram\"})"[..], None),
                        (&b"scalar(metric)"[..], None),
                    ] {
                        let query = engine
                            .compile(expression, options(delayed), &control)
                            .unwrap();
                        let (result, _) = query
                            .collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control)
                            .unwrap();
                        let value = result.scalar.unwrap().1;
                        if let Some(expected) = expected {
                            assert_eq!(value, expected);
                        } else {
                            assert!(value.is_nan());
                        }
                        assert!(result.annotations.is_empty());
                    }
                }
            }
            provider.assert_closed();
        }
    }

    #[test]
    fn scalar_binary_ranges_have_one_row_without_releasing_the_vector_order_guard() {
        let provider = FixtureProvider::new(1, 1, 1);
        let engine = Engine::default();
        let control = Control::default();
        let request = QueryRequest::Range {
            start_ms: 1000,
            end_ms: 4000,
            step_ms: 1000,
        };
        for expression in [&b"time() / 2 >= bool 1"[..], &b"1 / -0"[..]] {
            let query = engine
                .compile(expression, EngineOptions::default(), &control)
                .unwrap();
            let (result, _) = query.collect(&provider, request, &control).unwrap();
            assert_eq!(result.series.len(), 1);
            assert!(result.series[0].labels.0.is_empty());
            assert_eq!(
                result.series[0]
                    .points
                    .iter()
                    .map(float_bits)
                    .collect::<Vec<_>>(),
                if expression[0] == b't' {
                    vec![
                        0.0f64.to_bits(),
                        1.0f64.to_bits(),
                        1.0f64.to_bits(),
                        1.0f64.to_bits(),
                    ]
                } else {
                    vec![f64::NEG_INFINITY.to_bits(); 4]
                }
            );
        }
        for expression in [&b"counter_total * 1"[..], &b"scalar(counter_total * 1)"[..]] {
            let query = engine
                .compile(expression, EngineOptions::default(), &control)
                .unwrap();
            assert!(matches!(
                query.collect(&provider, request, &control),
                Err(ExecutionError::Evaluation(EvaluationError::Unsupported(
                    "unordered intermediate matrix policy pending"
                )))
            ));
        }
        provider.assert_closed();
    }

    #[test]
    fn delayed_cleanup_failure_keeps_owned_infos_while_ordinary_evaluation_failure_discards_them() {
        let mut provider = FixtureProvider::new(2, 1, 2);
        for (series, name) in provider.series.iter_mut().zip([b"a", b"b"]) {
            series.labels = vec![
                (b"__name__".to_vec(), name.to_vec()),
                (b"job".to_vec(), b"x".to_vec()),
            ];
            series.source.sample = diagnostic_sample;
        }
        let engine = Engine::default();
        let control = Control::default();
        for delayed in [false, true] {
            let query = engine
                .compile(b"rate({__name__=~\"a|b\"}[2s])", options(delayed), &control)
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Instant { time_ms: 2000 },
                &control,
            );
            assert!(is_duplicate(report.result.as_ref().unwrap_err()));
            assert_eq!(report.annotations.len(), if delayed { 2 } else { 0 });
            for info in report.annotations.iter() {
                assert!(info.message.ends_with(b"(1:6)"));
            }
            provider.assert_closed();
            drop(query);
            if delayed {
                assert!(
                    engine.accounting.snapshot().memory[MemoryKind::Result as usize].current > 0
                );
            }
            drop(report);
            assert_eq!(
                engine.accounting.snapshot().memory[MemoryKind::Result as usize].current,
                0
            );
        }
    }
    #[test]
    fn range_function_duplicate_detection_includes_disjoint_timelines() {
        let mut provider = FixtureProvider::new(2, 1, 2);
        for (series, name) in provider.series.iter_mut().zip([b"a", b"b"]) {
            series.labels = vec![
                (b"__name__".to_vec(), name.to_vec()),
                (b"job".to_vec(), b"x".to_vec()),
            ];
        }
        provider.series[0].source.sample = early_rate;
        provider.series[1].source.sample = late_rate;
        let engine = Engine::default();
        let control = Control::default();
        for delayed in [false, true] {
            let query = engine
                .compile(
                    b"sum(rate({__name__=~\"a|b\"}[1s]))",
                    options(delayed),
                    &control,
                )
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Range {
                    start_ms: 1000,
                    end_ms: 2000,
                    step_ms: 1000,
                },
                &control,
            );
            if !delayed {
                assert!(is_duplicate(&report.result.unwrap_err()));
                assert!(report.annotations.is_empty());
            } else {
                let result = report.result.unwrap();
                assert_eq!(report.annotations.len(), 2);
                for (entry, name) in report.annotations.iter().zip([b"a", b"b"]) {
                    assert_eq!(
                        entry.kind,
                        crate::annotations::AnnotationKind::PossibleNonCounter
                    );
                    assert!(entry.message.ends_with(if name == b"a" {
                        &b"\"a\" (1:10)"[..]
                    } else {
                        &b"\"b\" (1:10)"[..]
                    }));
                }
                assert_eq!(result.series.len(), 1);
                assert_eq!(
                    result.series[0]
                        .points
                        .iter()
                        .map(float_bits)
                        .collect::<Vec<_>>(),
                    vec![1.125f64.to_bits(); 2]
                );
            }
        }
        provider.assert_closed();
    }
    #[test]
    fn left_child_finishes_before_a_right_scalar_is_read() {
        fn panic_sample(_: usize) -> crate::value::Sample {
            panic!("right child must not be read");
        }
        let mut provider = FixtureProvider::new(1, 1, 2);
        provider.series[0].source.fail_page = Some(0);
        provider.series[1].labels = vec![(b"__name__".to_vec(), b"other_total".to_vec())];
        provider.series[1].source.sample = panic_sample;
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(
                b"counter_total + scalar(other_total)",
                EngineOptions::default(),
                &control,
            )
            .unwrap();
        assert!(matches!(
            query.collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control),
            Err(ExecutionError::Evaluation(EvaluationError::Storage(
                StorageError::Provider(_)
            )))
        ));
        provider.assert_closed();
    }
    #[test]
    fn zero_lookback_uses_the_source_default_and_fractional_rate_keeps_threshold_bits() {
        fn fractional(index: usize) -> crate::value::Sample {
            crate::value::Sample {
                time_ms: 1000 + index as i64 * 100,
                start_time_ms: 0,
                value: crate::value::SampleValue::Float(index as f64),
            }
        }
        let mut provider = FixtureProvider::new(1, 1, 1);
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(
                b"counter_total",
                EngineOptions {
                    lookback_ms: 0,
                    ..EngineOptions::default()
                },
                &control,
            )
            .unwrap();
        assert_eq!(
            query
                .collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control)
                .unwrap()
                .0
                .series
                .len(),
            1
        );
        provider.series[0].labels = vec![(b"__name__".to_vec(), b"m_total".to_vec())];
        provider.series[0].source.count = 2;
        provider.series[0].source.sample = fractional;
        for delayed in [false, true] {
            let query = engine
                .compile(
                    b"rate(m_total[1118ms]) > 1.0554561717352413",
                    options(delayed),
                    &control,
                )
                .unwrap();
            let (result, _) = query
                .collect(&provider, QueryRequest::Instant { time_ms: 1118 }, &control)
                .unwrap();
            assert_eq!(result.series.len(), 1);
            assert_eq!(float_bits(&result.series[0].points[0]), 0x3ff0e32602bf6d74);
        }
        provider.assert_closed();
    }
    #[test]
    fn a_panic_after_provisional_chunks_aborts_and_releases_the_context() {
        let engine = Engine::default();
        let provider = FixtureProvider::new(10, 1, 1);
        let control = Control::default();
        let query = engine
            .compile(b"counter_total", EngineOptions::default(), &control)
            .unwrap();
        let mut sink = RecordingSink {
            panic_after: Some(2),
            ..RecordingSink::default()
        };
        assert!(matches!(
            query.execute(
                &provider,
                QueryRequest::Range {
                    start_ms: 1000,
                    end_ms: 10000,
                    step_ms: 1000
                },
                &control,
                &mut sink
            ),
            Err(ExecutionError::Internal)
        ));
        assert_eq!(sink.chunks, 2);
        assert!(sink.aborted && !sink.committed);
        provider.assert_closed();
        assert_eq!(engine.accounting.snapshot().active_executions, 0);
        assert!(
            query
                .collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control)
                .is_ok()
        );
        provider.assert_closed();
    }

    #[test]
    fn source_group_hash_delimiters_can_merge_distinct_byte_labelsets() {
        let mut provider = FixtureProvider::new(1, 1, 2);
        provider.series[0].labels = vec![
            (b"__name__".to_vec(), b"m".to_vec()),
            (b"a".to_vec(), b"b\xffc\xffd".to_vec()),
            (b"id".to_vec(), b"one".to_vec()),
        ];
        provider.series[1].labels = vec![
            (b"__name__".to_vec(), b"m".to_vec()),
            (b"a".to_vec(), b"b".to_vec()),
            (b"c".to_vec(), b"d".to_vec()),
            (b"id".to_vec(), b"two".to_vec()),
        ];
        provider.series[1].source.sample = second_value;
        let engine = Engine::default();
        let control = Control::default();
        for delayed in [false, true] {
            let query = engine
                .compile(b"sum by(a,c)(m)", options(delayed), &control)
                .unwrap();
            let (result, _) = query
                .collect(&provider, QueryRequest::Instant { time_ms: 1000 }, &control)
                .unwrap();
            assert_eq!(result.series.len(), 1);
            assert_eq!(float_bits(&result.series[0].points[0]), 3.0f64.to_bits());
            assert_eq!(result.series[0].labels.0.len(), 1);
            assert_eq!(&*result.series[0].labels.0[0].value, b"b\xffc\xffd");
        }
        provider.assert_closed();
    }
    #[test]
    fn a_group_first_productive_later_still_precedes_a_first_step_group() {
        fn late(_: usize) -> crate::value::Sample {
            crate::value::Sample {
                time_ms: 1000,
                start_time_ms: 0,
                value: crate::value::SampleValue::Float(0.0),
            }
        }
        fn negative(_: usize) -> crate::value::Sample {
            crate::value::Sample {
                time_ms: 0,
                start_time_ms: 0,
                value: crate::value::SampleValue::Float(-0.0),
            }
        }
        fn positive(_: usize) -> crate::value::Sample {
            crate::value::Sample {
                time_ms: 0,
                start_time_ms: 0,
                value: crate::value::SampleValue::Float(0.0),
            }
        }
        let mut provider = FixtureProvider::new(1, 1, 3);
        for (series, (group, id, sample)) in provider.series.iter_mut().zip([
            (b"a", b"A", late as fn(usize) -> crate::value::Sample),
            (b"b", b"B", negative),
            (b"a", b"C", positive),
        ]) {
            series.labels = vec![
                (b"__name__".to_vec(), b"m".to_vec()),
                (b"group".to_vec(), group.to_vec()),
                (b"id".to_vec(), id.to_vec()),
            ];
            series.source.sample = sample;
        }
        let engine = Engine::default();
        let control = Control::default();
        for delayed in [false, true] {
            let query = engine
                .compile(b"min(min by(group)(m))", options(delayed), &control)
                .unwrap();
            let (result, _) = query
                .collect(
                    &provider,
                    QueryRequest::Range {
                        start_ms: 0,
                        end_ms: 1000,
                        step_ms: 1000,
                    },
                    &control,
                )
                .unwrap();
            assert_eq!(result.series.len(), 1);
            assert_eq!(
                result.series[0]
                    .points
                    .iter()
                    .map(float_bits)
                    .collect::<Vec<_>>(),
                vec![0, 0]
            );
        }
        provider.assert_closed();
    }

    #[test]
    fn delayed_cleanup_merges_disjoint_output_chunks_into_one_series() {
        let mut provider = FixtureProvider::new(2, 1, 2);
        for (series, name) in provider.series.iter_mut().zip([b"a", b"b"]) {
            series.labels = vec![
                (b"__name__".to_vec(), name.to_vec()),
                (b"job".to_vec(), b"x".to_vec()),
            ];
        }
        provider.series[0].source.sample = early_rate;
        provider.series[1].source.sample = late_rate;
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(b"rate({__name__=~\"a|b\"}[1s])", options(true), &control)
            .unwrap();
        let (result, _) = query
            .collect(
                &provider,
                QueryRequest::Range {
                    start_ms: 1000,
                    end_ms: 2000,
                    step_ms: 1000,
                },
                &control,
            )
            .unwrap();
        assert_eq!(result.series.len(), 1);
        assert_eq!(
            result.series[0]
                .points
                .iter()
                .map(|point| (point.time_ms, float_bits(point)))
                .collect::<Vec<_>>(),
            vec![(1000, 1.125f64.to_bits()), (2000, 1.125f64.to_bits())]
        );
        provider.assert_closed();
    }
    #[test]
    fn immutable_compiled_plan_allows_independent_concurrent_executions() {
        use std::sync::{Arc, Mutex, mpsc};
        struct GatedProvider {
            provider: FixtureProvider,
            entered: mpsc::SyncSender<()>,
            resume: Mutex<mpsc::Receiver<()>>,
            opens: std::sync::atomic::AtomicUsize,
        }
        impl Provider for GatedProvider {
            type Query<'a> = crate::test_storage::FixtureQuery<'a>;
            fn open(&self, control: &Control) -> Result<Self::Query<'_>, StorageError> {
                if self.opens.fetch_add(1, Ordering::Relaxed) == 0 {
                    self.entered.send(()).unwrap();
                    self.resume
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                }
                self.provider.open(control)
            }
        }
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (resume_tx, resume_rx) = mpsc::sync_channel(1);
        let provider = Arc::new(GatedProvider {
            provider: FixtureProvider::new(1, 1, 2),
            entered: entered_tx,
            resume: Mutex::new(resume_rx),
            opens: std::sync::atomic::AtomicUsize::new(0),
        });
        let engine = Engine::default();
        let control = Control::default();
        let query = Arc::new(
            engine
                .compile(b"sum(counter_total)", EngineOptions::default(), &control)
                .unwrap(),
        );
        let other_query = query.clone();
        let other_provider = provider.clone();
        let first = std::thread::spawn(move || {
            other_query.collect(
                &*other_provider,
                QueryRequest::Instant { time_ms: 1000 },
                &Control::default(),
            )
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let second = query.collect(
            &*provider,
            QueryRequest::Instant { time_ms: 1000 },
            &control,
        );
        resume_tx.send(()).unwrap();
        let first = first.join().unwrap();
        for result in [first, second] {
            let (result, _) = result.unwrap();
            assert_eq!(float_bits(&result.series[0].points[0]), 2.0f64.to_bits());
        }
        assert_eq!(engine.accounting.snapshot().peak_executions, 2);
        assert_eq!(engine.accounting.snapshot().active_executions, 0);
        provider.provider.assert_closed();
    }

    #[test]
    fn failed_execution_retains_counters_and_releases_partial_output() {
        let engine = Engine::default();
        let provider = FixtureProvider::new(10, 1, 1);
        let control = Control::default();
        let query = engine
            .compile(b"counter_total", EngineOptions::default(), &control)
            .unwrap();
        let mut sink = RecordingSink {
            fail_after: Some(2),
            ..RecordingSink::default()
        };
        let report = query.execute_reported(
            &provider,
            QueryRequest::Range {
                start_ms: 1000,
                end_ms: 10000,
                step_ms: 1000,
            },
            &control,
            &mut sink,
        );
        assert!(matches!(report.result, Err(ExecutionError::Sink(_))));
        assert_eq!(report.observations.output_samples, 1);
        assert!(report.observations.provider_calls > 0 && report.observations.samples_fetched > 0);
        assert_eq!(
            report.observations.memory[MemoryKind::ProviderHeld as usize].current,
            0
        );
        assert_eq!(
            report.observations.memory[MemoryKind::Workspace as usize].current,
            0
        );
        assert!(sink.aborted);
        provider.assert_closed();
        let cancelled = Control::default();
        cancelled.cancellation.cancel();
        let report = query.collect_reported(
            &provider,
            QueryRequest::Instant { time_ms: 1000 },
            &cancelled,
        );
        assert!(report.result.is_err());
        assert_eq!(
            report.observations.memory[MemoryKind::Result as usize].current,
            0
        );
        assert_eq!(engine.accounting.snapshot().active_executions, 0);
    }

    #[test]
    fn native_pages_copy_owned_histograms_and_account_buffer_capacities() {
        use crate::{
            histogram::{IntegerHistogram, Span},
            value::{Sample, SampleValue},
        };
        fn native(index: usize) -> Sample {
            let mut spans = Vec::with_capacity(9);
            spans.push(Span {
                offset: 0,
                length: 2,
            });
            let mut buckets = Vec::with_capacity(7);
            buckets.extend([1, 1]);
            Sample {
                time_ms: (index as i64 + 1) * 1000,
                start_time_ms: 0,
                value: SampleValue::IntegerHistogram(IntegerHistogram {
                    schema: -53,
                    count: 3,
                    sum: 5.0,
                    positive_spans: Some(spans),
                    positive_buckets: Some(buckets),
                    custom_values: Some([1.0, 3.0].into()),
                    ..IntegerHistogram::default()
                }),
            }
        }
        let mut provider = FixtureProvider::new(2, 1, 1);
        provider.series[0].source.sample = native;
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(b"counter_total[2s]", EngineOptions::default(), &control)
            .unwrap();
        let (result, observations) = query
            .collect(&provider, QueryRequest::Instant { time_ms: 2000 }, &control)
            .unwrap();
        let expected = size_of::<Sample>()
            + 9 * size_of::<Span>()
            + 7 * size_of::<i64>()
            + 2 * size_of::<f64>();
        assert_eq!(
            observations.memory[MemoryKind::ProviderHeld as usize].peak,
            expected as u64
        );
        assert_eq!(
            observations.memory[MemoryKind::ProviderHeld as usize].current,
            0
        );
        provider.assert_closed();
        assert_eq!(result.series.len(), 1);
        assert_eq!(result.series[0].points.len(), 2);
        drop(query);
        drop(provider);
        for point in result.series[0].points.iter() {
            let OwnedValue::Histogram(histogram) = &point.value else {
                panic!("expected histogram")
            };
            let value = histogram.as_ref();
            assert_eq!(value.positive_buckets, Some(&[1.0, 2.0][..]));
            assert_eq!(value.custom_values, Some(&[1.0, 3.0][..]));
        }
        drop(result);
        assert!(
            engine
                .accounting
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }
}
