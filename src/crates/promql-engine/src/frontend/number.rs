// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus parse.go and common/model/time.go literal algorithms; see NOTICE.
use super::{float, float::digit, quote, work::Work};
use crate::{accounting::AccountedVec, storage::Interrupted};

#[derive(Debug)]
pub struct Number<E = Vec<u8>> {
    pub value: f64,
    pub error: Option<E>,
}

fn integer(mut input: &[u8], work: Work<'_>) -> Result<Option<i64>, Interrupted> {
    let negative = input.first() == Some(&b'-');
    if matches!(input.first(), Some(b'+' | b'-')) {
        input = &input[1..];
    }
    let (base, prefix) = if input.first() == Some(&b'0') {
        match input.get(1) {
            Some(b'x' | b'X') => (16, 2),
            Some(b'o' | b'O') => (8, 2),
            Some(b'b' | b'B') => (2, 2),
            _ if input.len() > 1 => (8, 1),
            _ => (10, 0),
        }
    } else {
        (10, 0)
    };
    let digits = &input[prefix..];
    if digits.is_empty() {
        return Ok(None);
    }
    let limit = (i64::MAX as u64) + u64::from(negative);
    let mut value = 0u64;
    let mut saw_digit = false;
    for (index, &byte) in digits.iter().enumerate() {
        work.step(1)?;
        if byte == b'_' {
            if (index == 0 && prefix == 0)
                || (index > 0 && digit(digits[index - 1]) >= base)
                || digits
                    .get(index + 1)
                    .is_none_or(|&next| digit(next) >= base)
            {
                return Ok(None);
            }
            continue;
        }
        let d = digit(byte);
        if d >= base {
            return Ok(None);
        }
        let Some(next) = value
            .checked_mul(u64::from(base))
            .and_then(|value| value.checked_add(u64::from(d)))
        else {
            return Ok(None);
        };
        value = next;
        if value > limit {
            return Ok(None);
        }
        saw_digit = true;
    }
    if !saw_digit {
        return Ok(None);
    }
    Ok(Some(if negative {
        (value as i64).wrapping_neg()
    } else {
        value as i64
    }))
}

/// Parse a NUMBER token using the upstream integer-first conversion.
pub fn number(input: &[u8]) -> Number {
    let parsed = number_in(input, Work::default()).expect("inert number parsing");
    Number {
        value: parsed.value,
        error: parsed.error.map(AccountedVec::into_unobserved),
    }
}

pub(super) fn number_in(
    input: &[u8],
    work: Work<'_>,
) -> Result<Number<AccountedVec<u8>>, Interrupted> {
    work.step(0)?;
    if let Some(value) = integer(input, work)? {
        return Ok(Number {
            value: value as f64,
            error: None,
        });
    }
    let parsed = float::parse_checked(input, work.context, work.account, work.control)?;
    let Some(error) = parsed.error else {
        return Ok(Number {
            value: parsed.value,
            error: None,
        });
    };
    let reason = error.description();
    Ok(Number {
        value: parsed.value,
        error: Some(work.format(format_args!(
            "error parsing number: strconv.ParseFloat: parsing {}: {reason}",
            quote::quoted(input)
        ))?),
    })
}

pub fn duration(input: &[u8]) -> Result<i64, Vec<u8>> {
    duration_in(input, Work::default())
        .expect("inert duration parsing")
        .map_err(AccountedVec::into_unobserved)
}

#[cfg(feature = "fixtures")]
pub fn float_fixture(input: &[u8]) -> (f64, &'static str) {
    let parsed = float::parse_checked(input, None, None, None).expect("inert float parsing");
    let error = match parsed.error {
        Some(float::FloatError::Syntax) => "syntax",
        Some(float::FloatError::Range) => "range",
        None => "",
    };
    (parsed.value, error)
}

pub(super) fn duration_in(
    input: &[u8],
    work: Work<'_>,
) -> Result<Result<i64, AccountedVec<u8>>, Interrupted> {
    work.step(0)?;
    if input == b"0" {
        return Ok(Ok(0));
    }
    if input.is_empty() {
        return Ok(Err(work.copy(b"empty duration string")?));
    }
    let invalid = || {
        work.format(format_args!(
            "not a valid duration string: {}",
            quote::quoted(input)
        ))
    };
    let mut rest = input;
    let mut total = 0u64;
    let mut last_unit = 0;
    while !rest.is_empty() {
        work.step(1)?;
        let mut count = 0;
        for &byte in rest {
            work.step(1)?;
            if !byte.is_ascii_digit() {
                break;
            }
            count += 1;
        }
        if count == 0 {
            return Ok(Err(invalid()?));
        }
        let mut amount = 0usize;
        for &byte in &rest[..count] {
            work.step(1)?;
            let Some(next) = amount
                .checked_mul(10)
                .and_then(|value| value.checked_add(usize::from(byte - b'0')))
            else {
                return Ok(Err(invalid()?));
            };
            amount = next;
        }
        let amount = amount as u64;
        rest = &rest[count..];
        let mut count = 0;
        for &byte in rest {
            work.step(1)?;
            if byte.is_ascii_digit() {
                break;
            }
            count += 1;
        }
        if count == 0 {
            return Ok(Err(invalid()?));
        }
        let unit = &rest[..count];
        rest = &rest[count..];
        let (position, multiplier) = match unit {
            b"y" => (1, 31_536_000_000_000_000u64),
            b"w" => (2, 604_800_000_000_000),
            b"d" => (3, 86_400_000_000_000),
            b"h" => (4, 3_600_000_000_000),
            b"m" => (5, 60_000_000_000),
            b"s" => (6, 1_000_000_000),
            b"ms" => (7, 1_000_000),
            _ => {
                return Ok(Err(work.format(format_args!(
                    "unknown unit {} in duration {}",
                    quote::quoted(unit),
                    quote::quoted(input)
                ))?));
            }
        };
        if position <= last_unit {
            return Ok(Err(invalid()?));
        }
        last_unit = position;
        if amount > (1u64 << 63) / multiplier {
            return Ok(Err(work.copy(b"duration out of range")?));
        }
        total += amount * multiplier;
        if total > i64::MAX as u64 {
            return Ok(Err(work.copy(b"duration out of range")?));
        }
    }
    Ok(Ok(total as i64))
}

pub fn seconds(nanoseconds: i64) -> f64 {
    let whole = nanoseconds / 1_000_000_000;
    let remainder = nanoseconds % 1_000_000_000;
    whole as f64 + remainder as f64 / 1_000_000_000.0
}

pub fn duration_out_of_range(seconds: f64) -> bool {
    seconds > (1u64 << 63) as f64 / 1_000_000_000.0
        || seconds < -((1u64 << 63) as f64) / 1_000_000_000.0
}

/// Pinned Go/amd64 conversion returns MinInt64 for a nonrepresentable value.
pub fn go_i64(value: f64) -> i64 {
    if !value.is_finite() || !(-((1u64 << 63) as f64)..(1u64 << 63) as f64).contains(&value) {
        i64::MIN
    } else {
        value as i64
    }
}

pub fn nanoseconds(seconds: f64) -> i64 {
    go_i64((seconds * 1_000_000_000.0).round())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_first_numbers_fall_back_to_hex_floats() {
        // Go ParseInt fails on these; ParseFloat supplies the value and rounding.
        for (input, bits) in [
            (b"0x1p0".as_slice(), 0x3ff0_0000_0000_0000),
            (b"0x1.000000000000081p0", 0x3ff0_0000_0000_0001),
            (b"-0x0p999999999999999", 0x8000_0000_0000_0000),
            (b"0x_1p0", 0x3ff0_0000_0000_0000),
        ] {
            let parsed = number(input);
            assert_eq!(parsed.value.to_bits(), bits, "{input:?}");
            assert_eq!(parsed.error, None, "{input:?}");
        }
        assert_eq!(number(b"010").value, 8.0);
        assert_eq!(number(b"-0").value.to_bits(), 0);
        assert_eq!(number(b"0x10").value, 16.0);
    }
}
