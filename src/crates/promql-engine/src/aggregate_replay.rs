// SPDX-License-Identifier: GPL-3.0-or-later
//! Query-owned contributor identities; raw values continue through the ordinary evaluator.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    evaluator::RowFilter,
    frontend::ast::NodeId,
    storage::{Control, Interrupted},
    vector_matching::{CompletedInterval, ReplayKey},
};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
};

pub(crate) struct Builder {
    selector: NodeId,
    entries: AccountedVec<(usize, usize)>,
    collect: bool,
}
impl Builder {
    pub(crate) fn new(selector: NodeId, collect: bool, account: &QueryAccounting) -> Self {
        Self {
            selector,
            entries: AccountedVec::new(account, MemoryKind::Workspace),
            collect,
        }
    }
    pub(crate) fn record(
        &mut self,
        group: usize,
        ordinal: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        control.check()?;
        account.work(1);
        if self.collect {
            self.entries.push((group, ordinal));
        }
        Ok(())
    }
    pub(crate) fn finish(
        mut self,
        count: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Plan, Interrupted> {
        let mut groups = AccountedVec::new(account, MemoryKind::Workspace);
        let mut members = AccountedVec::new(account, MemoryKind::Workspace);
        if count > 1 {
            crate::evaluator_order::sort(&mut self.entries, control, account, Ord::cmp)?;
            let mut position = 0;
            for group in 0..count {
                control.check()?;
                account.work(1);
                let start = members.len();
                let mut previous = None;
                while let Some(&(owner, ordinal)) = self.entries.get(position) {
                    control.check()?;
                    account.work(1);
                    if owner != group {
                        break;
                    }
                    if previous != Some(ordinal) {
                        members.push(ordinal);
                        previous = Some(ordinal);
                    }
                    position += 1;
                }
                groups.push(start..members.len());
            }
            debug_assert_eq!(position, self.entries.len());
        }
        control.check()?;
        Ok(Plan {
            selector: self.selector,
            groups,
            members,
            completed: Cell::new(None),
            allow_interval: false,
            interval: RefCell::new(None),
            active: RefCell::new(Active {
                single: None,
                union: AccountedVec::new(account, MemoryKind::Workspace),
            }),
        })
    }
}

pub(crate) struct Plan {
    selector: NodeId,
    groups: AccountedVec<Range<usize>>,
    members: AccountedVec<usize>,
    completed: Cell<Option<ReplayKey>>,
    allow_interval: bool,
    interval: RefCell<Option<AccountedBox<CompletedInterval>>>,
    active: RefCell<Active>,
}
struct Active {
    single: Option<usize>,
    union: AccountedVec<usize>,
}
impl Plan {
    pub(crate) fn allow_interval(&mut self) {
        self.allow_interval = true;
    }
    pub(crate) fn supports_interval(&self) -> bool {
        self.allow_interval
    }
    pub(crate) fn completed_interval(
        &self,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        control.check()?;
        account.work(1);
        Ok(self.allow_interval
            && self
                .interval
                .borrow()
                .as_ref()
                .is_some_and(|interval| interval.contains(key)))
    }
    pub(crate) fn activate_interval(
        &self,
        filter: RowFilter<'_>,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        if !self.completed_interval(key, control, account)?
            || self.groups.is_empty()
            || matches!(filter, RowFilter::All)
        {
            return Ok(false);
        }
        self.activate_members(filter, control, account)
    }
    pub(crate) fn complete_interval(
        &self,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        control.check()?;
        account.work(1);
        if !self.allow_interval || !CompletedInterval::valid(key) {
            return Ok(());
        }
        let mut interval = self.interval.borrow_mut();
        if let Some(interval) = interval.as_mut() {
            interval.record(key);
        } else {
            *interval = Some(AccountedBox::new(
                CompletedInterval::new(key),
                &AllocationContext::new(account, MemoryKind::Workspace),
            ));
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn has_completion(&self) -> bool {
        self.completed.get().is_some()
    }
    #[cfg(test)]
    pub(crate) fn has_interval(&self) -> bool {
        self.interval.borrow().is_some()
    }
    #[cfg(test)]
    pub(crate) fn interval_contains(&self, key: ReplayKey) -> bool {
        self.interval
            .borrow()
            .as_ref()
            .is_some_and(|interval| interval.contains(key))
    }
    pub(crate) fn selector(&self) -> NodeId {
        self.selector
    }
    pub(crate) fn activate(
        &self,
        filter: RowFilter<'_>,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        control.check()?;
        account.work(1);
        if self.completed.get() != Some(key)
            || self.groups.is_empty()
            || matches!(filter, RowFilter::All)
        {
            return Ok(false);
        }
        self.activate_members(filter, control, account)
    }
    fn activate_members(
        &self,
        filter: RowFilter<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        if matches!(filter, RowFilter::Contributors(plan) if std::ptr::eq(plan, self)) {
            return Ok(false);
        }
        let single = filter.singleton();
        let mut active = self.active.borrow_mut();
        if let Some(single) = single {
            if single >= self.groups.len() {
                return Ok(false);
            }
            active.single = Some(single);
            return Ok(true);
        }
        active.single = None;
        active.union.clear();
        filter.visit_ordinals(control, account, |group| {
            if let Some(range) = self.groups.get(group) {
                for &ordinal in &self.members[range.clone()] {
                    control.check()?;
                    account.work(1);
                    active.union.push(ordinal);
                }
            }
            Ok(())
        })?;
        crate::evaluator_order::sort(&mut active.union, control, account, Ord::cmp)?;
        let mut end = 0;
        for position in 0..active.union.len() {
            control.check()?;
            account.work(1);
            let value = active.union[position];
            if end == 0 || active.union[end - 1] != value {
                active.union[end] = value;
                end += 1;
            }
        }
        active.union.truncate(end);
        control.check()?;
        Ok(true)
    }
    fn with_members<R>(&self, read: impl FnOnce(&[usize]) -> R) -> R {
        let active = self.active.borrow();
        let members = active.single.map_or(&*active.union, |group| {
            &self.members[self.groups[group].clone()]
        });
        read(members)
    }
    pub(crate) fn singleton(&self) -> Option<usize> {
        self.with_members(|members| match members {
            [value] => Some(*value),
            _ => None,
        })
    }
    pub(crate) fn next_ordinal(
        &self,
        start: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        self.with_members(|members| {
            control.check()?;
            let (mut left, mut right) = (0, members.len());
            while left < right {
                control.check()?;
                account.work(1);
                let middle = left + (right - left) / 2;
                if members[middle] < start {
                    left = middle + 1;
                } else {
                    right = middle;
                }
            }
            control.check()?;
            Ok(members.get(left).copied())
        })
    }
    pub(crate) fn visit_members(
        &self,
        control: &Control,
        account: &QueryAccounting,
        mut visit: impl FnMut(usize) -> Result<(), Interrupted>,
    ) -> Result<(), Interrupted> {
        // Sorted ordinal membership filters the original traversal, never the arithmetic order.
        self.with_members(|members| {
            for &ordinal in members {
                control.check()?;
                account.work(1);
                visit(ordinal)?;
            }
            control.check()
        })
    }
    pub(crate) fn complete(
        &self,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        control.check()?;
        account.work(1);
        control.check()?;
        self.completed.set(Some(key));
        Ok(())
    }
    pub(crate) fn clear(&self) {
        self.completed.set(None);
        self.interval.borrow_mut().take();
        let mut active = self.active.borrow_mut();
        active.single = None;
        active.union.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounting::EngineAccounting, evaluator::EvaluationOptions, storage::TimeBounds};

    fn options() -> EvaluationOptions {
        EvaluationOptions {
            time_ms: 1000,
            start_ms: 1000,
            end_ms: 3000,
            step_ms: 1000,
            lookback_ms: 300000,
            start_timestamps: false,
            delayed_name_removal: false,
        }
    }
    fn key(options: EvaluationOptions) -> ReplayKey {
        ReplayKey::new(
            options,
            0,
            300000000000,
            TimeBounds {
                start_ms: 0,
                end_ms: 3000,
            },
        )
    }
    #[test]
    fn grouped_intervals_require_success_preserve_domains_and_release() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut builder = Builder::new(NodeId(7), true, &account);
        for (group, ordinal) in [(0, 0), (1, 1), (0, 2), (1, 3)] {
            builder.record(group, ordinal, &control, &account).unwrap();
        }
        let mut plan = builder.finish(2, &control, &account).unwrap();
        let clock = |index| {
            ReplayKey::new(
                EvaluationOptions {
                    time_ms: 1000 + index * 1000,
                    ..options()
                },
                index,
                300000000000,
                TimeBounds {
                    start_ms: 0,
                    end_ms: 3000,
                },
            )
        };
        plan.complete_interval(clock(0), &control, &account)
            .unwrap();
        assert!(!plan.has_interval());
        plan.allow_interval();
        plan.complete(clock(0), &control, &account).unwrap();
        assert!(
            !plan
                .activate_interval(RowFilter::One(0), clock(0), &control, &account)
                .unwrap()
        );
        for index in [0, 2, 1, 0] {
            plan.complete_interval(clock(index), &control, &account)
                .unwrap();
            assert!(
                plan.activate_interval(RowFilter::One(1), clock(index), &control, &account)
                    .unwrap()
            );
            plan.with_members(|members| assert_eq!(members, &[1, 3]));
            if index == 2 {
                assert!(
                    !plan
                        .activate_interval(RowFilter::One(0), clock(0), &control, &account)
                        .unwrap()
                );
                assert!(
                    !plan
                        .activate_interval(RowFilter::One(0), clock(1), &control, &account)
                        .unwrap()
                );
            }
        }
        for index in 0..3 {
            assert!(
                plan.activate_interval(RowFilter::One(0), clock(index), &control, &account)
                    .unwrap()
            );
        }
        let changed = key(EvaluationOptions {
            lookback_ms: 1,
            ..options()
        });
        assert!(
            !plan
                .activate_interval(RowFilter::One(0), changed, &control, &account)
                .unwrap()
        );
        assert!(
            !plan
                .activate_interval(RowFilter::All, clock(0), &control, &account)
                .unwrap()
        );
        assert!(
            plan.activate(RowFilter::One(0), clock(0), &control, &account)
                .unwrap()
        );
        plan.clear();
        assert!(!plan.has_interval() && !plan.has_completion());
        control.cancellation.cancel();
        assert!(matches!(
            plan.complete_interval(clock(0), &control, &account),
            Err(Interrupted::Cancelled)
        ));
        assert!(!plan.has_interval());
        drop((plan, account));
        engine.snapshot().assert_memory_released();
    }
    #[test]
    fn aliases_form_ordinal_unions_only_after_complete_exact_clock() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut builder = Builder::new(NodeId(7), true, &account);
        for (group, ordinal) in [(1, 8), (0, 7), (1, 2), (0, 3), (1, 8)] {
            builder.record(group, ordinal, &control, &account).unwrap();
        }
        let plan = builder.finish(3, &control, &account).unwrap();
        let original = key(options());
        assert_eq!(plan.selector(), NodeId(7));
        assert!(
            !plan
                .activate(RowFilter::One(0), original, &control, &account)
                .unwrap()
        );
        plan.complete(original, &control, &account).unwrap();
        for (group, expected) in [(0, &[3, 7][..]), (1, &[2, 8][..]), (2, &[][..])] {
            assert!(
                plan.activate(RowFilter::One(group), original, &control, &account)
                    .unwrap()
            );
            plan.with_members(|members| assert_eq!(members, expected));
        }
        assert!(
            !plan
                .activate(RowFilter::One(3), original, &control, &account)
                .unwrap()
        );
        assert!(
            plan.activate(
                RowFilter::Ordinals(&[1, 0, 1]),
                original,
                &control,
                &account
            )
            .unwrap()
        );
        plan.with_members(|members| assert_eq!(members, [2, 3, 7, 8]));
        assert_eq!(plan.next_ordinal(4, &control, &account).unwrap(), Some(7));
        assert_eq!(
            plan.next_ordinal(usize::MAX, &control, &account).unwrap(),
            None
        );
        let changed = [
            key(EvaluationOptions {
                time_ms: 1001,
                ..options()
            }),
            key(EvaluationOptions {
                start_ms: 1001,
                ..options()
            }),
            key(EvaluationOptions {
                end_ms: 3001,
                ..options()
            }),
            key(EvaluationOptions {
                step_ms: 1001,
                ..options()
            }),
            key(EvaluationOptions {
                lookback_ms: 300001,
                ..options()
            }),
            key(EvaluationOptions {
                start_timestamps: true,
                ..options()
            }),
            key(EvaluationOptions {
                delayed_name_removal: true,
                ..options()
            }),
            ReplayKey::new(
                options(),
                1,
                300000000000,
                TimeBounds {
                    start_ms: 0,
                    end_ms: 3000,
                },
            ),
            ReplayKey::new(
                options(),
                0,
                300000000001,
                TimeBounds {
                    start_ms: 0,
                    end_ms: 3000,
                },
            ),
            ReplayKey::new(
                options(),
                0,
                300000000000,
                TimeBounds {
                    start_ms: -1,
                    end_ms: 3000,
                },
            ),
            ReplayKey::new(
                options(),
                0,
                300000000000,
                TimeBounds {
                    start_ms: 0,
                    end_ms: 3001,
                },
            ),
        ];
        for changed in changed {
            assert!(
                !plan
                    .activate(RowFilter::One(0), changed, &control, &account)
                    .unwrap()
            );
        }
        plan.complete(changed[0], &control, &account).unwrap();
        assert!(
            !plan
                .activate(RowFilter::One(0), original, &control, &account)
                .unwrap()
        );
        plan.clear();
        assert!(
            !plan
                .activate(RowFilter::One(0), changed[0], &control, &account)
                .unwrap()
        );
        drop((plan, account));
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn singleton_proofs_keep_no_contributor_array_and_cancelled_work_has_no_credit() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut builder = Builder::new(NodeId(0), false, &account);
        builder.record(0, 3, &control, &account).unwrap();
        let plan = builder.finish(1, &control, &account).unwrap();
        assert_eq!(plan.groups.capacity(), 0);
        assert_eq!(plan.members.capacity(), 0);
        control.cancellation.cancel();
        assert!(plan.complete(key(options()), &control, &account).is_err());
        assert!(plan.completed.get().is_none());
        assert!(
            plan.activate(RowFilter::One(0), key(options()), &control, &account)
                .is_err()
        );
        drop((plan, account));
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn nested_unions_borrow_ancestors_and_large_member_lookup_is_logarithmic() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut outer = Builder::new(NodeId(0), true, &account);
        for (group, ordinal) in [(0, 0), (0, 2), (1, 1), (1, 3)] {
            outer.record(group, ordinal, &control, &account).unwrap();
        }
        let outer = outer.finish(2, &control, &account).unwrap();
        let mut inner = Builder::new(NodeId(1), true, &account);
        for (group, ordinal) in [(0, 6), (0, 2), (1, 8), (2, 4), (2, 9), (3, 5)] {
            inner.record(group, ordinal, &control, &account).unwrap();
        }
        let inner = inner.finish(4, &control, &account).unwrap();
        let key = key(options());
        outer.complete(key, &control, &account).unwrap();
        inner.complete(key, &control, &account).unwrap();
        assert!(
            outer
                .activate(RowFilter::One(0), key, &control, &account)
                .unwrap()
        );
        assert!(
            inner
                .activate(RowFilter::Contributors(&outer), key, &control, &account)
                .unwrap()
        );
        inner.with_members(|members| assert_eq!(members, [2, 4, 6, 9]));
        outer.with_members(|members| assert_eq!(members, [0, 2]));
        assert!(
            !inner
                .activate(RowFilter::Contributors(&inner), key, &control, &account)
                .unwrap()
        );

        let mut large = Builder::new(NodeId(2), true, &account);
        for ordinal in (0..2048).rev() {
            large
                .record(ordinal % 2, ordinal, &control, &account)
                .unwrap();
        }
        let large = large.finish(2, &control, &account).unwrap();
        large.complete(key, &control, &account).unwrap();
        assert!(
            large
                .activate(RowFilter::One(0), key, &control, &account)
                .unwrap()
        );
        let before = account.snapshot().work;
        for ordinal in 0..1024 {
            assert_eq!(
                large.next_ordinal(ordinal * 2, &control, &account).unwrap(),
                Some(ordinal * 2)
            );
            assert!(
                !RowFilter::Contributors(&large)
                    .accepts(ordinal * 2 + 1, &control, &account)
                    .unwrap()
            );
        }
        assert!(account.snapshot().work - before < 32768);
        drop((outer, inner, large, account));
        engine.snapshot().assert_memory_released();
    }
}
