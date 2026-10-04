// SPDX-License-Identifier: GPL-3.0-or-later
// Flat Go program to Thompson NFA; construction and execution use heap work stacks.
use regex_automata::{
    MatchKind,
    nfa::thompson::{BuildError, Builder, Transition, pikevm::PikeVM},
    util::{look::Look, primitives::StateID},
};
use regex_syntax::utf8::Utf8Sequences;

use super::{
    program::{Code, Program},
    runes,
};

#[derive(Clone, Copy)]
struct Fragment {
    start: StateID,
    end: StateID,
}

fn class(builder: &mut Builder, ranges: &[(u32, u32)]) -> Result<Fragment, Box<BuildError>> {
    let mut alternatives = Vec::new();
    let exit = builder.add_empty()?;
    for &(lo, hi) in ranges {
        for (lo, hi) in [(lo, hi.min(0xd7ff)), (lo.max(0xe000), hi)] {
            if lo > hi {
                continue;
            }
            for encoding in
                Utf8Sequences::new(char::from_u32(lo).unwrap(), char::from_u32(hi).unwrap())
            {
                let mut first = None;
                let mut previous = None;
                for range in encoding.as_slice() {
                    let state = builder.add_range(Transition {
                        start: range.start,
                        end: range.end,
                        next: StateID::ZERO,
                    })?;
                    if let Some(previous) = previous {
                        builder.patch(previous, state)?;
                    } else {
                        first = Some(state);
                    }
                    previous = Some(state);
                }
                builder.patch(previous.unwrap(), exit)?;
                alternatives.push(first.unwrap());
            }
        }
    }
    if alternatives.is_empty() {
        let fail = builder.add_fail()?;
        return Ok(Fragment {
            start: fail,
            end: fail,
        });
    }
    let start = builder.add_union(alternatives)?;
    Ok(Fragment { start, end: exit })
}

fn look(bits: usize) -> Look {
    match bits {
        1 => Look::StartLF,
        2 => Look::EndLF,
        4 => Look::Start,
        8 => Look::End,
        16 => Look::WordAscii,
        32 => Look::WordAsciiNegate,
        _ => unreachable!("Go assertion"),
    }
}

fn construct(program: &Program) -> Result<(PikeVM, Vec<u8>), Box<BuildError>> {
    let (prefix, prefix_instructions) = program.prefix();
    let mut reachable = vec![false; program.instructions.len()];
    let mut pending = vec![program.start];
    while let Some(pc) = pending.pop() {
        if reachable[pc] {
            continue;
        }
        reachable[pc] = true;
        let instruction = &program.instructions[pc];
        match instruction.code {
            Code::Fail | Code::Match => {}
            Code::Alt => {
                pending.push(instruction.out);
                pending.push(instruction.arg);
            }
            _ => pending.push(instruction.out),
        }
    }
    let mut builder = Builder::new();
    builder.set_utf8(true);
    builder.set_size_limit(None)?;
    assert_eq!(builder.add_fail()?, StateID::ZERO);
    builder.start_pattern()?;
    let capture_zero = builder.add_capture_start(StateID::ZERO, 0, None)?;
    let fail = Fragment {
        start: StateID::ZERO,
        end: StateID::ZERO,
    };
    let mut fragments = vec![fail; program.instructions.len()];
    for (pc, instruction) in program.instructions.iter().enumerate() {
        if !reachable[pc] {
            continue;
        }
        let state = match instruction.code {
            Code::Fail => StateID::ZERO,
            Code::Nop => builder.add_empty()?,
            Code::Alt => builder.add_union(Vec::new())?,
            Code::Empty => builder.add_look(StateID::ZERO, look(instruction.arg))?,
            Code::Capture if instruction.arg % 2 == 0 => {
                builder.add_capture_start(StateID::ZERO, (instruction.arg / 2) as u32, None)?
            }
            Code::Capture => {
                builder.add_capture_end(StateID::ZERO, (instruction.arg / 2) as u32)?
            }
            Code::Match => {
                let end = builder.add_capture_end(StateID::ZERO, 0)?;
                let matched = builder.add_match()?;
                builder.patch(end, matched)?;
                end
            }
            _ => {
                let mut ranges = if instruction.runes.len() == 1 {
                    let rune = instruction.runes[0];
                    let rune =
                        if prefix_instructions.contains(&pc) && (0xd800..=0xdfff).contains(&rune) {
                            0xfffd
                        } else {
                            rune
                        };
                    if instruction.arg & 1 != 0 {
                        runes::folded(vec![(rune, rune)])
                    } else {
                        vec![(rune, rune)]
                    }
                } else {
                    instruction
                        .runes
                        .chunks_exact(2)
                        .map(|pair| (pair[0], pair[1]))
                        .collect()
                };
                ranges = runes::clean(ranges);
                fragments[pc] = class(&mut builder, &ranges)?;
                continue;
            }
        };
        fragments[pc] = Fragment {
            start: state,
            end: state,
        };
    }
    for (pc, instruction) in program.instructions.iter().enumerate() {
        if !reachable[pc] {
            continue;
        }
        match instruction.code {
            Code::Fail | Code::Match => {}
            Code::Alt => {
                builder.patch(fragments[pc].end, fragments[instruction.out].start)?;
                builder.patch(fragments[pc].end, fragments[instruction.arg].start)?;
            }
            _ => builder.patch(fragments[pc].end, fragments[instruction.out].start)?,
        }
    }
    builder.patch(capture_zero, fragments[program.start].start)?;
    builder.finish_pattern(capture_zero)?;
    let unanchored = builder.add_union(vec![capture_zero])?;
    let byte = builder.add_range(Transition {
        start: 0,
        end: 255,
        next: unanchored,
    })?;
    builder.patch(unanchored, byte)?;
    let nfa = builder.build(capture_zero, unanchored)?;
    let vm = PikeVM::builder()
        .configure(PikeVM::config().match_kind(MatchKind::LeftmostFirst))
        .build_from_nfa(nfa)?;
    Ok((vm, prefix))
}

pub fn build(program: &Program) -> Result<(PikeVM, Vec<u8>), Vec<u8>> {
    construct(program)
        .map_err(|error| format!("internal regex backend error: {error}").into_bytes())
}
