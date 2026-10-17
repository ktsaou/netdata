// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned promql/functions.go:1438-1471 and quantile.go:717-738; see NOTICE.
use crate::numerical::source_sub;
use crate::{
    accounting::QueryAccounting,
    annotations::AnnotationKind,
    range_numeric::{NumericRangeInput, NumericRangeOutput, scan},
    range_quantile_rank::{Plan, Selector, ordered_value},
    storage::{Control, Series, StorageError},
    value::{SampleRef, SampleValueRef},
};

const SOURCE_NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

fn deviation(value: f64, median: f64) -> f64 {
    source_sub(value, median).abs()
}

struct Median {
    center: Option<f64>,
    _center_samples: Option<crate::statistics::OwnedSamples>,
    plan: Plan,
    selector: Selector,
}
impl Median {
    fn new(count: u64, center: Option<f64>, account: &QueryAccounting) -> Self {
        let plan = Plan::new(0.5, count);
        Self {
            center,
            _center_samples: center.map(|_| crate::statistics::OwnedSamples::new(account, 1)),
            plan,
            selector: Selector::new(plan),
        }
    }
    fn receive(&mut self, value: f64, account: &QueryAccounting) {
        let value = self.center.map_or(value, |median| deviation(value, median));
        self.selector.receive(value, account);
    }
    fn finish_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<f64>, StorageError> {
        Ok(self
            .selector
            .finish_pass(control, account)?
            .map(|keys| self.plan.interpolate(keys.map(ordered_value))))
    }
}

#[derive(Default)]
pub(crate) struct Step {
    count: u64,
    raw_nan: bool,
    histograms: bool,
    median: Option<Median>,
}
impl Step {
    pub(crate) fn receive(
        &mut self,
        sample: SampleRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        control.check()?;
        if sample.value.is_stale() {
            return Ok(());
        }
        if let Some(median) = &mut self.median {
            if let SampleValueRef::Float(value) = sample.value {
                median.receive(value, account);
            }
        } else {
            account.work(1);
            if let SampleValueRef::Float(value) = sample.value {
                self.count += 1;
                self.raw_nan |= value.is_nan();
            } else {
                self.histograms = true;
            }
        }
        Ok(())
    }
    pub(crate) fn finish_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<NumericRangeOutput>, StorageError> {
        let value = if let Some(median) = &mut self.median {
            let Some(value) = median.finish_pass(control, account)? else {
                return Ok(None);
            };
            if median.center.is_none() {
                self.median = Some(Median::new(self.count, Some(value), account));
                return Ok(None);
            }
            Some(value)
        } else if self.count == 0 {
            None
        } else if self.raw_nan {
            Some(SOURCE_NAN)
        } else {
            // Complete membership precedes the shortcut and every median pass.
            self.median = Some(Median::new(self.count, None, account));
            return Ok(None);
        };
        Ok(Some(NumericRangeOutput {
            float_samples: value.map(|_| crate::statistics::OwnedSamples::new(account, 1)),
            progress: None,
            value,
            invalid_quantile: false,
            nonempty: self.count != 0 || self.histograms,
            annotation: (self.histograms && self.count > 0)
                .then_some(AnnotationKind::HistogramIgnoredInMixedRange),
        }))
    }
}

pub(crate) fn evaluate<S: Series>(
    series: &S,
    input: impl Into<NumericRangeInput>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<NumericRangeOutput, StorageError> {
    let mut input = input.into();
    let mut step = Step::default();
    loop {
        let progress = scan(series, input, control, account, |sample| {
            step.receive(sample, control, account)
        })?;
        if let Some(mut output) = step.finish_pass(control, account)? {
            output.progress = progress;
            return Ok(output);
        }
        input.matrix = input.matrix.map(|window| window.with_progress(progress));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        histogram::FloatHistogram,
        storage::{Interrupted, LabelRef, ProviderResources, SampleCursor, SamplePage, TimeBounds},
        test_storage::{GeneratedCursor, GeneratedSeries, counter},
        value::{STALE_NAN_BITS, Sample, SampleValue},
    };
    use std::sync::atomic::Ordering;

    fn window(count: usize) -> TimeBounds {
        TimeBounds {
            start_ms: 0,
            end_ms: count as i64 * 1000,
        }
    }

    #[test]
    fn ieee_medians_keep_both_weighted_terms_and_raw_nan_provenance() {
        fn bits(case: usize) -> &'static [u64] {
            match case {
                0 => &[0x8000_0000_0000_0001, 1],
                1 => &[0, 0, 0x7ff0_0000_0000_0000],
                2 => &[0xfff0_0000_0000_0000, 0, 0],
                3 => &[0x7ff8_0000_0000_0042, 0x3ff0_0000_0000_0000],
                4 => &[0xfff8_0000_0000_0043, 0x3ff0_0000_0000_0000],
                5 => &[0x7ff0_0000_0000_0043, 0x3ff0_0000_0000_0000],
                6 => &[0x8000_0000_0000_0000, 0, 0x8000_0000_0000_0000],
                7 => &[0x8000_0000_0000_0000],
                _ => unreachable!("declared source IEEE cases"),
            }
        }
        fn sample<const CASE: usize>(index: usize) -> Sample {
            Sample {
                value: SampleValue::Float(f64::from_bits(bits(CASE)[index])),
                ..counter(index)
            }
        }
        // Original registered Go outcomes include positive and negative generated NaNs.
        type SourceCase = (fn(usize) -> Sample, u64);
        let cases: [SourceCase; 8] = [
            (sample::<0>, 0),
            (sample::<1>, 0x7ff8_0000_0000_0000),
            (sample::<2>, 0xfff8_0000_0000_0000),
            (sample::<3>, 0x7ff8_0000_0000_0001),
            (sample::<4>, 0x7ff8_0000_0000_0001),
            (sample::<5>, 0x7ff8_0000_0000_0001),
            (sample::<6>, 0),
            (sample::<7>, 0),
        ];
        for page in [1, 17, 1024] {
            for (case, (sample, expected)) in cases.iter().enumerate() {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let count = bits(case).len();
                let source = GeneratedSeries::new(count, page, *sample);
                let result =
                    evaluate(&source, window(count), &Control::default(), &account).unwrap();
                assert_eq!(
                    result.value.unwrap().to_bits(),
                    *expected,
                    "case{case}/page{page}"
                );
                assert_eq!(result.annotation, None);
                assert_eq!(
                    source.counters.opened.load(Ordering::Relaxed),
                    [4, 4, 5, 1, 1, 1, 3, 3][case]
                );
                assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                account.snapshot().assert_memory_released();
                drop(result);
                assert_eq!(account.current_samples(), 0);
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn stale_empty_and_native_windows_keep_float_membership() {
        fn histogram(index: usize) -> Sample {
            Sample {
                value: SampleValue::FloatHistogram(FloatHistogram::default()),
                ..counter(index)
            }
        }
        fn stale(index: usize) -> Sample {
            Sample {
                value: SampleValue::Float(f64::from_bits(STALE_NAN_BITS)),
                ..counter(index)
            }
        }
        fn mixed(index: usize) -> Sample {
            if index == 0 {
                counter(index)
            } else {
                histogram(index)
            }
        }
        for page in [1, 17, 1024] {
            for (count, sample, expected, note) in [
                (0, counter as fn(usize) -> Sample, None, None),
                (3, stale, None, None),
                (3, histogram, None, None),
                (
                    3,
                    mixed,
                    Some(0.0),
                    Some(AnnotationKind::HistogramIgnoredInMixedRange),
                ),
            ] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let source = GeneratedSeries::new(count, page, sample);
                let result =
                    evaluate(&source, window(count), &Control::default(), &account).unwrap();
                assert_eq!(result.value, expected);
                assert_eq!(result.annotation, note);
                assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                assert_eq!(
                    source.counters.opened.load(Ordering::Relaxed),
                    source.counters.closed.load(Ordering::Relaxed)
                );
                account.snapshot().assert_memory_released();
                drop(result);
                assert_eq!(account.current_samples(), 0);
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn long_histories_keep_one_page_and_constant_owned_workspace() {
        for page in [1, 17, 1024] {
            for (sample, scans, expected) in [
                (
                    counter as fn(usize) -> Sample,
                    [15, 17],
                    [512.0f64.to_bits(), 25_000.0f64.to_bits()],
                ),
                (
                    crate::test_storage::subnormal_counter,
                    [9, 12],
                    [512, 25_000],
                ),
                (crate::test_storage::mad_prefix_ladder, [33, 33], [0; 2]),
                (
                    crate::test_storage::two_classes,
                    [4, 4],
                    [0.5f64.to_bits(); 2],
                ),
            ] {
                let mut previous = None;
                for ((count, scans), expected) in
                    [2048, 100_000].into_iter().zip(scans).zip(expected)
                {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let source = GeneratedSeries::new(count, page, sample);
                    let result =
                        evaluate(&source, window(count), &Control::default(), &account).unwrap();
                    assert_eq!(result.value.unwrap().to_bits(), expected);
                    let snapshot = account.snapshot();
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
                    assert_eq!(snapshot.samples_fetched, scans * count as u64);
                    assert!(snapshot.work > snapshot.samples_fetched);
                    assert_eq!(
                        source.counters.opened.load(Ordering::Relaxed),
                        scans as usize
                    );
                    assert_eq!(
                        source.counters.closed.load(Ordering::Relaxed),
                        scans as usize
                    );
                    assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                    assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                    snapshot.assert_memory_released();
                    drop(result);
                    assert_eq!(account.current_samples(), 0);
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    struct ReplayFault {
        source: GeneratedSeries,
        open: usize,
        cancel: bool,
        seek_ms: Option<i64>,
    }
    struct ReplayCursor<'a> {
        inner: GeneratedCursor<'a>,
        seek_ms: Option<i64>,
    }
    impl SampleCursor for ReplayCursor<'_> {
        fn resources(&self) -> ProviderResources {
            self.inner.resources()
        }
        fn max_page_samples(&self) -> usize {
            self.inner.max_page_samples()
        }
        fn seek(&mut self, time_ms: i64, control: &Control) -> Result<(), StorageError> {
            if let Some(expected) = self.seek_ms {
                assert_eq!(time_ms, expected);
            }
            self.inner.seek(time_ms, control)
        }
        fn next(
            &mut self,
            control: &Control,
            resources: &mut ProviderResources,
        ) -> Result<Option<SamplePage<'_>>, StorageError> {
            self.inner.next(control, resources)
        }
    }
    impl Series for ReplayFault {
        type Cursor<'a> = ReplayCursor<'a>;
        fn label_count(&self) -> usize {
            0
        }
        fn label(&self, _: usize) -> LabelRef<'_> {
            unreachable!("unlabelled MAD source")
        }
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn cursor(
            &self,
            bounds: TimeBounds,
            control: &Control,
        ) -> Result<Self::Cursor<'_>, StorageError> {
            if self.source.counters.opened.load(Ordering::Relaxed) + 1 == self.open {
                if self.cancel {
                    control.cancellation.cancel();
                    control.check()?;
                }
                return Err(StorageError::Provider(
                    b"injected median replay failure".to_vec().into(),
                ));
            }
            Ok(ReplayCursor {
                inner: self.source.cursor(bounds, control)?,
                seek_ms: self.seek_ms,
            })
        }
    }

    #[test]
    fn errors_and_cancellation_release_every_median_replay_phase() {
        for page in [1, 17, 1024] {
            for open in [1, 2, 17, 18, 33] {
                for cancel in [false, true] {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let source = ReplayFault {
                        source: GeneratedSeries::new(
                            35,
                            page,
                            crate::test_storage::mad_prefix_ladder,
                        ),
                        open,
                        cancel,
                        seek_ms: None,
                    };
                    let error =
                        evaluate(&source, window(35), &Control::default(), &account).unwrap_err();
                    if cancel {
                        assert_eq!(error, StorageError::Interrupted(Interrupted::Cancelled));
                    } else {
                        assert!(matches!(error, StorageError::Provider(_)));
                    }
                    assert_eq!(source.source.counters.active.load(Ordering::Relaxed), 0);
                    assert_eq!(
                        source.source.counters.opened.load(Ordering::Relaxed),
                        open - 1
                    );
                    assert_eq!(
                        source.source.counters.closed.load(Ordering::Relaxed),
                        open - 1
                    );
                    drop(error);
                    account.snapshot().assert_memory_released();
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn original_seek_target_survives_clipped_cursor_bounds() {
        for page in [1, 17, 1024] {
            for (sample, count, expected, scans) in [
                (counter as fn(usize) -> Sample, 5, 1.0f64, 8),
                (crate::test_storage::subnormal_counter, 5, 0.0, 5),
                (crate::test_storage::mad_prefix_ladder, 35, 0.0, 33),
            ] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let source = ReplayFault {
                    source: GeneratedSeries::new(count, page, sample),
                    open: 0,
                    cancel: false,
                    seek_ms: Some(-3000),
                };
                let input = NumericRangeInput {
                    bounds: Some(TimeBounds {
                        start_ms: 2000,
                        end_ms: count as i64 * 1000,
                    }),
                    seek_ms: -3000,
                    matrix: None,
                };
                let result = evaluate(&source, input, &Control::default(), &account).unwrap();
                assert_eq!(result.value.unwrap().to_bits(), expected.to_bits());
                assert_eq!(source.source.counters.opened.load(Ordering::Relaxed), scans);
                assert_eq!(source.source.counters.closed.load(Ordering::Relaxed), scans);
                assert_eq!(source.source.counters.active.load(Ordering::Relaxed), 0);
                account.snapshot().assert_memory_released();
                drop(result);
                assert_eq!(account.current_samples(), 0);
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn a_raw_nan_does_not_hide_a_later_page_failure() {
        fn sample(index: usize) -> Sample {
            if index == 0 {
                Sample {
                    value: SampleValue::Float(f64::from_bits(0x7ff8_0000_0000_0042)),
                    ..counter(index)
                }
            } else {
                counter(index)
            }
        }
        for page in [1, 17, 1024] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let mut source = GeneratedSeries::new(page + 1, page, sample);
            source.fail_page = Some(1);
            let error =
                evaluate(&source, window(page + 1), &Control::default(), &account).unwrap_err();
            assert!(matches!(error, StorageError::Provider(_)));
            assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.closed.load(Ordering::Relaxed), 1);
            assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
            drop(error);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }
}
