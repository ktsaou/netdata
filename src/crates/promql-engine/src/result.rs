// SPDX-License-Identifier: GPL-3.0-or-later
//! Borrowed provisional output and an independently owned convenience collector.
use crate::{
    accounting::{
        AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting,
        QueryObservations,
    },
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
    pub(crate) fn set(
        &mut self,
        name: &[u8],
        value: &[u8],
        account: &QueryAccounting,
        kind: MemoryKind,
    ) {
        let position = self.0.binary_search_by(|label| label.name[..].cmp(name));
        if value.is_empty() {
            if let Ok(position) = position {
                self.0.remove(position);
            }
        } else {
            match position {
                Ok(position) => {
                    self.0[position].value.clear();
                    self.0[position].value.extend_from_slice(value);
                }
                Err(position) => {
                    self.push(name, value, account, kind);
                    self.0[position..].rotate_right(1);
                }
            }
        }
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
    payload: AccountedBox<HistogramPayload>,
}
#[derive(Debug)]
struct HistogramPayload {
    value: FloatHistogram,
    context: AllocationContext,
}
impl OwnedHistogram {
    fn new(value: FloatHistogram, context: AllocationContext) -> Self {
        Self {
            payload: AccountedBox::new(
                HistogramPayload {
                    value,
                    context: context.clone(),
                },
                &context,
            ),
        }
    }
    pub fn as_ref(&self) -> crate::histogram::FloatHistogramRef<'_> {
        self.payload.value.as_ref()
    }
    pub(crate) fn copy_from_sample_checked(
        &mut self,
        source: SampleValueRef<'_>,
        control: &crate::storage::Control,
        account: &QueryAccounting,
    ) -> Result<(), crate::storage::Interrupted> {
        let payload = &mut *self.payload;
        let work = crate::histogram_work::HistogramWork::new(control, account);
        match source {
            SampleValueRef::FloatHistogram(value) => {
                value.copy_to_with(&mut payload.value, Some(&payload.context), work)
            }
            SampleValueRef::IntegerHistogram(value) => {
                value.to_float_into_with(&mut payload.value, Some(&payload.context), work)
            }
            SampleValueRef::Float(_) => unreachable!("classified histogram value"),
        }
    }
    pub(crate) fn copy_to_schema_checked(
        &self,
        schema: i32,
        control: &crate::storage::Control,
        account: &QueryAccounting,
    ) -> Result<Self, crate::histogram_work::HistogramEvaluationError> {
        let value = self.as_ref().copy_to_schema_with(
            schema,
            Some(&self.payload.context),
            crate::histogram_work::HistogramWork::new(control, account),
        )?;
        Ok(Self::new(value, self.payload.context.clone()))
    }
    pub(crate) fn arithmetic_checked(
        &mut self,
        other: crate::histogram::FloatHistogramRef<'_>,
        subtract: bool,
        control: &crate::storage::Control,
        account: &QueryAccounting,
    ) -> Result<
        crate::histogram::HistogramArithmetic,
        crate::histogram_work::HistogramEvaluationError,
    > {
        let payload = &mut *self.payload;
        payload.value.add_or_sub_with(
            other,
            subtract,
            Some(&payload.context),
            crate::histogram_work::HistogramWork::new(control, account),
        )
    }
    pub(crate) fn copy(&self) -> Self {
        Self::new(
            self.payload.value.copy_in(Some(&self.payload.context)),
            self.payload.context.clone(),
        )
    }
    pub(crate) fn normalize_copy_fields(&mut self) {
        self.payload.value.normalize_copy_fields();
    }
    pub(crate) fn mul(&mut self, factor: f64) {
        self.payload.value.mul(factor);
    }
    pub(crate) fn div(&mut self, divisor: f64) {
        self.payload.value.div(divisor);
    }
    pub(crate) fn mul_checked(
        &mut self,
        factor: f64,
        control: &crate::storage::Control,
        account: &QueryAccounting,
    ) -> Result<(), crate::storage::Interrupted> {
        self.payload.value.mul_with(factor, || {
            control.check()?;
            account.work(1);
            Ok(())
        })
    }
    pub(crate) fn div_checked(
        &mut self,
        divisor: f64,
        control: &crate::storage::Control,
        account: &QueryAccounting,
    ) -> Result<(), crate::storage::Interrupted> {
        self.payload.value.div_with(divisor, || {
            control.check()?;
            account.work(1);
            Ok(())
        })
    }
    pub(crate) fn compact_checked(
        &mut self,
        control: &crate::storage::Control,
        account: &QueryAccounting,
    ) -> Result<(), crate::storage::Interrupted> {
        let payload = &mut *self.payload;
        payload
            .value
            .compact_with(0, Some(&payload.context), |work| {
                control.check()?;
                account.work(work as u64);
                Ok(())
            })
    }
    pub(crate) fn set_reset_hint(&mut self, hint: u8) {
        self.payload.value.reset_hint = hint;
    }
    pub(crate) fn has_overflow(&self) -> bool {
        let h = self.as_ref();
        h.zero_count.is_infinite()
            || h.count.is_infinite()
            || h.sum.is_infinite()
            || h.positive_buckets
                .into_iter()
                .flatten()
                .chain(h.negative_buckets.into_iter().flatten())
                .chain(h.custom_values.into_iter().flatten())
                .any(|value| value.is_infinite())
    }
    pub(crate) fn add(
        &mut self,
        other: crate::histogram::FloatHistogramRef<'_>,
    ) -> Result<crate::histogram::HistogramArithmetic, crate::histogram::HistogramError> {
        let payload = &mut *self.payload;
        payload.value.add_in(other, Some(&payload.context))
    }
    pub(crate) fn kahan_add(
        &mut self,
        other: crate::histogram::FloatHistogramRef<'_>,
        correction: &mut Option<Self>,
    ) -> Result<crate::histogram::HistogramArithmetic, crate::histogram::HistogramError> {
        // Keep the correction box and its charge while the helper owns its arrays.
        let mut retained = correction.take();
        let mut value = retained
            .as_mut()
            .map(|histogram| std::mem::take(&mut histogram.payload.value));
        let payload = &mut *self.payload;
        let result = payload
            .value
            .kahan_add_in(other, &mut value, Some(&payload.context));
        *correction = value.map(|value| {
            if let Some(mut histogram) = retained {
                histogram.payload.value = value;
                histogram
            } else {
                Self::new(value, payload.context.clone())
            }
        });
        result
    }
}
impl OwnedValue {
    pub(crate) fn kind_bit(&self) -> u8 {
        match self {
            Self::Float(_) => 1,
            Self::Histogram(_) => 2,
        }
    }
    pub fn copy_from(
        value: SampleValueRef<'_>,
        account: &QueryAccounting,
        kind: MemoryKind,
    ) -> Self {
        Self::copy_from_with(
            value,
            account,
            kind,
            crate::histogram_work::HistogramWork::default(),
        )
        .expect("inert histogram work")
    }
    pub(crate) fn copy_from_checked(
        value: SampleValueRef<'_>,
        control: &crate::storage::Control,
        account: &QueryAccounting,
        kind: MemoryKind,
    ) -> Result<Self, crate::storage::Interrupted> {
        Self::copy_from_with(
            value,
            account,
            kind,
            crate::histogram_work::HistogramWork::new(control, account),
        )
    }
    fn copy_from_with(
        value: SampleValueRef<'_>,
        account: &QueryAccounting,
        kind: MemoryKind,
        work: crate::histogram_work::HistogramWork<'_>,
    ) -> Result<Self, crate::storage::Interrupted> {
        work.step(1)?;
        if let SampleValueRef::Float(value) = value {
            return Ok(Self::Float(value));
        }
        let context = AllocationContext::new(account, kind);
        let value = match value {
            SampleValueRef::Float(_) => unreachable!(),
            SampleValueRef::IntegerHistogram(value) => value.to_float_with(Some(&context), work)?,
            SampleValueRef::FloatHistogram(value) => value.to_owned_with(Some(&context), work)?,
        };
        Ok(Self::Histogram(OwnedHistogram::new(value, context)))
    }
    fn copy_result_from(value: SampleValueRef<'_>, account: &QueryAccounting) -> Self {
        let SampleValueRef::FloatHistogram(histogram) = value else {
            return Self::copy_from(value, account, MemoryKind::Result);
        };
        let context = AllocationContext::new(account, MemoryKind::Result);
        Self::Histogram(OwnedHistogram::new(
            histogram.to_owned_result_in(&context),
            context,
        ))
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
                        value: OwnedValue::copy_result_from(point.value, &self.account),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{CUSTOM_BUCKETS_SCHEMA, Span},
    };

    fn histogram(schema: i32, buckets: &[f64], bounds: Option<&[f64]>) -> FloatHistogram {
        FloatHistogram {
            schema,
            count: buckets.iter().sum(),
            sum: 5.0,
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: buckets.len() as u32,
                }]
                .into(),
            ),
            positive_buckets: Some(buckets.to_vec().into()),
            custom_values: bounds.map(|bounds| bounds.to_vec().into()),
            ..FloatHistogram::default()
        }
    }
    fn own(histogram: &FloatHistogram, account: &QueryAccounting) -> OwnedHistogram {
        let OwnedValue::Histogram(owned) = OwnedValue::copy_from(
            SampleValueRef::FloatHistogram(histogram.as_ref()),
            account,
            MemoryKind::Workspace,
        ) else {
            unreachable!()
        };
        owned
    }
    fn box_bytes() -> u64 {
        AccountedBox::<HistogramPayload>::allocation_bytes() as u64
    }

    fn bounds_wrapper_bytes() -> u64 {
        size_of::<crate::shared::Allocation<AccountedVec<f64>>>() as u64
    }

    #[test]
    fn collected_native_results_preserve_transformed_fields_after_source_release() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut value = own(
            &histogram(CUSTOM_BUCKETS_SCHEMA, &[1.0, 2.0], Some(&[1.0])),
            &account,
        );
        value.div(0.0);
        let zero_bits = value.as_ref().zero_count.to_bits();
        assert!(value.as_ref().zero_count.is_nan());
        let mut collector = Collector::new(&account);
        collector
            .begin(OutputHeader {
                value_type: ValueType::Vector,
                start_ms: 1000,
                end_ms: 1000,
                step_ms: 0,
            })
            .unwrap();
        collector
            .chunk(OutputChunk::Series {
                series_id: 7,
                labels: &[],
                points: &[SampleRef {
                    time_ms: 1000,
                    start_time_ms: 0,
                    value: SampleValueRef::FloatHistogram(value.as_ref()),
                }],
            })
            .unwrap();
        collector.commit(&account.snapshot()).unwrap();
        drop(value);
        assert_eq!(account.snapshot().memory[1].current, 0);
        let result = collector.take().unwrap();
        drop(collector);
        drop(account);
        let OwnedValue::Histogram(value) = &result.series[0].points[0].value else {
            unreachable!()
        };
        let value = value.as_ref();
        assert_eq!(value.zero_count.to_bits(), zero_bits);
        assert_eq!(value.count, f64::INFINITY);
        assert_eq!(value.sum, f64::INFINITY);
        assert!(value.positive_spans.is_none());
        assert!(value.positive_buckets.is_none());
        assert_eq!(value.custom_values, Some(&[1.0][..]));
        assert!(engine.snapshot().memory[2].current > 0);
        drop(result);
        assert!(engine.snapshot().memory.iter().all(|m| m.current == 0));
    }

    #[test]
    fn interrupted_native_maps_release_all_attached_buffers() {
        use crate::storage::{Control, Interrupted};
        let engine = EngineAccounting::default();
        let account = engine.query();
        for operation in 0..3 {
            let control = Control::default();
            let mut value = own(&histogram(0, &[0.0, 1.0, 0.0, 2.0, 0.0], None), &account);
            let mut polls = 0;
            let mut poll = |work| {
                polls += 1;
                account.work(work as u64);
                if polls == 4 {
                    control.cancellation.cancel();
                }
                control.check()
            };
            let payload = &mut *value.payload;
            let outcome = match operation {
                0 => payload.value.mul_with(2.0, || poll(1)),
                1 => payload.value.div_with(2.0, || poll(1)),
                _ => payload.value.compact_with(0, Some(&payload.context), poll),
            };
            assert_eq!(outcome, Err(Interrupted::Cancelled));
            assert_eq!(polls, 4);
            drop(value);
            let memory = account.snapshot().memory[1];
            assert_eq!(memory.current, 0);
            assert_eq!(memory.acquired, memory.released);
        }
        assert!(account.snapshot().work >= 12);
    }

    #[test]
    fn float_output_slots_remain_compact_without_native_allocations() {
        assert_eq!(size_of::<OwnedHistogram>(), size_of::<usize>());
        assert!(size_of::<OwnedValue>() <= 16);
        assert!(size_of::<OutputPoint>() <= 24);
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut points = AccountedVec::new(&account, MemoryKind::Result);
        for time_ms in 0..100_000 {
            points.push(OutputPoint {
                time_ms,
                value: OwnedValue::Float(2.0),
            });
        }
        assert_eq!(
            account.snapshot().memory[2].current,
            (points.capacity() * size_of::<OutputPoint>()) as u64
        );
        assert_eq!(account.snapshot().memory[1].current, 0);
        drop(account);
        assert!(engine.snapshot().memory[2].current > 0);
        drop(points);
        assert_eq!(engine.snapshot().memory[2].current, 0);
    }

    #[test]
    fn scalar_native_sum_reuses_the_retained_compensation_box() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let operand = FloatHistogram {
            count: 1.0,
            sum: 2.0,
            ..FloatHistogram::default()
        };
        let mut sum = own(&operand, &account);
        let mut correction = None;
        for _ in 0..1024 {
            sum.kahan_add(operand.as_ref(), &mut correction).unwrap();
        }
        assert_eq!((sum.as_ref().count, sum.as_ref().sum), (1025.0, 2050.0));
        let observation = account.snapshot().memory[1];
        assert_eq!(observation.current, 2 * box_bytes());
        assert_eq!(observation.acquired, 2 * box_bytes());
        drop(sum);
        assert_eq!(account.snapshot().memory[1].current, box_bytes());
        drop(account);
        assert_eq!(engine.snapshot().memory[1].current, box_bytes());
        drop(correction);
        assert_eq!(engine.snapshot().memory[1].current, 0);
    }

    #[test]
    fn incompatible_input_preserves_the_retained_compensation_on_error() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let operand = histogram(0, &[1.0], None);
        let mut sum = own(&operand, &account);
        let mut correction = None;
        sum.kahan_add(operand.as_ref(), &mut correction).unwrap();
        let before = account.snapshot().memory[1];
        let custom = histogram(CUSTOM_BUCKETS_SCHEMA, &[1.0, 2.0], Some(&[1.0]));
        assert_eq!(
            sum.kahan_add(custom.as_ref(), &mut correction),
            Err(crate::histogram::HistogramError::IncompatibleSchema)
        );
        assert_eq!(account.snapshot().memory[1], before);
        assert_eq!(sum.as_ref().positive_buckets, Some(&[2.0][..]));
        assert_eq!(
            correction.as_ref().unwrap().as_ref().positive_buckets,
            Some(&[0.0][..])
        );
        drop(sum);
        drop(correction);
        assert_eq!(account.snapshot().memory[1].current, 0);
    }

    #[test]
    fn arithmetic_unwind_releases_a_temporarily_emptied_compensation_box() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let operand = histogram(0, &[1.0], None);
            let mut sum = own(&operand, &account);
            let mut correction = None;
            sum.kahan_add(operand.as_ref(), &mut correction).unwrap();
            let mut malformed = histogram(2, &[1.0, 2.0], None);
            malformed.positive_spans.as_mut().unwrap()[0].length = 8;
            sum.kahan_add(malformed.as_ref(), &mut correction).unwrap();
        }));
        assert!(panic.is_err());
        let observation = account.snapshot().memory[1];
        assert_eq!(observation.current, 0);
        assert_eq!(observation.acquired, observation.released);
    }

    #[test]
    fn mutable_copies_are_independent_and_shared_bounds_keep_one_lifetime_charge() {
        for drop_original_first in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let source = histogram(CUSTOM_BUCKETS_SCHEMA, &[1.0, 2.0, 3.0], Some(&[1.0, 2.0]));
            let original = own(&source, &account);
            assert_eq!(
                account.snapshot().memory[1].current,
                80 + box_bytes() + bounds_wrapper_bytes()
            );
            let mut copy = original.copy();
            assert!(
                original
                    .payload
                    .value
                    .custom_values
                    .as_ref()
                    .unwrap()
                    .same_allocation(copy.payload.value.custom_values.as_ref().unwrap())
            );
            copy.payload.value.positive_buckets.as_mut().unwrap()[0] = 7.0;
            assert_eq!(
                original.as_ref().positive_buckets,
                Some(&[1.0, 2.0, 3.0][..])
            );
            assert_eq!(
                account.snapshot().memory[1].current,
                144 + 2 * box_bytes() + bounds_wrapper_bytes()
            );
            let remaining = if drop_original_first {
                drop(original);
                copy
            } else {
                drop(copy);
                original
            };
            assert_eq!(
                account.snapshot().memory[1].current,
                80 + box_bytes() + bounds_wrapper_bytes()
            );
            drop(account);
            drop(source);
            assert_eq!(remaining.as_ref().custom_values, Some(&[1.0, 2.0][..]));
            assert_eq!(
                engine.snapshot().memory[1].current,
                80 + box_bytes() + bounds_wrapper_bytes()
            );
            drop(remaining);
            assert_eq!(engine.snapshot().memory[1].current, 0);
        }
    }

    #[test]
    fn absent_arrays_retain_the_query_factory_for_new_sum_and_compensation() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut sum = own(&FloatHistogram::default(), &account);
        assert_eq!(account.snapshot().memory[1].current, box_bytes());
        let operand = histogram(0, &[1.0, 2.0, 3.0], None);
        let mut correction = None;
        sum.kahan_add(operand.as_ref(), &mut correction).unwrap();
        let correction = correction.unwrap();
        assert_eq!(sum.as_ref().positive_buckets, Some(&[1.0, 2.0, 3.0][..]));
        for value in [&sum.payload.value, &correction.payload.value] {
            assert!(value.positive_spans.as_ref().unwrap().context().is_some());
            assert!(value.positive_buckets.as_ref().unwrap().context().is_some());
        }
        assert_eq!(account.snapshot().memory[1].current, 104 + 2 * box_bytes());
        drop(sum);
        drop(correction);
        let observation = account.snapshot().memory[1];
        assert_eq!(observation.current, 0);
        assert_eq!(observation.acquired, observation.released);
    }

    #[test]
    fn reduced_operand_scratch_is_observed_while_the_receiver_is_retained() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut sum = own(&histogram(0, &[1.0], None), &account);
        let baseline = account.snapshot().memory[1].current;
        let operand = histogram(2, &[1.0; 8], None);
        sum.add(operand.as_ref()).unwrap();
        let observation = account.snapshot().memory[1];
        assert_eq!(observation.current, baseline);
        assert!(observation.peak >= baseline + 64);
        assert_eq!(sum.as_ref().positive_buckets, Some(&[2.0, 4.0, 3.0][..]));
        drop(sum);
        assert_eq!(account.snapshot().memory[1].current, 0);
    }

    #[test]
    fn ordinary_custom_add_observes_intersection_and_temporary_compensation() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let source = histogram(CUSTOM_BUCKETS_SCHEMA, &[1.0, 2.0, 3.0], Some(&[1.0, 2.0]));
        let operand = histogram(CUSTOM_BUCKETS_SCHEMA, &[1.0, 2.0, 3.0], Some(&[2.0, 3.0]));
        let mut sum = own(&source, &account);
        let flags = sum.add(operand.as_ref()).unwrap();
        assert!(flags.nhcb_bounds_reconciled);
        assert_eq!(sum.as_ref().custom_values, Some(&[2.0][..]));
        assert_eq!(sum.as_ref().positive_buckets, Some(&[4.0, 8.0][..]));
        let observation = account.snapshot().memory[1];
        assert_eq!(
            (observation.current, observation.peak),
            (
                64 + box_bytes() + bounds_wrapper_bytes(),
                128 + box_bytes() + 2 * bounds_wrapper_bytes()
            )
        );
        drop(sum);
        let observation = account.snapshot().memory[1];
        assert_eq!(observation.current, 0);
        assert_eq!(observation.acquired, observation.released);
    }

    #[test]
    fn malformed_reduction_releases_receiver_and_partly_allocated_scratch_on_unwind() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut sum = own(&histogram(0, &[1.0], None), &account);
            let mut malformed = histogram(2, &[1.0, 2.0], None);
            malformed.positive_spans.as_mut().unwrap()[0].length = 8;
            sum.add(malformed.as_ref()).unwrap();
        }));
        assert!(panic.is_err());
        let observation = account.snapshot().memory[1];
        assert_eq!(observation.current, 0);
        assert!(observation.peak >= 128);
        assert_eq!(observation.acquired, observation.released);
    }
}
