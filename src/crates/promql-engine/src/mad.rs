// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned promql/functions.go:1438-1471 and quantile.go:717-738; see NOTICE.
#[cfg(target_arch = "x86_64")]
use crate::numerical::source_sub;
use crate::{
    accounting::QueryAccounting,
    annotations::AnnotationKind,
    numerical::{floor, source_add, source_mul},
    range_numeric::{NumericRangeInput, NumericRangeOutput, scan},
    storage::{Control, Series, StorageError},
    value::SampleValueRef,
};

const SOURCE_NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

fn ordered_key(value: f64) -> u64 {
    if value.is_nan() {
        // Only the homogeneous, Abs-generated NaN class reaches median replay.
        return 0;
    }
    let bits = if value == 0.0 { 0 } else { value.to_bits() };
    if bits >> 63 != 0 {
        !bits
    } else {
        bits ^ (1 << 63)
    }
}

fn ordered_value(key: u64) -> f64 {
    let bits = if key == 0 {
        0x7ff8_0000_0000_0000
    } else if key >> 63 == 0 {
        !key
    } else {
        key ^ (1 << 63)
    };
    f64::from_bits(bits)
}

fn deviation(value: f64, median: f64) -> f64 {
    #[cfg(target_arch = "x86_64")]
    {
        source_sub(value, median).abs()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        (value - median).abs()
    }
}

fn median<S: Series>(
    series: &S,
    input: NumericRangeInput,
    count: u64,
    center: Option<f64>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<f64, StorageError> {
    let n = count as f64;
    let rank = source_mul(0.5, n - 1.0);
    let lower = floor(rank).max(0.0);
    let upper = (n - 1.0).min(lower + 1.0);
    let weight = rank - floor(rank);
    let mut ranks = [lower as u64, upper as u64];
    let mut prefixes = [0u64; 2];
    for nibble in 0..16 {
        control.check()?;
        let shift = 60 - 4 * nibble;
        let mask = if nibble == 0 {
            0
        } else {
            u64::MAX << (shift + 4)
        };
        let mut counters = [[0u64; 16]; 2];
        scan(series, input, control, account, |sample| {
            if sample.value.is_stale() {
                return Ok(());
            }
            let SampleValueRef::Float(value) = sample.value else {
                return Ok(());
            };
            account.work(1);
            let value = center.map_or(value, |median| deviation(value, median));
            let key = ordered_key(value);
            let digit = ((key >> shift) & 15) as usize;
            for lane in 0..2 {
                account.work(1);
                if key & mask == prefixes[lane] {
                    counters[lane][digit] += 1;
                }
            }
            Ok(())
        })?;
        for lane in 0..2 {
            control.check()?;
            let mut selected = None;
            for (digit, count) in counters[lane].iter().enumerate() {
                account.work(1);
                if ranks[lane] < *count {
                    selected = Some(digit);
                    break;
                }
                ranks[lane] -= count;
            }
            let digit = selected.ok_or(StorageError::Invalid("MAD rank missing during replay"))?;
            prefixes[lane] |= (digit as u64) << shift;
        }
    }
    let lower_product = source_mul(1.0 - weight, ordered_value(prefixes[0]));
    let upper_product = source_mul(ordered_value(prefixes[1]), weight);
    // Zero-weight terms remain observable for terminal infinities and generated NaNs.
    Ok(source_add(upper_product, lower_product))
}

pub(crate) fn evaluate<S: Series>(
    series: &S,
    input: impl Into<NumericRangeInput>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<NumericRangeOutput, StorageError> {
    let input = input.into();
    let mut count = 0u64;
    let mut raw_nan = false;
    let mut histograms = false;
    // Decode the complete window before the raw-NaN shortcut, preserving iterator errors.
    scan(series, input, control, account, |sample| {
        if !sample.value.is_stale() {
            account.work(1);
            if let SampleValueRef::Float(value) = sample.value {
                count += 1;
                raw_nan |= value.is_nan();
            } else {
                histograms = true;
            }
        }
        Ok(())
    })?;
    let value = if count == 0 {
        None
    } else if raw_nan {
        Some(SOURCE_NAN)
    } else {
        let center = median(series, input, count, None, control, account)?;
        Some(median(
            series,
            input,
            count,
            Some(center),
            control,
            account,
        )?)
    };
    Ok(NumericRangeOutput {
        value,
        annotation: (histograms && count > 0)
            .then_some(AnnotationKind::HistogramIgnoredInMixedRange),
    })
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

    #[cfg(target_arch = "x86_64")]
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
                    if (3..=5).contains(&case) { 1 } else { 33 }
                );
                assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                account.snapshot().assert_memory_released();
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
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn long_histories_keep_one_page_and_constant_owned_workspace() {
        for page in [1, 17, 1024] {
            let mut previous = None;
            for count in [2048, 100_000] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let source = GeneratedSeries::new(count, page, counter);
                let result =
                    evaluate(&source, window(count), &Control::default(), &account).unwrap();
                // Consecutive integers in these even, divisible-by-four windows have MAD n/4.
                assert_eq!(result.value, Some(count as f64 / 4.0));
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
                assert_eq!(snapshot.samples_fetched, 33 * count as u64);
                assert!(snapshot.work > snapshot.samples_fetched);
                assert_eq!(source.counters.opened.load(Ordering::Relaxed), 33);
                assert_eq!(source.counters.closed.load(Ordering::Relaxed), 33);
                assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                snapshot.assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
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
                        source: GeneratedSeries::new(35, page, counter),
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
            let engine = EngineAccounting::default();
            let account = engine.query();
            let source = ReplayFault {
                source: GeneratedSeries::new(5, page, counter),
                open: 0,
                cancel: false,
                seek_ms: Some(-3000),
            };
            let input = NumericRangeInput {
                bounds: Some(TimeBounds {
                    start_ms: 2000,
                    end_ms: 5000,
                }),
                seek_ms: -3000,
                matrix: None,
            };
            let result = evaluate(&source, input, &Control::default(), &account).unwrap();
            assert_eq!(result.value, Some(1.0));
            assert_eq!(source.source.counters.opened.load(Ordering::Relaxed), 33);
            assert_eq!(source.source.counters.closed.load(Ordering::Relaxed), 33);
            assert_eq!(source.source.counters.active.load(Ordering::Relaxed), 0);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
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
