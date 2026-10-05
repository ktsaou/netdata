// SPDX-License-Identifier: GPL-3.0-or-later
// Float-only pointwise functions follow pinned promql/functions.go; see NOTICE.
use crate::{
    calendar::{CalendarFunction, float_seconds},
    numerical,
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum FloatFunction {
    Abs,
    Ceil,
    Floor,
    Exp,
    Sqrt,
    Ln,
    Log2,
    Log10,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    Rad,
    Deg,
    Sgn,
    Round,
    Clamp,
    ClampMin,
    ClampMax,
    Calendar(CalendarFunction),
}
impl FloatFunction {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "abs" => Self::Abs,
            "ceil" => Self::Ceil,
            "floor" => Self::Floor,
            "exp" => Self::Exp,
            "sqrt" => Self::Sqrt,
            "ln" => Self::Ln,
            "log2" => Self::Log2,
            "log10" => Self::Log10,
            "sin" => Self::Sin,
            "cos" => Self::Cos,
            "tan" => Self::Tan,
            "asin" => Self::Asin,
            "acos" => Self::Acos,
            "atan" => Self::Atan,
            "sinh" => Self::Sinh,
            "cosh" => Self::Cosh,
            "tanh" => Self::Tanh,
            "asinh" => Self::Asinh,
            "acosh" => Self::Acosh,
            "atanh" => Self::Atanh,
            "rad" => Self::Rad,
            "deg" => Self::Deg,
            "sgn" => Self::Sgn,
            "round" => Self::Round,
            "clamp" => Self::Clamp,
            "clamp_min" => Self::ClampMin,
            "clamp_max" => Self::ClampMax,
            _ => Self::Calendar(CalendarFunction::parse(name)?),
        })
    }
    pub(crate) fn at_time(self, time_ms: i64) -> Option<f64> {
        if let Self::Calendar(function) = self {
            Some(function.apply(time_ms / 1000))
        } else {
            None
        }
    }
    pub(crate) fn apply(self, value: f64, parameters: [f64; 2]) -> Option<f64> {
        use std::f64::consts::PI;
        Some(match self {
            Self::Abs => value.abs(),
            Self::Ceil => numerical::ceil(value),
            Self::Floor => numerical::floor(value),
            Self::Exp => numerical::exp(value),
            Self::Sqrt => numerical::sqrt(value),
            Self::Ln => numerical::log(value),
            Self::Log2 => numerical::log2(value),
            Self::Log10 => numerical::log10(value),
            Self::Sin => numerical::sin(value),
            Self::Cos => numerical::cos(value),
            Self::Tan => numerical::tan(value),
            Self::Asin => numerical::asin(value),
            Self::Acos => numerical::acos(value),
            Self::Atan => numerical::atan(value),
            Self::Sinh => numerical::sinh(value),
            Self::Cosh => numerical::cosh(value),
            Self::Tanh => numerical::tanh(value),
            Self::Asinh => numerical::asinh(value),
            Self::Acosh => numerical::acosh(value),
            Self::Atanh => numerical::atanh(value),
            Self::Rad => value * PI / 180.0,
            Self::Deg => value * 180.0 / PI,
            Self::Sgn => {
                if value < 0.0 {
                    -1.0
                } else if value > 0.0 {
                    1.0
                } else {
                    value
                }
            }
            Self::Round => {
                let inverse = 1.0 / parameters[0];
                numerical::floor(value * inverse + 0.5) / inverse
            }
            Self::Clamp | Self::ClampMin | Self::ClampMax => {
                let (min, max) = match self {
                    Self::Clamp => (parameters[0], parameters[1]),
                    Self::ClampMin => (parameters[0], f64::INFINITY),
                    Self::ClampMax => (f64::NEG_INFINITY, parameters[0]),
                    _ => unreachable!(),
                };
                if max < min {
                    return None;
                }
                numerical::max(min, numerical::min(max, value))
            }
            Self::Calendar(function) => function.apply(float_seconds(value)),
        })
    }
}

#[cfg(any(test, feature = "fixtures"))]
pub fn fixture(name: &str, value: f64, parameters: [f64; 2]) -> Option<f64> {
    FloatFunction::parse(name)?.apply(value, parameters)
}

#[cfg(any(test, feature = "fixtures"))]
pub fn time_fixture(name: &str, time_ms: i64) -> Option<f64> {
    FloatFunction::parse(name)?.at_time(time_ms)
}
