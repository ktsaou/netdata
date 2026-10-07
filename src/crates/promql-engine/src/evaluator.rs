// SPDX-License-Identifier: GPL-3.0-or-later
// Evaluation mechanics derive from pinned Prometheus promql/engine.go; see NOTICE.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    aggregation::Group,
    annotations::{AnnotationBuilder, AnnotationKind},
    evaluator_roster::{Descriptor, Rosters, group_key, projection_index},
    float_functions::FloatFunction,
    frontend::{
        ast::{Expr, NodeId, ParsedExpression, ValueType, VectorSelector},
        token as tokens,
    },
    histogram_functions,
    histogram_stats::StatisticsWindow,
    label_hash,
    query_catalog::Catalog,
    query_preparation::Prepared,
    range_float::{RateInput, RateWindow},
    range_numeric::{self, NumericRangeError, NumericRangeFunction},
    range_rates::{self, RateFunction},
    range_reduce::{FoldKind, FoldOptions, RangeFunction, fold},
    result::{OwnedLabels, OwnedValue},
    storage::{
        Control, Interrupted, QueryProvider, Series, StorageError, TimeBounds, label,
        matches_checked, validate_labels,
    },
};
use tokens::TokenKind;

const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

fn matrix_argument(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
) -> Result<(NodeId, NodeId), EvaluationError> {
    let Expr::Call { arguments, .. } = &expression.arena[node].expression else {
        unreachable!("typed range call")
    };
    let argument = prepared.preprocessing.child(expression, node, arguments[0]);
    let Expr::Matrix { selector, .. } = expression.arena[argument].expression else {
        return Err(EvaluationError::Unsupported("subquery range reducer"));
    };
    Ok((argument, selector))
}
pub(crate) fn range_selector(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
) -> Result<NodeId, EvaluationError> {
    let (_, selector) = matrix_argument(expression, prepared, node)?;
    let Expr::Selector(value) = &expression.arena[selector].expression else {
        unreachable!("typed matrix selector")
    };
    if value.anchored || value.smoothed {
        return Err(EvaluationError::Unsupported("extended selector reducer"));
    }
    Ok(selector)
}

#[derive(Debug, PartialEq)]
pub enum EvaluationError {
    Interrupted(Interrupted),
    Storage(StorageError),
    Unsupported(&'static str),
    Invalid(&'static str),
    Preparation(AccountedVec<u8>),
    Runtime(AccountedVec<u8>),
    WrappedRuntime(AccountedVec<u8>),
    Regex(AccountedVec<u8>),
    Expansion(AccountedVec<u8>),
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
    pub source_start_time_ms: i64,
    pub drop_name: bool,
}

#[derive(Clone, Copy, Debug)]
enum SelectorOperation {
    Instant,
    Rate {
        kind: RateFunction,
        range_ns: i64,
    },
    Fold {
        kind: FoldKind,
        range_ns: i64,
    },
    Numeric {
        function: NumericRangeFunction,
        parameters: [f64; 2],
        range_ns: i64,
    },
}
impl SelectorOperation {
    fn drops_name(self) -> bool {
        match self {
            Self::Instant => false,
            Self::Rate { .. } | Self::Numeric { .. } => true,
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
    StartTimestamp { propagate: bool },
    HistogramCount,
    HistogramSum,
    HistogramAvg,
    HistogramStdVar,
    HistogramStdDev,
    Float(FloatFunction, [f64; 2]),
}
impl PointwiseFunction {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "timestamp" => Self::Timestamp,
            "start_timestamp" => Self::StartTimestamp { propagate: false },
            "histogram_count" => Self::HistogramCount,
            "histogram_sum" => Self::HistogramSum,
            "histogram_avg" => Self::HistogramAvg,
            "histogram_stdvar" => Self::HistogramStdVar,
            "histogram_stddev" => Self::HistogramStdDev,
            _ => Self::Float(FloatFunction::parse(name)?, [1.0, 0.0]),
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
        if let Self::StartTimestamp { propagate } = self {
            return Ok(propagate.then_some(sample.source_start_time_ms as f64 / 1000.0));
        }
        if let Self::Float(function, parameters) = self {
            return Ok(if let OwnedValue::Float(value) = &sample.value {
                function.apply(*value, parameters)
            } else {
                None
            });
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
            Self::Timestamp | Self::StartTimestamp { .. } | Self::Float(..) => unreachable!(),
        }))
    }
    fn drops_metadata(self) -> bool {
        matches!(
            self,
            Self::Timestamp | Self::StartTimestamp { .. } | Self::Float(..)
        )
    }
}

#[derive(Clone, Copy)]
enum ParameterizedFunction {
    Pointwise(FloatFunction),
    Range(RangeFunction),
}

enum State<'a> {
    Enter,
    OrderedRows {
        descriptors: &'a [Descriptor],
        sources: &'a [usize],
        position: usize,
        running: bool,
        phase: u8,
    },
    Pass,
    Labels {
        descriptors: &'a [Descriptor],
        bindings: &'a [(usize, usize)],
    },
    Map {
        operation: MapOperation,
        seen: AccountedVec<(u64, u8)>,
        last_diagnostic: Option<crate::binary_value::Diagnostic>,
    },
    Selection {
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
    Absent {
        present: bool,
        labels: OwnedLabels,
        drop_name: bool,
    },
    ScalarBinary {
        left: Option<f64>,
        right: Option<f64>,
        phase: u8,
    },
    Parameters {
        function: ParameterizedFunction,
        parameters: [f64; 2],
        position: usize,
    },
    MixedBinary {
        scalar: Option<f64>,
        scalar_node: NodeId,
        vector_node: NodeId,
        scalar_left: bool,
        collecting: bool,
    },
    Binary(AccountedBox<crate::vector_matching::Step<'a>>),
    Histogram(AccountedBox<crate::histogram_call::Step<'a>>),
    Info(AccountedBox<crate::info::Step<'a>>),
    CountValues {
        step: AccountedBox<crate::count_values::Step<'a>>,
        collecting: bool,
    },
}
struct Frame<'a> {
    node: NodeId,
    state: State<'a>,
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
    fn next_ordinal(
        self,
        start: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        match self {
            Self::All => Ok(Some(start)),
            Self::Ordinals(values) => {
                let mut next = None;
                for value in values {
                    control.check()?;
                    account.work(1);
                    if *value >= start && next.is_none_or(|previous| *value < previous) {
                        next = Some(*value);
                    }
                }
                Ok(next)
            }
        }
    }
    fn next_position(
        self,
        descriptors: &[Descriptor],
        mut start: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        control.check()?;
        if matches!(self, Self::All) {
            return Ok((start < descriptors.len()).then_some(start));
        }
        while let Some(descriptor) = descriptors.get(start) {
            let Some(wanted) = self.next_ordinal(descriptor.ordinal, control, account)? else {
                return Ok(None);
            };
            // Ordered rosters may contain gaps and repeated ordinals.
            let (mut left, mut right) = (start, descriptors.len());
            while left < right {
                control.check()?;
                account.work(1);
                let middle = left + (right - left) / 2;
                if descriptors[middle].ordinal < wanted {
                    left = middle + 1;
                } else {
                    right = middle;
                }
            }
            if descriptors
                .get(left)
                .is_some_and(|value| value.ordinal == wanted)
            {
                return Ok(Some(left));
            }
            start = left;
        }
        Ok(None)
    }
}

pub(crate) struct EvaluationMetadata<'a> {
    pub(crate) prepared: &'a Prepared<'a>,
    pub(crate) rosters: &'a Rosters,
    pub(crate) timestamp_selectors: &'a [bool],
    pub(crate) data_bounds: TimeBounds,
    pub(crate) annotations: Option<&'a mut AnnotationBuilder>,
    pub(crate) binary_preparation: Option<crate::vector_matching::Preparation<'a>>,
    pub(crate) histogram_preparation: Option<crate::histogram_call::Preparation<'a>>,
    pub(crate) info_preparation: Option<crate::info::Preparation<'a>>,
    pub(crate) count_preparation: Option<crate::count_values::Preparation<'a>>,
}
pub(crate) struct Machine<'a, 'q, Q: QueryProvider + 'q> {
    catalog: &'a Catalog<'q, Q>,
    expression: &'a ParsedExpression,
    prepared: &'a Prepared<'a>,
    options: EvaluationOptions,
    control: &'a Control,
    account: &'a QueryAccounting,
    frames: AccountedVec<Frame<'a>>,
    rosters: &'a Rosters,
    invariants: &'a [bool],
    timestamp_selectors: &'a [bool],
    data_bounds: TimeBounds,
    diagnostic_node: NodeId,
    annotations: Option<&'a mut AnnotationBuilder>,
    binary_preparation: Option<crate::vector_matching::Preparation<'a>>,
    histogram_preparation: Option<crate::histogram_call::Preparation<'a>>,
    info_preparation: Option<crate::info::Preparation<'a>>,
    count_preparation: Option<crate::count_values::Preparation<'a>>,
}

impl<'a, 'q, Q: QueryProvider + 'q> Machine<'a, 'q, Q> {
    pub(crate) fn new(
        catalog: &'a Catalog<'q, Q>,
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
            catalog,
            expression,
            prepared: metadata.prepared,
            options,
            control,
            account,
            frames,
            rosters: metadata.rosters,
            invariants: &metadata.prepared.flags().invariants,
            timestamp_selectors: metadata.timestamp_selectors,
            data_bounds: metadata.data_bounds,
            diagnostic_node: root,
            annotations: metadata.annotations,
            binary_preparation: metadata.binary_preparation,
            histogram_preparation: metadata.histogram_preparation,
            info_preparation: metadata.info_preparation,
            count_preparation: metadata.count_preparation,
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
    pub(crate) fn bypass_root_order(&mut self) {
        self.frames[0].bypass_order = true;
    }
    pub(crate) fn replay<'b>(
        &'b self,
        node: NodeId,
        options: EvaluationOptions,
        annotations: Option<&'b mut AnnotationBuilder>,
    ) -> Machine<'b, 'q, Q> {
        Machine::new(
            self.catalog,
            self.expression,
            node,
            options,
            self.control,
            self.account,
            EvaluationMetadata {
                prepared: self.prepared,
                rosters: self.rosters,
                timestamp_selectors: self.timestamp_selectors,
                data_bounds: self.data_bounds,
                annotations,
                binary_preparation: self.binary_preparation,
                histogram_preparation: self.histogram_preparation,
                info_preparation: self.info_preparation,
                count_preparation: self.count_preparation,
            },
        )
    }
    fn map(&self, operation: MapOperation) -> State<'a> {
        State::Map {
            operation,
            seen: AccountedVec::new(self.account, MemoryKind::Workspace),
            last_diagnostic: None,
        }
    }
    fn sample(&self, value: f64) -> EvaluatedSample {
        EvaluatedSample {
            ordinal: 0,
            labels: OwnedLabels::new(self.account, MemoryKind::Workspace),
            value: OwnedValue::Float(value),
            source_time_ms: self.options.time_ms,
            source_start_time_ms: 0,
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
        self.lookup_at(node, self.options.time_ms)
    }
    fn lookup_at(&self, node: NodeId, time_ms: i64) -> i64 {
        let selector = self.selector(node);
        let fixed = self.prepared.timestamp(node, &selector.modifiers);
        let time = if self.invariants[node.0] {
            self.options.start_ms
        } else {
            time_ms
        };
        // Direct timestamp selectors replace the lookup offset; storage retains the original.
        if self.timestamp_selectors[node.0]
            && let Some(reference) = fixed
        {
            let offset = time.wrapping_sub(reference).wrapping_mul(1_000_000);
            return time.wrapping_sub(offset / 1_000_000);
        }
        time.wrapping_sub(
            self.prepared
                .adjusted_offset(node, &selector.modifiers, self.options.start_ms)
                / 1_000_000,
        )
    }
    fn matrix_bounds(&self, end_ms: i64, range_ns: i64) -> Option<TimeBounds> {
        let start_ms = end_ms.wrapping_sub(range_ns / 1_000_000).checked_add(1)?;
        let bounds = TimeBounds {
            start_ms: start_ms.max(self.data_bounds.start_ms),
            end_ms: end_ms.min(self.data_bounds.end_ms),
        };
        (bounds.start_ms <= bounds.end_ms).then_some(bounds)
    }
    fn statistics_window(
        &self,
        node: NodeId,
        range_ns: i64,
        end_ms: i64,
    ) -> Option<StatisticsWindow> {
        let selector = self.selector(node);
        selector.skip_histogram_buckets.then_some(StatisticsWindow {
            first_end_ms: if self.invariants[node.0] {
                end_ms
            } else {
                self.options.start_ms.wrapping_sub(
                    self.prepared
                        .adjusted_offset(node, &selector.modifiers, self.options.start_ms)
                        / 1_000_000,
                )
            },
            range_ms: range_ns / 1_000_000,
            step_ms: self.options.step_ms,
            data_bounds: self.data_bounds,
        })
    }

    fn select(
        &self,
        selector_node: NodeId,
        operation: SelectorOperation,
    ) -> Result<State<'a>, EvaluationError> {
        let selector = self.selector(selector_node);
        if selector.anchored || selector.smoothed {
            return Err(EvaluationError::Unsupported("extended selector reducer"));
        }
        Ok(State::Selection {
            selector: selector_node,
            operation,
            ordinal: 0,
        })
    }
    fn range_spec(
        &self,
        node: NodeId,
        function: RangeFunction,
        parameters: [f64; 2],
    ) -> Result<(NodeId, SelectorOperation), EvaluationError> {
        let (argument, selector) = matrix_argument(self.expression, self.prepared, node)?;
        let Expr::Matrix { range, .. } = self.expression.arena[argument].expression else {
            unreachable!("typed matrix argument")
        };
        let range_ns = self.prepared.range(argument, range);
        let operation = match function {
            RangeFunction::Rate(kind) => SelectorOperation::Rate { kind, range_ns },
            RangeFunction::Fold(kind) => SelectorOperation::Fold { kind, range_ns },
            RangeFunction::Numeric(function) => SelectorOperation::Numeric {
                function,
                parameters,
                range_ns,
            },
        };
        Ok((selector, operation))
    }
    fn range_call(
        &self,
        node: NodeId,
        function: RangeFunction,
        parameters: [f64; 2],
    ) -> Result<State<'a>, EvaluationError> {
        let (selector, operation) = self.range_spec(node, function, parameters)?;
        self.select(selector, operation)
    }
    pub(crate) fn raw_series_sample<S: Series>(
        &mut self,
        node: NodeId,
        ordinal: usize,
        series: &S,
    ) -> Result<Option<EvaluatedSample>, EvaluationError> {
        self.select(node, SelectorOperation::Instant)?;
        self.series_sample(node, node, SelectorOperation::Instant, ordinal, series)
    }
    pub(crate) fn range_series_sample<S: Series>(
        &mut self,
        node: NodeId,
        function: RangeFunction,
        series_ordinal: usize,
        series: &S,
    ) -> Result<Option<EvaluatedSample>, EvaluationError> {
        self.control.check()?;
        self.account.work(1);
        let mut parameters = [0.0; 2];
        if let RangeFunction::Numeric(kind) = function {
            let Expr::Call { arguments, .. } = &self.expression.arena[node].expression else {
                unreachable!("range call")
            };
            for (position, &argument) in arguments[1..].iter().take(kind.parameters()).enumerate() {
                let mut child = self.replay(argument, self.options, None);
                child.frames[0].mode = OutputMode::Vector;
                while let Some(sample) = child.next()? {
                    let OwnedValue::Float(value) = sample.value else {
                        unreachable!("scalar parameter")
                    };
                    parameters[position] = value;
                }
            }
        }
        let (selector, operation) = self.range_spec(node, function, parameters)?;
        self.series_sample(node, selector, operation, series_ordinal, series)
    }
    fn series_sample<S: Series>(
        &mut self,
        node: NodeId,
        selector_node: NodeId,
        operation: SelectorOperation,
        series_ordinal: usize,
        series: &S,
    ) -> Result<Option<EvaluatedSample>, EvaluationError> {
        let series_labels = series.label_views(self.account);
        validate_labels(&series_labels)?;
        let selector = self.selector(selector_node);
        if !matches_checked(
            &series_labels,
            &selector.matchers,
            Some(self.account),
            self.control,
        )? {
            return Ok(None);
        }
        let end_ms = self.lookup(selector_node);
        let value = match operation {
            SelectorOperation::Instant => {
                match self.rosters.clock_initial(selector_node, series_ordinal) {
                    Some(crate::storage::InitialSample::Empty) => return Ok(None),
                    Some(crate::storage::InitialSample::Error(bytes)) => {
                        let mut failure = AccountedVec::new(self.account, MemoryKind::Workspace);
                        crate::result::append_bytes_checked(
                            &mut failure,
                            bytes,
                            self.account,
                            self.control,
                        )?;
                        return Err(StorageError::Provider(failure).into());
                    }
                    _ => (),
                }
                let fixed = self.invariants[selector_node.0];
                let window = crate::instant_select::InstantWindow {
                    first_lookup_ms: if fixed {
                        end_ms
                    } else {
                        self.lookup_at(selector_node, self.options.start_ms)
                    },
                    lookup_ms: end_ms,
                    step_ms: self.options.step_ms,
                    lookback_ms: self.options.lookback_ms,
                    delta_ms: self
                        .options
                        .lookback_ms
                        .wrapping_sub(i64::from(self.timestamp_selectors[selector_node.0])),
                    data_bounds: self.data_bounds,
                };
                if selector.skip_histogram_buckets {
                    crate::instant_statistics::select(series, window, self.control, self.account)?
                } else {
                    crate::instant_select::select::<_, _, StorageError>(
                        series,
                        window,
                        self.control,
                        self.account,
                        |value| {
                            Ok(OwnedValue::from_sample_checked(
                                value,
                                self.control,
                                self.account,
                                MemoryKind::Workspace,
                            )?)
                        },
                    )?
                }
            }
            SelectorOperation::Rate { kind, range_ns } => {
                let result = range_rates::evaluate(
                    series,
                    RateInput {
                        window: RateWindow {
                            start_ms: end_ms.wrapping_sub(range_ns / 1_000_000),
                            end_ms,
                            range_ns,
                            start_timestamps: self.options.start_timestamps,
                        },
                        range_start_ms: (if self.invariants[node.0] {
                            self.options.start_ms
                        } else {
                            self.options.time_ms
                        })
                        .wrapping_sub(
                            range_ns.wrapping_add(self.prepared.adjusted_offset(
                                selector_node,
                                &selector.modifiers,
                                self.options.start_ms,
                            )) / 1_000_000,
                        ),
                        data_bounds: Some(self.data_bounds),
                    },
                    kind,
                    self.statistics_window(selector_node, range_ns, end_ms),
                    self.control,
                    self.account,
                )?;
                if node == self.diagnostic_node
                    && let Some(annotations) = &mut self.annotations
                {
                    let Expr::Call { arguments, .. } = &self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    let argument =
                        self.prepared
                            .preprocessing
                            .child(self.expression, node, arguments[0]);
                    let span = self.expression.arena[argument].span;
                    let metric = label(&series_labels, b"__name__");
                    for kind in result.annotations.into_iter().flatten() {
                        annotations.add(kind, metric, span, &self.expression.source, self.account);
                    }
                    if kind.checks_counter_name()
                        && matches!(result.value, Some(OwnedValue::Float(_)))
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
                result.value.map(|value| (self.options.time_ms, 0, value))
            }
            SelectorOperation::Fold { kind, range_ns } => {
                let Some(bounds) = self.matrix_bounds(end_ms, range_ns) else {
                    return Ok(None);
                };
                let result = fold(
                    series,
                    bounds,
                    kind,
                    FoldOptions {
                        start_timestamps: self.options.start_timestamps,
                        statistics: self.statistics_window(selector_node, range_ns, end_ms),
                    },
                    self.control,
                    self.account,
                )?;
                if node == self.diagnostic_node
                    && let Some(annotations) = &mut self.annotations
                {
                    let Expr::Call { arguments, .. } = &self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    let argument =
                        self.prepared
                            .preprocessing
                            .child(self.expression, node, arguments[0]);
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
                result.value.map(|value| (self.options.time_ms, 0, value))
            }
            SelectorOperation::Numeric {
                function,
                parameters,
                range_ns,
            } => {
                let Some(bounds) = self.matrix_bounds(end_ms, range_ns) else {
                    return Ok(None);
                };
                let result = range_numeric::evaluate(
                    series,
                    bounds,
                    function,
                    parameters,
                    self.options.time_ms,
                    self.control,
                    self.account,
                )
                .map_err(|error| match error {
                    NumericRangeError::Storage(error) => EvaluationError::Storage(error),
                    NumericRangeError::Factor { trend, value } => EvaluationError::Runtime(
                        NumericRangeError::message(trend, value, self.account),
                    ),
                })?;
                if let Some(kind) = result.annotation
                    && node == self.diagnostic_node
                    && let Some(annotations) = &mut self.annotations
                {
                    let Expr::Call { arguments, .. } = &self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    let argument =
                        self.prepared
                            .preprocessing
                            .child(self.expression, node, arguments[0]);
                    annotations.add(
                        kind,
                        label(&series_labels, b"__name__"),
                        self.expression.arena[argument].span,
                        &self.expression.source,
                        self.account,
                    );
                }
                result
                    .value
                    .map(|value| (self.options.time_ms, 0, OwnedValue::Float(value)))
            }
        };
        Ok(value.map(|(source_time_ms, source_start_time_ms, value)| {
            let mut labels =
                OwnedLabels::copy_from(&series_labels, self.account, MemoryKind::Workspace);
            let drop_name = operation.drops_name();
            if drop_name && !self.options.delayed_name_removal {
                labels.retain(|name| !metadata(name));
            }
            EvaluatedSample {
                ordinal: series_ordinal,
                labels,
                value,
                source_time_ms,
                source_start_time_ms,
                drop_name,
            }
        }))
    }

    fn enter(&mut self, index: usize) -> Result<Option<EvaluatedSample>, EvaluationError> {
        let node = self.frames[index].node;
        let expression = &self.expression.arena[node].expression;
        if !self.frames[index].bypass_order
            && !crate::count_values::is_count(expression)
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
                operator: TokenKind::COUNT_VALUES,
                ..
            } => {
                let plan = self
                    .rosters
                    .count(node)
                    .ok_or(EvaluationError::Invalid("count_values plan is missing"))?;
                self.frames[index].state = State::CountValues {
                    step: AccountedBox::new(
                        crate::count_values::Step::new(
                            plan,
                            !self.frames[index].bypass_order,
                            self.account,
                        ),
                        &AllocationContext::new(self.account, MemoryKind::Workspace),
                    ),
                    collecting: true,
                };
                self.push_all(*child);
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
            Expr::Binary {
                left,
                right,
                operator,
                matching,
                return_bool,
            } => {
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
                    self.frames[index].state = State::Binary(AccountedBox::new(
                        crate::vector_matching::Step::new(
                            self.rosters
                                .binary(node)
                                .ok_or(EvaluationError::Invalid("binary plan is missing"))?,
                            matching.as_ref().expect("typed vector matching"),
                            *operator,
                            *return_bool,
                            self.account,
                        ),
                        &AllocationContext::new(self.account, MemoryKind::Workspace),
                    ));
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
                "pi" => {
                    self.frames.pop();
                    Ok(Some(self.sample(std::f64::consts::PI)))
                }
                "info" => {
                    self.frames[index].state = State::Info(AccountedBox::new(
                        crate::info::Step::new(
                            self.rosters
                                .info(node)
                                .ok_or(EvaluationError::Invalid("INFO plan is missing"))?,
                            self.account,
                        ),
                        &AllocationContext::new(self.account, MemoryKind::Workspace),
                    ));
                    Ok(None)
                }
                "histogram_quantile" | "histogram_fraction" => {
                    let function = crate::histogram_call::Function::parse(function.name)
                        .expect("histogram function");
                    let scalar =
                        self.prepared
                            .preprocessing
                            .child(self.expression, node, arguments[0]);
                    let vector = self.prepared.preprocessing.child(
                        self.expression,
                        node,
                        arguments[function.vector_argument()],
                    );
                    self.frames[index].state = State::Histogram(AccountedBox::new(
                        crate::histogram_call::Step::new(
                            self.rosters
                                .histogram(node)
                                .ok_or(EvaluationError::Invalid("histogram plan is missing"))?,
                            function,
                            self.expression.arena[scalar].span,
                            self.expression.arena[vector].span,
                            self.histogram_preparation
                                .is_some_and(|value| value.node == node),
                            match self.frames[index].filter {
                                RowFilter::All => None,
                                RowFilter::Ordinals(values) => Some(values),
                            },
                            self.account,
                        ),
                        &AllocationContext::new(self.account, MemoryKind::Workspace),
                    ));
                    Ok(None)
                }
                "min_of" | "max_of" => {
                    self.frames[index].state = State::ScalarBinary {
                        left: None,
                        right: None,
                        phase: 0,
                    };
                    self.push_all(arguments[0]);
                    Ok(None)
                }
                "vector" => {
                    self.frames[index].state = State::Pass;
                    self.push(arguments[0]);
                    Ok(None)
                }
                "label_replace" | "label_join" => {
                    let projection = self
                        .rosters
                        .projection(node)
                        .ok_or(EvaluationError::Invalid("label projection is missing"))?;
                    self.frames[index].state = State::Labels {
                        descriptors: projection.descriptors,
                        bindings: projection.bindings,
                    };
                    self.push_matrix(arguments[0]);
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
                "absent" | "absent_over_time" => {
                    let over_time = function.name == "absent_over_time";
                    self.frames[index].state = State::Absent {
                        present: false,
                        labels: crate::absence::labels(
                            self.expression,
                            arguments[0],
                            self.account,
                            self.control,
                        )?,
                        drop_name: over_time,
                    };
                    if over_time {
                        let state = self.range_call(
                            node,
                            RangeFunction::Fold(FoldKind::Present),
                            [0.0; 2],
                        )?;
                        self.frames.push(Frame {
                            node,
                            state,
                            filter: RowFilter::All,
                            bypass_order: true,
                            mode: OutputMode::Vector,
                        });
                    } else {
                        self.push_all(arguments[0]);
                    }
                    Ok(None)
                }
                _ => {
                    if let Some(mut function) = PointwiseFunction::parse(function.name) {
                        if let PointwiseFunction::StartTimestamp { propagate } = &mut function {
                            let argument = self.prepared.preprocessing.child(
                                self.expression,
                                node,
                                arguments[0],
                            );
                            *propagate = self.options.start_timestamps
                                && matches!(
                                    self.expression.arena[argument].expression,
                                    Expr::Selector(_)
                                );
                        }
                        if let PointwiseFunction::Float(float, _) = function {
                            if arguments.is_empty() {
                                let result = float
                                    .at_time(self.options.time_ms)
                                    .expect("calendar function");
                                self.frames.pop();
                                return Ok(Some(self.sample(result)));
                            }
                            if arguments.len() > 1 {
                                self.frames[index].state = State::Parameters {
                                    function: ParameterizedFunction::Pointwise(float),
                                    parameters: [1.0, 0.0],
                                    position: 0,
                                };
                                self.push_all(arguments[1]);
                                return Ok(None);
                            }
                        }
                        self.frames[index].state = self.map(MapOperation::Pointwise(function));
                        self.push(arguments[0]);
                        return Ok(None);
                    }
                    let function = RangeFunction::parse(function.name)
                        .ok_or(EvaluationError::Unsupported("function reducer"))?;
                    if let RangeFunction::Numeric(kind) = function
                        && kind.parameters() > 0
                    {
                        self.frames[index].state = State::Parameters {
                            function: ParameterizedFunction::Range(function),
                            parameters: [0.0, 0.0],
                            position: 0,
                        };
                        self.push_all(arguments[1]);
                    } else {
                        self.frames[index].state = self.range_call(node, function, [0.0; 2])?;
                    }
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
                State::Binary(step) => {
                    step.receive(sample, self.control, self.account)?;
                    return Ok(None);
                }
                State::Histogram(step) => {
                    step.receive(sample, self.control, self.account)?;
                    return Ok(None);
                }
                State::Info(step) => {
                    step.receive(sample, self.control, self.account)?;
                    return Ok(None);
                }
                State::CountValues { step, .. } => {
                    step.receive(sample, self.control, self.account)?;
                    return Ok(None);
                }
                State::Pass => (),
                State::Labels {
                    descriptors,
                    bindings,
                } => {
                    let position =
                        projection_index(bindings, sample.ordinal, self.control, self.account)?
                            .ok_or(EvaluationError::Invalid(
                                "label projection source is missing",
                            ))?;
                    let descriptor = &descriptors[position];
                    sample.labels = descriptor.labels.copy_checked(
                        self.account,
                        MemoryKind::Workspace,
                        self.control,
                    )?;
                    sample.drop_name = descriptor.drop_name;
                    sample.source_time_ms = self.options.time_ms;
                    sample.source_start_time_ms = 0;
                }
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
                State::Map {
                    operation,
                    seen,
                    last_diagnostic,
                } => {
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
                        } => {
                            let vector_float = if let OwnedValue::Float(value) = &sample.value {
                                Some(*value)
                            } else {
                                None
                            };
                            let vector =
                                std::mem::replace(&mut sample.value, OwnedValue::Float(0.0));
                            let scalar = OwnedValue::Float(scalar);
                            let (left, right) = if scalar_left {
                                (scalar, vector)
                            } else {
                                (vector, scalar)
                            };
                            let pair = crate::binary_value::evaluate(
                                operator,
                                left,
                                right,
                                self.control,
                                self.account,
                            )?;
                            if pair.diagnostic.is_some() {
                                *last_diagnostic = pair.diagnostic;
                            }
                            let Some(mut value) = pair.value else {
                                return Ok(None);
                            };
                            if return_bool {
                                value = OwnedValue::Float(f64::from(pair.keep));
                            } else if !pair.keep {
                                return Ok(None);
                            } else if scalar_left && operator.is_comparison() {
                                value = OwnedValue::Float(
                                    vector_float.expect("compatible scalar comparison"),
                                );
                            }
                            (
                                value,
                                crate::binary_value::changes_metadata(operator) || return_bool,
                            )
                        }
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
                    sample.source_start_time_ms = 0;
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
                                self.expression.arena[self.prepared.preprocessing.child(
                                    self.expression,
                                    node,
                                    *child,
                                )]
                                .span,
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
                State::Absent { present, .. } => {
                    *present = true;
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
                State::Parameters {
                    parameters,
                    position,
                    ..
                } => {
                    let OwnedValue::Float(value) = sample.value else {
                        unreachable!("scalar parameter")
                    };
                    parameters[*position] = value;
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
                State::CountValues { step, collecting } => {
                    if *collecting {
                        let mut options = self.options;
                        if self.invariants[node.0] {
                            options.end_ms = options.start_ms;
                        }
                        step.finish(
                            options,
                            self.count_preparation.filter(|value| value.node == node),
                            self.control,
                            self.account,
                        )?;
                        *collecting = false;
                    }
                    if let Some(sample) = step.next(self.options.time_ms, self.account) {
                        if filter.accepts(sample.ordinal)
                            && let Some(sample) = self.propagate(sample, index)?
                        {
                            return Ok(Some(sample));
                        }
                    } else {
                        self.frames.pop();
                    }
                }
                State::Info(step) => {
                    match step.advance(
                        node,
                        self.options,
                        (self.catalog, self.data_bounds),
                        self.info_preparation,
                        self.control,
                        self.account,
                    )? {
                        crate::info::Action::Child(ordinal) => {
                            let Expr::Call { arguments, .. } =
                                &self.expression.arena[node].expression
                            else {
                                unreachable!()
                            };
                            let filter = ordinal.map_or(RowFilter::All, |ordinal| {
                                RowFilter::Ordinals(std::slice::from_ref(ordinal))
                            });
                            self.push_filtered(arguments[0], filter, false, OutputMode::Vector);
                        }
                        crate::info::Action::Sample(sample) => {
                            if filter.accepts(sample.ordinal)
                                && let Some(sample) = self.propagate(sample, index)?
                            {
                                return Ok(Some(sample));
                            }
                        }
                        crate::info::Action::Done => {
                            self.frames.pop();
                        }
                    }
                }
                State::Histogram(step) => {
                    let mut options = self.options;
                    if self.invariants[node.0] {
                        options.end_ms = options.start_ms;
                    }
                    match step.advance(
                        node,
                        options,
                        self.histogram_preparation,
                        &self.expression.source,
                        self.control,
                        self.account,
                    )? {
                        crate::histogram_call::Action::Child { argument, ordinal } => {
                            let Expr::Call { arguments, .. } =
                                &self.expression.arena[node].expression
                            else {
                                unreachable!()
                            };
                            let child = arguments[argument];
                            let filter = ordinal.map_or(RowFilter::All, |value| {
                                RowFilter::Ordinals(std::slice::from_ref(value))
                            });
                            self.push_filtered(child, filter, false, OutputMode::Vector);
                        }
                        crate::histogram_call::Action::Sample(sample) => {
                            if filter.accepts(sample.ordinal)
                                && let Some(sample) = self.propagate(sample, index)?
                            {
                                return Ok(Some(sample));
                            }
                        }
                        crate::histogram_call::Action::Done(notes) => {
                            if node == self.diagnostic_node
                                && let Some(notes) = notes
                                && let Some(annotations) = &mut self.annotations
                            {
                                let preparation = self
                                    .histogram_preparation
                                    .filter(|value| value.node == node)
                                    .expect("logical histogram diagnostics");
                                preparation.cache.borrow_mut().callbacks.merge(
                                    notes,
                                    annotations,
                                    &self.expression.source,
                                    self.control,
                                    self.account,
                                )?;
                            }
                            self.frames.pop();
                        }
                    }
                }
                State::Binary(step) => {
                    let mut options = self.options;
                    if self.invariants[node.0] {
                        options.end_ms = options.start_ms;
                    }
                    match step.advance(
                        node,
                        options,
                        self.binary_preparation,
                        self.control,
                        self.account,
                    )? {
                        crate::vector_matching::Action::Child { side, ordinal } => {
                            let Expr::Binary { left, right, .. } =
                                self.expression.arena[node].expression
                            else {
                                unreachable!()
                            };
                            let child = if side == 0 { left } else { right };
                            let filter = ordinal.map_or(RowFilter::All, |value| {
                                RowFilter::Ordinals(std::slice::from_ref(value))
                            });
                            self.push_filtered(child, filter, false, OutputMode::Vector);
                        }
                        crate::vector_matching::Action::Sample(sample) => {
                            if filter.accepts(sample.ordinal)
                                && let Some(sample) = self.propagate(sample, index)?
                            {
                                return Ok(Some(sample));
                            }
                        }
                        crate::vector_matching::Action::Done(diagnostic) => {
                            if node == self.diagnostic_node
                                && let Some(crate::binary_value::Diagnostic::Annotation(kind)) =
                                    diagnostic
                                && let Some(annotations) = &mut self.annotations
                            {
                                annotations.add(
                                    kind,
                                    b"",
                                    self.expression.arena[node].span,
                                    &self.expression.source,
                                    self.account,
                                );
                            }
                            self.frames.pop();
                        }
                    }
                }
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
                    let Some(next) =
                        filter.next_position(descriptors, *position, self.control, self.account)?
                    else {
                        self.frames.pop();
                        continue;
                    };
                    if next != *position {
                        *phase = 0;
                        *position = next;
                    }
                    let descriptor = &descriptors[*position];
                    let filter = RowFilter::Ordinals(&sources[descriptor.sources.clone()]);
                    *running = true;
                    self.push_filtered(node, filter, true, OutputMode::Matrix);
                }
                State::Map {
                    last_diagnostic, ..
                } => {
                    if node == self.diagnostic_node
                        && let Some(crate::binary_value::Diagnostic::Annotation(kind)) =
                            last_diagnostic.take()
                        && let Some(annotations) = &mut self.annotations
                    {
                        annotations.add(
                            kind,
                            b"",
                            self.expression.arena[node].span,
                            &self.expression.source,
                            self.account,
                        );
                    }
                    self.frames.pop();
                }
                State::Pass | State::Labels { .. } => {
                    self.frames.pop();
                }
                State::Selection {
                    selector,
                    operation,
                    ordinal,
                } => {
                    let selector_node = *selector;
                    let operation = *operation;
                    let Some(series_ordinal) =
                        filter.next_ordinal(*ordinal, self.control, self.account)?
                    else {
                        self.frames.pop();
                        continue;
                    };
                    if series_ordinal >= self.catalog.len(selector_node) {
                        self.frames.pop();
                        continue;
                    }
                    *ordinal = series_ordinal + 1;
                    let catalog = self.catalog;
                    let sample = catalog
                        .with_series(selector_node, series_ordinal, |series| {
                            self.series_sample(
                                node,
                                selector_node,
                                operation,
                                series_ordinal,
                                series,
                            )
                        })
                        .expect("catalog ordinal")?;
                    if let Some(sample) = sample
                        && let Some(sample) = self.propagate(sample, index)?
                    {
                        return Ok(Some(sample));
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
                                self.expression.arena[self.prepared.preprocessing.child(
                                    self.expression,
                                    node,
                                    child,
                                )]
                                .span,
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
                State::Absent {
                    present,
                    labels,
                    drop_name,
                } => {
                    let sample = (!*present).then(|| EvaluatedSample {
                        ordinal: 0,
                        labels: std::mem::replace(
                            labels,
                            OwnedLabels::new(self.account, MemoryKind::Workspace),
                        ),
                        value: OwnedValue::Float(1.0),
                        source_time_ms: self.options.time_ms,
                        source_start_time_ms: 0,
                        drop_name: *drop_name,
                    });
                    self.frames.pop();
                    if let Some(sample) = sample
                        && let Some(sample) = self.propagate(sample, self.frames.len())?
                    {
                        return Ok(Some(sample));
                    }
                }
                State::ScalarBinary { left, right, phase } => {
                    let (operator, right_node) = match &self.expression.arena[node].expression {
                        Expr::Binary {
                            operator, right, ..
                        } => (*operator, *right),
                        Expr::Call {
                            function: Some(function),
                            arguments,
                        } => (
                            if function.name == "min_of" {
                                TokenKind::MIN
                            } else {
                                TokenKind::MAX
                            },
                            arguments[1],
                        ),
                        _ => unreachable!(),
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
                State::Parameters {
                    function,
                    parameters,
                    position,
                } => {
                    let Expr::Call { arguments, .. } = &self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    if *position + 2 < arguments.len() {
                        *position += 1;
                        let child = arguments[*position + 1];
                        self.push_all(child);
                    } else {
                        let (function, parameters) = (*function, *parameters);
                        match function {
                            ParameterizedFunction::Pointwise(function) => {
                                let operation = MapOperation::Pointwise(PointwiseFunction::Float(
                                    function, parameters,
                                ));
                                self.frames[index].state = self.map(operation);
                                self.push(arguments[0]);
                            }
                            ParameterizedFunction::Range(function) => {
                                self.frames[index].state =
                                    self.range_call(node, function, parameters)?;
                            }
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

pub(crate) fn scalar_binary(
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
        ATAN2 => crate::numerical::atan2(left, right),
        MIN => crate::numerical::min(left, right),
        MAX => crate::numerical::max(left, right),
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

#[cfg(test)]
mod filter_tests {
    use super::*;
    use crate::accounting::EngineAccounting;

    #[test]
    fn ordered_filter_preserves_gaps_repeats_and_cancellation() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let descriptors: Vec<_> = [2, 2, 7, 9, 11]
            .into_iter()
            .map(|ordinal| Descriptor {
                labels: OwnedLabels::new(&account, MemoryKind::Workspace),
                ordinal,
                hash: 0,
                drop_name: false,
                first_order: 0,
                sources: 0..0,
                kinds: 0,
                collision_time_ms: None,
            })
            .collect();
        for accepted in [
            b"".as_slice(),
            b"\x02\x09",
            b"\x07\x02\x02",
            b"\x00\x01\x03\x08\x0c",
        ] {
            let ordinals: Vec<_> = accepted.iter().map(|value| *value as usize).collect();
            for start in 0..=descriptors.len() + 1 {
                let expected = descriptors
                    .iter()
                    .enumerate()
                    .skip(start)
                    .find(|(_, descriptor)| ordinals.contains(&descriptor.ordinal))
                    .map(|(position, _)| position);
                assert_eq!(
                    RowFilter::Ordinals(&ordinals)
                        .next_position(&descriptors, start, &control, &account)
                        .unwrap(),
                    expected
                );
                assert_eq!(
                    RowFilter::All
                        .next_position(&descriptors, start, &control, &account)
                        .unwrap(),
                    (start < descriptors.len()).then_some(start)
                );
            }
        }
        assert_eq!(
            RowFilter::Ordinals(&[usize::MAX])
                .next_position(&descriptors, 0, &control, &account)
                .unwrap(),
            None
        );
        control.cancellation.cancel();
        assert!(matches!(
            RowFilter::Ordinals(&[2]).next_position(&descriptors, 0, &control, &account),
            Err(Interrupted::Cancelled)
        ));
    }
}
