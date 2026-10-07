// SPDX-License-Identifier: GPL-3.0-or-later
// Sorting derives from Go 1.27.1 slices/{sort,zsortanyfunc}.go; see NOTICE.
use crate::go_sort_program::{Operation, Program};
use std::{cmp::Ordering, convert::Infallible};

pub(crate) fn sort_by<T, F: Fn(&T, &T) -> Ordering>(data: &mut [T], compare: F) {
    sort_by_with(data, compare, || Ok::<_, Infallible>(())).unwrap_or_else(|error| match error {});
}

pub(crate) fn sort_by_with<T, E>(
    data: &mut [T],
    compare: impl Fn(&T, &T) -> Ordering,
    mut poll: impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    run_program(data, Program::sort(data.len()), &compare, &mut poll)
}

fn run_program<T, E>(
    data: &mut [T],
    mut program: Program,
    compare: &impl Fn(&T, &T) -> Ordering,
    poll: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let mut reply = None;
    loop {
        poll()?;
        match program.next(reply.take()) {
            Operation::Step => (),
            Operation::Less(a, b) => reply = Some(compare(&data[a], &data[b]).is_lt()),
            Operation::Swap(a, b) => data.swap(a, b),
            Operation::Done => return Ok(()),
        }
    }
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

#[cfg(feature = "fixtures")]
pub fn selection_order_fixture(
    request: &serde_json::Value,
) -> Result<serde_json::Value, &'static str> {
    struct Trace {
        comparisons: u64,
        swaps: u64,
        hash: u64,
    }
    impl Trace {
        fn record(&mut self, kind: u8, left: usize, right: usize) {
            self.hash = (self.hash ^ u64::from(kind)).wrapping_mul(1099511628211);
            for ordinal in [left, right] {
                for byte in (ordinal as u64).to_le_bytes() {
                    self.hash = (self.hash ^ u64::from(byte)).wrapping_mul(1099511628211);
                }
            }
        }
        fn drive(
            &mut self,
            mut program: Program,
            data: &mut [(u64, usize)],
            bottom: bool,
            reverse: bool,
        ) -> Option<Vec<usize>> {
            let mut reply = None;
            loop {
                match program.next(reply.take()) {
                    Operation::Step => (),
                    Operation::Less(mut a, mut b) => {
                        if reverse {
                            std::mem::swap(&mut a, &mut b);
                        }
                        self.comparisons += 1;
                        self.record(1, data[a].1, data[b].1);
                        let (a, b) = (f64::from_bits(data[a].0), f64::from_bits(data[b].0));
                        reply = Some(a.is_nan() || if bottom { a > b } else { a < b });
                    }
                    Operation::Swap(a, b) => {
                        self.swaps += 1;
                        self.record(2, data[a].1, data[b].1);
                        data.swap(a, b);
                    }
                    Operation::Done => return program.result(),
                }
            }
        }
    }
    fn bits(value: &serde_json::Value) -> Result<u64, &'static str> {
        let text = value
            .as_str()
            .ok_or("ordering value must be a hexadecimal string")?;
        if text.len() != 16 {
            return Err("ordering value needs 16 hexadecimal digits");
        }
        u64::from_str_radix(text, 16).map_err(|_| "invalid ordering value")
    }
    let mut data = request["values"]
        .as_array()
        .ok_or("missing ordering values")?
        .iter()
        .enumerate()
        .map(|(ordinal, value)| Ok((bits(value)?, ordinal)))
        .collect::<Result<Vec<_>, &'static str>>()?;
    let length = data.len();
    let bottom = request["bottom"]
        .as_bool()
        .ok_or("missing ordering direction")?;
    let reverse = request["reverse"]
        .as_bool()
        .ok_or("missing ordering reversal")?;
    let action = request["action"]
        .as_str()
        .ok_or("missing ordering action")?;
    let mut trace = Trace {
        comparisons: 0,
        swaps: 0,
        hash: 14695981039346656037,
    };
    let result = match action {
        "push" | "fix" => {
            let input = std::mem::take(&mut data);
            for item in input {
                data.push(item);
                trace.drive(Program::heap_push(data.len()), &mut data, bottom, false);
            }
            if action == "fix" {
                let index = request["index"]
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .filter(|index| *index < length)
                    .ok_or("invalid heap Fix index")?;
                data[index].0 = bits(&request["replacement"])?;
                trace.drive(Program::heap_fix(index, length), &mut data, bottom, false);
            }
            None
        }
        "sort" => trace.drive(Program::sort(length), &mut data, bottom, reverse),
        "insertion" | "partial" | "heap" | "pivot" | "reverse" | "patterns" | "partition"
        | "equal" => {
            if length == 0 {
                return Err("private ordering controls need a nonempty array");
            }
            let pivot = if matches!(action, "partition" | "equal") {
                request["pivot"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|value| *value < length)
                    .ok_or("invalid partition pivot")?
            } else {
                0
            };
            trace.drive(
                Program::phase(action, length, pivot),
                &mut data,
                bottom,
                reverse,
            )
        }
        _ => return Err("unknown ordering action"),
    };
    Ok(
        serde_json::json!({"items": data.into_iter().map(|(bits, ordinal)|
        serde_json::json!({"bits": format!("{bits:016x}"), "ordinal": ordinal})).collect::<Vec<_>>(),
        "comparisons": trace.comparisons, "swaps": trace.swaps,
        "trace": format!("{:016x}", trace.hash), "result": result}),
    )
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
            let result = run_program(&mut data, Program::phase(phase, 64, 32), &cmp, &mut poll);
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
