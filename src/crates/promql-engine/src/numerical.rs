// SPDX-License-Identifier: GPL-3.0-or-later
// Compensated summation derives from Prometheus util/kahansum; see NOTICE.

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
