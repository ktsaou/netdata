// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    label_hash,
    storage::{Control, Interrupted},
};

/// Lookup metadata never determines the owner's traversal or equality rule.
pub(crate) struct PositionIndex {
    slots: AccountedVec<u32>,
    // Without slots, zero is empty and a nonzero value is the singleton position + 1.
    // Allocated tables use the high bit for width and the remaining bits for count.
    state: usize,
}

const WIDE: usize = if usize::BITS > 32 {
    1 << (usize::BITS - 1)
} else {
    0
};

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
            self.state & !WIDE
        }
    }

    fn wide(&self) -> bool {
        !self.slots.is_empty() && self.state & WIDE != 0
    }

    fn wide_for(&self, position: usize) -> bool {
        usize::BITS > 32
            && (self.wide()
                || position >= u32::MAX as usize
                || (self.slots.is_empty() && self.state > u32::MAX as usize))
    }

    fn capacity(&self) -> usize {
        self.slots.len() >> usize::from(self.wide())
    }

    fn publish(&mut self, slots: AccountedVec<u32>, count: usize, wide: bool) {
        debug_assert!(!slots.is_empty() && count & WIDE == 0);
        self.slots = slots;
        self.state = count | if wide { WIDE } else { 0 };
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
        if self.wide() {
            find::<true>(&self.slots, hash, matches, control, account)
        } else {
            find::<false>(&self.slots, hash, matches, control, account)
        }
    }

    pub(crate) fn next_capacity_for_insert(&self) -> Option<usize> {
        if self.len() == 0 {
            None
        } else if self.slots.is_empty() {
            Some(4)
        } else {
            let capacity = self.capacity();
            let maximum = capacity / 3 * 2 + capacity % 3 * 2 / 3;
            (self.len() == maximum).then(|| {
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
        let wide = self.wide_for(position);
        if let Some(capacity) = self.next_capacity_for_insert() {
            let slots = if wide {
                self.rebuild::<true>(capacity, hash, position, &mut hash_of, control, account)?
            } else {
                self.rebuild::<false>(capacity, hash, position, &mut hash_of, control, account)?
            };
            self.publish(slots, count + 1, wide);
        } else if wide && !self.wide() {
            // Width alone must not rehash or change the physical probe order.
            let mut slots = self.widen(control, account)?;
            insert::<true>(&mut slots, hash, position, control, account)?;
            self.publish(slots, count + 1, true);
        } else {
            if wide {
                insert::<true>(&mut self.slots, hash, position, control, account)?;
            } else {
                insert::<false>(&mut self.slots, hash, position, control, account)?;
            }
            self.state += 1;
        }
        Ok(())
    }

    // Share cold growth loops across callback owners.
    #[inline(never)]
    fn rebuild<const WIDE_SLOTS: bool>(
        &self,
        capacity: usize,
        hash: u64,
        position: usize,
        hash_of: &mut dyn FnMut(usize) -> Result<u64, Interrupted>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<AccountedVec<u32>, Interrupted> {
        let mut slots = empty_slots::<WIDE_SLOTS>(capacity, control, account)?;
        self.visit_positions(control, account, |prior| {
            insert::<WIDE_SLOTS>(&mut slots, hash_of(prior)?, prior, control, account)
        })?;
        insert::<WIDE_SLOTS>(&mut slots, hash, position, control, account)?;
        Ok(slots)
    }

    fn widen(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<AccountedVec<u32>, Interrupted> {
        debug_assert!(!self.slots.is_empty() && !self.wide());
        let mut slots = empty_slots::<true>(self.slots.len(), control, account)?;
        for (position, &value) in self.slots.iter().enumerate() {
            control.check()?;
            account.work(1);
            set_value::<true>(&mut slots, position, value as usize);
        }
        Ok(slots)
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
        } else if self.wide() {
            visit_slots::<true>(&self.slots, control, account, visit)?;
        } else {
            visit_slots::<false>(&self.slots, control, account, visit)?;
        }
        Ok(())
    }
}

fn encoded(position: usize) -> usize {
    position
        .checked_add(1)
        .expect("position index ordinal overflow")
}

fn value<const WIDE_SLOTS: bool>(slots: &[u32], position: usize) -> usize {
    if WIDE_SLOTS {
        (u64::from(slots[position * 2]) | (u64::from(slots[position * 2 + 1]) << 32)) as usize
    } else {
        slots[position] as usize
    }
}

fn set_value<const WIDE_SLOTS: bool>(slots: &mut [u32], position: usize, value: usize) {
    if WIDE_SLOTS {
        slots[position * 2] = value as u32;
        slots[position * 2 + 1] = ((value as u64) >> 32) as u32;
    } else {
        slots[position] = value as u32;
    }
}

fn visit_slots<const WIDE_SLOTS: bool>(
    slots: &[u32],
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl FnMut(usize) -> Result<(), Interrupted>,
) -> Result<(), Interrupted> {
    for position in 0..slots.len() >> usize::from(WIDE_SLOTS) {
        control.check()?;
        account.work(1);
        let value = value::<WIDE_SLOTS>(slots, position);
        if value != 0 {
            visit(value - 1)?;
        }
    }
    Ok(())
}

fn find<const WIDE_SLOTS: bool>(
    slots: &[u32],
    hash: u64,
    mut matches: impl FnMut(usize) -> Result<bool, Interrupted>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<usize>, Interrupted> {
    let mask = (slots.len() >> usize::from(WIDE_SLOTS)) - 1;
    let mut slot = label_hash::avalanche(hash) as usize & mask;
    loop {
        control.check()?;
        account.work(1);
        let value = value::<WIDE_SLOTS>(slots, slot);
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

fn empty_slots<const WIDE_SLOTS: bool>(
    count: usize,
    control: &Control,
    account: &QueryAccounting,
) -> Result<AccountedVec<u32>, Interrupted> {
    control.check()?;
    let mut slots = AccountedVec::with_capacity_in(
        count
            .checked_mul(if WIDE_SLOTS { 2 } else { 1 })
            .expect("position index capacity overflow"),
        Some(&crate::accounting::AllocationContext::new(
            account,
            MemoryKind::Workspace,
        )),
    );
    let mut filled = 0;
    while filled < count {
        control.check()?;
        let amount = (count - filled).min(64);
        account.work(amount as u64);
        slots.extend_from_slice(&[0; 64][..amount]);
        if WIDE_SLOTS {
            slots.extend_from_slice(&[0; 64][..amount]);
        }
        filled += amount;
    }
    Ok(slots)
}

fn insert<const WIDE_SLOTS: bool>(
    slots: &mut [u32],
    hash: u64,
    position: usize,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    let mask = (slots.len() >> usize::from(WIDE_SLOTS)) - 1;
    let mut slot = label_hash::avalanche(hash) as usize & mask;
    loop {
        control.check()?;
        account.work(1);
        if value::<WIDE_SLOTS>(slots, slot) == 0 {
            set_value::<WIDE_SLOTS>(slots, slot, encoded(position));
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
            let value = if self.positions.wide() {
                value::<true>(&self.positions.slots, signature)
            } else {
                value::<false>(&self.positions.slots, signature)
            };
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
            if self.positions.wide_for(position) && !self.positions.wide() {
                let mut slots = self.positions.widen(control, account)?;
                set_value::<true>(&mut slots, signature, encoded(position));
                self.positions
                    .publish(slots, self.positions.len() + 1, true);
            } else {
                if self.positions.wide() {
                    set_value::<true>(&mut self.positions.slots, signature, encoded(position));
                } else {
                    set_value::<false>(&mut self.positions.slots, signature, encoded(position));
                }
                self.positions.state += 1;
            }
            return Ok(());
        }
        if self
            .positions
            .next_capacity_for_insert()
            .is_some_and(|capacity| domain <= capacity)
        {
            // Promotion must not exceed the sparse allocation this insertion already needs.
            let wide = self.positions.wide_for(position);
            let slots = if wide {
                self.dense_slots::<true>(signature, position, domain, &key_of, control, account)?
            } else {
                self.dense_slots::<false>(signature, position, domain, &key_of, control, account)?
            };
            let count = self.positions.len();
            self.positions.publish(slots, count + 1, wide);
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

    #[inline(never)]
    fn dense_slots<const WIDE_SLOTS: bool>(
        &self,
        signature: usize,
        position: usize,
        domain: usize,
        key_of: &dyn Fn(usize) -> usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<AccountedVec<u32>, Interrupted> {
        let mut slots = empty_slots::<WIDE_SLOTS>(domain, control, account)?;
        self.positions.visit_positions(control, account, |prior| {
            account.work(1);
            set_value::<WIDE_SLOTS>(&mut slots, key_of(prior), encoded(prior));
            Ok(())
        })?;
        control.check()?;
        account.work(1);
        set_value::<WIDE_SLOTS>(&mut slots, signature, encoded(position));
        Ok(slots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::EngineAccounting;

    #[test]
    fn full_width_singletons_and_positions_survive_collisions_and_growth() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut index = PositionIndex::new(&account);
        let positions = [usize::MAX - 1, usize::MAX / 2, 0, 7, 17, 32, 64];
        for (count, &position) in positions.iter().enumerate() {
            index
                .insert_new(0, position, |_| Ok(0), &control, &account)
                .unwrap();
            assert_eq!(index.len(), count + 1);
            if count == 0 {
                assert!(index.slots.is_empty());
                assert!(!index.wide());
                assert_eq!(
                    account.snapshot().memory[MemoryKind::Workspace as usize].current,
                    0
                );
            }
        }
        for &position in positions.iter().rev() {
            assert_eq!(
                index
                    .find(0, |prior| Ok(prior == position), &control, &account)
                    .unwrap(),
                Some(position)
            );
        }
        assert_eq!(index.wide(), usize::BITS > 32);
        assert_eq!(
            size_of::<PositionIndex>(),
            size_of::<(AccountedVec<usize>, usize)>()
        );
        println!(
            "slot layouts: PositionIndex={} SignatureIndex={} native_slot={} compact_slot={}",
            size_of::<PositionIndex>(),
            size_of::<SignatureIndex>(),
            size_of::<usize>(),
            size_of::<u32>()
        );
        drop(index);
        account.snapshot().assert_memory_released();
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn encoded_u32_boundary_grows_losslessly_and_preserves_rehash_order() {
        for positions in [[u32::MAX as usize - 1, 7], [7, u32::MAX as usize - 1]] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut index = PositionIndex::new(&account);
            for position in positions {
                index
                    .insert_new(0, position, |_| Ok(0), &control, &account)
                    .unwrap();
            }
            assert!(!index.wide());
            assert_eq!(index.capacity(), 4);
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].current,
                16
            );
            let mut visited = Vec::new();
            index
                .insert_new(
                    0,
                    u32::MAX as usize,
                    |prior| {
                        visited.push(prior);
                        Ok(0)
                    },
                    &control,
                    &account,
                )
                .unwrap();
            assert_eq!(visited, positions);
            assert!(index.wide());
            assert_eq!(index.capacity(), 8);
            assert_eq!(index.len(), 3);
            for position in positions.into_iter().chain([u32::MAX as usize]) {
                assert_eq!(
                    index
                        .find(0, |p| Ok(p == position), &control, &account)
                        .unwrap(),
                    Some(position)
                );
            }
            let memory = account.snapshot().memory[MemoryKind::Workspace as usize];
            assert_eq!((memory.current, memory.peak), (64, 80));
            drop(index);
            account.snapshot().assert_memory_released();
        }
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn width_only_promotion_preserves_bucket_offsets_without_rehashing() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut index = PositionIndex::new(&account);
        for position in 0..3 {
            index
                .insert_new(0, position, |_| Ok(0), &control, &account)
                .unwrap();
        }
        let before = index.slots.to_vec();
        index
            .insert_new(
                0,
                u32::MAX as usize,
                |_| panic!("width-only promotion must not rehash"),
                &control,
                &account,
            )
            .unwrap();
        assert!(index.wide());
        assert_eq!(index.capacity(), before.len());
        assert_eq!(index.len(), 4);
        for (slot, &prior) in before.iter().enumerate().filter(|(_, v)| **v != 0) {
            assert_eq!(value::<true>(&index.slots, slot), prior as usize);
        }
        let mut visited = Vec::new();
        assert_eq!(
            index
                .find(
                    0,
                    |p| {
                        visited.push(p);
                        Ok(false)
                    },
                    &control,
                    &account
                )
                .unwrap(),
            None
        );
        assert_eq!(visited, [0, 1, 2, u32::MAX as usize]);
        let memory = account.snapshot().memory[MemoryKind::Workspace as usize];
        assert_eq!((memory.current, memory.peak), (64, 96));
        println!(
            "width-only overlap: current={} peak={}",
            memory.current, memory.peak
        );
        drop(index);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn dense_promotion_retains_a_high_singleton_and_signature_offsets() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut index = SignatureIndex::new(&account);
        let high = usize::MAX - 1;
        index
            .insert_new(2, high, 3, |_| 2, &control, &account)
            .unwrap();
        assert!(index.positions.slots.is_empty());
        index
            .insert_new(0, 0, 3, |_| 2, &control, &account)
            .unwrap();
        assert!(index.dense);
        assert_eq!(index.positions.capacity(), 3);
        assert_eq!(
            index
                .find(2, |_| unreachable!(), &control, &account)
                .unwrap(),
            Some(high)
        );
        assert_eq!(
            index
                .find(0, |_| unreachable!(), &control, &account)
                .unwrap(),
            Some(0)
        );
        assert_eq!(
            index
                .find(1, |_| unreachable!(), &control, &account)
                .unwrap(),
            None
        );
        index
            .insert_new(1, 1, 3, |_| unreachable!(), &control, &account)
            .unwrap();
        assert_eq!(index.positions.len(), 3);
        drop(index);
        account.snapshot().assert_memory_released();
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn dense_width_promotion_keeps_non_power_of_two_offsets() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut index = SignatureIndex::new(&account);
        let signatures = [2, 0, 4];
        for (position, signature) in signatures.into_iter().enumerate() {
            index
                .insert_new(
                    signature,
                    position,
                    5,
                    |p| signatures[p],
                    &control,
                    &account,
                )
                .unwrap();
        }
        assert!(index.dense && !index.positions.wide());
        index
            .insert_new(
                1,
                u32::MAX as usize,
                5,
                |_| unreachable!(),
                &control,
                &account,
            )
            .unwrap();
        assert!(index.positions.wide());
        assert_eq!(index.positions.len(), 4);
        for (position, signature) in signatures.into_iter().enumerate() {
            assert_eq!(
                index
                    .find(signature, |_| unreachable!(), &control, &account)
                    .unwrap(),
                Some(position)
            );
        }
        assert_eq!(
            index
                .find(1, |_| unreachable!(), &control, &account)
                .unwrap(),
            Some(u32::MAX as usize)
        );
        assert_eq!(
            index
                .find(3, |_| unreachable!(), &control, &account)
                .unwrap(),
            None
        );
        let memory = account.snapshot().memory[MemoryKind::Workspace as usize];
        assert_eq!((memory.current, memory.peak), (40, 60));
        drop(index);
        account.snapshot().assert_memory_released();
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn interrupted_width_promotions_retain_sparse_and_dense_prior_owners() {
        for domain in [5, 4096] {
            let mut interrupted_allocations = 0;
            for budget in 1..=32 {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let mut index = SignatureIndex::new(&account);
                for position in 0..3 {
                    index
                        .insert_new(position, position, domain, |p| p, &control, &account)
                        .unwrap();
                }
                assert_eq!(index.dense, domain == 5);
                let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
                let result =
                    crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                        index.insert_new(
                            3,
                            u32::MAX as usize,
                            domain,
                            |_| unreachable!(),
                            &control,
                            &account,
                        )
                    });
                if matches!(result, Err(Interrupted::Cancelled)) {
                    let memory = account.snapshot().memory[MemoryKind::Workspace as usize];
                    assert_eq!(memory.current, before);
                    interrupted_allocations += usize::from(memory.peak > before * 2);
                    assert_eq!(index.positions.len(), 3);
                    assert!(!index.positions.wide());
                    let fresh = Control::default();
                    for position in 0..3 {
                        assert_eq!(
                            index.find(position, |p| p, &fresh, &account).unwrap(),
                            Some(position)
                        );
                    }
                    assert_eq!(index.find(3, |p| p, &fresh, &account).unwrap(), None);
                } else {
                    assert!(result.is_ok());
                    assert!(index.positions.wide());
                    assert_eq!(index.positions.len(), 4);
                }
                drop(index);
                account.snapshot().assert_memory_released();
            }
            assert!(interrupted_allocations > 0);
        }
    }

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
                (steady_slots * size_of::<u32>()) as u64
            );
            assert_eq!(observation.peak, (peak_slots * size_of::<u32>()) as u64);
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
