// SPDX-License-Identifier: GPL-3.0-or-later
// Evaluation mechanics derive from pinned Prometheus promql/engine.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    annotations::{AnnotationBuilder, AnnotationKind},
    evaluator_roster::{GroupDescriptor, Rosters, group_key},
    frontend::{
        ast::{Expr, NodeId, ParsedExpression, ValueType, VectorSelector},
        token as tokens,
    },
    label_hash,
    numerical::compensated_add,
    range_float::{RateKind, RateValue, RateWindow, rate},
    range_reduce::{FoldKind, RangeFunction, fold},
    result::{OwnedLabels, OwnedValue},
    storage::{
        Control, Interrupted, QueryProvider, ResourceLease, ResourceOwner, SelectionRequest,
        Series, SeriesSelection, StorageError, TimeBounds, label, matches_in, scan,
        validate_labels,
    },
};
use tokens::TokenKind;

const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

#[derive(Debug, PartialEq)]
pub enum EvaluationError {
    Interrupted(Interrupted),
    Storage(StorageError),
    Unsupported(&'static str),
    Invalid(&'static str),
}
impl From<Interrupted> for EvaluationError {
    fn from(value: Interrupted) -> Self {
        Self::Interrupted(value)
    }
}
impl From<StorageError> for EvaluationError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EvaluationOptions {
    pub time_ms: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub step_ms: i64,
    pub lookback_ms: i64,
    pub start_timestamps: bool,
    pub delayed_name_removal: bool,
}

#[derive(Debug)]
pub(crate) struct EvaluatedSample {
    pub ordinal: usize,
    pub labels: OwnedLabels,
    pub value: OwnedValue,
    pub source_time_ms: i64,
    pub drop_name: bool,
}

#[derive(Clone, Copy, Debug)]
enum SelectorOperation {
    Instant,
    Rate { kind: RateKind, range_ns: i64 },
    Fold { kind: FoldKind, range_ns: i64 },
}
impl SelectorOperation {
    fn drops_name(self) -> bool {
        match self {
            Self::Instant => false,
            Self::Rate { .. } => true,
            Self::Fold { kind, .. } => !kind.preserves_name(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum MapOperation {
    Unary(TokenKind),
    Binary {
        operator: TokenKind,
        scalar: f64,
        scalar_left: bool,
        return_bool: bool,
    },
}

#[derive(Debug)]
struct FloatGroup<'a> {
    labels: &'a OwnedLabels,
    ordinal: usize,
    hash: u64,
    seen: bool,
    value: f64,
    compensation: f64,
    mean: f64,
    count: f64,
    incremental_mean: bool,
    drop_name: bool,
}
impl<'a> FloatGroup<'a> {
    fn new(group: &'a GroupDescriptor, ordinal: usize) -> Self {
        Self {
            labels: &group.labels,
            hash: group.hash,
            ordinal,
            seen: false,
            value: 0.0,
            compensation: 0.0,
            mean: 0.0,
            count: 0.0,
            incremental_mean: false,
            drop_name: false,
        }
    }
    fn add(
        &mut self,
        operator: TokenKind,
        value: f64,
        drop_name: bool,
    ) -> Result<(), EvaluationError> {
        use TokenKind::*;
        if !self.seen {
            self.seen = true;
            self.value = value;
            self.mean = value;
            self.count = 1.0;
            self.drop_name = drop_name;
            return Ok(());
        }
        self.drop_name |= drop_name;
        match operator {
            SUM => {
                (self.value, self.compensation) =
                    compensated_add(value, self.value, self.compensation)
            }
            AVG => {
                self.count += 1.0;
                if !self.incremental_mean {
                    let (next, compensation) =
                        compensated_add(value, self.value, self.compensation);
                    if !next.is_infinite() {
                        self.value = next;
                        self.compensation = compensation;
                        return Ok(());
                    }
                    self.incremental_mean = true;
                    self.mean = self.value / (self.count - 1.0);
                    self.compensation /= self.count - 1.0;
                }
                let q = (self.count - 1.0) / self.count;
                (self.mean, self.compensation) =
                    compensated_add(value / self.count, q * self.mean, q * self.compensation);
            }
            COUNT => self.count += 1.0,
            MIN => {
                if self.value > value || self.value.is_nan() {
                    self.value = value;
                }
            }
            MAX => {
                if self.value < value || self.value.is_nan() {
                    self.value = value;
                }
            }
            GROUP => (),
            _ => return Err(EvaluationError::Unsupported("aggregation kernel")),
        }
        Ok(())
    }
    fn finish(
        self,
        operator: TokenKind,
        time_ms: i64,
        account: &QueryAccounting,
    ) -> EvaluatedSample {
        use TokenKind::*;
        let value = match operator {
            SUM => self.value + self.compensation,
            AVG if self.incremental_mean => self.mean + self.compensation,
            AVG => self.value / self.count + self.compensation / self.count,
            COUNT => self.count,
            GROUP => 1.0,
            _ => self.value,
        };
        let views = self.labels.views(account);
        let mut labels = OwnedLabels::copy_from(&views, account, MemoryKind::Workspace);
        // The source label builder removes empty values after hashing the raw group key.
        labels.retain_labels(|label| !label.value.is_empty());
        EvaluatedSample {
            ordinal: self.ordinal,
            labels,
            value: OwnedValue::Float(value),
            source_time_ms: time_ms,
            drop_name: self.drop_name,
        }
    }
}

enum State<'a, Q: QueryProvider + 'a> {
    Enter,
    Pass,
    Map {
        operation: MapOperation,
        seen: AccountedVec<u64>,
    },
    Selection {
        selection: Q::Selection<'a>,
        resources: ResourceLease,
        selector: NodeId,
        operation: SelectorOperation,
        ordinal: usize,
    },
    Aggregate {
        groups: AccountedVec<FloatGroup<'a>>,
        collecting: bool,
    },
    Scalar {
        count: usize,
        value: Option<f64>,
        collecting: bool,
    },
    ScalarBinary {
        left: Option<f64>,
        right: Option<f64>,
        phase: u8,
    },
    MixedBinary {
        scalar: Option<f64>,
        scalar_node: NodeId,
        vector_node: NodeId,
        scalar_left: bool,
        collecting: bool,
    },
}
struct Frame<'a, Q: QueryProvider + 'a> {
    node: NodeId,
    state: State<'a, Q>,
}

pub(crate) struct EvaluationMetadata<'a> {
    pub(crate) rosters: &'a Rosters,
    pub(crate) annotations: Option<&'a mut AnnotationBuilder>,
}
pub(crate) struct Machine<'a, Q: QueryProvider + 'a> {
    query: &'a Q,
    expression: &'a ParsedExpression,
    options: EvaluationOptions,
    control: &'a Control,
    account: &'a QueryAccounting,
    frames: AccountedVec<Frame<'a, Q>>,
    rosters: &'a Rosters,
    diagnostic_node: NodeId,
    annotations: Option<&'a mut AnnotationBuilder>,
}

impl<'a, Q: QueryProvider + 'a> Machine<'a, Q> {
    pub(crate) fn new(
        query: &'a Q,
        expression: &'a ParsedExpression,
        root: NodeId,
        options: EvaluationOptions,
        control: &'a Control,
        account: &'a QueryAccounting,
        metadata: EvaluationMetadata<'a>,
    ) -> Self {
        let mut frames = AccountedVec::new(account, MemoryKind::Workspace);
        frames.push(Frame {
            node: root,
            state: State::Enter,
        });
        Self {
            query,
            expression,
            options,
            control,
            account,
            frames,
            rosters: metadata.rosters,
            diagnostic_node: root,
            annotations: metadata.annotations,
        }
    }
    fn push(&mut self, node: NodeId) {
        self.frames.push(Frame {
            node,
            state: State::Enter,
        });
    }
    fn map(&self, operation: MapOperation) -> State<'a, Q> {
        State::Map {
            operation,
            seen: AccountedVec::new(self.account, MemoryKind::Workspace),
        }
    }
    fn sample(&self, value: f64) -> EvaluatedSample {
        EvaluatedSample {
            ordinal: 0,
            labels: OwnedLabels::new(self.account, MemoryKind::Workspace),
            value: OwnedValue::Float(value),
            source_time_ms: self.options.time_ms,
            drop_name: false,
        }
    }
    fn selector(&self, node: NodeId) -> &VectorSelector {
        let Expr::Selector(selector) = &self.expression.arena[node].expression else {
            unreachable!("typed matrix selector")
        };
        selector
    }
    fn lookup(&self, selector: &VectorSelector) -> Result<i64, EvaluationError> {
        if selector.modifiers.original_offset_expression.is_some() {
            return Err(EvaluationError::Unsupported("duration offset expression"));
        }
        let time = selector
            .modifiers
            .timestamp
            .unwrap_or(match selector.modifiers.start_or_end {
                Some(TokenKind::START) => self.options.start_ms,
                Some(TokenKind::END) => self.options.end_ms,
                _ => self.options.time_ms,
            });
        Ok(time.wrapping_sub(selector.modifiers.original_offset / 1_000_000))
    }
    fn select(
        &self,
        selector_node: NodeId,
        operation: SelectorOperation,
    ) -> Result<State<'a, Q>, EvaluationError> {
        let selector = self.selector(selector_node);
        if selector.anchored || selector.smoothed {
            return Err(EvaluationError::Unsupported("extended selector reducer"));
        }
        let end_ms = self.lookup(selector)?;
        let duration = match operation {
            SelectorOperation::Instant => self.options.lookback_ms,
            SelectorOperation::Rate { range_ns, .. } | SelectorOperation::Fold { range_ns, .. } => {
                range_ns / 1_000_000
            }
        };
        let fixed =
            selector.modifiers.timestamp.is_some() || selector.modifiers.start_or_end.is_some();
        let offset = selector.modifiers.original_offset / 1_000_000;
        // Use the same conservative roster at every step; cursor reads remain step-local.
        let bounds = TimeBounds {
            start_ms: (if fixed {
                end_ms
            } else {
                self.options.start_ms.wrapping_sub(offset)
            })
            .wrapping_sub(duration)
            .wrapping_add(1),
            end_ms: if fixed {
                end_ms
            } else {
                self.options.end_ms.wrapping_sub(offset)
            },
        };
        self.control.check()?;
        self.account.provider_call();
        let selection = self.query.select(
            SelectionRequest {
                matchers: &selector.matchers,
                bounds,
                accounting: Some(self.account),
            },
            self.control,
        )?;
        let resources = ResourceLease::new(self.account, selection.resources());
        Ok(State::Selection {
            selection,
            resources,
            selector: selector_node,
            operation,
            ordinal: 0,
        })
    }
    fn enter(&mut self, index: usize) -> Result<Option<EvaluatedSample>, EvaluationError> {
        let node = self.frames[index].node;
        let expression = &self.expression.arena[node].expression;
        match expression {
            Expr::Number { value, .. } => {
                let value = *value;
                self.frames.pop();
                Ok(Some(self.sample(value)))
            }
            Expr::Paren(child) => {
                self.frames[index].state = State::Pass;
                self.push(*child);
                Ok(None)
            }
            Expr::Unary {
                operator,
                expression,
            } => {
                self.frames[index].state = self.map(MapOperation::Unary(*operator));
                self.push(*expression);
                Ok(None)
            }
            Expr::Selector(_) => {
                self.frames[index].state = self.select(node, SelectorOperation::Instant)?;
                Ok(None)
            }
            Expr::Aggregate {
                expression: Some(child),
                parameter: None,
                operator,
                ..
            } => {
                if !matches!(
                    operator,
                    TokenKind::SUM
                        | TokenKind::AVG
                        | TokenKind::COUNT
                        | TokenKind::MIN
                        | TokenKind::MAX
                        | TokenKind::GROUP
                ) {
                    return Err(EvaluationError::Unsupported("aggregation kernel"));
                }
                let mut groups = AccountedVec::new(self.account, MemoryKind::Workspace);
                for (ordinal, labels) in self
                    .rosters
                    .groups(node)
                    .ok_or(EvaluationError::Invalid("aggregation roster is missing"))?
                    .iter()
                    .enumerate()
                {
                    groups.push(FloatGroup::new(labels, ordinal));
                }
                self.frames[index].state = State::Aggregate {
                    groups,
                    collecting: true,
                };
                self.push(*child);
                Ok(None)
            }
            Expr::Binary { left, right, .. } => {
                let left_scalar = self.expression.arena[*left].value_type == ValueType::Scalar;
                let right_scalar = self.expression.arena[*right].value_type == ValueType::Scalar;
                if left_scalar && right_scalar {
                    self.frames[index].state = State::ScalarBinary {
                        left: None,
                        right: None,
                        phase: 0,
                    };
                    self.push(*left);
                } else if left_scalar || right_scalar {
                    let (scalar_node, vector_node) = if left_scalar {
                        (*left, *right)
                    } else {
                        (*right, *left)
                    };
                    self.frames[index].state = State::MixedBinary {
                        scalar: None,
                        scalar_node,
                        vector_node,
                        scalar_left: left_scalar,
                        collecting: true,
                    };
                    self.push(scalar_node);
                } else {
                    return Err(EvaluationError::Unsupported("vector matching reducer"));
                }
                Ok(None)
            }
            Expr::Call {
                function: Some(function),
                arguments,
            } => match function.name {
                "time" => {
                    self.frames.pop();
                    Ok(Some(self.sample(self.options.time_ms as f64 / 1000.0)))
                }
                "vector" => {
                    self.frames[index].state = State::Pass;
                    self.push(arguments[0]);
                    Ok(None)
                }
                "scalar" => {
                    self.frames[index].state = State::Scalar {
                        count: 0,
                        value: None,
                        collecting: true,
                    };
                    self.push(arguments[0]);
                    Ok(None)
                }
                _ => {
                    let function = RangeFunction::parse(function.name)
                        .ok_or(EvaluationError::Unsupported("function reducer"))?;
                    let argument = self.expression.arena.unparenthesized(arguments[0]);
                    let Expr::Matrix {
                        selector,
                        range,
                        range_expression: None,
                    } = self.expression.arena[argument].expression
                    else {
                        return Err(EvaluationError::Unsupported("subquery range reducer"));
                    };
                    let operation = match function {
                        RangeFunction::Rate(kind) => SelectorOperation::Rate {
                            kind,
                            range_ns: range,
                        },
                        RangeFunction::Fold(kind) => SelectorOperation::Fold {
                            kind,
                            range_ns: range,
                        },
                    };
                    self.frames[index].state = self.select(selector, operation)?;
                    Ok(None)
                }
            },
            _ => Err(EvaluationError::Unsupported("expression reducer")),
        }
    }

    fn propagate(
        &mut self,
        mut sample: EvaluatedSample,
        origin: usize,
    ) -> Result<Option<EvaluatedSample>, EvaluationError> {
        for index in (0..origin).rev() {
            self.control.check()?;
            self.account.work(1);
            let node = self.frames[index].node;
            match &mut self.frames[index].state {
                State::Pass => (),
                State::Map { operation, seen } => {
                    let OwnedValue::Float(value) = sample.value else {
                        return Err(EvaluationError::Unsupported("histogram map kernel"));
                    };
                    let (result, drop_name) = match *operation {
                        MapOperation::Unary(TokenKind::SUB) => (-value, true),
                        MapOperation::Unary(_) => (value, false),
                        MapOperation::Binary {
                            operator,
                            scalar,
                            scalar_left,
                            return_bool,
                        } => {
                            let (left, right) = if scalar_left {
                                (scalar, value)
                            } else {
                                (value, scalar)
                            };
                            let result = scalar_binary(operator, left, right)?;
                            if operator.is_comparison() && !return_bool {
                                if result == 0.0 {
                                    return Ok(None);
                                }
                                (value, false)
                            } else {
                                (result, !operator.is_comparison() || return_bool)
                            }
                        }
                    };
                    sample.value = OwnedValue::Float(result);
                    sample.drop_name |= drop_name;
                    if drop_name && !self.options.delayed_name_removal {
                        sample.labels.retain(|name| !metadata(name));
                    }
                    sample.source_time_ms = self.options.time_ms;
                    let duplicate_check = match operation {
                        MapOperation::Unary(TokenKind::SUB) => !self.options.delayed_name_removal,
                        MapOperation::Unary(_) => false,
                        MapOperation::Binary { .. } => {
                            !self.options.delayed_name_removal
                                || self.options.start_ms != self.options.end_ms
                        }
                    };
                    if duplicate_check {
                        remember_unique(seen, &sample.labels, self.account, self.control)?;
                    }
                }
                State::Aggregate {
                    groups,
                    collecting: true,
                } => {
                    let Expr::Aggregate {
                        operator,
                        grouping,
                        without,
                        ..
                    } = &self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    let OwnedValue::Float(value) = sample.value else {
                        return Err(EvaluationError::Unsupported("histogram aggregation kernel"));
                    };
                    let views = sample.labels.views(self.account);
                    let mut labels =
                        OwnedLabels::copy_from(&views, self.account, MemoryKind::Workspace);
                    group_key(&mut labels, grouping.as_deref(), *without);
                    let hash = if !without && grouping.as_ref().is_none_or(|names| names.is_empty())
                    {
                        0
                    } else {
                        label_hash::grouping(&labels, self.control, self.account)?
                    };
                    if let Some(group) = groups.iter_mut().find(|group| group.hash == hash) {
                        group.add(*operator, value, sample.drop_name)?;
                    } else {
                        return Err(EvaluationError::Invalid("aggregation group is missing"));
                    }
                    return Ok(None);
                }
                State::Scalar {
                    count,
                    value,
                    collecting: true,
                } => {
                    if let OwnedValue::Float(f) = sample.value {
                        *count = (*count + 1).min(2);
                        *value = if *count == 1 { Some(f) } else { None };
                    }
                    return Ok(None);
                }
                State::ScalarBinary { left, right, phase } => {
                    let OwnedValue::Float(value) = sample.value else {
                        unreachable!("scalar value")
                    };
                    if *phase == 0 {
                        *left = Some(value);
                    } else {
                        *right = Some(value);
                    }
                    return Ok(None);
                }
                State::MixedBinary {
                    scalar,
                    collecting: true,
                    ..
                } => {
                    let OwnedValue::Float(value) = sample.value else {
                        unreachable!("scalar value")
                    };
                    *scalar = Some(value);
                    return Ok(None);
                }
                _ => unreachable!("active parent consumes its child"),
            }
        }
        Ok(Some(sample))
    }

    pub(crate) fn next(&mut self) -> Result<Option<EvaluatedSample>, EvaluationError> {
        loop {
            self.control.check()?;
            let Some(index) = self.frames.len().checked_sub(1) else {
                return Ok(None);
            };
            self.account.work(1);
            if matches!(self.frames[index].state, State::Enter) {
                if let Some(sample) = self.enter(index)?
                    && let Some(sample) = self.propagate(sample, self.frames.len())?
                {
                    return Ok(Some(sample));
                }
                continue;
            }
            let node = self.frames[index].node;
            match &mut self.frames[index].state {
                State::Pass | State::Map { .. } => {
                    self.frames.pop();
                }
                State::Selection {
                    selection,
                    resources,
                    selector,
                    operation,
                    ordinal,
                } => {
                    self.account.provider_call();
                    let series = selection.next(self.control);
                    resources.update(selection.resources());
                    let series = series?;
                    let Some(series) = series else {
                        self.frames.pop();
                        continue;
                    };
                    let series_ordinal = *ordinal;
                    *ordinal += 1;
                    let resources = series.resources();
                    let series = ResourceOwner::new(series, resources, self.account);
                    let series = &series.value;
                    let series_labels = series.label_views(self.account);
                    validate_labels(&series_labels)?;
                    let selector_node = *selector;
                    let operation = *operation;
                    let selector = self.selector(selector_node);
                    if !matches_in(&series_labels, &selector.matchers, Some(self.account)) {
                        continue;
                    }
                    let end_ms = self.lookup(selector)?;
                    let value = match operation {
                        SelectorOperation::Instant => {
                            let bounds = TimeBounds {
                                start_ms: end_ms
                                    .wrapping_sub(self.options.lookback_ms)
                                    .wrapping_add(1),
                                end_ms,
                            };
                            let mut selected = None;
                            scan(series, bounds, self.control, self.account, |sample| {
                                selected = if sample.value.is_stale() {
                                    None
                                } else {
                                    Some((
                                        sample.time_ms,
                                        OwnedValue::copy_from(
                                            sample.value,
                                            self.account,
                                            MemoryKind::Workspace,
                                        ),
                                    ))
                                };
                                Ok(())
                            })?;
                            selected
                        }
                        SelectorOperation::Rate { kind, range_ns } => {
                            let result = rate(
                                series,
                                RateWindow {
                                    start_ms: end_ms.wrapping_sub(range_ns / 1_000_000),
                                    end_ms,
                                    range_ns,
                                    start_timestamps: self.options.start_timestamps,
                                },
                                kind,
                                self.control,
                                self.account,
                            )?;
                            if node == self.diagnostic_node
                                && let Some(annotations) = &mut self.annotations
                            {
                                let Expr::Call { arguments, .. } =
                                    &self.expression.arena[node].expression
                                else {
                                    unreachable!()
                                };
                                let argument = self.expression.arena.unparenthesized(arguments[0]);
                                let span = self.expression.arena[argument].span;
                                let metric = label(&series_labels, b"__name__");
                                if result.start_time_overlap {
                                    annotations.add(
                                        AnnotationKind::StartTimeOverlap,
                                        metric,
                                        span,
                                        &self.expression.source,
                                        self.account,
                                    );
                                }
                                if matches!(result.value, RateValue::Mixed) {
                                    annotations.add(
                                        AnnotationKind::MixedFloatsHistograms,
                                        metric,
                                        span,
                                        &self.expression.source,
                                        self.account,
                                    );
                                }
                                if kind != RateKind::Delta
                                    && matches!(result.value, RateValue::Float(_))
                                    && !metric.is_empty()
                                    && ![
                                        &b"_total"[..],
                                        &b"_sum"[..],
                                        &b"_count"[..],
                                        &b"_bucket"[..],
                                    ]
                                    .iter()
                                    .any(|suffix| metric.ends_with(suffix))
                                {
                                    annotations.add(
                                        AnnotationKind::PossibleNonCounter,
                                        metric,
                                        span,
                                        &self.expression.source,
                                        self.account,
                                    );
                                }
                            }
                            match result.value {
                                RateValue::Float(value) => {
                                    Some((self.options.time_ms, OwnedValue::Float(value)))
                                }
                                RateValue::Empty => None,
                                RateValue::Mixed => None,
                                RateValue::Histogram => {
                                    return Err(EvaluationError::Unsupported(
                                        "histogram rate kernel",
                                    ));
                                }
                            }
                        }
                        SelectorOperation::Fold { kind, range_ns } => {
                            let Some(start_ms) =
                                end_ms.wrapping_sub(range_ns / 1_000_000).checked_add(1)
                            else {
                                continue;
                            };
                            let result = fold(
                                series,
                                TimeBounds { start_ms, end_ms },
                                kind,
                                self.options.start_timestamps,
                                self.control,
                                self.account,
                            )?;
                            if node == self.diagnostic_node
                                && let Some(annotations) = &mut self.annotations
                            {
                                let Expr::Call { arguments, .. } =
                                    &self.expression.arena[node].expression
                                else {
                                    unreachable!()
                                };
                                let argument = self.expression.arena.unparenthesized(arguments[0]);
                                for kind in result.annotations.into_iter().flatten() {
                                    annotations.add(
                                        kind,
                                        label(&series_labels, b"__name__"),
                                        self.expression.arena[argument].span,
                                        &self.expression.source,
                                        self.account,
                                    );
                                }
                            }
                            result.value.map(|value| (self.options.time_ms, value))
                        }
                    };
                    if let Some((source_time_ms, value)) = value {
                        let mut labels = OwnedLabels::copy_from(
                            &series_labels,
                            self.account,
                            MemoryKind::Workspace,
                        );
                        let drop_name = operation.drops_name();
                        if drop_name && !self.options.delayed_name_removal {
                            labels.retain(|name| !metadata(name));
                        }
                        let sample = EvaluatedSample {
                            ordinal: series_ordinal,
                            labels,
                            value,
                            source_time_ms,
                            drop_name,
                        };
                        if let Some(sample) = self.propagate(sample, index)? {
                            return Ok(Some(sample));
                        }
                    }
                }
                State::Aggregate { groups, collecting } => {
                    if *collecting {
                        *collecting = false;
                        groups.reverse();
                    }
                    if let Some(group) = groups.pop() {
                        if !group.seen {
                            continue;
                        }
                        let Expr::Aggregate { operator, .. } =
                            self.expression.arena[node].expression
                        else {
                            unreachable!()
                        };
                        let sample = group.finish(operator, self.options.time_ms, self.account);
                        if let Some(sample) = self.propagate(sample, index)? {
                            return Ok(Some(sample));
                        }
                    } else {
                        self.frames.pop();
                    }
                }
                State::Scalar {
                    count,
                    value,
                    collecting,
                } => {
                    *collecting = false;
                    let value = if *count == 1 {
                        value.unwrap_or(NAN)
                    } else {
                        NAN
                    };
                    self.frames.pop();
                    let sample = self.sample(value);
                    if let Some(sample) = self.propagate(sample, self.frames.len())? {
                        return Ok(Some(sample));
                    }
                }
                State::ScalarBinary { left, right, phase } => {
                    let Expr::Binary {
                        operator,
                        right: right_node,
                        ..
                    } = self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    if *phase == 0 {
                        *phase = 1;
                        self.push(right_node);
                    } else {
                        let result = scalar_binary(
                            operator,
                            left.expect("left scalar"),
                            right.expect("right scalar"),
                        )?;
                        self.frames.pop();
                        let sample = self.sample(result);
                        if let Some(sample) = self.propagate(sample, self.frames.len())? {
                            return Ok(Some(sample));
                        }
                    }
                }
                State::MixedBinary {
                    scalar,
                    scalar_node,
                    vector_node,
                    scalar_left,
                    collecting,
                } => {
                    let _ = scalar_node;
                    let _ = collecting;
                    let Expr::Binary {
                        operator,
                        return_bool,
                        ..
                    } = self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    let operation = MapOperation::Binary {
                        operator,
                        scalar: scalar.expect("scalar operand"),
                        scalar_left: *scalar_left,
                        return_bool,
                    };
                    let child = *vector_node;
                    self.frames[index].state = self.map(operation);
                    self.push(child);
                }
                State::Enter => unreachable!(),
            }
        }
    }
}

pub(crate) fn remember_unique(
    seen: &mut AccountedVec<u64>,
    labels: &OwnedLabels,
    account: &QueryAccounting,
    control: &Control,
) -> Result<(), EvaluationError> {
    let hash = label_hash::full(labels, control, account)?;
    if seen.contains(&hash) {
        return Err(EvaluationError::Invalid(
            "vector cannot contain metrics with the same labelset",
        ));
    }
    seen.push(hash);
    Ok(())
}

pub(crate) fn metadata(name: &[u8]) -> bool {
    matches!(name, b"__name__" | b"__type__" | b"__unit__")
}

fn scalar_binary(operator: TokenKind, left: f64, right: f64) -> Result<f64, EvaluationError> {
    use TokenKind::*;
    Ok(match operator {
        ADD => left + right,
        SUB => left - right,
        MUL => left * right,
        DIV => left / right,
        EQLC => f64::from(left == right),
        NEQ => f64::from(left != right),
        GTR => f64::from(left > right),
        LSS => f64::from(left < right),
        GTE => f64::from(left >= right),
        LTE => f64::from(left <= right),
        _ => return Err(EvaluationError::Unsupported("binary numerical kernel")),
    })
}
