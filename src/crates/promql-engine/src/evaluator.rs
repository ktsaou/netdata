// SPDX-License-Identifier: GPL-3.0-or-later
// Evaluation mechanics derive from pinned Prometheus promql/engine.go; see NOTICE.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    aggregation::Group,
    annotations::{AnnotationBuilder, AnnotationKind},
    evaluator_roster::{Descriptor, Rosters, copy_group_key, projection_index},
    float_functions::FloatFunction,
    frontend::{
        ast::{Expr, NodeId, ParsedExpression, ValueType, VectorSelector},
        token as tokens,
    },
    histogram_functions, label_hash,
    matrix_reader::MatrixWindow,
    query_catalog::Catalog,
    query_preparation::Prepared,
    range_float::{RateInput, RateWindow},
    range_numeric::{self, NumericRangeError, NumericRangeFunction, NumericRangeInput},
    range_rates::{self, RateFunction},
    range_reduce::{FoldKind, FoldOptions, RangeFunction, fold},
    result::{OwnedLabels, OwnedValue},
    storage::{
        Control, InitialSample, Interrupted, QueryProvider, Series, StorageError, TimeBounds,
        label, matches_checked, validate_labels_checked,
    },
    value::{SampleRef, SampleValueRef},
};
use tokens::TokenKind;

const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

pub(crate) fn supports_selector_schedule(
    selector: NodeId,
    value: &VectorSelector,
    prepared: &Prepared,
    timestamp_selector: bool,
    lookback_ms: i64,
    grid: crate::query_scopes::Grid,
) -> bool {
    if !grid.forward_without_wrap()
        || grid
            .end_ms
            .checked_sub(grid.start_ms)
            .is_none_or(|width| width < 0)
        || value.smoothed
    {
        return false;
    }
    let modifiers = &value.modifiers;
    if let Some(reference) = prepared.timestamp(selector, modifiers) {
        let Some(offset) = grid
            .start_ms
            .checked_sub(reference)
            .and_then(|delta| delta.checked_mul(1_000_000))
            .and_then(|delta| {
                delta.checked_add(prepared.offset(selector, modifiers.original_offset))
            })
        else {
            return false;
        };
        if !timestamp_selector
            && offset != prepared.adjusted_offset(selector, modifiers, grid.start_ms)
        {
            return false;
        }
        if timestamp_selector
            && grid
                .end_ms
                .checked_sub(reference)
                .and_then(|delta| delta.checked_mul(1_000_000))
                .is_none()
        {
            return false;
        }
    }
    let offset = prepared.adjusted_offset(selector, modifiers, grid.start_ms) / 1_000_000;
    for time in [grid.start_ms, grid.end_ms] {
        let lookup = if timestamp_selector
            && let Some(reference) = prepared.timestamp(selector, modifiers)
        {
            reference
        } else {
            let Some(lookup) = time.checked_sub(offset) else {
                return false;
            };
            lookup
        };
        if lookup.checked_sub(lookback_ms).is_none() {
            return false;
        }
    }
    true
}

pub(crate) fn matrix_argument(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<(usize, NodeId)>, Interrupted> {
    let Expr::Call {
        function: Some(function),
        arguments,
    } = &expression.arena[node].expression
    else {
        return Ok(None);
    };
    let Some(position) = function
        .arguments
        .iter()
        .position(|kind| *kind == ValueType::Matrix)
    else {
        return Ok(None);
    };
    Ok(Some((
        position,
        prepared
            .preprocessing
            .child(expression, node, arguments[position], control, account)?,
    )))
}

pub(crate) fn generic_matrix_call(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<&'static str>, Interrupted> {
    let Some((_, argument)) = matrix_argument(expression, prepared, node, control, account)? else {
        return Ok(None);
    };
    if !matches!(expression.arena[argument].expression, Expr::Paren(_)) {
        return Ok(None);
    }
    let Expr::Call {
        function: Some(function),
        ..
    } = &expression.arena[node].expression
    else {
        unreachable!("typed matrix function")
    };
    Ok(Some(function.name))
}

fn scalar_arguments(
    expression: &ParsedExpression,
    node: NodeId,
) -> impl Iterator<Item = NodeId> + '_ {
    let Expr::Call {
        function: Some(function),
        arguments,
    } = &expression.arena[node].expression
    else {
        unreachable!("typed function parameters")
    };
    arguments
        .iter()
        .copied()
        .zip(function.arguments.iter())
        .filter_map(|(node, kind)| (*kind == ValueType::Scalar).then_some(node))
}

fn raw_matrix_argument(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(NodeId, NodeId), EvaluationError> {
    let Some((_, argument)) = matrix_argument(expression, prepared, node, control, account)? else {
        unreachable!("typed range call")
    };
    let Expr::Matrix { selector, .. } = expression.arena[argument].expression else {
        return Err(EvaluationError::Unsupported("subquery range reducer"));
    };
    Ok((argument, selector))
}
pub(crate) fn validate_matrix_modifier(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), EvaluationError> {
    let Some((_, argument)) = matrix_argument(expression, prepared, node, control, account)? else {
        return Ok(());
    };
    let Expr::Matrix { selector, .. } = expression.arena[argument].expression else {
        return Ok(());
    };
    let Expr::Selector(selector) = &expression.arena[selector].expression else {
        unreachable!("typed matrix selector")
    };
    let Expr::Call {
        function: Some(function),
        ..
    } = &expression.arena[node].expression
    else {
        unreachable!("typed matrix call")
    };
    let (modifier, allowed, safe) = if selector.anchored {
        (
            "anchored",
            "changes, delta, increase, rate, resets",
            matches!(
                function.name,
                "changes" | "delta" | "increase" | "rate" | "resets"
            ),
        )
    } else if selector.smoothed {
        (
            "smoothed",
            "delta, increase, rate",
            matches!(function.name, "delta" | "increase" | "rate"),
        )
    } else {
        return Ok(());
    };
    if safe {
        return Ok(());
    }
    Err(EvaluationError::Runtime(crate::binary_labels::formatted(
        format_args!(
            "{modifier} modifier can only be used with: {allowed} - not with {}",
            function.name
        ),
        control,
        account,
    )?))
}

pub(crate) fn range_selector(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<NodeId>, EvaluationError> {
    let (_, argument) =
        matrix_argument(expression, prepared, node, control, account)?.expect("typed range call");
    let Expr::Matrix { selector, .. } = expression.arena[argument].expression else {
        return Ok(None);
    };
    let Expr::Selector(value) = &expression.arena[selector].expression else {
        unreachable!("typed matrix selector")
    };
    let anchored_transition = matches!(&expression.arena[node].expression,
        Expr::Call { function: Some(function), .. }
        if matches!(function.name, "changes" | "resets"));
    let extended_rate = matches!(&expression.arena[node].expression,
        Expr::Call { function: Some(function), .. }
        if matches!(function.name, "rate" | "increase" | "delta"));
    if value.smoothed && !extended_rate || value.anchored && !anchored_transition && !extended_rate
    {
        return Err(EvaluationError::Unsupported("extended selector reducer"));
    }
    Ok(Some(selector))
}

pub(crate) fn raw_selector_shortcut(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
) -> bool {
    let Expr::Selector(selector) = &expression.arena[node].expression else {
        return false;
    };
    // Fixed smoothing produces once, then follows the source's invariant expansion.
    !selector.smoothed || !prepared.flags().invariants[node.0]
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

pub(crate) fn numeric_range_error(
    error: NumericRangeError,
    account: &QueryAccounting,
) -> EvaluationError {
    match error {
        NumericRangeError::Storage(error) => EvaluationError::Storage(error),
        NumericRangeError::Factor { trend, value } => {
            EvaluationError::Runtime(NumericRangeError::message(trend, value, account))
        }
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
    pub(crate) float_samples: Option<crate::statistics::OwnedSamples>,
    pub source_time_ms: i64,
    pub source_start_time_ms: i64,
    pub drop_name: bool,
}
impl EvaluatedSample {
    pub(crate) fn into_parts(self) -> crate::statistics::ValueParts {
        crate::statistics::ValueParts {
            value: self.value,
            float_samples: self.float_samples,
        }
    }
    fn reconcile_samples(&mut self, account: &QueryAccounting) {
        if matches!(self.value, OwnedValue::Float(_)) {
            if self.float_samples.is_none() {
                self.float_samples = crate::statistics::hold_float(account, self.value.as_ref());
            }
        } else {
            self.float_samples = None;
        }
    }
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
            Self::HistogramStdVar => histogram_functions::histogram_stdvar_with(
                histogram,
                crate::histogram_work::HistogramWork::new(control, account),
                poll,
            )?,
            Self::HistogramStdDev => {
                crate::numerical_math::sqrt(histogram_functions::histogram_stdvar_with(
                    histogram,
                    crate::histogram_work::HistogramWork::new(control, account),
                    poll,
                )?)
            }
            Self::Timestamp | Self::StartTimestamp { .. } | Self::Float(..) => unreachable!(),
        }))
    }
    pub(crate) fn drops_metadata(self) -> bool {
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
    Gather(AccountedBox<crate::matrix_heads::Step<'a>>),
    OrderedRows {
        descriptors: &'a [Descriptor],
        sources: &'a [usize],
        position: usize,
        running: bool,
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
        lookup: Option<AccountedBox<crate::position_index::PositionIndex>>,
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
    RawParameters {
        function: RangeFunction,
        selector: NodeId,
        ordinal: usize,
        initial: Option<InitialSample>,
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
    TopK(AccountedBox<crate::topk::Step<'a>>),
    Quantile(AccountedBox<crate::quantile_aggregate::Step<'a>>),
    Sort(AccountedBox<crate::vector_sort::Step<'a>>),
    Range(AccountedBox<crate::derived_range::Step<'a>>),
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
    options: EvaluationOptions,
    step_index: i64,
    boundary_time_ms: Option<i64>,
    float_samples: [Option<crate::statistics::OwnedSamples>; 2],
}
impl<'a> Frame<'a> {
    fn new(
        node: NodeId,
        filter: RowFilter<'a>,
        bypass_order: bool,
        mode: OutputMode,
        mut options: EvaluationOptions,
        boundary: bool,
        step_index: Option<i64>,
    ) -> Self {
        let boundary_time_ms = boundary.then_some(options.time_ms);
        let mut step_index = step_index.unwrap_or_else(|| {
            crate::query_scopes::quotient(
                options.time_ms.wrapping_sub(options.start_ms),
                options.step_ms,
            )
            .unwrap_or(0)
        });
        if boundary && options.end_ms >= options.start_ms {
            options.time_ms = options.start_ms;
            options.end_ms = options.start_ms;
            step_index = 0;
        }
        Self {
            node,
            state: State::Enter,
            filter,
            bypass_order,
            mode,
            options,
            step_index,
            boundary_time_ms,
            float_samples: [None, None],
        }
    }
}

#[derive(Clone, Copy)]
enum OutputMode {
    Matrix,
    Vector,
}
fn set_discovery_target(
    mode: OutputMode,
    bypass_order: bool,
    filter: RowFilter<'_>,
    node: NodeId,
    preparation: Option<crate::vector_matching::Preparation<'_>>,
) -> Option<Option<usize>> {
    if !matches!(mode, OutputMode::Matrix) {
        return None;
    }
    match filter {
        RowFilter::One(target) if bypass_order => Some(Some(target)),
        RowFilter::Ordinals([target]) if bypass_order => Some(Some(*target)),
        RowFilter::All if preparation.is_some_and(|owner| owner.node == node) => Some(None),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(crate) enum RowFilter<'a> {
    All,
    Ordinals(&'a [usize]),
    One(usize),
}
impl RowFilter<'_> {
    pub(crate) fn accepts(
        self,
        ordinal: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        control.check()?;
        account.work(1);
        match self {
            Self::All => Ok(true),
            Self::Ordinals(values) => {
                crate::evaluator_order::contains(values, &ordinal, control, account)
            }
            Self::One(value) => Ok(value == ordinal),
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
            Self::One(value) => Ok((value >= start).then_some(value)),
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
#[derive(Clone, Copy)]
pub(crate) struct ScheduledRow {
    root: NodeId,
    pub(crate) selector: NodeId,
    pub(crate) physical_ordinal: usize,
    ordinal: usize,
    bypass_root: bool,
}
struct SelectedPoint {
    labels: OwnedLabels,
    value: Option<(i64, i64, crate::statistics::ValueParts)>,
}
pub(crate) struct Machine<'a, 'q, Q: QueryProvider + 'q> {
    catalog: &'a Catalog<'q, Q>,
    expression: &'a ParsedExpression,
    prepared: &'a Prepared<'a>,
    options: EvaluationOptions,
    step_index: i64,
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
    scheduled_selector: Option<(NodeId, usize)>,
    selected_point: Option<SelectedPoint>,
    primary_statistics: Option<&'a crate::statistics::PrimaryProgress<'a>>,
}

impl<'a, 'q, Q: QueryProvider + 'q> Machine<'a, 'q, Q> {
    #[cfg(test)]
    pub(crate) fn set_sort_test_context(
        &mut self,
        node: NodeId,
        ordinals: &'a [usize],
        matrix: bool,
        bypass_order: bool,
    ) {
        let frame = &mut self.frames[0];
        frame.node = node;
        frame.filter = RowFilter::Ordinals(ordinals);
        frame.mode = if matrix {
            OutputMode::Matrix
        } else {
            OutputMode::Vector
        };
        frame.bypass_order = bypass_order;
        frame.state = State::Sort(AccountedBox::new(
            crate::vector_sort::Step::new(
                self.rosters.sorting(node).unwrap(),
                frame.options,
                frame.step_index,
                false,
                self.account,
            ),
            &AllocationContext::new(self.account, MemoryKind::Workspace),
        ));
    }
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
        frames.push(Frame::new(
            root,
            RowFilter::All,
            false,
            OutputMode::Matrix,
            options,
            metadata.prepared.flags().boundaries[root.0],
            None,
        ));
        let options = frames[0].options;
        let step_index = frames[0].step_index;
        Self {
            catalog,
            expression,
            prepared: metadata.prepared,
            options,
            step_index,
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
            scheduled_selector: None,
            selected_point: None,
            primary_statistics: None,
        }
    }
    fn push(&mut self, node: NodeId) -> Result<(), EvaluationError> {
        let filter = self
            .frames
            .last()
            .map_or(RowFilter::All, |frame| frame.filter);
        self.push_filtered(node, filter, false, OutputMode::Vector)?;
        Ok(())
    }
    fn push_matrix(&mut self, node: NodeId) -> Result<(), EvaluationError> {
        let filter = self
            .frames
            .last()
            .map_or(RowFilter::All, |frame| frame.filter);
        self.push_filtered(node, filter, false, OutputMode::Matrix)?;
        Ok(())
    }
    fn push_all(&mut self, node: NodeId) -> Result<(), EvaluationError> {
        self.push_filtered(node, RowFilter::All, false, OutputMode::Vector)?;
        Ok(())
    }
    fn push_filtered(
        &mut self,
        node: NodeId,
        filter: RowFilter<'a>,
        bypass_order: bool,
        mode: OutputMode,
    ) -> Result<(), EvaluationError> {
        self.push_scoped(node, filter, bypass_order, mode, self.options)?;
        Ok(())
    }
    fn push_scoped(
        &mut self,
        node: NodeId,
        filter: RowFilter<'a>,
        bypass_order: bool,
        mode: OutputMode,
        options: EvaluationOptions,
    ) -> Result<(), EvaluationError> {
        let node = if bypass_order {
            node
        } else if let Some(parent) = self.frames.last() {
            self.prepared.preprocessing.child(
                self.expression,
                parent.node,
                node,
                self.control,
                self.account,
            )?
        } else {
            node
        };
        if matches!(mode, OutputMode::Vector)
            && let Some(shape) = self.rosters.matrix(node)
        {
            // Consumption belongs to the caller clock, before invariant value collapse.
            let mut frame = Frame::new(
                node,
                filter,
                bypass_order,
                mode,
                options,
                false,
                Some(self.step_index),
            );
            frame.state = State::Gather(AccountedBox::new(
                crate::matrix_heads::Step::new(shape, options, self.step_index, filter),
                &AllocationContext::new(self.account, MemoryKind::Workspace),
            ));
            self.frames.push(frame);
            return Ok(());
        }
        self.frames.push(Frame::new(
            node,
            filter,
            bypass_order,
            mode,
            options,
            !bypass_order && self.prepared.flags().boundaries[node.0],
            Some(self.step_index),
        ));
        Ok(())
    }
    pub(crate) fn set_step_index(&mut self, step_index: i64) {
        let step_index = if self.frames[0].boundary_time_ms.is_some() {
            0
        } else {
            step_index
        };
        self.frames[0].step_index = step_index;
        self.step_index = step_index;
    }
    pub(crate) fn enable_primary_statistics(
        &mut self,
        progress: &'a crate::statistics::PrimaryProgress<'a>,
    ) {
        self.primary_statistics = Some(progress);
    }
    fn credit_window(&mut self, node: NodeId, units: u64) {
        if let Some(progress) = self.primary_statistics
            && node == self.diagnostic_node
        {
            progress.add(units);
        }
    }
    pub(crate) fn restrict(&mut self, ordinals: &'a [usize]) {
        self.frames[0].filter = RowFilter::Ordinals(ordinals);
    }
    pub(crate) fn restrict_one(&mut self, ordinal: usize) {
        self.frames[0].filter = RowFilter::One(ordinal);
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
    pub(crate) fn supports_schedule(
        &self,
        selector: NodeId,
        grid: crate::query_scopes::Grid,
    ) -> bool {
        supports_selector_schedule(
            selector,
            self.selector(selector),
            self.prepared,
            self.timestamp_selectors[selector.0],
            self.options.lookback_ms,
            grid,
        )
    }

    pub(crate) fn scheduled_row(
        &self,
        root: NodeId,
        ordinal: usize,
        grid: crate::query_scopes::Grid,
        bypass_root: bool,
    ) -> Result<Option<ScheduledRow>, EvaluationError> {
        let mut node = root;
        let mut physical_ordinal = ordinal;
        loop {
            self.control.check()?;
            self.account.work(1);
            if !(node == root && bypass_root)
                && let Some((descriptors, sources)) = self.rosters.ordered(node)
            {
                let Some(descriptor) = descriptors
                    .get(physical_ordinal)
                    .filter(|descriptor| descriptor.ordinal == physical_ordinal)
                else {
                    return Ok(None);
                };
                if descriptor.sources.len() != 1 {
                    return Ok(None);
                }
                physical_ordinal = sources[descriptor.sources.start];
            }
            if matches!(self.expression.arena[node].expression, Expr::Selector(_)) {
                return Ok(self.supports_schedule(node, grid).then_some(ScheduledRow {
                    root,
                    selector: node,
                    physical_ordinal,
                    ordinal,
                    bypass_root,
                }));
            }
            let Some(child) = crate::scheduled_expression::vector_child(
                self.expression,
                self.prepared,
                node,
                self.control,
                self.account,
            )?
            else {
                return Ok(None);
            };
            node = child;
        }
    }

    pub(crate) fn visit_scheduled_points<
        S: Series,
        E: From<EvaluationError> + From<StorageError> + From<Interrupted>,
    >(
        &self,
        row: ScheduledRow,
        series: &S,
        grid: crate::query_scopes::Grid,
        mut annotations: Option<&mut AnnotationBuilder>,
        mut visit: impl FnMut(i64, i64, Option<EvaluatedSample>) -> Result<bool, E>,
    ) -> Result<u64, E> {
        let header = grid.header(self.prepared.header);
        let mut leaf = self.replay(
            row.selector,
            EvaluationOptions {
                time_ms: grid.end_ms,
                start_ms: grid.start_ms,
                end_ms: grid.end_ms,
                step_ms: grid.interval_ms,
                ..self.options
            },
            None,
        );
        leaf.primary_statistics = self.primary_statistics;
        leaf.visit_selected_series(
            row.selector,
            row.physical_ordinal,
            series,
            grid,
            None,
            |time, index, labels, value| {
                let mut machine = self.replay(
                    row.root,
                    EvaluationOptions {
                        time_ms: time,
                        start_ms: header.start_ms,
                        end_ms: header.end_ms,
                        step_ms: header.step_ms,
                        ..self.options
                    },
                    annotations.as_deref_mut(),
                );
                machine.set_step_index(index);
                machine.restrict_one(row.ordinal);
                if row.bypass_root {
                    machine.bypass_root_order();
                }
                machine.scheduled_selector = Some((row.selector, row.physical_ordinal));
                machine.selected_point = Some(SelectedPoint {
                    labels: OwnedLabels::copy_from_checked(
                        labels,
                        self.account,
                        MemoryKind::Workspace,
                        self.control,
                    )?,
                    value: value
                        .map(|sample| {
                            Ok::<_, EvaluationError>((
                                sample.time_ms,
                                sample.start_time_ms,
                                crate::statistics::ValueParts::new(
                                    OwnedValue::from_sample_checked(
                                        sample.value,
                                        self.control,
                                        self.account,
                                        MemoryKind::Workspace,
                                    )?,
                                    self.account,
                                ),
                            ))
                        })
                        .transpose()?,
                });
                let sample = machine.next()?;
                let done = visit(time, index, sample)?;
                // Complete the ordinary map's diagnostic phase even when its only point is filtered.
                if machine.next()?.is_some() {
                    return Err(EvaluationError::Invalid(
                        "scheduled expression returned multiple points",
                    )
                    .into());
                }
                Ok(done)
            },
        )
    }

    pub(crate) fn visit_scheduled_row<
        S: Series,
        E: From<EvaluationError> + From<StorageError> + From<Interrupted>,
    >(
        &self,
        row: ScheduledRow,
        series: &S,
        grid: crate::query_scopes::Grid,
        annotations: Option<&mut AnnotationBuilder>,
        mut visit: impl FnMut(i64, Option<SampleRef<'_>>) -> Result<bool, E>,
    ) -> Result<(), E> {
        self.visit_scheduled_points(row, series, grid, annotations, |time, _, sample| {
            visit(
                time,
                sample.as_ref().map(|sample| SampleRef {
                    time_ms: sample.source_time_ms,
                    start_time_ms: sample.source_start_time_ms,
                    value: sample.value.as_ref(),
                }),
            )
        })
        .map(|_| ())
    }

    pub(crate) fn visit_scheduled(
        &self,
        root: NodeId,
        ordinal: usize,
        grid: crate::query_scopes::Grid,
        mut visit: impl FnMut(i64, Option<SampleRef<'_>>) -> Result<bool, EvaluationError>,
    ) -> Result<(), EvaluationError> {
        if crate::evaluator::raw_selector_shortcut(self.expression, self.prepared, root) {
            return self
                .catalog
                .with_series(root, ordinal, |series| {
                    self.visit_raw_series(root, ordinal, series, grid, None, |time, _, value| {
                        visit(time, value)
                    })
                })
                .expect("raw scheduled ordinal")
                .map(|_| ());
        }
        let row =
            self.scheduled_row(root, ordinal, grid, false)?
                .ok_or(EvaluationError::Invalid(
                    "certified scheduled row is missing",
                ))?;
        self.catalog
            .with_series(row.selector, row.physical_ordinal, |series| {
                self.visit_scheduled_row(row, series, grid, None, visit)
            })
            .expect("computed scheduled ordinal")
    }

    fn materialize_range_bits(
        &self,
        source: crate::derived_range::BackingSource,
        writer: &mut crate::temporary_float_bits::Writer<'_>,
    ) -> Result<(), EvaluationError> {
        let mut reader = source.reader();
        if source.scheduled() {
            let grid = source.grid();
            let child = self.replay(source.node(), source.options(grid.end_ms), None);
            crate::derived_range::scheduled_reader_pass(
                &mut reader,
                self.control,
                self.account,
                |sample| {
                    if let SampleValueRef::Float(value) = sample.value
                        && !sample.value.is_stale()
                    {
                        writer.push(value.to_bits())?;
                    }
                    Ok(())
                },
                |visit| child.visit_scheduled(source.node(), source.ordinal, grid, visit),
            )?;
        } else {
            while let crate::derived_matrix::Action::Fetch(fetch) =
                reader.advance(self.control, self.account, |sample| {
                    if let SampleValueRef::Float(value) = sample.value
                        && !sample.value.is_stale()
                    {
                        writer.push(value.to_bits())?;
                    }
                    Ok(())
                })?
            {
                let mut child = self.replay(source.node(), source.options(fetch.time_ms), None);
                child.restrict_one(source.ordinal);
                while let Some(sample) = child.next()? {
                    if reader.receive(sample) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
    fn map(&self, operation: MapOperation) -> State<'a> {
        State::Map {
            operation,
            seen: AccountedVec::new(self.account, MemoryKind::Workspace),
            last_diagnostic: None,
        }
    }
    fn sample(&self, value: f64) -> EvaluatedSample {
        self.sample_owned(
            value,
            Some(crate::statistics::OwnedSamples::new(self.account, 1)),
        )
    }
    fn sample_owned(
        &self,
        value: f64,
        float_samples: Option<crate::statistics::OwnedSamples>,
    ) -> EvaluatedSample {
        EvaluatedSample {
            ordinal: 0,
            labels: OwnedLabels::new(self.account, MemoryKind::Workspace),
            float_samples,
            value: OwnedValue::Float(value),
            source_time_ms: self.options.time_ms,
            source_start_time_ms: 0,
            drop_name: false,
        }
    }
    fn rebind(mut sample: EvaluatedSample, boundary_time_ms: Option<i64>) -> EvaluatedSample {
        if let Some(time_ms) = boundary_time_ms {
            sample.source_time_ms = time_ms;
            sample.source_start_time_ms = 0;
        }
        sample
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
        self.matrix_bounds_ms(end_ms, range_ns / 1_000_000)
    }
    fn matrix_bounds_ms(&self, end_ms: i64, range_ms: i64) -> Option<TimeBounds> {
        let start_ms = end_ms.wrapping_sub(range_ms).checked_add(1)?;
        let bounds = TimeBounds {
            start_ms: start_ms.max(self.data_bounds.start_ms),
            end_ms: end_ms.min(self.data_bounds.end_ms),
        };
        (bounds.start_ms <= bounds.end_ms).then_some(bounds)
    }
    fn matrix_window(&self, node: NodeId, range_ns: i64, end_ms: i64) -> MatrixWindow {
        let selector = self.selector(node);
        MatrixWindow {
            first_end_ms: if self.invariants[node.0]
                || self.prepared.timestamp(node, &selector.modifiers).is_some()
            {
                end_ms
            } else {
                self.options.start_ms.wrapping_sub(
                    self.prepared
                        .adjusted_offset(node, &selector.modifiers, self.options.start_ms)
                        / 1_000_000,
                )
            },
            range_ms: range_ns / 1_000_000,
            buffer_ms: None,
            end_ms,
            step_ms: self.options.step_ms,
            data_bounds: self.data_bounds,
            statistics: selector.skip_histogram_buckets,
            reduce_delta: true,
            resume: None,
        }
    }

    fn fold_request(
        &self,
        identity: (NodeId, NodeId, usize),
        operation: SelectorOperation,
        window: MatrixWindow,
        input: crate::range_cache::Input,
    ) -> crate::range_cache::Request {
        let (function, parameters, range_ns) = match operation {
            SelectorOperation::Rate { kind, range_ns } => {
                (RangeFunction::Rate(kind), [0.0; 2], range_ns)
            }
            SelectorOperation::Fold { kind, range_ns } => {
                (RangeFunction::Fold(kind), [0.0; 2], range_ns)
            }
            SelectorOperation::Numeric {
                function,
                parameters,
                range_ns,
            } => (RangeFunction::Numeric(function), parameters, range_ns),
            SelectorOperation::Instant => unreachable!("range fold request"),
        };
        crate::range_cache::Request {
            owner: identity.0,
            selector: identity.1,
            ordinal: identity.2,
            options: self.options,
            lookback_ns: self.prepared.lookback_ns(self.options.lookback_ms),
            position: self.step_index,
            function,
            parameters,
            range_ns,
            window,
            input,
        }
    }

    fn smooths(&self, node: NodeId) -> bool {
        self.selector(node).smoothed && !self.timestamp_selectors[node.0]
    }

    fn smoothed_value<S: Series>(
        &self,
        node: NodeId,
        series: &S,
        time_ms: i64,
        grid: crate::query_scopes::Grid,
    ) -> Result<range_rates::RateOutput, StorageError> {
        let selector = self.selector(node);
        let offset_ms = self
            .prepared
            .adjusted_offset(node, &selector.modifiers, grid.start_ms)
            / 1_000_000;
        let lookback_ns = self.prepared.lookback_ns(self.options.lookback_ms);
        let lookback_ms = lookback_ns / 1_000_000;
        let data_ms = time_ms.wrapping_sub(offset_ms);
        let range_ms = lookback_ms.wrapping_mul(2);
        let input = RateInput {
            window: RateWindow {
                start_ms: data_ms.wrapping_sub(lookback_ms),
                end_ms: data_ms.wrapping_add(lookback_ms),
                range_ns: lookback_ns.wrapping_mul(2),
                start_timestamps: false,
            },
            range_start_ms: data_ms,
            range_end_ms: data_ms,
            data_bounds: Some(self.data_bounds),
        };
        let matrix = MatrixWindow {
            first_end_ms: grid
                .start_ms
                .wrapping_sub(offset_ms)
                .wrapping_add(lookback_ms),
            end_ms: input.window.end_ms,
            range_ms,
            buffer_ms: Some(
                grid.end_ms
                    .wrapping_sub(grid.start_ms)
                    .wrapping_add(range_ms),
            ),
            step_ms: grid.interval_ms,
            data_bounds: self.data_bounds,
            statistics: selector.skip_histogram_buckets,
            reduce_delta: false,
            resume: None,
        };
        crate::range_extended::smooth(series, input, matrix, self.control, self.account)
    }

    fn select(
        &self,
        selector_node: NodeId,
        operation: SelectorOperation,
    ) -> Result<State<'a>, EvaluationError> {
        let selector = self.selector(selector_node);
        let anchored_safe = matches!(
            operation,
            SelectorOperation::Instant
                | SelectorOperation::Rate {
                    kind: RateFunction::Extrapolated(_),
                    ..
                }
                | SelectorOperation::Fold {
                    kind: FoldKind::Changes | FoldKind::Resets,
                    ..
                }
        );
        let smoothed_safe = matches!(
            operation,
            SelectorOperation::Instant
                | SelectorOperation::Rate {
                    kind: RateFunction::Extrapolated(_),
                    ..
                }
        );
        if selector.smoothed && !smoothed_safe || selector.anchored && !anchored_safe {
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
        let (argument, selector) = raw_matrix_argument(
            self.expression,
            self.prepared,
            node,
            self.control,
            self.account,
        )?;
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
        let (_, argument) = matrix_argument(
            self.expression,
            self.prepared,
            node,
            self.control,
            self.account,
        )?
        .expect("typed range call");
        if let Expr::Subquery {
            expression: body,
            range,
            modifiers,
            ..
        } = &self.expression.arena[argument].expression
        {
            let grid = self
                .prepared
                .subquery_scope(argument, self.control, self.account)?;
            let body = self.prepared.preprocessing.child(
                self.expression,
                argument,
                *body,
                self.control,
                self.account,
            )?;
            let fixed = self.prepared.timestamp(argument, modifiers);
            let offset_ns = if let Some(timestamp) = fixed {
                self.prepared
                    .offset(argument, modifiers.original_offset)
                    .wrapping_add(
                        self.options
                            .start_ms
                            .wrapping_sub(timestamp)
                            .wrapping_mul(1_000_000),
                    )
            } else {
                self.prepared
                    .adjusted_offset(argument, modifiers, self.options.start_ms)
            };
            let filter = if matches!(&self.expression.arena[node].expression,
                Expr::Call { function: Some(function), .. } if function.name == "absent_over_time")
            {
                RowFilter::All
            } else {
                self.frames
                    .last()
                    .map_or(RowFilter::All, |frame| frame.filter)
            };
            return Ok(State::Range(AccountedBox::new(
                crate::derived_range::Step::new(
                    crate::derived_range::Input {
                        body,
                        grid,
                        options: self.options,
                        function,
                        parameters,
                        range_ns: self.prepared.range(argument, *range),
                        offset_ns,
                        fixed: fixed.is_some(),
                        scheduled: raw_selector_shortcut(self.expression, self.prepared, body)
                            || self
                                .rosters
                                .scheduled_selector(body)
                                .is_some_and(|selector| self.supports_schedule(selector, grid)),
                    },
                    self.rosters.descriptors(body),
                    filter,
                    self.account,
                ),
                &AllocationContext::new(self.account, MemoryKind::Workspace),
            )));
        }
        let (selector, operation) = self.range_spec(node, function, parameters)?;
        self.select(selector, operation)
    }
    fn instant_window(&self, node: NodeId) -> crate::instant_select::InstantWindow {
        let end_ms = self.lookup(node);
        crate::instant_select::InstantWindow {
            first_lookup_ms: if self.invariants[node.0] {
                end_ms
            } else {
                self.lookup_at(node, self.options.start_ms)
            },
            lookup_ms: end_ms,
            step_ms: self.options.step_ms,
            lookback_ms: self.options.lookback_ms,
            delta_ms: self
                .options
                .lookback_ms
                .wrapping_sub(i64::from(self.timestamp_selectors[node.0])),
            data_bounds: self.data_bounds,
            unit_weight: self.timestamp_selectors[node.0],
        }
    }
    pub(crate) fn visit_raw_series<
        S: Series,
        E: From<EvaluationError> + From<StorageError> + From<Interrupted>,
    >(
        &self,
        node: NodeId,
        ordinal: usize,
        series: &S,
        grid: crate::query_scopes::Grid,
        annotations: Option<&mut AnnotationBuilder>,
        mut visit: impl FnMut(
            i64,
            &[crate::storage::LabelRef<'_>],
            Option<SampleRef<'_>>,
        ) -> Result<bool, E>,
    ) -> Result<u64, E> {
        self.visit_selected_series(
            node,
            ordinal,
            series,
            grid,
            annotations,
            |time, index, labels, value| {
                if value.is_some() && !self.smooths(node) {
                    self.prepared.sample_statistics_index(
                        node,
                        index,
                        self.control,
                        self.account,
                    )?;
                }
                let sample = value.map(|sample| {
                    if self.frames[0].boundary_time_ms.is_some()
                        || !self.timestamp_selectors[node.0]
                    {
                        SampleRef {
                            time_ms: time,
                            start_time_ms: 0,
                            ..sample
                        }
                    } else {
                        sample
                    }
                });
                visit(time, labels, sample)
            },
        )
    }
    fn visit_selected_series<
        S: Series,
        E: From<EvaluationError> + From<StorageError> + From<Interrupted>,
    >(
        &self,
        node: NodeId,
        ordinal: usize,
        series: &S,
        grid: crate::query_scopes::Grid,
        mut annotations: Option<&mut AnnotationBuilder>,
        mut visit: impl FnMut(
            i64,
            i64,
            &[crate::storage::LabelRef<'_>],
            Option<SampleRef<'_>>,
        ) -> Result<bool, E>,
    ) -> Result<u64, E> {
        let mut sample_units = 0;
        if self
            .prepared
            .activate(
                grid.header(self.prepared.header),
                self.control,
                self.account,
            )?
            .is_none()
        {
            return Ok(0);
        }
        self.select(node, SelectorOperation::Instant)?;
        self.control.check()?;
        let labels = series.label_views_checked(self.account, self.control)?;
        validate_labels_checked(&labels, self.account, self.control)?;
        let selector = self.selector(node);
        if !matches_checked(
            &labels,
            &selector.matchers,
            Some(self.account),
            self.control,
        )? {
            return Ok(0);
        }
        if self.smooths(node) {
            let mut traversal = crate::query_scopes::Traversal::new(grid, false);
            while let Some((time, index)) = traversal.current() {
                self.control.check()?;
                self.account.work(1);
                let output = self.smoothed_value(node, series, time, grid)?;
                crate::statistics::add_units(
                    &mut sample_units,
                    output.progress.map_or(0, |progress| progress.sample_units),
                    self.account,
                );
                if let Some(progress) = self.primary_statistics {
                    progress.add(output.progress.map_or(0, |progress| progress.sample_units));
                }
                if let Some(annotations) = &mut annotations {
                    for kind in output.annotations.into_iter().flatten() {
                        annotations.add(
                            kind,
                            label(&labels, b"__name__"),
                            self.expression.arena[node].span,
                            &self.expression.source,
                            self.control,
                            self.account,
                        )?;
                    }
                }
                let sample = output.value.as_ref().map(|value| SampleRef {
                    time_ms: time,
                    start_time_ms: 0,
                    value: value.as_ref(),
                });
                if visit(time, index, &labels, sample)? {
                    break;
                }
                traversal.advance();
            }
            return Ok(sample_units);
        }
        let empty = match self.rosters.clock_initial(node, ordinal) {
            Some(crate::storage::InitialSample::Empty) => true,
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
            _ => false,
        };
        let fixed = self.invariants[node.0];
        if empty {
            let mut traversal = crate::query_scopes::Traversal::new(grid, false);
            while let Some((time, index)) = traversal.current() {
                self.control.check()?;
                self.account.work(1);
                if visit(time, index, &labels, None)? {
                    break;
                }
                traversal.advance();
            }
            return Ok(0);
        }
        let mut logical = crate::query_scopes::Traversal::new(grid, false);
        let mut emit = |value: Option<(i64, i64, crate::value::SampleValueRef<'_>)>| {
            loop {
                self.control.check()?;
                self.account.work(1);
                let Some((logical_time, index)) = logical.current() else {
                    return Ok::<_, E>(false);
                };
                let sample = value.map(|(time_ms, start_time_ms, value)| SampleRef {
                    time_ms,
                    start_time_ms,
                    value,
                });
                if let Some(sample) = sample {
                    let units = if self.timestamp_selectors[node.0] {
                        1
                    } else {
                        crate::statistics::weight(sample.value)
                    };
                    crate::statistics::add_units(&mut sample_units, units, self.account);
                    if let Some(progress) = self.primary_statistics {
                        progress.add(units);
                    }
                }
                if visit(logical_time, index, &labels, sample)? {
                    return Ok::<_, E>(true);
                }
                logical.advance();
                if !fixed || logical.current().is_none() {
                    return Ok(false);
                }
            }
        };
        let mut position = crate::query_scopes::Traversal::new(grid, false);
        let next_lookup = || {
            if fixed {
                return Ok(None);
            }
            position.advance();
            Ok(position
                .current()
                .map(|(time, _)| self.lookup_at(node, time)))
        };
        let first_lookup_ms = self.lookup_at(node, grid.start_ms);
        let window = crate::instant_select::InstantWindow {
            first_lookup_ms,
            lookup_ms: first_lookup_ms,
            ..self.instant_window(node)
        };
        if selector.skip_histogram_buckets {
            crate::instant_statistics::visit_scheduled(
                series,
                window,
                self.control,
                self.account,
                next_lookup,
                |_, value| emit(value),
            )?;
        } else {
            crate::instant_select::visit_mapped(
                series,
                window,
                self.control,
                self.account,
                |value| {
                    Ok::<_, E>(crate::statistics::ValueParts::new(
                        OwnedValue::from_sample_checked(
                            value,
                            self.control,
                            self.account,
                            MemoryKind::Workspace,
                        )?,
                        self.account,
                    ))
                },
                next_lookup,
                |_, value| emit(value.map(|(time, start, value)| (time, start, value.as_ref()))),
            )?;
        }
        Ok(sample_units)
    }
    pub(crate) fn range_parameters(
        &self,
        node: NodeId,
        function: RangeFunction,
    ) -> Result<crate::statistics::FloatParameters, EvaluationError> {
        self.control.check()?;
        self.account.work(1);
        let mut parameters = crate::statistics::FloatParameters::default();
        if let RangeFunction::Numeric(kind) = function {
            for (position, argument) in scalar_arguments(self.expression, node)
                .take(kind.parameters())
                .enumerate()
            {
                let argument = self.prepared.preprocessing.child(
                    self.expression,
                    node,
                    argument,
                    self.control,
                    self.account,
                )?;
                let options = self.range_parameter_options(argument)?;
                let mut child = self.replay(argument, options, None);
                child.frames[0].mode = OutputMode::Vector;
                while let Some(sample) = child.next()? {
                    let OwnedValue::Float(value) = sample.value else {
                        unreachable!("scalar parameter")
                    };
                    parameters.values[position] = value;
                    parameters.samples[position] = sample.float_samples;
                }
            }
        }
        Ok(parameters)
    }

    fn range_parameter_options(
        &self,
        argument: NodeId,
    ) -> Result<EvaluationOptions, EvaluationError> {
        let shape = self
            .rosters
            .scalar(argument)
            .ok_or(EvaluationError::Invalid(
                "completed scalar parameter shape is missing",
            ))?;
        self.prepared
            .index_error(0, shape.rows, self.control, self.account)?;
        self.prepared
            .index_error(self.step_index, shape.floats, self.control, self.account)?;
        Ok(EvaluationOptions {
            time_ms: shape
                .grid
                .start_ms
                .wrapping_add(shape.grid.interval_ms.wrapping_mul(self.step_index)),
            start_ms: shape.grid.start_ms,
            end_ms: shape.grid.end_ms,
            step_ms: shape.grid.interval_ms,
            ..self.options
        })
    }

    fn push_parameter(&mut self, child: NodeId, range: bool) -> Result<(), EvaluationError> {
        let parent = self.frames.last().expect("parameter parent").node;
        let child = self.prepared.preprocessing.child(
            self.expression,
            parent,
            child,
            self.control,
            self.account,
        )?;
        let options = if range {
            self.range_parameter_options(child)?
        } else {
            self.options
        };
        let mode = if range {
            OutputMode::Matrix
        } else {
            OutputMode::Vector
        };
        self.push_scoped(child, RowFilter::All, false, mode, options)?;
        Ok(())
    }

    fn aggregate_parameter_options(
        &self,
        child: NodeId,
    ) -> Result<Option<EvaluationOptions>, EvaluationError> {
        let parent = self.frames.last().expect("aggregate parameter parent").node;
        let child = self.prepared.preprocessing.child(
            self.expression,
            parent,
            child,
            self.control,
            self.account,
        )?;
        let shape = self.rosters.scalar(child).ok_or(EvaluationError::Invalid(
            "completed aggregate parameter shape is missing",
        ))?;
        if self.step_index < 0 || self.step_index >= shape.floats {
            return Ok(None);
        }
        Ok(Some(EvaluationOptions {
            time_ms: shape
                .grid
                .start_ms
                .wrapping_add(shape.grid.interval_ms.wrapping_mul(self.step_index)),
            start_ms: shape.grid.start_ms,
            end_ms: shape.grid.end_ms,
            step_ms: shape.grid.interval_ms,
            ..self.options
        }))
    }
    pub(crate) fn initialize_range_series<S: Series>(
        &self,
        selector: NodeId,
        series: &S,
    ) -> Result<Option<InitialSample>, EvaluationError> {
        let labels = series.label_views_checked(self.account, self.control)?;
        validate_labels_checked(&labels, self.account, self.control)?;
        if !matches_checked(
            &labels,
            &self.selector(selector).matchers,
            Some(self.account),
            self.control,
        )? {
            return Ok(None);
        }
        Ok(Some(crate::storage::initialize(
            series,
            self.data_bounds,
            self.control,
            self.account,
        )?))
    }

    pub(crate) fn range_series_sample<S: Series>(
        &mut self,
        node: NodeId,
        function: RangeFunction,
        parameters: [f64; 2],
        series_ordinal: usize,
        series: &S,
        initial: Option<&InitialSample>,
    ) -> Result<Option<EvaluatedSample>, EvaluationError> {
        self.control.check()?;
        self.account.work(1);
        self.prepared.activate(
            crate::query_scopes::Grid {
                start_ms: self.options.start_ms,
                end_ms: self.options.end_ms,
                interval_ms: self.options.step_ms,
            }
            .header(self.prepared.header),
            self.control,
            self.account,
        )?;
        let (selector, operation) = self.range_spec(node, function, parameters)?;
        let sample =
            self.series_sample(node, selector, operation, series_ordinal, series, initial)?;
        Ok(sample.map(|sample| Self::rebind(sample, self.frames[0].boundary_time_ms)))
    }
    fn series_sample<S: Series>(
        &mut self,
        node: NodeId,
        selector_node: NodeId,
        operation: SelectorOperation,
        series_ordinal: usize,
        series: &S,
        initial: Option<&InitialSample>,
    ) -> Result<Option<EvaluatedSample>, EvaluationError> {
        let series_labels = series.label_views_checked(self.account, self.control)?;
        validate_labels_checked(&series_labels, self.account, self.control)?;
        let selector = self.selector(selector_node);
        if !matches_checked(
            &series_labels,
            &selector.matchers,
            Some(self.account),
            self.control,
        )? {
            return Ok(None);
        }
        if let Some(initial) = initial
            && let SelectorOperation::Numeric { range_ns, .. } = operation
            && !initial.range_ready(range_ns / 1_000_000, self.control, self.account)?
        {
            return Ok(None);
        }
        let end_ms = if !matches!(operation, SelectorOperation::Instant)
            && self
                .prepared
                .timestamp(selector_node, &selector.modifiers)
                .is_some()
        {
            self.lookup_at(selector_node, self.options.start_ms)
        } else {
            self.lookup(selector_node)
        };
        let value = match operation {
            SelectorOperation::Instant if self.smooths(selector_node) => {
                let fallback = crate::query_scopes::Grid {
                    start_ms: self.options.start_ms,
                    end_ms: self.options.end_ms,
                    interval_ms: self.options.step_ms,
                }
                .header(self.prepared.header);
                let grid = self.prepared.scope_header(selector_node, fallback).into();
                let result =
                    self.smoothed_value(selector_node, series, self.options.time_ms, grid)?;
                if let Some(annotations) = &mut self.annotations {
                    for kind in result.annotations.into_iter().flatten() {
                        annotations.add(
                            kind,
                            label(&series_labels, b"__name__"),
                            self.expression.arena[selector_node].span,
                            &self.expression.source,
                            self.control,
                            self.account,
                        )?;
                    }
                }
                result
                    .into_parts()
                    .map(|value| (self.options.time_ms, 0, value))
            }
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
                let window = self.instant_window(selector_node);
                let options = self.options;
                let mut position = crate::query_scopes::Traversal::new(
                    crate::query_scopes::Grid {
                        start_ms: options.start_ms,
                        end_ms: options.end_ms,
                        interval_ms: options.step_ms,
                    },
                    false,
                );
                let target_index = self.step_index;
                let next_lookup = || {
                    if fixed
                        || position
                            .current()
                            .is_some_and(|(_, index)| index == target_index)
                    {
                        return Ok(None);
                    }
                    position.advance();
                    Ok(position
                        .current()
                        .map(|(time, _)| self.lookup_at(selector_node, time)))
                };
                let result = if selector.skip_histogram_buckets {
                    crate::instant_statistics::select_scheduled(
                        series,
                        window,
                        self.control,
                        self.account,
                        next_lookup,
                    )?
                } else {
                    crate::instant_select::select_mapped::<_, _, StorageError>(
                        series,
                        window,
                        self.control,
                        self.account,
                        |value| {
                            Ok(crate::statistics::ValueParts::new(
                                OwnedValue::from_sample_checked(
                                    value,
                                    self.control,
                                    self.account,
                                    MemoryKind::Workspace,
                                )?,
                                self.account,
                            ))
                        },
                        next_lookup,
                    )?
                };
                if result.is_some() {
                    self.note_instant_sample(node, selector_node)?;
                }
                result
            }
            SelectorOperation::Rate { kind, range_ns } => {
                let mut input = RateInput {
                    window: RateWindow {
                        start_ms: end_ms.wrapping_sub(range_ns / 1_000_000),
                        end_ms,
                        range_ns,
                        start_timestamps: self.options.start_timestamps,
                    },
                    range_start_ms: self.options.time_ms.wrapping_sub(
                        range_ns.wrapping_add(self.prepared.adjusted_offset(
                            selector_node,
                            &selector.modifiers,
                            self.options.start_ms,
                        )) / 1_000_000,
                    ),
                    range_end_ms: self.options.time_ms.wrapping_sub(
                        self.prepared.adjusted_offset(
                            selector_node,
                            &selector.modifiers,
                            self.options.start_ms,
                        ) / 1_000_000,
                    ),
                    data_bounds: Some(self.data_bounds),
                };
                let mut matrix = self.matrix_window(selector_node, range_ns, end_ms);
                let extended = if selector.anchored || selector.smoothed {
                    let RateFunction::Extrapolated(kind) = kind else {
                        unreachable!("validated extended rate function")
                    };
                    let lookback_ns = self.prepared.lookback_ns(self.options.lookback_ms);
                    let lookback_ms = lookback_ns / 1_000_000;
                    matrix.range_ms = matrix.range_ms.wrapping_add(if selector.smoothed {
                        lookback_ms.wrapping_mul(2)
                    } else {
                        lookback_ms
                    });
                    matrix.buffer_ms =
                        Some((range_ns / 1_000_000).wrapping_add(if selector.smoothed {
                            lookback_ns.wrapping_mul(2) / 1_000_000
                        } else {
                            lookback_ms
                        }));
                    if selector.smoothed {
                        matrix.first_end_ms = matrix.first_end_ms.wrapping_add(lookback_ms);
                        matrix.end_ms = matrix.end_ms.wrapping_add(lookback_ms);
                    }
                    input.window.start_ms = matrix.end_ms.wrapping_sub(matrix.range_ms);
                    input.window.end_ms = matrix.end_ms;
                    Some(kind)
                } else {
                    None
                };
                let request = self.fold_request(
                    (node, selector_node, series_ordinal),
                    operation,
                    matrix,
                    input.into(),
                );
                let result = self.prepared.range_folds.evaluate(
                    request,
                    self.control,
                    self.account,
                    |matrix| {
                        let output = if let Some(kind) = extended {
                            crate::range_extended::evaluate(
                                series,
                                input,
                                kind,
                                selector.smoothed,
                                matrix,
                                self.control,
                                self.account,
                            )?
                        } else {
                            range_rates::evaluate(
                                series,
                                input,
                                kind,
                                Some(matrix),
                                self.control,
                                self.account,
                            )?
                        };
                        Ok(output.into())
                    },
                )?;
                if node == self.diagnostic_node
                    && let Some(annotations) = &mut self.annotations
                {
                    let Expr::Call { arguments, .. } = &self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    let argument = self.prepared.preprocessing.child(
                        self.expression,
                        node,
                        arguments[0],
                        self.control,
                        self.account,
                    )?;
                    let span = self.expression.arena[argument].span;
                    let metric = label(&series_labels, b"__name__");
                    for kind in result.annotations.into_iter().flatten() {
                        annotations.add(
                            kind,
                            metric,
                            span,
                            &self.expression.source,
                            self.control,
                            self.account,
                        )?;
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
                            self.control,
                            self.account,
                        )?;
                    }
                }
                self.credit_window(node, result.sample_units());
                if result.nonempty {
                    self.prepared.sample_statistics_index(
                        node,
                        self.step_index,
                        self.control,
                        self.account,
                    )?;
                }
                result
                    .into_parts()
                    .map(|value| (self.options.time_ms, 0, value))
            }
            SelectorOperation::Fold { kind, range_ns } => {
                let mut matrix = self.matrix_window(selector_node, range_ns, end_ms);
                let anchor_start_ms = if selector.anchored {
                    matrix.range_ms = matrix.range_ms.wrapping_add(self.options.lookback_ms);
                    Some(self.options.time_ms.wrapping_sub(
                        range_ns.wrapping_add(self.prepared.adjusted_offset(
                            selector_node,
                            &selector.modifiers,
                            self.options.start_ms,
                        )) / 1_000_000,
                    ))
                } else {
                    None
                };
                let bounds = self.matrix_bounds_ms(end_ms, matrix.range_ms);
                let request = self.fold_request(
                    (node, selector_node, series_ordinal),
                    operation,
                    matrix,
                    crate::range_cache::Input::fold(bounds, anchor_start_ms),
                );
                let result = self.prepared.range_folds.evaluate(
                    request,
                    self.control,
                    self.account,
                    |matrix| {
                        Ok(fold(
                            series,
                            bounds,
                            kind,
                            FoldOptions {
                                start_timestamps: self.options.start_timestamps,
                                matrix: Some(matrix),
                                anchor_start_ms,
                            },
                            self.control,
                            self.account,
                        )?
                        .into())
                    },
                )?;
                if node == self.diagnostic_node
                    && let Some(annotations) = &mut self.annotations
                {
                    let Expr::Call { arguments, .. } = &self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    let argument = self.prepared.preprocessing.child(
                        self.expression,
                        node,
                        arguments[0],
                        self.control,
                        self.account,
                    )?;
                    for kind in result.annotations.into_iter().flatten() {
                        annotations.add(
                            kind,
                            label(&series_labels, b"__name__"),
                            self.expression.arena[argument].span,
                            &self.expression.source,
                            self.control,
                            self.account,
                        )?;
                    }
                }
                self.credit_window(node, result.sample_units());
                if result.nonempty {
                    self.prepared.sample_statistics_index(
                        node,
                        self.step_index,
                        self.control,
                        self.account,
                    )?;
                }
                result
                    .into_parts()
                    .map(|value| (self.options.time_ms, 0, value))
            }
            SelectorOperation::Numeric {
                function,
                parameters,
                range_ns,
            } => {
                let bounds = self.matrix_bounds(end_ms, range_ns);
                let matrix = self.matrix_window(selector_node, range_ns, end_ms);
                let input = NumericRangeInput {
                    bounds,
                    seek_ms: end_ms.wrapping_sub(range_ns / 1_000_000),
                    matrix: Some(matrix),
                };
                let request = self.fold_request(
                    (node, selector_node, series_ordinal),
                    operation,
                    matrix,
                    input.into(),
                );
                let result = self.prepared.range_folds.evaluate(
                    request,
                    self.control,
                    self.account,
                    |matrix| {
                        range_numeric::evaluate(
                            self.catalog.query(),
                            series,
                            NumericRangeInput {
                                matrix: Some(matrix),
                                ..input
                            },
                            function,
                            parameters,
                            self.options.time_ms,
                            self.control,
                            self.account,
                        )
                        .map(Into::into)
                        .map_err(|error| numeric_range_error(error, self.account))
                    },
                )?;
                if node == self.diagnostic_node
                    && let Some(annotations) = &mut self.annotations
                {
                    let Expr::Call { arguments, .. } = &self.expression.arena[node].expression
                    else {
                        unreachable!()
                    };
                    let argument = self.prepared.preprocessing.child(
                        self.expression,
                        node,
                        arguments[0],
                        self.control,
                        self.account,
                    )?;
                    let span = self.expression.arena[argument].span;
                    if result.invalid_quantile {
                        annotations.add_invalid_quantile(
                            parameters[0],
                            span,
                            &self.expression.source,
                            self.control,
                            self.account,
                        )?;
                    }
                    for kind in result.annotations.into_iter().flatten() {
                        annotations.add(
                            kind,
                            label(&series_labels, b"__name__"),
                            span,
                            &self.expression.source,
                            self.control,
                            self.account,
                        )?;
                    }
                }
                self.credit_window(node, result.sample_units());
                if result.nonempty {
                    self.prepared.sample_statistics_index(
                        node,
                        self.step_index,
                        self.control,
                        self.account,
                    )?;
                }
                result
                    .into_parts()
                    .map(|value| (self.options.time_ms, 0, value))
            }
        };
        self.finish_selector_sample(
            selector_node,
            operation,
            series_ordinal,
            &series_labels,
            value,
        )
    }
    fn note_instant_sample(&self, node: NodeId, selector: NodeId) -> Result<(), EvaluationError> {
        if self.timestamp_selectors[selector.0] {
            self.prepared.sample_statistics_time(
                node,
                self.options.time_ms,
                self.control,
                self.account,
            )?;
        } else {
            self.prepared.sample_statistics_index(
                node,
                self.step_index,
                self.control,
                self.account,
            )?;
        }
        Ok(())
    }
    fn finish_selector_sample(
        &self,
        selector_node: NodeId,
        operation: SelectorOperation,
        series_ordinal: usize,
        series_labels: &[crate::storage::LabelRef<'_>],
        value: Option<(i64, i64, crate::statistics::ValueParts)>,
    ) -> Result<Option<EvaluatedSample>, EvaluationError> {
        value
            .map(|(source_time_ms, source_start_time_ms, value)| {
                let (source_time_ms, source_start_time_ms) =
                    if matches!(operation, SelectorOperation::Instant)
                        && !self.timestamp_selectors[selector_node.0]
                    {
                        (self.options.time_ms, 0)
                    } else {
                        (source_time_ms, source_start_time_ms)
                    };
                let mut labels = OwnedLabels::copy_from_checked(
                    series_labels,
                    self.account,
                    MemoryKind::Workspace,
                    self.control,
                )?;
                let drop_name = operation.drops_name();
                if drop_name && !self.options.delayed_name_removal {
                    drop_reserved(&mut labels, false, self.account, self.control)?;
                }
                let crate::statistics::ValueParts {
                    value,
                    float_samples,
                } = value;
                Ok::<_, EvaluationError>(EvaluatedSample {
                    ordinal: series_ordinal,
                    labels,
                    float_samples,
                    value,
                    source_time_ms,
                    source_start_time_ms,
                    drop_name,
                })
            })
            .transpose()
    }

    fn enter(&mut self, index: usize) -> Result<Option<EvaluatedSample>, EvaluationError> {
        let node = self.frames[index].node;
        if self
            .prepared
            .activate(
                crate::query_scopes::Grid {
                    start_ms: self.options.start_ms,
                    end_ms: self.options.end_ms,
                    interval_ms: self.options.step_ms,
                }
                .header(self.prepared.header),
                self.control,
                self.account,
            )?
            .is_none()
        {
            self.frames.pop();
            return Ok(None);
        }
        let expression = &self.expression.arena[node].expression;
        if !self.frames[index].bypass_order
            && !crate::count_values::is_count(expression)
            && !crate::topk::is_selection(expression)
            && !self
                .rosters
                .sorting(node)
                .is_some_and(|plan| plan.singleton)
            && let Some((descriptors, sources)) = self.rosters.ordered(node)
        {
            self.frames[index].state = State::OrderedRows {
                descriptors,
                sources,
                position: 0,
                running: false,
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
                self.push_matrix(*child)?;
                Ok(None)
            }
            Expr::Unary {
                operator,
                expression,
            } => {
                self.frames[index].state = self.map(MapOperation::Unary(*operator));
                self.push_matrix(*expression)?;
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
                self.push_all(*child)?;
                Ok(None)
            }
            Expr::Aggregate {
                operator:
                    TokenKind::TOPK | TokenKind::BOTTOMK | TokenKind::LIMITK | TokenKind::LIMIT_RATIO,
                ..
            } => {
                self.frames[index].state = State::TopK(AccountedBox::new(
                    crate::topk::Step::new(
                        self.rosters
                            .selection(node)
                            .ok_or(EvaluationError::Invalid("selection plan is missing"))?,
                        self.options,
                        self.step_index,
                        !self.frames[index].bypass_order,
                        matches!(self.frames[index].mode, OutputMode::Vector),
                        self.rosters.ordered(node).map(|(rows, _)| rows),
                        self.account,
                    ),
                    &AllocationContext::new(self.account, MemoryKind::Workspace),
                ));
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
                    if !self.frames[index]
                        .filter
                        .accepts(ordinal, self.control, self.account)?
                    {
                        continue;
                    }
                    groups.push(Group::new(labels, ordinal));
                }
                let lookup = if groups.len() > 1 {
                    let mut lookup = crate::position_index::PositionIndex::new(self.account);
                    for position in 0..groups.len() {
                        let hash = groups[position].hash;
                        if lookup
                            .find(
                                hash,
                                |prior| Ok(groups[prior].hash == hash),
                                self.control,
                                self.account,
                            )?
                            .is_none()
                        {
                            lookup.insert_new(
                                hash,
                                position,
                                |prior| Ok(groups[prior].hash),
                                self.control,
                                self.account,
                            )?;
                        }
                    }
                    Some(AccountedBox::new(
                        lookup,
                        &AllocationContext::new(self.account, MemoryKind::Workspace),
                    ))
                } else {
                    None
                };
                self.frames[index].state = State::Aggregate {
                    groups,
                    lookup,
                    collecting: true,
                };
                self.push_all(*child)?;
                Ok(None)
            }
            Expr::Aggregate {
                operator: TokenKind::QUANTILE,
                ..
            } => {
                self.frames[index].state = State::Quantile(AccountedBox::new(
                    crate::quantile_aggregate::Step::new(
                        self.rosters
                            .quantile(node)
                            .ok_or(EvaluationError::Invalid("quantile plan is missing"))?,
                        self.rosters
                            .groups(node)
                            .ok_or(EvaluationError::Invalid("quantile groups are missing"))?,
                        self.options,
                        self.step_index,
                        self.account,
                    ),
                    &AllocationContext::new(self.account, MemoryKind::Workspace),
                ));
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
                    self.push_all(*left)?;
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
                    self.push_all(scalar_node)?;
                } else {
                    let mut step = crate::vector_matching::Step::new(
                        self.rosters
                            .binary(node)
                            .ok_or(EvaluationError::Invalid("binary plan is missing"))?,
                        matching.as_ref().expect("typed vector matching"),
                        *operator,
                        *return_bool,
                        self.account,
                    );
                    let frame = &self.frames[index];
                    if operator.is_set()
                        && set_discovery_target(
                            frame.mode,
                            frame.bypass_order,
                            frame.filter,
                            node,
                            self.binary_preparation,
                        )
                        .is_some()
                    {
                        step.enable_set_discovery(
                            crate::vector_matching::ReplayKey::new(
                                frame.options,
                                frame.step_index,
                                self.prepared.lookback_ns(frame.options.lookback_ms),
                                self.data_bounds,
                            ),
                            self.control,
                            self.account,
                        )?;
                    }
                    if matches!(frame.mode, OutputMode::Matrix)
                        && frame.bypass_order
                        && self.binary_preparation.is_none()
                        && let Some(target) = match frame.filter {
                            RowFilter::Ordinals([target]) => Some(*target),
                            RowFilter::One(target) => Some(target),
                            _ => None,
                        }
                    {
                        step.enable_replay(
                            target,
                            crate::vector_matching::ReplayKey::new(
                                frame.options,
                                frame.step_index,
                                self.prepared.lookback_ns(frame.options.lookback_ms),
                                self.data_bounds,
                            ),
                            self.control,
                            self.account,
                        )?;
                    }
                    self.frames[index].state = State::Binary(AccountedBox::new(
                        step,
                        &AllocationContext::new(self.account, MemoryKind::Workspace),
                    ));
                }
                Ok(None)
            }
            Expr::Call {
                function: Some(function),
                ..
            } if generic_matrix_call(
                self.expression,
                self.prepared,
                node,
                self.control,
                self.account,
            )?
            .is_some() =>
            {
                // Whole arguments were validated before this source nil-matrix callback.
                self.frames.pop();
                match function.name {
                    "rate" | "delta" | "increase" => Err(EvaluationError::WrappedRuntime(
                        crate::binary_labels::formatted(
                            format_args!(
                                "unexpected error: interface conversion: parser.Expr is *parser.ParenExpr, not *parser.MatrixSelector"
                            ),
                            self.control,
                            self.account,
                        )?,
                    )),
                    "irate" | "idelta" => {
                        self.prepared
                            .index_error(0, 0, self.control, self.account)?;
                        unreachable!("nil matrix index fails")
                    }
                    "absent_over_time" => Ok(Some(self.sample(1.0))),
                    _ => Ok(None),
                }
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
                "histogram_quantile" | "histogram_quantiles" | "histogram_fraction" => {
                    let function = crate::histogram_call::Function::parse(function.name)
                        .expect("histogram function");
                    let arguments = crate::histogram_call::Arguments::new(
                        function,
                        node,
                        self.expression,
                        self.prepared,
                        self.control,
                        self.account,
                    )?;
                    self.frames[index].state = State::Histogram(AccountedBox::new(
                        crate::histogram_call::Step::new(
                            self.rosters
                                .histogram(node)
                                .ok_or(EvaluationError::Invalid("histogram plan is missing"))?,
                            arguments,
                            self.histogram_preparation
                                .is_some_and(|value| value.node == node),
                            match self.frames[index].filter {
                                RowFilter::All => None,
                                RowFilter::Ordinals(values) => Some(values),
                                RowFilter::One(_) => None,
                            },
                            self.account,
                        ),
                        &AllocationContext::new(self.account, MemoryKind::Workspace),
                    ));
                    Ok(None)
                }
                "sort" | "sort_desc" | "sort_by_label" | "sort_by_label_desc" => {
                    self.frames[index].state = State::Sort(AccountedBox::new(
                        crate::vector_sort::Step::new(
                            self.rosters
                                .sorting(node)
                                .ok_or(EvaluationError::Invalid("sort plan is missing"))?,
                            self.options,
                            self.step_index,
                            !self.frames[index].bypass_order,
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
                    self.push_all(arguments[0])?;
                    Ok(None)
                }
                "vector" => {
                    self.frames[index].state = State::Pass;
                    self.push(arguments[0])?;
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
                    self.push_matrix(arguments[0])?;
                    Ok(None)
                }
                "scalar" => {
                    self.frames[index].state = State::Scalar {
                        count: 0,
                        value: None,
                        collecting: true,
                    };
                    self.push_all(arguments[0])?;
                    Ok(None)
                }
                "absent" | "absent_over_time" => {
                    let over_time = function.name == "absent_over_time";
                    let argument = self.prepared.preprocessing.child(
                        self.expression,
                        node,
                        arguments[0],
                        self.control,
                        self.account,
                    )?;
                    self.frames[index].state = State::Absent {
                        present: false,
                        labels: crate::absence::labels(
                            self.expression,
                            argument,
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
                            options: self.options,
                            step_index: self.step_index,
                            boundary_time_ms: None,
                            float_samples: [None, None],
                        });
                    } else {
                        self.push_all(arguments[0])?;
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
                                self.control,
                                self.account,
                            )?;
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
                                self.push_all(arguments[1])?;
                                return Ok(None);
                            }
                        }
                        self.frames[index].state = self.map(MapOperation::Pointwise(function));
                        self.push(arguments[0])?;
                        return Ok(None);
                    }
                    let function = RangeFunction::parse(function.name)
                        .ok_or(EvaluationError::Unsupported("function reducer"))?;
                    if let RangeFunction::Numeric(kind) = function
                        && kind.parameters() > 0
                    {
                        if let Some(selector) = range_selector(
                            self.expression,
                            self.prepared,
                            node,
                            self.control,
                            self.account,
                        )? {
                            self.frames[index].state = State::RawParameters {
                                function,
                                selector,
                                ordinal: 0,
                                initial: None,
                                parameters: [0.0; 2],
                                position: 0,
                            };
                            return Ok(None);
                        }
                        self.frames[index].state = State::Parameters {
                            function: ParameterizedFunction::Range(function),
                            parameters: [0.0, 0.0],
                            position: 0,
                        };
                        let parameter = scalar_arguments(self.expression, node)
                            .next()
                            .expect("typed range parameter");
                        self.push_parameter(parameter, true)?;
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
            self.options = self.frames[index].options;
            self.step_index = self.frames[index].step_index;
            let filter = self.frames[index].filter;
            let mode = self.frames[index].mode;
            match &mut self.frames[index].state {
                State::Gather(step) => {
                    if step.receive(sample) {
                        self.frames.truncate(index + 1);
                    }
                    return Ok(None);
                }
                State::Range(step) => {
                    if step.receive(sample, self.control, self.account)? {
                        self.frames.truncate(index + 1);
                    }
                    return Ok(None);
                }
                State::Binary(step) => {
                    step.receive(sample, self.control, self.account)?;
                    return Ok(None);
                }
                State::Histogram(step) => {
                    step.receive(sample, self.control, self.account)?;
                    return Ok(None);
                }
                State::Sort(step) => {
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
                State::TopK(step) => {
                    step.receive(sample, self.control, self.account)?;
                    return Ok(None);
                }
                State::Quantile(step) => {
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
                    ..
                } => {
                    let descriptor = &descriptors[*position];
                    if !filter.accepts(descriptor.ordinal, self.control, self.account)?
                        || matches!(mode, OutputMode::Vector)
                            && !descriptor
                                .heads
                                .accepts(sample.value.kind_bit(), self.options.time_ms)
                        || label_hash::full(&sample.labels, self.control, self.account)?
                            != descriptor.hash
                    {
                        return Ok(None);
                    }
                    sample.ordinal = descriptor.ordinal;
                    sample.drop_name = descriptor.drop_name;
                    if !crate::binary_labels::same(
                        &sample.labels,
                        &descriptor.labels,
                        self.control,
                        self.account,
                    )? {
                        sample.labels = descriptor.labels.copy_checked(
                            self.account,
                            MemoryKind::Workspace,
                            self.control,
                        )?;
                    }
                    if matches!(mode, OutputMode::Vector) {
                        *position += 1;
                        *running = false;
                        self.frames.truncate(index + 1);
                    }
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
                                    (OwnedValue::Float(crate::numerical::source_neg(value)), true)
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
                    sample.reconcile_samples(self.account);
                    sample.drop_name |= drop_name;
                    if drop_name && !self.options.delayed_name_removal {
                        let name_only = matches!(operation,
                            MapOperation::Pointwise(function) if !function.drops_metadata());
                        drop_reserved(&mut sample.labels, name_only, self.account, self.control)?;
                    }
                    sample.source_time_ms = self.options.time_ms;
                    sample.source_start_time_ms = 0;
                    let duplicate_check = match operation {
                        MapOperation::Unary(TokenKind::SUB) => {
                            !self.options.delayed_name_removal
                                && self.rosters.checks_merge(
                                    node,
                                    label_hash::full(&sample.labels, self.control, self.account)?,
                                    self.control,
                                    self.account,
                                )?
                        }
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
                    lookup,
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
                    let views = sample.labels.views_checked(self.account, self.control)?;
                    let labels = copy_group_key(
                        &sample.labels,
                        grouping.as_deref(),
                        *without,
                        self.control,
                        self.account,
                    )?;
                    let hash = if !without && grouping.as_ref().is_none_or(|names| names.is_empty())
                    {
                        0
                    } else {
                        label_hash::grouping(&labels, self.control, self.account)?
                    };
                    let position = if let Some(lookup) = lookup {
                        lookup.find(
                            hash,
                            |position| Ok(groups[position].hash == hash),
                            self.control,
                            self.account,
                        )?
                    } else {
                        crate::evaluator_roster::group_position(
                            groups,
                            hash,
                            |group| group.hash,
                            self.control,
                            self.account,
                        )?
                    };
                    if let Some(position) = position {
                        let group = &mut groups[position];
                        if let Some(kind) = group.add(
                            *operator,
                            &sample.value,
                            sample.drop_name,
                            self.control,
                            self.account,
                        )? && node == self.diagnostic_node
                            && let Some(annotations) = &mut self.annotations
                        {
                            annotations.add(
                                kind,
                                label(&views, b"__name__"),
                                self.expression.arena[self.prepared.preprocessing.child(
                                    self.expression,
                                    node,
                                    *child,
                                    self.control,
                                    self.account,
                                )?]
                                .span,
                                &self.expression.source,
                                self.control,
                                self.account,
                            )?;
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
                        self.frames[index].float_samples[0] = if *count == 1 {
                            sample.float_samples
                        } else {
                            None
                        };
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
                    let slot = usize::from(*phase != 0);
                    self.frames[index].float_samples[slot] = sample.float_samples;
                    return Ok(None);
                }
                State::Parameters {
                    parameters,
                    position,
                    ..
                }
                | State::RawParameters {
                    parameters,
                    position,
                    ..
                } => {
                    let OwnedValue::Float(value) = sample.value else {
                        unreachable!("scalar parameter")
                    };
                    parameters[*position] = value;
                    let slot = *position;
                    self.frames[index].float_samples[slot] = sample.float_samples;
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
                    self.frames[index].float_samples[0] = sample.float_samples;
                    return Ok(None);
                }
                _ => unreachable!("active parent consumes its child"),
            }
            sample = Self::rebind(sample, self.frames[index].boundary_time_ms);
        }
        Ok(Some(sample))
    }

    pub(crate) fn next(&mut self) -> Result<Option<EvaluatedSample>, EvaluationError> {
        let _inner_phase = self.account.phase(crate::statistics::Phase::Inner);
        loop {
            self.control.check()?;
            let Some(index) = self.frames.len().checked_sub(1) else {
                return Ok(None);
            };
            self.account.work(1);
            self.options = self.frames[index].options;
            self.step_index = self.frames[index].step_index;
            let boundary_time_ms = self.frames[index].boundary_time_ms;
            if matches!(self.frames[index].state, State::Enter) {
                if let Some(sample) = self.enter(index)?
                    && let Some(sample) =
                        self.propagate(Self::rebind(sample, boundary_time_ms), self.frames.len())?
                {
                    return Ok(Some(sample));
                }
                continue;
            }
            let node = self.frames[index].node;
            let filter = self.frames[index].filter;
            let mode = self.frames[index].mode;
            let bypass_order = self.frames[index].bypass_order;
            match &mut self.frames[index].state {
                State::Gather(step) => match step.advance(self.control, self.account)? {
                    crate::matrix_heads::Action::Child {
                        ordinal,
                        options,
                        step_index,
                    } => {
                        self.frames.push(Frame::new(
                            node,
                            RowFilter::One(ordinal),
                            false,
                            OutputMode::Matrix,
                            options,
                            self.prepared.flags().boundaries[node.0],
                            Some(step_index),
                        ));
                    }
                    crate::matrix_heads::Action::Sample(sample) => {
                        if let Some(sample) = self.propagate(sample, index)? {
                            return Ok(Some(sample));
                        }
                    }
                    crate::matrix_heads::Action::Done => {
                        self.frames.pop();
                    }
                },
                State::Range(step) => match step.advance(self.control, self.account)? {
                    crate::derived_range::Action::Backing { request, source } => {
                        let State::Range(mut step) =
                            std::mem::replace(&mut self.frames[index].state, State::Enter)
                        else {
                            unreachable!("range backing continuation")
                        };
                        let result = crate::temporary_float_bits::quantile(
                            self.catalog.query(),
                            request.count,
                            request.plan,
                            self.control,
                            self.account,
                            |writer| self.materialize_range_bits(source, writer),
                        );
                        if let Ok(value) = &result {
                            step.receive_backed(*value, self.account);
                        }
                        self.frames[index].state = State::Range(step);
                        result?;
                    }
                    crate::derived_range::Action::ScheduledPass {
                        node: body,
                        ordinal,
                        options,
                        grid,
                    } => {
                        let State::Range(mut step) =
                            std::mem::replace(&mut self.frames[index].state, State::Enter)
                        else {
                            unreachable!("range continuation")
                        };
                        let child = self.replay(body, options, None);
                        let result = step.scheduled_pass(self.control, self.account, |visit| {
                            child.visit_scheduled(body, ordinal, grid, visit)
                        });
                        self.frames[index].state = State::Range(step);
                        result?;
                    }
                    crate::derived_range::Action::Child {
                        node,
                        ordinal,
                        options,
                    } => {
                        self.frames.push(Frame::new(
                            node,
                            RowFilter::One(ordinal),
                            false,
                            OutputMode::Matrix,
                            options,
                            self.prepared.flags().boundaries[node.0],
                            None,
                        ));
                    }
                    crate::derived_range::Action::Series {
                        sample,
                        invalid_quantile,
                        annotations: notes,
                        metric,
                        nonempty,
                        sample_units,
                    } => {
                        self.credit_window(node, sample_units);
                        if node == self.diagnostic_node
                            && let Some(annotations) = &mut self.annotations
                        {
                            let span = if let Expr::Call {
                                function: Some(function),
                                arguments,
                            } = &self.expression.arena[node].expression
                                && function.name == "quantile_over_time"
                            {
                                self.expression.arena[arguments[0]].span
                            } else {
                                crate::frontend::Span::default()
                            };
                            if let Some(q) = invalid_quantile {
                                annotations.add_invalid_quantile(
                                    q,
                                    span,
                                    &self.expression.source,
                                    self.control,
                                    self.account,
                                )?;
                            }
                            for kind in notes.into_iter().flatten() {
                                annotations.add(
                                    kind,
                                    metric,
                                    span,
                                    &self.expression.source,
                                    self.control,
                                    self.account,
                                )?;
                            }
                        }
                        if nonempty {
                            self.prepared.sample_statistics_index(
                                node,
                                self.step_index,
                                self.control,
                                self.account,
                            )?;
                        }
                        if let Some(sample) = sample
                            && filter.accepts(sample.ordinal, self.control, self.account)?
                            && let Some(sample) =
                                self.propagate(Self::rebind(sample, boundary_time_ms), index)?
                        {
                            return Ok(Some(sample));
                        }
                    }
                    crate::derived_range::Action::Done => {
                        self.frames.pop();
                    }
                },
                State::Quantile(step) => match step.advance(self.control, self.account)? {
                    crate::quantile_aggregate::Action::Continue => (),
                    crate::quantile_aggregate::Action::Child { parameter, ordinal } => {
                        let Expr::Aggregate {
                            expression: Some(child),
                            parameter: Some(q),
                            ..
                        } = self.expression.arena[node].expression
                        else {
                            unreachable!("typed quantile operands")
                        };
                        if parameter {
                            if let Some(options) = self.aggregate_parameter_options(q)? {
                                self.push_scoped(
                                    q,
                                    RowFilter::All,
                                    false,
                                    OutputMode::Matrix,
                                    options,
                                )?;
                            } else {
                                let zero = self.sample(0.0);
                                let State::Quantile(step) = &mut self.frames[index].state else {
                                    unreachable!()
                                };
                                step.receive(zero, self.control, self.account)?;
                            }
                        } else {
                            let filter = ordinal.map_or(RowFilter::All, |ordinal| {
                                RowFilter::Ordinals(std::slice::from_ref(ordinal))
                            });
                            self.push_filtered(child, filter, false, OutputMode::Vector)?;
                        }
                    }
                    crate::quantile_aggregate::Action::Sample(sample) => {
                        if filter.accepts(sample.ordinal, self.control, self.account)?
                            && let Some(sample) =
                                self.propagate(Self::rebind(sample, boundary_time_ms), index)?
                        {
                            return Ok(Some(sample));
                        }
                    }
                    crate::quantile_aggregate::Action::Done(annotation) => {
                        if node == self.diagnostic_node
                            && let Some(kind) = annotation
                            && let Some(annotations) = &mut self.annotations
                        {
                            let Expr::Aggregate {
                                expression: Some(child),
                                ..
                            } = self.expression.arena[node].expression
                            else {
                                unreachable!("typed quantile body")
                            };
                            let child = self.prepared.preprocessing.child(
                                self.expression,
                                node,
                                child,
                                self.control,
                                self.account,
                            )?;
                            annotations.add(
                                kind,
                                b"",
                                self.expression.arena[child].span,
                                &self.expression.source,
                                self.control,
                                self.account,
                            )?;
                        }
                        self.frames.pop();
                    }
                },
                State::TopK(step) => match step.advance(self.control, self.account)? {
                    crate::topk::Action::Continue => (),
                    crate::topk::Action::Child { parameter, ordinal } => {
                        let Expr::Aggregate {
                            expression: Some(child),
                            parameter: Some(k),
                            ..
                        } = self.expression.arena[node].expression
                        else {
                            unreachable!()
                        };
                        if parameter {
                            if let Some(options) = self.aggregate_parameter_options(k)? {
                                self.push_scoped(
                                    k,
                                    RowFilter::All,
                                    false,
                                    OutputMode::Matrix,
                                    options,
                                )?;
                            } else {
                                let zero = self.sample(0.0);
                                let State::TopK(step) = &mut self.frames[index].state else {
                                    unreachable!()
                                };
                                step.receive(zero, self.control, self.account)?;
                            }
                        } else {
                            let filter = ordinal.map_or(RowFilter::All, |ordinal| {
                                RowFilter::Ordinals(std::slice::from_ref(ordinal))
                            });
                            self.push_filtered(child, filter, false, OutputMode::Vector)?;
                        }
                    }
                    crate::topk::Action::Sample(sample) => {
                        if filter.accepts(sample.ordinal, self.control, self.account)?
                            && let Some(sample) =
                                self.propagate(Self::rebind(sample, boundary_time_ms), index)?
                        {
                            return Ok(Some(sample));
                        }
                    }
                    crate::topk::Action::Done(annotation) => {
                        if node == self.diagnostic_node
                            && let Some(kind) = annotation
                            && let Some(annotations) = &mut self.annotations
                        {
                            annotations.add(
                                kind,
                                b"",
                                self.expression.arena[node].span,
                                &self.expression.source,
                                self.control,
                                self.account,
                            )?;
                        }
                        self.frames.pop();
                    }
                },
                State::CountValues { step, collecting } => {
                    if *collecting {
                        let options = self.options;
                        step.finish(
                            options,
                            self.count_preparation.filter(|value| value.node == node),
                            self.control,
                            self.account,
                        )?;
                        *collecting = false;
                    }
                    if let Some(sample) = step.next(self.options.time_ms, self.account) {
                        if filter.accepts(sample.ordinal, self.control, self.account)?
                            && let Some(sample) =
                                self.propagate(Self::rebind(sample, boundary_time_ms), index)?
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
                            self.push_filtered(arguments[0], filter, false, OutputMode::Vector)?;
                        }
                        crate::info::Action::Sample(sample) => {
                            if filter.accepts(sample.ordinal, self.control, self.account)?
                                && let Some(sample) =
                                    self.propagate(Self::rebind(sample, boundary_time_ms), index)?
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
                    let options = self.options;
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
                            self.push_filtered(child, filter, false, OutputMode::Vector)?;
                        }
                        crate::histogram_call::Action::Sample(sample) => {
                            if filter.accepts(sample.ordinal, self.control, self.account)?
                                && let Some(sample) =
                                    self.propagate(Self::rebind(sample, boundary_time_ms), index)?
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
                State::Sort(step) => {
                    let target = match filter {
                        RowFilter::Ordinals([target])
                            if matches!(mode, OutputMode::Matrix) && bypass_order =>
                        {
                            Some(if node == self.expression.root {
                                crate::vector_sort::Target::Complete(*target)
                            } else {
                                crate::vector_sort::Target::Prefix(*target)
                            })
                        }
                        _ => None,
                    };
                    let action = step.advance(self.expression, target, self.control, self.account);
                    if step.take_range_warning()
                        && node == self.diagnostic_node
                        && let Some(annotations) = &mut self.annotations
                    {
                        annotations.add(
                            AnnotationKind::SortInRangeQuery,
                            b"",
                            self.expression.arena[node].span,
                            &self.expression.source,
                            self.control,
                            self.account,
                        )?;
                    }
                    match action? {
                        crate::vector_sort::Action::Continue => (),
                        crate::vector_sort::Action::Child(ordinal) => {
                            let Expr::Call { arguments, .. } =
                                &self.expression.arena[node].expression
                            else {
                                unreachable!()
                            };
                            let filter = ordinal.map_or(RowFilter::All, |value| {
                                RowFilter::Ordinals(std::slice::from_ref(value))
                            });
                            self.push_filtered(arguments[0], filter, false, OutputMode::Vector)?;
                        }
                        crate::vector_sort::Action::Sample(sample) => {
                            let accepted =
                                filter.accepts(sample.ordinal, self.control, self.account)?;
                            if matches!(target, Some(crate::vector_sort::Target::Prefix(_))) {
                                step.complete_prefix(self.control, self.account)?;
                            }
                            if accepted
                                && let Some(sample) =
                                    self.propagate(Self::rebind(sample, boundary_time_ms), index)?
                            {
                                return Ok(Some(sample));
                            }
                        }
                        crate::vector_sort::Action::Done => {
                            self.frames.pop();
                        }
                    }
                }
                State::Binary(step) => {
                    let options = self.options;
                    let progress = if step.has_set_discovery() {
                        set_discovery_target(
                            mode,
                            bypass_order,
                            filter,
                            node,
                            self.binary_preparation,
                        )
                        .map(|target| {
                            (
                                crate::vector_matching::ReplayKey::new(
                                    options,
                                    self.step_index,
                                    self.prepared.lookback_ns(options.lookback_ms),
                                    self.data_bounds,
                                ),
                                target,
                            )
                        })
                    } else {
                        None
                    };
                    match step.advance(
                        node,
                        options,
                        self.binary_preparation,
                        progress.as_ref().map(|(key, target)| {
                            crate::vector_matching::SetProgress {
                                key,
                                target: *target,
                            }
                        }),
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
                            self.push_filtered(child, filter, false, OutputMode::Vector)?;
                        }
                        crate::vector_matching::Action::Sample(sample) => {
                            if filter.accepts(sample.ordinal, self.control, self.account)?
                                && let Some(sample) =
                                    self.propagate(Self::rebind(sample, boundary_time_ms), index)?
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
                                    self.control,
                                    self.account,
                                )?;
                            }
                            step.complete_replay(self.control)?;
                            self.frames.pop();
                        }
                    }
                }
                State::OrderedRows {
                    descriptors,
                    sources,
                    position,
                    running,
                } => {
                    if *running {
                        *position += 1;
                        *running = false;
                    }
                    let Some(next) =
                        filter.next_position(descriptors, *position, self.control, self.account)?
                    else {
                        self.frames.pop();
                        continue;
                    };
                    if next != *position {
                        *position = next;
                    }
                    let descriptor = &descriptors[*position];
                    let filter = RowFilter::Ordinals(&sources[descriptor.sources.clone()]);
                    *running = true;
                    self.push_filtered(node, filter, true, OutputMode::Matrix)?;
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
                            self.control,
                            self.account,
                        )?;
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
                    let sample = if let Some((scheduled_selector, scheduled_ordinal)) =
                        self.scheduled_selector
                    {
                        if selector_node != scheduled_selector
                            || series_ordinal != scheduled_ordinal
                            || !matches!(operation, SelectorOperation::Instant)
                        {
                            return Err(EvaluationError::Invalid(
                                "scheduled selector identity changed",
                            ));
                        }
                        let point = self.selected_point.take().ok_or(EvaluationError::Invalid(
                            "scheduled point consumed more than once",
                        ))?;
                        if point.value.is_some() {
                            self.note_instant_sample(node, selector_node)?;
                        }
                        let labels = point.labels.views_checked(self.account, self.control)?;
                        self.finish_selector_sample(
                            selector_node,
                            operation,
                            series_ordinal,
                            &labels,
                            point.value,
                        )?
                    } else {
                        catalog
                            .with_series(selector_node, series_ordinal, |series| {
                                self.series_sample(
                                    node,
                                    selector_node,
                                    operation,
                                    series_ordinal,
                                    series,
                                    None,
                                )
                            })
                            .expect("catalog ordinal")?
                    };
                    if let Some(sample) = sample
                        && let Some(sample) =
                            self.propagate(Self::rebind(sample, boundary_time_ms), index)?
                    {
                        return Ok(Some(sample));
                    }
                }
                State::Aggregate {
                    groups,
                    lookup,
                    collecting,
                } => {
                    if *collecting {
                        *collecting = false;
                        // Positions cease to identify the same groups after reversal and removal.
                        *lookup = None;
                        let length = groups.len();
                        for position in 0..length / 2 {
                            self.control.check()?;
                            self.account.work(1);
                            groups.swap(position, length - 1 - position);
                        }
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
                                    self.control,
                                    self.account,
                                )?]
                                .span,
                                &self.expression.source,
                                self.control,
                                self.account,
                            )?;
                        }
                        if let Some(sample) = sample
                            && let Some(sample) =
                                self.propagate(Self::rebind(sample, boundary_time_ms), index)?
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
                    let samples = self.frames[index].float_samples[0]
                        .take()
                        .unwrap_or_else(|| crate::statistics::OwnedSamples::new(self.account, 1));
                    self.frames.pop();
                    let sample = self.sample_owned(value, Some(samples));
                    if let Some(sample) =
                        self.propagate(Self::rebind(sample, boundary_time_ms), self.frames.len())?
                    {
                        return Ok(Some(sample));
                    }
                }
                State::Absent {
                    present,
                    labels,
                    drop_name,
                } => {
                    if !*present {
                        crate::label_encoding::finalized(labels, self.control, self.account)?;
                    }
                    let sample = (!*present).then(|| EvaluatedSample {
                        ordinal: 0,
                        labels: std::mem::replace(
                            labels,
                            OwnedLabels::new(self.account, MemoryKind::Workspace),
                        ),
                        float_samples: Some(crate::statistics::OwnedSamples::new(self.account, 1)),
                        value: OwnedValue::Float(1.0),
                        source_time_ms: self.options.time_ms,
                        source_start_time_ms: 0,
                        drop_name: *drop_name,
                    });
                    self.frames.pop();
                    if let Some(sample) = sample
                        && let Some(sample) = self
                            .propagate(Self::rebind(sample, boundary_time_ms), self.frames.len())?
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
                        self.push_all(right_node)?;
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
                        if let Some(sample) = self
                            .propagate(Self::rebind(sample, boundary_time_ms), self.frames.len())?
                        {
                            return Ok(Some(sample));
                        }
                    }
                }
                State::RawParameters { .. } => {
                    let State::RawParameters {
                        function,
                        selector,
                        mut ordinal,
                        mut initial,
                        mut parameters,
                        mut position,
                    } = std::mem::replace(&mut self.frames[index].state, State::Enter)
                    else {
                        unreachable!("raw parameter continuation")
                    };
                    if initial.is_none() {
                        let Some(selected) =
                            filter.next_ordinal(ordinal, self.control, self.account)?
                        else {
                            self.frames.pop();
                            continue;
                        };
                        if selected >= self.catalog.len(selector) {
                            self.frames.pop();
                            continue;
                        }
                        ordinal = selected;
                        initial = self
                            .catalog
                            .with_series(selector, ordinal, |series| {
                                self.initialize_range_series(selector, series)
                            })
                            .expect("raw initialization ordinal")?;
                        if initial.is_none() {
                            self.frames[index].state = State::RawParameters {
                                function,
                                selector,
                                ordinal: ordinal + 1,
                                initial,
                                parameters,
                                position,
                            };
                            continue;
                        }
                        parameters = [0.0; 2];
                        self.frames[index].float_samples = [None, None];
                        position = 0;
                        self.frames[index].state = State::RawParameters {
                            function,
                            selector,
                            ordinal,
                            initial,
                            parameters,
                            position,
                        };
                        let child = scalar_arguments(self.expression, node)
                            .next()
                            .expect("typed raw range parameter");
                        self.push_parameter(child, true)?;
                    } else if let Some(child) =
                        scalar_arguments(self.expression, node).nth(position + 1)
                    {
                        position += 1;
                        self.frames[index].state = State::RawParameters {
                            function,
                            selector,
                            ordinal,
                            initial,
                            parameters,
                            position,
                        };
                        self.push_parameter(child, true)?;
                    } else {
                        let (_, operation) = self.range_spec(node, function, parameters)?;
                        let catalog = self.catalog;
                        let sample = catalog
                            .with_series(selector, ordinal, |series| {
                                self.series_sample(
                                    node,
                                    selector,
                                    operation,
                                    ordinal,
                                    series,
                                    initial.as_ref(),
                                )
                            })
                            .expect("raw range ordinal");
                        drop(initial);
                        self.frames[index].state = State::RawParameters {
                            function,
                            selector,
                            ordinal: ordinal + 1,
                            initial: None,
                            parameters,
                            position,
                        };
                        if let Some(sample) = sample?
                            && let Some(sample) =
                                self.propagate(Self::rebind(sample, boundary_time_ms), index)?
                        {
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
                    if let Some(child) = scalar_arguments(self.expression, node).nth(*position + 1)
                    {
                        *position += 1;
                        let range = matches!(function, ParameterizedFunction::Range(_));
                        self.push_parameter(child, range)?;
                    } else {
                        let (function, parameters) = (*function, *parameters);
                        match function {
                            ParameterizedFunction::Pointwise(function) => {
                                let operation = MapOperation::Pointwise(PointwiseFunction::Float(
                                    function, parameters,
                                ));
                                self.frames[index].state = self.map(operation);
                                self.push(arguments[0])?;
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
                    self.push(child)?;
                }
                State::Enter => unreachable!(),
            }
        }
    }
}

pub(crate) fn remember_unique_kind(
    seen: &mut AccountedVec<(u64, u8)>,
    labels: &OwnedLabels,
    kind: u8,
    account: &QueryAccounting,
    control: &Control,
) -> Result<(), EvaluationError> {
    let identity = (label_hash::full(labels, control, account)?, kind);
    if crate::evaluator_order::contains(seen, &identity, control, account)? {
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

pub(crate) fn drop_reserved(
    labels: &mut OwnedLabels,
    name_only: bool,
    account: &QueryAccounting,
    control: &Control,
) -> Result<(), EvaluationError> {
    check_reserved_name(labels.0.first().map(|label| &*label.name), account, control)?;
    labels.retain_labels_checked(account, control, |label| {
        Ok::<_, Interrupted>(if name_only {
            &*label.name != b"__name__"
        } else {
            !metadata(&label.name)
        })
    })?;
    Ok(())
}

pub(crate) fn check_reserved_name(
    first: Option<&[u8]>,
    account: &QueryAccounting,
    control: &Control,
) -> Result<(), EvaluationError> {
    control.check()?;
    account.work(1);
    // Pinned DropReserved indexes the first sorted name before calling its predicate.
    if first.is_some_and(|name| name.is_empty()) {
        return Err(EvaluationError::WrappedRuntime(
            crate::binary_labels::formatted(
                format_args!(
                    "unexpected error: runtime error: index out of range [0] with length 0"
                ),
                control,
                account,
            )?,
        ));
    }
    Ok(())
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
        ADD => crate::numerical::source_add(left, right),
        SUB => crate::numerical::source_sub(left, right),
        MUL => crate::numerical::source_mul(left, right),
        DIV => crate::numerical::source_div(left, right),
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
        TRIM_LOWER | TRIM_UPPER => {
            return Err(EvaluationError::Runtime(crate::binary_labels::formatted(
                format_args!(
                    "operator {:?} not allowed for Scalar operations",
                    operator.text().expect("typed trim operator")
                ),
                control,
                account,
            )?));
        }
        _ => return Err(EvaluationError::Unsupported("binary numerical kernel")),
    })
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    use crate::accounting::EngineAccounting;

    #[test]
    fn float_map_parameters_interrupt_before_vector_acquisition() {
        use crate::frontend::{ast::ParserOptions, parser, regex::Selector};
        use crate::storage::Provider;
        use std::sync::atomic::Ordering;

        let mut parameter = "time()".to_owned();
        for _ in 0..6 {
            parameter = format!("({parameter}+{parameter})");
        }
        for page in [1, 17, 1024] {
            for position in [0, 1, 2, 3] {
                let source = match position {
                    0 => format!("clamp(counter_total, {parameter}/16, 100)"),
                    1 => format!("clamp(counter_total, 0, {parameter}/16)"),
                    2 => format!("counter_total + {parameter}/16"),
                    _ => format!("{parameter}/16 - counter_total"),
                };
                for interrupted in [true, false] {
                    let parsed =
                        parser::parse(source.as_bytes(), ParserOptions::default(), |pattern| {
                            Selector::compile_in(pattern, None)
                        });
                    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
                    let expression = ParsedExpression {
                        source: source.as_bytes().to_vec().into(),
                        arena: parsed.arena,
                        root: parsed.root.unwrap(),
                    };
                    let account = EngineAccounting::default().query();
                    let control = Control::default();
                    let preprocessing = crate::evaluator_order::preprocess(
                        &expression,
                        MemoryKind::Workspace,
                        &control,
                        &account,
                    )
                    .unwrap();
                    let prepared = Prepared::new(
                        &expression,
                        &preprocessing,
                        crate::result::OutputHeader {
                            value_type: ValueType::Vector,
                            start_ms: 2000,
                            end_ms: 2000,
                            step_ms: 0,
                        },
                        60_000,
                        &control,
                        &account,
                    )
                    .unwrap();
                    let header = prepared.evaluator_header();
                    let options = EvaluationOptions {
                        time_ms: 2000,
                        start_ms: header.start_ms,
                        end_ms: header.end_ms,
                        step_ms: header.step_ms,
                        lookback_ms: 2000,
                        start_timestamps: false,
                        delayed_name_removal: false,
                    };
                    let bounds = crate::query_bounds::find(
                        &expression,
                        options,
                        &prepared,
                        &control,
                        &account,
                    )
                    .unwrap();
                    let provider = crate::test_storage::FixtureProvider::new(3, page, 1);
                    let scope = provider.open(bounds, &control).unwrap();
                    let catalog = Catalog::new(
                        &scope,
                        &expression,
                        &prepared,
                        options,
                        60_000,
                        &control,
                        &account,
                    )
                    .unwrap();
                    for (index, node) in expression.arena.nodes.iter().enumerate() {
                        if matches!(node.expression, Expr::Selector(_)) {
                            drop(
                                catalog
                                    .expand(NodeId(index), &control, &account)
                                    .map_err(|failure| failure.error)
                                    .unwrap(),
                            );
                        }
                    }
                    let rosters =
                        Rosters::new(&account, expression.arena.nodes.len(), &control).unwrap();
                    let timestamps = vec![false; expression.arena.nodes.len()];
                    let counters = &provider.series[0].source.counters;
                    let before = counters.opened.load(Ordering::Relaxed);
                    let mut machine = Machine::new(
                        &catalog,
                        &expression,
                        expression.root,
                        options,
                        &control,
                        &account,
                        EvaluationMetadata {
                            prepared: &prepared,
                            rosters: &rosters,
                            timestamp_selectors: &timestamps,
                            data_bounds: bounds,
                            annotations: None,
                            binary_preparation: None,
                            histogram_preparation: None,
                            info_preparation: None,
                            count_preparation: None,
                        },
                    );
                    if interrupted {
                        let result = crate::test_storage::interrupt_after_work(
                            &account,
                            &control,
                            32,
                            || machine.next(),
                        );
                        assert!(matches!(
                            result,
                            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                        ));
                        let active =
                            machine
                                .frames
                                .iter()
                                .position(|frame| {
                                    frame.node == expression.root
                                        && match &frame.state {
                                            State::Parameters {
                                                function:
                                                    ParameterizedFunction::Pointwise(
                                                        FloatFunction::Clamp,
                                                    ),
                                                position: active,
                                                ..
                                            } => position < 2 && *active == position,
                                            State::MixedBinary {
                                                scalar,
                                                scalar_left,
                                                ..
                                            } => {
                                                position >= 2
                                                    && scalar.is_none()
                                                    && *scalar_left == (position == 3)
                                            }
                                            _ => false,
                                        }
                                })
                                .expect(
                                    "interruption must occur inside the requested scalar operand",
                                );
                        assert!(machine.frames[active + 1..].iter().any(|frame| {
                            expression.arena[frame.node].value_type == ValueType::Scalar
                                && !matches!(frame.state, State::Enter)
                        }));
                        assert_eq!(counters.opened.load(Ordering::Relaxed), before);
                    } else {
                        let sample = machine.next().unwrap().unwrap();
                        let OwnedValue::Float(value) = sample.value else {
                            panic!("mapped operand must yield a float");
                        };
                        assert_eq!(
                            value.to_bits(),
                            [8.0_f64, 2.0, 10.0, 6.0][position].to_bits()
                        );
                        assert!(counters.opened.load(Ordering::Relaxed) > before);
                        assert!(machine.next().unwrap().is_none());
                        drop(sample);
                    }
                    drop(machine);
                    drop(rosters);
                    drop(catalog);
                    drop(scope);
                    drop(prepared);
                    drop(preprocessing);
                    provider.assert_closed();
                    account.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn set_discovery_scope_shares_the_existing_activation_predicate() {
        let account = EngineAccounting::default().query();
        let cache = std::cell::RefCell::new(crate::vector_matching::Cache::new(&account));
        let owner = crate::vector_matching::Preparation {
            node: NodeId(1),
            cache: &cache,
        };
        let one = [7];
        let many = [3, 7];
        for mode in [OutputMode::Vector, OutputMode::Matrix] {
            for bypass_order in [false, true] {
                for (filter, target) in [
                    (RowFilter::One(7), Some(Some(7))),
                    (RowFilter::Ordinals(&one), Some(Some(7))),
                    (RowFilter::All, Some(None)),
                    (RowFilter::Ordinals(&[]), None),
                    (RowFilter::Ordinals(&many), None),
                ] {
                    for preparation in [None, Some(owner)] {
                        for node in [NodeId(1), NodeId(2)] {
                            let expected = if matches!(mode, OutputMode::Vector) {
                                None
                            } else if matches!(filter, RowFilter::All) {
                                (preparation.is_some() && node == owner.node).then_some(None)
                            } else if bypass_order {
                                target
                            } else {
                                None
                            };
                            assert_eq!(
                                set_discovery_target(mode, bypass_order, filter, node, preparation),
                                expected
                            );
                        }
                    }
                }
            }
        }
        drop(cache);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn scalar_trim_errors_keep_source_text_and_release_owned_messages() {
        for (operator, expected) in [
            (
                TokenKind::TRIM_LOWER,
                b"operator \">/\" not allowed for Scalar operations".as_slice(),
            ),
            (
                TokenKind::TRIM_UPPER,
                b"operator \"</\" not allowed for Scalar operations".as_slice(),
            ),
        ] {
            let account = EngineAccounting::default().query();
            let error = scalar_binary(operator, 1.0, 2.0, &Control::default(), &account)
                .expect_err("typed scalar trim is a source execution error");
            let EvaluationError::Runtime(message) = &error else {
                panic!("scalar trim must preserve the plain runtime error");
            };
            assert_eq!(&**message, expected);
            assert!(account.snapshot().memory[MemoryKind::Workspace as usize].current > 0);
            drop(error);
            account.snapshot().assert_memory_released();
        }
    }

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
                heads: crate::evaluator_roster::HeadSummary::default(),
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
