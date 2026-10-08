// SPDX-License-Identifier: GPL-3.0-or-later
// Compensated summation derives from Prometheus util/kahansum; see NOTICE.
pub use crate::float_text::{FloatText, FloatTextStyle, float_text};
pub use crate::numerical_exponential::{exp, exp2, ldexp};
pub use crate::numerical_hyperbolic::{acosh, asinh, atanh, cosh, log1p, sinh, tanh};
pub use crate::numerical_math::{
    ceil, floor, frexp, log, log2, log10, max, min, modulo, modulo_with, pow, pow_with, sqrt,
};
pub use crate::numerical_trigonometric::{acos, asin, atan, atan2, cos, sin, tan};
#[cfg(any(test, feature = "fixtures"))]
pub use crate::{
    calendar::fixture as calendar_fixture, float_functions::fixture as float_function_fixture,
    float_functions::time_fixture as float_function_time_fixture,
    numerical_hyperbolic::hyperbolic_dispatch,
};
#[cfg(any(test, feature = "fixtures"))]
pub use crate::{numerical_exponential::exp_dispatch, numerical_math::pow_dispatch};

/// Neumaier's compensation preserves upstream's chronological arithmetic order.
pub fn compensated_add(increment: f64, sum: f64, mut compensation: f64) -> (f64, f64) {
    let total = source_add(sum, increment);
    if total.is_infinite() {
        compensation = 0.0;
    } else if sum.abs() >= increment.abs() {
        compensation = source_add(source_add(source_sub(sum, total), increment), compensation);
    } else {
        // Go adds the difference into the stored sum before the old correction.
        compensation = source_add(source_add(sum, source_sub(increment, total)), compensation);
    }
    (total, compensation)
}

pub(crate) fn quiet_nan(value: f64) -> f64 {
    f64::from_bits(value.to_bits() | 0x0008_0000_0000_0000)
}

pub(crate) fn source_neg(value: f64) -> f64 {
    // Raw unary negation preserves a signaling payload; multiplication does not.
    f64::from_bits(value.to_bits() ^ (1 << 63))
}

pub(crate) fn source_add(destination: f64, operand: f64) -> f64 {
    // Make the qualified AMD64 operand priority independent of the device.
    if destination.is_nan() {
        quiet_nan(destination)
    } else if operand.is_nan() {
        quiet_nan(operand)
    } else if destination.is_infinite() && operand == -destination {
        // LLVM constant folding need not retain the AMD64 generated-invalid sign.
        f64::from_bits(0xfff8_0000_0000_0000)
    } else {
        destination + operand
    }
}

pub(crate) fn source_sub(destination: f64, operand: f64) -> f64 {
    if destination.is_nan() {
        quiet_nan(destination)
    } else if operand.is_nan() {
        quiet_nan(operand)
    } else if destination.is_infinite() && operand == destination {
        f64::from_bits(0xfff8_0000_0000_0000)
    } else {
        destination - operand
    }
}

pub(crate) fn source_mul(destination: f64, operand: f64) -> f64 {
    if destination.is_nan() {
        quiet_nan(destination)
    } else if operand.is_nan() {
        quiet_nan(operand)
    } else if (destination == 0.0 && operand.is_infinite())
        || (destination.is_infinite() && operand == 0.0)
    {
        f64::from_bits(0xfff8_0000_0000_0000)
    } else {
        destination * operand
    }
}

pub(crate) fn source_div(destination: f64, operand: f64) -> f64 {
    if destination.is_nan() {
        quiet_nan(destination)
    } else if operand.is_nan() {
        quiet_nan(operand)
    } else if (destination == 0.0 && operand == 0.0)
        || (destination.is_infinite() && operand.is_infinite())
    {
        f64::from_bits(0xfff8_0000_0000_0000)
    } else {
        destination / operand
    }
}

pub fn compensated_sub(decrement: f64, sum: f64, compensation: f64) -> (f64, f64) {
    compensated_add(source_neg(decrement), sum, compensation)
}

pub fn almost_equal(left: f64, right: f64, epsilon: f64) -> bool {
    let stale = crate::value::STALE_NAN_BITS;
    if left.to_bits() == stale || right.to_bits() == stale {
        return left.to_bits() == stale && right.to_bits() == stale;
    }
    if (left.is_nan() && right.is_nan()) || left == right {
        return true;
    }
    let absolute_sum = left.abs() + right.abs();
    let difference = (left - right).abs();
    let min_normal = f64::from_bits(0x0010_0000_0000_0000);
    if left == 0.0 || right == 0.0 || absolute_sum < min_normal {
        return difference < epsilon * min_normal;
    }
    difference / absolute_sum.min(f64::MAX) < epsilon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compensated_nan_provenance_matches_pinned_go_amd64() {
        // Original util/kahansum.Inc capture; values are not Rust stability pins.
        let cases = [
            (
                0x7ff8_0000_0000_0001,
                0x7ff8_0000_0000_0042,
                0x7ff8_0000_0000_0042,
            ),
            (
                0x7ff8_0000_0000_0042,
                0x7ff8_0000_0000_0001,
                0x7ff8_0000_0000_0001,
            ),
            (
                0x7ff8_0000_0000_0001,
                0x7ff0_0000_0000_0044,
                0x7ff8_0000_0000_0044,
            ),
            (
                0x7ff0_0000_0000_0044,
                0x7ff8_0000_0000_0001,
                0x7ff8_0000_0000_0001,
            ),
            (
                0xfff8_0000_0000_0043,
                0x7ff8_0000_0000_0001,
                0x7ff8_0000_0000_0001,
            ),
            (
                0x7ff0_0000_0000_0000,
                0xfff0_0000_0000_0000,
                0xfff8_0000_0000_0000,
            ),
        ];
        for (increment, sum, expected) in cases {
            let (value, compensation) =
                compensated_add(f64::from_bits(increment), f64::from_bits(sum), 0.0);
            assert_eq!(
                value.to_bits(),
                expected,
                "increment={increment:x} sum={sum:x}"
            );
            assert_eq!(compensation.to_bits(), expected);
        }
    }
}
