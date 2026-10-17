// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned promql/quantile.go:717-738; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    numerical::{floor, source_add, source_mul, source_sub},
    storage::{Control, StorageError},
};

pub(crate) const NAN_KEY: u64 = 0;
pub(crate) const ZERO_KEY: u64 = 1 << 63;

pub(crate) fn ordered_key(value: f64) -> u64 {
    if value.is_nan() {
        return NAN_KEY;
    }
    let bits = if value == 0.0 { 0 } else { value.to_bits() };
    if bits >> 63 != 0 {
        !bits
    } else {
        bits ^ ZERO_KEY
    }
}

pub(crate) fn ordered_value(key: u64) -> f64 {
    let bits = if key == NAN_KEY {
        0x7ff8_0000_0000_0000
    } else if key >> 63 == 0 {
        !key
    } else {
        key ^ ZERO_KEY
    };
    f64::from_bits(bits)
}

#[derive(Clone, Copy)]
pub(crate) struct Plan {
    pub(crate) ordinals: [u64; 2],
    weight: f64,
}
impl Plan {
    pub(crate) fn new(q: f64, count: u64) -> Self {
        assert!(count > 0 && (0.0..=1.0).contains(&q));
        let last = count as f64 - 1.0;
        let rank = source_mul(q, last);
        let lower = floor(rank).max(0.0);
        let upper = last.min(lower + 1.0);
        Self {
            ordinals: [lower as u64, upper as u64],
            weight: rank - floor(rank),
        }
    }

    pub(crate) fn interpolate(self, values: [f64; 2]) -> f64 {
        let lower = source_mul(source_sub(1.0, self.weight), values[0]);
        let upper = source_mul(values[1], self.weight);
        // Zero-weight terms and the compiled source's upper destination are observable.
        source_add(upper, lower)
    }
}

pub(crate) struct Selector {
    ranks: [u64; 2],
    prefixes: [u64; 2],
    counters: [[u64; 16]; 2],
    nibble: u8,
    seen: u8,
    nonuniform: u8,
}
impl Selector {
    pub(crate) fn new(plan: Plan) -> Self {
        Self {
            ranks: plan.ordinals,
            prefixes: [0; 2],
            counters: [[0; 16]; 2],
            nibble: 0,
            seen: 0,
            nonuniform: 0,
        }
    }

    fn mask(&self) -> u64 {
        if self.nibble == 0 {
            0
        } else {
            u64::MAX << (64 - 4 * self.nibble)
        }
    }

    pub(crate) fn receive(&mut self, value: f64, account: &QueryAccounting) {
        let shift = 60 - 4 * self.nibble;
        let mask = self.mask();
        // No control or fallible operation separates these three work units.
        account.work(3);
        let key = ordered_key(value);
        let digit = ((key >> shift) & 15) as usize;
        for lane in 0..2 {
            // Full representatives retain the same admitted high bits throughout this pass.
            if key & mask == self.prefixes[lane] & mask {
                self.counters[lane][digit] += 1;
                let bit = 1 << lane;
                if self.seen & bit == 0 {
                    self.prefixes[lane] = key;
                    self.seen |= bit;
                } else if self.prefixes[lane] != key {
                    self.nonuniform |= bit;
                }
            }
        }
    }

    pub(crate) fn finish_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<[u64; 2]>, StorageError> {
        let shift = 60 - 4 * self.nibble;
        let mask = self.mask();
        let representatives = self.prefixes;
        for (lane, representative) in representatives.into_iter().enumerate() {
            control.check()?;
            let mut selected = None;
            for (digit, count) in self.counters[lane].iter().enumerate() {
                account.work(1);
                if self.ranks[lane] < *count {
                    selected = Some(digit);
                    break;
                }
                self.ranks[lane] -= count;
            }
            let digit =
                selected.ok_or(StorageError::Invalid("quantile rank missing during replay"))?;
            // The first admitted key need not contain the selected digit.
            self.prefixes[lane] = (representative & mask) | (digit as u64) << shift;
        }
        self.nibble += 1;
        // Both ordinary rank/control validations precede this completed-population proof.
        if self.seen == 3 && self.nonuniform == 0 {
            return Ok(Some(representatives));
        }
        if self.nibble < 16 {
            self.counters = [[0; 16]; 2];
            self.seen = 0;
            self.nonuniform = 0;
            return Ok(None);
        }
        Ok(Some(self.prefixes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounting::EngineAccounting, storage::Interrupted};

    #[test]
    fn completed_populations_preserve_early_and_full_depth_ranks() {
        let near = [
            0x3ff0_0000_0000_0002,
            0x3ff0_0000_0000_0001,
            0x3ff0_0000_0000_0000,
        ]
        .map(f64::from_bits);
        let cases: &[(&[f64], f64, usize)] = &[
            (&[1.0, 2.0, 1.0, 2.0], 0.5, 2),
            (&[2.0, 1.0, 2.0], 0.0, 2),
            (&[-0.0, 0.0], 0.5, 1),
            (&[f64::from_bits(0x7ff8_0000_0000_0042), f64::NAN], 0.5, 1),
            (&[near[1], near[2], -1.0], 0.0, 16),
            (&near, 0.0, 16),
            (&near, 0.5, 16),
            (&near, 1.0, 16),
        ];
        for &(values, q, passes) in cases {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let plan = Plan::new(q, values.len() as u64);
            let mut sorted: Vec<_> = values.iter().copied().map(ordered_key).collect();
            sorted.sort_unstable();
            let expected = plan.ordinals.map(|ordinal| sorted[ordinal as usize]);
            let mut selector = Selector::new(plan);
            for pass in 1..=passes {
                for &value in values {
                    selector.receive(value, &account);
                }
                assert_eq!(
                    selector.finish_pass(&Control::default(), &account).unwrap(),
                    (pass == passes).then_some(expected),
                    "values={values:?}/q={q}/pass={pass}"
                );
            }
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn rank_completion_preserves_missing_rank_and_between_lane_cancellation() {
        for count in [0, 1] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let mut selector = Selector::new(Plan::new(0.5, 2));
            for _ in 0..count {
                selector.receive(f64::NAN, &account);
            }
            assert_eq!(
                selector.finish_pass(&Control::default(), &account),
                Err(StorageError::Invalid("quantile rank missing during replay"))
            );
            account.snapshot().assert_memory_released();
        }

        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut selector = Selector::new(Plan::new(0.5, 2));
        selector.receive(f64::NAN, &account);
        selector.receive(f64::NAN, &account);
        let before = account.snapshot().work;
        let result = crate::test_storage::interrupt_after_work(&account, &control, 1, || {
            selector.finish_pass(&control, &account)
        });
        assert_eq!(
            result,
            Err(StorageError::Interrupted(Interrupted::Cancelled))
        );
        assert_eq!(account.snapshot().work, before + 1);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn rank_reconstruction_keeps_low_bits_across_reordered_inputs() {
        for reverse in [false, true] {
            for count in 2..=35 {
                for q in [0.0, 0.25, 0.5, 0.95, 1.0] {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let plan = Plan::new(q, count);
                    let expected = plan
                        .ordinals
                        .map(|ordinal| ordered_key(f64::from_bits(ordinal + 1)));
                    let mut selector = Selector::new(plan);
                    let mut result = None;
                    for _ in 0..16 {
                        for index in 0..count {
                            let bits = if reverse { count - index } else { index + 1 };
                            selector.receive(f64::from_bits(bits), &account);
                        }
                        result = selector.finish_pass(&Control::default(), &account).unwrap();
                        if result.is_some() {
                            break;
                        }
                    }
                    assert_eq!(result, Some(expected), "reverse={reverse}/n={count}/q={q}");
                    account.snapshot().assert_memory_released();
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn rank_accounting_preserves_saturation_and_sticky_overflow() {
        for reserve in 0..=4u64 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            account.work(u64::MAX - reserve);
            let before = account.snapshot();
            let mut selector = Selector::new(Plan::new(0.5, 2));
            let bits = [
                0x7ff8_0000_0000_0042,
                0x8000_0000_0000_0000,
                0,
                0x7ff0_0000_0000_0000,
                0xfff0_0000_0000_0000,
            ][reserve as usize];
            selector.receive(f64::from_bits(bits), &account);
            let after = account.snapshot();
            assert_eq!(after.work, (u64::MAX - reserve).saturating_add(3));
            assert_eq!(after.counter_overflowed, reserve < 3);
            assert_eq!(after.memory, before.memory);
            assert_eq!(
                after.statistics.samples.total_queryable_samples,
                before.statistics.samples.total_queryable_samples
            );
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }

        let engine = EngineAccounting::default();
        let account = engine.query();
        account.work(u64::MAX);
        account.work(1);
        let before = account.snapshot();
        assert!(before.counter_overflowed);
        let mut selector = Selector::new(Plan::new(0.5, 2));
        selector.receive(1.0, &account);
        let after = account.snapshot();
        assert_eq!(after.work, u64::MAX);
        assert!(after.counter_overflowed);
        assert_eq!(after.memory, before.memory);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn rank_accounting_keeps_interruption_at_the_next_existing_poll() {
        for budget in 1..=4 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut selector = Selector::new(Plan::new(0.5, 2));
            let before = account.snapshot();
            crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                selector.receive(1.0, &account);
            });
            let after = account.snapshot();
            assert_eq!(after.work - before.work, 3);
            assert_eq!(after.memory, before.memory);
            if budget <= 3 {
                assert!(matches!(
                    selector.finish_pass(&control, &account),
                    Err(StorageError::Interrupted(Interrupted::Cancelled))
                ));
                assert_eq!(account.snapshot().work, after.work);
            } else {
                control.check().unwrap();
            }
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }
}
