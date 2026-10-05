// SPDX-License-Identifier: GPL-3.0-or-later
// Static read scopes derive from pinned PromQL FindMinMaxTime; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    evaluator::EvaluationOptions,
    frontend::{
        ast::{Expr, ParsedExpression, SelectorModifiers},
        token as tokens,
    },
    storage::{Control, Interrupted, TimeBounds},
};

fn fixed(modifiers: &SelectorModifiers, options: EvaluationOptions) -> Option<i64> {
    modifiers.timestamp.or(match modifiers.start_or_end {
        Some(tokens::TokenKind::START) => Some(options.start_ms),
        Some(tokens::TokenKind::END) => Some(options.end_ms),
        _ => None,
    })
}

#[derive(Clone, Copy, Default)]
struct Scope {
    offset_ns: i64,
    range_ns: i64,
    fixed_ms: Option<i64>,
    eval_range_ns: i64,
}

pub(crate) fn find(
    expression: &ParsedExpression,
    options: EvaluationOptions,
    control: &Control,
    account: &QueryAccounting,
) -> Result<TimeBounds, Interrupted> {
    let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
    tasks.push((expression.root, Scope::default()));
    let mut bounds = None::<TimeBounds>;
    while let Some((node, mut scope)) = tasks.pop() {
        control.check()?;
        account.work(1);
        match &expression.arena[node].expression {
            Expr::Selector(selector) => {
                let (mut start_ms, mut end_ms) =
                    if let Some(time) = fixed(&selector.modifiers, options) {
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
                } else if selector.anchored || selector.smoothed {
                    scope
                        .eval_range_ns
                        .wrapping_add(options.lookback_ms.wrapping_mul(1_000_000))
                        / 1_000_000
                } else {
                    scope.eval_range_ns / 1_000_000
                };
                start_ms = start_ms.wrapping_sub(range_ms.wrapping_sub(1));
                if selector.smoothed {
                    end_ms = end_ms.wrapping_add(options.lookback_ms);
                }
                let offset = selector.modifiers.original_offset / 1_000_000;
                start_ms = start_ms.wrapping_sub(offset);
                end_ms = end_ms.wrapping_sub(offset);
                if let Some(bounds) = &mut bounds {
                    bounds.start_ms = bounds.start_ms.min(start_ms);
                    bounds.end_ms = bounds.end_ms.max(end_ms);
                } else {
                    bounds = Some(TimeBounds { start_ms, end_ms });
                }
            }
            Expr::Matrix {
                selector, range, ..
            } => {
                scope.eval_range_ns = *range;
                tasks.push((*selector, scope));
            }
            Expr::Subquery {
                expression,
                range,
                modifiers,
                ..
            } => {
                scope.offset_ns = scope.offset_ns.wrapping_add(modifiers.original_offset);
                scope.range_ns = scope.range_ns.wrapping_add(*range);
                if let Some(time) = fixed(modifiers, options) {
                    scope.offset_ns = modifiers.original_offset;
                    scope.range_ns = *range;
                    scope.fixed_ms = (time != i64::MAX).then_some(time);
                }
                tasks.push((*expression, scope));
            }
            value => tasks.extend(value.children().rev().map(|child| (child, scope))),
        }
    }
    Ok(bounds.unwrap_or(TimeBounds {
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
            let actual = find(&expression, options, &Control::default(), &account).unwrap();
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
