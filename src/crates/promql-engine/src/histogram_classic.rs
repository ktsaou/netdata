// SPDX-License-Identifier: GPL-3.0-or-later
// Classic histogram kernels derive from Prometheus promql/quantile.go; see NOTICE.
use std::cmp::Ordering;

use crate::go_sort::sort_by;
use crate::numerical::almost_equal;

const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

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
    fn new(value: f64) -> Self {
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
    let mut rank = q * observations;
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
        let mut start = 0.0;
        let end = buckets[index].upper_bound;
        let mut count = buckets[index].count;
        if index > 0 {
            start = buckets[index - 1].upper_bound;
            count -= buckets[index - 1].count;
            rank -= buckets[index - 1].count;
        }
        start + (end - start) * (rank / count)
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
    let (mut rank, mut lower_rank, mut upper_rank) = (0.0, 0.0, 0.0);
    let (mut lower_set, mut upper_set) = (false, false);
    let mut lower_bound = if buckets[0].upper_bound <= 0.0 {
        f64::NEG_INFINITY
    } else {
        0.0
    };
    for (index, bucket) in buckets.iter().enumerate() {
        if index > 0 {
            lower_bound = buckets[index - 1].upper_bound;
        }
        let upper_bound = bucket.upper_bound;
        let interpolate = |value: f64| {
            if lower_bound == f64::NEG_INFINITY {
                bucket.count
            } else {
                rank + (bucket.count - rank) * (value - lower_bound) / (upper_bound - lower_bound)
            }
        };
        if !lower_set && lower_bound >= lower {
            lower_rank = rank;
            lower_set = true;
        }
        if !upper_set && lower_bound >= upper {
            upper_rank = rank;
            upper_set = true;
        }
        if lower_set && upper_set {
            break;
        }
        if !lower_set && lower_bound < lower && upper_bound > lower {
            lower_rank = interpolate(lower);
            lower_set = true;
        }
        if !upper_set && lower_bound < upper && upper_bound > upper {
            upper_rank = interpolate(upper);
            upper_set = true;
        }
        if lower_set && upper_set {
            break;
        }
        rank = bucket.count;
    }
    if !lower_set || lower_rank > count {
        lower_rank = count;
    }
    if !upper_set || upper_rank > count {
        upper_rank = count;
    }
    (upper_rank - lower_rank) / count
}

fn sort_buckets(buckets: &mut [ClassicBucket]) {
    sort_by(buckets, |left, right| {
        if left.upper_bound < right.upper_bound {
            Ordering::Less
        } else if left.upper_bound > right.upper_bound {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    });
}

fn coalesce(buckets: &mut [ClassicBucket]) -> usize {
    let mut last = buckets[0];
    let mut output = 0;
    for index in 1..buckets.len() {
        let bucket = buckets[index];
        if bucket.upper_bound == last.upper_bound {
            let count = last.count + bucket.count;
            // Pinned Go coalescing retains the arriving operand's NaN payload.
            last.count = if last.count.is_nan() && bucket.count.is_nan() {
                f64::from_bits(bucket.count.to_bits() | 0x0008_0000_0000_0000)
            } else {
                count
            };
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
        let current = bucket.count;
        if current == previous {
            continue;
        }
        if almost_equal(previous, current, 1e-12) {
            bucket.count = previous;
            result.fixed_precision = true;
            continue;
        }
        if current < previous {
            bucket.count = previous;
            result.forced_monotonicity = true;
            if bucket.upper_bound < result.min_bound {
                result.min_bound = bucket.upper_bound;
            }
            if bucket.upper_bound > result.max_bound {
                result.max_bound = bucket.upper_bound;
            }
            let decrease = previous - current;
            if decrease > result.max_decrease {
                result.max_decrease = decrease;
            }
            continue;
        }
        previous = current;
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
