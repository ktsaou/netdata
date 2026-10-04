// SPDX-License-Identifier: GPL-3.0-or-later
//! Borrowed provisional output and an independently owned convenience collector.
use crate::{
    accounting::{AccountedVec, MemoryKind, MemoryLease, QueryAccounting, QueryObservations},
    annotations::Annotations,
    frontend::ast::ValueType,
    histogram::FloatHistogram,
    storage::{LabelRef, LabelsRef},
    value::{SampleRef, SampleValueRef},
};

#[derive(Debug)]
pub struct OwnedLabel {
    pub name: AccountedVec<u8>,
    pub value: AccountedVec<u8>,
}

#[derive(Debug)]
pub struct OwnedLabels(pub AccountedVec<OwnedLabel>);
impl OwnedLabels {
    pub fn new(account: &QueryAccounting, kind: MemoryKind) -> Self {
        Self(AccountedVec::new(account, kind))
    }
    pub fn copy_from(labels: LabelsRef<'_>, account: &QueryAccounting, kind: MemoryKind) -> Self {
        let mut owned = Self::new(account, kind);
        for label in labels {
            owned.push(label.name, label.value, account, kind);
        }
        owned
    }
    pub fn push(&mut self, name: &[u8], value: &[u8], account: &QueryAccounting, kind: MemoryKind) {
        let mut owned_name = AccountedVec::new(account, kind);
        let mut owned_value = AccountedVec::new(account, kind);
        owned_name.extend_from_slice(name);
        owned_value.extend_from_slice(value);
        self.0.push(OwnedLabel {
            name: owned_name,
            value: owned_value,
        });
    }
    pub fn views<'a>(&'a self, account: &QueryAccounting) -> AccountedVec<LabelRef<'a>> {
        let mut result = AccountedVec::new(account, MemoryKind::Workspace);
        for label in self.0.iter() {
            result.push(LabelRef {
                name: &label.name,
                value: &label.value,
            });
        }
        result
    }
    pub fn same(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(other.0.iter())
                .all(|(a, b)| *a.name == *b.name && *a.value == *b.value)
    }
    pub fn compare(&self, other: &Self) -> std::cmp::Ordering {
        for (left, right) in self.0.iter().zip(other.0.iter()) {
            let order = left
                .name
                .cmp(&right.name)
                .then_with(|| left.value.cmp(&right.value));
            if !order.is_eq() {
                return order;
            }
        }
        self.0.len().cmp(&other.0.len())
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&[u8]) -> bool) {
        self.retain_labels(|label| keep(&label.name));
    }
    pub fn retain_labels(&mut self, mut keep: impl FnMut(&OwnedLabel) -> bool) {
        let mut index = 0;
        for next in 0..self.0.len() {
            if keep(&self.0[next]) {
                self.0.swap(index, next);
                index += 1;
            }
        }
        self.0.truncate(index);
    }
}

#[derive(Debug)]
pub enum OwnedValue {
    Float(f64),
    Histogram(OwnedHistogram),
}

#[derive(Debug)]
pub struct OwnedHistogram {
    value: FloatHistogram,
    _allocation: MemoryLease,
}
impl OwnedHistogram {
    pub fn as_ref(&self) -> crate::histogram::FloatHistogramRef<'_> {
        self.value.as_ref()
    }
}
impl OwnedValue {
    pub fn copy_from(
        value: SampleValueRef<'_>,
        account: &QueryAccounting,
        kind: MemoryKind,
    ) -> Self {
        let value = match value {
            SampleValueRef::Float(value) => return Self::Float(value),
            SampleValueRef::IntegerHistogram(value) => value.to_float(),
            SampleValueRef::FloatHistogram(value) => value.to_owned(),
        };
        let bytes = value
            .positive_spans
            .as_ref()
            .map_or(0, |v| v.capacity() * size_of::<crate::histogram::Span>())
            + value
                .negative_spans
                .as_ref()
                .map_or(0, |v| v.capacity() * size_of::<crate::histogram::Span>())
            + value
                .positive_buckets
                .as_ref()
                .map_or(0, |v| v.capacity() * size_of::<f64>())
            + value
                .negative_buckets
                .as_ref()
                .map_or(0, |v| v.capacity() * size_of::<f64>())
            + value
                .custom_values
                .as_ref()
                .map_or(0, |v| size_of_val(v.as_ref()));
        Self::Histogram(OwnedHistogram {
            value,
            _allocation: account.hold(kind, bytes as u64),
        })
    }
    pub fn as_ref(&self) -> SampleValueRef<'_> {
        match self {
            Self::Float(value) => SampleValueRef::Float(*value),
            Self::Histogram(value) => SampleValueRef::FloatHistogram(value.as_ref()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OutputHeader {
    pub value_type: ValueType,
    pub start_ms: i64,
    pub end_ms: i64,
    pub step_ms: i64,
}

#[derive(Debug)]
pub enum OutputChunk<'a> {
    Annotations(&'a Annotations),
    Scalar {
        time_ms: i64,
        value: f64,
    },
    String {
        time_ms: i64,
        value: &'a [u8],
    },
    Series {
        /// Query-local identity survives duplicate projected labelsets.
        series_id: usize,
        labels: LabelsRef<'a>,
        points: &'a [SampleRef<'a>],
    },
}

pub trait OutputSink {
    fn begin(&mut self, header: OutputHeader) -> Result<(), Vec<u8>>;
    fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), Vec<u8>>;
    fn commit(&mut self, observations: &QueryObservations) -> Result<(), Vec<u8>>;
    fn abort(&mut self);
}

#[derive(Debug)]
pub struct OutputPoint {
    pub time_ms: i64,
    pub value: OwnedValue,
}
#[derive(Debug)]
pub struct ResultSeries {
    pub series_id: usize,
    pub labels: OwnedLabels,
    pub points: AccountedVec<OutputPoint>,
}
#[derive(Debug)]
pub struct CollectedResult {
    pub header: OutputHeader,
    pub scalar: Option<(i64, f64)>,
    pub string: Option<(i64, AccountedVec<u8>)>,
    pub series: AccountedVec<ResultSeries>,
    pub annotations: Annotations,
}

pub struct Collector {
    account: QueryAccounting,
    result: Option<CollectedResult>,
    committed: bool,
}
impl Collector {
    pub fn new(account: &QueryAccounting) -> Self {
        Self {
            account: account.clone(),
            result: None,
            committed: false,
        }
    }
    pub fn take(&mut self) -> Option<CollectedResult> {
        if self.committed {
            self.result.take()
        } else {
            None
        }
    }
}
impl OutputSink for Collector {
    fn begin(&mut self, header: OutputHeader) -> Result<(), Vec<u8>> {
        self.committed = false;
        self.result = Some(CollectedResult {
            header,
            scalar: None,
            string: None,
            series: AccountedVec::new(&self.account, MemoryKind::Result),
            annotations: Annotations::default(),
        });
        Ok(())
    }
    fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), Vec<u8>> {
        let result = self.result.as_mut().expect("output begins before chunks");
        match chunk {
            OutputChunk::Annotations(annotations) => result.annotations = annotations.clone(),
            OutputChunk::Scalar { time_ms, value } => result.scalar = Some((time_ms, value)),
            OutputChunk::String { time_ms, value } => {
                let mut bytes = AccountedVec::new(&self.account, MemoryKind::Result);
                bytes.extend_from_slice(value);
                result.string = Some((time_ms, bytes));
            }
            OutputChunk::Series {
                series_id,
                labels,
                points,
            } => {
                let found = result
                    .series
                    .iter()
                    .position(|series| series.series_id == series_id);
                let index = found.unwrap_or_else(|| {
                    let index = result.series.len();
                    result.series.push(ResultSeries {
                        series_id,
                        labels: OwnedLabels::copy_from(labels, &self.account, MemoryKind::Result),
                        points: AccountedVec::new(&self.account, MemoryKind::Result),
                    });
                    index
                });
                for point in points {
                    result.series[index].points.push(OutputPoint {
                        time_ms: point.time_ms,
                        value: OwnedValue::copy_from(
                            point.value,
                            &self.account,
                            MemoryKind::Result,
                        ),
                    });
                }
            }
        }
        Ok(())
    }
    fn commit(&mut self, _: &QueryObservations) -> Result<(), Vec<u8>> {
        if let Some(result) = self.result.as_mut()
            && result.header.value_type == ValueType::Matrix
        {
            result
                .series
                .sort_unstable_by(|left, right| left.labels.compare(&right.labels));
        }
        self.committed = true;
        Ok(())
    }
    fn abort(&mut self) {
        self.committed = false;
        self.result = None;
    }
}
