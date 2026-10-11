// SPDX-License-Identifier: GPL-3.0-or-later
// Go 1.27.1 internal/strconv/uscale.go; see NOTICE.
use super::{
    float::{FloatError, ParsedFloat},
    float_powers::POWERS,
};

fn log2_pow10(exponent: i32) -> i32 {
    (exponent * 108853) >> 15
}

fn round(value: u64) -> u64 {
    value.wrapping_add(1).wrapping_add((value >> 2) & 1) >> 2
}

fn unmin(value: u64) -> u64 {
    (value << 2).wrapping_sub(2)
}

fn log10_pow2(exponent: i32) -> i32 {
    (exponent * 78913) >> 18
}

const DECIMAL_POWERS: [u64; 20] = [
    1,
    10,
    100,
    1_000,
    10_000,
    100_000,
    1_000_000,
    10_000_000,
    100_000_000,
    1_000_000_000,
    10_000_000_000,
    100_000_000_000,
    1_000_000_000_000,
    10_000_000_000_000,
    100_000_000_000_000,
    1_000_000_000_000_000,
    10_000_000_000_000_000,
    100_000_000_000_000_000,
    1_000_000_000_000_000_000,
    10_000_000_000_000_000_000,
];

pub(crate) fn num_digits(value: u64) -> usize {
    let digits = log10_pow2((64 - value.leading_zeros()) as i32) as usize;
    digits + usize::from(value >= DECIMAL_POWERS[digits])
}

struct Scaler {
    high: u64,
    low: u64,
    shift: i32,
}

impl Scaler {
    fn new(binary: i32, decimal: i32, log2_decimal: i32) -> Self {
        let (high, low) = POWERS[(decimal + 348) as usize];
        Self {
            high,
            low,
            shift: -(binary + log2_decimal + 3),
        }
    }

    fn scale(&self, value: u64) -> u64 {
        // The source power represents high << 64 minus low, rather than concatenation.
        let product = u128::from(value) * u128::from(self.high);
        let mut high = (product >> 64) as u64;
        let middle = product as u64;
        let shift = self.shift & 63;
        if (high >> shift) << shift != high {
            return (high >> shift) | 1;
        }
        let middle2 = ((u128::from(value) * u128::from(self.low)) >> 64) as u64;
        high = high.wrapping_sub(u64::from(middle < middle2));
        (high >> shift) | u64::from(middle.wrapping_sub(middle2) > 1)
    }
}

/// Shortest source decimal within the binary64 rounding interval.
pub(crate) fn shortest(mantissa: u64, binary: i32) -> (u64, i32) {
    // This exponent belongs to the left-justified mantissa, not the usual normal exponent.
    const MIN_EXP: i32 = -1085;
    let mut zero_bits = 11;
    let (minimum, maximum, odd, decimal) = if mantissa == 1 << 63 && binary > MIN_EXP {
        (
            mantissa - (1 << (zero_bits - 2)),
            mantissa + (1 << (zero_bits - 1)),
            (mantissa >> zero_bits) & 1,
            -(((binary + zero_bits) * 631305 - 261663) >> 21),
        )
    } else {
        if binary < MIN_EXP {
            zero_bits += MIN_EXP - binary;
        }
        (
            mantissa - (1 << (zero_bits - 1)),
            mantissa + (1 << (zero_bits - 1)),
            (mantissa >> zero_bits) & 1,
            -log10_pow2(binary + zero_bits),
        )
    };
    let scaler = Scaler::new(binary, decimal, log2_pow10(decimal));
    let lower = scaler.scale(minimum).wrapping_add(odd).wrapping_add(3) >> 2;
    let upper = scaler.scale(maximum).wrapping_sub(odd) >> 2;
    let fewer = upper / 10;
    if fewer * 10 >= lower {
        return (fewer, -(decimal - 1));
    }
    let digits = if lower < upper {
        round(scaler.scale(mantissa))
    } else {
        lower
    };
    (digits, -decimal)
}

/// Source two-significant-digit rounding, including a decimal carry.
pub(crate) fn fixed_two(mantissa: u64, binary: i32) -> (u64, i32) {
    let mut decimal = 1 - log10_pow2(binary + 63);
    let scaler = Scaler::new(binary, decimal, log2_pow10(decimal));
    let mut value = scaler.scale(mantissa);
    if value >= unmin(DECIMAL_POWERS[2]) {
        value = (value / 10) | (value & 1) | u64::from(!value.is_multiple_of(10));
        decimal -= 1;
    }
    (round(value), -decimal)
}

/// Round a source decimal mantissa with its frozen exponent to binary64.
pub(super) fn parse(mantissa: u64, exponent: i32, sign: u64) -> ParsedFloat {
    let bits = (64 - mantissa.leading_zeros()) as i32;
    let log2_decimal = log2_pow10(exponent);
    let mut binary = 1074.min(53 - bits - log2_decimal);
    let scaler = Scaler::new(binary - (64 - bits), exponent, log2_decimal);
    if scaler.shift >= 64 {
        return ParsedFloat {
            value: f64::from_bits(sign),
            error: None,
        };
    }
    let mut value = scaler.scale(mantissa << (64 - bits));
    let shift = i32::from(value >= unmin(1 << 53));
    value = (value >> shift) | (value & 1);
    binary -= shift;
    let mantissa = sign | round(value);
    let exponent = -binary;
    if mantissa & (1 << 52) == 0 {
        ParsedFloat {
            value: f64::from_bits(mantissa),
            error: None,
        }
    } else if exponent >= 0x7ff - 1075 {
        ParsedFloat {
            value: f64::from_bits((mantissa & (1 << 63)) | (0x7ff << 52)),
            error: Some(FloatError::Range),
        }
    } else {
        ParsedFloat {
            value: f64::from_bits((mantissa & !(1 << 52)) | (((1075 + exponent) as u64) << 52)),
            error: None,
        }
    }
}
