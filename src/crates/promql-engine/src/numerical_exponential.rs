// SPDX-License-Identifier: GPL-3.0-or-later
// Exponential reduction and scaling derive from the pinned Go math package; see NOTICE.

pub fn exp(value: f64) -> f64 {
    // The retained oracle's fused rounding must not depend on device features.
    exp_kernel(value, true)
}

pub(crate) fn exp_kernel(value: f64, fused: bool) -> f64 {
    const LOG_2_E: f64 = f64::from_bits(0x3ff7_1547_652b_82fe);
    const LN_2_UPPER: f64 = f64::from_bits(0x3fe6_2e42_fefa_3000);
    const LN_2_LOWER: f64 = f64::from_bits(0x3d53_de6a_f278_ece6);
    const OVERFLOW: f64 = f64::from_bits(0x4086_2e42_fefa_39ef);
    const COEFFICIENTS: [f64; 8] = [
        f64::from_bits(0x3efa_01a0_1a01_a01a),
        f64::from_bits(0x3f2a_01a0_1a01_a01a),
        f64::from_bits(0x3f56_c16c_16c1_6c17),
        f64::from_bits(0x3f81_1111_1111_1111),
        f64::from_bits(0x3fa5_5555_5555_5555),
        f64::from_bits(0x3fc5_5555_5555_5555),
        0.5,
        1.0,
    ];
    if value.to_bits() & !(1 << 63) >= f64::INFINITY.to_bits() {
        return if value == f64::NEG_INFINITY {
            0.0
        } else {
            value
        };
    }
    if value > OVERFLOW {
        return f64::INFINITY;
    }
    let rounded = (value * LOG_2_E).round_ties_even();
    // CVTSD2SL returns its integer-indefinite value on an out-of-range conversion.
    let exponent = if rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
        i32::MIN
    } else {
        rounded as i32
    };
    let scale = f64::from(exponent);
    let residual = if fused {
        (-scale).mul_add(LN_2_LOWER, (-scale).mul_add(LN_2_UPPER, value))
    } else {
        (value - scale * LN_2_UPPER) - scale * LN_2_LOWER
    } * 0.0625;
    let mut polynomial = COEFFICIENTS[0];
    for &coefficient in &COEFFICIENTS[1..] {
        polynomial = if fused {
            polynomial.mul_add(residual, coefficient)
        } else {
            polynomial * residual + coefficient
        };
    }
    let mut result = residual * polynomial;
    for _ in 0..3 {
        result *= 2.0 + result;
    }
    result = if fused {
        result.mul_add(2.0 + result, 1.0)
    } else {
        result * (2.0 + result) + 1.0
    };
    let biased = exponent.wrapping_add(1023);
    if biased >= 2047 {
        return f64::INFINITY;
    }
    if biased <= 0 {
        if biased < -52 {
            return 0.0;
        }
        return (result * f64::from_bits(((biased + 1022) as u64) << 52)) * f64::MIN_POSITIVE;
    }
    result * f64::from_bits((biased as u64) << 52)
}

#[cfg(any(test, feature = "fixtures"))]
pub fn exp_dispatch(value: f64, fused: bool) -> f64 {
    exp_kernel(value, fused)
}

pub fn ldexp(mut fraction: f64, mut exponent: i64) -> f64 {
    if fraction == 0.0 || !fraction.is_finite() {
        return fraction;
    }
    if fraction.abs() < f64::MIN_POSITIVE {
        fraction *= (1_u64 << 52) as f64;
        exponent = exponent.wrapping_sub(52);
    }
    let mut bits = fraction.to_bits();
    exponent = exponent.wrapping_add(((bits >> 52) & 0x7ff) as i64 - 1023);
    if exponent < -1075 {
        return f64::from_bits(bits & (1 << 63));
    }
    if exponent > 1023 {
        return if fraction < 0.0 {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
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

pub fn exp2(value: f64) -> f64 {
    const LN_2_HIGH: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
    const LN_2_LOW: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);
    const OVERFLOW: f64 = f64::from_bits(0x408f_ffff_ffff_ffff);
    if value.is_nan() {
        return value;
    }
    if value > OVERFLOW {
        return f64::INFINITY;
    }
    if value < -1074.0 {
        return 0.0;
    }
    let exponent = if value > 0.0 {
        (value + 0.5) as i64
    } else if value < 0.0 {
        (value - 0.5) as i64
    } else {
        0
    };
    let residual = value - exponent as f64;
    exp_multi(residual * LN_2_HIGH, -residual * LN_2_LOW, exponent)
}

fn exp_multi(high: f64, low: f64, exponent: i64) -> f64 {
    const P1: f64 = f64::from_bits(0x3fc5_5555_5555_5555);
    const P2: f64 = f64::from_bits(0xbf66_c16c_16be_bd93);
    const P3: f64 = f64::from_bits(0x3f11_566a_af25_de2c);
    const P4: f64 = f64::from_bits(0xbebb_bd41_c5d2_6bf1);
    const P5: f64 = f64::from_bits(0x3e66_3769_72be_a4d0);
    let residual = high - low;
    let square = residual * residual;
    let correction =
        residual - square * (P1 + square * (P2 + square * (P3 + square * (P4 + square * P5))));
    let result = 1.0 - ((low - (residual * correction) / (2.0 - correction)) - high);
    ldexp(result, exponent)
}
