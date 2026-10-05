// SPDX-License-Identifier: GPL-3.0-or-later
// Ordinary range reducers derive from pinned promql/functions.go; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    annotations::{AnnotationKind, HistogramOperation},
    histogram::{COUNTER_RESET, FloatHistogramRef, HistogramError, NOT_RESET},
    numerical::{compensated_add, sqrt},
    range_float::{RateKind, start_timestamp_reset},
    result::{OwnedHistogram, OwnedValue},
    storage::{Control, Series, StorageError, TimeBounds, scan},
    value::{SampleRef, SampleValueRef},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RangeFunction {
    Rate(RateKind),
    Fold(FoldKind),
}
impl RangeFunction {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        use FoldKind::*;
        Some(match name {
            "rate" => Self::Rate(RateKind::Rate),
            "increase" => Self::Rate(RateKind::Increase),
            "delta" => Self::Rate(RateKind::Delta),
            "count_over_time" => Self::Fold(Count),
            "present_over_time" => Self::Fold(Present),
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
    sum: OwnedHistogram,
    compensation: Option<OwnedHistogram>,
    count: usize,
    average: bool,
    incremental: bool,
    reset_seen: bool,
    not_reset_seen: bool,
    reconciled: bool,
}
impl NativeFold {
    fn new(kind: FoldKind, sum: OwnedHistogram) -> Self {
        let hint = sum.as_ref().reset_hint;
        Self {
            sum,
            compensation: None,
            count: 1,
            average: kind == FoldKind::Avg,
            incremental: false,
            reset_seen: hint == COUNTER_RESET,
            not_reset_seen: hint == NOT_RESET,
            reconciled: false,
        }
    }
    fn add(
        &mut self,
        value: FloatHistogramRef<'_>,
        account: &QueryAccounting,
    ) -> Result<(), HistogramError> {
        // The source tracks original hints even after the accumulator becomes unknown.
        self.reset_seen |= value.reset_hint == COUNTER_RESET;
        self.not_reset_seen |= value.reset_hint == NOT_RESET;
        if !self.average {
            self.reconciled |= self
                .sum
                .kahan_add(value, &mut self.compensation)?
                .nhcb_bounds_reconciled;
            return Ok(());
        }
        self.count += 1;
        let count = self.count as f64;
        if !self.incremental {
            let mut trial_sum = self.sum.copy();
            let mut trial_compensation = self.compensation.as_ref().map(OwnedHistogram::copy);
            self.reconciled |= trial_sum
                .kahan_add(value, &mut trial_compensation)?
                .nhcb_bounds_reconciled;
            if !trial_sum.has_overflow() {
                self.sum = trial_sum;
                self.compensation = trial_compensation;
                return Ok(());
            }
            drop(trial_sum);
            drop(trial_compensation);
            self.incremental = true;
            let mut mean = self.sum.copy();
            mean.div(count - 1.0);
            self.sum = mean;
            if let Some(compensation) = &mut self.compensation {
                compensation.div(count - 1.0);
            }
        }
        let q = (count - 1.0) / count;
        if let Some(compensation) = &mut self.compensation {
            compensation.mul(q);
        }
        let mut to_add = retain_histogram(SampleValueRef::FloatHistogram(value), account);
        to_add.div(count);
        self.sum.mul(q);
        self.reconciled |= self
            .sum
            .kahan_add(to_add.as_ref(), &mut self.compensation)?
            .nhcb_bounds_reconciled;
        Ok(())
    }
    fn finish(mut self) -> Result<FoldResult, HistogramError> {
        if self.average && !self.incremental {
            self.sum.div(self.count as f64);
            if let Some(compensation) = &mut self.compensation {
                compensation.div(self.count as f64);
            }
        }
        if let Some(compensation) = &self.compensation {
            let flags = self.sum.add(compensation.as_ref())?;
            // Average's final correction intentionally does not contribute annotations.
            if !self.average {
                self.reconciled |= flags.nhcb_bounds_reconciled;
            }
        }
        Ok(FoldResult {
            value: Some(OwnedValue::Histogram(self.sum)),
            annotations: [
                (self.reset_seen && self.not_reset_seen).then_some(
                    AnnotationKind::HistogramCounterResetCollision(HistogramOperation::Aggregation),
                ),
                self.reconciled
                    .then_some(AnnotationKind::MismatchedCustomBuckets(
                        HistogramOperation::Aggregation,
                    )),
            ],
        })
    }
}

fn retain_histogram(value: SampleValueRef<'_>, account: &QueryAccounting) -> OwnedHistogram {
    let OwnedValue::Histogram(value) = OwnedValue::copy_from(value, account, MemoryKind::Workspace)
    else {
        unreachable!("classified histogram window")
    };
    value
}

fn fold_histograms<S: Series>(
    series: &S,
    bounds: TimeBounds,
    kind: FoldKind,
    control: &Control,
    account: &QueryAccounting,
) -> Result<FoldResult, StorageError> {
    let mut state = None::<NativeFold>;
    let mut error = None;
    scan(series, bounds, control, account, |sample| {
        if sample.value.is_stale() || error.is_some() {
            return Ok(());
        }
        let Some(state) = &mut state else {
            state = Some(NativeFold::new(
                kind,
                retain_histogram(sample.value, account),
            ));
            return Ok(());
        };
        let converted;
        let value = match sample.value {
            SampleValueRef::FloatHistogram(value) => value,
            SampleValueRef::IntegerHistogram(_) => {
                converted = retain_histogram(sample.value, account);
                converted.as_ref()
            }
            SampleValueRef::Float(_) => unreachable!("classified histogram window"),
        };
        error = state.add(value, account).err();
        Ok(())
    })?;
    let result = match error {
        Some(error) => Err(error),
        None => state.expect("nonempty histogram window").finish(),
    };
    Ok(match result {
        Ok(result) => result,
        Err(HistogramError::IncompatibleSchema) => FoldResult {
            value: None,
            annotations: [Some(AnnotationKind::MixedExponentialCustomHistograms), None],
        },
        Err(error) => panic!("{error}"),
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
    fn copy(sample: SampleRef<'_>, use_start: bool, account: &QueryAccounting) -> Self {
        Self {
            value: OwnedValue::copy_from(sample.value, account, MemoryKind::Workspace),
            time_ms: sample.time_ms,
            start_time_ms: if use_start { sample.start_time_ms } else { 0 },
        }
    }
}

pub(crate) fn fold<S: Series>(
    series: &S,
    bounds: TimeBounds,
    kind: FoldKind,
    start_timestamps: bool,
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
    scan(series, bounds, control, account, |sample| {
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
            selected = Some(OwnedValue::copy_from(
                sample.value,
                account,
                MemoryKind::Workspace,
            ));
        }
        if kind.previous() {
            let current = Previous::copy(sample, kind == Resets && start_timestamps, account);
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
                            !value.equals(old)
                        } else {
                            start_timestamp_reset(
                                previous.start_time_ms,
                                previous.time_ms,
                                current.start_time_ms,
                                current.time_ms,
                            ) || value.detect_reset(old)
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
    })?;
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
        return fold_histograms(series, bounds, kind, control, account);
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
