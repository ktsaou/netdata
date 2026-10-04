// SPDX-License-Identifier: GPL-3.0-or-later
// Sorting derives from Go 1.27.1 slices/{sort,zsortanyfunc}.go; see NOTICE.
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq)]
enum Hint {
    Unknown,
    Increasing,
    Decreasing,
}

pub(crate) fn sort_by<T, F: Fn(&T, &T) -> Ordering>(data: &mut [T], compare: F) {
    let length = data.len();
    pdqsort(
        data,
        0,
        length,
        usize::BITS - length.leading_zeros(),
        &compare,
    );
}

fn insertion_sort<T, F: Fn(&T, &T) -> Ordering>(data: &mut [T], a: usize, b: usize, cmp: &F) {
    for i in a + 1..b {
        let mut j = i;
        while j > a && cmp(&data[j], &data[j - 1]).is_lt() {
            data.swap(j, j - 1);
            j -= 1;
        }
    }
}

fn sift_down<T, F: Fn(&T, &T) -> Ordering>(
    data: &mut [T],
    mut root: usize,
    hi: usize,
    first: usize,
    cmp: &F,
) {
    loop {
        let mut child = 2 * root + 1;
        if child >= hi {
            break;
        }
        if child + 1 < hi && cmp(&data[first + child], &data[first + child + 1]).is_lt() {
            child += 1;
        }
        if !cmp(&data[first + root], &data[first + child]).is_lt() {
            return;
        }
        data.swap(first + root, first + child);
        root = child;
    }
}

fn heap_sort<T, F: Fn(&T, &T) -> Ordering>(data: &mut [T], a: usize, b: usize, cmp: &F) {
    let hi = b - a;
    for i in (0..=(hi - 1) / 2).rev() {
        sift_down(data, i, hi, a, cmp);
    }
    for i in (0..hi).rev() {
        data.swap(a, a + i);
        sift_down(data, 0, i, a, cmp);
    }
}

fn pdqsort<T, F: Fn(&T, &T) -> Ordering>(
    data: &mut [T],
    mut a: usize,
    mut b: usize,
    mut limit: u32,
    cmp: &F,
) {
    let (mut was_balanced, mut was_partitioned) = (true, true);
    loop {
        let length = b - a;
        if length <= 12 {
            insertion_sort(data, a, b, cmp);
            return;
        }
        if limit == 0 {
            heap_sort(data, a, b, cmp);
            return;
        }
        if !was_balanced {
            break_patterns(data, a, b);
            limit -= 1;
        }
        let (mut pivot, mut hint) = choose_pivot(data, a, b, cmp);
        if hint == Hint::Decreasing {
            reverse_range(data, a, b);
            pivot = (b - 1) - (pivot - a);
            hint = Hint::Increasing;
        }
        if was_balanced
            && was_partitioned
            && hint == Hint::Increasing
            && partial_insertion_sort(data, a, b, cmp)
        {
            return;
        }
        if a > 0 && !cmp(&data[a - 1], &data[pivot]).is_lt() {
            a = partition_equal(data, a, b, pivot, cmp);
            continue;
        }
        let (mid, partitioned) = partition(data, a, b, pivot, cmp);
        was_partitioned = partitioned;
        // The pinned balance calculation includes the pivot on the right.
        let (left_len, right_len) = (mid - a, b - mid);
        let balance_threshold = length / 8;
        if left_len < right_len {
            was_balanced = left_len >= balance_threshold;
            pdqsort(data, a, mid, limit, cmp);
            a = mid + 1;
        } else {
            was_balanced = right_len >= balance_threshold;
            pdqsort(data, mid + 1, b, limit, cmp);
            b = mid;
        }
    }
}

fn partition<T, F: Fn(&T, &T) -> Ordering>(
    data: &mut [T],
    a: usize,
    b: usize,
    pivot: usize,
    cmp: &F,
) -> (usize, bool) {
    data.swap(a, pivot);
    let (mut i, mut j) = (a + 1, b - 1);
    while i <= j && cmp(&data[i], &data[a]).is_lt() {
        i += 1;
    }
    while i <= j && !cmp(&data[j], &data[a]).is_lt() {
        j -= 1;
    }
    if i > j {
        data.swap(j, a);
        return (j, true);
    }
    data.swap(i, j);
    i += 1;
    j -= 1;
    loop {
        while i <= j && cmp(&data[i], &data[a]).is_lt() {
            i += 1;
        }
        while i <= j && !cmp(&data[j], &data[a]).is_lt() {
            j -= 1;
        }
        if i > j {
            break;
        }
        data.swap(i, j);
        i += 1;
        j -= 1;
    }
    data.swap(j, a);
    (j, false)
}

fn partition_equal<T, F: Fn(&T, &T) -> Ordering>(
    data: &mut [T],
    a: usize,
    b: usize,
    pivot: usize,
    cmp: &F,
) -> usize {
    data.swap(a, pivot);
    let (mut i, mut j) = (a + 1, b - 1);
    loop {
        while i <= j && !cmp(&data[a], &data[i]).is_lt() {
            i += 1;
        }
        while i <= j && cmp(&data[a], &data[j]).is_lt() {
            j -= 1;
        }
        if i > j {
            break;
        }
        data.swap(i, j);
        i += 1;
        j -= 1;
    }
    i
}

fn partial_insertion_sort<T, F: Fn(&T, &T) -> Ordering>(
    data: &mut [T],
    a: usize,
    b: usize,
    cmp: &F,
) -> bool {
    let mut i = a + 1;
    for _ in 0..5 {
        while i < b && !cmp(&data[i], &data[i - 1]).is_lt() {
            i += 1;
        }
        if i == b {
            return true;
        }
        if b - a < 50 {
            return false;
        }
        data.swap(i, i - 1);
        if i - a >= 2 {
            let mut j = i - 1;
            // Go shifts against the global slice boundary, even when a > 0.
            while j >= 1 {
                if !cmp(&data[j], &data[j - 1]).is_lt() {
                    break;
                }
                data.swap(j, j - 1);
                j -= 1;
            }
        }
        if b - i >= 2 {
            for j in i + 1..b {
                if !cmp(&data[j], &data[j - 1]).is_lt() {
                    break;
                }
                data.swap(j, j - 1);
            }
        }
    }
    false
}

fn break_patterns<T>(data: &mut [T], a: usize, b: usize) {
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
            data.swap(index, a + other);
        }
    }
}

fn choose_pivot<T, F: Fn(&T, &T) -> Ordering>(
    data: &[T],
    a: usize,
    b: usize,
    cmp: &F,
) -> (usize, Hint) {
    let length = b - a;
    let (mut i, mut j, mut k) = (a + length / 4, a + (length / 4) * 2, a + (length / 4) * 3);
    let mut swaps = 0;
    if length >= 8 {
        if length >= 50 {
            i = median(data, i - 1, i, i + 1, &mut swaps, cmp);
            j = median(data, j - 1, j, j + 1, &mut swaps, cmp);
            k = median(data, k - 1, k, k + 1, &mut swaps, cmp);
        }
        j = median(data, i, j, k, &mut swaps, cmp);
    }
    let hint = match swaps {
        0 => Hint::Increasing,
        12 => Hint::Decreasing,
        _ => Hint::Unknown,
    };
    (j, hint)
}

fn order2<T, F: Fn(&T, &T) -> Ordering>(
    data: &[T],
    a: usize,
    b: usize,
    swaps: &mut usize,
    cmp: &F,
) -> (usize, usize) {
    if cmp(&data[b], &data[a]).is_lt() {
        *swaps += 1;
        (b, a)
    } else {
        (a, b)
    }
}

fn median<T, F: Fn(&T, &T) -> Ordering>(
    data: &[T],
    a: usize,
    b: usize,
    c: usize,
    swaps: &mut usize,
    cmp: &F,
) -> usize {
    let (a, b) = order2(data, a, b, swaps, cmp);
    let (b, _) = order2(data, b, c, swaps, cmp);
    let (_, b) = order2(data, a, b, swaps, cmp);
    b
}

fn reverse_range<T>(data: &mut [T], a: usize, b: usize) {
    let (mut i, mut j) = (a, b - 1);
    while i < j {
        data.swap(i, j);
        i += 1;
        j -= 1;
    }
}
