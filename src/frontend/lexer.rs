// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from prometheus/prometheus @ 5241a27fe3c6983549fccc32f6e65917408c63cd,
// promql/parser/lex.go. Copyright The Prometheus Authors; see NOTICE.
use crate::storage::Interrupted;

use super::{
    Span,
    bytes::Bytes,
    quote,
    token::{TokenKind, TokenKind::*},
    work::Work,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item<'a> {
    pub kind: TokenKind,
    pub span: Span,
    pub value: Bytes<'a>,
}
impl Item<'_> {
    pub(super) fn copy_in(&self, work: Work<'_>) -> Result<Self, Interrupted> {
        Ok(Self {
            kind: self.kind,
            span: self.span,
            value: self.value.copy_in(work)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Statements,
    Braces,
    Histogram,
    HistogramDescriptor,
    Buckets,
    Sequence,
    Escape,
    String,
    RawString,
    Space,
    Comment,
    Number,
    NumberDuration,
    Identifier,
    Keyword,
    Duration,
    Done,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistogramState {
    None,
    Open,
    Mul,
    Add,
    Sub,
}

/// A byte scanner. Consumers parsing expressions stop at the first ERROR item.
pub struct Lexer<'a, 'w> {
    work: Work<'w>,
    input: &'a [u8],
    state: State,
    pos: usize,
    start: usize,
    width: usize,
    pending: Option<Item<'a>>,
    parens: i64,
    brace_open: bool,
    bracket_open: bool,
    got_colon: bool,
    got_duration: bool,
    string_open: i32,
    series_description: bool,
    histogram: HistogramState,
}

fn space(r: i32) -> bool {
    matches!(r, 32 | 9 | 10 | 13)
}
fn alpha(r: i32) -> bool {
    r == 95 || (65..=90).contains(&r) || (97..=122).contains(&r)
}
fn digit(r: i32) -> bool {
    (48..=57).contains(&r)
}
fn alphanumeric(r: i32) -> bool {
    alpha(r) || digit(r)
}
fn duration_start(r: i32) -> bool {
    matches!(r, 83 | 115 | 82 | 114 | 77 | 109)
}
fn digit_value(r: i32) -> u32 {
    match r {
        48..=57 => (r - 48) as u32,
        65..=70 => (r - 65 + 10) as u32,
        97..=102 => (r - 97 + 10) as u32,
        _ => 16,
    }
}

const KEYWORDS: &[(&[u8], TokenKind)] = &[
    (b"and", LAND),
    (b"or", LOR),
    (b"unless", LUNLESS),
    (b"atan2", ATAN2),
    (b"sum", SUM),
    (b"avg", AVG),
    (b"count", COUNT),
    (b"min", MIN),
    (b"max", MAX),
    (b"group", GROUP),
    (b"stddev", STDDEV),
    (b"stdvar", STDVAR),
    (b"topk", TOPK),
    (b"bottomk", BOTTOMK),
    (b"count_values", COUNT_VALUES),
    (b"quantile", QUANTILE),
    (b"limitk", LIMITK),
    (b"limit_ratio", LIMIT_RATIO),
    (b"offset", OFFSET),
    (b"smoothed", SMOOTHED),
    (b"anchored", ANCHORED),
    (b"by", BY),
    (b"without", WITHOUT),
    (b"on", ON),
    (b"ignoring", IGNORING),
    (b"group_left", GROUP_LEFT),
    (b"group_right", GROUP_RIGHT),
    (b"fill", FILL),
    (b"fill_left", FILL_LEFT),
    (b"fill_right", FILL_RIGHT),
    (b"bool", BOOL),
    (b"start", START),
    (b"end", END),
    (b"step", STEP),
    (b"range", RANGE),
    (b"max_of", MAX_OF),
    (b"min_of", MIN_OF),
    (b"inf", NUMBER),
    (b"nan", NUMBER),
];
const HISTOGRAM_DESCRIPTORS: &[(&[u8], TokenKind)] = &[
    (b"sum", SUM_DESC),
    (b"count", COUNT_DESC),
    (b"schema", SCHEMA_DESC),
    (b"offset", OFFSET_DESC),
    (b"n_offset", NEGATIVE_OFFSET_DESC),
    (b"buckets", BUCKETS_DESC),
    (b"n_buckets", NEGATIVE_BUCKETS_DESC),
    (b"z_bucket", ZERO_BUCKET_DESC),
    (b"z_bucket_w", ZERO_BUCKET_WIDTH_DESC),
    (b"custom_values", CUSTOM_VALUES_DESC),
    (b"counter_reset_hint", COUNTER_RESET_HINT_DESC),
];
const RESET_HINTS: &[(&[u8], TokenKind)] = &[
    (b"unknown", UNKNOWN_COUNTER_RESET),
    (b"reset", COUNTER_RESET),
    (b"not_reset", NOT_COUNTER_RESET),
    (b"gauge", GAUGE_TYPE),
];
fn lookup(word: &[u8], table: &[(&[u8], TokenKind)]) -> Option<TokenKind> {
    table
        .iter()
        .find_map(|(key, kind)| word.eq_ignore_ascii_case(key).then_some(*kind))
}

impl<'a, 'w> Lexer<'a, 'w> {
    pub fn new(input: &'a [u8]) -> Self {
        Self::with_series_description(input, false)
    }

    pub fn with_series_description(input: &'a [u8], series_description: bool) -> Self {
        Self::new_in(input, series_description, Work::default())
    }
    pub(super) fn new_in(input: &'a [u8], series_description: bool, work: Work<'w>) -> Self {
        Self {
            work,
            input,
            state: State::Statements,
            pos: 0,
            start: 0,
            width: 0,
            pending: None,
            parens: 0,
            brace_open: false,
            bracket_open: false,
            got_colon: false,
            got_duration: false,
            string_open: 0,
            series_description,
            histogram: HistogramState::None,
        }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    fn next_rune(&mut self) -> Result<i32, Interrupted> {
        self.work.step(1)?;
        let (r, width) = quote::decode_rune(&self.input[self.pos..]);
        self.width = width;
        self.pos += width;
        Ok(r)
    }
    fn backup(&mut self) {
        self.pos -= self.width;
    }
    fn peek(&mut self) -> Result<i32, Interrupted> {
        let r = self.next_rune()?;
        self.backup();
        Ok(r)
    }
    fn accept(&mut self, valid: &[u8]) -> Result<bool, Interrupted> {
        let r = self.next_rune()?;

        Ok(if (0..128).contains(&r) && valid.contains(&(r as u8)) {
            true
        } else {
            self.backup();
            false
        })
    }
    fn is(&mut self, valid: &[u8]) -> Result<bool, Interrupted> {
        let r = self.peek()?;
        Ok((0..128).contains(&r) && valid.contains(&(r as u8)))
    }
    fn accept_run(&mut self, valid: &[u8]) -> Result<(), Interrupted> {
        while self.accept(valid)? {}
        Ok(())
    }
    fn ignore(&mut self) {
        self.start = self.pos;
    }
    fn skip_spaces(&mut self) -> Result<(), Interrupted> {
        while space(self.peek()?) {
            self.next_rune()?;
        }
        self.ignore();
        Ok(())
    }
    fn emit(&mut self, kind: TokenKind) {
        self.pending = Some(Item {
            kind,
            span: Span {
                start: self.start,
                end: self.pos,
            },
            value: Bytes::Borrowed(&self.input[self.start..self.pos]),
        });
        self.start = self.pos;
    }
    fn error(&mut self, message: impl std::fmt::Display) -> Result<State, Interrupted> {
        self.pending = Some(Item {
            kind: ERROR,
            span: Span {
                start: self.start,
                end: self.pos,
            },
            value: Bytes::Owned(self.work.format(format_args!("{message}"))?),
        });
        Ok(State::Done)
    }
    fn error_rune(&mut self, message: &str, r: i32) -> Result<State, Interrupted> {
        self.error(format_args!("{message}{}", quote::rune(r)))
    }

    fn statements(&mut self) -> Result<State, Interrupted> {
        if self.histogram != HistogramState::None {
            return Ok(State::Histogram);
        }
        if self.brace_open {
            return Ok(State::Braces);
        }
        if self.input.get(self.pos) == Some(&b'#') {
            return Ok(State::Comment);
        }
        let r = self.next_rune()?;
        match r {
            -1 => {
                if self.parens != 0 {
                    return self.error("unclosed left parenthesis");
                }
                if self.bracket_open {
                    return self.error("unclosed left bracket");
                }
                self.emit(EOF);
                return Ok(State::Done);
            }
            44 => self.emit(COMMA),
            42 => self.emit(MUL),
            47 => self.emit(DIV),
            37 => self.emit(MOD),
            43 => self.emit(ADD),
            45 => self.emit(SUB),
            94 => self.emit(POW),
            61 => match self.peek()? {
                61 => {
                    self.next_rune()?;
                    self.emit(EQLC);
                }
                126 => return self.error_rune("unexpected character after '=': ", 126),
                _ => self.emit(EQL),
            },
            33 => {
                let t = self.next_rune()?;
                if t != 61 {
                    return self.error_rune("unexpected character after '!': ", t);
                }
                self.emit(NEQ);
            }
            60 | 62 => match self.peek()? {
                61 => {
                    self.next_rune()?;
                    self.emit(if r == 60 { LTE } else { GTE });
                }
                47 => {
                    self.next_rune()?;
                    self.emit(if r == 60 { TRIM_UPPER } else { TRIM_LOWER });
                }
                _ => self.emit(if r == 60 { LSS } else { GTR }),
            },
            34 | 39 => {
                self.string_open = r;
                return Ok(State::String);
            }
            96 => {
                self.string_open = r;
                return Ok(State::RawString);
            }
            40 => {
                self.emit(LEFT_PAREN);
                self.parens += 1;
            }
            41 => {
                self.emit(RIGHT_PAREN);
                self.parens -= 1;
                if self.parens < 0 {
                    return self.error_rune("unexpected right parenthesis ", r);
                }
            }
            123 => {
                self.emit(LEFT_BRACE);
                self.brace_open = true;
                return Ok(State::Braces);
            }
            91 => {
                if self.bracket_open {
                    return self.error_rune("unexpected left bracket ", r);
                }
                self.got_colon = false;
                self.emit(LEFT_BRACKET);
                if space(self.peek()?) {
                    self.skip_spaces()?;
                }
                self.bracket_open = true;
                return Ok(State::Duration);
            }
            93 => {
                if !self.bracket_open {
                    return self.error_rune("unexpected right bracket ", r);
                }
                self.emit(RIGHT_BRACKET);
                self.bracket_open = false;
            }
            64 => self.emit(AT),
            _ if space(r) => return Ok(State::Space),
            _ if digit(r) || (r == 46 && digit(self.peek()?)) => {
                self.backup();
                return Ok(State::NumberDuration);
            }
            _ if alpha(r) || r == 58 => {
                if !self.bracket_open {
                    self.backup();
                    return Ok(State::Keyword);
                }
                if r == 58 {
                    if self.got_colon {
                        return self.error_rune("unexpected colon ", r);
                    }
                    self.emit(COLON);
                    self.got_colon = true;
                    return Ok(State::Statements);
                }
                if duration_start(r) && self.scan_duration_keyword()? {
                    return Ok(State::Statements);
                }
                return self.error(format_args!(
                    "unexpected character: {}, expected ':'",
                    quote::rune(r)
                ));
            }
            _ => return self.error_rune("unexpected character: ", r),
        }
        Ok(State::Statements)
    }

    fn braces(&mut self) -> Result<State, Interrupted> {
        if self.input.get(self.pos) == Some(&b'#') {
            return Ok(State::Comment);
        }
        let r = self.next_rune()?;
        match r {
            -1 => return self.error("unexpected end of input inside braces"),
            _ if space(r) => return Ok(State::Space),
            _ if alpha(r) => {
                self.backup();
                return Ok(State::Identifier);
            }
            44 => self.emit(COMMA),
            34 | 39 => {
                self.string_open = r;
                return Ok(State::String);
            }
            96 => {
                self.string_open = r;
                return Ok(State::RawString);
            }
            61 => {
                if self.next_rune()? == 126 {
                    self.emit(EQL_REGEX);
                } else {
                    self.backup();
                    self.emit(EQL);
                }
            }
            33 => match self.next_rune()? {
                126 => self.emit(NEQ_REGEX),
                61 => self.emit(NEQ),
                r => {
                    return self.error_rune("unexpected character after '!' inside braces: ", r);
                }
            },
            123 => return self.error_rune("unexpected left brace ", r),
            125 => {
                self.emit(RIGHT_BRACE);
                self.brace_open = false;
                return Ok(if self.series_description {
                    State::Sequence
                } else {
                    State::Statements
                });
            }
            _ => return self.error_rune("unexpected character inside braces: ", r),
        }
        Ok(State::Braces)
    }

    fn histogram(&mut self) -> Result<State, Interrupted> {
        match self.histogram {
            HistogramState::Mul | HistogramState::Add | HistogramState::Sub => {
                let token = match self.histogram {
                    HistogramState::Mul => TIMES,
                    HistogramState::Add => ADD,
                    _ => SUB,
                };
                self.histogram = HistogramState::None;
                self.next_rune()?;
                self.emit(token);
                return Ok(State::Sequence);
            }
            _ => {}
        }
        if self.bracket_open {
            return Ok(State::Buckets);
        }
        let r = self.next_rune()?;
        match r {
            _ if space(r) => {
                self.emit(SPACE);
                return Ok(State::Space);
            }
            _ if alpha(r) => {
                self.backup();
                return Ok(State::HistogramDescriptor);
            }
            58 => self.emit(COLON),
            45 => self.emit(SUB),
            120 => {
                self.emit(TIMES);
                return Ok(State::Number);
            }
            _ if digit(r) => {
                self.backup();
                return Ok(State::Number);
            }
            91 => {
                self.bracket_open = true;
                self.got_colon = false;
                self.got_duration = false;
                self.emit(LEFT_BRACKET);
                return Ok(State::Buckets);
            }
            125 if self.peek()? == 125 => {
                self.next_rune()?;
                self.emit(CLOSE_HIST);
                self.histogram = match self.peek()? {
                    120 => HistogramState::Mul,
                    43 => HistogramState::Add,
                    45 => HistogramState::Sub,
                    _ => HistogramState::None,
                };
                return Ok(if self.histogram == HistogramState::None {
                    State::Sequence
                } else {
                    State::Histogram
                });
            }
            _ => return self.error_rune("histogram description incomplete unexpected: ", r),
        }
        Ok(State::Histogram)
    }

    fn histogram_descriptor(&mut self) -> Result<State, Interrupted> {
        while alpha(self.next_rune()?) {}
        self.backup();
        let word = &self.input[self.start..self.pos];
        if let Some(kind) = lookup(word, HISTOGRAM_DESCRIPTORS) {
            if self.peek()? == 58 {
                self.emit(kind);
                return Ok(State::Histogram);
            }
            self.error("missing `:` for histogram descriptor")?;
        } else if lookup(word, KEYWORDS) == Some(NUMBER) {
            self.emit(NUMBER);
            return Ok(State::Histogram);
        } else if let Some(kind) = lookup(word, RESET_HINTS) {
            self.emit(kind);
            return Ok(State::Histogram);
        } else {
            self.error(format_args!(
                "bad histogram descriptor found: {}",
                quote::quoted(word)
            ))?;
        }
        Ok(State::Statements)
    }

    fn buckets(&mut self) -> Result<State, Interrupted> {
        let r = self.next_rune()?;

        Ok(match r {
            _ if space(r) => {
                self.emit(SPACE);
                State::Space
            }
            45 => {
                self.emit(SUB);
                State::Number
            }
            _ if digit(r) => {
                self.backup();
                State::Number
            }
            93 => {
                self.bracket_open = false;
                self.emit(RIGHT_BRACKET);
                State::Histogram
            }
            _ if alpha(r) => {
                if lookup(&self.input[self.start..self.pos], KEYWORDS) == Some(NUMBER) {
                    self.emit(NUMBER);
                    State::Statements
                } else {
                    State::Buckets
                }
            }
            _ => self.error_rune("invalid character in buckets description: ", r)?,
        })
    }

    fn sequence(&mut self) -> Result<State, Interrupted> {
        if self.histogram != HistogramState::None {
            return Ok(State::Histogram);
        }
        let r = self.next_rune()?;
        match r {
            -1 => return Ok(State::Statements),
            123 if self.peek()? == 123 => {
                self.histogram = HistogramState::Open;
                self.next_rune()?;
                self.emit(OPEN_HIST);
                return Ok(State::Histogram);
            }
            _ if space(r) => {
                self.emit(SPACE);
                self.space()?;
            }
            43 => self.emit(ADD),
            45 => self.emit(SUB),
            120 => self.emit(TIMES),
            95 => self.emit(BLANK),
            _ if digit(r) || (r == 46 && digit(self.peek()?)) => {
                self.backup();
                self.number(false)?;
            }
            _ if alpha(r) => {
                self.backup();
                return Ok(State::Keyword);
            }
            _ => return self.error_rune("unexpected character in series sequence: ", r),
        }
        Ok(State::Sequence)
    }

    fn escape(&mut self) -> Result<State, Interrupted> {
        let mut r = self.next_rune()?;
        let (mut n, base, maximum) = match r {
            97 | 98 | 102 | 110 | 114 | 116 | 118 | 92 => return Ok(State::String),
            _ if r == self.string_open => return Ok(State::String),
            48..=55 => (3, 8, 255),
            120 => {
                r = self.next_rune()?;
                (2, 16, 255)
            }
            117 => {
                r = self.next_rune()?;
                (4, 16, 0x10ffff)
            }
            85 => {
                r = self.next_rune()?;
                (8, 16, 0x10ffff)
            }
            -1 => {
                self.error("escape sequence not terminated")?;
                return Ok(State::String);
            }
            _ => {
                self.error(format_args!(
                    "unknown escape sequence {}",
                    quote::unicode_rune(r)
                ))?;
                return Ok(State::String);
            }
        };
        let mut x = 0u32;
        while n > 0 {
            let d = digit_value(r);
            if d >= base {
                if r == -1 {
                    self.error("escape sequence not terminated")?;
                } else {
                    self.error(format_args!(
                        "illegal character {} in escape sequence",
                        quote::unicode_rune(r)
                    ))?;
                }
                return Ok(State::String);
            }
            x = x * base + d;
            n -= 1;
            if n > 0 {
                r = self.next_rune()?;
            }
        }
        if x > maximum || (0xd800..0xe000).contains(&x) {
            self.error("escape sequence is an invalid Unicode code point")?;
        }
        Ok(State::String)
    }

    fn string(&mut self, raw: bool) -> Result<State, Interrupted> {
        loop {
            let r = self.next_rune()?;
            if !raw && r == 92 {
                return Ok(State::Escape);
            }
            if r == 0xfffd {
                self.error("invalid UTF-8 rune")?;
                return Ok(if raw { State::RawString } else { State::String });
            }
            if r == -1 || (!raw && r == 10) {
                // The original parser stops on this error; do not repeat it forever at raw EOF.
                return self.error(if raw {
                    "unterminated raw string"
                } else {
                    "unterminated quoted string"
                });
            }
            if r == self.string_open {
                self.emit(STRING);
                return Ok(State::Statements);
            }
        }
    }

    fn space(&mut self) -> Result<State, Interrupted> {
        self.skip_spaces()?;
        Ok(State::Statements)
    }
    fn comment(&mut self) -> Result<State, Interrupted> {
        self.pos += 1;
        loop {
            if matches!(self.next_rune()?, -1 | 10 | 13) {
                break;
            }
        }
        self.backup();
        self.emit(COMMENT);
        Ok(State::Statements)
    }

    fn scan_number(&mut self) -> Result<bool, Interrupted> {
        let initial = self.pos;
        let digits: &[u8] =
            if !self.series_description && self.accept(b"0")? && self.accept(b"xX")? {
                self.accept(b"_")?;
                b"0123456789abcdefABCDEF"
            } else {
                b"0123456789"
            };
        self.accept(b".")?;
        self.accept(digits)?;
        let mut dot = false;
        let mut exponent = false;
        while self.is(digits)? || self.is(b"._eE")? {
            if self.is(b".")? && dot {
                self.accept(b".")?;
                return Ok(false);
            }
            if self.is(b"eE")? && exponent {
                self.accept(b"eE")?;
                return Ok(false);
            }
            if self.accept(b".")? {
                dot = true;
                if self.accept(b"_.")? || digits.len() > 10 {
                    return Ok(false);
                }
                continue;
            }
            if self.accept(b"eE")? {
                exponent = true;
                self.accept(b"+-")?;
                if self.accept(b"._eE")? || self.peek()? == -1 {
                    return Ok(false);
                }
                continue;
            }
            if self.accept(b"_")? {
                if self.accept(b"._eE")? || self.peek()? == -1 {
                    return Ok(false);
                }
                continue;
            }
            self.accept_run(digits)?;
        }
        if self.pos == initial {
            return Ok(false);
        }
        let r = self.peek()?;
        Ok((self.series_description && r == 120) || !alphanumeric(r))
    }

    fn remaining_duration(&mut self) -> Result<bool, Interrupted> {
        if !self.accept(b"smhdwy")? {
            return Ok(false);
        }
        self.accept(b"s")?;
        while self.accept(b"0123456789")? {
            while self.accept(b"0123456789")? {}
            if !self.accept(b"smhdw")? {
                return Ok(false);
            }
            self.accept(b"s")?;
        }
        Ok(!alphanumeric(self.next_rune()?))
    }
    fn number(&mut self, allow_duration: bool) -> Result<State, Interrupted> {
        if self.scan_number()? {
            self.emit(NUMBER);
            return Ok(State::Statements);
        }
        if allow_duration && self.remaining_duration()? {
            self.backup();
            self.emit(DURATION);
            return Ok(State::Statements);
        }
        let syntax = if allow_duration {
            "bad number or duration syntax: "
        } else {
            "bad number syntax: "
        };
        self.error(format_args!(
            "{syntax}{}",
            quote::quoted(&self.input[self.start..self.pos])
        ))
    }

    fn identifier(&mut self, keyword: bool) -> Result<State, Interrupted> {
        loop {
            let r = self.next_rune()?;
            if !(alphanumeric(r) || (keyword && r == 58)) {
                break;
            }
        }
        self.backup();
        let word = &self.input[self.start..self.pos];
        let mut kind = IDENTIFIER;
        if keyword {
            kind = lookup(word, KEYWORDS).unwrap_or(if self.work.contains(word, b':')? {
                METRIC_IDENTIFIER
            } else {
                IDENTIFIER
            });
            if matches!(kind, FILL | FILL_LEFT | FILL_RIGHT) {
                let mut next = None;
                for &byte in &self.input[self.pos..] {
                    self.work.step(1)?;
                    if !space(i32::from(byte)) {
                        next = Some(byte);
                        break;
                    }
                }
                if next != Some(b'(') {
                    kind = IDENTIFIER;
                }
            }
        }
        self.emit(kind);

        Ok(
            if keyword && self.series_description && self.peek()? != 123 {
                State::Sequence
            } else {
                State::Statements
            },
        )
    }

    fn scan_duration_keyword(&mut self) -> Result<bool, Interrupted> {
        while alpha(self.next_rune()?) {}
        self.backup();

        Ok(
            if let Some(kind @ (STEP | RANGE | MAX_OF | MIN_OF)) =
                lookup(&self.input[self.start..self.pos], KEYWORDS)
            {
                self.emit(kind);
                true
            } else {
                false
            },
        )
    }
    fn duration(&mut self) -> Result<State, Interrupted> {
        let r = self.next_rune()?;
        match r {
            -1 => return self.error("unexpected end of input in duration expression"),
            93 => {
                self.emit(RIGHT_BRACKET);
                self.bracket_open = false;
                self.got_colon = false;
                return Ok(State::Statements);
            }
            58 => {
                self.emit(COLON);
                if !self.got_duration {
                    return self.error("unexpected colon before duration in duration expression");
                }
                if self.got_colon {
                    return self.error("unexpected repeated colon in duration expression");
                }
                self.got_colon = true;
            }
            40 => {
                self.emit(LEFT_PAREN);
                self.parens += 1;
            }
            41 => {
                self.emit(RIGHT_PAREN);
                self.parens -= 1;
                if self.parens < 0 {
                    return self.error_rune("unexpected right parenthesis ", r);
                }
            }
            43 => self.emit(ADD),
            45 => self.emit(SUB),
            42 => self.emit(MUL),
            47 => self.emit(DIV),
            37 => self.emit(MOD),
            94 => self.emit(POW),
            44 => self.emit(COMMA),
            _ if space(r) => self.skip_spaces()?,
            _ if duration_start(r) => {
                if self.scan_duration_keyword()? {
                    return Ok(State::Duration);
                }
                return self.error_rune("unexpected character in duration expression: ", r);
            }
            _ if digit(r) || (r == 46 && digit(self.peek()?)) => {
                self.backup();
                self.got_duration = true;
                return Ok(State::NumberDuration);
            }
            _ => return self.error_rune("unexpected character in duration expression: ", r),
        }
        Ok(State::Duration)
    }

    fn step(&mut self) -> Result<State, Interrupted> {
        self.work.step(1)?;

        Ok(match self.state {
            State::Statements => self.statements()?,
            State::Braces => self.braces()?,
            State::Histogram => self.histogram()?,
            State::HistogramDescriptor => self.histogram_descriptor()?,
            State::Buckets => self.buckets()?,
            State::Sequence => self.sequence()?,
            State::Escape => self.escape()?,
            State::String => self.string(false)?,
            State::RawString => self.string(true)?,
            State::Space => self.space()?,
            State::Comment => self.comment()?,
            State::Number => self.number(false)?,
            State::NumberDuration => self.number(true)?,
            State::Identifier => self.identifier(false)?,
            State::Keyword => self.identifier(true)?,
            State::Duration => self.duration()?,
            State::Done => State::Done,
        })
    }
    pub(super) fn next_in(&mut self) -> Result<Option<Item<'a>>, Interrupted> {
        self.pending = None;
        while self.state != State::Done && self.pending.is_none() {
            self.state = self.step()?;
        }
        Ok(self.pending.take())
    }
}

impl<'a, 'w> Iterator for Lexer<'a, 'w> {
    type Item = Item<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_in().expect("inert lexer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_escapes_use_literal_printable_unicode_diagnostics() {
        // Pinned Go lex.go uses fmt's %#U, which does not escape printable runes.
        for (input, expected) in [
            (&b"\"\\'\""[..], &b"unknown escape sequence U+0027 '''"[..]),
            (
                &b"\"\\x\\0\""[..],
                &b"illegal character U+005C '\\' in escape sequence"[..],
            ),
        ] {
            let first = Lexer::new(input).next().unwrap();
            assert_eq!(first.kind, ERROR);
            assert_eq!(first.span.start, 0);
            assert_eq!(first.value.as_ref(), expected);
        }
    }
}
