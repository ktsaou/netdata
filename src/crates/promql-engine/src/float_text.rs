// SPDX-License-Identifier: GPL-3.0-or-later
// Go internal/strconv/ftoa.go and Prometheus model/labels/float.go; see NOTICE.
use std::fmt;

use crate::frontend::float_scale::{fixed_two, num_digits, shortest};
use crate::{
    accounting::{AccountedVec, QueryAccounting},
    result::append_bytes_checked,
    storage::{Control, Interrupted},
};

/// Source projections used by PromQL numeric annotations and generated labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FloatTextStyle {
    /// Go's shortest lowercase g representation, preserving signed zero.
    Shortest,
    /// Go's lowercase g representation rounded to two significant digits.
    TwoSignificant,
    /// Prometheus label formatting, with decimal zero and integer suffixes.
    OpenMetrics,
    /// Prometheus API sample text, before the surrounding JSON quotes.
    PrometheusApi,
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
    let decimal = Decimal::new(value, style != FloatTextStyle::TwoSignificant);
    if let Some(special) = decimal.special {
        output.extend(special);
        return output;
    }
    general(
        &mut output,
        decimal.negative && !(value == 0.0 && style == FloatTextStyle::OpenMetrics),
        &decimal.digits[..decimal.count],
        decimal.point,
        if style == FloatTextStyle::TwoSignificant {
            2
        } else {
            decimal.count as i32
        },
        style != FloatTextStyle::TwoSignificant,
        if style == FloatTextStyle::PrometheusApi {
            Some(value != 0.0 && (value.abs() < 1e-6 || value.abs() >= 1e21))
        } else {
            None
        },
    );
    if style == FloatTextStyle::OpenMetrics
        && !output.as_bytes().iter().any(|&b| b == b'.' || b == b'e')
    {
        output.extend(b".0");
    }
    output
}

struct Decimal {
    digits: [u8; 20],
    count: usize,
    point: i32,
    negative: bool,
    special: Option<&'static [u8]>,
}
impl Decimal {
    fn new(value: f64, is_shortest: bool) -> Self {
        let bits = value.to_bits();
        let negative = bits >> 63 != 0;
        let mut exponent = ((bits >> 52) & 0x7ff) as i32;
        let mut mantissa = bits & ((1 << 52) - 1);
        if exponent == 0x7ff {
            return Self {
                digits: [0; 20],
                count: 0,
                point: 0,
                negative,
                special: Some(if mantissa != 0 {
                    b"NaN"
                } else if negative {
                    b"-Inf"
                } else {
                    b"+Inf"
                }),
            };
        }
        if exponent == 0 && mantissa == 0 {
            return Self {
                digits: [b'0'; 20],
                count: 1,
                point: 1,
                negative,
                special: None,
            };
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
        Self {
            digits,
            count,
            point,
            negative,
            special: None,
        }
    }
}

/// Go's shortest fixed notation writes padding directly into its owned destination.
pub(crate) fn append_fixed(
    output: &mut AccountedVec<u8>,
    value: f64,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    control.check()?;
    account.work(1);
    render_fixed(&Decimal::new(value, true), |bytes| {
        append_bytes_checked(output, bytes, account, control)
    })
}

pub(crate) fn fixed_text_into(value: f64, target: &mut [u8]) -> Result<usize, std::fmt::Error> {
    let mut len = 0;
    render_fixed(&Decimal::new(value, true), |bytes| {
        let end = len + bytes.len();
        let output = target.get_mut(len..end).ok_or(std::fmt::Error)?;
        output.copy_from_slice(bytes);
        len = end;
        Ok(())
    })?;
    Ok(len)
}

fn render_fixed<E>(
    decimal: &Decimal,
    mut append: impl FnMut(&[u8]) -> Result<(), E>,
) -> Result<(), E> {
    if let Some(special) = decimal.special {
        return append(special);
    }
    if decimal.negative {
        append(b"-")?;
    }
    let digits = &decimal.digits[..decimal.count];
    let point = decimal.point;
    if point > 0 {
        let end = digits.len().min(point as usize);
        append(&digits[..end])?;
        fixed_zeros(point as usize - end, &mut append)?;
    } else {
        append(b"0")?;
    }
    if decimal.count as i32 > point {
        append(b".")?;
        fixed_zeros((-point).max(0) as usize, &mut append)?;
        append(&digits[point.max(0) as usize..])?;
    }
    Ok(())
}
fn fixed_zeros<E>(
    mut count: usize,
    append: &mut impl FnMut(&[u8]) -> Result<(), E>,
) -> Result<(), E> {
    const ZEROS: &[u8] = &[b'0'; 32];
    while count != 0 {
        let next = count.min(ZEROS.len());
        append(&ZEROS[..next])?;
        count -= next;
    }
    Ok(())
}

fn general(
    output: &mut FloatText,
    negative: bool,
    digits: &[u8],
    point: i32,
    mut precision: i32,
    shortest: bool,
    scientific: Option<bool>,
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
    if scientific.unwrap_or(exponent < -4 || exponent >= exponent_precision) {
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

    #[test]
    fn fixed_labels_preserve_specials_and_pad_without_exponent_notation() {
        use crate::accounting::{EngineAccounting, MemoryKind};
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        for (value, expected) in [
            (0.0, "0"),
            (-0.0, "-0"),
            (1e-7, "0.0000001"),
            (2_000_000.0, "2000000"),
            (f64::INFINITY, "+Inf"),
            (f64::NEG_INFINITY, "-Inf"),
            (f64::from_bits(0xfff8_0000_0000_0042), "NaN"),
        ] {
            let mut output = AccountedVec::new(&account, MemoryKind::Workspace);
            append_fixed(&mut output, value, &control, &account).unwrap();
            assert_eq!(&*output, expected.as_bytes());
        }
        let mut output = AccountedVec::new(&account, MemoryKind::Workspace);
        append_fixed(
            &mut output,
            f64::from_bits((1 << 63) | 1),
            &control,
            &account,
        )
        .unwrap();
        let expected = format!("-0.{}5", "0".repeat(323));
        assert_eq!(&*output, expected.as_bytes());
        assert_eq!(output.len(), 327);
        drop(output);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}
