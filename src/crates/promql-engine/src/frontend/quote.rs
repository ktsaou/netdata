// SPDX-License-Identifier: GPL-3.0-or-later
// Go-compatible diagnostics, derived from Go 1.27.1 strconv/quote.go and fmt/format.go.
// Copyright 2009 The Go Authors. See LICENSE-GO-BSD.
use super::unicode::is_print;

pub(super) fn decode_rune(bytes: &[u8]) -> (i32, usize) {
    let Some(&first) = bytes.first() else {
        return (-1, 0);
    };
    if first < 0x80 {
        return (i32::from(first), 1);
    }
    let width = match first {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 1,
    };
    if width > 1
        && bytes.len() >= width
        && let Ok(text) = std::str::from_utf8(&bytes[..width])
    {
        return (text.chars().next().unwrap() as i32, width);
    }
    (0xfffd, 1)
}

fn escaped(r: u32, delimiter: char) -> String {
    if r == delimiter as u32 || r == b'\\' as u32 {
        return format!("\\{}", char::from_u32(r).unwrap());
    }
    if is_print(r) {
        return char::from_u32(r).unwrap().to_string();
    }
    match r {
        7 => "\\a".into(),
        8 => "\\b".into(),
        12 => "\\f".into(),
        10 => "\\n".into(),
        13 => "\\r".into(),
        9 => "\\t".into(),
        11 => "\\v".into(),
        0..=31 | 0x7f => format!("\\x{r:02x}"),
        0..=0xffff => format!("\\u{r:04x}"),
        _ => format!("\\U{r:08x}"),
    }
}

pub(super) fn rune(r: i32) -> String {
    let r = char::from_u32(r as u32).map_or(0xfffd, |c| c as u32);
    format!("'{}'", escaped(r, '\''))
}

pub(super) fn unicode_rune(r: i32) -> String {
    if r >= 0 && is_print(r as u32) {
        format!("U+{:04X} '{}'", r, char::from_u32(r as u32).unwrap())
    } else {
        format!("U+{:04X}", i64::from(r) as u64)
    }
}

pub(crate) fn bytes(mut bytes: &[u8]) -> String {
    let mut out = String::from("\"");
    while !bytes.is_empty() {
        let (r, width) = decode_rune(bytes);
        if r == 0xfffd && width == 1 {
            out.push_str(&format!("\\x{:02x}", bytes[0]));
        } else {
            out.push_str(&escaped(r as u32, '"'));
        }
        bytes = &bytes[width..];
    }
    out.push('"');
    out
}
