// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus functions.go:71-442 and engine.go:1770-1879,2891-2957,4946-4993; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    annotations::{AnnotationKind, HistogramOperation},
    histogram::{CUSTOM_BUCKETS_SCHEMA, FloatHistogramRef, GAUGE, HistogramError},
    histogram_counter::detect_reset_with,
    histogram_work::{HistogramEvaluationError, HistogramWork},
    matrix_reader::{Acquisition, MatrixWindow, acquire},
    numerical::{source_add, source_div, source_mul, source_sub},
    range_float::{RateInput, RateKind},
    range_rates::RateOutput,
    result::{OwnedHistogram, OwnedValue},
    storage::{Control, Interrupted, Series, StorageError},
    value::{SampleRef, SampleValueRef},
};

struct Reader<'a, S> {
    series: &'a S,
    input: RateInput,
    matrix: MatrixWindow,
    control: &'a Control,
    account: &'a QueryAccounting,
}
impl<S: Series> Reader<'_, S> {
    fn walk<E: From<StorageError> + From<Interrupted>>(
        &self,
        mut visit: impl for<'a> FnMut(u64, SampleRef<'a>) -> Result<(), E>,
    ) -> Result<Acquisition, E> {
        let mut position = 0;
        acquire::<_, E>(
            self.series,
            self.input.bounds(),
            self.matrix,
            self.control,
            self.account,
            |sample| {
                self.control.check()?;
                self.account.work(1);
                visit(position, sample)?;
                position += 1;
                Ok(())
            },
        )
    }

    fn point(&self, index: u64) -> Result<Point, StorageError> {
        let mut point = None;
        self.walk::<StorageError>(|position, sample| {
            if position == index {
                let value = crate::statistics::ValueParts::new(
                    OwnedValue::from_sample_checked(
                        sample.value,
                        self.control,
                        self.account,
                        MemoryKind::Workspace,
                    )?,
                    self.account,
                );
                point = Some(Point {
                    time: sample.time_ms,
                    value: value.value,
                    float_samples: value.float_samples,
                });
            }
            Ok(())
        })?;
        point.ok_or(StorageError::Invalid(
            "provider replay changed sample count",
        ))
    }

    fn describe(&self) -> Result<Description, StorageError> {
        let mut result = Description::default();
        let acquisition = self.walk::<StorageError>(|index, sample| {
            result.floats |= matches!(sample.value, SampleValueRef::Float(_));
            result.histograms |= !matches!(sample.value, SampleValueRef::Float(_));
            if result.after_start.is_none() && sample.time_ms > self.input.range_start_ms {
                result.after_start = Some((index, result.last_time.unwrap_or(sample.time_ms)));
            }
            if result.at_end.is_none() && sample.time_ms >= self.input.range_end_ms {
                result.at_end = Some((index, sample.time_ms));
            }
            result.penultimate_time = result.last_time;
            result.last_time = Some(sample.time_ms);
            result.count += 1;
            Ok(())
        })?;
        result.histograms_allocated = acquisition.histograms_allocated;
        result.progress = Some(acquisition.progress);
        Ok(result)
    }

    fn validate_native(
        &self,
        selected: Selected,
        counter: bool,
        output: &mut RateOutput,
    ) -> Result<bool, StorageError> {
        let mut custom = None;
        let mut valid = true;
        self.walk::<StorageError>(|index, sample| {
            if !valid || index < selected.first || index > selected.last {
                return Ok(());
            }
            let (schema, hint) = match sample.value {
                SampleValueRef::FloatHistogram(h) => (h.schema, h.reset_hint),
                SampleValueRef::IntegerHistogram(h) => (h.schema, h.reset_hint),
                SampleValueRef::Float(_) => return Err(changed_kind()),
            };
            let uses_custom = schema == CUSTOM_BUCKETS_SCHEMA;
            if custom.is_some_and(|first| first != uses_custom) {
                output.note(AnnotationKind::MixedExponentialCustomHistograms);
                valid = false;
                return Ok(());
            }
            custom = Some(uses_custom);
            if counter && hint == GAUGE {
                output.note(AnnotationKind::NativeHistogramNotCounter);
            }
            Ok(())
        })?;
        Ok(valid)
    }

    fn boundary(
        &self,
        index: u64,
        left: bool,
        smoothed: bool,
        counter: bool,
        output: &mut RateOutput,
    ) -> Result<Option<crate::statistics::ValueParts>, StorageError> {
        let mut point = self.point(index)?;
        let time = if left {
            self.input.range_start_ms
        } else {
            self.input.range_end_ms
        };
        if smoothed && left && point.time < time {
            return self.interpolate(point, self.point(index + 1)?, time, counter, output);
        }
        if smoothed && !left && index > 0 && point.time > time {
            return self.interpolate(self.point(index - 1)?, point, time, counter, output);
        }
        // Original operands retain their fields until the source's Copy site.
        if let OwnedValue::Histogram(value) = &mut point.value {
            value.normalize_copy_fields();
        }
        Ok(Some(point.into_parts()))
    }

    fn interpolate(
        &self,
        mut first: Point,
        mut second: Point,
        time: i64,
        counter: bool,
        output: &mut RateOutput,
    ) -> Result<Option<crate::statistics::ValueParts>, StorageError> {
        if matches!(first.value, OwnedValue::Histogram(_)) {
            let exact = if time == first.time {
                Some(&mut first.value)
            } else if time == second.time {
                Some(&mut second.value)
            } else {
                None
            };
            if let Some(OwnedValue::Histogram(value)) = exact {
                value.normalize_copy_fields();
                return Ok(Some(if time == first.time {
                    first.into_parts()
                } else {
                    second.into_parts()
                }));
            }
        }
        let elapsed = time.wrapping_sub(first.time) as f64;
        let interval = second.time.wrapping_sub(first.time) as f64;
        match (first.value, second.value) {
            (OwnedValue::Float(mut first), OwnedValue::Float(second)) => {
                if counter && second < first {
                    first = 0.0;
                }
                // Division follows multiplication, as in the original scalar interpolation.
                let value = source_add(
                    first,
                    source_div(source_mul(source_sub(second, first), elapsed), interval),
                );
                Ok(Some(crate::statistics::ValueParts::new(
                    OwnedValue::Float(value),
                    self.account,
                )))
            }
            (OwnedValue::Histogram(first), OwnedValue::Histogram(mut second)) => {
                let fraction = source_div(elapsed, interval);
                let reset = counter
                    && detect_reset_with(
                        second.as_ref(),
                        first.as_ref(),
                        HistogramWork::new(self.control, self.account),
                    )?;
                second.normalize_copy_fields();
                if !reset && !arithmetic(&mut second, first.as_ref(), true, output, self)? {
                    return Ok(None);
                }
                second.mul_checked(fraction, self.control, self.account)?;
                if !reset && !arithmetic(&mut second, first.as_ref(), false, output, self)? {
                    return Ok(None);
                }
                Ok(Some(crate::statistics::ValueParts {
                    value: OwnedValue::Histogram(second),
                    float_samples: None,
                }))
            }
            _ => Err(changed_kind()),
        }
    }

    fn float_correction(
        &self,
        selected: Selected,
        left: f64,
        right: f64,
    ) -> Result<f64, StorageError> {
        let first = selected.first + u64::from(selected.first_time <= self.input.range_start_ms);
        let end = selected.last + u64::from(selected.last_time < self.input.range_end_ms);
        assert!(first <= end, "source float correction slice");
        let mut previous = left;
        let _previous_samples = crate::statistics::OwnedSamples::new(self.account, 1);
        let mut correction = 0.0;
        self.walk::<StorageError>(|index, sample| {
            if index < first || index >= end {
                return Ok(());
            }
            let SampleValueRef::Float(value) = sample.value else {
                return Err(changed_kind());
            };
            if value < previous {
                correction = source_add(correction, previous);
            }
            previous = value;
            Ok(())
        })?;
        if right < previous {
            correction = source_add(correction, previous);
        }
        Ok(correction)
    }

    fn native_correction(
        &self,
        selected: Selected,
        left: &OwnedHistogram,
        right: &OwnedHistogram,
        smoothed: bool,
        output: &mut RateOutput,
    ) -> Result<Correction, StorageError> {
        let mut first = selected.first + 1;
        let mut previous = None;
        if smoothed && selected.first_time < self.input.range_start_ms {
            let a = self.point(selected.first)?;
            let b = self.point(selected.first + 1)?;
            let (OwnedValue::Histogram(a), OwnedValue::Histogram(b)) = (a.value, b.value) else {
                return Err(changed_kind());
            };
            if detect_reset_with(
                b.as_ref(),
                a.as_ref(),
                HistogramWork::new(self.control, self.account),
            )? {
                first += 1;
                previous = Some(b);
            }
        }
        if first > selected.last {
            return Ok(Correction::Complete(None));
        }
        let mut correction = None;
        let mut valid = true;
        self.walk::<StorageError>(|index, sample| {
            if !valid || index < first || index >= selected.last {
                return Ok(());
            }
            let mut converted = None;
            let current = match sample.value {
                SampleValueRef::FloatHistogram(value) => value,
                SampleValueRef::IntegerHistogram(_) => {
                    converted = Some(retain_histogram(sample.value, self)?);
                    converted.as_ref().unwrap().as_ref()
                }
                SampleValueRef::Float(_) => return Err(changed_kind()),
            };
            let prior = previous.as_ref().unwrap_or(left).as_ref();
            if detect_reset_with(
                current,
                prior,
                HistogramWork::new(self.control, self.account),
            )? {
                valid = add_correction(&mut correction, prior, output, self)?;
            }
            if !valid {
                return Ok(());
            }
            if let Some(converted) = converted {
                previous = Some(converted);
            } else {
                // Pointer retention must not apply the source's separate Copy normalization.
                previous = Some(retain_histogram(sample.value, self)?);
            }
            Ok(())
        })?;
        if valid {
            let prior = previous.as_ref().unwrap_or(left).as_ref();
            if detect_reset_with(
                right.as_ref(),
                prior,
                HistogramWork::new(self.control, self.account),
            )? {
                valid = add_correction(&mut correction, prior, output, self)?;
            }
        }
        if !valid {
            return Ok(Correction::Omit);
        }
        Ok(Correction::Complete(correction))
    }
}

struct Point {
    time: i64,
    value: OwnedValue,
    float_samples: Option<crate::statistics::OwnedSamples>,
}
impl Point {
    fn into_parts(self) -> crate::statistics::ValueParts {
        crate::statistics::ValueParts {
            value: self.value,
            float_samples: self.float_samples,
        }
    }
}

enum Correction {
    Complete(Option<OwnedHistogram>),
    Omit,
}

#[derive(Default)]
struct Description {
    progress: Option<crate::matrix_reader::Progress>,
    count: u64,
    floats: bool,
    histograms: bool,
    histograms_allocated: bool,
    after_start: Option<(u64, i64)>,
    at_end: Option<(u64, i64)>,
    penultimate_time: Option<i64>,
    last_time: Option<i64>,
}
#[derive(Clone, Copy)]
struct Selected {
    first: u64,
    last: u64,
    first_time: i64,
    last_time: i64,
}
impl Description {
    fn endpoints(&self, smoothed: bool) -> Option<Selected> {
        let last = self.count.checked_sub(1)?;
        let first = self
            .after_start
            .map_or(last, |(index, _)| index.min(last))
            .saturating_sub(1);
        let first_time = self.after_start.map_or_else(
            || self.penultimate_time.or(self.last_time).unwrap(),
            |(_, time)| time,
        );
        let (last, last_time) = if smoothed {
            self.at_end.unwrap_or((last, self.last_time.unwrap()))
        } else {
            (last, self.last_time.unwrap())
        };
        Some(Selected {
            first,
            last,
            first_time,
            last_time,
        })
    }
    fn select(&self, input: RateInput, smoothed: bool) -> Option<Selected> {
        let selected = self.endpoints(smoothed)?;
        (selected.last_time > input.range_start_ms
            && (!smoothed || selected.first_time <= input.range_end_ms))
            .then_some(selected)
    }
}

fn changed_kind() -> StorageError {
    StorageError::Invalid("provider replay changed sample kinds")
}

fn retain_histogram<S: Series>(
    value: SampleValueRef<'_>,
    reader: &Reader<'_, S>,
) -> Result<OwnedHistogram, StorageError> {
    let OwnedValue::Histogram(value) = OwnedValue::from_sample_checked(
        value,
        reader.control,
        reader.account,
        MemoryKind::Workspace,
    )?
    else {
        return Err(changed_kind());
    };
    Ok(value)
}

fn arithmetic<S: Series>(
    base: &mut OwnedHistogram,
    other: FloatHistogramRef<'_>,
    subtract: bool,
    output: &mut RateOutput,
    reader: &Reader<'_, S>,
) -> Result<bool, StorageError> {
    match base.arithmetic_checked(other, subtract, reader.control, reader.account) {
        Ok(flags) => {
            if flags.nhcb_bounds_reconciled {
                output.note(AnnotationKind::MismatchedCustomBuckets(if subtract {
                    HistogramOperation::Subtraction
                } else {
                    HistogramOperation::Addition
                }));
            }
            Ok(true)
        }
        Err(HistogramEvaluationError::Interrupted(error)) => Err(error.into()),
        Err(HistogramEvaluationError::Source(error)) => Err(error),
        Err(HistogramEvaluationError::Arithmetic(error)) => {
            if error == HistogramError::IncompatibleSchema {
                output.note(AnnotationKind::MixedExponentialCustomHistograms);
            }
            Ok(false)
        }
    }
}

fn add_correction<S: Series>(
    correction: &mut Option<OwnedHistogram>,
    prior: FloatHistogramRef<'_>,
    output: &mut RateOutput,
    reader: &Reader<'_, S>,
) -> Result<bool, StorageError> {
    if let Some(correction) = correction {
        return arithmetic(correction, prior, false, output, reader);
    }
    let OwnedValue::Histogram(value) = OwnedValue::copy_from_checked(
        SampleValueRef::FloatHistogram(prior),
        reader.control,
        reader.account,
        MemoryKind::Workspace,
    )?
    else {
        unreachable!();
    };
    *correction = Some(value);
    Ok(true)
}

enum MatrixOutcome {
    Empty,
    Histograms,
    Floats { first: u64, end: u64 },
}

pub(crate) struct MatrixExtension<'a, S> {
    reader: Reader<'a, S>,
    outcome: MatrixOutcome,
    left: f64,
    right: f64,
    endpoint_samples: [Option<crate::statistics::OwnedSamples>; 2],
    sample_units: u64,
}
impl<S: Series> MatrixExtension<'_, S> {
    pub(crate) fn sample_units(&self) -> u64 {
        self.sample_units
    }
    pub(crate) fn histograms(&self) -> bool {
        matches!(self.outcome, MatrixOutcome::Histograms)
    }

    pub(crate) fn capacity(&self) -> Option<i64> {
        let MatrixOutcome::Floats { first, end } = self.outcome else {
            return None;
        };
        Some(
            (end as i64)
                .wrapping_sub(first as i64)
                .max(0)
                .wrapping_add(2),
        )
    }

    pub(crate) fn invalid_slice(&self) -> Option<(u64, u64)> {
        let MatrixOutcome::Floats { first, end } = self.outcome else {
            return None;
        };
        (first > end).then_some((first, end))
    }

    pub(crate) fn visit<E: From<StorageError> + From<Interrupted>>(
        self,
        mut visit: impl FnMut(i64, f64) -> Result<(), E>,
    ) -> Result<(), E> {
        let MatrixOutcome::Floats { first, end } = self.outcome else {
            return Ok(());
        };
        visit(self.reader.input.range_start_ms, self.left)?;
        self.reader.walk::<E>(|index, sample| {
            if index >= first && index < end {
                let SampleValueRef::Float(value) = sample.value else {
                    return Err(changed_kind().into());
                };
                visit(sample.time_ms, value)?;
            }
            Ok(())
        })?;
        visit(self.reader.input.range_end_ms, self.right)
    }
}

pub(crate) fn matrix<'a, S: Series>(
    series: &'a S,
    input: RateInput,
    matrix: MatrixWindow,
    smoothed: bool,
    control: &'a Control,
    account: &'a QueryAccounting,
) -> Result<MatrixExtension<'a, S>, StorageError> {
    let reader = Reader {
        series,
        input,
        matrix,
        control,
        account,
    };
    let description = reader.describe()?;
    let mut extension = MatrixExtension {
        reader,
        outcome: MatrixOutcome::Empty,
        left: 0.0,
        right: 0.0,
        endpoint_samples: [None, None],
        sample_units: description
            .progress
            .map_or(0, |progress| progress.sample_units),
    };
    // Even an admitted stale histogram allocates the source histogram lane.
    if description.histograms_allocated {
        extension.outcome = MatrixOutcome::Histograms;
        return Ok(extension);
    }
    let Some(selected) = description.endpoints(smoothed) else {
        return Ok(extension);
    };
    if selected.last_time <= input.range_start_ms {
        return Ok(extension);
    }
    let mut output = RateOutput::default();
    let Some(left) =
        extension
            .reader
            .boundary(selected.first, true, smoothed, false, &mut output)?
    else {
        return Err(changed_kind());
    };
    let Some(right) =
        extension
            .reader
            .boundary(selected.last, false, smoothed, false, &mut output)?
    else {
        return Err(changed_kind());
    };
    let (OwnedValue::Float(left_value), OwnedValue::Float(right_value)) = (left.value, right.value)
    else {
        return Err(changed_kind());
    };
    extension.left = left_value;
    extension.right = right_value;
    extension.endpoint_samples = [left.float_samples, right.float_samples];
    extension.outcome = MatrixOutcome::Floats {
        first: selected.first + u64::from(selected.first_time <= input.range_start_ms),
        end: selected.last + u64::from(selected.last_time < input.range_end_ms),
    };
    Ok(extension)
}

pub(crate) fn smooth<S: Series>(
    series: &S,
    input: RateInput,
    matrix: MatrixWindow,
    control: &Control,
    account: &QueryAccounting,
) -> Result<RateOutput, StorageError> {
    let reader = Reader {
        series,
        input,
        matrix,
        control,
        account,
    };
    let description = reader.describe()?;
    let mut output = RateOutput {
        nonempty: description.count != 0,
        progress: description.progress,
        ..RateOutput::default()
    };
    // Classification covers the complete acquired window, including distant neighbors.
    if description.floats && description.histograms {
        output.note(AnnotationKind::MixedFloatsHistograms);
        return Ok(output);
    }
    let time = input.range_end_ms;
    if let Some((index, next_time)) = description.at_end {
        if next_time == time {
            let mut point = reader.point(index)?;
            if let OwnedValue::Histogram(value) = &mut point.value {
                value.normalize_copy_fields();
            }
            output.float_samples = point.float_samples;
            output.value = Some(point.value);
        } else if index > 0 {
            let previous = reader.point(index - 1)?;
            let next = reader.point(index)?;
            let counter = match (&previous.value, &next.value) {
                (OwnedValue::Histogram(previous), OwnedValue::Histogram(next)) => {
                    let previous = previous.as_ref();
                    let next = next.as_ref();
                    if (previous.schema == CUSTOM_BUCKETS_SCHEMA)
                        != (next.schema == CUSTOM_BUCKETS_SCHEMA)
                    {
                        output.note(AnnotationKind::MixedExponentialCustomHistograms);
                        return Ok(output);
                    }
                    previous.reset_hint != GAUGE || next.reset_hint != GAUGE
                }
                _ => false,
            };
            if let Some(value) = reader.interpolate(previous, next, time, counter, &mut output)? {
                output.float_samples = value.float_samples;
                output.value = Some(value.value);
            }
        }
    } else if let Some(index) = description.count.checked_sub(1) {
        let mut point = reader.point(index)?;
        if let OwnedValue::Histogram(value) = &mut point.value {
            value.normalize_copy_fields();
            value.set_reset_hint(crate::histogram::UNKNOWN_RESET);
        }
        output.float_samples = point.float_samples;
        output.value = Some(point.value);
    }
    output.account_value(account);
    Ok(output)
}

pub(crate) fn evaluate<S: Series>(
    series: &S,
    input: RateInput,
    kind: RateKind,
    smoothed: bool,
    matrix: MatrixWindow,
    control: &Control,
    account: &QueryAccounting,
) -> Result<RateOutput, StorageError> {
    let reader = Reader {
        series,
        input,
        matrix,
        control,
        account,
    };
    let description = reader.describe()?;
    let mut output = RateOutput {
        nonempty: description.count != 0,
        progress: description.progress,
        ..RateOutput::default()
    };
    if description.floats && description.histograms {
        output.note(AnnotationKind::MixedFloatsHistograms);
        return Ok(output);
    }
    let Some(selected) = description.select(input, smoothed) else {
        return Ok(output);
    };
    let counter = kind != RateKind::Delta;
    if description.histograms && !reader.validate_native(selected, counter, &mut output)? {
        return Ok(output);
    }
    let Some(left) = reader.boundary(selected.first, true, smoothed, counter, &mut output)? else {
        return Ok(output);
    };
    let Some(right) = reader.boundary(selected.last, false, smoothed, counter, &mut output)? else {
        return Ok(output);
    };
    let value = match (left.value, right.value) {
        (OwnedValue::Float(left), OwnedValue::Float(right)) => {
            let mut result = source_sub(right, left);
            if counter {
                result = source_add(result, reader.float_correction(selected, left, right)?);
            }
            if kind == RateKind::Rate {
                result = source_div(
                    result,
                    crate::frontend::number::seconds(input.window.range_ns),
                );
            }
            OwnedValue::Float(result)
        }
        (OwnedValue::Histogram(left), OwnedValue::Histogram(right)) => {
            if !counter && (left.as_ref().reset_hint != GAUGE || right.as_ref().reset_hint != GAUGE)
            {
                output.note(AnnotationKind::NativeHistogramNotGauge);
            }
            let mut result = right.copy_checked(control, account)?;
            if !arithmetic(&mut result, left.as_ref(), true, &mut output, &reader)? {
                return Ok(output);
            }
            if counter {
                match reader.native_correction(selected, &left, &right, smoothed, &mut output)? {
                    Correction::Omit => return Ok(output),
                    Correction::Complete(Some(correction)) => {
                        if !arithmetic(
                            &mut result,
                            correction.as_ref(),
                            false,
                            &mut output,
                            &reader,
                        )? {
                            return Ok(output);
                        }
                    }
                    Correction::Complete(None) => (),
                }
            }
            if kind == RateKind::Rate {
                result.div_checked(
                    crate::frontend::number::seconds(input.window.range_ns),
                    control,
                    account,
                )?;
            }
            result.set_reset_hint(GAUGE);
            result.compact_checked(control, account)?;
            OwnedValue::Histogram(result)
        }
        _ => return Err(changed_kind()),
    };
    output.value = Some(value);
    output.account_value(account);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range_float::RateWindow;

    #[test]
    fn completed_lane_search_preserves_ordinals_above_u32() {
        let boundary = u64::from(u32::MAX) + 1;
        let input = RateInput {
            window: RateWindow {
                start_ms: 0,
                end_ms: 10,
                range_ns: 10_000_000,
                start_timestamps: false,
            },
            range_start_ms: 0,
            range_end_ms: 10,
            data_bounds: None,
        };
        // A completed-lane descriptor exercises the boundary without retaining its history.
        let description = Description {
            progress: None,
            count: boundary + 4,
            floats: true,
            histograms: false,
            histograms_allocated: false,
            after_start: Some((boundary + 1, 0)),
            at_end: Some((boundary + 2, 10)),
            penultimate_time: Some(10),
            last_time: Some(11),
        };
        for (smoothed, last) in [(false, boundary + 3), (true, boundary + 2)] {
            let selected = description.select(input, smoothed).unwrap();
            assert_eq!(selected.first, boundary);
            assert_eq!(selected.last, last);
            assert_eq!(selected.first_time, 0);
            assert_eq!(selected.last_time, if smoothed { 10 } else { 11 });
        }
    }
}
