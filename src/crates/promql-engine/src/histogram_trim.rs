// SPDX-License-Identifier: GPL-3.0-or-later
// In-place histogram trimming derives from the pinned Prometheus model; see NOTICE.
use crate::histogram::{Bucket, FloatHistogram};
use crate::numerical::{source_add, source_div, source_mul, source_neg, source_sub, sqrt};
use crate::{
    accounting::AllocationContext, histogram_iterator::visit_float_buckets_mut_with,
    histogram_work::HistogramWork, storage::Interrupted,
};

impl FloatHistogram {
    pub fn trim_buckets(&mut self, threshold: f64, upper_trim: bool) -> &mut Self {
        self.trim_buckets_with(threshold, upper_trim, None, HistogramWork::default())
            .expect("inert histogram work")
    }

    pub(crate) fn trim_buckets_with(
        &mut self,
        threshold: f64,
        upper_trim: bool,
        context: Option<&AllocationContext>,
        work: HistogramWork<'_>,
    ) -> Result<&mut Self, Interrupted> {
        work.step(1)?;
        let custom = self.uses_custom_buckets();
        let mut count = 0.0;
        let mut sum = 0.0;
        let mut trimmed = false;
        let mut has_positive = false;
        let mut has_negative = false;

        for positive in [true, false] {
            let (spans, buckets, present) = if positive {
                (
                    &self.positive_spans,
                    &mut self.positive_buckets,
                    &mut has_positive,
                )
            } else {
                (
                    &self.negative_spans,
                    &mut self.negative_buckets,
                    &mut has_negative,
                )
            };
            visit_float_buckets_mut_with(
                self.schema,
                positive,
                spans.as_deref().unwrap_or(&[]),
                buckets.as_deref_mut().unwrap_or(&mut []),
                self.custom_values.as_deref(),
                work,
                |bucket, slot| {
                    if bucket.count == 0.0 {
                        return Ok(());
                    }
                    *present = true;
                    let (keep, midpoint) =
                        trim_bucket(bucket, threshold, upper_trim, positive, custom);
                    count = source_add(count, keep);
                    let product = source_mul(midpoint, keep);
                    sum = if positive {
                        source_add(product, sum)
                    } else {
                        source_add(sum, product)
                    };
                    if *slot != keep {
                        *slot = keep;
                        trimmed = true;
                    }
                    Ok(())
                },
            )?;
        }

        if self.zero_count > 0.0 {
            let (keep, midpoint) = trim_zero_bucket(
                self.zero_bucket(),
                threshold,
                has_negative,
                has_positive,
                upper_trim,
            );
            if self.zero_count != keep {
                self.zero_count = keep;
                trimmed = true;
            }
            sum = source_add(sum, source_mul(midpoint, keep));
            count = source_add(count, keep);
        }
        if trimmed {
            self.count = count;
            self.sum = sum;
            self.compact_with(0, context, |units| work.step(units))?;
        }
        Ok(self)
    }
}

fn trim_bucket(
    bucket: Bucket,
    rhs: f64,
    upper_trim: bool,
    positive: bool,
    linear: bool,
) -> (f64, f64) {
    let (keep_all, contains) = if upper_trim {
        (bucket.upper <= rhs, bucket.lower < rhs)
    } else {
        (bucket.lower >= rhs, bucket.upper > rhs)
    };
    if keep_all {
        return (
            bucket.count,
            midpoint(bucket.lower, bucket.upper, positive, linear),
        );
    }
    if !contains {
        return (0.0, 0.0);
    }
    if bucket.lower == f64::NEG_INFINITY || bucket.upper == f64::INFINITY {
        return trim_infinite_bucket(bucket, rhs, upper_trim);
    }
    let below = if rhs <= bucket.lower {
        0.0
    } else if rhs >= bucket.upper {
        bucket.count
    } else {
        source_mul(bucket.fraction_below(rhs, linear), bucket.count)
    };
    if upper_trim {
        (below, midpoint(bucket.lower, rhs, positive, linear))
    } else {
        (
            source_sub(bucket.count, below),
            midpoint(rhs, bucket.upper, positive, linear),
        )
    }
}

fn trim_infinite_bucket(bucket: Bucket, rhs: f64, upper_trim: bool) -> (f64, f64) {
    fn zero_if_infinite(value: f64) -> f64 {
        if value.is_infinite() { 0.0 } else { value }
    }
    if bucket.lower == f64::NEG_INFINITY {
        if upper_trim {
            if rhs >= bucket.upper {
                return (bucket.count, 0.0);
            }
            if rhs > 0.0 && bucket.upper > 0.0 && bucket.upper != f64::INFINITY {
                return (
                    source_div(source_mul(bucket.count, rhs), bucket.upper),
                    source_mul(0.5, rhs),
                );
            }
            if bucket.upper <= 0.0 {
                return (bucket.count, rhs);
            }
            return (0.0, zero_if_infinite(bucket.upper));
        }
        if rhs <= bucket.lower {
            return (bucket.count, 0.0);
        }
        if rhs >= 0.0 && bucket.upper > rhs && bucket.upper != f64::INFINITY {
            return (
                source_mul(source_sub(1.0, source_div(rhs, bucket.upper)), bucket.count),
                source_mul(source_add(bucket.upper, rhs), 0.5),
            );
        }
        return (0.0, zero_if_infinite(bucket.upper));
    }
    if upper_trim {
        return (0.0, zero_if_infinite(bucket.lower));
    }
    if rhs >= bucket.lower {
        (bucket.count, rhs)
    } else {
        (0.0, zero_if_infinite(bucket.lower))
    }
}

fn trim_zero_bucket(
    bucket: Bucket,
    rhs: f64,
    has_negative: bool,
    has_positive: bool,
    upper_trim: bool,
) -> (f64, f64) {
    let mut lower = bucket.lower;
    let mut upper = bucket.upper;
    if has_negative && !has_positive {
        upper = 0.0;
    }
    if has_positive && !has_negative {
        lower = 0.0;
    }
    let (fraction, midpoint) = if upper_trim {
        if rhs <= lower {
            return (0.0, 0.0);
        }
        if rhs >= upper {
            return (bucket.count, source_mul(source_add(lower, upper), 0.5));
        }
        (
            source_div(source_sub(rhs, lower), source_sub(upper, lower)),
            source_mul(source_add(lower, rhs), 0.5),
        )
    } else {
        if rhs <= lower {
            return (bucket.count, source_mul(source_add(lower, upper), 0.5));
        }
        if rhs >= upper {
            return (0.0, 0.0);
        }
        (
            source_div(source_sub(upper, rhs), source_sub(upper, lower)),
            source_mul(0.5, source_add(upper, rhs)),
        )
    };
    (source_mul(fraction, bucket.count), midpoint)
}

fn midpoint(lower: f64, upper: f64, positive: bool, linear: bool) -> f64 {
    if lower.is_infinite() {
        if upper.is_infinite() {
            return 0.0;
        }
        return if upper > 0.0 {
            source_mul(upper, 0.5)
        } else {
            upper
        };
    }
    if upper.is_infinite() {
        return lower;
    }
    if linear {
        return source_mul(source_add(lower, upper), 0.5);
    }
    let geometric = sqrt(source_mul(upper, lower).abs());
    if positive {
        geometric
    } else {
        source_neg(geometric)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::histogram::Span;

    #[test]
    fn large_in_place_trim_reuses_buffers_with_a_small_caller_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut spans = Vec::with_capacity(8);
                spans.push(Span {
                    offset: 0,
                    length: 100_000,
                });
                let mut buckets = Vec::with_capacity(100_008);
                buckets.resize(100_000, 1.0);
                let original_sum = f64::from_bits(0x7ff8_0000_0000_0042);
                let mut histogram = FloatHistogram {
                    count: 100_000.0,
                    sum: original_sum,
                    positive_spans: Some(spans.into()),
                    positive_buckets: Some(buckets.into()),
                    ..FloatHistogram::default()
                };
                let address = std::ptr::from_ref(&histogram);
                let spans = histogram.positive_spans.as_ref().unwrap();
                let span_storage = (spans.as_ptr(), spans.capacity());
                let buckets = histogram.positive_buckets.as_ref().unwrap();
                let bucket_storage = (buckets.as_ptr(), buckets.capacity());
                assert!(std::ptr::eq(
                    address,
                    histogram.trim_buckets(f64::INFINITY, true)
                ));
                assert_eq!(histogram.count.to_bits(), 100_000.0_f64.to_bits());
                assert_eq!(histogram.sum.to_bits(), original_sum.to_bits());
                assert!(std::ptr::eq(address, histogram.trim_buckets(4.0, true)));
                let spans = histogram.positive_spans.as_ref().unwrap();
                let buckets = histogram.positive_buckets.as_ref().unwrap();
                assert_eq!((spans.as_ptr(), spans.capacity()), span_storage);
                assert_eq!((buckets.as_ptr(), buckets.capacity()), bucket_storage);
                assert!(buckets.len() < 100_000);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
