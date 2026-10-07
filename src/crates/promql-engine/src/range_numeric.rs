// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus promql/functions.go:981-1031,1949-2043; see NOTICE.
#[cfg(target_arch = "x86_64")]
use crate::numerical::{source_add, source_div, source_sub};
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    annotations::AnnotationKind,
    numerical::{compensated_add, source_mul},
    storage::{Control, CursorAction, Series, StorageError, TimeBounds, WalkStart, walk},
    value::{SampleRef, SampleValueRef},
};
use std::fmt::Write;

#[derive(Clone, Copy)]
pub(crate) struct NumericRangeInput {
    pub(crate) bounds: TimeBounds,
    pub(crate) seek_ms: i64,
}
impl From<TimeBounds> for NumericRangeInput {
    fn from(bounds: TimeBounds) -> Self {
        Self {
            bounds,
            seek_ms: bounds.start_ms.saturating_sub(1),
        }
    }
}

pub(crate) fn scan<S: Series>(
    series: &S,
    input: NumericRangeInput,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    // Go's buffer seeks the excluded left endpoint; returned pages retain included bounds.
    walk(
        series,
        input.bounds,
        WalkStart::Seek(input.seek_ms),
        control,
        account,
        |sample| {
            visit(sample)?;
            Ok(CursorAction::Next)
        },
    )
    .map(|_| ())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NumericRangeFunction {
    Deriv,
    PredictLinear,
    Smoothing,
    Mad,
}
impl NumericRangeFunction {
    pub(crate) fn parameters(self) -> usize {
        match self {
            Self::Deriv | Self::Mad => 0,
            Self::PredictLinear => 1,
            Self::Smoothing => 2,
        }
    }
}

#[derive(Debug)]
pub(crate) struct NumericRangeOutput {
    pub(crate) value: Option<f64>,
    pub(crate) annotation: Option<AnnotationKind>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum NumericRangeError {
    Storage(StorageError),
    Factor { trend: bool, value: f64 },
}
impl From<StorageError> for NumericRangeError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}
impl NumericRangeError {
    pub(crate) fn message(trend: bool, value: f64, account: &QueryAccounting) -> AccountedVec<u8> {
        let mut out = AccountedVec::new(account, MemoryKind::Workspace);
        let factor = if trend { "trend" } else { "smoothing" };
        write!(out, "invalid {factor} factor. Expected: 0 < ").expect("accounted message writer");
        let parameter = if trend { "tf" } else { "sf" };
        write!(out, "{parameter} < 1, got: ").expect("accounted message writer");
        if value == f64::INFINITY {
            out.extend_from_slice(b"+Inf");
        } else if value == f64::NEG_INFINITY {
            out.extend_from_slice(b"-Inf");
        } else {
            write!(out, "{value:.6}").expect("accounted message writer");
        }
        out
    }
}

#[derive(Default)]
struct Regression {
    count: usize,
    n: f64,
    x: f64,
    cx: f64,
    y: f64,
    cy: f64,
    xy: f64,
    cxy: f64,
    x2: f64,
    cx2: f64,
    initial: f64,
    constant: bool,
    intercept_time: i64,
}
impl Regression {
    fn add(&mut self, time: i64, value: f64, intercept_time: Option<i64>) {
        if self.count == 0 {
            self.initial = value;
            self.constant = true;
            self.intercept_time = intercept_time.unwrap_or(time);
        } else if self.constant && value != self.initial {
            self.constant = false;
        }
        self.count += 1;
        self.n += 1.0;
        let x = time.wrapping_sub(self.intercept_time) as f64 / 1000.0;
        (self.x, self.cx) = compensated_add(x, self.x, self.cx);
        (self.y, self.cy) = compensated_add(value, self.y, self.cy);
        (self.xy, self.cxy) = compensated_add(source_mul(x, value), self.xy, self.cxy);
        (self.x2, self.cx2) = compensated_add(x * x, self.x2, self.cx2);
    }
    fn finish(mut self) -> (f64, f64) {
        if self.constant {
            return if self.initial.is_infinite() {
                let nan = f64::from_bits(0x7ff8_0000_0000_0001);
                (nan, nan)
            } else {
                (0.0, self.initial)
            };
        }
        #[cfg(target_arch = "x86_64")]
        {
            self.x = source_add(self.x, self.cx);
            self.y = source_add(self.y, self.cy);
            self.xy = source_add(self.xy, self.cxy);
            self.x2 = source_add(self.x2, self.cx2);
            // The compiled Go products retain sumY and saved sumX, respectively.
            let covariance = source_sub(self.xy, source_div(source_mul(self.y, self.x), self.n));
            let variance = source_sub(self.x2, source_div(source_mul(self.x, self.x), self.n));
            let slope = source_div(covariance, variance);
            let intercept = source_sub(
                source_div(self.y, self.n),
                source_div(source_mul(self.x, slope), self.n),
            );
            (slope, intercept)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self.x += self.cx;
            self.y += self.cy;
            self.xy += self.cxy;
            self.x2 += self.cx2;
            let covariance = self.xy - self.x * self.y / self.n;
            let variance = self.x2 - self.x * self.x / self.n;
            let slope = covariance / variance;
            let intercept = self.y / self.n - slope * self.x / self.n;
            (slope, intercept)
        }
    }
}

#[derive(Default)]
struct Smoothing {
    count: usize,
    previous: f64,
    current: f64,
    trend: f64,
}
impl Smoothing {
    fn add(&mut self, value: f64, sf: f64, tf: f64) {
        if self.count == 0 {
            self.current = value;
        } else {
            if self.count == 1 {
                self.trend = value - self.current;
            }
            let scaled = sf * value;
            if self.count > 1 {
                let x = tf * (self.current - self.previous);
                let y = (1.0 - tf) * self.trend;
                self.trend = x + y;
            }
            let smoothed = (1.0 - sf) * (self.current + self.trend);
            self.previous = self.current;
            self.current = scaled + smoothed;
        }
        self.count += 1;
    }
}

pub(crate) fn evaluate<S: Series>(
    series: &S,
    input: impl Into<NumericRangeInput>,
    function: NumericRangeFunction,
    parameters: [f64; 2],
    time_ms: i64,
    control: &Control,
    account: &QueryAccounting,
) -> Result<NumericRangeOutput, NumericRangeError> {
    let input = input.into();
    if function == NumericRangeFunction::Mad {
        return crate::mad::evaluate(series, input, control, account).map_err(Into::into);
    }
    let invalid_factor = if function == NumericRangeFunction::Smoothing {
        parameters
            .into_iter()
            .enumerate()
            .find(|(_, value)| *value <= 0.0 || *value >= 1.0)
    } else {
        None
    };
    let mut present = false;
    let mut histograms = false;
    let mut regression = Regression::default();
    let mut smoothing = Smoothing::default();
    scan(series, input, control, account, |sample| {
        if sample.value.is_stale() {
            return Ok(());
        }
        present = true;
        if invalid_factor.is_some() {
            return Ok(());
        }
        if let SampleValueRef::Float(value) = sample.value {
            account.work(1);
            if function == NumericRangeFunction::Smoothing {
                smoothing.add(value, parameters[0], parameters[1]);
            } else {
                regression.add(
                    sample.time_ms,
                    value,
                    (function == NumericRangeFunction::PredictLinear).then_some(time_ms),
                );
            }
        } else {
            histograms = true;
        }
        Ok(())
    })?;
    if present && let Some((index, value)) = invalid_factor {
        return Err(NumericRangeError::Factor {
            trend: index == 1,
            value,
        });
    }
    let count = if function == NumericRangeFunction::Smoothing {
        smoothing.count
    } else {
        regression.count
    };
    let value = (count >= 2).then(|| match function {
        NumericRangeFunction::Deriv => regression.finish().0,
        NumericRangeFunction::PredictLinear => {
            let (slope, intercept) = regression.finish();
            #[cfg(target_arch = "x86_64")]
            {
                source_add(intercept, source_mul(slope, parameters[0]))
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                slope * parameters[0] + intercept
            }
        }
        NumericRangeFunction::Smoothing => smoothing.current,
        NumericRangeFunction::Mad => unreachable!("MAD uses bounded median replay"),
    });
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
        accounting::EngineAccounting,
        histogram::FloatHistogram,
        storage::Interrupted,
        test_storage::GeneratedSeries,
        value::{Sample, SampleValue},
    };
    use std::sync::atomic::Ordering;

    fn linear(index: usize) -> Sample {
        Sample {
            time_ms: index as i64 * 1000,
            start_time_ms: 0,
            value: SampleValue::Float(2.0 * index as f64 + 7.0),
        }
    }
    fn constant(index: usize) -> Sample {
        Sample {
            value: SampleValue::Float(7.0),
            ..linear(index)
        }
    }
    fn window(count: usize) -> TimeBounds {
        TimeBounds {
            start_ms: 0,
            end_ms: count as i64 * 1000,
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn prediction_nan_provenance_survives_paged_regression() {
        fn middle(index: usize) -> Sample {
            Sample {
                time_ms: (index as i64 - 3) * 1000,
                start_time_ms: 0,
                value: SampleValue::Float(f64::from_bits(
                    [
                        0x3ff0_0000_0000_0000,
                        0x7ff8_0000_0000_0042,
                        0x4000_0000_0000_0000,
                        0x4008_0000_0000_0000,
                    ][index],
                )),
            }
        }
        // Original registered Go functions retain the sample NaN through prediction.
        for page in [1, 17, 1024] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let source = GeneratedSeries::new(4, page, middle);
            for function in [
                NumericRangeFunction::Deriv,
                NumericRangeFunction::PredictLinear,
            ] {
                let result = evaluate(
                    &source,
                    TimeBounds {
                        start_ms: -10_000,
                        end_ms: 0,
                    },
                    function,
                    [f64::from_bits(0x7ff8_0000_0000_0001), 0.0],
                    0,
                    &Control::default(),
                    &account,
                )
                .unwrap();
                assert_eq!(result.value.unwrap().to_bits(), 0x7ff8_0000_0000_0042);
                assert_eq!(result.annotation, None);
            }
            assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
            assert_eq!(
                source.counters.opened.load(Ordering::Relaxed),
                source.counters.closed.load(Ordering::Relaxed)
            );
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn regression_and_smoothing_history_length_keep_constant_workspace() {
        for page in [1, 17, 1024] {
            for function in [
                NumericRangeFunction::Deriv,
                NumericRangeFunction::PredictLinear,
                NumericRangeFunction::Smoothing,
            ] {
                let mut previous_peak = None;
                for count in [2048, 100_000] {
                    let sample = if function == NumericRangeFunction::Smoothing {
                        constant
                    } else {
                        linear
                    };
                    let source = GeneratedSeries::new(count, page, sample);
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let parameters = if function == NumericRangeFunction::Smoothing {
                        [0.5, 0.5]
                    } else {
                        [15.0, 0.0]
                    };
                    let result = evaluate(
                        &source,
                        window(count),
                        function,
                        parameters,
                        count as i64 * 1000,
                        &Control::default(),
                        &account,
                    )
                    .unwrap();
                    // Independent pinned-Go controls include the long regression's rounding.
                    let expected_bits = match (function, count) {
                        (NumericRangeFunction::Deriv, 2048) => 0x4000_0000_0000_0000,
                        (NumericRangeFunction::Deriv, _) => 0x4000_0000_0000_0002,
                        (NumericRangeFunction::PredictLinear, 2048) => 0x40b0_2500_0000_0000,
                        (NumericRangeFunction::PredictLinear, _) => 0x4108_6b28_0000_0002,
                        (NumericRangeFunction::Smoothing, _) => 0x401c_0000_0000_0000,
                        (NumericRangeFunction::Mad, _) => unreachable!("regression test roster"),
                    };
                    assert_eq!(result.value.unwrap().to_bits(), expected_bits);
                    assert_eq!(result.annotation, None);
                    let snapshot = account.snapshot();
                    let peak = snapshot.memory[MemoryKind::Workspace as usize].peak;
                    if let Some(previous) = previous_peak {
                        assert_eq!(peak, previous);
                    }
                    previous_peak = Some(peak);
                    assert!(snapshot.work > snapshot.samples_fetched);
                    assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                    assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                    assert_eq!(
                        source.counters.opened.load(Ordering::Relaxed),
                        source.counters.closed.load(Ordering::Relaxed)
                    );
                    snapshot.assert_memory_released();
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn smoothing_retains_initial_trend_and_ignores_histograms() {
        fn changing(index: usize) -> Sample {
            Sample {
                value: SampleValue::Float([3.0, 5.0, 4.0, 8.0][index]),
                ..linear(index)
            }
        }
        fn mixed(index: usize) -> Sample {
            if index == 0 {
                constant(index)
            } else {
                Sample {
                    value: SampleValue::FloatHistogram(FloatHistogram::default()),
                    ..linear(index)
                }
            }
        }
        fn histogram(index: usize) -> Sample {
            Sample {
                value: SampleValue::FloatHistogram(FloatHistogram::default()),
                ..linear(index)
            }
        }
        for page in [1, 17, 1024] {
            let account = EngineAccounting::default().query();
            let source = GeneratedSeries::new(4, page, changing);
            let result = evaluate(
                &source,
                window(4),
                NumericRangeFunction::Smoothing,
                [0.5, 0.5],
                4000,
                &Control::default(),
                &account,
            )
            .unwrap();
            // Levels are 5, 5.5 and 7.375; their trends are 2, 2 and 1.25.
            assert_eq!(result.value, Some(7.375));
            let source = GeneratedSeries::new(2, page, changing);
            let result = evaluate(
                &source,
                window(2),
                NumericRangeFunction::Smoothing,
                [0.5, f64::NAN],
                2000,
                &Control::default(),
                &account,
            )
            .unwrap();
            assert_eq!(result.value, Some(5.0));
            for function in [
                NumericRangeFunction::Deriv,
                NumericRangeFunction::PredictLinear,
                NumericRangeFunction::Smoothing,
            ] {
                let source = GeneratedSeries::new(2, page, mixed);
                let result = evaluate(
                    &source,
                    window(2),
                    function,
                    [0.5, 0.5],
                    2000,
                    &Control::default(),
                    &account,
                )
                .unwrap();
                assert_eq!(result.value, None);
                assert_eq!(
                    result.annotation,
                    Some(AnnotationKind::HistogramIgnoredInMixedRange)
                );
                let source = GeneratedSeries::new(2, page, histogram);
                let result = evaluate(
                    &source,
                    window(2),
                    function,
                    [0.5, 0.5],
                    2000,
                    &Control::default(),
                    &account,
                )
                .unwrap();
                assert_eq!(result.value, None);
                assert_eq!(result.annotation, None);
            }
            account.snapshot().assert_memory_released();
            // The scanner's address-region table is retained; histogram payloads are borrowed.
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].peak,
                64
            );
        }
    }

    #[test]
    fn factor_errors_require_a_typed_window_and_follow_provider_errors() {
        fn stale(index: usize) -> Sample {
            Sample {
                value: SampleValue::Float(f64::from_bits(crate::value::STALE_NAN_BITS)),
                ..linear(index)
            }
        }
        for page in [1, 17, 1024] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            for count in [0, 1] {
                let source = GeneratedSeries::new(count, page, stale);
                assert!(
                    evaluate(
                        &source,
                        window(2),
                        NumericRangeFunction::Smoothing,
                        [0.0, 1.0],
                        2000,
                        &control,
                        &account
                    )
                    .unwrap()
                    .value
                    .is_none()
                );
            }
            let source = GeneratedSeries::new(1, page, constant);
            let error = evaluate(
                &source,
                window(2),
                NumericRangeFunction::Smoothing,
                [-0.0, 1.0],
                2000,
                &control,
                &account,
            )
            .unwrap_err();
            assert!(
                matches!(error, NumericRangeError::Factor { trend: false, value } if value.to_bits() == (-0.0f64).to_bits())
            );
            let message = NumericRangeError::message(false, -0.0, &account);
            assert_eq!(
                message.as_ref(),
                b"invalid smoothing factor. Expected: 0 < sf < 1, got: -0.000000"
            );
            assert!(account.snapshot().memory[MemoryKind::Workspace as usize].current > 0);
            drop(message);
            let message = NumericRangeError::message(true, f64::INFINITY, &account);
            assert_eq!(
                message.as_ref(),
                b"invalid trend factor. Expected: 0 < tf < 1, got: +Inf"
            );
            drop(message);
            let mut source = GeneratedSeries::new(page + 1, page, constant);
            source.fail_page = Some(1);
            assert!(matches!(
                evaluate(
                    &source,
                    window(page + 1),
                    NumericRangeFunction::Smoothing,
                    [0.0, 1.0],
                    2000,
                    &control,
                    &account
                ),
                Err(NumericRangeError::Storage(StorageError::Provider(_)))
            ));
            assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
            control.cancellation.cancel();
            assert!(matches!(
                evaluate(
                    &source,
                    window(2),
                    NumericRangeFunction::Deriv,
                    [0.0; 2],
                    2000,
                    &control,
                    &account
                ),
                Err(NumericRangeError::Storage(StorageError::Interrupted(
                    Interrupted::Cancelled
                )))
            ));
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }
}
