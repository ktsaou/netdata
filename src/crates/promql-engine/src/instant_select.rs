// SPDX-License-Identifier: GPL-3.0-or-later
// Prefix replay derives from pinned MemoizedSeriesIterator and vectorSelectorSingle; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    storage::{
        Control, CursorAction, Interrupted, Series, StorageError, TimeBounds, WalkStart, walk,
    },
    value::{SampleRef, SampleValueRef},
};

#[derive(Clone, Copy)]
pub(crate) struct InstantWindow {
    pub(crate) first_lookup_ms: i64,
    pub(crate) lookup_ms: i64,
    pub(crate) step_ms: i64,
    pub(crate) lookback_ms: i64,
    pub(crate) delta_ms: i64,
    pub(crate) data_bounds: TimeBounds,
}
impl InstantWindow {
    #[cfg(test)]
    pub(crate) fn single(lookup_ms: i64, lookback_ms: i64, data_bounds: TimeBounds) -> Self {
        Self {
            first_lookup_ms: lookup_ms,
            lookup_ms,
            step_ms: 0,
            lookback_ms,
            delta_ms: lookback_ms,
            data_bounds,
        }
    }
    fn next_lookup(self, current: i64) -> Result<i64, StorageError> {
        current
            .checked_add(self.step_ms)
            .filter(|next| *next > current && *next <= self.lookup_ms)
            .ok_or(StorageError::Invalid("invalid instant replay schedule"))
    }
}

type Selected<T> = Option<(i64, i64, T)>;

pub(crate) trait Projector {
    type Value;
    type Error: From<StorageError> + From<Interrupted>;
    fn moved(&mut self) {}
    fn clear(&mut self) {}
    fn project(
        &mut self,
        value: SampleValueRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self::Value, Self::Error>;
}
struct Plain<F>(F);
impl<T, E, F> Projector for Plain<F>
where
    E: From<StorageError> + From<Interrupted>,
    F: FnMut(SampleValueRef<'_>) -> Result<T, E>,
{
    type Value = T;
    type Error = E;
    fn project(
        &mut self,
        value: SampleValueRef<'_>,
        _: &Control,
        _: &QueryAccounting,
    ) -> Result<T, E> {
        self.0(value)
    }
}
struct Point<T> {
    time_ms: i64,
    start_time_ms: i64,
    stale: bool,
    value: T,
}
impl<T> Point<T> {
    fn valid(self, lookup_ms: i64, lookback_ms: i64) -> Option<Self> {
        (self.time_ms != i64::MIN
            && self.time_ms > lookup_ms.wrapping_sub(lookback_ms)
            && !self.stale)
            .then_some(self)
    }
    fn tuple(self) -> (i64, i64, T) {
        (self.time_ms, self.start_time_ms, self.value)
    }
}
fn access<P: Projector>(
    projector: &mut P,
    sample: SampleRef<'_>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Point<P::Value>, P::Error> {
    Ok(Point {
        time_ms: sample.time_ms,
        start_time_ms: sample.start_time_ms,
        stale: sample.value.is_stale(),
        value: projector.project(sample.value, control, account)?,
    })
}

pub(crate) fn select<S: Series, T, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    project: impl FnMut(SampleValueRef<'_>) -> Result<T, E>,
) -> Result<Selected<T>, E> {
    select_with(series, window, control, account, Plain(project))
}

pub(crate) fn select_with<S: Series, P: Projector>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    projector: P,
) -> Result<Selected<P::Value>, P::Error> {
    let mut position_ms = window.first_lookup_ms;
    select_scheduled(series, window, control, account, projector, || {
        if position_ms == window.lookup_ms {
            Ok(None)
        } else {
            position_ms = window.next_lookup(position_ms)?;
            Ok(Some(position_ms))
        }
    })
}

pub(crate) fn select_mapped<S: Series, T, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    project: impl FnMut(SampleValueRef<'_>) -> Result<T, E>,
    next_lookup: impl FnMut() -> Result<Option<i64>, StorageError>,
) -> Result<Selected<T>, E> {
    select_scheduled(
        series,
        window,
        control,
        account,
        Plain(project),
        next_lookup,
    )
}

pub(crate) fn select_scheduled<S: Series, P: Projector>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    mut projector: P,
    mut next_lookup: impl FnMut() -> Result<Option<i64>, StorageError>,
) -> Result<Selected<P::Value>, P::Error> {
    let mut lookup_ms = window.first_lookup_ms;
    let mut last_time = i64::MIN;
    let mut previous = None::<Point<P::Value>>;
    let mut selected = None;
    let mut initial = true;
    let outcome = walk::<_, P::Error>(
        series,
        window.data_bounds,
        WalkStart::Next,
        control,
        account,
        |sample| {
            projector.moved();
            if !initial {
                last_time = sample.time_ms;
            }
            initial = false;
            loop {
                control.check()?;
                account.work(1);
                let lower = lookup_ms.wrapping_sub(window.delta_ms);
                if lower > last_time {
                    previous = None;
                    if lower > sample.time_ms {
                        projector.clear();
                        return Ok(CursorAction::Seek(lower));
                    }
                    last_time = sample.time_ms;
                }
                if last_time < lookup_ms {
                    // Release the old buffer before projecting its replacement.
                    previous = None;
                    previous = Some(access(&mut projector, sample, control, account)?);
                    return Ok(CursorAction::Next);
                }
                // Future access can mutate statistics state even when the predecessor wins.
                let current = access(&mut projector, sample, control, account)?;
                let Some(next) = next_lookup()? else {
                    selected = if current.time_ms > lookup_ms {
                        previous
                            .take()
                            .and_then(|point| point.valid(lookup_ms, window.lookback_ms))
                    } else {
                        (!current.stale).then_some(current)
                    };
                    return Ok(CursorAction::Stop);
                };
                drop(current);
                lookup_ms = next;
            }
        },
    )?;
    if outcome.is_continue() {
        // Successful EOF preserves the memoized predecessor; errors never reach fallback.
        loop {
            control.check()?;
            account.work(1);
            let Some(next) = next_lookup()? else {
                selected = previous
                    .take()
                    .and_then(|point| point.valid(lookup_ms, window.lookback_ms));
                break;
            };
            lookup_ms = next;
        }
    }
    Ok(selected.map(Point::tuple))
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
            InstantWindow::single(lookup_ms, lookback_ms, data_bounds),
            control,
            account,
            |value| {
                Ok(OwnedValue::copy_from_checked(
                    value,
                    control,
                    account,
                    MemoryKind::Workspace,
                )?)
            },
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
    fn repeated_lookup_endpoints_still_observe_each_logical_access_epoch() {
        fn sample(index: usize) -> Sample {
            Sample {
                time_ms: index as i64 * 1000,
                start_time_ms: 0,
                value: SampleValue::Float(index as f64),
            }
        }
        for page in [1, 17, 1024] {
            let series = GeneratedSeries::new(2, page, sample);
            let account = EngineAccounting::default().query();
            let control = Control::default();
            let mut accesses = Vec::new();
            let mut schedule = [Some(500), Some(500), None].into_iter();
            let selected = select_mapped::<_, _, StorageError>(
                &series,
                InstantWindow::single(500, 1000, bounds(1000)),
                &control,
                &account,
                |value| {
                    let SampleValueRef::Float(value) = value else {
                        unreachable!()
                    };
                    accesses.push(value);
                    Ok(value)
                },
                || Ok(schedule.next().expect("each grid step is consumed once")),
            )
            .unwrap();
            assert_eq!(selected, Some((0, 0, 0.0)));
            assert_eq!(accesses, [0.0, 1.0, 1.0, 1.0]);
            assert!(schedule.next().is_none());
            assert_closed(&series, &account);
        }
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
                    InstantWindow::single(lookup, 10_000, bounds(5000)),
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
    fn replacing_owned_histograms_retains_a_constant_number_of_projections() {
        for page_size in [1, 17, 1024] {
            let account = EngineAccounting::default().query();
            let series = GeneratedSeries::new(100, page_size, histogram);
            let mut retained_size = 0;
            let control = Control::default();
            let selected = select::<_, _, StorageError>(
                &series,
                InstantWindow::single(99, 100, bounds(99)),
                &control,
                &account,
                |value| {
                    let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
                    assert!(before <= retained_size);
                    let value = OwnedValue::copy_from_checked(
                        value,
                        &control,
                        &account,
                        MemoryKind::Workspace,
                    )?;
                    retained_size =
                        account.snapshot().memory[MemoryKind::Workspace as usize].current - before;
                    Ok(value)
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

    #[test]
    fn exact_endpoint_does_not_read_an_armed_terminal_failure() {
        for page_size in [1, 17, 1024] {
            let account = EngineAccounting::default().query();
            let mut series = GeneratedSeries::new(1, page_size, counter);
            series.fail_page = Some(1);
            let selected = owned(
                &series,
                1000,
                1000,
                bounds(2000),
                &Control::default(),
                &account,
            )
            .unwrap()
            .unwrap();
            let OwnedValue::Float(value) = selected.2 else {
                panic!("expected the exact endpoint float")
            };
            assert_eq!((selected.0, selected.1, value), (1000, 0, 1.0));
            assert_eq!(series.counters.next_calls.load(Ordering::Relaxed), 1);
            assert_closed(&series, &account);
        }
    }

    #[test]
    fn required_future_and_eof_failures_precede_predecessor_fallback() {
        for count in [1, 2] {
            let account = EngineAccounting::default().query();
            let mut series = GeneratedSeries::new(count, 1, counter);
            series.fail_page = Some(1);
            let result = owned(
                &series,
                1500,
                2000,
                bounds(2000),
                &Control::default(),
                &account,
            );
            let Err(StorageError::Provider(bytes)) = result else {
                panic!("a required raw read error must precede the eligible predecessor")
            };
            assert_eq!(&*bytes, b"injected page failure");
            assert_eq!(series.counters.next_calls.load(Ordering::Relaxed), 2);
            drop(bytes);
            assert_closed(&series, &account);
        }
    }

    #[test]
    fn cancellation_inside_one_large_histogram_projection_releases_every_owner() {
        fn large_histogram(_: usize) -> Sample {
            Sample {
                time_ms: 0,
                start_time_ms: 0,
                value: SampleValue::FloatHistogram(FloatHistogram {
                    positive_buckets: Some(vec![1.0; 100_000].into()),
                    ..FloatHistogram::default()
                }),
            }
        }
        let account = EngineAccounting::default().query();
        let control = Control::default();
        let series = GeneratedSeries::new(1, 1, large_histogram);
        let result = interrupt_during(&account, &control, || {
            owned(&series, 0, 1000, bounds(1000), &control, &account)
        });
        assert!(matches!(
            result,
            Err(StorageError::Interrupted(Interrupted::Cancelled))
        ));
        let work = account.snapshot().work;
        assert!((8192..100_000).contains(&work));
        assert_eq!(series.counters.next_calls.load(Ordering::Relaxed), 1);
        drop(result);
        assert_closed(&series, &account);
    }
}
