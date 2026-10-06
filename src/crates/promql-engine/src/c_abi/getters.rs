// SPDX-License-Identifier: GPL-3.0-or-later
//! Immutable individual views never materialize a second result or access the provider.
use super::*;

fn index(value: u64) -> Result<usize, Invalid> {
    usize::try_from(value).map_err(|_| Invalid("index overflow"))
}
fn error(kind: u32, message: &[u8]) -> types::Error {
    types::Error {
        kind,
        message: types::Bytes::of(message),
        ..types::Error::default()
    }
}
fn interrupted(value: Interrupted) -> types::Error {
    match value {
        Interrupted::Cancelled => error(10, b"query cancelled"),
        Interrupted::Deadline => error(11, b"query deadline exceeded"),
    }
}
fn execution_error(value: &ExecutionError) -> types::Error {
    match value {
        ExecutionError::Internal => error(12, b"internal PromQL error"),
        ExecutionError::Sink(value) => error(3, value),
        ExecutionError::Evaluation(value) => match value {
            EvaluationError::Interrupted(value)
            | EvaluationError::Storage(StorageError::Interrupted(value)) => interrupted(*value),
            EvaluationError::Storage(StorageError::Provider(value)) => error(2, value),
            EvaluationError::Storage(StorageError::Invalid(value)) => error(4, value.as_bytes()),
            EvaluationError::Preparation(value) => error(5, value),
            EvaluationError::Runtime(value) => error(6, value),
            EvaluationError::Expansion(value) => error(7, value),
            EvaluationError::Invalid(value) => error(8, value.as_bytes()),
            EvaluationError::Unsupported(value) => error(9, value.as_bytes()),
        },
    }
}
impl Report {
    fn status(&self) -> u32 {
        match &self.outcome {
            Outcome::Compile(value) => compile_status(value),
            Outcome::Execute(value) => execution_status(value),
        }
    }
    fn error_count(&self) -> usize {
        match &self.outcome {
            Outcome::Compile(Err(CompileError::Parse(errors))) => errors.len(),
            Outcome::Compile(Err(_)) | Outcome::Execute(Err(_)) => 1,
            _ => 0,
        }
    }
    fn error(&self, offset: usize) -> Result<types::Error, Invalid> {
        if offset >= self.error_count() {
            return Err(Invalid("error index out of bounds"));
        }
        Ok(match &self.outcome {
            Outcome::Compile(Err(CompileError::Parse(errors))) => {
                let value = &errors[offset];
                types::Error {
                    kind: 1,
                    position_present: 1,
                    position: types::Position {
                        start: value.span.start as u64,
                        end: value.span.end as u64,
                    },
                    message: types::Bytes::of(&value.message),
                }
            }
            Outcome::Compile(Err(CompileError::Interrupted(value))) => interrupted(*value),
            Outcome::Compile(Err(CompileError::Internal)) => error(12, b"internal PromQL error"),
            Outcome::Execute(Err(value)) => execution_error(value),
            _ => unreachable!("error count excludes success"),
        })
    }
}
unsafe fn result<'a>(value: *const types::ResultHandle) -> Result<&'a CollectedResult, Invalid> {
    unsafe { borrow::<ResultOwner, _>(value) }?
        .value
        .as_ref()
        .ok_or(Invalid("unpublished result"))
}
fn series(value: &CollectedResult, offset: u64) -> Result<&crate::result::ResultSeries, Invalid> {
    value
        .series
        .get(index(offset)?)
        .ok_or(Invalid("series index out of bounds"))
}
fn point(
    value: &CollectedResult,
    series_index: u64,
    point_index: u64,
) -> Result<&crate::result::OutputPoint, Invalid> {
    series(value, series_index)?
        .points
        .get(index(point_index)?)
        .ok_or(Invalid("point index out of bounds"))
}
fn annotation(value: &Annotations, offset: u64) -> Result<types::Annotation, Invalid> {
    value
        .get(index(offset)?)
        .map(output::annotation)
        .ok_or(Invalid("annotation index out of bounds"))
}

export! {
    fn netdata_promql_engine_observations(value: *const types::Engine, out: *mut types::EngineObservations) {
        unsafe { checked::output(out) }?;
        let value = observations::engine(&unsafe { borrow::<Engine, _>(value) }?.core.accounting.snapshot());
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_query_source(value: *const types::Query, out: *mut types::Bytes) {
        unsafe { checked::output(out) }?;
        let value = types::Bytes::of(unsafe { borrow::<CompiledQuery, _>(value) }?.source());
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_query_type(value: *const types::Query, out: *mut u32) {
        unsafe { checked::output(out) }?;
        let value = output::value_type(unsafe { borrow::<CompiledQuery, _>(value) }?.value_type());
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_query_compilation_observations(value: *const types::Query, out: *mut types::QueryObservations) {
        unsafe { checked::output(out) }?;
        let value = observations::query(&unsafe { borrow::<CompiledQuery, _>(value) }?.compilation_accounting().snapshot());
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_report_status(value: *const types::Report, out: *mut u32) {
        unsafe { checked::output(out) }?;
        let value = unsafe { borrow::<Report, _>(value) }?.status();
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_report_error_count(value: *const types::Report, out: *mut u64) {
        unsafe { checked::output(out) }?;
        let value = unsafe { borrow::<Report, _>(value) }?.error_count() as u64;
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_report_error(value: *const types::Report, offset: u64, out: *mut types::Error) {
        unsafe { checked::output(out) }?;
        let value = unsafe { borrow::<Report, _>(value) }?.error(index(offset)?)?;
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_report_annotation_count(value: *const types::Report, out: *mut u64) {
        unsafe { checked::output(out) }?;
        let value = unsafe { borrow::<Report, _>(value) }?.annotations.len() as u64;
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_report_annotation(value: *const types::Report, offset: u64, out: *mut types::Annotation) {
        unsafe { checked::output(out) }?;
        let value = annotation(&unsafe { borrow::<Report, _>(value) }?.annotations, offset)?;
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_report_observations(value: *const types::Report, out: *mut types::QueryObservations) {
        unsafe { checked::output(out) }?;
        let value = observations::query(&unsafe { borrow::<Report, _>(value) }?.observations);
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_report_live_observations(value: *const types::Report, out: *mut types::QueryObservations) {
        unsafe { checked::output(out) }?;
        let value = observations::query(&unsafe { borrow::<Report, _>(value) }?.account.snapshot());
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_result_info(value: *const types::ResultHandle, out: *mut types::ResultInfo) {
        unsafe { checked::output(out) }?;
        let value = unsafe { result(value) }?;
        let value = types::ResultInfo { header: output::header(value.header), scalar_present: u32::from(value.scalar.is_some()), string_present: u32::from(value.string.is_some()), series_count: value.series.len() as u64, annotation_count: value.annotations.len() as u64 };
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_result_scalar(value: *const types::ResultHandle, out: *mut types::Scalar) {
        unsafe { checked::output(out) }?;
        let (time_ms, value) = unsafe { result(value) }?.scalar.ok_or(Invalid("result has no scalar"))?;
        unsafe { out.write(types::Scalar { time_ms, value_bits: value.to_bits() }) };
        Ok(types::OK)
    }
    fn netdata_promql_result_string(value: *const types::ResultHandle, out: *mut types::StringValue) {
        unsafe { checked::output(out) }?;
        let (time_ms, value) = unsafe { result(value) }?.string.as_ref().ok_or(Invalid("result has no string"))?;
        unsafe { out.write(types::StringValue { time_ms: *time_ms, value: types::Bytes::of(value) }) };
        Ok(types::OK)
    }
    fn netdata_promql_result_series_info(value: *const types::ResultHandle, offset: u64, out: *mut types::SeriesInfo) {
        unsafe { checked::output(out) }?;
        let value = series(unsafe { result(value) }?, offset)?;
        unsafe { out.write(types::SeriesInfo { series_id: value.series_id as u64, label_count: value.labels.0.len() as u64, point_count: value.points.len() as u64 }) };
        Ok(types::OK)
    }
    fn netdata_promql_result_label(value: *const types::ResultHandle, offset: u64, label: u64, out: *mut types::Label) {
        unsafe { checked::output(out) }?;
        let value = series(unsafe { result(value) }?, offset)?.labels.0.get(index(label)?).ok_or(Invalid("label index out of bounds"))?;
        unsafe { out.write(types::Label { name: types::Bytes::of(&value.name), value: types::Bytes::of(&value.value) }) };
        Ok(types::OK)
    }
    fn netdata_promql_result_point(value: *const types::ResultHandle, offset: u64, point_index: u64, out: *mut types::PointInfo) {
        unsafe { checked::output(out) }?;
        let value = point(unsafe { result(value) }?, offset, point_index)?;
        let (value_kind, value_bits) = match &value.value { crate::result::OwnedValue::Float(value) => (types::VALUE_FLOAT, value.to_bits()), crate::result::OwnedValue::Histogram(_) => (types::VALUE_FLOAT_HISTOGRAM, 0) };
        unsafe { out.write(types::PointInfo { time_ms: value.time_ms, value_kind, reserved: 0, value_bits }) };
        Ok(types::OK)
    }
    fn netdata_promql_result_histogram(value: *const types::ResultHandle, offset: u64, point_index: u64, out: *mut types::FloatHistogram) {
        unsafe { checked::output(out) }?;
        let value = point(unsafe { result(value) }?, offset, point_index)?;
        let crate::result::OwnedValue::Histogram(value) = &value.value else { return Err(Invalid("point has no histogram")); };
        unsafe { out.write(output::float_histogram(value.as_ref())) };
        Ok(types::OK)
    }
    fn netdata_promql_result_annotation(value: *const types::ResultHandle, offset: u64, out: *mut types::Annotation) {
        unsafe { checked::output(out) }?;
        let value = annotation(&unsafe { result(value) }?.annotations, offset)?;
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_result_live_observations(value: *const types::ResultHandle, out: *mut types::QueryObservations) {
        unsafe { checked::output(out) }?;
        let value = observations::query(&unsafe { borrow::<ResultOwner, _>(value) }?.account.snapshot());
        unsafe { out.write(value) };
        Ok(types::OK)
    }
}
