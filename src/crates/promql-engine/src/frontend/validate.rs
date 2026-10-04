// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus parse.go ordered AST validation; see NOTICE.
use super::{
    Span,
    ast::{Arena, Cardinality, Expr, NodeId, ParseError, ValueType},
    functions::Function,
    quote,
    token::{TokenKind, TokenKind::*},
};

#[derive(Clone, Copy)]
enum Context {
    AggregateExpression,
    AggregateParameter,
    Call(&'static Function),
}

impl Context {
    fn text(self) -> String {
        match self {
            Self::AggregateExpression => "aggregation expression".into(),
            Self::AggregateParameter => "aggregation parameter".into(),
            Self::Call(function) => format!(
                "call to function {}",
                quote::bytes(function.name.as_bytes())
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

fn error(errors: &mut Vec<ParseError>, span: Span, message: impl AsRef<[u8]>) {
    errors.push(ParseError {
        span,
        message: message.as_ref().to_vec(),
    });
}

/// Construction errors suppress this entire pass, as in the upstream parser.
pub fn validate(input: &[u8], arena: &mut Arena, root: NodeId, errors: &mut Vec<ParseError>) {
    if !errors.is_empty() {
        return;
    }
    let mut frames = vec![Frame::Enter(root)];
    while let Some(frame) = frames.pop() {
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
                            error(
                                errors,
                                span,
                                format!(
                                    "aggregation operator expected in aggregation expression but got {}",
                                    operator_quote(operator)
                                ),
                            );
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
                                error(
                                    errors,
                                    span,
                                    format!(
                                        "expected {nargs} argument(s) in call to {}, got {count}",
                                        quote::bytes(function.name.as_bytes())
                                    ),
                                );
                            }
                        } else {
                            let minimum = nargs - 1;
                            if minimum > count {
                                error(
                                    errors,
                                    span,
                                    format!(
                                        "expected at least {minimum} argument(s) in call to {}, got {count}",
                                        quote::bytes(function.name.as_bytes())
                                    ),
                                );
                            } else if function.variadic > 0
                                && minimum + (function.variadic as usize) < count
                            {
                                error(
                                    errors,
                                    span,
                                    format!(
                                        "expected at most {} argument(s) in call to {}, got {count}",
                                        minimum + (function.variadic as usize),
                                        quote::bytes(function.name.as_bytes())
                                    ),
                                );
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
                            frames.push(Frame::Expect(
                                arguments[index],
                                function.arguments[index.min(nargs - 1)],
                                Context::Call(function),
                            ));
                        }
                        if let Some(second) = info_second {
                            if arena[second].value_type != ValueType::Vector {
                                error(
                                    errors,
                                    span,
                                    format!(
                                        "expected type instant vector in call to function \"info\", got {}",
                                        arena[second].value_type.diagnostic_name()
                                    ),
                                );
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
                                ),
                                _ => error(errors, second_span, "expected label selectors only"),
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
                            );
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
                                if matcher.name == b"__name__" {
                                    error(
                                        errors,
                                        span,
                                        format!(
                                            "metric name must not be set twice: {} or {}",
                                            quote::bytes(&selector.name),
                                            quote::bytes(&matcher.value)
                                        ),
                                    );
                                }
                            }
                        } else if !selector.bypass_empty_matcher_check
                            && selector
                                .matchers
                                .iter()
                                .flatten()
                                .all(|matcher| matcher.matches(b""))
                        {
                            error(
                                errors,
                                span,
                                "vector selector must contain at least one non-empty matcher",
                            );
                        }
                    }
                    Expr::Duration { .. } => {
                        error(errors, span, "unknown node type: *parser.DurationExpr")
                    }
                }
            }
            Frame::Expect(id, wanted, context) => {
                frames.push(Frame::Expected(id, wanted, context));
                frames.push(Frame::Enter(id));
            }
            Frame::Expected(id, wanted, context) => {
                let actual = arena[id].value_type;
                if actual != wanted {
                    error(
                        errors,
                        arena[id].span,
                        format!(
                            "expected type {} in {}, got {}",
                            wanted.diagnostic_name(),
                            context.text(),
                            actual.diagnostic_name()
                        ),
                    );
                }
            }
            Frame::Binary(id) => binary(input, arena, id, errors),
            Frame::Unary(id, child) => {
                let value_type = arena[child].value_type;
                if !matches!(value_type, ValueType::Scalar | ValueType::Vector) {
                    error(
                        errors,
                        arena[id].span,
                        format!(
                            "unary expression only allowed on expressions of type scalar or instant vector, got {}",
                            quote::bytes(value_type.diagnostic_name().as_bytes())
                        ),
                    );
                }
            }
            Frame::Subquery(id, child) => {
                let value_type = arena[child].value_type;
                if value_type != ValueType::Vector {
                    error(
                        errors,
                        arena[id].span,
                        format!(
                            "subquery is only allowed on instant vector, got {} instead",
                            value_type.name()
                        ),
                    );
                }
            }
        }
    }
}

fn operator_quote(operator: TokenKind) -> String {
    quote::bytes(operator.text().unwrap_or("<unknown>").as_bytes())
}

fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn binary(input: &[u8], arena: &mut Arena, id: NodeId, errors: &mut Vec<ParseError>) {
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
    let operator_span = || {
        let mut start = left_span.end;
        while input.get(start).is_some_and(|&b| space(b)) {
            start += 1;
        }
        let mut end = right_span.start.saturating_sub(1);
        while end > 0 && input.get(end).is_some_and(|&b| space(b)) {
            end -= 1;
        }
        // The upstream diagnostic stores the final operator byte index, not index + 1.
        Span { start, end }
    };
    if return_bool && !operator.is_comparison() {
        error(
            errors,
            operator_span(),
            "bool modifier can only be used on comparison operators",
        );
    }
    if operator.is_comparison()
        && !return_bool
        && left_type == ValueType::Scalar
        && right_type == ValueType::Scalar
    {
        error(
            errors,
            operator_span(),
            "comparisons between scalars must use BOOL modifier",
        );
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
                if label == included {
                    error(
                        errors,
                        operator_span(),
                        format!(
                            "label {} must not occur in ON and GROUP clause at once",
                            quote::bytes(label)
                        ),
                    );
                }
            }
        }
    }
    if !operator.is_operator() {
        error(
            errors,
            span,
            format!(
                "binary expression does not support operator {}",
                operator_quote(operator)
            ),
        );
    }
    if !matches!(left_type, ValueType::Scalar | ValueType::Vector) {
        error(
            errors,
            left_span,
            "binary expression must contain only scalar and instant vector types",
        );
    }
    if !matches!(right_type, ValueType::Scalar | ValueType::Vector) {
        error(
            errors,
            right_span,
            "binary expression must contain only scalar and instant vector types",
        );
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
            );
        }
        if filled {
            error(
                errors,
                span,
                "filling in missing series only allowed between instant vectors",
            );
        }
        *matching = None;
    } else if operator.is_set() {
        if matches!(
            matching_value.cardinality,
            Cardinality::OneToMany | Cardinality::ManyToOne
        ) {
            error(
                errors,
                span,
                format!(
                    "no grouping allowed for {} operation",
                    operator_quote(operator)
                ),
            );
        }
        if matching_value.cardinality != Cardinality::ManyToMany {
            error(errors, span, "set operations must always be many-to-many");
        }
        if filled {
            error(
                errors,
                span,
                "filling in missing series not allowed for set operators",
            );
        }
    }
    if (left_type == ValueType::Scalar || right_type == ValueType::Scalar) && operator.is_set() {
        error(
            errors,
            span,
            format!(
                "set operator {} not allowed in binary scalar expression",
                operator_quote(operator)
            ),
        );
    }
}
