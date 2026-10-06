// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus parse.go and common/model/time.go literal algorithms; see NOTICE.
use super::{quote, work::Work};
use crate::{accounting::AccountedVec, storage::Interrupted};

#[derive(Debug)]
pub struct Number<E = Vec<u8>> {
    pub value: f64,
    pub error: Option<E>,
}

fn digit(byte: u8) -> u32 {
    match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'f' => u32::from(byte - b'a' + 10),
        b'A'..=b'F' => u32::from(byte - b'A' + 10),
        _ => 16,
    }
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

fn decimal(input: &[u8], work: Work<'_>) -> Result<Option<AccountedVec<u8>>, Interrupted> {
    let mut position = usize::from(matches!(input.first(), Some(b'+' | b'-')));
    let mut digits = 0;
    let mut out = AccountedVec::with_capacity_in(input.len(), work.context);
    for (index, &byte) in input.iter().enumerate() {
        work.step(1)?;
        if byte == b'_' {
            if index == 0
                || !input[index - 1].is_ascii_digit()
                || input.get(index + 1).is_none_or(|b| !b.is_ascii_digit())
            {
                return Ok(None);
            }
        } else {
            let mut encoded = [0; 4];
            out.extend_from_slice(char::from(byte).encode_utf8(&mut encoded).as_bytes());
        }
    }
    let bytes = &*out;
    while bytes.get(position).is_some_and(u8::is_ascii_digit) {
        work.step(1)?;
        position += 1;
        digits += 1;
    }
    if bytes.get(position) == Some(&b'.') {
        position += 1;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            work.step(1)?;
            position += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return Ok(None);
    }
    if matches!(bytes.get(position), Some(b'e' | b'E')) {
        position += 1;
        if matches!(bytes.get(position), Some(b'+' | b'-')) {
            position += 1;
        }
        let start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            work.step(1)?;
            position += 1;
        }
        if position == start {
            return Ok(None);
        }
    }
    Ok((position == bytes.len()).then_some(out))
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
    let special = if input.eq_ignore_ascii_case(b"nan") {
        Some(f64::from_bits(0x7ff8_0000_0000_0001))
    } else if [b"inf".as_slice(), b"infinity", b"+inf", b"+infinity"]
        .iter()
        .any(|name| input.eq_ignore_ascii_case(name))
    {
        Some(f64::INFINITY)
    } else if [b"-inf".as_slice(), b"-infinity"]
        .iter()
        .any(|name| input.eq_ignore_ascii_case(name))
    {
        Some(f64::NEG_INFINITY)
    } else {
        None
    };
    if let Some(value) = special {
        return Ok(Number { value, error: None });
    }
    let parsed = decimal(input, work)?.and_then(|text| {
        std::str::from_utf8(&text)
            .expect("encoded decimal bytes")
            .parse::<f64>()
            .ok()
    });
    work.step(0)?;
    let reason = match parsed {
        Some(value) if !value.is_infinite() => return Ok(Number { value, error: None }),
        Some(_) => "value out of range",
        None => "invalid syntax",
    };
    Ok(Number {
        value: parsed.unwrap_or(0.0),
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
