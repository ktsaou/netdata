// SPDX-License-Identifier: GPL-3.0-or-later
// Go 1.27.1 hyperbolic functions and Log1p; see NOTICE.
#![allow(clippy::excessive_precision)]

use crate::{
    numerical_exponential::exp_kernel,
    numerical_math::{log, sqrt},
};
use std::f64::consts::LN_2;

const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

pub fn log1p(value: f64) -> f64 {
    const SQRT_2_MINUS_1: f64 = 4.142135623730950488017e-01;
    const HALF_SQRT_2_MINUS_1: f64 = -2.928932188134524755992e-01;
    const LN_2_HIGH: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
    const LN_2_LOW: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);
    const LP: [f64; 7] = [
        f64::from_bits(0x3fe5_5555_5555_5593),
        f64::from_bits(0x3fd9_9999_9997_fa04),
        f64::from_bits(0x3fd2_4924_9422_9359),
        f64::from_bits(0x3fcc_71c5_1d8e_78af),
        f64::from_bits(0x3fc7_4664_96cb_03de),
        f64::from_bits(0x3fc3_9a09_d078_c69f),
        f64::from_bits(0x3fc2_f112_df3e_5244),
    ];
    if value < -1.0 || value.is_nan() {
        return NAN;
    }
    if value == -1.0 {
        return f64::NEG_INFINITY;
    }
    if value == f64::INFINITY {
        return value;
    }
    let magnitude = value.abs();
    let mut fraction = 0.0;
    let mut mantissa = 0;
    let mut exponent = 1;
    if magnitude < SQRT_2_MINUS_1 {
        if magnitude < 1.0 / (1_u64 << 29) as f64 {
            if magnitude < 1.0 / (1_u64 << 54) as f64 {
                return value;
            }
            return value - value * value * 0.5;
        }
        if value > HALF_SQRT_2_MINUS_1 {
            exponent = 0;
            fraction = value;
            mantissa = 1;
        }
    }
    let mut correction = 0.0;
    if exponent != 0 {
        let mut normalized;
        if magnitude < (1_u64 << 53) as f64 {
            normalized = 1.0 + value;
            mantissa = normalized.to_bits();
            exponent = (mantissa >> 52) as i32 - 1023;
            correction = if exponent > 0 {
                1.0 - (normalized - value)
            } else {
                value - (normalized - 1.0)
            };
            correction /= normalized;
        } else {
            normalized = value;
            mantissa = normalized.to_bits();
            exponent = (mantissa >> 52) as i32 - 1023;
        }
        mantissa &= 0x000f_ffff_ffff_ffff;
        if mantissa < 0x0006_a09e_667f_3bcd {
            normalized = f64::from_bits(mantissa | 0x3ff0_0000_0000_0000);
        } else {
            exponent += 1;
            normalized = f64::from_bits(mantissa | 0x3fe0_0000_0000_0000);
            mantissa = (0x0010_0000_0000_0000 - mantissa) >> 2;
        }
        fraction = normalized - 1.0;
    }
    let half_square = 0.5 * fraction * fraction;
    let exponent = f64::from(exponent);
    if mantissa == 0 {
        if fraction == 0.0 {
            if exponent == 0.0 {
                return 0.0;
            }
            correction += exponent * LN_2_LOW;
            return exponent * LN_2_HIGH + correction;
        }
        let remainder = half_square * (1.0 - 0.66666666666666666 * fraction);
        if exponent == 0.0 {
            return fraction - remainder;
        }
        return exponent * LN_2_HIGH
            - ((remainder - (exponent * LN_2_LOW + correction)) - fraction);
    }
    let ratio = fraction / (2.0 + fraction);
    let square = ratio * ratio;
    let remainder = square
        * (LP[0]
            + square
                * (LP[1]
                    + square
                        * (LP[2]
                            + square
                                * (LP[3] + square * (LP[4] + square * (LP[5] + square * LP[6]))))));
    if exponent == 0.0 {
        return fraction - (half_square - ratio * (half_square + remainder));
    }
    exponent * LN_2_HIGH
        - ((half_square - (ratio * (half_square + remainder) + (exponent * LN_2_LOW + correction)))
            - fraction)
}

fn sinh_kernel(value: f64, fused: bool) -> f64 {
    if value.is_nan() {
        return crate::numerical::quiet_nan(value);
    }
    let magnitude = if value < 0.0 { -value } else { value };
    let result = if magnitude > 21.0 {
        exp_kernel(magnitude, fused) * 0.5
    } else if magnitude > 0.5 {
        let exponential = exp_kernel(magnitude, fused);
        (exponential - 1.0 / exponential) * 0.5
    } else {
        let square = magnitude * magnitude;
        let numerator = (((-0.2630563213397497062819489e+2 * square
            - 0.2894211355989563807284660366e+4)
            * square
            - 0.8991272022039509355398013511e+5)
            * square
            - 0.6307673640497716991184787251e+6)
            * magnitude;
        numerator
            / (((square - 0.173678953558233699533450911e+3) * square
                + 0.1521517378790019070696485176e+5)
                * square
                - 0.6307673640497716991212077277e+6)
    };
    if value < 0.0 { -result } else { result }
}
pub fn sinh(value: f64) -> f64 {
    sinh_kernel(value, true)
}

fn cosh_kernel(value: f64, fused: bool) -> f64 {
    if value.is_nan() {
        return crate::numerical::quiet_nan(value.abs());
    }
    let magnitude = value.abs();
    if magnitude > 21.0 {
        return exp_kernel(magnitude, fused) * 0.5;
    }
    let exponential = exp_kernel(magnitude, fused);
    (exponential + 1.0 / exponential) * 0.5
}
pub fn cosh(value: f64) -> f64 {
    cosh_kernel(value, true)
}

fn tanh_kernel(value: f64, fused: bool) -> f64 {
    if value.is_nan() {
        return crate::numerical::quiet_nan(value);
    }
    let magnitude = value.abs();
    if magnitude > 0.5 * 8.8029691931113054295988e+01 {
        return if value < 0.0 { -1.0 } else { 1.0 };
    }
    if magnitude >= 0.625 {
        let exponential = exp_kernel(2.0 * magnitude, fused);
        let result = 1.0 - 2.0 / (exponential + 1.0);
        return if value < 0.0 { -result } else { result };
    }
    if value == 0.0 {
        return value;
    }
    let square = value * value;
    value
        + value
            * square
            * ((-9.64399179425052238628e-1 * square - 9.92877231001918586564e1) * square
                - 1.61468768441708447952e3)
            / (((square + 1.12811678491632931402e2) * square + 2.23548839060100448583e3) * square
                + 4.84406305325125486048e3)
}
pub fn tanh(value: f64) -> f64 {
    tanh_kernel(value, true)
}

pub fn asinh(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let magnitude = if value < 0.0 { -value } else { value };
    let result = if magnitude > (1_u64 << 28) as f64 {
        log(magnitude) + LN_2
    } else if magnitude > 2.0 {
        log(2.0 * magnitude + 1.0 / (sqrt(magnitude * magnitude + 1.0) + magnitude))
    } else if magnitude < 1.0 / (1_u64 << 28) as f64 {
        magnitude
    } else {
        log1p(magnitude + magnitude * magnitude / (1.0 + sqrt(1.0 + magnitude * magnitude)))
    };
    if value < 0.0 { -result } else { result }
}

pub fn acosh(value: f64) -> f64 {
    if value < 1.0 || value.is_nan() {
        return NAN;
    }
    if value == 1.0 {
        return 0.0;
    }
    if value >= (1_u64 << 28) as f64 {
        return log(value) + LN_2;
    }
    if value > 2.0 {
        return log(2.0 * value - 1.0 / (value + sqrt(value * value - 1.0)));
    }
    let difference = value - 1.0;
    log1p(difference + sqrt(2.0 * difference + difference * difference))
}

pub fn atanh(value: f64) -> f64 {
    if !(-1.0..=1.0).contains(&value) {
        return NAN;
    }
    if value == 1.0 {
        return f64::INFINITY;
    }
    if value == -1.0 {
        return f64::NEG_INFINITY;
    }
    let magnitude = if value < 0.0 { -value } else { value };
    let result = if magnitude < 1.0 / (1_u64 << 28) as f64 {
        magnitude
    } else if magnitude < 0.5 {
        let twice = magnitude + magnitude;
        0.5 * log1p(twice + twice * magnitude / (1.0 - magnitude))
    } else {
        0.5 * log1p((magnitude + magnitude) / (1.0 - magnitude))
    };
    if value < 0.0 { -result } else { result }
}

#[cfg(any(test, feature = "fixtures"))]
pub fn hyperbolic_dispatch(name: &str, value: f64, fused: bool) -> Option<f64> {
    Some(match name {
        "sinh" => sinh_kernel(value, fused),
        "cosh" => cosh_kernel(value, fused),
        "tanh" => tanh_kernel(value, fused),
        _ => return None,
    })
}
