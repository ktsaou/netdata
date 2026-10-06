// SPDX-License-Identifier: GPL-3.0-or-later
// Owned Go regexp syntax and ordered diagnostics, derived from syntax/parse.go; see NOTICE.
use super::runes::{self, Class};
use super::{AllocationContext, Buffer, Failure, Work};
use crate::frontend::quote::decode_rune;
use crate::storage::Interrupted;

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
    pub(super) fn copy(&self, work: Work<'_>) -> std::result::Result<Self, Interrupted> {
        Ok(Self {
            op: self.op,
            flags: self.flags,
            runes: work.elements(&self.runes)?,
            class: work.elements(&self.class)?,
            children: work.elements(&self.children)?,
            min: self.min,
            max: self.max,
            capture: self.capture,
            name: work.copy(&self.name)?,
            height: self.height,
            size: self.size,
        })
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
    fn failure(code: &'static str, expression: &[u8], work: Work<'_>) -> Failure<Self> {
        match work.copy(expression) {
            Ok(expression) => Failure::Error(Self { code, expression }),
            Err(error) => Failure::Interrupted(error),
        }
    }
    #[cfg(feature = "fixtures")]
    pub fn bytes(&self) -> Buffer<u8> {
        let context = self.expression.context();
        self.bytes_checked(Work::new(context.as_ref(), None))
            .expect("inert regex diagnostic")
    }
    pub(super) fn bytes_checked(
        &self,
        work: Work<'_>,
    ) -> std::result::Result<Buffer<u8>, Interrupted> {
        let mut result = work.buffer();
        for bytes in [
            b"error parsing regexp: ".as_slice(),
            self.code.as_bytes(),
            b": `",
            &self.expression,
            b"`",
        ] {
            work.extend(&mut result, bytes)?;
        }
        Ok(result)
    }
}

type Result<T> = std::result::Result<T, Failure<Error>>;

struct Parser<'a, 'w> {
    work: Work<'w>,
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
pub fn parse(input: &[u8], flags: Flags) -> std::result::Result<Tree, Error> {
    super::inert(parse_profile(input, flags, false, Work::default()))
}

#[cfg(feature = "fixtures")]
pub fn parse_standard(input: &[u8], flags: Flags) -> std::result::Result<Tree, Error> {
    super::inert(parse_profile(input, flags, true, Work::default()))
}

pub(super) fn parse_in(input: &[u8], flags: Flags, work: Work<'_>) -> Result<Tree> {
    parse_profile(input, flags, false, work)
}

pub(super) fn parse_standard_in(input: &[u8], flags: Flags, work: Work<'_>) -> Result<Tree> {
    parse_profile(input, flags, true, work)
}

fn parse_profile(input: &[u8], flags: Flags, standard: bool, work: Work<'_>) -> Result<Tree> {
    work.step(0)?;
    let context = work.context;
    if flags & LITERAL != 0 {
        check_utf8(input, work)?;
        let mut position = 0;
        let mut node = Node::new(Op::Literal, flags, context);
        while position < input.len() {
            let rune = next_rune(input, &mut position, work)?;
            node.runes.push(rune);
        }
        return Ok(Tree {
            nodes: buffer![context; node],
            root: 0,
            names: buffer![context; Buffer::new_in(context)],
        });
    }
    Parser {
        work,
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

fn next_rune(input: &[u8], position: &mut usize, work: Work<'_>) -> Result<u32> {
    work.step(1)?;
    let (rune, width) = decode_rune(&input[*position..]);
    if rune == 0xfffd && width == 1 {
        return Err(Error::failure("invalid UTF-8", &input[*position..], work));
    }
    *position += width;
    Ok(if width == 0 { 0xfffd } else { rune as u32 })
}

fn check_utf8(mut input: &[u8], work: Work<'_>) -> Result<()> {
    while !input.is_empty() {
        let mut used = 0;
        next_rune(input, &mut used, work)?;
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

impl Parser<'_, '_> {
    fn alloc(&mut self, op: Op) -> Id {
        let id = self.nodes.len();
        self.nodes
            .push(Node::new(op, self.flags, self.work.context));
        id
    }

    fn refresh(&mut self, id: Id) -> Result<()> {
        self.work.step(1)?;
        let node = &self.nodes[id];
        let sub = node
            .children
            .first()
            .map_or(0, |&child| self.nodes[child].size);
        let (mut sum, mut maximum) = (0, 0);
        for &child in &node.children {
            self.work.step(1)?;
            sum += self.nodes[child].size;
            maximum = maximum.max(self.nodes[child].height);
        }
        let height = 1 + maximum;
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
        Ok(())
    }

    fn limits(&mut self, id: Id) -> Result<()> {
        self.refresh(id)?;
        if self.rune_count > MAX_RUNES || self.nodes[id].size > MAX_SIZE {
            return Err(Error::failure(
                "expression too large",
                self.input,
                self.work,
            ));
        }
        if self.nodes[id].height > MAX_HEIGHT {
            return Err(Error::failure(
                "expression nests too deeply",
                self.input,
                self.work,
            ));
        }
        Ok(())
    }

    fn maybe_concat(&mut self, next: Option<(u32, Flags)>) -> Result<bool> {
        if self.stack.len() < 2 {
            return Ok(false);
        }
        let first = self.stack[self.stack.len() - 2];
        let last = self.stack[self.stack.len() - 1];
        if self.nodes[first].op != Op::Literal
            || self.nodes[last].op != Op::Literal
            || self.nodes[first].flags & FOLD_CASE != self.nodes[last].flags & FOLD_CASE
        {
            return Ok(false);
        }
        let runes = std::mem::take(&mut self.nodes[last].runes);
        self.work.extend_iter(&mut self.nodes[first].runes, runes)?;
        self.refresh(first)?;
        if let Some((rune, flags)) = next {
            self.nodes[last].runes.push(rune);
            self.nodes[last].flags = flags;
            self.refresh(last)?;
            Ok(true)
        } else {
            self.stack.pop();
            Ok(false)
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
                if self.maybe_concat(Some((rune, flags)))? {
                    return Ok(());
                }
                self.nodes[id].op = Op::Literal;
                self.nodes[id].runes = buffer![self.work.context; rune];
                self.nodes[id].class.clear();
                self.nodes[id].flags = flags;
            } else {
                self.maybe_concat(None)?;
            }
        } else {
            self.maybe_concat(None)?;
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

    fn character_class(&self, id: Id) -> Result<Class> {
        Ok(match self.nodes[id].op {
            Op::Any => buffer![self.work.context; (0, runes::MAX_RUNE)],
            Op::AnyNotNL => buffer![self.work.context; (0, 9), (11, runes::MAX_RUNE)],
            Op::Class => self.work.elements(&self.nodes[id].class)?,
            Op::Literal if self.nodes[id].flags & FOLD_CASE != 0 => {
                let rune = self.nodes[id].runes[0];
                runes::folded(buffer![self.work.context; (rune, rune)], self.work)?
            }
            Op::Literal => {
                buffer![self.work.context; (self.nodes[id].runes[0], self.nodes[id].runes[0])]
            }
            _ => unreachable!("character-like expression"),
        })
    }

    fn merge_class(&mut self, destination: Id, source: Id) -> Result<()> {
        if self.nodes[destination].op == Op::Any {
            return Ok(());
        }
        if self.nodes[destination].op == Op::AnyNotNL {
            if runes::contains(&self.character_class(source)?, 10) {
                self.nodes[destination].op = Op::Any;
            }
            return Ok(());
        }
        if self.nodes[destination].op == Op::Literal
            && self.nodes[source].op == Op::Literal
            && self.nodes[destination].runes == self.nodes[source].runes
            && self.nodes[destination].flags == self.nodes[source].flags
        {
            return Ok(());
        }
        let mut class = self.character_class(destination)?;
        self.work
            .extend_iter(&mut class, self.character_class(source)?)?;
        self.nodes[destination].op = Op::Class;
        self.nodes[destination].runes.clear();
        self.nodes[destination].class = class;
        self.refresh(destination)?;
        Ok(())
    }

    fn clean_alternative(&mut self, id: Id) -> Result<()> {
        if self.nodes[id].op == Op::Class {
            self.nodes[id].class =
                runes::clean(std::mem::take(&mut self.nodes[id].class), self.work)?;
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
            self.refresh(id)?;
        }
        Ok(())
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

    fn remove_string(&mut self, id: Id, count: usize) -> Result<Id> {
        let leading = self.leading(id);
        if self.nodes[leading].op != Op::Literal {
            return Ok(id);
        }
        self.work
            .discard_prefix(&mut self.nodes[leading].runes, count)?;
        if self.nodes[leading].runes.is_empty() {
            self.nodes[leading].op = Op::Empty;
        }
        self.refresh(leading)?;
        if leading != id && self.nodes[leading].op == Op::Empty {
            self.work.discard_prefix(&mut self.nodes[id].children, 1)?;
            if self.nodes[id].children.len() == 1 {
                return Ok(self.nodes[id].children[0]);
            }
            if self.nodes[id].children.is_empty() {
                self.nodes[id].op = Op::Empty;
            }
        }
        self.refresh(id)?;
        Ok(id)
    }

    fn remove_leading(&mut self, id: Id) -> Result<Id> {
        if self.nodes[id].op == Op::Concat && !self.nodes[id].children.is_empty() {
            self.work.discard_prefix(&mut self.nodes[id].children, 1)?;
            if self.nodes[id].children.len() == 1 {
                return Ok(self.nodes[id].children[0]);
            }
            if self.nodes[id].children.is_empty() {
                self.nodes[id].op = Op::Empty;
            }
            self.refresh(id)?;
            Ok(id)
        } else {
            Ok(self.alloc(Op::Empty))
        }
    }

    fn equal(&self, left: Id, right: Id) -> Result<bool> {
        let mut pending = buffer![self.work.context; (left, right)];
        while let Some((left, right)) = pending.pop() {
            self.work.step(1)?;
            let left = &self.nodes[left];
            let right = &self.nodes[right];
            if left.op != right.op {
                return Ok(false);
            }
            match left.op {
                Op::EndText if left.flags & WAS_DOLLAR != right.flags & WAS_DOLLAR => {
                    return Ok(false);
                }
                Op::Literal | Op::Class => {
                    if !self.work.same(&left.runes, &right.runes)?
                        || !self.work.same(&left.class, &right.class)?
                        || self.standard && left.flags & FOLD_CASE != right.flags & FOLD_CASE
                    {
                        return Ok(false);
                    }
                }
                Op::Star | Op::Plus | Op::Quest | Op::Repeat => {
                    if left.flags & NON_GREEDY != right.flags & NON_GREEDY
                        || left.op == Op::Repeat && (left.min != right.min || left.max != right.max)
                    {
                        return Ok(false);
                    }
                }
                Op::Capture
                    if left.capture != right.capture
                        || !self.work.equal(&left.name, &right.name)? =>
                {
                    return Ok(false);
                }
                _ => {}
            }
            if left.children.len() != right.children.len() {
                return Ok(false);
            }
            self.work.extend_iter(
                &mut pending,
                left.children
                    .iter()
                    .copied()
                    .zip(right.children.iter().copied()),
            )?;
        }
        Ok(true)
    }

    fn factored_concat(&mut self, prefix: Id, suffix: Id) -> Result<Id> {
        let id = self.alloc(Op::Concat);
        self.nodes[id].children = buffer![self.work.context; prefix, suffix];
        self.refresh(id)?;
        Ok(id)
    }

    fn collapse(&mut self, children: Buffer<Id>, op: Op) -> Result<Id> {
        let mut frames = buffer![self.work.context; CollapseFrame::Collapse(children, op)];
        let mut values = Buffer::new_in(self.work.context);
        'frames: while let Some(frame) = frames.pop() {
            self.work.step(1)?;
            match frame {
                CollapseFrame::Collapse(children, op) => {
                    if children.len() == 1 {
                        values.push(children[0]);
                        continue;
                    }
                    let id = self.alloc(op);
                    let mut subs = Buffer::new_in(self.work.context);
                    for child in children {
                        self.work.step(1)?;
                        if self.nodes[child].op == op {
                            self.work.extend_iter(
                                &mut subs,
                                self.nodes[child].children.iter().copied(),
                            )?;
                        } else {
                            subs.push(child);
                        }
                    }
                    if op == Op::Alternate {
                        frames.push(CollapseFrame::Literal {
                            id,
                            subs,
                            index: 0,
                            output: Buffer::new_in(self.work.context),
                        });
                    } else {
                        self.nodes[id].children = subs;
                        self.refresh(id)?;
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
                        self.work.step(1)?;
                        let (prefix, flags) = self.leading_string(subs[index]);
                        let mut prefix = self.work.elements(prefix)?;
                        let mut end = index + 1;
                        while end < subs.len() {
                            self.work.step(1)?;
                            let (next, next_flags) = self.leading_string(subs[end]);
                            let mut same = 0;
                            for (a, b) in prefix.iter().zip(next) {
                                self.work.step(1)?;
                                if a != b {
                                    break;
                                }
                                same += 1;
                            }
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
                        self.refresh(prefix_id)?;
                        for sub in &mut subs[index..end] {
                            self.work.step(1)?;
                            *sub = self.remove_string(*sub, self.nodes[prefix_id].runes.len())?;
                            self.limits(*sub)?;
                        }
                        let suffixes = self.work.elements(&subs[index..end])?;
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
                        output: Buffer::new_in(self.work.context),
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
                    output.push(self.factored_concat(prefix, suffix)?);
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
                        self.work.step(1)?;
                        let first = self.leading(subs[index]);
                        let node = &self.nodes[first];
                        let eligible = self.is_class(first)
                            || node.op == Op::Repeat
                                && node.max == Some(node.min)
                                && self.is_class(node.children[0]);
                        let mut end = index + 1;
                        while eligible
                            && end < subs.len()
                            && self.equal(first, self.leading(subs[end]))?
                        {
                            self.work.step(1)?;
                            end += 1;
                        }
                        if end == index + 1 {
                            output.push(subs[index]);
                            index = end;
                            continue;
                        }
                        for sub in &mut subs[index..end] {
                            self.work.step(1)?;
                            *sub = self.remove_leading(*sub)?;
                            self.limits(*sub)?;
                        }
                        let suffixes = self.work.elements(&subs[index..end])?;
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
                    output.push(self.factored_concat(prefix, suffix)?);
                    frames.push(CollapseFrame::Simple {
                        id,
                        subs,
                        index,
                        output,
                    });
                }
                CollapseFrame::Classes { id, subs } => {
                    let mut output = Buffer::new_in(self.work.context);
                    let mut index = 0;
                    while index < subs.len() {
                        self.work.step(1)?;
                        if !self.is_class(subs[index]) {
                            output.push(subs[index]);
                            index += 1;
                            continue;
                        }
                        let mut end = index + 1;
                        while end < subs.len() && self.is_class(subs[end]) {
                            self.work.step(1)?;
                            end += 1;
                        }
                        let mut destination = subs[index];
                        for &candidate in &subs[index + 1..end] {
                            self.work.step(1)?;
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
                            self.work.step(1)?;
                            if source != destination {
                                self.merge_class(destination, source)?;
                            }
                        }
                        self.clean_alternative(destination)?;
                        output.push(destination);
                        index = end;
                    }
                    let mut deduplicated = Buffer::new_in(self.work.context);
                    for (index, &sub) in output.iter().enumerate() {
                        self.work.step(1)?;
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
                        self.refresh(id)?;
                        values.push(id);
                    }
                }
            }
        }
        Ok(values.pop().unwrap())
    }

    fn concat(&mut self) -> Result<Id> {
        self.maybe_concat(None)?;
        let mut begin = self.stack.len();
        while begin > 0 && !self.nodes[self.stack[begin - 1]].op.pseudo() {
            self.work.step(1)?;
            begin -= 1;
        }
        let children = self.work.split(&mut self.stack, begin)?;
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
            self.work.step(1)?;
            begin -= 1;
        }
        let children = self.work.split(&mut self.stack, begin)?;
        if let Some(&last) = children.last() {
            self.clean_alternative(last)?;
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
        if self.swap_vertical_bar()? {
            return Ok(());
        }
        self.op(Op::VerticalBar)?;
        Ok(())
    }

    fn swap_vertical_bar(&mut self) -> Result<bool> {
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
            self.merge_class(destination, source)?;
            self.stack.pop();
            return Ok(true);
        }
        if n >= 2 && self.nodes[self.stack[n - 2]].op == Op::VerticalBar {
            if n >= 3 {
                self.clean_alternative(self.stack[n - 3])?;
            }
            self.stack.swap(n - 1, n - 2);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn close(&mut self) -> Result<()> {
        self.concat()?;
        if self.swap_vertical_bar()? {
            self.stack.pop();
        }
        self.alternate()?;
        if self.stack.len() < 2 {
            return Err(Error::failure("unexpected )", self.input, self.work));
        }
        let child = self.stack.pop().unwrap();
        let open = self.stack.pop().unwrap();
        if self.nodes[open].op != Op::LeftParen {
            return Err(Error::failure("unexpected )", self.input, self.work));
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

    fn valid_capture_name(&self, name: &[u8]) -> Result<bool> {
        for &byte in name {
            self.work.step(1)?;
            if !byte.is_ascii_alphanumeric() && byte != b'_' {
                return Ok(false);
            }
        }
        Ok(true)
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
            let Some(end) = self.work.find(suffix, b">")? else {
                check_utf8(suffix, self.work)?;
                return Err(Error::failure("invalid named capture", suffix, self.work));
            };
            let name = &suffix[name_begin..end];
            check_utf8(name, self.work)?;
            if name.is_empty() || !self.valid_capture_name(name)? {
                return Err(Error::failure(
                    "invalid named capture",
                    &suffix[..=end],
                    self.work,
                ));
            }
            self.names.push(self.work.copy(name)?);
            let id = self.op(Op::LeftParen)?;
            self.nodes[id].capture = (self.names.len() - 1) as u32;
            self.nodes[id].name = self.work.copy(name)?;
            self.position += end + 1;
            return Ok(());
        }
        self.position += 2;
        let mut flags = self.flags;
        let mut negative = false;
        let mut saw_flag = false;
        while self.position < self.input.len() {
            self.work.step(1)?;
            let rune = next_rune(self.input, &mut self.position, self.work)?;
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
        Err(Error::failure(
            "invalid or unsupported Perl syntax",
            &self.input[start..self.position],
            self.work,
        ))
    }

    fn escape(&mut self) -> Result<u32> {
        let start = self.position;
        self.position += 1;
        if self.position == self.input.len() {
            return Err(Error::failure(
                "trailing backslash at end of expression",
                b"",
                self.work,
            ));
        }
        let rune = next_rune(self.input, &mut self.position, self.work)?;
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
                    self.work.step(1)?;
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
                let first = next_rune(self.input, &mut self.position, self.work)?;
                if first == 123 {
                    let mut value = 0;
                    let mut digits = 0;
                    while self.position < self.input.len() {
                        self.work.step(1)?;
                        let digit = next_rune(self.input, &mut self.position, self.work)?;
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
                    let second = next_rune(self.input, &mut self.position, self.work)?;
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
        Err(Error::failure(
            "invalid escape sequence",
            &self.input[start..self.position],
            self.work,
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
        let rune = next_rune(self.input, &mut self.position, self.work)?;
        let mut name = if rune != 123 {
            &self.input[start + 2..self.position]
        } else {
            let Some(relative) = self.work.find(&self.input[start..], b"}")? else {
                check_utf8(&self.input[start..], self.work)?;
                return Err(Error::failure(
                    "invalid character class range",
                    &self.input[start..],
                    self.work,
                ));
            };
            self.position = start + relative + 1;
            let name = &self.input[start + 3..self.position - 1];
            check_utf8(name, self.work)?;
            name
        };
        if name.starts_with(b"^") {
            name = &name[1..];
            negate = !negate;
        }
        let Some(ranges) =
            runes::property(name, self.flags & FOLD_CASE != 0, self.standard, self.work)?
        else {
            return Err(Error::failure(
                "invalid character class range",
                &self.input[start..self.position],
                self.work,
            ));
        };
        Ok(Some(if negate {
            runes::complement(ranges, self.work)?
        } else {
            ranges
        }))
    }

    fn perl_class(&mut self) -> Result<Option<Class>> {
        self.work.step(0)?;
        if self.flags & PERL_X == 0 || self.input.get(self.position) != Some(&b'\\') {
            return Ok(None);
        }
        let Some(&name) = self.input.get(self.position + 1) else {
            return Ok(None);
        };
        let Some((mut ranges, negate)) = runes::perl(name, self.work.context) else {
            return Ok(None);
        };
        self.position += 2;
        if self.flags & FOLD_CASE != 0 {
            ranges = runes::folded(ranges, self.work)?;
        }
        Ok(Some(if negate {
            runes::complement(ranges, self.work)?
        } else {
            ranges
        }))
    }

    fn class_char(&mut self, start: usize) -> Result<u32> {
        if self.position == self.input.len() {
            return Err(Error::failure(
                "missing closing ]",
                &self.input[start..],
                self.work,
            ));
        }
        if self.input[self.position] == b'\\' {
            self.escape()
        } else {
            next_rune(self.input, &mut self.position, self.work)
        }
    }

    fn class(&mut self) -> Result<()> {
        let start = self.position;
        self.position += 1;
        let negate = self.input.get(self.position) == Some(&b'^');
        if negate {
            self.position += 1;
        }
        let mut ranges = Buffer::new_in(self.work.context);
        if negate && self.flags & CLASS_NL == 0 {
            ranges.push((10, 10));
        }
        let mut first = true;
        while first || self.input.get(self.position) != Some(&b']') {
            self.work.step(1)?;
            if self.input.get(self.position) == Some(&b'-')
                && self.flags & PERL_X == 0
                && !first
                && self.input.get(self.position + 1) != Some(&b']')
            {
                let (_, width) = decode_rune(&self.input[self.position + 1..]);
                return Err(Error::failure(
                    "invalid character class range",
                    &self.input[self.position..self.position + 1 + width],
                    self.work,
                ));
            }
            first = false;
            if self.input[self.position..].starts_with(b"[:")
                && let Some(relative) = self.work.find(&self.input[self.position + 2..], b":]")?
            {
                let end = self.position + relative + 4;
                let name = &self.input[self.position + 2..end - 2];
                let Some((mut class, negate)) = runes::posix(name, self.work.context) else {
                    return Err(Error::failure(
                        "invalid character class range",
                        &self.input[self.position..end],
                        self.work,
                    ));
                };
                if self.flags & FOLD_CASE != 0 {
                    class = runes::folded(class, self.work)?;
                }
                self.work.extend_iter(
                    &mut ranges,
                    if negate {
                        runes::complement(class, self.work)?
                    } else {
                        class
                    },
                )?;
                self.position = end;
                continue;
            }
            if let Some(class) = self.unicode_class()? {
                self.work.extend_iter(&mut ranges, class)?;
                continue;
            }
            if let Some(class) = self.perl_class()? {
                self.work.extend_iter(&mut ranges, class)?;
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
                return Err(Error::failure(
                    "invalid character class range",
                    &self.input[range_start..self.position],
                    self.work,
                ));
            }
            let class = buffer![self.work.context; (lo, hi)];
            self.work.extend_iter(
                &mut ranges,
                if self.flags & FOLD_CASE != 0 {
                    runes::folded(class, self.work)?
                } else {
                    class
                },
            )?;
        }
        self.position += 1;
        let mut class = runes::clean(ranges, self.work)?;
        if negate {
            class = runes::complement(class, self.work)?;
        }
        let id = self.alloc(Op::Class);
        self.nodes[id].class = class;
        self.push(id)
    }

    fn decimal(input: &[u8], position: &mut usize, work: Work<'_>) -> Result<Option<i64>> {
        let start = *position;
        if !input.get(start).is_some_and(u8::is_ascii_digit) {
            return Ok(None);
        }
        if input[start] == b'0' && input.get(start + 1).is_some_and(u8::is_ascii_digit) {
            return Ok(None);
        }
        let mut value = 0i64;
        while let Some(&digit) = input.get(*position).filter(|byte| byte.is_ascii_digit()) {
            work.step(1)?;
            if value >= 100_000_000 {
                value = -1;
            } else if value >= 0 {
                value = value * 10 + i64::from(digit - b'0');
            }
            *position += 1;
        }
        Ok(Some(value))
    }

    fn repeat_count(&self) -> Result<Option<(i64, i64, usize)>> {
        let mut position = self.position + 1;
        let Some(mut min) = Self::decimal(self.input, &mut position, self.work)? else {
            return Ok(None);
        };
        let max = if self.input.get(position) == Some(&b',') {
            position += 1;
            if self.input.get(position) == Some(&b'}') {
                -1
            } else {
                let Some(max) = Self::decimal(self.input, &mut position, self.work)? else {
                    return Ok(None);
                };
                if max < 0 {
                    min = -1;
                }
                max
            }
        } else {
            min
        };
        if self.input.get(position) != Some(&b'}') {
            return Ok(None);
        }
        Ok(Some((min, max, position + 1)))
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
                return Err(Error::failure(
                    "invalid nested repetition operator",
                    &self.input[last..self.position],
                    self.work,
                ));
            }
        }
        let Some(&child) = self.stack.last().filter(|&&id| !self.nodes[id].op.pseudo()) else {
            return Err(Error::failure(
                "missing argument to repetition operator",
                &self.input[start..self.position],
                self.work,
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
            let mut pending = buffer![self.work.context; (id, 1000)];
            while let Some((id, mut budget)) = pending.pop() {
                self.work.step(1)?;
                let node = &self.nodes[id];
                if node.op == Op::Repeat {
                    if node.max == Some(0) {
                        continue;
                    }
                    let amount = node.max.unwrap_or(node.min);
                    if amount > budget {
                        return Err(Error::failure(
                            "invalid repeat count",
                            &self.input[start..self.position],
                            self.work,
                        ));
                    }
                    budget = budget.checked_div(amount).unwrap_or(budget);
                }
                self.work.extend_iter(
                    &mut pending,
                    node.children.iter().map(|&child| (child, budget)),
                )?;
            }
        }
        Ok(())
    }

    fn run(mut self) -> Result<Tree> {
        let mut last_repeat = None;
        while self.position < self.input.len() {
            self.work.step(1)?;
            let mut repeat = None;
            match self.input[self.position] {
                b'(' => {
                    if self.flags & PERL_X != 0 && self.input.get(self.position + 1) == Some(&b'?')
                    {
                        self.flags()?;
                    } else {
                        self.names.push(Buffer::new_in(self.work.context));
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
                    if let Some((min, max, end)) = self.repeat_count()? {
                        if !(0..=1000).contains(&min) || max > 1000 || max >= 0 && min > max {
                            return Err(Error::failure(
                                "invalid repeat count",
                                &self.input[self.position..end],
                                self.work,
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
                                return Err(Error::failure(
                                    "invalid escape sequence",
                                    &self.input[self.position..self.position + 2],
                                    self.work,
                                ));
                            }
                            b'Q' => {
                                self.position += 2;
                                let end = self
                                    .work
                                    .find(&self.input[self.position..], b"\\E")?
                                    .map(|i| self.position + i);
                                let quoted_end = end.unwrap_or(self.input.len());
                                while self.position < quoted_end {
                                    self.work.step(1)?;
                                    let rune = next_rune(
                                        &self.input[..quoted_end],
                                        &mut self.position,
                                        self.work,
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
                    } else if let Some(class) = self.perl_class()? {
                        let id = self.alloc(Op::Class);
                        self.nodes[id].class = class;
                        self.push(id)?;
                    } else {
                        let rune = self.escape()?;
                        self.literal(rune)?;
                    }
                }
                _ => {
                    let rune = next_rune(self.input, &mut self.position, self.work)?;
                    self.literal(rune)?;
                }
            }
            last_repeat = repeat;
        }
        self.concat()?;
        if self.swap_vertical_bar()? {
            self.stack.pop();
        }
        self.alternate()?;
        if self.stack.len() != 1 {
            return Err(Error::failure("missing closing )", self.input, self.work));
        }
        Ok(Tree {
            nodes: self.nodes,
            root: self.stack[0],
            names: self.names,
        })
    }
}
