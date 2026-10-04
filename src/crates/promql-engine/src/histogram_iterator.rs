// SPDX-License-Identifier: GPL-3.0-or-later
// Borrowed bucket traversal derives from the pinned Prometheus model; see NOTICE.
use crate::histogram::{CUSTOM_BUCKETS_SCHEMA, FloatHistogramRef, Span, bucket_bound};
use crate::histogram_layout::target_index;

#[derive(Clone, Copy, Debug, Default)]
pub struct Bucket {
    pub lower: f64,
    pub upper: f64,
    pub lower_inclusive: bool,
    pub upper_inclusive: bool,
    pub count: f64,
    pub index: i32,
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
        Self {
            layout: Layout::new(histogram, positive),
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
        loop {
            if !self.advance_unfiltered() {
                return false;
            }
            // Consume skipped buckets in a loop instead of growing the caller's stack.
            if !self.bound_reached
                && (-4..=8).contains(&self.target_schema)
                && bucket_bound(self.current_index, self.target_schema, None) <= self.start
            {
                continue;
            }
            self.bound_reached = true;
            return true;
        }
    }

    fn advance_unfiltered(&mut self) -> bool {
        if self.span_index >= self.layout.spans.len() {
            return false;
        }
        let mut span = self.layout.spans[self.span_index];
        if self.layout.schema == self.target_schema {
            self.current_index = if self.bucket_index == 0 {
                span.offset
            } else {
                self.current_index.wrapping_add(1)
            };
            if self.bucket_index >= self.layout.buckets.len() {
                return false;
            }
            while self.within_span >= span.length {
                self.within_span = 0;
                self.span_index += 1;
                if self.span_index >= self.layout.spans.len() {
                    return false;
                }
                span = self.layout.spans[self.span_index];
                self.current_index = self.current_index.wrapping_add(span.offset);
            }
            self.current_count = self.layout.buckets[self.bucket_index];
            self.within_span = self.within_span.wrapping_add(1);
            self.bucket_index += 1;
            return true;
        }

        let (mut original_index, mut span_index, mut within_span) =
            (self.original_index, self.span_index, self.within_span);
        let mut first = true;
        self.current_count = 0.0;
        'merge: loop {
            original_index = if self.bucket_index == 0 {
                span.offset
            } else {
                original_index.wrapping_add(1)
            };
            if self.bucket_index >= self.layout.buckets.len() {
                return !first;
            }
            while within_span >= span.length {
                within_span = 0;
                span_index += 1;
                if span_index >= self.layout.spans.len() {
                    return !first;
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
        true
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
}

impl Iterator for FloatBucketIterator<'_> {
    type Item = Bucket;

    fn next(&mut self) -> Option<Self::Item> {
        self.advance().then(|| self.at())
    }
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
                    positive_spans: Some(vec![Span {
                        offset: -50_000,
                        length: 100_000,
                    }]),
                    positive_buckets: Some(vec![1.0; 100_000]),
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
