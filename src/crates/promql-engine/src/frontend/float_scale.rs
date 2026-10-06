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
