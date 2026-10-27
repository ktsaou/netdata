// SPDX-License-Identifier: GPL-3.0-or-later
// Prefix replay derives from pinned MemoizedSeriesIterator and vectorSelectorSingle; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    storage::{
        Control, CursorAction, Interrupted, PageSamples, Series, StopPageObserver, StorageError,
        TimeBounds, WalkStart, walk_scheduled_pages,
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
    pub(crate) unit_weight: bool,
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
            unit_weight: false,
        }
    }
    pub(crate) fn bounds_after(self, previous_lookup_ms: i64) -> Option<TimeBounds> {
        if self.first_lookup_ms != self.lookup_ms
            || self.lookback_ms <= 0
            || self.delta_ms < 0
            || previous_lookup_ms >= self.lookup_ms
        {
            return None;
        }
        previous_lookup_ms.checked_sub(self.lookback_ms)?;
        previous_lookup_ms.checked_sub(self.delta_ms)?;
        self.lookup_ms.checked_sub(self.lookback_ms)?;
        self.lookup_ms.checked_sub(self.delta_ms)?;
        let start_ms = self
            .data_bounds
            .start_ms
            .max(previous_lookup_ms.checked_add(1)?);
        (start_ms <= self.data_bounds.end_ms).then_some(TimeBounds {
            start_ms,
            end_ms: self.data_bounds.end_ms,
        })
    }

    fn next_lookup(self, current: i64) -> Result<i64, StorageError> {
        current
            .checked_add(self.step_ms)
            .filter(|next| *next > current && *next <= self.lookup_ms)
            .ok_or(StorageError::Invalid("invalid instant replay schedule"))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FloatSeed {
    pub(crate) lookup_ms: i64,
    pub(crate) point: Option<(i64, i64, f64)>,
}

type Selected<T> = Option<(i64, i64, T)>;

pub(crate) trait Projector {
    type Value;
    type Error: From<StorageError> + From<Interrupted>;
    fn moved(&mut self) {}
    fn clear(&mut self) {}
    fn sample_units(&self, _: &Self::Value, source: SampleValueRef<'_>) -> u64 {
        crate::statistics::weight(source)
    }
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
    units: u64,
    admitted: std::cell::Cell<bool>,
}
impl<T> Point<T> {
    fn admit(&self, account: &QueryAccounting) {
        if !self.admitted.replace(true) {
            account.samples_read(self.units);
        }
    }
    fn valid_ref(&self, lookup_ms: i64, lookback_ms: i64) -> Option<&Self> {
        (self.time_ms != i64::MIN
            && self.time_ms > lookup_ms.wrapping_sub(lookback_ms)
            && !self.stale)
            .then_some(self)
    }
    fn valid(self, lookup_ms: i64, lookback_ms: i64) -> Option<Self> {
        self.valid_ref(lookup_ms, lookback_ms)
            .is_some()
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
    unit_weight: bool,
    admitted: bool,
) -> Result<Point<P::Value>, P::Error> {
    let value = projector.project(sample.value, control, account)?;
    let units = if unit_weight {
        1
    } else {
        projector.sample_units(&value, sample.value)
    };
    Ok(Point {
        time_ms: sample.time_ms,
        start_time_ms: sample.start_time_ms,
        stale: sample.value.is_stale(),
        value,
        units,
        admitted: std::cell::Cell::new(admitted),
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

#[cfg(test)]
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

#[cfg(test)]
pub(crate) fn select_mapped_after<S: Series, T, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    window: InstantWindow,
    seed: FloatSeed,
    control: &Control,
    account: &QueryAccounting,
    project: impl FnMut(SampleValueRef<'_>) -> Result<T, E>,
) -> Result<Selected<T>, E> {
    scheduled(
        series,
        window,
        control,
        account,
        Plain(project),
        || Ok(None),
        |_, _| Ok(false),
        Some(seed),
        None,
    )
}

pub(crate) fn select_scheduled<S: Series, P: Projector>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    projector: P,
    next_lookup: impl FnMut() -> Result<Option<i64>, StorageError>,
) -> Result<Selected<P::Value>, P::Error> {
    scheduled(
        series,
        window,
        control,
        account,
        projector,
        next_lookup,
        |_, _| Ok(false),
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn select_mapped_observed<S: Series, T, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    window: InstantWindow,
    seed: Option<FloatSeed>,
    control: &Control,
    account: &QueryAccounting,
    project: impl FnMut(SampleValueRef<'_>) -> Result<T, E>,
    next_lookup: impl FnMut() -> Result<Option<i64>, StorageError>,
    observer: Option<&mut StopPageObserver<'_, E>>,
) -> Result<Selected<T>, E> {
    scheduled(
        series,
        window,
        control,
        account,
        Plain(project),
        next_lookup,
        |_, _| Ok(false),
        seed,
        observer,
    )
}

pub(crate) fn visit_future_floats(
    page: PageSamples<'_>,
    grid: crate::query_scopes::Grid,
    mut index: i64,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl FnMut(i64, i64, i64, Selected<f64>) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    if page.is_empty()
        || index < 0
        || !grid.forward_without_wrap()
        || window.lookback_ms <= 0
        || ![window.lookback_ms, window.lookback_ms - 1].contains(&window.delta_ms)
    {
        return Ok(());
    }
    let Some(mut time) = grid
        .interval_ms
        .checked_mul(index)
        .and_then(|offset| grid.start_ms.checked_add(offset))
    else {
        return Ok(());
    };
    for position in 0..page.len() {
        control.check()?;
        account.work(1);
        if !matches!(page.sample(position).value, SampleValueRef::Float(_)) {
            return Ok(());
        }
    }
    let first = page.sample(0).time_ms;
    let last = page.sample(page.len() - 1).time_ms;
    let mut lookup = window.lookup_ms;
    let mut position = 0;
    // Bound eager work even when a sparse page spans millions of requested clock steps.
    for _ in 0..page.len() {
        control.check()?;
        account.work(1);
        let (Some(next_time), Some(next_lookup), Some(next_index)) = (
            time.checked_add(grid.interval_ms),
            lookup.checked_add(grid.interval_ms),
            index.checked_add(1),
        ) else {
            break;
        };
        if next_time > grid.end_ms || next_lookup > last {
            break;
        }
        (time, lookup, index) = (next_time, next_lookup, next_index);
        if lookup < first {
            continue;
        }
        let Some(lower) = lookup.checked_sub(window.lookback_ms) else {
            break;
        };
        while position + 1 < page.len() && page.sample(position + 1).time_ms <= lookup {
            control.check()?;
            account.work(1);
            position += 1;
        }
        let sample = page.sample(position);
        let SampleValueRef::Float(value) = sample.value else {
            unreachable!("validated Float page")
        };
        let selected = (sample.time_ms > lower && !sample.value.is_stale()).then_some((
            sample.time_ms,
            sample.start_time_ms,
            value,
        ));
        visit(time, index, lookup, selected)?;
    }
    Ok(())
}

pub(crate) fn visit_scheduled<S: Series, P: Projector>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    projector: P,
    next_lookup: impl FnMut() -> Result<Option<i64>, StorageError>,
    visit: impl FnMut(i64, Option<(i64, i64, &P::Value)>) -> Result<bool, P::Error>,
) -> Result<(), P::Error> {
    scheduled(
        series,
        window,
        control,
        account,
        projector,
        next_lookup,
        visit,
        None,
        None,
    )
    .map(|_| ())
}

pub(crate) fn visit_mapped<S: Series, T, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    project: impl FnMut(SampleValueRef<'_>) -> Result<T, E>,
    next_lookup: impl FnMut() -> Result<Option<i64>, StorageError>,
    visit: impl FnMut(i64, Option<(i64, i64, &T)>) -> Result<bool, E>,
) -> Result<(), E> {
    visit_scheduled(
        series,
        window,
        control,
        account,
        Plain(project),
        next_lookup,
        visit,
    )
}

#[allow(clippy::too_many_arguments)]
fn scheduled<S: Series, P: Projector>(
    series: &S,
    window: InstantWindow,
    control: &Control,
    account: &QueryAccounting,
    mut projector: P,
    mut next_lookup: impl FnMut() -> Result<Option<i64>, StorageError>,
    mut visit: impl FnMut(i64, Option<(i64, i64, &P::Value)>) -> Result<bool, P::Error>,
    seed: Option<FloatSeed>,
    observer: Option<&mut StopPageObserver<'_, P::Error>>,
) -> Result<Selected<P::Value>, P::Error> {
    let mut lookup_ms = window.first_lookup_ms;
    let mut last_time = i64::MIN;
    let mut previous = None::<Point<P::Value>>;
    let mut selected = None;
    let mut initial = true;
    let bounds = if let Some(seed) = seed {
        control.check()?;
        let bounds = window
            .bounds_after(seed.lookup_ms)
            .ok_or(StorageError::Invalid("invalid instant continuation bounds"))?;
        // The completed prior lookup owns the predecessor; the new cursor starts after it.
        last_time = seed.lookup_ms;
        initial = false;
        if let Some((time_ms, start_time_ms, value)) = seed.point {
            previous = Some(access(
                &mut projector,
                SampleRef {
                    time_ms,
                    start_time_ms,
                    value: SampleValueRef::Float(value),
                },
                control,
                account,
                window.unit_weight,
                false,
            )?);
        }
        bounds
    } else {
        window.data_bounds
    };
    let outcome = walk_scheduled_pages::<_, P::Error>(
        series,
        bounds,
        WalkStart::Next,
        control,
        account,
        |sample| {
            let mut admitted = false;
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
                    previous = Some(access(
                        &mut projector,
                        sample,
                        control,
                        account,
                        window.unit_weight,
                        admitted,
                    )?);
                    return Ok(CursorAction::Next);
                }
                // Future access can mutate statistics state even when the predecessor wins.
                let current = access(
                    &mut projector,
                    sample,
                    control,
                    account,
                    window.unit_weight,
                    admitted,
                )?;
                let offered = if current.time_ms > lookup_ms {
                    previous
                        .as_ref()
                        .and_then(|point| point.valid_ref(lookup_ms, window.lookback_ms))
                } else {
                    (!current.stale).then_some(&current)
                };
                if let Some(point) = offered {
                    point.admit(account);
                }
                if visit(
                    lookup_ms,
                    offered.map(|point| (point.time_ms, point.start_time_ms, &point.value)),
                )? {
                    return Ok(CursorAction::Stop);
                }
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
                admitted = current.admitted.get();
                drop(current);
                lookup_ms = next;
            }
        },
        observer,
    )?;
    if outcome.is_continue() {
        // Successful EOF preserves the memoized predecessor; errors never reach fallback.
        loop {
            control.check()?;
            account.work(1);
            let offered = previous
                .as_ref()
                .and_then(|point| point.valid_ref(lookup_ms, window.lookback_ms));
            if let Some(point) = offered {
                point.admit(account);
            }
            if visit(
                lookup_ms,
                offered.map(|point| (point.time_ms, point.start_time_ms, &point.value)),
            )? {
                break;
            }
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
    fn validated_future_float_points_match_sparse_stale_and_offset_lookups() {
        fn sparse(index: usize) -> Sample {
            Sample {
                time_ms: [0, 1000, 5000, 9000][index],
                start_time_ms: -77,
                value: SampleValue::Float(match index {
                    0 => -0.0,
                    1 => f64::from_bits(STALE_NAN_BITS),
                    2 => f64::from_bits(0xfff8_0000_0000_1234),
                    _ => 9.0,
                }),
            }
        }
        let samples = (0..4).map(sparse).collect::<Vec<_>>();
        let source = GeneratedSeries::new(4, 17, sparse);
        let account = EngineAccounting::default().query();
        let control = Control::default();
        for offset in [-9000, 0, 9000] {
            for lookback in [500, 501, 1000, 5000] {
                for index in 0..18 {
                    let grid = crate::query_scopes::Grid {
                        start_ms: -offset,
                        end_ms: 9000 - offset,
                        interval_ms: 500,
                    };
                    let mut visited = 0;
                    visit_future_floats(
                        PageSamples::Samples(&samples),
                        grid,
                        index,
                        InstantWindow::single(index * 500, lookback, bounds(10000)),
                        &control,
                        &account,
                        |time, future_index, lookup, point| {
                            assert_eq!(time, lookup - offset);
                            assert_eq!(lookup, future_index * 500);
                            let expected = owned(
                                &source,
                                lookup,
                                lookback,
                                bounds(10000),
                                &control,
                                &account,
                            )?;
                            let expected = expected.map(|(time, start, value)| {
                                let OwnedValue::Float(value) = value else {
                                    panic!("Float reference")
                                };
                                (time, start, value.to_bits())
                            });
                            assert_eq!(
                                point.map(|(time, start, value)| (time, start, value.to_bits())),
                                expected
                            );
                            visited += 1;
                            Ok(())
                        },
                    )
                    .unwrap();
                    assert!(visited > 0 && visited <= samples.len());
                }
            }
        }
        let mut native = samples;
        native[3].value = SampleValue::FloatHistogram(FloatHistogram::default());
        visit_future_floats(
            PageSamples::Samples(&native),
            crate::query_scopes::Grid {
                start_ms: 0,
                end_ms: 9000,
                interval_ms: 500,
            },
            0,
            InstantWindow::single(0, 5000, bounds(10000)),
            &control,
            &account,
            |_, _, _, _| panic!("native page must decline before publishing"),
        )
        .unwrap();
        let sparse_gap = [
            sparse(0),
            Sample {
                time_ms: 1_000_000,
                ..sparse(3)
            },
        ];
        let before = account.snapshot().work;
        let mut visited = 0;
        visit_future_floats(
            PageSamples::Samples(&sparse_gap),
            crate::query_scopes::Grid {
                start_ms: 0,
                end_ms: 1_000_000,
                interval_ms: 1,
            },
            0,
            InstantWindow::single(0, 5000, bounds(1_000_000)),
            &control,
            &account,
            |_, _, _, _| {
                visited += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(visited, sparse_gap.len());
        assert!(account.snapshot().work - before < 16);
        assert_closed(&source, &account);
    }

    #[test]
    fn stop_page_observer_runs_after_tail_validation_without_fetching_another_page() {
        fn malformed(index: usize) -> Sample {
            Sample {
                time_ms: [1000, 2000, 1500][index],
                start_time_ms: 0,
                value: SampleValue::Float(index as f64),
            }
        }
        for fault in 0..5 {
            let account = EngineAccounting::default().query();
            let control = Control::default();
            let mut source = if fault == 1 {
                GeneratedSeries::new(3, 3, malformed)
            } else {
                GeneratedSeries::new(6, 3, counter)
            };
            source.fail_page = Some(if fault == 4 { 0 } else { 1 });
            let mut calls = 0;
            let mut observer = |page: PageSamples<'_>| {
                calls += 1;
                assert_eq!(page.len(), 3);
                assert!(account.snapshot().memory[MemoryKind::BorrowedView as usize].current > 0);
                if fault == 2 {
                    return Err(StorageError::Invalid("stop observer failure"));
                }
                if fault == 3 {
                    control.cancellation.cancel();
                }
                Ok(())
            };
            let result = select_mapped_observed::<_, _, StorageError>(
                &source,
                InstantWindow::single(2000, 5000, bounds(10000)),
                None,
                &control,
                &account,
                |value| {
                    Ok(OwnedValue::copy_from_checked(
                        value,
                        &control,
                        &account,
                        MemoryKind::Workspace,
                    )?)
                },
                || Ok(None),
                Some(&mut observer),
            );
            match fault {
                0 => assert!(matches!(
                    result,
                    Ok(Some((2000, 0, OwnedValue::Float(2.0))))
                )),
                1 => assert!(matches!(
                    result,
                    Err(StorageError::Invalid(
                        "sample timestamps are not strictly increasing"
                    ))
                )),
                2 => assert!(matches!(
                    result,
                    Err(StorageError::Invalid("stop observer failure"))
                )),
                3 => assert!(matches!(result, Err(StorageError::Interrupted(_)))),
                _ => assert!(matches!(result, Err(StorageError::Provider(_)))),
            }
            assert_eq!(calls, usize::from(![1, 4].contains(&fault)));
            assert_eq!(source.counters.next_calls.load(Ordering::Relaxed), 1);
            drop(result);
            assert_closed(&source, &account);
        }
    }

    #[test]
    fn seeded_float_lookup_keeps_sparse_future_stale_and_native_outcomes() {
        fn sparse(index: usize) -> Sample {
            Sample {
                time_ms: [0, 1000, 9000, 13000][index],
                start_time_ms: -37,
                value: match index {
                    0 => SampleValue::Float(-0.0),
                    1 => SampleValue::Float(f64::from_bits(STALE_NAN_BITS)),
                    2 => SampleValue::Float(9.0),
                    _ => SampleValue::FloatHistogram(FloatHistogram {
                        count: 1.0,
                        sum: 7.0,
                        ..Default::default()
                    }),
                },
            }
        }
        struct Tracked {
            source: GeneratedSeries,
            bounds: std::cell::RefCell<Vec<TimeBounds>>,
        }
        impl Series for Tracked {
            type Cursor<'a> = crate::test_storage::GeneratedCursor<'a>;
            fn label_count(&self) -> usize {
                0
            }
            fn label(&self, _: usize) -> crate::storage::LabelRef<'_> {
                unreachable!()
            }
            fn resources(&self) -> crate::storage::ProviderResources {
                self.source.resources()
            }
            fn cursor(
                &self,
                bounds: TimeBounds,
                control: &Control,
            ) -> Result<Self::Cursor<'_>, StorageError> {
                self.bounds.borrow_mut().push(bounds);
                self.source.cursor(bounds, control)
            }
        }
        for page in [1, 17, 1024] {
            for (previous, current) in [
                (0, 500),
                (500, 900),
                (900, 1000),
                (1000, 5000),
                (5000, 8999),
                (8999, 9000),
                (9000, 9500),
                (9500, 12000),
                (12000, 12500),
                (12500, 15000),
            ] {
                let source = Tracked {
                    source: GeneratedSeries::new(4, page, sparse),
                    bounds: Default::default(),
                };
                let control = Control::default();
                let account = EngineAccounting::default().query();
                let prior = owned(
                    &source.source,
                    previous,
                    5000,
                    bounds(16000),
                    &control,
                    &account,
                )
                .unwrap();
                let point = prior.as_ref().map(|(time, start, value)| {
                    let OwnedValue::Float(value) = value else {
                        panic!("Float seed fixture")
                    };
                    (*time, *start, *value)
                });
                let seed = FloatSeed {
                    lookup_ms: previous,
                    point,
                };
                drop(prior);
                let expected = owned(
                    &source.source,
                    current,
                    5000,
                    bounds(16000),
                    &control,
                    &account,
                )
                .unwrap();
                let before = account.snapshot().statistics.samples.samples_read;
                let actual = select_mapped_after::<_, _, StorageError>(
                    &source,
                    InstantWindow::single(current, 5000, bounds(16000)),
                    seed,
                    &control,
                    &account,
                    |value| {
                        Ok(OwnedValue::copy_from_checked(
                            value,
                            &control,
                            &account,
                            MemoryKind::Workspace,
                        )?)
                    },
                )
                .unwrap();
                assert_eq!(
                    source
                        .bounds
                        .borrow()
                        .iter()
                        .map(|bounds| (bounds.start_ms, bounds.end_ms))
                        .collect::<Vec<_>>(),
                    [(previous + 1, 16000)]
                );
                assert_eq!(
                    actual.as_ref().map(|(time, start, _)| (*time, *start)),
                    expected.as_ref().map(|(time, start, _)| (*time, *start))
                );
                match (actual.as_ref(), expected.as_ref()) {
                    (Some((_, _, OwnedValue::Float(a))), Some((_, _, OwnedValue::Float(b)))) => {
                        assert_eq!(a.to_bits(), b.to_bits())
                    }
                    _ => assert_eq!(format!("{actual:?}"), format!("{expected:?}")),
                }
                if let Some((_, _, OwnedValue::Float(_))) = actual {
                    assert_eq!(
                        account.snapshot().statistics.samples.samples_read - before,
                        1
                    );
                }
                drop((actual, expected));
                assert_closed(&source.source, &account);
            }
        }
    }

    #[test]
    fn seeded_lookup_validates_the_unconsumed_page_tail() {
        fn malformed(index: usize) -> Sample {
            Sample {
                time_ms: [2000, 3000, 2500][index],
                start_time_ms: 11,
                value: SampleValue::Float(index as f64),
            }
        }
        for page in [17, 1024] {
            for (sample, invalid) in [(counter as fn(usize) -> Sample, false), (malformed, true)] {
                let account = EngineAccounting::default().query();
                let control = Control::default();
                let source = GeneratedSeries::new(3, page, sample);
                let result = select_mapped_after::<_, _, StorageError>(
                    &source,
                    InstantWindow::single(2000, 5000, bounds(5000)),
                    FloatSeed {
                        lookup_ms: 1000,
                        point: Some((1000, 11, 1.0)),
                    },
                    &control,
                    &account,
                    |value| {
                        Ok(OwnedValue::copy_from_checked(
                            value,
                            &control,
                            &account,
                            MemoryKind::Workspace,
                        )?)
                    },
                );
                if invalid {
                    assert!(matches!(
                        result,
                        Err(StorageError::Invalid(
                            "sample timestamps are not strictly increasing"
                        ))
                    ));
                    assert_eq!(source.counters.samples_returned.load(Ordering::Relaxed), 3);
                } else {
                    assert!(matches!(
                        result,
                        Ok(Some((2000, 0, OwnedValue::Float(2.0))))
                    ));
                }
                assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
                assert_eq!(source.counters.next_calls.load(Ordering::Relaxed), 1);
                drop(result);
                assert_closed(&source, &account);
            }
        }
    }

    #[test]
    fn seeded_lookup_keeps_fresh_cursor_errors_and_checked_geometry() {
        let account = EngineAccounting::default().query();
        let control = Control::default();
        let mut source = GeneratedSeries::new(4, 1, counter);
        source.fail_page = Some(0);
        let result = select_mapped_after::<_, _, StorageError>(
            &source,
            InstantWindow::single(2000, 5000, bounds(5000)),
            FloatSeed {
                lookup_ms: 1000,
                point: Some((1000, 11, 1.0)),
            },
            &control,
            &account,
            |value| {
                Ok(crate::statistics::ValueParts::new(
                    OwnedValue::copy_from_checked(
                        value,
                        &control,
                        &account,
                        MemoryKind::Workspace,
                    )?,
                    &account,
                ))
            },
        );
        assert!(matches!(result, Err(StorageError::Provider(_))));
        assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
        assert_eq!(source.counters.next_calls.load(Ordering::Relaxed), 1);
        drop(result);
        assert_closed(&source, &account);
        for (current, previous, lookback) in [
            (5, 5, 10),
            (5, 6, 10),
            (5, 4, 0),
            (i64::MIN + 1, i64::MIN, 10),
        ] {
            assert!(
                InstantWindow::single(current, lookback, bounds(100))
                    .bounds_after(previous)
                    .is_none()
            );
        }
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
