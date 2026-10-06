// SPDX-License-Identifier: GPL-3.0-or-later
// Preserve Prometheus FastRegexMatcher's observable optimization order; see NOTICE.
use std::collections::{HashMap, HashSet};

use super::{
    AllocationContext, Buffer, GoRegex, normalization, runes, string,
    syntax::{self, FOLD_CASE, Id, Op, Tree},
};
use crate::accounting::{MemoryKind, MemoryLease};
use crate::frontend::quote::decode_rune;

const SET_THRESHOLD: usize = 256;
const MAP_THRESHOLD: usize = 16;

fn table_bytes<T>(capacity: usize) -> u64 {
    if capacity == 0 {
        return 0;
    }
    // Public usable capacity cannot expose bucket layout or hidden growth peaks.
    capacity
        .checked_mul(size_of::<T>() + 1)
        .and_then(|bytes| bytes.checked_add(16))
        .expect("regex table estimate overflow") as u64
}
fn table_owner(context: Option<&AllocationContext>) -> Option<MemoryLease> {
    context.map(|context| context.with_kind(MemoryKind::RegexEstimate).hold(0))
}

#[derive(Debug)]
struct ByteSet {
    values: HashSet<Buffer<u8>>,
    estimate: Option<MemoryLease>,
}
impl ByteSet {
    fn from_values(
        values: impl IntoIterator<Item = Buffer<u8>>,
        context: Option<&AllocationContext>,
    ) -> Self {
        let mut result = Self {
            values: HashSet::new(),
            estimate: table_owner(context),
        };
        for value in values {
            result.values.insert(value);
            if let Some(estimate) = &mut result.estimate {
                estimate.resize(table_bytes::<Buffer<u8>>(result.values.capacity()));
            }
        }
        result
    }
    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    fn contains(&self, value: &[u8]) -> bool {
        self.values.contains(value)
    }
}

#[derive(Debug)]
struct ByteMap {
    values: HashMap<Buffer<u8>, Buffer<usize>>,
    estimate: Option<MemoryLease>,
}
impl ByteMap {
    fn new(context: Option<&AllocationContext>) -> Self {
        Self {
            values: HashMap::new(),
            estimate: table_owner(context),
        }
    }
    fn push(&mut self, key: Buffer<u8>, value: usize, context: Option<&AllocationContext>) {
        self.values
            .entry(key)
            .or_insert_with(|| Buffer::new_in(context))
            .push(value);
        if let Some(estimate) = &mut self.estimate {
            estimate.resize(table_bytes::<(Buffer<u8>, Buffer<usize>)>(
                self.values.capacity(),
            ));
        }
    }
    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    fn get(&self, key: &[u8]) -> Option<&Buffer<usize>> {
        self.values.get(key)
    }
}

fn sensitive(tree: &Tree, id: Id) -> bool {
    tree.nodes[id].flags & FOLD_CASE == 0
}

fn clear_capture(tree: &mut Tree, id: Id) {
    while tree.nodes[id].op == Op::Capture {
        tree.nodes[id] = tree.nodes[tree.nodes[id].children[0]].clone();
    }
}

fn clear_anchors(tree: &mut Tree, id: Id) {
    let node = &tree.nodes[id];
    if node.op == Op::Alternate || node.children.is_empty() {
        return;
    }
    if node.children.len() == 1
        && matches!(tree.nodes[node.children[0]].op, Op::BeginText | Op::EndText)
    {
        tree.nodes[id].op = Op::Empty;
        tree.nodes[id].children.clear();
        return;
    }
    let mut children = tree.nodes[id].children.clone();
    if tree.nodes[children[0]].op == Op::BeginText {
        children.remove(0);
    }
    if children
        .last()
        .is_some_and(|&child| tree.nodes[child].op == Op::EndText)
    {
        children.pop();
    }
    tree.nodes[id].children = children;
}

#[derive(Debug)]
struct Set {
    values: Buffer<Buffer<u8>>,
    sensitive: bool,
}

enum SetFrame {
    Visit(Id, Buffer<u8>),
    Alternate {
        children: Buffer<Id>,
        base: Buffer<u8>,
        index: usize,
        values: Buffer<Buffer<u8>>,
        sensitive: Option<bool>,
    },
    Concat {
        children: Buffer<Id>,
        child: usize,
        prefixes: Buffer<Buffer<u8>>,
        prefix: usize,
        values: Buffer<Buffer<u8>>,
        sensitive: Option<bool>,
    },
}

fn append_set(
    values: &mut Buffer<Buffer<u8>>,
    sensitivity: &mut Option<bool>,
    found: Option<Set>,
) -> bool {
    let Some(found) = found else {
        return false;
    };
    if values.len() + found.values.len() > SET_THRESHOLD
        || sensitivity.is_some_and(|s| s != found.sensitive)
    {
        return false;
    }
    *sensitivity = Some(found.sensitive);
    values.extend(found.values);
    true
}

fn find_set(tree: &mut Tree, id: Id) -> Option<Set> {
    let context = tree.context();
    let mut frames =
        buffer![context.as_ref(); SetFrame::Visit(id, Buffer::new_in(context.as_ref()))];
    let mut results = Buffer::<Option<Set>>::new_in(context.as_ref());
    while let Some(frame) = frames.pop() {
        match frame {
            SetFrame::Visit(id, base) => {
                clear_capture(tree, id);
                let node = &tree.nodes[id];
                match node.op {
                    Op::Literal => {
                        let mut value = base;
                        value.extend(runes::text(&node.runes, context.as_ref()));
                        results.push(Some(Set {
                            values: buffer![context.as_ref(); value],
                            sensitive: sensitive(tree, id),
                        }));
                    }
                    Op::Empty if !base.is_empty() => results.push(Some(Set {
                        values: buffer![context.as_ref(); base],
                        sensitive: sensitive(tree, id),
                    })),
                    Op::Class
                        if !node.class.is_empty()
                            && node
                                .class
                                .iter()
                                .map(|&(lo, hi)| u64::from(hi - lo) + 1)
                                .sum::<u64>()
                                <= SET_THRESHOLD as u64 =>
                    {
                        let mut values = Buffer::new_in(context.as_ref());
                        for &(lo, hi) in &node.class {
                            for rune in lo..=hi {
                                let mut value = base.clone();
                                value.extend(runes::text(&[rune], context.as_ref()));
                                values.push(value);
                            }
                        }
                        results.push(Some(Set {
                            values,
                            sensitive: sensitive(tree, id),
                        }));
                    }
                    Op::Alternate if !node.children.is_empty() => {
                        let children = node.children.clone();
                        let first = children[0];
                        frames.push(SetFrame::Alternate {
                            children,
                            base: base.clone(),
                            index: 0,
                            values: Buffer::new_in(context.as_ref()),
                            sensitive: None,
                        });
                        frames.push(SetFrame::Visit(first, base));
                    }
                    Op::Concat if !node.children.is_empty() => {
                        let children = node.children.clone();
                        for &child in &children {
                            clear_capture(tree, child);
                        }
                        let first = children[0];
                        frames.push(SetFrame::Concat {
                            children,
                            child: 0,
                            prefixes: buffer![context.as_ref(); base.clone()],
                            prefix: 0,
                            values: Buffer::new_in(context.as_ref()),
                            sensitive: None,
                        });
                        frames.push(SetFrame::Visit(first, base));
                    }
                    _ => results.push(None),
                }
            }
            SetFrame::Alternate {
                children,
                base,
                index,
                mut values,
                mut sensitive,
            } => {
                if !append_set(&mut values, &mut sensitive, results.pop().unwrap()) {
                    results.push(None);
                } else if index + 1 == children.len() {
                    results.push(Some(Set {
                        values,
                        sensitive: sensitive.unwrap(),
                    }));
                } else {
                    let next = children[index + 1];
                    frames.push(SetFrame::Alternate {
                        children,
                        base: base.clone(),
                        index: index + 1,
                        values,
                        sensitive,
                    });
                    frames.push(SetFrame::Visit(next, base));
                }
            }
            SetFrame::Concat {
                children,
                mut child,
                mut prefixes,
                mut prefix,
                mut values,
                mut sensitive,
            } => {
                if !append_set(&mut values, &mut sensitive, results.pop().unwrap()) {
                    results.push(None);
                    continue;
                }
                prefix += 1;
                if prefix == prefixes.len() {
                    child += 1;
                    if child == children.len() {
                        results.push(Some(Set {
                            values,
                            sensitive: sensitive.unwrap(),
                        }));
                        continue;
                    }
                    prefixes = values;
                    values = Buffer::new_in(context.as_ref());
                    prefix = 0;
                }
                let next = children[child];
                let base = prefixes[prefix].clone();
                frames.push(SetFrame::Concat {
                    children,
                    child,
                    prefixes,
                    prefix,
                    values,
                    sensitive,
                });
                frames.push(SetFrame::Visit(next, base));
            }
        }
    }
    results.pop().unwrap()
}

fn is_any(tree: &Tree, id: Id) -> bool {
    let node = &tree.nodes[id];
    node.op == Op::Star && tree.nodes[node.children[0]].op == Op::Any
}

fn optimize_contains(tree: &mut Tree) {
    let context = tree.context();
    let root = tree.root;
    if tree.nodes[root].op != Op::Alternate || tree.nodes[root].children.len() < 2 {
        return;
    }
    let alternatives = tree.nodes[root].children.clone();
    let mut literals = Buffer::new_in(context.as_ref());
    for &alternative in &alternatives {
        let node = &tree.nodes[alternative];
        if node.op != Op::Concat
            || node.children.len() != 3
            || !is_any(tree, node.children[0])
            || !is_any(tree, node.children[2])
            || tree.nodes[node.children[1]].op != Op::Literal
            || !sensitive(tree, node.children[1])
        {
            return;
        }
        literals.push(node.children[1]);
    }
    let mut alternate = tree.nodes[root].clone();
    alternate.flags = 0;
    alternate.children = literals;
    let middle = tree.nodes.len();
    tree.nodes.push(alternate);
    let mut wildcard = tree.nodes[tree.nodes[alternatives[0]].children[0]].clone();
    let mut any = tree.nodes[wildcard.children[0]].clone();
    any.flags = 0;
    let any_id = tree.nodes.len();
    tree.nodes.push(any);
    wildcard.flags = syntax::PERL | syntax::DOT_NL;
    wildcard.children = buffer![context.as_ref(); any_id];
    let first = tree.nodes.len();
    tree.nodes.push(wildcard.clone());
    let last = tree.nodes.len();
    tree.nodes.push(wildcard);
    tree.nodes[root].op = Op::Concat;
    tree.nodes[root].flags = 0;
    tree.nodes[root].children = buffer![context.as_ref(); first, middle, last];
}

#[derive(Debug)]
enum Matcher {
    True,
    Empty,
    NoNewline,
    NonEmpty(bool),
    OptionalRune(bool),
    Equal(Buffer<u8>, bool),
    Or(Buffer<usize>),
    Prefix {
        value: Buffer<u8>,
        sensitive: bool,
        right: usize,
    },
    Suffix {
        value: Buffer<u8>,
        sensitive: bool,
        left: usize,
    },
    Contains {
        values: Buffer<Buffer<u8>>,
        left: Option<usize>,
        right: Option<usize>,
    },
    Map {
        values: ByteSet,
        prefixes: ByteMap,
        min_prefix: usize,
        sensitive: bool,
    },
}

#[derive(Debug, Default)]
struct Plans {
    nodes: Buffer<Matcher>,
}

impl Plans {
    fn add(&mut self, matcher: Matcher) -> usize {
        let id = self.nodes.len();
        self.nodes.push(matcher);
        id
    }

    fn multi(&mut self, values: Buffer<Buffer<u8>>, sensitive: bool) -> usize {
        let context = self.nodes.context();
        if values.len() < MAP_THRESHOLD {
            let nodes = Buffer::from_iter_in(
                values
                    .into_iter()
                    .map(|value| self.add(Matcher::Equal(value, sensitive))),
                context.as_ref(),
            );
            self.add(Matcher::Or(nodes))
        } else {
            self.add(Matcher::Map {
                values: ByteSet::from_values(
                    values.into_iter().map(|value| {
                        if sensitive {
                            value
                        } else {
                            normalization::normalized_lower(&value, context.as_ref())
                        }
                    }),
                    context.as_ref(),
                ),
                prefixes: ByteMap::new(context.as_ref()),
                min_prefix: 0,
                sensitive,
            })
        }
    }

    fn optimize_or(&mut self, root: usize) -> usize {
        let context = self.nodes.context();
        let Matcher::Or(children) = &self.nodes[root] else {
            return root;
        };
        let mut pending = Buffer::from_iter_in(children.iter().rev().copied(), context.as_ref());
        let mut values = Buffer::new_in(context.as_ref());
        let mut prefixes = Buffer::new_in(context.as_ref());
        let mut sensitivity = None;
        let mut min_prefix = usize::MAX;
        while let Some(id) = pending.pop() {
            let (sensitive, value, prefix) = match &self.nodes[id] {
                Matcher::Or(children) => {
                    pending.extend(children.iter().rev().copied());
                    continue;
                }
                Matcher::Equal(value, sensitive) => (*sensitive, value, false),
                Matcher::Prefix {
                    value, sensitive, ..
                } => (*sensitive, value, true),
                _ => return root,
            };
            if sensitivity.is_some_and(|s| s != sensitive) {
                return root;
            }
            sensitivity = Some(sensitive);
            if prefix {
                min_prefix = min_prefix.min(value.len());
                prefixes.push((value.clone(), id));
            } else {
                values.push(value.clone());
            }
        }
        if values.len() + prefixes.len() < MAP_THRESHOLD {
            return root;
        }
        let sensitive = sensitivity.unwrap();
        let mut prefix_map = ByteMap::new(context.as_ref());
        for (value, id) in prefixes {
            let key = &value[..min_prefix];
            let key = if sensitive {
                Buffer::from_slice_in(key, context.as_ref())
            } else {
                normalization::lower(key, context.as_ref())
            };
            prefix_map.push(key, id, context.as_ref());
        }
        let min_prefix = if prefix_map.is_empty() { 0 } else { min_prefix };
        self.add(Matcher::Map {
            values: ByteSet::from_values(
                values.into_iter().map(|value| {
                    if sensitive {
                        value
                    } else {
                        normalization::normalized_lower(&value, context.as_ref())
                    }
                }),
                context.as_ref(),
            ),
            prefixes: prefix_map,
            min_prefix,
            sensitive,
        })
    }
}

enum PlanFrame {
    Visit(Id),
    Or(usize),
    Concat {
        id: Id,
        left: Option<Id>,
        right: Option<Id>,
    },
    Fallback {
        values: Buffer<Buffer<u8>>,
        sensitive: bool,
        left: Option<usize>,
        right: Option<usize>,
        prefix: bool,
    },
}

fn combine(
    plans: &mut Plans,
    set: Set,
    left: Option<usize>,
    right: Option<usize>,
) -> Option<usize> {
    let context = plans.nodes.context();
    if set.values.is_empty() {
        return None;
    }
    match (left, right) {
        (None, None) => {
            let children = Buffer::from_iter_in(
                set.values
                    .into_iter()
                    .map(|value| plans.add(Matcher::Equal(value, set.sensitive))),
                context.as_ref(),
            );
            Some(plans.add(Matcher::Or(children)))
        }
        (None, Some(right)) if set.values.len() == 1 => Some(plans.add(Matcher::Prefix {
            value: set.values.into_iter().next().unwrap(),
            sensitive: set.sensitive,
            right,
        })),
        (Some(left), None) if set.values.len() == 1 => Some(plans.add(Matcher::Suffix {
            value: set.values.into_iter().next().unwrap(),
            sensitive: set.sensitive,
            left,
        })),
        _ if set.sensitive => Some(plans.add(Matcher::Contains {
            values: set.values,
            left,
            right,
        })),
        _ => None,
    }
}

fn from_tree(tree: &mut Tree, plans: &mut Plans) -> Option<usize> {
    let context = tree.context();
    let mut frames = buffer![context.as_ref(); PlanFrame::Visit(tree.root)];
    let mut results = Buffer::<Option<usize>>::new_in(context.as_ref());
    while let Some(frame) = frames.pop() {
        match frame {
            PlanFrame::Visit(id) => {
                clear_capture(tree, id);
                let node = &tree.nodes[id];
                let plan = match node.op {
                    Op::Empty => Some(Matcher::Empty),
                    Op::Literal => Some(Matcher::Equal(
                        runes::text(&node.runes, context.as_ref()),
                        sensitive(tree, id),
                    )),
                    Op::Star | Op::Plus | Op::Quest
                        if matches!(tree.nodes[node.children[0]].op, Op::Any | Op::AnyNotNL) =>
                    {
                        let newline = tree.nodes[node.children[0]].op == Op::Any;
                        Some(match node.op {
                            Op::Star if newline => Matcher::True,
                            Op::Star => Matcher::NoNewline,
                            Op::Plus => Matcher::NonEmpty(newline),
                            _ => Matcher::OptionalRune(newline),
                        })
                    }
                    Op::Alternate => {
                        frames.push(PlanFrame::Or(node.children.len()));
                        frames.extend(node.children.iter().rev().copied().map(PlanFrame::Visit));
                        continue;
                    }
                    Op::Concat => {
                        let mut children = node.children.clone();
                        for &child in &children {
                            clear_capture(tree, child);
                        }
                        if children.is_empty() {
                            results.push(Some(plans.add(Matcher::Empty)));
                            continue;
                        }
                        if children.len() == 1 {
                            frames.push(PlanFrame::Visit(children[0]));
                            continue;
                        }
                        let wildcard =
                            |id: Id| matches!(tree.nodes[id].op, Op::Star | Op::Plus | Op::Quest);
                        let left = wildcard(children[0]).then(|| children.remove(0));
                        let right = children.last().copied().filter(|&id| wildcard(id));
                        if right.is_some() {
                            children.pop();
                        }
                        tree.nodes[id].children = children;
                        frames.push(PlanFrame::Concat { id, left, right });
                        if let Some(right) = right {
                            frames.push(PlanFrame::Visit(right));
                        }
                        if let Some(left) = left {
                            frames.push(PlanFrame::Visit(left));
                        }
                        continue;
                    }
                    _ => None,
                };
                results.push(plan.map(|plan| plans.add(plan)));
            }
            PlanFrame::Or(count) => {
                let children = results.split_off(results.len() - count);
                results.push(
                    children
                        .into_iter()
                        .try_fold(Buffer::new_in(context.as_ref()), |mut values, child| {
                            values.push(child?);
                            Some(values)
                        })
                        .map(|children| plans.add(Matcher::Or(children))),
                );
            }
            PlanFrame::Concat { id, left, right } => {
                let right_plan = right.map(|_| results.pop().unwrap());
                let left_plan = left.map(|_| results.pop().unwrap());
                if left_plan == Some(None) || right_plan == Some(None) {
                    results.push(None);
                    continue;
                }
                let left = left_plan.flatten();
                let right = right_plan.flatten();
                if let Some(set) = find_set(tree, id).filter(|set| !set.values.is_empty()) {
                    results.push(combine(plans, set, left, right));
                    continue;
                }
                let children = &tree.nodes[id].children;
                if children.len() == 2 {
                    let prefix = right.is_none() && tree.nodes[children[0]].op == Op::Literal;
                    let suffix =
                        !prefix && left.is_none() && tree.nodes[children[1]].op == Op::Literal;
                    if prefix || suffix {
                        let literal = children[usize::from(suffix)];
                        let other = children[usize::from(prefix)];
                        frames.push(PlanFrame::Fallback {
                            values: buffer![context.as_ref(); runes::text(&tree.nodes[literal].runes, context.as_ref())],
                            sensitive: sensitive(tree, literal),
                            left,
                            right,
                            prefix,
                        });
                        frames.push(PlanFrame::Visit(other));
                        continue;
                    }
                }
                results.push(None);
            }
            PlanFrame::Fallback {
                values,
                sensitive,
                left,
                right,
                prefix,
            } => {
                let other = results.pop().unwrap();
                results.push(other.and_then(|other| {
                    combine(
                        plans,
                        Set { values, sensitive },
                        if prefix { left } else { Some(other) },
                        if prefix { Some(other) } else { right },
                    )
                }));
            }
        }
    }
    results.pop().unwrap().map(|id| plans.optimize_or(id))
}

fn prefix_fold(subject: &[u8], prefix: &[u8]) -> Option<usize> {
    let (mut position, mut pattern) = (0, 0);
    while pattern < prefix.len() {
        if position == subject.len() {
            return None;
        }
        let (a, aw) = decode_rune(&subject[position..]);
        let (b, bw) = decode_rune(&prefix[pattern..]);
        if runes::min_fold(a as u32) != runes::min_fold(b as u32) {
            return None;
        }
        position += aw;
        pattern += bw;
    }
    Some(position)
}

fn equal_fold(a: &[u8], b: &[u8]) -> bool {
    prefix_fold(a, b) == Some(a.len())
}

fn suffix_fold(subject: &[u8], suffix: &[u8]) -> Option<usize> {
    fn last(input: &[u8]) -> (i32, usize) {
        if input.is_empty() {
            return (0xfffd, 0);
        }
        let begin = input.len().saturating_sub(4);
        for position in begin..input.len() {
            let (rune, width) = decode_rune(&input[position..]);
            if width == input.len() - position && (width > 1 || position + 1 == input.len()) {
                return (rune, width);
            }
        }
        unreachable!()
    }
    let (mut position, mut pattern) = (subject.len(), suffix.len());
    while pattern > 0 {
        if position == 0 {
            return None;
        }
        let (a, aw) = last(&subject[..position]);
        let (b, bw) = last(&suffix[..pattern]);
        if runes::min_fold(a as u32) != runes::min_fold(b as u32) {
            return None;
        }
        position -= aw;
        pattern -= bw;
    }
    Some(subject.len() - position)
}

fn index(subject: &[u8], value: &[u8]) -> Option<usize> {
    if value.is_empty() {
        Some(0)
    } else {
        subject
            .windows(value.len())
            .position(|window| window == value)
    }
}

enum Eval {
    Visit(usize, usize, usize),
    Or(usize, usize, usize, usize),
    Contains(usize, usize, usize, usize, usize),
}

enum Resume {
    Retry(Eval),
    And(usize, usize, usize),
}

impl Plans {
    fn matches(&self, root: usize, subject: &[u8], allocation: Option<&AllocationContext>) -> bool {
        let context = allocation.cloned();
        let mut action = Eval::Visit(root, 0, subject.len());
        let mut resumes = Buffer::new_in(context.as_ref());
        loop {
            let value = match action {
                Eval::Visit(id, begin, end) => {
                    let value = &subject[begin..end];
                    match &self.nodes[id] {
                        Matcher::True => true,
                        Matcher::Empty => value.is_empty(),
                        Matcher::NoNewline => !value.contains(&b'\n'),
                        Matcher::NonEmpty(newline) => {
                            !value.is_empty() && (*newline || !value.contains(&b'\n'))
                        }
                        Matcher::OptionalRune(newline) => {
                            let (rune, width) = decode_rune(value);
                            if rune == 0xfffd {
                                width == value.len()
                            } else {
                                width == value.len() && (*newline || value.first() != Some(&b'\n'))
                            }
                        }
                        Matcher::Equal(expected, sensitive) => {
                            if *sensitive {
                                expected == value
                            } else {
                                equal_fold(value, expected)
                            }
                        }
                        Matcher::Or(_) => {
                            action = Eval::Or(id, 0, begin, end);
                            continue;
                        }
                        Matcher::Prefix {
                            value: expected,
                            sensitive,
                            right,
                        } => {
                            let width = if *sensitive {
                                value.starts_with(expected).then_some(expected.len())
                            } else {
                                prefix_fold(value, expected)
                            };
                            if let Some(width) = width {
                                action = Eval::Visit(*right, begin + width, end);
                                continue;
                            }
                            false
                        }
                        Matcher::Suffix {
                            value: expected,
                            sensitive,
                            left,
                        } => {
                            let width = if *sensitive {
                                value.ends_with(expected).then_some(expected.len())
                            } else {
                                suffix_fold(value, expected)
                            };
                            if let Some(width) = width {
                                action = Eval::Visit(*left, begin, end - width);
                                continue;
                            }
                            false
                        }
                        Matcher::Contains { .. } => {
                            action = Eval::Contains(id, 0, 0, begin, end);
                            continue;
                        }
                        Matcher::Map {
                            values,
                            prefixes,
                            min_prefix,
                            sensitive,
                        } => {
                            let matched = !values.is_empty() && {
                                let normalized = (!sensitive).then(|| {
                                    normalization::normalized_lower(value, context.as_ref())
                                });
                                values.contains(normalized.as_deref().unwrap_or(value))
                            };
                            if matched {
                                true
                            } else if *min_prefix > 0 && value.len() >= *min_prefix {
                                let prefix = &value[..*min_prefix];
                                let normalized = (!sensitive).then(|| {
                                    normalization::normalized_lower(prefix, context.as_ref())
                                });
                                if let Some(children) =
                                    prefixes.get(normalized.as_deref().unwrap_or(prefix))
                                {
                                    // The map stores full prefix matcher IDs, including their right-hand conditions.
                                    for &child in children[1..].iter().rev() {
                                        resumes.push(Resume::Retry(Eval::Visit(child, begin, end)));
                                    }
                                    action = Eval::Visit(children[0], begin, end);
                                    continue;
                                }
                                false
                            } else {
                                false
                            }
                        }
                    }
                }
                Eval::Or(id, index, begin, end) => {
                    let Matcher::Or(children) = &self.nodes[id] else {
                        unreachable!()
                    };
                    if index == children.len() {
                        false
                    } else {
                        resumes.push(Resume::Retry(Eval::Or(id, index + 1, begin, end)));
                        action = Eval::Visit(children[index], begin, end);
                        continue;
                    }
                }
                Eval::Contains(id, mut which, mut start, begin, end) => {
                    let Matcher::Contains {
                        values,
                        left,
                        right,
                    } = &self.nodes[id]
                    else {
                        unreachable!()
                    };
                    let mut found = None;
                    while which < values.len() {
                        let value = &values[which];
                        let position = match (left, right) {
                            (Some(_), Some(_)) if start <= end - begin => {
                                index(&subject[begin + start..end], value)
                                    .map(|position| start + position)
                            }
                            (Some(_), None) => subject[begin..end]
                                .ends_with(value)
                                .then_some(end - begin - value.len().min(end - begin)),
                            (None, Some(_)) => subject[begin..end].starts_with(value).then_some(0),
                            _ => None,
                        };
                        if let Some(position) = position {
                            let retry = if left.is_some() && right.is_some() {
                                (which, position + 1)
                            } else {
                                (which + 1, 0)
                            };
                            resumes.push(Resume::Retry(Eval::Contains(
                                id, retry.0, retry.1, begin, end,
                            )));
                            if let Some(left) = left {
                                if let Some(right) = right {
                                    resumes.push(Resume::And(
                                        *right,
                                        begin + position + value.len(),
                                        end,
                                    ));
                                }
                                found = Some(Eval::Visit(*left, begin, begin + position));
                            } else if let Some(right) = right {
                                found = Some(Eval::Visit(*right, begin + value.len(), end));
                            }
                            break;
                        }
                        which += 1;
                        start = 0;
                    }
                    if let Some(found) = found {
                        action = found;
                        continue;
                    }
                    false
                }
            };
            let mut next = None;
            while let Some(resume) = resumes.pop() {
                match resume {
                    Resume::Retry(retry) if !value => {
                        next = Some(retry);
                        break;
                    }
                    Resume::And(id, begin, end) if value => {
                        next = Some(Eval::Visit(id, begin, end));
                        break;
                    }
                    _ => {}
                }
            }
            if let Some(next) = next {
                action = next;
            } else {
                return value;
            }
        }
    }
}

#[derive(Debug)]
pub struct Program {
    plans: Plans,
    matcher: Option<usize>,
    regex: Option<GoRegex>,
    prefix: Buffer<u8>,
    suffix: Buffer<u8>,
    contains: Buffer<Buffer<u8>>,
    insensitive_prefix: bool,
    equality: Option<Buffer<u8>>,
}

impl Program {
    pub fn compile(
        pattern: &[u8],
        allocation: Option<&AllocationContext>,
    ) -> Result<Self, Buffer<u8>> {
        let context = allocation.cloned();
        let mut program = Self {
            plans: Plans {
                nodes: Buffer::new_in(context.as_ref()),
            },
            matcher: None,
            regex: None,
            prefix: Buffer::new_in(context.as_ref()),
            suffix: Buffer::new_in(context.as_ref()),
            contains: Buffer::new_in(context.as_ref()),
            insensitive_prefix: false,
            equality: None,
        };
        let literals = Buffer::from_iter_in(
            pattern
                .split(|&byte| byte == b'|')
                .map(|value| Buffer::from_slice_in(value, context.as_ref())),
            context.as_ref(),
        );
        if literals
            .iter()
            .all(|value| !value.iter().any(|byte| b"\\.+*?()|[]{}^$".contains(byte)))
        {
            program.matcher = Some(program.plans.multi(literals, true));
            return Ok(program);
        }
        let mut tree = syntax::parse_in(pattern, syntax::PERL | syntax::DOT_NL, context.as_ref())
            .map_err(|error| error.bytes())?;
        optimize_contains(&mut tree);
        program.regex = Some(GoRegex::compile_label_replace_in(
            &string::canonical(&tree),
            context.as_ref(),
        )?);
        let root = tree.root;
        clear_capture(&mut tree, root);
        if tree.nodes[root].op == Op::Concat {
            let mut children = tree.nodes[root].children.clone();
            for &child in &children {
                clear_capture(&mut tree, child);
            }
            if children
                .first()
                .is_some_and(|&id| tree.nodes[id].op == Op::BeginText)
            {
                children.remove(0);
            }
            if children
                .last()
                .is_some_and(|&id| tree.nodes[id].op == Op::EndText)
            {
                children.pop();
            }
            if let Some(&first) = children
                .first()
                .filter(|&&id| tree.nodes[id].op == Op::Literal)
            {
                program.prefix = runes::text(&tree.nodes[first].runes, context.as_ref());
                program.insensitive_prefix = !sensitive(&tree, first);
            }
            if let Some(&last) = children
                .last()
                .filter(|&&id| tree.nodes[id].op == Op::Literal && sensitive(&tree, id))
            {
                program.suffix = runes::text(&tree.nodes[last].runes, context.as_ref());
            }
            for &id in children
                .iter()
                .skip(1)
                .take(children.len().saturating_sub(2))
            {
                if tree.nodes[id].op == Op::Literal && sensitive(&tree, id) {
                    program
                        .contains
                        .push(runes::text(&tree.nodes[id].runes, context.as_ref()));
                }
            }
        }
        clear_anchors(&mut tree, root);
        if let Some(set) = find_set(&mut tree, root).filter(|set| !set.values.is_empty()) {
            if set.sensitive && set.values.len() == 1 {
                program.equality = Some(set.values[0].clone());
            }
            if set.values.len() > 1 {
                program.matcher = Some(program.plans.multi(set.values, set.sensitive));
            }
        }
        if program.matcher.is_none() {
            let children = &tree.nodes[root].children;
            let simple = tree.nodes[root].op == Op::Concat
                && children.len() >= 3
                && children.len() % 2 == 1
                && children.iter().enumerate().all(|(i, &id)| {
                    if i % 2 == 0 {
                        is_any(&tree, id)
                    } else {
                        tree.nodes[id].op == Op::Literal && sensitive(&tree, id)
                    }
                });
            if simple {
                program.matcher = Some(program.plans.add(Matcher::True));
            } else {
                clear_anchors(&mut tree, root);
                program.matcher = from_tree(&mut tree, &mut program.plans);
            }
        }
        Ok(program)
    }

    pub fn matches(&self, value: &[u8], context: Option<&AllocationContext>) -> bool {
        if let Some(equality) = &self.equality {
            return value == equality.as_ref();
        }
        let regex = || {
            self.regex
                .as_ref()
                .unwrap()
                .captures_in(value, context)
                .is_some()
        };
        if self.prefix.is_empty()
            && self.suffix.is_empty()
            && self.contains.is_empty()
            && let Some(matcher) = self.matcher
        {
            return self.plans.matches(matcher, value, context);
        }
        if self.insensitive_prefix && !self.prefix.is_empty() {
            return prefix_fold(value, &self.prefix).is_some() && regex();
        }
        if !value.starts_with(&self.prefix) || !value.ends_with(&self.suffix) {
            return false;
        }
        let mut offset = 0;
        for contains in &self.contains {
            let Some(position) = index(&value[offset..], contains) else {
                return false;
            };
            offset += position + contains.len();
        }
        self.matcher
            .map_or_else(regex, |matcher| self.plans.matches(matcher, value, context))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::EngineAccounting;

    #[test]
    fn prefix_table_keys_retry_growth_and_normalization_use_separate_owners() {
        let engine = EngineAccounting::default();
        let compiled = engine.query();
        let context = AllocationContext::new(&compiled, MemoryKind::Compiled);
        let mut plans = Plans {
            nodes: Buffer::new_in(Some(&context)),
        };
        let mut children = Buffer::new_in(Some(&context));
        for index in 0..64 {
            let right = plans.add(Matcher::Equal(
                Buffer::from_slice_in(format!("tail{index}").as_bytes(), Some(&context)),
                false,
            ));
            children.push(plans.add(Matcher::Prefix {
                value: Buffer::from_slice_in(b"P", Some(&context)),
                sensitive: false,
                right,
            }));
        }
        let root = plans.add(Matcher::Or(children));
        let root = plans.optimize_or(root);
        let Matcher::Map { prefixes, .. } = &plans.nodes[root] else {
            panic!("expected prefix table")
        };
        assert_eq!(prefixes.values.len(), 1);
        assert_eq!(prefixes.get(b"p").unwrap().len(), 64);
        let retained = compiled.snapshot();
        assert!(retained.memory[MemoryKind::RegexEstimate as usize].current > 0);
        let search = engine.query();
        let scratch = AllocationContext::new(&search, MemoryKind::Workspace);
        assert!(plans.matches(root, b"pTAIL63", Some(&scratch)));
        assert!(!plans.matches(root, b"pother", Some(&scratch)));
        let observed = search.snapshot();
        assert!(
            observed.memory[MemoryKind::Workspace as usize].peak
                >= (63 * size_of::<Resume>()) as u64
        );
        observed.assert_memory_released();
        assert_eq!(compiled.snapshot(), retained);
        drop(plans);

        let mut plans = Plans {
            nodes: Buffer::new_in(Some(&context)),
        };
        let alternatives = Buffer::from_iter_in(
            (0..20).map(|index| {
                Buffer::from_slice_in(format!("\u{fb03}x{index}").as_bytes(), Some(&context))
            }),
            Some(&context),
        );
        let root = plans.multi(alternatives, false);
        let retained = compiled.snapshot();
        assert!(plans.matches(root, "\u{fb03}X17".as_bytes(), Some(&scratch)));
        assert!(!plans.matches(root, "\u{fb03}Xmissing".as_bytes(), Some(&scratch)));
        assert_eq!(compiled.snapshot(), retained);
        search.snapshot().assert_memory_released();
        drop(plans);
        engine.snapshot().assert_memory_released_with_queries(2);
    }
}
