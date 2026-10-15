// SPDX-License-Identifier: GPL-3.0-or-later
// Flat Go program to Thompson NFA; construction and execution use heap work stacks.
use crate::storage::Interrupted;
use regex_automata::{
    MatchKind,
    nfa::thompson::{BuildError, Builder, Transition, pikevm::PikeVM},
    util::{look::Look, primitives::StateID},
};
use regex_syntax::utf8::Utf8Sequences;

use super::{
    Buffer, Failure, Work,
    program::{Code, Program},
    runes,
};
use crate::accounting::{MemoryKind, MemoryLease};

macro_rules! change {
    ($builder:ident, $method:ident($($argument:expr),* $(,)?)) => {
        $builder.change(|value| value.$method($($argument),*).map_err(Box::new))?.map_err(Failure::Error)
    };
}

struct ObservedBuilder<'a> {
    work: Work<'a>,
    value: Builder,
    estimate: Option<MemoryLease>,
}
impl<'a> ObservedBuilder<'a> {
    fn new(work: Work<'a>) -> Self {
        let context = work.context;
        Self {
            work,
            value: Builder::new(),
            estimate: context.map(|context| context.with_kind(MemoryKind::RegexEstimate).hold(0)),
        }
    }
    fn change<R>(&mut self, operation: impl FnOnce(&mut Builder) -> R) -> Result<R, Interrupted> {
        self.work.step(0)?;
        let result = operation(&mut self.value);
        if let Some(estimate) = &mut self.estimate {
            let bytes = self.value.memory_usage() as u64;
            if bytes != estimate.bytes() {
                estimate.resize(bytes);
            }
        }
        self.work.step(0)?;
        Ok(result)
    }
}

#[derive(Clone, Copy)]
struct Fragment {
    start: StateID,
    end: StateID,
}

fn class(
    builder: &mut ObservedBuilder<'_>,
    ranges: &[(u32, u32)],
    work: Work<'_>,
) -> Result<Fragment, Failure<Box<BuildError>>> {
    let context = work.context;
    let mut alternatives = Buffer::new_in(context);
    let exit = change!(builder, add_empty())?;
    for &(lo, hi) in ranges {
        work.step(1)?;
        for (lo, hi) in [(lo, hi.min(0xd7ff)), (lo.max(0xe000), hi)] {
            work.step(1)?;
            if lo > hi {
                continue;
            }
            for encoding in
                Utf8Sequences::new(char::from_u32(lo).unwrap(), char::from_u32(hi).unwrap())
            {
                work.step(1)?;
                let mut first = None;
                let mut previous = None;
                for range in encoding.as_slice() {
                    work.step(1)?;
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

fn construct(
    program: &Program,
    work: Work<'_>,
    capture_groups: bool,
) -> Result<Built, Failure<Box<BuildError>>> {
    work.step(0)?;
    let context = work.context;
    let (prefix, prefix_instructions) = program.prefix(work)?;
    let mut reachable = work.filled(program.instructions.len(), false)?;
    let mut pending = buffer![context; program.start];
    while let Some(pc) = pending.pop() {
        work.step(1)?;
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
    let mut builder = ObservedBuilder::new(work);
    builder.change(|b| {
        b.set_utf8(true);
    })?;
    change!(builder, set_size_limit(None))?;
    assert_eq!(change!(builder, add_fail())?, StateID::ZERO);
    change!(builder, start_pattern())?;
    let capture_zero = change!(builder, add_capture_start(StateID::ZERO, 0, None))?;
    let fail = Fragment {
        start: StateID::ZERO,
        end: StateID::ZERO,
    };
    let mut fragments = work.filled(program.instructions.len(), fail)?;
    for (pc, instruction) in program.instructions.iter().enumerate() {
        work.step(1)?;
        if !reachable[pc] {
            continue;
        }
        let state = match instruction.code {
            Code::Fail => StateID::ZERO,
            Code::Nop => change!(builder, add_empty())?,
            Code::Alt => change!(builder, add_union(Vec::new()))?,
            Code::Empty => change!(builder, add_look(StateID::ZERO, look(instruction.arg)))?,
            // Keep frontend capture boundaries for prefix analysis; selectors only omit backend markers.
            Code::Capture if !capture_groups => change!(builder, add_empty())?,
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
                    let rune = if work.any(prefix_instructions.iter(), |&id| id == pc)?
                        && (0xd800..=0xdfff).contains(&rune)
                    {
                        0xfffd
                    } else {
                        rune
                    };
                    if instruction.arg & 1 != 0 {
                        runes::folded(buffer![context; (rune, rune)], work)?
                    } else {
                        buffer![context; (rune, rune)]
                    }
                } else {
                    work.collect(
                        instruction
                            .runes
                            .chunks_exact(2)
                            .map(|pair| (pair[0], pair[1])),
                    )?
                };
                ranges = runes::clean(ranges, work)?;
                fragments[pc] = class(&mut builder, &ranges, work)?;
                continue;
            }
        };
        fragments[pc] = Fragment {
            start: state,
            end: state,
        };
    }
    for (pc, instruction) in program.instructions.iter().enumerate() {
        work.step(1)?;
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
    let unanchored = buffer![context; capture_zero]
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
    let estimate = context.map(|context| {
        context
            .with_kind(MemoryKind::RegexEstimate)
            .hold(nfa.memory_usage() as u64)
    });
    work.step(0)?;
    let vm = PikeVM::builder()
        .configure(PikeVM::config().match_kind(MatchKind::LeftmostFirst))
        .build_from_nfa(nfa)
        .map_err(|error| Failure::Error(Box::new(error)))?;
    work.step(0)?;
    Ok((vm, prefix, estimate))
}

pub fn build(
    program: &Program,
    work: Work<'_>,
    capture_groups: bool,
) -> Result<Built, Failure<Buffer<u8>>> {
    match construct(program, work, capture_groups) {
        Ok(value) => Ok(value),
        Err(Failure::Interrupted(error)) => Err(Failure::Interrupted(error)),
        Err(Failure::Error(error)) => Err(Failure::Error(
            work.format(format_args!("internal regex backend error: {error}"))?,
        )),
    }
}
