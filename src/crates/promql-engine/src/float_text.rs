// SPDX-License-Identifier: GPL-3.0-or-later
// Go internal/strconv/ftoa.go and Prometheus model/labels/float.go; see NOTICE.
use std::fmt;

use crate::frontend::float_scale::{fixed_two, num_digits, shortest};

/// Source projections used by PromQL numeric annotations and generated labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FloatTextStyle {
    /// Go's shortest lowercase g representation, preserving signed zero.
    Shortest,
    /// Go's lowercase g representation rounded to two significant digits.
    TwoSignificant,
    /// Prometheus label formatting, with decimal zero and integer suffixes.
    OpenMetrics,
}

/// Inline numeric text; final destination ownership stays with the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FloatText {
    bytes: [u8; 32],
    len: usize,
}

impl FloatText {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.as_bytes()).expect("numeric text is ASCII")
    }

    fn push(&mut self, byte: u8) {
        self.bytes[self.len] = byte;
        self.len += 1;
    }

    fn extend(&mut self, bytes: &[u8]) {
        self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    fn zeros(&mut self, count: i32) {
        for _ in 0..count {
            self.push(b'0');
        }
    }
}

impl AsRef<[u8]> for FloatText {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Display for FloatText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Render binary64 text without heap allocation or input-sized scratch.
pub fn float_text(value: f64, style: FloatTextStyle) -> FloatText {
    let mut output = FloatText {
        bytes: [0; 32],
        len: 0,
    };
    let bits = value.to_bits();
    let negative = bits >> 63 != 0;
    let mut exponent = ((bits >> 52) & 0x7ff) as i32;
    let mut mantissa = bits & ((1 << 52) - 1);
    if exponent == 0x7ff {
        output.extend(if mantissa != 0 {
            b"NaN"
        } else if negative {
            b"-Inf"
        } else {
            b"+Inf"
        });
        return output;
    }
    if exponent == 0 && mantissa == 0 {
        if negative && style != FloatTextStyle::OpenMetrics {
            output.push(b'-');
        }
        output.extend(if style == FloatTextStyle::OpenMetrics {
            b"0.0"
        } else {
            b"0"
        });
        return output;
    }
    if exponent == 0 {
        exponent += 1;
    } else {
        mantissa |= 1 << 52;
    }
    exponent -= 1023;
    let shift = mantissa.leading_zeros();
    let binary = exponent - shift as i32 - 52;
    mantissa <<= shift;
    let is_shortest = style != FloatTextStyle::TwoSignificant;
    let (mut value, power) = if is_shortest {
        shortest(mantissa, binary)
    } else {
        fixed_two(mantissa, binary)
    };
    let mut count = num_digits(value);
    let point = count as i32 + power;
    let mut digits = [0; 20];
    for slot in digits[..count].iter_mut().rev() {
        *slot = (value % 10) as u8 + b'0';
        value /= 10;
    }
    while count > 0 && digits[count - 1] == b'0' {
        count -= 1;
    }
    let precision = if is_shortest { count as i32 } else { 2 };
    general(
        &mut output,
        negative,
        &digits[..count],
        point,
        precision,
        is_shortest,
    );
    if style == FloatTextStyle::OpenMetrics
        && !output.as_bytes().iter().any(|&b| b == b'.' || b == b'e')
    {
        output.extend(b".0");
    }
    output
}

fn general(
    output: &mut FloatText,
    negative: bool,
    digits: &[u8],
    point: i32,
    mut precision: i32,
    shortest: bool,
) {
    let count = digits.len() as i32;
    let mut exponent_precision = precision;
    if exponent_precision > count && count >= point {
        exponent_precision = count;
    }
    if shortest {
        exponent_precision = 6;
    }
    if negative {
        output.push(b'-');
    }
    let mut exponent = point - 1;
    if exponent < -4 || exponent >= exponent_precision {
        precision = precision.min(count) - 1;
        output.push(digits.first().copied().unwrap_or(b'0'));
        if precision > 0 {
            output.push(b'.');
            let end = count.min(precision + 1);
            if end > 1 {
                output.extend(&digits[1..end as usize]);
            }
            output.zeros(precision + 1 - end.max(1));
        }
        output.push(b'e');
        if count == 0 {
            exponent = 0;
        }
        if exponent < 0 {
            output.push(b'-');
            exponent = -exponent;
        } else {
            output.push(b'+');
        }
        if exponent >= 100 {
            output.push((exponent / 100) as u8 + b'0');
        }
        output.push(((exponent / 10) % 10) as u8 + b'0');
        output.push((exponent % 10) as u8 + b'0');
        return;
    }
    if precision > point {
        precision = count;
    }
    precision = (precision - point).max(0);
    if point > 0 {
        let end = count.min(point);
        output.extend(&digits[..end as usize]);
        output.zeros(point - end);
    } else {
        output.push(b'0');
    }
    if precision > 0 {
        output.push(b'.');
        let leading = precision.min((-point).max(0));
        let offset = point + leading;
        let middle = (precision - leading).min((count - offset).max(0));
        let trailing = (precision - leading - middle).max(0);
        output.zeros(leading);
        output.extend(&digits[offset as usize..(offset + middle) as usize]);
        output.zeros(trailing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_go_general_rows() {
        // Original internal/strconv/ftoa_test.go assertions, compiled as binary64.
        for (value, style, expected) in [
            (200_000.0, FloatTextStyle::Shortest, "200000"),
            (2_000_000.0, FloatTextStyle::Shortest, "2e+06"),
            (400.0, FloatTextStyle::TwoSignificant, "4e+02"),
            (40.0, FloatTextStyle::TwoSignificant, "40"),
            (0.0004, FloatTextStyle::TwoSignificant, "0.0004"),
            (0.00004, FloatTextStyle::TwoSignificant, "4e-05"),
            (1e23, FloatTextStyle::Shortest, "1e+23"),
            (f64::from_bits(1), FloatTextStyle::Shortest, "5e-324"),
        ] {
            assert_eq!(float_text(value, style).as_str(), expected);
        }
    }

    #[test]
    fn source_specials_and_openmetrics_zero_projection() {
        // Captured original Go and Prometheus model/labels/float.go behaviors.
        for (bits, general, metric) in [
            (0, "0", "0.0"),
            (1 << 63, "-0", "0.0"),
            (0x3ff0_0000_0000_0000, "1", "1.0"),
            (0xbff0_0000_0000_0000, "-1", "-1.0"),
            (0x7ff0_0000_0000_0000, "+Inf", "+Inf"),
            (0xfff0_0000_0000_0000, "-Inf", "-Inf"),
            (0xfff0_0000_0000_0044, "NaN", "NaN"),
        ] {
            let value = f64::from_bits(bits);
            assert_eq!(
                float_text(value, FloatTextStyle::Shortest).as_str(),
                general
            );
            assert_eq!(
                float_text(value, FloatTextStyle::TwoSignificant).as_str(),
                general
            );
            assert_eq!(
                float_text(value, FloatTextStyle::OpenMetrics).as_str(),
                metric
            );
        }
    }
}
