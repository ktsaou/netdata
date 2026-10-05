// SPDX-License-Identifier: GPL-3.0-or-later
// Logical evaluation scope derives from pinned promql/engine.go preprocessing; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    frontend::{
        ast::{Expr, ParsedExpression},
        token as tokens,
    },
    storage::{Control, Interrupted},
};
use std::cmp::Ordering;
use tokens::TokenKind;

pub(crate) fn invariants(
    expression: &ParsedExpression,
    control: &Control,
    account: &QueryAccounting,
) -> Result<AccountedVec<bool>, Interrupted> {
    let mut result = AccountedVec::new(account, MemoryKind::Compiled);
    result.extend((0..expression.arena.nodes.len()).map(|_| false));
    let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
    tasks.push((expression.root, false));
    while let Some((node, finish)) = tasks.pop() {
        control.check()?;
        account.work(1);
        let value = &expression.arena[node].expression;
        if !finish {
            tasks.push((node, true));
            tasks.extend(value.children().rev().map(|child| (child, false)));
            continue;
        }
        result[node.0] = match value {
            Expr::Number { .. } | Expr::String(_) | Expr::Duration { .. } => true,
            Expr::Selector(selector) => {
                selector.modifiers.timestamp.is_some() || selector.modifiers.start_or_end.is_some()
            }
            Expr::Subquery { modifiers, .. } => {
                modifiers.timestamp.is_some() || modifiers.start_or_end.is_some()
            }
            Expr::Aggregate {
                expression: Some(child),
                ..
            }
            | Expr::Matrix {
                selector: child, ..
            }
            | Expr::Paren(child)
            | Expr::Unary {
                expression: child, ..
            } => result[child.0],
            Expr::Binary { left, right, .. } => result[left.0] && result[right.0],
            Expr::Call {
                function: Some(function),
                arguments,
            } => {
                let unsafe_function = matches!(
                    function.name,
                    "days_in_month"
                        | "day_of_month"
                        | "day_of_week"
                        | "day_of_year"
                        | "hour"
                        | "minute"
                        | "month"
                        | "year"
                        | "predict_linear"
                        | "time"
                        | "timestamp"
                );
                let fixed_timestamp = function.name == "timestamp"
                    && arguments.iter().all(|&arg| {
                        let child = expression.arena.unparenthesized(arg);
                        result[child.0]
                            && matches!(expression.arena[child].expression, Expr::Selector(_))
                    });
                fixed_timestamp
                    || (!unsafe_function && arguments.iter().all(|child| result[child.0]))
            }
            _ => false,
        };
    }
    Ok(result)
}

pub(crate) fn invariant_boundaries(
    expression: &ParsedExpression,
    invariants: &[bool],
    control: &Control,
    account: &QueryAccounting,
) -> Result<AccountedVec<bool>, Interrupted> {
    let mut result = AccountedVec::new(account, MemoryKind::Compiled);
    result.extend((0..expression.arena.nodes.len()).map(|_| false));
    let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
    tasks.push((expression.root, false));
    while let Some((node, parent_invariant)) = tasks.pop() {
        control.check()?;
        account.work(1);
        let value = &expression.arena[node].expression;
        let invariant = invariants[node.0];
        // Preprocessing wraps the highest safe expression, not each invariant descendant.
        result[node.0] = invariant
            && !parent_invariant
            && !matches!(
                value,
                Expr::Matrix { .. } | Expr::Number { .. } | Expr::String(_)
            );
        tasks.extend(value.children().rev().map(|child| (child, invariant)));
    }
    Ok(result)
}

/// Metadata sorting is interruptible and uses no retained auxiliary array.
pub(crate) fn sort<T>(
    values: &mut [T],
    control: &Control,
    account: &QueryAccounting,
    mut compare: impl FnMut(&T, &T) -> Ordering,
) -> Result<(), Interrupted> {
    fn sift<T>(
        values: &mut [T],
        mut root: usize,
        end: usize,
        control: &Control,
        account: &QueryAccounting,
        compare: &mut impl FnMut(&T, &T) -> Ordering,
    ) -> Result<(), Interrupted> {
        loop {
            control.check()?;
            account.work(1);
            let child = root * 2 + 1;
            if child >= end {
                return Ok(());
            }
            let largest = if child + 1 < end && compare(&values[child], &values[child + 1]).is_lt()
            {
                child + 1
            } else {
                child
            };
            if !compare(&values[root], &values[largest]).is_lt() {
                return Ok(());
            }
            values.swap(root, largest);
            root = largest;
        }
    }
    control.check()?;
    for root in (0..values.len() / 2).rev() {
        sift(values, root, values.len(), control, account, &mut compare)?;
    }
    for end in (1..values.len()).rev() {
        values.swap(0, end);
        sift(values, 0, end, control, account, &mut compare)?;
    }
    Ok(())
}

pub(crate) fn unary_merge(expression: &Expr, delayed: bool) -> bool {
    !delayed
        && matches!(
            expression,
            Expr::Unary {
                operator: TokenKind::SUB,
                ..
            }
        )
}
