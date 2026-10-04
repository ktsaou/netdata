// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from prometheus/prometheus @ 5241a27fe3c6983549fccc32f6e65917408c63cd,
// promql/parser/lex.go. Copyright The Prometheus Authors; see NOTICE.
use std::borrow::Cow;

use super::{
    Span, quote,
    token::{TokenKind, TokenKind::*},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item<'a> {
    pub kind: TokenKind,
    pub span: Span,
    pub value: Cow<'a, [u8]>,
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
pub struct Lexer<'a> {
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

impl<'a> Lexer<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self::with_series_description(input, false)
    }

    pub fn with_series_description(input: &'a [u8], series_description: bool) -> Self {
        Self {
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

    fn next_rune(&mut self) -> i32 {
        let (r, width) = quote::decode_rune(&self.input[self.pos..]);
        self.width = width;
        self.pos += width;
        r
    }
    fn backup(&mut self) {
        self.pos -= self.width;
    }
    fn peek(&mut self) -> i32 {
        let r = self.next_rune();
        self.backup();
        r
    }
    fn accept(&mut self, valid: &[u8]) -> bool {
        let r = self.next_rune();
        if (0..128).contains(&r) && valid.contains(&(r as u8)) {
            true
        } else {
            self.backup();
            false
        }
    }
    fn is(&mut self, valid: &[u8]) -> bool {
        let r = self.peek();
        (0..128).contains(&r) && valid.contains(&(r as u8))
    }
    fn accept_run(&mut self, valid: &[u8]) {
        while self.accept(valid) {}
    }
    fn ignore(&mut self) {
        self.start = self.pos;
    }
    fn skip_spaces(&mut self) {
        while space(self.peek()) {
            self.next_rune();
        }
        self.ignore();
    }
    fn emit(&mut self, kind: TokenKind) {
        self.pending = Some(Item {
            kind,
            span: Span {
                start: self.start,
                end: self.pos,
            },
            value: Cow::Borrowed(&self.input[self.start..self.pos]),
        });
        self.start = self.pos;
    }
    fn error(&mut self, message: impl Into<String>) -> State {
        self.pending = Some(Item {
            kind: ERROR,
            span: Span {
                start: self.start,
                end: self.pos,
            },
            value: Cow::Owned(message.into().into_bytes()),
        });
        State::Done
    }
    fn error_rune(&mut self, message: &str, r: i32) -> State {
        self.error(format!("{message}{}", quote::rune(r)))
    }

    fn statements(&mut self) -> State {
        if self.histogram != HistogramState::None {
            return State::Histogram;
        }
        if self.brace_open {
            return State::Braces;
        }
        if self.input.get(self.pos) == Some(&b'#') {
            return State::Comment;
        }
        let r = self.next_rune();
        match r {
            -1 => {
                if self.parens != 0 {
                    return self.error("unclosed left parenthesis");
                }
                if self.bracket_open {
                    return self.error("unclosed left bracket");
                }
                self.emit(EOF);
                return State::Done;
            }
            44 => self.emit(COMMA),
            42 => self.emit(MUL),
            47 => self.emit(DIV),
            37 => self.emit(MOD),
            43 => self.emit(ADD),
            45 => self.emit(SUB),
            94 => self.emit(POW),
            61 => match self.peek() {
                61 => {
                    self.next_rune();
                    self.emit(EQLC);
                }
                126 => return self.error_rune("unexpected character after '=': ", 126),
                _ => self.emit(EQL),
            },
            33 => {
                let t = self.next_rune();
                if t != 61 {
                    return self.error_rune("unexpected character after '!': ", t);
                }
                self.emit(NEQ);
            }
            60 | 62 => match self.peek() {
                61 => {
                    self.next_rune();
                    self.emit(if r == 60 { LTE } else { GTE });
                }
                47 => {
                    self.next_rune();
                    self.emit(if r == 60 { TRIM_UPPER } else { TRIM_LOWER });
                }
                _ => self.emit(if r == 60 { LSS } else { GTR }),
            },
            34 | 39 => {
                self.string_open = r;
                return State::String;
            }
            96 => {
                self.string_open = r;
                return State::RawString;
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
                return State::Braces;
            }
            91 => {
                if self.bracket_open {
                    return self.error_rune("unexpected left bracket ", r);
                }
                self.got_colon = false;
                self.emit(LEFT_BRACKET);
                if space(self.peek()) {
                    self.skip_spaces();
                }
                self.bracket_open = true;
                return State::Duration;
            }
            93 => {
                if !self.bracket_open {
                    return self.error_rune("unexpected right bracket ", r);
                }
                self.emit(RIGHT_BRACKET);
                self.bracket_open = false;
            }
            64 => self.emit(AT),
            _ if space(r) => return State::Space,
            _ if digit(r) || (r == 46 && digit(self.peek())) => {
                self.backup();
                return State::NumberDuration;
            }
            _ if alpha(r) || r == 58 => {
                if !self.bracket_open {
                    self.backup();
                    return State::Keyword;
                }
                if r == 58 {
                    if self.got_colon {
                        return self.error_rune("unexpected colon ", r);
                    }
                    self.emit(COLON);
                    self.got_colon = true;
                    return State::Statements;
                }
                if duration_start(r) && self.scan_duration_keyword() {
                    return State::Statements;
                }
                return self.error(format!(
                    "unexpected character: {}, expected ':'",
                    quote::rune(r)
                ));
            }
            _ => return self.error_rune("unexpected character: ", r),
        }
        State::Statements
    }

    fn braces(&mut self) -> State {
        if self.input.get(self.pos) == Some(&b'#') {
            return State::Comment;
        }
        let r = self.next_rune();
        match r {
            -1 => return self.error("unexpected end of input inside braces"),
            _ if space(r) => return State::Space,
            _ if alpha(r) => {
                self.backup();
                return State::Identifier;
            }
            44 => self.emit(COMMA),
            34 | 39 => {
                self.string_open = r;
                return State::String;
            }
            96 => {
                self.string_open = r;
                return State::RawString;
            }
            61 => {
                if self.next_rune() == 126 {
                    self.emit(EQL_REGEX);
                } else {
                    self.backup();
                    self.emit(EQL);
                }
            }
            33 => match self.next_rune() {
                126 => self.emit(NEQ_REGEX),
                61 => self.emit(NEQ),
                r => return self.error_rune("unexpected character after '!' inside braces: ", r),
            },
            123 => return self.error_rune("unexpected left brace ", r),
            125 => {
                self.emit(RIGHT_BRACE);
                self.brace_open = false;
                return if self.series_description {
                    State::Sequence
                } else {
                    State::Statements
                };
            }
            _ => return self.error_rune("unexpected character inside braces: ", r),
        }
        State::Braces
    }

    fn histogram(&mut self) -> State {
        match self.histogram {
            HistogramState::Mul | HistogramState::Add | HistogramState::Sub => {
                let token = match self.histogram {
                    HistogramState::Mul => TIMES,
                    HistogramState::Add => ADD,
                    _ => SUB,
                };
                self.histogram = HistogramState::None;
                self.next_rune();
                self.emit(token);
                return State::Sequence;
            }
            _ => {}
        }
        if self.bracket_open {
            return State::Buckets;
        }
        let r = self.next_rune();
        match r {
            _ if space(r) => {
                self.emit(SPACE);
                return State::Space;
            }
            _ if alpha(r) => {
                self.backup();
                return State::HistogramDescriptor;
            }
            58 => self.emit(COLON),
            45 => self.emit(SUB),
            120 => {
                self.emit(TIMES);
                return State::Number;
            }
            _ if digit(r) => {
                self.backup();
                return State::Number;
            }
            91 => {
                self.bracket_open = true;
                self.got_colon = false;
                self.got_duration = false;
                self.emit(LEFT_BRACKET);
                return State::Buckets;
            }
            125 if self.peek() == 125 => {
                self.next_rune();
                self.emit(CLOSE_HIST);
                self.histogram = match self.peek() {
                    120 => HistogramState::Mul,
                    43 => HistogramState::Add,
                    45 => HistogramState::Sub,
                    _ => HistogramState::None,
                };
                return if self.histogram == HistogramState::None {
                    State::Sequence
                } else {
                    State::Histogram
                };
            }
            _ => return self.error_rune("histogram description incomplete unexpected: ", r),
        }
        State::Histogram
    }

    fn histogram_descriptor(&mut self) -> State {
        while alpha(self.next_rune()) {}
        self.backup();
        let word = &self.input[self.start..self.pos];
        if let Some(kind) = lookup(word, HISTOGRAM_DESCRIPTORS) {
            if self.peek() == 58 {
                self.emit(kind);
                return State::Histogram;
            }
            self.error("missing `:` for histogram descriptor");
        } else if lookup(word, KEYWORDS) == Some(NUMBER) {
            self.emit(NUMBER);
            return State::Histogram;
        } else if let Some(kind) = lookup(word, RESET_HINTS) {
            self.emit(kind);
            return State::Histogram;
        } else {
            self.error(format!(
                "bad histogram descriptor found: {}",
                quote::bytes(word)
            ));
        }
        State::Statements
    }

    fn buckets(&mut self) -> State {
        let r = self.next_rune();
        match r {
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
            _ => self.error_rune("invalid character in buckets description: ", r),
        }
    }

    fn sequence(&mut self) -> State {
        if self.histogram != HistogramState::None {
            return State::Histogram;
        }
        let r = self.next_rune();
        match r {
            -1 => return State::Statements,
            123 if self.peek() == 123 => {
                self.histogram = HistogramState::Open;
                self.next_rune();
                self.emit(OPEN_HIST);
                return State::Histogram;
            }
            _ if space(r) => {
                self.emit(SPACE);
                self.space();
            }
            43 => self.emit(ADD),
            45 => self.emit(SUB),
            120 => self.emit(TIMES),
            95 => self.emit(BLANK),
            _ if digit(r) || (r == 46 && digit(self.peek())) => {
                self.backup();
                self.number(false);
            }
            _ if alpha(r) => {
                self.backup();
                return State::Keyword;
            }
            _ => return self.error_rune("unexpected character in series sequence: ", r),
        }
        State::Sequence
    }

    fn escape(&mut self) -> State {
        let mut r = self.next_rune();
        let (mut n, base, maximum) = match r {
            97 | 98 | 102 | 110 | 114 | 116 | 118 | 92 => return State::String,
            _ if r == self.string_open => return State::String,
            48..=55 => (3, 8, 255),
            120 => {
                r = self.next_rune();
                (2, 16, 255)
            }
            117 => {
                r = self.next_rune();
                (4, 16, 0x10ffff)
            }
            85 => {
                r = self.next_rune();
                (8, 16, 0x10ffff)
            }
            -1 => {
                self.error("escape sequence not terminated");
                return State::String;
            }
            _ => {
                self.error(format!(
                    "unknown escape sequence {}",
                    quote::unicode_rune(r)
                ));
                return State::String;
            }
        };
        let mut x = 0u32;
        while n > 0 {
            let d = digit_value(r);
            if d >= base {
                self.error(if r == -1 {
                    "escape sequence not terminated".into()
                } else {
                    format!(
                        "illegal character {} in escape sequence",
                        quote::unicode_rune(r)
                    )
                });
                return State::String;
            }
            x = x * base + d;
            n -= 1;
            if n > 0 {
                r = self.next_rune();
            }
        }
        if x > maximum || (0xd800..0xe000).contains(&x) {
            self.error("escape sequence is an invalid Unicode code point");
        }
        State::String
    }

    fn string(&mut self, raw: bool) -> State {
        loop {
            let r = self.next_rune();
            if !raw && r == 92 {
                return State::Escape;
            }
            if r == 0xfffd {
                self.error("invalid UTF-8 rune");
                return if raw { State::RawString } else { State::String };
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
                return State::Statements;
            }
        }
    }

    fn space(&mut self) -> State {
        self.skip_spaces();
        State::Statements
    }
    fn comment(&mut self) -> State {
        self.pos += 1;
        loop {
            if matches!(self.next_rune(), -1 | 10 | 13) {
                break;
            }
        }
        self.backup();
        self.emit(COMMENT);
        State::Statements
    }

    fn scan_number(&mut self) -> bool {
        let initial = self.pos;
        let digits: &[u8] = if !self.series_description && self.accept(b"0") && self.accept(b"xX") {
            self.accept(b"_");
            b"0123456789abcdefABCDEF"
        } else {
            b"0123456789"
        };
        self.accept(b".");
        self.accept(digits);
        let mut dot = false;
        let mut exponent = false;
        while self.is(digits) || self.is(b"._eE") {
            if self.is(b".") && dot {
                self.accept(b".");
                return false;
            }
            if self.is(b"eE") && exponent {
                self.accept(b"eE");
                return false;
            }
            if self.accept(b".") {
                dot = true;
                if self.accept(b"_.") || digits.len() > 10 {
                    return false;
                }
                continue;
            }
            if self.accept(b"eE") {
                exponent = true;
                self.accept(b"+-");
                if self.accept(b"._eE") || self.peek() == -1 {
                    return false;
                }
                continue;
            }
            if self.accept(b"_") {
                if self.accept(b"._eE") || self.peek() == -1 {
                    return false;
                }
                continue;
            }
            self.accept_run(digits);
        }
        if self.pos == initial {
            return false;
        }
        let r = self.peek();
        (self.series_description && r == 120) || !alphanumeric(r)
    }

    fn remaining_duration(&mut self) -> bool {
        if !self.accept(b"smhdwy") {
            return false;
        }
        self.accept(b"s");
        while self.accept(b"0123456789") {
            while self.accept(b"0123456789") {}
            if !self.accept(b"smhdw") {
                return false;
            }
            self.accept(b"s");
        }
        !alphanumeric(self.next_rune())
    }
    fn number(&mut self, allow_duration: bool) -> State {
        if self.scan_number() {
            self.emit(NUMBER);
            return State::Statements;
        }
        if allow_duration && self.remaining_duration() {
            self.backup();
            self.emit(DURATION);
            return State::Statements;
        }
        let syntax = if allow_duration {
            "bad number or duration syntax: "
        } else {
            "bad number syntax: "
        };
        self.error(format!(
            "{syntax}{}",
            quote::bytes(&self.input[self.start..self.pos])
        ))
    }

    fn identifier(&mut self, keyword: bool) -> State {
        loop {
            let r = self.next_rune();
            if !(alphanumeric(r) || (keyword && r == 58)) {
                break;
            }
        }
        self.backup();
        let word = &self.input[self.start..self.pos];
        let mut kind = IDENTIFIER;
        if keyword {
            kind = lookup(word, KEYWORDS).unwrap_or(if word.contains(&b':') {
                METRIC_IDENTIFIER
            } else {
                IDENTIFIER
            });
            if matches!(kind, FILL | FILL_LEFT | FILL_RIGHT) {
                let next = self.input[self.pos..]
                    .iter()
                    .copied()
                    .find(|&b| !space(i32::from(b)));
                if next != Some(b'(') {
                    kind = IDENTIFIER;
                }
            }
        }
        self.emit(kind);
        if keyword && self.series_description && self.peek() != 123 {
            State::Sequence
        } else {
            State::Statements
        }
    }

    fn scan_duration_keyword(&mut self) -> bool {
        while alpha(self.next_rune()) {}
        self.backup();
        if let Some(kind @ (STEP | RANGE | MAX_OF | MIN_OF)) =
            lookup(&self.input[self.start..self.pos], KEYWORDS)
        {
            self.emit(kind);
            true
        } else {
            false
        }
    }
    fn duration(&mut self) -> State {
        let r = self.next_rune();
        match r {
            -1 => return self.error("unexpected end of input in duration expression"),
            93 => {
                self.emit(RIGHT_BRACKET);
                self.bracket_open = false;
                self.got_colon = false;
                return State::Statements;
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
            _ if space(r) => self.skip_spaces(),
            _ if duration_start(r) => {
                if self.scan_duration_keyword() {
                    return State::Duration;
                }
                return self.error_rune("unexpected character in duration expression: ", r);
            }
            _ if digit(r) || (r == 46 && digit(self.peek())) => {
                self.backup();
                self.got_duration = true;
                return State::NumberDuration;
            }
            _ => return self.error_rune("unexpected character in duration expression: ", r),
        }
        State::Duration
    }

    fn step(&mut self) -> State {
        match self.state {
            State::Statements => self.statements(),
            State::Braces => self.braces(),
            State::Histogram => self.histogram(),
            State::HistogramDescriptor => self.histogram_descriptor(),
            State::Buckets => self.buckets(),
            State::Sequence => self.sequence(),
            State::Escape => self.escape(),
            State::String => self.string(false),
            State::RawString => self.string(true),
            State::Space => self.space(),
            State::Comment => self.comment(),
            State::Number => self.number(false),
            State::NumberDuration => self.number(true),
            State::Identifier => self.identifier(false),
            State::Keyword => self.identifier(true),
            State::Duration => self.duration(),
            State::Done => State::Done,
        }
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Item<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.pending = None;
        while self.state != State::Done && self.pending.is_none() {
            self.state = self.step();
        }
        self.pending.take()
    }
}
