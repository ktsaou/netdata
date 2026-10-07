// SPDX-License-Identifier: GPL-3.0-or-later
// Borrowed bucket traversal derives from the pinned Prometheus model; see NOTICE.
use crate::histogram::{CUSTOM_BUCKETS_SCHEMA, FloatHistogramRef, Span, bucket_bound};
use crate::histogram_layout::target_index;
use crate::numerical::log2;

#[derive(Clone, Copy, Debug, Default)]
pub struct Bucket {
    pub lower: f64,
    pub upper: f64,
    pub lower_inclusive: bool,
    pub upper_inclusive: bool,
    pub count: f64,
    pub index: i32,
}

impl Bucket {
    pub fn fraction_below(self, value: f64, linear: bool) -> f64 {
        if linear {
            return (value - self.lower) / (self.upper - self.lower);
        }
        let lower = log2(self.lower.abs());
        let upper = log2(self.upper.abs());
        let value_log = log2(value.abs());
        if value > 0.0 {
            (value_log - lower) / (upper - lower)
        } else {
            1.0 - ((value_log - upper) / (lower - upper))
        }
    }
}

#[derive(Clone, Copy)]
struct Layout<'a> {
    schema: i32,
    positive: bool,
    spans: &'a [Span],
    buckets: &'a [f64],
    custom_values: Option<&'a [f64]>,
}

impl<'a> Layout<'a> {
    fn new(histogram: FloatHistogramRef<'a>, positive: bool) -> Self {
        Self {
            schema: histogram.schema,
            positive,
            spans: if positive {
                histogram.positive_spans
            } else {
                histogram.negative_spans
            }
            .unwrap_or(&[]),
            buckets: if positive {
                histogram.positive_buckets
            } else {
                histogram.negative_buckets
            }
            .unwrap_or(&[]),
            custom_values: if positive {
                histogram.custom_values
            } else {
                None
            },
        }
    }

    fn at(self, schema: i32, index: i32, count: f64) -> Bucket {
        let bound = bucket_bound(index, schema, self.custom_values);
        let previous = bucket_bound(index.wrapping_sub(1), schema, self.custom_values);
        let (lower, upper) = if self.positive {
            (previous, bound)
        } else {
            (-bound, -previous)
        };
        Bucket {
            lower,
            upper,
            lower_inclusive: if schema == CUSTOM_BUCKETS_SCHEMA {
                index == 0
            } else {
                lower < 0.0
            },
            upper_inclusive: schema == CUSTOM_BUCKETS_SCHEMA || upper > 0.0,
            count,
            index,
        }
    }
}

/// Traversal borrows one payload and retains only indices and one merged count.
pub struct FloatBucketIterator<'a> {
    layout: Layout<'a>,
    target_schema: i32,
    start: f64,
    bound_reached: bool,
    span_index: usize,
    within_span: u32,
    bucket_index: usize,
    original_index: i32,
    current_index: i32,
    current_count: f64,
}

impl<'a> FloatBucketIterator<'a> {
    pub fn new(histogram: FloatHistogramRef<'a>, positive: bool, start: f64, target: i32) -> Self {
        if histogram.schema == CUSTOM_BUCKETS_SCHEMA && target != histogram.schema {
            panic!("cannot merge from custom buckets schema to exponential schema");
        }
        if histogram.schema != CUSTOM_BUCKETS_SCHEMA && target == CUSTOM_BUCKETS_SCHEMA {
            panic!("cannot merge from exponential buckets schema to custom schema");
        }
        if target > histogram.schema {
            panic!("cannot merge from schema {} to {target}", histogram.schema);
        }
        Self::from_layout(Layout::new(histogram, positive), start, target)
    }

    fn from_layout(layout: Layout<'a>, start: f64, target: i32) -> Self {
        Self {
            layout,
            target_schema: target,
            start,
            bound_reached: start == 0.0,
            span_index: 0,
            within_span: 0,
            bucket_index: 0,
            original_index: 0,
            current_index: 0,
            current_count: 0.0,
        }
    }

    pub fn advance(&mut self) -> bool {
        self.advance_with(crate::histogram_work::HistogramWork::default())
            .expect("inert histogram work")
    }

    pub(crate) fn advance_with(
        &mut self,
        work: crate::histogram_work::HistogramWork<'_>,
    ) -> Result<bool, crate::storage::Interrupted> {
        loop {
            work.step(1)?;
            if !self.advance_unfiltered(work)? {
                return Ok(false);
            }
            // Consume skipped buckets in a loop instead of growing the caller's stack.
            if !self.bound_reached
                && (-4..=8).contains(&self.target_schema)
                && bucket_bound(self.current_index, self.target_schema, None) <= self.start
            {
                continue;
            }
            self.bound_reached = true;
            return Ok(true);
        }
    }

    fn advance_unfiltered(
        &mut self,
        work: crate::histogram_work::HistogramWork<'_>,
    ) -> Result<bool, crate::storage::Interrupted> {
        if self.span_index >= self.layout.spans.len() {
            return Ok(false);
        }
        let mut span = self.layout.spans[self.span_index];
        if self.layout.schema == self.target_schema {
            let Some(position) = self.advance_position_with(self.layout.buckets.len(), work)?
            else {
                return Ok(false);
            };
            self.current_count = self.layout.buckets[position];
            return Ok(true);
        }

        let (mut original_index, mut span_index, mut within_span) =
            (self.original_index, self.span_index, self.within_span);
        let mut first = true;
        self.current_count = 0.0;
        'merge: loop {
            work.step(1)?;
            original_index = if self.bucket_index == 0 {
                span.offset
            } else {
                original_index.wrapping_add(1)
            };
            if self.bucket_index >= self.layout.buckets.len() {
                return Ok(!first);
            }
            while within_span >= span.length {
                work.step(1)?;
                within_span = 0;
                span_index += 1;
                if span_index >= self.layout.spans.len() {
                    return Ok(!first);
                }
                span = self.layout.spans[span_index];
                original_index = original_index.wrapping_add(span.offset);
            }
            let index = target_index(original_index, self.layout.schema, self.target_schema);
            if first {
                self.current_index = index;
                first = false;
            } else if index != self.current_index {
                break 'merge;
            }
            self.current_count += self.layout.buckets[self.bucket_index];
            within_span = within_span.wrapping_add(1);
            self.bucket_index += 1;
            self.original_index = original_index;
            self.span_index = span_index;
            self.within_span = within_span;
        }
        Ok(true)
    }

    fn advance_position_with(
        &mut self,
        bucket_count: usize,
        work: crate::histogram_work::HistogramWork<'_>,
    ) -> Result<Option<usize>, crate::storage::Interrupted> {
        work.step(1)?;
        let Some(mut span) = self.layout.spans.get(self.span_index).copied() else {
            return Ok(None);
        };
        self.current_index = if self.bucket_index == 0 {
            span.offset
        } else {
            self.current_index.wrapping_add(1)
        };
        if self.bucket_index >= bucket_count {
            return Ok(None);
        }
        while self.within_span >= span.length {
            work.step(1)?;
            self.within_span = 0;
            self.span_index += 1;
            let Some(next) = self.layout.spans.get(self.span_index).copied() else {
                return Ok(None);
            };
            span = next;
            self.current_index = self.current_index.wrapping_add(span.offset);
        }
        self.within_span = self.within_span.wrapping_add(1);
        let position = self.bucket_index;
        self.bucket_index += 1;
        Ok(Some(position))
    }

    pub fn at(&self) -> Bucket {
        self.layout
            .at(self.target_schema, self.current_index, self.current_count)
    }

    pub fn current_index(&self) -> i32 {
        self.current_index
    }

    pub fn current_count(&self) -> f64 {
        self.current_count
    }

    pub(crate) fn reverse_productive_prefix(&self) -> ProductiveReverse<'a> {
        assert_eq!(self.target_schema, self.layout.schema);
        assert!(self.bucket_index != 0);
        ProductiveReverse {
            layout: self.layout,
            span_end: self.span_index + 1,
            within_span: self.within_span,
            remaining: self.bucket_index,
            current_index: self.current_index.wrapping_add(1),
            current_count: 0.0,
        }
    }
}

// String reverses successful forward visits, which may end before the supplied spans or counts.
pub(crate) struct ProductiveReverse<'a> {
    layout: Layout<'a>,
    span_end: usize,
    within_span: u32,
    remaining: usize,
    current_index: i32,
    current_count: f64,
}
impl ProductiveReverse<'_> {
    pub(crate) fn advance_with(
        &mut self,
        work: crate::histogram_work::HistogramWork<'_>,
    ) -> Result<bool, crate::storage::Interrupted> {
        work.step(1)?;
        if self.remaining == 0 {
            return Ok(false);
        }
        self.current_index = self.current_index.wrapping_sub(1);
        while self.within_span == 0 {
            work.step(1)?;
            self.current_index = self
                .current_index
                .wrapping_sub(self.layout.spans[self.span_end - 1].offset);
            self.span_end -= 1;
            self.within_span = self.layout.spans[self.span_end - 1].length;
        }
        self.within_span -= 1;
        self.remaining -= 1;
        self.current_count = self.layout.buckets[self.remaining];
        Ok(true)
    }
    pub(crate) fn current_index(&self) -> i32 {
        self.current_index
    }
    pub(crate) fn at(&self) -> Bucket {
        self.layout
            .at(self.layout.schema, self.current_index, self.current_count)
    }
}

impl Iterator for FloatBucketIterator<'_> {
    type Item = Bucket;

    fn next(&mut self) -> Option<Self::Item> {
        self.advance().then(|| self.at())
    }
}

pub(crate) fn visit_float_buckets_mut_with(
    schema: i32,
    positive: bool,
    spans: &[Span],
    buckets: &mut [f64],
    custom_values: Option<&[f64]>,
    work: crate::histogram_work::HistogramWork<'_>,
    mut visit: impl FnMut(Bucket, &mut f64) -> Result<(), crate::storage::Interrupted>,
) -> Result<(), crate::storage::Interrupted> {
    let layout = Layout {
        schema,
        positive,
        spans,
        buckets: &[],
        custom_values: if positive { custom_values } else { None },
    };
    let mut cursor = FloatBucketIterator::from_layout(layout, 0.0, schema);
    while let Some(position) = cursor.advance_position_with(buckets.len(), work)? {
        let slot = &mut buckets[position];
        visit(layout.at(schema, cursor.current_index, *slot), slot)?;
    }
    Ok(())
}

pub struct ReverseFloatBucketIterator<'a> {
    layout: Layout<'a>,
    span_index: isize,
    within_span: i32,
    bucket_index: isize,
    current_index: i32,
    current_count: f64,
}

impl<'a> ReverseFloatBucketIterator<'a> {
    pub fn new(histogram: FloatHistogramRef<'a>, positive: bool) -> Self {
        let layout = Layout::new(histogram, positive);
        let span_index = layout.spans.len() as isize - 1;
        let within_span = if span_index >= 0 {
            (layout.spans[span_index as usize].length as i32).wrapping_sub(1)
        } else {
            0
        };
        let current_index = layout.spans.iter().fold(0_i32, |index, span| {
            index.wrapping_add(span.offset.wrapping_add(span.length as i32))
        });
        Self {
            layout,
            span_index,
            within_span,
            bucket_index: layout.buckets.len() as isize - 1,
            current_index,
            current_count: 0.0,
        }
    }

    pub fn advance(&mut self) -> bool {
        self.current_index = self.current_index.wrapping_sub(1);
        if self.bucket_index < 0 {
            return false;
        }
        while self.within_span < 0 {
            self.span_index -= 1;
            if self.span_index < 0 {
                return false;
            }
            self.within_span =
                (self.layout.spans[self.span_index as usize].length as i32).wrapping_sub(1);
            self.current_index = self
                .current_index
                .wrapping_sub(self.layout.spans[self.span_index as usize + 1].offset);
        }
        self.current_count = self.layout.buckets[self.bucket_index as usize];
        self.bucket_index -= 1;
        self.within_span = self.within_span.wrapping_sub(1);
        true
    }

    pub fn at(&self) -> Bucket {
        self.layout
            .at(self.layout.schema, self.current_index, self.current_count)
    }
}

impl Iterator for ReverseFloatBucketIterator<'_> {
    type Item = Bucket;

    fn next(&mut self) -> Option<Self::Item> {
        self.advance().then(|| self.at())
    }
}

pub struct AllFloatBucketIterator<'a> {
    histogram: FloatHistogramRef<'a>,
    left: ReverseFloatBucketIterator<'a>,
    right: FloatBucketIterator<'a>,
    state: i8,
    current: Bucket,
}

impl<'a> AllFloatBucketIterator<'a> {
    pub fn new(histogram: FloatHistogramRef<'a>, reverse: bool) -> Self {
        Self {
            histogram,
            left: ReverseFloatBucketIterator::new(histogram, reverse),
            right: FloatBucketIterator::new(histogram, !reverse, 0.0, histogram.schema),
            state: -1,
            current: Bucket::default(),
        }
    }

    pub fn advance(&mut self) -> bool {
        loop {
            match self.state {
                -1 => {
                    if self.left.advance() {
                        self.current = self.left.at();
                        self.clip(true);
                        return true;
                    }
                    self.state = 0;
                }
                0 => {
                    self.state = 1;
                    if self.histogram.zero_count > 0.0 {
                        self.current = self.histogram.zero_bucket();
                        return true;
                    }
                }
                1 => {
                    if self.right.advance() {
                        self.current = self.right.at();
                        self.clip(false);
                        return true;
                    }
                    self.state = 42;
                    return false;
                }
                _ => return false,
            }
        }
    }

    fn clip(&mut self, upper_first: bool) {
        let threshold = self.histogram.zero_threshold;
        let clip_upper = self.current.upper < 0.0 && self.current.upper > -threshold;
        let clip_lower = self.current.lower > 0.0 && self.current.lower < threshold;
        if upper_first {
            if clip_upper {
                self.current.upper = -threshold;
            } else if clip_lower {
                self.current.lower = threshold;
            }
        } else if clip_lower {
            self.current.lower = threshold;
        } else if clip_upper {
            self.current.upper = -threshold;
        }
    }

    pub fn at(&self) -> Bucket {
        self.current
    }
}

impl Iterator for AllFloatBucketIterator<'_> {
    type Item = Bucket;

    fn next(&mut self) -> Option<Self::Item> {
        self.advance().then(|| self.at())
    }
}

#[cfg(test)]
mod tests {
    use crate::histogram::{FloatHistogram, Span};

    #[test]
    fn long_threshold_skip_uses_a_small_caller_stack_and_preserves_the_payload() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let histogram = FloatHistogram {
                    schema: 8,
                    count: 100_000.0,
                    positive_spans: Some(
                        vec![Span {
                            offset: -50_000,
                            length: 100_000,
                        }]
                        .into(),
                    ),
                    positive_buckets: Some(vec![1.0; 100_000].into()),
                    ..FloatHistogram::default()
                };
                let buckets = histogram.positive_buckets.as_deref().unwrap();
                let pointer = buckets.as_ptr();
                let capacity = histogram.positive_buckets.as_ref().unwrap().capacity();
                let mut iterator = histogram.bucket_iterator(true, f64::INFINITY, 8);
                assert!(iterator.next().is_none());
                assert!(!iterator.advance());
                assert_eq!(buckets.as_ptr(), pointer);
                assert_eq!(
                    histogram.positive_buckets.as_ref().unwrap().capacity(),
                    capacity
                );
                assert_eq!(buckets.len(), 100_000);
                assert!(
                    buckets
                        .iter()
                        .all(|value| value.to_bits() == 1.0_f64.to_bits())
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
