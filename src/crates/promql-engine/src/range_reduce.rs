// SPDX-License-Identifier: GPL-3.0-or-later
// Ordinary range reducers derive from pinned promql/functions.go; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    range_float::{RateKind, start_timestamp_reset},
    result::OwnedValue,
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
}

pub(crate) struct FoldResult {
    pub(crate) value: Option<OwnedValue>,
    pub(crate) ignored_histograms: bool,
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
    scan(series, bounds, control, account, |sample| {
        if sample.value.is_stale() {
            return Ok(());
        }
        count += 1;
        first_time.get_or_insert(sample.time_ms);
        if let SampleValueRef::Float(value) = sample.value {
            float_last = Some(sample.time_ms);
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
    let value = if count == 0 {
        None
    } else {
        match kind {
            Count => Some(OwnedValue::Float(count as f64)),
            Present => Some(OwnedValue::Float(1.0)),
            First | Last => selected,
            Min | Max => float.map(|(value, _)| OwnedValue::Float(value)),
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
        ignored_histograms: kind.extrema() && float_last.is_some() && histogram_last.is_some(),
    })
}
