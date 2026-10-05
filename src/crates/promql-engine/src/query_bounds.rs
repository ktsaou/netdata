// SPDX-License-Identifier: GPL-3.0-or-later
// Request read scopes derive from pinned PromQL FindMinMaxTime; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    evaluator::EvaluationOptions,
    frontend::ast::{Expr, ParsedExpression},
    query_preparation::{Prepared, SelectorReference},
    storage::{Control, Interrupted, TimeBounds},
};

#[derive(Clone, Copy, Default)]
struct Scope {
    offset_ns: i64,
    range_ns: i64,
    fixed_ms: Option<i64>,
    eval_range_ns: i64,
}

fn selector_bounds(
    options: EvaluationOptions,
    scope: Scope,
    reference: SelectorReference,
    anchored: bool,
    smoothed: bool,
) -> TimeBounds {
    let (mut start_ms, mut end_ms) = if let Some(time) = reference.timestamp_ms {
        (time, time)
    } else {
        let start = scope.fixed_ms.unwrap_or(options.start_ms);
        let end = scope.fixed_ms.unwrap_or(options.end_ms);
        (
            start
                .wrapping_sub(scope.offset_ns / 1_000_000)
                .wrapping_sub(scope.range_ns / 1_000_000),
            end.wrapping_sub(scope.offset_ns / 1_000_000),
        )
    };
    let range_ms = if scope.eval_range_ns == 0 {
        options.lookback_ms
    } else if anchored || smoothed {
        scope
            .eval_range_ns
            .wrapping_add(options.lookback_ms.wrapping_mul(1_000_000))
            / 1_000_000
    } else {
        scope.eval_range_ns / 1_000_000
    };
    start_ms = start_ms.wrapping_sub(range_ms.wrapping_sub(1));
    if smoothed {
        end_ms = end_ms.wrapping_add(options.lookback_ms);
    }
    let offset_ms = reference.offset_ns / 1_000_000;
    TimeBounds {
        start_ms: start_ms.wrapping_sub(offset_ms),
        end_ms: end_ms.wrapping_sub(offset_ms),
    }
}

fn include(bounds: &mut Option<TimeBounds>, value: TimeBounds) {
    if let Some(bounds) = bounds {
        bounds.start_ms = bounds.start_ms.min(value.start_ms);
        bounds.end_ms = bounds.end_ms.max(value.end_ms);
    } else {
        *bounds = Some(value);
    }
}

pub(crate) fn find(
    expression: &ParsedExpression,
    options: EvaluationOptions,
    prepared: &Prepared,
    control: &Control,
    account: &QueryAccounting,
) -> Result<TimeBounds, Interrupted> {
    let options = EvaluationOptions {
        start_ms: prepared.context.start_ms,
        end_ms: prepared.context.end_ms,
        ..options
    };
    let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
    tasks.push((expression.root, Scope::default()));
    let mut bounds = None::<TimeBounds>;
    while let Some((node, mut scope)) = tasks.pop() {
        control.check()?;
        account.work(1);
        match &expression.arena[node].expression {
            Expr::Selector(selector) => {
                include(
                    &mut bounds,
                    selector_bounds(
                        options,
                        scope,
                        SelectorReference {
                            timestamp_ms: prepared.timestamp(&selector.modifiers),
                            offset_ns: prepared.offset(node, selector.modifiers.original_offset),
                        },
                        selector.anchored,
                        selector.smoothed,
                    ),
                );
            }
            Expr::Matrix {
                selector, range, ..
            } => {
                scope.eval_range_ns = prepared.range(node, *range);
                tasks.push((*selector, scope));
            }
            Expr::Subquery {
                expression,
                range,
                modifiers,
                ..
            } => {
                let offset = prepared.offset(node, modifiers.original_offset);
                let range = prepared.range(node, *range);
                scope.offset_ns = scope.offset_ns.wrapping_add(offset);
                scope.range_ns = scope.range_ns.wrapping_add(range);
                if let Some(time) = prepared.timestamp(modifiers) {
                    scope.offset_ns = offset;
                    scope.range_ns = range;
                    scope.fixed_ms = (time != i64::MAX).then_some(time);
                }
                tasks.push((*expression, scope));
            }
            Expr::Call {
                function: Some(function),
                arguments,
            } if function.name == "info" => {
                let reference =
                    prepared.info_reference(expression, arguments[0], control, account)?;
                let mut info_scope = scope;
                info_scope.eval_range_ns = 0;
                include(
                    &mut bounds,
                    selector_bounds(options, info_scope, reference, false, false),
                );
                tasks.extend(arguments.iter().rev().map(|child| (*child, scope)));
            }
            value => tasks.extend(value.children().rev().map(|child| (child, scope))),
        }
    }
    Ok(bounds
        .filter(|bounds| bounds.end_ms != i64::MIN)
        .unwrap_or(TimeBounds {
            start_ms: 0,
            end_ms: 0,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        frontend::{ast::ParserOptions, parser, regex::Selector},
    };

    #[test]
    fn info_read_bounds_include_implicit_selection_in_original_scopes() {
        for (query, start_ms, end_ms, expected) in [
            ("info(last_over_time(m[1ms]))", 4000, 4000, (2001, 4000)),
            ("info(m @1 or n @3)", 1000, 4500, (-999, 4500)),
            (
                "last_over_time(info(m @1 or n @3)[2s:1s] offset 1s)",
                1000,
                4500,
                (-3999, 3500),
            ),
        ] {
            let parsed = parser::parse(
                query.as_bytes(),
                ParserOptions {
                    experimental_functions: true,
                    ..ParserOptions::default()
                },
                |pattern| Selector::compile_in(pattern, None),
            );
            assert!(parsed.errors.is_empty(), "{query}: {:?}", parsed.errors);
            let expression = ParsedExpression {
                source: query.as_bytes().into(),
                arena: parsed.arena,
                root: parsed.root.unwrap(),
            };
            let account = EngineAccounting::default().query();
            let control = Control::default();
            let prepared = Prepared::new(
                &expression,
                crate::result::OutputHeader {
                    value_type: crate::frontend::ast::ValueType::Vector,
                    start_ms,
                    end_ms,
                    step_ms: 1000,
                },
                &control,
                &account,
            )
            .unwrap();
            let bounds = find(
                &expression,
                EvaluationOptions {
                    time_ms: start_ms,
                    start_ms,
                    end_ms,
                    step_ms: 1000,
                    lookback_ms: 2000,
                    start_timestamps: false,
                    delayed_name_removal: false,
                },
                &prepared,
                &control,
                &account,
            )
            .unwrap();
            assert_eq!((bounds.start_ms, bounds.end_ms), expected, "{query}");
            drop(prepared);
            assert!(
                account
                    .snapshot()
                    .memory
                    .iter()
                    .all(|memory| memory.current == 0)
            );
        }
    }

    #[test]
    fn static_scopes_union_all_selectors_and_use_request_endpoints() {
        let options = EvaluationOptions {
            time_ms: 1000,
            start_ms: 1000,
            end_ms: 5007,
            step_ms: 4000,
            lookback_ms: 500,
            start_timestamps: false,
            delayed_name_removal: false,
        };
        for (query, start_ms, end_ms) in [
            ("0", 0, 0),
            ("m", 501, 5007),
            ("m offset 1ms", 500, 5006),
            ("m[2s]", -999, 5007),
            ("m @ end() offset 1ms", 4507, 5006),
            ("sum_over_time((m @ start())[2s:1s] offset 1s)", 501, 1000),
            ("sum_over_time(m[2s:1s] @ end() offset 1s)", 1508, 4007),
            (
                "sum_over_time(sum_over_time(m[2s:1s] offset 1s)[3s:1s] offset 2s)",
                -7499,
                2007,
            ),
            (
                "sum_over_time(sum_over_time(m[2s:1s] @ start() offset 1s)[3s:1s] offset 2s)",
                -2499,
                0,
            ),
            ("m + scalar(anchor offset 1ms)", 500, 5007),
        ] {
            let parsed = parser::parse(query.as_bytes(), ParserOptions::default(), |pattern| {
                Selector::compile_in(pattern, None)
            });
            assert!(parsed.errors.is_empty(), "{query}: {:?}", parsed.errors);
            let expression = ParsedExpression {
                source: query.as_bytes().into(),
                arena: parsed.arena,
                root: parsed.root.unwrap(),
            };
            let account = EngineAccounting::default().query();
            let prepared = Prepared::new(
                &expression,
                crate::result::OutputHeader {
                    value_type: crate::frontend::ast::ValueType::Matrix,
                    start_ms: options.start_ms,
                    end_ms: options.end_ms,
                    step_ms: options.step_ms,
                },
                &Control::default(),
                &account,
            )
            .unwrap();
            let actual = find(
                &expression,
                options,
                &prepared,
                &Control::default(),
                &account,
            )
            .unwrap();
            assert_eq!(
                (actual.start_ms, actual.end_ms),
                (start_ms, end_ms),
                "{query}"
            );
            assert!(
                account
                    .snapshot()
                    .memory
                    .iter()
                    .all(|memory| memory.current == 0)
            );
        }
    }
}
