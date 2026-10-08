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
    crate::numerical::source_add(
        crate::numerical::source_mul(INVERSE_LN_2, log(fraction)),
        f64::from(exponent),
    )
}

pub fn sqrt(value: f64) -> f64 {
    if value.is_nan() {
        crate::numerical::quiet_nan(value)
    } else if value < 0.0 {
        f64::from_bits(0xfff8_0000_0000_0000)
    } else {
        value.sqrt()
    }
}

pub fn log10(value: f64) -> f64 {
    // Go rounds the untyped reciprocal once, before multiplication.
    crate::numerical::source_mul(f64::from_bits(0x3fdb_cb7b_1526_e50e), log(value))
}

pub fn floor(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let integer = value.trunc();
    if value < integer {
        integer - 1.0
    } else {
        integer
    }
}

pub fn ceil(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let integer = value.trunc();
    if value > integer {
        integer + 1.0
    } else {
        integer
    }
}

pub fn min(left: f64, right: f64) -> f64 {
    if left == f64::NEG_INFINITY || right == f64::NEG_INFINITY {
        f64::NEG_INFINITY
    } else if left.is_nan() || right.is_nan() {
        NAN
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() { left } else { right }
    } else if left < right {
        left
    } else {
        right
    }
}

pub fn max(left: f64, right: f64) -> f64 {
    if left == f64::INFINITY || right == f64::INFINITY {
        f64::INFINITY
    } else if left.is_nan() || right.is_nan() {
        NAN
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() { right } else { left }
    } else if left > right {
        left
    } else {
        right
    }
}

fn completed(result: Result<f64, std::convert::Infallible>) -> f64 {
    match result {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

pub fn modulo(left: f64, right: f64) -> f64 {
    completed(modulo_with(left, right, || Ok(())))
}

pub fn modulo_with<E>(
    left: f64,
    right: f64,
    mut check: impl FnMut() -> Result<(), E>,
) -> Result<f64, E> {
    if right == 0.0 || left.is_infinite() || left.is_nan() || right.is_nan() {
        return Ok(NAN);
    }
    let divisor = right.abs();
    let (divisor_fraction, divisor_exponent) = frexp(divisor);
    let mut remainder = if left < 0.0 { -left } else { left };
    while remainder >= divisor {
        check()?;
        let (fraction, mut exponent) = frexp(remainder);
        if fraction < divisor_fraction {
            exponent -= 1;
        }
        remainder -=
            crate::numerical_exponential::ldexp(divisor, i64::from(exponent - divisor_exponent));
    }
    Ok(if left < 0.0 { -remainder } else { remainder })
}

fn odd_integer(value: f64) -> bool {
    value.abs() < (1_u64 << 53) as f64 && value == value.trunc() && value as i64 & 1 == 1
}

pub fn pow(left: f64, right: f64) -> f64 {
    completed(pow_with(left, right, || Ok(())))
}

pub fn pow_with<E>(left: f64, right: f64, check: impl FnMut() -> Result<(), E>) -> Result<f64, E> {
    pow_kernel(left, right, true, check)
}

fn pow_kernel<E>(
    left: f64,
    right: f64,
    fused: bool,
    mut check: impl FnMut() -> Result<(), E>,
) -> Result<f64, E> {
    if right == 0.0 || left == 1.0 {
        return Ok(1.0);
    }
    if right == 1.0 {
        return Ok(left);
    }
    if left.is_nan() || right.is_nan() {
        return Ok(NAN);
    }
    if left == 0.0 {
        return Ok(if right < 0.0 {
            if left.is_sign_negative() && odd_integer(right) {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        } else if left.is_sign_negative() && odd_integer(right) {
            left
        } else {
            0.0
        });
    }
    if right.is_infinite() {
        return Ok(if left == -1.0 {
            1.0
        } else if (left.abs() < 1.0) == (right == f64::INFINITY) {
            0.0
        } else {
            f64::INFINITY
        });
    }
    if left.is_infinite() {
        return if left == f64::NEG_INFINITY {
            pow_kernel(1.0 / left, -right, fused, check)
        } else {
            Ok(if right < 0.0 { 0.0 } else { f64::INFINITY })
        };
    }
    if right == 0.5 {
        return Ok(sqrt(left));
    }
    if right == -0.5 {
        return Ok(crate::numerical::source_div(1.0, sqrt(left)));
    }
    let mut integer = right.abs().trunc();
    let mut fraction = right.abs() - integer;
    if fraction != 0.0 && left < 0.0 {
        return Ok(NAN);
    }
    if integer >= (1_u64 << 63) as f64 {
        return Ok(if left == -1.0 {
            1.0
        } else if (left.abs() < 1.0) == (right > 0.0) {
            0.0
        } else {
            f64::INFINITY
        });
    }
    let mut result = 1.0;
    let mut result_exponent = 0;
    if fraction != 0.0 {
        if fraction > 0.5 {
            fraction -= 1.0;
            integer += 1.0;
        }
        result = crate::numerical_exponential::exp_kernel(fraction * log(left), fused);
    }
    let (mut factor, mut exponent) = frexp(left);
    let mut bits = integer as i64;
    while bits != 0 {
        check()?;
        if !(-4096..=4096).contains(&exponent) {
            result_exponent += exponent;
            break;
        }
        if bits & 1 == 1 {
            result *= factor;
            result_exponent += exponent;
        }
        factor *= factor;
        exponent <<= 1;
        if factor < 0.5 {
            factor += factor;
            exponent -= 1;
        }
        bits >>= 1;
    }
    if right < 0.0 {
        result = 1.0 / result;
        result_exponent = -result_exponent;
    }
    Ok(crate::numerical_exponential::ldexp(
        result,
        i64::from(result_exponent),
    ))
}

#[cfg(any(test, feature = "fixtures"))]
pub fn pow_dispatch(left: f64, right: f64, fused: bool) -> f64 {
    completed(pow_kernel(left, right, fused, || Ok(())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_runtime_routes_preserve_retained_fused_rounding() {
        let input = std::hint::black_box(0.5);
        let exponent = std::hint::black_box(1.5);
        assert_eq!(pow(input, exponent).to_bits(), 0x3fd6_a09e_667f_3bcd);
        assert_eq!(
            pow_with(input, exponent, || Ok::<_, ()>(()))
                .unwrap()
                .to_bits(),
            0x3fd6_a09e_667f_3bcd
        );
        assert!(pow(input, exponent) >= 0.3535533905932738);
        let exp_input = std::hint::black_box(f64::from_bits(0x3fd2_bec3_3301_8868));
        assert_eq!(
            crate::numerical_exponential::exp(exp_input).to_bits(),
            0x3ff5_71de_0f7b_a33d
        );
        let hyperbolic_input = std::hint::black_box(std::f64::consts::PI);
        for (call, expected) in [
            (
                crate::numerical_hyperbolic::sinh as fn(f64) -> f64,
                0x4027_18f4_5d72_e672,
            ),
            (crate::numerical_hyperbolic::cosh, 0x4027_2f14_7fee_4000),
            (crate::numerical_hyperbolic::tanh, 0x3fef_e175_fa29_2810),
        ] {
            assert_eq!(call(hyperbolic_input).to_bits(), expected);
        }
    }

    #[test]
    fn function_value_rounding_and_untyped_log10_scale_follow_source() {
        let signalling = f64::from_bits(0x7ff0_0000_0000_0001);
        assert_eq!(ceil(signalling).to_bits(), signalling.to_bits());
        assert_eq!(floor(signalling).to_bits(), signalling.to_bits());
        assert_eq!(ceil(-0.5).to_bits(), (-0.0f64).to_bits());
        assert_eq!(floor(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(log10(f64::MIN_POSITIVE).to_bits(), 0xc073_3a71_46f7_2a42);
    }

    #[test]
    fn source_dispatch_changes_power_thresholds_and_preserves_special_values() {
        let threshold = 0.3535533905932738;
        let fused = pow_dispatch(0.5, 1.5, true);
        let separate = pow_dispatch(0.5, 1.5, false);
        assert_eq!(fused.to_bits(), 0x3fd6_a09e_667f_3bcd);
        assert_eq!(separate.to_bits(), 0x3fd6_a09e_667f_3bcc);
        assert!(fused >= threshold);
        assert!(separate < threshold);
        for (left, right, expected) in [
            (-0.0, 0.5, 0),
            (-0.0, 3.0, (-0.0_f64).to_bits()),
            (-0.0, -3.0, f64::NEG_INFINITY.to_bits()),
            (
                f64::from_bits(0x7ff8_0000_0000_0123),
                1.0,
                0x7ff8_0000_0000_0123,
            ),
            (f64::from_bits(0x7ff8_0000_0000_0123), 2.0, NAN.to_bits()),
        ] {
            assert_eq!(pow(left, right).to_bits(), expected);
        }
        assert_eq!(min(f64::NEG_INFINITY, NAN), f64::NEG_INFINITY);
        assert_eq!(max(NAN, f64::INFINITY), f64::INFINITY);
        assert_eq!(min(0.0, -0.0).to_bits(), (-0.0_f64).to_bits());
        assert_eq!(max(-0.0, 0.0).to_bits(), 0);
        assert_eq!(modulo(-4.0, 2.0).to_bits(), (-0.0_f64).to_bits());
        assert_eq!(modulo(-0.0, 2.0).to_bits(), (-0.0_f64).to_bits());
        for fused in [false, true] {
            assert_eq!(
                crate::numerical_exponential::exp_dispatch(709.5, fused),
                f64::INFINITY
            );
        }
    }

    #[test]
    fn numerical_loops_observe_interruption_without_an_iteration_cap() {
        let mut work = 0;
        let result = modulo_with(f64::MAX, f64::from_bits(3), || {
            work += 1;
            if work == 5 {
                Err("interrupted")
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Err("interrupted"));
        assert_eq!(work, 5);
        let mut work = 0;
        let result = pow_with(1.01, 123.0, || {
            work += 1;
            if work == 3 {
                Err("interrupted")
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Err("interrupted"));
        assert_eq!(work, 3);
    }
}
