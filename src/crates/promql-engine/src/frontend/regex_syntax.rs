// SPDX-License-Identifier: GPL-3.0-or-later
// Owned Go regexp syntax and ordered diagnostics, derived from syntax/parse.go; see NOTICE.
use super::runes::{self, Class};
use super::{AllocationContext, Buffer};
use crate::frontend::quote::decode_rune;

pub type Id = usize;
pub type Flags = u16;
pub const FOLD_CASE: Flags = 1;
pub const LITERAL: Flags = 2;
pub const CLASS_NL: Flags = 4;
pub const DOT_NL: Flags = 8;
pub const ONE_LINE: Flags = 16;
pub const NON_GREEDY: Flags = 32;
pub const PERL_X: Flags = 64;
pub const UNICODE_GROUPS: Flags = 128;
pub const WAS_DOLLAR: Flags = 256;
pub const PERL: Flags = CLASS_NL | ONE_LINE | PERL_X | UNICODE_GROUPS;

const MAX_HEIGHT: usize = 1000;
const MAX_SIZE: u64 = (128 << 20) / 40;
const MAX_RUNES: usize = (128 << 20) / 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Op {
    NoMatch,
    Empty,
    Literal,
    Class,
    AnyNotNL,
    Any,
    BeginLine,
    EndLine,
    BeginText,
    EndText,
    WordBoundary,
    NoWordBoundary,
    Capture,
    Star,
    Plus,
    Quest,
    Repeat,
    Concat,
    Alternate,
    LeftParen,
    VerticalBar,
}

impl Op {
    fn pseudo(self) -> bool {
        self >= Self::LeftParen
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub op: Op,
    pub flags: Flags,
    pub runes: Buffer<u32>,
    pub class: Class,
    pub children: Buffer<Id>,
    pub min: u32,
    pub max: Option<u32>,
    pub capture: u32,
    pub name: Buffer<u8>,
    height: usize,
    size: u64,
}

impl Node {
    pub(super) fn new(op: Op, flags: Flags, context: Option<&AllocationContext>) -> Self {
        Self {
            op,
            flags,
            runes: Buffer::new_in(context),
            class: Buffer::new_in(context),
            children: Buffer::new_in(context),
            min: 0,
            max: Some(0),
            capture: 0,
            name: Buffer::new_in(context),
            height: 1,
            size: 1,
        }
    }
}

#[derive(Debug)]
pub struct Tree {
    pub nodes: Buffer<Node>,
    pub root: Id,
    pub names: Buffer<Buffer<u8>>,
}
impl Tree {
    pub(super) fn context(&self) -> Option<AllocationContext> {
        self.nodes.context()
    }
}

#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub expression: Buffer<u8>,
}

impl Error {
    fn new(code: &'static str, expression: &[u8], context: Option<&AllocationContext>) -> Self {
        Self {
            code,
            expression: Buffer::from_slice_in(expression, context),
        }
    }

    pub fn bytes(&self) -> Buffer<u8> {
        let mut result = Buffer::new_in(self.expression.context().as_ref());
        result.extend_from_slice(b"error parsing regexp: ");
        result.extend_from_slice(self.code.as_bytes());
        result.extend_from_slice(b": `");
        result.extend_from_slice(&self.expression);
        result.push(b'`');
        result
    }
}

type Result<T> = std::result::Result<T, Error>;

struct Parser<'a> {
    context: Option<AllocationContext>,
    input: &'a [u8],
    position: usize,
    flags: Flags,
    nodes: Buffer<Node>,
    stack: Buffer<Id>,
    names: Buffer<Buffer<u8>>,
    rune_count: usize,
    standard: bool,
}

enum CollapseFrame {
    Collapse(Buffer<Id>, Op),
    Literal {
        id: Id,
        subs: Buffer<Id>,
        index: usize,
        output: Buffer<Id>,
    },
    LiteralResume {
        id: Id,
        subs: Buffer<Id>,
        index: usize,
        output: Buffer<Id>,
        prefix: Id,
    },
    Simple {
        id: Id,
        subs: Buffer<Id>,
        index: usize,
        output: Buffer<Id>,
    },
    SimpleResume {
        id: Id,
        subs: Buffer<Id>,
        index: usize,
        output: Buffer<Id>,
        prefix: Id,
    },
    Classes {
        id: Id,
        subs: Buffer<Id>,
    },
}

#[cfg(feature = "fixtures")]
pub fn parse(input: &[u8], flags: Flags) -> Result<Tree> {
    parse_profile(input, flags, false, None)
}

#[cfg(feature = "fixtures")]
pub fn parse_standard(input: &[u8], flags: Flags) -> Result<Tree> {
    parse_profile(input, flags, true, None)
}

pub(super) fn parse_in(
    input: &[u8],
    flags: Flags,
    context: Option<&AllocationContext>,
) -> Result<Tree> {
    parse_profile(input, flags, false, context)
}

pub(super) fn parse_standard_in(
    input: &[u8],
    flags: Flags,
    context: Option<&AllocationContext>,
) -> Result<Tree> {
    parse_profile(input, flags, true, context)
}

fn parse_profile(
    input: &[u8],
    flags: Flags,
    standard: bool,
    context: Option<&AllocationContext>,
) -> Result<Tree> {
    if flags & LITERAL != 0 {
        check_utf8(input, context)?;
        let mut position = 0;
        let mut node = Node::new(Op::Literal, flags, context);
        while position < input.len() {
            let rune = next_rune(input, &mut position, context)?;
            node.runes.push(rune);
        }
        return Ok(Tree {
            nodes: buffer![context; node],
            root: 0,
            names: buffer![context; Buffer::new_in(context)],
        });
    }
    Parser {
        context: context.cloned(),
        input,
        position: 0,
        flags,
        nodes: Buffer::new_in(context),
        stack: Buffer::new_in(context),
        names: buffer![context; Buffer::new_in(context)],
        rune_count: 0,
        standard,
    }
    .run()
}

fn next_rune(
    input: &[u8],
    position: &mut usize,
    context: Option<&AllocationContext>,
) -> Result<u32> {
    let (rune, width) = decode_rune(&input[*position..]);
    if rune == 0xfffd && width == 1 {
        return Err(Error::new("invalid UTF-8", &input[*position..], context));
    }
    *position += width;
    Ok(if width == 0 { 0xfffd } else { rune as u32 })
}

fn check_utf8(mut input: &[u8], context: Option<&AllocationContext>) -> Result<()> {
    while !input.is_empty() {
        let mut used = 0;
        next_rune(input, &mut used, context)?;
        input = &input[used..];
    }
    Ok(())
}

fn hex(rune: u32) -> Option<u32> {
    match rune {
        48..=57 => Some(rune - 48),
        65..=70 => Some(rune - 65 + 10),
        97..=102 => Some(rune - 97 + 10),
        _ => None,
    }
}

impl Parser<'_> {
    fn alloc(&mut self, op: Op) -> Id {
        let id = self.nodes.len();
        self.nodes
            .push(Node::new(op, self.flags, self.context.as_ref()));
        id
    }

    fn refresh(&mut self, id: Id) {
        let node = &self.nodes[id];
        let sub = node
            .children
            .first()
            .map_or(0, |&child| self.nodes[child].size);
        let sum = node
            .children
            .iter()
            .map(|&child| self.nodes[child].size)
            .sum::<u64>();
        let height = 1 + node
            .children
            .iter()
            .map(|&child| self.nodes[child].height)
            .max()
            .unwrap_or(0);
        let size = match node.op {
            Op::Literal => node.runes.len() as u64,
            Op::Capture | Op::Star => 2 + sub,
            Op::Plus | Op::Quest => 1 + sub,
            Op::Concat => sum,
            Op::Alternate => sum + node.children.len().saturating_sub(1) as u64,
            Op::Repeat => match node.max {
                None if node.min == 0 => 2 + sub,
                None => 1 + u64::from(node.min) * sub,
                Some(max) => u64::from(max) * sub + u64::from(max - node.min),
            },
            _ => 1,
        };
        self.nodes[id].height = height;
        self.nodes[id].size = size.max(1);
    }

    fn limits(&mut self, id: Id) -> Result<()> {
        self.refresh(id);
        if self.rune_count > MAX_RUNES || self.nodes[id].size > MAX_SIZE {
            return Err(Error::new(
                "expression too large",
                self.input,
                self.context.as_ref(),
            ));
        }
        if self.nodes[id].height > MAX_HEIGHT {
            return Err(Error::new(
                "expression nests too deeply",
                self.input,
                self.context.as_ref(),
            ));
        }
        Ok(())
    }

    fn maybe_concat(&mut self, next: Option<(u32, Flags)>) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        let first = self.stack[self.stack.len() - 2];
        let last = self.stack[self.stack.len() - 1];
        if self.nodes[first].op != Op::Literal
            || self.nodes[last].op != Op::Literal
            || self.nodes[first].flags & FOLD_CASE != self.nodes[last].flags & FOLD_CASE
        {
            return false;
        }
        let runes = std::mem::take(&mut self.nodes[last].runes);
        self.nodes[first].runes.extend(runes);
        self.refresh(first);
        if let Some((rune, flags)) = next {
            self.nodes[last].runes.push(rune);
            self.nodes[last].flags = flags;
            self.refresh(last);
            true
        } else {
            self.stack.pop();
            false
        }
    }

    fn push(&mut self, id: Id) -> Result<()> {
        self.rune_count += self.nodes[id].runes.len() + self.nodes[id].class.len() * 2;
        if self.nodes[id].op == Op::Class {
            let class = &self.nodes[id].class;
            let literal = if class.len() == 1 && class[0].0 == class[0].1 {
                Some((class[0].0, self.flags & !FOLD_CASE))
            } else if (class.len() == 2
                && class[0].0 == class[0].1
                && class[1].0 == class[1].1
                && runes::fold(class[0].0) == class[1].0
                && runes::fold(class[1].0) == class[0].0)
                || (class.len() == 1
                    && class[0].0 + 1 == class[0].1
                    && runes::fold(class[0].0) == class[0].1
                    && runes::fold(class[0].1) == class[0].0)
            {
                Some((class[0].0, self.flags | FOLD_CASE))
            } else {
                None
            };
            if let Some((rune, flags)) = literal {
                if self.maybe_concat(Some((rune, flags))) {
                    return Ok(());
                }
                self.nodes[id].op = Op::Literal;
                self.nodes[id].runes = buffer![self.context.as_ref(); rune];
                self.nodes[id].class.clear();
                self.nodes[id].flags = flags;
            } else {
                self.maybe_concat(None);
            }
        } else {
            self.maybe_concat(None);
        }
        self.stack.push(id);
        self.limits(id)
    }

    fn literal(&mut self, rune: u32) -> Result<()> {
        let id = self.alloc(Op::Literal);
        self.nodes[id].runes.push(if self.flags & FOLD_CASE != 0 {
            runes::min_fold(rune)
        } else {
            rune
        });
        self.push(id)
    }

    fn op(&mut self, op: Op) -> Result<Id> {
        let id = self.alloc(op);
        self.push(id)?;
        Ok(id)
    }

    fn is_class(&self, id: Id) -> bool {
        matches!(self.nodes[id].op, Op::Class | Op::Any | Op::AnyNotNL)
            || self.nodes[id].op == Op::Literal && self.nodes[id].runes.len() == 1
    }

    fn character_class(&self, id: Id) -> Class {
        match self.nodes[id].op {
            Op::Any => buffer![self.context.as_ref(); (0, runes::MAX_RUNE)],
            Op::AnyNotNL => buffer![self.context.as_ref(); (0, 9), (11, runes::MAX_RUNE)],
            Op::Class => self.nodes[id].class.clone(),
            Op::Literal if self.nodes[id].flags & FOLD_CASE != 0 => {
                let rune = self.nodes[id].runes[0];
                runes::folded(buffer![self.context.as_ref(); (rune, rune)])
            }
            Op::Literal => {
                buffer![self.context.as_ref(); (self.nodes[id].runes[0], self.nodes[id].runes[0])]
            }
            _ => unreachable!("character-like expression"),
        }
    }

    fn merge_class(&mut self, destination: Id, source: Id) {
        if self.nodes[destination].op == Op::Any {
            return;
        }
        if self.nodes[destination].op == Op::AnyNotNL {
            if runes::contains(&self.character_class(source), 10) {
                self.nodes[destination].op = Op::Any;
            }
            return;
        }
        if self.nodes[destination].op == Op::Literal
            && self.nodes[source].op == Op::Literal
            && self.nodes[destination].runes == self.nodes[source].runes
            && self.nodes[destination].flags == self.nodes[source].flags
        {
            return;
        }
        let mut class = self.character_class(destination);
        class.extend(self.character_class(source));
        self.nodes[destination].op = Op::Class;
        self.nodes[destination].runes.clear();
        self.nodes[destination].class = class;
        self.refresh(destination);
    }

    fn clean_alternative(&mut self, id: Id) {
        if self.nodes[id].op == Op::Class {
            self.nodes[id].class = runes::clean(std::mem::take(&mut self.nodes[id].class));
            let class = &self.nodes[id].class;
            let replacement = if class == &[(0, runes::MAX_RUNE)] {
                Some(Op::Any)
            } else if class == &[(0, 9), (11, runes::MAX_RUNE)] {
                Some(Op::AnyNotNL)
            } else {
                None
            };
            if let Some(op) = replacement {
                self.nodes[id].op = op;
                self.nodes[id].class.clear();
            }
            self.refresh(id);
        }
    }

    fn leading(&self, id: Id) -> Id {
        if self.nodes[id].op == Op::Concat && !self.nodes[id].children.is_empty() {
            self.nodes[id].children[0]
        } else {
            id
        }
    }

    fn leading_string(&self, id: Id) -> (&[u32], Flags) {
        let node = &self.nodes[self.leading(id)];
        if node.op == Op::Literal {
            (&node.runes, node.flags & FOLD_CASE)
        } else {
            (&[], 0)
        }
    }

    fn remove_string(&mut self, id: Id, count: usize) -> Id {
        let leading = self.leading(id);
        if self.nodes[leading].op != Op::Literal {
            return id;
        }
        self.nodes[leading].runes.drain(..count);
        if self.nodes[leading].runes.is_empty() {
            self.nodes[leading].op = Op::Empty;
        }
        self.refresh(leading);
        if leading != id && self.nodes[leading].op == Op::Empty {
            self.nodes[id].children.remove(0);
            if self.nodes[id].children.len() == 1 {
                return self.nodes[id].children[0];
            }
            if self.nodes[id].children.is_empty() {
                self.nodes[id].op = Op::Empty;
            }
        }
        self.refresh(id);
        id
    }

    fn remove_leading(&mut self, id: Id) -> Id {
        if self.nodes[id].op == Op::Concat && !self.nodes[id].children.is_empty() {
            self.nodes[id].children.remove(0);
            if self.nodes[id].children.len() == 1 {
                return self.nodes[id].children[0];
            }
            if self.nodes[id].children.is_empty() {
                self.nodes[id].op = Op::Empty;
            }
            self.refresh(id);
            id
        } else {
            self.alloc(Op::Empty)
        }
    }

    fn equal(&self, left: Id, right: Id) -> bool {
        let mut pending = buffer![self.context.as_ref(); (left, right)];
        while let Some((left, right)) = pending.pop() {
            let left = &self.nodes[left];
            let right = &self.nodes[right];
            if left.op != right.op {
                return false;
            }
            match left.op {
                Op::EndText if left.flags & WAS_DOLLAR != right.flags & WAS_DOLLAR => return false,
                Op::Literal | Op::Class => {
                    if left.runes != right.runes
                        || left.class != right.class
                        || self.standard && left.flags & FOLD_CASE != right.flags & FOLD_CASE
                    {
                        return false;
                    }
                }
                Op::Star | Op::Plus | Op::Quest | Op::Repeat => {
                    if left.flags & NON_GREEDY != right.flags & NON_GREEDY
                        || left.op == Op::Repeat && (left.min != right.min || left.max != right.max)
                    {
                        return false;
                    }
                }
                Op::Capture if left.capture != right.capture || left.name != right.name => {
                    return false;
                }
                _ => {}
            }
            if left.children.len() != right.children.len() {
                return false;
            }
            pending.extend(
                left.children
                    .iter()
                    .copied()
                    .zip(right.children.iter().copied()),
            );
        }
        true
    }

    fn factored_concat(&mut self, prefix: Id, suffix: Id) -> Id {
        let id = self.alloc(Op::Concat);
        self.nodes[id].children = buffer![self.context.as_ref(); prefix, suffix];
        self.refresh(id);
        id
    }

    fn collapse(&mut self, children: Buffer<Id>, op: Op) -> Result<Id> {
        let mut frames = buffer![self.context.as_ref(); CollapseFrame::Collapse(children, op)];
        let mut values = Buffer::new_in(self.context.as_ref());
        'frames: while let Some(frame) = frames.pop() {
            match frame {
                CollapseFrame::Collapse(children, op) => {
                    if children.len() == 1 {
                        values.push(children[0]);
                        continue;
                    }
                    let id = self.alloc(op);
                    let mut subs = Buffer::new_in(self.context.as_ref());
                    for child in children {
                        if self.nodes[child].op == op {
                            subs.extend_from_slice(&self.nodes[child].children);
                        } else {
                            subs.push(child);
                        }
                    }
                    if op == Op::Alternate {
                        frames.push(CollapseFrame::Literal {
                            id,
                            subs,
                            index: 0,
                            output: Buffer::new_in(self.context.as_ref()),
                        });
                    } else {
                        self.nodes[id].children = subs;
                        self.refresh(id);
                        values.push(id);
                    }
                }
                CollapseFrame::Literal {
                    id,
                    mut subs,
                    mut index,
                    mut output,
                } => {
                    while index < subs.len() {
                        let (prefix, flags) = self.leading_string(subs[index]);
                        let mut prefix = Buffer::from_slice_in(prefix, self.context.as_ref());
                        let mut end = index + 1;
                        while end < subs.len() {
                            let (next, next_flags) = self.leading_string(subs[end]);
                            let same = prefix.iter().zip(next).take_while(|(a, b)| a == b).count();
                            if flags != next_flags || same == 0 {
                                break;
                            }
                            prefix.truncate(same);
                            end += 1;
                        }
                        if end == index + 1 {
                            output.push(subs[index]);
                            index = end;
                            continue;
                        }
                        let prefix_id = self.alloc(Op::Literal);
                        self.nodes[prefix_id].flags = flags;
                        self.nodes[prefix_id].runes = prefix;
                        self.refresh(prefix_id);
                        for sub in &mut subs[index..end] {
                            *sub = self.remove_string(*sub, self.nodes[prefix_id].runes.len());
                            self.limits(*sub)?;
                        }
                        let suffixes =
                            Buffer::from_slice_in(&subs[index..end], self.context.as_ref());
                        frames.push(CollapseFrame::LiteralResume {
                            id,
                            subs,
                            index: end,
                            output,
                            prefix: prefix_id,
                        });
                        frames.push(CollapseFrame::Collapse(suffixes, Op::Alternate));
                        continue 'frames;
                    }
                    frames.push(CollapseFrame::Simple {
                        id,
                        subs: output,
                        index: 0,
                        output: Buffer::new_in(self.context.as_ref()),
                    });
                }
                CollapseFrame::LiteralResume {
                    id,
                    subs,
                    index,
                    mut output,
                    prefix,
                } => {
                    let suffix = values.pop().unwrap();
                    output.push(self.factored_concat(prefix, suffix));
                    frames.push(CollapseFrame::Literal {
                        id,
                        subs,
                        index,
                        output,
                    });
                }
                CollapseFrame::Simple {
                    id,
                    mut subs,
                    mut index,
                    mut output,
                } => {
                    while index < subs.len() {
                        let first = self.leading(subs[index]);
                        let node = &self.nodes[first];
                        let eligible = self.is_class(first)
                            || node.op == Op::Repeat
                                && node.max == Some(node.min)
                                && self.is_class(node.children[0]);
                        let mut end = index + 1;
                        while eligible
                            && end < subs.len()
                            && self.equal(first, self.leading(subs[end]))
                        {
                            end += 1;
                        }
                        if end == index + 1 {
                            output.push(subs[index]);
                            index = end;
                            continue;
                        }
                        for sub in &mut subs[index..end] {
                            *sub = self.remove_leading(*sub);
                            self.limits(*sub)?;
                        }
                        let suffixes =
                            Buffer::from_slice_in(&subs[index..end], self.context.as_ref());
                        frames.push(CollapseFrame::SimpleResume {
                            id,
                            subs,
                            index: end,
                            output,
                            prefix: first,
                        });
                        frames.push(CollapseFrame::Collapse(suffixes, Op::Alternate));
                        continue 'frames;
                    }
                    frames.push(CollapseFrame::Classes { id, subs: output });
                }
                CollapseFrame::SimpleResume {
                    id,
                    subs,
                    index,
                    mut output,
                    prefix,
                } => {
                    let suffix = values.pop().unwrap();
                    output.push(self.factored_concat(prefix, suffix));
                    frames.push(CollapseFrame::Simple {
                        id,
                        subs,
                        index,
                        output,
                    });
                }
                CollapseFrame::Classes { id, subs } => {
                    let mut output = Buffer::new_in(self.context.as_ref());
                    let mut index = 0;
                    while index < subs.len() {
                        if !self.is_class(subs[index]) {
                            output.push(subs[index]);
                            index += 1;
                            continue;
                        }
                        let mut end = index + 1;
                        while end < subs.len() && self.is_class(subs[end]) {
                            end += 1;
                        }
                        let mut destination = subs[index];
                        for &candidate in &subs[index + 1..end] {
                            let key = |id: Id| {
                                (
                                    self.nodes[id].op,
                                    self.nodes[id].runes.len() + self.nodes[id].class.len() * 2,
                                )
                            };
                            if key(candidate) > key(destination) {
                                destination = candidate;
                            }
                        }
                        for &source in &subs[index..end] {
                            if source != destination {
                                self.merge_class(destination, source);
                            }
                        }
                        self.clean_alternative(destination);
                        output.push(destination);
                        index = end;
                    }
                    let mut deduplicated = Buffer::new_in(self.context.as_ref());
                    for (index, &sub) in output.iter().enumerate() {
                        if self.nodes[sub].op == Op::Empty
                            && output
                                .get(index + 1)
                                .is_some_and(|&next| self.nodes[next].op == Op::Empty)
                        {
                            continue;
                        }
                        deduplicated.push(sub);
                    }
                    if deduplicated.len() == 1 {
                        values.push(deduplicated[0]);
                    } else {
                        self.nodes[id].children = deduplicated;
                        self.refresh(id);
                        values.push(id);
                    }
                }
            }
        }
        Ok(values.pop().unwrap())
    }

    fn concat(&mut self) -> Result<Id> {
        self.maybe_concat(None);
        let mut begin = self.stack.len();
        while begin > 0 && !self.nodes[self.stack[begin - 1]].op.pseudo() {
            begin -= 1;
        }
        let children = self.stack.split_off(begin);
        let id = if children.is_empty() {
            self.alloc(Op::Empty)
        } else {
            self.collapse(children, Op::Concat)?
        };
        self.push(id)?;
        Ok(id)
    }

    fn alternate(&mut self) -> Result<Id> {
        let mut begin = self.stack.len();
        while begin > 0 && !self.nodes[self.stack[begin - 1]].op.pseudo() {
            begin -= 1;
        }
        let children = self.stack.split_off(begin);
        if let Some(&last) = children.last() {
            self.clean_alternative(last);
        }
        let id = if children.is_empty() {
            self.alloc(Op::NoMatch)
        } else {
            self.collapse(children, Op::Alternate)?
        };
        self.push(id)?;
        Ok(id)
    }

    fn vertical_bar(&mut self) -> Result<()> {
        self.concat()?;
        if self.swap_vertical_bar() {
            return Ok(());
        }
        self.op(Op::VerticalBar)?;
        Ok(())
    }

    fn swap_vertical_bar(&mut self) -> bool {
        let n = self.stack.len();
        if n >= 3
            && self.nodes[self.stack[n - 2]].op == Op::VerticalBar
            && self.is_class(self.stack[n - 1])
            && self.is_class(self.stack[n - 3])
        {
            let mut source = self.stack[n - 1];
            let mut destination = self.stack[n - 3];
            if self.nodes[source].op > self.nodes[destination].op {
                std::mem::swap(&mut source, &mut destination);
                self.stack[n - 3] = destination;
            }
            self.merge_class(destination, source);
            self.stack.pop();
            return true;
        }
        if n >= 2 && self.nodes[self.stack[n - 2]].op == Op::VerticalBar {
            if n >= 3 {
                self.clean_alternative(self.stack[n - 3]);
            }
            self.stack.swap(n - 1, n - 2);
            true
        } else {
            false
        }
    }

    fn close(&mut self) -> Result<()> {
        self.concat()?;
        if self.swap_vertical_bar() {
            self.stack.pop();
        }
        self.alternate()?;
        if self.stack.len() < 2 {
            return Err(Error::new(
                "unexpected )",
                self.input,
                self.context.as_ref(),
            ));
        }
        let child = self.stack.pop().unwrap();
        let open = self.stack.pop().unwrap();
        if self.nodes[open].op != Op::LeftParen {
            return Err(Error::new(
                "unexpected )",
                self.input,
                self.context.as_ref(),
            ));
        }
        self.flags = self.nodes[open].flags;
        if self.nodes[open].capture == 0 {
            self.push(child)?;
        } else {
            self.nodes[open].op = Op::Capture;
            self.nodes[open].children.push(child);
            self.push(open)?;
        }
        Ok(())
    }

    fn flags(&mut self) -> Result<()> {
        let start = self.position;
        let suffix = &self.input[start..];
        let named = if suffix.len() > 4 && suffix.starts_with(b"(?P<") {
            Some(4)
        } else if suffix.len() > 3 && suffix.starts_with(b"(?<") {
            Some(3)
        } else {
            None
        };
        if let Some(name_begin) = named {
            let Some(end) = suffix.iter().position(|&byte| byte == b'>') else {
                check_utf8(suffix, self.context.as_ref())?;
                return Err(Error::new(
                    "invalid named capture",
                    suffix,
                    self.context.as_ref(),
                ));
            };
            let name = &suffix[name_begin..end];
            check_utf8(name, self.context.as_ref())?;
            if name.is_empty()
                || !name
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                return Err(Error::new(
                    "invalid named capture",
                    &suffix[..=end],
                    self.context.as_ref(),
                ));
            }
            self.names
                .push(Buffer::from_slice_in(name, self.context.as_ref()));
            let id = self.op(Op::LeftParen)?;
            self.nodes[id].capture = (self.names.len() - 1) as u32;
            self.nodes[id].name = Buffer::from_slice_in(name, self.context.as_ref());
            self.position += end + 1;
            return Ok(());
        }
        self.position += 2;
        let mut flags = self.flags;
        let mut negative = false;
        let mut saw_flag = false;
        while self.position < self.input.len() {
            let rune = next_rune(self.input, &mut self.position, self.context.as_ref())?;
            match rune {
                105 => {
                    flags |= FOLD_CASE;
                    saw_flag = true;
                }
                109 => {
                    flags &= !ONE_LINE;
                    saw_flag = true;
                }
                115 => {
                    flags |= DOT_NL;
                    saw_flag = true;
                }
                85 => {
                    flags |= NON_GREEDY;
                    saw_flag = true;
                }
                45 if !negative => {
                    negative = true;
                    flags = !flags;
                    saw_flag = false;
                }
                58 | 41 => {
                    if negative {
                        if !saw_flag {
                            break;
                        }
                        flags = !flags;
                    }
                    if rune == 58 {
                        self.op(Op::LeftParen)?;
                    }
                    self.flags = flags;
                    return Ok(());
                }
                _ => break,
            }
        }
        Err(Error::new(
            "invalid or unsupported Perl syntax",
            &self.input[start..self.position],
            self.context.as_ref(),
        ))
    }

    fn escape(&mut self) -> Result<u32> {
        let start = self.position;
        self.position += 1;
        if self.position == self.input.len() {
            return Err(Error::new(
                "trailing backslash at end of expression",
                b"",
                self.context.as_ref(),
            ));
        }
        let rune = next_rune(self.input, &mut self.position, self.context.as_ref())?;
        match rune {
            48..=55
                if rune == 48
                    || self
                        .input
                        .get(self.position)
                        .is_some_and(|byte| (b'0'..=b'7').contains(byte)) =>
            {
                let mut value = rune - 48;
                for _ in 1..3 {
                    let Some(&byte) = self
                        .input
                        .get(self.position)
                        .filter(|byte| (b'0'..=b'7').contains(byte))
                    else {
                        break;
                    };
                    value = value * 8 + u32::from(byte - b'0');
                    self.position += 1;
                }
                return Ok(value);
            }
            120 if self.position < self.input.len() => {
                let first = next_rune(self.input, &mut self.position, self.context.as_ref())?;
                if first == 123 {
                    let mut value = 0;
                    let mut digits = 0;
                    while self.position < self.input.len() {
                        let digit =
                            next_rune(self.input, &mut self.position, self.context.as_ref())?;
                        if digit == 125 {
                            if digits > 0 {
                                return Ok(value);
                            }
                            break;
                        }
                        let Some(digit) = hex(digit) else {
                            break;
                        };
                        value = value * 16 + digit;
                        if value > runes::MAX_RUNE {
                            break;
                        }
                        digits += 1;
                    }
                } else {
                    let second = next_rune(self.input, &mut self.position, self.context.as_ref())?;
                    if let (Some(first), Some(second)) = (hex(first), hex(second)) {
                        return Ok(first * 16 + second);
                    }
                }
            }
            97 => return Ok(7),
            102 => return Ok(12),
            110 => return Ok(10),
            114 => return Ok(13),
            116 => return Ok(9),
            118 => return Ok(11),
            rune if rune < 128 && !(rune as u8).is_ascii_alphanumeric() => return Ok(rune),
            _ => {}
        }
        Err(Error::new(
            "invalid escape sequence",
            &self.input[start..self.position],
            self.context.as_ref(),
        ))
    }

    fn unicode_class(&mut self) -> Result<Option<Class>> {
        if self.flags & UNICODE_GROUPS == 0
            || !self.input[self.position..].starts_with(b"\\p")
                && !self.input[self.position..].starts_with(b"\\P")
        {
            return Ok(None);
        }
        let start = self.position;
        let mut negate = self.input[self.position + 1] == b'P';
        self.position += 2;
        let rune = next_rune(self.input, &mut self.position, self.context.as_ref())?;
        let mut name = if rune != 123 {
            &self.input[start + 2..self.position]
        } else {
            let Some(relative) = self.input[start..].iter().position(|&byte| byte == b'}') else {
                check_utf8(&self.input[start..], self.context.as_ref())?;
                return Err(Error::new(
                    "invalid character class range",
                    &self.input[start..],
                    self.context.as_ref(),
                ));
            };
            self.position = start + relative + 1;
            let name = &self.input[start + 3..self.position - 1];
            check_utf8(name, self.context.as_ref())?;
            name
        };
        if name.starts_with(b"^") {
            name = &name[1..];
            negate = !negate;
        }
        let Some(ranges) = runes::property(
            name,
            self.flags & FOLD_CASE != 0,
            self.standard,
            self.context.as_ref(),
        ) else {
            return Err(Error::new(
                "invalid character class range",
                &self.input[start..self.position],
                self.context.as_ref(),
            ));
        };
        Ok(Some(if negate {
            runes::complement(ranges)
        } else {
            ranges
        }))
    }

    fn perl_class(&mut self) -> Option<Class> {
        if self.flags & PERL_X == 0 || self.input.get(self.position) != Some(&b'\\') {
            return None;
        }
        let (mut ranges, negate) =
            runes::perl(*self.input.get(self.position + 1)?, self.context.as_ref())?;
        self.position += 2;
        if self.flags & FOLD_CASE != 0 {
            ranges = runes::folded(ranges);
        }
        Some(if negate {
            runes::complement(ranges)
        } else {
            ranges
        })
    }

    fn class_char(&mut self, start: usize) -> Result<u32> {
        if self.position == self.input.len() {
            return Err(Error::new(
                "missing closing ]",
                &self.input[start..],
                self.context.as_ref(),
            ));
        }
        if self.input[self.position] == b'\\' {
            self.escape()
        } else {
            next_rune(self.input, &mut self.position, self.context.as_ref())
        }
    }

    fn class(&mut self) -> Result<()> {
        let start = self.position;
        self.position += 1;
        let negate = self.input.get(self.position) == Some(&b'^');
        if negate {
            self.position += 1;
        }
        let mut ranges = Buffer::new_in(self.context.as_ref());
        if negate && self.flags & CLASS_NL == 0 {
            ranges.push((10, 10));
        }
        let mut first = true;
        while first || self.input.get(self.position) != Some(&b']') {
            if self.input.get(self.position) == Some(&b'-')
                && self.flags & PERL_X == 0
                && !first
                && self.input.get(self.position + 1) != Some(&b']')
            {
                let (_, width) = decode_rune(&self.input[self.position + 1..]);
                return Err(Error::new(
                    "invalid character class range",
                    &self.input[self.position..self.position + 1 + width],
                    self.context.as_ref(),
                ));
            }
            first = false;
            if self.input[self.position..].starts_with(b"[:")
                && let Some(relative) = self.input[self.position + 2..]
                    .windows(2)
                    .position(|bytes| bytes == b":]")
            {
                let end = self.position + relative + 4;
                let name = &self.input[self.position + 2..end - 2];
                let Some((mut class, negate)) = runes::posix(name, self.context.as_ref()) else {
                    return Err(Error::new(
                        "invalid character class range",
                        &self.input[self.position..end],
                        self.context.as_ref(),
                    ));
                };
                if self.flags & FOLD_CASE != 0 {
                    class = runes::folded(class);
                }
                ranges.extend(if negate {
                    runes::complement(class)
                } else {
                    class
                });
                self.position = end;
                continue;
            }
            if let Some(class) = self.unicode_class()? {
                ranges.extend(class);
                continue;
            }
            if let Some(class) = self.perl_class() {
                ranges.extend(class);
                continue;
            }
            let range_start = self.position;
            let lo = self.class_char(start)?;
            let hi = if self.input.get(self.position) == Some(&b'-')
                && self
                    .input
                    .get(self.position + 1)
                    .is_some_and(|&byte| byte != b']')
            {
                self.position += 1;
                self.class_char(start)?
            } else {
                lo
            };
            if hi < lo {
                return Err(Error::new(
                    "invalid character class range",
                    &self.input[range_start..self.position],
                    self.context.as_ref(),
                ));
            }
            let class = buffer![self.context.as_ref(); (lo, hi)];
            ranges.extend(if self.flags & FOLD_CASE != 0 {
                runes::folded(class)
            } else {
                class
            });
        }
        self.position += 1;
        let mut class = runes::clean(ranges);
        if negate {
            class = runes::complement(class);
        }
        let id = self.alloc(Op::Class);
        self.nodes[id].class = class;
        self.push(id)
    }

    fn decimal(input: &[u8], position: &mut usize) -> Option<i64> {
        let start = *position;
        if !input.get(start).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        if input[start] == b'0' && input.get(start + 1).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        let mut value = 0i64;
        while let Some(&digit) = input.get(*position).filter(|byte| byte.is_ascii_digit()) {
            if value >= 100_000_000 {
                value = -1;
            } else if value >= 0 {
                value = value * 10 + i64::from(digit - b'0');
            }
            *position += 1;
        }
        Some(value)
    }

    fn repeat_count(&self) -> Option<(i64, i64, usize)> {
        let mut position = self.position + 1;
        let mut min = Self::decimal(self.input, &mut position)?;
        let max = if self.input.get(position) == Some(&b',') {
            position += 1;
            if self.input.get(position) == Some(&b'}') {
                -1
            } else {
                let max = Self::decimal(self.input, &mut position)?;
                if max < 0 {
                    min = -1;
                }
                max
            }
        } else {
            min
        };
        if self.input.get(position) != Some(&b'}') {
            return None;
        }
        Some((min, max, position + 1))
    }

    fn repeat(
        &mut self,
        op: Op,
        min: u32,
        max: Option<u32>,
        start: usize,
        end: usize,
        last: Option<usize>,
    ) -> Result<()> {
        self.position = end;
        let mut flags = self.flags;
        if self.flags & PERL_X != 0 {
            if self.input.get(self.position) == Some(&b'?') {
                self.position += 1;
                flags ^= NON_GREEDY;
            }
            if let Some(last) = last {
                return Err(Error::new(
                    "invalid nested repetition operator",
                    &self.input[last..self.position],
                    self.context.as_ref(),
                ));
            }
        }
        let Some(&child) = self.stack.last().filter(|&&id| !self.nodes[id].op.pseudo()) else {
            return Err(Error::new(
                "missing argument to repetition operator",
                &self.input[start..self.position],
                self.context.as_ref(),
            ));
        };
        let id = self.alloc(op);
        self.nodes[id].flags = flags;
        self.nodes[id].min = min;
        self.nodes[id].max = max;
        self.nodes[id].children.push(child);
        *self.stack.last_mut().unwrap() = id;
        self.limits(id)?;
        if op == Op::Repeat && (min >= 2 || max.is_some_and(|max| max >= 2)) {
            let mut pending = buffer![self.context.as_ref(); (id, 1000)];
            while let Some((id, mut budget)) = pending.pop() {
                let node = &self.nodes[id];
                if node.op == Op::Repeat {
                    if node.max == Some(0) {
                        continue;
                    }
                    let amount = node.max.unwrap_or(node.min);
                    if amount > budget {
                        return Err(Error::new(
                            "invalid repeat count",
                            &self.input[start..self.position],
                            self.context.as_ref(),
                        ));
                    }
                    budget = budget.checked_div(amount).unwrap_or(budget);
                }
                pending.extend(node.children.iter().map(|&child| (child, budget)));
            }
        }
        Ok(())
    }

    fn run(mut self) -> Result<Tree> {
        let mut last_repeat = None;
        while self.position < self.input.len() {
            let mut repeat = None;
            match self.input[self.position] {
                b'(' => {
                    if self.flags & PERL_X != 0 && self.input.get(self.position + 1) == Some(&b'?')
                    {
                        self.flags()?;
                    } else {
                        self.names.push(Buffer::new_in(self.context.as_ref()));
                        let id = self.op(Op::LeftParen)?;
                        self.nodes[id].capture = (self.names.len() - 1) as u32;
                        self.position += 1;
                    }
                }
                b')' => {
                    self.close()?;
                    self.position += 1;
                }
                b'|' => {
                    self.vertical_bar()?;
                    self.position += 1;
                }
                b'^' => {
                    self.op(if self.flags & ONE_LINE != 0 {
                        Op::BeginText
                    } else {
                        Op::BeginLine
                    })?;
                    self.position += 1;
                }
                b'$' => {
                    let id = self.op(if self.flags & ONE_LINE != 0 {
                        Op::EndText
                    } else {
                        Op::EndLine
                    })?;
                    if self.flags & ONE_LINE != 0 {
                        self.nodes[id].flags |= WAS_DOLLAR;
                    }
                    self.position += 1;
                }
                b'.' => {
                    self.op(if self.flags & DOT_NL != 0 {
                        Op::Any
                    } else {
                        Op::AnyNotNL
                    })?;
                    self.position += 1;
                }
                b'[' => self.class()?,
                b'*' | b'+' | b'?' => {
                    let start = self.position;
                    let op = match self.input[start] {
                        b'*' => Op::Star,
                        b'+' => Op::Plus,
                        _ => Op::Quest,
                    };
                    self.repeat(op, 0, None, start, start + 1, last_repeat)?;
                    repeat = Some(start);
                }
                b'{' => {
                    if let Some((min, max, end)) = self.repeat_count() {
                        if !(0..=1000).contains(&min) || max > 1000 || max >= 0 && min > max {
                            return Err(Error::new(
                                "invalid repeat count",
                                &self.input[self.position..end],
                                self.context.as_ref(),
                            ));
                        }
                        let start = self.position;
                        self.repeat(
                            Op::Repeat,
                            min as u32,
                            (max >= 0).then_some(max as u32),
                            start,
                            end,
                            last_repeat,
                        )?;
                        repeat = Some(start);
                    } else {
                        self.literal(u32::from(b'{'))?;
                        self.position += 1;
                    }
                }
                b'\\' => {
                    let escape = self.input.get(self.position + 1).copied();
                    if self.flags & PERL_X != 0
                        && matches!(escape, Some(b'A' | b'b' | b'B' | b'C' | b'Q' | b'z'))
                    {
                        match escape.unwrap() {
                            b'C' => {
                                return Err(Error::new(
                                    "invalid escape sequence",
                                    &self.input[self.position..self.position + 2],
                                    self.context.as_ref(),
                                ));
                            }
                            b'Q' => {
                                self.position += 2;
                                let end = self.input[self.position..]
                                    .windows(2)
                                    .position(|bytes| bytes == b"\\E")
                                    .map(|i| self.position + i);
                                let quoted_end = end.unwrap_or(self.input.len());
                                while self.position < quoted_end {
                                    let rune = next_rune(
                                        &self.input[..quoted_end],
                                        &mut self.position,
                                        self.context.as_ref(),
                                    )?;
                                    self.literal(rune)?;
                                }
                                self.position = end.map_or(quoted_end, |end| end + 2);
                            }
                            byte => {
                                self.op(match byte {
                                    b'A' => Op::BeginText,
                                    b'z' => Op::EndText,
                                    b'b' => Op::WordBoundary,
                                    _ => Op::NoWordBoundary,
                                })?;
                                self.position += 2;
                            }
                        }
                    } else if let Some(class) = self.unicode_class()? {
                        let id = self.alloc(Op::Class);
                        self.nodes[id].class = class;
                        self.push(id)?;
                    } else if let Some(class) = self.perl_class() {
                        let id = self.alloc(Op::Class);
                        self.nodes[id].class = class;
                        self.push(id)?;
                    } else {
                        let rune = self.escape()?;
                        self.literal(rune)?;
                    }
                }
                _ => {
                    let rune = next_rune(self.input, &mut self.position, self.context.as_ref())?;
                    self.literal(rune)?;
                }
            }
            last_repeat = repeat;
        }
        self.concat()?;
        if self.swap_vertical_bar() {
            self.stack.pop();
        }
        self.alternate()?;
        if self.stack.len() != 1 {
            return Err(Error::new(
                "missing closing )",
                self.input,
                self.context.as_ref(),
            ));
        }
        Ok(Tree {
            nodes: self.nodes,
            root: self.stack[0],
            names: self.names,
        })
    }
}
