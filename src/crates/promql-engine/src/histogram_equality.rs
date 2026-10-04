// SPDX-License-Identifier: GPL-3.0-or-later
// Data equality and overflow scans follow the pinned Prometheus model; see NOTICE.
use crate::histogram::{
    CUSTOM_BUCKETS_SCHEMA, FloatHistogram, FloatHistogramRef, Span, custom_bounds_match,
};

impl FloatHistogram {
    pub fn equals(&self, other: FloatHistogramRef<'_>) -> bool {
        self.as_ref().equals(other)
    }

    pub fn has_overflow(&self) -> bool {
        self.as_ref().has_overflow()
    }
}

impl FloatHistogramRef<'_> {
    pub fn equals(self, other: Self) -> bool {
        if self.schema != other.schema
            || self.count.to_bits() != other.count.to_bits()
            || self.sum.to_bits() != other.sum.to_bits()
        {
            return false;
        }
        if self.schema == CUSTOM_BUCKETS_SCHEMA
            && !custom_bounds_match(self.custom_values, other.custom_values)
        {
            return false;
        }
        if self.zero_threshold != other.zero_threshold
            || self.zero_count.to_bits() != other.zero_count.to_bits()
        {
            return false;
        }
        spans_match(self.negative_spans, other.negative_spans)
            && float_buckets_match(self.negative_buckets, other.negative_buckets)
            && spans_match(self.positive_spans, other.positive_spans)
            && float_buckets_match(self.positive_buckets, other.positive_buckets)
    }

    pub fn has_overflow(self) -> bool {
        if self.zero_count.is_infinite() || self.count.is_infinite() || self.sum.is_infinite() {
            return true;
        }
        for values in [
            self.positive_buckets,
            self.negative_buckets,
            self.custom_values,
        ] {
            for value in values.unwrap_or(&[]) {
                if value.is_infinite() {
                    return true;
                }
            }
        }
        false
    }
}

fn float_buckets_match(left: Option<&[f64]>, right: Option<&[f64]>) -> bool {
    let (left, right) = (left.unwrap_or(&[]), right.unwrap_or(&[]));
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.to_bits() == right.to_bits())
}

fn spans_match(left: Option<&[Span]>, right: Option<&[Span]>) -> bool {
    let (left, right) = (left.unwrap_or(&[]), right.unwrap_or(&[]));
    let (mut left_index, mut right_index) = (0, 0);
    loop {
        if left_index >= left.len() {
            return right[right_index..].iter().all(|span| span.length == 0);
        }
        if right_index >= right.len() {
            return left[left_index..].iter().all(|span| span.length == 0);
        }
        let left_span = next_span(left, &mut left_index);
        let right_span = next_span(right, &mut right_index);
        if left_span.length == 0 && right_span.length == 0 {
            return true;
        }
        if left_span != right_span {
            return false;
        }
    }
}

fn next_span(spans: &[Span], index: &mut usize) -> Span {
    let mut current = spans[*index];
    *index += 1;
    if current.length == 0 {
        while *index < spans.len() && spans[*index].length == 0 {
            current.offset = current.offset.wrapping_add(spans[*index].offset);
            *index += 1;
        }
        if *index < spans.len() {
            current.offset = current.offset.wrapping_add(spans[*index].offset);
            current.length = spans[*index].length;
            *index += 1;
        }
    }
    current
}
