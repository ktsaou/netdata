// SPDX-License-Identifier: GPL-3.0-or-later
// Numerical operations follow the pinned Go runtime profile; see NOTICE.

const EXPONENT: u64 = 0x7ff << 52;
const FRACTION: u64 = (1 << 52) - 1;
const HALF: u64 = 1022 << 52;
const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

pub fn frexp(value: f64) -> (f64, i32) {
    if value == 0.0 || !value.is_finite() {
        return (value, 0);
    }
    let (value, mut exponent) = if value.abs() < f64::MIN_POSITIVE {
        (value * (1_u64 << 52) as f64, -52)
    } else {
        (value, 0)
    };
    let bits = value.to_bits();
    exponent += ((bits >> 52) & 0x7ff) as i32 - 1022;
    (f64::from_bits((bits & !EXPONENT) | HALF), exponent)
}

pub fn log(value: f64) -> f64 {
    let bits = value.to_bits();
    if bits & !(1 << 63) == 0 {
        return f64::NEG_INFINITY;
    }
    if bits >> 63 != 0 {
        return NAN;
    }
    if bits >= f64::INFINITY.to_bits() {
        return value;
    }
    // Upstream amd64 extracts subnormal exponents without Frexp normalization.
    let fraction = f64::from_bits((bits & FRACTION) | HALF);
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1022;
    log_polynomial(fraction, exponent)
}

fn log_polynomial(mut fraction: f64, mut exponent: i32) -> f64 {
    const HALF_SQRT_2: f64 = f64::from_bits(0x3fe6_a09e_667f_3bcd);
    const LN_2_HIGH: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
    const LN_2_LOW: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);
    const L1: f64 = f64::from_bits(0x3fe5_5555_5555_5593);
    const L2: f64 = f64::from_bits(0x3fd9_9999_9997_fa04);
    const L3: f64 = f64::from_bits(0x3fd2_4924_9422_9359);
    const L4: f64 = f64::from_bits(0x3fcc_71c5_1d8e_78af);
    const L5: f64 = f64::from_bits(0x3fc7_4664_96cb_03de);
    const L6: f64 = f64::from_bits(0x3fc3_9a09_d078_c69f);
    const L7: f64 = f64::from_bits(0x3fc2_f112_df3e_5244);
    // CMPNLT includes equality at the compiled half-sqrt-two boundary.
    if fraction <= HALF_SQRT_2 {
        fraction *= 2.0;
        exponent -= 1;
    }
    let f = fraction - 1.0;
    let k = f64::from(exponent);
    let s = f / (2.0 + f);
    let s2 = s * s;
    let s4 = s2 * s2;
    let t1 = s2 * (L1 + s4 * (L3 + s4 * (L5 + s4 * L7)));
    let t2 = s4 * (L2 + s4 * (L4 + s4 * L6));
    let r = t1 + t2;
    let half_square = 0.5 * f * f;
    k * LN_2_HIGH - ((half_square - (s * (r + half_square) + LN_2_LOW * k)) - f)
}

pub fn log2(value: f64) -> f64 {
    const INVERSE_LN_2: f64 = f64::from_bits(0x3ff7_1547_652b_82fe);
    let (fraction, exponent) = frexp(value);
    if fraction == 0.5 {
        return f64::from(exponent - 1);
    }
    log(fraction) * INVERSE_LN_2 + f64::from(exponent)
}

pub fn sqrt(value: f64) -> f64 {
    // Go's public Sqrt lowers to this same hardware primitive on qualified targets.
    value.sqrt()
}
