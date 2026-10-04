// SPDX-License-Identifier: GPL-3.0-or-later
// Counter and zero-threshold logic derives from the pinned Prometheus model; see NOTICE.
use crate::histogram::{
    Bucket, COUNTER_RESET, CUSTOM_BUCKETS_SCHEMA, FloatBucketIterator, FloatHistogramRef, NOT_RESET,
};
use crate::numerical::compensated_add;

pub fn custom_bounds_match(left: Option<&[f64]>, right: Option<&[f64]>) -> bool {
    let (left, right) = (left.unwrap_or(&[]), right.unwrap_or(&[]));
    left.len() == right.len() && left.iter().zip(right).all(|(left, right)| left == right)
}

#[derive(Clone, Copy, Debug)]
pub struct ZeroCount {
    pub count: f64,
    pub threshold: f64,
    pub compensation: f64,
}

fn threshold_text(value: f64) -> String {
    if value == f64::INFINITY {
        "+Inf".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-Inf".to_owned()
    } else {
        format!("{value:.6}")
    }
}

pub fn zero_count_for_threshold(
    histogram: FloatHistogramRef<'_>,
    mut threshold: f64,
    correction: Option<FloatHistogramRef<'_>>,
) -> ZeroCount {
    let mut compensation = correction.map_or(0.0, |value| value.zero_count);
    if threshold == histogram.zero_threshold {
        return ZeroCount {
            count: histogram.zero_count,
            threshold,
            compensation,
        };
    }
    if threshold < histogram.zero_threshold {
        panic!(
            "new threshold {} is less than old threshold {}",
            threshold_text(threshold),
            threshold_text(histogram.zero_threshold)
        );
    }
    'outer: loop {
        let mut count = histogram.zero_count;
        let mut index = 0;
        for bucket in histogram.bucket_iterator(true, 0.0, histogram.schema) {
            if bucket.lower >= threshold {
                break;
            }
            (count, compensation) = compensated_add(bucket.count, count, compensation);
            if let Some(correction) = correction {
                (count, compensation) = compensated_add(
                    correction.positive_buckets.unwrap_or(&[])[index],
                    count,
                    compensation,
                );
            }
            if bucket.upper > threshold {
                if bucket.count != 0.0 {
                    threshold = bucket.upper;
                }
                break;
            }
            index += 1;
        }
        index = 0;
        for bucket in histogram.bucket_iterator(false, 0.0, histogram.schema) {
            if bucket.upper <= -threshold {
                break;
            }
            (count, compensation) = compensated_add(bucket.count, count, compensation);
            if let Some(correction) = correction {
                (count, compensation) = compensated_add(
                    correction.negative_buckets.unwrap_or(&[])[index],
                    count,
                    compensation,
                );
            }
            if bucket.lower < -threshold {
                if bucket.count != 0.0 {
                    threshold = -bucket.lower;
                    // Go carries compensation through the restarted positive/negative scan.
                    continue 'outer;
                }
                break;
            }
            index += 1;
        }
        return ZeroCount {
            count,
            threshold,
            compensation,
        };
    }
}

pub fn detect_reset(current: FloatHistogramRef<'_>, previous: FloatHistogramRef<'_>) -> bool {
    if current.reset_hint == COUNTER_RESET {
        return true;
    }
    if current.reset_hint == NOT_RESET {
        return false;
    }
    if current.count < previous.count {
        return true;
    }
    if current.schema == CUSTOM_BUCKETS_SCHEMA {
        if previous.schema != CUSTOM_BUCKETS_SCHEMA {
            return true;
        }
        if !custom_bounds_match(current.custom_values, previous.custom_values) {
            return detect_custom_reset(current, previous);
        }
    }
    if current.schema > previous.schema || current.zero_threshold < previous.zero_threshold {
        return true;
    }
    let previous_zero = zero_count_for_threshold(previous, current.zero_threshold, None);
    if previous_zero.threshold != current.zero_threshold || current.zero_count < previous_zero.count
    {
        return true;
    }
    let mut current_it = current.bucket_iterator(true, current.zero_threshold, current.schema);
    let mut previous_it = previous.bucket_iterator(true, current.zero_threshold, current.schema);
    if reset_between_iterators(&mut current_it, &mut previous_it) {
        return true;
    }
    let mut current_it = current.bucket_iterator(false, current.zero_threshold, current.schema);
    let mut previous_it = previous.bucket_iterator(false, current.zero_threshold, current.schema);
    reset_between_iterators(&mut current_it, &mut previous_it)
}

fn remaining_nonzero(iterator: &mut FloatBucketIterator<'_>) -> bool {
    loop {
        if iterator.current_count() != 0.0 {
            return true;
        }
        if !iterator.advance() {
            return false;
        }
    }
}

fn reset_between_iterators(
    current: &mut FloatBucketIterator<'_>,
    previous: &mut FloatBucketIterator<'_>,
) -> bool {
    if !previous.advance() {
        return false;
    }
    if !current.advance() {
        return remaining_nonzero(previous);
    }
    loop {
        while current.current_index() < previous.current_index() {
            if !current.advance() {
                return remaining_nonzero(previous);
            }
        }
        if current.current_index() > previous.current_index() {
            if previous.current_count() != 0.0 {
                return true;
            }
        } else if current.current_count() < previous.current_count() {
            return true;
        }
        if !previous.advance() {
            return false;
        }
    }
}

fn rollup_for_bound(
    iterator: &mut FloatBucketIterator<'_>,
    started: bool,
    mut bucket: Bucket,
    bound: f64,
) -> (f64, Bucket, bool) {
    if !started {
        if !iterator.advance() {
            return (0.0, Bucket::default(), false);
        }
        bucket = iterator.at();
    }
    let mut sum = 0.0;
    while bucket.upper <= bound {
        sum += bucket.count;
        if !iterator.advance() {
            return (sum, Bucket::default(), false);
        }
        bucket = iterator.at();
    }
    (sum, bucket, true)
}

fn detect_custom_reset(current: FloatHistogramRef<'_>, previous: FloatHistogramRef<'_>) -> bool {
    let current_bounds = current.custom_values.unwrap_or(&[]);
    let previous_bounds = previous.custom_values.unwrap_or(&[]);
    let mut current_it = current.bucket_iterator(true, 0.0, CUSTOM_BUCKETS_SCHEMA);
    let mut previous_it = previous.bucket_iterator(true, 0.0, CUSTOM_BUCKETS_SCHEMA);
    let (mut current_index, mut previous_index) = (0, 0);
    let (mut current_bucket, mut previous_bucket) = (Bucket::default(), Bucket::default());
    let (mut current_started, mut current_more) = (false, false);
    let (mut previous_started, mut previous_more) = (false, false);
    while current_index <= current_bounds.len() && previous_index <= previous_bounds.len() {
        let current_bound = current_bounds
            .get(current_index)
            .copied()
            .unwrap_or(f64::INFINITY);
        let previous_bound = previous_bounds
            .get(previous_index)
            .copied()
            .unwrap_or(f64::INFINITY);
        if current_bound == previous_bound {
            let mut current_sum = 0.0;
            if !current_started || current_more {
                (current_sum, current_bucket, current_more) = rollup_for_bound(
                    &mut current_it,
                    current_started,
                    current_bucket,
                    current_bound,
                );
                current_started = true;
            }
            let mut previous_sum = 0.0;
            if !previous_started || previous_more {
                (previous_sum, previous_bucket, previous_more) = rollup_for_bound(
                    &mut previous_it,
                    previous_started,
                    previous_bucket,
                    current_bound,
                );
                previous_started = true;
            }
            if current_sum < previous_sum {
                return true;
            }
            current_index += 1;
            previous_index += 1;
        } else if current_bound < previous_bound {
            current_index += 1;
        } else {
            previous_index += 1;
        }
    }
    false
}
