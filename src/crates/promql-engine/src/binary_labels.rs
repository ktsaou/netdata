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
    equal_with(left, right, control, |units| account.work(units))
}

fn equal_with(
    left: &[u8],
    right: &[u8],
    control: &Control,
    mut work: impl FnMut(u64),
) -> Result<bool, Interrupted> {
    control.check()?;
    if left.len() != right.len() {
        work(1);
        return Ok(false);
    }
    for (left, right) in left.chunks(1024).zip(right.chunks(1024)) {
        control.check()?;
        work(left.len() as u64);
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

pub(crate) fn equal_encoded(
    stored: &[u8],
    operands: [&OwnedLabels; 2],
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    equal_encoded_with(stored, operands, control, |units| account.work(units))
}

fn equal_encoded_with(
    mut stored: &[u8],
    operands: [&OwnedLabels; 2],
    control: &Control,
    mut observe: impl FnMut(u64),
) -> Result<bool, Interrupted> {
    // Small prefixes must not each take the query observations lock.
    let mut pending = 0;
    let mut work = |units| {
        pending += units;
        if pending >= 1024 {
            observe(std::mem::take(&mut pending));
        }
    };
    let result = (|| {
        control.check()?;
        // The caller validates both complete streams before probing any cache entry.
        for label in operands.into_iter().flat_map(|labels| labels.0.iter()) {
            control.check()?;
            work(1);
            for field in [&*label.name, &*label.value] {
                let (prefix, length) = crate::label_encoding::prefix(field.len());
                for part in [&prefix[..length], field] {
                    let Some((head, tail)) = stored.split_at_checked(part.len()) else {
                        work(1);
                        return Ok(false);
                    };
                    if !equal_with(head, part, control, &mut work)? {
                        return Ok(false);
                    }
                    stored = tail;
                }
            }
        }
        control.check()?;
        Ok(stored.is_empty())
    })();
    // Keep completed work even when a later poll interrupts the comparison.
    if pending != 0 {
        observe(pending);
    }
    let matched = result?;
    control.check()?;
    Ok(matched)
}

fn field(
    target: &mut AccountedVec<u8>,
    value: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), EvaluationError> {
    crate::label_encoding::field(value, control, account)?;
    let (prefix, length) = crate::label_encoding::prefix(value.len());
    target.extend_from_slice(&prefix[..length]);
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
    fn borrowed_keys_keep_canonical_boundaries_split_aliases_and_complete_bytes() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        for (width, prefix) in [
            (0, &[0][..]),
            (1, &[1][..]),
            (31, &[31][..]),
            (32, &[32][..]),
            (33, &[33][..]),
            (254, &[254][..]),
            (255, &[255, 255, 0, 0][..]),
            (256, &[255, 0, 1, 0][..]),
            (32767, &[255, 255, 127, 0][..]),
            (32768, &[255, 0, 128, 0][..]),
            (32769, &[255, 1, 128, 0][..]),
        ] {
            let name = vec![b'n'; width];
            let value = vec![255; width];
            let fields: &[(&[u8], &[u8])] =
                &[(b"empty", b""), (&name, &value), (b"tail", &[0, 255])];
            let source = labels(fields, &account);
            let mut encoded = AccountedVec::new(&account, MemoryKind::Workspace);
            append(&mut encoded, &source, None, &control, &account).unwrap();
            let mut expected = vec![5];
            expected.extend_from_slice(b"empty");
            expected.push(0);
            expected.extend_from_slice(prefix);
            expected.extend_from_slice(&name);
            expected.extend_from_slice(prefix);
            expected.extend_from_slice(&value);
            expected.extend_from_slice(&[4, b't', b'a', b'i', b'l', 2, 0, 255]);
            assert_eq!(&*encoded, &expected);
            let hash = crate::label_hash::bytes(&expected, &control, &account).unwrap();
            assert_eq!(
                crate::label_hash::full(&source, &control, &account).unwrap(),
                hash
            );
            let mut trailing = expected.clone();
            trailing.push(0);
            let mut noncanonical = vec![255, 5, 0, 0];
            noncanonical.extend_from_slice(&expected[1..]);
            for split in 0..=fields.len() {
                let left = labels(&fields[..split], &account);
                let right = labels(&fields[split..], &account);
                let operands = [&left, &right];
                let before = account.snapshot().memory;
                assert_eq!(
                    crate::label_hash::full_pair(&left, &right, &control, &account).unwrap(),
                    hash
                );
                let start = account.snapshot().work;
                assert!(equal_encoded(&encoded, operands, &control, &account).unwrap());
                let work = account.snapshot().work - start;
                assert_eq!(work, (expected.len() + fields.len()) as u64);
                assert_eq!(account.snapshot().memory, before);
                for malformed in [
                    &expected[..0],
                    &expected[..1],
                    &expected[..expected.len() - 1],
                    &trailing,
                    &noncanonical,
                ] {
                    assert!(!equal_encoded(malformed, operands, &control, &account).unwrap());
                }
                for remaining in [1, work / 2, work] {
                    let cancelled = Control::default();
                    let result = crate::test_storage::interrupt_after_work(
                        &account,
                        &cancelled,
                        remaining,
                        || equal_encoded(&encoded, operands, &cancelled, &account),
                    );
                    assert!(matches!(result, Err(Interrupted::Cancelled)));
                }
                assert_eq!(account.snapshot().memory, before);
            }
        }
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn borrowed_key_accounting_batches_updates_and_keeps_partial_work_and_controls() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        for width in [0, 1, 254, 255, 1023, 1024, 1025, 32769] {
            let value = vec![b'x'; width];
            let left = labels(&[(b"a", &value)], &account);
            let right = labels(&[(b"b", &value)], &account);
            let operands = [&left, &right];
            let mut encoded = AccountedVec::new(&account, MemoryKind::Workspace);
            for source in operands {
                append(&mut encoded, source, None, &control, &account).unwrap();
            }
            let expected_work = (encoded.len() + 2) as u64;
            let before = account.snapshot();
            let mut updates = Vec::new();
            assert!(
                equal_encoded_with(&encoded, operands, &control, |units| {
                    updates.push(units);
                    account.work(units);
                })
                .unwrap()
            );
            assert_eq!(updates.iter().sum::<u64>(), expected_work);
            assert!(updates.len() as u64 <= expected_work.div_ceil(1024));
            assert!(updates.iter().all(|units| (1..=2047).contains(units)));
            assert_eq!(account.snapshot().work - before.work, expected_work);
            assert_eq!(account.snapshot().memory, before.memory);
            for threshold in [1, expected_work / 2, expected_work] {
                let cancelled = Control::default();
                let start = account.snapshot().work;
                assert_eq!(
                    crate::test_storage::interrupt_after_work(
                        &account,
                        &cancelled,
                        threshold,
                        || equal_encoded(&encoded, operands, &cancelled, &account)
                    ),
                    Err(Interrupted::Cancelled)
                );
                let reached = account.snapshot().work - start;
                assert!((threshold..=threshold + 2046).contains(&reached));
                assert!(reached <= expected_work);
            }
        }
        {
            let left = labels(&[(b"a", b"x")], &account);
            let right = labels(&[(b"b", b"y")], &account);
            let operands = [&left, &right];
            for (stored, matched, expected_work) in [
                (&b"\x01a\x01x\x01b\x01y"[..], true, 10),
                (&b""[..], false, 2),
                (&b"\x02a\x01x\x01b\x01y"[..], false, 2),
                (&b"\x01z\x01x\x01b\x01y"[..], false, 3),
                (&b"\x01a\x01z\x01b\x01y"[..], false, 5),
                (&b"\x01a"[..], false, 4),
                (&b"\x01a\x01x\x01b\x01"[..], false, 10),
                (&b"\x01a\x01x\x01b\x01y\x00"[..], false, 10),
            ] {
                let start = account.snapshot().work;
                let mut updates = Vec::new();
                assert_eq!(
                    equal_encoded_with(stored, operands, &control, |units| {
                        updates.push(units);
                        account.work(units);
                    }),
                    Ok(matched)
                );
                assert_eq!(updates, [expected_work]);
                assert_eq!(account.snapshot().work - start, expected_work);
                let cancelled = Control::default();
                assert_eq!(
                    crate::test_storage::interrupt_after_work(
                        &account,
                        &cancelled,
                        expected_work,
                        || equal_encoded(stored, operands, &cancelled, &account)
                    ),
                    Err(Interrupted::Cancelled)
                );
            }
            for cancelled in [false, true] {
                let expired = Control {
                    deadline: Some(std::time::Instant::now()),
                    ..Control::default()
                };
                if cancelled {
                    expired.cancellation.cancel();
                }
                let before = account.snapshot().work;
                assert_eq!(
                    equal_encoded(b"", operands, &expired, &account),
                    Err(if cancelled {
                        Interrupted::Cancelled
                    } else {
                        Interrupted::Deadline
                    })
                );
                assert_eq!(account.snapshot().work, before);
            }
            let empty = labels(&[], &account);
            let before = account.snapshot().work;
            for (stored, expected) in [(&b""[..], true), (&b"x"[..], false)] {
                assert_eq!(
                    equal_encoded_with(stored, [&empty, &empty], &control, |_| {
                        panic!("empty operands do not charge byte work")
                    }),
                    Ok(expected)
                );
            }
            assert_eq!(account.snapshot().work, before);
        }
        account.snapshot().assert_memory_released();
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
