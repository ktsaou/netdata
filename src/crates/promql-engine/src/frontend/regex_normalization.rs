// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus map normalization: NFKD followed by Go Unicode ToLower.
use super::normalization_data::{self, Entry};
use super::unicode;
use super::{Buffer, Work};
use crate::frontend::quote::decode_rune;
use crate::storage::Interrupted;

fn property(rune: u32) -> Option<&'static Entry> {
    normalization_data::NFKD
        .binary_search_by_key(&rune, |entry| entry.0)
        .ok()
        .map(|index| &normalization_data::NFKD[index])
}

fn lower_rune(rune: u32) -> u32 {
    unicode::LOWER
        .binary_search_by_key(&rune, |entry| entry.0)
        .map_or(rune, |index| unicode::LOWER[index].1)
}

fn append(output: &mut Buffer<u8>, rune: u32) {
    let mut buffer = [0; 4];
    output.extend_from_slice(
        char::from_u32(rune)
            .unwrap()
            .encode_utf8(&mut buffer)
            .as_bytes(),
    );
}

pub fn lower(input: &[u8], work: Work<'_>) -> Result<Buffer<u8>, Interrupted> {
    work.step(0)?;
    let context = work.context;
    let mut output = Buffer::with_capacity_in(input.len(), context);
    let mut position = 0;
    while position < input.len() {
        work.step(1)?;
        let (rune, width) = decode_rune(&input[position..]);
        append(&mut output, lower_rune(rune as u32));
        position += width;
    }
    Ok(output)
}

fn flush(
    segment: &mut Buffer<(u32, u8)>,
    output: &mut Buffer<u8>,
    work: Work<'_>,
) -> Result<(), Interrupted> {
    for (rune, _) in segment.drain(..) {
        work.step(1)?;
        append(output, lower_rune(rune));
    }
    Ok(())
}

fn insert(
    rune: u32,
    segment: &mut Buffer<(u32, u8)>,
    output: &mut Buffer<u8>,
    work: Work<'_>,
) -> Result<(), Interrupted> {
    work.step(1)?;
    let ccc = property(rune).map_or(0, |entry| entry.1);
    if ccc == 0 {
        flush(segment, output, work)?;
        segment.push((rune, ccc));
    } else {
        let mut position = 0;
        for index in (0..segment.len()).rev() {
            work.step(1)?;
            if segment[index].1 <= ccc {
                position = index + 1;
                break;
            }
        }
        segment.push((rune, ccc));
        for index in (position..segment.len() - 1).rev() {
            work.step(1)?;
            segment.swap(index, index + 1);
        }
    }
    Ok(())
}

pub fn normalized_lower(input: &[u8], work: Work<'_>) -> Result<Buffer<u8>, Interrupted> {
    work.step(0)?;
    let context = work.context;
    let mut ascii = true;
    for chunk in input.chunks(1024) {
        work.step(chunk.len())?;
        if !chunk.is_ascii() {
            ascii = false;
            break;
        }
    }
    if ascii {
        return work.collect(input.iter().map(u8::to_ascii_lowercase));
    }
    let mut output = Buffer::with_capacity_in(input.len(), context);
    let mut segment = Buffer::new_in(context);
    let mut nonstarters = 0u8;
    let mut position = 0;
    while position < input.len() {
        work.step(1)?;
        let (rune, width) = decode_rune(&input[position..]);
        let rune = rune as u32;
        position += width;
        let entry = property(rune);
        let hangul = (0xac00..=0xd7a3).contains(&rune);
        let (leading, trailing) = if hangul {
            (0, 1 + u8::from(!(rune - 0xac00).is_multiple_of(28)))
        } else {
            entry.map_or((0, 0), |entry| (entry.2, entry.3))
        };
        // Go inserts CGJ before the original rune, before expanding its decomposition.
        if nonstarters + leading > 30 {
            insert(0x34f, &mut segment, &mut output, work)?;
            nonstarters = 0;
        }
        nonstarters = if leading == 0 {
            trailing
        } else {
            nonstarters + leading
        };
        if hangul {
            let index = rune - 0xac00;
            insert(0x1100 + index / 588, &mut segment, &mut output, work)?;
            insert(0x1161 + (index % 588) / 28, &mut segment, &mut output, work)?;
            if !index.is_multiple_of(28) {
                insert(0x11a7 + index % 28, &mut segment, &mut output, work)?;
            }
        } else if let Some(entry) = entry.filter(|entry| !entry.4.is_empty()) {
            for &decomposed in entry.4 {
                work.step(1)?;
                insert(decomposed, &mut segment, &mut output, work)?;
            }
        } else {
            insert(rune, &mut segment, &mut output, work)?;
        }
    }
    flush(&mut segment, &mut output, work)?;
    Ok(output)
}
