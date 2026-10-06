// SPDX-License-Identifier: GPL-3.0-or-later
// Logical evaluation scope derives from pinned promql/engine.go preprocessing; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    frontend::{
        ast::{Expr, NodeId, ParsedExpression},
        token as tokens,
    },
    storage::{Control, Interrupted},
};
use std::cmp::Ordering;
use tokens::TokenKind;

pub(crate) fn at_modifier_unsafe(name: &str) -> bool {
    matches!(
        name,
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
    )
}

pub(crate) struct Preprocessing {
    pub(crate) visited: AccountedVec<bool>,
    pub(crate) invariants: AccountedVec<bool>,
    pub(crate) boundaries: AccountedVec<bool>,
}
impl Preprocessing {
    pub(crate) fn child(
        &self,
        expression: &ParsedExpression,
        parent: NodeId,
        child: NodeId,
    ) -> NodeId {
        if self.visited[parent.0]
            && matches!(
                expression.arena[parent].expression,
                Expr::Aggregate { .. } | Expr::Call { .. }
            )
        {
            expression.arena.unparenthesized(child)
        } else {
            child
        }
    }
}

/// Retain source helper visits and virtual edges without mutating the typed AST.
pub(crate) fn preprocess(
    expression: &ParsedExpression,
    kind: MemoryKind,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Preprocessing, Interrupted> {
    let mut result = Preprocessing {
        visited: AccountedVec::new(account, kind),
        invariants: AccountedVec::new(account, kind),
        boundaries: AccountedVec::new(account, kind),
    };
    for _ in &expression.arena.nodes {
        control.check()?;
        account.work(1);
        result.visited.push(false);
        result.invariants.push(false);
        result.boundaries.push(false);
    }
    let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
    tasks.push((expression.root, false));
    while let Some((node, finish)) = tasks.pop() {
        control.check()?;
        account.work(1);
        let value = &expression.arena[node].expression;
        if !finish {
            result.visited[node.0] = true;
            tasks.push((node, true));
            if let Expr::Aggregate {
                expression: Some(child),
                ..
            } = value
            {
                // The source strips Param's outer parentheses but does not visit its subtree.
                tasks.push((expression.arena.unparenthesized(*child), false));
            } else {
                tasks.extend(
                    value
                        .children()
                        .rev()
                        .map(|child| (result.child(expression, node, child), false)),
                );
            }
            continue;
        }
        let child = |child| result.child(expression, node, child);
        result.invariants[node.0] = match value {
            Expr::Number { .. } | Expr::String(_) | Expr::Duration { .. } => true,
            Expr::Selector(selector) => {
                selector.modifiers.timestamp.is_some() || selector.modifiers.start_or_end.is_some()
            }
            Expr::Subquery { modifiers, .. } => {
                modifiers.timestamp.is_some() || modifiers.start_or_end.is_some()
            }
            Expr::Aggregate {
                expression: Some(inner),
                ..
            } => result.invariants[child(*inner).0],
            Expr::Matrix {
                selector: inner, ..
            }
            | Expr::Paren(inner)
            | Expr::Unary {
                expression: inner, ..
            } => result.invariants[inner.0],
            Expr::Binary { left, right, .. } => {
                result.invariants[left.0] && result.invariants[right.0]
            }
            Expr::Call {
                function: Some(function),
                arguments,
            } => {
                let mut invariant = !at_modifier_unsafe(function.name);
                let mut fixed_timestamp = function.name == "timestamp";
                for (index, arg) in arguments.iter().enumerate() {
                    let arg = child(*arg);
                    if function.name == "info" && index == 1 {
                        continue;
                    }
                    invariant &= result.invariants[arg.0];
                    fixed_timestamp &= result.invariants[arg.0]
                        && matches!(expression.arena[arg].expression, Expr::Selector(_));
                }
                invariant || fixed_timestamp
            }
            _ => false,
        };
        let should_wrap = |child: NodeId| {
            result.invariants[child.0]
                && !matches!(
                    expression.arena[child].expression,
                    Expr::Matrix { .. }
                        | Expr::Subquery { .. }
                        | Expr::Number { .. }
                        | Expr::String(_)
                )
                && !matches!(&expression.arena[child].expression,
                    Expr::Call { function: Some(function), .. } if matches!(function.name, "start" | "end" | "range" | "step"))
        };
        match value {
            Expr::Binary { left, right, .. } if !result.invariants[node.0] => {
                let left_wrap = should_wrap(*left);
                let right_wrap = should_wrap(*right);
                result.boundaries[left.0] |= left_wrap;
                result.boundaries[right.0] |= right_wrap;
            }
            Expr::Call {
                function: Some(function),
                arguments,
            } if !result.invariants[node.0] => {
                for (index, arg) in arguments.iter().enumerate() {
                    if function.name == "info" && index == 1 {
                        continue;
                    }
                    let arg = result.child(expression, node, *arg);
                    let wrap = should_wrap(arg);
                    result.boundaries[arg.0] |= wrap;
                }
            }
            Expr::Subquery {
                expression: inner, ..
            } => {
                result.boundaries[inner.0] |= result.invariants[inner.0];
            }
            _ => (),
        }
    }
    let root = expression.root;
    result.boundaries[root.0] |= result.invariants[root.0]
        && !matches!(
            expression.arena[root].expression,
            Expr::Matrix { .. } | Expr::Subquery { .. } | Expr::Number { .. } | Expr::String(_)
        )
        && !matches!(&expression.arena[root].expression,
            Expr::Call { function: Some(function), .. } if matches!(function.name, "start" | "end" | "range" | "step"));
    Ok(result)
}

/// Metadata sorting is interruptible and uses no retained auxiliary array.
pub(crate) fn sort<T>(
    values: &mut [T],
    control: &Control,
    account: &QueryAccounting,
    compare: impl FnMut(&T, &T) -> Ordering,
) -> Result<(), Interrupted> {
    sort_with(values, Some(control), Some(account), compare)
}

pub(crate) fn sort_with<T>(
    values: &mut [T],
    control: Option<&Control>,
    account: Option<&QueryAccounting>,
    mut compare: impl FnMut(&T, &T) -> Ordering,
) -> Result<(), Interrupted> {
    sort_by_with(values, control, account, |left, right| {
        Ok(compare(left, right))
    })
}

pub(crate) fn sort_by_with<T>(
    values: &mut [T],
    control: Option<&Control>,
    account: Option<&QueryAccounting>,
    mut compare: impl FnMut(&T, &T) -> Result<Ordering, Interrupted>,
) -> Result<(), Interrupted> {
    fn check(
        control: Option<&Control>,
        account: Option<&QueryAccounting>,
    ) -> Result<(), Interrupted> {
        if let Some(control) = control {
            control.check()?;
        }
        if let Some(account) = account {
            account.work(1);
        }
        Ok(())
    }
    fn sift<T>(
        values: &mut [T],
        mut root: usize,
        end: usize,
        control: Option<&Control>,
        account: Option<&QueryAccounting>,
        compare: &mut impl FnMut(&T, &T) -> Result<Ordering, Interrupted>,
    ) -> Result<(), Interrupted> {
        loop {
            check(control, account)?;
            let child = root * 2 + 1;
            if child >= end {
                return Ok(());
            }
            let largest = if child + 1 < end && compare(&values[child], &values[child + 1])?.is_lt()
            {
                child + 1
            } else {
                child
            };
            if !compare(&values[root], &values[largest])?.is_lt() {
                return Ok(());
            }
            values.swap(root, largest);
            root = largest;
        }
    }
    if let Some(control) = control {
        control.check()?;
    }
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
