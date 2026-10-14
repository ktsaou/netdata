// SPDX-License-Identifier: GPL-3.0-or-later
// Ordinary range reducers derive from pinned promql/functions.go; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    annotations::{AnnotationKind, HistogramOperation},
    histogram::{FloatHistogramRef, HistogramError},
    histogram_accumulator::{Finalization, NativeAccumulator},
    histogram_counter::detect_reset_with,
    histogram_work::{HistogramEvaluationError, HistogramWork},
    matrix_reader::{MatrixWindow, Progress, scan as scan_matrix},
    numerical::{compensated_add, source_add, source_div, source_mul, source_sub, sqrt},
    range_float::{RateKind, start_timestamp_reset},
    range_numeric::NumericRangeFunction,
    range_rates::RateFunction,
    result::{OwnedHistogram, OwnedValue},
    storage::{Control, Interrupted, Series, StorageError, TimeBounds},
    value::{SampleRef, SampleValueRef},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RangeFunction {
    Rate(RateFunction),
    Fold(FoldKind),
    Numeric(NumericRangeFunction),
}
impl RangeFunction {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        use FoldKind::*;
        Some(match name {
            "rate" => Self::Rate(RateFunction::Extrapolated(RateKind::Rate)),
            "increase" => Self::Rate(RateFunction::Extrapolated(RateKind::Increase)),
            "delta" => Self::Rate(RateFunction::Extrapolated(RateKind::Delta)),
            "irate" => Self::Rate(RateFunction::Instant { counter: true }),
            "idelta" => Self::Rate(RateFunction::Instant { counter: false }),
            "deriv" => Self::Numeric(NumericRangeFunction::Deriv),
            "predict_linear" => Self::Numeric(NumericRangeFunction::PredictLinear),
            "double_exponential_smoothing" => Self::Numeric(NumericRangeFunction::Smoothing),
            "mad_over_time" => Self::Numeric(NumericRangeFunction::Mad),
            "quantile_over_time" => Self::Numeric(NumericRangeFunction::Quantile),
            "count_over_time" => Self::Fold(Count),
            "present_over_time" | "absent_over_time" => Self::Fold(Present),
            "first_over_time" => Self::Fold(First),
            "last_over_time" => Self::Fold(Last),
            "min_over_time" => Self::Fold(Min),
            "max_over_time" => Self::Fold(Max),
            "sum_over_time" => Self::Fold(Sum),
            "avg_over_time" => Self::Fold(Avg),
            "stdvar_over_time" => Self::Fold(Stdvar),
            "stddev_over_time" => Self::Fold(Stddev),
            "ts_of_first_over_time" => Self::Fold(TsFirst),
            "ts_of_last_over_time" => Self::Fold(TsLast),
            "ts_of_min_over_time" => Self::Fold(TsMin),
            "ts_of_max_over_time" => Self::Fold(TsMax),
            "changes" => Self::Fold(Changes),
            "resets" => Self::Fold(Resets),
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FoldKind {
    Count,
    Present,
    First,
    Last,
    Min,
    Max,
    Sum,
    Avg,
    Stdvar,
    Stddev,
    TsFirst,
    TsLast,
    TsMin,
    TsMax,
    Changes,
    Resets,
}
impl FoldKind {
    pub(crate) fn preserves_name(self) -> bool {
        matches!(self, Self::First | Self::Last)
    }
    fn extrema(self) -> bool {
        matches!(self, Self::Min | Self::Max | Self::TsMin | Self::TsMax)
    }
    fn previous(self) -> bool {
        matches!(self, Self::Changes | Self::Resets)
    }
    fn variance(self) -> bool {
        matches!(self, Self::Stdvar | Self::Stddev)
    }
}

pub(crate) struct FoldResult {
    pub(crate) float_samples: Option<crate::statistics::OwnedSamples>,
    pub(crate) value: Option<OwnedValue>,
    pub(crate) annotations: [Option<AnnotationKind>; 2],
    pub(crate) nonempty: bool,
    pub(crate) progress: Option<Progress>,
}

struct NativeFold {
    accumulator: NativeAccumulator,
    count: u64,
}
impl NativeFold {
    fn new(kind: FoldKind, sum: OwnedHistogram) -> Self {
        Self {
            accumulator: NativeAccumulator::new(kind == FoldKind::Avg, sum),
            count: 1,
        }
    }
    fn add(
        &mut self,
        value: FloatHistogramRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), HistogramEvaluationError> {
        self.count += 1;
        self.accumulator
            .add(value, self.count as f64, control, account)
            .map(|_| ())
    }
    fn finish(
        self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<FoldResult, HistogramEvaluationError> {
        let result =
            self.accumulator
                .finish(self.count as f64, Finalization::Range, control, account)?;
        Ok(FoldResult {
            float_samples: None,
            progress: None,
            value: Some(OwnedValue::Histogram(result.value)),
            nonempty: true,
            annotations: [
                result
                    .reset_collision
                    .then_some(AnnotationKind::HistogramCounterResetCollision(
                        HistogramOperation::Aggregation,
                    )),
                result
                    .reconciled
                    .then_some(AnnotationKind::MismatchedCustomBuckets(
                        HistogramOperation::Aggregation,
                    )),
            ],
        })
    }
}

fn retain_histogram(
    value: SampleValueRef<'_>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<OwnedHistogram, Interrupted> {
    let OwnedValue::Histogram(value) =
        OwnedValue::copy_from_checked(value, control, account, MemoryKind::Workspace)?
    else {
        unreachable!("classified histogram window")
    };
    Ok(value)
}

enum NumericalFold {
    Sum {
        sum: f64,
        compensation: f64,
    },
    Average {
        sum: f64,
        compensation: f64,
        mean: f64,
        count: u64,
        incremental: bool,
    },
    Variance {
        count: f64,
        mean: f64,
        mean_compensation: f64,
        auxiliary: f64,
        auxiliary_compensation: f64,
    },
}
impl NumericalFold {
    fn new(kind: FoldKind) -> Option<Self> {
        Some(match kind {
            FoldKind::Sum => Self::Sum {
                sum: 0.0,
                compensation: 0.0,
            },
            FoldKind::Avg => Self::Average {
                sum: 0.0,
                compensation: 0.0,
                mean: 0.0,
                count: 0,
                incremental: false,
            },
            FoldKind::Stdvar | FoldKind::Stddev => Self::Variance {
                count: 0.0,
                mean: 0.0,
                mean_compensation: 0.0,
                auxiliary: 0.0,
                auxiliary_compensation: 0.0,
            },
            _ => return None,
        })
    }
    fn add(&mut self, value: f64) {
        match self {
            Self::Sum { sum, compensation } => {
                (*sum, *compensation) = compensated_add(value, *sum, *compensation);
            }
            Self::Average {
                sum,
                compensation,
                mean,
                count,
                incremental,
            } => {
                *count += 1;
                if *count == 1 {
                    *sum = value;
                    return;
                }
                let count = *count as f64;
                if !*incremental {
                    let (new_sum, new_compensation) = compensated_add(value, *sum, *compensation);
                    if !new_sum.is_infinite() {
                        (*sum, *compensation) = (new_sum, new_compensation);
                        return;
                    }
                    // Discard the overflowing trial before switching algorithms.
                    *incremental = true;
                    *mean = source_div(*sum, count - 1.0);
                    *compensation = source_div(*compensation, count - 1.0);
                }
                let q = source_div(count - 1.0, count);
                (*mean, *compensation) = compensated_add(
                    source_div(value, count),
                    source_mul(*mean, q),
                    source_mul(*compensation, q),
                );
            }
            Self::Variance {
                count,
                mean,
                mean_compensation,
                auxiliary,
                auxiliary_compensation,
            } => {
                *count += 1.0;
                let delta = source_sub(value, source_add(*mean, *mean_compensation));
                (*mean, *mean_compensation) =
                    compensated_add(source_div(delta, *count), *mean, *mean_compensation);
                (*auxiliary, *auxiliary_compensation) = compensated_add(
                    source_mul(
                        delta,
                        source_sub(value, source_add(*mean, *mean_compensation)),
                    ),
                    *auxiliary,
                    *auxiliary_compensation,
                );
            }
        }
    }
    fn finish(self, kind: FoldKind) -> f64 {
        match self {
            Self::Sum { sum, compensation } => {
                if sum.is_infinite() {
                    sum
                } else {
                    source_add(sum, compensation)
                }
            }
            Self::Average {
                sum,
                compensation,
                mean,
                count,
                incremental,
            } => {
                if incremental {
                    source_add(mean, compensation)
                } else {
                    source_add(
                        source_div(sum, count as f64),
                        source_div(compensation, count as f64),
                    )
                }
            }
            Self::Variance {
                count,
                auxiliary,
                auxiliary_compensation,
                ..
            } => {
                let variance = source_div(source_add(auxiliary, auxiliary_compensation), count);
                if kind == FoldKind::Stddev {
                    sqrt(variance)
                } else {
                    variance
                }
            }
        }
    }
}
struct Previous {
    value: OwnedValue,
    time_ms: i64,
    start_time_ms: i64,
    _float_samples: Option<crate::statistics::OwnedSamples>,
}
impl Previous {
    fn copy(
        sample: SampleRef<'_>,
        use_start: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, Interrupted> {
        let value = crate::statistics::ValueParts::new(
            OwnedValue::from_sample_checked(sample.value, control, account, MemoryKind::Workspace)?,
            account,
        );
        Ok(Self {
            _float_samples: value.float_samples,
            value: value.value,
            time_ms: sample.time_ms,
            start_time_ms: if use_start { sample.start_time_ms } else { 0 },
        })
    }
}

pub(crate) struct FoldOptions {
    pub(crate) start_timestamps: bool,
    pub(crate) matrix: Option<MatrixWindow>,
    pub(crate) anchor_start_ms: Option<i64>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum FoldError {
    Interrupted(Interrupted),
    Storage(StorageError),
    NonAdvancingMerge { kind: FoldKind, time_ms: i64 },
}
impl From<Interrupted> for FoldError {
    fn from(value: Interrupted) -> Self {
        Self::Interrupted(value)
    }
}
impl From<HistogramEvaluationError> for FoldError {
    fn from(error: HistogramEvaluationError) -> Self {
        match error {
            HistogramEvaluationError::Interrupted(error) => error.into(),
            error => Self::Storage(error.into()),
        }
    }
}
impl FoldError {
    fn storage(self) -> StorageError {
        match self {
            Self::Interrupted(error) => error.into(),
            Self::Storage(error) => error,
            Self::NonAdvancingMerge { kind, .. } => StorageError::Invalid(match kind {
                FoldKind::Changes => {
                    "changes() cannot advance past equal float and histogram timestamps"
                }
                FoldKind::Resets => {
                    "resets() cannot advance past equal float and histogram timestamps"
                }
                _ => unreachable!("mixed merge consumer"),
            }),
        }
    }
}

/// One owned window accumulator; None from finish_pass requests native replay.
pub(crate) struct FoldStep {
    selected_samples: Option<crate::statistics::OwnedSamples>,
    extrema_samples: Option<crate::statistics::OwnedSamples>,
    kind: FoldKind,
    start_timestamps: bool,
    count: u64,
    changes: i64,
    selected: Option<OwnedValue>,
    selected_time: Option<i64>,
    previous: Option<Previous>,
    float: Option<(f64, i64)>,
    float_last: Option<i64>,
    histogram_last: Option<i64>,
    first_time: Option<i64>,
    numerical: Option<NumericalFold>,
    native_pass: bool,
    native: Option<NativeFold>,
    native_error: Option<HistogramError>,
}
impl FoldStep {
    pub(crate) fn new(kind: FoldKind, start_timestamps: bool) -> Self {
        Self {
            selected_samples: None,
            extrema_samples: None,
            kind,
            start_timestamps,
            count: 0,
            changes: 0,
            selected: None,
            selected_time: None,
            previous: None,
            float: None,
            float_last: None,
            histogram_last: None,
            first_time: None,
            numerical: NumericalFold::new(kind),
            native_pass: false,
            native: None,
            native_error: None,
        }
    }

    pub(crate) fn receive(
        &mut self,
        sample: SampleRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), FoldError> {
        use FoldKind::*;
        control.check()?;
        if sample.value.is_stale() {
            return Ok(());
        }
        if self.native_pass {
            return self.receive_native(sample.value, control, account);
        }
        let kind = self.kind;
        let is_float = matches!(sample.value, SampleValueRef::Float(_));
        let other_last = if is_float {
            self.histogram_last
        } else {
            self.float_last
        };
        if kind.previous() && other_last == Some(sample.time_ms) {
            // Approved11.A: the source's two-head merge has no equal-time branch.
            return Err(FoldError::NonAdvancingMerge {
                kind,
                time_ms: sample.time_ms,
            });
        }
        self.count += 1;
        self.first_time.get_or_insert(sample.time_ms);
        if let SampleValueRef::Float(value) = sample.value {
            self.float_last = Some(sample.time_ms);
            if let Some(numerical) = &mut self.numerical {
                numerical.add(value);
            }
            if kind.extrema() {
                let replace = self.float.is_none_or(|(old, _)| {
                    old.is_nan()
                        || match kind {
                            Min => value < old,
                            Max => value > old,
                            TsMin => value <= old,
                            TsMax => value >= old,
                            _ => unreachable!(),
                        }
                });
                if replace {
                    self.float = Some((value, sample.time_ms));
                    if self.extrema_samples.is_none() {
                        self.extrema_samples = crate::statistics::hold_float(account, sample.value);
                    }
                }
            }
        } else {
            self.histogram_last = Some(sample.time_ms);
        }
        let histogram_tie = !is_float
            && self.selected_time == Some(sample.time_ms)
            && matches!(self.selected, Some(OwnedValue::Float(_)));
        let replace = if kind == First {
            self.count == 1 || histogram_tie
        } else if kind == Last {
            self.selected_time != Some(sample.time_ms)
                || !is_float
                || matches!(self.selected, Some(OwnedValue::Float(_)))
        } else {
            false
        };
        if replace {
            let value = crate::statistics::ValueParts::new(
                OwnedValue::copy_from_checked(
                    sample.value,
                    control,
                    account,
                    MemoryKind::Workspace,
                )?,
                account,
            );
            self.selected = Some(value.value);
            self.selected_samples = value.float_samples;
            self.selected_time = Some(sample.time_ms);
        }
        if kind.previous() {
            let current = Previous::copy(
                sample,
                kind == Resets && self.start_timestamps,
                control,
                account,
            )?;
            if let Some(previous) = &self.previous {
                let changed = match (current.value.as_ref(), previous.value.as_ref()) {
                    (SampleValueRef::Float(value), SampleValueRef::Float(old)) => {
                        if kind == Changes {
                            value != old && !(value.is_nan() && old.is_nan())
                        } else {
                            value < old
                                || start_timestamp_reset(
                                    previous.start_time_ms,
                                    previous.time_ms,
                                    current.start_time_ms,
                                    current.time_ms,
                                )
                        }
                    }
                    (
                        SampleValueRef::FloatHistogram(value),
                        SampleValueRef::FloatHistogram(old),
                    ) => {
                        if kind == Changes {
                            !value.equals_with(old, HistogramWork::new(control, account))?
                        } else {
                            start_timestamp_reset(
                                previous.start_time_ms,
                                previous.time_ms,
                                current.start_time_ms,
                                current.time_ms,
                            ) || detect_reset_with(
                                value,
                                old,
                                HistogramWork::new(control, account),
                            )?
                        }
                    }
                    _ => true,
                };
                if changed {
                    self.changes = self.changes.wrapping_add(1);
                }
            }
            self.previous = Some(current);
        }
        Ok(())
    }

    fn receive_native(
        &mut self,
        value: SampleValueRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), FoldError> {
        if self.native_error.is_some() {
            return Ok(());
        }
        let Some(native) = &mut self.native else {
            self.native = Some(NativeFold::new(
                self.kind,
                retain_histogram(value, control, account)?,
            ));
            return Ok(());
        };
        let converted;
        let value = match value {
            SampleValueRef::FloatHistogram(value) => value,
            SampleValueRef::IntegerHistogram(_) => {
                converted = retain_histogram(value, control, account)?;
                converted.as_ref()
            }
            SampleValueRef::Float(_) => unreachable!("classified histogram window"),
        };
        match native.add(value, control, account) {
            Ok(()) => Ok(()),
            Err(HistogramEvaluationError::Arithmetic(error)) => {
                self.native_error = Some(error);
                Ok(())
            }
            Err(HistogramEvaluationError::Interrupted(error)) => Err(error.into()),
            Err(HistogramEvaluationError::Source(error)) => Err(FoldError::Storage(error)),
        }
    }

    pub(crate) fn finish_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<FoldResult>, FoldError> {
        use FoldKind::*;
        let kind = self.kind;
        let mixed = self.float_last.is_some() && self.histogram_last.is_some();
        if self.native_pass {
            let result = match self.native_error.take() {
                Some(error) => Err(HistogramEvaluationError::Arithmetic(error)),
                None => self
                    .native
                    .take()
                    .expect("nonempty histogram window")
                    .finish(control, account),
            };
            return Ok(Some(match result {
                Ok(result) => result,
                Err(HistogramEvaluationError::Arithmetic(HistogramError::IncompatibleSchema)) => {
                    FoldResult {
                        float_samples: None,
                        progress: None,
                        value: None,
                        nonempty: true,
                        annotations: [Some(AnnotationKind::MixedExponentialCustomHistograms), None],
                    }
                }
                Err(HistogramEvaluationError::Arithmetic(error)) => {
                    return Err(HistogramWork::new(control, account)
                        .source(format_args!("{error}"), false)
                        .into());
                }
                Err(HistogramEvaluationError::Interrupted(error)) => return Err(error.into()),
                Err(HistogramEvaluationError::Source(error)) => {
                    return Err(FoldError::Storage(error));
                }
            }));
        }
        if matches!(kind, Sum | Avg) && self.histogram_last.is_some() {
            if mixed {
                return Ok(Some(FoldResult {
                    float_samples: None,
                    progress: None,
                    value: None,
                    nonempty: true,
                    annotations: [Some(AnnotationKind::MixedFloatsHistograms), None],
                }));
            }
            // Complete membership precedes native arithmetic and its diagnostics.
            self.native_pass = true;
            return Ok(None);
        }
        let value = if self.count == 0 {
            None
        } else {
            match kind {
                Count => Some(OwnedValue::Float(self.count as f64)),
                Present => Some(OwnedValue::Float(1.0)),
                First | Last => self.selected.take(),
                Min | Max => self.float.map(|(value, _)| OwnedValue::Float(value)),
                Sum | Avg | Stdvar | Stddev => self.float_last.map(|_| {
                    OwnedValue::Float(
                        self.numerical
                            .take()
                            .expect("numerical fold state")
                            .finish(kind),
                    )
                }),
                TsMin | TsMax => self
                    .float
                    .map(|(_, time)| OwnedValue::Float(time as f64 / 1000.0)),
                TsFirst => Some(OwnedValue::Float(
                    self.first_time.expect("nonempty range") as f64 / 1000.0,
                )),
                TsLast => Some(OwnedValue::Float(
                    self.float_last
                        .unwrap_or(0)
                        .max(self.histogram_last.unwrap_or(0)) as f64
                        / 1000.0,
                )),
                Changes | Resets => Some(OwnedValue::Float(self.changes as f64)),
            }
        };
        let float_samples = if matches!(kind, First | Last) {
            self.selected_samples.take()
        } else {
            value
                .as_ref()
                .and_then(|value| crate::statistics::hold_float(account, value.as_ref()))
        };
        Ok(Some(FoldResult {
            float_samples,
            progress: None,
            value,
            nonempty: self.count != 0,
            annotations: [
                (mixed && (kind.extrema() || kind.variance()))
                    .then_some(AnnotationKind::HistogramIgnoredInMixedRange),
                None,
            ],
        }))
    }
}

pub(crate) fn fold<S: Series>(
    series: &S,
    bounds: impl Into<Option<TimeBounds>>,
    kind: FoldKind,
    mut options: FoldOptions,
    control: &Control,
    account: &QueryAccounting,
) -> Result<FoldResult, StorageError> {
    let bounds = bounds.into();
    let mut anchor = None::<(i64, bool, u64)>;
    let mut full_nonempty = false;
    if let Some(start_ms) = options.anchor_start_ms {
        let mut found = false;
        let mut position = 0;
        let progress = scan_matrix::<_, StorageError>(
            series,
            bounds,
            options.matrix,
            control,
            account,
            |sample| {
                control.check()?;
                account.work(1);
                full_nonempty = true;
                if sample.time_ms > start_ms {
                    found = true;
                } else {
                    let is_float = matches!(sample.value, SampleValueRef::Float(_));
                    if anchor.is_none_or(|(time, prior_float, _)| {
                        sample.time_ms > time
                            || sample.time_ms == time && (is_float || !prior_float)
                    }) {
                        anchor = Some((sample.time_ms, is_float, position));
                    }
                }
                position += 1;
                Ok(())
            },
        )?;
        if !found {
            return Ok(FoldResult {
                float_samples: None,
                progress,
                value: None,
                nonempty: full_nonempty,
                annotations: [None, None],
            });
        }
        options.matrix = options.matrix.map(|window| window.with_progress(progress));
    }
    let mut step = FoldStep::new(kind, options.start_timestamps);
    let reuse_initial_average = kind == FoldKind::Avg
        && options.anchor_start_ms.is_none()
        && options.matrix.is_some_and(|window| {
            !window.statistics
                && window.buffer_ms.is_none()
                && window.reduce_delta
                && window.range_ms > 0
                && window
                    .end_ms
                    .checked_sub(window.range_ms)
                    .and_then(|lower| lower.checked_add(1))
                    .is_some_and(|lower| {
                        bounds.is_some_and(|bounds| {
                            bounds.start_ms >= lower
                                && bounds.end_ms <= window.end_ms
                                && bounds.start_ms <= bounds.end_ms
                        })
                    })
        });
    loop {
        let mut position = 0u64;
        let progress = if reuse_initial_average && !step.native_pass {
            let window = options.matrix.expect("certified average window");
            let acquisition = crate::matrix_reader::probe::<_, StorageError>(
                series,
                bounds,
                window,
                control,
                account,
                |sample| {
                    step.receive(sample, control, account)
                        .map_err(FoldError::storage)
                },
            )?;
            let progress = if acquisition.populated && acquisition.histograms_allocated {
                crate::matrix_reader::consume::<_, StorageError>(
                    series,
                    bounds,
                    window.with_progress(Some(acquisition.progress)),
                    control,
                    account,
                    |_| Ok(()),
                )?
                .progress
            } else {
                account.samples_read(acquisition.progress.sample_units);
                acquisition.progress
            };
            Some(progress)
        } else {
            scan_matrix::<_, StorageError>(
                series,
                bounds,
                options.matrix,
                control,
                account,
                |sample| {
                    control.check()?;
                    let include = options.anchor_start_ms.is_none_or(|start_ms| {
                        sample.time_ms > start_ms
                            || anchor.is_some_and(|(_, _, index)| index == position)
                    });
                    position += 1;
                    if include {
                        step.receive(sample, control, account)
                            .map_err(FoldError::storage)?;
                    }
                    Ok(())
                },
            )?
        };
        if let Some(mut result) = step
            .finish_pass(control, account)
            .map_err(FoldError::storage)?
        {
            result.progress = progress;
            if options.anchor_start_ms.is_some() {
                result.nonempty = full_nonempty;
            }
            return Ok(result);
        }
        options.matrix = options.matrix.map(|window| window.with_progress(progress));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_value_replacement_counts_both_initialized_owners() {
        let histogram = crate::histogram::FloatHistogram::default();
        for (first, last, expected) in [
            (SampleValueRef::Float(1.0), SampleValueRef::Float(2.0), 2),
            (
                SampleValueRef::Float(1.0),
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                12,
            ),
            (
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                SampleValueRef::Float(2.0),
                12,
            ),
            (
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                22,
            ),
        ] {
            let account = crate::accounting::EngineAccounting::default().query();
            let control = Control::default();
            let mut step = FoldStep::new(FoldKind::Last, false);
            for (time, value) in [(1000, first), (2000, last)] {
                step.receive(
                    SampleRef {
                        time_ms: time,
                        start_time_ms: 0,
                        value,
                    },
                    &control,
                    &account,
                )
                .unwrap();
            }
            assert_eq!(account.snapshot().statistics.samples.peak_samples, expected);
            let output = step.finish_pass(&control, &account).unwrap().unwrap();
            drop((output, step));
            assert_eq!(account.current_samples(), 0);
        }
    }

    #[test]
    fn failed_float_predecessor_construction_does_not_raise_sample_peak() {
        let account = crate::accounting::EngineAccounting::default().query();
        let control = Control::default();
        control.cancellation.cancel();
        let outcome = Previous::copy(
            SampleRef {
                time_ms: 1000,
                start_time_ms: 0,
                value: SampleValueRef::Float(1.0),
            },
            false,
            &control,
            &account,
        );
        assert!(matches!(outcome, Err(Interrupted::Cancelled)));
        assert_eq!(account.snapshot().statistics.samples.peak_samples, 0);
        assert_eq!(account.current_samples(), 0);
    }
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        histogram::{FloatHistogram, Span},
        storage::{Cancellation, LabelRef, ProviderResources, SampleCursor, SamplePage},
        test_storage::{GeneratedCursor, GeneratedSeries, interrupt_during},
        value::{Sample, SampleValue},
    };
    use std::{cell::RefCell, sync::atomic::Ordering};

    fn average_options() -> FoldOptions {
        FoldOptions {
            start_timestamps: false,
            anchor_start_ms: None,
            matrix: Some(MatrixWindow {
                first_end_ms: 2000,
                end_ms: 2000,
                range_ms: 2000,
                buffer_ms: None,
                step_ms: 0,
                data_bounds: TimeBounds {
                    start_ms: 1,
                    end_ms: 2000,
                },
                statistics: false,
                reduce_delta: true,
                resume: None,
            }),
        }
    }

    fn average_pair(index: usize) -> Sample {
        Sample {
            time_ms: (index as i64 + 1) * 1000,
            start_time_ms: 0,
            value: SampleValue::Float(index as f64 + 1.0),
        }
    }
    fn average_native(index: usize) -> Sample {
        Sample {
            value: SampleValue::FloatHistogram(FloatHistogram {
                count: (index + 1) as f64,
                sum: (index + 1) as f64,
                ..FloatHistogram::default()
            }),
            ..average_pair(index)
        }
    }
    fn average_mixed(index: usize) -> Sample {
        if index == 0 {
            average_pair(index)
        } else {
            average_native(index)
        }
    }
    fn average_stale_histogram(index: usize) -> Sample {
        if index == 0 {
            average_pair(index)
        } else {
            let mut sample = average_native(index);
            let SampleValue::FloatHistogram(histogram) = &mut sample.value else {
                unreachable!()
            };
            histogram.sum = f64::from_bits(crate::value::STALE_NAN_BITS);
            sample
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum AverageOperation {
        Next(usize, usize),
        Seek(usize, i64),
        Close(usize),
    }
    struct AverageSeries {
        source: GeneratedSeries,
        failure: Option<AverageOperation>,
        cancel_at: Option<AverageOperation>,
        cancellation: Cancellation,
        operations: RefCell<Vec<AverageOperation>>,
    }
    impl AverageSeries {
        fn observe(&self, operation: AverageOperation) -> Result<(), StorageError> {
            self.operations.borrow_mut().push(operation);
            if self.cancel_at == Some(operation) {
                self.cancellation.cancel();
            }
            if self.failure == Some(operation) {
                return Err(StorageError::Provider(
                    b"average operation failed".to_vec().into(),
                ));
            }
            Ok(())
        }
    }
    struct AverageCursor<'a> {
        inner: GeneratedCursor<'a>,
        source: &'a AverageSeries,
        ordinal: usize,
        next_calls: usize,
    }
    impl Series for AverageSeries {
        type Cursor<'a> = AverageCursor<'a>;
        fn label_count(&self) -> usize {
            0
        }
        fn label(&self, _: usize) -> LabelRef<'_> {
            panic!("unlabelled average fixture")
        }
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn cursor(
            &self,
            bounds: TimeBounds,
            control: &Control,
        ) -> Result<Self::Cursor<'_>, StorageError> {
            let inner = self.source.cursor(bounds, control)?;
            Ok(AverageCursor {
                inner,
                source: self,
                ordinal: self.source.counters.opened.load(Ordering::Relaxed),
                next_calls: 0,
            })
        }
    }
    impl SampleCursor for AverageCursor<'_> {
        fn resources(&self) -> ProviderResources {
            self.inner.resources()
        }
        fn max_page_samples(&self) -> usize {
            self.inner.max_page_samples()
        }
        fn seek(&mut self, target: i64, control: &Control) -> Result<(), StorageError> {
            self.source
                .observe(AverageOperation::Seek(self.ordinal, target))?;
            self.inner.seek(target, control)
        }
        fn next(
            &mut self,
            control: &Control,
            resources: &mut ProviderResources,
        ) -> Result<Option<SamplePage<'_>>, StorageError> {
            let call = self.next_calls;
            self.next_calls += 1;
            self.source
                .observe(AverageOperation::Next(self.ordinal, call))?;
            self.inner.next(control, resources)
        }
    }
    impl Drop for AverageCursor<'_> {
        fn drop(&mut self) {
            self.source
                .observe(AverageOperation::Close(self.ordinal))
                .unwrap();
        }
    }
    fn assert_average_closed(source: &GeneratedSeries, account: &QueryAccounting) {
        assert_eq!(
            source.counters.opened.load(Ordering::Relaxed),
            source.counters.closed.load(Ordering::Relaxed)
        );
        assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn average_acquisition_failures_and_native_consumption_keep_their_phases() {
        let mut failures = 0;
        for page in [1, 17, 1024] {
            for (sample, expected_cursors, expected_read, expected_kind) in [
                (average_pair as fn(usize) -> Sample, 1, 2, 0),
                (average_stale_histogram, 2, 1, 1),
                (average_mixed, 2, 12, 2),
                (average_native, 4, 44, 3),
            ] {
                let control = Control::default();
                let normal = AverageSeries {
                    source: GeneratedSeries::new(2, page, sample),
                    failure: None,
                    cancel_at: None,
                    cancellation: control.cancellation.clone(),
                    operations: RefCell::new(Vec::new()),
                };
                let account = EngineAccounting::default().query();
                let output = fold(
                    &normal,
                    TimeBounds {
                        start_ms: 1,
                        end_ms: 2000,
                    },
                    FoldKind::Avg,
                    average_options(),
                    &control,
                    &account,
                )
                .unwrap();
                assert!(output.nonempty);
                match (expected_kind, &output.value) {
                    (0, Some(OwnedValue::Float(value))) => {
                        assert_eq!(value.to_bits(), 1.5f64.to_bits())
                    }
                    (1, Some(OwnedValue::Float(value))) => {
                        assert_eq!(value.to_bits(), 1.0f64.to_bits())
                    }
                    (2, None) => assert_eq!(
                        output.annotations,
                        [Some(AnnotationKind::MixedFloatsHistograms), None]
                    ),
                    (3, Some(OwnedValue::Histogram(value))) => {
                        assert_eq!(value.as_ref().count.to_bits(), 1.5f64.to_bits());
                        assert_eq!(value.as_ref().sum.to_bits(), 1.5f64.to_bits());
                    }
                    _ => panic!("complete average output kind"),
                }
                if expected_kind != 2 {
                    assert_eq!(output.annotations, [None, None]);
                }
                assert_eq!(
                    normal.source.counters.opened.load(Ordering::Relaxed),
                    expected_cursors
                );
                assert_eq!(
                    account.snapshot().statistics.samples.samples_read,
                    expected_read
                );
                drop(output);
                assert_average_closed(&normal.source, &account);
                let operations = normal.operations.into_inner();
                for operation in operations
                    .into_iter()
                    .filter(|op| !matches!(op, AverageOperation::Close(_)))
                {
                    let control = Control::default();
                    let source = AverageSeries {
                        source: GeneratedSeries::new(2, page, sample),
                        failure: Some(operation),
                        cancel_at: None,
                        cancellation: control.cancellation.clone(),
                        operations: RefCell::new(Vec::new()),
                    };
                    let account = EngineAccounting::default().query();
                    let output = fold(
                        &source,
                        TimeBounds {
                            start_ms: 1,
                            end_ms: 2000,
                        },
                        FoldKind::Avg,
                        average_options(),
                        &control,
                        &account,
                    );
                    assert!(
                        matches!(&output, Err(StorageError::Provider(message)) if message.as_ref() == b"average operation failed"),
                        "{operation:?}"
                    );
                    if matches!(
                        operation,
                        AverageOperation::Next(1, _) | AverageOperation::Seek(1, _)
                    ) {
                        assert_eq!(account.snapshot().statistics.samples.samples_read, 0);
                    }
                    drop(output);
                    assert_average_closed(&source.source, &account);
                    failures += 1;
                }
            }
        }
        println!("{{\"avg_reuse_operation_failures\":{failures}}}");
        assert!(failures >= 72);
    }

    #[test]
    fn average_reuse_cancellation_discards_provisional_credits_and_result() {
        for page in [1, 17, 1024] {
            for sample in [
                average_pair as fn(usize) -> Sample,
                average_stale_histogram,
                average_native,
            ] {
                for cancel_at in [AverageOperation::Next(1, 0), AverageOperation::Close(1)] {
                    let control = Control::default();
                    let source = AverageSeries {
                        source: GeneratedSeries::new(2, page, sample),
                        failure: None,
                        cancel_at: Some(cancel_at),
                        cancellation: control.cancellation.clone(),
                        operations: RefCell::new(Vec::new()),
                    };
                    let account = EngineAccounting::default().query();
                    let output = fold(
                        &source,
                        TimeBounds {
                            start_ms: 1,
                            end_ms: 2000,
                        },
                        FoldKind::Avg,
                        average_options(),
                        &control,
                        &account,
                    );
                    assert!(matches!(
                        output,
                        Err(StorageError::Interrupted(Interrupted::Cancelled))
                    ));
                    assert_eq!(account.snapshot().statistics.samples.samples_read, 0);
                    drop(output);
                    assert_average_closed(&source.source, &account);
                }
            }
        }
    }

    #[test]
    fn wider_average_bounds_keep_the_consuming_window() {
        fn sample(index: usize) -> Sample {
            Sample {
                time_ms: [1200, 1800, 2100, 3200, 3800][index],
                value: SampleValue::Float([1.0, 2.0, 9.0, 3.0, 4.0][index]),
                start_time_ms: 0,
            }
        }
        for page in [1, 17, 1024] {
            let source = GeneratedSeries::new(5, page, sample);
            let account = EngineAccounting::default().query();
            let bounds = TimeBounds {
                start_ms: 0,
                end_ms: 10_000,
            };
            let mut options = average_options();
            options.matrix = Some(MatrixWindow {
                first_end_ms: 2000,
                end_ms: 4000,
                range_ms: 1000,
                step_ms: 2000,
                data_bounds: bounds,
                ..options.matrix.unwrap()
            });
            let output = fold(
                &source,
                bounds,
                FoldKind::Avg,
                options,
                &Control::default(),
                &account,
            )
            .unwrap();
            let Some(OwnedValue::Float(value)) = &output.value else {
                panic!("float average")
            };
            assert_eq!(value.to_bits(), 3.5f64.to_bits());
            assert_eq!(output.annotations, [None, None]);
            assert_eq!(account.snapshot().statistics.samples.samples_read, 2);
            assert_eq!(source.counters.opened.load(Ordering::Relaxed), 2);
            drop(output);
            assert_average_closed(&source, &account);
        }
    }

    #[test]
    fn average_reuse_preserves_overflow_nan_zero_and_stale_values() {
        fn overflow(index: usize) -> Sample {
            Sample {
                value: SampleValue::Float(f64::MAX),
                ..average_pair(index)
            }
        }
        fn negative_zero(index: usize) -> Sample {
            Sample {
                value: SampleValue::Float(-0.0),
                ..average_pair(index)
            }
        }
        fn nan(index: usize) -> Sample {
            Sample {
                value: SampleValue::Float(if index == 0 {
                    f64::from_bits(0x7ff8_0000_0000_1234)
                } else {
                    1.0
                }),
                ..average_pair(index)
            }
        }
        fn stale(index: usize) -> Sample {
            Sample {
                value: SampleValue::Float(if index == 0 {
                    1.0
                } else {
                    f64::from_bits(crate::value::STALE_NAN_BITS)
                }),
                ..average_pair(index)
            }
        }
        fn empty(index: usize) -> Sample {
            Sample {
                value: SampleValue::Float(f64::from_bits(crate::value::STALE_NAN_BITS)),
                ..average_pair(index)
            }
        }
        for page in [1, 17, 1024] {
            for (sample, expected, read) in [
                (overflow as fn(usize) -> Sample, Some(f64::MAX.to_bits()), 2),
                (negative_zero, Some(0.0f64.to_bits()), 2),
                (nan, Some(0x7ff8_0000_0000_1234), 2),
                (stale, Some(1.0f64.to_bits()), 1),
                (empty, None, 0),
            ] {
                let source = GeneratedSeries::new(2, page, sample);
                let account = EngineAccounting::default().query();
                let output = fold(
                    &source,
                    TimeBounds {
                        start_ms: 1,
                        end_ms: 2000,
                    },
                    FoldKind::Avg,
                    average_options(),
                    &Control::default(),
                    &account,
                )
                .unwrap();
                assert_eq!(output.nonempty, expected.is_some());
                assert_eq!(output.annotations, [None, None]);
                assert_eq!(
                    output.value.as_ref().map(|value| match value {
                        OwnedValue::Float(value) => value.to_bits(),
                        _ => panic!("float average"),
                    }),
                    expected
                );
                assert_eq!(account.snapshot().statistics.samples.samples_read, read);
                assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
                drop(output);
                assert_average_closed(&source, &account);
            }
        }
    }

    #[test]
    fn average_reuse_keeps_long_histories_page_bounded_and_interruptible() {
        for page in [1, 17, 1024] {
            let mut previous_peaks = None;
            for count in [2048, 100_000] {
                let source = GeneratedSeries::new(count, page, crate::test_storage::counter);
                let mut options = average_options();
                let bounds = TimeBounds {
                    start_ms: 1,
                    end_ms: count as i64 * 1000,
                };
                options.matrix = Some(MatrixWindow {
                    first_end_ms: bounds.end_ms,
                    end_ms: bounds.end_ms,
                    range_ms: bounds.end_ms,
                    data_bounds: bounds,
                    ..options.matrix.unwrap()
                });
                let account = EngineAccounting::default().query();
                let output = fold(
                    &source,
                    bounds,
                    FoldKind::Avg,
                    options,
                    &Control::default(),
                    &account,
                )
                .unwrap();
                let Some(OwnedValue::Float(value)) = &output.value else {
                    panic!("float average")
                };
                assert_eq!(value.to_bits(), ((count as f64 + 1.0) / 2.0).to_bits());
                assert_eq!(output.annotations, [None, None]);
                assert_eq!(
                    account.snapshot().statistics.samples.samples_read,
                    count as u64
                );
                assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
                drop(output);
                assert_average_closed(&source, &account);
                let snapshot = account.snapshot();
                let peaks = [
                    MemoryKind::Workspace,
                    MemoryKind::ProviderHeld,
                    MemoryKind::BorrowedView,
                ]
                .map(|kind| snapshot.memory[kind as usize].peak);
                if let Some(previous) = previous_peaks {
                    assert_eq!(peaks, previous);
                }
                previous_peaks = Some(peaks);
            }
            let source = GeneratedSeries::new(100_000, page, crate::test_storage::counter);
            let account = EngineAccounting::default().query();
            let control = Control::default();
            let bounds = TimeBounds {
                start_ms: 1,
                end_ms: 100_000_000,
            };
            let mut options = average_options();
            options.matrix = Some(MatrixWindow {
                first_end_ms: bounds.end_ms,
                end_ms: bounds.end_ms,
                range_ms: bounds.end_ms,
                data_bounds: bounds,
                ..options.matrix.unwrap()
            });
            let output = interrupt_during(&account, &control, || {
                fold(&source, bounds, FoldKind::Avg, options, &control, &account)
            });
            assert!(matches!(
                output,
                Err(StorageError::Interrupted(Interrupted::Cancelled))
            ));
            assert!(source.counters.samples_returned.load(Ordering::Relaxed) > 0);
            assert_eq!(account.snapshot().statistics.samples.samples_read, 0);
            drop(output);
            assert_average_closed(&source, &account);
        }
    }

    #[test]
    fn streamed_membership_and_average_counts_preserve_u32_rollover() {
        let account = EngineAccounting::default().query();
        let control = Control::default();
        let prefix = u64::from(u32::MAX);
        let sample = SampleRef {
            time_ms: (prefix + 1) as i64,
            start_time_ms: 0,
            value: SampleValueRef::Float(1.0),
        };
        let mut count = FoldStep::new(FoldKind::Count, false);
        count.count = prefix;
        count.receive(sample, &control, &account).unwrap();
        let result = count.finish_pass(&control, &account).unwrap().unwrap();
        assert!(result.nonempty);
        let Some(OwnedValue::Float(value)) = result.value else {
            panic!("count result");
        };
        assert_eq!(value.to_bits(), 4_294_967_296.0f64.to_bits());

        let mut average = NumericalFold::new(FoldKind::Avg).unwrap();
        if let NumericalFold::Average { count, sum, .. } = &mut average {
            // Compress a prefix of identical values into its sufficient state.
            *count = prefix;
            *sum = prefix as f64;
        }
        average.add(1.0);
        assert_eq!(average.finish(FoldKind::Avg).to_bits(), 1.0f64.to_bits());

        let histogram = FloatHistogram {
            count: prefix as f64,
            sum: prefix as f64,
            zero_count: prefix as f64,
            ..FloatHistogram::default()
        };
        let mut average = NativeFold::new(
            FoldKind::Avg,
            retain_histogram(
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                &control,
                &account,
            )
            .unwrap(),
        );
        average.count = prefix;
        let next = FloatHistogram {
            count: 1.0,
            sum: 1.0,
            zero_count: 1.0,
            ..FloatHistogram::default()
        };
        average.add(next.as_ref(), &control, &account).unwrap();
        let result = average.finish(&control, &account).unwrap();
        let Some(OwnedValue::Histogram(value)) = result.value else {
            panic!("native average");
        };
        assert_eq!(value.as_ref().count.to_bits(), 1.0f64.to_bits());
        assert_eq!(value.as_ref().sum.to_bits(), 1.0f64.to_bits());
        assert_eq!(value.as_ref().zero_count.to_bits(), 1.0f64.to_bits());
        drop(value);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn owned_fold_ties_preserve_finite_consumers_and_reject_only_nonadvancing_merges() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let histogram = FloatHistogram {
            count: 7.0,
            ..FloatHistogram::default()
        };
        let points = [
            SampleRef {
                time_ms: 1000,
                start_time_ms: 0,
                value: SampleValueRef::Float(2.0),
            },
            SampleRef {
                time_ms: 1000,
                start_time_ms: 0,
                value: SampleValueRef::FloatHistogram(histogram.as_ref()),
            },
        ];
        for kind in [
            FoldKind::First,
            FoldKind::Last,
            FoldKind::Count,
            FoldKind::Changes,
            FoldKind::Resets,
        ] {
            let mut step = FoldStep::new(kind, true);
            step.receive(points[0], &control, &account).unwrap();
            let received = step.receive(points[1], &control, &account);
            if matches!(kind, FoldKind::Changes | FoldKind::Resets) {
                assert_eq!(
                    received,
                    Err(FoldError::NonAdvancingMerge {
                        kind,
                        time_ms: 1000
                    })
                );
            } else {
                received.unwrap();
                let result = step.finish_pass(&control, &account).unwrap().unwrap();
                match (kind, result.value.unwrap()) {
                    (FoldKind::First | FoldKind::Last, OwnedValue::Histogram(value)) => {
                        assert_eq!(value.as_ref().count, 7.0)
                    }
                    (FoldKind::Count, OwnedValue::Float(value)) => assert_eq!(value, 2.0),
                    _ => panic!("typed fold output"),
                }
            }
        }
        for kind in [FoldKind::Changes, FoldKind::Resets] {
            let mut step = FoldStep::new(kind, false);
            step.receive(points[0], &control, &account).unwrap();
            step.receive(points[0], &control, &account).unwrap();
            let result = step.finish_pass(&control, &account).unwrap().unwrap();
            assert!(matches!(result.value, Some(OwnedValue::Float(0.0))));
        }
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn owned_native_fold_waits_for_complete_membership_before_requesting_arithmetic() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let histogram = FloatHistogram {
            count: 2.0,
            sum: 3.0,
            ..FloatHistogram::default()
        };
        let sample = SampleRef {
            time_ms: 1000,
            start_time_ms: 0,
            value: SampleValueRef::FloatHistogram(histogram.as_ref()),
        };
        for kind in [FoldKind::Sum, FoldKind::Avg] {
            {
                let mut step = FoldStep::new(kind, false);
                step.receive(sample, &control, &account).unwrap();
                step.receive(
                    SampleRef {
                        time_ms: 2000,
                        value: SampleValueRef::Float(1.0),
                        ..sample
                    },
                    &control,
                    &account,
                )
                .unwrap();
                assert!(step.native.is_none());
                let result = step.finish_pass(&control, &account).unwrap().unwrap();
                assert!(result.value.is_none());
                assert_eq!(
                    result.annotations,
                    [Some(AnnotationKind::MixedFloatsHistograms), None]
                );
            }
            {
                let mut step = FoldStep::new(kind, false);
                step.receive(sample, &control, &account).unwrap();
                step.receive(
                    SampleRef {
                        time_ms: 2000,
                        ..sample
                    },
                    &control,
                    &account,
                )
                .unwrap();
                assert!(step.finish_pass(&control, &account).unwrap().is_none());
                for time_ms in [1000, 2000] {
                    step.receive(SampleRef { time_ms, ..sample }, &control, &account)
                        .unwrap();
                }
                let result = step.finish_pass(&control, &account).unwrap().unwrap();
                let Some(OwnedValue::Histogram(value)) = result.value else {
                    panic!("native fold result")
                };
                let factor = if kind == FoldKind::Sum { 2.0 } else { 1.0 };
                assert_eq!(value.as_ref().count, 2.0 * factor);
                assert_eq!(value.as_ref().sum, 3.0 * factor);
            }
            account.snapshot().assert_memory_released();
        }
    }

    fn large_native_sample(index: usize) -> Sample {
        Sample {
            time_ms: 1000 + index as i64 * 1000,
            start_time_ms: 0,
            value: SampleValue::FloatHistogram(FloatHistogram {
                count: 16_384.0,
                sum: 16_384.0,
                positive_spans: Some(
                    vec![Span {
                        offset: 0,
                        length: 16_384,
                    }]
                    .into(),
                ),
                positive_buckets: Some(vec![1.0; 16_384].into()),
                ..FloatHistogram::default()
            }),
        }
    }

    #[test]
    fn native_range_retention_interrupts_and_closes_its_page_cursor() {
        for kind in [
            FoldKind::First,
            FoldKind::Last,
            FoldKind::Sum,
            FoldKind::Avg,
            FoldKind::Changes,
            FoldKind::Resets,
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let series = GeneratedSeries::new(2, 1, large_native_sample);
            let result = interrupt_during(&account, &control, || {
                fold(
                    &series,
                    TimeBounds {
                        start_ms: 1000,
                        end_ms: 2000,
                    },
                    kind,
                    FoldOptions {
                        start_timestamps: true,
                        matrix: None,
                        anchor_start_ms: None,
                    },
                    &control,
                    &account,
                )
            });
            assert!(matches!(
                result,
                Err(StorageError::Interrupted(Interrupted::Cancelled))
            ));
            assert_eq!(
                series.counters.opened.load(Ordering::Relaxed),
                series.counters.closed.load(Ordering::Relaxed)
            );
            assert_eq!(series.counters.peak.load(Ordering::Relaxed), 1);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }
}
