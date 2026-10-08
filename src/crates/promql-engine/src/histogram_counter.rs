// SPDX-License-Identifier: GPL-3.0-or-later
// Counter and zero-threshold logic derives from the pinned Prometheus model; see NOTICE.
use crate::histogram::{
    Bucket, COUNTER_RESET, CUSTOM_BUCKETS_SCHEMA, FloatBucketIterator, FloatHistogramRef, NOT_RESET,
};
use crate::numerical::compensated_add;
use crate::{
    histogram_work::{HistogramEvaluationError, HistogramWork},
    storage::Interrupted,
};

pub fn custom_bounds_match(left: Option<&[f64]>, right: Option<&[f64]>) -> bool {
    custom_bounds_match_with(left, right, HistogramWork::default()).expect("inert histogram work")
}

pub(crate) fn custom_bounds_match_with(
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
        if left != right {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Clone, Copy, Debug)]
pub struct ZeroCount {
    pub count: f64,
    pub threshold: f64,
    pub compensation: f64,
}

struct Threshold(f64);
impl std::fmt::Display for Threshold {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            f64::INFINITY => output.write_str("+Inf"),
            f64::NEG_INFINITY => output.write_str("-Inf"),
            value => write!(output, "{value:.6}"),
        }
    }
}

fn correction_bucket(
    buckets: Option<&[f64]>,
    index: usize,
    work: HistogramWork<'_>,
) -> Result<f64, HistogramEvaluationError> {
    let buckets = buckets.unwrap_or(&[]);
    buckets.get(index).copied().ok_or_else(|| {
        work.source(
            format_args!(
                "runtime error: index out of range [{index}] with length {}",
                buckets.len()
            ),
            true,
        )
    })
}

pub fn zero_count_for_threshold(
    histogram: FloatHistogramRef<'_>,
    threshold: f64,
    correction: Option<FloatHistogramRef<'_>>,
) -> ZeroCount {
    zero_count_for_threshold_with(histogram, threshold, correction, HistogramWork::default())
        .unwrap_or_else(|error| panic!("{}", error.arithmetic()))
}

pub(crate) fn zero_count_for_threshold_with(
    histogram: FloatHistogramRef<'_>,
    mut threshold: f64,
    correction: Option<FloatHistogramRef<'_>>,
    work: HistogramWork<'_>,
) -> Result<ZeroCount, HistogramEvaluationError> {
    work.step(1)?;
    let mut compensation = correction.map_or(0.0, |value| value.zero_count);
    if threshold == histogram.zero_threshold {
        return Ok(ZeroCount {
            count: histogram.zero_count,
            threshold,
            compensation,
        });
    }
    if threshold < histogram.zero_threshold {
        return Err(work.source(
            format_args!(
                "new threshold {} is less than old threshold {}",
                Threshold(threshold),
                Threshold(histogram.zero_threshold)
            ),
            false,
        ));
    }
    'outer: loop {
        work.step(1)?;
        let mut count = histogram.zero_count;
        let mut index = 0;
        let mut iterator = histogram.bucket_iterator_with(true, 0.0, histogram.schema, work)?;
        while iterator.advance_with(work)? {
            let bucket = iterator.at_with(work)?;
            work.step(1)?;
            if bucket.lower >= threshold {
                break;
            }
            (count, compensation) = compensated_add(bucket.count, count, compensation);
            if let Some(correction) = correction {
                (count, compensation) = compensated_add(
                    correction_bucket(correction.positive_buckets, index, work)?,
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
        let mut iterator = histogram.bucket_iterator_with(false, 0.0, histogram.schema, work)?;
        while iterator.advance_with(work)? {
            let bucket = iterator.at_with(work)?;
            work.step(1)?;
            if bucket.upper <= -threshold {
                break;
            }
            (count, compensation) = compensated_add(bucket.count, count, compensation);
            if let Some(correction) = correction {
                (count, compensation) = compensated_add(
                    correction_bucket(correction.negative_buckets, index, work)?,
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
        return Ok(ZeroCount {
            count,
            threshold,
            compensation,
        });
    }
}

pub fn detect_reset(current: FloatHistogramRef<'_>, previous: FloatHistogramRef<'_>) -> bool {
    detect_reset_with(current, previous, HistogramWork::default())
        .unwrap_or_else(|error| panic!("{}", error.arithmetic()))
}

pub(crate) fn detect_reset_with(
    current: FloatHistogramRef<'_>,
    previous: FloatHistogramRef<'_>,
    work: HistogramWork<'_>,
) -> Result<bool, HistogramEvaluationError> {
    work.step(1)?;
    if current.reset_hint == COUNTER_RESET {
        return Ok(true);
    }
    if current.reset_hint == NOT_RESET {
        return Ok(false);
    }
    if current.count < previous.count {
        return Ok(true);
    }
    if current.schema == CUSTOM_BUCKETS_SCHEMA {
        if previous.schema != CUSTOM_BUCKETS_SCHEMA {
            return Ok(true);
        }
        if !custom_bounds_match_with(current.custom_values, previous.custom_values, work)? {
            return detect_custom_reset(current, previous, work);
        }
    }
    if current.schema > previous.schema || current.zero_threshold < previous.zero_threshold {
        return Ok(true);
    }
    let previous_zero =
        zero_count_for_threshold_with(previous, current.zero_threshold, None, work)?;
    if previous_zero.threshold != current.zero_threshold || current.zero_count < previous_zero.count
    {
        return Ok(true);
    }
    let mut current_it =
        current.bucket_iterator_with(true, current.zero_threshold, current.schema, work)?;
    let mut previous_it =
        previous.bucket_iterator_with(true, current.zero_threshold, current.schema, work)?;
    if reset_between_iterators(&mut current_it, &mut previous_it, work)? {
        return Ok(true);
    }
    let mut current_it =
        current.bucket_iterator_with(false, current.zero_threshold, current.schema, work)?;
    let mut previous_it =
        previous.bucket_iterator_with(false, current.zero_threshold, current.schema, work)?;
    reset_between_iterators(&mut current_it, &mut previous_it, work)
}

fn remaining_nonzero(
    iterator: &mut FloatBucketIterator<'_>,
    work: HistogramWork<'_>,
) -> Result<bool, HistogramEvaluationError> {
    loop {
        work.step(1)?;
        if iterator.current_count() != 0.0 {
            return Ok(true);
        }
        if !iterator.advance_with(work)? {
            return Ok(false);
        }
    }
}

fn reset_between_iterators(
    current: &mut FloatBucketIterator<'_>,
    previous: &mut FloatBucketIterator<'_>,
    work: HistogramWork<'_>,
) -> Result<bool, HistogramEvaluationError> {
    work.step(1)?;
    if !previous.advance_with(work)? {
        return Ok(false);
    }
    if !current.advance_with(work)? {
        return remaining_nonzero(previous, work);
    }
    loop {
        work.step(1)?;
        while current.current_index() < previous.current_index() {
            work.step(1)?;
            if !current.advance_with(work)? {
                return remaining_nonzero(previous, work);
            }
        }
        if current.current_index() > previous.current_index() {
            if previous.current_count() != 0.0 {
                return Ok(true);
            }
        } else if current.current_count() < previous.current_count() {
            return Ok(true);
        }
        if !previous.advance_with(work)? {
            return Ok(false);
        }
    }
}

fn rollup_for_bound(
    iterator: &mut FloatBucketIterator<'_>,
    started: bool,
    mut bucket: Bucket,
    bound: f64,
    work: HistogramWork<'_>,
) -> Result<(f64, Bucket, bool), HistogramEvaluationError> {
    work.step(1)?;
    if !started {
        if !iterator.advance_with(work)? {
            return Ok((0.0, Bucket::default(), false));
        }
        bucket = iterator.at_with(work)?;
    }
    let mut sum = 0.0;
    while bucket.upper <= bound {
        work.step(1)?;
        sum += bucket.count;
        if !iterator.advance_with(work)? {
            return Ok((sum, Bucket::default(), false));
        }
        bucket = iterator.at_with(work)?;
    }
    Ok((sum, bucket, true))
}

fn detect_custom_reset(
    current: FloatHistogramRef<'_>,
    previous: FloatHistogramRef<'_>,
    work: HistogramWork<'_>,
) -> Result<bool, HistogramEvaluationError> {
    let current_bounds = current.custom_values.unwrap_or(&[]);
    let previous_bounds = previous.custom_values.unwrap_or(&[]);
    let mut current_it = current.bucket_iterator_with(true, 0.0, CUSTOM_BUCKETS_SCHEMA, work)?;
    let mut previous_it = previous.bucket_iterator_with(true, 0.0, CUSTOM_BUCKETS_SCHEMA, work)?;
    let (mut current_index, mut previous_index) = (0, 0);
    let (mut current_bucket, mut previous_bucket) = (Bucket::default(), Bucket::default());
    let (mut current_started, mut current_more) = (false, false);
    let (mut previous_started, mut previous_more) = (false, false);
    while current_index <= current_bounds.len() && previous_index <= previous_bounds.len() {
        work.step(1)?;
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
                    work,
                )?;
                current_started = true;
            }
            let mut previous_sum = 0.0;
            if !previous_started || previous_more {
                (previous_sum, previous_bucket, previous_more) = rollup_for_bound(
                    &mut previous_it,
                    previous_started,
                    previous_bucket,
                    current_bound,
                    work,
                )?;
                previous_started = true;
            }
            if current_sum < previous_sum {
                return Ok(true);
            }
            current_index += 1;
            previous_index += 1;
        } else if current_bound < previous_bound {
            current_index += 1;
        } else {
            previous_index += 1;
        }
    }
    Ok(false)
}
