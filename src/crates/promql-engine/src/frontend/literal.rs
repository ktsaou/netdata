// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from prometheus/prometheus @ 5241a27fe3c6983549fccc32f6e65917408c63cd,
// util/strutil/quote.go. See NOTICE, LICENSE-APACHE-2.0 and LICENSE-GO-BSD.
use std::{borrow::Cow, fmt};

use super::{bytes::Bytes, quote::decode_rune, work::Work};
use crate::storage::Interrupted;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyntaxError;

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid syntax")
    }
}
impl std::error::Error for SyntaxError {}

fn hex(byte: u8) -> Result<u32, SyntaxError> {
    match byte {
        b'0'..=b'9' => Ok(u32::from(byte - b'0')),
        b'a'..=b'f' => Ok(u32::from(byte - b'a' + 10)),
        b'A'..=b'F' => Ok(u32::from(byte - b'A' + 10)),
        _ => Err(SyntaxError),
    }
}

/// Decode Go-style byte escapes without converting them to Unicode characters.
pub fn unquote(input: &[u8]) -> Result<Cow<'_, [u8]>, SyntaxError> {
    unquote_in(input, Work::default())
        .expect("inert literal decoding")
        .map(|value| match value {
            Bytes::Borrowed(bytes) => Cow::Borrowed(bytes),
            Bytes::Owned(bytes) => Cow::Owned(bytes.into_unobserved()),
        })
}

pub(super) fn unquote_in<'a>(
    input: &'a [u8],
    work: Work<'_>,
) -> Result<Result<Bytes<'a>, SyntaxError>, Interrupted> {
    work.step(0)?;
    if input.len() < 2 {
        return Ok(Err(SyntaxError));
    }
    let delimiter = input[0];
    if input.last() != Some(&delimiter) {
        return Ok(Err(SyntaxError));
    }
    let body = &input[1..input.len() - 1];
    if delimiter == b'`' {
        return if work.contains(body, b'`')? {
            Ok(Err(SyntaxError))
        } else {
            Ok(Ok(Bytes::Borrowed(body)))
        };
    }
    if !matches!(delimiter, b'\'' | b'"') || work.contains(body, b'\n')? {
        return Ok(Err(SyntaxError));
    }
    if !work.contains(body, b'\\')? && !work.contains(body, delimiter)? {
        return Ok(Ok(Bytes::Borrowed(body)));
    }
    let mut rest = body;
    let mut out = work.buffer();
    while let Some(&first) = rest.first() {
        work.step(1)?;
        if first == delimiter {
            return Ok(Err(SyntaxError));
        }
        if first >= 0x80 {
            let (r, width) = decode_rune(rest);
            let mut encoded = [0; 4];
            out.extend_from_slice(
                char::from_u32(r as u32)
                    .unwrap()
                    .encode_utf8(&mut encoded)
                    .as_bytes(),
            );
            rest = &rest[width..];
            continue;
        }
        if first != b'\\' {
            out.push(first);
            rest = &rest[1..];
            continue;
        }
        let Some(&code) = rest.get(1) else {
            return Ok(Err(SyntaxError));
        };
        rest = &rest[2..];
        match code {
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(10),
            b'r' => out.push(13),
            b't' => out.push(9),
            b'v' => out.push(11),
            b'\\' => out.push(b'\\'),
            b'\'' | b'"' if code == delimiter => out.push(code),
            b'x' | b'u' | b'U' => {
                let count = match code {
                    b'x' => 2,
                    b'u' => 4,
                    _ => 8,
                };
                if rest.len() < count {
                    return Ok(Err(SyntaxError));
                }
                let mut value = 0u32;
                for &digit in &rest[..count] {
                    let Ok(digit) = hex(digit) else {
                        return Ok(Err(SyntaxError));
                    };
                    value = (value << 4) | digit;
                }
                rest = &rest[count..];
                if code == b'x' {
                    out.push(value as u8);
                } else {
                    if value > 0x10ffff {
                        return Ok(Err(SyntaxError));
                    }
                    let mut encoded = [0; 4];
                    out.extend_from_slice(
                        char::from_u32(value)
                            .unwrap_or('\u{fffd}')
                            .encode_utf8(&mut encoded)
                            .as_bytes(),
                    );
                }
            }
            b'0'..=b'7' => {
                if rest.len() < 2 {
                    return Ok(Err(SyntaxError));
                }
                let mut value = u32::from(code - b'0');
                for &digit in &rest[..2] {
                    if !matches!(digit, b'0'..=b'7') {
                        return Ok(Err(SyntaxError));
                    }
                    value = (value << 3) | u32::from(digit - b'0');
                }
                if value > 255 {
                    return Ok(Err(SyntaxError));
                }
                out.push(value as u8);
                rest = &rest[2..];
            }
            _ => return Ok(Err(SyntaxError)),
        }
    }
    Ok(Ok(Bytes::Owned(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_escapes_preserve_non_utf8_and_nul() {
        assert_eq!(
            unquote(br#""\xFF\377\000\u00FF""#).unwrap().as_ref(),
            &[255, 255, 0, 195, 191]
        );
        assert_eq!(
            unquote(br#"'\xFF\377\000\u00FF'"#).unwrap().as_ref(),
            &[255, 255, 0, 195, 191]
        );
    }

    #[test]
    fn raw_strings_preserve_cr_and_backslashes() {
        assert_eq!(unquote(b"`a\r\nb\\c`").unwrap().as_ref(), b"a\r\nb\\c");
    }

    #[test]
    fn invalid_literals_fail() {
        for value in [
            &b""[..],
            b"'",
            b"'a\"",
            b"'a\nb'",
            br#""\q""#,
            br#""\xF""#,
            br#""\400""#,
            br#""\U00110000""#,
            br#""\'""#,
        ] {
            assert_eq!(unquote(value), Err(SyntaxError));
        }
    }
}
