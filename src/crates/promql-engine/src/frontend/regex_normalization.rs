// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus map normalization: NFKD followed by Go Unicode ToLower.
use super::normalization_data::{self, Entry};
use super::unicode;
use super::{AllocationContext, Buffer};
use crate::frontend::quote::decode_rune;

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

pub fn lower(input: &[u8], context: Option<&AllocationContext>) -> Buffer<u8> {
    let mut output = Buffer::with_capacity_in(input.len(), context);
    let mut position = 0;
    while position < input.len() {
        let (rune, width) = decode_rune(&input[position..]);
        append(&mut output, lower_rune(rune as u32));
        position += width;
    }
    output
}

fn flush(segment: &mut Buffer<(u32, u8)>, output: &mut Buffer<u8>) {
    for (rune, _) in segment.drain(..) {
        append(output, lower_rune(rune));
    }
}

fn insert(rune: u32, segment: &mut Buffer<(u32, u8)>, output: &mut Buffer<u8>) {
    let ccc = property(rune).map_or(0, |entry| entry.1);
    if ccc == 0 {
        flush(segment, output);
        segment.push((rune, ccc));
    } else {
        let position = segment
            .iter()
            .rposition(|entry| entry.1 <= ccc)
            .map_or(0, |position| position + 1);
        segment.insert(position, (rune, ccc));
    }
}

pub fn normalized_lower(input: &[u8], context: Option<&AllocationContext>) -> Buffer<u8> {
    if input.is_ascii() {
        return Buffer::from_iter_in(input.iter().map(u8::to_ascii_lowercase), context);
    }
    let mut output = Buffer::with_capacity_in(input.len(), context);
    let mut segment = Buffer::new_in(context);
    let mut nonstarters = 0u8;
    let mut position = 0;
    while position < input.len() {
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
            insert(0x34f, &mut segment, &mut output);
            nonstarters = 0;
        }
        nonstarters = if leading == 0 {
            trailing
        } else {
            nonstarters + leading
        };
        if hangul {
            let index = rune - 0xac00;
            insert(0x1100 + index / 588, &mut segment, &mut output);
            insert(0x1161 + (index % 588) / 28, &mut segment, &mut output);
            if !index.is_multiple_of(28) {
                insert(0x11a7 + index % 28, &mut segment, &mut output);
            }
        } else if let Some(entry) = entry.filter(|entry| !entry.4.is_empty()) {
            for &decomposed in entry.4 {
                insert(decomposed, &mut segment, &mut output);
            }
        } else {
            insert(rune, &mut segment, &mut output);
        }
    }
    flush(&mut segment, &mut output);
    output
}
