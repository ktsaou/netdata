// SPDX-License-Identifier: GPL-3.0-or-later
// Bucket bounds derive from the pinned Prometheus model and Go Ldexp; see NOTICE.
use crate::histogram::CUSTOM_BUCKETS_SCHEMA;
use crate::histogram_bounds_table::EXPONENTIAL_BOUNDS;

/// Compute one bucket boundary without retaining or generating a boundary array.
pub fn bucket_bound(index: i32, schema: i32, custom_values: Option<&[f64]>) -> f64 {
    if schema == CUSTOM_BUCKETS_SCHEMA {
        let values = custom_values.unwrap_or(&[]);
        let length = values.len() as isize;
        let index = index as isize;
        if index > length || index < -1 {
            panic!("index {index} out of bounds for custom bounds of length {length}");
        }
        if index == length {
            return f64::INFINITY;
        }
        if index == -1 {
            return f64::NEG_INFINITY;
        }
        return values[index as usize];
    }
    exponential_bound(index, schema)
}

fn exponential_bound(index: i32, schema: i32) -> f64 {
    if schema < 0 {
        let shift = schema.wrapping_neg();
        assert!(shift >= 0, "negative shift amount");
        let shift = shift as u32;
        let exponent = if shift >= isize::BITS {
            0
        } else {
            (index as isize).wrapping_shl(shift)
        };
        if exponent == 1024 {
            return f64::MAX;
        }
        return normal_fraction_ldexp(1.0, exponent);
    }

    let values = EXPONENTIAL_BOUNDS[schema as usize];
    let fraction_index = index & ((1_i32 << schema) - 1);
    let fraction = f64::from_bits(values[fraction_index as usize]);
    let exponent = ((index as isize) >> schema) + 1;
    if fraction == 0.5 && exponent == 1025 {
        return f64::MAX;
    }
    normal_fraction_ldexp(fraction, exponent)
}

// The fixed bound fractions are positive and normal, so Go's special-case and
// subnormal-input normalization paths cannot occur here.
fn normal_fraction_ldexp(fraction: f64, exponent: isize) -> f64 {
    let mut bits = fraction.to_bits();
    let mut exponent = exponent.wrapping_add(((bits >> 52) & 0x7ff) as isize - 1023);
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
