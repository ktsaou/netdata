// SPDX-License-Identifier: GPL-3.0-or-later
// Sorting derives from Go 1.27.1 slices/{sort,zsortanyfunc}.go; see NOTICE.
use std::{cmp::Ordering, convert::Infallible};

#[derive(Clone, Copy, PartialEq)]
enum Hint {
    Unknown,
    Increasing,
    Decreasing,
}

pub(crate) fn sort_by<T, F: Fn(&T, &T) -> Ordering>(data: &mut [T], compare: F) {
    sort_by_with(data, compare, || Ok::<_, Infallible>(())).unwrap_or_else(|error| match error {});
}

pub(crate) fn sort_by_with<T, E>(
    data: &mut [T],
    compare: impl Fn(&T, &T) -> Ordering,
    mut poll: impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let length = data.len();
    pdqsort(
        data,
        0,
        length,
        usize::BITS - length.leading_zeros(),
        &compare,
        &mut poll,
    )
}

fn less_with<T, E>(
    left: &T,
    right: &T,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<bool, E> {
    poll()?;
    Ok(cmp(left, right).is_lt())
}

fn swap_with<T, E>(
    data: &mut [T],
    a: usize,
    b: usize,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    poll()?;
    data.swap(a, b);
    Ok(())
}

fn insertion_sort<T, E>(
    data: &mut [T],
    a: usize,
    b: usize,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    for i in a + 1..b {
        let mut j = i;
        while j > a && less_with(&data[j], &data[j - 1], cmp, poll)? {
            swap_with(data, j, j - 1, poll)?;
            j -= 1;
        }
    }
    Ok(())
}

fn sift_down<T, E>(
    data: &mut [T],
    mut root: usize,
    hi: usize,
    first: usize,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    loop {
        let mut child = 2 * root + 1;
        if child >= hi {
            break;
        }
        if child + 1 < hi && less_with(&data[first + child], &data[first + child + 1], cmp, poll)? {
            child += 1;
        }
        if !less_with(&data[first + root], &data[first + child], cmp, poll)? {
            return Ok(());
        }
        swap_with(data, first + root, first + child, poll)?;
        root = child;
    }
    Ok(())
}

fn heap_sort<T, E>(
    data: &mut [T],
    a: usize,
    b: usize,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let hi = b - a;
    for i in (0..=(hi - 1) / 2).rev() {
        sift_down(data, i, hi, a, cmp, poll)?;
    }
    for i in (0..hi).rev() {
        swap_with(data, a, a + i, poll)?;
        sift_down(data, 0, i, a, cmp, poll)?;
    }
    Ok(())
}

fn pdqsort<T, E>(
    data: &mut [T],
    mut a: usize,
    mut b: usize,
    mut limit: u32,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let (mut was_balanced, mut was_partitioned) = (true, true);
    loop {
        poll()?;
        let length = b - a;
        if length <= 12 {
            return insertion_sort(data, a, b, cmp, poll);
        }
        if limit == 0 {
            return heap_sort(data, a, b, cmp, poll);
        }
        if !was_balanced {
            break_patterns(data, a, b, poll)?;
            limit -= 1;
        }
        let (mut pivot, mut hint) = choose_pivot(data, a, b, cmp, poll)?;
        if hint == Hint::Decreasing {
            reverse_range(data, a, b, poll)?;
            pivot = (b - 1) - (pivot - a);
            hint = Hint::Increasing;
        }
        if was_balanced
            && was_partitioned
            && hint == Hint::Increasing
            && partial_insertion_sort(data, a, b, cmp, poll)?
        {
            return Ok(());
        }
        if a > 0 && !less_with(&data[a - 1], &data[pivot], cmp, poll)? {
            a = partition_equal(data, a, b, pivot, cmp, poll)?;
            continue;
        }
        let (mid, partitioned) = partition(data, a, b, pivot, cmp, poll)?;
        was_partitioned = partitioned;
        // The pinned balance calculation includes the pivot on the right.
        let (left_len, right_len) = (mid - a, b - mid);
        let balance_threshold = length / 8;
        if left_len < right_len {
            was_balanced = left_len >= balance_threshold;
            pdqsort(data, a, mid, limit, cmp, poll)?;
            a = mid + 1;
        } else {
            was_balanced = right_len >= balance_threshold;
            pdqsort(data, mid + 1, b, limit, cmp, poll)?;
            b = mid;
        }
    }
}

fn partition<T, E>(
    data: &mut [T],
    a: usize,
    b: usize,
    pivot: usize,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(usize, bool), E> {
    swap_with(data, a, pivot, poll)?;
    let (mut i, mut j) = (a + 1, b - 1);
    while i <= j && less_with(&data[i], &data[a], cmp, poll)? {
        i += 1;
    }
    while i <= j && !less_with(&data[j], &data[a], cmp, poll)? {
        j -= 1;
    }
    if i > j {
        swap_with(data, j, a, poll)?;
        return Ok((j, true));
    }
    swap_with(data, i, j, poll)?;
    i += 1;
    j -= 1;
    loop {
        while i <= j && less_with(&data[i], &data[a], cmp, poll)? {
            i += 1;
        }
        while i <= j && !less_with(&data[j], &data[a], cmp, poll)? {
            j -= 1;
        }
        if i > j {
            break;
        }
        swap_with(data, i, j, poll)?;
        i += 1;
        j -= 1;
    }
    swap_with(data, j, a, poll)?;
    Ok((j, false))
}

fn partition_equal<T, E>(
    data: &mut [T],
    a: usize,
    b: usize,
    pivot: usize,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<usize, E> {
    swap_with(data, a, pivot, poll)?;
    let (mut i, mut j) = (a + 1, b - 1);
    loop {
        while i <= j && !less_with(&data[a], &data[i], cmp, poll)? {
            i += 1;
        }
        while i <= j && less_with(&data[a], &data[j], cmp, poll)? {
            j -= 1;
        }
        if i > j {
            break;
        }
        swap_with(data, i, j, poll)?;
        i += 1;
        j -= 1;
    }
    Ok(i)
}

fn partial_insertion_sort<T, E>(
    data: &mut [T],
    a: usize,
    b: usize,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<bool, E> {
    let mut i = a + 1;
    for _ in 0..5 {
        while i < b && !less_with(&data[i], &data[i - 1], cmp, poll)? {
            i += 1;
        }
        if i == b {
            return Ok(true);
        }
        if b - a < 50 {
            return Ok(false);
        }
        swap_with(data, i, i - 1, poll)?;
        if i - a >= 2 {
            let mut j = i - 1;
            // Go shifts against the global slice boundary, even when a > 0.
            while j >= 1 {
                if !less_with(&data[j], &data[j - 1], cmp, poll)? {
                    break;
                }
                swap_with(data, j, j - 1, poll)?;
                j -= 1;
            }
        }
        if b - i >= 2 {
            for j in i + 1..b {
                if !less_with(&data[j], &data[j - 1], cmp, poll)? {
                    break;
                }
                swap_with(data, j, j - 1, poll)?;
            }
        }
    }
    Ok(false)
}

fn break_patterns<T, E>(
    data: &mut [T],
    a: usize,
    b: usize,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let length = b - a;
    if length >= 8 {
        let mut random = length as u64;
        let modulus = 1usize
            .checked_shl(usize::BITS - length.leading_zeros())
            .unwrap_or(0);
        for index in a + (length / 4) * 2 - 1..=a + (length / 4) * 2 + 1 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let mut other = random as usize & modulus.wrapping_sub(1);
            if other >= length {
                other -= length;
            }
            swap_with(data, index, a + other, poll)?;
        }
    }
    Ok(())
}

fn choose_pivot<T, E>(
    data: &[T],
    a: usize,
    b: usize,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(usize, Hint), E> {
    let length = b - a;
    let (mut i, mut j, mut k) = (a + length / 4, a + (length / 4) * 2, a + (length / 4) * 3);
    let mut swaps = 0;
    if length >= 8 {
        if length >= 50 {
            i = median(data, i - 1, i, i + 1, &mut swaps, cmp, poll)?;
            j = median(data, j - 1, j, j + 1, &mut swaps, cmp, poll)?;
            k = median(data, k - 1, k, k + 1, &mut swaps, cmp, poll)?;
        }
        j = median(data, i, j, k, &mut swaps, cmp, poll)?;
    }
    let hint = match swaps {
        0 => Hint::Increasing,
        12 => Hint::Decreasing,
        _ => Hint::Unknown,
    };
    Ok((j, hint))
}

fn order2<T, E>(
    data: &[T],
    a: usize,
    b: usize,
    swaps: &mut usize,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(usize, usize), E> {
    if less_with(&data[b], &data[a], cmp, poll)? {
        *swaps += 1;
        Ok((b, a))
    } else {
        Ok((a, b))
    }
}

fn median<T, E>(
    data: &[T],
    a: usize,
    b: usize,
    c: usize,
    swaps: &mut usize,
    cmp: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<usize, E> {
    let (a, b) = order2(data, a, b, swaps, cmp, poll)?;
    let (b, _) = order2(data, b, c, swaps, cmp, poll)?;
    let (_, b) = order2(data, a, b, swaps, cmp, poll)?;
    Ok(b)
}

fn reverse_range<T, E>(
    data: &mut [T],
    a: usize,
    b: usize,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let (mut i, mut j) = (a, b - 1);
    while i < j {
        swap_with(data, i, j, poll)?;
        i += 1;
        j -= 1;
    }
    Ok(())
}

#[cfg(feature = "fixtures")]
pub fn fixture(bounds: Vec<u64>) -> serde_json::Value {
    use std::cell::Cell;

    let mut items: Vec<_> = bounds.into_iter().zip(0usize..).collect();
    let (comparisons, trace) = (Cell::new(0u64), Cell::new(14695981039346656037u64));
    sort_by(&mut items, |a, b| {
        comparisons.set(comparisons.get() + 1);
        let mut hash = trace.get();
        for ordinal in [a.1, b.1] {
            for byte in (ordinal as u64).to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(1099511628211);
            }
        }
        trace.set(hash);
        let (a, b) = (f64::from_bits(a.0), f64::from_bits(b.0));
        if a < b {
            Ordering::Less
        } else if a > b {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    });
    serde_json::json!({"items": items.into_iter().map(|(bits, ordinal)|
        serde_json::json!({"bound": format!("{bits:016x}"), "ordinal": ordinal}))
        .collect::<Vec<_>>(), "comparisons": comparisons.get(),
        "trace": format!("{:016x}", trace.get())})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{AccountedVec, EngineAccounting, MemoryKind},
        storage::{Control, Interrupted},
    };

    #[test]
    fn interruption_in_each_sort_phase_preserves_items_and_allocation_observations() {
        for phase in [
            "insertion",
            "partial",
            "partition",
            "equal",
            "heap",
            "reverse",
            "patterns",
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let mut data = AccountedVec::new(&account, MemoryKind::Workspace);
            for ordinal in 0..64usize {
                data.push((64 - ordinal, ordinal));
            }
            let memory = account.snapshot().memory;
            let control = Control::default();
            let mut visits = 0;
            let mut poll = || {
                visits += 1;
                if visits == 2 {
                    control.cancellation.cancel();
                }
                control.check()?;
                account.work(1);
                Ok::<_, Interrupted>(())
            };
            let cmp = |a: &(usize, usize), b: &(usize, usize)| a.0.cmp(&b.0);
            let result = match phase {
                "insertion" => insertion_sort(&mut data, 0, 64, &cmp, &mut poll),
                "partial" => partial_insertion_sort(&mut data, 0, 64, &cmp, &mut poll).map(|_| ()),
                "partition" => partition(&mut data, 0, 64, 32, &cmp, &mut poll).map(|_| ()),
                "equal" => partition_equal(&mut data, 0, 64, 32, &cmp, &mut poll).map(|_| ()),
                "heap" => pdqsort(&mut data, 0, 64, 0, &cmp, &mut poll),
                "reverse" => reverse_range(&mut data, 0, 64, &mut poll),
                "patterns" => break_patterns(&mut data, 0, 64, &mut poll),
                _ => unreachable!(),
            };
            assert_eq!(result, Err(Interrupted::Cancelled), "{phase}");
            assert_eq!(visits, 2, "{phase}");
            assert_eq!(account.snapshot().work, 1, "{phase}");
            assert_eq!(account.snapshot().memory, memory, "{phase}");
            let mut present = 0u64;
            for &(bound, ordinal) in &data {
                assert!(ordinal < 64);
                assert_eq!(bound, 64 - ordinal);
                assert_eq!(present & (1 << ordinal), 0);
                present |= 1 << ordinal;
            }
            assert_eq!(present, u64::MAX);
            drop(data);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn expired_control_returns_before_comparison_or_mutation() {
        let mut data = [2, 1];
        let control = Control {
            deadline: Some(std::time::Instant::now()),
            ..Control::default()
        };
        assert_eq!(
            sort_by_with(&mut data, |_, _| unreachable!(), || control.check()),
            Err(Interrupted::Deadline),
        );
        assert_eq!(data, [2, 1]);
    }

    #[test]
    fn large_pollable_metadata_sort_uses_small_stack_and_no_new_accounted_buffer() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut data = AccountedVec::new(&account, MemoryKind::Workspace);
        for ordinal in 0..100000usize {
            data.push(((ordinal * 65537) % 100003, ordinal));
        }
        let memory = account.snapshot().memory;
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(move || {
                let control = Control::default();
                let mut work = 0;
                sort_by_with(
                    &mut data,
                    |a, b| a.0.cmp(&b.0),
                    || {
                        control.check()?;
                        work += 1;
                        Ok::<_, Interrupted>(())
                    },
                )
                .unwrap();
                account.work(work);
                assert!(work >= data.len() as u64);
                assert_eq!(account.snapshot().memory, memory);
                assert!(data.windows(2).all(|items| items[0].0 < items[1].0));
                for &(bound, ordinal) in &data {
                    assert_eq!(bound, (ordinal * 65537) % 100003);
                }
                drop(data);
                account.snapshot().assert_memory_released();
            })
            .unwrap()
            .join()
            .unwrap();
        engine.snapshot().assert_memory_released();
    }
}
