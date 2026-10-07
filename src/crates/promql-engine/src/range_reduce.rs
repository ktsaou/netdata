// SPDX-License-Identifier: GPL-3.0-or-later
// Ordinary range reducers derive from pinned promql/functions.go; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    annotations::{AnnotationKind, HistogramOperation},
    histogram::{FloatHistogramRef, HistogramError},
    histogram_accumulator::{Finalization, NativeAccumulator},
    histogram_counter::detect_reset_with,
    histogram_stats::{StatisticsWindow, scan_statistics},
    histogram_work::{HistogramEvaluationError, HistogramWork},
    numerical::{compensated_add, sqrt},
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
    pub(crate) value: Option<OwnedValue>,
    pub(crate) annotations: [Option<AnnotationKind>; 2],
}

struct NativeFold {
    accumulator: NativeAccumulator,
    count: usize,
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
            value: Some(OwnedValue::Histogram(result.value)),
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

fn fold_histograms<S: Series>(
    series: &S,
    bounds: TimeBounds,
    kind: FoldKind,
    statistics: Option<StatisticsWindow>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<FoldResult, StorageError> {
    let mut state = None::<NativeFold>;
    let mut error = None;
    scan_statistics(series, bounds, statistics, control, account, |sample| {
        if sample.value.is_stale() || error.is_some() {
            return Ok(());
        }
        let Some(state) = &mut state else {
            state = Some(NativeFold::new(
                kind,
                retain_histogram(sample.value, control, account)?,
            ));
            return Ok(());
        };
        let converted;
        let value = match sample.value {
            SampleValueRef::FloatHistogram(value) => value,
            SampleValueRef::IntegerHistogram(_) => {
                converted = retain_histogram(sample.value, control, account)?;
                converted.as_ref()
            }
            SampleValueRef::Float(_) => unreachable!("classified histogram window"),
        };
        match state.add(value, control, account) {
            Ok(()) => (),
            Err(HistogramEvaluationError::Arithmetic(failure)) => error = Some(failure),
            Err(HistogramEvaluationError::Interrupted(failure)) => return Err(failure.into()),
        }
        Ok(())
    })?;
    let result = match error {
        Some(error) => Err(HistogramEvaluationError::Arithmetic(error)),
        None => state
            .expect("nonempty histogram window")
            .finish(control, account),
    };
    Ok(match result {
        Ok(result) => result,
        Err(HistogramEvaluationError::Arithmetic(HistogramError::IncompatibleSchema)) => {
            FoldResult {
                value: None,
                annotations: [Some(AnnotationKind::MixedExponentialCustomHistograms), None],
            }
        }
        Err(HistogramEvaluationError::Arithmetic(error)) => panic!("{error}"),
        Err(HistogramEvaluationError::Interrupted(error)) => return Err(error.into()),
    })
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
        count: usize,
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
                    *mean = *sum / (count - 1.0);
                    *compensation /= count - 1.0;
                }
                let q = (count - 1.0) / count;
                (*mean, *compensation) =
                    compensated_add(value / count, q * *mean, q * *compensation);
            }
            Self::Variance {
                count,
                mean,
                mean_compensation,
                auxiliary,
                auxiliary_compensation,
            } => {
                *count += 1.0;
                let delta = value - (*mean + *mean_compensation);
                (*mean, *mean_compensation) =
                    compensated_add(delta / *count, *mean, *mean_compensation);
                (*auxiliary, *auxiliary_compensation) = compensated_add(
                    delta * (value - (*mean + *mean_compensation)),
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
                    sum + compensation
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
                    mean + compensation
                } else {
                    sum / count as f64 + compensation / count as f64
                }
            }
            Self::Variance {
                count,
                auxiliary,
                auxiliary_compensation,
                ..
            } => {
                let variance = (auxiliary + auxiliary_compensation) / count;
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
}
impl Previous {
    fn copy(
        sample: SampleRef<'_>,
        use_start: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, Interrupted> {
        Ok(Self {
            value: OwnedValue::copy_from_checked(
                sample.value,
                control,
                account,
                MemoryKind::Workspace,
            )?,
            time_ms: sample.time_ms,
            start_time_ms: if use_start { sample.start_time_ms } else { 0 },
        })
    }
}

pub(crate) struct FoldOptions {
    pub(crate) start_timestamps: bool,
    pub(crate) statistics: Option<StatisticsWindow>,
}

pub(crate) fn fold<S: Series>(
    series: &S,
    bounds: TimeBounds,
    kind: FoldKind,
    options: FoldOptions,
    control: &Control,
    account: &QueryAccounting,
) -> Result<FoldResult, StorageError> {
    use FoldKind::*;
    let mut count = 0usize;
    let mut changes = 0i64;
    let mut selected = None;
    let mut previous = None::<Previous>;
    let mut float = None::<(f64, i64)>;
    let mut float_last = None::<i64>;
    let mut histogram_last = None::<i64>;
    let mut first_time = None::<i64>;
    let mut numerical = NumericalFold::new(kind);
    scan_statistics(
        series,
        bounds,
        options.statistics,
        control,
        account,
        |sample| {
            if sample.value.is_stale() {
                return Ok(());
            }
            count += 1;
            first_time.get_or_insert(sample.time_ms);
            if let SampleValueRef::Float(value) = sample.value {
                float_last = Some(sample.time_ms);
                if let Some(numerical) = &mut numerical {
                    numerical.add(value);
                }
                if kind.extrema() {
                    let replace = float.is_none_or(|(old, _)| {
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
                        float = Some((value, sample.time_ms));
                    }
                }
            } else {
                histogram_last = Some(sample.time_ms);
            }
            if kind == Last || kind == First && count == 1 {
                selected = Some(OwnedValue::copy_from_checked(
                    sample.value,
                    control,
                    account,
                    MemoryKind::Workspace,
                )?);
            }
            if kind.previous() {
                let current = Previous::copy(
                    sample,
                    kind == Resets && options.start_timestamps,
                    control,
                    account,
                )?;
                if let Some(previous) = &previous {
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
                        changes = changes.wrapping_add(1);
                    }
                }
                previous = Some(current);
            }
            Ok(())
        },
    )?;
    let mixed = float_last.is_some() && histogram_last.is_some();
    let native_fold = matches!(kind, Sum | Avg) && histogram_last.is_some();
    if native_fold {
        if mixed {
            return Ok(FoldResult {
                value: None,
                annotations: [Some(AnnotationKind::MixedFloatsHistograms), None],
            });
        }
        // Classify before arithmetic so a late float suppresses all native outcomes.
        return fold_histograms(series, bounds, kind, options.statistics, control, account);
    }
    let value = if count == 0 {
        None
    } else {
        match kind {
            Count => Some(OwnedValue::Float(count as f64)),
            Present => Some(OwnedValue::Float(1.0)),
            First | Last => selected,
            Min | Max => float.map(|(value, _)| OwnedValue::Float(value)),
            Sum | Avg | Stdvar | Stddev => float_last
                .map(|_| OwnedValue::Float(numerical.expect("numerical fold state").finish(kind))),
            TsMin | TsMax => float.map(|(_, time)| OwnedValue::Float(time as f64 / 1000.0)),
            TsFirst => Some(OwnedValue::Float(
                first_time.expect("nonempty range") as f64 / 1000.0,
            )),
            // Missing kinds have timestamp zero in the source, even with negative timestamps.
            TsLast => Some(OwnedValue::Float(
                float_last.unwrap_or(0).max(histogram_last.unwrap_or(0)) as f64 / 1000.0,
            )),
            Changes | Resets => Some(OwnedValue::Float(changes as f64)),
        }
    };
    Ok(FoldResult {
        value,
        annotations: [
            if mixed && (kind.extrema() || kind.variance()) {
                Some(AnnotationKind::HistogramIgnoredInMixedRange)
            } else {
                None
            },
            None,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{FloatHistogram, Span},
        test_storage::{GeneratedSeries, interrupt_during},
        value::{Sample, SampleValue},
    };
    use std::sync::atomic::Ordering;

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
                        statistics: None,
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
