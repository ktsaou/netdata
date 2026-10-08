// SPDX-License-Identifier: GPL-3.0-or-later
// Statistics decoding and access epochs derive from pinned PromQL/storage; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    histogram_stats::{Projection, StatisticsDecoder},
    instant_select::{self, InstantWindow, Projector},
    result::OwnedValue,
    storage::{Control, Interrupted, Series, StorageError},
    value::SampleValueRef,
};

impl Projector for StatisticsDecoder {
    type Value = Projection;
    type Error = StorageError;
    fn moved(&mut self) {
        StatisticsDecoder::moved(self);
    }
    fn clear(&mut self) {
        StatisticsDecoder::clear(self);
    }
    fn project(
        &mut self,
        value: SampleValueRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Projection, StorageError> {
        StatisticsDecoder::project(self, value, control, account)
    }
}

struct VisitorDecoder<E>(StatisticsDecoder, std::marker::PhantomData<E>);
impl<E: From<StorageError> + From<Interrupted>> Projector for VisitorDecoder<E> {
    type Value = Projection;
    type Error = E;
    fn moved(&mut self) {
        self.0.moved();
    }
    fn clear(&mut self) {
        self.0.clear();
    }
    fn project(
        &mut self,
        value: SampleValueRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Projection, E> {
        self.0.project(value, control, account).map_err(Into::into)
    }
}

pub(crate) fn visit_scheduled<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    next_lookup: impl FnMut() -> Result<Option<i64>, StorageError>,
    mut visit: impl FnMut(i64, Option<(i64, i64, SampleValueRef<'_>)>) -> Result<bool, E>,
) -> Result<(), E> {
    instant_select::visit_scheduled(
        series,
        window,
        control,
        account,
        VisitorDecoder(StatisticsDecoder::default(), std::marker::PhantomData::<E>),
        next_lookup,
        |lookup, value| {
            visit(
                lookup,
                value.map(|(time, start, value)| (time, start, value.as_ref())),
            )
        },
    )
}

#[cfg(test)]
pub(crate) fn select<S: Series>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<(i64, i64, OwnedValue)>, StorageError> {
    owned(
        instant_select::select_with(
            series,
            window,
            control,
            account,
            StatisticsDecoder::default(),
        )?,
        control,
        account,
    )
}

pub(crate) fn select_scheduled<S: Series>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    next_lookup: impl FnMut() -> Result<Option<i64>, StorageError>,
) -> Result<Option<(i64, i64, OwnedValue)>, StorageError> {
    owned(
        instant_select::select_scheduled(
            series,
            window,
            control,
            account,
            StatisticsDecoder::default(),
            next_lookup,
        )?,
        control,
        account,
    )
}

fn owned(
    selected: Option<(i64, i64, Projection)>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<(i64, i64, OwnedValue)>, StorageError> {
    selected
        .map(|(time, start, value)| {
            Ok((
                time,
                start,
                OwnedValue::copy_from_checked(
                    value.as_ref(),
                    control,
                    account,
                    MemoryKind::Workspace,
                )?,
            ))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{COUNTER_RESET, FloatHistogram, NOT_RESET},
        storage::TimeBounds,
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
        account.snapshot().assert_memory_released();
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
