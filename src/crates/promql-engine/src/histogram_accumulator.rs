// SPDX-License-Identifier: GPL-3.0-or-later
// Shared native arithmetic derives from pinned PromQL vector and range SUM/AVG; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    histogram::{COUNTER_RESET, FloatHistogramRef, HistogramError, NOT_RESET},
    result::{OwnedHistogram, OwnedValue},
    value::SampleValueRef,
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Finalization {
    Range,
    Vector,
}

pub(crate) struct NativeResult {
    pub(crate) value: OwnedHistogram,
    pub(crate) reset_collision: bool,
    pub(crate) reconciled: bool,
}

#[derive(Debug)]
pub(crate) struct NativeAccumulator {
    sum: OwnedHistogram,
    compensation: Option<OwnedHistogram>,
    average: bool,
    incremental: bool,
    reset_seen: bool,
    not_reset_seen: bool,
    reconciled: bool,
}
impl NativeAccumulator {
    pub(crate) fn new(average: bool, sum: OwnedHistogram) -> Self {
        let hint = sum.as_ref().reset_hint;
        Self {
            sum,
            compensation: None,
            average,
            incremental: false,
            reset_seen: hint == COUNTER_RESET,
            not_reset_seen: hint == NOT_RESET,
            reconciled: false,
        }
    }
    pub(crate) fn add(
        &mut self,
        value: FloatHistogramRef<'_>,
        count: f64,
        account: &QueryAccounting,
    ) -> Result<bool, HistogramError> {
        self.reset_seen |= value.reset_hint == COUNTER_RESET;
        self.not_reset_seen |= value.reset_hint == NOT_RESET;
        let reconciled = if !self.average {
            self.sum
                .kahan_add(value, &mut self.compensation)?
                .nhcb_bounds_reconciled
        } else {
            let mut reconciled = false;
            if !self.incremental {
                let mut trial_sum = self.sum.copy();
                let mut trial_compensation = self.compensation.as_ref().map(OwnedHistogram::copy);
                reconciled |= trial_sum
                    .kahan_add(value, &mut trial_compensation)?
                    .nhcb_bounds_reconciled;
                if !trial_sum.has_overflow() {
                    self.sum = trial_sum;
                    self.compensation = trial_compensation;
                    self.reconciled |= reconciled;
                    return Ok(reconciled);
                }
                drop(trial_sum);
                drop(trial_compensation);
                self.incremental = true;
                let mut mean = self.sum.copy();
                mean.div(count - 1.0);
                self.sum = mean;
                if let Some(compensation) = &mut self.compensation {
                    compensation.div(count - 1.0);
                }
            }
            let q = (count - 1.0) / count;
            if let Some(compensation) = &mut self.compensation {
                compensation.mul(q);
            }
            let OwnedValue::Histogram(mut to_add) = OwnedValue::copy_from(
                SampleValueRef::FloatHistogram(value),
                account,
                MemoryKind::Workspace,
            ) else {
                unreachable!()
            };
            to_add.div(count);
            self.sum.mul(q);
            reconciled |= self
                .sum
                .kahan_add(to_add.as_ref(), &mut self.compensation)?
                .nhcb_bounds_reconciled;
            reconciled
        };
        self.reconciled |= reconciled;
        Ok(reconciled)
    }
    pub(crate) fn finish(
        mut self,
        count: f64,
        policy: Finalization,
    ) -> Result<NativeResult, HistogramError> {
        if self.average && !self.incremental {
            self.sum.div(count);
            if let Some(compensation) = &mut self.compensation {
                compensation.div(count);
            }
        }
        if let Some(compensation) = &self.compensation {
            let result = self.sum.add(compensation.as_ref());
            // Vector aggregation ignores final correction flags; range SUM retains them.
            if policy == Finalization::Range {
                let flags = result?;
                if !self.average {
                    self.reconciled |= flags.nhcb_bounds_reconciled;
                }
            }
        }
        Ok(NativeResult {
            value: self.sum,
            reset_collision: self.reset_seen && self.not_reset_seen,
            reconciled: self.reconciled,
        })
    }
}
