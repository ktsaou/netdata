// SPDX-License-Identifier: GPL-3.0-or-later
//! Temporary descriptors borrow provisional output only for their synchronous callback.
use super::{
    checked::{self, Invalid},
    types,
};
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting, QueryObservations},
    result::{OutputChunk, OutputError, OutputHeader, OutputSink},
    storage::{Control, Interrupted},
    value::SampleValueRef,
};

pub(crate) fn value_type(value: crate::frontend::ast::ValueType) -> u32 {
    match value {
        crate::frontend::ast::ValueType::Scalar => types::TYPE_SCALAR,
        crate::frontend::ast::ValueType::Vector => types::TYPE_VECTOR,
        crate::frontend::ast::ValueType::Matrix => types::TYPE_MATRIX,
        crate::frontend::ast::ValueType::String => types::TYPE_STRING,
        crate::frontend::ast::ValueType::Invalid => 0,
    }
}
pub(crate) fn header(value: OutputHeader) -> types::OutputHeader {
    types::OutputHeader {
        value_type: value_type(value.value_type),
        reserved: 0,
        start_ms: value.start_ms,
        end_ms: value.end_ms,
        step_ms: value.step_ms,
    }
}
fn spans(value: Option<&[crate::histogram::Span]>) -> types::SpanArray {
    value.map_or_else(types::SpanArray::default, |value| types::SpanArray {
        data: value.as_ptr(),
        len: value.len() as u64,
        present: 1,
        reserved: 0,
    })
}
fn floats(value: Option<&[f64]>) -> types::F64Array {
    value.map_or_else(types::F64Array::default, |value| types::F64Array {
        data: value.as_ptr(),
        len: value.len() as u64,
        present: 1,
        reserved: 0,
    })
}
fn integers(value: Option<&[i64]>) -> types::I64Array {
    value.map_or_else(types::I64Array::default, |value| types::I64Array {
        data: value.as_ptr(),
        len: value.len() as u64,
        present: 1,
        reserved: 0,
    })
}
pub(crate) fn float_histogram(
    value: crate::histogram::FloatHistogramRef<'_>,
) -> types::FloatHistogram {
    types::FloatHistogram {
        reset_hint: value.reset_hint as u32,
        schema: value.schema,
        zero_threshold_bits: value.zero_threshold.to_bits(),
        zero_count_bits: value.zero_count.to_bits(),
        count_bits: value.count.to_bits(),
        sum_bits: value.sum.to_bits(),
        positive_spans: spans(value.positive_spans),
        negative_spans: spans(value.negative_spans),
        positive_buckets: floats(value.positive_buckets),
        negative_buckets: floats(value.negative_buckets),
        custom_values: floats(value.custom_values),
    }
}
fn integer_histogram(value: crate::histogram::IntegerHistogramRef<'_>) -> types::IntegerHistogram {
    types::IntegerHistogram {
        reset_hint: value.reset_hint as u32,
        schema: value.schema,
        zero_threshold_bits: value.zero_threshold.to_bits(),
        zero_count: value.zero_count,
        count: value.count,
        sum_bits: value.sum.to_bits(),
        positive_spans: spans(value.positive_spans),
        negative_spans: spans(value.negative_spans),
        positive_buckets: integers(value.positive_buckets),
        negative_buckets: integers(value.negative_buckets),
        custom_values: floats(value.custom_values),
    }
}
pub(crate) fn annotation(value: crate::annotations::AnnotationRef<'_>) -> types::Annotation {
    types::Annotation {
        severity: match value.severity {
            crate::annotations::Severity::Info => 0,
            crate::annotations::Severity::Warning => 1,
        },
        position_present: u32::from(value.span.is_some()),
        position: value
            .span
            .map_or_else(types::Position::default, |span| types::Position {
                start: span.start as u64,
                end: span.end as u64,
            }),
        occurrences: value.occurrences,
        raw_message: types::Bytes::of(value.raw_message),
        message: types::Bytes::of(value.message),
    }
}

pub(crate) struct Sink {
    table: types::Output,
    finished: bool,
}
impl Sink {
    pub(crate) unsafe fn new(table: *const types::Output) -> Result<Self, Invalid> {
        let table = unsafe { checked::versioned(table) }?;
        if table.begin.is_none()
            || table.chunk.is_none()
            || table.commit.is_none()
            || table.abort.is_none()
        {
            return Err(Invalid("missing output callback"));
        }
        Ok(Self {
            table,
            finished: false,
        })
    }
    unsafe fn status(
        &self,
        status: u32,
        error: types::Bytes,
        account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        match status {
            types::OK => Ok(()),
            types::CANCELLED => Err(Interrupted::Cancelled.into()),
            types::DEADLINE => Err(Interrupted::Deadline.into()),
            types::OUTPUT_ERROR => {
                let bytes = match unsafe { checked::returned_error(error, account) } {
                    Ok(bytes) => bytes,
                    Err(error) => Self::invalid(error.0, account),
                };
                Err(OutputError::Failure(bytes))
            }
            _ => Err(OutputError::Failure(Self::invalid(
                "invalid output callback status",
                account,
            ))),
        }
    }
    fn invalid(message: &'static str, account: &QueryAccounting) -> AccountedVec<u8> {
        let mut bytes = AccountedVec::new(account, MemoryKind::Workspace);
        bytes.extend_from_slice(message.as_bytes());
        bytes
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        if !self.finished {
            self.abort();
        }
    }
}
impl OutputSink for Sink {
    fn begin(&mut self, _: OutputHeader) -> Result<(), AccountedVec<u8>> {
        panic!("foreign output requires checked delivery")
    }
    fn chunk(&mut self, _: OutputChunk<'_>) -> Result<(), AccountedVec<u8>> {
        panic!("foreign output requires checked delivery")
    }
    fn commit(&mut self, _: &QueryObservations) -> Result<(), AccountedVec<u8>> {
        panic!("foreign output requires checked delivery")
    }
    fn abort(&mut self) {
        if !self.finished {
            self.finished = true;
            unsafe { self.table.abort.expect("validated output abort")(self.table.context) };
        }
    }
    fn begin_checked(
        &mut self,
        value: OutputHeader,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        let value = header(value);
        control.check()?;
        let mut error = types::Bytes::default();
        let status = unsafe {
            self.table.begin.expect("validated output begin")(
                self.table.context,
                &value,
                checked::callback_control(control),
                &mut error,
            )
        };
        unsafe { self.status(status, error, account) }
    }
    fn chunk_checked(
        &mut self,
        value: OutputChunk<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        let mut labels = AccountedVec::new(account, MemoryKind::Workspace);
        let mut points = AccountedVec::new(account, MemoryKind::Workspace);
        let mut histograms = AccountedVec::new(account, MemoryKind::Workspace);
        let mut integer_histograms = AccountedVec::new(account, MemoryKind::Workspace);
        let mut annotations = AccountedVec::new(account, MemoryKind::Workspace);
        control.check()?;
        let value = match value {
            OutputChunk::Annotations(value) => {
                for value in value.iter() {
                    control.check()?;
                    account.work(1);
                    annotations.push(annotation(value));
                }
                types::Chunk {
                    kind: types::CHUNK_ANNOTATIONS,
                    reserved: 0,
                    payload: types::ChunkPayload {
                        annotations: types::Annotations {
                            data: annotations.as_ptr(),
                            len: annotations.len() as u64,
                        },
                    },
                }
            }
            OutputChunk::Scalar { time_ms, value } => types::Chunk {
                kind: types::CHUNK_SCALAR,
                reserved: 0,
                payload: types::ChunkPayload {
                    scalar: types::Scalar {
                        time_ms,
                        value_bits: value.to_bits(),
                    },
                },
            },
            OutputChunk::String { time_ms, value } => types::Chunk {
                kind: types::CHUNK_STRING,
                reserved: 0,
                payload: types::ChunkPayload {
                    string: types::StringValue {
                        time_ms,
                        value: types::Bytes::of(value),
                    },
                },
            },
            OutputChunk::Series {
                series_id,
                labels: source_labels,
                points: source_points,
            } => {
                for value in source_labels {
                    control.check()?;
                    account.work(1);
                    labels.push(types::Label {
                        name: types::Bytes::of(value.name),
                        value: types::Bytes::of(value.value),
                    });
                }
                // Finish all histogram buffers before recording pointers that growth could invalidate.
                for point in source_points {
                    control.check()?;
                    account.work(1);
                    match point.value {
                        SampleValueRef::Float(_) => (),
                        SampleValueRef::IntegerHistogram(value) => {
                            integer_histograms.push(integer_histogram(value))
                        }
                        SampleValueRef::FloatHistogram(value) => {
                            histograms.push(float_histogram(value))
                        }
                    }
                }
                let (mut float_index, mut integer_index) = (0, 0);
                for point in source_points {
                    control.check()?;
                    account.work(1);
                    let value = match point.value {
                        SampleValueRef::Float(value) => types::Value {
                            kind: types::VALUE_FLOAT,
                            reserved: 0,
                            payload: types::ValuePayload {
                                float_bits: value.to_bits(),
                            },
                        },
                        SampleValueRef::IntegerHistogram(_) => {
                            let value = types::Value {
                                kind: types::VALUE_INTEGER_HISTOGRAM,
                                reserved: 0,
                                payload: types::ValuePayload {
                                    integer_histogram: &integer_histograms[integer_index],
                                },
                            };
                            integer_index += 1;
                            value
                        }
                        SampleValueRef::FloatHistogram(_) => {
                            let value = types::Value {
                                kind: types::VALUE_FLOAT_HISTOGRAM,
                                reserved: 0,
                                payload: types::ValuePayload {
                                    float_histogram: &histograms[float_index],
                                },
                            };
                            float_index += 1;
                            value
                        }
                    };
                    points.push(types::Sample {
                        time_ms: point.time_ms,
                        start_time_ms: point.start_time_ms,
                        value,
                    });
                }
                types::Chunk {
                    kind: types::CHUNK_SERIES,
                    reserved: 0,
                    payload: types::ChunkPayload {
                        series: types::SeriesChunk {
                            series_id: series_id as u64,
                            labels: types::Labels {
                                data: labels.as_ptr(),
                                len: labels.len() as u64,
                            },
                            points: types::Samples {
                                data: points.as_ptr(),
                                len: points.len() as u64,
                            },
                        },
                    },
                }
            }
        };
        control.check()?;
        let mut error = types::Bytes::default();
        let status = unsafe {
            self.table.chunk.expect("validated output chunk")(
                self.table.context,
                &value,
                checked::callback_control(control),
                &mut error,
            )
        };
        unsafe { self.status(status, error, account) }
    }
    fn commit_checked(
        &mut self,
        value: &QueryObservations,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        let value = super::observations::query(value);
        control.check()?;
        let mut error = types::Bytes::default();
        let callback = self.table.commit.expect("validated output commit");
        let status = unsafe {
            callback(
                self.table.context,
                &value,
                checked::callback_control(control),
                &mut error,
            )
        };
        if status == types::OK {
            // Publication is final; no cancellation poll or fallible setup can follow it.
            self.finished = true;
            Ok(())
        } else {
            unsafe { self.status(status, error, account) }
        }
    }
}
