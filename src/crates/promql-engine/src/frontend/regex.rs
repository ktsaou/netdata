// SPDX-License-Identifier: GPL-3.0-or-later
// Go-compatible frontend over an explicit regex-automata NFA; see NOTICE.
use std::{borrow::Cow, ops::Range, sync::Arc};

use regex_automata::{Input, nfa::thompson::pikevm::PikeVM};

use super::{ast::CompiledMatcher, quote::decode_rune};

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
    bytes: Cow<'a, [u8]>,
    original_offsets: Vec<usize>,
}

impl<'a> Subject<'a> {
    fn new(original: &'a [u8]) -> Self {
        if std::str::from_utf8(original).is_ok() {
            return Self {
                original,
                bytes: Cow::Borrowed(original),
                original_offsets: Vec::new(),
            };
        }
        let mut bytes = Vec::new();
        let mut offsets = vec![0];
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
            bytes: Cow::Owned(bytes),
            original_offsets: offsets,
        }
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
    prefix: Vec<u8>,
    names: Vec<Vec<u8>>,
}

impl GoRegex {
    pub fn compile(pattern: &[u8]) -> Result<Self, Vec<u8>> {
        let tree = syntax::parse_standard(pattern, syntax::PERL).map_err(|error| error.bytes())?;
        Self::from_tree(tree)
    }

    pub fn compile_label_replace(pattern: &[u8]) -> Result<Self, Vec<u8>> {
        let mut anchored = b"^(?s:".to_vec();
        anchored.extend_from_slice(pattern);
        anchored.extend_from_slice(b")$");
        Self::compile(&anchored)
    }

    fn from_tree(mut tree: Tree) -> Result<Self, Vec<u8>> {
        let flat = program::Program::compile(&mut tree);
        let (program, prefix) = nfa::build(&flat)?;
        Ok(Self {
            program,
            prefix,
            names: tree.names,
        })
    }

    pub fn names(&self) -> &[Vec<u8>] {
        &self.names
    }

    pub fn subexp_index(&self, name: &[u8]) -> Option<usize> {
        if name.is_empty() {
            return None;
        }
        self.names.iter().position(|candidate| candidate == name)
    }

    pub fn captures(&self, original: &[u8]) -> Option<Vec<Option<Range<usize>>>> {
        let subject = Subject::new(original);
        self.captures_subject(&subject, 0)
    }

    fn captures_subject(
        &self,
        subject: &Subject<'_>,
        start: usize,
    ) -> Option<Vec<Option<Range<usize>>>> {
        if !subject.original.starts_with(&self.prefix) {
            return None;
        }
        let mut cache = self.program.create_cache();
        let mut captures = self.program.create_captures();
        let input =
            Input::new(&subject.bytes).span(subject.normalized_offset(start)..subject.bytes.len());
        self.program.search(&mut cache, &input, &mut captures);
        if !captures.is_match() {
            return None;
        }
        Some(
            (0..self.names.len())
                .map(|index| {
                    captures
                        .get_group(index)
                        .map(|span| subject.offset(span.start)..subject.offset(span.end))
                })
                .collect(),
        )
    }

    /// Supplemental Go FindAll qualification; production label replacement uses one anchored match.
    #[cfg(feature = "fixtures")]
    pub fn captures_all(&self, original: &[u8]) -> Vec<Vec<Option<Range<usize>>>> {
        let subject = Subject::new(original);
        let mut result = Vec::new();
        let mut position = 0;
        let mut previous_end = None;
        while position <= original.len() {
            let Some(captures) = self.captures_subject(&subject, position) else {
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
                result.push(captures);
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
        let mut output = Vec::new();
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
    let mut tree = syntax::parse_standard(pattern, syntax::PERL).map_err(|error| error.bytes())?;
    Ok(program::Program::compile(&mut tree).inspect())
}

#[cfg(feature = "fixtures")]
pub fn inspect_normalization(input: &[u8]) -> (Vec<u8>, Vec<u8>) {
    (
        normalization::normalized_lower(input),
        normalization::lower(input),
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
        canonical: string::canonical(&tree),
        names: tree.names,
    })
}

#[derive(Debug)]
pub struct Selector {
    program: selector::Program,
}

impl Selector {
    pub fn compile(pattern: &[u8]) -> Result<Arc<dyn CompiledMatcher>, Vec<u8>> {
        let program = selector::Program::compile(pattern)?;
        Ok(Arc::new(Self { program }))
    }
}

impl CompiledMatcher for Selector {
    fn matches(&self, value: &[u8]) -> bool {
        self.program.matches(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{GoRegex, Selector};

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
