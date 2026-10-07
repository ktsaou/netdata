// SPDX-License-Identifier: GPL-3.0-or-later
// Classic histogram kernels derive from Prometheus promql/quantile.go; see NOTICE.
use std::cmp::Ordering;

use crate::go_sort::sort_by;
use crate::numerical::{almost_equal, source_mul};
#[cfg(target_arch = "x86_64")]
use crate::numerical::{source_add, source_div, source_sub};

pub(crate) const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

#[derive(Clone, Copy, Debug)]
pub struct ClassicBucket {
    pub upper_bound: f64,
    pub count: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct BucketQuantileResult {
    pub value: f64,
    pub forced_monotonicity: bool,
    pub fixed_precision: bool,
    pub min_bound: f64,
    pub max_bound: f64,
    pub max_decrease: f64,
}

impl BucketQuantileResult {
    pub(crate) fn new(value: f64) -> Self {
        Self {
            value,
            forced_monotonicity: false,
            fixed_precision: false,
            min_bound: 0.0,
            max_bound: 0.0,
            max_decrease: 0.0,
        }
    }
}

/// Mutates a nonempty group of classic bucket aggregates in place.
pub fn bucket_quantile(q: f64, buckets: &mut [ClassicBucket]) -> BucketQuantileResult {
    let mut result = BucketQuantileResult::new(NAN);
    if q.is_nan() {
        return result;
    }
    if q < 0.0 {
        result.value = f64::NEG_INFINITY;
        return result;
    }
    if q > 1.0 {
        result.value = f64::INFINITY;
        return result;
    }
    sort_buckets(buckets);
    if buckets[buckets.len() - 1].upper_bound != f64::INFINITY {
        return result;
    }
    let length = coalesce(buckets);
    let buckets = &mut buckets[..length];
    ensure_monotonic(buckets, &mut result);
    if length < 2 {
        return result;
    }
    let observations = buckets[length - 1].count;
    if observations == 0.0 {
        return result;
    }
    let rank = source_mul(q, observations);
    // Go's predicate visits remain observable when counts include NaN.
    let (mut left, mut right) = (0, length - 1);
    while left < right {
        let middle = left.wrapping_add(right) >> 1;
        if buckets[middle].count >= rank {
            right = middle;
        } else {
            left = middle + 1;
        }
    }
    let index = left;
    result.value = if index == length - 1 {
        buckets[length - 2].upper_bound
    } else if index == 0 && buckets[0].upper_bound <= 0.0 {
        buckets[0].upper_bound
    } else {
        quantile_interpolate(
            rank,
            buckets[index],
            index.checked_sub(1).map(|i| buckets[i]),
        )
    };
    result
}

/// Mutates a nonempty group; fraction does not repair cumulative counts.
pub fn bucket_fraction(lower: f64, upper: f64, buckets: &mut [ClassicBucket]) -> f64 {
    sort_buckets(buckets);
    if buckets[buckets.len() - 1].upper_bound != f64::INFINITY {
        return NAN;
    }
    let length = coalesce(buckets);
    let buckets = &buckets[..length];
    let count = buckets[length - 1].count;
    if count == 0.0 || lower.is_nan() || upper.is_nan() {
        return NAN;
    }
    if lower >= upper {
        return 0.0;
    }
    let mut ranks = FractionRanks::new(lower, upper);
    for bucket in buckets {
        if ranks.push(*bucket) {
            break;
        }
    }
    ranks.finish(count)
}

pub(crate) fn bound_order(left: f64, right: f64) -> Ordering {
    if left < right {
        Ordering::Less
    } else if left > right {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

fn sort_buckets(buckets: &mut [ClassicBucket]) {
    sort_by(buckets, |left, right| {
        bound_order(left.upper_bound, right.upper_bound)
    });
}

fn coalesce(buckets: &mut [ClassicBucket]) -> usize {
    let mut last = buckets[0];
    let mut output = 0;
    for index in 1..buckets.len() {
        let bucket = buckets[index];
        if bucket.upper_bound == last.upper_bound {
            last.count = coalesced_count(last.count, bucket.count);
        } else {
            buckets[output] = last;
            last = bucket;
            output += 1;
        }
    }
    buckets[output] = last;
    output + 1
}

fn ensure_monotonic(buckets: &mut [ClassicBucket], result: &mut BucketQuantileResult) {
    result.min_bound = f64::INFINITY;
    result.max_bound = f64::NEG_INFINITY;
    let mut previous = buckets[0].count;
    for bucket in &mut buckets[1..] {
        bucket.count = corrected_count(bucket.count, bucket.upper_bound, &mut previous, result);
    }
}

pub(crate) fn coalesced_count(previous: f64, arriving: f64) -> f64 {
    #[cfg(target_arch = "x86_64")]
    {
        source_add(arriving, previous)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        if previous.is_nan() && arriving.is_nan() {
            f64::from_bits(arriving.to_bits() | 0x0008_0000_0000_0000)
        } else {
            previous + arriving
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum CountCorrection {
    KeepState,
    UsePrevious,
    AdvancePrevious,
}

pub(crate) fn count_correction(
    current: f64,
    bound: f64,
    previous: &mut f64,
    result: &mut BucketQuantileResult,
) -> CountCorrection {
    if current == *previous {
        return CountCorrection::KeepState;
    }
    if almost_equal(*previous, current, 1e-12) {
        result.fixed_precision = true;
        return CountCorrection::UsePrevious;
    }
    if current < *previous {
        result.forced_monotonicity = true;
        if bound < result.min_bound {
            result.min_bound = bound;
        }
        if bound > result.max_bound {
            result.max_bound = bound;
        }
        let decrease = *previous - current;
        if decrease > result.max_decrease {
            result.max_decrease = decrease;
        }
        return CountCorrection::UsePrevious;
    }
    *previous = current;
    CountCorrection::AdvancePrevious
}

pub(crate) fn corrected_count(
    current: f64,
    bound: f64,
    previous: &mut f64,
    result: &mut BucketQuantileResult,
) -> f64 {
    match count_correction(current, bound, previous, result) {
        CountCorrection::UsePrevious => *previous,
        CountCorrection::KeepState | CountCorrection::AdvancePrevious => current,
    }
}

pub(crate) fn quantile_interpolate(
    mut rank: f64,
    bucket: ClassicBucket,
    previous: Option<ClassicBucket>,
) -> f64 {
    let mut count = bucket.count;
    let start = previous.map_or(0.0, |bucket| bucket.upper_bound);
    #[cfg(target_arch = "x86_64")]
    {
        if let Some(previous) = previous {
            count = source_sub(count, previous.count);
            rank = source_sub(rank, previous.count);
        }
        let width = source_sub(bucket.upper_bound, start);
        source_add(source_mul(width, source_div(rank, count)), start)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        if let Some(previous) = previous {
            count -= previous.count;
            rank -= previous.count;
        }
        start + (bucket.upper_bound - start) * (rank / count)
    }
}

pub(crate) struct FractionRanks {
    lower: f64,
    upper: f64,
    rank: f64,
    lower_rank: f64,
    upper_rank: f64,
    lower_set: bool,
    upper_set: bool,
    previous_bound: Option<f64>,
}

impl FractionRanks {
    pub(crate) fn new(lower: f64, upper: f64) -> Self {
        Self {
            lower,
            upper,
            rank: 0.0,
            lower_rank: 0.0,
            upper_rank: 0.0,
            lower_set: false,
            upper_set: false,
            previous_bound: None,
        }
    }

    pub(crate) fn push(&mut self, bucket: ClassicBucket) -> bool {
        let lower_bound = match self.previous_bound {
            Some(bound) => bound,
            None if bucket.upper_bound <= 0.0 => f64::NEG_INFINITY,
            None => 0.0,
        };
        self.previous_bound = Some(bucket.upper_bound);
        if !self.lower_set && lower_bound >= self.lower {
            self.lower_rank = self.rank;
            self.lower_set = true;
        }
        if !self.upper_set && lower_bound >= self.upper {
            self.upper_rank = self.rank;
            self.upper_set = true;
        }
        if self.lower_set && self.upper_set {
            return true;
        }
        if !self.lower_set && lower_bound < self.lower && bucket.upper_bound > self.lower {
            self.lower_rank =
                fraction_interpolate(self.lower, lower_bound, bucket, self.rank, false);
            self.lower_set = true;
        }
        if !self.upper_set && lower_bound < self.upper && bucket.upper_bound > self.upper {
            self.upper_rank =
                fraction_interpolate(self.upper, lower_bound, bucket, self.rank, true);
            self.upper_set = true;
        }
        if self.lower_set && self.upper_set {
            return true;
        }
        self.rank = bucket.count;
        false
    }

    pub(crate) fn finish(&self, count: f64) -> f64 {
        let lower = if !self.lower_set || self.lower_rank > count {
            count
        } else {
            self.lower_rank
        };
        let upper = if !self.upper_set || self.upper_rank > count {
            count
        } else {
            self.upper_rank
        };
        #[cfg(target_arch = "x86_64")]
        {
            source_div(source_sub(upper, lower), count)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            (upper - lower) / count
        }
    }
}

fn fraction_interpolate(
    value: f64,
    lower: f64,
    bucket: ClassicBucket,
    rank: f64,
    upper: bool,
) -> f64 {
    if lower == f64::NEG_INFINITY {
        return bucket.count;
    }
    #[cfg(target_arch = "x86_64")]
    {
        let product = source_div(
            source_mul(source_sub(value, lower), source_sub(bucket.count, rank)),
            source_sub(bucket.upper_bound, lower),
        );
        // The two original Go call sites use different ADD destinations.
        if upper {
            source_add(rank, product)
        } else {
            source_add(product, rank)
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = upper;
        rank + (bucket.count - rank) * (value - lower) / (bucket.upper_bound - lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_in_place_sort_and_interpolation_use_a_small_caller_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut buckets: Vec<_> = (1..=100_000)
                    .rev()
                    .map(|value| ClassicBucket {
                        upper_bound: f64::from(value),
                        count: f64::from(value),
                    })
                    .collect();
                buckets.push(ClassicBucket {
                    upper_bound: f64::INFINITY,
                    count: 100_000.0,
                });
                let storage = (buckets.as_ptr(), buckets.capacity(), buckets.len());
                assert_eq!(bucket_quantile(0.5, &mut buckets).value, 50_000.0);
                for index in 0..100_000 {
                    buckets.swap(index, (index * 48_271 + 17) % 100_000);
                }
                assert_eq!(bucket_quantile(0.5, &mut buckets).value, 50_000.0);
                assert_eq!(bucket_fraction(25_000.0, 75_000.0, &mut buckets), 0.5);
                assert_eq!(
                    storage,
                    (buckets.as_ptr(), buckets.capacity(), buckets.len())
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
