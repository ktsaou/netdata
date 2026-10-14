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

#[derive(Clone, Copy, Debug)]
pub(crate) struct MatrixWindow {
    pub(crate) first_end_ms: i64,
    pub(crate) end_ms: i64,
    pub(crate) range_ms: i64,
    pub(crate) buffer_ms: Option<i64>,
    pub(crate) step_ms: i64,
    pub(crate) data_bounds: TimeBounds,
    pub(crate) statistics: bool,
    pub(crate) reduce_delta: bool,
    pub(crate) resume: Option<Resume>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Geometry {
    first_end_ms: i64,
    range_ms: i64,
    buffer_ms: Option<i64>,
    step_ms: i64,
    data_bounds: [i64; 2],
    statistics: bool,
    reduce_delta: bool,
}
impl From<MatrixWindow> for Geometry {
    fn from(window: MatrixWindow) -> Self {
        Self {
            first_end_ms: window.first_end_ms,
            range_ms: window.range_ms,
            buffer_ms: window.buffer_ms,
            step_ms: window.step_ms,
            data_bounds: [window.data_bounds.start_ms, window.data_bounds.end_ms],
            statistics: window.statistics,
            reduce_delta: window.reduce_delta,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Start {
    target_ms: i64,
    histograms_allocated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Resume {
    geometry: Geometry,
    end_ms: i64,
    start: Start,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Progress {
    geometry: Geometry,
    end_ms: i64,
    last_time_ms: i64,
    current_time_ms: Option<i64>,
    histograms_allocated: bool,
    start: Option<Start>,
    pub(crate) sample_units: u64,
}
impl Progress {
    pub(crate) fn resume(self, window: MatrixWindow) -> Option<Resume> {
        if self.geometry != Geometry::from(window) || !window.prunable() {
            return None;
        }
        let start = if window.end_ms == self.end_ms {
            self.start?
        } else {
            if self.end_ms.checked_add(window.step_ms)? != window.end_ms {
                return None;
            }
            let target_ms = window.end_ms.checked_sub(window.range_ms)?;
            if target_ms <= self.last_time_ms || target_ms <= self.current_time_ms? {
                return None;
            }
            Start {
                target_ms,
                histograms_allocated: self.histograms_allocated,
            }
        };
        Some(Resume {
            geometry: self.geometry,
            end_ms: window.end_ms,
            start,
        })
    }
}

impl MatrixWindow {
    fn prunable(self) -> bool {
        self.buffer_ms.is_none()
            && self.range_ms > 0
            && self.step_ms >= self.range_ms
            && self
                .end_ms
                .checked_sub(self.first_end_ms)
                .is_some_and(|distance| distance >= 0 && distance % self.step_ms == 0)
            && self.first_end_ms.checked_sub(self.range_ms).is_some()
            && self.end_ms.checked_sub(self.range_ms).is_some()
    }

    pub(crate) fn with_progress(mut self, progress: Option<Progress>) -> Self {
        self.resume = progress.and_then(|progress| progress.resume(self));
        self
    }

    fn certified_start(self) -> Option<Start> {
        self.resume
            .filter(|resume| {
                resume.geometry == Geometry::from(self)
                    && resume.end_ms == self.end_ms
                    && self.prunable()
                    && self.end_ms.checked_sub(self.range_ms) == Some(resume.start.target_ms)
            })
            .map(|resume| resume.start)
    }
}

pub(crate) struct Acquisition {
    pub(crate) populated: bool,
    pub(crate) histograms_allocated: bool,
    pub(crate) progress: Progress,
}

struct Replay {
    window: MatrixWindow,
    end_ms: i64,
    delta_ms: i64,
    last_time_ms: i64,
    materialized: [Option<i64>; 2],
    emitted: Option<i64>,
    histograms_allocated: bool,
    current_time_ms: Option<i64>,
    start: Option<Start>,
    decoder: StatisticsDecoder,
    sample_units: u64,
    consuming: bool,
}
impl Replay {
    fn new(window: MatrixWindow, consuming: bool) -> Self {
        let start = window.certified_start();
        Self {
            end_ms: if start.is_some() {
                window.end_ms
            } else {
                window.first_end_ms
            },
            delta_ms: window.buffer_ms.unwrap_or(window.range_ms),
            last_time_ms: i64::MIN,
            materialized: [None, None],
            emitted: None,
            histograms_allocated: start.is_some_and(|start| start.histograms_allocated),
            current_time_ms: None,
            start,
            decoder: StatisticsDecoder::default(),
            sample_units: 0,
            consuming,
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
        let _projection_samples = projection.map(|value| {
            crate::statistics::OwnedSamples::new(account, crate::statistics::weight(value.as_ref()))
        });
        let value = projection.map_or(sample.value, |value| value.as_ref());
        if !admit {
            return Ok(());
        }
        let lower = self.end_ms.wrapping_sub(self.window.range_ms);
        if sample.time_ms <= lower {
            return Ok(());
        }
        if !matches!(value, SampleValueRef::Float(_)) {
            // Source H-slice allocation precedes its stale-value rejection.
            self.histograms_allocated = true;
        }
        if value.is_stale() {
            return Ok(());
        }
        let kind = usize::from(!matches!(value, SampleValueRef::Float(_)));
        self.materialized[kind] = Some(sample.time_ms);
        if bounds.is_some_and(|bounds| {
            sample.time_ms >= bounds.start_ms && sample.time_ms <= bounds.end_ms
        }) && self.emitted.is_none_or(|time| sample.time_ms > time)
        {
            // Ring CopyTo precedes the consumer; exact current points retain provider fields.
            let value = match value {
                SampleValueRef::FloatHistogram(mut histogram) if sample.time_ms < self.end_ms => {
                    if histogram.schema == crate::histogram::CUSTOM_BUCKETS_SCHEMA {
                        histogram.zero_threshold = 0.0;
                        histogram.zero_count = 0.0;
                        histogram.negative_spans = None;
                        histogram.negative_buckets = None;
                    } else {
                        histogram.custom_values = None;
                    }
                    SampleValueRef::FloatHistogram(histogram)
                }
                value => value,
            };
            let units = crate::statistics::weight(value);
            crate::statistics::add_units(&mut self.sample_units, units, account);
            if self.consuming {
                account.samples_read(units);
            }
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
            self.delta_ms = self.delta_ms.min(
                self.window
                    .buffer_ms
                    .unwrap_or(self.window.range_ms)
                    .min(self.window.step_ms),
            );
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
    mut visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), E>,
) -> Result<Option<Progress>, E> {
    let bounds = bounds.into();
    let Some(window) = window else {
        return match bounds {
            Some(bounds) => scan_with(series, bounds, control, account, |sample| {
                if !sample.value.is_stale() {
                    account.samples_read(crate::statistics::weight(sample.value));
                }
                visit(sample)
            })
            .map(|_| None),
            None => Ok(None),
        };
    };
    acquire(series, bounds, window, control, account, visit)
        .map(|acquisition| Some(acquisition.progress))
}

pub(crate) fn acquire<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: impl Into<Option<TimeBounds>>,
    window: MatrixWindow,
    control: &Control,
    account: &QueryAccounting,
    visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), E>,
) -> Result<Acquisition, E> {
    let bounds = bounds.into();
    let acquisition = probe(series, bounds, window, control, account, |_| Ok::<_, E>(()))?;
    if !acquisition.populated {
        return Ok(acquisition);
    }
    let window = window.with_progress(Some(acquisition.progress));
    consume(series, bounds, window, control, account, visit)
}

/// Provisional aggregation must not introduce helper errors before validation completes.
pub(crate) fn probe<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: Option<TimeBounds>,
    window: MatrixWindow,
    control: &Control,
    account: &QueryAccounting,
    visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), E>,
) -> Result<Acquisition, E> {
    replay(series, bounds, window, false, control, account, visit)
}

pub(crate) fn consume<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: Option<TimeBounds>,
    window: MatrixWindow,
    control: &Control,
    account: &QueryAccounting,
    visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), E>,
) -> Result<Acquisition, E> {
    replay(series, bounds, window, true, control, account, visit)
}

fn replay<S: Series, E: From<StorageError> + From<Interrupted>>(
    series: &S,
    bounds: Option<TimeBounds>,
    window: MatrixWindow,
    consuming: bool,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), E>,
) -> Result<Acquisition, E> {
    let mut replay = Replay::new(window, consuming);
    let mut initial_failure = None;
    let start = if let Some(start) = replay.start {
        WalkStart::Seek(start.target_ms)
    } else {
        WalkStart::Probe(&mut initial_failure)
    };
    let outcome = walk_observed::<_, E>(
        series,
        window.data_bounds,
        start,
        control,
        account,
        |sample, origin| {
            replay.current_time_ms = Some(sample.time_ms);
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
                    if replay.end_ms == window.end_ms
                        && window.prunable()
                        && target > sample.time_ms
                    {
                        // This actual source Seek forgets both the ring and statistics predecessor.
                        replay.start = Some(Start {
                            target_ms: target,
                            histograms_allocated: replay.histograms_allocated,
                        });
                    }
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
        replay.current_time_ms = None;
        // EOF has no current access; cached matrix membership survives until its lower bound.
        while !replay.finish_window(control, account)? {}
    }
    control.check()?;
    Ok(Acquisition {
        populated: replay.emitted.is_some(),
        histograms_allocated: replay.histograms_allocated,
        progress: Progress {
            geometry: window.into(),
            end_ms: window.end_ms,
            last_time_ms: replay.last_time_ms,
            current_time_ms: replay.current_time_ms,
            histograms_allocated: replay.histograms_allocated,
            start: replay.start,
            sample_units: replay.sample_units,
        },
    })
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
    use std::{
        cell::{Cell, RefCell},
        sync::atomic::Ordering,
        time::Instant,
    };

    struct ObservedSeries {
        source: GeneratedSeries,
        seeks: RefCell<Vec<i64>>,
        fail_seek: Option<i64>,
        panic_next_at: Cell<Option<usize>>,
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
            if self.owner.panic_next_at.get()
                == Some(
                    self.owner
                        .source
                        .counters
                        .next_calls
                        .load(Ordering::Relaxed),
                )
            {
                assert!(
                    self.inner.resources().held_bytes
                        > (self.inner.max_page_samples() * size_of::<Sample>()) as u64
                );
                panic!("provider next panic after a native page");
            }
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
            buffer_ms: None,
            step_ms: 0,
            data_bounds: TimeBounds {
                start_ms: 501,
                end_ms: 1000,
            },
            statistics: false,
            reduce_delta: true,
            resume: None,
        }
    }

    #[test]
    fn matrix_copy_phase_preserves_buffered_and_direct_native_fields() {
        fn custom(index: usize) -> Sample {
            Sample {
                time_ms: [500, 2000][index],
                start_time_ms: 0,
                value: SampleValue::FloatHistogram(FloatHistogram {
                    schema: crate::histogram::CUSTOM_BUCKETS_SCHEMA,
                    zero_threshold: [0.001, 0.002][index],
                    zero_count: 7.0 + index as f64,
                    count: 1.0,
                    sum: 1.0,
                    custom_values: Some(vec![1.0].into()),
                    positive_spans: Some(
                        vec![Span {
                            offset: 0,
                            length: 1,
                        }]
                        .into(),
                    ),
                    positive_buckets: Some(vec![1.0].into()),
                    negative_spans: Some(
                        vec![Span {
                            offset: 1,
                            length: 1,
                        }]
                        .into(),
                    ),
                    negative_buckets: Some(vec![2.0].into()),
                    ..FloatHistogram::default()
                }),
            }
        }
        fn exponential(index: usize) -> Sample {
            let mut sample = custom(index);
            let SampleValue::FloatHistogram(histogram) = &mut sample.value else {
                unreachable!()
            };
            histogram.schema = 0;
            sample
        }
        for page in [1, 17, 1024] {
            for end_ms in [2000, 3000] {
                for sample in [custom as fn(usize) -> Sample, exponential] {
                    let source = GeneratedSeries::new(2, page, sample);
                    let account = EngineAccounting::default().query();
                    let physical = MatrixWindow {
                        first_end_ms: 2000,
                        end_ms,
                        range_ms: 3000,
                        step_ms: 1000,
                        data_bounds: TimeBounds {
                            start_ms: -999,
                            end_ms,
                        },
                        ..window()
                    };
                    let mut count = 0;
                    scan::<_, StorageError>(
                        &source,
                        TimeBounds {
                            start_ms: 1,
                            end_ms,
                        },
                        Some(physical),
                        &Control::default(),
                        &account,
                        |sample| {
                            let SampleValueRef::FloatHistogram(histogram) = sample.value else {
                                panic!("native point")
                            };
                            let direct = sample.time_ms == 2000;
                            if histogram.schema == crate::histogram::CUSTOM_BUCKETS_SCHEMA {
                                assert_eq!(
                                    histogram.zero_threshold,
                                    if direct { 0.002 } else { 0.0 }
                                );
                                assert_eq!(histogram.zero_count, if direct { 8.0 } else { 0.0 });
                                assert_eq!(histogram.negative_buckets.is_some(), direct);
                                assert_eq!(histogram.custom_values, Some([1.0].as_slice()));
                            } else {
                                assert_eq!(
                                    histogram.zero_threshold,
                                    if direct { 0.002 } else { 0.001 }
                                );
                                assert_eq!(histogram.zero_count, if direct { 8.0 } else { 7.0 });
                                assert!(histogram.negative_buckets.is_some());
                                assert_eq!(histogram.custom_values.is_some(), direct);
                            }
                            count += 1;
                            Ok(())
                        },
                    )
                    .unwrap();
                    assert_eq!(count, 2);
                    assert_closed(&source, &account);
                    assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                }
            }
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

    fn disjoint(index: usize) -> Sample {
        Sample {
            time_ms: [1200, 1800, 2100, 3200, 3800, 4100, 5200, 5800][index],
            start_time_ms: 0,
            value: SampleValue::Float([10.0, 12.0, 13.0, 14.0, 16.0, 17.0, 18.0, 20.0][index]),
        }
    }
    fn disjoint_window(end_ms: i64) -> MatrixWindow {
        MatrixWindow {
            first_end_ms: 2000,
            end_ms,
            range_ms: 1000,
            step_ms: 2000,
            data_bounds: TimeBounds {
                start_ms: 0,
                end_ms: 10_000,
            },
            ..window()
        }
    }

    fn before_epoch(index: usize) -> Sample {
        let mut sample = disjoint(index);
        sample.time_ms -= 6000;
        sample
    }
    fn near_minimum(index: usize) -> Sample {
        let mut sample = disjoint(index);
        sample.time_ms += i64::MIN + 10_000;
        sample
    }
    fn near_maximum(index: usize) -> Sample {
        let mut sample = disjoint(index);
        sample.time_ms += i64::MAX - 10_000;
        sample
    }

    #[test]
    fn clipped_float_probes_preserve_complete_consuming_progress() {
        let mut cases = 0;
        for page in [1, 17, 1024] {
            for count in [0, 2, 3, 8] {
                for step_ms in [500, 1000, 2000] {
                    for clipped in [false, true] {
                        for (offset, sample) in [
                            (0, disjoint as fn(usize) -> Sample),
                            (-6000, before_epoch),
                            (i64::MIN + 10_000, near_minimum),
                            (i64::MAX - 10_000, near_maximum),
                        ] {
                            let original = GeneratedSeries::new(count, page, sample);
                            let provisional = GeneratedSeries::new(count, page, sample);
                            let original_account = EngineAccounting::default().query();
                            let provisional_account = EngineAccounting::default().query();
                            let control = Control::default();
                            let mut original_progress = None;
                            let mut provisional_progress = None;
                            for relative_end in
                                [2000, 2000 + step_ms, 2000 + 2 * step_ms, 2000 + 2 * step_ms]
                            {
                                let end_ms = offset + relative_end;
                                let data_bounds = TimeBounds {
                                    start_ms: offset + if clipped { 1500 } else { 0 },
                                    end_ms: offset + if clipped { 4500 } else { 10_000 },
                                };
                                let bounds = TimeBounds {
                                    start_ms: (end_ms - 999).max(data_bounds.start_ms),
                                    end_ms: end_ms.min(data_bounds.end_ms),
                                };
                                let bounds = (bounds.start_ms <= bounds.end_ms).then_some(bounds);
                                let physical = MatrixWindow {
                                    first_end_ms: offset + 2000,
                                    end_ms,
                                    step_ms,
                                    data_bounds,
                                    ..disjoint_window(end_ms)
                                };
                                let mut expected = Vec::new();
                                let consumed = acquire::<_, StorageError>(
                                    &original,
                                    bounds,
                                    physical.with_progress(original_progress),
                                    &control,
                                    &original_account,
                                    |sample| {
                                        let SampleValueRef::Float(value) = sample.value else {
                                            unreachable!()
                                        };
                                        expected.push((sample.time_ms, value.to_bits()));
                                        Ok(())
                                    },
                                )
                                .unwrap();
                                let mut actual = Vec::new();
                                let acquired = probe::<_, StorageError>(
                                    &provisional,
                                    bounds,
                                    physical.with_progress(provisional_progress),
                                    &control,
                                    &provisional_account,
                                    |sample| {
                                        let SampleValueRef::Float(value) = sample.value else {
                                            unreachable!()
                                        };
                                        actual.push((sample.time_ms, value.to_bits()));
                                        Ok(())
                                    },
                                )
                                .unwrap();
                                assert_eq!(actual, expected);
                                assert_eq!(acquired.populated, consumed.populated);
                                assert_eq!(
                                    acquired.histograms_allocated,
                                    consumed.histograms_allocated
                                );
                                assert_eq!(
                                    acquired.progress, consumed.progress,
                                    "page {page}, count {count}, step {step_ms}, clipped {clipped}, offset {offset}, end {relative_end}"
                                );
                                assert_eq!(
                                    provisional_account
                                        .snapshot()
                                        .statistics
                                        .samples
                                        .samples_read,
                                    0
                                );
                                original_progress = Some(consumed.progress);
                                provisional_progress = Some(acquired.progress);
                                cases += 1;
                            }
                            assert_closed(&original, &original_account);
                            assert_closed(&provisional, &provisional_account);
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 1152);
    }

    #[test]
    fn adjacent_source_seek_certificates_preserve_window_membership() {
        for page in [1, 17, 1024] {
            let source = ObservedSeries {
                source: GeneratedSeries::new(8, page, disjoint),
                seeks: RefCell::new(Vec::new()),
                fail_seek: None,
                panic_next_at: Cell::new(None),
            };
            let account = EngineAccounting::default().query();
            let mut progress = None;
            for (end_ms, expected) in [
                (2000, [1200, 1800]),
                (4000, [3200, 3800]),
                (6000, [5200, 5800]),
                (6000, [5200, 5800]),
            ] {
                let mut times = Vec::new();
                progress = scan::<_, StorageError>(
                    &source,
                    TimeBounds {
                        start_ms: end_ms - 999,
                        end_ms,
                    },
                    Some(disjoint_window(end_ms).with_progress(progress)),
                    &Control::default(),
                    &account,
                    |sample| {
                        times.push(sample.time_ms);
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(times, expected);
                if end_ms == 4000 {
                    let progress = progress.unwrap();
                    let next = disjoint_window(6000);
                    assert!(progress.resume(next).is_some());
                    for changed in [
                        MatrixWindow {
                            end_ms: 8000,
                            ..next
                        },
                        MatrixWindow {
                            range_ms: 0,
                            ..next
                        },
                        MatrixWindow {
                            buffer_ms: Some(1000),
                            ..next
                        },
                        MatrixWindow {
                            step_ms: 1000,
                            ..next
                        },
                        MatrixWindow {
                            statistics: true,
                            ..next
                        },
                        MatrixWindow {
                            first_end_ms: 4000,
                            ..next
                        },
                    ] {
                        assert!(progress.resume(changed).is_none());
                    }
                }
            }
            assert_eq!(
                &*source.seeks.borrow(),
                &[1000, 1000, 3000, 3000, 5000, 5000, 5000, 5000]
            );
            assert_closed(&source.source, &account);
            assert_eq!(source.source.counters.peak.load(Ordering::Relaxed), 1);
        }
    }

    fn future(index: usize) -> Sample {
        if index < 2 {
            disjoint(index)
        } else {
            Sample {
                time_ms: 7000,
                ..disjoint(2)
            }
        }
    }

    #[test]
    fn future_current_and_eof_do_not_certify_a_seek() {
        for page in [1, 17, 1024] {
            for count in [2, 3] {
                let source = ObservedSeries {
                    source: GeneratedSeries::new(count, page, future),
                    seeks: RefCell::new(Vec::new()),
                    fail_seek: Some(3000),
                    panic_next_at: Cell::new(None),
                };
                let account = EngineAccounting::default().query();
                let progress = scan::<_, StorageError>(
                    &source,
                    TimeBounds {
                        start_ms: 1001,
                        end_ms: 2000,
                    },
                    Some(disjoint_window(2000)),
                    &Control::default(),
                    &account,
                    |_| Ok(()),
                )
                .unwrap()
                .unwrap();
                assert_eq!(progress.current_time_ms, (count == 3).then_some(7000));
                assert!(progress.resume(disjoint_window(4000)).is_none());
                scan::<_, StorageError>(
                    &source,
                    TimeBounds {
                        start_ms: 3001,
                        end_ms: 4000,
                    },
                    Some(disjoint_window(4000).with_progress(Some(progress))),
                    &Control::default(),
                    &account,
                    |_| Ok(()),
                )
                .unwrap();
                assert!(!source.seeks.borrow().contains(&3000));
                assert_closed(&source.source, &account);
            }
        }
    }

    fn disjoint_histogram(index: usize) -> Sample {
        let sample = disjoint(index);
        Sample {
            value: SampleValue::FloatHistogram(FloatHistogram {
                count: (index + 1) as f64,
                sum: (index + 1) as f64,
                ..FloatHistogram::default()
            }),
            ..sample
        }
    }
    fn stale_histogram(index: usize) -> Sample {
        if index == 0 {
            let mut sample = disjoint_histogram(index);
            let SampleValue::FloatHistogram(histogram) = &mut sample.value else {
                unreachable!()
            };
            histogram.sum = f64::from_bits(crate::value::STALE_NAN_BITS);
            sample
        } else {
            disjoint(index)
        }
    }

    #[test]
    fn resumed_windows_clear_statistics_predecessors_and_keep_stale_h_allocation() {
        for page in [1, 17, 1024] {
            for statistics in [false, true] {
                let source = GeneratedSeries::new(8, page, stale_histogram);
                let account = EngineAccounting::default().query();
                let mut progress = None;
                for end_ms in [2000, 4000] {
                    let acquisition = acquire::<_, StorageError>(
                        &source,
                        TimeBounds {
                            start_ms: end_ms - 999,
                            end_ms,
                        },
                        MatrixWindow {
                            statistics,
                            ..disjoint_window(end_ms)
                        }
                        .with_progress(progress),
                        &Control::default(),
                        &account,
                        |_| Ok(()),
                    )
                    .unwrap();
                    assert!(acquisition.histograms_allocated);
                    progress = Some(acquisition.progress);
                }
                assert_closed(&source, &account);
            }
            let source = GeneratedSeries::new(8, page, disjoint_histogram);
            let account = EngineAccounting::default().query();
            let mut progress = None;
            for end_ms in [2000, 4000] {
                let mut hints = Vec::new();
                progress = scan::<_, StorageError>(
                    &source,
                    TimeBounds {
                        start_ms: end_ms - 999,
                        end_ms,
                    },
                    Some(
                        MatrixWindow {
                            statistics: true,
                            ..disjoint_window(end_ms)
                        }
                        .with_progress(progress),
                    ),
                    &Control::default(),
                    &account,
                    |sample| {
                        let SampleValueRef::FloatHistogram(histogram) = sample.value else {
                            unreachable!()
                        };
                        hints.push(histogram.reset_hint);
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(hints, [0, NOT_RESET]);
            }
            assert_closed(&source, &account);
        }
    }

    #[test]
    fn advancing_rate_helpers_have_linear_work_and_history_independent_memory() {
        use crate::{
            range_float::{RateKind, RateWindow},
            range_rates::{RateFunction, evaluate},
            result::OwnedValue,
        };
        for page in [1, 17, 1024] {
            let mut previous = None;
            for steps in [64, 128] {
                let source = GeneratedSeries::new(100_000, page, counter);
                let account = EngineAccounting::default().query();
                let mut progress = None;
                for step in 0..steps {
                    let end_ms = 3000 + step * 4000;
                    let matrix = MatrixWindow {
                        first_end_ms: 3000,
                        end_ms,
                        range_ms: 2000,
                        step_ms: 4000,
                        data_bounds: TimeBounds {
                            start_ms: 1,
                            end_ms: 100_000_000,
                        },
                        ..window()
                    }
                    .with_progress(progress);
                    let output = evaluate(
                        &source,
                        RateWindow {
                            start_ms: end_ms - 2000,
                            end_ms,
                            range_ns: 2_000_000_000,
                            start_timestamps: false,
                        },
                        RateFunction::Extrapolated(RateKind::Rate),
                        Some(matrix),
                        &Control::default(),
                        &account,
                    )
                    .unwrap();
                    let Some(OwnedValue::Float(value)) = output.value else {
                        panic!("float rate")
                    };
                    assert_eq!(value.to_bits(), 1.0f64.to_bits());
                    progress = output.progress;
                }
                let snapshot = account.snapshot();
                assert!(snapshot.samples_fetched <= 16 * steps as u64 * page as u64);
                let peaks = [
                    MemoryKind::Workspace,
                    MemoryKind::ProviderHeld,
                    MemoryKind::BorrowedView,
                ]
                .map(|kind| snapshot.memory[kind as usize].peak);
                if let Some(previous) = previous {
                    assert_eq!(peaks, previous);
                }
                previous = Some(peaks);
                assert_closed(&source, &account);
                assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
            }
        }
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
                        panic_next_at: Cell::new(None),
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
                        buffer_ms: None,
                        step_ms: 1000,
                        data_bounds: TimeBounds {
                            start_ms: 1,
                            end_ms,
                        },
                        statistics,
                        reduce_delta: true,
                        resume: None,
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

    #[test]
    fn resumed_validation_cancellation_and_provider_panic_release_native_state() {
        for page in [1, 17, 1024] {
            for interrupt in [true, false] {
                let source = ObservedSeries {
                    source: GeneratedSeries::new(10_000, page, native),
                    seeks: RefCell::new(Vec::new()),
                    fail_seek: None,
                    panic_next_at: Cell::new(None),
                };
                let account = EngineAccounting::default().query();
                let cold = MatrixWindow {
                    first_end_ms: 2_048_000,
                    end_ms: 2_048_000,
                    range_ms: 2_048_000,
                    step_ms: 4_096_000,
                    data_bounds: TimeBounds {
                        start_ms: 1000,
                        end_ms: 10_000_000,
                    },
                    statistics: true,
                    ..window()
                };
                let progress = scan::<_, StorageError>(
                    &source,
                    cold.data_bounds,
                    Some(cold),
                    &Control::default(),
                    &account,
                    |_| Ok(()),
                )
                .unwrap()
                .expect("completed cold acquisition");
                assert_closed(&source.source, &account);
                let warm = MatrixWindow {
                    end_ms: 6_144_000,
                    ..cold
                }
                .with_progress(Some(progress));
                assert!(warm.resume.is_some());
                source.seeks.borrow_mut().clear();
                let supplied = source
                    .source
                    .counters
                    .samples_returned
                    .load(Ordering::Relaxed);
                let helper_called = Cell::new(false);
                if interrupt {
                    let control = Control::default();
                    let failure = interrupt_during(&account, &control, || {
                        scan::<_, StorageError>(
                            &source,
                            warm.data_bounds,
                            Some(warm),
                            &control,
                            &account,
                            |_| {
                                helper_called.set(true);
                                Ok(())
                            },
                        )
                    });
                    assert_eq!(
                        failure,
                        Err(StorageError::Interrupted(Interrupted::Cancelled))
                    );
                } else {
                    source.panic_next_at.set(Some(
                        source.source.counters.next_calls.load(Ordering::Relaxed) + 1,
                    ));
                    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        scan::<_, StorageError>(
                            &source,
                            warm.data_bounds,
                            Some(warm),
                            &Control::default(),
                            &account,
                            |_| {
                                helper_called.set(true);
                                Ok(())
                            },
                        )
                    }));
                    assert!(failure.is_err());
                }
                assert!(!helper_called.get());
                assert_eq!(source.seeks.borrow()[0], 4_096_000);
                assert!(
                    source
                        .source
                        .counters
                        .samples_returned
                        .load(Ordering::Relaxed)
                        > supplied
                );
                assert_closed(&source.source, &account);

                source.panic_next_at.set(None);
                source.seeks.borrow_mut().clear();
                let mut members = 0;
                let recovered = scan::<_, StorageError>(
                    &source,
                    warm.data_bounds,
                    Some(warm),
                    &Control::default(),
                    &account,
                    |_| {
                        members += 1;
                        Ok(())
                    },
                )
                .unwrap();
                assert!(recovered.is_some());
                assert_eq!(members, 2048);
                assert_eq!(source.seeks.borrow()[0], 4_096_000);
                assert_closed(&source.source, &account);
            }
        }
    }
}
