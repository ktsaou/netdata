// SPDX-License-Identifier: GPL-3.0-or-later
// Ordinary buffer replay derives from pinned storage/buffer.go and matrixIterSlice; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    histogram_stats::StatisticsDecoder,
    storage::{
        Control, CursorAction, CursorOrigin, Interrupted, Series, StorageError, TimeBounds,
        WalkStart, scan_with, walk_observed,
    },
    value::{SampleRef, SampleValueRef},
};

#[derive(Clone, Copy)]
pub(crate) struct MatrixWindow {
    pub(crate) first_end_ms: i64,
    pub(crate) end_ms: i64,
    pub(crate) range_ms: i64,
    pub(crate) step_ms: i64,
    pub(crate) data_bounds: TimeBounds,
    pub(crate) statistics: bool,
    pub(crate) reduce_delta: bool,
}

struct Replay {
    window: MatrixWindow,
    end_ms: i64,
    delta_ms: i64,
    last_time_ms: i64,
    materialized: [Option<i64>; 2],
    emitted: Option<i64>,
    decoder: StatisticsDecoder,
}
impl Replay {
    fn new(window: MatrixWindow) -> Self {
        Self {
            end_ms: window.first_end_ms,
            delta_ms: window.range_ms,
            last_time_ms: i64::MIN,
            materialized: [None, None],
            emitted: None,
            decoder: StatisticsDecoder::default(),
            window,
        }
    }
    fn access<E: From<StorageError> + From<Interrupted>>(
        &mut self,
        sample: SampleRef<'_>,
        admit: bool,
        bounds: Option<TimeBounds>,
        control: &Control,
        account: &QueryAccounting,
        visit: &mut impl for<'a> FnMut(SampleRef<'a>) -> Result<(), E>,
    ) -> Result<(), E> {
        let projection = if self.window.statistics {
            Some(self.decoder.project(sample.value, control, account)?)
        } else {
            None
        };
        let value = projection.map_or(sample.value, |value| value.as_ref());
        if !admit || value.is_stale() {
            return Ok(());
        }
        let lower = self.end_ms.wrapping_sub(self.window.range_ms);
        if sample.time_ms <= lower {
            return Ok(());
        }
        let kind = usize::from(!matches!(value, SampleValueRef::Float(_)));
        self.materialized[kind] = Some(sample.time_ms);
        if bounds.is_some_and(|bounds| {
            sample.time_ms >= bounds.start_ms && sample.time_ms <= bounds.end_ms
        }) && self.emitted.is_none_or(|time| sample.time_ms > time)
        {
            visit(SampleRef { value, ..sample })?;
            self.emitted = Some(sample.time_ms);
        }
        Ok(())
    }
    fn finish_window(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, StorageError> {
        control.check()?;
        account.work(1);
        let lower = self.end_ms.wrapping_sub(self.window.range_ms);
        if self.window.reduce_delta && self.materialized.iter().flatten().any(|&time| time > lower)
        {
            self.delta_ms = self
                .delta_ms
                .min(self.window.range_ms.min(self.window.step_ms));
        }
        if self.end_ms == self.window.end_ms {
            return Ok(true);
        }
        self.end_ms = self
            .end_ms
            .checked_add(self.window.step_ms)
            .filter(|&next| next > self.end_ms && next <= self.window.end_ms)
            .ok_or(StorageError::Invalid("invalid matrix replay schedule"))?;
        Ok(false)
    }
}

/// Validate acquisition before any helper callback can fail.
pub(crate) fn scan<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: impl Into<Option<TimeBounds>>,
    window: Option<MatrixWindow>,
    control: &Control,
    account: &QueryAccounting,
    visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), E>,
) -> Result<(), E> {
    let bounds = bounds.into();
    let Some(window) = window else {
        return match bounds {
            Some(bounds) => scan_with(series, bounds, control, account, visit),
            None => Ok(()),
        };
    };
    let populated = replay(series, bounds, window, control, account, |_| Ok::<_, E>(()))?;
    if populated {
        replay(series, bounds, window, control, account, visit)?;
    }
    Ok(())
}

fn replay<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: Option<TimeBounds>,
    window: MatrixWindow,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), E>,
) -> Result<bool, E> {
    let mut replay = Replay::new(window);
    let mut initial_failure = None;
    let outcome = walk_observed::<_, E>(
        series,
        window.data_bounds,
        WalkStart::Probe(&mut initial_failure),
        control,
        account,
        |sample, origin| {
            match origin {
                CursorOrigin::InitialNext => replay.decoder.moved(),
                CursorOrigin::Next => {
                    replay.decoder.moved();
                    replay.last_time_ms = sample.time_ms;
                }
                CursorOrigin::SeekLanding => replay.last_time_ms = sample.time_ms,
            }
            loop {
                control.check()?;
                account.work(1);
                if window.range_ms == 0 {
                    if replay.finish_window(control, account)? {
                        return Ok(CursorAction::Stop);
                    }
                    continue;
                }
                let target = replay.end_ms.wrapping_sub(replay.delta_ms);
                if target > replay.last_time_ms {
                    if window.statistics && target > sample.time_ms {
                        replay.decoder.clear();
                    }
                    return Ok(
                        if origin == CursorOrigin::InitialNext && target <= sample.time_ms {
                            CursorAction::SeekInitial(target)
                        } else {
                            CursorAction::Seek(target)
                        },
                    );
                }
                if replay.last_time_ms < replay.end_ms {
                    replay.access(sample, true, bounds, control, account, &mut visit)?;
                    return Ok(CursorAction::Next);
                }
                // A future float is accessed, while a future histogram only supplies AtT.
                if matches!(sample.value, SampleValueRef::Float(_))
                    || sample.time_ms == replay.end_ms
                {
                    replay.access(
                        sample,
                        sample.time_ms == replay.end_ms,
                        bounds,
                        control,
                        account,
                        &mut visit,
                    )?;
                }
                if replay.finish_window(control, account)? {
                    return Ok(CursorAction::Stop);
                }
            }
        },
    )?;
    if let Some(error) = initial_failure
        && window.range_ms != 0
    {
        return Err(error.into());
    }
    if outcome.is_continue() {
        // EOF has no current access; cached matrix membership survives until its lower bound.
        while !replay.finish_window(control, account)? {}
    }
    control.check()?;
    Ok(replay.emitted.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        histogram::{FloatHistogram, NOT_RESET, Span},
        storage::{LabelRef, ProviderResources, SampleCursor, SamplePage},
        test_storage::{GeneratedCursor, GeneratedSeries, counter, interrupt_during},
        value::{Sample, SampleValue},
    };
    use std::{cell::RefCell, sync::atomic::Ordering, time::Instant};

    struct ObservedSeries {
        source: GeneratedSeries,
        seeks: RefCell<Vec<i64>>,
        fail_seek: Option<i64>,
    }
    struct ObservedCursor<'a> {
        inner: GeneratedCursor<'a>,
        owner: &'a ObservedSeries,
    }
    impl Series for ObservedSeries {
        type Cursor<'a> = ObservedCursor<'a>;
        fn label_count(&self) -> usize {
            0
        }
        fn label(&self, _: usize) -> LabelRef<'_> {
            panic!("unlabelled fixture")
        }
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn cursor(
            &self,
            bounds: TimeBounds,
            control: &Control,
        ) -> Result<Self::Cursor<'_>, StorageError> {
            Ok(ObservedCursor {
                inner: self.source.cursor(bounds, control)?,
                owner: self,
            })
        }
    }
    impl SampleCursor for ObservedCursor<'_> {
        fn resources(&self) -> ProviderResources {
            self.inner.resources()
        }
        fn max_page_samples(&self) -> usize {
            self.inner.max_page_samples()
        }
        fn seek(&mut self, target: i64, control: &Control) -> Result<(), StorageError> {
            self.owner.seeks.borrow_mut().push(target);
            if self.owner.fail_seek == Some(target) {
                return Err(StorageError::Provider(
                    b"current seek failed".to_vec().into(),
                ));
            }
            self.inner.seek(target, control)
        }
        fn next(
            &mut self,
            control: &Control,
            resources: &mut ProviderResources,
        ) -> Result<Option<SamplePage<'_>>, StorageError> {
            self.inner.next(control, resources)
        }
    }
    fn endpoints(index: usize) -> Sample {
        Sample {
            time_ms: [600, 1000][index],
            start_time_ms: 0,
            value: SampleValue::Float(index as f64 + 1.0),
        }
    }
    fn window() -> MatrixWindow {
        MatrixWindow {
            first_end_ms: 1000,
            end_ms: 1000,
            range_ms: 500,
            step_ms: 0,
            data_bounds: TimeBounds {
                start_ms: 501,
                end_ms: 1000,
            },
            statistics: false,
            reduce_delta: true,
        }
    }
    fn assert_closed(source: &GeneratedSeries, account: &QueryAccounting) {
        assert_eq!(
            source.counters.opened.load(Ordering::Relaxed),
            source.counters.closed.load(Ordering::Relaxed)
        );
        assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
        assert_eq!(
            account.snapshot().samples_fetched,
            source.counters.samples_returned.load(Ordering::Relaxed) as u64
        );
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn initial_current_seek_is_observable_even_without_included_membership() {
        // These physical targets are bound by the ordinary and clipped source controls.
        for page in [1, 17, 1024] {
            for clipped in [false, true] {
                for failed in [false, true] {
                    let physical = if clipped {
                        MatrixWindow {
                            first_end_ms: 999,
                            end_ms: 999,
                            range_ms: 1,
                            data_bounds: TimeBounds {
                                start_ms: 1000,
                                end_ms: 1000,
                            },
                            ..window()
                        }
                    } else {
                        window()
                    };
                    let target = if clipped { 998 } else { 500 };
                    let source = ObservedSeries {
                        source: GeneratedSeries::new(2, page, endpoints),
                        seeks: RefCell::new(Vec::new()),
                        fail_seek: failed.then_some(target),
                    };
                    let account = EngineAccounting::default().query();
                    let mut times = Vec::new();
                    let bounds = (!clipped).then_some(physical.data_bounds);
                    let outcome = scan::<_, StorageError>(
                        &source,
                        bounds,
                        Some(physical),
                        &Control::default(),
                        &account,
                        |sample| {
                            times.push(sample.time_ms);
                            Ok(())
                        },
                    );
                    assert!(!source.seeks.borrow().is_empty());
                    assert!(source.seeks.borrow().iter().all(|&seek| seek == target));
                    if failed {
                        assert_eq!(
                            outcome,
                            Err(StorageError::Provider(
                                b"current seek failed".to_vec().into()
                            ))
                        );
                        assert!(times.is_empty());
                        drop(outcome);
                    } else {
                        outcome.unwrap();
                        assert_eq!(times, if clipped { vec![] } else { vec![600, 1000] });
                    }
                    assert_closed(&source.source, &account);
                    assert_eq!(source.source.counters.peak.load(Ordering::Relaxed), 1);
                }
            }
        }
    }

    #[test]
    fn acquisition_errors_precede_helpers_and_zero_range_ignores_initial_provider_error() {
        for range_ms in [0, 1] {
            let mut source = GeneratedSeries::new(2, 1, endpoints);
            source.fail_page = Some(0);
            let account = EngineAccounting::default().query();
            let mut called = false;
            let outcome = scan::<_, StorageError>(
                &source,
                None,
                Some(MatrixWindow {
                    range_ms,
                    ..window()
                }),
                &Control::default(),
                &account,
                |_| {
                    called = true;
                    Err(StorageError::Invalid("helper must not run"))
                },
            );
            assert!(!called);
            assert_eq!(source.counters.next_calls.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
            if range_ms == 0 {
                outcome.unwrap();
            } else {
                assert_eq!(
                    outcome,
                    Err(StorageError::Provider(
                        b"injected page failure".to_vec().into()
                    ))
                );
                drop(outcome);
            }
            assert_closed(&source, &account);
        }
        let mut source = GeneratedSeries::new(2, 1, endpoints);
        source.fail_page = Some(1);
        let account = EngineAccounting::default().query();
        let mut called = false;
        let outcome = scan::<_, StorageError>(
            &source,
            window().data_bounds,
            Some(window()),
            &Control::default(),
            &account,
            |_| {
                called = true;
                Err(StorageError::Invalid("helper must not run"))
            },
        );
        assert_eq!(
            outcome,
            Err(StorageError::Provider(
                b"injected page failure".to_vec().into()
            ))
        );
        assert!(!called);
        drop(outcome);
        assert_closed(&source, &account);
    }

    fn native(index: usize) -> Sample {
        Sample {
            value: SampleValue::FloatHistogram(FloatHistogram {
                reset_hint: NOT_RESET,
                count: 10.0,
                sum: 10.0,
                positive_spans: Some(
                    vec![Span {
                        offset: 0,
                        length: 4,
                    }]
                    .into(),
                ),
                positive_buckets: Some(vec![1.0, 2.0, 3.0, 4.0].into()),
                ..FloatHistogram::default()
            }),
            ..counter(index)
        }
    }

    #[test]
    fn many_steps_and_long_native_histories_keep_raw_memory_page_bounded() {
        for page in [1, 17, 1024] {
            for statistics in [false, true] {
                let mut peaks = Vec::new();
                for count in [2048, 100_000] {
                    let source = GeneratedSeries::new(count, page, native);
                    let account = EngineAccounting::default().query();
                    let end_ms = count as i64 * 1000;
                    let physical = MatrixWindow {
                        first_end_ms: 1000,
                        end_ms,
                        range_ms: 1000,
                        step_ms: 1000,
                        data_bounds: TimeBounds {
                            start_ms: 1,
                            end_ms,
                        },
                        statistics,
                        reduce_delta: true,
                    };
                    let mut outputs = 0;
                    scan::<_, StorageError>(
                        &source,
                        TimeBounds {
                            start_ms: end_ms - 999,
                            end_ms,
                        },
                        Some(physical),
                        &Control::default(),
                        &account,
                        |sample| {
                            outputs += 1;
                            assert_eq!(sample.time_ms, end_ms);
                            let SampleValueRef::FloatHistogram(histogram) = sample.value else {
                                panic!("native value")
                            };
                            assert_eq!(histogram.count, 10.0);
                            assert_eq!(histogram.positive_buckets.is_none(), statistics);
                            Ok(())
                        },
                    )
                    .unwrap();
                    assert_eq!(outputs, 1);
                    assert_closed(&source, &account);
                    assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                    let observation = account.snapshot();
                    peaks.push(
                        [
                            MemoryKind::ProviderHeld,
                            MemoryKind::BorrowedView,
                            MemoryKind::Workspace,
                        ]
                        .map(|kind| observation.memory[kind as usize].peak),
                    );
                }
                assert_eq!(peaks[0], peaks[1], "page {page}, statistics {statistics}");
            }
        }
    }

    #[test]
    fn interruption_and_unwind_release_the_current_page_and_decoder() {
        for page in [1, 17, 1024] {
            let source = GeneratedSeries::new(100_000, page, native);
            let account = EngineAccounting::default().query();
            let control = Control::default();
            let physical = MatrixWindow {
                first_end_ms: 100_000_000,
                end_ms: 100_000_000,
                range_ms: 100_000_000,
                data_bounds: TimeBounds {
                    start_ms: 1,
                    end_ms: 100_000_000,
                },
                statistics: true,
                ..window()
            };
            let outcome = interrupt_during(&account, &control, || {
                scan::<_, StorageError>(
                    &source,
                    physical.data_bounds,
                    Some(physical),
                    &control,
                    &account,
                    |_| Ok(()),
                )
            });
            assert_eq!(
                outcome,
                Err(StorageError::Interrupted(Interrupted::Cancelled))
            );
            assert!(source.counters.samples_returned.load(Ordering::Relaxed) > 0);
            assert_closed(&source, &account);
        }
        for (sample, physical) in [
            (endpoints as fn(usize) -> Sample, window()),
            (
                native,
                MatrixWindow {
                    first_end_ms: 2000,
                    end_ms: 2000,
                    range_ms: 2000,
                    data_bounds: TimeBounds {
                        start_ms: 1,
                        end_ms: 2000,
                    },
                    statistics: true,
                    ..window()
                },
            ),
        ] {
            let source = GeneratedSeries::new(2, 1, sample);
            let account = EngineAccounting::default().query();
            let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                scan::<_, StorageError>(
                    &source,
                    physical.data_bounds,
                    Some(physical),
                    &Control::default(),
                    &account,
                    |_| panic!("helper panic"),
                )
            }));
            assert!(unwind.is_err());
            assert_closed(&source, &account);
        }

        let source = GeneratedSeries::new(2, 1, endpoints);
        let account = EngineAccounting::default().query();
        let control = Control {
            deadline: Some(Instant::now()),
            ..Control::default()
        };
        let outcome = scan::<_, StorageError>(
            &source,
            window().data_bounds,
            Some(window()),
            &control,
            &account,
            |_| panic!("expired helper"),
        );
        assert_eq!(
            outcome,
            Err(StorageError::Interrupted(Interrupted::Deadline))
        );
        assert_eq!(source.counters.opened.load(Ordering::Relaxed), 0);
        assert_closed(&source, &account);
    }
}
