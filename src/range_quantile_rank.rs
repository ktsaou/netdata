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
}
impl Selector {
    pub(crate) fn new(plan: Plan) -> Self {
        Self {
            ranks: plan.ordinals,
            prefixes: [0; 2],
            counters: [[0; 16]; 2],
            nibble: 0,
        }
    }

    pub(crate) fn receive(&mut self, value: f64, account: &QueryAccounting) {
        let shift = 60 - 4 * self.nibble;
        let mask = if self.nibble == 0 {
            0
        } else {
            u64::MAX << (shift + 4)
        };
        account.work(1);
        let key = ordered_key(value);
        let digit = ((key >> shift) & 15) as usize;
        for lane in 0..2 {
            account.work(1);
            if key & mask == self.prefixes[lane] {
                self.counters[lane][digit] += 1;
            }
        }
    }

    pub(crate) fn finish_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<[u64; 2]>, StorageError> {
        let shift = 60 - 4 * self.nibble;
        for lane in 0..2 {
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
            self.prefixes[lane] |= (digit as u64) << shift;
        }
        self.nibble += 1;
        if self.nibble < 16 {
            self.counters = [[0; 16]; 2];
            return Ok(None);
        }
        Ok(Some(self.prefixes))
    }
}
