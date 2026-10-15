// SPDX-License-Identifier: GPL-3.0-or-later
//! Absence label inference mirrors pinned Prometheus promql/functions.go:2848-2880.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    frontend::ast::{Expr, MatchType, NodeId, ParsedExpression},
    result::OwnedLabels,
    storage::{Control, Interrupted},
};

pub(crate) fn labels(
    expression: &ParsedExpression,
    argument: NodeId,
    account: &QueryAccounting,
    control: &Control,
) -> Result<OwnedLabels, Interrupted> {
    let mut labels = OwnedLabels::new(account, MemoryKind::Workspace);
    let selector = match &expression.arena[argument].expression {
        Expr::Selector(selector) => selector,
        Expr::Matrix { selector, .. } => {
            let Expr::Selector(selector) = &expression.arena[*selector].expression else {
                unreachable!("matrix selector")
            };
            selector
        }
        _ => return Ok(labels),
    };
    let mut equalities = AccountedVec::<&[u8]>::new(account, MemoryKind::Workspace);
    for matcher in selector.matchers.iter().flatten() {
        control.check()?;
        account.work(1);
        if matcher.name == b"__name__" {
            continue;
        }
        let mut seen = false;
        for name in &*equalities {
            control.check()?;
            account.work(1);
            if *name == &*matcher.name {
                seen = true;
                break;
            }
        }
        if matcher.kind == MatchType::Equal && !seen {
            labels.set(
                &matcher.name,
                &matcher.value,
                account,
                MemoryKind::Workspace,
                control,
            )?;
            // Empty equality also prevents a later equality from restoring this label.
            equalities.push(&matcher.name);
        } else {
            labels.set(&matcher.name, b"", account, MemoryKind::Workspace, control)?;
        }
    }
    Ok(labels)
}
