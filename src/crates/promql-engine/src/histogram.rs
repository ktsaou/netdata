// SPDX-License-Identifier: GPL-3.0-or-later
// Histogram value operations derive from the pinned Prometheus model; see NOTICE.
pub use crate::histogram_arithmetic::HistogramArithmetic;
pub use crate::histogram_bounds::bucket_bound;
pub use crate::histogram_classic::{
    BucketQuantileResult, ClassicBucket, bucket_fraction, bucket_quantile,
};
pub use crate::histogram_counter::{ZeroCount, custom_bounds_match};
pub use crate::histogram_functions::{
    HistogramFunctionResult, NativeHistogramInfo, histogram_avg, histogram_count,
    histogram_fraction, histogram_quantile, histogram_stddev, histogram_stdvar, histogram_sum,
};
pub use crate::histogram_iterator::{
    AllFloatBucketIterator, Bucket, FloatBucketIterator, ReverseFloatBucketIterator,
};
pub use crate::histogram_layout::HistogramError;
use crate::histogram_layout::{
    compact_float, compact_float_pair_with, reduce_float_copy_with, reduce_float_in_place,
};
use crate::histogram_work::{HistogramEvaluationError, HistogramWork};
use crate::storage::Interrupted;
use crate::{
    accounting::{AccountedVec, AllocationContext},
    shared::Strong,
};
use std::sync::Arc;

pub const CUSTOM_BUCKETS_SCHEMA: i32 = -53;
pub const UNKNOWN_RESET: u8 = 0;
pub const COUNTER_RESET: u8 = 1;
pub const NOT_RESET: u8 = 2;
pub const GAUGE: u8 = 3;

/// Immutable bounds retain one observation through all owning aliases.
#[derive(Clone, Debug)]
pub struct HistogramBounds(BoundsOwner);
#[derive(Clone, Debug)]
enum BoundsOwner {
    Owned(Strong<AccountedVec<f64>>),
    External(Arc<[f64]>),
}
impl HistogramBounds {
    pub(crate) fn from_slice_in(values: &[f64], context: Option<&AllocationContext>) -> Self {
        Self::from_owned(AccountedVec::from_slice_in(values, context))
    }
    pub(crate) fn from_owned(values: AccountedVec<f64>) -> Self {
        let context = values.context();
        Self(BoundsOwner::Owned(Strong::new_in(values, context.as_ref())))
    }
    pub(crate) fn clone_in(&self, context: Option<&AllocationContext>) -> Self {
        if let Some(context) = context
            && !matches!(&self.0, BoundsOwner::Owned(values) if values.owns(context))
        {
            return Self::from_slice_in(self, Some(context));
        }
        self.clone()
    }
    pub fn same_allocation(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (BoundsOwner::Owned(left), BoundsOwner::Owned(right)) => left.same_allocation(right),
            (BoundsOwner::External(left), BoundsOwner::External(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}
impl std::ops::Deref for HistogramBounds {
    type Target = [f64];
    fn deref(&self) -> &Self::Target {
        match &self.0 {
            BoundsOwner::Owned(values) => values,
            BoundsOwner::External(values) => values,
        }
    }
}
impl AsRef<[f64]> for HistogramBounds {
    fn as_ref(&self) -> &[f64] {
        self
    }
}
impl From<Arc<[f64]>> for HistogramBounds {
    fn from(values: Arc<[f64]>) -> Self {
        Self(BoundsOwner::External(values))
    }
}
impl From<Vec<f64>> for HistogramBounds {
    fn from(values: Vec<f64>) -> Self {
        Self::from_owned(AccountedVec::from_vec_in(values, None))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct Span {
    pub offset: i32,
    pub length: u32,
}

/// One owned sample payload; it does not retain any series history.
#[derive(Debug, Default)]
pub struct FloatHistogram {
    pub reset_hint: u8,
    pub schema: i32,
    pub zero_threshold: f64,
    pub zero_count: f64,
    pub count: f64,
    pub sum: f64,
    pub positive_spans: Option<AccountedVec<Span>>,
    pub negative_spans: Option<AccountedVec<Span>>,
    pub positive_buckets: Option<AccountedVec<f64>>,
    pub negative_buckets: Option<AccountedVec<f64>>,
    pub custom_values: Option<HistogramBounds>,
}

/// A cursor's histogram view cannot outlive its borrowed page.
#[derive(Clone, Copy, Debug)]
pub struct FloatHistogramRef<'a> {
    pub reset_hint: u8,
    pub schema: i32,
    pub zero_threshold: f64,
    pub zero_count: f64,
    pub count: f64,
    pub sum: f64,
    pub positive_spans: Option<&'a [Span]>,
    pub negative_spans: Option<&'a [Span]>,
    pub positive_buckets: Option<&'a [f64]>,
    pub negative_buckets: Option<&'a [f64]>,
    pub custom_values: Option<&'a [f64]>,
}

#[derive(Debug, Default)]
pub struct IntegerHistogram {
    pub reset_hint: u8,
    pub schema: i32,
    pub zero_threshold: f64,
    pub zero_count: u64,
    pub count: u64,
    pub sum: f64,
    pub positive_spans: Option<Vec<Span>>,
    pub negative_spans: Option<Vec<Span>>,
    pub positive_buckets: Option<Vec<i64>>,
    pub negative_buckets: Option<Vec<i64>>,
    pub custom_values: Option<Arc<[f64]>>,
}

#[derive(Clone, Copy, Debug)]
pub struct IntegerHistogramRef<'a> {
    pub reset_hint: u8,
    pub schema: i32,
    pub zero_threshold: f64,
    pub zero_count: u64,
    pub count: u64,
    pub sum: f64,
    pub positive_spans: Option<&'a [Span]>,
    pub negative_spans: Option<&'a [Span]>,
    pub positive_buckets: Option<&'a [i64]>,
    pub negative_buckets: Option<&'a [i64]>,
    pub custom_values: Option<&'a [f64]>,
}

trait CopyBuffer<T: Copy> {
    fn empty(context: Option<&AllocationContext>) -> Self;
    fn clear(&mut self);
    fn reserve(&mut self, additional: usize);
    fn extend_from_slice(&mut self, source: &[T]);
}
impl<T: Copy> CopyBuffer<T> for Vec<T> {
    fn empty(context: Option<&AllocationContext>) -> Self {
        assert!(
            context.is_none(),
            "raw fixture buffer cannot acquire query memory"
        );
        Self::new()
    }
    fn clear(&mut self) {
        Vec::clear(self);
    }
    fn reserve(&mut self, additional: usize) {
        Vec::reserve(self, additional);
    }
    fn extend_from_slice(&mut self, source: &[T]) {
        Vec::extend_from_slice(self, source);
    }
}
impl<T: Copy> CopyBuffer<T> for AccountedVec<T> {
    fn empty(context: Option<&AllocationContext>) -> Self {
        Self::new_in(context)
    }
    fn clear(&mut self) {
        AccountedVec::clear(self);
    }
    fn reserve(&mut self, additional: usize) {
        AccountedVec::reserve(self, additional);
    }
    fn extend_from_slice(&mut self, source: &[T]) {
        AccountedVec::extend_from_slice(self, source);
    }
}
fn overwrite<T: Copy, B: CopyBuffer<T>>(
    destination: &mut Option<B>,
    source: Option<&[T]>,
    context: Option<&AllocationContext>,
) {
    overwrite_with(destination, source, context, HistogramWork::default())
        .expect("inert histogram work");
}
fn overwrite_with<T: Copy, B: CopyBuffer<T>>(
    destination: &mut Option<B>,
    source: Option<&[T]>,
    context: Option<&AllocationContext>,
    work: HistogramWork<'_>,
) -> Result<(), Interrupted> {
    work.step(1)?;
    let source = source.unwrap_or(&[]);
    if destination.is_none() && source.is_empty() {
        return Ok(());
    }
    let destination = destination.get_or_insert_with(|| B::empty(context));
    destination.clear();
    destination.reserve(source.len());
    for chunk in source.chunks(1024) {
        work.step(chunk.len())?;
        destination.extend_from_slice(chunk);
    }
    Ok(())
}

fn clear_if_present<T: Copy, B: CopyBuffer<T>>(destination: &mut Option<B>) {
    if let Some(destination) = destination {
        destination.clear();
    }
}

impl FloatHistogram {
    pub fn detect_reset(&self, previous: FloatHistogramRef<'_>) -> bool {
        self.as_ref().detect_reset(previous)
    }

    pub fn zero_count_for_threshold(
        &self,
        threshold: f64,
        correction: Option<FloatHistogramRef<'_>>,
    ) -> ZeroCount {
        self.as_ref()
            .zero_count_for_threshold(threshold, correction)
    }

    pub fn bucket_iterator(
        &self,
        positive: bool,
        start: f64,
        target: i32,
    ) -> FloatBucketIterator<'_> {
        self.as_ref().bucket_iterator(positive, start, target)
    }

    pub fn reverse_bucket_iterator(&self, positive: bool) -> ReverseFloatBucketIterator<'_> {
        self.as_ref().reverse_bucket_iterator(positive)
    }

    pub fn all_bucket_iterator(&self, reverse: bool) -> AllFloatBucketIterator<'_> {
        self.as_ref().all_bucket_iterator(reverse)
    }

    pub fn zero_bucket(&self) -> Bucket {
        self.as_ref().zero_bucket()
    }

    pub fn as_ref(&self) -> FloatHistogramRef<'_> {
        FloatHistogramRef {
            reset_hint: self.reset_hint,
            schema: self.schema,
            zero_threshold: self.zero_threshold,
            zero_count: self.zero_count,
            count: self.count,
            sum: self.sum,
            positive_spans: self.positive_spans.as_deref(),
            negative_spans: self.negative_spans.as_deref(),
            positive_buckets: self.positive_buckets.as_deref(),
            negative_buckets: self.negative_buckets.as_deref(),
            custom_values: self.custom_values.as_deref(),
        }
    }

    pub fn uses_custom_buckets(&self) -> bool {
        self.schema == CUSTOM_BUCKETS_SCHEMA
    }

    pub(crate) fn normalize_copy_fields(&mut self) {
        // A uniquely owned map input needs Copy's field semantics without another bucket allocation.
        if self.uses_custom_buckets() {
            self.zero_threshold = 0.0;
            self.zero_count = 0.0;
            self.negative_spans = None;
            self.negative_buckets = None;
        } else {
            self.custom_values = None;
        }
        if self.positive_spans.as_ref().is_some_and(|v| v.is_empty()) {
            self.positive_spans = None;
        }
        if self.negative_spans.as_ref().is_some_and(|v| v.is_empty()) {
            self.negative_spans = None;
        }
        if self.positive_buckets.as_ref().is_some_and(|v| v.is_empty()) {
            self.positive_buckets = None;
        }
        if self.negative_buckets.as_ref().is_some_and(|v| v.is_empty()) {
            self.negative_buckets = None;
        }
    }

    pub fn compact(&mut self, max_empty_buckets: isize) {
        self.compact_in(max_empty_buckets, None);
    }
    pub(crate) fn compact_in(
        &mut self,
        max_empty_buckets: isize,
        context: Option<&AllocationContext>,
    ) {
        compact_float(
            &mut self.positive_buckets,
            &mut self.positive_spans,
            max_empty_buckets,
            context,
        );
        compact_float(
            &mut self.negative_buckets,
            &mut self.negative_spans,
            max_empty_buckets,
            context,
        );
    }
    pub(crate) fn compact_with<E>(
        &mut self,
        max_empty_buckets: isize,
        context: Option<&AllocationContext>,
        mut poll: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<(), E> {
        compact_float_pair_with(
            &mut self.positive_buckets,
            &mut self.positive_spans,
            None,
            max_empty_buckets,
            context,
            &mut poll,
        )?;
        compact_float_pair_with(
            &mut self.negative_buckets,
            &mut self.negative_spans,
            None,
            max_empty_buckets,
            context,
            poll,
        )
    }

    pub fn copy_to_schema(&self, target: i32) -> Self {
        if target == self.schema {
            return self.copy();
        }
        self.as_ref().copy_to_schema(target)
    }

    pub fn reduce_resolution(&mut self, target: i32) -> Result<(), HistogramError> {
        if self.uses_custom_buckets() {
            return Err(HistogramError::CustomSource);
        }
        if target == CUSTOM_BUCKETS_SCHEMA {
            return Err(HistogramError::CustomTarget);
        }
        if target >= self.schema {
            return Err(HistogramError::NonReducing {
                origin: self.schema,
                target,
            });
        }
        reduce_float_in_place(
            &mut self.positive_spans,
            &mut self.positive_buckets,
            self.schema,
            target,
        )?;
        reduce_float_in_place(
            &mut self.negative_spans,
            &mut self.negative_buckets,
            self.schema,
            target,
        )?;
        self.schema = target;
        Ok(())
    }

    /// Copy only the fields that upstream uses for this schema.
    pub fn copy(&self) -> Self {
        self.copy_in(None)
    }

    pub(crate) fn copy_in(&self, context: Option<&AllocationContext>) -> Self {
        self.copy_with(context, HistogramWork::default())
            .expect("inert histogram work")
    }
    pub(crate) fn copy_with(
        &self,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<Self, Interrupted> {
        let mut result = Self::default();
        self.as_ref()
            .copy_fields_to_with(&mut result, context, work)?;
        result.custom_values = if self.uses_custom_buckets() {
            if let Some(values) = &self.custom_values {
                work.step(values.len())?;
                Some(values.clone_in(context))
            } else {
                None
            }
        } else {
            None
        };
        work.step(1)?;
        Ok(result)
    }

    pub fn copy_to(&self, destination: &mut Self) {
        self.copy_to_in(destination, None);
    }

    fn copy_to_in(&self, destination: &mut Self, context: Option<&AllocationContext>) {
        self.as_ref().copy_fields_to(destination, context);
        destination.custom_values = if self.uses_custom_buckets() {
            self.custom_values
                .as_ref()
                .map(|values| values.clone_in(context))
        } else {
            None
        };
    }

    pub fn mul(&mut self, factor: f64) {
        let result: Result<(), std::convert::Infallible> = self.mul_with(factor, || Ok(()));
        match result {
            Ok(()) => (),
            Err(never) => match never {},
        }
    }
    pub(crate) fn mul_with<E>(
        &mut self,
        factor: f64,
        mut poll: impl FnMut() -> Result<(), E>,
    ) -> Result<(), E> {
        poll()?;
        self.zero_count *= factor;
        self.count *= factor;
        self.sum *= factor;
        for bucket in self.positive_buckets.iter_mut().flatten() {
            poll()?;
            *bucket *= factor;
        }
        for bucket in self.negative_buckets.iter_mut().flatten() {
            poll()?;
            *bucket *= factor;
        }
        if factor < 0.0 {
            self.reset_hint = GAUGE;
        }
        Ok(())
    }

    pub fn div(&mut self, divisor: f64) {
        let result: Result<(), std::convert::Infallible> = self.div_with(divisor, || Ok(()));
        match result {
            Ok(()) => (),
            Err(never) => match never {},
        }
    }
    pub(crate) fn div_with<E>(
        &mut self,
        divisor: f64,
        mut poll: impl FnMut() -> Result<(), E>,
    ) -> Result<(), E> {
        poll()?;
        self.zero_count /= divisor;
        self.count /= divisor;
        self.sum /= divisor;
        if divisor == 0.0 {
            // Upstream drops these allocations and returns before changing the hint.
            self.positive_spans = None;
            self.negative_spans = None;
            self.positive_buckets = None;
            self.negative_buckets = None;
            return Ok(());
        }
        for bucket in self.positive_buckets.iter_mut().flatten() {
            poll()?;
            *bucket /= divisor;
        }
        for bucket in self.negative_buckets.iter_mut().flatten() {
            poll()?;
            *bucket /= divisor;
        }
        if divisor < 0.0 {
            self.reset_hint = GAUGE;
        }
        Ok(())
    }
}

impl<'a> FloatHistogramRef<'a> {
    pub fn detect_reset(self, previous: FloatHistogramRef<'_>) -> bool {
        crate::histogram_counter::detect_reset(self, previous)
    }

    pub fn zero_count_for_threshold(
        self,
        threshold: f64,
        correction: Option<FloatHistogramRef<'_>>,
    ) -> ZeroCount {
        crate::histogram_counter::zero_count_for_threshold(self, threshold, correction)
    }

    pub fn bucket_iterator(
        self,
        positive: bool,
        start: f64,
        target: i32,
    ) -> FloatBucketIterator<'a> {
        FloatBucketIterator::new(self, positive, start, target)
    }

    pub fn reverse_bucket_iterator(self, positive: bool) -> ReverseFloatBucketIterator<'a> {
        ReverseFloatBucketIterator::new(self, positive)
    }

    pub fn all_bucket_iterator(self, reverse: bool) -> AllFloatBucketIterator<'a> {
        AllFloatBucketIterator::new(self, reverse)
    }

    pub fn zero_bucket(self) -> Bucket {
        if self.schema == CUSTOM_BUCKETS_SCHEMA {
            panic!("histograms with custom buckets have no zero bucket");
        }
        Bucket {
            lower: -self.zero_threshold,
            upper: self.zero_threshold,
            lower_inclusive: true,
            upper_inclusive: true,
            count: self.zero_count,
            index: 0,
        }
    }

    pub fn copy_to_schema(self, target: i32) -> FloatHistogram {
        self.copy_to_schema_with(target, None, HistogramWork::default())
            .unwrap_or_else(|error| panic!("{}", error.arithmetic()))
    }
    pub(crate) fn copy_to_schema_with(
        self,
        target: i32,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<FloatHistogram, HistogramEvaluationError> {
        work.step(1)?;
        if target == self.schema {
            return Ok(self.to_owned_with(context, work)?);
        }
        assert!(
            self.schema != CUSTOM_BUCKETS_SCHEMA,
            "cannot reduce resolution to {target} when there are custom buckets"
        );
        assert!(
            target != CUSTOM_BUCKETS_SCHEMA,
            "cannot reduce resolution to custom buckets schema"
        );
        assert!(
            target <= self.schema,
            "cannot copy from schema {} to {target}",
            self.schema
        );
        let mut result = FloatHistogram {
            schema: target,
            zero_threshold: self.zero_threshold,
            zero_count: self.zero_count,
            count: self.count,
            sum: self.sum,
            ..FloatHistogram::default()
        };
        (result.positive_spans, result.positive_buckets) = reduce_float_copy_with(
            self.positive_spans,
            self.positive_buckets,
            self.schema,
            target,
            context,
            work,
        )?;
        (result.negative_spans, result.negative_buckets) = reduce_float_copy_with(
            self.negative_spans,
            self.negative_buckets,
            self.schema,
            target,
            context,
            work,
        )?;
        Ok(result)
    }
    fn copy_fields_to(self, destination: &mut FloatHistogram, context: Option<&AllocationContext>) {
        self.copy_fields_to_with(destination, context, HistogramWork::default())
            .expect("inert histogram work");
    }
    fn copy_fields_to_with(
        self,
        destination: &mut FloatHistogram,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<(), Interrupted> {
        work.step(1)?;
        destination.reset_hint = self.reset_hint;
        destination.schema = self.schema;
        destination.count = self.count;
        destination.sum = self.sum;
        if self.schema == CUSTOM_BUCKETS_SCHEMA {
            destination.zero_threshold = 0.0;
            destination.zero_count = 0.0;
            clear_if_present(&mut destination.negative_spans);
            clear_if_present(&mut destination.negative_buckets);
        } else {
            destination.zero_threshold = self.zero_threshold;
            destination.zero_count = self.zero_count;
            overwrite_with(
                &mut destination.negative_spans,
                self.negative_spans,
                context,
                work,
            )?;
            overwrite_with(
                &mut destination.negative_buckets,
                self.negative_buckets,
                context,
                work,
            )?;
        }
        overwrite_with(
            &mut destination.positive_spans,
            self.positive_spans,
            context,
            work,
        )?;
        overwrite_with(
            &mut destination.positive_buckets,
            self.positive_buckets,
            context,
            work,
        )?;
        Ok(())
    }

    pub fn copy_to(self, destination: &mut FloatHistogram) {
        self.copy_to_in(destination, None);
    }
    pub(crate) fn copy_to_in(
        self,
        destination: &mut FloatHistogram,
        context: Option<&AllocationContext>,
    ) {
        self.copy_to_with(destination, context, HistogramWork::default())
            .expect("inert histogram work");
    }
    pub(crate) fn copy_to_with(
        self,
        destination: &mut FloatHistogram,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<(), Interrupted> {
        self.copy_fields_to_with(destination, context, work)?;
        // Retaining a provider view requires ownership independent of the page.
        destination.custom_values = if self.schema == CUSTOM_BUCKETS_SCHEMA {
            if let Some(values) = self.custom_values {
                work.step(values.len())?;
                Some(HistogramBounds::from_slice_in(values, context))
            } else {
                None
            }
        } else {
            None
        };
        work.step(1)?;
        Ok(())
    }

    pub fn to_owned(self) -> FloatHistogram {
        self.to_owned_in(None)
    }
    pub(crate) fn to_owned_in(self, context: Option<&AllocationContext>) -> FloatHistogram {
        self.to_owned_with(context, HistogramWork::default())
            .expect("inert histogram work")
    }
    pub(crate) fn to_owned_with(
        self,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<FloatHistogram, Interrupted> {
        let mut result = FloatHistogram::default();
        self.copy_to_with(&mut result, context, work)?;
        Ok(result)
    }
    pub(crate) fn to_owned_result_in(self, context: &AllocationContext) -> FloatHistogram {
        // Result ownership preserves evaluated fields, including ignored custom fields changed by arithmetic.
        let context = Some(context);
        FloatHistogram {
            reset_hint: self.reset_hint,
            schema: self.schema,
            zero_threshold: self.zero_threshold,
            zero_count: self.zero_count,
            count: self.count,
            sum: self.sum,
            positive_spans: self
                .positive_spans
                .map(|values| AccountedVec::from_slice_in(values, context)),
            negative_spans: self
                .negative_spans
                .map(|values| AccountedVec::from_slice_in(values, context)),
            positive_buckets: self
                .positive_buckets
                .map(|values| AccountedVec::from_slice_in(values, context)),
            negative_buckets: self
                .negative_buckets
                .map(|values| AccountedVec::from_slice_in(values, context)),
            custom_values: self
                .custom_values
                .map(|values| HistogramBounds::from_slice_in(values, context)),
        }
    }
}

impl IntegerHistogram {
    pub fn as_ref(&self) -> IntegerHistogramRef<'_> {
        IntegerHistogramRef {
            reset_hint: self.reset_hint,
            schema: self.schema,
            zero_threshold: self.zero_threshold,
            zero_count: self.zero_count,
            count: self.count,
            sum: self.sum,
            positive_spans: self.positive_spans.as_deref(),
            negative_spans: self.negative_spans.as_deref(),
            positive_buckets: self.positive_buckets.as_deref(),
            negative_buckets: self.negative_buckets.as_deref(),
            custom_values: self.custom_values.as_deref(),
        }
    }

    pub fn copy(&self) -> Self {
        let mut result = Self::default();
        self.copy_to(&mut result);
        result
    }

    pub fn copy_to(&self, destination: &mut Self) {
        self.as_ref().copy_fields_to(destination);
        destination.custom_values = if self.schema == CUSTOM_BUCKETS_SCHEMA {
            self.custom_values.clone()
        } else {
            None
        };
    }

    pub fn to_float(&self) -> FloatHistogram {
        let mut result = FloatHistogram::default();
        self.to_float_into(&mut result);
        result
    }

    pub fn to_float_into(&self, destination: &mut FloatHistogram) {
        self.as_ref().convert_fields_to(destination, None);
        destination.custom_values = if self.schema == CUSTOM_BUCKETS_SCHEMA {
            self.custom_values.clone().map(Into::into)
        } else {
            None
        };
    }
}

fn decode_deltas(
    destination: &mut Option<AccountedVec<f64>>,
    deltas: Option<&[i64]>,
    context: Option<&AllocationContext>,
    work: HistogramWork<'_>,
) -> Result<(), Interrupted> {
    work.step(1)?;
    let deltas = deltas.unwrap_or(&[]);
    if destination.is_none() && deltas.is_empty() {
        return Ok(());
    }
    let destination = destination.get_or_insert_with(|| AccountedVec::new_in(context));
    destination.clear();
    destination.reserve(deltas.len());
    let mut current = 0.0;
    for &delta in deltas {
        work.step(1)?;
        // Convert before adding: integer accumulation changes rounding above 2^53.
        current += delta as f64;
        destination.push(current);
    }
    Ok(())
}

impl IntegerHistogramRef<'_> {
    fn copy_fields_to(self, destination: &mut IntegerHistogram) {
        destination.reset_hint = self.reset_hint;
        destination.schema = self.schema;
        destination.count = self.count;
        destination.sum = self.sum;
        if self.schema == CUSTOM_BUCKETS_SCHEMA {
            destination.zero_threshold = 0.0;
            destination.zero_count = 0;
            clear_if_present(&mut destination.negative_spans);
            clear_if_present(&mut destination.negative_buckets);
        } else {
            destination.zero_threshold = self.zero_threshold;
            destination.zero_count = self.zero_count;
            overwrite(&mut destination.negative_spans, self.negative_spans, None);
            overwrite(
                &mut destination.negative_buckets,
                self.negative_buckets,
                None,
            );
        }
        overwrite(&mut destination.positive_spans, self.positive_spans, None);
        overwrite(
            &mut destination.positive_buckets,
            self.positive_buckets,
            None,
        );
    }

    pub fn copy_to(self, destination: &mut IntegerHistogram) {
        self.copy_fields_to(destination);
        destination.custom_values = if self.schema == CUSTOM_BUCKETS_SCHEMA {
            self.custom_values.map(Arc::from)
        } else {
            None
        };
    }

    pub fn to_owned(self) -> IntegerHistogram {
        let mut result = IntegerHistogram::default();
        self.copy_to(&mut result);
        result
    }

    fn convert_fields_to(
        self,
        destination: &mut FloatHistogram,
        context: Option<&AllocationContext>,
    ) {
        self.convert_fields_to_with(destination, context, HistogramWork::default())
            .expect("inert histogram work");
    }
    fn convert_fields_to_with(
        self,
        destination: &mut FloatHistogram,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<(), Interrupted> {
        work.step(1)?;
        destination.reset_hint = self.reset_hint;
        destination.schema = self.schema;
        destination.count = self.count as f64;
        destination.sum = self.sum;
        if self.schema == CUSTOM_BUCKETS_SCHEMA {
            destination.zero_threshold = 0.0;
            destination.zero_count = 0.0;
            clear_if_present(&mut destination.negative_spans);
            clear_if_present(&mut destination.negative_buckets);
        } else {
            destination.zero_threshold = self.zero_threshold;
            destination.zero_count = self.zero_count as f64;
            overwrite_with(
                &mut destination.negative_spans,
                self.negative_spans,
                context,
                work,
            )?;
            decode_deltas(
                &mut destination.negative_buckets,
                self.negative_buckets,
                context,
                work,
            )?;
        }
        overwrite_with(
            &mut destination.positive_spans,
            self.positive_spans,
            context,
            work,
        )?;
        decode_deltas(
            &mut destination.positive_buckets,
            self.positive_buckets,
            context,
            work,
        )?;
        Ok(())
    }

    pub fn to_float_into(self, destination: &mut FloatHistogram) {
        self.to_float_into_in(destination, None);
    }
    fn to_float_into_in(
        self,
        destination: &mut FloatHistogram,
        context: Option<&AllocationContext>,
    ) {
        self.to_float_into_with(destination, context, HistogramWork::default())
            .expect("inert histogram work");
    }
    pub(crate) fn to_float_into_with(
        self,
        destination: &mut FloatHistogram,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<(), Interrupted> {
        self.convert_fields_to_with(destination, context, work)?;
        destination.custom_values = if self.schema == CUSTOM_BUCKETS_SCHEMA {
            if let Some(values) = self.custom_values {
                work.step(values.len())?;
                Some(HistogramBounds::from_slice_in(values, context))
            } else {
                None
            }
        } else {
            None
        };
        work.step(1)?;
        Ok(())
    }

    pub fn to_float(self) -> FloatHistogram {
        self.to_float_in(None)
    }
    pub(crate) fn to_float_in(self, context: Option<&AllocationContext>) -> FloatHistogram {
        self.to_float_with(context, HistogramWork::default())
            .expect("inert histogram work")
    }
    pub(crate) fn to_float_with(
        self,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<FloatHistogram, Interrupted> {
        let mut result = FloatHistogram::default();
        self.to_float_into_with(&mut result, context, work)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::{EngineAccounting, MemoryKind};

    #[test]
    fn bounds_move_retained_capacity_and_share_only_the_same_query_and_kind() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let other_account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Workspace);
        let mut values = Vec::with_capacity(257);
        values.extend_from_slice(&[1.0, 2.0]);
        let capacity = values.capacity();
        let address = values.as_ptr();
        let bounds = HistogramBounds::from_owned(AccountedVec::from_vec_in(values, Some(&context)));
        let wrapper = size_of::<crate::shared::Allocation<AccountedVec<f64>>>() as u64;
        let bytes = wrapper + (capacity * size_of::<f64>()) as u64;
        assert_eq!(bounds.as_ptr(), address);
        assert_eq!(account.snapshot().memory[1].current, bytes);
        let before = account.snapshot();
        let alias = bounds.clone_in(Some(&context));
        assert!(bounds.same_allocation(&alias));
        assert_eq!(account.snapshot(), before);
        let different_kind = bounds.clone_in(Some(&context.with_kind(MemoryKind::Result)));
        let different_query = bounds.clone_in(Some(&AllocationContext::new(
            &other_account,
            MemoryKind::Workspace,
        )));
        assert!(!bounds.same_allocation(&different_kind));
        assert!(!bounds.same_allocation(&different_query));
        assert_eq!(account.snapshot().memory[2].current, wrapper + 16);
        assert_eq!(other_account.snapshot().memory[1].current, wrapper + 16);
        drop((bounds, context, account));
        assert_eq!(&*alias, &[1.0, 2.0]);
        assert_eq!(engine.snapshot().memory[1].current, bytes + wrapper + 16);
        drop((alias, different_kind, different_query, other_account));
        assert!(
            engine
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }

    #[test]
    fn bounds_external_import_and_empty_owned_headers_keep_their_distinct_identity() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Result);
        let external: Arc<[f64]> = Arc::from([1.0, 2.0]);
        let first = HistogramBounds::from(external.clone());
        let second = HistogramBounds::from(external);
        assert!(first.same_allocation(&second));
        assert!(first.same_allocation(&first.clone_in(None)));
        let copied = first.clone_in(Some(&context));
        assert!(!first.same_allocation(&copied));
        assert!(copied.same_allocation(&copied.clone_in(Some(&context))));
        let left = HistogramBounds::from_slice_in(&[], Some(&context));
        let right = HistogramBounds::from_slice_in(&[], Some(&context));
        assert_eq!(left.as_ptr(), right.as_ptr());
        assert!(!left.same_allocation(&right));
        assert!(left.same_allocation(&left.clone()));
        let histogram = FloatHistogram {
            schema: CUSTOM_BUCKETS_SCHEMA,
            custom_values: Some(left),
            ..FloatHistogram::default()
        };
        assert_eq!(histogram.as_ref().custom_values, Some([].as_slice()));
        let absent = FloatHistogram {
            schema: CUSTOM_BUCKETS_SCHEMA,
            ..FloatHistogram::default()
        };
        assert_eq!(absent.as_ref().custom_values, None);
        let wrapper = size_of::<crate::shared::Allocation<AccountedVec<f64>>>() as u64;
        assert_eq!(account.snapshot().memory[2].current, 3 * wrapper + 16);
        drop((copied, right, histogram));
        assert_eq!(account.snapshot().memory[2].current, 0);
    }

    #[test]
    fn reduction_keeps_arrays_attached_when_an_invalid_schema_panics() {
        let mut value = FloatHistogram {
            schema: i32::MAX,
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: 1,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![3.0].into()),
            ..FloatHistogram::default()
        };
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| value.reduce_resolution(-1)));
        assert!(result.is_err());
        assert_eq!(value.schema, i32::MAX);
        assert_eq!(
            value.positive_spans.as_deref(),
            Some(
                [Span {
                    offset: 0,
                    length: 1
                }]
                .as_slice()
            )
        );
        assert_eq!(value.positive_buckets.as_deref(), Some([3.0].as_slice()));
    }

    #[test]
    fn owned_copy_keeps_mutable_arrays_independent_and_shares_immutable_bounds() {
        let mut source = FloatHistogram {
            schema: CUSTOM_BUCKETS_SCHEMA,
            sum: 12.0,
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: 2,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![-0.0, 3.0].into()),
            custom_values: Some(Arc::<[f64]>::from([1.0, 2.0]).into()),
            ..FloatHistogram::default()
        };
        let copied = source.copy();
        assert!(
            source
                .custom_values
                .as_ref()
                .unwrap()
                .same_allocation(copied.custom_values.as_ref().unwrap())
        );
        source.sum = 4.0;
        source.positive_spans.as_mut().unwrap()[0].offset = 2;
        source.positive_buckets.as_mut().unwrap()[0] = 7.0;
        assert_eq!(copied.sum, 12.0);
        assert_eq!(copied.positive_spans.as_ref().unwrap()[0].offset, 0);
        assert_eq!(
            copied.positive_buckets.as_ref().unwrap()[0].to_bits(),
            (-0.0f64).to_bits()
        );

        let mut source = IntegerHistogram {
            schema: CUSTOM_BUCKETS_SCHEMA,
            positive_spans: Some(vec![Span {
                offset: 0,
                length: 1,
            }]),
            positive_buckets: Some(vec![3]),
            custom_values: Some(Arc::from([2.0])),
            ..IntegerHistogram::default()
        };
        let copied = source.copy();
        assert!(Arc::ptr_eq(
            source.custom_values.as_ref().unwrap(),
            copied.custom_values.as_ref().unwrap()
        ));
        source.positive_spans.as_mut().unwrap()[0].length = 2;
        source.positive_buckets.as_mut().unwrap()[0] = 7;
        assert_eq!(copied.positive_spans.as_ref().unwrap()[0].length, 1);
        assert_eq!(copied.positive_buckets.as_deref(), Some([3].as_slice()));
    }

    #[test]
    fn retained_cursor_views_survive_release_of_every_provider_array() {
        let (copied, integer_copy, converted) = {
            let positive_spans = vec![Span {
                offset: 0,
                length: 2,
            }];
            let negative_spans = vec![Span {
                offset: -1,
                length: 1,
            }];
            let positive = vec![-0.0, 3.0];
            let negative = vec![2.0];
            let deltas = vec![3, 1];
            let negative_deltas = vec![2];
            let bounds = vec![1.0, 2.0];
            let float = FloatHistogramRef {
                reset_hint: GAUGE,
                schema: CUSTOM_BUCKETS_SCHEMA,
                zero_threshold: 99.0,
                zero_count: 12.0,
                count: 3.0,
                sum: -0.0,
                positive_spans: Some(&positive_spans),
                negative_spans: Some(&negative_spans),
                positive_buckets: Some(&positive),
                negative_buckets: Some(&negative),
                custom_values: Some(&bounds),
            };
            let integer = IntegerHistogramRef {
                reset_hint: NOT_RESET,
                schema: CUSTOM_BUCKETS_SCHEMA,
                zero_threshold: 99.0,
                zero_count: 12,
                count: 4,
                sum: -0.0,
                positive_spans: Some(&positive_spans),
                negative_spans: Some(&negative_spans),
                positive_buckets: Some(&deltas),
                negative_buckets: Some(&negative_deltas),
                custom_values: Some(&bounds),
            };
            (float.to_owned(), integer.to_owned(), integer.to_float())
        };
        assert_eq!(
            copied.positive_buckets.as_ref().unwrap()[0].to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(copied.positive_buckets.as_ref().unwrap()[1], 3.0);
        assert_eq!(copied.custom_values.as_deref(), Some([1.0, 2.0].as_slice()));
        assert_eq!(copied.sum.to_bits(), (-0.0f64).to_bits());
        assert!(copied.negative_spans.is_none() && copied.negative_buckets.is_none());
        assert_eq!(copied.zero_threshold, 0.0);
        assert_eq!(
            integer_copy.positive_buckets.as_deref(),
            Some([3, 1].as_slice())
        );
        assert_eq!(
            integer_copy.custom_values.as_deref(),
            Some([1.0, 2.0].as_slice())
        );
        assert_eq!(
            converted.positive_buckets.as_deref(),
            Some([3.0, 4.0].as_slice())
        );
        assert_eq!(
            converted.custom_values.as_deref(),
            Some([1.0, 2.0].as_slice())
        );
        assert!(converted.negative_buckets.is_none());
    }
}
