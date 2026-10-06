// SPDX-License-Identifier: GPL-3.0-or-later
// Go-compatible simplification, flat program and one-pass analysis; see NOTICE.
use super::{
    Buffer, Work,
    runes::{self, Class},
    syntax::{FOLD_CASE, Id, NON_GREEDY, Node, Op, Tree},
};

use crate::storage::Interrupted;

const BEGIN_LINE: usize = 1;
const END_LINE: usize = 2;
pub const BEGIN_TEXT: usize = 4;
pub const END_TEXT: usize = 8;
const WORD: usize = 16;
const NO_WORD: usize = 32;

fn add(tree: &mut Tree, node: Node) -> Id {
    let id = tree.nodes.len();
    tree.nodes.push(node);
    id
}

fn unary(tree: &mut Tree, op: Op, flags: u16, sub: Id, original: Option<Id>) -> Id {
    let context = tree.context();
    let node = &tree.nodes[sub];
    if node.op == Op::Empty || node.op == op && node.flags & NON_GREEDY == flags & NON_GREEDY {
        return sub;
    }
    if let Some(original) = original.filter(|&id| tree.nodes[id].children[0] == sub) {
        return original;
    }
    let mut node = Node::new(op, flags, context.as_ref());
    node.children.push(sub);
    add(tree, node)
}

fn concatenation(tree: &mut Tree, children: Buffer<Id>) -> Id {
    let context = tree.context();
    let mut node = Node::new(Op::Concat, 0, context.as_ref());
    node.children = children;
    add(tree, node)
}

fn simplify(tree: &mut Tree, work: Work<'_>) -> Result<(), Interrupted> {
    work.step(0)?;
    let context = work.context.cloned();
    let mut frames = buffer![context.as_ref(); (tree.root, false)];
    let mut values = Buffer::new_in(context.as_ref());
    while let Some((id, finish)) = frames.pop() {
        work.step(1)?;
        let op = tree.nodes[id].op;
        if !finish {
            if op == Op::Repeat && tree.nodes[id].min == 0 && tree.nodes[id].max == Some(0) {
                values.push(add(tree, Node::new(Op::Empty, 0, context.as_ref())));
                continue;
            }
            if matches!(
                op,
                Op::Capture
                    | Op::Concat
                    | Op::Alternate
                    | Op::Star
                    | Op::Plus
                    | Op::Quest
                    | Op::Repeat
            ) {
                frames.push((id, true));
                work.extend_iter(
                    &mut frames,
                    tree.nodes[id].children.iter().rev().map(|&id| (id, false)),
                )?;
            } else {
                values.push(id);
            }
            continue;
        }
        let node = tree.nodes[id].copy(work)?;
        let begin = values.len() - node.children.len();
        let children = work.split(&mut values, begin)?;
        let result = match op {
            Op::Star | Op::Plus | Op::Quest => unary(tree, op, node.flags, children[0], Some(id)),
            Op::Repeat => {
                let sub = children[0];
                match node.max {
                    None if node.min <= 1 => unary(
                        tree,
                        if node.min == 0 { Op::Star } else { Op::Plus },
                        node.flags,
                        sub,
                        None,
                    ),
                    None => {
                        let last = unary(tree, Op::Plus, node.flags, sub, None);
                        let mut children = work.filled(node.min as usize - 1, sub)?;
                        children.push(last);
                        concatenation(tree, children)
                    }
                    Some(1) if node.min == 1 => sub,
                    Some(max) => {
                        let mut children = work.filled(node.min as usize, sub)?;
                        if max > node.min {
                            let mut suffix = unary(tree, Op::Quest, node.flags, sub, None);
                            for _ in node.min + 1..max {
                                work.step(1)?;
                                let concat =
                                    concatenation(tree, buffer![context.as_ref(); sub, suffix]);
                                suffix = unary(tree, Op::Quest, node.flags, concat, None);
                            }
                            if node.min == 0 {
                                values.push(suffix);
                                continue;
                            }
                            children.push(suffix);
                        }
                        if node.min == 0 {
                            add(tree, Node::new(Op::NoMatch, 0, context.as_ref()))
                        } else {
                            concatenation(tree, children)
                        }
                    }
                }
            }
            _ if work.same(&children, &node.children)? => id,
            _ => {
                let mut node = node;
                node.children = children;
                node.runes.clear();
                add(tree, node)
            }
        };
        values.push(result);
    }
    tree.root = values.pop().unwrap();
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Code {
    Fail,
    Nop,
    Match,
    Capture,
    Empty,
    Alt,
    Rune,
    Rune1,
    Any,
    AnyNotNL,
}

#[derive(Clone, Debug)]
pub struct Instruction {
    pub code: Code,
    pub out: usize,
    pub arg: usize,
    pub runes: Buffer<u32>,
}

#[derive(Debug)]
pub struct Program {
    pub instructions: Buffer<Instruction>,
    pub start: usize,
}

impl Instruction {
    fn copy(&self, work: Work<'_>) -> Result<Self, Interrupted> {
        Ok(Self {
            code: self.code,
            out: self.out,
            arg: self.arg,
            runes: work.elements(&self.runes)?,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct Link {
    head: usize,
    tail: usize,
}

#[derive(Clone, Copy, Default)]
struct Fragment {
    start: usize,
    exits: Link,
    nullable: bool,
}

enum Frame {
    Visit(Id),
    Concat(usize),
    Alternate {
        id: Id,
        index: usize,
        head: Fragment,
    },
    CaptureEnd(Fragment, u32),
    Unary(Op, u16),
}

impl Program {
    fn instruction(&mut self, code: Code) -> Fragment {
        let context = self.instructions.context();
        let start = self.instructions.len();
        self.instructions.push(Instruction {
            code,
            out: 0,
            arg: 0,
            runes: Buffer::new_in(context.as_ref()),
        });
        Fragment {
            start,
            exits: Link::default(),
            nullable: true,
        }
    }

    fn edge(&self, edge: usize) -> usize {
        if edge & 1 == 0 {
            self.instructions[edge / 2].out
        } else {
            self.instructions[edge / 2].arg
        }
    }
    fn set_edge(&mut self, edge: usize, value: usize) {
        if edge & 1 == 0 {
            self.instructions[edge / 2].out = value;
        } else {
            self.instructions[edge / 2].arg = value;
        }
    }
    fn patch(
        &mut self,
        mut exits: Link,
        destination: usize,
        work: Work<'_>,
    ) -> Result<(), Interrupted> {
        while exits.head != 0 {
            work.step(1)?;
            let next = self.edge(exits.head);
            self.set_edge(exits.head, destination);
            exits.head = next;
        }
        Ok(())
    }
    fn append(&mut self, left: Link, right: Link) -> Link {
        if left.head == 0 {
            return right;
        }
        if right.head == 0 {
            return left;
        }
        self.set_edge(left.tail, right.head);
        Link {
            head: left.head,
            tail: right.tail,
        }
    }
    fn outgoing(&mut self, code: Code, arg: usize) -> Fragment {
        let mut fragment = self.instruction(code);
        self.instructions[fragment.start].arg = arg;
        fragment.exits = Link {
            head: fragment.start * 2,
            tail: fragment.start * 2,
        };
        fragment
    }
    fn concat(
        &mut self,
        left: Fragment,
        right: Fragment,
        work: Work<'_>,
    ) -> Result<Fragment, Interrupted> {
        if left.start == 0 || right.start == 0 {
            return Ok(Fragment::default());
        }
        self.patch(left.exits, right.start, work)?;
        Ok(Fragment {
            start: left.start,
            exits: right.exits,
            nullable: left.nullable && right.nullable,
        })
    }
    fn alternate(&mut self, left: Fragment, right: Fragment) -> Fragment {
        if left.start == 0 {
            return right;
        }
        if right.start == 0 {
            return left;
        }
        let mut result = self.instruction(Code::Alt);
        self.instructions[result.start].out = left.start;
        self.instructions[result.start].arg = right.start;
        result.exits = self.append(left.exits, right.exits);
        result.nullable = left.nullable || right.nullable;
        result
    }
    fn question(&mut self, body: Fragment, lazy: bool) -> Fragment {
        let mut result = self.instruction(Code::Alt);
        let exit = result.start * 2 + usize::from(!lazy);
        self.set_edge(result.start * 2 + usize::from(lazy), body.start);
        result.exits = self.append(
            Link {
                head: exit,
                tail: exit,
            },
            body.exits,
        );
        result
    }
    fn plus(
        &mut self,
        body: Fragment,
        lazy: bool,
        work: Work<'_>,
    ) -> Result<Fragment, Interrupted> {
        let choice = self.loop_fragment(body, lazy, work)?;
        Ok(Fragment {
            start: body.start,
            exits: choice.exits,
            nullable: body.nullable,
        })
    }
    fn loop_fragment(
        &mut self,
        body: Fragment,
        lazy: bool,
        work: Work<'_>,
    ) -> Result<Fragment, Interrupted> {
        let mut result = self.instruction(Code::Alt);
        let exit = result.start * 2 + usize::from(!lazy);
        self.set_edge(result.start * 2 + usize::from(lazy), body.start);
        self.patch(body.exits, result.start, work)?;
        result.exits = Link {
            head: exit,
            tail: exit,
        };
        Ok(result)
    }
    fn rune(&mut self, runes: Buffer<u32>, flags: u16) -> Fragment {
        let fold = flags & FOLD_CASE != 0 && runes.len() == 1 && runes::fold(runes[0]) != runes[0];
        let code = if !fold && (runes.len() == 1 || runes.len() == 2 && runes[0] == runes[1]) {
            Code::Rune1
        } else if runes == [0, runes::MAX_RUNE] {
            Code::Any
        } else if runes == [0, 9, 11, runes::MAX_RUNE] {
            Code::AnyNotNL
        } else {
            Code::Rune
        };
        let mut result = self.outgoing(code, usize::from(fold));
        result.nullable = false;
        self.instructions[result.start].runes = runes;
        result
    }

    pub fn compile(tree: &mut Tree, work: Work<'_>) -> Result<Self, Interrupted> {
        work.step(0)?;
        let context = work.context.cloned();
        simplify(tree, work)?;
        let mut program = Self {
            instructions: Buffer::new_in(context.as_ref()),
            start: 0,
        };
        program.instruction(Code::Fail);
        let mut frames = buffer![context.as_ref(); Frame::Visit(tree.root)];
        let mut values = Buffer::<Fragment>::new_in(context.as_ref());
        while let Some(frame) = frames.pop() {
            work.step(1)?;
            let result = match frame {
                Frame::Visit(id) => {
                    let node = &tree.nodes[id];
                    match node.op {
                        Op::NoMatch => Fragment::default(),
                        Op::Empty => program.outgoing(Code::Nop, 0),
                        Op::Literal => {
                            let mut value = None;
                            for &rune in &node.runes {
                                work.step(1)?;
                                let next =
                                    program.rune(buffer![context.as_ref(); rune], node.flags);
                                value = Some(match value {
                                    Some(previous) => program.concat(previous, next, work)?,
                                    None => next,
                                });
                            }
                            value.unwrap_or_else(|| program.outgoing(Code::Nop, 0))
                        }
                        Op::Class => program.rune(
                            work.collect(node.class.iter().flat_map(|&(lo, hi)| [lo, hi]))?,
                            node.flags,
                        ),
                        Op::Any => program.rune(buffer![context.as_ref(); 0, runes::MAX_RUNE], 0),
                        Op::AnyNotNL => {
                            program.rune(buffer![context.as_ref(); 0, 9, 11, runes::MAX_RUNE], 0)
                        }
                        Op::BeginText => program.outgoing(Code::Empty, BEGIN_TEXT),
                        Op::EndText => program.outgoing(Code::Empty, END_TEXT),
                        Op::BeginLine => program.outgoing(Code::Empty, BEGIN_LINE),
                        Op::EndLine => program.outgoing(Code::Empty, END_LINE),
                        Op::WordBoundary => program.outgoing(Code::Empty, WORD),
                        Op::NoWordBoundary => program.outgoing(Code::Empty, NO_WORD),
                        Op::Capture => {
                            let begin = program.outgoing(Code::Capture, node.capture as usize * 2);
                            frames.push(Frame::CaptureEnd(begin, node.capture));
                            frames.push(Frame::Visit(node.children[0]));
                            continue;
                        }
                        Op::Star | Op::Plus | Op::Quest => {
                            frames.push(Frame::Unary(node.op, node.flags));
                            frames.push(Frame::Visit(node.children[0]));
                            continue;
                        }
                        Op::Alternate if !node.children.is_empty() => {
                            frames.push(Frame::Alternate {
                                id,
                                index: 0,
                                head: Fragment::default(),
                            });
                            frames.push(Frame::Visit(node.children[0]));
                            continue;
                        }
                        Op::Alternate => Fragment::default(),
                        Op::Concat => {
                            frames.push(Frame::Concat(node.children.len()));
                            work.extend_iter(
                                &mut frames,
                                node.children.iter().rev().copied().map(Frame::Visit),
                            )?;
                            continue;
                        }
                        _ => unreachable!("simplified regex tree"),
                    }
                }
                Frame::Concat(count) => {
                    let begin = values.len() - count;
                    let mut result = if matches!(frame, Frame::Concat(0)) {
                        program.outgoing(Code::Nop, 0)
                    } else {
                        Fragment::default()
                    };
                    for (index, &fragment) in values[begin..].iter().enumerate() {
                        work.step(1)?;
                        result = if index == 0 {
                            fragment
                        } else {
                            program.concat(result, fragment, work)?
                        };
                    }
                    values.truncate(begin);
                    result
                }
                Frame::Alternate { id, index, head } => {
                    let child = values.pop().unwrap();
                    let result = program.alternate(head, child);
                    let children = &tree.nodes[id].children;
                    if index + 1 < children.len() {
                        frames.push(Frame::Alternate {
                            id,
                            index: index + 1,
                            head: result,
                        });
                        frames.push(Frame::Visit(children[index + 1]));
                        continue;
                    }
                    result
                }
                Frame::CaptureEnd(begin, capture) => {
                    let sub = values.pop().unwrap();
                    let end = program.outgoing(Code::Capture, capture as usize * 2 + 1);
                    let head = program.concat(begin, sub, work)?;
                    program.concat(head, end, work)?
                }
                Frame::Unary(op, flags) => {
                    let body = values.pop().unwrap();
                    let lazy = flags & NON_GREEDY != 0;
                    match op {
                        Op::Quest => program.question(body, lazy),
                        Op::Plus => program.plus(body, lazy, work)?,
                        _ if body.nullable => {
                            let plus = program.plus(body, lazy, work)?;
                            program.question(plus, lazy)
                        }
                        _ => program.loop_fragment(body, lazy, work)?,
                    }
                }
            };
            values.push(result);
        }
        let root = values.pop().unwrap();
        let matched = program.instruction(Code::Match);
        program.patch(root.exits, matched.start, work)?;
        program.start = root.start;
        Ok(program)
    }

    fn one_pass(&self, work: Work<'_>) -> Result<bool, Interrupted> {
        work.step(0)?;
        let context = work.context.cloned();
        if self.start == 0 || self.instructions.len() >= 1000 {
            return Ok(false);
        }
        let first = &self.instructions[self.start];
        if first.code != Code::Empty || first.arg & BEGIN_TEXT == 0 {
            return Ok(false);
        }
        let has_alt = work.any(self.instructions.iter(), |instruction| {
            instruction.code == Code::Alt
        })?;
        for instruction in &self.instructions {
            work.step(1)?;
            let matched = self.instructions[instruction.out].code == Code::Match;
            match instruction.code {
                Code::Alt if matched || self.instructions[instruction.arg].code == Code::Match => {
                    return Ok(false);
                }
                Code::Empty if matched && instruction.arg & END_TEXT == 0 => return Ok(false),
                Code::Alt | Code::Empty => {}
                _ if matched && has_alt => return Ok(false),
                _ => {}
            }
        }
        let mut instructions = work.buffer();
        for instruction in &self.instructions {
            instructions.push(instruction.copy(work)?);
        }
        let mut program = Self {
            instructions,
            start: self.start,
        };
        for pc in 0..program.instructions.len() {
            work.step(1)?;
            if program.instructions[pc].code != Code::Alt {
                continue;
            }
            let mut a_alt = pc * 2 + 1;
            let mut a_other = pc * 2;
            if program.instructions[program.edge(a_alt)].code != Code::Alt {
                std::mem::swap(&mut a_alt, &mut a_other);
                if program.instructions[program.edge(a_alt)].code != Code::Alt {
                    continue;
                }
            }
            if program.instructions[program.edge(a_other)].code == Code::Alt {
                continue;
            }
            let b = program.edge(a_alt);
            let mut b_alt = b * 2;
            let mut b_other = b * 2 + 1;
            if program.edge(b_alt) == pc {
                let value = program.edge(a_other);
                program.set_edge(b_alt, value);
            } else if program.edge(b_other) == pc {
                std::mem::swap(&mut b_alt, &mut b_other);
                let value = program.edge(a_other);
                program.set_edge(b_alt, value);
            }
            if program.edge(a_other) == program.edge(b_alt) {
                let value = program.edge(b_other);
                program.set_edge(a_alt, value);
            }
        }
        let count = program.instructions.len();
        let mut queue = buffer![context.as_ref(); self.start];
        let mut queued = work.filled(count, false)?;
        queued[self.start] = true;
        let mut initialized = work.filled(count, false)?;
        let mut nullable = work.filled(count, false)?;
        let mut sets = work.collect((0..count).map(|_| Class::new_in(context.as_ref())))?;
        let mut cursor = 0;
        while cursor < queue.len() {
            work.step(1)?;
            let mut visited = work.filled(count, false)?;
            let mut frames = buffer![context.as_ref(); (queue[cursor], false)];
            cursor += 1;
            while let Some((pc, finish)) = frames.pop() {
                work.step(1)?;
                let instruction = &program.instructions[pc];
                if !finish {
                    if visited[pc] {
                        continue;
                    }
                    visited[pc] = true;
                    match instruction.code {
                        Code::Alt => {
                            frames.push((pc, true));
                            frames.push((instruction.arg, false));
                            frames.push((instruction.out, false));
                        }
                        Code::Capture | Code::Nop | Code::Empty => {
                            frames.push((pc, true));
                            frames.push((instruction.out, false));
                        }
                        Code::Match | Code::Fail => nullable[pc] = instruction.code == Code::Match,
                        _ => {
                            nullable[pc] = false;
                            if initialized[pc] {
                                continue;
                            }
                            initialized[pc] = true;
                            if !queued[instruction.out] {
                                queued[instruction.out] = true;
                                queue.push(instruction.out);
                            }
                            sets[pc] = if instruction.runes.len() == 1
                                || instruction.code == Code::Rune1
                            {
                                let rune = instruction.runes[0];
                                if instruction.arg & FOLD_CASE as usize != 0 {
                                    runes::folded(buffer![context.as_ref(); (rune, rune)], work)?
                                } else {
                                    buffer![context.as_ref(); (rune, rune)]
                                }
                            } else {
                                work.collect(
                                    instruction
                                        .runes
                                        .chunks_exact(2)
                                        .map(|pair| (pair[0], pair[1])),
                                )?
                            };
                        }
                    }
                } else if instruction.code == Code::Alt {
                    let (a, b) = (instruction.out, instruction.arg);
                    if nullable[a] && nullable[b] {
                        return Ok(false);
                    }
                    let mut ranges = work.elements(&sets[a])?;
                    work.extend_iter(&mut ranges, sets[b].iter().copied())?;
                    work.sort(&mut ranges)?;
                    if work.any(ranges.windows(2), |pair| pair[1].0 <= pair[0].1)? {
                        return Ok(false);
                    }
                    sets[pc] = ranges;
                    nullable[pc] = nullable[a] || nullable[b];
                } else {
                    nullable[pc] = nullable[instruction.out];
                    sets[pc] = work.elements(&sets[instruction.out])?;
                }
            }
        }
        Ok(true)
    }

    pub fn prefix(&self, work: Work<'_>) -> Result<(Buffer<u8>, Buffer<usize>), Interrupted> {
        let context = work.context.cloned();
        let mut selected = Buffer::new_in(context.as_ref());
        if !self.one_pass(work)? {
            return Ok((Buffer::new_in(context.as_ref()), selected));
        }
        let mut pc = self.instructions[self.start].out;
        while self.instructions[pc].code == Code::Nop {
            work.step(1)?;
            pc = self.instructions[pc].out;
        }
        let mut prefix = Buffer::new_in(context.as_ref());
        loop {
            work.step(1)?;
            let instruction = &self.instructions[pc];
            if !matches!(
                instruction.code,
                Code::Rune | Code::Rune1 | Code::Any | Code::AnyNotNL
            ) || instruction.runes.len() != 1
                || instruction.arg & FOLD_CASE as usize != 0
                || instruction.runes[0] == 0xfffd
            {
                break;
            }
            work.extend(&mut prefix, &runes::text(&instruction.runes, work)?)?;
            selected.push(pc);
            pc = instruction.out;
        }
        Ok((prefix, selected))
    }
}

#[cfg(feature = "fixtures")]
pub struct Inspection {
    pub instructions: Buffer<Instruction>,
    pub start: usize,
    pub num_cap: usize,
    pub one_pass: bool,
    pub prefix: Buffer<u8>,
    pub prefix_end: usize,
    pub complete: bool,
}

#[cfg(feature = "fixtures")]
impl Instruction {
    pub fn rune_field(&self) -> Option<&[u32]> {
        matches!(
            self.code,
            Code::Rune | Code::Rune1 | Code::Any | Code::AnyNotNL
        )
        .then_some(&self.runes)
    }
}

#[cfg(feature = "fixtures")]
impl Code {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fail => "InstFail",
            Self::Nop => "InstNop",
            Self::Match => "InstMatch",
            Self::Capture => "InstCapture",
            Self::Empty => "InstEmptyWidth",
            Self::Alt => "InstAlt",
            Self::Rune => "InstRune",
            Self::Rune1 => "InstRune1",
            Self::Any => "InstRuneAny",
            Self::AnyNotNL => "InstRuneAnyNotNL",
        }
    }
}

#[cfg(feature = "fixtures")]
impl Program {
    pub fn inspect(&self) -> Inspection {
        let context = self.instructions.context();
        let one_pass = self
            .one_pass(Work::default())
            .expect("inert one-pass inspection");
        let (mut prefix, selected) = self
            .prefix(Work::default())
            .expect("inert prefix inspection");
        let mut pc = self.start;
        let prefix_end;
        let complete;
        if one_pass {
            pc = self.instructions[pc].out;
            while self.instructions[pc].code == Code::Nop {
                pc = self.instructions[pc].out;
            }
            if !matches!(
                self.instructions[pc].code,
                Code::Rune | Code::Rune1 | Code::Any | Code::AnyNotNL
            ) || self.instructions[pc].runes.len() != 1
            {
                complete = self.instructions[pc].code == Code::Match;
                prefix_end = self.start;
            } else {
                if let Some(&last) = selected.last() {
                    pc = self.instructions[last].out;
                }
                complete = self.instructions[pc].code == Code::Empty
                    && self.instructions[pc].arg & END_TEXT != 0
                    && self.instructions[self.instructions[pc].out].code == Code::Match;
                prefix_end = pc;
            }
        } else {
            let skip = |mut pc: usize| {
                while matches!(self.instructions[pc].code, Code::Nop | Code::Capture) {
                    pc = self.instructions[pc].out;
                }
                pc
            };
            pc = skip(pc);
            loop {
                let instruction = &self.instructions[pc];
                if !matches!(
                    instruction.code,
                    Code::Rune | Code::Rune1 | Code::Any | Code::AnyNotNL
                ) || instruction.runes.len() != 1
                    || instruction.arg & 1 != 0
                    || instruction.runes[0] == 0xfffd
                {
                    break;
                }
                prefix.extend(
                    runes::text(&instruction.runes, Work::new(context.as_ref(), None))
                        .expect("inert rune text"),
                );
                pc = skip(instruction.out);
            }
            complete = self.instructions[pc].code == Code::Match;
            prefix_end = 0;
        }
        Inspection {
            instructions: self.instructions.clone(),
            start: self.start,
            one_pass,
            prefix,
            prefix_end,
            complete,
            num_cap: self
                .instructions
                .iter()
                .filter(|i| i.code == Code::Capture)
                .map(|i| i.arg + 1)
                .max()
                .unwrap_or(2)
                .max(2),
        }
    }
}
