// SPDX-License-Identifier: GPL-3.0-or-later
// Go-compatible frontend over an explicit regex-automata NFA; see NOTICE.
use crate::shared::{Allocation, Strong};
use std::ops::Range;

use regex_automata::{Input, nfa::thompson::pikevm::PikeVM};

use super::{ast::CompiledMatcher, quote::decode_rune, work::Work};
use crate::accounting::{
    AccountedVec as Buffer, AllocationContext, MemoryKind, MemoryLease, QueryAccounting,
};
use crate::storage::{Control, Interrupted};

#[derive(Debug)]
enum Failure<E> {
    Error(E),
    Interrupted(Interrupted),
}
impl<E> From<Interrupted> for Failure<E> {
    fn from(error: Interrupted) -> Self {
        Self::Interrupted(error)
    }
}
fn separate<T, E>(result: Result<T, Failure<E>>) -> Result<Result<T, E>, Interrupted> {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(Failure::Error(error)) => Ok(Err(error)),
        Err(Failure::Interrupted(error)) => Err(error),
    }
}
fn inert<T, E>(result: Result<T, Failure<E>>) -> Result<T, E> {
    separate(result).expect("inert regex work")
}
fn syntax_result<T>(
    result: Result<T, Failure<syntax::Error>>,
    work: Work<'_>,
) -> Result<T, Failure<Buffer<u8>>> {
    match result {
        Ok(value) => Ok(value),
        Err(Failure::Error(error)) => Err(Failure::Error(error.bytes_checked(work)?)),
        Err(Failure::Interrupted(error)) => Err(Failure::Interrupted(error)),
    }
}

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
    fn new(original: &'a [u8], work: Work<'_>) -> Result<Self, Interrupted> {
        work.step(0)?;
        let context = work.context;
        if work.valid_utf8(original)? {
            return Ok(Self {
                original,
                bytes: None,
                original_offsets: Buffer::new_in(None),
            });
        }
        let mut bytes = Buffer::new_in(context);
        let mut offsets = buffer![context; 0];
        let mut position = 0;
        while position < original.len() {
            work.step(1)?;
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
        Ok(Self {
            original,
            bytes: Some(bytes),
            original_offsets: offsets,
        })
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

    fn normalized_offset(&self, original: usize, work: Work<'_>) -> Result<usize, Interrupted> {
        work.step(0)?;
        if self.original_offsets.is_empty() {
            return Ok(original);
        }
        let (mut begin, mut end) = (0, self.original_offsets.len());
        while begin < end {
            work.step(1)?;
            let middle = begin + (end - begin) / 2;
            if self.original_offsets[middle] < original {
                begin = middle + 1;
            } else {
                end = middle;
            }
        }
        Ok(begin)
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
        inert(Self::compile_work(pattern, Work::new(context, None)))
    }
    fn compile_work(pattern: &[u8], work: Work<'_>) -> Result<Self, Failure<Buffer<u8>>> {
        work.step(0)?;
        let tree = syntax_result(syntax::parse_standard_in(pattern, syntax::PERL, work), work)?;
        Self::from_tree(tree, work)
    }
    pub fn compile_label_replace(pattern: &[u8]) -> Result<Self, Vec<u8>> {
        Self::compile_label_replace_in(pattern, None).map_err(Buffer::into_unobserved)
    }
    pub(crate) fn compile_label_replace_in(
        pattern: &[u8],
        context: Option<&AllocationContext>,
    ) -> Result<Self, Buffer<u8>> {
        inert(Self::compile_label_replace_work(
            pattern,
            Work::new(context, None),
        ))
    }
    pub(crate) fn compile_label_replace_checked(
        pattern: &[u8],
        context: Option<&AllocationContext>,
        control: &Control,
    ) -> Result<Result<Self, Buffer<u8>>, Interrupted> {
        separate(Self::compile_label_replace_work(
            pattern,
            Work::new(context, Some(control)),
        ))
    }
    fn compile_label_replace_work(
        pattern: &[u8],
        work: Work<'_>,
    ) -> Result<Self, Failure<Buffer<u8>>> {
        work.step(0)?;
        let mut anchored = work.copy(b"^(?s:")?;
        work.extend(&mut anchored, pattern)?;
        work.extend(&mut anchored, b")$")?;
        Self::compile_work(&anchored, work)
    }
    fn from_tree(mut tree: Tree, work: Work<'_>) -> Result<Self, Failure<Buffer<u8>>> {
        let flat = program::Program::compile(&mut tree, work)?;
        let (program, prefix, estimate) = nfa::build(&flat, work)?;
        work.step(0)?;
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
        self.captures_work(original, Work::new(context, None))
            .expect("inert regex capture")
    }
    pub(crate) fn captures_checked(
        &self,
        original: &[u8],
        context: Option<&AllocationContext>,
        control: &Control,
    ) -> Result<Option<Buffer<Option<Range<usize>>>>, Interrupted> {
        self.captures_work(original, Work::new(context, Some(control)))
    }
    fn captures_work(
        &self,
        original: &[u8],
        work: Work<'_>,
    ) -> Result<Option<Buffer<Option<Range<usize>>>>, Interrupted> {
        let subject = Subject::new(original, work)?;
        self.captures_subject(&subject, 0, work)
    }
    fn captures_subject(
        &self,
        subject: &Subject<'_>,
        start: usize,
        work: Work<'_>,
    ) -> Result<Option<Buffer<Option<Range<usize>>>>, Interrupted> {
        work.step(0)?;
        if !work.starts(subject.original, &self.prefix)? {
            return Ok(None);
        }
        // Backend cache creation and search are atomic from this interface.
        let mut cache = self.program.create_cache();
        let mut captures = self.program.create_captures();
        let mut estimate = work.context.map(|context| {
            context
                .with_kind(MemoryKind::RegexEstimate)
                .hold((cache.memory_usage() + size_of_val(captures.slots())) as u64)
        });
        work.step(0)?;
        let input = Input::new(subject.bytes())
            .span(subject.normalized_offset(start, work)?..subject.bytes().len());
        work.step(0)?;
        self.program.search(&mut cache, &input, &mut captures);
        if let Some(estimate) = &mut estimate {
            estimate.resize((cache.memory_usage() + size_of_val(captures.slots())) as u64);
        }
        work.step(0)?;
        if !captures.is_match() {
            return Ok(None);
        }
        Ok(Some(work.collect((0..self.names.len()).map(|index| {
            captures
                .get_group(index)
                .map(|span| subject.offset(span.start)..subject.offset(span.end))
        }))?))
    }

    /// Supplemental Go FindAll qualification; production label replacement uses one anchored match.
    #[cfg(feature = "fixtures")]
    pub fn captures_all(&self, original: &[u8]) -> Vec<Vec<Option<Range<usize>>>> {
        let subject = Subject::new(original, Work::default()).expect("inert subject");
        let mut result = Vec::new();
        let mut position = 0;
        let mut previous_end = None;
        while position <= original.len() {
            let Some(captures) = self
                .captures_subject(&subject, position, Work::default())
                .expect("inert regex capture")
            else {
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
        self.expand_work(template, original, captures, Work::new(context, None))
            .expect("inert regex expansion")
    }
    pub(crate) fn expand_checked(
        &self,
        template: &[u8],
        original: &[u8],
        captures: &[Option<Range<usize>>],
        context: Option<&AllocationContext>,
        control: &Control,
    ) -> Result<Buffer<u8>, Interrupted> {
        self.expand_work(
            template,
            original,
            captures,
            Work::new(context, Some(control)),
        )
    }
    fn expand_work(
        &self,
        template: &[u8],
        original: &[u8],
        captures: &[Option<Range<usize>>],
        work: Work<'_>,
    ) -> Result<Buffer<u8>, Interrupted> {
        work.step(0)?;
        let mut output = work.buffer();
        let mut position = 0;
        while let Some(relative) = work.find(&template[position..], b"$")? {
            work.step(1)?;
            let dollar = position + relative;
            work.extend(&mut output, &template[position..dollar])?;
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
                work.step(1)?;
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
                work.step(1)?;
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
                let mut found = None;
                for (candidate, range) in self.names.iter().zip(captures) {
                    work.step(1)?;
                    if work.equal(candidate, name)?
                        && let Some(range) = range.as_ref()
                    {
                        found = Some(range);
                        break;
                    }
                }
                found
            };
            if let Some(range) = range {
                work.extend(&mut output, &original[range.clone()])?;
            }
        }
        work.extend(&mut output, &template[position..])?;
        Ok(output)
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
    Ok(program::Program::compile(&mut tree, Work::default())
        .expect("inert program inspection")
        .inspect())
}

#[cfg(feature = "fixtures")]
pub fn inspect_normalization(input: &[u8]) -> (Vec<u8>, Vec<u8>) {
    (
        normalization::normalized_lower(input, Work::default())
            .expect("inert normalization")
            .into_unobserved(),
        normalization::lower(input, Work::default())
            .expect("inert normalization")
            .into_unobserved(),
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
        canonical: string::canonical(&tree, Work::default())
            .expect("inert canonicalization")
            .into_unobserved(),
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
        Self::compile_checked(pattern, context, None).expect("inert selector compile")
    }
    pub(crate) fn compile_checked(
        pattern: &[u8],
        context: Option<&AllocationContext>,
        control: Option<&Control>,
    ) -> Result<Result<Strong<dyn CompiledMatcher>, Buffer<u8>>, Interrupted> {
        let work = Work::new(context, control);
        separate((|| {
            let program = selector::Program::compile(pattern, work)?;
            work.step(0)?;
            let charge = Allocation::<Self>::charge(context);
            let allocation: Box<Allocation<dyn CompiledMatcher>> =
                Box::new(Allocation::new(Self { program }));
            let result = Strong::finish(allocation, charge);
            work.step(0)?;
            Ok(result)
        })())
    }
}

impl CompiledMatcher for Selector {
    fn matches_in(&self, value: &[u8], accounting: Option<&QueryAccounting>) -> bool {
        let context =
            accounting.map(|account| AllocationContext::new(account, MemoryKind::Workspace));
        self.program
            .matches(value, Work::new(context.as_ref(), None))
            .expect("inert selector match")
    }
    fn matches_checked(
        &self,
        value: &[u8],
        accounting: Option<&QueryAccounting>,
        control: &Control,
    ) -> Result<bool, Interrupted> {
        let context =
            accounting.map(|account| AllocationContext::new(account, MemoryKind::Workspace));
        self.program.matches(
            value,
            Work {
                context: context.as_ref(),
                account: accounting,
                control: Some(control),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{GoRegex, Selector};
    use crate::accounting::{AllocationContext, EngineAccounting, MemoryKind};

    #[test]
    fn owned_regex_compile_stages_release_partial_buffers_on_cancellation() {
        use super::{Buffer, Failure, Work, nfa, program, runes, string, syntax};
        use crate::{
            storage::{Control, Interrupted},
            test_storage::interrupt_during,
        };
        let input = vec![b'a'; 400_000];
        for stage in ["literal", "syntax", "canonical", "program", "nfa", "class"] {
            let tree =
                syntax::parse_in(&input, syntax::PERL | syntax::LITERAL, Work::default()).unwrap();
            let flat = if stage == "nfa" {
                let mut tree = syntax::parse_in(
                    &input[..40_000],
                    syntax::PERL | syntax::LITERAL,
                    Work::default(),
                )
                .unwrap();
                Some(program::Program::compile(&mut tree, Work::default()).unwrap())
            } else {
                None
            };
            let engine = EngineAccounting::default();
            let account = engine.query();
            let context = AllocationContext::new(&account, MemoryKind::Compiled);
            let control = Control::default();
            let work = Work::new(Some(&context), Some(&control));
            let result = interrupt_during(&account, &control, || -> Result<(), Interrupted> {
                match stage {
                    "literal" => {
                        super::Selector::compile_checked(
                            &b"a|".repeat(200_000),
                            Some(&context),
                            Some(&control),
                        )?
                        .unwrap();
                    }
                    "syntax" => match syntax::parse_in(&input, syntax::PERL, work) {
                        Err(Failure::Interrupted(error)) => return Err(error),
                        other => {
                            other.unwrap();
                        }
                    },
                    "canonical" => {
                        string::canonical(&tree, work)?;
                    }
                    "program" => {
                        let mut tree = tree;
                        program::Program::compile(&mut tree, work)?;
                    }
                    "nfa" => match nfa::build(flat.as_ref().unwrap(), work) {
                        Err(Failure::Interrupted(error)) => return Err(error),
                        other => {
                            other.unwrap();
                        }
                    },
                    "class" => {
                        let ranges = Buffer::from_iter_in(
                            (0..40_000).rev().map(|rune| (rune, rune)),
                            Some(&context),
                        );
                        runes::clean(ranges, work)?;
                    }
                    _ => unreachable!(),
                }
                Ok(())
            });
            assert_eq!(result, Err(Interrupted::Cancelled), "{stage}");
            assert!(
                account.snapshot().memory[MemoryKind::Compiled as usize].peak > 0,
                "{stage}"
            );
            account.snapshot().assert_memory_released();
            drop(context);
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn owned_subject_capture_and_replacement_walks_return_cancellation() {
        use super::Work;
        use crate::{
            storage::{Control, Interrupted},
            test_storage::interrupt_during,
        };
        let original = vec![b'a'; 400_000];
        let regex = GoRegex::compile(b"(.*)").unwrap();
        let captures = regex.captures(&original).unwrap();
        let many = GoRegex::compile(&b"()".repeat(9_000)).unwrap();
        for stage in [
            "subject-valid",
            "subject-invalid",
            "capture-export",
            "replacement-copy",
            "replacement-name",
            "replacement-scan",
            "normalize",
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let context = AllocationContext::new(&account, MemoryKind::Workspace);
            let control = Control::default();
            let work = Work::new(Some(&context), Some(&control));
            let result = interrupt_during(&account, &control, || -> Result<(), Interrupted> {
                match stage {
                    "subject-valid" => {
                        regex.captures_work(&original, work)?;
                    }
                    "subject-invalid" => {
                        regex.captures_work(&vec![0xff; 400_000], work)?;
                    }
                    "capture-export" => {
                        many.captures_work(b"", work)?;
                    }
                    "replacement-copy" => {
                        regex.expand_work(b"$1", &original, &captures, work)?;
                    }
                    "replacement-name" => {
                        let template = [b"$".as_slice(), &original].concat();
                        regex.expand_work(&template, &original, &captures, work)?;
                    }
                    "replacement-scan" => {
                        regex.expand_work(&original, &original, &captures, work)?;
                    }
                    "normalize" => {
                        super::normalization::normalized_lower(
                            &"\u{fb03}A\u{301}".repeat(80_000).into_bytes(),
                            work,
                        )?;
                    }
                    _ => unreachable!(),
                }
                Ok(())
            });
            assert_eq!(result, Err(Interrupted::Cancelled), "{stage}");
            account.snapshot().assert_memory_released();
            drop(context);
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn checked_regex_shortcuts_preserve_deadline_errors() {
        use super::Work;
        use crate::storage::{Control, Interrupted};
        let engine = EngineAccounting::default();
        let account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Workspace);
        let control = Control {
            deadline: Some(std::time::Instant::now()),
            ..Control::default()
        };
        let work = Work::new(Some(&context), Some(&control));
        assert!(matches!(
            Selector::compile_checked(b"", Some(&context), Some(&control)),
            Err(Interrupted::Deadline)
        ));
        let regex = GoRegex::compile(b"a").unwrap();
        assert_eq!(regex.captures_work(b"", work), Err(Interrupted::Deadline));
        assert_eq!(
            regex.expand_work(b"", b"", &[], work),
            Err(Interrupted::Deadline)
        );
        assert_eq!(
            super::normalization::normalized_lower(b"", work),
            Err(Interrupted::Deadline)
        );
        account.snapshot().assert_memory_released();
    }

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
        engine.snapshot().assert_memory_released();
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
        let output = super::normalization::normalized_lower(
            "\u{fb03}A\u{301}".as_bytes(),
            super::Work::new(Some(&context), None),
        )
        .unwrap();
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
                    observed.assert_memory_released();
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
        engine.snapshot().assert_memory_released();
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
        compiled.snapshot().assert_memory_released();
        searched.snapshot().assert_memory_released();
        engine.snapshot().assert_memory_released_with_queries(2);
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
