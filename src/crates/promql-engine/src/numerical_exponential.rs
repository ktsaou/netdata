// SPDX-License-Identifier: GPL-3.0-or-later
// Exponential reduction and scaling derive from the pinned Go math package; see NOTICE.

pub fn ldexp(mut fraction: f64, mut exponent: isize) -> f64 {
    if fraction == 0.0 || !fraction.is_finite() {
        return fraction;
    }
    if fraction.abs() < f64::MIN_POSITIVE {
        fraction *= (1_u64 << 52) as f64;
        exponent = exponent.wrapping_sub(52);
    }
    let mut bits = fraction.to_bits();
    exponent = exponent.wrapping_add(((bits >> 52) & 0x7ff) as isize - 1023);
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
        (value + 0.5) as isize
    } else if value < 0.0 {
        (value - 0.5) as isize
    } else {
        0
    };
    let residual = value - exponent as f64;
    exp_multi(residual * LN_2_HIGH, -residual * LN_2_LOW, exponent)
}

fn exp_multi(high: f64, low: f64, exponent: isize) -> f64 {
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
