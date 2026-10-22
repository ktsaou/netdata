// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus promql/functions.go:981-1031,1949-2043; see NOTICE.
use crate::numerical::{source_add, source_div, source_sub};
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    annotations::AnnotationKind,
    matrix_reader::{MatrixWindow, Progress, scan as scan_matrix},
    numerical::{compensated_add, source_mul},
    storage::{Control, CursorAction, Series, StorageError, TimeBounds, WalkStart, walk},
    value::{SampleRef, SampleValueRef},
};
use std::fmt::Write;

#[derive(Clone, Copy)]
pub(crate) struct NumericRangeInput {
    pub(crate) bounds: Option<TimeBounds>,
    pub(crate) seek_ms: i64,
    pub(crate) matrix: Option<MatrixWindow>,
}
impl From<TimeBounds> for NumericRangeInput {
    fn from(bounds: TimeBounds) -> Self {
        Self {
            bounds: Some(bounds),
            seek_ms: bounds.start_ms.saturating_sub(1),
            matrix: None,
        }
    }
}

pub(crate) fn scan<S: Series>(
    series: &S,
    input: NumericRangeInput,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), StorageError>,
) -> Result<Option<Progress>, StorageError> {
    if input.matrix.is_some() {
        return scan_matrix(series, input.bounds, input.matrix, control, account, visit);
    }
    let Some(bounds) = input.bounds else {
        return Ok(None);
    };
    // Go's buffer seeks the excluded left endpoint; returned pages retain included bounds.
    walk(
        series,
        bounds,
        WalkStart::Seek(input.seek_ms),
        control,
        account,
        |sample| {
            if !sample.value.is_stale() {
                account.samples_read(crate::statistics::weight(sample.value));
            }
            visit(sample)?;
            Ok(CursorAction::Next)
        },
    )
    .map(|_| None)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NumericRangeFunction {
    Deriv,
    PredictLinear,
    Smoothing,
    Mad,
    Quantile,
}
impl NumericRangeFunction {
    pub(crate) fn parameters(self) -> usize {
        match self {
            Self::Deriv | Self::Mad => 0,
            Self::PredictLinear | Self::Quantile => 1,
            Self::Smoothing => 2,
        }
    }
}

#[derive(Debug)]
pub(crate) struct NumericRangeOutput {
    pub(crate) float_samples: Option<crate::statistics::OwnedSamples>,
    pub(crate) value: Option<f64>,
    pub(crate) invalid_quantile: bool,
    pub(crate) annotation: Option<AnnotationKind>,
    pub(crate) nonempty: bool,
    pub(crate) progress: Option<Progress>,
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
    count: u64,
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
    }
}

#[derive(Default)]
struct Smoothing {
    count: u64,
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
                self.trend = source_sub(value, self.current);
            }
            let scaled = source_mul(value, sf);
            if self.count > 1 {
                let x = source_mul(source_sub(self.current, self.previous), tf);
                let y = source_mul(self.trend, source_sub(1.0, tf));
                self.trend = source_add(y, x);
            }
            let smoothed = source_mul(source_add(self.current, self.trend), source_sub(1.0, sf));
            self.previous = self.current;
            self.current = source_add(smoothed, scaled);
        }
        self.count += 1;
    }
}

enum Accumulator {
    Regression(Regression),
    Smoothing(Smoothing),
}

pub(crate) struct NumericStep {
    function: NumericRangeFunction,
    parameters: [f64; 2],
    time_ms: i64,
    invalid_factor: Option<(usize, f64)>,
    present: bool,
    histograms: bool,
    accumulator: Accumulator,
    _parameters_samples: crate::statistics::OwnedSamples,
    state_samples: Option<crate::statistics::OwnedSamples>,
}
impl NumericStep {
    pub(crate) fn new(
        function: NumericRangeFunction,
        parameters: [f64; 2],
        time_ms: i64,
        account: &QueryAccounting,
    ) -> Self {
        assert!(
            !matches!(
                function,
                NumericRangeFunction::Mad | NumericRangeFunction::Quantile
            ),
            "order statistics use rank replay"
        );
        Self {
            function,
            parameters,
            time_ms,
            invalid_factor: (function == NumericRangeFunction::Smoothing)
                .then(|| {
                    parameters
                        .into_iter()
                        .enumerate()
                        .find(|(_, value)| *value <= 0.0 || *value >= 1.0)
                })
                .flatten(),
            present: false,
            histograms: false,
            accumulator: if function == NumericRangeFunction::Smoothing {
                Accumulator::Smoothing(Smoothing::default())
            } else {
                Accumulator::Regression(Regression::default())
            },
            _parameters_samples: crate::statistics::OwnedSamples::new(
                account,
                function.parameters() as u64,
            ),
            state_samples: None,
        }
    }

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
        self.present = true;
        if self.invalid_factor.is_some() {
            return Ok(());
        }
        if let SampleValueRef::Float(value) = sample.value {
            account.work(1);
            match &mut self.accumulator {
                Accumulator::Smoothing(smoothing) => {
                    smoothing.add(value, self.parameters[0], self.parameters[1])
                }
                Accumulator::Regression(regression) => regression.add(
                    sample.time_ms,
                    value,
                    (self.function == NumericRangeFunction::PredictLinear).then_some(self.time_ms),
                ),
            }
            let units = match &self.accumulator {
                Accumulator::Smoothing(state) => state.count.min(2),
                Accumulator::Regression(_) => 1,
            };
            self.state_samples
                .get_or_insert_with(|| crate::statistics::OwnedSamples::new(account, 0))
                .resize(units);
        } else {
            self.histograms = true;
        }
        Ok(())
    }

    pub(crate) fn finish(
        mut self,
        account: &QueryAccounting,
    ) -> Result<NumericRangeOutput, NumericRangeError> {
        if self.present
            && let Some((index, value)) = self.invalid_factor
        {
            return Err(NumericRangeError::Factor {
                trend: index == 1,
                value,
            });
        }
        let count = match &self.accumulator {
            Accumulator::Regression(state) => state.count,
            Accumulator::Smoothing(state) => state.count,
        };
        let value = (count >= 2).then(|| match self.accumulator {
            Accumulator::Regression(regression) => {
                let (slope, intercept) = regression.finish();
                if self.function == NumericRangeFunction::Deriv {
                    slope
                } else {
                    source_add(intercept, source_mul(slope, self.parameters[0]))
                }
            }
            Accumulator::Smoothing(smoothing) => smoothing.current,
        });
        let state_samples = self.state_samples.take();
        let float_samples = if value.is_some() && self.function == NumericRangeFunction::Smoothing {
            let mut samples = state_samples.expect("completed smoothing state");
            samples.resize(1);
            Some(samples)
        } else {
            drop(state_samples);
            value.map(|_| crate::statistics::OwnedSamples::new(account, 1))
        };
        Ok(NumericRangeOutput {
            float_samples,
            progress: None,
            value,
            invalid_quantile: false,
            nonempty: self.present,
            annotation: (self.histograms && count > 0)
                .then_some(AnnotationKind::HistogramIgnoredInMixedRange),
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn evaluate<S: Series, Q: crate::storage::QueryProvider>(
    query: &Q,
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
    if function == NumericRangeFunction::Quantile {
        return crate::temporal_quantile::evaluate(
            query,
            series,
            input,
            parameters[0],
            control,
            account,
        );
    }
    let mut step = NumericStep::new(function, parameters, time_ms, account);
    let progress = if matches!(
        function,
        NumericRangeFunction::Deriv | NumericRangeFunction::PredictLinear
    ) && let Some(window) = input
        .matrix
        .filter(|window| window.can_reuse_acquisition(input.bounds))
    {
        let acquisition = crate::matrix_reader::probe::<_, StorageError>(
            series,
            input.bounds,
            window,
            control,
            account,
            |sample| step.receive(sample, control, account),
        )?;
        let progress = if acquisition.populated && acquisition.histograms_allocated {
            crate::matrix_reader::consume::<_, StorageError>(
                series,
                input.bounds,
                window.with_progress(Some(acquisition.progress)),
                control,
                account,
                crate::matrix_reader::ignore_sample::<StorageError>,
            )?
            .progress
        } else {
            account.samples_read(acquisition.progress.sample_units);
            acquisition.progress
        };
        Some(progress)
    } else {
        scan(series, input, control, account, |sample| {
            step.receive(sample, control, account)
        })?
    };
    let mut output = step.finish(account)?;
    output.progress = progress;
    Ok(output)
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

    #[test]
    fn streamed_numeric_initialization_preserves_u32_rollover() {
        let prefix = u64::from(u32::MAX);
        let mut regression = Regression {
            count: prefix,
            n: prefix as f64,
            initial: 1.0,
            constant: true,
            ..Regression::default()
        };
        regression.add(prefix as i64, 2.0, None);
        regression.add((prefix + 1) as i64, 1.0, None);
        assert_eq!(regression.count, prefix + 2);
        assert!(!regression.constant);

        let mut smoothing = Smoothing {
            count: prefix,
            previous: 1.0,
            current: 1.0,
            trend: 0.0,
        };
        smoothing.add(2.0, 0.5, 0.5);
        smoothing.add(2.0, 0.5, 0.5);
        assert_eq!(smoothing.count, prefix + 2);
        assert_eq!(smoothing.current.to_bits(), 1.875f64.to_bits());
    }

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

    fn acquisition_input(bounds: TimeBounds) -> NumericRangeInput {
        NumericRangeInput {
            bounds: Some(bounds),
            seek_ms: bounds.start_ms - 1,
            matrix: Some(MatrixWindow {
                first_end_ms: bounds.end_ms,
                end_ms: bounds.end_ms,
                range_ms: bounds.end_ms - bounds.start_ms + 1,
                buffer_ms: None,
                step_ms: 1000,
                data_bounds: bounds,
                statistics: false,
                reduce_delta: true,
                resume: None,
            }),
        }
    }

    fn acquisition_linear(index: usize) -> Sample {
        linear(index + 1)
    }
    fn acquisition_native(index: usize) -> Sample {
        Sample {
            value: SampleValue::FloatHistogram(FloatHistogram::default()),
            ..acquisition_linear(index)
        }
    }
    fn acquisition_last_native(index: usize) -> Sample {
        if index == 2 {
            acquisition_native(index)
        } else {
            acquisition_linear(index)
        }
    }
    fn acquisition_one_float(index: usize) -> Sample {
        if index == 0 {
            acquisition_linear(index)
        } else {
            acquisition_native(index)
        }
    }
    fn acquisition_stale(index: usize) -> Sample {
        Sample {
            value: SampleValue::Float(f64::from_bits(crate::value::STALE_NAN_BITS)),
            ..acquisition_linear(index)
        }
    }
    fn assert_acquisition_closed(source: &GeneratedSeries, account: &QueryAccounting) {
        assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
        assert_eq!(
            source.counters.opened.load(Ordering::Relaxed),
            source.counters.closed.load(Ordering::Relaxed)
        );
        assert_eq!(account.current_samples(), 0);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn regression_acquisition_preserves_selection_warnings_and_time_origin() {
        let bounds = TimeBounds {
            start_ms: 1,
            end_ms: 3000,
        };
        for function in [
            NumericRangeFunction::Deriv,
            NumericRangeFunction::PredictLinear,
        ] {
            for page in [1, 17, 1024] {
                for (count, sample, populated, warning, value, cursors) in [
                    (
                        3,
                        acquisition_linear as fn(usize) -> Sample,
                        true,
                        false,
                        true,
                        1,
                    ),
                    (1, acquisition_linear, true, false, false, 1),
                    (0, acquisition_linear, false, false, false, 1),
                    (3, acquisition_native, true, false, false, 2),
                    (3, acquisition_last_native, true, true, true, 2),
                    (3, acquisition_one_float, true, true, false, 2),
                    (3, acquisition_stale, false, false, false, 1),
                ] {
                    let account = EngineAccounting::default().query();
                    let source = GeneratedSeries::new(count, page, sample);
                    let result = evaluate(
                        &crate::test_storage::NoTemporary,
                        &source,
                        acquisition_input(bounds),
                        function,
                        [15.0, 0.0],
                        5000,
                        &Control::default(),
                        &account,
                    )
                    .unwrap();
                    // Prediction uses evaluation time 5s, not the selector endpoint 3s.
                    let expected: f64 = if function == NumericRangeFunction::Deriv {
                        2.0
                    } else {
                        47.0
                    };
                    assert_eq!(
                        result.value.map(f64::to_bits),
                        value.then_some(expected.to_bits())
                    );
                    assert_eq!(result.nonempty, populated);
                    assert_eq!(
                        result.annotation,
                        warning.then_some(AnnotationKind::HistogramIgnoredInMixedRange)
                    );
                    assert!(!result.invalid_quantile && result.progress.is_some());
                    assert_eq!(source.counters.opened.load(Ordering::Relaxed), cursors);
                    assert_eq!(account.current_samples(), u64::from(value));
                    drop(result);
                    assert_acquisition_closed(&source, &account);
                }
            }
        }
    }

    #[test]
    fn regression_acquisition_preserves_fallback_geometry() {
        for function in [
            NumericRangeFunction::Deriv,
            NumericRangeFunction::PredictLinear,
        ] {
            for mode in 0..6 {
                let mut input = acquisition_input(TimeBounds {
                    start_ms: 1,
                    end_ms: 3000,
                });
                match mode {
                    0 => {}
                    1 => input.matrix = None,
                    2 => input.matrix.as_mut().unwrap().statistics = true,
                    3 => input.matrix.as_mut().unwrap().buffer_ms = Some(5000),
                    4 => input.matrix.as_mut().unwrap().reduce_delta = false,
                    5 => input.bounds.as_mut().unwrap().start_ms = 0,
                    _ => unreachable!(),
                }
                let account = EngineAccounting::default().query();
                let source = GeneratedSeries::new(3, 1, acquisition_linear);
                let result = evaluate(
                    &crate::test_storage::NoTemporary,
                    &source,
                    input,
                    function,
                    [15.0, 0.0],
                    5000,
                    &Control::default(),
                    &account,
                )
                .unwrap();
                assert_eq!(
                    result.value,
                    Some(if function == NumericRangeFunction::Deriv {
                        2.0
                    } else {
                        47.0
                    })
                );
                assert_eq!(result.annotation, None);
                assert_eq!(result.progress.is_some(), mode != 1);
                assert_eq!(
                    source.counters.opened.load(Ordering::Relaxed),
                    if mode < 2 { 1 } else { 2 }
                );
                drop(result);
                assert_acquisition_closed(&source, &account);
            }
        }
    }

    #[test]
    fn regression_acquisition_provider_failures_release_provisional_state() {
        for function in [
            NumericRangeFunction::Deriv,
            NumericRangeFunction::PredictLinear,
        ] {
            for mode in 0..3 {
                let account = EngineAccounting::default().query();
                let mut source = GeneratedSeries::new(
                    3,
                    1,
                    if mode == 2 {
                        acquisition_last_native
                    } else {
                        acquisition_linear
                    },
                );
                if mode == 2 {
                    source.fail_acquisition = Some(2);
                } else {
                    source.fail_page = Some(if mode == 0 { 0 } else { 2 });
                }
                let result = evaluate(
                    &crate::test_storage::NoTemporary,
                    &source,
                    acquisition_input(TimeBounds {
                        start_ms: 1,
                        end_ms: 3000,
                    }),
                    function,
                    [15.0, 0.0],
                    5000,
                    &Control::default(),
                    &account,
                );
                assert!(matches!(
                    result,
                    Err(NumericRangeError::Storage(StorageError::Provider(_)))
                ));
                assert_eq!(account.snapshot().statistics.samples.samples_read, 0);
                assert_eq!(
                    source.counters.opened.load(Ordering::Relaxed),
                    if mode == 2 { 2 } else { 1 }
                );
                if mode > 0 {
                    assert_eq!(
                        account.snapshot().statistics.samples.peak_samples,
                        1 + u64::from(function == NumericRangeFunction::PredictLinear)
                    );
                }
                drop(result);
                assert_acquisition_closed(&source, &account);
            }
        }
    }

    #[test]
    fn regression_acquisition_cancellation_discards_unfinished_work() {
        for function in [
            NumericRangeFunction::Deriv,
            NumericRangeFunction::PredictLinear,
        ] {
            for page in [1, 17, 1024] {
                let account = EngineAccounting::default().query();
                let source = GeneratedSeries::new(2048, page, acquisition_linear);
                let completed = evaluate(
                    &crate::test_storage::NoTemporary,
                    &source,
                    acquisition_input(TimeBounds {
                        start_ms: 1,
                        end_ms: 2_048_000,
                    }),
                    function,
                    [15.0, 0.0],
                    2_048_000,
                    &Control::default(),
                    &account,
                )
                .unwrap();
                assert!(completed.value.is_some());
                let completed_work = account.snapshot().work;
                println!(
                    "regression cancellation function={function:?} page={page} completed_work={completed_work}"
                );
                drop(completed);
                assert_acquisition_closed(&source, &account);
                let account = EngineAccounting::default().query();
                let control = Control::default();
                let source = GeneratedSeries::new(2048, page, acquisition_linear);
                let result = crate::test_storage::interrupt_after_work(
                    &account,
                    &control,
                    completed_work / 2,
                    || {
                        evaluate(
                            &crate::test_storage::NoTemporary,
                            &source,
                            acquisition_input(TimeBounds {
                                start_ms: 1,
                                end_ms: 2_048_000,
                            }),
                            function,
                            [15.0, 0.0],
                            2_048_000,
                            &control,
                            &account,
                        )
                    },
                );
                assert!(matches!(
                    result,
                    Err(NumericRangeError::Storage(StorageError::Interrupted(
                        Interrupted::Cancelled
                    )))
                ));
                assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
                assert_eq!(account.snapshot().statistics.samples.samples_read, 0);
                assert_eq!(
                    account.snapshot().statistics.samples.peak_samples,
                    1 + u64::from(function == NumericRangeFunction::PredictLinear)
                );
                drop(result);
                assert_acquisition_closed(&source, &account);
            }
        }
    }

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
                    &crate::test_storage::NoTemporary,
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
                for matrix in [false, true] {
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
                            &crate::test_storage::NoTemporary,
                            &source,
                            if matrix {
                                acquisition_input(window(count))
                            } else {
                                window(count).into()
                            },
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
                            (NumericRangeFunction::Mad | NumericRangeFunction::Quantile, _) => {
                                unreachable!("regression test roster")
                            }
                        };
                        assert_eq!(result.value.unwrap().to_bits(), expected_bits);
                        assert_eq!(result.annotation, None);
                        assert_eq!(result.progress.is_some(), matrix);
                        assert_eq!(
                            source.counters.opened.load(Ordering::Relaxed),
                            if matrix && function == NumericRangeFunction::Smoothing {
                                2
                            } else {
                                1
                            }
                        );
                        let snapshot = account.snapshot();
                        assert_eq!(
                            snapshot.statistics.samples.peak_samples,
                            match function {
                                NumericRangeFunction::Deriv => 1,
                                NumericRangeFunction::PredictLinear => 2,
                                NumericRangeFunction::Smoothing => 4,
                                _ => unreachable!("regression test roster"),
                            }
                        );
                        assert_eq!(account.current_samples(), 1);
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
                        drop(result);
                        assert_eq!(account.current_samples(), 0);
                        drop(account);
                        engine.snapshot().assert_memory_released();
                    }
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
                &crate::test_storage::NoTemporary,
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
                &crate::test_storage::NoTemporary,
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
                    &crate::test_storage::NoTemporary,
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
                    &crate::test_storage::NoTemporary,
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
            // Four two-pointer address ranges are retained; histogram payloads are borrowed.
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].peak,
                size_of::<[(usize, usize); 4]>() as u64
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
                        &crate::test_storage::NoTemporary,
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
                &crate::test_storage::NoTemporary,
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
                    &crate::test_storage::NoTemporary,
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
                    &crate::test_storage::NoTemporary,
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
