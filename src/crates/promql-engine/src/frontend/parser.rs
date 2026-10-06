// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus generated_parser.y and parse.go construction semantics; see NOTICE.
use crate::{
    Strong,
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    storage::{Control, Interrupted},
};
use std::fmt;

use super::{
    Span,
    ast::{
        Arena, Cardinality, CompiledMatcher, Expr, MatchType, Matcher, NodeId, ParseError,
        ParserOptions, SelectorModifiers, VectorMatching, VectorSelector,
    },
    bytes::Bytes,
    functions::function,
    lexer::{Item, Lexer},
    literal, number, quote,
    token::{TokenKind, TokenKind::*},
    work::Work,
};

type Grouping = Option<AccountedVec<AccountedVec<u8>>>;
type Matchers = AccountedVec<Option<Matcher>>;

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
        grouping: Grouping,
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
        arguments: AccountedVec<NodeId>,
        mode: Mode,
    },
    DurationFirst(Callee<'a>),
    DurationSecond(Callee<'a>, NodeId),
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
    pub errors: AccountedVec<ParseError>,
}

pub fn parse<C, E>(input: &[u8], options: ParserOptions, compile: C) -> ParseOutput
where
    C: FnMut(&[u8]) -> Result<Strong<dyn CompiledMatcher>, E>,
    E: Into<crate::accounting::AccountedVec<u8>>,
{
    parse_in(input, options, compile, None, None).expect("inert parser")
}

pub(crate) fn parse_in<C, E>(
    input: &[u8],
    options: ParserOptions,
    compile: C,
    accounting: Option<&QueryAccounting>,
    control: Option<&Control>,
) -> Result<ParseOutput, Interrupted>
where
    C: FnMut(&[u8]) -> Result<Strong<dyn CompiledMatcher>, E>,
    E: Into<crate::accounting::AccountedVec<u8>>,
{
    let context = accounting.map(|account| AllocationContext::new(account, MemoryKind::Compiled));
    let work = Work {
        context: context.as_ref(),
        account: accounting,
        control,
    };
    let mut output = parse_syntax_in(input, options, compile, work)?;
    if let Some(root) = output.root {
        super::validate::validate_in(input, &mut output.arena, root, &mut output.errors, work)?;
    }
    Ok(output)
}

/// Regex compilation happens at grammar reductions, before ordered type validation.
pub fn parse_syntax<C, E>(input: &[u8], options: ParserOptions, compile: C) -> ParseOutput
where
    C: FnMut(&[u8]) -> Result<Strong<dyn CompiledMatcher>, E>,
    E: Into<crate::accounting::AccountedVec<u8>>,
{
    parse_syntax_in(input, options, compile, Work::default()).expect("inert syntax parser")
}

fn parse_syntax_in<C, E>(
    input: &[u8],
    options: ParserOptions,
    compile: C,
    work: Work<'_>,
) -> Result<ParseOutput, Interrupted>
where
    C: FnMut(&[u8]) -> Result<Strong<dyn CompiledMatcher>, E>,
    E: Into<AccountedVec<u8>>,
{
    Parser {
        input,
        work,
        lexer: Lexer::new_in(input, false, work),
        lookahead: None,
        terminal: false,
        recovery_shifts: 0,
        last_closing: 0,
        arena: Arena::new_in(work.context),
        errors: work.buffer(),
        options,
        compile,
    }
    .run()
}

struct Parser<'a, 'w, C> {
    work: Work<'w>,
    input: &'a [u8],
    lexer: Lexer<'a, 'w>,
    lookahead: Option<Item<'a>>,
    terminal: bool,
    recovery_shifts: u8,
    last_closing: usize,
    arena: Arena,
    errors: AccountedVec<ParseError>,
    options: ParserOptions,
    compile: C,
}

impl<'a, 'w, C, E> Parser<'a, 'w, C>
where
    C: FnMut(&[u8]) -> Result<Strong<dyn CompiledMatcher>, E>,
    E: Into<crate::accounting::AccountedVec<u8>>,
{
    fn error_owned(&mut self, span: Span, message: AccountedVec<u8>) -> Result<(), Interrupted> {
        self.work.step(0)?;
        self.errors.push(ParseError { span, message });
        Ok(())
    }
    fn error_fmt(&mut self, span: Span, message: fmt::Arguments<'_>) -> Result<(), Interrupted> {
        let message = self.work.format(message)?;
        self.error_owned(span, message)
    }
    fn error(&mut self, span: Span, message: impl AsRef<[u8]>) -> Result<(), Interrupted> {
        self.errors.push(ParseError {
            span,
            message: self.work.copy(message.as_ref())?,
        });
        Ok(())
    }

    fn peek(&mut self) -> Result<Item<'a>, Interrupted> {
        if self.lookahead.is_none() {
            let item = if self.terminal {
                Item {
                    kind: EOF,
                    span: Span {
                        start: self.input.len(),
                        end: self.input.len(),
                    },
                    value: Bytes::Borrowed(b""),
                }
            } else {
                loop {
                    self.work.step(1)?;
                    let Some(item) = self.lexer.next_in()? else {
                        break Item {
                            kind: EOF,
                            span: Span {
                                start: self.input.len(),
                                end: self.input.len(),
                            },
                            value: Bytes::Borrowed(b""),
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
                )?;
                self.terminal = true;
            } else if matches!(
                item.kind,
                RIGHT_BRACE | RIGHT_PAREN | RIGHT_BRACKET | DURATION | NUMBER
            ) {
                self.last_closing = item.span.end;
            }
            self.lookahead = Some(item);
        }
        self.lookahead.as_ref().unwrap().copy_in(self.work)
    }

    fn take(&mut self) -> Result<Item<'a>, Interrupted> {
        self.peek()?;
        let item = self.lookahead.take().unwrap();
        if !matches!(item.kind, EOF | ERROR) {
            self.recovery_shifts = self.recovery_shifts.saturating_sub(1);
        }
        Ok(item)
    }

    fn accept(&mut self, kind: TokenKind) -> Result<Option<Item<'a>>, Interrupted> {
        Ok(if self.peek()?.kind == kind {
            Some(self.take()?)
        } else {
            None
        })
    }

    fn unexpected(&mut self, context: &str, expected: &str) -> Result<(), Interrupted> {
        let item = self.peek()?;
        if item.kind == ERROR {
            return Ok(());
        }
        let mut message = self
            .work
            .format(format_args!("unexpected {}", description(&item)))?;
        if !context.is_empty() {
            let suffix = self.work.format(format_args!(" in {context}"))?;
            self.work.extend(&mut message, &suffix)?;
        }
        if !expected.is_empty() {
            let suffix = self.work.format(format_args!(", expected {expected}"))?;
            self.work.extend(&mut message, &suffix)?;
        }
        self.error_owned(item.span, message)?;
        Ok(())
    }

    fn expect(
        &mut self,
        kind: TokenKind,
        context: &str,
        expected: &str,
    ) -> Result<Option<Item<'a>>, Interrupted> {
        Ok(if let Some(item) = self.accept(kind)? {
            Some(item)
        } else {
            self.unexpected(context, expected)?;
            None
        })
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

    fn literal(&mut self, item: Item<'a>) -> Result<NodeId, Interrupted> {
        let (value, duration) = if item.kind == DURATION {
            let value = match number::duration_in(&item.value, self.work)? {
                Ok(value) => number::seconds(value),
                Err(error) => {
                    self.error_owned(item.span, error)?;
                    0.0
                }
            };
            (value, true)
        } else {
            let number = number::number_in(&item.value, self.work)?;
            if let Some(error) = number.error {
                self.error_owned(item.span, error)?;
            }
            (number.value, false)
        };
        Ok(self.arena.push(Expr::Number { value, duration }, item.span))
    }

    fn unquote(&mut self, item: &Item<'a>) -> Result<AccountedVec<u8>, Interrupted> {
        Ok(match literal::unquote_in(&item.value, self.work)? {
            Ok(value) => value.into_owned(self.work)?,
            Err(error) => {
                self.error_fmt(
                    item.span,
                    format_args!(
                        "error unquoting string {}: {error}",
                        quote::quoted(&item.value)
                    ),
                )?;
                self.work.buffer()
            }
        })
    }

    fn check_duration(&mut self, id: NodeId) -> Result<NodeId, Interrupted> {
        if let Expr::Number { value, .. } = self.arena[id].expression
            && number::duration_out_of_range(value)
        {
            self.error(self.arena[id].span, "duration out of range")?;
            return Ok(self.zero());
        }
        Ok(id)
    }

    fn positive(&mut self, id: NodeId) -> Result<NodeId, Interrupted> {
        if let Expr::Number { value, .. } = self.arena[id].expression
            && value <= 0.0
        {
            self.error(self.arena[id].span, "duration must be greater than 0")?;
            return Ok(self.zero());
        }
        Ok(id)
    }

    fn experimental_duration(&mut self, id: NodeId) -> Result<(), Interrupted> {
        if !self.options.duration_expressions {
            self.error(
                self.arena[id].span,
                "experimental duration expression is not enabled",
            )?;
        }
        Ok(())
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

    fn unary(
        &mut self,
        operator: Item<'a>,
        mut id: NodeId,
        mode: Mode,
        wrapped: bool,
    ) -> Result<NodeId, Interrupted> {
        if wrapped {
            let span = self.arena[id].span;
            id = self.wrap_duration(id, span.start, span.end);
        }
        if let Expr::Number { value, .. } = &mut self.arena[id].expression {
            if operator.kind == SUB {
                *value *= -1.0;
            }
            if mode != Mode::Normal && number::duration_out_of_range(*value) {
                self.error(operator.span, "duration out of range")?;
                return Ok(self.zero());
            }
            self.arena[id].span.start = operator.span.start;
            return Ok(id);
        }
        if mode == Mode::Normal {
            let span = Span {
                start: operator.span.start,
                end: self.arena[id].span.end,
            };
            return Ok(self.arena.push(
                Expr::Unary {
                    operator: operator.kind,
                    expression: id,
                },
                span,
            ));
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
            self.experimental_duration(result)?;
            return Ok(result);
        }
        Ok(if operator.kind == SUB {
            self.duration_node(SUB, None, Some(id), false, operator.span.start, 0)
        } else {
            id
        })
    }

    fn grouping(&mut self) -> Result<Grouping, Interrupted> {
        if self.expect(LEFT_PAREN, "grouping opts", "\"(\"")?.is_none() {
            self.recover_to(&[EOF, ERROR])?;
            return Ok(None);
        }
        let mut labels = self.work.buffer();
        if self.accept(RIGHT_PAREN)?.is_some() {
            return Ok(Some(labels));
        }
        loop {
            self.work.step(1)?;
            let item = self.peek()?;
            let mut synthetic = false;
            let value = if item.kind == STRING {
                self.take()?;
                self.unquote(&item)?
            } else if item.kind.is_grouping_label() {
                self.take()?;
                self.work.copy(&item.value)?
            } else {
                self.unexpected("grouping opts", "label")?;
                if !self.recover_to(&[COMMA, RIGHT_PAREN])? {
                    return Ok(Some(labels));
                }
                synthetic = true;
                self.work.buffer()
            };
            if !synthetic && (value.is_empty() || !self.work.valid_utf8(&value)?) {
                self.error_fmt(
                    item.span,
                    format_args!("invalid label name for grouping: {}", quote::quoted(&value)),
                )?;
            }
            labels.push(value);
            if self.accept(COMMA)?.is_some() {
                if self.accept(RIGHT_PAREN)?.is_some() {
                    return Ok(Some(labels));
                }
            } else if self.accept(RIGHT_PAREN)?.is_some() {
                return Ok(Some(labels));
            } else {
                self.unexpected("grouping opts", "\",\" or \")\"")?;
                if !self.recover_to(&[COMMA, RIGHT_PAREN])? {
                    return Ok(Some(labels));
                }
                if self.accept(RIGHT_PAREN)?.is_some() {
                    return Ok(Some(labels));
                }
                self.take()?;
                if self.accept(RIGHT_PAREN)?.is_some() {
                    return Ok(Some(labels));
                }
            }
        }
    }

    fn aggregate_grouping(&mut self) -> Result<(Grouping, bool), Interrupted> {
        let item = self.peek()?;
        Ok(if matches!(item.kind, BY | WITHOUT) {
            self.take()?;
            (self.grouping()?, item.kind == WITHOUT)
        } else {
            (None, false)
        })
    }

    fn recover_to(&mut self, kinds: &[TokenKind]) -> Result<bool, Interrupted> {
        loop {
            self.work.step(1)?;
            let kind = self.peek()?.kind;
            if kinds.contains(&kind) {
                return Ok(true);
            }
            if matches!(kind, EOF | ERROR) {
                return Ok(false);
            }
            self.lookahead.take();
        }
    }

    fn matcher(&mut self) -> Result<Option<Matcher>, Interrupted> {
        let label = self.peek()?;
        if !matches!(label.kind, IDENTIFIER | STRING) {
            self.unexpected("label matching", "identifier or \"}\"")?;
            return Ok(None);
        }
        self.take()?;
        let name = if label.kind == STRING {
            self.unquote(&label)?
        } else {
            self.work.copy(&label.value)?
        };
        let operator = self.peek()?;
        if label.kind == STRING && !matches!(operator.kind, EQL | NEQ | EQL_REGEX | NEQ_REGEX) {
            return Ok(Some(Matcher {
                kind: MatchType::Equal,
                name: self.work.copy(b"__name__")?,
                value: name,
                compiled: None,
            }));
        }
        let kind = match operator.kind {
            EQL => MatchType::Equal,
            NEQ => MatchType::NotEqual,
            EQL_REGEX => MatchType::Regex,
            NEQ_REGEX => MatchType::NotRegex,
            _ => {
                self.unexpected("label matching", "label matching operator")?;
                return Ok(None);
            }
        };
        self.take()?;
        let Some(item) = self.expect(STRING, "label matching", "string")? else {
            return Ok(None);
        };
        let value = self.unquote(&item)?;
        let compiled = if matches!(kind, MatchType::Regex | MatchType::NotRegex) {
            match (self.compile)(&value) {
                Ok(compiled) => Some(compiled),
                Err(error) => {
                    let start = label.span.start + usize::from(label.kind == STRING);
                    self.errors.push(ParseError {
                        span: Span {
                            start,
                            end: item.span.end,
                        },
                        message: error.into(),
                    });
                    return Ok(None);
                }
            }
        } else {
            None
        };
        Ok(Some(Matcher {
            kind,
            name,
            value,
            compiled,
        }))
    }

    fn matchers(&mut self) -> Result<Option<(Matchers, usize)>, Interrupted> {
        self.take()?;
        let mut matchers = self.work.buffer();
        if let Some(close) = self.accept(RIGHT_BRACE)? {
            return Ok(Some((matchers, close.span.end)));
        }
        loop {
            self.work.step(1)?;
            let matcher = self.matcher()?;
            let recovered = matcher.is_none();
            matchers.push(matcher);
            if recovered && !self.recover_to(&[COMMA, RIGHT_BRACE])? {
                return Ok(None);
            }
            if self.accept(COMMA)?.is_some() {
                if let Some(close) = self.accept(RIGHT_BRACE)? {
                    return Ok(Some((matchers, close.span.end)));
                }
            } else if let Some(close) = self.accept(RIGHT_BRACE)? {
                return Ok(Some((matchers, close.span.end)));
            } else {
                self.unexpected("label matching", "\",\" or \"}\"")?;
                if !self.recover_to(&[COMMA, RIGHT_BRACE])? {
                    return Ok(None);
                }
                if let Some(close) = self.accept(RIGHT_BRACE)? {
                    return Ok(Some((matchers, close.span.end)));
                }
                self.take()?;
            }
        }
    }

    fn selector(&mut self, item: Item<'a>) -> Result<Option<NodeId>, Interrupted> {
        let name = if item.kind == LEFT_BRACE {
            self.work.buffer()
        } else {
            self.work.copy(&item.value)?
        };
        let mut span = item.span;
        let mut matchers = if item.kind == LEFT_BRACE || self.peek()?.kind == LEFT_BRACE {
            let Some((matchers, end)) = self.matchers()? else {
                return Ok(None);
            };
            span.end = end;
            matchers
        } else {
            self.work.buffer()
        };
        if !name.is_empty() {
            matchers.push(Some(Matcher {
                kind: MatchType::Equal,
                name: self.work.copy(b"__name__")?,
                value: self.work.copy(&name)?,
                compiled: None,
            }));
        }
        Ok(Some(self.arena.push(
            Expr::Selector(VectorSelector {
                name,
                matchers,
                ..VectorSelector::default()
            }),
            span,
        )))
    }

    fn fill_value(&mut self) -> Result<Option<f64>, Interrupted> {
        if self.expect(LEFT_PAREN, "", "")?.is_none() {
            return Ok(None);
        }
        let sign = if matches!(self.peek()?.kind, ADD | SUB) {
            Some(self.take()?.kind)
        } else {
            None
        };
        let item = self.peek()?;
        if !matches!(item.kind, NUMBER | DURATION) {
            self.unexpected("", "")?;
            return Ok(None);
        }
        self.take()?;
        let id = self.literal(item)?;
        if self.expect(RIGHT_PAREN, "", "")?.is_none() {
            return Ok(None);
        }
        let Expr::Number { mut value, .. } = self.arena[id].expression else {
            unreachable!()
        };
        if sign == Some(SUB) {
            value *= -1.0;
        }
        Ok(Some(value))
    }

    fn binary_matching(&mut self) -> Result<Option<(VectorMatching, bool)>, Interrupted> {
        let return_bool = self.accept(BOOL)?.is_some();
        let mut matching = VectorMatching::default();
        let item = self.peek()?;
        if matches!(item.kind, ON | IGNORING) {
            self.take()?;
            matching.on = item.kind == ON;
            matching.labels = self.grouping()?;
            let item = self.peek()?;
            if matches!(item.kind, GROUP_LEFT | GROUP_RIGHT) {
                self.take()?;
                matching.cardinality = if item.kind == GROUP_LEFT {
                    Cardinality::ManyToOne
                } else {
                    Cardinality::OneToMany
                };
                if self.peek()?.kind == LEFT_PAREN {
                    matching.include = self.grouping()?;
                }
            }
        }
        let first = self.peek()?.kind;
        if matches!(first, FILL | FILL_LEFT | FILL_RIGHT) {
            self.take()?;
            let Some(value) = self.fill_value()? else {
                return Ok(None);
            };
            if first != FILL_RIGHT {
                matching.fill_left = Some(value);
            }
            if first != FILL_LEFT {
                matching.fill_right = Some(value);
            }
            let second = self.peek()?.kind;
            if (first == FILL_LEFT && second == FILL_RIGHT)
                || (first == FILL_RIGHT && second == FILL_LEFT)
            {
                self.take()?;
                let Some(value) = self.fill_value()? else {
                    return Ok(None);
                };
                if second == FILL_LEFT {
                    matching.fill_left = Some(value);
                } else {
                    matching.fill_right = Some(value);
                }
            }
        }
        Ok(Some((matching, return_bool)))
    }

    fn finish_call(
        &mut self,
        callee: Callee<'a>,
        arguments: AccountedVec<NodeId>,
        close: usize,
    ) -> Result<NodeId, Interrupted> {
        Ok(match callee {
            Callee::Function(item) => {
                let function = function(&item.value);
                if function.is_none() {
                    self.error_fmt(
                        item.span,
                        format_args!("unknown function with name {}", quote::quoted(&item.value)),
                    )?;
                } else if function.is_some_and(|function| function.experimental)
                    && !self.options.experimental_functions
                {
                    self.error_fmt(
                        item.span,
                        format_args!("function {} is not enabled", quote::quoted(&item.value)),
                    )?;
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
                if grouping.is_none() && matches!(self.peek()?.kind, BY | WITHOUT) {
                    (grouping, without) = self.aggregate_grouping()?;
                    end = self.last_closing;
                }
                let span = Span {
                    start: item.span.start,
                    end,
                };
                let mut parameter = None;
                let mut expression = None;
                if arguments.is_empty() {
                    self.error(span, "no arguments for aggregate expression provided")?;
                } else if item.kind.is_experimental_aggregator()
                    && !self.options.experimental_functions
                {
                    self.error_fmt(span, format_args!("{}() is experimental and must be enabled with --enable-feature=promql-experimental-functions", item.kind.text().unwrap()))?;
                } else {
                    let count = if item.kind.takes_parameter() {
                        parameter = Some(arguments[0]);
                        2
                    } else {
                        1
                    };
                    if arguments.len() != count {
                        self.error_fmt(span, format_args!("wrong number of arguments for aggregate expression provided, expected {count}, got {}", arguments.len()))?;
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
                        self.unexpected("", "")?;
                        return Ok(self.zero());
                    }
                    (Some(arguments[0]), Some(arguments[1]))
                } else {
                    if !arguments.is_empty() {
                        self.unexpected("", "")?;
                        return Ok(self.zero());
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
                    self.experimental_duration(id)?;
                }
                id
            }
        })
    }

    fn modifiers_mut(&mut self, id: NodeId) -> &mut SelectorModifiers {
        match &mut self.arena[id].expression {
            Expr::Selector(selector) => &mut selector.modifiers,
            Expr::Subquery { modifiers, .. } => modifiers,
            _ => unreachable!("selector modifier owner"),
        }
    }

    fn modifier_owner(
        &mut self,
        id: NodeId,
        modifier: &str,
    ) -> Result<Option<NodeId>, Interrupted> {
        Ok(match &self.arena[id].expression {
            Expr::Selector(_) | Expr::Subquery { .. } => Some(id),
            Expr::Matrix { selector, .. } => {
                let selector = *selector;
                if matches!(self.arena[selector].expression, Expr::Selector(_)) {
                    Some(selector)
                } else {
                    self.error(
                        self.arena[id].span,
                        "ranges only allowed for vector selectors",
                    )?;
                    None
                }
            }
            _ => {
                self.error_fmt(self.arena[id].span, format_args!("{modifier} modifier must be preceded by an instant vector selector or range vector selector or a subquery"))?;
                None
            }
        })
    }

    fn add_offset(&mut self, id: NodeId, offset: NodeId) -> Result<(), Interrupted> {
        let Some(owner) = self.modifier_owner(id, "offset")? else {
            return Ok(());
        };
        let (value, expression) = match self.arena[offset].expression {
            Expr::Number { value, .. } => (number::nanoseconds(value), None),
            Expr::Duration { .. } => (0, Some(offset)),
            _ => unreachable!("duration offset"),
        };
        let existing = self.modifiers_mut(owner);
        if existing.original_offset != 0 || existing.original_offset_expression.is_some() {
            self.error(self.arena[id].span, "offset may not be set multiple times")?;
        } else {
            existing.original_offset = value;
            existing.original_offset_expression = expression;
        }
        self.arena[id].span.end = self.last_closing;
        Ok(())
    }

    fn at(&mut self, id: NodeId) -> Result<(), Interrupted> {
        let first = self.peek()?;
        let (timestamp, preprocessor) = if matches!(first.kind, START | END) {
            self.take()?;
            if self.expect(LEFT_PAREN, "@", "timestamp")?.is_none() {
                return Ok(());
            }
            if self.expect(RIGHT_PAREN, "@", "timestamp")?.is_none() {
                return Ok(());
            }
            (None, Some(first.kind))
        } else {
            let sign = if matches!(first.kind, ADD | SUB) {
                Some(self.take()?.kind)
            } else {
                None
            };
            let item = self.peek()?;
            if !matches!(item.kind, NUMBER | DURATION) {
                self.unexpected("@", "timestamp")?;
                return Ok(());
            }
            self.take()?;
            let number = self.literal(item)?;
            let Expr::Number { mut value, .. } = self.arena[number].expression else {
                unreachable!()
            };
            if sign == Some(SUB) {
                value *= -1.0;
            }
            if !value.is_finite() || value >= i64::MAX as f64 || value <= i64::MIN as f64 {
                let text = Timestamp(value);
                self.error_fmt(
                    self.arena[id].span,
                    format_args!("timestamp out of bounds for @ modifier: {text}"),
                )?;
            }
            (Some(number::go_i64((value * 1000.0).round())), None)
        };
        let Some(owner) = self.modifier_owner(id, "@")? else {
            return Ok(());
        };
        let existing = self.modifiers_mut(owner);
        if existing.timestamp.is_some() || matches!(existing.start_or_end, Some(START | END)) {
            self.error(
                self.arena[id].span,
                "@ <timestamp> may not be set multiple times",
            )?;
            return Ok(());
        }
        existing.timestamp = timestamp;
        existing.start_or_end = preprocessor;
        self.arena[id].span.end = self.last_closing;
        Ok(())
    }

    fn extended(&mut self, id: NodeId, operator: TokenKind) -> Result<(), Interrupted> {
        let name = operator.text().unwrap();
        if !self.options.extended_range_selectors {
            self.error_fmt(
                self.arena[id].span,
                format_args!("{name} modifier is experimental and not enabled"),
            )?;
            return Ok(());
        }
        let owner = match &self.arena[id].expression {
            Expr::Selector(_) => id,
            Expr::Matrix { selector, .. } => *selector,
            Expr::Subquery { .. } => {
                self.error_fmt(
                    self.arena[id].span,
                    format_args!("{name} modifier is not supported for subqueries"),
                )?;
                return Ok(());
            }
            _ => {
                self.error_fmt(
                    self.arena[id].span,
                    format_args!("{name} modifier not implemented"),
                )?;
                return Ok(());
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
                )?;
            }
        }
        Ok(())
    }

    fn finish_range(
        &mut self,
        left: NodeId,
        opening: Item<'a>,
        range: NodeId,
        close: Item<'a>,
    ) -> Result<NodeId, Interrupted> {
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
            )?;
        }
        let (range_value, expression) = self.duration_value(range);
        Ok(self.arena.push(
            Expr::Matrix {
                selector: left,
                range: range_value,
                range_expression: expression,
            },
            Span {
                start: self.arena[left].span.start,
                end: close.span.end,
            },
        ))
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
        frames: &mut AccountedVec<Frame<'a>>,
        values: &mut AccountedVec<NodeId>,
        diagnosed: bool,
    ) -> Result<bool, Interrupted> {
        let mut recovery = None;
        for (index, frame) in frames.iter().enumerate().rev() {
            self.work.step(1)?;
            if matches!(
                frame,
                Frame::Range { .. }
                    | Frame::RangeStep { .. }
                    | Frame::Offset(_)
                    | Frame::Arguments {
                        callee: Callee::Aggregate { .. },
                        ..
                    }
            ) {
                recovery = Some(index);
                break;
            }
        }
        if !diagnosed {
            match recovery.map(|index| &frames[index]) {
                Some(Frame::Range { .. }) => self.unexpected(
                    "subquery or range selector",
                    "number, duration, step(), or range()",
                )?,
                Some(Frame::RangeStep { .. }) => self.unexpected(
                    "subquery selector",
                    "number, duration, step(), range(), or \"]\"",
                )?,
                Some(Frame::Offset(_)) => {
                    self.unexpected("offset", "number, duration, step(), or range()")?
                }
                Some(Frame::Arguments {
                    callee: Callee::Aggregate { .. },
                    ..
                }) => self.unexpected("aggregation", "")?,
                _ => self.unexpected("", "")?,
            }
        }
        let Some(index) = recovery else {
            return Ok(false);
        };
        self.recovery_shifts = 3;
        frames.truncate(index + 1);
        match frames.pop().unwrap() {
            Frame::Range { left, .. } | Frame::RangeStep { left, .. } => {
                if self.recover_to(&[RIGHT_BRACKET, EOF, ERROR])? {
                    self.accept(RIGHT_BRACKET)?;
                }
                values.push(left);
            }
            Frame::Offset(left) => values.push(left),
            Frame::Arguments { callee, .. } => {
                values.push(self.finish_call(callee, self.work.buffer(), self.last_closing)?);
            }
            _ => unreachable!(),
        }
        Ok(true)
    }

    fn run(mut self) -> Result<ParseOutput, Interrupted> {
        let mut frames = self.work.buffer();
        frames.push(Frame::Parse(0, Mode::Normal));
        let mut values = self.work.buffer();
        if self.peek()?.kind == EOF {
            self.error(Span::default(), "no expression found in input")?;
            frames.clear();
        }
        let mut failed = false;
        while let Some(frame) = frames.pop() {
            self.work.step(1)?;
            match frame {
                Frame::Parse(precedence, mode) => {
                    frames.push(Frame::Rest(precedence, mode));
                    frames.push(Frame::Prefix(mode));
                }
                Frame::Prefix(mode) => {
                    let item = self.peek()?;
                    match item.kind {
                        NUMBER | DURATION => {
                            self.take()?;
                            let id = self.literal(item)?;
                            values.push(if mode == Mode::Duration || mode == Mode::Offset {
                                self.check_duration(id)?
                            } else {
                                id
                            });
                        }
                        STRING if mode == Mode::Normal => {
                            self.take()?;
                            let value = self.unquote(&item)?;
                            values.push(self.arena.push(Expr::String(value), item.span));
                        }
                        ADD | SUB => {
                            self.take()?;
                            let wrapped = mode == Mode::Offset && self.peek()?.kind == LEFT_PAREN;
                            let chained =
                                mode.is_offset() && matches!(self.peek()?.kind, ADD | SUB);
                            if chained {
                                frames.push(Frame::Rest(0, Mode::Duration));
                            }
                            frames.push(Frame::Unary(
                                item,
                                if chained { Mode::Duration } else { mode },
                                wrapped,
                            ));
                            frames.push(Frame::Parse(
                                6,
                                if chained {
                                    Mode::Duration
                                } else if mode == Mode::Offset {
                                    Mode::OffsetUnary
                                } else {
                                    mode
                                },
                            ));
                        }
                        LEFT_PAREN => {
                            self.take()?;
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
                            if let Some(id) = self.selector(item)? {
                                values.push(id);
                            } else {
                                failed = true;
                                break;
                            }
                        }
                        _ if mode == Mode::Normal && item.kind.is_metric_identifier() => {
                            self.take()?;
                            let next = self.peek()?.kind;
                            let callee = if item.kind.is_aggregator()
                                && matches!(next, BY | WITHOUT | LEFT_PAREN)
                            {
                                let (grouping, without) = self.aggregate_grouping()?;
                                if self.peek()?.kind == ERROR {
                                    failed = true;
                                    break;
                                }
                                Some(Callee::Aggregate {
                                    item: item.copy_in(self.work)?,
                                    grouping,
                                    without,
                                })
                            } else if item.kind.is_function_identifier() && next == LEFT_PAREN {
                                Some(Callee::Function(item.copy_in(self.work)?))
                            } else {
                                None
                            };
                            if let Some(callee) = callee {
                                if self.expect(LEFT_PAREN, "aggregation", "")?.is_none() {
                                    if matches!(callee, Callee::Aggregate { .. }) {
                                        values.push(self.finish_call(
                                            callee,
                                            self.work.buffer(),
                                            self.last_closing,
                                        )?);
                                    } else {
                                        failed = true;
                                        break;
                                    }
                                } else if let Some(close) = self.accept(RIGHT_PAREN)? {
                                    values.push(self.finish_call(
                                        callee,
                                        self.work.buffer(),
                                        close.span.end,
                                    )?);
                                } else {
                                    frames.push(Frame::Arguments {
                                        callee,
                                        arguments: self.work.buffer(),
                                        mode,
                                    });
                                    frames.push(Frame::Parse(0, mode));
                                }
                            } else if let Some(id) = self.selector(item)? {
                                values.push(id);
                            } else {
                                failed = true;
                                break;
                            }
                        }
                        STEP | RANGE | MAX_OF | MIN_OF if mode != Mode::Normal => {
                            self.take()?;
                            if self.accept(LEFT_PAREN)?.is_none() {
                                if !self.recover_prefix(&mut frames, &mut values, false)? {
                                    failed = true;
                                    break;
                                }
                                continue;
                            }
                            let callee = Callee::Duration(item, mode);
                            if let Callee::Duration(ref item, _) = callee
                                && matches!(item.kind, STEP | RANGE)
                            {
                                if let Some(close) = self.accept(RIGHT_PAREN)? {
                                    values.push(self.finish_call(
                                        callee,
                                        self.work.buffer(),
                                        close.span.end,
                                    )?);
                                } else if !self.recover_prefix(&mut frames, &mut values, false)? {
                                    failed = true;
                                    break;
                                }
                            } else {
                                frames.push(Frame::DurationFirst(callee));
                                frames.push(Frame::Parse(0, Mode::Duration));
                            }
                        }
                        _ => {
                            if !self.recover_prefix(&mut frames, &mut values, false)? {
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
                    let item = self.peek()?;
                    if mode == Mode::Normal && item.kind == LEFT_BRACKET {
                        self.take()?;
                        frames.push(Frame::Rest(precedence, mode));
                        frames.push(Frame::Range {
                            left,
                            opening: item,
                        });
                        frames.push(Frame::Parse(0, Mode::Duration));
                    } else if mode == Mode::Normal && item.kind == OFFSET {
                        self.take()?;
                        frames.push(Frame::Rest(precedence, mode));
                        frames.push(Frame::Offset(left));
                        frames.push(Frame::Parse(0, Mode::Offset));
                    } else if mode == Mode::Normal && item.kind == AT {
                        self.take()?;
                        self.at(left)?;
                        values.push(left);
                        frames.push(Frame::Rest(precedence, mode));
                    } else if mode == Mode::Normal && matches!(item.kind, ANCHORED | SMOOTHED) {
                        self.take()?;
                        self.extended(left, item.kind)?;
                        values.push(left);
                        frames.push(Frame::Rest(precedence, mode));
                    } else if let Some(binding) = item.kind.precedence()
                        && binding >= precedence
                        && !mode.is_offset()
                        && (mode == Mode::Normal
                            || matches!(item.kind, ADD | SUB | MUL | DIV | MOD | POW))
                    {
                        self.take()?;
                        let (matching, return_bool) = if mode == Mode::Normal {
                            let Some(modifiers) = self.binary_matching()? else {
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
                            operator: item.copy_in(self.work)?,
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
                    values.push(self.unary(operator, child, mode, wrapped)?);
                }
                Frame::Paren(open, mode) => {
                    let child = values.pop().unwrap();
                    let Some(close) = self.expect(RIGHT_PAREN, "", "")? else {
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
                        self.experimental_duration(child)?;
                        values.push(self.wrap_duration(child, open.span.start, close.span.end));
                        if mode == Mode::Offset {
                            // Leading parentheses use the general duration alternative, including lookahead.
                            frames.push(Frame::Rest(0, Mode::Duration));
                        }
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
                        self.experimental_duration(left)?;
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
                            )?;
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
                            )?;
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
                } => {
                    arguments.push(values.pop().unwrap());
                    if let Some(comma) = self.accept(COMMA)? {
                        if let Some(close) = self.accept(RIGHT_PAREN)? {
                            self.error(
                                comma.span,
                                "trailing commas not allowed in function call args",
                            )?;
                            values.push(self.finish_call(callee, arguments, close.span.end)?);
                        } else {
                            let mut last_comma = comma.span;
                            loop {
                                self.work.step(1)?;
                                let next = self.peek()?.kind;
                                if next == NUMBER
                                    || next == DURATION
                                    || next == STRING
                                    || next == LEFT_PAREN
                                    || next == LEFT_BRACE
                                    || next == ADD
                                    || next == SUB
                                    || next.is_metric_identifier()
                                {
                                    break;
                                }
                                self.error(
                                    last_comma,
                                    "trailing commas not allowed in function call args",
                                )?;
                                if let Some(comma) = self.accept(COMMA)? {
                                    last_comma = comma.span;
                                } else {
                                    break;
                                }
                            }
                            if let Some(close) = self.accept(RIGHT_PAREN)? {
                                values.push(self.finish_call(callee, arguments, close.span.end)?);
                                continue;
                            }
                            frames.push(Frame::Arguments {
                                callee,
                                arguments,
                                mode,
                            });
                            frames.push(Frame::Parse(0, mode));
                        }
                    } else if let Some(close) = self.accept(RIGHT_PAREN)? {
                        values.push(self.finish_call(callee, arguments, close.span.end)?);
                    } else {
                        self.unexpected("", "")?;
                        failed = true;
                        break;
                    }
                }
                Frame::DurationFirst(callee) => {
                    let first = values.pop().unwrap();
                    if self.accept(COMMA)?.is_some() {
                        frames.push(Frame::DurationSecond(callee, first));
                        frames.push(Frame::Parse(0, Mode::Duration));
                    } else if !self.recover_prefix(&mut frames, &mut values, false)? {
                        failed = true;
                        break;
                    }
                }
                Frame::DurationSecond(callee, first) => {
                    let second = values.pop().unwrap();
                    if let Some(close) = self.accept(RIGHT_PAREN)? {
                        values.push(self.finish_call(
                            callee,
                            AccountedVec::from_slice_in(&[first, second], self.work.context),
                            close.span.end,
                        )?);
                    } else if !self.recover_prefix(&mut frames, &mut values, false)? {
                        failed = true;
                        break;
                    }
                }
                Frame::Range { left, opening } => {
                    let range = self.positive(values.pop().unwrap())?;
                    if self.accept(COLON)?.is_some() {
                        if let Some(close) = self.accept(RIGHT_BRACKET)? {
                            values.push(self.subquery(left, range, None, close.span.end));
                        } else {
                            frames.push(Frame::RangeStep { left, range });
                            frames.push(Frame::Parse(0, Mode::Duration));
                        }
                    } else if let Some(close) = self.accept(RIGHT_BRACKET)? {
                        values.push(self.finish_range(left, opening, range, close)?);
                    } else {
                        self.unexpected("subquery or range", "\":\" or \"]\"")?;
                        values.push(left);
                    }
                }
                Frame::RangeStep { left, range } => {
                    let step = self.positive(values.pop().unwrap())?;
                    if let Some(close) = self.expect(RIGHT_BRACKET, "subquery selector", "\"]\"")? {
                        values.push(self.subquery(left, range, Some(step), close.span.end));
                    } else {
                        values.push(left);
                    }
                }
                Frame::Offset(left) => {
                    let offset = values.pop().unwrap();
                    self.add_offset(left, offset)?;
                    values.push(left);
                }
            }
        }
        let root = if failed { None } else { values.pop() };
        if root.is_some() && !matches!(self.peek()?.kind, EOF | ERROR) {
            if self.recovery_shifts == 3 {
                self.recover_to(&[EOF, ERROR])?;
            } else {
                self.unexpected("", "")?;
            }
        }
        Ok(ParseOutput {
            arena: self.arena,
            root,
            errors: self.errors,
        })
    }
}

struct Description<'a, 'b>(&'a Item<'b>);
fn description<'a, 'b>(item: &'a Item<'b>) -> Description<'a, 'b> {
    Description(item)
}
impl fmt::Display for Description<'_, '_> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let item = self.0;
        if item.kind == EOF {
            return out.write_str("end of input");
        }
        if item.kind.text().is_some() {
            return if item.kind.is_keyword() {
                write!(
                    out,
                    "<{}>",
                    std::str::from_utf8(&item.value).expect("ASCII keyword")
                )
            } else if item.kind.is_operator() {
                write!(
                    out,
                    "<op:{}>",
                    std::str::from_utf8(&item.value).expect("ASCII operator")
                )
            } else if item.kind.is_aggregator() {
                write!(
                    out,
                    "<aggr:{}>",
                    std::str::from_utf8(&item.value).expect("ASCII aggregator")
                )
            } else {
                write!(out, "{}", quote::quoted(&item.value))
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
        let mut end = item.value.len();
        let truncated = end > 10 && !matches!(item.kind, IDENTIFIER | METRIC_IDENTIFIER);
        if truncated {
            end = 0;
            for _ in 0..10 {
                if end == item.value.len() {
                    break;
                }
                end += quote::decode_rune(&item.value[end..]).1;
            }
        }
        write!(
            out,
            "{kind} {}{}",
            quote::quoted(&item.value[..end]),
            if truncated { "..." } else { "" }
        )
    }
}

struct Timestamp(f64);
impl fmt::Display for Timestamp {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.0;
        if value.is_nan() {
            out.write_str("NaN")
        } else if value == f64::INFINITY {
            out.write_str("+Inf")
        } else if value == f64::NEG_INFINITY {
            out.write_str("-Inf")
        } else {
            write!(out, "{value:.6}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        frontend::{ast::ValueType, regex::Selector},
        test_storage::interrupt_during,
    };

    fn expression(query: &str) -> ParseOutput {
        let output = parse(query.as_bytes(), ParserOptions::default(), |pattern| {
            Selector::compile_in(pattern, None)
        });
        assert!(output.errors.is_empty(), "{query}: {:?}", output.errors);
        output
    }

    #[test]
    fn deep_parse_counts_retained_slots_and_releases_transient_frames() {
        let depth = 4096;
        let input = ["(".repeat(depth), "m".into(), ")".repeat(depth)].concat();
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let output = parse_in(
            input.as_bytes(),
            ParserOptions::default(),
            |_| -> Result<Strong<dyn CompiledMatcher>, Vec<u8>> { panic!("unexpected matcher") },
            Some(&account),
            Some(&control),
        )
        .unwrap();
        assert!(output.errors.is_empty());
        assert_eq!(output.arena.nodes.len(), depth + 1);
        assert_eq!(
            output.arena[output.root.unwrap()].value_type,
            ValueType::Vector
        );
        let Expr::Selector(selector) = &output.arena.nodes[0].expression else {
            panic!("expected selector")
        };
        let retained = output.arena.nodes.capacity() * size_of::<super::super::ast::Node>()
            + selector.name.capacity()
            + selector.matchers.capacity() * size_of::<Option<Matcher>>()
            + selector
                .matchers
                .iter()
                .flatten()
                .map(|matcher| matcher.name.capacity() + matcher.value.capacity())
                .sum::<usize>();
        let observed = account.snapshot();
        assert_eq!(
            observed.memory[MemoryKind::Compiled as usize].current,
            retained as u64
        );
        assert!(observed.memory[MemoryKind::Compiled as usize].peak > retained as u64);
        assert!(observed.memory[MemoryKind::Compiled as usize].released > 0);
        eprintln!(
            "frontend slots: node={} frame={} item={} matcher={} error={} nested_bytes={}; retained={} peak={}",
            size_of::<super::super::ast::Node>(),
            size_of::<Frame<'_>>(),
            size_of::<Item<'_>>(),
            size_of::<Matcher>(),
            size_of::<ParseError>(),
            size_of::<AccountedVec<u8>>(),
            retained,
            observed.memory[MemoryKind::Compiled as usize].peak,
        );
        drop(output);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn cancellation_during_scanning_and_recovery_releases_partial_parse() {
        for input in [
            [b"sum by(".as_slice(), &b"a,".repeat(100_000), b"a)(m)"].concat(),
            [b"sum by(".as_slice(), &b"@ ".repeat(100_000), b")(m)"].concat(),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let output = interrupt_during(&account, &control, || {
                parse_in(
                    &input,
                    ParserOptions::default(),
                    |_| -> Result<Strong<dyn CompiledMatcher>, Vec<u8>> {
                        panic!("unexpected matcher")
                    },
                    Some(&account),
                    Some(&control),
                )
            });
            assert!(matches!(output, Err(Interrupted::Cancelled)));
            assert!(account.snapshot().memory[MemoryKind::Compiled as usize].peak > 0);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn large_invalid_expression_retains_ordered_errors_after_ast_release() {
        let count = 1024;
        let input = [b"1".as_slice(), &b" + \"x\"".repeat(count)].concat();
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let output = parse_in(
            &input,
            ParserOptions::default(),
            |_| -> Result<Strong<dyn CompiledMatcher>, Vec<u8>> { panic!("unexpected matcher") },
            Some(&account),
            Some(&control),
        )
        .unwrap();
        assert_eq!(output.errors.len(), count);
        for (index, error) in output.errors.iter().enumerate() {
            assert_eq!(
                error.span,
                Span {
                    start: 4 + index * 6,
                    end: 7 + index * 6
                }
            );
            assert_eq!(
                error.message.as_ref(),
                b"binary expression must contain only scalar and instant vector types"
            );
        }
        let ParseOutput { arena, errors, .. } = output;
        drop(arena);
        let retained = errors.capacity() * size_of::<ParseError>()
            + errors
                .iter()
                .map(|error| error.message.capacity())
                .sum::<usize>();
        assert_eq!(
            account.snapshot().memory[MemoryKind::Compiled as usize].current,
            retained as u64
        );
        drop(errors);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn regex_callback_unwind_releases_constructed_nodes_and_parser_scratch() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let input = [
            b"(".repeat(1024),
            b"m + m{l=~\"a\"}".to_vec(),
            b")".repeat(1024),
        ]
        .concat();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parse_in(
                &input,
                ParserOptions::default(),
                |_| -> Result<Strong<dyn CompiledMatcher>, Vec<u8>> {
                    assert!(account.snapshot().memory[MemoryKind::Compiled as usize].current > 0);
                    panic!("regex callback fault")
                },
                Some(&account),
                Some(&control),
            )
        }));
        assert!(result.is_err());
        assert!(account.snapshot().memory[MemoryKind::Compiled as usize].peak > 0);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn parenthesized_offsets_follow_the_duration_fallback_grammar() {
        for (query, selector_root) in [
            ("m offset (1s) + 1", true),
            ("m offset (1s) ^ 2", true),
            ("m offset --1s + 1", true),
            ("m offset +(1s) + 1", false),
            ("m offset -(1s) + 1", false),
            ("m offset 1s + 1", false),
            ("m offset step() + 1", false),
            ("m offset min_of(1s,2s) + 1", false),
        ] {
            let output = expression(query);
            assert_eq!(
                matches!(
                    output.arena[output.root.unwrap()].expression,
                    Expr::Selector(_)
                ),
                selector_root,
                "{query}"
            );
        }
    }

    #[test]
    fn parenthesized_offsets_retain_source_lookahead_positions() {
        for (query, include_enclosing_close) in [
            ("abs(m offset (1s*1))", true),
            ("abs(m offset --1s)", true),
            ("abs(m offset +(1s*1))", false),
            ("abs(m offset 1s)", false),
            ("abs(m offset step())", false),
        ] {
            let output = expression(query);
            let selector = output
                .arena
                .nodes
                .iter()
                .find(|node| matches!(node.expression, Expr::Selector(_)))
                .unwrap();
            assert_eq!(
                selector.span,
                Span {
                    start: 4,
                    end: query.len() - usize::from(!include_enclosing_close)
                },
                "{query}"
            );
        }
    }
}
