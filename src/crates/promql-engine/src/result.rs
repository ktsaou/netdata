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
    histogram_work::HistogramWork,
    statistics::OwnedSamples,
    storage::{Control, Interrupted, LabelRef, LabelsRef},
    value::{SampleRef, SampleValueRef},
};

#[derive(Debug)]
pub struct OwnedLabel {
    pub name: AccountedVec<u8>,
    pub value: AccountedVec<u8>,
}

#[derive(Debug)]
pub struct OwnedLabels(pub AccountedVec<OwnedLabel>);
pub(crate) fn append_bytes_checked(
    target: &mut AccountedVec<u8>,
    bytes: &[u8],
    account: &QueryAccounting,
    control: &Control,
) -> Result<(), Interrupted> {
    append_bytes_with(target, bytes, HistogramWork::new(control, account))
}
fn append_bytes_with(
    target: &mut AccountedVec<u8>,
    bytes: &[u8],
    work: HistogramWork<'_>,
) -> Result<(), Interrupted> {
    for chunk in bytes.chunks(1024) {
        work.step(chunk.len())?;
        target.extend_from_slice(chunk);
    }
    Ok(())
}
fn copy_label_with(
    label: LabelRef<'_>,
    account: &QueryAccounting,
    kind: MemoryKind,
    work: HistogramWork<'_>,
) -> Result<OwnedLabel, Interrupted> {
    let context = AllocationContext::new(account, kind);
    let mut name = AccountedVec::with_capacity_in(label.name.len(), Some(&context));
    let mut value = AccountedVec::with_capacity_in(label.value.len(), Some(&context));
    append_bytes_with(&mut name, label.name, work)?;
    append_bytes_with(&mut value, label.value, work)?;
    Ok(OwnedLabel { name, value })
}
impl OwnedLabels {
    pub fn new(account: &QueryAccounting, kind: MemoryKind) -> Self {
        Self(AccountedVec::new(account, kind))
    }
    pub fn copy_from(labels: LabelsRef<'_>, account: &QueryAccounting, kind: MemoryKind) -> Self {
        Self::copy_labels_with(
            labels.iter().copied(),
            account,
            kind,
            HistogramWork::default(),
        )
        .expect("inert label copy")
    }
    pub(crate) fn copy_from_checked(
        labels: LabelsRef<'_>,
        account: &QueryAccounting,
        kind: MemoryKind,
        control: &crate::storage::Control,
    ) -> Result<Self, Interrupted> {
        Self::copy_labels_with(
            labels.iter().copied(),
            account,
            kind,
            HistogramWork::new(control, account),
        )
    }
    fn copy_labels_with<'a>(
        labels: impl ExactSizeIterator<Item = LabelRef<'a>>,
        account: &QueryAccounting,
        kind: MemoryKind,
        work: HistogramWork<'_>,
    ) -> Result<Self, Interrupted> {
        let capacity_hint = labels.len();
        let mut result = Self::new(account, kind);
        for label in labels {
            work.step(1)?;
            result.0.push_with_capacity_hint(
                copy_label_with(label, account, kind, work)?,
                capacity_hint,
            );
        }
        work.step(1)?;
        Ok(result)
    }
    pub(crate) fn copy_filtered_checked(
        &self,
        account: &QueryAccounting,
        kind: MemoryKind,
        control: &Control,
        mut keep: impl FnMut(&OwnedLabel) -> Result<bool, Interrupted>,
    ) -> Result<Self, Interrupted> {
        let work = HistogramWork::new(control, account);
        let capacity_hint = self.0.len();
        let mut result = Self::new(account, kind);
        for label in &*self.0 {
            work.step(1)?;
            if keep(label)? {
                result.0.push_with_capacity_hint(
                    copy_label_with(
                        LabelRef {
                            name: &label.name,
                            value: &label.value,
                        },
                        account,
                        kind,
                        work,
                    )?,
                    capacity_hint,
                );
            }
        }
        work.step(1)?;
        Ok(result)
    }
    fn push_owned(&mut self, label: OwnedLabel) {
        // Compact copies can end between ordinary geometric growth boundaries.
        let capacity_hint = if self.0.len() == self.0.capacity() {
            self.0
                .len()
                .saturating_add(1)
                .checked_next_power_of_two()
                .unwrap_or(usize::MAX)
                .max(4)
        } else {
            usize::MAX
        };
        self.0.push_with_capacity_hint(label, capacity_hint);
    }
    pub fn push(&mut self, name: &[u8], value: &[u8], account: &QueryAccounting, kind: MemoryKind) {
        let mut owned_name = AccountedVec::new(account, kind);
        let mut owned_value = AccountedVec::new(account, kind);
        owned_name.extend_from_slice(name);
        owned_value.extend_from_slice(value);
        self.push_owned(OwnedLabel {
            name: owned_name,
            value: owned_value,
        });
    }
    pub(crate) fn copy_checked(
        &self,
        account: &QueryAccounting,
        kind: MemoryKind,
        control: &Control,
    ) -> Result<Self, Interrupted> {
        Self::copy_labels_with(
            self.0.iter().map(|label| LabelRef {
                name: &label.name,
                value: &label.value,
            }),
            account,
            kind,
            HistogramWork::new(control, account),
        )
    }
    pub(crate) fn position(
        &self,
        name: &[u8],
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<Result<usize, usize>, Interrupted> {
        control.check()?;
        let (mut left, mut right) = (0, self.0.len());
        while left < right {
            control.check()?;
            let middle = left + (right - left) / 2;
            account.work(1);
            match crate::binary_labels::compare(&self.0[middle].name, name, control, account)? {
                std::cmp::Ordering::Less => left = middle + 1,
                std::cmp::Ordering::Greater => right = middle,
                std::cmp::Ordering::Equal => {
                    control.check()?;
                    return Ok(Ok(middle));
                }
            }
        }
        control.check()?;
        Ok(Err(left))
    }
    pub(crate) fn get_checked(
        &self,
        name: &[u8],
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<&[u8], Interrupted> {
        Ok(self
            .position(name, account, control)?
            .map_or(&b""[..], |position| &self.0[position].value))
    }
    pub(crate) fn set(
        &mut self,
        name: &[u8],
        value: &[u8],
        account: &QueryAccounting,
        kind: MemoryKind,
        control: &Control,
    ) -> Result<(), Interrupted> {
        let position = self.position(name, account, control)?;
        if value.is_empty() {
            if let Ok(position) = position {
                for next in position..self.0.len() - 1 {
                    control.check()?;
                    account.work(1);
                    self.0.swap(next, next + 1);
                }
                control.check()?;
                account.work(1);
                drop(self.0.pop());
            }
        } else {
            match position {
                Ok(position) => {
                    self.0[position].value.clear();
                    append_bytes_checked(&mut self.0[position].value, value, account, control)?;
                }
                Err(position) => {
                    let mut owned_name = AccountedVec::new(account, kind);
                    let mut owned_value = AccountedVec::new(account, kind);
                    append_bytes_checked(&mut owned_name, name, account, control)?;
                    append_bytes_checked(&mut owned_value, value, account, control)?;
                    self.push_owned(OwnedLabel {
                        name: owned_name,
                        value: owned_value,
                    });
                    for next in (position + 1..self.0.len()).rev() {
                        control.check()?;
                        account.work(1);
                        self.0.swap(next - 1, next);
                    }
                }
            }
        }
        control.check()
    }
    pub(crate) fn reset_checked(
        &mut self,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<(), Interrupted> {
        self.retain_labels_checked(account, control, |label| Ok(!label.value.is_empty()))
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
    pub(crate) fn views_checked<'a>(
        &'a self,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<AccountedVec<LabelRef<'a>>, Interrupted> {
        let mut result = AccountedVec::new(account, MemoryKind::Workspace);
        for label in &*self.0 {
            control.check()?;
            account.work(1);
            result.push(LabelRef {
                name: &label.name,
                value: &label.value,
            });
        }
        control.check()?;
        Ok(result)
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
    pub(crate) fn compare_checked(
        &self,
        other: &Self,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<std::cmp::Ordering, Interrupted> {
        for (left, right) in self.0.iter().zip(other.0.iter()) {
            for (left, right) in [(&left.name, &right.name), (&left.value, &right.value)] {
                let order = crate::binary_labels::compare(left, right, control, account)?;
                if !order.is_eq() {
                    return Ok(order);
                }
            }
        }
        control.check()?;
        Ok(self.0.len().cmp(&other.0.len()))
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&[u8]) -> bool) {
        self.retain_labels(|label| keep(&label.name));
    }
    pub fn retain_labels(&mut self, mut keep: impl FnMut(&OwnedLabel) -> bool) {
        self.retain_labels_with(|label| Ok::<_, std::convert::Infallible>(keep(label)))
            .unwrap();
    }
    pub(crate) fn retain_labels_with<E>(
        &mut self,
        keep: impl FnMut(&OwnedLabel) -> Result<bool, E>,
    ) -> Result<(), E> {
        self.retain_labels_steps(keep, || Ok(()))
    }
    pub(crate) fn retain_labels_checked<E: From<Interrupted>>(
        &mut self,
        account: &QueryAccounting,
        control: &Control,
        keep: impl FnMut(&OwnedLabel) -> Result<bool, E>,
    ) -> Result<(), E> {
        self.retain_labels_steps(keep, || {
            control.check().map_err(E::from)?;
            account.work(1);
            Ok(())
        })
    }
    fn retain_labels_steps<E>(
        &mut self,
        mut keep: impl FnMut(&OwnedLabel) -> Result<bool, E>,
        mut step: impl FnMut() -> Result<(), E>,
    ) -> Result<(), E> {
        step()?;
        let mut index = 0;
        for next in 0..self.0.len() {
            step()?;
            if keep(&self.0[next])? {
                step()?;
                self.0.swap(index, next);
                index += 1;
            }
        }
        // Normal suffix release is observable; interruption leaves infallible owner cleanup.
        while self.0.len() > index {
            step()?;
            drop(self.0.pop());
        }
        step()
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
    samples: OwnedSamples,
}

struct HistogramMutation<'a>(&'a mut HistogramPayload);
impl std::ops::Deref for HistogramMutation<'_> {
    type Target = HistogramPayload;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}
impl std::ops::DerefMut for HistogramMutation<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}
impl Drop for HistogramMutation<'_> {
    fn drop(&mut self) {
        self.0
            .samples
            .resize(crate::statistics::weight(SampleValueRef::FloatHistogram(
                self.0.value.as_ref(),
            )));
    }
}
impl OwnedHistogram {
    fn new(value: FloatHistogram, context: AllocationContext) -> Self {
        let samples = OwnedSamples::new(
            context.accounting(),
            crate::statistics::weight(SampleValueRef::FloatHistogram(value.as_ref())),
        );
        Self {
            payload: AccountedBox::new(
                HistogramPayload {
                    value,
                    context: context.clone(),
                    samples,
                },
                &context,
            ),
        }
    }
    pub fn as_ref(&self) -> crate::histogram::FloatHistogramRef<'_> {
        self.payload.value.as_ref()
    }
    fn mutation(&mut self) -> HistogramMutation<'_> {
        HistogramMutation(&mut self.payload)
    }
    pub(crate) fn copy_from_sample_checked(
        &mut self,
        source: SampleValueRef<'_>,
        control: &crate::storage::Control,
        account: &QueryAccounting,
    ) -> Result<(), crate::storage::Interrupted> {
        let mut mutation = self.mutation();
        let payload = &mut *mutation;
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
    pub(crate) fn retain_from_sample_checked(
        &mut self,
        source: SampleValueRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut mutation = self.mutation();
        let payload = &mut *mutation;
        let work = HistogramWork::new(control, account);
        match source {
            SampleValueRef::FloatHistogram(value) => {
                value.retain_fields_to_with(&mut payload.value, &payload.context, work)
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
        let mut mutation = self.mutation();
        let payload = &mut *mutation;
        payload.value.add_or_sub_with(
            other,
            subtract,
            Some(&payload.context),
            crate::histogram_work::HistogramWork::new(control, account),
        )
    }
    #[cfg(test)]
    pub(crate) fn copy(&self) -> Self {
        self.copy_with(HistogramWork::default())
            .expect("inert histogram work")
    }
    pub(crate) fn copy_checked(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, Interrupted> {
        self.copy_with(HistogramWork::new(control, account))
    }
    fn copy_with(&self, work: HistogramWork<'_>) -> Result<Self, Interrupted> {
        Ok(Self::new(
            self.payload
                .value
                .copy_with(Some(&self.payload.context), work)?,
            self.payload.context.clone(),
        ))
    }
    pub(crate) fn normalize_copy_fields(&mut self) {
        self.mutation().value.normalize_copy_fields();
    }
    #[cfg(test)]
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
        let mut mutation = self.mutation();
        let payload = &mut *mutation;
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
    pub(crate) fn trim_checked(
        &mut self,
        threshold: f64,
        upper: bool,
        control: &crate::storage::Control,
        account: &QueryAccounting,
    ) -> Result<(), crate::storage::StorageError> {
        let mut mutation = self.mutation();
        let payload = &mut *mutation;
        payload.value.trim_buckets_with(
            threshold,
            upper,
            Some(&payload.context),
            crate::histogram_work::HistogramWork::new(control, account),
        )?;
        Ok(())
    }
    pub(crate) fn has_overflow_checked(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        self.as_ref()
            .has_overflow_with(HistogramWork::new(control, account))
    }
    #[cfg(test)]
    pub(crate) fn kahan_add(
        &mut self,
        other: crate::histogram::FloatHistogramRef<'_>,
        correction: &mut Option<Self>,
    ) -> Result<crate::histogram::HistogramArithmetic, crate::histogram::HistogramError> {
        self.kahan_add_with(other, correction, HistogramWork::default())
            .map_err(crate::histogram_work::HistogramEvaluationError::arithmetic)
    }
    pub(crate) fn kahan_add_checked(
        &mut self,
        other: crate::histogram::FloatHistogramRef<'_>,
        correction: &mut Option<Self>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<
        crate::histogram::HistogramArithmetic,
        crate::histogram_work::HistogramEvaluationError,
    > {
        self.kahan_add_with(other, correction, HistogramWork::new(control, account))
    }
    fn kahan_add_with(
        &mut self,
        other: crate::histogram::FloatHistogramRef<'_>,
        correction: &mut Option<Self>,
        work: HistogramWork<'_>,
    ) -> Result<
        crate::histogram::HistogramArithmetic,
        crate::histogram_work::HistogramEvaluationError,
    > {
        // Keep the correction box and its charge while the helper owns its arrays.
        let mut retained = correction.take();
        let mut value = retained
            .as_mut()
            .map(|histogram| std::mem::take(&mut histogram.payload.value));
        let mut mutation = self.mutation();
        let payload = &mut *mutation;
        let result = payload
            .value
            .kahan_add_with(other, &mut value, Some(&payload.context), work);
        *correction = value.map(|value| {
            if let Some(mut histogram) = retained {
                histogram.payload.value = value;
                let units = crate::statistics::weight(SampleValueRef::FloatHistogram(
                    histogram.payload.value.as_ref(),
                ));
                histogram.payload.samples.resize(units);
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
    fn copy_result_with(
        value: SampleValueRef<'_>,
        account: &QueryAccounting,
        work: HistogramWork<'_>,
    ) -> Result<Self, Interrupted> {
        Self::from_sample_with(value, account, MemoryKind::Result, work)
    }
    pub(crate) fn from_sample_checked(
        value: SampleValueRef<'_>,
        control: &crate::storage::Control,
        account: &QueryAccounting,
        kind: MemoryKind,
    ) -> Result<Self, Interrupted> {
        Self::from_sample_with(value, account, kind, HistogramWork::new(control, account))
    }
    fn from_sample_with(
        value: SampleValueRef<'_>,
        account: &QueryAccounting,
        kind: MemoryKind,
        work: HistogramWork<'_>,
    ) -> Result<Self, Interrupted> {
        let SampleValueRef::FloatHistogram(histogram) = value else {
            return Self::copy_from_with(value, account, kind, work);
        };
        let context = AllocationContext::new(account, kind);
        Ok(Self::Histogram(OwnedHistogram::new(
            histogram.to_owned_fields_with(&context, work)?,
            context,
        )))
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
    fn begin(&mut self, header: OutputHeader) -> Result<(), AccountedVec<u8>>;
    fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), AccountedVec<u8>>;
    fn commit(&mut self, observations: &QueryObservations) -> Result<(), AccountedVec<u8>>;
    fn abort(&mut self);
    fn begin_checked(
        &mut self,
        header: OutputHeader,
        control: &Control,
        _account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        control.check()?;
        self.begin(header).map_err(OutputError::Failure)
    }
    fn chunk_checked(
        &mut self,
        chunk: OutputChunk<'_>,
        control: &Control,
        _account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        control.check()?;
        self.chunk(chunk).map_err(OutputError::Failure)
    }
    fn commit_checked(
        &mut self,
        observations: &QueryObservations,
        control: &Control,
        _account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        control.check()?;
        self.commit(observations).map_err(OutputError::Failure)
    }
}

#[derive(Debug)]
pub enum OutputError {
    Failure(AccountedVec<u8>),
    Interrupted(Interrupted),
}
impl From<Interrupted> for OutputError {
    fn from(error: Interrupted) -> Self {
        Self::Interrupted(error)
    }
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
    float_samples: OwnedSamples,
}

pub struct Collector {
    account: QueryAccounting,
    result: Option<CollectedResult>,
    committed: bool,
    positional_ids: bool,
}
impl Collector {
    pub fn new(account: &QueryAccounting) -> Self {
        Self {
            account: account.clone(),
            result: None,
            committed: false,
            positional_ids: true,
        }
    }
    pub fn take(&mut self) -> Option<CollectedResult> {
        if self.committed {
            self.result.take()
        } else {
            None
        }
    }
    fn begin_output(&mut self, header: OutputHeader) {
        self.committed = false;
        self.positional_ids = true;
        self.result = Some(CollectedResult {
            header,
            scalar: None,
            string: None,
            series: AccountedVec::new(&self.account, MemoryKind::Result),
            annotations: Annotations::default(),
            float_samples: OwnedSamples::new(&self.account, 0),
        });
    }
    fn chunk_with(
        &mut self,
        chunk: OutputChunk<'_>,
        control: Option<&Control>,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let work = control.map_or_else(HistogramWork::default, |control| {
            HistogramWork::new(control, account)
        });
        work.step(1)?;
        let result = self.result.as_mut().expect("output begins before chunks");
        match chunk {
            OutputChunk::Annotations(annotations) => result.annotations = annotations.clone(),
            OutputChunk::Scalar { time_ms, value } => {
                if result.scalar.is_none() {
                    result.float_samples.add(1);
                }
                result.scalar = Some((time_ms, value));
            }
            OutputChunk::String { time_ms, value } => {
                let mut bytes = AccountedVec::with_capacity_in(
                    value.len(),
                    Some(&AllocationContext::new(&self.account, MemoryKind::Result)),
                );
                append_bytes_with(&mut bytes, value, work)?;
                result.string = Some((time_ms, bytes));
            }
            OutputChunk::Series {
                series_id,
                labels,
                points,
            } => {
                let found = if self.positional_ids {
                    work.step(1)?;
                    // Positional identities prove both direct hits and missing new IDs.
                    (series_id < result.series.len()).then_some(series_id)
                } else {
                    let mut found = None;
                    for (index, series) in result.series.iter().enumerate() {
                        work.step(1)?;
                        if series.series_id == series_id {
                            found = Some(index);
                            break;
                        }
                    }
                    found
                };
                let index = if let Some(index) = found {
                    index
                } else {
                    let index = result.series.len();
                    result.series.push(ResultSeries {
                        series_id,
                        labels: OwnedLabels::copy_labels_with(
                            labels.iter().copied(),
                            &self.account,
                            MemoryKind::Result,
                            work,
                        )?,
                        points: AccountedVec::new(&self.account, MemoryKind::Result),
                    });
                    self.positional_ids &= series_id == index;
                    index
                };
                for point in points {
                    work.step(1)?;
                    result.series[index].points.push(OutputPoint {
                        time_ms: point.time_ms,
                        value: OwnedValue::copy_result_with(point.value, &self.account, work)?,
                    });
                    if matches!(point.value, SampleValueRef::Float(_)) {
                        result.float_samples.add(1);
                    }
                }
            }
        }
        if let Some(control) = control {
            control.check()?;
        }
        Ok(())
    }
    fn commit_with(
        &mut self,
        control: Option<&Control>,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if let Some(result) = self.result.as_mut()
            && result.header.value_type == ValueType::Matrix
        {
            // A failed sort can already have moved rows away from their ID positions.
            self.positional_ids = false;
            // Source sort.Sort determines observable provenance among equal projected labels.
            crate::go_sort::try_sort_by_with(
                &mut result.series,
                |left, right| match control {
                    Some(control) => left.labels.compare_checked(&right.labels, account, control),
                    None => Ok(left.labels.compare(&right.labels)),
                },
                || {
                    if let Some(control) = control {
                        control.check()?;
                    }
                    account.work(1);
                    Ok(())
                },
            )?;
        }
        if let Some(control) = control {
            control.check()?;
        }
        self.committed = true;
        Ok(())
    }
}
impl OutputSink for Collector {
    fn begin(&mut self, header: OutputHeader) -> Result<(), AccountedVec<u8>> {
        self.begin_output(header);
        Ok(())
    }
    fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), AccountedVec<u8>> {
        let account = self.account.clone();
        self.chunk_with(chunk, None, &account)
            .expect("inert output copy");
        Ok(())
    }
    fn commit(&mut self, _: &QueryObservations) -> Result<(), AccountedVec<u8>> {
        let account = self.account.clone();
        self.commit_with(None, &account)
            .expect("inert output ordering");
        Ok(())
    }
    fn begin_checked(
        &mut self,
        header: OutputHeader,
        control: &Control,
        _account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        control.check()?;
        self.begin_output(header);
        Ok(())
    }
    fn chunk_checked(
        &mut self,
        chunk: OutputChunk<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        self.chunk_with(chunk, Some(control), account)
            .map_err(OutputError::from)
    }
    fn commit_checked(
        &mut self,
        _: &QueryObservations,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), OutputError> {
        self.commit_with(Some(control), account)
            .map_err(OutputError::from)
    }
    fn abort(&mut self) {
        self.committed = false;
        self.result = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod collector_lookup {
        use super::*;

        fn header(value_type: ValueType) -> OutputHeader {
            OutputHeader {
                value_type,
                start_ms: 0,
                end_ms: 2000,
                step_ms: 1000,
            }
        }

        fn put(
            collector: &mut Collector,
            checked: bool,
            context: (&QueryAccounting, &Control),
            series_id: usize,
            label: &[u8],
            point: (i64, f64),
        ) -> Result<(), OutputError> {
            let labels = [LabelRef {
                name: b"id",
                value: label,
            }];
            let points = [SampleRef {
                time_ms: point.0,
                start_time_ms: 0,
                value: SampleValueRef::Float(point.1),
            }];
            let chunk = OutputChunk::Series {
                series_id,
                labels: &labels,
                points: &points,
            };
            if checked {
                collector.chunk_checked(chunk, context.1, context.0)
            } else {
                collector.chunk(chunk).map_err(OutputError::Failure)
            }
        }

        fn finish(
            collector: &mut Collector,
            checked: bool,
            account: &QueryAccounting,
            control: &Control,
        ) -> Result<(), OutputError> {
            if checked {
                collector.commit_checked(&account.snapshot(), control, account)
            } else {
                collector
                    .commit(&account.snapshot())
                    .map_err(OutputError::Failure)
            }
        }

        fn float_bits(point: &OutputPoint) -> u64 {
            let OwnedValue::Float(value) = &point.value else {
                panic!("expected an independently owned float")
            };
            value.to_bits()
        }

        #[test]
        fn dense_rows_preserve_complete_results_before_linear_work_bound() {
            let mut measurements = Vec::new();
            for length in [1usize, 16, 256] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let mut collector = Collector::new(&account);
                collector.begin(header(ValueType::Matrix)).unwrap();
                for tick in 0..3 {
                    for offset in 0..length {
                        let series_id = if tick == 1 {
                            length - offset - 1
                        } else {
                            offset
                        };
                        let label = if tick == 0 {
                            format!("{series_id:05}")
                        } else {
                            "ignored".into()
                        };
                        put(
                            &mut collector,
                            true,
                            (&account, &control),
                            series_id,
                            label.as_bytes(),
                            (tick * 1000, (tick * 1000) as f64 + series_id as f64),
                        )
                        .unwrap();
                    }
                }
                let work = account.snapshot().work;
                assert!(collector.take().is_none());
                finish(&mut collector, true, &account, &control).unwrap();
                let output = collector.take().unwrap();
                assert_eq!(output.series.len(), length);
                for (series_id, row) in output.series.iter().enumerate() {
                    assert_eq!(row.series_id, series_id);
                    assert_eq!(&*row.labels.0[0].name, b"id");
                    assert_eq!(
                        &*row.labels.0[0].value,
                        format!("{series_id:05}").as_bytes()
                    );
                    assert_eq!(row.points.len(), 3);
                    for (tick, point) in row.points.iter().enumerate() {
                        assert_eq!(point.time_ms, tick as i64 * 1000);
                        assert_eq!(
                            float_bits(point),
                            (tick as f64 * 1000.0 + series_id as f64).to_bits()
                        );
                    }
                }
                drop((output, collector));
                assert_eq!(account.current_samples(), 0);
                account.snapshot().assert_memory_released();
                measurements.push((length, work));
            }
            println!("complete dense Collector results; pre-commit work: {measurements:?}");
            for (length, work) in measurements {
                assert!(
                    work <= 64 * length as u64,
                    "repeated identity scans: N={length}, work={work}"
                );
            }
        }

        #[test]
        fn arbitrary_ids_keep_first_labels_and_point_arrival_order() {
            let cases = [
                (
                    vec![0, 1, 9, 0, usize::MAX, 1, 2, 9],
                    vec![
                        (0, vec![0, 3]),
                        (1, vec![1, 5]),
                        (9, vec![2, 7]),
                        (usize::MAX, vec![4]),
                        (2, vec![6]),
                    ],
                ),
                (
                    vec![2, 0, 1, 2, 0],
                    vec![(2, vec![0, 3]), (0, vec![1, 4]), (1, vec![2])],
                ),
                (
                    vec![usize::MAX, 0, usize::MAX],
                    vec![(usize::MAX, vec![0, 2]), (0, vec![1])],
                ),
            ];
            for checked in [false, true] {
                for (ids, expected) in &cases {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let mut collector = Collector::new(&account);
                    collector.begin(header(ValueType::Vector)).unwrap();
                    for (ordinal, &series_id) in ids.iter().enumerate() {
                        let mut label = format!("packet{ordinal}").into_bytes();
                        put(
                            &mut collector,
                            checked,
                            (&account, &control),
                            series_id,
                            &label,
                            (ordinal as i64 * 1000, ordinal as f64),
                        )
                        .unwrap();
                        label.fill(b'x');
                    }
                    finish(&mut collector, checked, &account, &control).unwrap();
                    let output = collector.take().unwrap();
                    assert_eq!(output.series.len(), expected.len());
                    for (row, (series_id, positions)) in output.series.iter().zip(expected) {
                        assert_eq!(row.series_id, *series_id);
                        assert_eq!(
                            &*row.labels.0[0].value,
                            format!("packet{}", positions[0]).as_bytes()
                        );
                        assert_eq!(row.points.len(), positions.len());
                        for (point, &position) in row.points.iter().zip(positions) {
                            assert_eq!(point.time_ms, position as i64 * 1000);
                            assert_eq!(float_bits(point), (position as f64).to_bits());
                        }
                    }
                    drop((output, collector));
                    assert_eq!(account.current_samples(), 0);
                    account.snapshot().assert_memory_released();
                }
            }
        }

        #[test]
        fn sorted_output_transfer_abort_and_rebegin_preserve_identity() {
            for checked in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let mut collector = Collector::new(&account);
                collector.begin(header(ValueType::Matrix)).unwrap();
                for (series_id, label) in [(0, b"z"), (1, b"a")] {
                    put(
                        &mut collector,
                        checked,
                        (&account, &control),
                        series_id,
                        label,
                        (0, series_id as f64),
                    )
                    .unwrap();
                }
                finish(&mut collector, checked, &account, &control).unwrap();
                put(
                    &mut collector,
                    checked,
                    (&account, &control),
                    0,
                    b"ignored",
                    (1000, 10.0),
                )
                .unwrap();
                finish(&mut collector, checked, &account, &control).unwrap();
                let cancelled = Control::default();
                cancelled.cancellation.cancel();
                assert!(matches!(
                    collector.begin_checked(header(ValueType::Vector), &cancelled, &account),
                    Err(OutputError::Interrupted(Interrupted::Cancelled))
                ));
                let old = collector.take().unwrap();
                assert_eq!(
                    old.series
                        .iter()
                        .map(|row| row.series_id)
                        .collect::<Vec<_>>(),
                    [1, 0]
                );
                assert_eq!(&*old.series[1].labels.0[0].value, b"z");
                assert_eq!(
                    old.series[0]
                        .points
                        .iter()
                        .map(float_bits)
                        .collect::<Vec<_>>(),
                    [1.0f64.to_bits()]
                );
                assert_eq!(
                    old.series[1]
                        .points
                        .iter()
                        .map(float_bits)
                        .collect::<Vec<_>>(),
                    [0.0f64.to_bits(), 10.0f64.to_bits()]
                );
                collector.begin(header(ValueType::Vector)).unwrap();
                put(
                    &mut collector,
                    checked,
                    (&account, &control),
                    0,
                    b"aborted",
                    (0, 20.0),
                )
                .unwrap();
                collector.abort();
                assert!(collector.take().is_none());
                collector.begin(header(ValueType::Vector)).unwrap();
                for series_id in 0..2 {
                    put(
                        &mut collector,
                        checked,
                        (&account, &control),
                        series_id,
                        b"same",
                        (0, 100.0 + series_id as f64),
                    )
                    .unwrap();
                }
                finish(&mut collector, checked, &account, &control).unwrap();
                let new = collector.take().unwrap();
                drop(collector);
                assert_eq!(
                    new.series
                        .iter()
                        .map(|row| row.series_id)
                        .collect::<Vec<_>>(),
                    [0, 1]
                );
                assert_eq!(
                    new.series
                        .iter()
                        .map(|row| float_bits(&row.points[0]))
                        .collect::<Vec<_>>(),
                    [100.0f64.to_bits(), 101.0f64.to_bits()]
                );
                assert_eq!(old.series[1].points[1].time_ms, 1000);
                assert_eq!(account.current_samples(), 5);
                drop((old, new));
                assert_eq!(account.current_samples(), 0);
                account.snapshot().assert_memory_released();
            }
        }

        #[test]
        fn interrupted_matrix_sort_retains_correct_ids_after_partial_swaps() {
            let mut interrupted_reordered = 0;
            for remaining in [1, 2, 4, 8, 16, 32, 64, 128] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let mut collector = Collector::new(&account);
                collector.begin(header(ValueType::Matrix)).unwrap();
                for series_id in 0..8 {
                    put(
                        &mut collector,
                        true,
                        (&account, &control),
                        series_id,
                        &[b'h' - series_id as u8],
                        (0, series_id as f64),
                    )
                    .unwrap();
                }
                let outcome = crate::test_storage::interrupt_after_work(
                    &account,
                    &control,
                    remaining,
                    || collector.commit_checked(&account.snapshot(), &control, &account),
                );
                if matches!(
                    outcome,
                    Err(OutputError::Interrupted(Interrupted::Cancelled))
                ) {
                    if collector
                        .result
                        .as_ref()
                        .unwrap()
                        .series
                        .iter()
                        .enumerate()
                        .any(|(index, row)| row.series_id != index)
                    {
                        interrupted_reordered += 1;
                    }
                } else {
                    outcome.unwrap();
                }
                let retry = Control::default();
                for series_id in 0..8 {
                    put(
                        &mut collector,
                        true,
                        (&account, &retry),
                        series_id,
                        b"ignored",
                        (1000, 100.0 + series_id as f64),
                    )
                    .unwrap();
                }
                finish(&mut collector, true, &account, &retry).unwrap();
                let output = collector.take().unwrap();
                for (index, row) in output.series.iter().enumerate() {
                    assert_eq!(row.series_id, 7 - index);
                    assert_eq!(row.points.len(), 2);
                    assert_eq!(float_bits(&row.points[0]), (row.series_id as f64).to_bits());
                    assert_eq!(
                        float_bits(&row.points[1]),
                        (100.0 + row.series_id as f64).to_bits()
                    );
                }
                drop((output, collector));
                assert_eq!(account.current_samples(), 0);
                account.snapshot().assert_memory_released();
            }
            assert!(
                interrupted_reordered > 0,
                "must reach cancellation after an actual sort swap"
            );
        }

        #[test]
        fn direct_hit_and_absence_retain_cancelled_and_deadline_entry_checks() {
            for series_id in [0, 1] {
                for deadline in [false, true] {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let mut collector = Collector::new(&account);
                    collector.begin(header(ValueType::Vector)).unwrap();
                    put(
                        &mut collector,
                        true,
                        (&account, &control),
                        0,
                        b"original",
                        (0, 1.0),
                    )
                    .unwrap();
                    let before = account.snapshot();
                    let mut interrupted = Control::default();
                    let expected = if deadline {
                        interrupted.deadline = Some(std::time::Instant::now());
                        Interrupted::Deadline
                    } else {
                        interrupted.cancellation.cancel();
                        Interrupted::Cancelled
                    };
                    assert!(
                        matches!(collector.begin_checked(header(ValueType::Matrix), &interrupted, &account),
                        Err(OutputError::Interrupted(error)) if error == expected)
                    );
                    assert!(
                        matches!(put(&mut collector, true, (&account, &interrupted), series_id, b"ignored", (1000, 2.0)),
                        Err(OutputError::Interrupted(error)) if error == expected)
                    );
                    assert_eq!(account.snapshot().work, before.work);
                    assert_eq!(
                        account.snapshot().memory.map(|value| value.current),
                        before.memory.map(|value| value.current)
                    );
                    finish(&mut collector, true, &account, &control).unwrap();
                    let output = collector.take().unwrap();
                    assert_eq!(output.series.len(), 1);
                    assert_eq!(output.series[0].points.len(), 1);
                    assert_eq!(float_bits(&output.series[0].points[0]), 1.0f64.to_bits());
                    assert_eq!(&*output.series[0].labels.0[0].value, b"original");
                    drop((output, collector));
                    assert_eq!(account.current_samples(), 0);
                    account.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn native_value_units_follow_copy_compaction_and_mutation_unwind() {
        let account = crate::accounting::EngineAccounting::default().query();
        let control = Control::default();
        let input = FloatHistogram {
            positive_spans: Some(
                vec![crate::histogram::Span {
                    offset: 0,
                    length: 4,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![1.0, 0.0, 0.0, 0.0].into()),
            count: 1.0,
            sum: 1.0,
            ..FloatHistogram::default()
        };
        let OwnedValue::Histogram(mut value) = OwnedValue::from_sample_checked(
            SampleValueRef::FloatHistogram(input.as_ref()),
            &control,
            &account,
            MemoryKind::Workspace,
        )
        .unwrap() else {
            unreachable!()
        };
        assert_eq!(account.current_samples(), 13);
        let copy = value.copy_checked(&control, &account).unwrap();
        assert_eq!(account.current_samples(), 26);
        value.compact_checked(&control, &account).unwrap();
        assert_eq!(account.current_samples(), 25);
        drop(copy);
        assert_eq!(account.current_samples(), 12);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut mutation = value.mutation();
            mutation.value.positive_buckets = None;
            panic!("injected histogram mutation unwind");
        }));
        assert!(outcome.is_err());
        assert_eq!(account.current_samples(), 11);
        assert_eq!(account.snapshot().statistics.samples.peak_samples, 26);
        drop(value);
        assert_eq!(account.current_samples(), 0);
        account.snapshot().assert_memory_released();
    }
    use crate::{
        accounting::EngineAccounting,
        histogram::{CUSTOM_BUCKETS_SCHEMA, Span},
    };

    fn label_fields(count: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
        (0..count)
            .map(|index| {
                (
                    format!("k{index:02}").into_bytes(),
                    vec![index as u8, 0, 255, index as u8],
                )
            })
            .collect()
    }

    #[test]
    fn known_label_copies_bound_retained_bytes_and_outlive_their_source() {
        let mut excess = Vec::new();
        for kind in [MemoryKind::Workspace, MemoryKind::Result] {
            for checked in [false, true] {
                for count in [0, 1, 2, 3, 4, 8, 9, 16, 17, 32] {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let mut source = label_fields(count);
                    let expected = source.clone();
                    let labels: Vec<_> = source
                        .iter()
                        .map(|(name, value)| LabelRef { name, value })
                        .collect();
                    let copied = if checked {
                        OwnedLabels::copy_from_checked(&labels, &account, kind, &control).unwrap()
                    } else {
                        OwnedLabels::copy_from(&labels, &account, kind)
                    };
                    drop(labels);
                    for (name, value) in &mut source {
                        name.fill(b'x');
                        value.fill(b'y');
                    }
                    source.clear();
                    assert_eq!(copied.0.len(), count);
                    for (actual, (name, value)) in copied.0.iter().zip(&expected) {
                        assert_eq!(&*actual.name, name);
                        assert_eq!(&*actual.value, value);
                    }
                    let required = (count * size_of::<OwnedLabel>()
                        + expected
                            .iter()
                            .map(|(name, value)| name.len() + value.len())
                            .sum::<usize>()) as u64;
                    let observed = account.snapshot();
                    assert_eq!(
                        observed.work,
                        if checked { count as u64 * 8 + 1 } else { 0 }
                    );
                    let memory = observed.memory[kind as usize];
                    println!(
                        "label-copy kind={kind:?} checked={checked} count={count} retained={} required={required} peak={} acquired={}",
                        memory.current, memory.peak, memory.acquired
                    );
                    if memory.current != required {
                        excess.push((kind, checked, count, memory.current, required));
                    }
                    assert!(memory.peak >= memory.current && memory.acquired >= memory.peak);
                    drop(copied);
                    account.snapshot().assert_memory_released();
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
        assert!(
            excess.is_empty(),
            "completed known-count copies retain excess descriptor storage: {excess:?}"
        );
    }

    #[test]
    fn bounded_filtered_label_copies_keep_single_predicate_visits_and_failure_priority() {
        for count in [0, 1, 3, 9, 17, 32] {
            for mode in 0..3 {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let fields = label_fields(count);
                let mut source = OwnedLabels::new(&account, MemoryKind::Workspace);
                for (name, value) in &fields {
                    source.push(name, value, &account, MemoryKind::Workspace);
                }
                let mut visits = 0;
                let copied = source
                    .copy_filtered_checked(&account, MemoryKind::Result, &control, |label| {
                        let index = visits;
                        visits += 1;
                        assert_eq!(&*label.name, &fields[index].0);
                        Ok(mode == 2 || mode == 1 && index % 2 == 0)
                    })
                    .unwrap();
                let expected: Vec<_> = fields
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mode == 2 || mode == 1 && index % 2 == 0)
                    .map(|(_, field)| field)
                    .collect();
                assert_eq!(visits, count);
                assert_eq!(copied.0.len(), expected.len());
                assert!(copied.0.capacity() <= count);
                for (actual, (name, value)) in copied.0.iter().zip(&expected) {
                    assert_eq!(&*actual.name, name);
                    assert_eq!(&*actual.value, value);
                }
                let observation = account.snapshot();
                assert_eq!(observation.work, (count + expected.len() * 7 + 1) as u64);
                assert_eq!(
                    observation.memory[MemoryKind::Result as usize].current,
                    (copied.0.capacity() * size_of::<OwnedLabel>() + expected.len() * 7) as u64
                );
                drop(copied);
                assert_eq!(
                    account.snapshot().memory[MemoryKind::Result as usize].current,
                    0
                );
                drop(source);
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let fields = label_fields(17);
        let labels: Vec<_> = fields
            .iter()
            .map(|(name, value)| LabelRef { name, value })
            .collect();
        let source = OwnedLabels::copy_from(&labels, &account, MemoryKind::Workspace);
        let mut visits = 0;
        let failure = source.copy_filtered_checked(&account, MemoryKind::Result, &control, |_| {
            visits += 1;
            if visits == 9 {
                control.cancellation.cancel();
                Err(Interrupted::Deadline)
            } else {
                Ok(true)
            }
        });
        assert!(matches!(failure, Err(Interrupted::Deadline)));
        assert_eq!(visits, 9);
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            0
        );
        drop(source);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn bounded_label_copy_interruption_releases_bytes_before_and_after_growth() {
        for threshold in [1, 64, 65, 68, 1092, 2116, 2118] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut fields = label_fields(9);
            fields[8].1 = vec![255; 2050];
            let labels: Vec<_> = fields
                .iter()
                .map(|(name, value)| LabelRef { name, value })
                .collect();
            let outcome =
                crate::test_storage::interrupt_after_work(&account, &control, threshold, || {
                    OwnedLabels::copy_from_checked(&labels, &account, MemoryKind::Result, &control)
                });
            assert!(
                matches!(outcome, Err(Interrupted::Cancelled)),
                "{threshold}"
            );
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        control.cancellation.cancel();
        assert!(matches!(
            OwnedLabels::copy_from_checked(&[], &account, MemoryKind::Result, &control),
            Err(Interrupted::Cancelled)
        ));
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].acquired,
            0
        );
    }

    #[test]
    fn collector_keeps_one_label_owner_across_chunks_and_source_replacement() {
        for count in [9, 17] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut collector = Collector::new(&account);
            collector
                .begin(OutputHeader {
                    value_type: ValueType::Matrix,
                    start_ms: 0,
                    end_ms: 2000,
                    step_ms: 1000,
                })
                .unwrap();
            let mut source = label_fields(count);
            let expected = source.clone();
            let mut acquired = 0;
            for tick in 0..3 {
                let fields = if tick == 0 { &source } else { &expected };
                let labels: Vec<_> = fields
                    .iter()
                    .map(|(name, value)| LabelRef { name, value })
                    .collect();
                collector
                    .chunk_checked(
                        OutputChunk::Series {
                            series_id: 99,
                            labels: &labels,
                            points: &[SampleRef {
                                time_ms: tick * 1000,
                                start_time_ms: 0,
                                value: SampleValueRef::Float(tick as f64 + 1.0),
                            }],
                        },
                        &control,
                        &account,
                    )
                    .unwrap();
                drop(labels);
                if tick == 0 {
                    acquired = account.snapshot().memory[MemoryKind::Result as usize].acquired;
                    for (name, value) in &mut source {
                        name.fill(b'x');
                        value.fill(b'y');
                    }
                    source.clear();
                } else {
                    assert_eq!(
                        account.snapshot().memory[MemoryKind::Result as usize].acquired,
                        acquired
                    );
                }
            }
            collector
                .commit_checked(&account.snapshot(), &control, &account)
                .unwrap();
            let result = collector.take().unwrap();
            assert_eq!(result.series.len(), 1);
            let row = &result.series[0];
            assert_eq!(row.series_id, 99);
            assert_eq!(row.labels.0.len(), count);
            for (actual, (name, value)) in row.labels.0.iter().zip(&expected) {
                assert_eq!(&*actual.name, name);
                assert_eq!(&*actual.value, value);
            }
            assert_eq!(row.points.len(), 3);
            for (tick, point) in row.points.iter().enumerate() {
                assert_eq!(point.time_ms, tick as i64 * 1000);
                let OwnedValue::Float(value) = point.value else {
                    panic!("float result")
                };
                assert_eq!(value.to_bits(), (tick as f64 + 1.0).to_bits());
            }
            drop((result, collector));
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn compact_label_mutations_restore_ordinary_geometric_capacity_and_overlap() {
        for count in [0, 1, 2, 3, 4, 5, 8, 9, 16, 17, 32] {
            for set in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let fields = label_fields(count);
                let labels: Vec<_> = fields
                    .iter()
                    .map(|(name, value)| LabelRef { name, value })
                    .collect();
                let mut copied = OwnedLabels::copy_from(&labels, &account, MemoryKind::Workspace);
                for index in 0..count + 6 {
                    let name = format!("z{index:02}");
                    if set {
                        copied
                            .set(
                                name.as_bytes(),
                                b"x",
                                &account,
                                MemoryKind::Workspace,
                                &control,
                            )
                            .unwrap();
                    } else {
                        copied.push(name.as_bytes(), b"x", &account, MemoryKind::Workspace);
                    }
                    assert_eq!(
                        copied.0.capacity(),
                        (count + index + 1).next_power_of_two().max(4)
                    );
                    assert_eq!(&*copied.0[count + index].name, name.as_bytes());
                    assert_eq!(&*copied.0[count + index].value, b"x");
                }
                for (actual, (name, value)) in copied.0.iter().zip(&fields) {
                    assert_eq!(&*actual.name, name);
                    assert_eq!(&*actual.value, value);
                }
                drop(copied);
                let mut ordinary = OwnedLabels::new(&account, MemoryKind::Workspace);
                let mut untracked = OwnedLabels(AccountedVec::default());
                let mut plain = Vec::new();
                for index in 0..33 {
                    ordinary.push(b"k", b"v", &account, MemoryKind::Workspace);
                    untracked.push(b"k", b"v", &account, MemoryKind::Workspace);
                    plain.push(OwnedLabel {
                        name: AccountedVec::default(),
                        value: AccountedVec::default(),
                    });
                    assert_eq!(
                        ordinary.0.capacity(),
                        (index + 1_usize).next_power_of_two().max(4)
                    );
                    assert_eq!(untracked.0.capacity(), plain.capacity());
                }
                drop((ordinary, untracked, plain));
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
        let engine = EngineAccounting::default();
        let account = engine.query();
        let fields = label_fields(17);
        let labels: Vec<_> = fields
            .iter()
            .map(|(name, value)| LabelRef { name, value })
            .collect();
        let mut copied = OwnedLabels::copy_from(&labels, &account, MemoryKind::Workspace);
        copied.push(b"z", b"x", &account, MemoryKind::Workspace);
        let memory = account.snapshot().memory[MemoryKind::Workspace as usize];
        assert_eq!(
            memory.current,
            (32 * size_of::<OwnedLabel>() + 17 * 7 + 8) as u64
        );
        assert_eq!(
            memory.peak,
            (49 * size_of::<OwnedLabel>() + 17 * 7 + 8) as u64
        );
        assert_eq!(
            memory.acquired,
            (77 * size_of::<OwnedLabel>() + 17 * 7 + 8) as u64
        );
        drop(copied);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn compact_label_insertion_keeps_the_existing_post_value_cancellation_phase() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let fields = label_fields(9);
        let labels: Vec<_> = fields
            .iter()
            .map(|(name, value)| LabelRef { name, value })
            .collect();
        let mut copied = OwnedLabels::copy_from(&labels, &account, MemoryKind::Workspace);
        let work = account.snapshot().work;
        assert_eq!(copied.position(b"a", &account, &control).unwrap(), Err(0));
        let lookup_work = account.snapshot().work - work;
        let value = vec![255; 2050];
        let outcome = crate::test_storage::interrupt_after_work(
            &account,
            &control,
            lookup_work + 1 + value.len() as u64,
            || copied.set(b"a", &value, &account, MemoryKind::Workspace, &control),
        );
        assert_eq!(outcome, Err(Interrupted::Cancelled));
        assert_eq!(copied.0.len(), 10);
        assert_eq!(&*copied.0[9].name, b"a");
        assert_eq!(&*copied.0[9].value, &value);
        for (actual, (name, value)) in copied.0.iter().zip(&fields) {
            assert_eq!(&*actual.name, name);
            assert_eq!(&*actual.value, value);
        }
        assert_eq!(copied.0.capacity(), 16);
        let retained = copied.0.capacity() * size_of::<OwnedLabel>()
            + copied
                .0
                .iter()
                .map(|label| label.name.capacity() + label.value.capacity())
                .sum::<usize>();
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].current,
            retained as u64
        );
        drop(copied);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn checked_label_lookup_observes_long_names_and_empty_interruptions() {
        for length in [8192, 16_384] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let name = vec![b'a'; length];
            let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
            labels.push(&name, b"value", &account, MemoryKind::Workspace);
            let result = crate::test_storage::interrupt_during(&account, &control, || {
                labels.position(&name, &account, &control)
            });
            assert_eq!(result, Err(Interrupted::Cancelled));
            drop(labels);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
        let account = EngineAccounting::default().query();
        let labels = OwnedLabels::new(&account, MemoryKind::Workspace);
        for deadline in [false, true] {
            let mut control = Control::default();
            if deadline {
                control.deadline = Some(std::time::Instant::now());
            } else {
                control.cancellation.cancel();
            }
            assert_eq!(
                labels.position(b"a", &account, &control),
                Err(if deadline {
                    Interrupted::Deadline
                } else {
                    Interrupted::Cancelled
                })
            );
        }
        drop(labels);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn checked_label_set_preserves_order_and_replacement_and_removal() {
        let account = EngineAccounting::default().query();
        let control = Control::default();
        let kind = MemoryKind::Workspace;
        let mut labels = OwnedLabels::new(&account, kind);
        for name in [b"d", b"f"] {
            labels.push(name, b"old", &account, kind);
        }
        for (name, value) in [
            (b"b".as_slice(), b"new".as_slice()),
            (b"e", b"new"),
            (b"g", b"new"),
            (b"e", b"updated"),
            (b"f", b""),
            (b"z", b""),
        ] {
            labels.set(name, value, &account, kind, &control).unwrap();
        }
        assert_eq!(
            labels
                .0
                .iter()
                .map(|label| (&*label.name, &*label.value))
                .collect::<Vec<_>>(),
            [
                (b"b".as_slice(), b"new".as_slice()),
                (b"d", b"old"),
                (b"e", b"updated"),
                (b"g", b"new"),
            ]
        );
        drop(labels);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn checked_label_set_interrupts_insertion_and_removal_moves() {
        for insert in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let kind = MemoryKind::Workspace;
            let mut labels = OwnedLabels::new(&account, kind);
            for index in 0..10_000 {
                labels.push(
                    format!("label_{index:05}").as_bytes(),
                    b"value",
                    &account,
                    kind,
                );
            }
            let result = crate::test_storage::interrupt_during(&account, &control, || {
                if insert {
                    labels.set(b"a", b"value", &account, kind, &control)
                } else {
                    labels.set(b"label_00000", b"", &account, kind, &control)
                }
            });
            assert_eq!(result, Err(Interrupted::Cancelled));
            assert_eq!(labels.0.len(), if insert { 10_001 } else { 10_000 });
            assert!(account.snapshot().work >= 8192);
            drop(labels);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn checked_retention_interrupts_selection_swaps_and_normal_release() {
        for (count, keep, phase) in [
            (10_000, false, "selection"),
            (10_000, true, "swaps"),
            (6_000, false, "release"),
        ] {
            let engine = crate::accounting::EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
            for index in 0..count {
                labels.push(
                    format!("label_{index:05}").as_bytes(),
                    b"value",
                    &account,
                    MemoryKind::Workspace,
                );
            }
            let mut visited = 0;
            let result = crate::test_storage::interrupt_during(&account, &control, || {
                labels.retain_labels_checked(&account, &control, |_| {
                    visited += 1;
                    Ok::<_, Interrupted>(keep)
                })
            });
            assert_eq!(result, Err(Interrupted::Cancelled), "{phase}");
            if phase == "release" {
                assert_eq!(visited, count);
                assert!(!labels.0.is_empty() && labels.0.len() < count);
            } else {
                assert!(visited > 0 && visited < count);
            }
            drop(labels);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn checked_retention_preserves_order_and_checks_empty_entry_and_deadlines() {
        let account = crate::accounting::EngineAccounting::default().query();
        let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
        for name in [b"a", b"b", b"c", b"d", b"e"] {
            labels.push(name, b"value", &account, MemoryKind::Workspace);
        }
        labels
            .retain_labels_checked(&account, &Control::default(), |label| {
                Ok::<_, Interrupted>(label.name[0] == b'b' || label.name[0] == b'e')
            })
            .unwrap();
        assert_eq!(
            labels
                .0
                .iter()
                .map(|label| &*label.name)
                .collect::<Vec<_>>(),
            [b"b", b"e"]
        );
        drop(labels);
        let mut empty = OwnedLabels::new(&account, MemoryKind::Workspace);
        for deadline in [false, true] {
            let mut control = Control::default();
            if deadline {
                control.deadline = Some(std::time::Instant::now());
            } else {
                control.cancellation.cancel();
            }
            let result = empty.retain_labels_checked(&account, &control, |_| {
                panic!("empty labels do not invoke the predicate")
            });
            assert_eq!(
                result,
                Err(if deadline {
                    Interrupted::Deadline
                } else {
                    Interrupted::Cancelled
                })
            );
        }
        drop(empty);
        account.snapshot().assert_memory_released();
    }
    #[test]
    fn collected_equal_label_rows_keep_source_sort_provenance_in_both_entry_points() {
        for checked in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let baseline = account.snapshot().memory.map(|memory| memory.current);
            let control = Control::default();
            let mut collector = Collector::new(&account);
            collector
                .begin(OutputHeader {
                    value_type: ValueType::Matrix,
                    start_ms: 0,
                    end_ms: 0,
                    step_ms: 1000,
                })
                .unwrap();
            for series_id in [1, 2] {
                collector
                    .chunk(OutputChunk::Series {
                        series_id,
                        labels: &[],
                        points: &[SampleRef {
                            time_ms: 0,
                            start_time_ms: 0,
                            value: SampleValueRef::Float(series_id as f64),
                        }],
                    })
                    .unwrap();
            }
            if checked {
                collector
                    .commit_checked(&account.snapshot(), &control, &account)
                    .unwrap();
            } else {
                collector.commit(&account.snapshot()).unwrap();
            }
            let result = collector.take().unwrap();
            assert_eq!(
                result
                    .series
                    .iter()
                    .map(|row| row.series_id)
                    .collect::<Vec<_>>(),
                [1, 2]
            );
            drop(result);
            drop(collector);
            assert_eq!(
                account.snapshot().memory.map(|memory| memory.current),
                baseline
            );
        }
    }

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

    #[test]
    fn checked_native_copy_and_overflow_scan_interrupt_without_retaining_scratch() {
        for copy in [true, false] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let source = histogram(0, &vec![1.0; 16_384], None);
            let value = own(&source, &account);
            let baseline = account.snapshot().memory[MemoryKind::Workspace as usize].current;
            let result = crate::test_storage::interrupt_during(&account, &control, || {
                if copy {
                    value.copy_checked(&control, &account).map(drop)
                } else {
                    value.has_overflow_checked(&control, &account).map(|_| ())
                }
            });
            assert_eq!(result, Err(Interrupted::Cancelled));
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].current,
                baseline
            );
            drop(value);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn checked_kahan_restores_the_compensation_box_after_interrupted_reduction_or_merge() {
        for (schema, length, other_schema, other_length) in
            [(0, 16_384, 0, 16_384), (2, 16_384, 0, 1), (0, 1, 2, 16_384)]
        {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let source = histogram(schema, &vec![1.0; length], None);
            let operand = histogram(other_schema, &vec![1.0; other_length], None);
            let mut sum = own(&source, &account);
            let mut correction = None;
            sum.kahan_add(source.as_ref(), &mut correction).unwrap();
            let pointer = &*correction.as_ref().unwrap().payload as *const HistogramPayload;
            let result = crate::test_storage::interrupt_during(&account, &control, || {
                sum.kahan_add_checked(operand.as_ref(), &mut correction, &control, &account)
            });
            assert!(matches!(
                result,
                Err(
                    crate::histogram_work::HistogramEvaluationError::Interrupted(
                        Interrupted::Cancelled
                    )
                )
            ));
            assert_eq!(
                &*correction.as_ref().unwrap().payload as *const HistogramPayload,
                pointer
            );
            drop(sum);
            drop(correction);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    fn bounds_wrapper_bytes() -> u64 {
        size_of::<crate::shared::Allocation<AccountedVec<f64>>>() as u64
    }

    #[test]
    fn checked_collector_interrupts_copy_lookup_and_final_comparison_work() {
        use crate::test_storage::interrupt_during;
        for operation in [
            "string",
            "name",
            "value",
            "points",
            "buckets",
            "bounds",
            "lookup",
            "comparison",
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut collector = Collector::new(&account);
            collector
                .begin(OutputHeader {
                    value_type: if operation == "comparison" {
                        ValueType::Matrix
                    } else {
                        ValueType::Vector
                    },
                    start_ms: 0,
                    end_ms: 0,
                    step_ms: 0,
                })
                .unwrap();
            let bytes = vec![b'a'; 32_768];
            let buckets = vec![1.0; 16_384];
            let points = vec![
                SampleRef {
                    time_ms: 0,
                    start_time_ms: 0,
                    value: SampleValueRef::Float(1.0)
                };
                9000
            ];
            let histogram = FloatHistogram {
                schema: if operation == "bounds" {
                    CUSTOM_BUCKETS_SCHEMA
                } else {
                    0
                },
                positive_buckets: (operation == "buckets").then(|| buckets.clone().into()),
                custom_values: (operation == "bounds").then(|| buckets.clone().into()),
                ..FloatHistogram::default()
            };
            if operation == "lookup" {
                // A nonpositional chunk keeps this fixture on its long lookup path.
                collector
                    .chunk(OutputChunk::Series {
                        series_id: usize::MAX,
                        labels: &[],
                        points: &[],
                    })
                    .unwrap();
                let result = collector.result.as_mut().unwrap();
                for series_id in 0..9000 {
                    result.series.push(ResultSeries {
                        series_id,
                        labels: OwnedLabels::new(&account, MemoryKind::Result),
                        points: AccountedVec::new(&account, MemoryKind::Result),
                    });
                }
            } else if operation == "comparison" {
                for (series_id, value) in [b"a".as_slice(), b"b"].into_iter().enumerate() {
                    collector
                        .chunk(OutputChunk::Series {
                            series_id,
                            labels: &[LabelRef {
                                name: &bytes,
                                value,
                            }],
                            points: &[],
                        })
                        .unwrap();
                }
            }
            let labels = [LabelRef {
                name: if operation == "name" {
                    &bytes
                } else {
                    b"label"
                },
                value: if operation == "value" {
                    &bytes
                } else {
                    b"value"
                },
            }];
            let histogram_point = [SampleRef {
                time_ms: 0,
                start_time_ms: 0,
                value: SampleValueRef::FloatHistogram(histogram.as_ref()),
            }];
            let outcome = interrupt_during(&account, &control, || {
                if operation == "comparison" {
                    collector.commit_checked(&account.snapshot(), &control, &account)
                } else {
                    let chunk = if operation == "string" {
                        OutputChunk::String {
                            time_ms: 0,
                            value: &bytes,
                        }
                    } else {
                        OutputChunk::Series {
                            series_id: if operation == "lookup" { 9000 } else { 0 },
                            labels: &labels,
                            points: if operation == "points" {
                                &points
                            } else if operation == "buckets" || operation == "bounds" {
                                &histogram_point
                            } else {
                                &[]
                            },
                        }
                    };
                    collector.chunk_checked(chunk, &control, &account)
                }
            });
            assert!(
                matches!(
                    outcome,
                    Err(OutputError::Interrupted(Interrupted::Cancelled))
                ),
                "{operation}: {outcome:?}"
            );
            assert!(collector.take().is_none());
            assert!(account.snapshot().work >= 8192, "{operation}");
            collector.abort();
            drop(collector);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn checked_native_result_copy_preserves_bits_and_present_empty_fields() {
        for schema in [0, CUSTOM_BUCKETS_SCHEMA] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let source = FloatHistogram {
                reset_hint: crate::histogram::GAUGE,
                schema,
                zero_threshold: -0.0,
                zero_count: f64::from_bits(0x7ff8_0000_0000_0123),
                count: f64::NEG_INFINITY,
                sum: f64::from_bits(0xfff8_0000_0000_0456),
                positive_spans: Some(Vec::new().into()),
                negative_spans: None,
                positive_buckets: None,
                negative_buckets: Some(Vec::new().into()),
                custom_values: Some(Vec::new().into()),
            };
            let mut collector = Collector::new(&account);
            collector
                .begin_checked(
                    OutputHeader {
                        value_type: ValueType::Vector,
                        start_ms: 0,
                        end_ms: 0,
                        step_ms: 0,
                    },
                    &control,
                    &account,
                )
                .unwrap();
            collector
                .chunk_checked(
                    OutputChunk::Series {
                        series_id: 17,
                        labels: &[],
                        points: &[SampleRef {
                            time_ms: 0,
                            start_time_ms: 0,
                            value: SampleValueRef::FloatHistogram(source.as_ref()),
                        }],
                    },
                    &control,
                    &account,
                )
                .unwrap();
            collector
                .commit_checked(&account.snapshot(), &control, &account)
                .unwrap();
            let result = collector.take().unwrap();
            drop((source, collector));
            let SampleValueRef::FloatHistogram(value) = result.series[0].points[0].value.as_ref()
            else {
                panic!("native result");
            };
            assert_eq!(result.series[0].series_id, 17);
            assert_eq!(
                (value.reset_hint, value.schema),
                (crate::histogram::GAUGE, schema)
            );
            assert_eq!(value.zero_threshold.to_bits(), (-0.0f64).to_bits());
            assert_eq!(value.zero_count.to_bits(), 0x7ff8_0000_0000_0123);
            assert_eq!(value.count.to_bits(), f64::NEG_INFINITY.to_bits());
            assert_eq!(value.sum.to_bits(), 0xfff8_0000_0000_0456);
            assert_eq!(value.positive_spans, Some(&[][..]));
            assert_eq!(value.negative_spans, None);
            assert_eq!(value.positive_buckets, None);
            assert_eq!(value.negative_buckets, Some(&[][..]));
            assert_eq!(value.custom_values, Some(&[][..]));
            assert!(!control.cancellation.cancelled());
            drop(result);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
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
        engine.snapshot().assert_memory_released();
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
        sum.arithmetic_checked(operand.as_ref(), false, &Control::default(), &account)
            .unwrap();
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
        let flags = sum
            .arithmetic_checked(operand.as_ref(), false, &Control::default(), &account)
            .unwrap();
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
            sum.arithmetic_checked(malformed.as_ref(), false, &Control::default(), &account)
                .unwrap();
        }));
        assert!(panic.is_err());
        let observation = account.snapshot().memory[1];
        assert_eq!(observation.current, 0);
        assert!(observation.peak >= 128);
        assert_eq!(observation.acquired, observation.released);
    }
}
