// SPDX-License-Identifier: GPL-3.0-or-later
// Native histogram kernels derive from Prometheus promql/{quantile,functions}.go; see NOTICE.
use crate::frontend::quote;
use crate::histogram::{AllFloatBucketIterator, Bucket, CUSTOM_BUCKETS_SCHEMA, FloatHistogramRef};
use crate::numerical::{
    compensated_add, exp2, log2, source_add, source_div, source_mul, source_sub, sqrt,
};

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

    pub(crate) fn prefix(self) -> &'static str {
        match self {
            Self::QuantileNaNResult => {
                "PromQL info: input to histogram_quantile has NaN observations, result is NaN"
            }
            Self::QuantileNaNSkew => {
                "PromQL info: input to histogram_quantile has NaN observations, result is skewed higher"
            }
            Self::FractionNaNs => {
                "PromQL info: input to histogram_fraction has NaN observations, which are excluded from all fractions"
            }
        }
    }

    pub fn raw_message(self, metric_name: &[u8]) -> String {
        let mut result = String::from(self.prefix());
        if !metric_name.is_empty() {
            result.push_str(" for metric name ");
            result.push_str(&quote::bytes(metric_name));
        }
        result
    }

    pub fn formatted_message(self, metric_name: &[u8], query: &[u8], start: isize) -> String {
        let mut result = self.raw_message(metric_name);
        crate::annotations::write_position_suffix(query, start, &mut result)
            .expect("string writer");
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
    histogram_quantile_with(q, h, || Ok::<(), std::convert::Infallible>(())).unwrap()
}

pub(crate) fn histogram_quantile_with<E>(
    q: f64,
    h: FloatHistogramRef<'_>,
    mut poll: impl FnMut() -> Result<(), E>,
) -> Result<HistogramFunctionResult, E> {
    if q < 0.0 {
        return Ok(HistogramFunctionResult::scalar(f64::NEG_INFINITY));
    }
    if q > 1.0 {
        return Ok(HistogramFunctionResult::scalar(f64::INFINITY));
    }
    if h.count == 0.0 || q.is_nan() {
        return Ok(HistogramFunctionResult::scalar(NAN));
    }

    let forward = h.sum.is_nan() || q < 0.5;
    let mut iterator = AllFloatBucketIterator::new(h, !forward);
    let mut rank = if forward {
        source_mul(h.count, q)
    } else {
        source_mul(source_sub(1.0, q), h.count)
    };
    let mut count = 0.0;
    let mut bucket = Bucket::default();
    while iterator.advance() {
        poll()?;
        bucket = iterator.at();
        if bucket.count == 0.0 {
            continue;
        }
        count = add_bucket_count(count, bucket.count);
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
                return Ok(HistogramFunctionResult::scalar(bucket.upper));
            }
            bucket.lower = 0.0;
        } else if bucket.upper == f64::INFINITY {
            return Ok(HistogramFunctionResult::scalar(bucket.lower));
        }
    }
    if count > h.count {
        count = h.count;
    }
    if count < rank {
        return Ok(if h.sum.is_nan() {
            HistogramFunctionResult {
                value: NAN,
                info: Some(NativeHistogramInfo::QuantileNaNResult),
            }
        } else {
            HistogramFunctionResult::scalar(bucket.upper)
        });
    }
    rank = if forward {
        source_sub(rank, source_sub(count, bucket.count))
    } else {
        source_sub(count, rank)
    };
    let mut info = None;
    if h.sum.is_nan() {
        // Pinned NaN-skew traversal overwrites the interpolation bucket as it finishes.
        while iterator.advance() {
            poll()?;
            bucket = iterator.at();
            // Unlike the main scan, Go keeps the old accumulator as the destination here.
            count = source_add(count, bucket.count);
        }
        if count < h.count {
            info = Some(NativeHistogramInfo::QuantileNaNSkew);
        }
    }
    let fraction = source_div(rank, bucket.count);
    let value = if custom || (bucket.lower <= 0.0 && bucket.upper >= 0.0) {
        {
            source_add(
                bucket.lower,
                source_mul(source_sub(bucket.upper, bucket.lower), fraction),
            )
        }
    } else {
        let lower = log2(bucket.lower.abs());
        let upper = log2(bucket.upper.abs());
        if bucket.lower > 0.0 {
            {
                exp2(source_add(
                    source_mul(source_sub(upper, lower), fraction),
                    lower,
                ))
            }
        } else {
            {
                -exp2(source_add(
                    upper,
                    source_mul(source_sub(1.0, fraction), source_sub(lower, upper)),
                ))
            }
        }
    };
    Ok(HistogramFunctionResult { value, info })
}

pub fn histogram_fraction(
    lower: f64,
    upper: f64,
    h: FloatHistogramRef<'_>,
) -> HistogramFunctionResult {
    histogram_fraction_with(lower, upper, h, || Ok::<(), std::convert::Infallible>(())).unwrap()
}

pub(crate) fn histogram_fraction_with<E>(
    lower: f64,
    upper: f64,
    h: FloatHistogramRef<'_>,
    mut poll: impl FnMut() -> Result<(), E>,
) -> Result<HistogramFunctionResult, E> {
    if h.count == 0.0 || lower.is_nan() || upper.is_nan() {
        return Ok(HistogramFunctionResult::scalar(NAN));
    }
    if lower >= upper {
        return Ok(HistogramFunctionResult::scalar(0.0));
    }
    let (mut count, mut rank, mut lower_rank, mut upper_rank) = (0.0, 0.0, 0.0, 0.0);
    let (mut lower_set, mut upper_set) = (false, false);
    let mut iterator = AllFloatBucketIterator::new(h, false);
    while iterator.advance() {
        poll()?;
        let mut bucket = iterator.at();
        count = add_bucket_count(count, bucket.count);
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
                let fraction = bucket.fraction_below(value, linear);
                source_add(source_mul(fraction, bucket.count), rank)
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
        rank = add_bucket_count(rank, bucket.count);
    }
    let mut info = None;
    if h.sum.is_nan() {
        while iterator.advance() {
            poll()?;
            count = add_bucket_count(count, iterator.at().count);
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
    let value = source_div(source_sub(upper_rank, lower_rank), h.count);
    Ok(HistogramFunctionResult { value, info })
}

fn add_bucket_count(previous: f64, bucket: f64) -> f64 {
    {
        // Go's main scans preserve the arriving bucket when both counts are NaNs.
        source_add(bucket, previous)
    }
}

pub fn histogram_count(h: FloatHistogramRef<'_>) -> f64 {
    h.count
}

pub fn histogram_sum(h: FloatHistogramRef<'_>) -> f64 {
    h.sum
}

pub fn histogram_avg(h: FloatHistogramRef<'_>) -> f64 {
    source_div(h.sum, h.count)
}

pub fn histogram_stdvar(h: FloatHistogramRef<'_>) -> f64 {
    histogram_stdvar_with(h, || Ok::<(), std::convert::Infallible>(())).unwrap()
}

pub(crate) fn histogram_stdvar_with<E>(
    h: FloatHistogramRef<'_>,
    mut poll: impl FnMut() -> Result<(), E>,
) -> Result<f64, E> {
    let mean = source_div(h.sum, h.count);
    let (mut variance, mut compensation) = (0.0, 0.0);
    let mut iterator = AllFloatBucketIterator::new(h, false);
    while iterator.advance() {
        poll()?;
        let bucket = iterator.at();
        if bucket.count == 0.0 {
            continue;
        }
        let value = if h.schema == CUSTOM_BUCKETS_SCHEMA {
            source_mul(source_add(bucket.upper, bucket.lower), 0.5)
        } else if bucket.lower <= 0.0 && bucket.upper >= 0.0 {
            0.0
        } else {
            let value = sqrt(source_mul(bucket.lower, bucket.upper));
            if bucket.upper < 0.0 { -value } else { value }
        };
        let contribution = {
            let delta = source_sub(value, mean);
            // Go retains the original delta in the second product; native equality observes it.
            source_mul(delta, source_mul(bucket.count, delta))
        };
        // Squaring delta first can overflow a finite upstream contribution.
        (variance, compensation) = compensated_add(contribution, variance, compensation);
    }
    Ok(source_div(source_add(variance, compensation), h.count))
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
    fn native_nan_counts_preserve_original_go_forward_and_reverse_destinations() {
        // Original Go helper capture, including signaling payload permutations.
        let cases = [
            (0xfff8_0000_0000_0027, 0x7ff8_0000_0000_0063),
            (0x7ff8_0000_0000_0063, 0xfff8_0000_0000_0027),
            (0xfff8_0000_0000_0027, 0x7ff0_0000_0000_0044),
            (0x7ff0_0000_0000_0044, 0xfff8_0000_0000_0027),
            (0x7ff8_0000_0000_0063, 0x7ff0_0000_0000_0044),
            (0x7ff0_0000_0000_0044, 0x7ff8_0000_0000_0063),
        ];
        let spans = [Span {
            offset: 0,
            length: 2,
        }];
        for (a, b) in cases {
            let buckets = [f64::from_bits(a), f64::from_bits(b)];
            let histogram = FloatHistogramRef {
                reset_hint: 0,
                schema: 0,
                zero_threshold: 0.0,
                zero_count: 0.0,
                count: f64::from_bits(0x7ff8_0000_0000_0042),
                sum: f64::from_bits(0x7ff8_0000_0000_0099),
                positive_spans: Some(&spans),
                positive_buckets: Some(&buckets),
                negative_spans: None,
                negative_buckets: None,
                custom_values: None,
            };
            assert_eq!(
                histogram_fraction(f64::NEG_INFINITY, f64::INFINITY, histogram)
                    .value
                    .to_bits(),
                b | 0x0008_0000_0000_0000,
            );
            let reverse = FloatHistogramRef {
                sum: 0.0,
                ..histogram
            };
            for q in [0.5, 0.75, 1.0] {
                assert_eq!(
                    histogram_quantile(q, reverse).value.to_bits(),
                    a | 0x0008_0000_0000_0000
                );
            }
            assert_eq!(buckets.map(f64::to_bits), [a, b]);
        }
    }

    #[test]
    fn quantile_and_fraction_polls_interrupt_main_and_nan_tail_scans_without_allocations() {
        use crate::{
            accounting::{AllocationContext, EngineAccounting, MemoryKind},
            storage::{Control, Interrupted},
        };
        for (quantile, tail) in [(true, false), (true, true), (false, false), (false, true)] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let context = AllocationContext::new(&account, MemoryKind::Workspace);
            let histogram = FloatHistogramRef {
                reset_hint: 0,
                schema: CUSTOM_BUCKETS_SCHEMA,
                zero_threshold: 0.0,
                zero_count: 0.0,
                count: 5.0,
                sum: if tail { NAN } else { 10.0 },
                positive_spans: Some(&[Span {
                    offset: 0,
                    length: 4,
                }]),
                positive_buckets: Some(&[1.0; 4]),
                negative_spans: None,
                negative_buckets: None,
                custom_values: Some(&[1.0, 2.0, 3.0]),
            }
            .to_owned_in(Some(&context));
            let memory = account.snapshot().memory;
            let control = Control::default();
            let mut visited = 0;
            let poll = || {
                visited += 1;
                if visited == 2 {
                    control.cancellation.cancel();
                }
                control.check()?;
                account.work(1);
                Ok::<(), Interrupted>(())
            };
            let result = if quantile {
                histogram_quantile_with(if tail { 0.1 } else { 0.5 }, histogram.as_ref(), poll)
            } else {
                histogram_fraction_with(
                    f64::NEG_INFINITY,
                    if tail { 0.5 } else { f64::INFINITY },
                    histogram.as_ref(),
                    poll,
                )
            };
            assert!(matches!(result, Err(Interrupted::Cancelled)));
            assert_eq!(visited, 2);
            assert_eq!(account.snapshot().work, 1);
            assert_eq!(account.snapshot().memory, memory);
            assert_eq!(histogram.positive_buckets.as_deref(), Some(&[1.0; 4][..]));
            assert_eq!(
                histogram.custom_values.as_deref(),
                Some(&[1.0, 2.0, 3.0][..])
            );
            drop(histogram);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn variance_nan_provenance_retains_mean_after_each_bucket_product() {
        // Original Go histogramVariance capture; native equality observes these bits.
        let buckets = [
            f64::from_bits(0xfff8_0000_0000_0027),
            f64::from_bits(0x7ff8_0000_0000_0063),
        ];
        let spans = [Span {
            offset: 0,
            length: 2,
        }];
        for (sum, expected) in [
            (0x7ff8_0000_0000_0099, 0x7ff8_0000_0000_0099),
            (0, 0x7ff8_0000_0000_0042),
        ] {
            let histogram = FloatHistogramRef {
                reset_hint: 0,
                schema: 0,
                zero_threshold: 0.0,
                zero_count: 0.0,
                count: f64::from_bits(0x7ff8_0000_0000_0042),
                sum: f64::from_bits(sum),
                positive_spans: Some(&spans),
                positive_buckets: Some(&buckets),
                negative_spans: None,
                negative_buckets: None,
                custom_values: None,
            };
            assert_eq!(histogram_stdvar(histogram).to_bits(), expected);
            assert_eq!(histogram_stddev(histogram).to_bits(), expected);
        }
    }

    #[test]
    fn variance_poll_interrupts_bucket_traversal_without_mutating_input() {
        use crate::{
            accounting::{AllocationContext, EngineAccounting, MemoryKind},
            storage::{Control, Interrupted},
        };
        let engine = EngineAccounting::default();
        let account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Workspace);
        let histogram = FloatHistogramRef {
            reset_hint: 0,
            schema: CUSTOM_BUCKETS_SCHEMA,
            zero_threshold: 0.0,
            zero_count: 0.0,
            count: 2.0,
            sum: 4.0,
            positive_spans: Some(&[Span {
                offset: 1,
                length: 2,
            }]),
            positive_buckets: Some(&[1.0, 1.0]),
            negative_spans: None,
            negative_buckets: None,
            custom_values: Some(&[0.0, 2.0, 4.0]),
        }
        .to_owned_in(Some(&context));
        let control = Control::default();
        let mut visited = 0;
        let result = histogram_stdvar_with(histogram.as_ref(), || {
            visited += 1;
            if visited == 2 {
                control.cancellation.cancel();
            }
            control.check()?;
            account.work(1);
            Ok::<(), Interrupted>(())
        });
        assert_eq!(result, Err(Interrupted::Cancelled));
        assert_eq!(visited, 2);
        assert_eq!(account.snapshot().work, 1);
        assert_eq!(
            histogram_stdvar(histogram.as_ref()).to_bits(),
            1.0f64.to_bits()
        );
        assert_eq!(histogram.positive_buckets.as_deref(), Some(&[1.0, 1.0][..]));
        drop(histogram);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn long_borrowed_scans_preserve_caller_buffers_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let h = FloatHistogram {
                    schema: CUSTOM_BUCKETS_SCHEMA,
                    count: 100_000.0,
                    sum: 100_000.0,
                    positive_spans: Some(
                        vec![Span {
                            offset: 0,
                            length: 100_000,
                        }]
                        .into(),
                    ),
                    positive_buckets: Some(vec![1.0; 100_000].into()),
                    custom_values: Some((1..100_000).map(f64::from).collect::<Vec<_>>().into()),
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
