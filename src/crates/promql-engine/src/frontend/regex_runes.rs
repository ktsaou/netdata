// SPDX-License-Identifier: GPL-3.0-or-later
// Go-compatible Unicode and character-class operations; see NOTICE.
use super::unicode::{self, Table};
use super::{AllocationContext, Buffer, Work};
use crate::storage::Interrupted;

pub const MAX_RUNE: u32 = 0x10ffff;
pub type Class = Buffer<(u32, u32)>;

pub fn fold(rune: u32) -> u32 {
    unicode::SIMPLE_FOLD
        .binary_search_by_key(&rune, |pair| pair.0)
        .map_or(rune, |index| unicode::SIMPLE_FOLD[index].1)
}

pub fn min_fold(rune: u32) -> u32 {
    let mut result = rune;
    let mut next = fold(rune);
    while next != rune {
        result = result.min(next);
        next = fold(next);
    }
    result
}

pub fn clean(mut ranges: Class, work: Work<'_>) -> Result<Class, Interrupted> {
    work.step(0)?;
    let context = ranges.context();
    work.sort(&mut ranges)?;
    let mut output: Class = Buffer::with_capacity_in(ranges.len(), context.as_ref());
    for (lo, hi) in ranges {
        work.step(1)?;
        if let Some(last) = output.last_mut()
            && lo <= last.1.saturating_add(1)
        {
            last.1 = last.1.max(hi);
        } else {
            output.push((lo, hi));
        }
    }
    Ok(output)
}

pub fn contains(ranges: &[(u32, u32)], rune: u32) -> bool {
    let index = ranges.partition_point(|range| range.1 < rune);
    ranges.get(index).is_some_and(|range| range.0 <= rune)
}

pub fn complement(ranges: Class, work: Work<'_>) -> Result<Class, Interrupted> {
    let ranges = clean(ranges, work)?;
    let mut result = Buffer::with_capacity_in(ranges.len() + 1, ranges.context().as_ref());
    let mut next = 0;
    for (lo, hi) in ranges {
        work.step(1)?;
        if next < lo {
            result.push((next, lo - 1));
        }
        next = hi + 1;
    }
    if next <= MAX_RUNE {
        result.push((next, MAX_RUNE));
    }
    Ok(result)
}

pub fn folded(ranges: Class, work: Work<'_>) -> Result<Class, Interrupted> {
    let mut result = clean(ranges, work)?;
    let mut added = Buffer::new_in(result.context().as_ref());
    for &(rune, next) in unicode::SIMPLE_FOLD {
        work.step(1)?;
        if contains(&result, rune) {
            let mut next = next;
            while next != rune {
                work.step(1)?;
                added.push((next, next));
                next = fold(next);
            }
        }
    }
    work.extend_iter(&mut result, added)?;
    clean(result, work)
}

pub fn table(ranges: Table, work: Work<'_>) -> Result<Class, Interrupted> {
    work.step(0)?;
    let context = work.context;
    let mut result = Buffer::new_in(context);
    for &(lo, hi, stride) in ranges {
        work.step(1)?;
        if stride == 1 {
            result.push((lo, hi));
        } else {
            work.extend_iter(
                &mut result,
                (lo..=hi).step_by(stride as usize).map(|rune| (rune, rune)),
            )?;
        }
    }
    clean(result, work)
}

fn lookup(tables: &[(&str, Table)], name: &[u8]) -> Option<Table> {
    tables
        .binary_search_by(|entry| entry.0.as_bytes().cmp(name))
        .ok()
        .map(|i| tables[i].1)
}

fn canonical_name(name: &[u8], work: Work<'_>) -> Result<Buffer<u8>, Interrupted> {
    let mut result = work.buffer();
    for &byte in name {
        work.step(1)?;
        if matches!(byte, b' ' | b'_' | b'-') {
            continue;
        }
        result.push(if result.is_empty() {
            byte.to_ascii_uppercase()
        } else {
            byte.to_ascii_lowercase()
        });
    }
    Ok(result)
}

pub fn property(
    name: &[u8],
    insensitive: bool,
    standard: bool,
    work: Work<'_>,
) -> Result<Option<Class>, Interrupted> {
    work.step(0)?;
    let context = work.context;
    let normalized = if standard {
        Some(canonical_name(name, work)?)
    } else {
        None
    };
    let mut name = normalized.as_deref().unwrap_or(name);
    if standard {
        match name {
            b"Assigned" => {
                let Some(ranges) = lookup(unicode::CATEGORIES, b"Cn") else {
                    return Ok(None);
                };
                return Ok(Some(complement(table(ranges, work)?, work)?));
            }
            b"Ascii" => {
                return Ok(Some(if insensitive {
                    buffer![context; (0, 0x7f), (0x17f, 0x17f), (0x212a, 0x212a)]
                } else {
                    buffer![context; (0, 0x7f)]
                }));
            }
            b"Lc" => name = b"LC",
            _ => {
                if lookup(unicode::CATEGORIES, name).is_none()
                    && lookup(unicode::SCRIPTS, name).is_none()
                {
                    let mut category = None;
                    for &(alias, value) in unicode::CATEGORY_ALIASES {
                        work.step(1)?;
                        if work.equal(&canonical_name(alias.as_bytes(), work)?, name)? {
                            category = Some(value);
                            break;
                        }
                    }
                    if let Some(value) = category {
                        name = value.as_bytes();
                    } else {
                        for &(script, _) in unicode::SCRIPTS {
                            work.step(1)?;
                            if work.equal(&canonical_name(script.as_bytes(), work)?, name)? {
                                name = script.as_bytes();
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    if name == b"Any" {
        return Ok(Some(buffer![context; (0, MAX_RUNE)]));
    }
    let (ranges, folds) = if let Some(ranges) = lookup(unicode::CATEGORIES, name) {
        (ranges, lookup(unicode::FOLD_CATEGORIES, name))
    } else {
        let Some(ranges) = lookup(unicode::SCRIPTS, name) else {
            return Ok(None);
        };
        (ranges, lookup(unicode::FOLD_SCRIPTS, name))
    };
    let mut result = table(ranges, work)?;
    if insensitive && let Some(folds) = folds {
        work.extend_iter(&mut result, table(folds, work)?)?;
    }
    Ok(Some(clean(result, work)?))
}

pub fn perl(name: u8, context: Option<&AllocationContext>) -> Option<(Class, bool)> {
    let ranges = match name.to_ascii_lowercase() {
        b'd' => buffer![context; (0x30, 0x39)],
        b's' => buffer![context; (9, 10), (12, 13), (32, 32)],
        b'w' => buffer![context; (0x30, 0x39), (0x41, 0x5a), (0x5f, 0x5f), (0x61, 0x7a)],
        _ => return None,
    };
    Some((ranges, name.is_ascii_uppercase()))
}

pub fn posix(name: &[u8], context: Option<&AllocationContext>) -> Option<(Class, bool)> {
    let (name, negate) = name
        .strip_prefix(b"^")
        .map_or((name, false), |name| (name, true));
    let ranges = match name {
        b"alnum" => buffer![context; (0x30, 0x39), (0x41, 0x5a), (0x61, 0x7a)],
        b"alpha" => buffer![context; (0x41, 0x5a), (0x61, 0x7a)],
        b"ascii" => buffer![context; (0, 0x7f)],
        b"blank" => buffer![context; (9, 9), (32, 32)],
        b"cntrl" => buffer![context; (0, 31), (0x7f, 0x7f)],
        b"digit" => buffer![context; (0x30, 0x39)],
        b"graph" => buffer![context; (0x21, 0x7e)],
        b"lower" => buffer![context; (0x61, 0x7a)],
        b"print" => buffer![context; (0x20, 0x7e)],
        b"punct" => buffer![context; (0x21, 0x2f), (0x3a, 0x40), (0x5b, 0x60), (0x7b, 0x7e)],
        b"space" => buffer![context; (9, 13), (32, 32)],
        b"upper" => buffer![context; (0x41, 0x5a)],
        b"word" => perl(b'w', context)?.0,
        b"xdigit" => buffer![context; (0x30, 0x39), (0x41, 0x46), (0x61, 0x66)],
        _ => return None,
    };
    Some((ranges, negate))
}

pub fn text(runes: &[u32], work: Work<'_>) -> Result<Buffer<u8>, Interrupted> {
    let mut result = work.buffer();
    for &rune in runes {
        work.step(1)?;
        let mut buffer = [0; 4];
        result.extend_from_slice(
            char::from_u32(rune)
                .unwrap_or('\u{fffd}')
                .encode_utf8(&mut buffer)
                .as_bytes(),
        );
    }
    Ok(result)
}

pub fn is_letter_or_digit(rune: u32) -> bool {
    [b"L".as_slice(), b"Nd".as_slice()].iter().any(|name| {
        lookup(unicode::CATEGORIES, name).is_some_and(|ranges| {
            let index = ranges.partition_point(|range| range.1 < rune);
            ranges.get(index).is_some_and(|&(lo, hi, stride)| {
                lo <= rune && rune <= hi && (rune - lo).is_multiple_of(stride)
            })
        })
    })
}
