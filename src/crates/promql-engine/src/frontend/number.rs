// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus parse.go and common/model/time.go literal algorithms; see NOTICE.
use super::quote;

#[derive(Debug)]
pub struct Number {
    pub value: f64,
    pub error: Option<Vec<u8>>,
}

fn digit(byte: u8) -> u32 {
    match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'f' => u32::from(byte - b'a' + 10),
        b'A'..=b'F' => u32::from(byte - b'A' + 10),
        _ => 16,
    }
}

fn integer(mut input: &[u8]) -> Option<i64> {
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
        return None;
    }
    let limit = (i64::MAX as u64) + u64::from(negative);
    let mut value = 0u64;
    let mut saw_digit = false;
    for (index, &byte) in digits.iter().enumerate() {
        if byte == b'_' {
            if (index == 0 && prefix == 0)
                || (index > 0 && digit(digits[index - 1]) >= base)
                || digits
                    .get(index + 1)
                    .is_none_or(|&next| digit(next) >= base)
            {
                return None;
            }
            continue;
        }
        let d = digit(byte);
        if d >= base {
            return None;
        }
        value = value
            .checked_mul(u64::from(base))?
            .checked_add(u64::from(d))?;
        if value > limit {
            return None;
        }
        saw_digit = true;
    }
    if !saw_digit {
        return None;
    }
    Some(if negative {
        (value as i64).wrapping_neg()
    } else {
        value as i64
    })
}

fn decimal(input: &[u8]) -> Option<String> {
    let mut position = usize::from(matches!(input.first(), Some(b'+' | b'-')));
    let mut digits = 0;
    let mut out = String::with_capacity(input.len());
    for (index, &byte) in input.iter().enumerate() {
        if byte == b'_' {
            if index == 0
                || !input[index - 1].is_ascii_digit()
                || input.get(index + 1).is_none_or(|b| !b.is_ascii_digit())
            {
                return None;
            }
        } else {
            out.push(char::from(byte));
        }
    }
    let bytes = out.as_bytes();
    while bytes.get(position).is_some_and(u8::is_ascii_digit) {
        position += 1;
        digits += 1;
    }
    if bytes.get(position) == Some(&b'.') {
        position += 1;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(position), Some(b'e' | b'E')) {
        position += 1;
        if matches!(bytes.get(position), Some(b'+' | b'-')) {
            position += 1;
        }
        let start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        if position == start {
            return None;
        }
    }
    (position == bytes.len()).then_some(out)
}

/// Parse a NUMBER token using the upstream integer-first conversion.
pub fn number(input: &[u8]) -> Number {
    if let Some(value) = integer(input) {
        return Number {
            value: value as f64,
            error: None,
        };
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
        return Number { value, error: None };
    }
    let parsed = decimal(input).and_then(|text| text.parse::<f64>().ok());
    let reason = match parsed {
        Some(value) if !value.is_infinite() => return Number { value, error: None },
        Some(_) => "value out of range",
        None => "invalid syntax",
    };
    Number {
        value: parsed.unwrap_or(0.0),
        error: Some(
            format!(
                "error parsing number: strconv.ParseFloat: parsing {}: {reason}",
                quote::bytes(input)
            )
            .into_bytes(),
        ),
    }
}

pub fn duration(input: &[u8]) -> Result<i64, Vec<u8>> {
    if input == b"0" {
        return Ok(0);
    }
    if input.is_empty() {
        return Err(b"empty duration string".to_vec());
    }
    let invalid = || format!("not a valid duration string: {}", quote::bytes(input)).into_bytes();
    let mut rest = input;
    let mut total = 0u64;
    let mut last_unit = 0;
    while !rest.is_empty() {
        let count = rest.iter().take_while(|b| b.is_ascii_digit()).count();
        if count == 0 {
            return Err(invalid());
        }
        let amount = rest[..count]
            .iter()
            .try_fold(0usize, |value, &byte| {
                value.checked_mul(10)?.checked_add(usize::from(byte - b'0'))
            })
            .ok_or_else(&invalid)? as u64;
        rest = &rest[count..];
        let count = rest.iter().take_while(|b| !b.is_ascii_digit()).count();
        if count == 0 {
            return Err(invalid());
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
                return Err(format!(
                    "unknown unit {} in duration {}",
                    quote::bytes(unit),
                    quote::bytes(input)
                )
                .into_bytes());
            }
        };
        if position <= last_unit {
            return Err(invalid());
        }
        last_unit = position;
        if amount > (1u64 << 63) / multiplier {
            return Err(b"duration out of range".to_vec());
        }
        total += amount * multiplier;
        if total > i64::MAX as u64 {
            return Err(b"duration out of range".to_vec());
        }
    }
    Ok(total as i64)
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
