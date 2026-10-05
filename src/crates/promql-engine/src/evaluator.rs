// SPDX-License-Identifier: GPL-3.0-or-later
// Evaluation mechanics derive from pinned Prometheus promql/engine.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    aggregation::Group,
    annotations::{AnnotationBuilder, AnnotationKind},
    evaluator_roster::{Descriptor, Rosters, group_key},
    frontend::{
        ast::{Expr, NodeId, ParsedExpression, ValueType, VectorSelector},
        token as tokens,
    },
    histogram_functions,
    histogram_stats::StatisticsWindow,
    label_hash,
    query_preparation::Prepared,
    range_float::{RateKind, RateValue, RateWindow, rate},
    range_reduce::{FoldKind, FoldOptions, RangeFunction, fold},
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
    Preparation(AccountedVec<u8>),
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
    Pointwise(PointwiseFunction),
    Binary {
        operator: TokenKind,
        scalar: f64,
        scalar_left: bool,
        return_bool: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PointwiseFunction {
    Timestamp,
    HistogramCount,
    HistogramSum,
    HistogramAvg,
    HistogramStdVar,
    HistogramStdDev,
}
impl PointwiseFunction {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "timestamp" => Self::Timestamp,
            "histogram_count" => Self::HistogramCount,
            "histogram_sum" => Self::HistogramSum,
            "histogram_avg" => Self::HistogramAvg,
            "histogram_stdvar" => Self::HistogramStdVar,
            "histogram_stddev" => Self::HistogramStdDev,
            _ => return None,
        })
    }
    fn apply(
        self,
        sample: &EvaluatedSample,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<Option<f64>, EvaluationError> {
        if matches!(self, Self::Timestamp) {
            return Ok(Some(sample.source_time_ms as f64 / 1000.0));
        }
        let OwnedValue::Histogram(histogram) = &sample.value else {
            return Ok(None);
        };
        let histogram = histogram.as_ref();
        let poll = || {
            control.check()?;
            account.work(1);
            Ok::<(), EvaluationError>(())
        };
        Ok(Some(match self {
            Self::HistogramCount => histogram_functions::histogram_count(histogram),
            Self::HistogramSum => histogram_functions::histogram_sum(histogram),
            Self::HistogramAvg => histogram_functions::histogram_avg(histogram),
            Self::HistogramStdVar => histogram_functions::histogram_stdvar_with(histogram, poll)?,
            Self::HistogramStdDev => crate::numerical_math::sqrt(
                histogram_functions::histogram_stdvar_with(histogram, poll)?,
            ),
            Self::Timestamp => unreachable!(),
        }))
    }
    fn drops_metadata(self) -> bool {
        matches!(self, Self::Timestamp)
    }
}

enum State<'a, Q: QueryProvider + 'a> {
    Enter,
    OrderedRows {
        descriptors: &'a [Descriptor],
        sources: &'a [usize],
        position: usize,
        running: bool,
        phase: u8,
    },
    Pass,
    Map {
        operation: MapOperation,
        seen: AccountedVec<(u64, u8)>,
    },
    Selection {
        selection: Q::Selection<'a>,
        resources: ResourceLease,
        selector: NodeId,
        operation: SelectorOperation,
        ordinal: usize,
    },
    Aggregate {
        groups: AccountedVec<Group<'a>>,
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
    filter: RowFilter<'a>,
    bypass_order: bool,
    mode: OutputMode,
}

#[derive(Clone, Copy)]
enum OutputMode {
    Matrix,
    Vector,
}

fn advance_row(
    position: &mut usize,
    phase: &mut u8,
    descriptor: &Descriptor,
    mode: OutputMode,
    time_ms: i64,
    matched: bool,
) {
    // A float/histogram collision leaves the histogram head unconsumed in nextValues.
    if *phase == 0
        && descriptor.collision_time_ms.is_some_and(|collision| {
            matches!(mode, OutputMode::Matrix) || (!matched && time_ms < collision)
        })
    {
        *phase = 1;
    } else {
        *position += 1;
        *phase = 0;
    }
}

#[derive(Clone, Copy)]
enum RowFilter<'a> {
    All,
    Ordinals(&'a [usize]),
}
impl RowFilter<'_> {
    fn accepts(self, ordinal: usize) -> bool {
        match self {
            Self::All => true,
            Self::Ordinals(values) => values.contains(&ordinal),
        }
    }
}

pub(crate) struct EvaluationMetadata<'a> {
    pub(crate) prepared: &'a Prepared,
    pub(crate) rosters: &'a Rosters,
    pub(crate) invariants: &'a [bool],
    pub(crate) timestamp_selectors: &'a [bool],
    pub(crate) data_bounds: TimeBounds,
    pub(crate) annotations: Option<&'a mut AnnotationBuilder>,
}
pub(crate) struct Machine<'a, Q: QueryProvider + 'a> {
    query: &'a Q,
    expression: &'a ParsedExpression,
    prepared: &'a Prepared,
    options: EvaluationOptions,
    control: &'a Control,
    account: &'a QueryAccounting,
    frames: AccountedVec<Frame<'a, Q>>,
    rosters: &'a Rosters,
    invariants: &'a [bool],
    timestamp_selectors: &'a [bool],
    data_bounds: TimeBounds,
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
            filter: RowFilter::All,
            bypass_order: false,
            mode: OutputMode::Matrix,
        });
        Self {
            query,
            expression,
            prepared: metadata.prepared,
            options,
            control,
            account,
            frames,
            rosters: metadata.rosters,
            invariants: metadata.invariants,
            timestamp_selectors: metadata.timestamp_selectors,
            data_bounds: metadata.data_bounds,
            diagnostic_node: root,
            annotations: metadata.annotations,
        }
    }
    fn push(&mut self, node: NodeId) {
        let filter = self
            .frames
            .last()
            .map_or(RowFilter::All, |frame| frame.filter);
        self.push_filtered(node, filter, false, OutputMode::Vector);
    }
    fn push_matrix(&mut self, node: NodeId) {
        let filter = self
            .frames
            .last()
            .map_or(RowFilter::All, |frame| frame.filter);
        self.push_filtered(node, filter, false, OutputMode::Matrix);
    }
    fn push_all(&mut self, node: NodeId) {
        self.push_filtered(node, RowFilter::All, false, OutputMode::Vector);
    }
    fn push_filtered(
        &mut self,
        node: NodeId,
        filter: RowFilter<'a>,
        bypass_order: bool,
        mode: OutputMode,
    ) {
        self.frames.push(Frame {
            node,
            state: State::Enter,
            filter,
            bypass_order,
            mode,
        });
    }
    pub(crate) fn restrict(&mut self, ordinals: &'a [usize]) {
        self.frames[0].filter = RowFilter::Ordinals(ordinals);
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
    fn lookup(&self, node: NodeId) -> i64 {
        let selector = self.selector(node);
        let time = self
            .prepared
            .timestamp(&selector.modifiers)
            .unwrap_or(self.options.time_ms);
        time.wrapping_sub(
            self.prepared
                .offset(node, selector.modifiers.original_offset)
                / 1_000_000,
        )
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
        let end_ms = self.lookup(selector_node);
        let duration = match operation {
            SelectorOperation::Instant => self.options.lookback_ms,
            SelectorOperation::Rate { range_ns, .. } | SelectorOperation::Fold { range_ns, .. } => {
                range_ns / 1_000_000
            }
        };
        let fixed =
            selector.modifiers.timestamp.is_some() || selector.modifiers.start_or_end.is_some();
        let offset = self
            .prepared
            .offset(selector_node, selector.modifiers.original_offset)
            / 1_000_000;
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
        if !self.frames[index].bypass_order
            && let Some((descriptors, sources)) = self.rosters.ordered(node)
        {
            self.frames[index].state = State::OrderedRows {
                descriptors,
                sources,
                position: 0,
                running: false,
                phase: 0,
            };
            return Ok(None);
        }
        let expression = &self.expression.arena[node].expression;
        match expression {
            Expr::Number { value, .. } => {
                let value = *value;
                self.frames.pop();
                Ok(Some(self.sample(value)))
            }
            Expr::Paren(child) => {
                self.frames[index].state = State::Pass;
                self.push_matrix(*child);
                Ok(None)
            }
            Expr::Unary {
                operator,
                expression,
            } => {
                self.frames[index].state = self.map(MapOperation::Unary(*operator));
                self.push_matrix(*expression);
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
                        | TokenKind::STDVAR
                        | TokenKind::STDDEV
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
                    if !self.frames[index].filter.accepts(ordinal) {
                        continue;
                    }
                    groups.push(Group::new(labels, ordinal));
                }
                self.frames[index].state = State::Aggregate {
                    groups,
                    collecting: true,
                };
                self.push_all(*child);
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
                    self.push_all(*left);
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
                    self.push_all(scalar_node);
                } else {
                    return Err(EvaluationError::Unsupported("vector matching reducer"));
                }
                Ok(None)
            }
            Expr::Call {
                function: Some(function),
                arguments,
            } => match function.name {
                "start" | "end" | "range" | "step" => {
                    self.frames.pop();
                    Ok(Some(
                        self.sample(
                            self.prepared
                                .context
                                .scalar(function.name)
                                .expect("context function"),
                        ),
                    ))
                }
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
                    self.push_all(arguments[0]);
                    Ok(None)
                }
                _ => {
                    if let Some(function) = PointwiseFunction::parse(function.name) {
                        self.frames[index].state = self.map(MapOperation::Pointwise(function));
                        self.push(arguments[0]);
                        return Ok(None);
                    }
                    let function = RangeFunction::parse(function.name)
                        .ok_or(EvaluationError::Unsupported("function reducer"))?;
                    let argument = self.expression.arena.unparenthesized(arguments[0]);
                    let Expr::Matrix {
                        selector, range, ..
                    } = self.expression.arena[argument].expression
                    else {
                        return Err(EvaluationError::Unsupported("subquery range reducer"));
                    };
                    let range = self.prepared.range(argument, range);
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
            let filter = self.frames[index].filter;
            let mode = self.frames[index].mode;
            match &mut self.frames[index].state {
                State::Pass => (),
                State::OrderedRows {
                    descriptors,
                    position,
                    running,
                    phase,
                    ..
                } => {
                    let descriptor = &descriptors[*position];
                    let kind = if descriptor.collision_time_ms.is_none() {
                        0
                    } else if *phase == 0 {
                        1
                    } else {
                        2
                    };
                    if !filter.accepts(descriptor.ordinal)
                        || kind != 0 && sample.value.kind_bit() != kind
                        || label_hash::full(&sample.labels, self.control, self.account)?
                            != descriptor.hash
                    {
                        return Ok(None);
                    }
                    sample.ordinal = descriptor.ordinal;
                    sample.drop_name = descriptor.drop_name;
                    if !sample.labels.same(&descriptor.labels) {
                        sample.labels = OwnedLabels::copy_from(
                            &descriptor.labels.views(self.account),
                            self.account,
                            MemoryKind::Workspace,
                        );
                    }
                    advance_row(
                        position,
                        phase,
                        descriptor,
                        mode,
                        self.options.time_ms,
                        true,
                    );
                    *running = false;
                    self.frames.truncate(index + 1);
                }
                State::Map { operation, seen } => {
                    let (result, drop_name) = match *operation {
                        MapOperation::Pointwise(function) => {
                            let Some(result) =
                                function.apply(&sample, self.account, self.control)?
                            else {
                                return Ok(None);
                            };
                            (OwnedValue::Float(result), true)
                        }
                        MapOperation::Unary(operator) => match sample.value {
                            OwnedValue::Float(value) => {
                                if operator == TokenKind::SUB {
                                    (OwnedValue::Float(-value), true)
                                } else {
                                    (OwnedValue::Float(value), false)
                                }
                            }
                            OwnedValue::Histogram(mut histogram) => {
                                let negative = operator == TokenKind::SUB;
                                if negative {
                                    histogram.normalize_copy_fields();
                                    histogram.mul_checked(-1.0, self.control, self.account)?;
                                }
                                (OwnedValue::Histogram(histogram), negative)
                            }
                        },
                        MapOperation::Binary {
                            operator,
                            scalar,
                            scalar_left,
                            return_bool,
                        } => match sample.value {
                            OwnedValue::Float(value) => {
                                let (left, right) = if scalar_left {
                                    (scalar, value)
                                } else {
                                    (value, scalar)
                                };
                                let result = scalar_binary(
                                    operator,
                                    left,
                                    right,
                                    self.control,
                                    self.account,
                                )?;
                                if operator.is_comparison() && !return_bool {
                                    if result == 0.0 {
                                        return Ok(None);
                                    }
                                    (OwnedValue::Float(value), false)
                                } else {
                                    (
                                        OwnedValue::Float(result),
                                        !operator.is_comparison() || return_bool,
                                    )
                                }
                            }
                            OwnedValue::Histogram(mut histogram) => {
                                match operator {
                                    TokenKind::MUL => {
                                        histogram.normalize_copy_fields();
                                        histogram.mul_checked(
                                            scalar,
                                            self.control,
                                            self.account,
                                        )?;
                                    }
                                    TokenKind::DIV if !scalar_left => {
                                        histogram.normalize_copy_fields();
                                        histogram.div_checked(
                                            scalar,
                                            self.control,
                                            self.account,
                                        )?;
                                    }
                                    TokenKind::TRIM_LOWER | TokenKind::TRIM_UPPER
                                        if !scalar_left =>
                                    {
                                        return Err(EvaluationError::Unsupported(
                                            "histogram map kernel",
                                        ));
                                    }
                                    _ => {
                                        if node == self.diagnostic_node
                                            && let Some(annotations) = &mut self.annotations
                                        {
                                            annotations.add(
                                                AnnotationKind::IncompatibleBinaryTypes {
                                                    left_histogram: !scalar_left,
                                                    operator,
                                                    right_histogram: scalar_left,
                                                },
                                                b"",
                                                self.expression.arena[node].span,
                                                &self.expression.source,
                                                self.account,
                                            );
                                        }
                                        return Ok(None);
                                    }
                                }
                                histogram.compact_checked(self.control, self.account)?;
                                (OwnedValue::Histogram(histogram), true)
                            }
                        },
                    };
                    sample.value = result;
                    sample.drop_name |= drop_name;
                    if drop_name && !self.options.delayed_name_removal {
                        let name_only = matches!(operation,
                            MapOperation::Pointwise(function) if !function.drops_metadata());
                        sample.labels.retain(|name| {
                            if name_only {
                                name != b"__name__"
                            } else {
                                !metadata(name)
                            }
                        });
                    }
                    sample.source_time_ms = self.options.time_ms;
                    let duplicate_check = match operation {
                        MapOperation::Unary(TokenKind::SUB) => !self.options.delayed_name_removal,
                        MapOperation::Unary(_) => false,
                        MapOperation::Binary { .. } | MapOperation::Pointwise(_) => {
                            !self.options.delayed_name_removal
                                || self.options.start_ms != self.options.end_ms
                                    && !self.invariants[node.0]
                        }
                    };
                    if duplicate_check {
                        let kind = if matches!(operation, MapOperation::Unary(_)) {
                            sample.value.kind_bit()
                        } else {
                            0
                        };
                        remember_unique_kind(
                            seen,
                            &sample.labels,
                            kind,
                            self.account,
                            self.control,
                        )?;
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
                        expression: Some(child),
                        ..
                    } = &self.expression.arena[node].expression
                    else {
                        unreachable!()
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
                        if let Some(kind) =
                            group.add(*operator, &sample.value, sample.drop_name, self.account)
                            && node == self.diagnostic_node
                            && let Some(annotations) = &mut self.annotations
                        {
                            annotations.add(
                                kind,
                                label(&views, b"__name__"),
                                self.expression.arena[*child].span,
                                &self.expression.source,
                                self.account,
                            );
                        }
                    } else if matches!(filter, RowFilter::All) {
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
            let filter = self.frames[index].filter;
            let mode = self.frames[index].mode;
            match &mut self.frames[index].state {
                State::OrderedRows {
                    descriptors,
                    sources,
                    position,
                    running,
                    phase,
                } => {
                    if *running {
                        advance_row(
                            position,
                            phase,
                            &descriptors[*position],
                            mode,
                            self.options.time_ms,
                            false,
                        );
                        *running = false;
                    }
                    while *position < descriptors.len()
                        && !filter.accepts(descriptors[*position].ordinal)
                    {
                        self.control.check()?;
                        self.account.work(1);
                        *position += 1;
                    }
                    let Some(descriptor) = descriptors.get(*position) else {
                        self.frames.pop();
                        continue;
                    };
                    let filter = RowFilter::Ordinals(&sources[descriptor.sources.clone()]);
                    *running = true;
                    self.push_filtered(node, filter, true, OutputMode::Matrix);
                }
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
                    if !filter.accepts(series_ordinal) {
                        continue;
                    }
                    let selector_node = *selector;
                    let operation = *operation;
                    let selector = self.selector(selector_node);
                    if !matches_in(&series_labels, &selector.matchers, Some(self.account)) {
                        continue;
                    }
                    let end_ms = self.lookup(selector_node);
                    let value = match operation {
                        SelectorOperation::Instant => {
                            if selector.skip_histogram_buckets {
                                let fixed = selector.modifiers.timestamp.is_some()
                                    || selector.modifiers.start_or_end.is_some();
                                crate::instant_statistics::select(
                                    series,
                                    crate::instant_statistics::InstantWindow {
                                        first_lookup_ms: if fixed {
                                            end_ms
                                        } else {
                                            self.options.start_ms.wrapping_sub(
                                                self.prepared.offset(
                                                    selector_node,
                                                    selector.modifiers.original_offset,
                                                ) / 1_000_000,
                                            )
                                        },
                                        lookup_ms: end_ms,
                                        step_ms: self.options.step_ms,
                                        lookback_ms: self.options.lookback_ms,
                                        delta_ms: self.options.lookback_ms.wrapping_sub(i64::from(
                                            self.timestamp_selectors[selector_node.0],
                                        )),
                                        data_bounds: self.data_bounds,
                                    },
                                    self.control,
                                    self.account,
                                )?
                            } else {
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
                                FoldOptions {
                                    start_timestamps: self.options.start_timestamps,
                                    statistics: selector.skip_histogram_buckets.then_some(
                                        StatisticsWindow {
                                            first_end_ms: if selector.modifiers.timestamp.is_some()
                                                || selector.modifiers.start_or_end.is_some()
                                            {
                                                end_ms
                                            } else {
                                                self.options.start_ms.wrapping_sub(
                                                    self.prepared.offset(
                                                        selector_node,
                                                        selector.modifiers.original_offset,
                                                    ) / 1_000_000,
                                                )
                                            },
                                            range_ms: range_ns / 1_000_000,
                                            step_ms: self.options.step_ms,
                                            data_bounds: self.data_bounds,
                                        },
                                    ),
                                },
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
                        let Expr::Aggregate {
                            operator,
                            expression: Some(child),
                            ..
                        } = self.expression.arena[node].expression
                        else {
                            unreachable!()
                        };
                        let (sample, annotation) = group.finish(
                            operator,
                            self.options.time_ms,
                            self.control,
                            self.account,
                        )?;
                        if let Some(kind) = annotation
                            && node == self.diagnostic_node
                            && let Some(annotations) = &mut self.annotations
                        {
                            annotations.add(
                                kind,
                                b"",
                                self.expression.arena[child].span,
                                &self.expression.source,
                                self.account,
                            );
                        }
                        if let Some(sample) = sample
                            && let Some(sample) = self.propagate(sample, index)?
                        {
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
                        self.push_all(right_node);
                    } else {
                        let result = scalar_binary(
                            operator,
                            left.expect("left scalar"),
                            right.expect("right scalar"),
                            self.control,
                            self.account,
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

pub(crate) fn remember_unique_kind(
    seen: &mut AccountedVec<(u64, u8)>,
    labels: &OwnedLabels,
    kind: u8,
    account: &QueryAccounting,
    control: &Control,
) -> Result<(), EvaluationError> {
    let identity = (label_hash::full(labels, control, account)?, kind);
    if seen.contains(&identity) {
        return Err(EvaluationError::Invalid(
            "vector cannot contain metrics with the same labelset",
        ));
    }
    seen.push(identity);
    Ok(())
}

pub(crate) fn metadata(name: &[u8]) -> bool {
    matches!(name, b"__name__" | b"__type__" | b"__unit__")
}

fn scalar_binary(
    operator: TokenKind,
    left: f64,
    right: f64,
    control: &Control,
    account: &QueryAccounting,
) -> Result<f64, EvaluationError> {
    use TokenKind::*;
    Ok(match operator {
        ADD => left + right,
        SUB => left - right,
        MUL => left * right,
        DIV => left / right,
        POW | MOD => {
            let check = || {
                account.work(1);
                control.check()
            };
            if operator == POW {
                crate::numerical::pow_with(left, right, check)?
            } else {
                crate::numerical::modulo_with(left, right, check)?
            }
        }
        EQLC => f64::from(left == right),
        NEQ => f64::from(left != right),
        GTR => f64::from(left > right),
        LSS => f64::from(left < right),
        GTE => f64::from(left >= right),
        LTE => f64::from(left <= right),
        _ => return Err(EvaluationError::Unsupported("binary numerical kernel")),
    })
}
