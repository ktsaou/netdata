// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    accounting::QueryAccounting,
    histogram::HistogramError,
    storage::{Control, Interrupted},
};

#[derive(Clone, Copy, Default)]
pub(crate) struct HistogramWork<'a> {
    control: Option<&'a Control>,
    account: Option<&'a QueryAccounting>,
}
impl<'a> HistogramWork<'a> {
    pub(crate) fn new(control: &'a Control, account: &'a QueryAccounting) -> Self {
        Self {
            control: Some(control),
            account: Some(account),
        }
    }
    pub(crate) fn step(self, units: usize) -> Result<(), Interrupted> {
        if let Some(control) = self.control {
            control.check()?;
        }
        if let Some(account) = self.account {
            account.work(units as u64);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum HistogramEvaluationError {
    Arithmetic(HistogramError),
    Interrupted(Interrupted),
}
impl From<HistogramError> for HistogramEvaluationError {
    fn from(error: HistogramError) -> Self {
        Self::Arithmetic(error)
    }
}
impl From<Interrupted> for HistogramEvaluationError {
    fn from(error: Interrupted) -> Self {
        Self::Interrupted(error)
    }
}
impl HistogramEvaluationError {
    pub(crate) fn panic_arithmetic(self) -> Self {
        match self {
            Self::Arithmetic(error) => panic!("{error}"),
            error => error,
        }
    }
    pub(crate) fn arithmetic(self) -> HistogramError {
        match self {
            Self::Arithmetic(error) => error,
            Self::Interrupted(_) => unreachable!("inert histogram operation"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{AllocationContext, EngineAccounting, MemoryKind},
        histogram::{FloatHistogram, Span},
        histogram_counter::detect_reset_with,
        result::OwnedValue,
        test_storage::interrupt_during,
        value::SampleValueRef,
    };

    #[test]
    fn long_native_operations_observe_cancellation_inside_bucket_loops() {
        let histogram = FloatHistogram {
            count: 400_000.0,
            sum: 400_000.0,
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: 400_000,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![1.0; 400_000].into()),
            ..FloatHistogram::default()
        };
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let context = AllocationContext::new(&account, MemoryKind::Workspace);
        let copied = interrupt_during(&account, &control, || {
            histogram.as_ref().copy_to_schema_with(
                -4,
                Some(&context),
                HistogramWork::new(&control, &account),
            )
        });
        assert!(matches!(
            copied,
            Err(HistogramEvaluationError::Interrupted(
                Interrupted::Cancelled
            ))
        ));
        assert!(account.snapshot().memory[MemoryKind::Workspace as usize].peak > 0);
        account.snapshot().assert_memory_released();

        let account = engine.query();
        let control = Control::default();
        let reset = interrupt_during(&account, &control, || {
            detect_reset_with(
                histogram.as_ref(),
                histogram.as_ref(),
                HistogramWork::new(&control, &account),
            )
        });
        assert_eq!(reset, Err(Interrupted::Cancelled));
        account.snapshot().assert_memory_released();

        let account = engine.query();
        let control = Control::default();
        let equality = interrupt_during(&account, &control, || {
            histogram
                .as_ref()
                .equals_with(histogram.as_ref(), HistogramWork::new(&control, &account))
        });
        assert_eq!(equality, Err(Interrupted::Cancelled));
        account.snapshot().assert_memory_released();

        let account = engine.query();
        let control = Control::default();
        let OwnedValue::Histogram(mut trimmed) = OwnedValue::copy_from(
            SampleValueRef::FloatHistogram(histogram.as_ref()),
            &account,
            MemoryKind::Workspace,
        ) else {
            unreachable!()
        };
        let trim = interrupt_during(&account, &control, || {
            trimmed.trim_checked(f64::INFINITY, false, &control, &account)
        });
        assert_eq!(trim, Err(Interrupted::Cancelled));
        drop(trimmed);
        account.snapshot().assert_memory_released();

        let account = engine.query();
        let control = Control::default();
        let OwnedValue::Histogram(mut value) = OwnedValue::copy_from(
            SampleValueRef::FloatHistogram(histogram.as_ref()),
            &account,
            MemoryKind::Workspace,
        ) else {
            unreachable!()
        };
        let arithmetic = interrupt_during(&account, &control, || {
            value.arithmetic_checked(histogram.as_ref(), false, &control, &account)
        });
        assert!(matches!(
            arithmetic,
            Err(HistogramEvaluationError::Interrupted(
                Interrupted::Cancelled
            ))
        ));
        drop(value);
        account.snapshot().assert_memory_released();
    }
}
