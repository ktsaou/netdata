// SPDX-License-Identifier: GPL-3.0-or-later
//! Paged instantaneous selection with one retained caller-owned projection.
use crate::{
    accounting::QueryAccounting,
    storage::{Control, Interrupted, Series, StorageError, TimeBounds, scan_with},
    value::SampleValueRef,
};

pub(crate) fn select<S: Series, T, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    lookup_ms: i64,
    lookback_ms: i64,
    data_bounds: TimeBounds,
    control: &Control,
    account: &QueryAccounting,
    mut project: impl FnMut(SampleValueRef<'_>) -> Result<T, E>,
) -> Result<Option<(i64, i64, T)>, E> {
    let bounds = TimeBounds {
        start_ms: lookup_ms
            .wrapping_sub(lookback_ms)
            .wrapping_add(1)
            .max(data_bounds.start_ms),
        end_ms: lookup_ms.min(data_bounds.end_ms),
    };
    let mut selected = None;
    if bounds.start_ms <= bounds.end_ms {
        scan_with::<_, E>(series, bounds, control, account, |sample| {
            // Free the prior payload before constructing its replacement.
            selected = None;
            if !sample.value.is_stale() {
                selected = Some((sample.time_ms, sample.start_time_ms, project(sample.value)?));
            }
            Ok(())
        })?;
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        histogram::FloatHistogram,
        result::OwnedValue,
        storage::Interrupted,
        test_storage::{GeneratedSeries, counter, interrupt_during},
        value::{STALE_NAN_BITS, Sample, SampleValue},
    };
    use std::sync::atomic::Ordering;

    fn bounds(end_ms: i64) -> TimeBounds {
        TimeBounds {
            start_ms: 0,
            end_ms,
        }
    }
    fn owned(
        series: &GeneratedSeries,
        lookup_ms: i64,
        lookback_ms: i64,
        data_bounds: TimeBounds,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<(i64, i64, OwnedValue)>, StorageError> {
        select(
            series,
            lookup_ms,
            lookback_ms,
            data_bounds,
            control,
            account,
            |value| Ok(OwnedValue::copy_from(value, account, MemoryKind::Workspace)),
        )
    }
    fn assert_closed(series: &GeneratedSeries, account: &QueryAccounting) {
        assert_eq!(series.counters.active.load(Ordering::Relaxed), 0);
        assert_eq!(
            series.counters.opened.load(Ordering::Relaxed),
            series.counters.closed.load(Ordering::Relaxed)
        );
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn exclusive_lookback_and_available_bounds_do_not_select_future_points() {
        for page_size in [1, 17, 1024] {
            let account = EngineAccounting::default().query();
            let series = GeneratedSeries::new(20, page_size, counter);
            let control = Control::default();
            for (lookup, lookback, available, expected) in [
                (1500, 500, bounds(20_000), None),
                (1500, 501, bounds(20_000), Some((1000, 1.0))),
                (2000, 1, bounds(20_000), Some((2000, 2.0))),
                (20_000, 20_000, bounds(2999), Some((2000, 2.0))),
                (20_000, 10_000, bounds(2999), None),
            ] {
                let actual = owned(&series, lookup, lookback, available, &control, &account)
                    .unwrap()
                    .map(|(time, _, value)| {
                        let OwnedValue::Float(value) = value else {
                            panic!("expected float")
                        };
                        (time, value)
                    });
                assert_eq!(actual, expected);
            }
            assert_closed(&series, &account);
        }
    }

    fn mixed(index: usize) -> Sample {
        let value = match index {
            0 => SampleValue::FloatHistogram(FloatHistogram::default()),
            1 => SampleValue::Float(4.0),
            2 => SampleValue::Float(f64::from_bits(STALE_NAN_BITS)),
            3 => SampleValue::FloatHistogram(FloatHistogram {
                sum: f64::from_bits(STALE_NAN_BITS),
                ..FloatHistogram::default()
            }),
            _ => SampleValue::Float(f64::from_bits(0xfff8_0000_0000_1234)),
        };
        Sample {
            time_ms: 1000 * (index as i64 + 1),
            start_time_ms: 71,
            value,
        }
    }

    #[test]
    fn stale_points_clear_prior_values_and_histograms_can_be_projected_without_copying() {
        for page_size in [1, 17, 1024] {
            let account = EngineAccounting::default().query();
            let series = GeneratedSeries::new(5, page_size, mixed);
            let control = Control::default();
            for (lookup, expected) in [
                (1000, Some((1000, 71, true))),
                (2000, Some((2000, 71, false))),
                (3000, None),
                (4000, None),
                (5000, Some((5000, 71, false))),
            ] {
                let actual = select::<_, _, StorageError>(
                    &series,
                    lookup,
                    10_000,
                    bounds(5000),
                    &control,
                    &account,
                    |value| {
                        assert_eq!(
                            account.snapshot().memory[MemoryKind::Workspace as usize].current,
                            0
                        );
                        Ok(!matches!(value, SampleValueRef::Float(_)))
                    },
                )
                .unwrap();
                assert_eq!(actual, expected);
            }
            let (_, _, value) = owned(&series, 5000, 10_000, bounds(5000), &control, &account)
                .unwrap()
                .unwrap();
            let OwnedValue::Float(value) = value else {
                panic!("expected float")
            };
            assert_eq!(value.to_bits(), 0xfff8_0000_0000_1234);
            assert_closed(&series, &account);
        }
    }

    fn histogram(index: usize) -> Sample {
        Sample {
            time_ms: index as i64,
            start_time_ms: 0,
            value: SampleValue::FloatHistogram(FloatHistogram {
                positive_buckets: Some(vec![index as f64; 64].into()),
                ..FloatHistogram::default()
            }),
        }
    }

    #[test]
    fn replacing_owned_histograms_retains_only_one_projection() {
        for page_size in [1, 17, 1024] {
            let account = EngineAccounting::default().query();
            let series = GeneratedSeries::new(100, page_size, histogram);
            let selected = select::<_, _, StorageError>(
                &series,
                99,
                100,
                bounds(99),
                &Control::default(),
                &account,
                |value| {
                    assert_eq!(
                        account.snapshot().memory[MemoryKind::Workspace as usize].current,
                        0
                    );
                    Ok(OwnedValue::copy_from(
                        value,
                        &account,
                        MemoryKind::Workspace,
                    ))
                },
            )
            .unwrap()
            .unwrap();
            assert!(matches!(selected.2, OwnedValue::Histogram(_)));
            let memory = account.snapshot().memory[MemoryKind::Workspace as usize];
            assert!(memory.current > 0);
            drop(selected);
            assert_closed(&series, &account);
        }
    }

    #[test]
    fn retained_peaks_are_independent_of_history_length() {
        for page_size in [1, 17, 1024] {
            let mut previous = None;
            for count in [2048, 100_000] {
                let account = EngineAccounting::default().query();
                let series = GeneratedSeries::new(count, page_size, counter);
                let end = count as i64 * 1000;
                let selected = owned(
                    &series,
                    end,
                    end,
                    bounds(end),
                    &Control::default(),
                    &account,
                )
                .unwrap()
                .unwrap();
                assert_eq!(selected.0, end);
                drop(selected);
                assert_eq!(series.counters.peak.load(Ordering::Relaxed), 1);
                let observations = account.snapshot();
                let peaks = [
                    MemoryKind::Workspace,
                    MemoryKind::ProviderHeld,
                    MemoryKind::BorrowedView,
                ]
                .map(|kind| observations.memory[kind as usize].peak);
                if let Some(previous) = previous {
                    assert_eq!(peaks, previous);
                }
                previous = Some(peaks);
                assert_closed(&series, &account);
            }
        }
    }

    #[test]
    fn page_failure_and_mid_read_cancellation_release_projection_and_cursor() {
        for cancelled in [false, true] {
            let account = EngineAccounting::default().query();
            let control = Control::default();
            let mut series = GeneratedSeries::new(10_000, 17, histogram);
            if !cancelled {
                series.fail_page = Some(3);
            }
            let result = interrupt_during(&account, &control, || {
                owned(&series, 10_000, 10_001, bounds(10_000), &control, &account)
            });
            if cancelled {
                assert!(matches!(
                    result,
                    Err(StorageError::Interrupted(Interrupted::Cancelled))
                ));
            } else {
                assert!(matches!(result, Err(StorageError::Provider(_))));
            }
            drop(result);
            assert!(series.counters.next_calls.load(Ordering::Relaxed) > 1);
            assert_closed(&series, &account);
        }
    }
}
