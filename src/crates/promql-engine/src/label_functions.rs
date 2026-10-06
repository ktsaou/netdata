// SPDX-License-Identifier: GPL-3.0-or-later
//! Whole-series label operations derive from pinned promql/functions.go:2483-2571.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    evaluator::EvaluationError,
    evaluator_roster::Descriptor,
    frontend::{
        ast::{Expr, NodeId, ParsedExpression},
        regex::GoRegex,
    },
    query_preparation::Prepared,
    result::append_bytes_checked,
    storage::Control,
};

pub(crate) fn is_label_call(expression: &Expr) -> bool {
    matches!(expression, Expr::Call { function: Some(function), .. }
        if matches!(function.name, "label_replace" | "label_join"))
}

pub(crate) enum LabelOperation<'a> {
    Replace {
        destination: &'a [u8],
        replacement: &'a [u8],
        source: &'a [u8],
        regex: GoRegex,
    },
    Join {
        destination: &'a [u8],
        separator: &'a [u8],
        sources: AccountedVec<&'a [u8]>,
    },
}

fn runtime_error(
    prefix: &[u8],
    value: &[u8],
    account: &QueryAccounting,
    control: &Control,
) -> Result<EvaluationError, EvaluationError> {
    let mut message = AccountedVec::new(account, MemoryKind::Workspace);
    append_bytes_checked(&mut message, prefix, account, control)?;
    append_bytes_checked(&mut message, value, account, control)?;
    Ok(EvaluationError::Runtime(message))
}

fn valid_name(
    name: &[u8],
    account: &QueryAccounting,
    control: &Control,
) -> Result<bool, EvaluationError> {
    for chunk in name.chunks(1024) {
        control.check()?;
        account.work(chunk.len() as u64);
    }
    Ok(!name.is_empty() && std::str::from_utf8(name).is_ok())
}

impl<'a> LabelOperation<'a> {
    pub(crate) fn prepare(
        expression: &'a ParsedExpression,
        node: NodeId,
        prepared: &Prepared,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<AccountedBox<Self>, EvaluationError> {
        let Expr::Call {
            function: Some(function),
            arguments,
        } = &expression.arena[node].expression
        else {
            unreachable!("label call")
        };
        let string = |index| {
            let argument = prepared
                .preprocessing
                .child(expression, node, arguments[index]);
            if let Expr::String(value) = &expression.arena[argument].expression {
                Ok(&value[..])
            } else {
                // Unvisited aggregation parameters keep their source assertion branch open.
                Err(EvaluationError::Unsupported(
                    "unpreprocessed label string argument",
                ))
            }
        };
        let context = AllocationContext::new(account, MemoryKind::Workspace);
        let operation = if function.name == "label_replace" {
            let destination = string(1)?;
            let replacement = string(2)?;
            let source = string(3)?;
            let pattern = string(4)?;
            control.check()?;
            account.work(pattern.len() as u64);
            let compiled = GoRegex::compile_label_replace_in(pattern, Some(&context));
            control.check()?;
            let regex = match compiled {
                Ok(regex) => regex,
                Err(_) => {
                    return Err(runtime_error(
                        b"invalid regular expression in label_replace(): ",
                        pattern,
                        account,
                        control,
                    )?);
                }
            };
            if !valid_name(destination, account, control)? {
                return Err(runtime_error(
                    b"invalid destination label name in label_replace(): ",
                    destination,
                    account,
                    control,
                )?);
            }
            Self::Replace {
                destination,
                replacement,
                source,
                regex,
            }
        } else {
            let destination = string(1)?;
            let separator = string(2)?;
            let mut sources = AccountedVec::new(account, MemoryKind::Workspace);
            for index in 3..arguments.len() {
                control.check()?;
                account.work(1);
                let source = string(index)?;
                if !valid_name(source, account, control)? {
                    return Err(runtime_error(
                        b"invalid source label name in label_join(): ",
                        source,
                        account,
                        control,
                    )?);
                }
                sources.push(source);
            }
            if !valid_name(destination, account, control)? {
                return Err(runtime_error(
                    b"invalid destination label name in label_join(): ",
                    destination,
                    account,
                    control,
                )?);
            }
            Self::Join {
                destination,
                separator,
                sources,
            }
        };
        Ok(AccountedBox::new(operation, &context))
    }

    pub(crate) fn apply(
        &self,
        descriptor: &mut Descriptor,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<(), EvaluationError> {
        control.check()?;
        account.work(1);
        let context = AllocationContext::new(account, MemoryKind::Workspace);
        let (destination, value) = match self {
            Self::Replace {
                destination,
                replacement,
                source,
                regex,
            } => {
                let original = descriptor.labels.get_checked(source, account, control)?;
                account.work(original.len() as u64);
                let captures = regex.captures_in(original, Some(&context));
                control.check()?;
                let Some(captures) = captures else {
                    return Ok(());
                };
                account.work(replacement.len() as u64);
                let value = regex.expand_in(replacement, original, &captures, Some(&context));
                control.check()?;
                (*destination, value)
            }
            Self::Join {
                destination,
                separator,
                sources,
            } => {
                let mut value = AccountedVec::new(account, MemoryKind::Workspace);
                for (index, source) in sources.iter().enumerate() {
                    control.check()?;
                    account.work(1);
                    if index != 0 {
                        append_bytes_checked(&mut value, separator, account, control)?;
                    }
                    let original = descriptor.labels.get_checked(source, account, control)?;
                    append_bytes_checked(&mut value, original, account, control)?;
                }
                (*destination, value)
            }
        };
        descriptor.labels.reset_checked(account, control)?;
        descriptor
            .labels
            .set(destination, &value, account, MemoryKind::Workspace, control)?;
        if destination == b"__name__" {
            descriptor.drop_name = false;
        }
        Ok(())
    }
}
