// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    label_hash,
    storage::{Control, Interrupted},
};

/// Lookup metadata never determines the owner's traversal or equality rule.
pub(crate) struct PositionIndex {
    slots: AccountedVec<usize>,
    // Without slots, zero is empty and a nonzero value is the singleton position + 1.
    state: usize,
}

impl PositionIndex {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            slots: AccountedVec::new(account, MemoryKind::Workspace),
            state: 0,
        }
    }

    fn len(&self) -> usize {
        if self.slots.is_empty() {
            usize::from(self.state != 0)
        } else {
            self.state
        }
    }

    pub(crate) fn find(
        &self,
        hash: u64,
        mut matches: impl FnMut(usize) -> Result<bool, Interrupted>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        control.check()?;
        if self.slots.is_empty() {
            if self.state == 0 {
                return Ok(None);
            }
            account.work(1);
            let position = self.state - 1;
            return Ok(matches(position)?.then_some(position));
        }
        let mask = self.slots.len() - 1;
        let mut slot = label_hash::avalanche(hash) as usize & mask;
        loop {
            control.check()?;
            account.work(1);
            let value = self.slots[slot];
            if value == 0 {
                return Ok(None);
            }
            let position = value - 1;
            if matches(position)? {
                return Ok(Some(position));
            }
            slot = (slot + 1) & mask;
        }
    }

    pub(crate) fn next_capacity_for_insert(&self) -> Option<usize> {
        if self.len() == 0 {
            None
        } else if self.slots.is_empty() {
            Some(4)
        } else {
            let capacity = self.slots.len();
            let maximum = capacity / 3 * 2 + capacity % 3 * 2 / 3;
            (self.state == maximum).then(|| {
                capacity
                    .checked_mul(2)
                    .expect("position index capacity overflow")
            })
        }
    }

    pub(crate) fn insert_new(
        &mut self,
        hash: u64,
        position: usize,
        mut hash_of: impl FnMut(usize) -> Result<u64, Interrupted>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        control.check()?;
        if self.len() == 0 {
            self.state = encoded(position);
            return Ok(());
        }
        let count = self.len();
        if let Some(capacity) = self.next_capacity_for_insert() {
            let mut slots = empty_slots(capacity, control, account)?;
            self.visit_positions(control, account, |prior| {
                insert(&mut slots, hash_of(prior)?, prior, control, account)
            })?;
            insert(&mut slots, hash, position, control, account)?;
            self.slots = slots;
        } else {
            insert(&mut self.slots, hash, position, control, account)?;
        }
        self.state = count + 1;
        Ok(())
    }

    fn visit_positions(
        &self,
        control: &Control,
        account: &QueryAccounting,
        mut visit: impl FnMut(usize) -> Result<(), Interrupted>,
    ) -> Result<(), Interrupted> {
        control.check()?;
        if self.slots.is_empty() {
            if self.state != 0 {
                account.work(1);
                visit(self.state - 1)?;
            }
        } else {
            for &value in &*self.slots {
                control.check()?;
                account.work(1);
                if value != 0 {
                    visit(value - 1)?;
                }
            }
        }
        Ok(())
    }
}

fn encoded(position: usize) -> usize {
    position
        .checked_add(1)
        .expect("position index ordinal overflow")
}

fn empty_slots(
    count: usize,
    control: &Control,
    account: &QueryAccounting,
) -> Result<AccountedVec<usize>, Interrupted> {
    control.check()?;
    let mut slots = AccountedVec::with_capacity_in(
        count,
        Some(&crate::accounting::AllocationContext::new(
            account,
            MemoryKind::Workspace,
        )),
    );
    while slots.len() < count {
        control.check()?;
        let amount = (count - slots.len()).min(64);
        account.work(amount as u64);
        slots.extend_from_slice(&[0; 64][..amount]);
    }
    Ok(slots)
}

fn insert(
    slots: &mut [usize],
    hash: u64,
    position: usize,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    let mask = slots.len() - 1;
    let mut slot = label_hash::avalanche(hash) as usize & mask;
    loop {
        control.check()?;
        account.work(1);
        if slots[slot] == 0 {
            slots[slot] = encoded(position);
            return Ok(());
        }
        slot = (slot + 1) & mask;
    }
}

pub(crate) struct SignatureIndex {
    positions: PositionIndex,
    dense: bool,
}

impl SignatureIndex {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            positions: PositionIndex::new(account),
            dense: false,
        }
    }

    pub(crate) fn find(
        &self,
        signature: usize,
        key_of: impl Fn(usize) -> usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        if self.dense {
            control.check()?;
            account.work(1);
            let value = self.positions.slots[signature];
            Ok((value != 0).then(|| value - 1))
        } else {
            self.positions.find(
                signature as u64,
                |position| Ok(key_of(position) == signature),
                control,
                account,
            )
        }
    }

    pub(crate) fn insert_new(
        &mut self,
        signature: usize,
        position: usize,
        domain: usize,
        key_of: impl Fn(usize) -> usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        control.check()?;
        if self.dense {
            account.work(1);
            self.positions.slots[signature] = encoded(position);
            self.positions.state += 1;
            return Ok(());
        }
        if self
            .positions
            .next_capacity_for_insert()
            .is_some_and(|capacity| domain <= capacity)
        {
            // Promotion must not exceed the sparse allocation this insertion already needs.
            let mut slots = empty_slots(domain, control, account)?;
            self.positions.visit_positions(control, account, |prior| {
                account.work(1);
                slots[key_of(prior)] = encoded(prior);
                Ok(())
            })?;
            control.check()?;
            account.work(1);
            slots[signature] = encoded(position);
            let count = self.positions.len();
            self.positions.slots = slots;
            self.positions.state = count + 1;
            self.dense = true;
            return Ok(());
        }
        self.positions.insert_new(
            signature as u64,
            position,
            |prior| Ok(key_of(prior) as u64),
            control,
            account,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::EngineAccounting;

    #[test]
    fn collided_positions_keep_original_predicates_and_first_writer() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut index = PositionIndex::new(&account);
        assert_eq!(
            index.find(0, |_| Ok(true), &control, &account).unwrap(),
            None
        );
        for position in (0..128).rev() {
            assert_eq!(
                index
                    .find(0, |prior| Ok(prior == position), &control, &account)
                    .unwrap(),
                None
            );
            index
                .insert_new(0, position, |_| Ok(0), &control, &account)
                .unwrap();
        }
        for position in 0..128 {
            assert_eq!(
                index
                    .find(0, |prior| Ok(prior == position), &control, &account)
                    .unwrap(),
                Some(position)
            );
        }
        let bytes = account.snapshot().memory[MemoryKind::Workspace as usize].current;
        assert_eq!(
            index
                .find(0, |prior| Ok(prior == 127), &control, &account)
                .unwrap(),
            Some(127)
        );
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].current,
            bytes
        );
        drop(index);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn active_slots_promote_only_at_a_smaller_required_allocation() {
        for (unique, domain, steady_slots, peak_slots) in [
            (1, 4096, 0, 0),
            (32, 4096, 64, 96),
            (64, 64, 64, 96),
            (85, 85, 85, 149),
            (86, 86, 86, 150),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut index = SignatureIndex::new(&account);
            for position in 0..unique {
                assert_eq!(
                    index
                        .find(position, |prior| prior, &control, &account)
                        .unwrap(),
                    None
                );
                index
                    .insert_new(
                        position,
                        position,
                        domain,
                        |prior| prior,
                        &control,
                        &account,
                    )
                    .unwrap();
            }
            for position in 0..unique {
                assert_eq!(
                    index
                        .find(position, |prior| prior, &control, &account)
                        .unwrap(),
                    Some(position)
                );
            }
            let observation = account.snapshot().memory[MemoryKind::Workspace as usize];
            assert_eq!(
                observation.current,
                (steady_slots * size_of::<usize>()) as u64
            );
            assert_eq!(observation.peak, (peak_slots * size_of::<usize>()) as u64);
            drop(index);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn interrupted_rebuild_releases_new_slots_and_retains_prior_positions() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut index = PositionIndex::new(&account);
        for position in 0..2 {
            index
                .insert_new(
                    position as u64,
                    position,
                    |prior| Ok(prior as u64),
                    &control,
                    &account,
                )
                .unwrap();
        }
        let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
        let result = index.insert_new(
            2,
            2,
            |_| {
                control.cancellation.cancel();
                control.check()?;
                Ok(0)
            },
            &control,
            &account,
        );
        assert!(matches!(result, Err(Interrupted::Cancelled)));
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].current,
            before
        );
        let fresh_control = Control::default();
        for position in 0..2 {
            assert_eq!(
                index
                    .find(
                        position as u64,
                        |prior| Ok(prior == position),
                        &fresh_control,
                        &account
                    )
                    .unwrap(),
                Some(position)
            );
        }
        drop(index);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn cancelled_dense_promotion_retains_the_original_singleton() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut index = SignatureIndex::new(&account);
        index
            .insert_new(0, 0, 4, |prior| prior, &control, &account)
            .unwrap();
        let result = index.insert_new(
            1,
            1,
            4,
            |prior| {
                control.cancellation.cancel();
                prior
            },
            &control,
            &account,
        );
        assert!(matches!(result, Err(Interrupted::Cancelled)));
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].current,
            0
        );
        let fresh = Control::default();
        assert_eq!(
            index.find(0, |prior| prior, &fresh, &account).unwrap(),
            Some(0)
        );
        assert_eq!(
            index.find(1, |prior| prior, &fresh, &account).unwrap(),
            None
        );
        drop(index);
        account.snapshot().assert_memory_released();
    }
}
