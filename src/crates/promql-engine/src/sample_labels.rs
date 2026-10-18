// SPDX-License-Identifier: GPL-3.0-or-later
//! Temporary values may borrow frozen labels until mutation or retained ownership.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    evaluator::{EvaluationError, check_reserved_name, drop_reserved, metadata},
    result::OwnedLabels,
    storage::{Control, Interrupted},
};
use std::ops::Deref;

#[derive(Debug)]
pub(crate) enum SampleLabels<'a> {
    Borrowed(&'a OwnedLabels),
    Owned(OwnedLabels),
}

impl<'a> SampleLabels<'a> {
    pub(crate) fn as_ref(&self) -> &OwnedLabels {
        match self {
            Self::Borrowed(labels) => labels,
            Self::Owned(labels) => labels,
        }
    }

    pub(crate) fn aggregate(
        labels: &'a OwnedLabels,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<Self, Interrupted> {
        control.check()?;
        account.work(1);
        for label in &*labels.0 {
            control.check()?;
            account.work(1);
            if label.value.is_empty() {
                // Empty values belong to raw group identity, but not the finalized output.
                let mut labels = labels.copy_checked(account, MemoryKind::Workspace, control)?;
                labels.reset_checked(account, control)?;
                control.check()?;
                return Ok(Self::Owned(labels));
            }
        }
        control.check()?;
        Ok(Self::Borrowed(labels))
    }

    pub(crate) fn into_owned(
        self,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<OwnedLabels, Interrupted> {
        match self {
            Self::Owned(labels) => Ok(labels),
            Self::Borrowed(labels) => {
                let labels = labels.copy_checked(account, MemoryKind::Workspace, control)?;
                control.check()?;
                Ok(labels)
            }
        }
    }

    pub(crate) fn make_owned(
        &mut self,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<&mut OwnedLabels, Interrupted> {
        if let Self::Borrowed(labels) = self {
            let owned = labels.copy_checked(account, MemoryKind::Workspace, control)?;
            control.check()?;
            *self = Self::Owned(owned);
        }
        let Self::Owned(labels) = self else {
            unreachable!("materialized sample labels")
        };
        Ok(labels)
    }

    pub(crate) fn drop_reserved(
        &mut self,
        name_only: bool,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<(), EvaluationError> {
        let Self::Borrowed(labels) = self else {
            return drop_reserved(
                self.make_owned(account, control)?,
                name_only,
                account,
                control,
            );
        };
        check_reserved_name(labels.0.first().map(|label| &*label.name), account, control)?;
        let mut removes = false;
        for label in &*labels.0 {
            control.check()?;
            account.work(1);
            let removes_label = if name_only {
                &*label.name == b"__name__"
            } else {
                metadata(&label.name)
            };
            if removes_label {
                removes = true;
                break;
            }
        }
        control.check()?;
        if removes {
            drop_reserved(
                self.make_owned(account, control)?,
                name_only,
                account,
                control,
            )?;
            control.check()?;
        }
        Ok(())
    }
}

impl From<OwnedLabels> for SampleLabels<'_> {
    fn from(labels: OwnedLabels) -> Self {
        Self::Owned(labels)
    }
}

impl Deref for SampleLabels<'_> {
    type Target = OwnedLabels;
    fn deref(&self) -> &OwnedLabels {
        self.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounting::EngineAccounting, storage::LabelRef};

    fn labels(account: &QueryAccounting, pairs: &[(&[u8], &[u8])]) -> OwnedLabels {
        let refs: Vec<_> = pairs
            .iter()
            .map(|(name, value)| LabelRef { name, value })
            .collect();
        OwnedLabels::copy_from(&refs, account, MemoryKind::Workspace)
    }

    fn label_bytes(labels: &OwnedLabels) -> Vec<(Vec<u8>, Vec<u8>)> {
        labels
            .0
            .iter()
            .map(|label| (label.name.to_vec(), label.value.to_vec()))
            .collect()
    }

    #[test]
    fn borrowed_removal_matches_owned_cleanup_before_short_scan_guard() {
        let mut cases: Vec<Vec<(&[u8], &[u8])>> = vec![
            vec![],
            vec![(b"id", b"v")],
            vec![(b"__name__", b"m")],
            vec![(b"__type__", b"gauge")],
            vec![(b"__unit__", b"KiB")],
            vec![(b"id", b""), (b"z", b"\xff\0")],
            vec![
                (b"__name___", b"v"),
                (b"__type", b"v"),
                (b"__unit__suffix", b"v"),
            ],
            vec![
                (b"z", b"v"),
                (b"__name__", b""),
                (b"__type__", b"gauge"),
                (b"__name__", b"second"),
                (b"__unit__", b"KiB"),
                (b"a", b""),
            ],
            vec![
                (b"\xff\0", b"v"),
                (b"", b"later"),
                (b"__unit__", b""),
                (b"id", b"v"),
            ],
        ];
        for reserved in [
            (b"__name__".as_slice(), b"m".as_slice()),
            (b"__type__".as_slice(), b"gauge".as_slice()),
            (b"__unit__".as_slice(), b"KiB".as_slice()),
        ] {
            for position in 0..=3 {
                let mut case = vec![
                    (b"alpha".as_slice(), b"".as_slice()),
                    (b"beta".as_slice(), b"\xff\0".as_slice()),
                    (b"gamma".as_slice(), b"v".as_slice()),
                ];
                case.insert(position, reserved);
                cases.push(case);
            }
        }
        let mut complete = 0;
        for name_only in [false, true] {
            for case in &cases {
                let account = EngineAccounting::default().query();
                let source = labels(&account, case);
                let frozen = label_bytes(&source);
                let mut expected = source
                    .copy_checked(&account, MemoryKind::Workspace, &Control::default())
                    .unwrap();
                drop_reserved(&mut expected, name_only, &account, &Control::default()).unwrap();
                let mut value = SampleLabels::Borrowed(&source);
                let before = account.snapshot().memory;
                value
                    .drop_reserved(name_only, &account, &Control::default())
                    .unwrap();
                assert_eq!(label_bytes(&value), label_bytes(&expected));
                assert_eq!(label_bytes(&source), frozen);
                let removes = case.iter().any(|(name, _)| {
                    if name_only {
                        *name == b"__name__"
                    } else {
                        metadata(name)
                    }
                });
                if removes {
                    assert!(matches!(value, SampleLabels::Owned(_)));
                } else {
                    assert!(
                        matches!(value, SampleLabels::Borrowed(owner) if std::ptr::eq(owner, &source))
                    );
                    assert_eq!(account.snapshot().memory, before);
                }
                drop((value, expected));
                drop(source);
                account.snapshot().assert_memory_released();
                complete += 1;
            }
            for case in [
                vec![(b"".as_slice(), b"v".as_slice())],
                vec![(b"".as_slice(), b"v".as_slice()), (b"__name__", b"m")],
            ] {
                let account = EngineAccounting::default().query();
                let source = labels(&account, &case);
                let frozen = label_bytes(&source);
                let mut expected = source
                    .copy_checked(&account, MemoryKind::Workspace, &Control::default())
                    .unwrap();
                let expected_error =
                    drop_reserved(&mut expected, name_only, &account, &Control::default())
                        .unwrap_err();
                let mut value = SampleLabels::Borrowed(&source);
                let error = value
                    .drop_reserved(name_only, &account, &Control::default())
                    .unwrap_err();
                let (
                    EvaluationError::WrappedRuntime(actual),
                    EvaluationError::WrappedRuntime(expected_message),
                ) = (&error, &expected_error)
                else {
                    panic!("first empty name must retain its runtime error");
                };
                assert_eq!(&**actual, &**expected_message);
                assert!(
                    matches!(value, SampleLabels::Borrowed(owner) if std::ptr::eq(owner, &source))
                );
                assert_eq!(label_bytes(&source), frozen);
                drop((value, error, expected_error, expected));
                drop(source);
                account.snapshot().assert_memory_released();
                complete += 1;
            }
        }

        let mut causal = Vec::new();
        for name_only in [false, true] {
            for count in [1, 9, 65] {
                let run = |first| {
                    let account = EngineAccounting::default().query();
                    let mut pairs = (0..count - 1)
                        .map(|index| (format!("label_{index:03}").into_bytes(), b"v".to_vec()))
                        .collect::<Vec<_>>();
                    pairs.insert(
                        if first { 0 } else { pairs.len() },
                        (b"__name__".to_vec(), b"m".to_vec()),
                    );
                    let views = pairs
                        .iter()
                        .map(|(name, value)| (name.as_slice(), value.as_slice()))
                        .collect::<Vec<_>>();
                    let source = labels(&account, &views);
                    let mut value = SampleLabels::Borrowed(&source);
                    let before = account.snapshot().work;
                    value
                        .drop_reserved(name_only, &account, &Control::default())
                        .unwrap();
                    assert!(matches!(value, SampleLabels::Owned(_)));
                    let bytes = label_bytes(&value);
                    let observed = account.snapshot();
                    drop(value);
                    drop(source);
                    account.snapshot().assert_memory_released();
                    (bytes, observed.work - before, observed.memory)
                };
                let (early, early_work, early_memory) = run(true);
                let (last, last_work, last_memory) = run(false);
                assert_eq!(early, last);
                assert_eq!(early_memory, last_memory);
                causal.push((count, name_only, early_work, last_work));
            }
        }
        eprintln!(
            "complete borrowed removal scenarios={complete}; canonical early/last pairs={}",
            causal.len()
        );
        for (count, name_only, early_work, last_work) in causal {
            assert_eq!(
                last_work - early_work,
                (count - 1) as u64,
                "detection should stop once removal is known: count{count} name_only{name_only}"
            );
        }
    }

    #[test]
    fn borrowed_removal_cancellation_preserves_each_ownership_phase() {
        let account = EngineAccounting::default().query();
        let source = labels(
            &account,
            &[
                (b"__name__", b"m"),
                (b"__type__", b"gauge"),
                (b"__unit__", b"KiB"),
                (b"id", b"v"),
            ],
        );
        let frozen = label_bytes(&source);
        for name_only in [false, true] {
            let mut value = SampleLabels::Borrowed(&source);
            let before = account.snapshot().work;
            value
                .drop_reserved(name_only, &account, &Control::default())
                .unwrap();
            let work = account.snapshot().work - before;
            drop(value);
            let mut phases = [false; 3];
            for budget in 1..=work {
                let before = account.snapshot().memory;
                let mut value = SampleLabels::Borrowed(&source);
                let control = Control::default();
                let outcome =
                    crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                        value.drop_reserved(name_only, &account, &control)
                    });
                assert!(
                    matches!(
                        outcome,
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    ),
                    "name_only{name_only} budget{budget}/{work}"
                );
                assert_eq!(label_bytes(&source), frozen);
                match &value {
                    SampleLabels::Borrowed(owner) => {
                        assert!(std::ptr::eq(*owner, &source));
                        if account.snapshot().memory[MemoryKind::Workspace as usize].acquired
                            == before[MemoryKind::Workspace as usize].acquired
                        {
                            phases[0] = true;
                        } else {
                            phases[1] = true;
                        }
                    }
                    SampleLabels::Owned(_) => {
                        phases[2] |= label_bytes(&value) != frozen;
                    }
                }
                drop(value);
                assert!(
                    account
                        .snapshot()
                        .memory
                        .iter()
                        .zip(before)
                        .all(|(new, old)| new.current == old.current)
                );
            }
            assert_eq!(
                phases, [true; 3],
                "actual detection/copy/owned-mutation phases"
            );
            let cancelled = Control::default();
            cancelled.cancellation.cancel();
            let mut value = SampleLabels::Borrowed(&source);
            let before = account.snapshot();
            assert!(matches!(
                value.drop_reserved(name_only, &account, &cancelled),
                Err(EvaluationError::Interrupted(Interrupted::Cancelled))
            ));
            assert!(matches!(value, SampleLabels::Borrowed(_)));
            assert_eq!(account.snapshot().memory, before.memory);
            assert_eq!(account.snapshot().work, before.work);
            drop(value);
        }
        drop(source);

        let source = labels(
            &account,
            &[(b"__name__", b"m"), (b"payload", &vec![b'x'; 65_536])],
        );
        let frozen = label_bytes(&source);
        let mut value = SampleLabels::Borrowed(&source);
        let control = Control::default();
        let before = account.snapshot().memory;
        let outcome = crate::test_storage::interrupt_after_work(&account, &control, 1026, || {
            value.drop_reserved(false, &account, &control)
        });
        assert!(matches!(
            outcome,
            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
        ));
        assert!(matches!(value, SampleLabels::Borrowed(owner) if std::ptr::eq(owner, &source)));
        assert!(
            account.snapshot().memory[MemoryKind::Workspace as usize].acquired
                > before[MemoryKind::Workspace as usize].acquired
        );
        assert!(
            account
                .snapshot()
                .memory
                .iter()
                .zip(before)
                .all(|(new, old)| new.current == old.current)
        );
        assert_eq!(label_bytes(&source), frozen);
        drop(value);
        drop(source);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn aggregate_labels_borrow_clean_bytes_and_normalize_only_owned_copies() {
        let account = EngineAccounting::default().query();
        let control = Control::default();
        let source = labels(&account, &[(b"__name__", b"m"), (b"id", b"\xff\0")]);
        let before = account.snapshot().memory;
        let result = SampleLabels::aggregate(&source, &account, &control).unwrap();
        assert!(matches!(result, SampleLabels::Borrowed(value) if std::ptr::eq(value, &source)));
        assert_eq!(result.0[1].value.as_ref(), b"\xff\0");
        assert_eq!(account.snapshot().memory, before);
        drop(result);

        let no_labels = OwnedLabels::new(&account, MemoryKind::Workspace);
        let before = account.snapshot().memory;
        let result = SampleLabels::aggregate(&no_labels, &account, &control).unwrap();
        assert!(matches!(result, SampleLabels::Borrowed(value) if std::ptr::eq(value, &no_labels)));
        drop(result);
        let cancelled = Control::default();
        let result = crate::test_storage::interrupt_after_work(&account, &cancelled, 1, || {
            SampleLabels::aggregate(&no_labels, &account, &cancelled)
        });
        assert_eq!(result.unwrap_err(), Interrupted::Cancelled);
        assert_eq!(account.snapshot().memory, before);
        drop(no_labels);

        let empty = labels(&account, &[(b"__name__", b"m"), (b"id", b""), (b"z", b"v")]);
        let result = SampleLabels::aggregate(&empty, &account, &control).unwrap();
        assert!(matches!(result, SampleLabels::Owned(_)));
        assert_eq!(result.0.len(), 2);
        assert_eq!(result.0[1].name.as_ref(), b"z");
        assert_eq!(empty.0.len(), 3);
        assert!(empty.0[1].value.is_empty());
        drop(result);
        drop((empty, source));
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn sample_label_conversion_is_transactional_and_owned_transfer_does_not_copy() {
        let account = EngineAccounting::default().query();
        let source = labels(&account, &[(b"id", &vec![b'x'; 65_536])]);
        let mut borrowed = SampleLabels::Borrowed(&source);
        let before = account.snapshot().memory;
        let control = Control::default();
        let result = crate::test_storage::interrupt_after_work(&account, &control, 1024, || {
            borrowed.make_owned(&account, &control)
        });
        assert_eq!(result.unwrap_err(), Interrupted::Cancelled);
        assert!(matches!(borrowed, SampleLabels::Borrowed(value) if std::ptr::eq(value, &source)));
        assert!(
            account
                .snapshot()
                .memory
                .iter()
                .zip(before)
                .all(|(now, old)| now.current == old.current)
        );
        borrowed
            .make_owned(&account, &Control::default())
            .unwrap()
            .set(
                b"id",
                b"changed",
                &account,
                MemoryKind::Workspace,
                &Control::default(),
            )
            .unwrap();
        assert_eq!(borrowed.0[0].value.as_ref(), b"changed");
        assert_eq!(source.0[0].value.len(), 65_536);
        assert!(source.0[0].value.iter().all(|byte| *byte == b'x'));

        let owned = labels(&account, &[(b"id", b"owned")]);
        let address = owned.0.as_ptr();
        let before = account.snapshot().memory;
        let cancelled = Control::default();
        cancelled.cancellation.cancel();
        let moved = SampleLabels::Owned(owned)
            .into_owned(&account, &cancelled)
            .unwrap();
        assert_eq!(moved.0.as_ptr(), address);
        assert_eq!(account.snapshot().memory, before);
        drop((moved, borrowed));
        drop(source);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn borrowed_reserved_removal_preserves_noop_ownership_and_first_name_errors() {
        let account = EngineAccounting::default().query();
        for name_only in [false, true] {
            let source = labels(&account, &[(b"id", b"v")]);
            let mut borrowed = SampleLabels::Borrowed(&source);
            let before = account.snapshot().memory;
            borrowed
                .drop_reserved(name_only, &account, &Control::default())
                .unwrap();
            assert!(
                matches!(borrowed, SampleLabels::Borrowed(value) if std::ptr::eq(value, &source))
            );
            assert_eq!(account.snapshot().memory, before);
            drop(borrowed);

            let reserved = labels(
                &account,
                &[
                    (b"__name__", b"m"),
                    (b"__type__", b"gauge"),
                    (b"__unit__", b"KiB"),
                    (b"id", b"v"),
                ],
            );
            let mut result = SampleLabels::Borrowed(&reserved);
            result
                .drop_reserved(name_only, &account, &Control::default())
                .unwrap();
            assert!(matches!(result, SampleLabels::Owned(_)));
            assert_eq!(result.0.len(), if name_only { 3 } else { 1 });
            assert_eq!(result.0.last().unwrap().name.as_ref(), b"id");
            assert_eq!(reserved.0.len(), 4);
            assert_eq!(reserved.0[0].name.as_ref(), b"__name__");
            drop(result);
            drop(reserved);

            let invalid = labels(&account, &[(b"", b"kept")]);
            let mut result = SampleLabels::Borrowed(&invalid);
            let error = result
                .drop_reserved(name_only, &account, &Control::default())
                .unwrap_err();
            assert!(matches!(error, EvaluationError::WrappedRuntime(_)));
            assert!(matches!(result, SampleLabels::Borrowed(_)));
            drop((error, result));
            drop((invalid, source));
        }
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn borrowed_label_walks_observe_initial_middle_and_terminal_cancellation() {
        let account = EngineAccounting::default().query();
        let mut source = OwnedLabels::new(&account, MemoryKind::Workspace);
        for index in 0..10_000 {
            source.push(
                format!("label_{index:05}").as_bytes(),
                b"value",
                &account,
                MemoryKind::Workspace,
            );
        }
        for operation in 0..3 {
            let run = |control: &Control| -> Result<(), Interrupted> {
                let mut value = SampleLabels::Borrowed(&source);
                match operation {
                    0 => SampleLabels::aggregate(&source, &account, control).map(drop),
                    1 => value
                        .drop_reserved(false, &account, control)
                        .map_err(|error| {
                            let EvaluationError::Interrupted(error) = error else {
                                panic!("valid borrowed labels only interrupt")
                            };
                            error
                        }),
                    _ => value.make_owned(&account, control).map(|_| ()),
                }
            };
            let before = account.snapshot().work;
            run(&Control::default()).unwrap();
            let work = account.snapshot().work - before;
            assert!(work > 8192);
            for budget in [1, 8192, work] {
                let control = Control::default();
                let before = account.snapshot().memory;
                let result =
                    crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                        run(&control)
                    });
                assert_eq!(
                    result,
                    Err(Interrupted::Cancelled),
                    "operation{operation} budget{budget}"
                );
                assert!(
                    account
                        .snapshot()
                        .memory
                        .iter()
                        .zip(before)
                        .all(|(now, old)| now.current == old.current)
                );
            }
            let control = Control::default();
            control.cancellation.cancel();
            assert_eq!(run(&control), Err(Interrupted::Cancelled));
        }
        drop(source);
        account.snapshot().assert_memory_released();
    }
}
