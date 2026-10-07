// SPDX-License-Identifier: GPL-3.0-or-later
// Exact keys and projections follow pinned stringlabels and resultMetric; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    evaluator::EvaluationError,
    frontend::{
        ast::{Cardinality, VectorMatching},
        quote,
        token::TokenKind,
    },
    result::{OwnedLabels, append_bytes_checked},
    storage::{Control, Interrupted},
};
use std::fmt::{self, Write};

pub(crate) fn compare(
    left: &[u8],
    right: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<std::cmp::Ordering, Interrupted> {
    compare_with(left, right, control, Some(account))
}
pub(crate) fn compare_with(
    left: &[u8],
    right: &[u8],
    control: &Control,
    account: Option<&QueryAccounting>,
) -> Result<std::cmp::Ordering, Interrupted> {
    control.check()?;
    for (left, right) in left.chunks(1024).zip(right.chunks(1024)) {
        control.check()?;
        if let Some(account) = account {
            account.work(left.len().min(right.len()) as u64);
        }
        let order = left.cmp(right);
        if !order.is_eq() {
            return Ok(order);
        }
    }
    Ok(left.len().cmp(&right.len()))
}

pub(crate) fn same(
    left: &OwnedLabels,
    right: &OwnedLabels,
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    if left.0.len() != right.0.len() {
        return Ok(false);
    }
    for (left, right) in left.0.iter().zip(right.0.iter()) {
        if !equal(&left.name, &right.name, control, account)?
            || !equal(&left.value, &right.value, control, account)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn equal(
    left: &[u8],
    right: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    control.check()?;
    if left.len() != right.len() {
        account.work(1);
        return Ok(false);
    }
    for (left, right) in left.chunks(1024).zip(right.chunks(1024)) {
        control.check()?;
        account.work(left.len() as u64);
        if left != right {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn named(
    name: &[u8],
    names: &[AccountedVec<u8>],
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    for candidate in names {
        if equal(name, candidate, control, account)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn field(
    target: &mut AccountedVec<u8>,
    value: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), EvaluationError> {
    crate::label_encoding::field(value, control, account)?;
    let n = value.len();
    if n < 255 {
        target.push(n as u8);
    } else {
        target.extend_from_slice(&[255, n as u8, (n >> 8) as u8, (n >> 16) as u8]);
    }
    append_bytes_checked(target, value, account, control)?;
    Ok(())
}

pub(crate) fn append(
    target: &mut AccountedVec<u8>,
    labels: &OwnedLabels,
    matching: Option<&VectorMatching>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), EvaluationError> {
    append_selected(target, labels, matching, None, control, account)
}

pub(crate) fn append_without(
    target: &mut AccountedVec<u8>,
    labels: &OwnedLabels,
    excluded: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), EvaluationError> {
    append_selected(target, labels, None, Some(excluded), control, account)
}

fn append_selected(
    target: &mut AccountedVec<u8>,
    labels: &OwnedLabels,
    matching: Option<&VectorMatching>,
    excluded: Option<&[u8]>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), EvaluationError> {
    for label in &*labels.0 {
        control.check()?;
        account.work(1);
        if excluded.is_some_and(|excluded| &*label.name == excluded) {
            continue;
        }
        if let Some(matching) = matching {
            let selected = named(
                &label.name,
                matching.labels.as_deref().unwrap_or(&[]),
                control,
                account,
            )?;
            if if matching.on {
                !selected
            } else {
                selected || &*label.name == b"__name__"
            } {
                continue;
            }
        }
        field(target, &label.name, control, account)?;
        field(target, &label.value, control, account)?;
    }
    Ok(())
}

pub(crate) fn matched(
    labels: &OwnedLabels,
    matching: &VectorMatching,
    control: &Control,
    account: &QueryAccounting,
) -> Result<OwnedLabels, Interrupted> {
    let mut result = labels.copy_checked(account, MemoryKind::Workspace, control)?;
    result.reset_checked(account, control)?;
    result.retain_labels_checked(account, control, |label| {
        let selected = named(
            &label.name,
            matching.labels.as_deref().unwrap_or(&[]),
            control,
            account,
        )?;
        Ok(if matching.on {
            selected
        } else {
            !selected && &*label.name != b"__name__"
        })
    })?;
    Ok(result)
}

pub(crate) fn projected(
    left: &OwnedLabels,
    right: &OwnedLabels,
    operator: TokenKind,
    matching: &VectorMatching,
    drop_name: bool,
    control: &Control,
    account: &QueryAccounting,
) -> Result<OwnedLabels, crate::evaluator::EvaluationError> {
    let mut result = left.copy_checked(account, MemoryKind::Workspace, control)?;
    result.reset_checked(account, control)?;
    if drop_name || crate::binary_value::changes_metadata(operator) {
        crate::evaluator::drop_reserved(&mut result, false, account, control)?;
    }
    if matching.cardinality == Cardinality::OneToOne {
        result.retain_labels_checked(account, control, |label| {
            let selected = named(
                &label.name,
                matching.labels.as_deref().unwrap_or(&[]),
                control,
                account,
            )?;
            Ok::<_, Interrupted>(if matching.on { selected } else { !selected })
        })?;
    }
    for name in matching.include.as_deref().unwrap_or(&[]) {
        let value = right.get_checked(name, account, control)?;
        result.set(name, value, account, MemoryKind::Workspace, control)?;
    }
    Ok(result)
}

struct Writer<'a> {
    bytes: &'a mut AccountedVec<u8>,
    control: &'a Control,
    account: &'a QueryAccounting,
    failure: Option<Interrupted>,
}
impl Write for Writer<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        append_bytes_checked(self.bytes, value.as_bytes(), self.account, self.control).map_err(
            |error| {
                self.failure = Some(error);
                fmt::Error
            },
        )
    }
}

pub(crate) fn render(
    labels: &OwnedLabels,
    control: &Control,
    account: &QueryAccounting,
) -> Result<AccountedVec<u8>, EvaluationError> {
    let mut bytes = AccountedVec::new(account, MemoryKind::Workspace);
    bytes.push(b'{');
    for (position, label) in labels.0.iter().enumerate() {
        if position != 0 {
            bytes.extend_from_slice(b", ");
        }
        let mut legacy = !label.name.is_empty()
            && (label.name[0].is_ascii_alphabetic() || label.name[0] == b'_');
        for chunk in label.name.chunks(1024) {
            control.check()?;
            account.work(chunk.len() as u64);
            legacy &= chunk
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
        }
        if legacy {
            append_bytes_checked(&mut bytes, &label.name, account, control)?;
        } else {
            quoted(&mut bytes, &label.name, control, account)?;
        }
        bytes.push(b'=');
        quoted(&mut bytes, &label.value, control, account)?;
    }
    bytes.push(b'}');
    Ok(bytes)
}
pub(crate) fn quoted(
    bytes: &mut AccountedVec<u8>,
    value: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    write_checked(bytes, control, account, |writer| {
        quote::write_bytes(value, writer)
    })
}

pub(crate) fn formatted(
    arguments: fmt::Arguments<'_>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<AccountedVec<u8>, Interrupted> {
    let mut bytes = AccountedVec::new(account, MemoryKind::Workspace);
    write_checked(&mut bytes, control, account, |writer| {
        writer.write_fmt(arguments)
    })?;
    Ok(bytes)
}

fn write_checked(
    bytes: &mut AccountedVec<u8>,
    control: &Control,
    account: &QueryAccounting,
    write: impl FnOnce(&mut Writer<'_>) -> fmt::Result,
) -> Result<(), Interrupted> {
    let mut writer = Writer {
        bytes,
        control,
        account,
        failure: None,
    };
    let result = write(&mut writer);
    if let Some(error) = writer.failure {
        return Err(error);
    }
    result.expect("accounted byte writer");
    Ok(())
}
