// SPDX-License-Identifier: GPL-3.0-or-later
// Prefix replay derives from pinned MemoizedSeriesIterator and vectorSelectorSingle; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    histogram_stats::{Projection, StatisticsDecoder},
    result::OwnedValue,
    storage::{Control, Series, StorageError, TimeBounds, scan_until},
    value::SampleRef,
};
use std::ops::ControlFlow;

#[derive(Clone, Copy)]
pub(crate) struct InstantWindow {
    pub(crate) first_lookup_ms: i64,
    pub(crate) lookup_ms: i64,
    pub(crate) step_ms: i64,
    pub(crate) lookback_ms: i64,
    pub(crate) delta_ms: i64,
    pub(crate) data_bounds: TimeBounds,
}

#[derive(Clone, Copy)]
struct Point {
    time_ms: i64,
    start_time_ms: i64,
    value: Projection,
}
impl Point {
    fn valid(self, lookup_ms: i64, lookback_ms: i64) -> Option<Self> {
        (self.time_ms > lookup_ms.wrapping_sub(lookback_ms) && !self.value.as_ref().is_stale())
            .then_some(self)
    }
}

pub(crate) fn select<S: Series>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<(i64, i64, OwnedValue)>, StorageError> {
    let bounds = TimeBounds {
        start_ms: window
            .data_bounds
            .start_ms
            .max(window.first_lookup_ms.wrapping_sub(window.delta_ms)),
        end_ms: window.data_bounds.end_ms,
    };
    let mut decoder = StatisticsDecoder::default();
    let mut lookup_ms = window.first_lookup_ms;
    let mut last_time = i64::MIN;
    let mut previous = None::<Point>;
    let mut selected = None::<Point>;
    let mut advanced = false;
    let mut seek_target = None;
    let outcome = scan_until::<_, StorageError>(
        series,
        bounds,
        control,
        account,
        |sample: SampleRef<'_>| {
            decoder.moved();
            if let Some(target) = seek_target {
                if sample.time_ms < target {
                    return Ok(ControlFlow::Continue(()));
                }
                seek_target = None;
                last_time = sample.time_ms;
            } else if advanced {
                last_time = sample.time_ms;
                advanced = false;
            }
            loop {
                control.check()?;
                account.work(1);
                let lower = lookup_ms.wrapping_sub(window.delta_ms);
                if lower > last_time {
                    previous = None;
                    // Raw Seek clears statistics only if it moves beyond the current raw sample.
                    if lower > sample.time_ms {
                        decoder.clear();
                        seek_target = Some(lower);
                        return Ok(ControlFlow::Continue(()));
                    }
                    last_time = sample.time_ms;
                }
                if last_time < lookup_ms {
                    previous = Some(Point {
                        time_ms: sample.time_ms,
                        start_time_ms: sample.start_time_ms,
                        value: decoder.project(sample.value, control, account)?,
                    });
                    advanced = true;
                    return Ok(ControlFlow::Continue(()));
                }
                // Access future current histograms before choosing the buffered predecessor.
                let current = Point {
                    time_ms: sample.time_ms,
                    start_time_ms: sample.start_time_ms,
                    value: decoder.project(sample.value, control, account)?,
                };
                selected = if current.time_ms > lookup_ms {
                    previous.and_then(|point| point.valid(lookup_ms, window.lookback_ms))
                } else {
                    (!current.value.as_ref().is_stale()).then_some(current)
                };
                if lookup_ms == window.lookup_ms {
                    return Ok(ControlFlow::Break(()));
                }
                lookup_ms = lookup_ms
                    .checked_add(window.step_ms)
                    .filter(|next| *next > lookup_ms && *next <= window.lookup_ms)
                    .ok_or(StorageError::Invalid("invalid instant replay schedule"))?;
            }
        },
    )?;
    if outcome.is_continue() {
        // EOF keeps the memoized predecessor; later Seek calls do not clear it.
        loop {
            control.check()?;
            account.work(1);
            selected = previous.and_then(|point| point.valid(lookup_ms, window.lookback_ms));
            if lookup_ms == window.lookup_ms {
                break;
            }
            lookup_ms = lookup_ms
                .checked_add(window.step_ms)
                .filter(|next| *next > lookup_ms && *next <= window.lookup_ms)
                .ok_or(StorageError::Invalid("invalid instant replay schedule"))?;
        }
    }
    Ok(selected.map(|point| {
        (
            point.time_ms,
            point.start_time_ms,
            OwnedValue::copy_from(point.value.as_ref(), account, MemoryKind::Workspace),
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{COUNTER_RESET, FloatHistogram, NOT_RESET},
        test_storage::{GeneratedSeries, counter},
        value::{STALE_NAN_BITS, Sample, SampleValue},
    };
    use std::sync::atomic::Ordering;

    fn histogram(time_ms: i64, count: f64, hint: u8) -> Sample {
        Sample {
            time_ms,
            start_time_ms: 0,
            value: SampleValue::FloatHistogram(FloatHistogram {
                reset_hint: hint,
                count,
                sum: count,
                zero_count: count,
                zero_threshold: 0.001,
                ..FloatHistogram::default()
            }),
        }
    }
    fn window() -> InstantWindow {
        InstantWindow {
            first_lookup_ms: 1000,
            lookup_ms: 5000,
            step_ms: 4000,
            lookback_ms: 500,
            delta_ms: 500,
            data_bounds: TimeBounds {
                start_ms: 501,
                end_ms: 5000,
            },
        }
    }
    fn assert_closed(source: &GeneratedSeries, account: &QueryAccounting) {
        assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
        assert_eq!(
            source.counters.opened.load(Ordering::Relaxed),
            source.counters.closed.load(Ordering::Relaxed)
        );
        assert!(
            account
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }

    #[test]
    fn start_timestamps_follow_statistics_predecessors() {
        // Pinned vectorSelectorSingle returns the predecessor's start time after future access.
        fn sample(index: usize) -> Sample {
            Sample {
                start_time_ms: [77, 4100][index],
                ..histogram([900, 5000][index], [8.0, 4.0][index], NOT_RESET)
            }
        }
        for page in [1, 17, 1024] {
            for (count, lookup_ms, expected) in [
                (1, 1000, Some((900, 77))),
                (2, 1000, Some((900, 77))),
                (2, 1500, Some((900, 77))),
                (2, 1900, None),
                (2, 5000, Some((5000, 4100))),
            ] {
                let source = GeneratedSeries::new(count, page, sample);
                let account = EngineAccounting::default().query();
                let result = select(
                    &source,
                    InstantWindow {
                        lookup_ms,
                        step_ms: lookup_ms - 1000,
                        lookback_ms: 1000,
                        delta_ms: 999,
                        data_bounds: TimeBounds {
                            start_ms: 1,
                            end_ms: 5000,
                        },
                        ..window()
                    },
                    &Control::default(),
                    &account,
                )
                .unwrap();
                assert_eq!(result.as_ref().map(|point| (point.0, point.1)), expected);
                drop(result);
                assert_closed(&source, &account);
            }
        }
    }

    #[test]
    fn future_access_cache_and_moving_seek_match_source_hints() {
        fn lookahead(index: usize) -> Sample {
            histogram([900, 5000][index], [8.0, 4.0][index], [NOT_RESET, 0][index])
        }
        fn endpoint(index: usize) -> Sample {
            histogram(
                [1000, 5000][index],
                [8.0, 4.0][index],
                [NOT_RESET, 0][index],
            )
        }
        fn moving(index: usize) -> Sample {
            histogram(
                [900, 2000, 5000][index],
                [8.0, 7.0, 4.0][index],
                [NOT_RESET, 0, 0][index],
            )
        }
        for page in [1, 17, 1024] {
            for (count, sample, expected) in [
                (2, lookahead as fn(usize) -> Sample, COUNTER_RESET),
                (2, endpoint, 0),
                (3, moving, 0),
            ] {
                let source = GeneratedSeries::new(count, page, sample);
                let account = EngineAccounting::default().query();
                let result = select(&source, window(), &Control::default(), &account)
                    .unwrap()
                    .unwrap();
                assert_eq!(result.0, 5000);
                let OwnedValue::Histogram(histogram) = &result.2 else {
                    panic!("histogram output")
                };
                let histogram = histogram.as_ref();
                assert_eq!(histogram.reset_hint, expected);
                assert_eq!(histogram.count, 4.0);
                assert_eq!(histogram.zero_count.to_bits(), 0);
                assert_eq!(histogram.zero_threshold.to_bits(), 0);
                assert!(histogram.positive_buckets.is_none());
                drop(result);
                assert_closed(&source, &account);
            }
        }
    }

    #[test]
    fn float_and_stale_gaps_preserve_last_full_histogram() {
        fn sample(index: usize) -> Sample {
            match index {
                0 => histogram(900, 8.0, NOT_RESET),
                1 => Sample {
                    time_ms: 2000,
                    start_time_ms: 0,
                    value: SampleValue::Float(100.0),
                },
                2 => {
                    let mut sample = histogram(3000, 0.0, 0);
                    let SampleValue::FloatHistogram(h) = &mut sample.value else {
                        unreachable!()
                    };
                    h.sum = f64::from_bits(STALE_NAN_BITS);
                    sample
                }
                _ => histogram(5000, 4.0, 0),
            }
        }
        for page in [1, 17, 1024] {
            let source = GeneratedSeries::new(4, page, sample);
            let account = EngineAccounting::default().query();
            let result = select(
                &source,
                InstantWindow {
                    lookback_ms: 5000,
                    delta_ms: 5000,
                    data_bounds: TimeBounds {
                        start_ms: -3999,
                        end_ms: 5000,
                    },
                    ..window()
                },
                &Control::default(),
                &account,
            )
            .unwrap()
            .unwrap();
            let OwnedValue::Histogram(h) = &result.2 else {
                panic!("histogram output")
            };
            assert_eq!(h.as_ref().reset_hint, COUNTER_RESET);
            drop(result);
            assert_closed(&source, &account);
        }
    }

    #[test]
    fn eof_preserves_only_an_eligible_predecessor() {
        fn sample(_: usize) -> Sample {
            histogram(1000, 8.0, NOT_RESET)
        }
        for page in [1, 17, 1024] {
            for (lookup_ms, exists) in [(1000, true), (1499, true), (1500, false), (5000, false)] {
                let source = GeneratedSeries::new(1, page, sample);
                let account = EngineAccounting::default().query();
                let result = select(
                    &source,
                    InstantWindow {
                        lookup_ms,
                        step_ms: lookup_ms - 1000,
                        ..window()
                    },
                    &Control::default(),
                    &account,
                )
                .unwrap();
                assert_eq!(result.is_some(), exists);
                drop(result);
                assert_closed(&source, &account);
            }
        }
    }

    #[test]
    fn early_stop_observes_tail_without_opening_more_pages() {
        for page in [1, 17, 1024] {
            let mut source = GeneratedSeries::new(100_000, page, counter);
            source.fail_page = Some(1);
            let account = EngineAccounting::default().query();
            let result = select(
                &source,
                InstantWindow {
                    first_lookup_ms: 1000,
                    lookup_ms: 1000,
                    data_bounds: TimeBounds {
                        start_ms: 501,
                        end_ms: 100_000_000,
                    },
                    ..window()
                },
                &Control::default(),
                &account,
            )
            .unwrap()
            .unwrap();
            assert_eq!(result.0, 1000);
            assert_eq!(source.counters.next_calls.load(Ordering::Relaxed), 1);
            assert_eq!(account.snapshot().samples_fetched, page as u64);
            drop(result);
            assert_closed(&source, &account);
        }
    }

    #[test]
    fn long_history_has_constant_workspace_and_releases_failure_owners() {
        for page in [1, 17, 1024] {
            let mut peaks = Vec::new();
            for count in [1024, 100_000] {
                let source = GeneratedSeries::new(count, page, counter);
                let account = EngineAccounting::default().query();
                let result = select(
                    &source,
                    InstantWindow {
                        first_lookup_ms: 1000,
                        lookup_ms: count as i64 * 1000,
                        step_ms: 1000,
                        lookback_ms: 5000,
                        delta_ms: 5000,
                        data_bounds: TimeBounds {
                            start_ms: -3999,
                            end_ms: count as i64 * 1000,
                        },
                    },
                    &Control::default(),
                    &account,
                )
                .unwrap()
                .unwrap();
                assert_eq!(result.0, count as i64 * 1000);
                drop(result);
                assert_closed(&source, &account);
                peaks.push(account.snapshot().memory[MemoryKind::Workspace as usize].peak);
                assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
            }
            assert_eq!(peaks[0], peaks[1]);
            let mut source = GeneratedSeries::new(100_000, page, counter);
            source.fail_page = Some(1);
            let account = EngineAccounting::default().query();
            assert!(matches!(
                select(
                    &source,
                    InstantWindow {
                        lookup_ms: 100_000_000,
                        step_ms: 1000,
                        data_bounds: TimeBounds {
                            start_ms: 501,
                            end_ms: 100_000_000
                        },
                        ..window()
                    },
                    &Control::default(),
                    &account
                ),
                Err(StorageError::Provider(_))
            ));
            assert_closed(&source, &account);
        }
    }
}
