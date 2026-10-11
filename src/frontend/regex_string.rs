// SPDX-License-Identifier: GPL-3.0-or-later
// Go AST flag spans and canonical serialization, derived from syntax/regexp.go; see NOTICE.
use super::{
    Buffer, Work, runes,
    syntax::{FOLD_CASE, Id, NON_GREEDY, Op, Tree, WAS_DOLLAR},
};
use crate::storage::Interrupted;
use std::fmt::Write as _;

const I: u8 = 1;
const M: u8 = 2;
const S: u8 = 4;
const OFF: u8 = 8;
const PRECEDENCE: u8 = 16;
const NEG_SHIFT: u8 = 5;

fn span(flags: &mut [u8], start: Id, last: Id, required: u8) {
    flags[start] = required;
    flags[last] |= OFF;
}

fn calc(tree: &Tree, work: Work<'_>) -> Result<(Buffer<u8>, u8, u8), Interrupted> {
    work.step(0)?;
    let context = work.context.cloned();
    let mut flags = work.filled(tree.nodes.len(), 0)?;
    let mut needs = work.filled(tree.nodes.len(), (0u8, 0u8))?;
    let mut pending = buffer![context.as_ref(); (tree.root, false)];
    while let Some((id, finish)) = pending.pop() {
        work.step(1)?;
        let node = &tree.nodes[id];
        if !finish {
            pending.push((id, true));
            work.extend_iter(
                &mut pending,
                node.children.iter().rev().map(|&id| (id, false)),
            )?;
            continue;
        }
        needs[id] = match node.op {
            Op::Literal if work.any(node.runes.iter(), |&rune| runes::fold(rune) != rune)? => {
                if node.flags & FOLD_CASE != 0 {
                    (I, 0)
                } else {
                    (0, I)
                }
            }
            Op::Class => {
                let incompatible =
                    work.any(super::unicode::SIMPLE_FOLD.iter(), |&(rune, folded)| {
                        runes::contains(&node.class, rune) && !runes::contains(&node.class, folded)
                    })?;
                (0, if incompatible { I } else { 0 })
            }
            Op::AnyNotNL => (0, S),
            Op::Any => (S, 0),
            Op::BeginLine | Op::EndLine => (M, 0),
            Op::EndText if node.flags & WAS_DOLLAR != 0 => (0, M),
            Op::Capture | Op::Star | Op::Plus | Op::Quest | Op::Repeat => needs[node.children[0]],
            Op::Concat | Op::Alternate => {
                let (mut must, mut cant, mut all_cant) = (0, 0, 0);
                let (mut start, mut last) = (0, 0);
                let mut conflict = false;
                for (index, &child) in node.children.iter().enumerate() {
                    work.step(1)?;
                    let (sub_must, sub_cant) = needs[child];
                    if must & sub_cant != 0 || sub_must & cant != 0 {
                        if must != 0 {
                            span(&mut flags, node.children[start], node.children[last], must);
                        }
                        must = 0;
                        cant = 0;
                        start = index;
                        conflict = true;
                    }
                    must |= sub_must;
                    cant |= sub_cant;
                    all_cant |= sub_cant;
                    if sub_must != 0 {
                        last = index;
                    }
                    if must == 0 && start == index {
                        start += 1;
                    }
                }
                if conflict {
                    if must != 0 {
                        span(&mut flags, node.children[start], node.children[last], must);
                    }
                    (0, all_cant)
                } else {
                    (must, cant)
                }
            }
            _ => (0, 0),
        };
    }
    let (must, cant) = needs[tree.root];
    Ok((flags, must, cant))
}

fn escape(
    output: &mut Buffer<u8>,
    rune: u32,
    force: bool,
    work: Work<'_>,
) -> Result<(), Interrupted> {
    work.step(1)?;
    if crate::frontend::unicode::is_print(rune) {
        if force || rune < 128 && b"\\.+*?()|[]{}^$".contains(&(rune as u8)) {
            output.push(b'\\');
        }
        work.extend(output, &runes::text(&[rune], work)?)?;
        return Ok(());
    }
    match rune {
        7 => output.extend_from_slice(b"\\a"),
        12 => output.extend_from_slice(b"\\f"),
        10 => output.extend_from_slice(b"\\n"),
        13 => output.extend_from_slice(b"\\r"),
        9 => output.extend_from_slice(b"\\t"),
        11 => output.extend_from_slice(b"\\v"),
        rune if rune < 0x100 => write!(output, "\\x{rune:02x}").unwrap(),
        rune => write!(output, "\\x{{{rune:x}}}").unwrap(),
    }
    Ok(())
}

fn range(output: &mut Buffer<u8>, lo: u32, hi: u32, work: Work<'_>) -> Result<(), Interrupted> {
    escape(output, lo, lo == u32::from(b'-'), work)?;
    if hi != lo {
        if hi != lo + 1 {
            output.push(b'-');
        }
        escape(output, hi, hi == u32::from(b'-'), work)?;
    }
    Ok(())
}

enum WriteFrame {
    Node(Id, u8),
    Bytes(Buffer<u8>),
}

pub fn canonical(tree: &Tree, work: Work<'_>) -> Result<Buffer<u8>, Interrupted> {
    work.step(0)?;
    let context = work.context.cloned();
    let (flags, mut must, cant) = calc(tree, work)?;
    must |= (cant & !I) << NEG_SHIFT;
    if must != 0 {
        must |= OFF;
    }
    let mut pending = buffer![context.as_ref(); WriteFrame::Node(tree.root, must)];
    let mut output = Buffer::new_in(context.as_ref());
    while let Some(frame) = pending.pop() {
        work.step(1)?;
        let WriteFrame::Node(id, mut active) = frame else {
            if let WriteFrame::Bytes(bytes) = frame {
                work.extend(&mut output, &bytes)?;
            }
            continue;
        };
        let node = &tree.nodes[id];
        active |= flags[id];
        if active & PRECEDENCE != 0 && active & !(OFF | PRECEDENCE) != 0 && active & OFF != 0 {
            active &= !PRECEDENCE;
        }
        if active & !(OFF | PRECEDENCE) != 0 {
            output.extend_from_slice(b"(?");
            for (flag, byte) in [(I, b'i'), (M, b'm'), (S, b's')] {
                work.step(1)?;
                if active & flag != 0 {
                    output.push(byte);
                }
            }
            if active & ((M | S) << NEG_SHIFT) != 0 {
                output.push(b'-');
                if active & (M << NEG_SHIFT) != 0 {
                    output.push(b'm');
                }
                if active & (S << NEG_SHIFT) != 0 {
                    output.push(b's');
                }
            }
            output.push(b':');
        }
        if active & OFF != 0 {
            pending.push(WriteFrame::Bytes(Buffer::from_slice_in(
                b")",
                context.as_ref(),
            )));
        }
        if active & PRECEDENCE != 0 {
            output.extend_from_slice(b"(?:");
            pending.push(WriteFrame::Bytes(Buffer::from_slice_in(
                b")",
                context.as_ref(),
            )));
        }
        match node.op {
            Op::NoMatch => output.extend_from_slice(br"[^\x00-\x{10FFFF}]"),
            Op::Empty => output.extend_from_slice(b"(?:)"),
            Op::Literal => {
                for &rune in &node.runes {
                    work.step(1)?;
                    escape(&mut output, rune, false, work)?;
                }
            }
            Op::Class => {
                output.push(b'[');
                if node.class.is_empty() {
                    output.extend_from_slice(br"^\x00-\x{10FFFF}");
                } else if node.class.len() > 1
                    && node.class[0].0 == 0
                    && node.class.last().unwrap().1 == runes::MAX_RUNE
                {
                    output.push(b'^');
                    for pair in node.class.windows(2) {
                        work.step(1)?;
                        range(&mut output, pair[0].1 + 1, pair[1].0 - 1, work)?;
                    }
                } else {
                    for &(lo, hi) in &node.class {
                        work.step(1)?;
                        range(&mut output, lo, hi, work)?;
                    }
                }
                output.push(b']');
            }
            Op::Any | Op::AnyNotNL => output.push(b'.'),
            Op::BeginLine => output.push(b'^'),
            Op::EndLine => output.push(b'$'),
            Op::BeginText => output.extend_from_slice(br"\A"),
            Op::EndText if node.flags & WAS_DOLLAR != 0 => output.push(b'$'),
            Op::EndText => output.extend_from_slice(br"\z"),
            Op::WordBoundary => output.extend_from_slice(br"\b"),
            Op::NoWordBoundary => output.extend_from_slice(br"\B"),
            Op::Capture => {
                if node.name.is_empty() {
                    output.push(b'(');
                } else {
                    output.extend_from_slice(b"(?P<");
                    work.extend(&mut output, &node.name)?;
                    output.push(b'>');
                }
                pending.push(WriteFrame::Bytes(Buffer::from_slice_in(
                    b")",
                    context.as_ref(),
                )));
                if tree.nodes[node.children[0]].op != Op::Empty {
                    pending.push(WriteFrame::Node(node.children[0], flags[node.children[0]]));
                }
            }
            Op::Star | Op::Plus | Op::Quest | Op::Repeat => {
                let mut suffix = Buffer::new_in(context.as_ref());
                match node.op {
                    Op::Star => suffix.push(b'*'),
                    Op::Plus => suffix.push(b'+'),
                    Op::Quest => suffix.push(b'?'),
                    _ if node.max == Some(node.min) => write!(suffix, "{{{}}}", node.min).unwrap(),
                    _ => {
                        write!(suffix, "{{{},", node.min).unwrap();
                        if let Some(max) = node.max {
                            write!(suffix, "{max}").unwrap();
                        }
                        suffix.push(b'}');
                    }
                }
                if node.flags & NON_GREEDY != 0 {
                    suffix.push(b'?');
                }
                pending.push(WriteFrame::Bytes(suffix));
                let child = &tree.nodes[node.children[0]];
                let precedence =
                    child.op > Op::Capture || child.op == Op::Literal && child.runes.len() > 1;
                pending.push(WriteFrame::Node(
                    node.children[0],
                    if precedence { PRECEDENCE } else { 0 },
                ));
            }
            Op::Concat => {
                for &child in node.children.iter().rev() {
                    work.step(1)?;
                    pending.push(WriteFrame::Node(
                        child,
                        if tree.nodes[child].op == Op::Alternate {
                            PRECEDENCE
                        } else {
                            0
                        },
                    ));
                }
            }
            Op::Alternate => {
                for (index, &child) in node.children.iter().enumerate().rev() {
                    work.step(1)?;
                    pending.push(WriteFrame::Node(child, 0));
                    if index > 0 {
                        pending.push(WriteFrame::Bytes(Buffer::from_slice_in(
                            b"|",
                            context.as_ref(),
                        )));
                    }
                }
            }
            Op::LeftParen | Op::VerticalBar => unreachable!("completed regex tree"),
        }
    }
    Ok(output)
}
