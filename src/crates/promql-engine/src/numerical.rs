// SPDX-License-Identifier: GPL-3.0-or-later
// Compensated summation derives from Prometheus util/kahansum; see NOTICE.
pub use crate::numerical_exponential::{exp, exp2, ldexp};
pub use crate::numerical_math::{
    frexp, log, log2, max, min, modulo, modulo_with, pow, pow_with, sqrt,
};
#[cfg(any(test, feature = "fixtures"))]
pub use crate::{numerical_exponential::exp_dispatch, numerical_math::pow_dispatch};

/// Neumaier's compensation preserves upstream's chronological arithmetic order.
pub fn compensated_add(increment: f64, sum: f64, mut compensation: f64) -> (f64, f64) {
    let total = sum + increment;
    if total.is_infinite() {
        compensation = 0.0;
    } else if sum.abs() >= increment.abs() {
        compensation += (sum - total) + increment;
    } else {
        compensation += (increment - total) + sum;
    }
    (total, compensation)
}

pub fn compensated_sub(decrement: f64, sum: f64, compensation: f64) -> (f64, f64) {
    compensated_add(-decrement, sum, compensation)
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
