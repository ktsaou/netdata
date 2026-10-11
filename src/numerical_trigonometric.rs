// SPDX-License-Identifier: GPL-3.0-or-later
// Go 1.27.1 circular functions and Payne-Hanek reduction; see NOTICE.
#![allow(clippy::excessive_precision)]

use crate::numerical_math::sqrt;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);
const SIN: [f64; 6] = [
    f64::from_bits(0x3de5_d8fd_1fd1_9ccd),
    f64::from_bits(0xbe5a_e5e5_a929_1f5d),
    f64::from_bits(0x3ec7_1de3_567d_48a1),
    f64::from_bits(0xbf2a_01a0_19bf_df03),
    f64::from_bits(0x3f81_1111_1110_f7d0),
    f64::from_bits(0xbfc5_5555_5555_5548),
];
const COS: [f64; 6] = [
    f64::from_bits(0xbda8_fa49_a086_1a9b),
    f64::from_bits(0x3e21_ee9d_7b4e_3f05),
    f64::from_bits(0xbe92_7e4f_7eac_4bc6),
    f64::from_bits(0x3efa_01a0_19c8_44f5),
    f64::from_bits(0xbf56_c16c_16c1_4f91),
    f64::from_bits(0x3fa5_5555_5555_554b),
];
const FOUR_OVER_PI: [u64; 20] = [
    0x0000000000000001,
    0x45f306dc9c882a53,
    0xf84eafa3ea69bb81,
    0xb6c52b3278872083,
    0xfca2c757bd778ac3,
    0x6e48dc74849ba5c0,
    0x0c925dd413a32439,
    0xfc3bd63962534e7d,
    0xd1046bea5d768909,
    0xd338e04d68befc82,
    0x7323ac7306a673e9,
    0x3908bf177bf25076,
    0x3ff12fffbc0b301f,
    0xde5e2316b414da3e,
    0xda6cfd9e4f96136e,
    0x9e8c7ecd3cbfd45a,
    0xea4f758fd7cbe2f6,
    0x7a0e73ef14a525d4,
    0xd7f6bf623f1aba10,
    0xac06608df8f6d757,
];

fn right(value: u64, shift: u64) -> u64 {
    if shift < 64 { value >> shift } else { 0 }
}
fn left(value: u64, shift: u64) -> u64 {
    if shift < 64 { value << shift } else { 0 }
}

fn large_reduce(value: f64) -> (u64, f64) {
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let mantissa = (bits & !(0x7ff << 52)) | (1 << 52);
    let digit = (exponent + 61) as usize / 64;
    let shift = (exponent + 61) as u64 % 64;
    let word =
        |index| left(FOUR_OVER_PI[index], shift) | right(FOUR_OVER_PI[index + 1], 64 - shift);
    let z0 = word(digit);
    let z1 = word(digit + 1);
    let z2 = word(digit + 2);
    let z2hi = ((u128::from(z2) * u128::from(mantissa)) >> 64) as u64;
    let product = u128::from(z1) * u128::from(mantissa);
    let z1hi = (product >> 64) as u64;
    let z1lo = product as u64;
    let (lo, carry) = z1lo.overflowing_add(z2hi);
    let mut hi = z0
        .wrapping_mul(mantissa)
        .wrapping_add(z1hi)
        .wrapping_add(u64::from(carry));
    let mut octant = hi >> 61;
    hi = hi << 3 | lo >> 61;
    let leading = u64::from(hi.leading_zeros());
    let exponent = 1023 - (leading + 1);
    hi = left(hi, leading + 1) | right(lo, 64_u64.wrapping_sub(leading + 1));
    hi >>= 12;
    hi |= exponent << 52;
    let mut fraction = f64::from_bits(hi);
    if octant & 1 == 1 {
        octant = (octant + 1) & 7;
        fraction -= 1.0;
    }
    (octant, fraction * FRAC_PI_4)
}

fn reduce(value: f64) -> (u64, f64) {
    if value >= (1_u64 << 29) as f64 {
        return large_reduce(value);
    }
    let mut octant = (value * (4.0 / PI)) as u64;
    let mut integer = octant as f64;
    if octant & 1 == 1 {
        octant += 1;
        integer += 1.0;
    }
    let reduced = ((value - integer * f64::from_bits(0x3fe9_21fb_4000_0000))
        - integer * f64::from_bits(0x3e64_442d_0000_0000))
        - integer * f64::from_bits(0x3ce8_4698_98cc_5170);
    (octant & 7, reduced)
}

fn polynomial(coefficients: &[f64; 6], square: f64) -> f64 {
    (((((coefficients[0] * square + coefficients[1]) * square + coefficients[2]) * square
        + coefficients[3])
        * square
        + coefficients[4])
        * square)
        + coefficients[5]
}

fn circular(value: f64, cosine: bool) -> f64 {
    if cosine {
        if !value.is_finite() {
            return NAN;
        }
    } else {
        if value == 0.0 || value.is_nan() {
            return value;
        }
        if value.is_infinite() {
            return NAN;
        }
    }
    let mut negative = !cosine && value < 0.0;
    let (mut octant, reduced) = reduce(value.abs());
    if octant > 3 {
        octant -= 4;
        negative = !negative;
    }
    if cosine && octant > 1 {
        negative = !negative;
    }
    let square = reduced * reduced;
    let cosine_polynomial = (octant == 1 || octant == 2) != cosine;
    let result = if cosine_polynomial {
        1.0 - 0.5 * square + square * square * polynomial(&COS, square)
    } else {
        reduced + reduced * square * polynomial(&SIN, square)
    };
    if negative { -result } else { result }
}

pub fn sin(value: f64) -> f64 {
    circular(value, false)
}
pub fn cos(value: f64) -> f64 {
    circular(value, true)
}

pub fn tan(value: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        return value;
    }
    if value.is_infinite() {
        return NAN;
    }
    let (octant, reduced) = reduce(value.abs());
    let square = reduced * reduced;
    let mut result = if square > 1e-14 {
        let p = [
            f64::from_bits(0xc0c9_92d8_d24f_3f38),
            f64::from_bits(0x4131_99ec_a5fc_9ddd),
            f64::from_bits(0xc171_1fea_d329_9176),
        ];
        let q = [
            f64::from_bits(0x40ca_b8a5_eeb3_6572),
            f64::from_bits(0xc134_27bc_582a_bc96),
            f64::from_bits(0x4177_d98f_c2ea_d8ef),
            f64::from_bits(0xc189_afe0_3cbe_5a31),
        ];
        reduced
            + reduced
                * (square * ((p[0] * square + p[1]) * square + p[2])
                    / ((((square + q[0]) * square + q[1]) * square + q[2]) * square + q[3]))
    } else {
        reduced
    };
    if octant & 2 == 2 {
        result = -1.0 / result;
    }
    if value < 0.0 { -result } else { result }
}

fn xatan(value: f64) -> f64 {
    let p = [
        -8.750608600031904122785e-01,
        -1.615753718733365076637e+01,
        -7.500855792314704667340e+01,
        -1.228866684490136173410e+02,
        -6.485021904942025371773e+01,
    ];
    let q = [
        2.485846490142306297962e+01,
        1.650270098316988542046e+02,
        4.328810604912902668951e+02,
        4.853903996359136964868e+02,
        1.945506571482613964425e+02,
    ];
    let square = value * value;
    let rational = square
        * ((((p[0] * square + p[1]) * square + p[2]) * square + p[3]) * square + p[4])
        / (((((square + q[0]) * square + q[1]) * square + q[2]) * square + q[3]) * square + q[4]);
    value * rational + value
}

fn satan(value: f64) -> f64 {
    const MORE_BITS: f64 = 6.123233995736765886130e-17;
    if value <= 0.66 {
        return xatan(value);
    }
    if value > 2.41421356237309504880 {
        return FRAC_PI_2 - xatan(1.0 / value) + MORE_BITS;
    }
    FRAC_PI_4 + xatan((value - 1.0) / (value + 1.0)) + 0.5 * MORE_BITS
}

pub fn atan(value: f64) -> f64 {
    if value.is_nan() {
        crate::numerical::quiet_nan(value)
    } else if value == 0.0 {
        value
    } else if value > 0.0 {
        satan(value)
    } else {
        -satan(-value)
    }
}

pub fn atan2(y: f64, x: f64) -> f64 {
    if y.is_nan() || x.is_nan() {
        return NAN;
    }
    if y == 0.0 {
        return if x >= 0.0 && !x.is_sign_negative() {
            0.0f64.copysign(y)
        } else {
            PI.copysign(y)
        };
    }
    if x == 0.0 {
        return FRAC_PI_2.copysign(y);
    }
    if x.is_infinite() {
        return if x == f64::INFINITY {
            if y.is_infinite() {
                FRAC_PI_4.copysign(y)
            } else {
                0.0f64.copysign(y)
            }
        } else if y.is_infinite() {
            (3.0 * FRAC_PI_4).copysign(y)
        } else {
            PI.copysign(y)
        };
    }
    if y.is_infinite() {
        return FRAC_PI_2.copysign(y);
    }
    let angle = atan(y / x);
    if x < 0.0 {
        if angle <= 0.0 { angle + PI } else { angle - PI }
    } else {
        angle
    }
}

pub fn asin(value: f64) -> f64 {
    if value.is_nan() {
        return crate::numerical::quiet_nan(value);
    }
    if value == 0.0 {
        return value;
    }
    let magnitude = if value < 0.0 { -value } else { value };
    if magnitude > 1.0 {
        return NAN;
    }
    let root = sqrt(1.0 - magnitude * magnitude);
    let result = if magnitude > 0.7 {
        FRAC_PI_2 - satan(root / magnitude)
    } else {
        satan(magnitude / root)
    };
    if value < 0.0 { -result } else { result }
}
pub fn acos(value: f64) -> f64 {
    if value.is_nan() {
        crate::numerical::quiet_nan(value)
    } else {
        crate::numerical::source_sub(FRAC_PI_2, asin(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_huge_reduction_and_underflow_quadrant_are_preserved() {
        assert_eq!(sin(f64::MAX).to_bits(), 0x3f74_52fc_98b3_4eb0);
        assert_eq!(cos(f64::MAX).to_bits(), 0xbfef_ffe6_2ecf_ab75);
        assert_eq!(tan(f64::MAX).to_bits(), 0xbf74_530c_fe72_949d);
        for y in [-1e-308, -f64::from_bits(1)] {
            assert_eq!(atan2(y, -1e308).to_bits(), PI.to_bits());
        }
        assert_eq!(sin(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(atan2(-0.0, -1.0).to_bits(), (-PI).to_bits());
    }
}
