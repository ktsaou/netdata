// SPDX-License-Identifier: GPL-3.0-or-later
// Inverse all-NaN permutation of pinned Go sort/zsortinterface.go; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    storage::{Control, StorageError},
};

fn bits(value: u128) -> u32 {
    u128::BITS - value.leading_zeros()
}

fn ancestor(parent: u64, child: u64) -> bool {
    let parent = u128::from(parent) + 1;
    let child = u128::from(child) + 1;
    let parent_depth = bits(parent);
    let child_depth = bits(child);
    child_depth >= parent_depth && child >> (child_depth - parent_depth) == parent
}

fn next_child(parent: u64, descendant: u64) -> u64 {
    let descendant = u128::from(descendant) + 1;
    let difference = bits(descendant) - bits(u128::from(parent) + 1);
    ((descendant >> (difference - 1)) - 1) as u64
}

fn terminal(root: u64, end: u64) -> u64 {
    if root >= end {
        return root;
    }
    let root = u128::from(root);
    let end = u128::from(end);
    let depth = bits((end + 1) / (root + 2)) - 1;
    let last = ((root + 2) << depth) - 2;
    if 2 * last + 1 < end {
        (2 * last + 1) as u64
    } else {
        last as u64
    }
}

fn inverse_sift(position: u64, root: u64, end: u64) -> u64 {
    let last = terminal(root, end);
    if position == last {
        root
    } else if ancestor(root, position) && ancestor(position, last) {
        next_child(position, last)
    } else {
        position
    }
}

fn swapped(position: u64, left: u64, right: u64) -> u64 {
    if position == left {
        right
    } else if position == right {
        left
    } else {
        position
    }
}

fn pattern_partners(count: u64) -> [u64; 3] {
    let mask = ((1u128 << bits(u128::from(count))) - 1) as u64;
    let mut random = count;
    std::array::from_fn(|_| {
        // The canonical source uses AMD64 xorshift even on narrower host targets.
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let index = random & mask;
        if index >= count { index - count } else { index }
    })
}

pub(crate) fn all_nan_ordinal(
    count: u64,
    mut position: u64,
    control: &Control,
    account: &QueryAccounting,
) -> Result<u64, StorageError> {
    if position >= count {
        return Err(StorageError::Invalid("quantile ordinal outside replay"));
    }
    let mut remaining = count;
    let mut limit = bits(u128::from(count));
    let mut balanced = true;
    let mut phases = 0;
    while remaining > 12 && limit > 0 {
        control.check()?;
        account.work(1);
        if !balanced {
            limit -= 1;
        }
        balanced = remaining / 8 <= 1;
        remaining -= 1;
        phases += 1;
    }
    if position < remaining {
        if remaining <= 12 {
            position = remaining - 1 - position;
        } else {
            for last in 0..remaining {
                control.check()?;
                account.work(1);
                if position < last {
                    position = inverse_sift(position, 0, last);
                }
                position = swapped(position, 0, last);
            }
            for root in 0..=(remaining - 1) / 2 {
                control.check()?;
                account.work(1);
                position = inverse_sift(position, root, remaining);
            }
        }
    }
    for phase in (0..phases).rev() {
        control.check()?;
        account.work(1);
        let length = count - phase;
        let was_balanced = phase == 0 || (u128::from(length) + 1) / 8 <= 1;
        let reversed = length >= 50;
        let pivot = if reversed {
            length - 1 - 2 * (length / 4)
        } else {
            2 * (length / 4)
        };
        position = swapped(position, 0, length - 1);
        position = swapped(position, 0, pivot);
        if position < length && was_balanced && reversed {
            position = ((u128::from(position) + 5) % u128::from(length)) as u64;
        }
        if position < length && reversed {
            position = length - 1 - position;
        }
        if !was_balanced {
            let partners = pattern_partners(length);
            let base = 2 * (length / 4) - 1;
            for index in (0..3).rev() {
                position = swapped(position, base + index as u64, partners[index]);
            }
        }
    }
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting, storage::Interrupted, test_storage::interrupt_during,
    };

    #[test]
    fn inverse_sort_retains_original_go_selected_origins() {
        // Frozen unchanged-Go ordering-native.jsonl; native and FMA-off origins agree.
        for (count, expected) in [
            (12, [11, 6, 0]),
            (13, [11, 5, 6]),
            (49, [38, 13, 24]),
            (50, [6, 31, 19]),
            (64, [60, 16, 27]),
            (100_000, [81_922, 1_565, 49_995]),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            for (position, origin) in [0, (count - 1) / 2, count - 1].into_iter().zip(expected) {
                assert_eq!(
                    all_nan_ordinal(count, position, &control, &account).unwrap(),
                    origin,
                    "count{count}/rank{position}"
                );
            }
            assert!(account.snapshot().work < 5 * count);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn inverse_heap_walk_checks_cancellation_without_raw_storage() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let result = interrupt_during(&account, &control, || {
            all_nan_ordinal(100_000, 49_999, &control, &account)
        });
        assert!(matches!(
            result,
            Err(StorageError::Interrupted(Interrupted::Cancelled))
        ));
        assert!(account.snapshot().work >= 8192);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}
