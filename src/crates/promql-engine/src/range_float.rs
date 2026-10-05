// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus promql/functions.go:474–629,759–803; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    histogram_stats::{StatisticsWindow, scan_statistics},
    storage::{Control, Series, StorageError, TimeBounds},
    value::{SampleRef, SampleValueRef},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateKind {
    Rate,
    Increase,
    Delta,
}

#[derive(Clone, Copy, Debug)]
pub struct RateWindow {
    pub start_ms: i64,
    pub end_ms: i64,
    pub range_ns: i64,
    pub start_timestamps: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateValue {
    Empty,
    Float(f64),
    Mixed,
    Histogram,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateResult {
    pub value: RateValue,
    pub start_time_overlap: bool,
}

#[derive(Clone, Copy, Debug)]
struct Point {
    time: i64,
    start: i64,
    value: f64,
}
impl Point {
    fn from_sample(sample: SampleRef<'_>, value: f64, use_start: bool) -> Self {
        Self {
            time: sample.time_ms,
            start: if use_start { sample.start_time_ms } else { 0 },
            value,
        }
    }
}

pub(crate) fn start_timestamp_reset(
    previous_start: i64,
    previous_time: i64,
    start: i64,
    time: i64,
) -> bool {
    if previous_start == start || start == 0 || start >= time || start < previous_time {
        return false;
    }
    if start > previous_time {
        return true;
    }
    if previous_start > previous_time {
        return false;
    }
    previous_start != 0 && previous_start != previous_time
}

pub(crate) fn start_time_overlap(previous_start: i64, previous_time: i64, start: i64) -> bool {
    start != 0 && start < previous_time && start != previous_start
}

pub(crate) fn uses_first_start(window: RateWindow, kind: RateKind, time: i64, start: i64) -> bool {
    kind != RateKind::Delta && start != 0 && start > window.start_ms && start < time
}

pub(crate) struct Extrapolation {
    pub(crate) first_time: i64,
    pub(crate) last_time: i64,
    pub(crate) first_start: i64,
    pub(crate) count: usize,
    pub(crate) first_value: f64,
    pub(crate) result_value: f64,
}

pub(crate) fn extrapolation_factor(
    window: RateWindow,
    kind: RateKind,
    points: Extrapolation,
) -> Option<f64> {
    let mut duration_to_start = points.first_time.wrapping_sub(window.start_ms) as f64 / 1000.0;
    let mut duration_to_end = window.end_ms.wrapping_sub(points.last_time) as f64 / 1000.0;
    let mut sampled_interval = points.last_time.wrapping_sub(points.first_time) as f64 / 1000.0;
    let average_duration = if points.count > 1 {
        sampled_interval / (points.count - 1) as f64
    } else {
        0.0
    };
    let threshold = average_duration * 1.1;
    if uses_first_start(window, kind, points.first_time, points.first_start) {
        duration_to_start = 0.0;
        sampled_interval = points.last_time.wrapping_sub(points.first_start) as f64 / 1000.0;
    } else if points.count == 1 {
        return None;
    } else {
        if duration_to_start >= threshold {
            duration_to_start = average_duration / 2.0;
        }
        if kind != RateKind::Delta {
            let duration_to_zero = if points.result_value > 0.0 && points.first_value >= 0.0 {
                sampled_interval * (points.first_value / points.result_value)
            } else {
                duration_to_start
            };
            if duration_to_zero < duration_to_start {
                duration_to_start = duration_to_zero;
            }
        }
    }
    if duration_to_end >= threshold {
        duration_to_end = average_duration / 2.0;
    }
    let mut factor = 1.0;
    if sampled_interval != 0.0 {
        factor = (sampled_interval + duration_to_start + duration_to_end) / sampled_interval;
    }
    if kind == RateKind::Rate {
        factor /= crate::frontend::number::seconds(window.range_ns);
    }
    Some(factor)
}

pub fn rate<S: Series>(
    series: &S,
    window: RateWindow,
    kind: RateKind,
    control: &Control,
    account: &QueryAccounting,
) -> Result<RateResult, StorageError> {
    rate_with(series, window, kind, None, control, account)
}

pub(crate) fn rate_with<S: Series>(
    series: &S,
    window: RateWindow,
    kind: RateKind,
    statistics: Option<StatisticsWindow>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<RateResult, StorageError> {
    let empty = |value| RateResult {
        value,
        start_time_overlap: false,
    };
    let Some(start_ms) = window.start_ms.checked_add(1) else {
        return Ok(empty(RateValue::Empty));
    };
    let bounds = TimeBounds {
        start_ms,
        end_ms: window.end_ms,
    };
    if start_ms > window.end_ms {
        return Ok(empty(RateValue::Empty));
    }
    let mut first = None;
    let mut last = None;
    let mut count = 0usize;
    let mut histograms = false;
    scan_statistics(series, bounds, statistics, control, account, |sample| {
        if sample.value.is_stale() {
            return Ok(());
        }
        if let SampleValueRef::Float(value) = sample.value {
            let point = Point::from_sample(sample, value, window.start_timestamps);
            first.get_or_insert(point);
            last = Some(point);
            count += 1;
        } else {
            histograms = true;
        }
        Ok(())
    })?;
    if histograms {
        return Ok(empty(if count > 0 {
            RateValue::Mixed
        } else {
            RateValue::Histogram
        }));
    }
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(empty(RateValue::Empty));
    };
    let counter = kind != RateKind::Delta;
    let mut result = last.value - first.value;
    let mut overlap = false;
    if counter && count > 1 {
        let mut previous = None::<Point>;
        scan_statistics(series, bounds, statistics, control, account, |sample| {
            if sample.value.is_stale() {
                return Ok(());
            }
            let SampleValueRef::Float(value) = sample.value else {
                return Err(StorageError::Invalid(
                    "provider replay changed sample kinds",
                ));
            };
            let current = Point::from_sample(sample, value, window.start_timestamps);
            if let Some(previous) = previous {
                overlap |= start_time_overlap(previous.start, previous.time, current.start);
                if current.value < previous.value
                    || start_timestamp_reset(
                        previous.start,
                        previous.time,
                        current.start,
                        current.time,
                    )
                {
                    result += previous.value;
                }
            }
            previous = Some(current);
            Ok(())
        })?;
    }
    if uses_first_start(window, kind, first.time, first.start) {
        result += first.value;
    }
    let Some(factor) = extrapolation_factor(
        window,
        kind,
        Extrapolation {
            first_time: first.time,
            last_time: last.time,
            first_start: first.start,
            count,
            first_value: first.value,
            result_value: result,
        },
    ) else {
        return Ok(empty(RateValue::Empty));
    };
    result *= factor;
    Ok(RateResult {
        value: RateValue::Float(result),
        start_time_overlap: overlap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_range_uses_go_duration_seconds_order() {
        assert_eq!(
            crate::frontend::number::seconds(1_118_000_000).to_bits(),
            0x3ff1_e353_f7ce_d916
        );
        assert_ne!(
            (1_118_000_000f64 / 1_000_000_000.0).to_bits(),
            0x3ff1_e353_f7ce_d916
        );
    }
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        test_storage::{GeneratedSeries, counter},
        value::{Sample, SampleValue},
    };
    use std::sync::atomic::Ordering;

    fn window(end_ms: i64) -> RateWindow {
        RateWindow {
            start_ms: 0,
            end_ms,
            range_ns: end_ms * 1_000_000,
            start_timestamps: false,
        }
    }

    #[test]
    fn long_history_replays_one_page_and_one_cursor_at_a_time() {
        for page_size in [1, 17, 1024] {
            let source = GeneratedSeries::new(100_000, page_size, counter);
            let account = EngineAccounting::default().query();
            let result = rate(
                &source,
                window(100_000_000),
                RateKind::Rate,
                &Control::default(),
                &account,
            )
            .unwrap();
            assert_eq!(result.value, RateValue::Float(1.0));
            let observations = account.snapshot();
            assert_eq!(observations.samples_fetched, 200_000);
            assert_eq!(
                observations.memory[MemoryKind::ProviderHeld as usize].current,
                0
            );
            assert_eq!(
                observations.memory[MemoryKind::BorrowedView as usize].current,
                0
            );
            assert_eq!(
                observations.memory[MemoryKind::BorrowedView as usize].peak,
                (page_size * size_of::<Sample>()) as u64
            );
            assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.opened.load(Ordering::Relaxed), 2);
            assert_eq!(source.counters.closed.load(Ordering::Relaxed), 2);
        }
    }

    #[test]
    fn reset_additions_replay_directly_in_source_order() {
        fn sample(index: usize) -> Sample {
            Sample {
                time_ms: (index as i64 + 1) * 1000,
                start_time_ms: 0,
                value: SampleValue::Float([18_014_398_509_481_984.0, 0.0, 1.0, 0.0][index]),
            }
        }
        for page_size in [1, 2, 1024] {
            let source = GeneratedSeries::new(4, page_size, sample);
            let account = EngineAccounting::default().query();
            let result = rate(
                &source,
                window(4000),
                RateKind::Increase,
                &Control::default(),
                &account,
            )
            .unwrap();
            assert_eq!(result.value, RateValue::Float(4.0 / 3.0));
        }
    }

    #[test]
    fn singleton_start_timestamp_is_feature_controlled() {
        fn sample(_: usize) -> Sample {
            Sample {
                time_ms: 5000,
                start_time_ms: 3000,
                value: SampleValue::Float(7.0),
            }
        }
        let source = GeneratedSeries::new(1, 1, sample);
        let account = EngineAccounting::default().query();
        let mut window = window(10_000);
        assert_eq!(
            rate(
                &source,
                window,
                RateKind::Rate,
                &Control::default(),
                &account
            )
            .unwrap()
            .value,
            RateValue::Empty
        );
        window.start_timestamps = true;
        assert_eq!(
            rate(
                &source,
                window,
                RateKind::Rate,
                &Control::default(),
                &account
            )
            .unwrap()
            .value,
            RateValue::Float(7.0 * (1.0 / 10.0))
        );
    }

    #[test]
    fn provider_errors_and_cancellation_release_cursor_and_page() {
        let mut source = GeneratedSeries::new(100, 2, counter);
        source.fail_page = Some(2);
        let account = EngineAccounting::default().query();
        assert_eq!(
            rate(
                &source,
                window(100_000),
                RateKind::Rate,
                &Control::default(),
                &account
            ),
            Err(StorageError::Provider(b"injected page failure".to_vec()))
        );
        assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
        let control = Control::default();
        control.cancellation.cancel();
        assert!(matches!(
            rate(&source, window(100_000), RateKind::Rate, &control, &account),
            Err(StorageError::Interrupted(_))
        ));
        assert_eq!(
            source.counters.opened.load(Ordering::Relaxed),
            source.counters.closed.load(Ordering::Relaxed)
        );
        assert!(
            account
                .snapshot()
                .memory
                .iter()
                .all(|kind| kind.current == 0)
        );
    }
}
