// SPDX-License-Identifier: GPL-3.0-or-later
//! Test-only projection into the independently captured flat Go AST format.
use std::collections::HashMap;

use netdata_promql_engine::frontend::{
    Span,
    ast::{Arena, Expr, MatchType, NodeId, SelectorModifiers},
    parser::ParseOutput,
};
use serde_json::{Value, json};

pub fn bytes(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(value >> 18) as usize] as char);
        out.push(ALPHABET[((value >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn span(span: Span) -> Value {
    json!({"Start": span.start, "End": span.end})
}
fn float(value: f64) -> Value {
    json!({"float_bits": format!("{:016x}", value.to_bits())})
}
fn labels(value: &Option<Vec<Vec<u8>>>) -> Value {
    value.as_ref().map_or(Value::Null, |labels| {
        json!(labels.iter().map(|label| bytes(label)).collect::<Vec<_>>())
    })
}

fn children(arena: &Arena, id: NodeId) -> Vec<NodeId> {
    match &arena[id].expression {
        Expr::Aggregate {
            expression,
            parameter,
            ..
        } => expression.iter().chain(parameter.iter()).copied().collect(),
        Expr::Binary { left, right, .. } => vec![*left, *right],
        Expr::Duration { left, right, .. } => left.iter().chain(right.iter()).copied().collect(),
        Expr::Call { arguments, .. } => arguments.clone(),
        Expr::Matrix {
            selector,
            range_expression,
            ..
        } => std::iter::once(*selector)
            .chain(range_expression.iter().copied())
            .collect(),
        Expr::Subquery {
            expression,
            range_expression,
            modifiers,
            step_expression,
            ..
        } => std::iter::once(*expression)
            .chain(range_expression.iter().copied())
            .chain(modifiers.original_offset_expression.iter().copied())
            .chain(step_expression.iter().copied())
            .collect(),
        Expr::Selector(selector) => selector
            .modifiers
            .original_offset_expression
            .iter()
            .copied()
            .collect(),
        Expr::Paren(child)
        | Expr::Unary {
            expression: child, ..
        } => vec![*child],
        _ => Vec::new(),
    }
}

fn modifier_fields(
    modifiers: &SelectorModifiers,
    reference: &impl Fn(Option<NodeId>) -> Value,
) -> Value {
    json!({
        "OriginalOffset": modifiers.original_offset,
        "OriginalOffsetExpr": reference(modifiers.original_offset_expression),
        "Offset": modifiers.offset,
        "Timestamp": modifiers.timestamp,
        "StartOrEnd": modifiers.start_or_end.map_or(0, |kind| kind as u32),
    })
}

pub fn result(id: &str, parsed: ParseOutput) -> Value {
    let arena = &parsed.arena;
    let mut order = Vec::new();
    let mut indices = HashMap::new();
    let mut stack: Vec<_> = parsed.root.into_iter().collect();
    while let Some(id) = stack.pop() {
        if indices.contains_key(&id) {
            continue;
        }
        indices.insert(id, order.len());
        order.push(id);
        stack.extend(children(arena, id).into_iter().rev());
    }
    let reference =
        |id: Option<NodeId>| id.map_or(Value::Null, |id| json!({"node_ref": indices[&id]}));
    let mut nodes = Vec::with_capacity(order.len());
    for id in order {
        let node = &arena[id];
        let (name, mut fields) = match &node.expression {
            Expr::Number { value, duration } => (
                "NumberLiteral",
                json!({"Val": float(*value), "Duration": duration, "PosRange": span(node.span)}),
            ),
            Expr::String(value) => (
                "StringLiteral",
                json!({"Val": bytes(value), "PosRange": span(node.span)}),
            ),
            Expr::Selector(selector) => {
                let matchers: Vec<_> = selector.matchers.iter().map(|matcher| {
                    let Some(matcher) = matcher else { return Value::Null; };
                    if matches!(matcher.kind, MatchType::Regex | MatchType::NotRegex) && matcher.compiled.is_none() { Value::Null }
                    else { json!({"Type": matcher.kind as u8, "Name": bytes(&matcher.name), "Value": bytes(&matcher.value)}) }
                }).collect();
                (
                    "VectorSelector",
                    json!({
                        "Name": bytes(&selector.name), "LabelMatchers": matchers,
                        "SkipHistogramBuckets": selector.skip_histogram_buckets, "UnexpandedSeriesSet": null,
                        "Series": null, "BypassEmptyMatcherCheck": selector.bypass_empty_matcher_check,
                        "Anchored": selector.anchored, "Smoothed": selector.smoothed, "PosRange": span(node.span),
                    }),
                )
            }
            Expr::Aggregate {
                operator,
                expression,
                parameter,
                grouping,
                without,
            } => (
                "AggregateExpr",
                json!({
                    "Op": *operator as u32, "Expr": reference(*expression), "Param": reference(*parameter),
                    "Grouping": labels(grouping), "Without": without, "PosRange": span(node.span),
                }),
            ),
            Expr::Binary {
                operator,
                left,
                right,
                matching,
                return_bool,
            } => {
                let matching = matching.as_ref().map_or(Value::Null, |matching| json!({
                    "Card": matching.cardinality as u8, "MatchingLabels": labels(&matching.labels),
                    "On": matching.on, "Include": labels(&matching.include),
                    "FillValues": {"LHS": matching.fill_left.map(float), "RHS": matching.fill_right.map(float)},
                }));
                (
                    "BinaryExpr",
                    json!({"Op": *operator as u32, "LHS": reference(Some(*left)), "RHS": reference(Some(*right)),
                    "VectorMatching": matching, "ReturnBool": return_bool}),
                )
            }
            Expr::Call {
                function,
                arguments,
            } => {
                let function = function.map_or(Value::Null, |function| json!({
                    "Name": bytes(function.name.as_bytes()), "ArgTypes": function.arguments.iter().map(|ty| bytes(ty.name().as_bytes())).collect::<Vec<_>>(),
                    "Variadic": function.variadic, "ReturnType": bytes(function.return_type.name().as_bytes()),
                    "Experimental": function.experimental,
                }));
                (
                    "Call",
                    json!({"Func": function, "Args": arguments.iter().map(|&id| reference(Some(id))).collect::<Vec<_>>(), "PosRange": span(node.span)}),
                )
            }
            Expr::Matrix {
                selector,
                range,
                range_expression,
            } => (
                "MatrixSelector",
                json!({
                    "VectorSelector": reference(Some(*selector)), "Range": range, "RangeExpr": reference(*range_expression), "EndPos": node.span.end,
                }),
            ),
            Expr::Subquery {
                expression,
                range,
                range_expression,
                step,
                step_expression,
                ..
            } => (
                "SubqueryExpr",
                json!({
                    "Expr": reference(Some(*expression)), "Range": range, "RangeExpr": reference(*range_expression),
                    "Step": step, "StepExpr": reference(*step_expression), "EndPos": node.span.end,
                }),
            ),
            Expr::Paren(child) => (
                "ParenExpr",
                json!({"Expr": reference(Some(*child)), "PosRange": span(node.span)}),
            ),
            Expr::Unary {
                operator,
                expression,
            } => (
                "UnaryExpr",
                json!({"Op": *operator as u32, "Expr": reference(Some(*expression)), "StartPos": node.span.start}),
            ),
            Expr::Duration {
                operator,
                left,
                right,
                wrapped,
                start,
                end,
            } => (
                "DurationExpr",
                json!({
                    "Op": *operator as u32, "LHS": reference(*left), "RHS": reference(*right), "Wrapped": wrapped,
                    "StartPos": start, "EndPos": end,
                }),
            ),
        };
        let modifiers = match &node.expression {
            Expr::Selector(selector) => Some(&selector.modifiers),
            Expr::Subquery { modifiers, .. } => Some(modifiers),
            _ => None,
        };
        if let Some(modifiers) = modifiers {
            fields.as_object_mut().unwrap().extend(
                modifier_fields(modifiers, &reference)
                    .as_object()
                    .unwrap()
                    .clone(),
            );
        }
        nodes.push(json!({"node": name, "fields": fields}));
    }
    let ast = parsed.root.map_or(
        Value::Null,
        |root| json!({"root": reference(Some(root)), "nodes": nodes}),
    );
    let errors: Vec<_> = parsed
        .errors
        .into_iter()
        .map(|error| {
            json!({
                "start": error.span.start, "end": error.span.end, "message": bytes(&error.message),
                "line_offset": 0, "query_ref": "input",
            })
        })
        .collect();
    json!({"id": id, "ast": ast, "errors": errors})
}
