// SPDX-License-Identifier: GPL-3.0-or-later
//! Static proofs for one scheduled selector; ambiguous expressions retain ordinary replay.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    evaluator::{EvaluationError, PointwiseFunction, drop_reserved},
    evaluator_roster::{Descriptor, Rosters},
    frontend::{
        ast::{Expr, NodeId, ParsedExpression, ValueType},
        token::TokenKind,
    },
    query_preparation::Prepared,
    storage::{Control, Interrupted},
};

pub(crate) fn fixed_quantile_vector(expression: &ParsedExpression, node: NodeId) -> Option<NodeId> {
    let Expr::Call {
        function: Some(function),
        arguments,
    } = &expression.arena[node].expression
    else {
        return None;
    };
    (function.name == "histogram_quantile"
        && arguments.len() == 2
        && matches!(
            expression.arena[arguments[0]].expression,
            Expr::Number { .. }
        ))
    .then(|| arguments[1])
}

pub(crate) fn unique_ratio_operands(
    expression: &ParsedExpression,
    node: NodeId,
) -> Option<[NodeId; 2]> {
    let Expr::Binary {
        operator: TokenKind::DIV,
        left,
        right,
        matching: Some(matching),
        ..
    } = &expression.arena[node].expression
    else {
        return None;
    };
    (matching.cardinality == crate::frontend::ast::Cardinality::OneToOne
        && matching.fill_left.is_none()
        && matching.fill_right.is_none())
    .then_some([*left, *right])
}

pub(crate) fn vector_child(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<NodeId>, Interrupted> {
    let child = match &expression.arena[node].expression {
        Expr::Paren(child)
        | Expr::Unary {
            expression: child, ..
        } => *child,
        Expr::Binary { left, right, .. } => {
            match (
                expression.arena[*left].value_type,
                expression.arena[*right].value_type,
            ) {
                (ValueType::Vector, ValueType::Scalar) => *left,
                (ValueType::Scalar, ValueType::Vector) => *right,
                _ => return Ok(None),
            }
        }
        Expr::Call {
            function: Some(function),
            arguments,
        } if PointwiseFunction::parse(function.name).is_some()
            || matches!(function.name, "label_join" | "label_replace") =>
        {
            let Some(&argument) = arguments.first() else {
                return Ok(None);
            };
            argument
        }
        _ => return Ok(None),
    };
    if expression.arena[child].value_type == ValueType::Vector {
        Ok(Some(
            prepared
                .preprocessing
                .child(expression, node, child, control, account)?,
        ))
    } else {
        Ok(None)
    }
}

pub(crate) fn fixed_map_vector(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<NodeId>, Interrupted> {
    let child = match &expression.arena[node].expression {
        Expr::Unary {
            operator: TokenKind::ADD | TokenKind::SUB,
            expression: child,
        } => *child,
        Expr::Call {
            function: Some(function),
            arguments,
        } if matches!(
            PointwiseFunction::parse(function.name),
            Some(PointwiseFunction::Float(..))
        ) =>
        {
            let Some((&child, parameters)) = arguments.split_first() else {
                return Ok(None);
            };
            if !parameters.iter().all(|parameter| {
                matches!(expression.arena[*parameter].expression, Expr::Number { .. })
            }) {
                return Ok(None);
            }
            child
        }
        Expr::Binary {
            operator,
            left,
            right,
            ..
        } if operator.is_comparison()
            || matches!(
                operator,
                TokenKind::ADD
                    | TokenKind::SUB
                    | TokenKind::MUL
                    | TokenKind::DIV
                    | TokenKind::MOD
                    | TokenKind::POW
                    | TokenKind::ATAN2
            ) =>
        {
            match (
                expression.arena[*left].value_type,
                expression.arena[*right].value_type,
            ) {
                (ValueType::Vector, ValueType::Scalar)
                    if matches!(expression.arena[*right].expression, Expr::Number { .. }) =>
                {
                    *left
                }
                (ValueType::Scalar, ValueType::Vector)
                    if matches!(expression.arena[*left].expression, Expr::Number { .. }) =>
                {
                    *right
                }
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    if expression.arena[child].value_type != ValueType::Vector {
        return Ok(None);
    }
    let child = prepared
        .preprocessing
        .child(expression, node, child, control, account)?;
    // Qualified ratios use their separate successful-prefix proof under Float calls.
    if matches!(expression.arena[node].expression, Expr::Call { .. })
        && !matches!(expression.arena[child].expression, Expr::Aggregate { .. })
        && unique_ratio_operands(expression, child).is_none()
    {
        return Ok(None);
    }
    Ok(Some(child))
}

pub(crate) fn literal_scalar_child(
    expression: &ParsedExpression,
    prepared: &Prepared,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<(NodeId, u8)>, Interrupted> {
    let Expr::Binary { left, right, .. } = expression.arena[node].expression else {
        return Ok(None);
    };
    if expression.arena[left].value_type != ValueType::Scalar
        || expression.arena[right].value_type != ValueType::Scalar
    {
        return Ok(None);
    }
    let (child, phase) = match (
        matches!(expression.arena[left].expression, Expr::Number { .. }),
        matches!(expression.arena[right].expression, Expr::Number { .. }),
    ) {
        (false, true) => (left, 0),
        (true, false) => (right, 1),
        _ => return Ok(None),
    };
    // Keep the actual operand edge; scalar arithmetic still evaluates both sides.
    Ok(Some((
        prepared
            .preprocessing
            .child(expression, node, child, control, account)?,
        phase,
    )))
}

fn pure_scalar(
    expression: &ParsedExpression,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    let mut pending = AccountedVec::new(account, MemoryKind::Workspace);
    pending.push(node);
    while let Some(node) = pending.pop() {
        control.check()?;
        account.work(1);
        if expression.arena[node].value_type != ValueType::Scalar {
            return Ok(false);
        }
        match &expression.arena[node].expression {
            Expr::Number { .. } => (),
            Expr::Paren(child)
            | Expr::Unary {
                expression: child, ..
            } => pending.push(*child),
            Expr::Binary {
                left,
                right,
                operator,
                ..
            } if !matches!(operator, TokenKind::TRIM_LOWER | TokenKind::TRIM_UPPER) => {
                pending.push(*right);
                pending.push(*left);
            }
            Expr::Call {
                function: Some(function),
                arguments,
            } if function.name == "time" && arguments.is_empty() => (),
            _ => return Ok(false),
        }
    }
    Ok(true)
}

pub(crate) fn candidate(
    expression: &ParsedExpression,
    prepared: &Prepared,
    rosters: &Rosters,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<(NodeId, NodeId)>, Interrupted> {
    let Some(child) = vector_child(expression, prepared, node, control, account)? else {
        return Ok(None);
    };
    let Some(selector) = rosters.scheduled_selector(child) else {
        return Ok(None);
    };
    Ok(pure_map(expression, node, control, account)?.then_some((child, selector)))
}

pub(crate) fn pure_map(
    expression: &ParsedExpression,
    node: NodeId,
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    match &expression.arena[node].expression {
        Expr::Paren(_) | Expr::Unary { .. } => (),
        Expr::Binary {
            left,
            right,
            operator,
            ..
        } => {
            if matches!(operator, TokenKind::TRIM_LOWER | TokenKind::TRIM_UPPER) {
                return Ok(false);
            }
            let scalar = if expression.arena[*left].value_type == ValueType::Scalar {
                *left
            } else {
                *right
            };
            if !pure_scalar(expression, scalar, control, account)? {
                return Ok(false);
            }
        }
        Expr::Call {
            function: Some(function),
            arguments,
        } => {
            let Some(function) = PointwiseFunction::parse(function.name) else {
                return Ok(false);
            };
            if matches!(
                function,
                PointwiseFunction::HistogramStdVar | PointwiseFunction::HistogramStdDev
            ) {
                return Ok(false);
            }
            for argument in &arguments[1..] {
                if !pure_scalar(expression, *argument, control, account)? {
                    return Ok(false);
                }
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub(crate) fn unique(
    mut hashes: AccountedVec<u64>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    crate::evaluator_order::sort(&mut hashes, control, account, u64::cmp)?;
    for pair in hashes.windows(2) {
        control.check()?;
        account.work(1);
        if pair[0] == pair[1] {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn unique_rows(
    rows: &[Descriptor],
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    let mut hashes = AccountedVec::new(account, MemoryKind::Workspace);
    for row in rows {
        control.check()?;
        account.work(1);
        if row.heads.multiple() || row.sources.len() > 1 {
            return Ok(false);
        }
        hashes.push(row.hash);
    }
    unique(hashes, control, account)
}

pub(crate) fn unique_map(
    expression: &ParsedExpression,
    node: NodeId,
    rows: &[Descriptor],
    delayed: bool,
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, EvaluationError> {
    let removal = if delayed {
        None
    } else {
        match &expression.arena[node].expression {
            Expr::Unary {
                operator: TokenKind::SUB,
                ..
            } => Some(false),
            Expr::Binary {
                operator,
                return_bool,
                ..
            } if crate::binary_value::changes_metadata(*operator) || *return_bool => Some(false),
            Expr::Call {
                function: Some(function),
                ..
            } => PointwiseFunction::parse(function.name).map(|function| !function.drops_metadata()),
            _ => None,
        }
    };
    let Some(name_only) = removal else {
        return Ok(unique_rows(rows, control, account)?);
    };
    let mut hashes = AccountedVec::new(account, MemoryKind::Workspace);
    for row in rows {
        control.check()?;
        account.work(1);
        if row.heads.multiple() || row.sources.len() > 1 {
            return Ok(false);
        }
        let mut labels = row
            .labels
            .copy_checked(account, MemoryKind::Workspace, control)?;
        drop_reserved(&mut labels, name_only, account, control)?;
        hashes.push(crate::label_hash::full(&labels, control, account)?);
    }
    Ok(unique(hashes, control, account)?)
}
