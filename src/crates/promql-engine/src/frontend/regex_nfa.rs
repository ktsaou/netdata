// SPDX-License-Identifier: GPL-3.0-or-later
// Flat Go program to Thompson NFA; construction and execution use heap work stacks.
use regex_automata::{
    MatchKind,
    nfa::thompson::{BuildError, Builder, Transition, pikevm::PikeVM},
    util::{look::Look, primitives::StateID},
};
use regex_syntax::utf8::Utf8Sequences;
use std::fmt::Write as _;

use super::{
    AllocationContext, Buffer,
    program::{Code, Program},
    runes,
};
use crate::accounting::{MemoryKind, MemoryLease};

macro_rules! change {
    ($builder:ident, $method:ident($($argument:expr),* $(,)?)) => {
        $builder.change(|value| value.$method($($argument),*).map_err(Box::new))
    };
}

struct ObservedBuilder {
    value: Builder,
    estimate: Option<MemoryLease>,
}
impl ObservedBuilder {
    fn new(context: Option<&AllocationContext>) -> Self {
        Self {
            value: Builder::new(),
            estimate: context.map(|context| context.with_kind(MemoryKind::RegexEstimate).hold(0)),
        }
    }
    fn change<R>(&mut self, operation: impl FnOnce(&mut Builder) -> R) -> R {
        let result = operation(&mut self.value);
        if let Some(estimate) = &mut self.estimate {
            let bytes = self.value.memory_usage() as u64;
            if bytes != estimate.bytes() {
                estimate.resize(bytes);
            }
        }
        result
    }
}

#[derive(Clone, Copy)]
struct Fragment {
    start: StateID,
    end: StateID,
}

fn class(
    builder: &mut ObservedBuilder,
    ranges: &[(u32, u32)],
    context: Option<&AllocationContext>,
) -> Result<Fragment, Box<BuildError>> {
    let mut alternatives = Buffer::new_in(context);
    let exit = change!(builder, add_empty())?;
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
                    let state = change!(
                        builder,
                        add_range(Transition {
                            start: range.start,
                            end: range.end,
                            next: StateID::ZERO,
                        })
                    )?;
                    if let Some(previous) = previous {
                        change!(builder, patch(previous, state))?;
                    } else {
                        first = Some(state);
                    }
                    previous = Some(state);
                }
                change!(builder, patch(previous.unwrap(), exit))?;
                alternatives.push(first.unwrap());
            }
        }
    }
    if alternatives.is_empty() {
        let fail = change!(builder, add_fail())?;
        return Ok(Fragment {
            start: fail,
            end: fail,
        });
    }
    let start =
        alternatives.transfer_to_dependency(|values| change!(builder, add_union(values)))?;
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

type Built = (PikeVM, Buffer<u8>, Option<MemoryLease>);

fn construct(program: &Program) -> Result<Built, Box<BuildError>> {
    let context = program.instructions.context();
    let (prefix, prefix_instructions) = program.prefix();
    let mut reachable = buffer![context.as_ref(); false; program.instructions.len()];
    let mut pending = buffer![context.as_ref(); program.start];
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
    let mut builder = ObservedBuilder::new(context.as_ref());
    builder.change(|b| {
        b.set_utf8(true);
    });
    change!(builder, set_size_limit(None))?;
    assert_eq!(change!(builder, add_fail())?, StateID::ZERO);
    change!(builder, start_pattern())?;
    let capture_zero = change!(builder, add_capture_start(StateID::ZERO, 0, None))?;
    let fail = Fragment {
        start: StateID::ZERO,
        end: StateID::ZERO,
    };
    let mut fragments = buffer![context.as_ref(); fail; program.instructions.len()];
    for (pc, instruction) in program.instructions.iter().enumerate() {
        if !reachable[pc] {
            continue;
        }
        let state = match instruction.code {
            Code::Fail => StateID::ZERO,
            Code::Nop => change!(builder, add_empty())?,
            Code::Alt => change!(builder, add_union(Vec::new()))?,
            Code::Empty => change!(builder, add_look(StateID::ZERO, look(instruction.arg)))?,
            Code::Capture if instruction.arg % 2 == 0 => change!(
                builder,
                add_capture_start(StateID::ZERO, (instruction.arg / 2) as u32, None)
            )?,
            Code::Capture => change!(
                builder,
                add_capture_end(StateID::ZERO, (instruction.arg / 2) as u32)
            )?,
            Code::Match => {
                let end = change!(builder, add_capture_end(StateID::ZERO, 0))?;
                let matched = change!(builder, add_match())?;
                change!(builder, patch(end, matched))?;
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
                        runes::folded(buffer![context.as_ref(); (rune, rune)])
                    } else {
                        buffer![context.as_ref(); (rune, rune)]
                    }
                } else {
                    Buffer::from_iter_in(
                        instruction
                            .runes
                            .chunks_exact(2)
                            .map(|pair| (pair[0], pair[1])),
                        context.as_ref(),
                    )
                };
                ranges = runes::clean(ranges);
                fragments[pc] = class(&mut builder, &ranges, context.as_ref())?;
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
                change!(
                    builder,
                    patch(fragments[pc].end, fragments[instruction.out].start)
                )?;
                change!(
                    builder,
                    patch(fragments[pc].end, fragments[instruction.arg].start)
                )?;
            }
            _ => change!(
                builder,
                patch(fragments[pc].end, fragments[instruction.out].start)
            )?,
        }
    }
    change!(builder, patch(capture_zero, fragments[program.start].start))?;
    change!(builder, finish_pattern(capture_zero))?;
    let unanchored = buffer![context.as_ref(); capture_zero]
        .transfer_to_dependency(|values| change!(builder, add_union(values)))?;
    let byte = change!(
        builder,
        add_range(Transition {
            start: 0,
            end: 255,
            next: unanchored,
        })
    )?;
    change!(builder, patch(unanchored, byte))?;
    let nfa = change!(builder, build(capture_zero, unanchored))?;
    let estimate = context.as_ref().map(|context| {
        context
            .with_kind(MemoryKind::RegexEstimate)
            .hold(nfa.memory_usage() as u64)
    });
    let vm = PikeVM::builder()
        .configure(PikeVM::config().match_kind(MatchKind::LeftmostFirst))
        .build_from_nfa(nfa)?;
    Ok((vm, prefix, estimate))
}

pub fn build(program: &Program) -> Result<Built, Buffer<u8>> {
    construct(program).map_err(|error| {
        let mut bytes = Buffer::new_in(program.instructions.context().as_ref());
        write!(bytes, "internal regex backend error: {error}").unwrap();
        bytes
    })
}
