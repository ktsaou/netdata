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
    if matching.cardinality == Cardinality::OneToOne
        && matching
            .include
            .as_ref()
            .is_none_or(|fields| fields.is_empty())
    {
        let remove_reserved = drop_name || crate::binary_value::changes_metadata(operator);
        if remove_reserved {
            let mut first = None;
            for label in &*left.0 {
                control.check()?;
                account.work(1);
                if !label.value.is_empty() {
                    first = Some(&*label.name);
                    break;
                }
            }
            // Reset precedes the reserved-name error; matching must not hide it.
            crate::evaluator::check_reserved_name(first, account, control)?;
        }
        return left
            .copy_filtered_checked(account, MemoryKind::Workspace, control, |label| {
                if label.value.is_empty()
                    || remove_reserved && crate::evaluator::metadata(&label.name)
                {
                    return Ok(false);
                }
                let selected = named(
                    &label.name,
                    matching.labels.as_deref().unwrap_or(&[]),
                    control,
                    account,
                )?;
                Ok(if matching.on { selected } else { !selected })
            })
            .map_err(Into::into);
    }
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

pub(crate) struct Writer<'a> {
    bytes: &'a mut AccountedVec<u8>,
    control: Option<&'a Control>,
    account: &'a QueryAccounting,
    failure: Option<Interrupted>,
}
impl Write for Writer<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if let Some(control) = self.control {
            append_bytes_checked(self.bytes, value.as_bytes(), self.account, control).map_err(
                |error| {
                    self.failure = Some(error);
                    fmt::Error
                },
            )
        } else {
            self.account.work(value.len() as u64);
            self.bytes.extend_from_slice(value.as_bytes());
            Ok(())
        }
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

pub(crate) fn write_checked(
    bytes: &mut AccountedVec<u8>,
    control: &Control,
    account: &QueryAccounting,
    write: impl FnOnce(&mut Writer<'_>) -> fmt::Result,
) -> Result<(), Interrupted> {
    write_with_control(bytes, Some(control), account, write)
}
pub(crate) fn write_with_control(
    bytes: &mut AccountedVec<u8>,
    control: Option<&Control>,
    account: &QueryAccounting,
    write: impl FnOnce(&mut Writer<'_>) -> fmt::Result,
) -> Result<(), Interrupted> {
    if let Some(control) = control {
        control.check()?;
    }
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
    if let Some(control) = control {
        control.check()?;
    }
    Ok(())
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    use crate::accounting::EngineAccounting;

    fn labels(values: &[(&[u8], &[u8])], account: &QueryAccounting) -> OwnedLabels {
        let mut result = OwnedLabels::new(account, MemoryKind::Workspace);
        for (name, value) in values {
            result.push(name, value, account, MemoryKind::Workspace);
        }
        result
    }

    fn matching(name: &[u8], account: &QueryAccounting) -> VectorMatching {
        let mut field = AccountedVec::new(account, MemoryKind::Workspace);
        field.extend_from_slice(name);
        let mut fields = AccountedVec::new(account, MemoryKind::Workspace);
        fields.push(field);
        VectorMatching {
            on: true,
            labels: Some(fields),
            ..Default::default()
        }
    }

    #[test]
    fn one_to_one_projection_memory_follows_retained_labels() {
        let mut retained = Vec::new();
        for count in [8, 17, 65] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut source = labels(&[(b"__name__", b"m"), (b"instance", b"a")], &account);
            for index in 2..count {
                source.push(
                    format!("tag_{index:02}").as_bytes(),
                    b"discarded metadata",
                    &account,
                    MemoryKind::Workspace,
                );
            }
            let matching = matching(b"instance", &account);
            let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
            let output = projected(
                &source,
                &source,
                TokenKind::DIV,
                &matching,
                false,
                &control,
                &account,
            )
            .unwrap();
            assert_eq!(output.0.len(), 1);
            assert_eq!(&*output.0[0].name, b"instance");
            assert_eq!(&*output.0[0].value, b"a");
            let bytes = account.snapshot().memory[MemoryKind::Workspace as usize].current - before;
            println!("{{\"projection_input_labels\":{count},\"retained_bytes\":{bytes}}}");
            retained.push(bytes);
            drop(output);
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].current,
                before
            );
            drop((matching, source));
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
        assert!(
            retained.iter().all(|bytes| *bytes == retained[0]),
            "one retained label must not retain input-sized capacity: {retained:?}"
        );
    }

    #[test]
    fn one_to_one_projection_preserves_reset_and_reserved_error_order() {
        for operator in [TokenKind::DIV, TokenKind::EQLC] {
            for drop_name in [false, true] {
                for empty_value in [false, true] {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let source = labels(
                        &[
                            (b"", if empty_value { b"" } else { b"generated" }),
                            (b"__name__", b"m"),
                            (b"__type__", b"gauge"),
                            (b"__unit__", b"KiB"),
                            (b"instance", b"a"),
                        ],
                        &account,
                    );
                    let matching = matching(b"instance", &account);
                    let outcome = projected(
                        &source, &source, operator, &matching, drop_name, &control, &account,
                    );
                    if !empty_value && (drop_name || operator == TokenKind::DIV) {
                        let EvaluationError::WrappedRuntime(message) = outcome.unwrap_err() else {
                            panic!("reserved removal must report its original empty-name error");
                        };
                        assert_eq!(
                            &*message,
                            b"unexpected error: runtime error: index out of range [0] with length 0"
                        );
                    } else {
                        let output = outcome.unwrap();
                        assert_eq!(output.0.len(), 1);
                        assert_eq!(&*output.0[0].name, b"instance");
                        assert_eq!(&*output.0[0].value, b"a");
                    }
                    drop((matching, source));
                    account.snapshot().assert_memory_released();
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn one_to_one_projection_interrupts_retained_field_copy() {
        for pending in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let value = vec![b'x'; 16_384];
            let source = labels(&[(b"instance", &value)], &account);
            let matching = matching(b"instance", &account);
            let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
            if pending {
                control.cancellation.cancel();
            }
            let outcome = crate::test_storage::interrupt_during(&account, &control, || {
                projected(
                    &source,
                    &source,
                    TokenKind::DIV,
                    &matching,
                    false,
                    &control,
                    &account,
                )
            });
            assert!(matches!(
                outcome,
                Err(EvaluationError::Interrupted(Interrupted::Cancelled))
            ));
            drop(outcome);
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].current,
                before
            );
            drop((matching, source));
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn compact_projection_keeps_matching_and_group_include_semantics() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let source = labels(
            &[
                (b"__name__", b"m"),
                (b"__type__", b"gauge"),
                (b"__unit__", b"KiB"),
                (b"empty", b""),
                (b"instance", b"a"),
                (b"job", b"worker"),
                (b"opaque", &[0, 255]),
            ],
            &account,
        );
        for operator in [TokenKind::DIV, TokenKind::EQLC] {
            for drop_name in [false, true] {
                for on in [false, true] {
                    for empty_fields in [false, true] {
                        let mut matching = matching(b"instance", &account);
                        matching.on = on;
                        if empty_fields {
                            matching.labels.as_mut().unwrap().clear();
                        }
                        let result = projected(
                            &source, &source, operator, &matching, drop_name, &control, &account,
                        )
                        .unwrap();
                        let actual: Vec<_> = result
                            .0
                            .iter()
                            .map(|label| (label.name.to_vec(), label.value.to_vec()))
                            .collect();
                        let expected: Vec<_> = source
                            .0
                            .iter()
                            .filter(|label| {
                                !label.value.is_empty()
                                    && (!(drop_name || operator == TokenKind::DIV)
                                        || !matches!(
                                            &*label.name,
                                            b"__name__" | b"__type__" | b"__unit__"
                                        ))
                                    && (if on {
                                        !empty_fields && &*label.name == b"instance"
                                    } else {
                                        empty_fields || &*label.name != b"instance"
                                    })
                            })
                            .map(|label| (label.name.to_vec(), label.value.to_vec()))
                            .collect();
                        assert_eq!(actual, expected);
                    }
                }
            }
        }
        let other = labels(&[(b"instance", b"b"), (b"new", b"included")], &account);
        for cardinality in [
            Cardinality::OneToOne,
            Cardinality::ManyToOne,
            Cardinality::OneToMany,
        ] {
            let mut matching = matching(b"instance", &account);
            matching.cardinality = cardinality;
            let mut include = AccountedVec::new(&account, MemoryKind::Workspace);
            for name in [b"instance".as_slice(), b"job", b"new"] {
                let mut field = AccountedVec::new(&account, MemoryKind::Workspace);
                field.extend_from_slice(name);
                include.push(field);
            }
            matching.include = Some(include);
            let output = projected(
                &source,
                &other,
                TokenKind::DIV,
                &matching,
                false,
                &control,
                &account,
            )
            .unwrap();
            let actual: Vec<_> = output
                .0
                .iter()
                .map(|label| (label.name.to_vec(), label.value.to_vec()))
                .collect();
            let mut expected = vec![
                (b"instance".to_vec(), b"b".to_vec()),
                (b"new".to_vec(), b"included".to_vec()),
            ];
            if cardinality != Cardinality::OneToOne {
                expected.push((b"opaque".to_vec(), vec![0, 255]));
            }
            assert_eq!(actual, expected);
        }
        drop((source, other));
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}
