// SPDX-License-Identifier: GPL-3.0-or-later
//! Successful pair identities; values and unfinished suffixes remain in the evaluator.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    storage::{Control, Interrupted},
};
use std::cell::RefCell;

pub(crate) struct Plan {
    pairs: AccountedVec<[usize; 2]>,
    by_many: AccountedVec<usize>,
    hashes: AccountedVec<u64>,
    credits: RefCell<AccountedVec<u64>>,
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
            hashes,
            credits: RefCell::new(AccountedVec::new(account, MemoryKind::Workspace)),
        }))
    }

    pub(crate) fn clear(&self) {
        self.credits.borrow_mut().clear();
    }

    pub(crate) fn reset(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut credits = self.credits.borrow_mut();
        credits.clear();
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
        Ok(credits
            .get(ordinal / 32)
            .and_then(|word| match (word >> (2 * (ordinal % 32))) & 3 {
                1 => Some(false),
                2 => Some(true),
                _ => None,
            }))
    }

    pub(crate) fn hash(&self, ordinal: usize) -> u64 {
        self.hashes[ordinal]
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
