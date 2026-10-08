// SPDX-License-Identifier: GPL-3.0-or-later
// Arithmetic derives from pinned Prometheus model/histogram; see NOTICE.
use crate::accounting::{AccountedVec, AllocationContext};
use crate::histogram::{
    CUSTOM_BUCKETS_SCHEMA, FloatHistogram, FloatHistogramRef, GAUGE, HistogramBounds,
    HistogramError, Span, UNKNOWN_RESET, bucket_bound,
};
use crate::histogram_counter::{custom_bounds_match_with, zero_count_for_threshold_with};
use crate::histogram_layout::{
    compact_float_pair, compact_float_pair_with, reduce_float_copy_with,
    reduce_float_in_place_with, reduce_kahan_copy_with, reduce_kahan_in_place_with,
};
use crate::histogram_work::{HistogramEvaluationError, HistogramWork};
use crate::numerical::{compensated_add, compensated_sub, source_add, source_neg, source_sub};
use crate::storage::Interrupted;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HistogramArithmetic {
    pub counter_reset_collision: bool,
    pub nhcb_bounds_reconciled: bool,
}

impl FloatHistogram {
    fn adjust_reset(&mut self, other: u8) -> bool {
        if other == self.reset_hint || self.reset_hint == GAUGE {
            return false;
        }
        if other == GAUGE {
            self.reset_hint = GAUGE;
        } else if self.reset_hint == UNKNOWN_RESET {
            return false;
        } else if other == UNKNOWN_RESET {
            self.reset_hint = UNKNOWN_RESET;
        } else {
            self.reset_hint = UNKNOWN_RESET;
            return true;
        }
        false
    }

    /// Compact paired counts; correction span headers follow the upstream helper's lifecycle.
    pub fn kahan_compact(&mut self, max_empty: i64, correction: &mut Self) {
        self.kahan_compact_in(max_empty, correction, None);
    }
    fn kahan_compact_in(
        &mut self,
        max_empty: i64,
        correction: &mut Self,
        context: Option<&AllocationContext>,
    ) {
        compact_float_pair(
            &mut self.positive_buckets,
            &mut self.positive_spans,
            Some(&mut correction.positive_buckets),
            max_empty,
            context,
        );
        compact_float_pair(
            &mut self.negative_buckets,
            &mut self.negative_spans,
            Some(&mut correction.negative_buckets),
            max_empty,
            context,
        );
    }

    fn trim_zero(
        &mut self,
        mut correction: Option<&mut Self>,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<(), Interrupted> {
        for positive in [true, false] {
            work.step(1)?;
            let mut count = 0;
            // Bounds are independent of counts; scan the borrowed prefix before zeroing it.
            let mut iterator = self.bucket_iterator(positive, 0.0, self.schema);
            while iterator.advance_with(work)? {
                let bucket = iterator.at();
                if if positive {
                    bucket.lower >= self.zero_threshold
                } else {
                    bucket.upper <= -self.zero_threshold
                } {
                    break;
                }
                count += 1;
            }
            let values = if positive {
                &mut self.positive_buckets
            } else {
                &mut self.negative_buckets
            };
            for index in 0..count {
                work.step(1)?;
                values.as_mut().unwrap()[index] = 0.0;
                if let Some(correction) = correction.as_mut() {
                    let values = if positive {
                        &mut correction.positive_buckets
                    } else {
                        &mut correction.negative_buckets
                    };
                    values.as_mut().unwrap()[index] = 0.0;
                }
            }
        }
        if let Some(correction) = correction {
            compact_float_pair_with(
                &mut self.positive_buckets,
                &mut self.positive_spans,
                Some(&mut correction.positive_buckets),
                0,
                context,
                |units| work.step(units),
            )?;
            compact_float_pair_with(
                &mut self.negative_buckets,
                &mut self.negative_spans,
                Some(&mut correction.negative_buckets),
                0,
                context,
                |units| work.step(units),
            )?;
        } else {
            self.compact_with(0, context, |units| work.step(units))?;
        }
        Ok(())
    }

    fn reconcile_zero(
        &mut self,
        other: FloatHistogramRef<'_>,
        mut correction: Option<&mut Self>,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<(f64, f64), Interrupted> {
        let (mut count, mut threshold, mut comp) = (other.zero_count, other.zero_threshold, 0.0);
        while threshold != self.zero_threshold {
            work.step(1)?;
            if self.zero_threshold > threshold {
                let zero = zero_count_for_threshold_with(other, self.zero_threshold, None, work)?;
                (count, threshold, comp) = (zero.count, zero.threshold, zero.compensation);
            }
            if threshold > self.zero_threshold {
                let zero = zero_count_for_threshold_with(
                    self.as_ref(),
                    threshold,
                    correction.as_deref().map(Self::as_ref),
                    work,
                )?;
                self.zero_count = zero.count;
                self.zero_threshold = zero.threshold;
                if let Some(correction) = correction.as_mut() {
                    correction.zero_count = zero.compensation;
                }
                self.trim_zero(correction.as_deref_mut(), context, work)?;
            }
        }
        Ok((count, comp))
    }

    pub fn add(
        &mut self,
        other: FloatHistogramRef<'_>,
    ) -> Result<HistogramArithmetic, HistogramError> {
        self.add_or_sub(other, false, None)
    }

    pub fn sub(
        &mut self,
        other: FloatHistogramRef<'_>,
    ) -> Result<HistogramArithmetic, HistogramError> {
        self.add_or_sub(other, true, None)
    }

    fn add_or_sub(
        &mut self,
        other: FloatHistogramRef<'_>,
        negative: bool,
        context: Option<&AllocationContext>,
    ) -> Result<HistogramArithmetic, HistogramError> {
        self.add_or_sub_with(other, negative, context, HistogramWork::default())
            .map_err(HistogramEvaluationError::arithmetic)
    }

    pub(crate) fn add_or_sub_with(
        &mut self,
        other: FloatHistogramRef<'_>,
        negative: bool,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<HistogramArithmetic, HistogramEvaluationError> {
        work.step(1)?;
        if self.uses_custom_buckets() != (other.schema == CUSTOM_BUCKETS_SCHEMA) {
            return Err(HistogramError::IncompatibleSchema.into());
        }
        let mut flags = HistogramArithmetic {
            counter_reset_collision: self.adjust_reset(other.reset_hint),
            nhcb_bounds_reconciled: false,
        };
        if !self.uses_custom_buckets() {
            let (zero, _) = self.reconcile_zero(other, None, context, work)?;
            if negative {
                self.zero_count = source_sub(self.zero_count, zero);
            } else {
                self.zero_count = source_add(zero, self.zero_count);
            }
        }
        if negative {
            self.count = source_sub(self.count, other.count);
            self.sum = source_sub(self.sum, other.sum);
        } else {
            self.count = source_add(self.count, other.count);
            self.sum = source_add(other.sum, self.sum);
        }
        if self.uses_custom_buckets() {
            if custom_bounds_match_with(self.custom_values.as_deref(), other.custom_values, work)? {
                merge_buckets(
                    self.schema,
                    self.zero_threshold,
                    negative,
                    &mut self.positive_spans,
                    &mut self.positive_buckets,
                    None,
                    other.positive_spans,
                    other.positive_buckets,
                    None,
                    context,
                    work,
                )?;
            } else {
                flags.nhcb_bounds_reconciled = true;
                let bounds = intersect_bounds(
                    self.custom_values.as_deref(),
                    other.custom_values,
                    context,
                    work,
                )?;
                map_custom(
                    &mut self.positive_spans,
                    &mut self.positive_buckets,
                    self.custom_values.as_deref(),
                    other,
                    None,
                    bounds.as_deref(),
                    negative,
                    context,
                    work,
                )?;
                self.custom_values = bounds;
            }
            return Ok(flags);
        }
        let (mut positive, mut negative_layout) = (None, None);
        if other.schema < self.schema {
            reduce_float_in_place_with(
                &mut self.positive_spans,
                &mut self.positive_buckets,
                self.schema,
                other.schema,
                work,
            )
            .map_err(HistogramEvaluationError::panic_arithmetic)?;
            reduce_float_in_place_with(
                &mut self.negative_spans,
                &mut self.negative_buckets,
                self.schema,
                other.schema,
                work,
            )
            .map_err(HistogramEvaluationError::panic_arithmetic)?;
            self.schema = other.schema;
        } else if other.schema > self.schema {
            positive = Some(
                reduce_float_copy_with(
                    other.positive_spans,
                    other.positive_buckets,
                    other.schema,
                    self.schema,
                    context,
                    work,
                )
                .map_err(HistogramEvaluationError::panic_arithmetic)?,
            );
            negative_layout = Some(
                reduce_float_copy_with(
                    other.negative_spans,
                    other.negative_buckets,
                    other.schema,
                    self.schema,
                    context,
                    work,
                )
                .map_err(HistogramEvaluationError::panic_arithmetic)?,
            );
        }
        let (pos_spans, pos_buckets) = positive.as_ref().map_or(
            (other.positive_spans, other.positive_buckets),
            |(spans, buckets)| (spans.as_deref(), buckets.as_deref()),
        );
        let (neg_spans, neg_buckets) = negative_layout.as_ref().map_or(
            (other.negative_spans, other.negative_buckets),
            |(spans, buckets)| (spans.as_deref(), buckets.as_deref()),
        );
        merge_buckets(
            self.schema,
            self.zero_threshold,
            negative,
            &mut self.positive_spans,
            &mut self.positive_buckets,
            None,
            pos_spans,
            pos_buckets,
            None,
            context,
            work,
        )?;
        merge_buckets(
            self.schema,
            self.zero_threshold,
            negative,
            &mut self.negative_spans,
            &mut self.negative_buckets,
            None,
            neg_spans,
            neg_buckets,
            None,
            context,
            work,
        )?;
        Ok(flags)
    }

    fn new_correction_with(
        &self,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<Self, Interrupted> {
        let mut positive_spans = None;
        let mut negative_spans = None;
        clone_spans_with(&mut positive_spans, &self.positive_spans, work)?;
        clone_spans_with(&mut negative_spans, &self.negative_spans, work)?;
        Ok(Self {
            reset_hint: self.reset_hint,
            schema: self.schema,
            zero_threshold: self.zero_threshold,
            positive_spans,
            negative_spans,
            positive_buckets: Some(zeros_with(
                self.positive_buckets.as_ref().map_or(0, |v| v.len()),
                context,
                work,
            )?),
            negative_buckets: if self.uses_custom_buckets() {
                None
            } else {
                Some(zeros_with(
                    self.negative_buckets.as_ref().map_or(0, |v| v.len()),
                    context,
                    work,
                )?)
            },
            custom_values: self.custom_values.clone(),
            ..Self::default()
        })
    }

    pub fn kahan_add(
        &mut self,
        other: FloatHistogramRef<'_>,
        correction: &mut Option<Self>,
    ) -> Result<HistogramArithmetic, HistogramError> {
        self.kahan_add_in(other, correction, None)
    }
    pub(crate) fn kahan_add_in(
        &mut self,
        other: FloatHistogramRef<'_>,
        correction: &mut Option<Self>,
        context: Option<&AllocationContext>,
    ) -> Result<HistogramArithmetic, HistogramError> {
        self.kahan_add_with(other, correction, context, HistogramWork::default())
            .map_err(HistogramEvaluationError::arithmetic)
    }

    pub(crate) fn kahan_add_with(
        &mut self,
        other: FloatHistogramRef<'_>,
        correction: &mut Option<Self>,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<HistogramArithmetic, HistogramEvaluationError> {
        work.step(1)?;
        if self.uses_custom_buckets() != (other.schema == CUSTOM_BUCKETS_SCHEMA) {
            return Err(HistogramError::IncompatibleSchema.into());
        }
        let mut flags = HistogramArithmetic {
            counter_reset_collision: self.adjust_reset(other.reset_hint),
            nhcb_bounds_reconciled: false,
        };
        if correction.is_none() {
            *correction = Some(self.new_correction_with(context, work)?);
        }
        let correction = correction.as_mut().unwrap();
        if !self.uses_custom_buckets() {
            let (zero, comp) = self.reconcile_zero(other, Some(correction), context, work)?;
            (self.zero_count, correction.zero_count) =
                compensated_add(zero, self.zero_count, correction.zero_count);
            (self.zero_count, correction.zero_count) =
                compensated_add(comp, self.zero_count, correction.zero_count);
        }
        (self.count, correction.count) = compensated_add(other.count, self.count, correction.count);
        (self.sum, correction.sum) = compensated_add(other.sum, self.sum, correction.sum);
        if self.uses_custom_buckets() {
            if custom_bounds_match_with(self.custom_values.as_deref(), other.custom_values, work)? {
                merge_buckets(
                    self.schema,
                    self.zero_threshold,
                    false,
                    &mut self.positive_spans,
                    &mut self.positive_buckets,
                    Some(&mut correction.positive_buckets),
                    other.positive_spans,
                    other.positive_buckets,
                    None,
                    context,
                    work,
                )?;
            } else {
                flags.nhcb_bounds_reconciled = true;
                let bounds = intersect_bounds(
                    self.custom_values.as_deref(),
                    other.custom_values,
                    context,
                    work,
                )?;
                map_custom(
                    &mut self.positive_spans,
                    &mut self.positive_buckets,
                    self.custom_values.as_deref(),
                    other,
                    Some(&mut correction.positive_buckets),
                    bounds.as_deref(),
                    false,
                    context,
                    work,
                )?;
                self.custom_values = bounds;
                correction.custom_values = self.custom_values.clone();
            }
            clone_spans_with(&mut correction.positive_spans, &self.positive_spans, work)?;
            work.step(1)?;
            return Ok(flags);
        }
        let (mut positive, mut negative) = (None, None);
        if other.schema < self.schema {
            reduce_kahan_in_place_with(
                &mut self.positive_spans,
                &mut self.positive_buckets,
                &mut correction.positive_buckets,
                self.schema,
                other.schema,
                work,
            )?;
            reduce_kahan_in_place_with(
                &mut self.negative_spans,
                &mut self.negative_buckets,
                &mut correction.negative_buckets,
                self.schema,
                other.schema,
                work,
            )?;
            self.schema = other.schema;
        } else if other.schema > self.schema {
            if other
                .positive_buckets
                .is_some_and(|values| !values.is_empty())
            {
                positive = Some(reduce_kahan_copy_with(
                    other.positive_spans,
                    other.positive_buckets,
                    other.schema,
                    self.schema,
                    context,
                    work,
                )?);
            }
            if other
                .negative_buckets
                .is_some_and(|values| !values.is_empty())
            {
                negative = Some(reduce_kahan_copy_with(
                    other.negative_spans,
                    other.negative_buckets,
                    other.schema,
                    self.schema,
                    context,
                    work,
                )?);
            }
        }
        let (pos_spans, pos_buckets, pos_comp) = positive.as_ref().map_or(
            (other.positive_spans, other.positive_buckets, None),
            |layout| {
                (
                    layout.spans.as_deref(),
                    layout.buckets.as_deref(),
                    layout.compensation.as_deref(),
                )
            },
        );
        let (neg_spans, neg_buckets, neg_comp) = negative.as_ref().map_or(
            (other.negative_spans, other.negative_buckets, None),
            |layout| {
                (
                    layout.spans.as_deref(),
                    layout.buckets.as_deref(),
                    layout.compensation.as_deref(),
                )
            },
        );
        merge_buckets(
            self.schema,
            self.zero_threshold,
            false,
            &mut self.positive_spans,
            &mut self.positive_buckets,
            Some(&mut correction.positive_buckets),
            pos_spans,
            pos_buckets,
            pos_comp,
            context,
            work,
        )?;
        merge_buckets(
            self.schema,
            self.zero_threshold,
            false,
            &mut self.negative_spans,
            &mut self.negative_buckets,
            Some(&mut correction.negative_buckets),
            neg_spans,
            neg_buckets,
            neg_comp,
            context,
            work,
        )?;
        correction.schema = self.schema;
        correction.zero_threshold = self.zero_threshold;
        clone_spans_with(&mut correction.positive_spans, &self.positive_spans, work)?;
        clone_spans_with(&mut correction.negative_spans, &self.negative_spans, work)?;
        work.step(1)?;
        Ok(flags)
    }
}

fn zeros_with(
    len: usize,
    context: Option<&AllocationContext>,
    work: HistogramWork<'_>,
) -> Result<AccountedVec<f64>, Interrupted> {
    work.step(1)?;
    let mut result = AccountedVec::with_capacity_in(len, context);
    while result.len() < len {
        let end = result.len() + (len - result.len()).min(1024);
        work.step(end - result.len())?;
        result.resize(end, 0.0);
    }
    work.step(1)?;
    Ok(result)
}

fn clone_spans_with(
    destination: &mut Option<AccountedVec<Span>>,
    source: &Option<AccountedVec<Span>>,
    work: HistogramWork<'_>,
) -> Result<(), Interrupted> {
    work.step(1)?;
    if let Some(source) = source {
        let destination = destination.get_or_insert_with(|| {
            AccountedVec::with_capacity_in(source.len(), source.context().as_ref())
        });
        destination.clear();
        destination.reserve(source.len());
        for chunk in source.chunks(1024) {
            work.step(chunk.len())?;
            destination.extend_from_slice(chunk);
        }
    } else {
        *destination = None;
    }
    work.step(1)
}

fn accumulate(bucket: &mut f64, correction: Option<&mut f64>, increment: f64, increment_comp: f64) {
    if let Some(correction) = correction {
        (*bucket, *correction) = compensated_add(increment, *bucket, *correction);
        if increment_comp != 0.0 {
            (*bucket, *correction) = compensated_add(increment_comp, *bucket, *correction);
        }
    } else {
        *bucket = source_add(increment, *bucket);
    }
}

#[allow(clippy::too_many_arguments)]
fn merge_buckets(
    schema: i32,
    threshold: f64,
    negative: bool,
    spans: &mut Option<AccountedVec<Span>>,
    buckets: &mut Option<AccountedVec<f64>>,
    mut correction: Option<&mut Option<AccountedVec<f64>>>,
    other_spans: Option<&[Span]>,
    other_buckets: Option<&[f64]>,
    other_correction: Option<&[f64]>,
    context: Option<&AllocationContext>,
    work: HistogramWork<'_>,
) -> Result<(), Interrupted> {
    work.step(1)?;
    let (mut new_spans, mut new_buckets, mut new_correction) = (
        AccountedVec::new_in(context),
        AccountedVec::new_in(context),
        AccountedVec::new_in(context),
    );
    let spans_a = spans.as_mut().unwrap_or(&mut new_spans);
    let buckets_a = buckets.as_mut().unwrap_or(&mut new_buckets);
    let paired = correction.is_some();
    let comp_a = correction
        .as_mut()
        .and_then(|value| value.as_mut())
        .unwrap_or(&mut new_correction);
    let buckets_b = other_buckets.unwrap_or(&[]);
    let (mut span_a, mut bucket_a, mut in_span) = (-1isize, -1isize, 0i32);
    let (mut index_a, mut index_b, mut bucket_b, mut lower_than_threshold) =
        (0i32, 0i32, 0usize, true);
    for span_b in other_spans.unwrap_or(&[]) {
        work.step(1)?;
        index_b = index_b.wrapping_add(span_b.offset);
        for _ in 0..span_b.length {
            work.step(1)?;
            'process: {
                if lower_than_threshold
                    && (-4..=8).contains(&schema)
                    && bucket_bound(index_b, schema, None) <= threshold
                {
                    break 'process;
                }
                lower_than_threshold = false;
                let mut value = buckets_b[bucket_b];
                let mut comp = other_correction.map_or(0.0, |values| values[bucket_b]);
                if negative {
                    value = source_neg(value);
                    comp = source_neg(comp);
                }
                if span_a == -1 {
                    if spans_a.is_empty() || spans_a[0].offset > index_b {
                        buckets_a.insert_copy_with(0, value, |units| work.step(units))?;
                        if paired {
                            comp_a.insert_copy_with(0, comp, |units| work.step(units))?;
                        }
                        if !spans_a.is_empty() && spans_a[0].offset == index_b.wrapping_add(1) {
                            spans_a[0].length = spans_a[0].length.wrapping_add(1);
                            spans_a[0].offset = spans_a[0].offset.wrapping_sub(1);
                            break 'process;
                        }
                        spans_a.insert_copy_with(
                            0,
                            Span {
                                offset: index_b,
                                length: 1,
                            },
                            |units| work.step(units),
                        )?;
                        if spans_a.len() > 1 {
                            spans_a[1].offset =
                                spans_a[1].offset.wrapping_sub(index_b.wrapping_add(1));
                        }
                        break 'process;
                    } else if spans_a[0].offset == index_b {
                        accumulate(
                            &mut buckets_a[0],
                            if paired { Some(&mut comp_a[0]) } else { None },
                            value,
                            comp,
                        );
                        break 'process;
                    }
                    (span_a, bucket_a, in_span) = (0, 0, 0);
                    index_a = spans_a[0].offset;
                }
                let mut delta = index_b.wrapping_sub(index_a);
                loop {
                    work.step(1)?;
                    let remaining = (spans_a[span_a as usize].length as i32).wrapping_sub(in_span);
                    if delta < remaining {
                        bucket_a = bucket_a.wrapping_add(delta as isize);
                        in_span = in_span.wrapping_add(delta);
                        accumulate(
                            &mut buckets_a[bucket_a as usize],
                            if paired {
                                Some(&mut comp_a[bucket_a as usize])
                            } else {
                                None
                            },
                            value,
                            comp,
                        );
                        break;
                    }
                    delta = delta.wrapping_sub(remaining);
                    bucket_a = bucket_a.wrapping_add(remaining as isize);
                    span_a += 1;
                    if span_a as usize == spans_a.len() || delta < spans_a[span_a as usize].offset {
                        buckets_a
                            .insert_copy_with(bucket_a as usize, value, |units| work.step(units))?;
                        if paired {
                            comp_a.insert_copy_with(bucket_a as usize, comp, |units| {
                                work.step(units)
                            })?;
                        }
                        if delta == 0 {
                            if (span_a as usize) < spans_a.len() {
                                spans_a[span_a as usize].offset =
                                    spans_a[span_a as usize].offset.wrapping_sub(1);
                            }
                            span_a -= 1;
                            in_span = spans_a[span_a as usize].length as i32;
                            spans_a[span_a as usize].length =
                                spans_a[span_a as usize].length.wrapping_add(1);
                        } else if (span_a as usize) < spans_a.len()
                            && delta == spans_a[span_a as usize].offset.wrapping_sub(1)
                        {
                            in_span = 0;
                            spans_a[span_a as usize].offset =
                                spans_a[span_a as usize].offset.wrapping_sub(1);
                            spans_a[span_a as usize].length =
                                spans_a[span_a as usize].length.wrapping_add(1);
                        } else {
                            in_span = 0;
                            if (span_a as usize) < spans_a.len() {
                                spans_a[span_a as usize].offset = spans_a[span_a as usize]
                                    .offset
                                    .wrapping_sub(delta.wrapping_add(1));
                            }
                            spans_a.insert_copy_with(
                                span_a as usize,
                                Span {
                                    offset: delta,
                                    length: 1,
                                },
                                |units| work.step(units),
                            )?;
                        }
                        break 'process;
                    } else {
                        delta = delta.wrapping_sub(spans_a[span_a as usize].offset);
                        in_span = 0;
                    }
                }
            }
            index_a = index_b;
            index_b = index_b.wrapping_add(1);
            bucket_b += 1;
        }
    }
    if spans.is_none() && !new_spans.is_empty() {
        *spans = Some(new_spans);
    }
    if buckets.is_none() && !new_buckets.is_empty() {
        *buckets = Some(new_buckets);
    }
    if let Some(correction) = correction
        && correction.is_none()
        && !new_correction.is_empty()
    {
        *correction = Some(new_correction);
    }
    work.step(1)?;
    Ok(())
}

fn intersect_bounds(
    left: Option<&[f64]>,
    right: Option<&[f64]>,
    context: Option<&AllocationContext>,
    work: HistogramWork<'_>,
) -> Result<Option<HistogramBounds>, Interrupted> {
    work.step(1)?;
    let (left, right) = (left.unwrap_or(&[]), right.unwrap_or(&[]));
    let (mut i, mut j, mut result) = (0, 0, None::<AccountedVec<f64>>);
    while i < left.len() && j < right.len() {
        work.step(1)?;
        if left[i] == right[j] {
            result
                .get_or_insert_with(|| {
                    AccountedVec::with_capacity_in(left.len().min(right.len()), context)
                })
                .push(left[i]);
            i += 1;
            j += 1;
        } else if left[i] < right[j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    if let Some(values) = result {
        work.step(values.len())?;
        let result = HistogramBounds::from_owned(values);
        work.step(1)?;
        Ok(Some(result))
    } else {
        Ok(None)
    }
}

#[allow(clippy::too_many_arguments)]
fn map_custom(
    spans: &mut Option<AccountedVec<Span>>,
    buckets: &mut Option<AccountedVec<f64>>,
    bounds: Option<&[f64]>,
    other: FloatHistogramRef<'_>,
    correction: Option<&mut Option<AccountedVec<f64>>>,
    intersected: Option<&[f64]>,
    negative: bool,
    context: Option<&AllocationContext>,
    work: HistogramWork<'_>,
) -> Result<(), Interrupted> {
    let intersected = intersected.unwrap_or(&[]);
    let mut target = zeros_with(intersected.len() + 1, context, work)?;
    let mut comp_target = zeros_with(intersected.len() + 1, context, work)?;
    let comp_buckets = correction.as_deref().and_then(|values| values.as_deref());
    for (source_spans, source_buckets, bounds, subtract, with_comp) in [
        (spans.as_deref(), buckets.as_deref(), bounds, false, true),
        (
            other.positive_spans,
            other.positive_buckets,
            other.custom_values,
            negative,
            false,
        ),
    ] {
        work.step(1)?;
        let (source_spans, source_buckets, bounds) = (
            source_spans.unwrap_or(&[]),
            source_buckets.unwrap_or(&[]),
            bounds.unwrap_or(&[]),
        );
        let (mut source_index, mut bucket_index, mut intersect_index) = (0i64, 0, 0);
        for span in source_spans {
            work.step(1)?;
            source_index = source_index.wrapping_add(i64::from(span.offset));
            for _ in 0..span.length {
                work.step(1)?;
                if bucket_index < source_buckets.len() {
                    let mut target_index = target.len() - 1;
                    if source_index < bounds.len() as i64 {
                        let bound = bounds[source_index as usize];
                        while intersect_index < intersected.len() {
                            work.step(1)?;
                            if intersected[intersect_index] >= bound {
                                target_index = intersect_index;
                                break;
                            }
                            intersect_index += 1;
                        }
                    }
                    let value = source_buckets[bucket_index];
                    if subtract {
                        (target[target_index], comp_target[target_index]) =
                            compensated_sub(value, target[target_index], comp_target[target_index]);
                    } else {
                        (target[target_index], comp_target[target_index]) =
                            compensated_add(value, target[target_index], comp_target[target_index]);
                        if with_comp && let Some(comp) = comp_buckets {
                            (target[target_index], comp_target[target_index]) = compensated_add(
                                comp[bucket_index],
                                target[target_index],
                                comp_target[target_index],
                            );
                        }
                    }
                }
                source_index = source_index.wrapping_add(1);
                bucket_index += 1;
            }
        }
    }
    let mut new_spans = AccountedVec::new_in(context);
    let dest_spans = spans.as_mut().unwrap_or(&mut new_spans);
    dest_spans.clear();
    let (mut written, mut last_index) = (0, -1i32);
    for index in 0..target.len() {
        work.step(1)?;
        if target[index] == 0.0 && comp_target[index] == 0.0 {
            continue;
        }
        target[written] = target[index];
        comp_target[written] = comp_target[index];
        written += 1;
        let index = index as i32;
        if !dest_spans.is_empty() && index == last_index.wrapping_add(1) {
            let span = dest_spans.last_mut().unwrap();
            span.length = span.length.wrapping_add(1);
        } else {
            dest_spans.push_copy_with(
                Span {
                    offset: if dest_spans.is_empty() {
                        index
                    } else {
                        index.wrapping_sub(last_index).wrapping_sub(1)
                    },
                    length: 1,
                },
                |units| work.step(units),
            )?;
        }
        last_index = index;
    }
    target.truncate(written);
    comp_target.truncate(written);
    if spans.is_none() && !new_spans.is_empty() {
        *spans = Some(new_spans);
    }
    *buckets = Some(target);
    if let Some(correction) = correction {
        *correction = Some(comp_target);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        storage::Control,
        test_storage::interrupt_during,
    };

    #[test]
    fn sparse_prefix_and_zero_run_suffix_moves_interrupt_in_bounded_chunks() {
        for compact in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let context = AllocationContext::new(&account, MemoryKind::Workspace);
            let mut buckets = AccountedVec::with_capacity_in(32_768, Some(&context));
            buckets.extend_from_slice(&vec![1.0; 16_384]);
            if compact {
                buckets[1] = 0.0;
            }
            let mut histogram = FloatHistogram {
                count: 16_384.0,
                sum: 16_384.0,
                positive_spans: Some(AccountedVec::from_slice_in(
                    &[Span {
                        offset: 100,
                        length: 16_384,
                    }],
                    Some(&context),
                )),
                positive_buckets: Some(buckets),
                ..FloatHistogram::default()
            };
            let source = FloatHistogram {
                count: 1.0,
                sum: 1.0,
                positive_spans: Some(
                    vec![Span {
                        offset: 99,
                        length: 1,
                    }]
                    .into(),
                ),
                positive_buckets: Some(vec![1.0].into()),
                ..FloatHistogram::default()
            };
            let before = account.snapshot().work;
            let result = interrupt_during(&account, &control, || {
                if compact {
                    compact_float_pair_with(
                        &mut histogram.positive_buckets,
                        &mut histogram.positive_spans,
                        None,
                        0,
                        Some(&context),
                        |units| {
                            assert!(units <= 1024);
                            HistogramWork::new(&control, &account).step(units)
                        },
                    )
                    .map_err(HistogramEvaluationError::from)
                } else {
                    histogram
                        .add_or_sub_with(
                            source.as_ref(),
                            false,
                            Some(&context),
                            HistogramWork::new(&control, &account),
                        )
                        .map(|_| ())
                }
            });
            assert!(matches!(
                result,
                Err(HistogramEvaluationError::Interrupted(
                    Interrupted::Cancelled
                ))
            ));
            assert!(account.snapshot().work - before <= 8192 + 1024);
            drop(histogram);
            drop(context);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn same_schema_accumulation_reuses_buffers_and_outlives_the_borrowed_operand() {
        let mut histogram = FloatHistogram {
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: 2,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![1.0, 2.0].into()),
            ..FloatHistogram::default()
        };
        let spans = histogram.positive_spans.as_ref().unwrap().as_ptr();
        let buckets = histogram.positive_buckets.as_ref().unwrap().as_ptr();
        {
            let source = FloatHistogram {
                positive_spans: Some(
                    vec![Span {
                        offset: 0,
                        length: 2,
                    }]
                    .into(),
                ),
                positive_buckets: Some(vec![3.0, 4.0].into()),
                ..FloatHistogram::default()
            };
            histogram.add(source.as_ref()).unwrap();
            assert_eq!(source.positive_buckets.as_deref(), Some(&[3.0, 4.0][..]));
        }
        assert_eq!(histogram.positive_buckets.as_deref(), Some(&[4.0, 6.0][..]));
        assert_eq!(histogram.positive_spans.as_ref().unwrap().as_ptr(), spans);
        assert_eq!(
            histogram.positive_buckets.as_ref().unwrap().as_ptr(),
            buckets
        );
    }

    #[test]
    fn paired_compaction_reuses_capacity_and_preserves_nonzero_corrections() {
        let spans = vec![
            Span {
                offset: 0,
                length: 2,
            },
            Span {
                offset: 2,
                length: 1,
            },
        ];
        let mut main = Vec::with_capacity(8);
        main.extend_from_slice(&[3.0, 0.0, 4.0]);
        let mut comp = Vec::with_capacity(8);
        comp.extend_from_slice(&[0.0, 0.5, 0.0]);
        let main_ptr = main.as_ptr();
        let comp_ptr = comp.as_ptr();
        let mut histogram = FloatHistogram {
            positive_spans: Some(spans.clone().into()),
            positive_buckets: Some(main.into()),
            ..FloatHistogram::default()
        };
        let mut correction = FloatHistogram {
            positive_spans: Some(spans.clone().into()),
            positive_buckets: Some(comp.into()),
            ..FloatHistogram::default()
        };
        histogram.kahan_compact(2, &mut correction);
        assert_eq!(
            histogram.positive_spans.as_deref(),
            Some(
                &[Span {
                    offset: 0,
                    length: 5
                }][..]
            )
        );
        assert_eq!(
            histogram.positive_buckets.as_deref(),
            Some(&[3.0, 0.0, 0.0, 0.0, 4.0][..])
        );
        assert_eq!(
            correction.positive_buckets.as_deref(),
            Some(&[0.0, 0.5, 0.0, 0.0, 0.0][..])
        );
        assert_eq!(correction.positive_spans.as_deref(), Some(spans.as_slice()));
        assert_eq!(
            histogram.positive_buckets.as_ref().unwrap().as_ptr(),
            main_ptr
        );
        assert_eq!(
            correction.positive_buckets.as_ref().unwrap().as_ptr(),
            comp_ptr
        );
    }
}
