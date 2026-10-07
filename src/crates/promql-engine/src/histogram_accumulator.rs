// SPDX-License-Identifier: GPL-3.0-or-later
// Shared native arithmetic derives from pinned PromQL vector and range SUM/AVG; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    histogram::{COUNTER_RESET, FloatHistogramRef, NOT_RESET},
    histogram_work::HistogramEvaluationError,
    result::{OwnedHistogram, OwnedValue},
    storage::Control,
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
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, HistogramEvaluationError> {
        control.check()?;
        account.work(1);
        self.reset_seen |= value.reset_hint == COUNTER_RESET;
        self.not_reset_seen |= value.reset_hint == NOT_RESET;
        let reconciled = if !self.average {
            self.sum
                .kahan_add_checked(value, &mut self.compensation, control, account)?
                .nhcb_bounds_reconciled
        } else {
            let mut reconciled = false;
            if !self.incremental {
                let mut trial_sum = self.sum.copy_checked(control, account)?;
                let mut trial_compensation = self
                    .compensation
                    .as_ref()
                    .map(|value| value.copy_checked(control, account))
                    .transpose()?;
                reconciled |= trial_sum
                    .kahan_add_checked(value, &mut trial_compensation, control, account)?
                    .nhcb_bounds_reconciled;
                if !trial_sum.has_overflow_checked(control, account)? {
                    self.sum = trial_sum;
                    self.compensation = trial_compensation;
                    self.reconciled |= reconciled;
                    control.check()?;
                    return Ok(reconciled);
                }
                drop(trial_sum);
                drop(trial_compensation);
                self.incremental = true;
                let mut mean = self.sum.copy_checked(control, account)?;
                mean.div_checked(count - 1.0, control, account)?;
                self.sum = mean;
                if let Some(compensation) = &mut self.compensation {
                    compensation.div_checked(count - 1.0, control, account)?;
                }
            }
            let q = (count - 1.0) / count;
            if let Some(compensation) = &mut self.compensation {
                compensation.mul_checked(q, control, account)?;
            }
            let OwnedValue::Histogram(mut to_add) = OwnedValue::copy_from_checked(
                SampleValueRef::FloatHistogram(value),
                control,
                account,
                MemoryKind::Workspace,
            )?
            else {
                unreachable!()
            };
            to_add.div_checked(count, control, account)?;
            self.sum.mul_checked(q, control, account)?;
            reconciled |= self
                .sum
                .kahan_add_checked(to_add.as_ref(), &mut self.compensation, control, account)?
                .nhcb_bounds_reconciled;
            reconciled
        };
        self.reconciled |= reconciled;
        control.check()?;
        Ok(reconciled)
    }
    pub(crate) fn finish(
        mut self,
        count: f64,
        policy: Finalization,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<NativeResult, HistogramEvaluationError> {
        control.check()?;
        account.work(1);
        if self.average && !self.incremental {
            self.sum.div_checked(count, control, account)?;
            if let Some(compensation) = &mut self.compensation {
                compensation.div_checked(count, control, account)?;
            }
        }
        if let Some(compensation) = &self.compensation {
            let result =
                self.sum
                    .arithmetic_checked(compensation.as_ref(), false, control, account);
            // Vector aggregation ignores final correction flags; range SUM retains them.
            match result {
                Err(HistogramEvaluationError::Interrupted(error)) => return Err(error.into()),
                Err(error) if policy == Finalization::Range => return Err(error),
                Ok(flags) if policy == Finalization::Range && !self.average => {
                    self.reconciled |= flags.nhcb_bounds_reconciled;
                }
                _ => (),
            }
        }
        control.check()?;
        Ok(NativeResult {
            value: self.sum,
            reset_collision: self.reset_seen && self.not_reset_seen,
            reconciled: self.reconciled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{FloatHistogram, Span},
        storage::{Control, Interrupted},
        test_storage::interrupt_during,
    };

    #[test]
    fn native_accumulation_observes_bucket_work_and_cancellation() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let histogram = FloatHistogram {
            count: 16_384.0,
            sum: 16_384.0,
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: 16_384,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![1.0; 16_384].into()),
            ..FloatHistogram::default()
        };
        let OwnedValue::Histogram(sum) = OwnedValue::copy_from(
            SampleValueRef::FloatHistogram(histogram.as_ref()),
            &account,
            MemoryKind::Workspace,
        ) else {
            unreachable!()
        };
        let mut accumulator = NativeAccumulator::new(false, sum);
        let result = interrupt_during(&account, &control, || {
            match accumulator.add(histogram.as_ref(), 2.0, &control, &account) {
                Ok(_) => control.check(),
                Err(HistogramEvaluationError::Interrupted(error)) => Err(error),
                Err(error) => panic!("{error:?}"),
            }
        });
        drop(accumulator);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
        assert_eq!(result, Err(Interrupted::Cancelled));
    }

    #[test]
    fn native_average_and_both_finalization_policies_propagate_bucket_interruption() {
        for operation in [
            "average-add",
            "incremental-add",
            "range-final",
            "vector-final",
            "average-final",
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let histogram = FloatHistogram {
                count: 16_384.0,
                sum: 16_384.0,
                positive_spans: Some(
                    vec![Span {
                        offset: 0,
                        length: 16_384,
                    }]
                    .into(),
                ),
                positive_buckets: Some(vec![1.0; 16_384].into()),
                ..FloatHistogram::default()
            };
            let OwnedValue::Histogram(sum) = OwnedValue::copy_from(
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                &account,
                MemoryKind::Workspace,
            ) else {
                unreachable!()
            };
            let mut accumulator = NativeAccumulator::new(
                matches!(
                    operation,
                    "average-add" | "incremental-add" | "average-final"
                ),
                sum,
            );
            if operation.ends_with("final") {
                accumulator
                    .sum
                    .kahan_add(histogram.as_ref(), &mut accumulator.compensation)
                    .unwrap();
            }
            accumulator.incremental = operation == "incremental-add";
            let result = interrupt_during(&account, &control, || {
                if operation.ends_with("add") {
                    let result = accumulator
                        .add(histogram.as_ref(), 2.0, &control, &account)
                        .map(|_| ());
                    drop(accumulator);
                    result
                } else {
                    accumulator
                        .finish(
                            2.0,
                            if operation == "vector-final" {
                                Finalization::Vector
                            } else {
                                Finalization::Range
                            },
                            &control,
                            &account,
                        )
                        .map(drop)
                }
            });
            assert!(
                matches!(
                    result,
                    Err(HistogramEvaluationError::Interrupted(
                        Interrupted::Cancelled
                    ))
                ),
                "{operation}"
            );
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }
}
