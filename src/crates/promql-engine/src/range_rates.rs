// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus promql/functions.go:452-758,830-964; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    annotations::{AnnotationKind, HistogramOperation},
    histogram::{CUSTOM_BUCKETS_SCHEMA, FloatHistogram, FloatHistogramRef, GAUGE, HistogramError},
    histogram_counter::detect_reset_with,
    histogram_work::{HistogramEvaluationError, HistogramWork},
    matrix_reader::{MatrixWindow, Progress, scan as scan_matrix},
    range_float::{
        Extrapolation, FloatStep, RateInput, RateKind, RateValue, RateWindow, extrapolation_factor,
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
    pub(crate) nonempty: bool,
    pub(crate) progress: Option<Progress>,
}
impl RateOutput {
    pub(crate) fn note(&mut self, kind: AnnotationKind) {
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
    mut matrix: Option<MatrixWindow>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<RateOutput, StorageError> {
    let input = input.into();
    let mut step = Step::new(input, function);
    loop {
        let progress = scan_matrix::<_, StorageError>(
            series,
            input.bounds(),
            matrix,
            control,
            account,
            |sample| step.receive(sample, control, account),
        )?;
        if let Some(mut output) = step.finish_pass(control, account)? {
            output.progress = progress;
            return Ok(output);
        }
        matrix = matrix.map(|window| window.with_progress(progress));
    }
}

pub(crate) enum Step {
    Float {
        input: RateInput,
        kind: RateKind,
        step: FloatStep,
    },
    Native(NativeStep),
    Instant(InstantStep),
}
impl Step {
    pub(crate) fn new(input: RateInput, function: RateFunction) -> Self {
        match function {
            RateFunction::Extrapolated(kind) => Self::Float {
                input,
                kind,
                step: FloatStep::new(input, kind),
            },
            RateFunction::Instant { counter } => Self::Instant(InstantStep {
                start_timestamps: input.window.start_timestamps,
                counter,
                floats: [None, None],
                histograms: [None, None],
            }),
        }
    }
    pub(crate) fn receive(
        &mut self,
        sample: SampleRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        match self {
            Self::Float { step, .. } => step.receive(sample, control),
            Self::Native(step) => step.receive(sample, control, account),
            Self::Instant(step) => step.receive(sample, control, account),
        }
    }
    pub(crate) fn finish_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<RateOutput>, StorageError> {
        let nonempty = match self {
            Self::Float { step, .. } => step.nonempty(),
            Self::Native(step) => step.count != 0,
            Self::Instant(step) => {
                step.floats.iter().any(Option::is_some)
                    || step.histograms.iter().any(Option::is_some)
            }
        };
        let mut result = match self {
            Self::Float { input, kind, step } => {
                let Some(result) = step.finish_pass() else {
                    return Ok(None);
                };
                if result.value == RateValue::Histogram {
                    *self = Self::Native(NativeStep::new(*input, *kind));
                    return Ok(None);
                }
                let mut output = RateOutput::default();
                if result.start_time_overlap {
                    output.note(AnnotationKind::StartTimeOverlap);
                }
                output.value = match result.value {
                    RateValue::Float(value) => Some(OwnedValue::Float(value)),
                    RateValue::Mixed => {
                        output.note(AnnotationKind::MixedFloatsHistograms);
                        None
                    }
                    RateValue::Empty => None,
                    RateValue::Histogram => unreachable!(),
                };
                Ok(Some(output))
            }
            Self::Native(step) => step.finish_pass(control, account),
            Self::Instant(step) => step.finish(control, account).map(Some),
        }?;
        if let Some(output) = &mut result {
            output.nonempty = nonempty;
        }
        Ok(result)
    }
}

fn arithmetic_error(
    error: HistogramEvaluationError,
    control: &Control,
    account: &QueryAccounting,
) -> StorageError {
    match error {
        HistogramEvaluationError::Interrupted(error) => error.into(),
        HistogramEvaluationError::Source(error) => error,
        HistogramEvaluationError::Arithmetic(error) => HistogramWork::new(control, account)
            .source(format_args!("{error}"), false)
            .into(),
    }
}

fn own_histogram(
    value: SampleValueRef<'_>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<OwnedHistogram, StorageError> {
    let OwnedValue::Histogram(value) =
        OwnedValue::from_sample_checked(value, control, account, MemoryKind::Workspace)?
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

#[derive(Clone, Copy)]
enum NativePass {
    Endpoints,
    Schemas,
    Resets,
}

pub(crate) struct NativeStep {
    input: RateInput,
    kind: RateKind,
    phase: NativePass,
    first: Option<HistogramPoint>,
    second: Option<HistogramPoint>,
    last: Option<HistogramPoint>,
    count: u64,
    zero: Option<OwnedHistogram>,
    schema: i32,
    custom: bool,
    value: Option<OwnedHistogram>,
    previous: Option<OwnedHistogram>,
    previous_time: i64,
    previous_start: i64,
    index: u64,
    mismatch: bool,
    output: RateOutput,
}
impl NativeStep {
    fn new(input: RateInput, kind: RateKind) -> Self {
        Self {
            input,
            kind,
            phase: NativePass::Endpoints,
            first: None,
            second: None,
            last: None,
            count: 0,
            zero: None,
            schema: 0,
            custom: false,
            value: None,
            previous: None,
            previous_time: 0,
            previous_start: 0,
            index: 0,
            mismatch: false,
            output: RateOutput::default(),
        }
    }
    fn last(&self) -> &HistogramPoint {
        self.last
            .as_ref()
            .or(self.second.as_ref())
            .or(self.first.as_ref())
            .expect("native endpoint")
    }
    fn effective_first(&self) -> &OwnedHistogram {
        self.zero
            .as_ref()
            .unwrap_or(&self.first.as_ref().expect("native first").value)
    }
    fn receive(
        &mut self,
        sample: SampleRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        control.check()?;
        if sample.value.is_stale() {
            return Ok(());
        }
        match self.phase {
            NativePass::Endpoints => {
                self.count += 1;
                match self.count {
                    1 => {
                        self.first = Some(HistogramPoint::copy(
                            sample,
                            self.input.window,
                            control,
                            account,
                        )?);
                    }
                    2 => {
                        self.second = Some(HistogramPoint::copy(
                            sample,
                            self.input.window,
                            control,
                            account,
                        )?);
                    }
                    _ => {
                        if let Some(last) = &mut self.last {
                            last.time = sample.time_ms;
                            last.start = if self.input.window.start_timestamps {
                                sample.start_time_ms
                            } else {
                                0
                            };
                            last.value.retain_from_sample_checked(
                                sample.value,
                                control,
                                account,
                            )?;
                        } else {
                            self.last = Some(HistogramPoint::copy(
                                sample,
                                self.input.window,
                                control,
                                account,
                            )?);
                        }
                    }
                }
            }
            NativePass::Schemas => {
                let interior = self.index > 0 && self.index < self.count - 1;
                self.index += 1;
                if !interior || self.mismatch {
                    return Ok(());
                }
                let (schema, hint) = match sample.value {
                    SampleValueRef::FloatHistogram(h) => (h.schema, h.reset_hint),
                    SampleValueRef::IntegerHistogram(h) => (h.schema, h.reset_hint),
                    SampleValueRef::Float(_) => {
                        return Err(StorageError::Invalid(
                            "provider replay changed sample kinds",
                        ));
                    }
                };
                if hint == GAUGE {
                    self.output.note(AnnotationKind::NativeHistogramNotCounter);
                }
                self.schema = self.schema.min(schema);
                self.mismatch = (schema == CUSTOM_BUCKETS_SCHEMA) != self.custom;
            }
            NativePass::Resets => {
                self.index += 1;
                if self.index == 1 || self.mismatch {
                    return Ok(());
                }
                let start = if self.input.window.start_timestamps {
                    sample.start_time_ms
                } else {
                    0
                };
                if start_time_overlap(self.previous_start, self.previous_time, start) {
                    self.output.note(AnnotationKind::StartTimeOverlap);
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
                let effective_first = self
                    .zero
                    .as_ref()
                    .unwrap_or(&self.first.as_ref().expect("native first").value);
                let prior = self.previous.as_ref().unwrap_or(effective_first).as_ref();
                if start_timestamp_reset(
                    self.previous_start,
                    self.previous_time,
                    start,
                    sample.time_ms,
                ) || detect_reset_with(current, prior, HistogramWork::new(control, account))?
                {
                    match self
                        .value
                        .as_mut()
                        .expect("native accumulator")
                        .arithmetic_checked(prior, false, control, account)
                    {
                        Ok(flags) => {
                            if flags.nhcb_bounds_reconciled {
                                self.output.note(AnnotationKind::MismatchedCustomBuckets(
                                    HistogramOperation::Addition,
                                ));
                            }
                        }
                        Err(HistogramEvaluationError::Arithmetic(
                            HistogramError::IncompatibleSchema,
                        )) => {
                            self.mismatch = true;
                        }
                        Err(error) => return Err(arithmetic_error(error, control, account)),
                    }
                }
                if let Some(converted) = converted {
                    self.previous = Some(converted);
                } else if let Some(previous) = &mut self.previous {
                    previous.retain_from_sample_checked(sample.value, control, account)?;
                } else {
                    self.previous = Some(own_histogram(sample.value, control, account)?);
                }
                self.previous_time = sample.time_ms;
                self.previous_start = start;
            }
        }
        Ok(())
    }
    fn prepare_endpoints(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, StorageError> {
        let first = self.first.as_ref().expect("native first");
        let second = self.second.as_ref().expect("native second");
        if self.kind != RateKind::Delta {
            if first.value.as_ref().reset_hint == GAUGE
                || self.last().value.as_ref().reset_hint == GAUGE
            {
                self.output.note(AnnotationKind::NativeHistogramNotCounter);
            }
            if start_timestamp_reset(first.start, first.time, second.start, second.time)
                || detect_reset_with(
                    second.value.as_ref(),
                    first.value.as_ref(),
                    HistogramWork::new(control, account),
                )?
            {
                let empty = FloatHistogram::default();
                self.zero = Some(own_histogram(
                    SampleValueRef::FloatHistogram(FloatHistogramRef {
                        schema: second.value.as_ref().schema,
                        custom_values: second.value.as_ref().custom_values,
                        ..empty.as_ref()
                    }),
                    control,
                    account,
                )?);
            }
        }
        self.custom = self.effective_first().as_ref().schema == CUSTOM_BUCKETS_SCHEMA;
        if (self.last().value.as_ref().schema == CUSTOM_BUCKETS_SCHEMA) != self.custom {
            self.output
                .note(AnnotationKind::MixedExponentialCustomHistograms);
            return Ok(false);
        }
        self.schema = self
            .last()
            .value
            .as_ref()
            .schema
            .min(self.effective_first().as_ref().schema);
        Ok(true)
    }
    fn subtract(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, StorageError> {
        let mut value = self
            .last()
            .value
            .copy_to_schema_checked(self.schema, control, account)
            .map_err(|error| arithmetic_error(error, control, account))?;
        match value.arithmetic_checked(self.effective_first().as_ref(), true, control, account) {
            Ok(flags) => {
                if flags.nhcb_bounds_reconciled {
                    self.output.note(AnnotationKind::MismatchedCustomBuckets(
                        HistogramOperation::Subtraction,
                    ));
                }
            }
            Err(HistogramEvaluationError::Arithmetic(HistogramError::IncompatibleSchema)) => {
                self.output = RateOutput::only(AnnotationKind::MixedExponentialCustomHistograms);
                return Ok(false);
            }
            Err(error) => return Err(arithmetic_error(error, control, account)),
        }
        self.value = Some(value);
        Ok(true)
    }
    fn finish_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<RateOutput>, StorageError> {
        match self.phase {
            NativePass::Endpoints => {
                if self.first.is_none() {
                    return Ok(Some(std::mem::take(&mut self.output)));
                }
                if self.count > 1 {
                    if !self.prepare_endpoints(control, account)? {
                        return Ok(Some(std::mem::take(&mut self.output)));
                    }
                    if self.kind != RateKind::Delta {
                        self.phase = NativePass::Schemas;
                        self.index = 0;
                        return Ok(None);
                    }
                    if !self.subtract(control, account)? {
                        return Ok(Some(std::mem::take(&mut self.output)));
                    }
                    if self.first.as_ref().unwrap().value.as_ref().reset_hint != GAUGE
                        || self.last().value.as_ref().reset_hint != GAUGE
                    {
                        self.output.note(AnnotationKind::NativeHistogramNotGauge);
                    }
                }
            }
            NativePass::Schemas => {
                if self.mismatch {
                    return Ok(Some(RateOutput::only(
                        AnnotationKind::MixedExponentialCustomHistograms,
                    )));
                }
                if !self.subtract(control, account)? {
                    return Ok(Some(std::mem::take(&mut self.output)));
                }
                let first = self.first.as_ref().unwrap();
                self.previous_time = first.time;
                self.previous_start = first.start;
                self.phase = NativePass::Resets;
                self.index = 0;
                return Ok(None);
            }
            NativePass::Resets => {
                if self.mismatch {
                    return Ok(Some(RateOutput::only(
                        AnnotationKind::MixedExponentialCustomHistograms,
                    )));
                }
            }
        }
        if let Some(value) = &mut self.value {
            value.set_reset_hint(GAUGE);
            value.compact_checked(control, account)?;
        }
        self.finish(control, account).map(Some)
    }
    fn finish(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<RateOutput, StorageError> {
        let first = self.first.as_ref().expect("native first");
        if uses_first_start(self.input, self.kind, first.time, first.start) {
            if let Some(value) = &mut self.value {
                match value.arithmetic_checked(first.value.as_ref(), false, control, account) {
                    Ok(flags) => {
                        if flags.nhcb_bounds_reconciled {
                            self.output.note(AnnotationKind::MismatchedCustomBuckets(
                                HistogramOperation::Addition,
                            ));
                        }
                    }
                    Err(HistogramEvaluationError::Arithmetic(
                        HistogramError::IncompatibleSchema,
                    )) => {
                        self.output
                            .note(AnnotationKind::MixedExponentialCustomHistograms);
                        return Ok(std::mem::take(&mut self.output));
                    }
                    Err(error) => return Err(arithmetic_error(error, control, account)),
                }
            } else {
                // The singleton extension retains its original hint and uncompacted layout.
                self.value = Some(
                    first
                        .value
                        .copy_to_schema_checked(first.value.as_ref().schema, control, account)
                        .map_err(|error| arithmetic_error(error, control, account))?,
                );
            }
        }
        let Some(mut value) = self.value.take() else {
            return Ok(std::mem::take(&mut self.output));
        };
        let Some(factor) = extrapolation_factor(
            self.input,
            self.kind,
            Extrapolation {
                first_time: first.time,
                last_time: self.last().time,
                first_start: first.start,
                count: self.count,
                first_value: first.value.as_ref().count,
                result_value: value.as_ref().count,
            },
        ) else {
            return Ok(std::mem::take(&mut self.output));
        };
        value.mul_checked(factor, control, account)?;
        self.output.value = Some(OwnedValue::Histogram(value));
        Ok(std::mem::take(&mut self.output))
    }
}

struct InstantPoint {
    time: i64,
    start: i64,
    value: OwnedValue,
}
pub(crate) struct InstantStep {
    counter: bool,
    start_timestamps: bool,
    floats: [Option<InstantPoint>; 2],
    histograms: [Option<InstantPoint>; 2],
}
impl InstantStep {
    fn receive(
        &mut self,
        sample: SampleRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        control.check()?;
        if sample.value.is_stale() {
            return Ok(());
        }
        let points = if matches!(sample.value, SampleValueRef::Float(_)) {
            &mut self.floats
        } else {
            &mut self.histograms
        };
        points[0] = points[1].take();
        points[1] = Some(InstantPoint {
            time: sample.time_ms,
            start: if self.start_timestamps {
                sample.start_time_ms
            } else {
                0
            },
            value: OwnedValue::from_sample_checked(
                sample.value,
                control,
                account,
                MemoryKind::Workspace,
            )?,
        });
        Ok(())
    }
    fn finish(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<RateOutput, StorageError> {
        let counter = self.counter;
        let mut first = None::<InstantPoint>;
        let mut last = None::<InstantPoint>;
        for point in std::mem::take(&mut self.floats).into_iter().flatten() {
            if first.is_none() {
                first = Some(point);
            } else {
                last = Some(point);
            }
        }
        // Per-kind tails precede the source insertion switch, including equal-time cases.
        for point in std::mem::take(&mut self.histograms).into_iter().flatten() {
            if let Some(left) = &first {
                if let Some(right) = &last {
                    if point.time < left.time {
                        continue;
                    }
                    if point.time > right.time {
                        first = last.take();
                        last = Some(point);
                    } else {
                        first = Some(point);
                    }
                } else if point.time < left.time {
                    last = first.take();
                    first = Some(point);
                } else {
                    last = Some(point);
                }
            } else {
                first = Some(point);
            }
        }
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
                    crate::numerical::source_sub(*last_value, *first_value)
                };
                if counter {
                    value = crate::numerical::source_div(
                        value,
                        crate::numerical::source_div(interval as f64, 1000.0),
                    );
                }
                Some(OwnedValue::Float(value))
            }
            (OwnedValue::Histogram(first_value), OwnedValue::Histogram(last_value)) => {
                let first_h = first_value.as_ref();
                let last_h = last_value.as_ref();
                let mut value = last_value
                    .copy_to_schema_checked(last_h.schema, control, account)
                    .map_err(|error| arithmetic_error(error, control, account))?;
                if counter && (first_h.reset_hint == GAUGE || last_h.reset_hint == GAUGE) {
                    output.note(AnnotationKind::NativeHistogramNotCounter);
                } else if !counter && (first_h.reset_hint != GAUGE || last_h.reset_hint != GAUGE) {
                    output.note(AnnotationKind::NativeHistogramNotGauge);
                }
                if !counter
                    || !(start_timestamp_reset(first.start, first.time, last.start, last.time)
                        || detect_reset_with(
                            last_h,
                            first_h,
                            HistogramWork::new(control, account),
                        )?)
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
                        Err(error) => return Err(arithmetic_error(error, control, account)),
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
    fn streamed_native_endpoints_and_replay_ordinals_preserve_u32_rollover() {
        let account = EngineAccounting::default().query();
        let control = Control::default();
        let prefix = u64::from(u32::MAX);
        let window = RateWindow {
            start_ms: 0,
            end_ms: (prefix + 2) as i64,
            range_ns: (prefix + 2) as i64 * 1_000_000,
            start_timestamps: false,
        };
        let mut step = NativeStep::new(window.into(), RateKind::Increase);
        let histogram = FloatHistogram {
            count: 1.0,
            sum: 1.0,
            zero_count: 1.0,
            reset_hint: NOT_RESET,
            ..FloatHistogram::default()
        };
        let mut sample = SampleRef {
            time_ms: 1,
            start_time_ms: 0,
            value: SampleValueRef::FloatHistogram(histogram.as_ref()),
        };
        for time in [1, 2, prefix as i64] {
            sample.time_ms = time;
            step.receive(sample, &control, &account).unwrap();
        }
        step.count = prefix;
        sample.time_ms = (prefix + 1) as i64;
        step.receive(sample, &control, &account).unwrap();
        assert_eq!(step.count, prefix + 1);
        assert_eq!(step.first.as_ref().unwrap().time, 1);
        assert_eq!(step.second.as_ref().unwrap().time, 2);
        assert_eq!(step.last().time, (prefix + 1) as i64);

        step.phase = NativePass::Schemas;
        step.index = prefix;
        step.receive(sample, &control, &account).unwrap();
        assert_eq!(step.index, prefix + 1);
        step.receive(sample, &control, &account).unwrap();
        assert_eq!(step.index, prefix + 2);
        drop(step);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn instant_typed_tails_preserve_source_equal_time_insertion() {
        for counter in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut step = Step::new(window(2).into(), RateFunction::Instant { counter });
            for (time_ms, value) in [(0, 0.0), (10, 10.0)] {
                step.receive(
                    SampleRef {
                        time_ms,
                        start_time_ms: 0,
                        value: SampleValueRef::Float(value),
                    },
                    &control,
                    &account,
                )
                .unwrap();
            }
            for time_ms in [10, 20] {
                let mut sample = native(0);
                sample.time_ms = time_ms;
                step.receive(sample.as_ref(), &control, &account).unwrap();
            }
            let result = step.finish_pass(&control, &account).unwrap().unwrap();
            assert!(result.value.is_none());
            assert_eq!(
                result.annotations[0],
                Some(AnnotationKind::MixedFloatsHistograms)
            );
            assert!(result.annotations[1..].iter().all(Option::is_none));
            drop((result, step));
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn instant_equal_selected_times_finish_before_histogram_warnings() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut step = Step::new(window(2).into(), RateFunction::Instant { counter: true });
        for value in [0.0, 10.0] {
            step.receive(
                SampleRef {
                    time_ms: value as i64,
                    start_time_ms: 0,
                    value: SampleValueRef::Float(value),
                },
                &control,
                &account,
            )
            .unwrap();
        }
        let mut sample = native(0);
        sample.time_ms = 10;
        step.receive(sample.as_ref(), &control, &account).unwrap();
        let result = step.finish_pass(&control, &account).unwrap().unwrap();
        assert!(result.value.is_none());
        assert!(result.annotations.iter().all(Option::is_none));
        drop((result, step));
        account.snapshot().assert_memory_released();
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
                resume: None,
                range_ms: window(100).end_ms,
                buffer_ms: None,
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
