// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus generated_parser.y and parse.go construction semantics; see NOTICE.
use std::{borrow::Cow, sync::Arc};

use super::{
    Span,
    ast::{
        Arena, Cardinality, CompiledMatcher, Expr, MatchType, Matcher, NodeId, ParseError,
        ParserOptions, SelectorModifiers, VectorMatching, VectorSelector,
    },
    functions::function,
    lexer::{Item, Lexer},
    literal, number, quote,
    token::{TokenKind, TokenKind::*},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Normal,
    Duration,
    Offset,
    OffsetUnary,
}

impl Mode {
    fn is_offset(self) -> bool {
        matches!(self, Self::Offset | Self::OffsetUnary)
    }
}

enum Callee<'a> {
    Function(Item<'a>),
    Aggregate {
        item: Item<'a>,
        grouping: Option<Vec<Vec<u8>>>,
        without: bool,
    },
    Duration(Item<'a>, Mode),
}

enum Frame<'a> {
    Parse(u8, Mode),
    Prefix(Mode),
    Rest(u8, Mode),
    Unary(Item<'a>, Mode, bool),
    Paren(Item<'a>, Mode),
    Binary {
        left: NodeId,
        operator: Item<'a>,
        matching: VectorMatching,
        return_bool: bool,
        mode: Mode,
    },
    Arguments {
        callee: Callee<'a>,
        arguments: Vec<NodeId>,
        mode: Mode,
        last_comma: Option<Span>,
    },
    Range {
        left: NodeId,
        opening: Item<'a>,
    },
    RangeStep {
        left: NodeId,
        range: NodeId,
    },
    Offset(NodeId),
}

pub struct ParseOutput {
    pub arena: Arena,
    pub root: Option<NodeId>,
    pub errors: Vec<ParseError>,
}

pub fn parse<C>(input: &[u8], options: ParserOptions, compile: C) -> ParseOutput
where
    C: FnMut(&[u8]) -> Result<Arc<dyn CompiledMatcher>, Vec<u8>>,
{
    let mut output = parse_syntax(input, options, compile);
    if let Some(root) = output.root {
        super::validate::validate(input, &mut output.arena, root, &mut output.errors);
    }
    output
}

/// Regex compilation happens at grammar reductions, before ordered type validation.
pub fn parse_syntax<C>(input: &[u8], options: ParserOptions, compile: C) -> ParseOutput
where
    C: FnMut(&[u8]) -> Result<Arc<dyn CompiledMatcher>, Vec<u8>>,
{
    Parser {
        input,
        lexer: Lexer::new(input),
        lookahead: None,
        terminal: false,
        recovery_shifts: 0,
        last_closing: 0,
        arena: Arena::default(),
        errors: Vec::new(),
        options,
        compile,
    }
    .run()
}

struct Parser<'a, C> {
    input: &'a [u8],
    lexer: Lexer<'a>,
    lookahead: Option<Item<'a>>,
    terminal: bool,
    recovery_shifts: u8,
    last_closing: usize,
    arena: Arena,
    errors: Vec<ParseError>,
    options: ParserOptions,
    compile: C,
}

impl<'a, C> Parser<'a, C>
where
    C: FnMut(&[u8]) -> Result<Arc<dyn CompiledMatcher>, Vec<u8>>,
{
    fn error(&mut self, span: Span, message: impl AsRef<[u8]>) {
        self.errors.push(ParseError {
            span,
            message: message.as_ref().to_vec(),
        });
    }

    fn peek(&mut self) -> Item<'a> {
        if self.lookahead.is_none() {
            let item = if self.terminal {
                Item {
                    kind: EOF,
                    span: Span {
                        start: self.input.len(),
                        end: self.input.len(),
                    },
                    value: Cow::Borrowed(b""),
                }
            } else {
                loop {
                    let Some(item) = self.lexer.next() else {
                        break Item {
                            kind: EOF,
                            span: Span {
                                start: self.input.len(),
                                end: self.input.len(),
                            },
                            value: Cow::Borrowed(b""),
                        };
                    };
                    if item.kind != COMMENT {
                        break item;
                    }
                }
            };
            if item.kind == ERROR {
                self.error(
                    Span {
                        start: item.span.start,
                        end: self.input.len(),
                    },
                    &item.value,
                );
                self.terminal = true;
            } else if matches!(
                item.kind,
                RIGHT_BRACE | RIGHT_PAREN | RIGHT_BRACKET | DURATION | NUMBER
            ) {
                self.last_closing = item.span.end;
            }
            self.lookahead = Some(item);
        }
        self.lookahead.as_ref().unwrap().clone()
    }

    fn take(&mut self) -> Item<'a> {
        self.peek();
        let item = self.lookahead.take().unwrap();
        if !matches!(item.kind, EOF | ERROR) {
            self.recovery_shifts = self.recovery_shifts.saturating_sub(1);
        }
        item
    }

    fn accept(&mut self, kind: TokenKind) -> Option<Item<'a>> {
        (self.peek().kind == kind).then(|| self.take())
    }

    fn unexpected(&mut self, context: &str, expected: &str) {
        let item = self.peek();
        if item.kind == ERROR {
            return;
        }
        if self.recovery_shifts > 0 {
            return;
        }
        let mut message = format!("unexpected {}", description(&item));
        if !context.is_empty() {
            message.push_str(&format!(" in {context}"));
        }
        if !expected.is_empty() {
            message.push_str(&format!(", expected {expected}"));
        }
        self.error(item.span, message);
        self.recovery_shifts = 3;
    }

    fn expect(&mut self, kind: TokenKind, context: &str, expected: &str) -> Option<Item<'a>> {
        if let Some(item) = self.accept(kind) {
            Some(item)
        } else {
            self.unexpected(context, expected);
            None
        }
    }

    fn zero(&mut self) -> NodeId {
        self.arena.push(
            Expr::Number {
                value: 0.0,
                duration: false,
            },
            Span::default(),
        )
    }

    fn literal(&mut self, item: Item<'a>) -> NodeId {
        let (value, duration) = if item.kind == DURATION {
            let value = match number::duration(&item.value) {
                Ok(value) => number::seconds(value),
                Err(error) => {
                    self.error(item.span, error);
                    0.0
                }
            };
            (value, true)
        } else {
            let number = number::number(&item.value);
            if let Some(error) = number.error {
                self.error(item.span, error);
            }
            (number.value, false)
        };
        self.arena.push(Expr::Number { value, duration }, item.span)
    }

    fn unquote(&mut self, item: &Item<'a>) -> Vec<u8> {
        match literal::unquote(&item.value) {
            Ok(value) => value.into_owned(),
            Err(error) => {
                self.error(
                    item.span,
                    format!(
                        "error unquoting string {}: {error}",
                        quote::bytes(&item.value)
                    ),
                );
                Vec::new()
            }
        }
    }

    fn check_duration(&mut self, id: NodeId) -> NodeId {
        if let Expr::Number { value, .. } = self.arena[id].expression
            && number::duration_out_of_range(value)
        {
            self.error(self.arena[id].span, "duration out of range");
            return self.zero();
        }
        id
    }

    fn positive(&mut self, id: NodeId) -> NodeId {
        if let Expr::Number { value, .. } = self.arena[id].expression
            && value <= 0.0
        {
            self.error(self.arena[id].span, "duration must be greater than 0");
            return self.zero();
        }
        id
    }

    fn experimental_duration(&mut self, id: NodeId) {
        if !self.options.duration_expressions {
            self.error(
                self.arena[id].span,
                "experimental duration expression is not enabled",
            );
        }
    }

    fn duration_node(
        &mut self,
        operator: TokenKind,
        left: Option<NodeId>,
        right: Option<NodeId>,
        wrapped: bool,
        start: usize,
        end: usize,
    ) -> NodeId {
        let span = match (left, right) {
            (Some(left), Some(right)) => Span {
                start: self.arena[left].span.start,
                end: self.arena[right].span.end,
            },
            (Some(child), None) | (None, Some(child)) => Span {
                start,
                end: self.arena[child].span.end,
            },
            _ => Span { start, end },
        };
        self.arena.push(
            Expr::Duration {
                operator,
                left,
                right,
                wrapped,
                start,
                end,
            },
            span,
        )
    }

    fn wrap_duration(&mut self, id: NodeId, start: usize, end: usize) -> NodeId {
        if let Expr::Duration { wrapped, .. } = &mut self.arena[id].expression {
            *wrapped = true;
            id
        } else {
            self.duration_node(ADD, None, Some(id), true, start, end)
        }
    }

    fn unary(&mut self, operator: Item<'a>, mut id: NodeId, mode: Mode, wrapped: bool) -> NodeId {
        if wrapped {
            let span = self.arena[id].span;
            id = self.wrap_duration(id, span.start, span.end);
        }
        if let Expr::Number { value, .. } = &mut self.arena[id].expression {
            if operator.kind == SUB {
                *value *= -1.0;
            }
            if mode != Mode::Normal && number::duration_out_of_range(*value) {
                self.error(operator.span, "duration out of range");
                return self.zero();
            }
            self.arena[id].span.start = operator.span.start;
            return id;
        }
        if mode == Mode::Normal {
            let span = Span {
                start: operator.span.start,
                end: self.arena[id].span.end,
            };
            return self.arena.push(
                Expr::Unary {
                    operator: operator.kind,
                    expression: id,
                },
                span,
            );
        }
        if mode == Mode::Offset && !wrapped {
            let end = match self.arena[id].expression {
                Expr::Duration {
                    operator: MIN_OF | MAX_OF,
                    end,
                    ..
                } => end,
                _ => 0,
            };
            let result = self.duration_node(
                operator.kind,
                None,
                Some(id),
                false,
                operator.span.start,
                end,
            );
            self.experimental_duration(result);
            return result;
        }
        if operator.kind == SUB {
            self.duration_node(SUB, None, Some(id), false, operator.span.start, 0)
        } else {
            id
        }
    }

    fn grouping(&mut self) -> Option<Vec<Vec<u8>>> {
        self.expect(LEFT_PAREN, "grouping opts", "\"(\"")?;
        let mut labels = Vec::new();
        if self.accept(RIGHT_PAREN).is_some() {
            return Some(labels);
        }
        loop {
            let item = self.peek();
            let mut synthetic = false;
            let value = if item.kind == STRING {
                self.take();
                self.unquote(&item)
            } else if item.kind.is_grouping_label() {
                self.take();
                item.value.to_vec()
            } else {
                self.unexpected("grouping opts", "label");
                if !self.recover_to(&[COMMA, RIGHT_PAREN]) {
                    return Some(labels);
                }
                synthetic = true;
                Vec::new()
            };
            if !synthetic && (value.is_empty() || std::str::from_utf8(&value).is_err()) {
                self.error(
                    item.span,
                    format!("invalid label name for grouping: {}", quote::bytes(&value)),
                );
            }
            labels.push(value);
            if self.accept(COMMA).is_some() {
                if self.accept(RIGHT_PAREN).is_some() {
                    return Some(labels);
                }
            } else if self.accept(RIGHT_PAREN).is_some() {
                return Some(labels);
            } else {
                self.unexpected("grouping opts", "\",\" or \")\"");
                return Some(labels);
            }
        }
    }

    fn aggregate_grouping(&mut self) -> (Option<Vec<Vec<u8>>>, bool) {
        let item = self.peek();
        if matches!(item.kind, BY | WITHOUT) {
            self.take();
            (self.grouping(), item.kind == WITHOUT)
        } else {
            (None, false)
        }
    }

    fn recover_to(&mut self, kinds: &[TokenKind]) -> bool {
        loop {
            let kind = self.peek().kind;
            if kinds.contains(&kind) {
                return true;
            }
            if matches!(kind, EOF | ERROR) {
                return false;
            }
            self.lookahead.take();
        }
    }

    fn matcher(&mut self) -> Option<Matcher> {
        let label = self.peek();
        if !matches!(label.kind, IDENTIFIER | STRING) {
            self.unexpected("label matching", "identifier or \"}\"");
            return None;
        }
        self.take();
        let name = if label.kind == STRING {
            self.unquote(&label)
        } else {
            label.value.to_vec()
        };
        let operator = self.peek();
        if label.kind == STRING && !matches!(operator.kind, EQL | NEQ | EQL_REGEX | NEQ_REGEX) {
            return Some(Matcher {
                kind: MatchType::Equal,
                name: b"__name__".to_vec(),
                value: name,
                compiled: None,
            });
        }
        let kind = match operator.kind {
            EQL => MatchType::Equal,
            NEQ => MatchType::NotEqual,
            EQL_REGEX => MatchType::Regex,
            NEQ_REGEX => MatchType::NotRegex,
            _ => {
                self.unexpected("label matching", "label matching operator");
                return None;
            }
        };
        self.take();
        let item = self.expect(STRING, "label matching", "string")?;
        let value = self.unquote(&item);
        let compiled = if matches!(kind, MatchType::Regex | MatchType::NotRegex) {
            match (self.compile)(&value) {
                Ok(compiled) => Some(compiled),
                Err(error) => {
                    let start = label.span.start + usize::from(label.kind == STRING);
                    self.error(
                        Span {
                            start,
                            end: item.span.end,
                        },
                        error,
                    );
                    return None;
                }
            }
        } else {
            None
        };
        Some(Matcher {
            kind,
            name,
            value,
            compiled,
        })
    }

    fn matchers(&mut self) -> Option<(Vec<Option<Matcher>>, usize)> {
        self.take();
        let mut matchers = Vec::new();
        if let Some(close) = self.accept(RIGHT_BRACE) {
            return Some((matchers, close.span.end));
        }
        loop {
            let matcher = self.matcher();
            let recovered = matcher.is_none();
            matchers.push(matcher);
            if recovered && !self.recover_to(&[COMMA, RIGHT_BRACE]) {
                return None;
            }
            if self.accept(COMMA).is_some() {
                if let Some(close) = self.accept(RIGHT_BRACE) {
                    return Some((matchers, close.span.end));
                }
            } else if let Some(close) = self.accept(RIGHT_BRACE) {
                return Some((matchers, close.span.end));
            } else {
                self.unexpected("label matching", "\",\" or \"}\"");
                if !self.recover_to(&[COMMA, RIGHT_BRACE]) {
                    return None;
                }
                if let Some(close) = self.accept(RIGHT_BRACE) {
                    return Some((matchers, close.span.end));
                }
                self.take();
            }
        }
    }

    fn selector(&mut self, item: Item<'a>) -> Option<NodeId> {
        let name = if item.kind == LEFT_BRACE {
            Vec::new()
        } else {
            item.value.to_vec()
        };
        let mut span = item.span;
        let mut matchers = if item.kind == LEFT_BRACE || self.peek().kind == LEFT_BRACE {
            let (matchers, end) = self.matchers()?;
            span.end = end;
            matchers
        } else {
            Vec::new()
        };
        if !name.is_empty() {
            matchers.push(Some(Matcher {
                kind: MatchType::Equal,
                name: b"__name__".to_vec(),
                value: name.clone(),
                compiled: None,
            }));
        }
        Some(self.arena.push(
            Expr::Selector(VectorSelector {
                name,
                matchers,
                ..VectorSelector::default()
            }),
            span,
        ))
    }

    fn fill_value(&mut self) -> Option<f64> {
        self.expect(LEFT_PAREN, "", "")?;
        let sign = if matches!(self.peek().kind, ADD | SUB) {
            Some(self.take().kind)
        } else {
            None
        };
        let item = self.peek();
        if !matches!(item.kind, NUMBER | DURATION) {
            self.unexpected("", "");
            return None;
        }
        self.take();
        let id = self.literal(item);
        self.expect(RIGHT_PAREN, "", "")?;
        let Expr::Number { mut value, .. } = self.arena[id].expression else {
            unreachable!()
        };
        if sign == Some(SUB) {
            value *= -1.0;
        }
        Some(value)
    }

    fn binary_matching(&mut self) -> Option<(VectorMatching, bool)> {
        let return_bool = self.accept(BOOL).is_some();
        let mut matching = VectorMatching::default();
        let item = self.peek();
        if matches!(item.kind, ON | IGNORING) {
            self.take();
            matching.on = item.kind == ON;
            matching.labels = self.grouping();
            let item = self.peek();
            if matches!(item.kind, GROUP_LEFT | GROUP_RIGHT) {
                self.take();
                matching.cardinality = if item.kind == GROUP_LEFT {
                    Cardinality::ManyToOne
                } else {
                    Cardinality::OneToMany
                };
                if self.peek().kind == LEFT_PAREN {
                    matching.include = self.grouping();
                }
            }
        }
        let first = self.peek().kind;
        if matches!(first, FILL | FILL_LEFT | FILL_RIGHT) {
            self.take();
            let value = self.fill_value()?;
            if first != FILL_RIGHT {
                matching.fill_left = Some(value);
            }
            if first != FILL_LEFT {
                matching.fill_right = Some(value);
            }
            let second = self.peek().kind;
            if (first == FILL_LEFT && second == FILL_RIGHT)
                || (first == FILL_RIGHT && second == FILL_LEFT)
            {
                self.take();
                let value = self.fill_value()?;
                if second == FILL_LEFT {
                    matching.fill_left = Some(value);
                } else {
                    matching.fill_right = Some(value);
                }
            }
        }
        Some((matching, return_bool))
    }

    fn finish_call(&mut self, callee: Callee<'a>, arguments: Vec<NodeId>, close: usize) -> NodeId {
        match callee {
            Callee::Function(item) => {
                let function = function(&item.value);
                if function.is_none() {
                    self.error(
                        item.span,
                        format!("unknown function with name {}", quote::bytes(&item.value)),
                    );
                } else if function.is_some_and(|function| function.experimental)
                    && !self.options.experimental_functions
                {
                    self.error(
                        item.span,
                        format!("function {} is not enabled", quote::bytes(&item.value)),
                    );
                }
                self.arena.push(
                    Expr::Call {
                        function,
                        arguments,
                    },
                    Span {
                        start: item.span.start,
                        end: close,
                    },
                )
            }
            Callee::Aggregate {
                item,
                mut grouping,
                mut without,
            } => {
                let mut end = close;
                if grouping.is_none() && matches!(self.peek().kind, BY | WITHOUT) {
                    (grouping, without) = self.aggregate_grouping();
                    end = self.last_closing;
                }
                let span = Span {
                    start: item.span.start,
                    end,
                };
                let mut parameter = None;
                let mut expression = None;
                if arguments.is_empty() {
                    self.error(span, "no arguments for aggregate expression provided");
                } else if item.kind.is_experimental_aggregator()
                    && !self.options.experimental_functions
                {
                    self.error(span, format!("{}() is experimental and must be enabled with --enable-feature=promql-experimental-functions", item.kind.text().unwrap()));
                } else {
                    let count = if item.kind.takes_parameter() {
                        parameter = Some(arguments[0]);
                        2
                    } else {
                        1
                    };
                    if arguments.len() != count {
                        self.error(span, format!("wrong number of arguments for aggregate expression provided, expected {count}, got {}", arguments.len()));
                    } else {
                        expression = Some(arguments[count - 1]);
                    }
                }
                self.arena.push(
                    Expr::Aggregate {
                        operator: item.kind,
                        expression,
                        parameter,
                        grouping,
                        without,
                    },
                    span,
                )
            }
            Callee::Duration(item, mode) => {
                let (left, right) = if matches!(item.kind, MAX_OF | MIN_OF) {
                    if arguments.len() != 2 {
                        self.unexpected("", "");
                        return self.zero();
                    }
                    (Some(arguments[0]), Some(arguments[1]))
                } else {
                    if !arguments.is_empty() {
                        self.unexpected("", "");
                        return self.zero();
                    }
                    (None, None)
                };
                let end = if mode == Mode::OffsetUnary && matches!(item.kind, MAX_OF | MIN_OF) {
                    self.arena[right.unwrap()].span.end
                } else {
                    close
                };
                let id = self.duration_node(item.kind, left, right, false, item.span.start, end);
                if mode != Mode::OffsetUnary {
                    self.experimental_duration(id);
                }
                id
            }
        }
    }

    fn modifiers_mut(&mut self, id: NodeId) -> &mut SelectorModifiers {
        match &mut self.arena[id].expression {
            Expr::Selector(selector) => &mut selector.modifiers,
            Expr::Subquery { modifiers, .. } => modifiers,
            _ => unreachable!("selector modifier owner"),
        }
    }

    fn modifier_owner(&mut self, id: NodeId, modifier: &str) -> Option<NodeId> {
        match &self.arena[id].expression {
            Expr::Selector(_) | Expr::Subquery { .. } => Some(id),
            Expr::Matrix { selector, .. } => {
                let selector = *selector;
                if matches!(self.arena[selector].expression, Expr::Selector(_)) {
                    Some(selector)
                } else {
                    self.error(
                        self.arena[id].span,
                        "ranges only allowed for vector selectors",
                    );
                    None
                }
            }
            _ => {
                self.error(self.arena[id].span, format!("{modifier} modifier must be preceded by an instant vector selector or range vector selector or a subquery"));
                None
            }
        }
    }

    fn add_offset(&mut self, id: NodeId, offset: NodeId) {
        let Some(owner) = self.modifier_owner(id, "offset") else {
            return;
        };
        let (value, expression) = match self.arena[offset].expression {
            Expr::Number { value, .. } => (number::nanoseconds(value), None),
            Expr::Duration { .. } => (0, Some(offset)),
            _ => unreachable!("duration offset"),
        };
        let existing = self.modifiers_mut(owner);
        if existing.original_offset != 0 || existing.original_offset_expression.is_some() {
            self.error(self.arena[id].span, "offset may not be set multiple times");
        } else {
            existing.original_offset = value;
            existing.original_offset_expression = expression;
        }
        self.arena[id].span.end = self.last_closing;
    }

    fn at(&mut self, id: NodeId) {
        let first = self.peek();
        let (timestamp, preprocessor) = if matches!(first.kind, START | END) {
            self.take();
            if self.expect(LEFT_PAREN, "@", "timestamp").is_none() {
                return;
            }
            if self.expect(RIGHT_PAREN, "@", "timestamp").is_none() {
                return;
            }
            (None, Some(first.kind))
        } else {
            let sign = if matches!(first.kind, ADD | SUB) {
                Some(self.take().kind)
            } else {
                None
            };
            let item = self.peek();
            if !matches!(item.kind, NUMBER | DURATION) {
                self.unexpected("@", "timestamp");
                return;
            }
            self.take();
            let number = self.literal(item);
            let Expr::Number { mut value, .. } = self.arena[number].expression else {
                unreachable!()
            };
            if sign == Some(SUB) {
                value *= -1.0;
            }
            if !value.is_finite() || value >= i64::MAX as f64 || value <= i64::MIN as f64 {
                let text = if value.is_nan() {
                    "NaN".into()
                } else if value == f64::INFINITY {
                    "+Inf".into()
                } else if value == f64::NEG_INFINITY {
                    "-Inf".into()
                } else {
                    format!("{value:.6}")
                };
                self.error(
                    self.arena[id].span,
                    format!("timestamp out of bounds for @ modifier: {text}"),
                );
            }
            (Some(number::go_i64((value * 1000.0).round())), None)
        };
        let Some(owner) = self.modifier_owner(id, "@") else {
            return;
        };
        let existing = self.modifiers_mut(owner);
        if existing.timestamp.is_some() || matches!(existing.start_or_end, Some(START | END)) {
            self.error(
                self.arena[id].span,
                "@ <timestamp> may not be set multiple times",
            );
            return;
        }
        existing.timestamp = timestamp;
        existing.start_or_end = preprocessor;
        self.arena[id].span.end = self.last_closing;
    }

    fn extended(&mut self, id: NodeId, operator: TokenKind) {
        let name = operator.text().unwrap();
        if !self.options.extended_range_selectors {
            self.error(
                self.arena[id].span,
                format!("{name} modifier is experimental and not enabled"),
            );
            return;
        }
        let owner = match &self.arena[id].expression {
            Expr::Selector(_) => id,
            Expr::Matrix { selector, .. } => *selector,
            Expr::Subquery { .. } => {
                self.error(
                    self.arena[id].span,
                    format!("{name} modifier is not supported for subqueries"),
                );
                return;
            }
            _ => {
                self.error(
                    self.arena[id].span,
                    format!("{name} modifier not implemented"),
                );
                return;
            }
        };
        if let Expr::Selector(selector) = &mut self.arena[owner].expression {
            if operator == ANCHORED {
                selector.anchored = true;
            } else {
                selector.smoothed = true;
            }
            if selector.anchored && selector.smoothed {
                self.error(
                    self.arena[id].span,
                    "anchored and smoothed modifiers cannot be used together",
                );
            }
        }
    }

    fn finish_range(
        &mut self,
        left: NodeId,
        opening: Item<'a>,
        range: NodeId,
        close: Item<'a>,
    ) -> NodeId {
        let message = match &self.arena[left].expression {
            Expr::Selector(selector)
                if selector.modifiers.original_offset != 0
                    || selector.modifiers.original_offset_expression.is_some() =>
            {
                Some("no offset modifiers allowed before range")
            }
            Expr::Selector(selector)
                if selector.modifiers.timestamp.is_some()
                    || selector.modifiers.start_or_end.is_some() =>
            {
                Some("no @ modifiers allowed before range")
            }
            Expr::Selector(_) => None,
            _ => Some("ranges only allowed for vector selectors"),
        };
        if let Some(message) = message {
            self.error(
                Span {
                    start: opening.span.start,
                    end: close.span.end,
                },
                message,
            );
        }
        let (range_value, expression) = self.duration_value(range);
        self.arena.push(
            Expr::Matrix {
                selector: left,
                range: range_value,
                range_expression: expression,
            },
            Span {
                start: self.arena[left].span.start,
                end: close.span.end,
            },
        )
    }

    fn duration_value(&self, id: NodeId) -> (i64, Option<NodeId>) {
        match self.arena[id].expression {
            Expr::Number { value, .. } => (number::nanoseconds(value), None),
            Expr::Duration { .. } => (0, Some(id)),
            _ => unreachable!("duration expression"),
        }
    }

    fn subquery(
        &mut self,
        left: NodeId,
        range: NodeId,
        step: Option<NodeId>,
        end: usize,
    ) -> NodeId {
        let (range, range_expression) = self.duration_value(range);
        let (step, step_expression) = step.map_or((0, None), |id| self.duration_value(id));
        self.arena.push(
            Expr::Subquery {
                expression: left,
                range,
                range_expression,
                step,
                step_expression,
                modifiers: SelectorModifiers::default(),
            },
            Span {
                start: self.arena[left].span.start,
                end,
            },
        )
    }

    fn recover_prefix(
        &mut self,
        frames: &mut Vec<Frame<'a>>,
        values: &mut Vec<NodeId>,
        diagnosed: bool,
    ) -> bool {
        let recovery = frames.iter().rposition(|frame| {
            matches!(
                frame,
                Frame::Range { .. }
                    | Frame::RangeStep { .. }
                    | Frame::Offset(_)
                    | Frame::Arguments {
                        callee: Callee::Aggregate { .. },
                        ..
                    }
            )
        });
        if !diagnosed {
            match recovery.map(|index| &frames[index]) {
                Some(Frame::Range { .. }) => self.unexpected(
                    "subquery or range selector",
                    "number, duration, step(), or range()",
                ),
                Some(Frame::RangeStep { .. }) => self.unexpected(
                    "subquery selector",
                    "number, duration, step(), range(), or \"]\"",
                ),
                Some(Frame::Offset(_)) => {
                    self.unexpected("offset", "number, duration, step(), or range()")
                }
                _ => self.unexpected("", ""),
            }
        }
        if self.peek().kind == ERROR
            && let Some(comma) = frames.iter().rev().find_map(|frame| {
                if let Frame::Arguments { last_comma, .. } = frame {
                    *last_comma
                } else {
                    None
                }
            })
        {
            self.error(comma, "trailing commas not allowed in function call args");
        }
        let Some(index) = recovery else {
            return false;
        };
        frames.truncate(index + 1);
        match frames.pop().unwrap() {
            Frame::Range { left, .. } | Frame::RangeStep { left, .. } => {
                if self.recover_to(&[RIGHT_BRACKET, EOF, ERROR]) {
                    self.accept(RIGHT_BRACKET);
                }
                values.push(left);
            }
            Frame::Offset(left) => values.push(left),
            Frame::Arguments { callee, .. } => {
                values.push(self.finish_call(callee, Vec::new(), self.last_closing));
            }
            _ => unreachable!(),
        }
        true
    }

    fn run(mut self) -> ParseOutput {
        let mut frames = vec![Frame::Parse(0, Mode::Normal)];
        let mut values = Vec::new();
        if self.peek().kind == EOF {
            self.error(Span::default(), "no expression found in input");
            frames.clear();
        }
        let mut failed = false;
        while let Some(frame) = frames.pop() {
            match frame {
                Frame::Parse(precedence, mode) => {
                    frames.push(Frame::Rest(precedence, mode));
                    frames.push(Frame::Prefix(mode));
                }
                Frame::Prefix(mode) => {
                    let item = self.peek();
                    match item.kind {
                        NUMBER | DURATION => {
                            self.take();
                            let id = self.literal(item);
                            values.push(if mode == Mode::Duration || mode == Mode::Offset {
                                self.check_duration(id)
                            } else {
                                id
                            });
                        }
                        STRING if mode == Mode::Normal => {
                            self.take();
                            let value = self.unquote(&item);
                            values.push(self.arena.push(Expr::String(value), item.span));
                        }
                        ADD | SUB => {
                            self.take();
                            let wrapped = mode == Mode::Offset && self.peek().kind == LEFT_PAREN;
                            frames.push(Frame::Unary(item, mode, wrapped));
                            frames.push(Frame::Parse(
                                6,
                                if mode == Mode::Offset {
                                    Mode::OffsetUnary
                                } else {
                                    mode
                                },
                            ));
                        }
                        LEFT_PAREN => {
                            self.take();
                            frames.push(Frame::Paren(item, mode));
                            frames.push(Frame::Parse(
                                0,
                                if mode.is_offset() {
                                    Mode::Duration
                                } else {
                                    mode
                                },
                            ));
                        }
                        LEFT_BRACE if mode == Mode::Normal => {
                            if let Some(id) = self.selector(item) {
                                values.push(id);
                            } else {
                                failed = true;
                                break;
                            }
                        }
                        _ if mode == Mode::Normal && item.kind.is_metric_identifier() => {
                            self.take();
                            let next = self.peek().kind;
                            let callee = if item.kind.is_aggregator()
                                && matches!(next, BY | WITHOUT | LEFT_PAREN)
                            {
                                let (grouping, without) = self.aggregate_grouping();
                                if self.peek().kind == ERROR {
                                    failed = true;
                                    break;
                                }
                                Some(Callee::Aggregate {
                                    item: item.clone(),
                                    grouping,
                                    without,
                                })
                            } else if item.kind.is_function_identifier() && next == LEFT_PAREN {
                                Some(Callee::Function(item.clone()))
                            } else {
                                None
                            };
                            if let Some(callee) = callee {
                                if self.expect(LEFT_PAREN, "aggregation", "").is_none() {
                                    if matches!(callee, Callee::Aggregate { .. }) {
                                        values.push(self.finish_call(
                                            callee,
                                            Vec::new(),
                                            self.last_closing,
                                        ));
                                    } else {
                                        failed = true;
                                        break;
                                    }
                                } else if let Some(close) = self.accept(RIGHT_PAREN) {
                                    values.push(self.finish_call(
                                        callee,
                                        Vec::new(),
                                        close.span.end,
                                    ));
                                } else {
                                    frames.push(Frame::Arguments {
                                        callee,
                                        arguments: Vec::new(),
                                        mode,
                                        last_comma: None,
                                    });
                                    frames.push(Frame::Parse(0, mode));
                                }
                            } else if let Some(id) = self.selector(item) {
                                values.push(id);
                            } else {
                                failed = true;
                                break;
                            }
                        }
                        STEP | RANGE | MAX_OF | MIN_OF if mode != Mode::Normal => {
                            self.take();
                            if self.accept(LEFT_PAREN).is_none() {
                                if !self.recover_prefix(&mut frames, &mut values, false) {
                                    failed = true;
                                    break;
                                }
                                continue;
                            }
                            let callee = Callee::Duration(item, mode);
                            if let Some(close) = self.accept(RIGHT_PAREN) {
                                values.push(self.finish_call(callee, Vec::new(), close.span.end));
                            } else {
                                frames.push(Frame::Arguments {
                                    callee,
                                    arguments: Vec::new(),
                                    mode: Mode::Duration,
                                    last_comma: None,
                                });
                                frames.push(Frame::Parse(0, Mode::Duration));
                            }
                        }
                        _ => {
                            if !self.recover_prefix(&mut frames, &mut values, false) {
                                failed = true;
                                break;
                            }
                        }
                    }
                }
                Frame::Rest(precedence, mode) => {
                    let left = values.pop().unwrap();
                    if mode.is_offset() {
                        values.push(left);
                        continue;
                    }
                    let item = self.peek();
                    if mode == Mode::Normal && item.kind == LEFT_BRACKET {
                        self.take();
                        frames.push(Frame::Rest(precedence, mode));
                        frames.push(Frame::Range {
                            left,
                            opening: item,
                        });
                        frames.push(Frame::Parse(0, Mode::Duration));
                    } else if mode == Mode::Normal && item.kind == OFFSET {
                        self.take();
                        frames.push(Frame::Rest(precedence, mode));
                        frames.push(Frame::Offset(left));
                        frames.push(Frame::Parse(0, Mode::Offset));
                    } else if mode == Mode::Normal && item.kind == AT {
                        self.take();
                        self.at(left);
                        values.push(left);
                        frames.push(Frame::Rest(precedence, mode));
                    } else if mode == Mode::Normal && matches!(item.kind, ANCHORED | SMOOTHED) {
                        self.take();
                        self.extended(left, item.kind);
                        values.push(left);
                        frames.push(Frame::Rest(precedence, mode));
                    } else if let Some(binding) = item.kind.precedence()
                        && binding >= precedence
                        && !mode.is_offset()
                        && (mode == Mode::Normal
                            || matches!(item.kind, ADD | SUB | MUL | DIV | MOD | POW))
                    {
                        self.take();
                        let (matching, return_bool) = if mode == Mode::Normal {
                            let Some(modifiers) = self.binary_matching() else {
                                failed = true;
                                break;
                            };
                            modifiers
                        } else {
                            (VectorMatching::default(), false)
                        };
                        frames.push(Frame::Rest(precedence, mode));
                        frames.push(Frame::Binary {
                            left,
                            operator: item.clone(),
                            matching,
                            return_bool,
                            mode,
                        });
                        frames.push(Frame::Parse(binding + u8::from(item.kind != POW), mode));
                    } else {
                        values.push(left);
                    }
                }
                Frame::Unary(operator, mode, wrapped) => {
                    let child = values.pop().unwrap();
                    values.push(self.unary(operator, child, mode, wrapped));
                }
                Frame::Paren(open, mode) => {
                    let child = values.pop().unwrap();
                    let Some(close) = self.expect(RIGHT_PAREN, "", "") else {
                        failed = true;
                        break;
                    };
                    if mode == Mode::Normal {
                        values.push(self.arena.push(
                            Expr::Paren(child),
                            Span {
                                start: open.span.start,
                                end: close.span.end,
                            },
                        ));
                    } else if mode == Mode::OffsetUnary {
                        values.push(child);
                    } else {
                        self.experimental_duration(child);
                        values.push(self.wrap_duration(child, open.span.start, close.span.end));
                    }
                }
                Frame::Binary {
                    left,
                    operator,
                    matching,
                    return_bool,
                    mode,
                } => {
                    let right = values.pop().unwrap();
                    let id = if mode != Mode::Normal {
                        self.experimental_duration(left);
                        if matches!(operator.kind, DIV | MOD)
                            && matches!(
                                self.arena[right].expression,
                                Expr::Number { value: 0.0, .. }
                            )
                        {
                            self.error(
                                operator.span,
                                if operator.kind == DIV {
                                    "division by zero"
                                } else {
                                    "modulo by zero"
                                },
                            );
                            self.zero()
                        } else {
                            self.duration_node(operator.kind, Some(left), Some(right), false, 0, 0)
                        }
                    } else {
                        let span = Span {
                            start: self.arena[left].span.start,
                            end: self.arena[right].span.end,
                        };
                        if !self.options.binop_fill_modifiers
                            && (matching.fill_left.is_some() || matching.fill_right.is_some())
                        {
                            self.error(
                                span,
                                "binop fill modifiers are experimental and not enabled",
                            );
                        }
                        self.arena.push(
                            Expr::Binary {
                                operator: operator.kind,
                                left,
                                right,
                                matching: Some(matching),
                                return_bool,
                            },
                            span,
                        )
                    };
                    values.push(id);
                }
                Frame::Arguments {
                    callee,
                    mut arguments,
                    mode,
                    last_comma: _,
                } => {
                    arguments.push(values.pop().unwrap());
                    if let Some(comma) = self.accept(COMMA) {
                        if let Some(close) = self.accept(RIGHT_PAREN) {
                            self.error(
                                comma.span,
                                "trailing commas not allowed in function call args",
                            );
                            values.push(self.finish_call(callee, arguments, close.span.end));
                        } else {
                            frames.push(Frame::Arguments {
                                callee,
                                arguments,
                                mode,
                                last_comma: Some(comma.span),
                            });
                            frames.push(Frame::Parse(0, mode));
                        }
                    } else if let Some(close) = self.accept(RIGHT_PAREN) {
                        values.push(self.finish_call(callee, arguments, close.span.end));
                    } else {
                        self.unexpected("", "");
                        failed = true;
                        break;
                    }
                }
                Frame::Range { left, opening } => {
                    let range = self.positive(values.pop().unwrap());
                    if self.accept(COLON).is_some() {
                        if let Some(close) = self.accept(RIGHT_BRACKET) {
                            values.push(self.subquery(left, range, None, close.span.end));
                        } else {
                            frames.push(Frame::RangeStep { left, range });
                            frames.push(Frame::Parse(0, Mode::Duration));
                        }
                    } else if let Some(close) = self.accept(RIGHT_BRACKET) {
                        values.push(self.finish_range(left, opening, range, close));
                    } else {
                        self.unexpected("subquery or range", "\":\" or \"]\"");
                        values.push(left);
                    }
                }
                Frame::RangeStep { left, range } => {
                    let step = self.positive(values.pop().unwrap());
                    if let Some(close) = self.expect(RIGHT_BRACKET, "subquery selector", "\"]\"") {
                        values.push(self.subquery(left, range, Some(step), close.span.end));
                    } else {
                        values.push(left);
                    }
                }
                Frame::Offset(left) => {
                    let offset = values.pop().unwrap();
                    self.add_offset(left, offset);
                    values.push(left);
                }
            }
        }
        let root = if failed { None } else { values.pop() };
        if root.is_some() && !matches!(self.peek().kind, EOF | ERROR) {
            self.unexpected("", "");
        }
        ParseOutput {
            arena: self.arena,
            root,
            errors: self.errors,
        }
    }
}

fn description(item: &Item<'_>) -> String {
    if item.kind == EOF {
        return "end of input".into();
    }
    if item.kind.text().is_some() {
        return if item.kind.is_keyword() {
            format!("<{}>", String::from_utf8_lossy(&item.value))
        } else if item.kind.is_operator() {
            format!("<op:{}>", String::from_utf8_lossy(&item.value))
        } else if item.kind.is_aggregator() {
            format!("<aggr:{}>", String::from_utf8_lossy(&item.value))
        } else {
            quote::bytes(&item.value)
        };
    }
    let kind = match item.kind {
        IDENTIFIER => "identifier",
        METRIC_IDENTIFIER => "metric identifier",
        STRING => "string",
        NUMBER => "number",
        DURATION => "duration",
        ERROR => "error",
        COMMENT => "comment",
        _ => "item",
    };
    let value = if item.value.len() > 10 && !matches!(item.kind, IDENTIFIER | METRIC_IDENTIFIER) {
        let mut position = 0;
        for _ in 0..10 {
            if position == item.value.len() {
                break;
            }
            position += quote::decode_rune(&item.value[position..]).1;
        }
        format!("{}...", quote::bytes(&item.value[..position]))
    } else {
        quote::bytes(&item.value)
    };
    format!("{kind} {value}")
}
