// SPDX-License-Identifier: GPL-3.0-or-later
//! Successful pair identities; values and unfinished suffixes remain in the evaluator.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    position_index::PositionIndex,
    storage::{Control, Interrupted},
};
use std::cell::{Cell, RefCell};

pub(crate) struct Inputs<'a> {
    pub(crate) members: &'a [u64],
    pub(crate) left_rows: usize,
    pub(crate) many: usize,
}

pub(crate) struct Plan {
    pairs: AccountedVec<[usize; 2]>,
    by_many: AccountedVec<usize>,
    credits: RefCell<AccountedVec<u64>>,
    mode: Mode,
}

enum Mode {
    Prefix(AccountedVec<u64>),
    Frontier(Cell<usize>),
}

impl Plan {
    pub(crate) fn new(
        pairs: AccountedVec<[usize; 2]>,
        hashes: AccountedVec<u64>,
        many: usize,
        rows: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<Self>, Interrupted> {
        if hashes.len() != pairs.len() {
            return Ok(None);
        }
        let mut by_many = AccountedVec::new(account, MemoryKind::Workspace);
        for _ in 0..rows {
            control.check()?;
            account.work(1);
            by_many.push(usize::MAX);
        }
        for (ordinal, pair) in pairs.iter().enumerate() {
            control.check()?;
            account.work(1);
            if pair.contains(&usize::MAX) {
                continue;
            }
            let Some(position) = by_many.get_mut(pair[many]) else {
                return Ok(None);
            };
            if *position != usize::MAX {
                return Ok(None);
            }
            *position = ordinal;
        }
        control.check()?;
        Ok(Some(Self {
            pairs,
            by_many,
            credits: RefCell::new(AccountedVec::new(account, MemoryKind::Workspace)),
            mode: Mode::Prefix(hashes),
        }))
    }

    pub(crate) fn certify_frontier(
        &mut self,
        signatures: [&[usize]; 2],
        signature_count: usize,
        many: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let Mode::Prefix(hashes) = &self.mode else {
            return control.check();
        };
        let mut index = PositionIndex::new(account);
        for (ordinal, pair) in self.pairs.iter().enumerate() {
            control.check()?;
            account.work(1);
            if pair.contains(&usize::MAX) {
                continue;
            }
            if signatures[0].get(pair[0]).is_none()
                || signatures[0].get(pair[0]) != signatures[1].get(pair[1])
            {
                return Ok(());
            }
            let hash = hashes[ordinal];
            if index
                .find(hash, |prior| Ok(hashes[prior] == hash), control, account)?
                .is_some()
            {
                return Ok(());
            }
            index.insert_new(hash, ordinal, |prior| Ok(hashes[prior]), control, account)?;
        }
        drop(index);
        let mut one = AccountedVec::new(account, MemoryKind::Workspace);
        for _ in 0..signature_count.div_ceil(64) {
            control.check()?;
            account.work(1);
            one.push(0_u64);
        }
        for &signature in signatures[1 - many] {
            control.check()?;
            account.work(1);
            let bit = 1_u64 << (signature % 64);
            let Some(word) = one.get_mut(signature / 64) else {
                return Ok(());
            };
            if *word & bit != 0 {
                return Ok(());
            }
            *word |= bit;
        }
        for (row, &signature) in signatures[many].iter().enumerate() {
            control.check()?;
            account.work(1);
            // An unbound matching row must retain its ordinary frozen-cache error.
            if one
                .get(signature / 64)
                .is_some_and(|word| word & (1_u64 << (signature % 64)) != 0)
                && self
                    .by_many
                    .get(row)
                    .is_none_or(|ordinal| *ordinal == usize::MAX)
            {
                return Ok(());
            }
        }
        control.check()?;
        // Direct traversal no longer reconstructs reservations from saved hashes.
        self.mode = Mode::Frontier(Cell::new(0));
        Ok(())
    }

    pub(crate) fn has_frontier(&self) -> bool {
        matches!(self.mode, Mode::Frontier(_))
    }

    pub(crate) fn certified_pair(&self, ordinal: usize) -> Option<[usize; 2]> {
        self.has_frontier()
            .then(|| self.pairs.get(ordinal).copied())
            .flatten()
            .filter(|pair| !pair.contains(&usize::MAX))
    }

    pub(crate) fn clear(&self) {
        self.credits.borrow_mut().clear();
        if let Mode::Frontier(frontier) = &self.mode {
            frontier.set(0);
        }
    }

    pub(crate) fn reset(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        self.clear();
        let mut credits = self.credits.borrow_mut();
        for _ in 0..self.pairs.len().div_ceil(32) {
            control.check()?;
            account.work(1);
            credits.push(0);
        }
        control.check()
    }

    pub(crate) fn ordinal(
        &self,
        rows: [usize; 2],
        many: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        control.check()?;
        account.work(1);
        let Some(&ordinal) = self.by_many.get(rows[many]) else {
            return Ok(None);
        };
        Ok(self
            .pairs
            .get(ordinal)
            .filter(|pair| **pair == rows)
            .map(|_| ordinal))
    }

    pub(crate) fn state(
        &self,
        ordinal: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<bool>, Interrupted> {
        control.check()?;
        account.work(1);
        let credits = self.credits.borrow();
        Ok(Self::credit(&credits, ordinal))
    }

    fn credit(credits: &[u64], ordinal: usize) -> Option<bool> {
        credits
            .get(ordinal / 32)
            .and_then(|word| match (word >> (2 * (ordinal % 32))) & 3 {
                1 => Some(false),
                2 => Some(true),
                _ => None,
            })
    }

    fn active_ordinal(&self, row: usize, inputs: &Inputs<'_>) -> Option<usize> {
        let &ordinal = self.by_many.get(row)?;
        let pair = self.pairs.get(ordinal)?;
        pair.iter()
            .enumerate()
            .all(|(side, &row)| {
                let position = row + if side == 0 { 0 } else { inputs.left_rows };
                inputs
                    .members
                    .get(position / 64)
                    .is_some_and(|word| word & (1_u64 << (position % 64)) != 0)
            })
            .then_some(ordinal)
    }

    fn first_unknown(
        &self,
        mut row: usize,
        inputs: &Inputs<'_>,
        credits: &[u64],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<usize, Interrupted> {
        while row < self.by_many.len() {
            control.check()?;
            account.work(1);
            if self
                .active_ordinal(row, inputs)
                .is_some_and(|ordinal| Self::credit(credits, ordinal).is_none())
            {
                break;
            }
            row += 1;
        }
        Ok(row)
    }

    pub(crate) fn next(
        &self,
        position: usize,
        target: Option<usize>,
        inputs: &Inputs<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<[usize; 2]>, Interrupted> {
        let Mode::Frontier(frontier) = &self.mode else {
            unreachable!("certified pair frontier")
        };
        let row = if let Some(target) = target {
            let credits = self.credits.borrow();
            let next = self.first_unknown(frontier.get(), inputs, &credits, control, account)?;
            control.check()?;
            frontier.set(next);
            let unknown = if next < position {
                self.first_unknown(position, inputs, &credits, control, account)?
            } else {
                next
            };
            let requested = self.pairs.get(target).and_then(|pair| {
                let row = pair[inputs.many];
                (row >= position && self.active_ordinal(row, inputs) == Some(target)).then_some(row)
            });
            requested.map_or(unknown, |row| row.min(unknown))
        } else {
            // An unscoped traversal still reads every active pair freshly.
            let mut row = position;
            while row < self.by_many.len() {
                control.check()?;
                account.work(1);
                if self.active_ordinal(row, inputs).is_some() {
                    break;
                }
                row += 1;
            }
            row
        };
        control.check()?;
        Ok(self
            .active_ordinal(row, inputs)
            .map(|ordinal| self.pairs[ordinal]))
    }

    pub(crate) fn hash(&self, ordinal: usize) -> u64 {
        let Mode::Prefix(hashes) = &self.mode else {
            unreachable!("ordinary pair-prefix hashes")
        };
        hashes[ordinal]
    }

    pub(crate) fn record(
        &self,
        ordinal: usize,
        kept: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        control.check()?;
        account.work(1);
        control.check()?;
        if let Some(word) = self.credits.borrow_mut().get_mut(ordinal / 32) {
            let shift = 2 * (ordinal % 32);
            *word = (*word & !(3 << shift)) | ((1 + u64::from(kept)) << shift);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::EngineAccounting;

    fn reversed_plan(control: &Control, account: &QueryAccounting) -> Plan {
        let mut pairs = AccountedVec::new(account, MemoryKind::Workspace);
        let mut hashes = AccountedVec::new(account, MemoryKind::Workspace);
        for ordinal in 0..65 {
            pairs.push([64 - ordinal, ordinal]);
            hashes.push(ordinal as u64 + 42);
        }
        Plan::new(pairs, hashes, 0, 65, control, account)
            .unwrap()
            .unwrap()
    }

    fn frontier(plan: &Plan) -> usize {
        let Mode::Frontier(frontier) = &plan.mode else {
            panic!("certified frontier expected")
        };
        frontier.get()
    }

    fn reversed_frontier(control: &Control, account: &QueryAccounting) -> Plan {
        let mut plan = reversed_plan(control, account);
        let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
        let left: Vec<_> = (0..65).rev().collect();
        let right: Vec<_> = (0..65).collect();
        plan.certify_frontier([&left, &right], 65, 0, control, account)
            .unwrap();
        assert!(plan.has_frontier());
        assert!(account.snapshot().memory[MemoryKind::Workspace as usize].current < before);
        plan.reset(control, account).unwrap();
        plan
    }

    #[test]
    fn physical_frontier_preserves_sparse_backward_suppressed_and_full_reads() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let plan = reversed_frontier(&control, &account);
        let mut members = [0_u64; 3];
        for (left, right) in [(0, 64), (32, 32), (64, 0)] {
            for position in [left, 65 + right] {
                members[position / 64] |= 1 << (position % 64);
            }
        }
        let inputs = Inputs {
            members: &members,
            left_rows: 65,
            many: 0,
        };
        assert_eq!(
            plan.next(0, Some(0), &inputs, &control, &account).unwrap(),
            Some([0, 64])
        );
        plan.record(64, false, &control, &account).unwrap();
        assert_eq!(
            plan.next(0, Some(64), &inputs, &control, &account).unwrap(),
            Some([0, 64])
        );
        assert_eq!(frontier(&plan), 32);
        assert_eq!(plan.state(64, &control, &account).unwrap(), Some(false));
        assert_eq!(
            plan.next(1, Some(0), &inputs, &control, &account).unwrap(),
            Some([32, 32])
        );
        plan.record(32, true, &control, &account).unwrap();
        assert_eq!(
            plan.next(33, Some(99), &inputs, &control, &account)
                .unwrap(),
            Some([64, 0])
        );
        plan.record(0, true, &control, &account).unwrap();
        assert_eq!(
            plan.next(65, Some(99), &inputs, &control, &account)
                .unwrap(),
            None
        );
        assert_eq!(frontier(&plan), 65);
        assert_eq!(
            plan.next(0, Some(64), &inputs, &control, &account).unwrap(),
            Some([0, 64])
        );
        assert_eq!(
            plan.next(0, None, &inputs, &control, &account).unwrap(),
            Some([0, 64])
        );
        assert_eq!(
            plan.next(1, None, &inputs, &control, &account).unwrap(),
            Some([32, 32])
        );
        plan.clear();
        assert_eq!(frontier(&plan), 0);
        plan.reset(&control, &account).unwrap();
        assert_eq!(
            plan.next(0, Some(0), &inputs, &control, &account).unwrap(),
            Some([0, 64])
        );
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn frontier_allows_globally_unmatched_rows_without_visiting_them() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut pairs = AccountedVec::new(&account, MemoryKind::Workspace);
        let mut hashes = AccountedVec::new(&account, MemoryKind::Workspace);
        for (pair, hash) in [([0, 0], 42), ([2, 1], 43)] {
            pairs.push(pair);
            hashes.push(hash);
        }
        let mut plan = Plan::new(pairs, hashes, 0, 3, &control, &account)
            .unwrap()
            .unwrap();
        plan.certify_frontier([&[0, 2, 1], &[0, 1]], 3, 0, &control, &account)
            .unwrap();
        assert!(plan.has_frontier());
        plan.reset(&control, &account).unwrap();
        let inputs = Inputs {
            members: &[31],
            left_rows: 3,
            many: 0,
        };
        assert_eq!(
            plan.next(0, Some(1), &inputs, &control, &account).unwrap(),
            Some([0, 0])
        );
        plan.record(0, true, &control, &account).unwrap();
        assert_eq!(
            plan.next(1, Some(1), &inputs, &control, &account).unwrap(),
            Some([2, 1])
        );
        assert_eq!(frontier(&plan), 2);
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn frontier_and_certificate_publish_only_after_terminal_control() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let plan = reversed_frontier(&control, &account);
        for boundary in [1, 65] {
            plan.reset(&control, &account).unwrap();
            for ordinal in 0..65 {
                plan.record(ordinal, true, &control, &account).unwrap();
            }
            let inputs = Inputs {
                members: &[u64::MAX; 3],
                left_rows: 65,
                many: 0,
            };
            let cancelled = Control::default();
            let result =
                crate::test_storage::interrupt_after_work(&account, &cancelled, boundary, || {
                    plan.next(0, Some(99), &inputs, &cancelled, &account)
                });
            assert!(matches!(result, Err(Interrupted::Cancelled)));
            assert_eq!(frontier(&plan), 0);
        }
        drop(plan);
        let mut plan = reversed_plan(&control, &account);
        let left: Vec<_> = (0..65).rev().collect();
        let right: Vec<_> = (0..65).collect();
        let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
        let cancelled = Control::default();
        let result = crate::test_storage::interrupt_after_work(&account, &cancelled, 50, || {
            plan.certify_frontier([&left, &right], 65, 0, &cancelled, &account)
        });
        assert!(matches!(result, Err(Interrupted::Cancelled)));
        assert!(!plan.has_frontier());
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].current,
            before
        );
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn sparse_pair_credit_crosses_words_without_retaining_failed_publication() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut pairs = AccountedVec::new(&account, MemoryKind::Workspace);
        let mut hashes = AccountedVec::new(&account, MemoryKind::Workspace);
        for ordinal in 0..65 {
            pairs.push([64 - ordinal, ordinal]);
            hashes.push(ordinal as u64 + 42);
        }
        let plan = Plan::new(pairs, hashes, 0, 65, &control, &account)
            .unwrap()
            .unwrap();
        plan.reset(&control, &account).unwrap();
        for (ordinal, kept) in [(0, true), (31, false), (32, true), (63, false), (64, true)] {
            plan.record(ordinal, kept, &control, &account).unwrap();
        }
        for ordinal in 0..65 {
            let expected = match ordinal {
                0 | 32 | 64 => Some(true),
                31 | 63 => Some(false),
                _ => None,
            };
            assert_eq!(plan.state(ordinal, &control, &account).unwrap(), expected);
            assert_eq!(
                plan.ordinal([64 - ordinal, ordinal], 0, &control, &account)
                    .unwrap(),
                Some(ordinal)
            );
            assert_eq!(plan.hash(ordinal), ordinal as u64 + 42);
        }
        assert_eq!(plan.ordinal([64, 1], 0, &control, &account).unwrap(), None);
        let cancelled = Control::default();
        let error = crate::test_storage::interrupt_after_work(&account, &cancelled, 1, || {
            plan.record(1, true, &cancelled, &account)
        });
        assert!(matches!(error, Err(Interrupted::Cancelled)));
        assert_eq!(plan.state(1, &control, &account).unwrap(), None);
        plan.clear();
        plan.reset(&control, &account).unwrap();
        for ordinal in 0..65 {
            assert_eq!(plan.state(ordinal, &control, &account).unwrap(), None);
        }
        drop(plan);
        account.snapshot().assert_memory_released();
    }
}
