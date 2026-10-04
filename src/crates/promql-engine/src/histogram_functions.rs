// SPDX-License-Identifier: GPL-3.0-or-later
// Native histogram kernels derive from Prometheus promql/{quantile,functions}.go; see NOTICE.
use crate::frontend::quote;
use crate::histogram::{AllFloatBucketIterator, Bucket, CUSTOM_BUCKETS_SCHEMA, FloatHistogramRef};
use crate::numerical::{compensated_add, exp2, log2, sqrt};

const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeHistogramInfo {
    QuantileNaNResult,
    QuantileNaNSkew,
    FractionNaNs,
}

impl NativeHistogramInfo {
    pub fn identity(self) -> &'static str {
        match self {
            Self::QuantileNaNResult => "NativeHistogramQuantileNaNResultInfo",
            Self::QuantileNaNSkew => "NativeHistogramQuantileNaNSkewInfo",
            Self::FractionNaNs => "NativeHistogramFractionNaNsInfo",
        }
    }

    pub fn raw_message(self, metric_name: &[u8]) -> String {
        let message = match self {
            Self::QuantileNaNResult => {
                "PromQL info: input to histogram_quantile has NaN observations, result is NaN"
            }
            Self::QuantileNaNSkew => {
                "PromQL info: input to histogram_quantile has NaN observations, result is skewed higher"
            }
            Self::FractionNaNs => {
                "PromQL info: input to histogram_fraction has NaN observations, which are excluded from all fractions"
            }
        };
        let mut result = String::from(message);
        if !metric_name.is_empty() {
            result.push_str(" for metric name ");
            result.push_str(&quote::bytes(metric_name));
        }
        result
    }

    pub fn formatted_message(self, metric_name: &[u8], query: &[u8], start: isize) -> String {
        let mut result = self.raw_message(metric_name);
        if query.is_empty() {
            return result;
        }
        if start < 0 || start as usize > query.len() {
            result.push_str(" (invalid position)");
            return result;
        }
        let start = start as usize;
        let mut line = 1;
        let mut column = start + 1;
        for (index, &byte) in query[..start].iter().enumerate() {
            if byte == b'\n' {
                line += 1;
                column = start - index;
            }
        }
        result.push_str(&format!(" ({line}:{column})"));
        result
    }
}

/// Numeric scans retain one borrowed bucket and an optional diagnostic identity.
#[derive(Clone, Copy, Debug)]
pub struct HistogramFunctionResult {
    pub value: f64,
    pub info: Option<NativeHistogramInfo>,
}

impl HistogramFunctionResult {
    fn scalar(value: f64) -> Self {
        Self { value, info: None }
    }
}

pub fn histogram_quantile(q: f64, h: FloatHistogramRef<'_>) -> HistogramFunctionResult {
    if q < 0.0 {
        return HistogramFunctionResult::scalar(f64::NEG_INFINITY);
    }
    if q > 1.0 {
        return HistogramFunctionResult::scalar(f64::INFINITY);
    }
    if h.count == 0.0 || q.is_nan() {
        return HistogramFunctionResult::scalar(NAN);
    }

    let forward = h.sum.is_nan() || q < 0.5;
    let mut iterator = AllFloatBucketIterator::new(h, !forward);
    let mut rank = if forward {
        q * h.count
    } else {
        (1.0 - q) * h.count
    };
    let mut count = 0.0;
    let mut bucket = Bucket::default();
    while iterator.advance() {
        bucket = iterator.at();
        if bucket.count == 0.0 {
            continue;
        }
        count += bucket.count;
        if count >= rank {
            break;
        }
    }
    let custom = h.schema == CUSTOM_BUCKETS_SCHEMA;
    if !custom && bucket.lower < 0.0 && bucket.upper > 0.0 {
        adjust_zero_bounds(&mut bucket, h);
    } else if custom {
        if bucket.lower == f64::NEG_INFINITY {
            if bucket.upper <= 0.0 {
                return HistogramFunctionResult::scalar(bucket.upper);
            }
            bucket.lower = 0.0;
        } else if bucket.upper == f64::INFINITY {
            return HistogramFunctionResult::scalar(bucket.lower);
        }
    }
    if count > h.count {
        count = h.count;
    }
    if count < rank {
        return if h.sum.is_nan() {
            HistogramFunctionResult {
                value: NAN,
                info: Some(NativeHistogramInfo::QuantileNaNResult),
            }
        } else {
            HistogramFunctionResult::scalar(bucket.upper)
        };
    }
    if forward {
        rank -= count - bucket.count;
    } else {
        rank = count - rank;
    }
    let mut info = None;
    if h.sum.is_nan() {
        // Pinned NaN-skew traversal overwrites the interpolation bucket as it finishes.
        while iterator.advance() {
            bucket = iterator.at();
            count += bucket.count;
        }
        if count < h.count {
            info = Some(NativeHistogramInfo::QuantileNaNSkew);
        }
    }
    let fraction = rank / bucket.count;
    let value = if custom || (bucket.lower <= 0.0 && bucket.upper >= 0.0) {
        bucket.lower + (bucket.upper - bucket.lower) * fraction
    } else {
        let lower = log2(bucket.lower.abs());
        let upper = log2(bucket.upper.abs());
        if bucket.lower > 0.0 {
            exp2(lower + (upper - lower) * fraction)
        } else {
            -exp2(upper + (lower - upper) * (1.0 - fraction))
        }
    };
    HistogramFunctionResult { value, info }
}

pub fn histogram_fraction(
    lower: f64,
    upper: f64,
    h: FloatHistogramRef<'_>,
) -> HistogramFunctionResult {
    if h.count == 0.0 || lower.is_nan() || upper.is_nan() {
        return HistogramFunctionResult::scalar(NAN);
    }
    if lower >= upper {
        return HistogramFunctionResult::scalar(0.0);
    }
    let (mut count, mut rank, mut lower_rank, mut upper_rank) = (0.0, 0.0, 0.0, 0.0);
    let (mut lower_set, mut upper_set) = (false, false);
    let mut iterator = AllFloatBucketIterator::new(h, false);
    while iterator.advance() {
        let mut bucket = iterator.at();
        count += bucket.count;
        let zero = bucket.lower <= 0.0 && bucket.upper >= 0.0;
        if zero {
            adjust_zero_bounds(&mut bucket, h);
        }
        if !lower_set && bucket.lower >= lower {
            lower_rank = rank;
            lower_set = true;
        }
        if !upper_set && bucket.lower >= upper {
            upper_rank = rank;
            upper_set = true;
        }
        if lower_set && upper_set {
            break;
        }
        let interpolate = |value| {
            let linear = h.schema == CUSTOM_BUCKETS_SCHEMA || zero;
            if linear && bucket.lower == f64::NEG_INFINITY {
                bucket.count
            } else {
                rank + bucket.count * bucket.fraction_below(value, linear)
            }
        };
        if !lower_set && bucket.lower < lower && bucket.upper > lower {
            lower_rank = interpolate(lower);
            lower_set = true;
        }
        if !upper_set && bucket.lower < upper && bucket.upper > upper {
            upper_rank = interpolate(upper);
            upper_set = true;
        }
        if lower_set && upper_set {
            break;
        }
        rank += bucket.count;
    }
    let mut info = None;
    if h.sum.is_nan() {
        while iterator.advance() {
            count += iterator.at().count;
        }
        if count < h.count {
            info = Some(NativeHistogramInfo::FractionNaNs);
        }
    } else {
        count = h.count;
    }
    if !lower_set || lower_rank > count {
        lower_rank = count;
    }
    if !upper_set || upper_rank > count {
        upper_rank = count;
    }
    HistogramFunctionResult {
        value: (upper_rank - lower_rank) / h.count,
        info,
    }
}

pub fn histogram_count(h: FloatHistogramRef<'_>) -> f64 {
    h.count
}

pub fn histogram_sum(h: FloatHistogramRef<'_>) -> f64 {
    h.sum
}

pub fn histogram_avg(h: FloatHistogramRef<'_>) -> f64 {
    h.sum / h.count
}

pub fn histogram_stdvar(h: FloatHistogramRef<'_>) -> f64 {
    let mean = h.sum / h.count;
    let (mut variance, mut compensation) = (0.0, 0.0);
    let mut iterator = AllFloatBucketIterator::new(h, false);
    while iterator.advance() {
        let bucket = iterator.at();
        if bucket.count == 0.0 {
            continue;
        }
        let value = if h.schema == CUSTOM_BUCKETS_SCHEMA {
            (bucket.upper + bucket.lower) / 2.0
        } else if bucket.lower <= 0.0 && bucket.upper >= 0.0 {
            0.0
        } else {
            let value = sqrt(bucket.upper * bucket.lower);
            if bucket.upper < 0.0 { -value } else { value }
        };
        let delta = value - mean;
        // Squaring delta first can overflow a finite upstream contribution.
        (variance, compensation) =
            compensated_add((bucket.count * delta) * delta, variance, compensation);
    }
    (variance + compensation) / h.count
}

pub fn histogram_stddev(h: FloatHistogramRef<'_>) -> f64 {
    sqrt(histogram_stdvar(h))
}

fn adjust_zero_bounds(bucket: &mut Bucket, histogram: FloatHistogramRef<'_>) {
    let negative = histogram.negative_buckets.unwrap_or(&[]);
    let positive = histogram.positive_buckets.unwrap_or(&[]);
    if negative.is_empty() && !positive.is_empty() {
        bucket.lower = 0.0;
    } else if positive.is_empty() && !negative.is_empty() {
        bucket.upper = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::histogram::{FloatHistogram, Span};

    #[test]
    fn long_borrowed_scans_preserve_caller_buffers_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let h = FloatHistogram {
                    schema: CUSTOM_BUCKETS_SCHEMA,
                    count: 100_000.0,
                    sum: 100_000.0,
                    positive_spans: Some(vec![Span {
                        offset: 0,
                        length: 100_000,
                    }]),
                    positive_buckets: Some(vec![1.0; 100_000]),
                    custom_values: Some((1..100_000).map(f64::from).collect()),
                    ..FloatHistogram::default()
                };
                let counts = h.positive_buckets.as_ref().unwrap();
                let bounds = h.custom_values.as_ref().unwrap();
                let storage = (
                    counts.as_ptr(),
                    counts.capacity(),
                    bounds.as_ptr(),
                    bounds.len(),
                );
                for q in [0.25, 0.5, 0.75] {
                    let result = histogram_quantile(q, h.as_ref());
                    assert_eq!(result.value, q * 100_000.0);
                    assert_eq!(result.info, None);
                }
                let result = histogram_fraction(9_998.5, 89_998.5, h.as_ref());
                assert_eq!(result.value, 0.8);
                assert_eq!(result.info, None);
                assert_eq!(histogram_stdvar(h.as_ref()), f64::INFINITY);
                assert_eq!(histogram_stddev(h.as_ref()), f64::INFINITY);
                assert_eq!(
                    storage,
                    (
                        counts.as_ptr(),
                        counts.capacity(),
                        bounds.as_ptr(),
                        bounds.len(),
                    )
                );
                assert!(counts.iter().all(|&count| count == 1.0));
                assert!(bounds.iter().enumerate().all(|(i, &b)| b == (i + 1) as f64));
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
