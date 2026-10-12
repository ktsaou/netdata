// SPDX-License-Identifier: GPL-3.0-or-later
//! Query-owned completed folds and source-certified cursor progress, without raw history.
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::AnnotationKind,
    evaluator::{EvaluationError, EvaluationOptions},
    frontend::ast::NodeId,
    matrix_reader::{Geometry, MatrixWindow, Progress},
    range_float::RateInput,
    range_numeric::{NumericRangeInput, NumericRangeOutput},
    range_rates::RateOutput,
    range_reduce::{FoldResult, RangeFunction},
    result::OwnedValue,
    shared::Strong,
    storage::{Control, Interrupted, TimeBounds},
};
use std::cell::RefCell;

#[derive(Clone, Copy, Eq, PartialEq)]
struct Scope {
    start_ms: i64,
    end_ms: i64,
    step_ms: i64,
    lookback_ms: i64,
    start_timestamps: bool,
    delayed_name_removal: bool,
}
impl From<EvaluationOptions> for Scope {
    fn from(options: EvaluationOptions) -> Self {
        Self {
            start_ms: options.start_ms,
            end_ms: options.end_ms,
            step_ms: options.step_ms,
            lookback_ms: options.lookback_ms,
            start_timestamps: options.start_timestamps,
            delayed_name_removal: options.delayed_name_removal,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Input {
    Rate {
        start_ms: i64,
        end_ms: i64,
        range_ns: i64,
        start_timestamps: bool,
        range_start_ms: i64,
        range_end_ms: i64,
        data_bounds: Option<[i64; 2]>,
    },
    Fold {
        bounds: Option<[i64; 2]>,
        anchor_start_ms: Option<i64>,
    },
    Numeric {
        bounds: Option<[i64; 2]>,
        seek_ms: i64,
    },
}
fn bounds_key(bounds: Option<TimeBounds>) -> Option<[i64; 2]> {
    bounds.map(|bounds| [bounds.start_ms, bounds.end_ms])
}
impl From<RateInput> for Input {
    fn from(input: RateInput) -> Self {
        Self::Rate {
            start_ms: input.window.start_ms,
            end_ms: input.window.end_ms,
            range_ns: input.window.range_ns,
            start_timestamps: input.window.start_timestamps,
            range_start_ms: input.range_start_ms,
            range_end_ms: input.range_end_ms,
            data_bounds: bounds_key(input.data_bounds),
        }
    }
}
impl From<NumericRangeInput> for Input {
    fn from(input: NumericRangeInput) -> Self {
        Self::Numeric {
            bounds: bounds_key(input.bounds),
            seek_ms: input.seek_ms,
        }
    }
}
impl Input {
    pub(crate) fn fold(bounds: Option<TimeBounds>, anchor_start_ms: Option<i64>) -> Self {
        Self::Fold {
            bounds: bounds_key(bounds),
            anchor_start_ms,
        }
    }
}

pub(crate) struct Request {
    pub(crate) owner: NodeId,
    pub(crate) selector: NodeId,
    pub(crate) ordinal: usize,
    pub(crate) options: EvaluationOptions,
    pub(crate) lookback_ns: i64,
    pub(crate) position: i64,
    pub(crate) function: RangeFunction,
    pub(crate) parameters: [f64; 2],
    pub(crate) range_ns: i64,
    pub(crate) window: MatrixWindow,
    pub(crate) input: Input,
}
#[derive(Clone, Copy, Eq, PartialEq)]
struct Key {
    scope: Scope,
    lookback_ns: i64,
    time_ms: i64,
    position: i64,
    function: RangeFunction,
    parameter_bits: [u64; 2],
    range_ns: i64,
    geometry: Geometry,
    end_ms: i64,
    input: Input,
}
impl From<&Request> for Key {
    fn from(request: &Request) -> Self {
        Self {
            scope: request.options.into(),
            lookback_ns: request.lookback_ns,
            time_ms: request.options.time_ms,
            position: request.position,
            function: request.function,
            parameter_bits: request.parameters.map(f64::to_bits),
            range_ns: request.range_ns,
            geometry: request.window.into(),
            end_ms: request.window.end_ms,
            input: request.input,
        }
    }
}

pub(crate) struct Folded {
    float_samples: Option<crate::statistics::OwnedSamples>,
    pub(crate) value: Option<OwnedValue>,
    pub(crate) annotations: [Option<AnnotationKind>; 6],
    pub(crate) invalid_quantile: bool,
    pub(crate) nonempty: bool,
    progress: Option<Progress>,
}
impl From<RateOutput> for Folded {
    fn from(output: RateOutput) -> Self {
        Self {
            float_samples: output.float_samples,
            value: output.value,
            annotations: output.annotations,
            invalid_quantile: false,
            nonempty: output.nonempty,
            progress: output.progress,
        }
    }
}
impl From<FoldResult> for Folded {
    fn from(output: FoldResult) -> Self {
        Self {
            float_samples: output.float_samples,
            value: output.value,
            annotations: [
                output.annotations[0],
                output.annotations[1],
                None,
                None,
                None,
                None,
            ],
            invalid_quantile: false,
            nonempty: output.nonempty,
            progress: output.progress,
        }
    }
}
impl From<NumericRangeOutput> for Folded {
    fn from(output: NumericRangeOutput) -> Self {
        Self {
            float_samples: output.float_samples,
            value: output.value.map(OwnedValue::Float),
            annotations: [output.annotation, None, None, None, None, None],
            invalid_quantile: output.invalid_quantile,
            nonempty: output.nonempty,
            progress: output.progress,
        }
    }
}
impl Folded {
    pub(crate) fn into_parts(self) -> Option<crate::statistics::ValueParts> {
        self.value.map(|value| crate::statistics::ValueParts {
            value,
            float_samples: self.float_samples,
        })
    }
    pub(crate) fn sample_units(&self) -> u64 {
        self.progress.map_or(0, |progress| progress.sample_units)
    }
    fn copy_checked(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, EvaluationError> {
        control.check()?;
        let value = self
            .value
            .as_ref()
            .map(|value| {
                OwnedValue::from_sample_checked(
                    value.as_ref(),
                    control,
                    account,
                    MemoryKind::Workspace,
                )
            })
            .transpose()?;
        Ok(Self {
            float_samples: value
                .as_ref()
                .and_then(|value| crate::statistics::hold_float(account, value.as_ref())),
            value,
            annotations: self.annotations,
            invalid_quantile: self.invalid_quantile,
            nonempty: self.nonempty,
            progress: self.progress,
        })
    }
}

struct Snapshot {
    key: Key,
    folded: Folded,
}
struct Record {
    identity: (usize, usize, usize),
    snapshot: Option<Strong<Snapshot>>,
}
pub(crate) struct Folds {
    records: RefCell<AccountedVec<Record>>,
}
impl Folds {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            records: RefCell::new(AccountedVec::new(account, MemoryKind::Workspace)),
        }
    }

    pub(crate) fn clear(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut records = self.records.borrow_mut();
        while records.pop().is_some() {
            control.check()?;
            account.work(1);
        }
        Ok(())
    }

    fn position(
        records: &[Record],
        identity: (usize, usize, usize),
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Result<usize, usize>, Interrupted> {
        let (mut first, mut end) = (0, records.len());
        while first < end {
            control.check()?;
            account.work(1);
            let middle = first + (end - first) / 2;
            match records[middle].identity.cmp(&identity) {
                std::cmp::Ordering::Less => first = middle + 1,
                std::cmp::Ordering::Greater => end = middle,
                std::cmp::Ordering::Equal => return Ok(Ok(middle)),
            }
        }
        Ok(Err(first))
    }

    pub(crate) fn evaluate(
        &self,
        request: Request,
        control: &Control,
        account: &QueryAccounting,
        evaluate: impl FnOnce(MatrixWindow) -> Result<Folded, EvaluationError>,
    ) -> Result<Folded, EvaluationError> {
        control.check()?;
        let key = Key::from(&request);
        let identity = (request.owner.0, request.selector.0, request.ordinal);
        let (cached, progress) = {
            let mut records = self.records.borrow_mut();
            match Self::position(&records, identity, control, account)? {
                Ok(index) => match &records[index].snapshot {
                    Some(snapshot) if snapshot.key == key => (Some(snapshot.clone()), None),
                    _ => {
                        let prior = records[index].snapshot.take();
                        let progress = prior
                            .as_ref()
                            .filter(|snapshot| snapshot.key.scope == key.scope)
                            .and_then(|snapshot| snapshot.folded.progress);
                        (None, progress)
                    }
                },
                Err(_) => (None, None),
            }
        };
        if let Some(cached) = cached {
            return cached.folded.copy_checked(control, account);
        }
        let mut folded = evaluate(request.window.with_progress(progress))?;
        if folded.float_samples.is_none() {
            folded.float_samples = folded
                .value
                .as_ref()
                .and_then(|value| crate::statistics::hold_float(account, value.as_ref()));
        }
        let snapshot = Strong::new_in(
            Snapshot { key, folded },
            Some(&AllocationContext::new(account, MemoryKind::Workspace)),
        );
        {
            let mut records = self.records.borrow_mut();
            match Self::position(&records, identity, control, account)? {
                Ok(index) => records[index].snapshot = Some(snapshot.clone()),
                Err(index) => {
                    account.work(records.len().saturating_sub(index) as u64 + 1);
                    records.insert(
                        index,
                        Record {
                            identity,
                            snapshot: Some(snapshot.clone()),
                        },
                    );
                }
            }
        }
        snapshot.folded.copy_checked(control, account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{CUSTOM_BUCKETS_SCHEMA, FloatHistogram, Span},
        range_numeric::NumericRangeFunction,
        test_storage::interrupt_during,
        value::SampleValueRef,
    };
    use std::cell::Cell;

    fn request(time_ms: i64, parameter: f64) -> Request {
        let window = MatrixWindow {
            first_end_ms: 1000,
            end_ms: time_ms,
            range_ms: 1000,
            buffer_ms: None,
            step_ms: 2000,
            data_bounds: TimeBounds {
                start_ms: 0,
                end_ms: 100_000_000,
            },
            statistics: false,
            reduce_delta: true,
            resume: None,
        };
        Request {
            owner: NodeId(2),
            lookback_ns: 1_000_000_000,
            selector: NodeId(0),
            ordinal: 0,
            options: EvaluationOptions {
                time_ms,
                start_ms: 1000,
                end_ms: 100_000_000,
                step_ms: 2000,
                lookback_ms: 1000,
                start_timestamps: false,
                delayed_name_removal: true,
            },
            position: (time_ms - 1000) / 2000,
            function: RangeFunction::Numeric(NumericRangeFunction::Quantile),
            parameters: [parameter, 0.0],
            range_ns: 1_000_000_000,
            input: NumericRangeInput {
                bounds: Some(TimeBounds {
                    start_ms: time_ms - 999,
                    end_ms: time_ms,
                }),
                seek_ms: time_ms - 1000,
                matrix: Some(window),
            }
            .into(),
            window,
        }
    }
    fn completed(value: Option<OwnedValue>) -> Folded {
        Folded {
            float_samples: None,
            value,
            annotations: [
                Some(AnnotationKind::HistogramIgnoredInMixedRange),
                None,
                None,
                None,
                None,
                None,
            ],
            invalid_quantile: true,
            nonempty: true,
            progress: None,
        }
    }

    #[test]
    fn exact_parameter_bits_clock_and_lifecycle_keep_callbacks_authoritative() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let control = Control::default();
        let calls = Cell::new(0);
        for (time, parameter, expected_calls) in [
            (1000, 0.0, 1),
            (1000, 0.0, 1),
            (1000, -0.0, 2),
            (3000, -0.0, 3),
            (3000, -0.0, 3),
            (1000, -0.0, 4),
        ] {
            let output = folds
                .evaluate(request(time, parameter), &control, &account, |_| {
                    calls.set(calls.get() + 1);
                    Ok(completed(None))
                })
                .unwrap();
            assert!(output.value.is_none());
            assert!(output.nonempty && output.invalid_quantile);
            assert_eq!(
                output.annotations[0],
                Some(AnnotationKind::HistogramIgnoredInMixedRange)
            );
            assert_eq!(calls.get(), expected_calls);
        }
        folds.clear(&control, &account).unwrap();
        folds
            .evaluate(request(1000, -0.0), &control, &account, |_| {
                calls.set(calls.get() + 1);
                Ok(completed(None))
            })
            .unwrap();
        assert_eq!(calls.get(), 5);
        folds.clear(&control, &account).unwrap();
        for _ in 0..2 {
            let failure = folds.evaluate(request(1000, -0.0), &control, &account, |_| {
                calls.set(calls.get() + 1);
                Err(EvaluationError::Invalid("source acquisition failure"))
            });
            assert!(matches!(
                failure,
                Err(EvaluationError::Invalid("source acquisition failure"))
            ));
        }
        assert_eq!(calls.get(), 7);
        drop(folds);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn cached_histogram_delivery_preserves_fields_and_independent_ownership() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let control = Control::default();
        let histogram = FloatHistogram {
            schema: CUSTOM_BUCKETS_SCHEMA,
            zero_threshold: 0.25,
            zero_count: -0.0,
            sum: f64::from_bits(0xfff8_0000_0000_0045),
            negative_spans: Some(Vec::new().into()),
            negative_buckets: Some(Vec::new().into()),
            custom_values: Some(vec![1.0].into()),
            ..FloatHistogram::default()
        };
        let expected = histogram.as_ref();
        let first = folds.evaluate(request(1000, 0.0), &control, &account, |_| {
            Ok(completed(Some(OwnedValue::from_sample_checked(
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                &control,
                &account,
                MemoryKind::Workspace,
            )?)))
        });
        let first = first.unwrap();
        let second = folds
            .evaluate(request(1000, 0.0), &control, &account, |_| {
                panic!("completed fold must not reacquire storage")
            })
            .unwrap();
        for output in [&first, &second] {
            let Some(OwnedValue::Histogram(histogram)) = &output.value else {
                panic!("native fold")
            };
            let actual = histogram.as_ref();
            assert_eq!(actual.schema, expected.schema);
            assert_eq!(
                actual.zero_threshold.to_bits(),
                expected.zero_threshold.to_bits()
            );
            assert_eq!(actual.zero_count.to_bits(), expected.zero_count.to_bits());
            assert_eq!(actual.sum.to_bits(), expected.sum.to_bits());
            assert_eq!(actual.negative_spans, expected.negative_spans);
            assert_eq!(actual.negative_buckets, expected.negative_buckets);
            assert_eq!(actual.custom_values, expected.custom_values);
        }
        drop(folds);
        assert!(first.value.is_some() && second.value.is_some());
        drop(first);
        drop(second);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn replacing_clocks_retains_one_fold_and_constant_requested_bytes() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let control = Control::default();
        let mut peak = None;
        for index in 0..4096 {
            let time_ms = 1000 + 2000 * index;
            let output = folds
                .evaluate(request(time_ms, 0.0), &control, &account, |_| {
                    Ok(completed(Some(OwnedValue::Float(time_ms as f64))))
                })
                .unwrap();
            drop(output);
            let current = account.snapshot().memory[MemoryKind::Workspace as usize].peak;
            if let Some(peak) = peak {
                assert_eq!(current, peak);
            }
            peak = Some(current);
            assert_eq!(folds.records.borrow().len(), 1);
        }
        drop(folds);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn interrupted_warm_histogram_copy_releases_scratch_and_keeps_the_snapshot() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let histogram = FloatHistogram {
            schema: CUSTOM_BUCKETS_SCHEMA,
            count: 100_000.0,
            sum: 100_000.0,
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: 100_000,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![1.0; 100_000].into()),
            custom_values: Some((1..100_000).map(f64::from).collect::<Vec<_>>().into()),
            ..FloatHistogram::default()
        };
        let control = Control::default();
        let first = folds
            .evaluate(request(1000, 0.0), &control, &account, |_| {
                Ok(completed(Some(OwnedValue::from_sample_checked(
                    SampleValueRef::FloatHistogram(histogram.as_ref()),
                    &control,
                    &account,
                    MemoryKind::Workspace,
                )?)))
            })
            .unwrap();
        drop(first);
        let before = account.snapshot();
        let failure = interrupt_during(&account, &control, || {
            folds.evaluate(request(1000, 0.0), &control, &account, |_| {
                panic!("warm fold must not reacquire storage")
            })
        });
        assert!(matches!(
            failure,
            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
        ));
        let after = account.snapshot();
        for (before, after) in before.memory.iter().zip(&after.memory) {
            assert_eq!(after.current, before.current);
            assert_eq!(
                after.acquired - before.acquired,
                after.released - before.released
            );
        }
        assert!(after.work - before.work >= 8192);
        let workspace = MemoryKind::Workspace as usize;
        assert!(after.memory[workspace].acquired - before.memory[workspace].acquired >= 800_000);

        let recovered = folds
            .evaluate(request(1000, 0.0), &Control::default(), &account, |_| {
                panic!("cancelled delivery must keep the completed snapshot")
            })
            .unwrap();
        let Some(OwnedValue::Histogram(value)) = &recovered.value else {
            panic!("native cached fold")
        };
        assert_eq!(
            value.as_ref().positive_buckets,
            histogram.as_ref().positive_buckets
        );
        assert_eq!(
            value.as_ref().custom_values,
            histogram.as_ref().custom_values
        );
        assert_eq!(folds.records.borrow().len(), 1);
        drop(folds);
        assert!(recovered.value.is_some());
        drop(recovered);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}
