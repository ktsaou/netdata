// SPDX-License-Identifier: GPL-3.0-or-later
// Bucket bounds derive from the pinned Prometheus model and Go Ldexp; see NOTICE.
use crate::histogram::CUSTOM_BUCKETS_SCHEMA;
use crate::histogram_bounds_table::EXPONENTIAL_BOUNDS;
use crate::histogram_work::{HistogramEvaluationError, HistogramWork};

/// Compute one bucket boundary without retaining or generating a boundary array.
pub fn bucket_bound(index: i32, schema: i32, custom_values: Option<&[f64]>) -> f64 {
    bucket_bound_with(index, schema, custom_values, HistogramWork::default())
        .unwrap_or_else(|error| panic!("{}", error.arithmetic()))
}

pub(crate) fn bucket_bound_with(
    index: i32,
    schema: i32,
    custom_values: Option<&[f64]>,
    work: HistogramWork<'_>,
) -> Result<f64, HistogramEvaluationError> {
    if schema == CUSTOM_BUCKETS_SCHEMA {
        let values = custom_values.unwrap_or(&[]);
        let length = values.len() as i32;
        if index > length || index < -1 {
            return Err(work.source(
                format_args!("index {index} out of bounds for custom bounds of length {length}"),
                false,
            ));
        }
        if index == length {
            return Ok(f64::INFINITY);
        }
        if index == -1 {
            return Ok(f64::NEG_INFINITY);
        }
        return Ok(values[index as usize]);
    }
    exponential_bound(index, schema, work)
}

fn exponential_bound(
    index: i32,
    schema: i32,
    work: HistogramWork<'_>,
) -> Result<f64, HistogramEvaluationError> {
    if schema < 0 {
        let shift = schema.wrapping_neg();
        if shift < 0 {
            return Err(work.source(format_args!("runtime error: negative shift amount"), true));
        }
        let shift = shift as u32;
        let exponent = if shift >= i64::BITS {
            0
        } else {
            i64::from(index).wrapping_shl(shift)
        };
        if exponent == 1024 {
            return Ok(f64::MAX);
        }
        return Ok(normal_fraction_ldexp(1.0, exponent));
    }

    let Some(values) = EXPONENTIAL_BOUNDS.get(schema as usize) else {
        return Err(work.source(
            format_args!("runtime error: index out of range [{schema}] with length 9"),
            true,
        ));
    };
    let fraction_index = index & ((1_i32 << schema) - 1);
    let fraction = f64::from_bits(values[fraction_index as usize]);
    let exponent = (i64::from(index) >> schema).wrapping_add(1);
    if fraction == 0.5 && exponent == 1025 {
        return Ok(f64::MAX);
    }
    Ok(normal_fraction_ldexp(fraction, exponent))
}

// The fixed bound fractions are positive and normal, so Go's special-case and
// subnormal-input normalization paths cannot occur here.
fn normal_fraction_ldexp(fraction: f64, exponent: i64) -> f64 {
    let mut bits = fraction.to_bits();
    let mut exponent = exponent.wrapping_add(((bits >> 52) & 0x7ff) as i64 - 1023);
    if exponent < -1075 {
        return 0.0;
    }
    if exponent > 1023 {
        return f64::INFINITY;
    }
    let mut multiplier = 1.0;
    if exponent < -1022 {
        exponent += 53;
        multiplier = f64::from_bits(0x3ca0_0000_0000_0000);
    }
    bits &= !(0x7ff << 52);
    bits |= ((exponent + 1023) as u64) << 52;
    multiplier * f64::from_bits(bits)
}
