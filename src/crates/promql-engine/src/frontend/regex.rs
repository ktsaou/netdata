// SPDX-License-Identifier: GPL-3.0-or-later
// Go-compatible frontend over an explicit regex-automata NFA; see NOTICE.
use crate::shared::{Allocation, Strong};
use std::ops::Range;

use regex_automata::{Input, nfa::thompson::pikevm::PikeVM};

use super::{ast::CompiledMatcher, quote::decode_rune};
use crate::accounting::{
    AccountedVec as Buffer, AllocationContext, MemoryKind, MemoryLease, QueryAccounting,
};

macro_rules! buffer {
    ($context:expr; $value:expr; $len:expr) => {
        Buffer::filled_in($len, $value, $context)
    };
    ($context:expr; $($value:expr),* $(,)?) => {
        Buffer::from_iter_in([$($value),*], $context)
    };
}

#[path = "regex_nfa.rs"]
mod nfa;
#[path = "regex_normalization.rs"]
mod normalization;
#[path = "regex_normalization_data.rs"]
mod normalization_data;
#[path = "regex_program.rs"]
mod program;
#[path = "regex_runes.rs"]
mod runes;
#[path = "regex_selector.rs"]
mod selector;
#[path = "regex_string.rs"]
mod string;
#[path = "regex_syntax.rs"]
mod syntax;
#[path = "regex_unicode.rs"]
mod unicode;

use syntax::Tree;

struct Subject<'a> {
    original: &'a [u8],
    bytes: Option<Buffer<u8>>,
    original_offsets: Buffer<usize>,
}

impl<'a> Subject<'a> {
    fn new(original: &'a [u8], context: Option<&AllocationContext>) -> Self {
        if std::str::from_utf8(original).is_ok() {
            return Self {
                original,
                bytes: None,
                original_offsets: Buffer::new_in(None),
            };
        }
        let mut bytes = Buffer::new_in(context);
        let mut offsets = buffer![context; 0];
        let mut position = 0;
        while position < original.len() {
            let (rune, width) = decode_rune(&original[position..]);
            let mut buffer = [0; 4];
            let encoded = char::from_u32(rune as u32)
                .unwrap()
                .encode_utf8(&mut buffer)
                .as_bytes();
            bytes.extend_from_slice(encoded);
            for _ in 1..encoded.len() {
                offsets.push(position);
            }
            position += width;
            offsets.push(position);
        }
        Self {
            original,
            bytes: Some(bytes),
            original_offsets: offsets,
        }
    }

    fn bytes(&self) -> &[u8] {
        self.bytes.as_deref().unwrap_or(self.original)
    }

    fn offset(&self, normalized: usize) -> usize {
        if self.original_offsets.is_empty() {
            normalized
        } else {
            self.original_offsets[normalized]
        }
    }

    fn normalized_offset(&self, original: usize) -> usize {
        if self.original_offsets.is_empty() {
            original
        } else {
            self.original_offsets
                .partition_point(|&offset| offset < original)
        }
    }
}

#[derive(Debug)]
pub struct GoRegex {
    program: PikeVM,
    prefix: Buffer<u8>,
    names: Buffer<Buffer<u8>>,
    _estimate: Option<MemoryLease>,
}

impl GoRegex {
    pub fn compile(pattern: &[u8]) -> Result<Self, Vec<u8>> {
        Self::compile_in(pattern, None).map_err(Buffer::into_unobserved)
    }

    pub(crate) fn compile_in(
        pattern: &[u8],
        context: Option<&AllocationContext>,
    ) -> Result<Self, Buffer<u8>> {
        let tree = syntax::parse_standard_in(pattern, syntax::PERL, context)
            .map_err(|error| error.bytes())?;
        Self::from_tree(tree)
    }

    pub fn compile_label_replace(pattern: &[u8]) -> Result<Self, Vec<u8>> {
        Self::compile_label_replace_in(pattern, None).map_err(Buffer::into_unobserved)
    }

    pub(crate) fn compile_label_replace_in(
        pattern: &[u8],
        context: Option<&AllocationContext>,
    ) -> Result<Self, Buffer<u8>> {
        let mut anchored = Buffer::from_slice_in(b"^(?s:", context);
        anchored.extend_from_slice(pattern);
        anchored.extend_from_slice(b")$");
        Self::compile_in(&anchored, context)
    }

    fn from_tree(mut tree: Tree) -> Result<Self, Buffer<u8>> {
        let flat = program::Program::compile(&mut tree);
        let (program, prefix, estimate) = nfa::build(&flat)?;
        Ok(Self {
            program,
            prefix,
            names: tree.names,
            _estimate: estimate,
        })
    }

    pub fn names(&self) -> &[Buffer<u8>] {
        &self.names
    }

    pub fn subexp_index(&self, name: &[u8]) -> Option<usize> {
        if name.is_empty() {
            return None;
        }
        self.names.iter().position(|candidate| candidate == name)
    }

    pub fn captures(&self, original: &[u8]) -> Option<Vec<Option<Range<usize>>>> {
        self.captures_in(original, None)
            .map(Buffer::into_unobserved)
    }

    pub(crate) fn captures_in(
        &self,
        original: &[u8],
        context: Option<&AllocationContext>,
    ) -> Option<Buffer<Option<Range<usize>>>> {
        let subject = Subject::new(original, context);
        self.captures_subject(&subject, 0, context)
    }

    fn captures_subject(
        &self,
        subject: &Subject<'_>,
        start: usize,
        context: Option<&AllocationContext>,
    ) -> Option<Buffer<Option<Range<usize>>>> {
        if !subject.original.starts_with(&self.prefix) {
            return None;
        }
        let mut cache = self.program.create_cache();
        let mut captures = self.program.create_captures();
        // Capture group metadata is shared with the retained NFA; only slots are new.
        let mut estimate = context.map(|context| {
            context
                .with_kind(MemoryKind::RegexEstimate)
                .hold((cache.memory_usage() + size_of_val(captures.slots())) as u64)
        });
        let input = Input::new(subject.bytes())
            .span(subject.normalized_offset(start)..subject.bytes().len());
        self.program.search(&mut cache, &input, &mut captures);
        if let Some(estimate) = &mut estimate {
            estimate.resize((cache.memory_usage() + size_of_val(captures.slots())) as u64);
        }
        if !captures.is_match() {
            return None;
        }
        Some(Buffer::from_iter_in(
            (0..self.names.len()).map(|index| {
                captures
                    .get_group(index)
                    .map(|span| subject.offset(span.start)..subject.offset(span.end))
            }),
            context,
        ))
    }

    /// Supplemental Go FindAll qualification; production label replacement uses one anchored match.
    #[cfg(feature = "fixtures")]
    pub fn captures_all(&self, original: &[u8]) -> Vec<Vec<Option<Range<usize>>>> {
        let subject = Subject::new(original, None);
        let mut result = Vec::new();
        let mut position = 0;
        let mut previous_end = None;
        while position <= original.len() {
            let Some(captures) = self.captures_subject(&subject, position, None) else {
                break;
            };
            let whole = captures[0].as_ref().unwrap();
            let accept = if whole.end == position {
                let accept = previous_end != Some(whole.start);
                let (_, width) = decode_rune(&original[position..]);
                position = if width == 0 {
                    original.len() + 1
                } else {
                    position + width
                };
                accept
            } else {
                position = whole.end;
                true
            };
            previous_end = Some(whole.end);
            if accept {
                result.push(captures.into_unobserved());
            }
        }
        result
    }

    pub fn expand(
        &self,
        template: &[u8],
        original: &[u8],
        captures: &[Option<Range<usize>>],
    ) -> Vec<u8> {
        self.expand_in(template, original, captures, None)
            .into_unobserved()
    }

    pub(crate) fn expand_in(
        &self,
        template: &[u8],
        original: &[u8],
        captures: &[Option<Range<usize>>],
        context: Option<&AllocationContext>,
    ) -> Buffer<u8> {
        let mut output = Buffer::new_in(context);
        let mut position = 0;
        while let Some(relative) = template[position..].iter().position(|&byte| byte == b'$') {
            let dollar = position + relative;
            output.extend_from_slice(&template[position..dollar]);
            position = dollar + 1;
            if template.get(position) == Some(&b'$') {
                output.push(b'$');
                position += 1;
                continue;
            }
            let brace = template.get(position) == Some(&b'{');
            let begin = position + usize::from(brace);
            let mut end = begin;
            while end < template.len() {
                let (rune, width) = decode_rune(&template[end..]);
                if rune != i32::from(b'_') && !runes::is_letter_or_digit(rune as u32) {
                    break;
                }
                end += width;
            }
            if end == begin || brace && template.get(end) != Some(&b'}') {
                output.push(b'$');
                continue;
            }
            let name = &template[begin..end];
            position = end + usize::from(brace);
            let mut number = Some(0usize);
            for &byte in name {
                number = number
                    .filter(|&n| byte.is_ascii_digit() && n < 100_000_000)
                    .map(|n| n * 10 + usize::from(byte - b'0'));
            }
            if name.len() > 1 && name[0] == b'0' {
                number = None;
            }
            let range = if let Some(index) = number {
                captures.get(index).and_then(|range| range.as_ref())
            } else {
                self.names
                    .iter()
                    .zip(captures)
                    .find_map(|(candidate, range)| {
                        if candidate == name {
                            range.as_ref()
                        } else {
                            None
                        }
                    })
            };
            if let Some(range) = range {
                output.extend_from_slice(&original[range.clone()]);
            }
        }
        output.extend_from_slice(&template[position..]);
        output
    }
}

#[cfg(feature = "fixtures")]
pub use syntax::Error as SyntaxError;

#[cfg(feature = "fixtures")]
pub use program::Inspection as ProgramInspection;

#[cfg(feature = "fixtures")]
pub fn inspect_program(pattern: &[u8]) -> Result<ProgramInspection, Vec<u8>> {
    let mut tree = syntax::parse_standard(pattern, syntax::PERL)
        .map_err(|error| error.bytes().into_unobserved())?;
    Ok(program::Program::compile(&mut tree).inspect())
}

#[cfg(feature = "fixtures")]
pub fn inspect_normalization(input: &[u8]) -> (Vec<u8>, Vec<u8>) {
    (
        normalization::normalized_lower(input, None).into_unobserved(),
        normalization::lower(input, None).into_unobserved(),
    )
}

#[cfg(feature = "fixtures")]
pub struct SyntaxInspection {
    pub names: Vec<Vec<u8>>,
    pub canonical: Vec<u8>,
}

#[cfg(feature = "fixtures")]
pub fn inspect_syntax(
    pattern: &[u8],
    flags: u16,
    standard: bool,
) -> Result<SyntaxInspection, SyntaxError> {
    if standard {
        syntax::parse_standard(pattern, flags)
    } else {
        syntax::parse(pattern, flags)
    }
    .map(|tree| SyntaxInspection {
        canonical: string::canonical(&tree).into_unobserved(),
        names: tree
            .names
            .into_iter()
            .map(Buffer::into_unobserved)
            .collect(),
    })
}

#[derive(Debug)]
pub struct Selector {
    program: selector::Program,
}

impl Selector {
    pub fn compile(pattern: &[u8]) -> Result<Strong<dyn CompiledMatcher>, Vec<u8>> {
        Self::compile_in(pattern, None).map_err(Buffer::into_unobserved)
    }
    pub(crate) fn compile_in(
        pattern: &[u8],
        context: Option<&AllocationContext>,
    ) -> Result<Strong<dyn CompiledMatcher>, Buffer<u8>> {
        let program = selector::Program::compile(pattern, context)?;
        let charge = Allocation::<Self>::charge(context);
        let allocation: Box<Allocation<dyn CompiledMatcher>> =
            Box::new(Allocation::new(Self { program }));
        Ok(Strong::finish(allocation, charge))
    }
}

impl CompiledMatcher for Selector {
    fn matches_in(&self, value: &[u8], accounting: Option<&QueryAccounting>) -> bool {
        let context =
            accounting.map(|accounting| AllocationContext::new(accounting, MemoryKind::Workspace));
        self.program.matches(value, context.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::{GoRegex, Selector};
    use crate::accounting::{AllocationContext, EngineAccounting, MemoryKind};

    #[test]
    fn compilation_scratch_errors_and_shared_matchers_keep_their_owners() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Compiled);
        for pattern in [b"(".as_slice(), b"\xff+", b"(?P<x>a)["] {
            let error = Selector::compile_in(pattern, Some(&context)).unwrap_err();
            assert_eq!(
                account.snapshot().memory[0].current,
                error.capacity() as u64
            );
            assert!(account.snapshot().memory[0].peak > error.capacity() as u64);
            assert_eq!(account.snapshot().memory[5].current, 0);
            drop(error);
            assert_eq!(account.snapshot().memory[0].current, 0);
        }
        let pattern = (0..20)
            .map(|index| format!("value{index}"))
            .collect::<Vec<_>>()
            .join("|");
        let matcher = Selector::compile_in(pattern.as_bytes(), Some(&context)).unwrap();
        let before = account.snapshot();
        assert!(before.memory[0].current > 0);
        assert!(before.memory[5].current > 0);
        let shared = matcher.clone();
        assert_eq!(account.snapshot(), before);
        drop(context);
        drop(account);
        drop(matcher);
        assert!(shared.matches(b"value17"));
        assert!(!shared.matches(b"other"));
        assert_eq!(
            engine.snapshot().memory[0].current,
            before.memory[0].current
        );
        assert_eq!(
            engine.snapshot().memory[5].current,
            before.memory[5].current
        );
        drop(shared);
        assert!(
            engine
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }

    #[test]
    fn search_buffers_and_returned_captures_use_the_execution_owner() {
        let engine = EngineAccounting::default();
        let compiled = engine.query();
        let compile_context = AllocationContext::new(&compiled, MemoryKind::Compiled);
        let regex =
            GoRegex::compile_label_replace_in(b"(?P<x>a[^0-9]+c)", Some(&compile_context)).unwrap();
        let retained = compiled.snapshot();
        let account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Workspace);
        let original = b"a\xffbc";
        let captures = regex.captures_in(original, Some(&context)).unwrap();
        assert_eq!(&*captures, &[Some(0..4), Some(0..4)]);
        assert_eq!(
            account.snapshot().memory[1].current,
            (captures.capacity() * size_of::<Option<std::ops::Range<usize>>>()) as u64
        );
        assert!(account.snapshot().memory[5].peak > 0);
        assert_eq!(account.snapshot().memory[5].current, 0);
        let expanded = regex.expand_in(b"$x$1", original, &captures, Some(&context));
        assert_eq!(&*expanded, b"a\xffbca\xffbc");
        let current = account.snapshot().memory[1].current;
        assert!(regex.captures_in(b"a\xff9c", Some(&context)).is_none());
        assert_eq!(account.snapshot().memory[1].current, current);
        assert_eq!(account.snapshot().memory[5].current, 0);
        assert_eq!(compiled.snapshot(), retained);
        drop(regex);
        drop(compile_context);
        drop(compiled);
        assert_eq!(engine.snapshot().memory[0].current, 0);
        assert_eq!(engine.snapshot().memory[5].current, 0);
        drop(context);
        drop(account);
        assert_eq!(&*expanded, b"a\xffbca\xffbc");
        assert_eq!(engine.snapshot().memory[1].current, current);
        drop(expanded);
        drop(captures);
        assert_eq!(engine.snapshot().memory[1].current, 0);
    }

    #[test]
    fn normalization_expansion_and_failed_prefix_scratch_are_observed() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Workspace);
        let output =
            super::normalization::normalized_lower("\u{fb03}A\u{301}".as_bytes(), Some(&context));
        assert_eq!(&*output, "ffia\u{301}".as_bytes());
        assert_eq!(
            account.snapshot().memory[1].current,
            output.capacity() as u64
        );
        assert!(account.snapshot().memory[1].peak > output.capacity() as u64);
        drop(output);
        let regex = GoRegex::compile_label_replace(b"abc").unwrap();
        assert!(regex.captures_in(b"\xffbc", Some(&context)).is_none());
        assert_eq!(account.snapshot().memory[1].current, 0);
        assert!(account.snapshot().memory[1].acquired > 0);
        assert_eq!(account.snapshot().memory[5].acquired, 0);
    }

    #[test]
    fn concurrent_searches_share_compilation_and_keep_separate_scratch_accounts() {
        let engine = EngineAccounting::default();
        let compiled = engine.query();
        let context = AllocationContext::new(&compiled, MemoryKind::Compiled);
        let matcher = Selector::compile_in(b"a[^0-9]+c", Some(&context)).unwrap();
        let retained = compiled.snapshot();
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let engine = &engine;
                let matcher = &matcher;
                let barrier = &barrier;
                scope.spawn(move || {
                    let account = engine.query();
                    let execution = account.execution();
                    barrier.wait();
                    let matched = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        (0..32).all(|_| {
                            matcher.matches_in(b"a\xffbc", Some(&account))
                                && !matcher.matches_in(b"a\xff9c", Some(&account))
                        })
                    }));
                    let observed = account.snapshot();
                    barrier.wait();
                    drop(execution);
                    assert!(matched.unwrap());
                    assert!(observed.memory[1].peak > 0);
                    assert!(observed.memory[5].peak > 0);
                    assert!(observed.memory.iter().all(|memory| memory.current == 0));
                });
            }
            barrier.wait();
            assert_eq!(engine.snapshot().active_executions, 2);
            barrier.wait();
        });
        assert_eq!(compiled.snapshot(), retained);
        assert_eq!(engine.snapshot().active_executions, 0);
        drop(context);
        drop(compiled);
        drop(matcher);
        assert!(
            engine
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }

    #[test]
    fn regex_compilation_search_and_retained_results_release_on_unwind() {
        let engine = EngineAccounting::default();
        let compiled = engine.query();
        let searched = engine.query();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let context = AllocationContext::new(&compiled, MemoryKind::Compiled);
            let search = AllocationContext::new(&searched, MemoryKind::Workspace);
            let regex =
                GoRegex::compile_label_replace_in(b"(?P<x>a[^0-9]+c)", Some(&context)).unwrap();
            let captures = regex.captures_in(b"a\xffbc", Some(&search)).unwrap();
            let _expanded = regex.expand_in(b"$x$1", b"a\xffbc", &captures, Some(&search));
            panic!("injected ordinary regex owner unwind")
        }));
        assert!(panic.is_err());
        assert!(
            compiled
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
        assert!(
            searched
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
        assert!(
            engine
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }

    #[test]
    fn compile_execute_and_drop_with_a_small_caller_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut captures = vec![b'('; 999];
                captures.push(b'a');
                captures.extend(std::iter::repeat_n(b')', 999));
                let regex = GoRegex::compile(&captures).unwrap();
                let result = regex.captures(b"a").unwrap();
                assert_eq!(result.len(), 1000);
                assert!(result.iter().all(|range| *range == Some(0..1)));
                drop(regex);

                let mut groups = b"(?:".repeat(1500);
                groups.push(b'a');
                groups.extend(std::iter::repeat_n(b')', 1500));
                assert!(Selector::compile(&groups).unwrap().matches(b"a"));
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
