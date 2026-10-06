// SPDX-License-Identifier: GPL-3.0-or-later
// Data equality and overflow scans follow the pinned Prometheus model; see NOTICE.
use crate::{
    histogram::{CUSTOM_BUCKETS_SCHEMA, FloatHistogram, FloatHistogramRef, Span},
    histogram_counter::custom_bounds_match_with,
    histogram_work::HistogramWork,
    storage::Interrupted,
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
        self.equals_with(other, HistogramWork::default())
            .expect("inert histogram work")
    }

    pub(crate) fn equals_with(
        self,
        other: Self,
        work: HistogramWork<'_>,
    ) -> Result<bool, Interrupted> {
        work.step(1)?;
        if self.schema != other.schema
            || self.count.to_bits() != other.count.to_bits()
            || self.sum.to_bits() != other.sum.to_bits()
        {
            return Ok(false);
        }
        if self.schema == CUSTOM_BUCKETS_SCHEMA
            && !custom_bounds_match_with(self.custom_values, other.custom_values, work)?
        {
            return Ok(false);
        }
        if self.zero_threshold != other.zero_threshold
            || self.zero_count.to_bits() != other.zero_count.to_bits()
        {
            return Ok(false);
        }
        Ok(
            spans_match(self.negative_spans, other.negative_spans, work)?
                && float_buckets_match(self.negative_buckets, other.negative_buckets, work)?
                && spans_match(self.positive_spans, other.positive_spans, work)?
                && float_buckets_match(self.positive_buckets, other.positive_buckets, work)?,
        )
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

fn float_buckets_match(
    left: Option<&[f64]>,
    right: Option<&[f64]>,
    work: HistogramWork<'_>,
) -> Result<bool, Interrupted> {
    let (left, right) = (left.unwrap_or(&[]), right.unwrap_or(&[]));
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.iter().zip(right) {
        work.step(1)?;
        if left.to_bits() != right.to_bits() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn spans_match(
    left: Option<&[Span]>,
    right: Option<&[Span]>,
    work: HistogramWork<'_>,
) -> Result<bool, Interrupted> {
    let (left, right) = (left.unwrap_or(&[]), right.unwrap_or(&[]));
    let (mut left_index, mut right_index) = (0, 0);
    loop {
        if left_index >= left.len() {
            return trailing_empty(&right[right_index..], work);
        }
        if right_index >= right.len() {
            return trailing_empty(&left[left_index..], work);
        }
        let left_span = next_span(left, &mut left_index, work)?;
        let right_span = next_span(right, &mut right_index, work)?;
        if left_span.length == 0 && right_span.length == 0 {
            return Ok(true);
        }
        if left_span != right_span {
            return Ok(false);
        }
    }
}

fn trailing_empty(spans: &[Span], work: HistogramWork<'_>) -> Result<bool, Interrupted> {
    for span in spans {
        work.step(1)?;
        if span.length != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn next_span(
    spans: &[Span],
    index: &mut usize,
    work: HistogramWork<'_>,
) -> Result<Span, Interrupted> {
    work.step(1)?;
    let mut current = spans[*index];
    *index += 1;
    if current.length == 0 {
        while *index < spans.len() && spans[*index].length == 0 {
            work.step(1)?;
            current.offset = current.offset.wrapping_add(spans[*index].offset);
            *index += 1;
        }
        if *index < spans.len() {
            work.step(1)?;
            current.offset = current.offset.wrapping_add(spans[*index].offset);
            current.length = spans[*index].length;
            *index += 1;
        }
    }
    Ok(current)
}
