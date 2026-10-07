// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus promql/functions.go:452-758,830-964; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    annotations::{AnnotationKind, HistogramOperation},
    histogram::{CUSTOM_BUCKETS_SCHEMA, FloatHistogram, FloatHistogramRef, GAUGE, HistogramError},
    histogram_counter::detect_reset_with,
    histogram_work::{HistogramEvaluationError, HistogramWork},
    matrix_reader::{MatrixWindow, scan as scan_matrix},
    range_float::{
        Extrapolation, RateInput, RateKind, RateValue, RateWindow, extrapolation_factor, rate_with,
        start_time_overlap, start_timestamp_reset, uses_first_start,
    },
    result::{OwnedHistogram, OwnedValue},
    storage::{Control, Series, StorageError},
    value::{SampleRef, SampleValueRef},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RateFunction {
    Extrapolated(RateKind),
    Instant { counter: bool },
}
impl RateFunction {
    pub(crate) fn checks_counter_name(self) -> bool {
        matches!(
            self,
            Self::Extrapolated(RateKind::Rate | RateKind::Increase)
        )
    }
}

#[derive(Default)]
pub(crate) struct RateOutput {
    pub(crate) value: Option<OwnedValue>,
    pub(crate) annotations: [Option<AnnotationKind>; 6],
}
impl RateOutput {
    fn note(&mut self, kind: AnnotationKind) {
        if self.annotations.contains(&Some(kind)) {
            return;
        }
        *self
            .annotations
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("source rate annotation set") = Some(kind);
    }
    fn only(kind: AnnotationKind) -> Self {
        let mut result = Self::default();
        result.note(kind);
        result
    }
}

pub(crate) fn evaluate<S: Series>(
    series: &S,
    input: impl Into<RateInput>,
    function: RateFunction,
    matrix: Option<MatrixWindow>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<RateOutput, StorageError> {
    let input = input.into();
    let RateFunction::Extrapolated(kind) = function else {
        let RateFunction::Instant { counter } = function else {
            unreachable!()
        };
        return instant(series, input, counter, matrix, control, account);
    };
    let result = rate_with(series, input, kind, matrix, control, account)?;
    let mut output = RateOutput::default();
    if result.start_time_overlap {
        output.note(AnnotationKind::StartTimeOverlap);
    }
    output.value = match result.value {
        RateValue::Float(value) => Some(OwnedValue::Float(value)),
        RateValue::Empty => None,
        RateValue::Mixed => {
            output.note(AnnotationKind::MixedFloatsHistograms);
            None
        }
        RateValue::Histogram => {
            return extrapolated_histogram(series, input, kind, matrix, control, account);
        }
    };
    Ok(output)
}

fn arithmetic_error(error: HistogramEvaluationError) -> StorageError {
    match error {
        HistogramEvaluationError::Interrupted(error) => error.into(),
        HistogramEvaluationError::Arithmetic(error) => panic!("{error}"),
    }
}

fn own_histogram(
    value: SampleValueRef<'_>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<OwnedHistogram, StorageError> {
    let OwnedValue::Histogram(value) =
        OwnedValue::copy_from_checked(value, control, account, MemoryKind::Workspace)?
    else {
        return Err(StorageError::Invalid(
            "provider replay changed sample kinds",
        ));
    };
    Ok(value)
}

struct HistogramPoint {
    time: i64,
    start: i64,
    value: OwnedHistogram,
}
impl HistogramPoint {
    fn copy(
        sample: SampleRef<'_>,
        window: RateWindow,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, StorageError> {
        Ok(Self {
            time: sample.time_ms,
            start: if window.start_timestamps {
                sample.start_time_ms
            } else {
                0
            },
            value: own_histogram(sample.value, control, account)?,
        })
    }
}

fn extrapolated_histogram<S: Series>(
    series: &S,
    input: RateInput,
    kind: RateKind,
    matrix: Option<MatrixWindow>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<RateOutput, StorageError> {
    let window = input.window;
    let bounds = input.bounds();
    let mut first = None;
    let mut second = None;
    let mut last = None::<HistogramPoint>;
    let mut count = 0usize;
    scan_matrix::<_, StorageError>(series, bounds, matrix, control, account, |sample| {
        if sample.value.is_stale() {
            return Ok(());
        }
        count += 1;
        match count {
            1 => first = Some(HistogramPoint::copy(sample, window, control, account)?),
            2 => second = Some(HistogramPoint::copy(sample, window, control, account)?),
            _ => {
                if let Some(last) = &mut last {
                    last.time = sample.time_ms;
                    last.start = if window.start_timestamps {
                        sample.start_time_ms
                    } else {
                        0
                    };
                    last.value
                        .copy_from_sample_checked(sample.value, control, account)?;
                } else {
                    last = Some(HistogramPoint::copy(sample, window, control, account)?);
                }
            }
        }
        Ok(())
    })?;
    let Some(first) = first else {
        return Ok(RateOutput::default());
    };
    let last = last.as_ref().or(second.as_ref()).unwrap_or(&first);
    let counter = kind != RateKind::Delta;
    let work = HistogramWork::new(control, account);
    let mut output = RateOutput::default();
    let mut value = if count > 1 {
        if counter
            && (first.value.as_ref().reset_hint == GAUGE || last.value.as_ref().reset_hint == GAUGE)
        {
            output.note(AnnotationKind::NativeHistogramNotCounter);
        }
        let second = second.as_ref().expect("second native endpoint");
        let mut zero = None;
        if counter
            && (start_timestamp_reset(first.start, first.time, second.start, second.time)
                || detect_reset_with(second.value.as_ref(), first.value.as_ref(), work)?)
        {
            let empty = FloatHistogram::default();
            zero = Some(own_histogram(
                SampleValueRef::FloatHistogram(FloatHistogramRef {
                    schema: second.value.as_ref().schema,
                    custom_values: second.value.as_ref().custom_values,
                    ..empty.as_ref()
                }),
                control,
                account,
            )?);
        }
        let relevant_first = zero.as_ref().unwrap_or(&first.value);
        let custom = relevant_first.as_ref().schema == CUSTOM_BUCKETS_SCHEMA;
        if (last.value.as_ref().schema == CUSTOM_BUCKETS_SCHEMA) != custom {
            output.note(AnnotationKind::MixedExponentialCustomHistograms);
            return Ok(output);
        }
        let mut schema = last
            .value
            .as_ref()
            .schema
            .min(relevant_first.as_ref().schema);
        if counter {
            let mut index = 0usize;
            let mut mismatch = false;
            scan_matrix::<_, StorageError>(series, bounds, matrix, control, account, |sample| {
                if sample.value.is_stale() {
                    return Ok(());
                }
                let interior = index > 0 && index < count - 1;
                index += 1;
                if !interior || mismatch {
                    return Ok(());
                }
                let (current_schema, hint) = match sample.value {
                    SampleValueRef::FloatHistogram(h) => (h.schema, h.reset_hint),
                    SampleValueRef::IntegerHistogram(h) => (h.schema, h.reset_hint),
                    SampleValueRef::Float(_) => {
                        return Err(StorageError::Invalid(
                            "provider replay changed sample kinds",
                        ));
                    }
                };
                if hint == GAUGE {
                    output.note(AnnotationKind::NativeHistogramNotCounter);
                }
                schema = schema.min(current_schema);
                mismatch = (current_schema == CUSTOM_BUCKETS_SCHEMA) != custom;
                Ok(())
            })?;
            if mismatch {
                return Ok(RateOutput::only(
                    AnnotationKind::MixedExponentialCustomHistograms,
                ));
            }
        }
        let mut value = last
            .value
            .copy_to_schema_checked(schema, control, account)
            .map_err(arithmetic_error)?;
        match value.arithmetic_checked(relevant_first.as_ref(), true, control, account) {
            Ok(flags) => {
                if flags.nhcb_bounds_reconciled {
                    output.note(AnnotationKind::MismatchedCustomBuckets(
                        HistogramOperation::Subtraction,
                    ));
                }
            }
            Err(HistogramEvaluationError::Arithmetic(HistogramError::IncompatibleSchema)) => {
                return Ok(RateOutput::only(
                    AnnotationKind::MixedExponentialCustomHistograms,
                ));
            }
            Err(error) => return Err(arithmetic_error(error)),
        }
        if counter {
            let mut index = 0usize;
            let mut previous = None::<OwnedHistogram>;
            let mut previous_time = first.time;
            let mut previous_start = first.start;
            let mut mismatch = false;
            scan_matrix::<_, StorageError>(series, bounds, matrix, control, account, |sample| {
                if sample.value.is_stale() {
                    return Ok(());
                }
                index += 1;
                if index == 1 || mismatch {
                    return Ok(());
                }
                let start = if window.start_timestamps {
                    sample.start_time_ms
                } else {
                    0
                };
                if start_time_overlap(previous_start, previous_time, start) {
                    output.note(AnnotationKind::StartTimeOverlap);
                }
                let mut converted = None;
                let current = match sample.value {
                    SampleValueRef::FloatHistogram(h) => h,
                    SampleValueRef::IntegerHistogram(_) => {
                        converted = Some(own_histogram(sample.value, control, account)?);
                        converted.as_ref().unwrap().as_ref()
                    }
                    SampleValueRef::Float(_) => {
                        return Err(StorageError::Invalid(
                            "provider replay changed sample kinds",
                        ));
                    }
                };
                let prior = previous.as_ref().unwrap_or(relevant_first).as_ref();
                if start_timestamp_reset(previous_start, previous_time, start, sample.time_ms)
                    || detect_reset_with(current, prior, work)?
                {
                    match value.arithmetic_checked(prior, false, control, account) {
                        Ok(flags) => {
                            if flags.nhcb_bounds_reconciled {
                                output.note(AnnotationKind::MismatchedCustomBuckets(
                                    HistogramOperation::Addition,
                                ));
                            }
                        }
                        Err(HistogramEvaluationError::Arithmetic(
                            HistogramError::IncompatibleSchema,
                        )) => mismatch = true,
                        Err(error) => return Err(arithmetic_error(error)),
                    }
                }
                if let Some(converted) = converted {
                    previous = Some(converted);
                } else if let Some(previous) = &mut previous {
                    previous.copy_from_sample_checked(sample.value, control, account)?;
                } else {
                    previous = Some(own_histogram(sample.value, control, account)?);
                }
                previous_time = sample.time_ms;
                previous_start = start;
                Ok(())
            })?;
            if mismatch {
                return Ok(RateOutput::only(
                    AnnotationKind::MixedExponentialCustomHistograms,
                ));
            }
        } else if first.value.as_ref().reset_hint != GAUGE
            || last.value.as_ref().reset_hint != GAUGE
        {
            output.note(AnnotationKind::NativeHistogramNotGauge);
        }
        value.set_reset_hint(GAUGE);
        value.compact_checked(control, account)?;
        Some(value)
    } else {
        None
    };
    if uses_first_start(input, kind, first.time, first.start) {
        if let Some(value) = &mut value {
            match value.arithmetic_checked(first.value.as_ref(), false, control, account) {
                Ok(flags) => {
                    if flags.nhcb_bounds_reconciled {
                        output.note(AnnotationKind::MismatchedCustomBuckets(
                            HistogramOperation::Addition,
                        ));
                    }
                }
                Err(HistogramEvaluationError::Arithmetic(HistogramError::IncompatibleSchema)) => {
                    output.note(AnnotationKind::MixedExponentialCustomHistograms);
                    return Ok(output);
                }
                Err(error) => return Err(arithmetic_error(error)),
            }
        } else {
            value = Some(
                first
                    .value
                    .copy_to_schema_checked(first.value.as_ref().schema, control, account)
                    .map_err(arithmetic_error)?,
            );
        }
    }
    let Some(mut value) = value else {
        return Ok(output);
    };
    let Some(factor) = extrapolation_factor(
        input,
        kind,
        Extrapolation {
            first_time: first.time,
            last_time: last.time,
            first_start: first.start,
            count,
            first_value: first.value.as_ref().count,
            result_value: value.as_ref().count,
        },
    ) else {
        return Ok(output);
    };
    value.mul_checked(factor, control, account)?;
    output.value = Some(OwnedValue::Histogram(value));
    Ok(output)
}

struct InstantPoint {
    time: i64,
    start: i64,
    value: OwnedValue,
}
fn instant<S: Series>(
    series: &S,
    input: RateInput,
    counter: bool,
    matrix: Option<MatrixWindow>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<RateOutput, StorageError> {
    let window = input.window;
    let bounds = input.bounds();
    let mut first = None;
    let mut last = None;
    scan_matrix::<_, StorageError>(series, bounds, matrix, control, account, |sample| {
        if sample.value.is_stale() {
            return Ok(());
        }
        first = last.take();
        last = Some(InstantPoint {
            time: sample.time_ms,
            start: if window.start_timestamps {
                sample.start_time_ms
            } else {
                0
            },
            value: OwnedValue::copy_from_checked(
                sample.value,
                control,
                account,
                MemoryKind::Workspace,
            )?,
        });
        Ok(())
    })?;
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(RateOutput::default());
    };
    let interval = last.time.wrapping_sub(first.time);
    if interval == 0 {
        return Ok(RateOutput::default());
    }
    let mut output = RateOutput::default();
    output.value = match (&first.value, &last.value) {
        (OwnedValue::Float(first_value), OwnedValue::Float(last_value)) => {
            let reset = counter
                && (*last_value < *first_value
                    || start_timestamp_reset(first.start, first.time, last.start, last.time));
            let mut value = if reset {
                *last_value
            } else {
                *last_value - *first_value
            };
            if counter {
                value /= interval as f64 / 1000.0;
            }
            Some(OwnedValue::Float(value))
        }
        (OwnedValue::Histogram(first_value), OwnedValue::Histogram(last_value)) => {
            let first_h = first_value.as_ref();
            let last_h = last_value.as_ref();
            let mut value = last_value
                .copy_to_schema_checked(last_h.schema, control, account)
                .map_err(arithmetic_error)?;
            if counter && (first_h.reset_hint == GAUGE || last_h.reset_hint == GAUGE) {
                output.note(AnnotationKind::NativeHistogramNotCounter);
            } else if !counter && (first_h.reset_hint != GAUGE || last_h.reset_hint != GAUGE) {
                output.note(AnnotationKind::NativeHistogramNotGauge);
            }
            if !counter
                || !(start_timestamp_reset(first.start, first.time, last.start, last.time)
                    || detect_reset_with(last_h, first_h, HistogramWork::new(control, account))?)
            {
                match value.arithmetic_checked(first_h, true, control, account) {
                    Ok(flags) => {
                        if flags.nhcb_bounds_reconciled {
                            output.note(AnnotationKind::MismatchedCustomBuckets(
                                HistogramOperation::Subtraction,
                            ));
                        }
                    }
                    Err(HistogramEvaluationError::Arithmetic(
                        HistogramError::IncompatibleSchema,
                    )) => {
                        output.note(AnnotationKind::MixedExponentialCustomHistograms);
                        return Ok(output);
                    }
                    Err(error) => return Err(arithmetic_error(error)),
                }
            }
            value.set_reset_hint(GAUGE);
            value.compact_checked(control, account)?;
            if counter {
                value.div_checked(interval as f64 / 1000.0, control, account)?;
            }
            Some(OwnedValue::Histogram(value))
        }
        _ => {
            output.note(AnnotationKind::MixedFloatsHistograms);
            None
        }
    };
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{NOT_RESET, Span},
        storage::TimeBounds,
        test_storage::GeneratedSeries,
        value::{Sample, SampleValue},
    };
    use std::sync::atomic::Ordering;

    fn native(index: usize) -> Sample {
        let value = index as f64 + 1.0;
        Sample {
            time_ms: (index as i64 + 1) * 1000,
            start_time_ms: 0,
            value: SampleValue::FloatHistogram(FloatHistogram {
                reset_hint: NOT_RESET,
                count: 3.0 * value,
                sum: 5.0 * value,
                positive_spans: Some(
                    vec![Span {
                        offset: 0,
                        length: 2,
                    }]
                    .into(),
                ),
                positive_buckets: Some(vec![value, 2.0 * value].into()),
                ..FloatHistogram::default()
            }),
        }
    }
    fn window(count: usize) -> RateWindow {
        RateWindow {
            start_ms: 0,
            end_ms: count as i64 * 1000,
            range_ns: count as i64 * 1_000_000_000,
            start_timestamps: false,
        }
    }

    #[test]
    fn native_rate_history_length_keeps_constant_workspace_and_one_cursor() {
        for page in [1, 17, 1024] {
            for function in [
                RateFunction::Extrapolated(RateKind::Rate),
                RateFunction::Extrapolated(RateKind::Increase),
                RateFunction::Extrapolated(RateKind::Delta),
                RateFunction::Instant { counter: true },
                RateFunction::Instant { counter: false },
            ] {
                let mut previous_peak = None;
                for count in [2048, 100_000] {
                    let source = GeneratedSeries::new(count, page, native);
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let result = evaluate(
                        &source,
                        window(count),
                        function,
                        None,
                        &Control::default(),
                        &account,
                    )
                    .unwrap();
                    let Some(OwnedValue::Histogram(value)) = &result.value else {
                        panic!("native result required");
                    };
                    let interval = (count - 1) as f64;
                    let (increment, factor) = match function {
                        RateFunction::Extrapolated(kind) => {
                            let to_zero = interval * (3.0 / (3.0 * interval));
                            let to_start = if kind != RateKind::Delta && to_zero < 1.0 {
                                to_zero
                            } else {
                                1.0
                            };
                            let mut factor = (interval + to_start) / interval;
                            if kind == RateKind::Rate {
                                factor /= count as f64;
                            }
                            (interval, factor)
                        }
                        RateFunction::Instant { .. } => (1.0, 1.0),
                    };
                    assert_eq!(
                        value.as_ref().count.to_bits(),
                        (3.0 * increment * factor).to_bits()
                    );
                    assert_eq!(
                        value.as_ref().sum.to_bits(),
                        (5.0 * increment * factor).to_bits()
                    );
                    assert_eq!(
                        value.as_ref().positive_buckets.unwrap(),
                        &[increment * factor, 2.0 * increment * factor]
                    );
                    assert_eq!(value.as_ref().reset_hint, GAUGE);
                    let snapshot = account.snapshot();
                    assert!(snapshot.work > snapshot.samples_fetched);
                    assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                    assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                    assert_eq!(
                        source.counters.opened.load(Ordering::Relaxed),
                        source.counters.closed.load(Ordering::Relaxed)
                    );
                    let peak = snapshot.memory[MemoryKind::Workspace as usize].peak;
                    if let Some(previous_peak) = previous_peak {
                        assert_eq!(peak, previous_peak);
                    }
                    previous_peak = Some(peak);
                    assert_eq!(
                        snapshot.memory[MemoryKind::BorrowedView as usize].current,
                        0
                    );
                    assert_eq!(
                        snapshot.memory[MemoryKind::ProviderHeld as usize].current,
                        0
                    );
                    assert!(snapshot.memory[MemoryKind::Workspace as usize].current > 0);
                    drop(account);
                    drop(result);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn native_singleton_start_timestamp_retains_hint_and_uncompacted_buckets() {
        fn sample(_: usize) -> Sample {
            Sample {
                time_ms: 5000,
                start_time_ms: 3000,
                value: SampleValue::FloatHistogram(FloatHistogram {
                    reset_hint: NOT_RESET,
                    count: 7.0,
                    sum: 14.0,
                    positive_spans: Some(
                        vec![Span {
                            offset: 0,
                            length: 3,
                        }]
                        .into(),
                    ),
                    positive_buckets: Some(vec![3.0, 0.0, 4.0].into()),
                    ..FloatHistogram::default()
                }),
            }
        }
        for page in [1, 17, 1024] {
            let source = GeneratedSeries::new(1, page, sample);
            let account = EngineAccounting::default().query();
            let mut window = window(10);
            let function = RateFunction::Extrapolated(RateKind::Rate);
            assert!(
                evaluate(
                    &source,
                    window,
                    function,
                    None,
                    &Control::default(),
                    &account
                )
                .unwrap()
                .value
                .is_none()
            );
            window.start_timestamps = true;
            let result = evaluate(
                &source,
                window,
                function,
                None,
                &Control::default(),
                &account,
            )
            .unwrap();
            assert!(result.annotations.iter().all(Option::is_none));
            let Some(OwnedValue::Histogram(value)) = result.value else {
                panic!("native singleton");
            };
            assert_eq!(value.as_ref().reset_hint, NOT_RESET);
            assert_eq!(value.as_ref().positive_spans.unwrap()[0].length, 3);
            assert_eq!(
                value.as_ref().positive_buckets.unwrap(),
                &[3.0 * 0.1, 0.0, 4.0 * 0.1]
            );
            assert_eq!(value.as_ref().count.to_bits(), (7.0f64 * 0.1).to_bits());
            drop(value);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn native_rate_failure_and_cancellation_release_every_owner() {
        for function in [
            RateFunction::Extrapolated(RateKind::Rate),
            RateFunction::Instant { counter: true },
        ] {
            let mut source = GeneratedSeries::new(100, 2, native);
            source.fail_page = Some(1);
            let account = EngineAccounting::default().query();
            let matrix = Some(MatrixWindow {
                first_end_ms: window(100).end_ms,
                end_ms: window(100).end_ms,
                statistics: true,
                reduce_delta: true,
                range_ms: window(100).end_ms,
                step_ms: 1,
                data_bounds: TimeBounds {
                    start_ms: 0,
                    end_ms: window(100).end_ms,
                },
            });
            assert!(matches!(
                evaluate(
                    &source,
                    window(100),
                    function,
                    matrix,
                    &Control::default(),
                    &account
                ),
                Err(StorageError::Provider(_))
            ));
            assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
            account.snapshot().assert_memory_released();
            let control = Control::default();
            control.cancellation.cancel();
            assert!(matches!(
                evaluate(&source, window(100), function, None, &control, &account),
                Err(StorageError::Interrupted(_))
            ));
            assert_eq!(
                source.counters.opened.load(Ordering::Relaxed),
                source.counters.closed.load(Ordering::Relaxed)
            );
            account.snapshot().assert_memory_released();
        }
    }
}
