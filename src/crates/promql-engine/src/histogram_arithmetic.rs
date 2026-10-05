// SPDX-License-Identifier: GPL-3.0-or-later
// Arithmetic derives from pinned Prometheus model/histogram; see NOTICE.
use crate::accounting::{AccountedVec, AllocationContext};
use crate::histogram::{
    CUSTOM_BUCKETS_SCHEMA, FloatHistogram, FloatHistogramRef, GAUGE, HistogramBounds,
    HistogramError, Span, UNKNOWN_RESET, bucket_bound, custom_bounds_match,
};
use crate::histogram_layout::{
    compact_float_pair, reduce_float_copy, reduce_float_in_place, reduce_kahan_copy,
    reduce_kahan_in_place,
};
use crate::numerical::{compensated_add, compensated_sub};

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
    pub fn kahan_compact(&mut self, max_empty: isize, correction: &mut Self) {
        self.kahan_compact_in(max_empty, correction, None);
    }
    fn kahan_compact_in(
        &mut self,
        max_empty: isize,
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
    ) {
        for positive in [true, false] {
            let mut count = 0;
            // Bounds are independent of counts; scan the borrowed prefix before zeroing it.
            for bucket in self.bucket_iterator(positive, 0.0, self.schema) {
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
            self.kahan_compact_in(0, correction, context);
        } else {
            self.compact_in(0, context);
        }
    }

    fn reconcile_zero(
        &mut self,
        other: FloatHistogramRef<'_>,
        mut correction: Option<&mut Self>,
        context: Option<&AllocationContext>,
    ) -> (f64, f64) {
        let (mut count, mut threshold, mut comp) = (other.zero_count, other.zero_threshold, 0.0);
        while threshold != self.zero_threshold {
            if self.zero_threshold > threshold {
                let zero = other.zero_count_for_threshold(self.zero_threshold, None);
                (count, threshold, comp) = (zero.count, zero.threshold, zero.compensation);
            }
            if threshold > self.zero_threshold {
                let zero = self
                    .zero_count_for_threshold(threshold, correction.as_deref().map(Self::as_ref));
                self.zero_count = zero.count;
                self.zero_threshold = zero.threshold;
                if let Some(correction) = correction.as_mut() {
                    correction.zero_count = zero.compensation;
                }
                self.trim_zero(correction.as_deref_mut(), context);
            }
        }
        (count, comp)
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

    pub(crate) fn add_in(
        &mut self,
        other: FloatHistogramRef<'_>,
        context: Option<&AllocationContext>,
    ) -> Result<HistogramArithmetic, HistogramError> {
        self.add_or_sub(other, false, context)
    }

    fn add_or_sub(
        &mut self,
        other: FloatHistogramRef<'_>,
        negative: bool,
        context: Option<&AllocationContext>,
    ) -> Result<HistogramArithmetic, HistogramError> {
        if self.uses_custom_buckets() != (other.schema == CUSTOM_BUCKETS_SCHEMA) {
            return Err(HistogramError::IncompatibleSchema);
        }
        let mut flags = HistogramArithmetic {
            counter_reset_collision: self.adjust_reset(other.reset_hint),
            nhcb_bounds_reconciled: false,
        };
        if !self.uses_custom_buckets() {
            let (zero, _) = self.reconcile_zero(other, None, context);
            if negative {
                self.zero_count -= zero;
            } else {
                self.zero_count += zero;
            }
        }
        if negative {
            self.count -= other.count;
            self.sum -= other.sum;
        } else {
            self.count += other.count;
            self.sum += other.sum;
        }
        if self.uses_custom_buckets() {
            if custom_bounds_match(self.custom_values.as_deref(), other.custom_values) {
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
                );
            } else {
                flags.nhcb_bounds_reconciled = true;
                let bounds =
                    intersect_bounds(self.custom_values.as_deref(), other.custom_values, context);
                map_custom(
                    &mut self.positive_spans,
                    &mut self.positive_buckets,
                    self.custom_values.as_deref(),
                    other,
                    None,
                    bounds.as_deref(),
                    negative,
                    context,
                );
                self.custom_values = bounds;
            }
            return Ok(flags);
        }
        let (mut positive, mut negative_layout) = (None, None);
        if other.schema < self.schema {
            if let Err(error) = reduce_float_in_place(
                &mut self.positive_spans,
                &mut self.positive_buckets,
                self.schema,
                other.schema,
            ) {
                panic!("{error}");
            }
            if let Err(error) = reduce_float_in_place(
                &mut self.negative_spans,
                &mut self.negative_buckets,
                self.schema,
                other.schema,
            ) {
                panic!("{error}");
            }
            self.schema = other.schema;
        } else if other.schema > self.schema {
            positive = Some(reduce_float_copy(
                other.positive_spans,
                other.positive_buckets,
                other.schema,
                self.schema,
                context,
            ));
            negative_layout = Some(reduce_float_copy(
                other.negative_spans,
                other.negative_buckets,
                other.schema,
                self.schema,
                context,
            ));
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
        );
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
        );
        Ok(flags)
    }

    fn new_correction(&self, context: Option<&AllocationContext>) -> Self {
        Self {
            reset_hint: self.reset_hint,
            schema: self.schema,
            zero_threshold: self.zero_threshold,
            positive_spans: self.positive_spans.clone(),
            negative_spans: self.negative_spans.clone(),
            positive_buckets: Some(AccountedVec::filled_in(
                self.positive_buckets.as_ref().map_or(0, |v| v.len()),
                0.0,
                context,
            )),
            negative_buckets: if self.uses_custom_buckets() {
                None
            } else {
                Some(AccountedVec::filled_in(
                    self.negative_buckets.as_ref().map_or(0, |v| v.len()),
                    0.0,
                    context,
                ))
            },
            custom_values: self.custom_values.clone(),
            ..Self::default()
        }
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
        if self.uses_custom_buckets() != (other.schema == CUSTOM_BUCKETS_SCHEMA) {
            return Err(HistogramError::IncompatibleSchema);
        }
        let mut flags = HistogramArithmetic {
            counter_reset_collision: self.adjust_reset(other.reset_hint),
            nhcb_bounds_reconciled: false,
        };
        let correction = correction.get_or_insert_with(|| self.new_correction(context));
        if !self.uses_custom_buckets() {
            let (zero, comp) = self.reconcile_zero(other, Some(correction), context);
            (self.zero_count, correction.zero_count) =
                compensated_add(zero, self.zero_count, correction.zero_count);
            (self.zero_count, correction.zero_count) =
                compensated_add(comp, self.zero_count, correction.zero_count);
        }
        (self.count, correction.count) = compensated_add(other.count, self.count, correction.count);
        (self.sum, correction.sum) = compensated_add(other.sum, self.sum, correction.sum);
        if self.uses_custom_buckets() {
            if custom_bounds_match(self.custom_values.as_deref(), other.custom_values) {
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
                );
            } else {
                flags.nhcb_bounds_reconciled = true;
                let bounds =
                    intersect_bounds(self.custom_values.as_deref(), other.custom_values, context);
                map_custom(
                    &mut self.positive_spans,
                    &mut self.positive_buckets,
                    self.custom_values.as_deref(),
                    other,
                    Some(&mut correction.positive_buckets),
                    bounds.as_deref(),
                    false,
                    context,
                );
                self.custom_values = bounds;
                correction.custom_values = self.custom_values.clone();
            }
            correction.positive_spans.clone_from(&self.positive_spans);
            return Ok(flags);
        }
        let (mut positive, mut negative) = (None, None);
        if other.schema < self.schema {
            reduce_kahan_in_place(
                &mut self.positive_spans,
                &mut self.positive_buckets,
                &mut correction.positive_buckets,
                self.schema,
                other.schema,
            );
            reduce_kahan_in_place(
                &mut self.negative_spans,
                &mut self.negative_buckets,
                &mut correction.negative_buckets,
                self.schema,
                other.schema,
            );
            self.schema = other.schema;
        } else if other.schema > self.schema {
            if other
                .positive_buckets
                .is_some_and(|values| !values.is_empty())
            {
                positive = Some(reduce_kahan_copy(
                    other.positive_spans,
                    other.positive_buckets,
                    other.schema,
                    self.schema,
                    context,
                ));
            }
            if other
                .negative_buckets
                .is_some_and(|values| !values.is_empty())
            {
                negative = Some(reduce_kahan_copy(
                    other.negative_spans,
                    other.negative_buckets,
                    other.schema,
                    self.schema,
                    context,
                ));
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
        );
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
        );
        correction.schema = self.schema;
        correction.zero_threshold = self.zero_threshold;
        correction.positive_spans.clone_from(&self.positive_spans);
        correction.negative_spans.clone_from(&self.negative_spans);
        Ok(flags)
    }
}

fn accumulate(bucket: &mut f64, correction: Option<&mut f64>, increment: f64, increment_comp: f64) {
    if let Some(correction) = correction {
        (*bucket, *correction) = compensated_add(increment, *bucket, *correction);
        if increment_comp != 0.0 {
            (*bucket, *correction) = compensated_add(increment_comp, *bucket, *correction);
        }
    } else {
        *bucket += increment;
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
) {
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
        index_b = index_b.wrapping_add(span_b.offset);
        for _ in 0..span_b.length {
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
                    value *= -1.0;
                    comp *= -1.0;
                }
                if span_a == -1 {
                    if spans_a.is_empty() || spans_a[0].offset > index_b {
                        buckets_a.insert(0, value);
                        if paired {
                            comp_a.insert(0, comp);
                        }
                        if !spans_a.is_empty() && spans_a[0].offset == index_b.wrapping_add(1) {
                            spans_a[0].length = spans_a[0].length.wrapping_add(1);
                            spans_a[0].offset = spans_a[0].offset.wrapping_sub(1);
                            break 'process;
                        }
                        spans_a.insert(
                            0,
                            Span {
                                offset: index_b,
                                length: 1,
                            },
                        );
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
                        buckets_a.insert(bucket_a as usize, value);
                        if paired {
                            comp_a.insert(bucket_a as usize, comp);
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
                            spans_a.insert(
                                span_a as usize,
                                Span {
                                    offset: delta,
                                    length: 1,
                                },
                            );
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
}

fn intersect_bounds(
    left: Option<&[f64]>,
    right: Option<&[f64]>,
    context: Option<&AllocationContext>,
) -> Option<HistogramBounds> {
    let (left, right) = (left.unwrap_or(&[]), right.unwrap_or(&[]));
    let (mut i, mut j, mut result) = (0, 0, None::<AccountedVec<f64>>);
    while i < left.len() && j < right.len() {
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
    result.map(|values| HistogramBounds::from_slice_in(&values, context))
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
) {
    let intersected = intersected.unwrap_or(&[]);
    let mut target = AccountedVec::filled_in(intersected.len() + 1, 0.0, context);
    let mut comp_target = AccountedVec::filled_in(intersected.len() + 1, 0.0, context);
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
        let (source_spans, source_buckets, bounds) = (
            source_spans.unwrap_or(&[]),
            source_buckets.unwrap_or(&[]),
            bounds.unwrap_or(&[]),
        );
        let (mut source_index, mut bucket_index, mut intersect_index) = (0isize, 0, 0);
        for span in source_spans {
            source_index = source_index.wrapping_add(span.offset as isize);
            for _ in 0..span.length {
                if bucket_index < source_buckets.len() {
                    let mut target_index = target.len() - 1;
                    if source_index < bounds.len() as isize {
                        let bound = bounds[source_index as usize];
                        while intersect_index < intersected.len() {
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
            dest_spans.push(Span {
                offset: if dest_spans.is_empty() {
                    index
                } else {
                    index.wrapping_sub(last_index).wrapping_sub(1)
                },
                length: 1,
            });
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
