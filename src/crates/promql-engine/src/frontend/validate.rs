// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus parse.go ordered AST validation; see NOTICE.
use super::{
    Span,
    ast::{Arena, Cardinality, Expr, NodeId, ParseError, ValueType},
    functions::Function,
    quote,
    token::{TokenKind, TokenKind::*},
    work::Work,
};
use crate::{accounting::AccountedVec, storage::Interrupted};
use std::fmt;

#[derive(Clone, Copy)]
enum Context {
    AggregateExpression,
    AggregateParameter,
    Call(&'static Function),
}

impl fmt::Display for Context {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AggregateExpression => out.write_str("aggregation expression"),
            Self::AggregateParameter => out.write_str("aggregation parameter"),
            Self::Call(function) => write!(
                out,
                "call to function {}",
                quote::quoted(function.name.as_bytes())
            ),
        }
    }
}

enum Frame {
    Enter(NodeId),
    Expect(NodeId, ValueType, Context),
    Expected(NodeId, ValueType, Context),
    Binary(NodeId),
    Unary(NodeId, NodeId),
    Subquery(NodeId, NodeId),
}

fn error(
    errors: &mut AccountedVec<ParseError>,
    span: Span,
    message: impl AsRef<[u8]>,
    work: Work<'_>,
) -> Result<(), Interrupted> {
    errors.push(ParseError {
        span,
        message: work.copy(message.as_ref())?,
    });
    Ok(())
}
fn error_fmt(
    errors: &mut AccountedVec<ParseError>,
    span: Span,
    message: fmt::Arguments<'_>,
    work: Work<'_>,
) -> Result<(), Interrupted> {
    let message = work.format(message)?;
    errors.push(ParseError { span, message });
    Ok(())
}

/// Construction errors suppress this entire pass, as in the upstream parser.
pub fn validate(
    input: &[u8],
    arena: &mut Arena,
    root: NodeId,
    errors: &mut AccountedVec<ParseError>,
) {
    validate_in(input, arena, root, errors, Work::default()).expect("inert validation");
}

pub(super) fn validate_in(
    input: &[u8],
    arena: &mut Arena,
    root: NodeId,
    errors: &mut AccountedVec<ParseError>,
    work: Work<'_>,
) -> Result<(), Interrupted> {
    if !errors.is_empty() {
        return Ok(());
    }
    let mut frames = work.buffer();
    frames.push(Frame::Enter(root));
    while let Some(frame) = frames.pop() {
        work.step(1)?;
        match frame {
            Frame::Enter(id) => {
                let span = arena[id].span;
                match &arena[id].expression {
                    Expr::Number { .. } | Expr::String(_) => {}
                    Expr::Aggregate {
                        operator,
                        expression,
                        parameter,
                        ..
                    } => {
                        let operator = *operator;
                        let expression = *expression;
                        let parameter = *parameter;
                        if !operator.is_aggregator() {
                            error_fmt(
                                errors,
                                span,
                                format_args!(
                                    "aggregation operator expected in aggregation expression but got {}",
                                    operator_quote(operator)
                                ),
                                work,
                            )?;
                        }
                        if operator.takes_parameter() {
                            let expected = if operator == COUNT_VALUES {
                                ValueType::String
                            } else {
                                ValueType::Scalar
                            };
                            frames.push(Frame::Expect(
                                parameter.expect("validated aggregate parameter"),
                                expected,
                                Context::AggregateParameter,
                            ));
                        }
                        frames.push(Frame::Expect(
                            expression.expect("validated aggregate expression"),
                            ValueType::Vector,
                            Context::AggregateExpression,
                        ));
                    }
                    Expr::Binary { left, right, .. } => {
                        frames.push(Frame::Binary(id));
                        frames.push(Frame::Enter(*right));
                        frames.push(Frame::Enter(*left));
                    }
                    Expr::Call {
                        function,
                        arguments,
                    } => {
                        let function = function.expect("validated function name");
                        let count = arguments.len();
                        let nargs = function.arguments.len();
                        if function.variadic == 0 {
                            if nargs != count {
                                error_fmt(
                                    errors,
                                    span,
                                    format_args!(
                                        "expected {nargs} argument(s) in call to {}, got {count}",
                                        quote::quoted(function.name.as_bytes())
                                    ),
                                    work,
                                )?;
                            }
                        } else {
                            let minimum = nargs - 1;
                            if minimum > count {
                                error_fmt(
                                    errors,
                                    span,
                                    format_args!(
                                        "expected at least {minimum} argument(s) in call to {}, got {count}",
                                        quote::quoted(function.name.as_bytes())
                                    ),
                                    work,
                                )?;
                            } else if function.variadic > 0
                                && minimum + (function.variadic as usize) < count
                            {
                                error_fmt(
                                    errors,
                                    span,
                                    format_args!(
                                        "expected at most {} argument(s) in call to {}, got {count}",
                                        minimum + (function.variadic as usize),
                                        quote::quoted(function.name.as_bytes())
                                    ),
                                    work,
                                )?;
                            }
                        }
                        let info_second =
                            (function.name == "info" && count > 1).then(|| arguments[1]);
                        let checked = if function.variadic == 0 {
                            count.min(nargs)
                        } else {
                            count
                        };
                        for index in (0..checked).rev() {
                            work.step(1)?;
                            frames.push(Frame::Expect(
                                arguments[index],
                                function.arguments[index.min(nargs - 1)],
                                Context::Call(function),
                            ));
                        }
                        if let Some(second) = info_second {
                            if arena[second].value_type != ValueType::Vector {
                                error_fmt(
                                    errors,
                                    span,
                                    format_args!(
                                        "expected type instant vector in call to function \"info\", got {}",
                                        arena[second].value_type.diagnostic_name()
                                    ),
                                    work,
                                )?;
                            }
                            let second_span = arena[second].span;
                            match &mut arena[second].expression {
                                Expr::Selector(selector) if selector.name.is_empty() => {
                                    selector.bypass_empty_matcher_check = true
                                }
                                Expr::Selector(_) => error(
                                    errors,
                                    second_span,
                                    "expected label selectors only, got vector selector instead",
                                    work,
                                )?,
                                _ => error(
                                    errors,
                                    second_span,
                                    "expected label selectors only",
                                    work,
                                )?,
                            }
                        }
                    }
                    Expr::Paren(child) => frames.push(Frame::Enter(*child)),
                    Expr::Unary {
                        operator,
                        expression,
                    } => {
                        if !matches!(operator, ADD | SUB) {
                            error(
                                errors,
                                span,
                                "only + and - operators allowed for unary expressions",
                                work,
                            )?;
                        }
                        frames.push(Frame::Unary(id, *expression));
                        frames.push(Frame::Enter(*expression));
                    }
                    Expr::Subquery { expression, .. } => {
                        frames.push(Frame::Subquery(id, *expression));
                        frames.push(Frame::Enter(*expression));
                    }
                    Expr::Matrix { selector, .. } => frames.push(Frame::Enter(*selector)),
                    Expr::Selector(selector) => {
                        if !selector.name.is_empty() {
                            for matcher in selector.matchers[..selector.matchers.len() - 1]
                                .iter()
                                .flatten()
                            {
                                work.step(1)?;
                                if matcher.name == b"__name__" {
                                    error_fmt(
                                        errors,
                                        span,
                                        format_args!(
                                            "metric name must not be set twice: {} or {}",
                                            quote::quoted(&selector.name),
                                            quote::quoted(&matcher.value)
                                        ),
                                        work,
                                    )?;
                                }
                            }
                        } else if !selector.bypass_empty_matcher_check {
                            let mut all_empty = true;
                            for matcher in selector.matchers.iter().flatten() {
                                work.step(1)?;
                                let matches =
                                    matcher.matches_with(b"", work.account, work.control)?;
                                work.step(0)?;
                                if !matches {
                                    all_empty = false;
                                    break;
                                }
                            }
                            if all_empty {
                                error(
                                    errors,
                                    span,
                                    "vector selector must contain at least one non-empty matcher",
                                    work,
                                )?;
                            }
                        }
                    }
                    Expr::Duration { .. } => error(
                        errors,
                        span,
                        "unknown node type: *parser.DurationExpr",
                        work,
                    )?,
                }
            }
            Frame::Expect(id, wanted, context) => {
                frames.push(Frame::Expected(id, wanted, context));
                frames.push(Frame::Enter(id));
            }
            Frame::Expected(id, wanted, context) => {
                let actual = arena[id].value_type;
                if actual != wanted {
                    error_fmt(
                        errors,
                        arena[id].span,
                        format_args!(
                            "expected type {} in {}, got {}",
                            wanted.diagnostic_name(),
                            context,
                            actual.diagnostic_name()
                        ),
                        work,
                    )?;
                }
            }
            Frame::Binary(id) => binary(input, arena, id, errors, work)?,
            Frame::Unary(id, child) => {
                let value_type = arena[child].value_type;
                if !matches!(value_type, ValueType::Scalar | ValueType::Vector) {
                    error_fmt(
                        errors,
                        arena[id].span,
                        format_args!(
                            "unary expression only allowed on expressions of type scalar or instant vector, got {}",
                            quote::quoted(value_type.diagnostic_name().as_bytes())
                        ),
                        work,
                    )?;
                }
            }
            Frame::Subquery(id, child) => {
                let value_type = arena[child].value_type;
                if value_type != ValueType::Vector {
                    error_fmt(
                        errors,
                        arena[id].span,
                        format_args!(
                            "subquery is only allowed on instant vector, got {} instead",
                            value_type.name()
                        ),
                        work,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn operator_quote(operator: TokenKind) -> impl fmt::Display {
    quote::quoted(operator.text().unwrap_or("<unknown>").as_bytes())
}

fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn binary(
    input: &[u8],
    arena: &mut Arena,
    id: NodeId,
    errors: &mut AccountedVec<ParseError>,
    work: Work<'_>,
) -> Result<(), Interrupted> {
    let Expr::Binary {
        operator,
        left,
        right,
        return_bool,
        ..
    } = arena[id].expression
    else {
        unreachable!()
    };
    let span = arena[id].span;
    let left_span = arena[left].span;
    let right_span = arena[right].span;
    let left_type = arena[left].value_type;
    let right_type = arena[right].value_type;
    let operator_span = || -> Result<Span, Interrupted> {
        let mut start = left_span.end;
        while input.get(start).is_some_and(|&b| space(b)) {
            work.step(1)?;
            start += 1;
        }
        let mut end = right_span.start.saturating_sub(1);
        while end > 0 && input.get(end).is_some_and(|&b| space(b)) {
            work.step(1)?;
            end -= 1;
        }
        // The upstream diagnostic stores the final operator byte index, not index + 1.
        Ok(Span { start, end })
    };
    if return_bool && !operator.is_comparison() {
        error(
            errors,
            operator_span()?,
            "bool modifier can only be used on comparison operators",
            work,
        )?;
    }
    if operator.is_comparison()
        && !return_bool
        && left_type == ValueType::Scalar
        && right_type == ValueType::Scalar
    {
        error(
            errors,
            operator_span()?,
            "comparisons between scalars must use BOOL modifier",
            work,
        )?;
    }
    let Expr::Binary { matching, .. } = &mut arena[id].expression else {
        unreachable!()
    };
    let matching_value = matching.as_mut().expect("constructed vector matching");
    if operator.is_set() && matching_value.cardinality == Cardinality::OneToOne {
        matching_value.cardinality = Cardinality::ManyToMany;
    }
    if matching_value.on {
        for label in matching_value.labels.as_deref().unwrap_or_default() {
            for included in matching_value.include.as_deref().unwrap_or_default() {
                work.step(1)?;
                if work.equal(label, included)? {
                    error_fmt(
                        errors,
                        operator_span()?,
                        format_args!(
                            "label {} must not occur in ON and GROUP clause at once",
                            quote::quoted(label)
                        ),
                        work,
                    )?;
                }
            }
        }
    }
    if !operator.is_operator() {
        error_fmt(
            errors,
            span,
            format_args!(
                "binary expression does not support operator {}",
                operator_quote(operator)
            ),
            work,
        )?;
    }
    if !matches!(left_type, ValueType::Scalar | ValueType::Vector) {
        error(
            errors,
            left_span,
            "binary expression must contain only scalar and instant vector types",
            work,
        )?;
    }
    if !matches!(right_type, ValueType::Scalar | ValueType::Vector) {
        error(
            errors,
            right_span,
            "binary expression must contain only scalar and instant vector types",
            work,
        )?;
    }
    let filled = matching_value.fill_left.is_some() || matching_value.fill_right.is_some();
    if left_type != ValueType::Vector || right_type != ValueType::Vector {
        if matching_value
            .labels
            .as_ref()
            .is_some_and(|labels| !labels.is_empty())
        {
            error(
                errors,
                span,
                "vector matching only allowed between instant vectors",
                work,
            )?;
        }
        if filled {
            error(
                errors,
                span,
                "filling in missing series only allowed between instant vectors",
                work,
            )?;
        }
        *matching = None;
    } else if operator.is_set() {
        if matches!(
            matching_value.cardinality,
            Cardinality::OneToMany | Cardinality::ManyToOne
        ) {
            error_fmt(
                errors,
                span,
                format_args!(
                    "no grouping allowed for {} operation",
                    operator_quote(operator)
                ),
                work,
            )?;
        }
        if matching_value.cardinality != Cardinality::ManyToMany {
            error(
                errors,
                span,
                "set operations must always be many-to-many",
                work,
            )?;
        }
        if filled {
            error(
                errors,
                span,
                "filling in missing series not allowed for set operators",
                work,
            )?;
        }
    }
    if (left_type == ValueType::Scalar || right_type == ValueType::Scalar) && operator.is_set() {
        error_fmt(
            errors,
            span,
            format_args!(
                "set operator {} not allowed in binary scalar expression",
                operator_quote(operator)
            ),
            work,
        )?;
    }
    Ok(())
}
