// SPDX-License-Identifier: GPL-3.0-or-later
// Repeated backing mutations derive from pinned promql/quantile.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    classic_replay::{Bound, ResultParts},
    evaluator::EvaluationError,
    go_sort::sort_by_with,
    histogram_classic::{
        BucketQuantileResult, CountCorrection, NAN, bound_order, coalesced_count, count_correction,
    },
};

pub(crate) struct Producers {
    values: AccountedVec<f64>,
    samples: crate::statistics::OwnedSamples,
}
impl Producers {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            values: AccountedVec::new(account, MemoryKind::Workspace),
            samples: crate::statistics::OwnedSamples::new(account, 0),
        }
    }
    fn push(&mut self, value: f64, samples: Option<crate::statistics::OwnedSamples>) {
        self.values.push(value);
        if let Some(samples) = samples {
            self.samples.absorb(samples);
        } else {
            self.samples.add(1);
        }
    }
    #[cfg(test)]
    fn accounting(&self) -> &QueryAccounting {
        self.samples.accounting()
    }
}
impl std::ops::Deref for Producers {
    type Target = [f64];
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Done,
    Coalesce,
    First,
    Arriving,
    Repair,
    RepairValue,
}

pub(crate) enum Action {
    Count(usize),
    Done { length: usize, result: ResultParts },
}

pub(crate) struct Mutation {
    phase: Phase,
    result: BucketQuantileResult,
    result_samples: crate::statistics::OwnedSamples,
    run: Option<Bound>,
    position: usize,
    length: usize,
    sum: Option<f64>,
    sum_samples: Option<crate::statistics::OwnedSamples>,
    previous: Option<(usize, f64)>,
    previous_samples: Option<crate::statistics::OwnedSamples>,
    waiting: bool,
}

impl Mutation {
    pub(crate) fn new<E>(
        q: f64,
        bounds: &mut [Bound],
        account: &QueryAccounting,
        mut poll: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, E> {
        let mut state = Self {
            phase: Phase::Done,
            result: BucketQuantileResult::new(NAN),
            result_samples: crate::statistics::OwnedSamples::new(account, 0),
            run: None,
            position: 0,
            length: 0,
            sum: None,
            sum_samples: None,
            previous: None,
            previous_samples: None,
            waiting: false,
        };
        if q.is_nan() {
            state.result_samples.resize(1);
            return Ok(state);
        }
        if q < 0.0 {
            state.result.value = f64::NEG_INFINITY;
            state.result_samples.resize(1);
            return Ok(state);
        }
        if q > 1.0 {
            state.result.value = f64::INFINITY;
            state.result_samples.resize(1);
            return Ok(state);
        }
        sort_by_with(
            bounds,
            |a, b| bound_order(a.upper_bound, b.upper_bound),
            &mut poll,
        )?;
        if bounds[bounds.len() - 1].upper_bound != f64::INFINITY {
            state.result_samples.resize(1);
            return Ok(state);
        }
        state.phase = Phase::Coalesce;
        state.run = Some(bounds[0]);
        state.position = 1;
        Ok(state)
    }

    pub(crate) fn next(
        &mut self,
        bounds: &mut [Bound],
        producers: &mut Producers,
        row_base: usize,
        mut poll: impl FnMut() -> Result<(), EvaluationError>,
    ) -> Result<Action, EvaluationError> {
        assert!(!self.waiting, "mutation count request has not completed");
        loop {
            poll()?;
            match self.phase {
                Phase::Done => {
                    let empty =
                        crate::statistics::OwnedSamples::new(self.result_samples.accounting(), 0);
                    return Ok(Action::Done {
                        length: self.length,
                        result: ResultParts {
                            result: self.result,
                            samples: std::mem::replace(&mut self.result_samples, empty),
                        },
                    });
                }
                Phase::Coalesce => {
                    let mut run = self.run.expect("coalesce run");
                    if self.position < bounds.len()
                        && bounds[self.position].upper_bound == run.upper_bound
                    {
                        self.phase = if self.sum.is_none() {
                            Phase::First
                        } else {
                            Phase::Arriving
                        };
                        continue;
                    }
                    if let Some(sum) = self.sum.take() {
                        run.ordinal = row_base.checked_add(producers.len()).ok_or(
                            EvaluationError::Invalid("classic producer reference overflows"),
                        )?;
                        // Only completed multi-input folds are retained; singletons stay borrowed.
                        producers.push(sum, self.sum_samples.take());
                    }
                    bounds[self.length] = run;
                    self.length += 1;
                    if self.position == bounds.len() {
                        self.position = 0;
                        self.result.min_bound = f64::INFINITY;
                        self.result.max_bound = f64::NEG_INFINITY;
                        self.phase = Phase::Repair;
                    } else {
                        self.run = Some(bounds[self.position]);
                        self.position += 1;
                    }
                }
                Phase::First => {
                    self.waiting = true;
                    return Ok(Action::Count(self.run.expect("coalesce run").ordinal));
                }
                Phase::Arriving => {
                    self.waiting = true;
                    return Ok(Action::Count(bounds[self.position].ordinal));
                }
                Phase::Repair => {
                    if self.position == self.length {
                        self.phase = Phase::Done;
                        if self.length < 2 {
                            self.result_samples.resize(1);
                        }
                    } else {
                        self.phase = Phase::RepairValue;
                        self.waiting = true;
                        return Ok(Action::Count(bounds[self.position].ordinal));
                    }
                }
                Phase::RepairValue => unreachable!("mutation count request is pending"),
            }
        }
    }

    pub(crate) fn receive(
        &mut self,
        value: f64,
        samples: Option<crate::statistics::OwnedSamples>,
        bounds: &mut [Bound],
    ) {
        assert!(self.waiting, "unsolicited mutation count");
        self.waiting = false;
        match self.phase {
            Phase::First => {
                self.sum = Some(value);
                self.sum_samples = samples;
                self.phase = Phase::Arriving;
            }
            Phase::Arriving => {
                self.sum = Some(coalesced_count(
                    self.sum.expect("coalesce accumulator"),
                    value,
                ));
                self.position += 1;
                self.phase = Phase::Coalesce;
            }
            Phase::RepairValue => {
                let bucket = &mut bounds[self.position];
                if let Some((reference, previous)) = self.previous.as_mut() {
                    match count_correction(value, bucket.upper_bound, previous, &mut self.result) {
                        CountCorrection::KeepState => (),
                        CountCorrection::UsePrevious => bucket.ordinal = *reference,
                        CountCorrection::AdvancePrevious => *reference = bucket.ordinal,
                    }
                } else {
                    self.previous = Some((bucket.ordinal, value));
                    self.previous_samples = samples;
                }
                self.position += 1;
                self.phase = Phase::Repair;
            }
            _ => unreachable!("mutation count consumer"),
        }
    }
}

#[cfg(feature = "fixtures")]
pub fn fixture(request: &serde_json::Value) -> serde_json::Value {
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        classic_replay::{self, Replay},
        storage::Control,
    };
    use serde_json::json;

    let number = |value: &serde_json::Value| {
        f64::from_bits(u64::from_str_radix(value.as_str().expect("expected bits"), 16).unwrap())
    };
    let inputs = request["buckets"].as_array().map_or(&[][..], Vec::as_slice);
    let engine = EngineAccounting::default();
    let account = engine.query();
    let control = Control::default();
    let mut bounds = AccountedVec::new(&account, MemoryKind::Workspace);
    for (ordinal, bucket) in inputs.iter().enumerate() {
        bounds.push(Bound {
            upper_bound: number(&bucket["upper_bound"]),
            ordinal,
        });
    }
    let mut producers = Producers::new(&account);
    let mut reads = 0usize;
    let mut steps = Vec::new();
    for q in request["quantiles"].as_array().expect("quantile list") {
        let q = number(q);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut poll = || {
                control.check()?;
                account.work(1);
                Ok::<_, EvaluationError>(())
            };
            let mut mutation = Mutation::new(q, &mut bounds, &account, &mut poll).unwrap();
            let (length, result) = loop {
                match mutation
                    .next(&mut bounds, &mut producers, inputs.len(), &mut poll)
                    .unwrap()
                {
                    Action::Count(reference) => {
                        let value = if reference < inputs.len() {
                            reads += 1;
                            number(&inputs[reference]["count"])
                        } else {
                            producers[reference - inputs.len()]
                        };
                        mutation.receive(
                            value,
                            Some(crate::statistics::OwnedSamples::new(&account, 1)),
                            &mut bounds,
                        );
                    }
                    Action::Done { length, result } => break (length, result),
                }
            };
            let mut replay = Replay::from_prepared(q, &bounds[..length], result);
            loop {
                match replay.next(&bounds[..length], &mut poll).unwrap() {
                    classic_replay::Action::Count(reference) => {
                        let value = if reference < inputs.len() {
                            reads += 1;
                            number(&inputs[reference]["count"])
                        } else {
                            producers[reference - inputs.len()]
                        };
                        replay.receive(
                            value,
                            Some(crate::statistics::OwnedSamples::new(&account, 1)),
                        );
                    }
                    classic_replay::Action::Done(result) => break result.result,
                }
            }
        }));
        let result = outcome.as_ref().ok().map(|result| {
            let value = result.value;
            let reciprocal = 1.0 / value;
            json!({
                "value": format!("{:016x}", value.to_bits()),
                "reciprocal": format!("{:016x}", reciprocal.to_bits()),
                "negative_reciprocal_infinite": reciprocal == f64::NEG_INFINITY,
                "positive_reciprocal_infinite": reciprocal == f64::INFINITY,
                "less_than_positive_infinity": value < f64::INFINITY,
                "quantile": {
                    "forced_monotonicity": result.forced_monotonicity,
                    "fixed_precision": result.fixed_precision,
                    "min_bound": format!("{:016x}", result.min_bound.to_bits()),
                    "max_bound": format!("{:016x}", result.max_bound.to_bits()),
                    "max_decrease": format!("{:016x}", result.max_decrease.to_bits()),
                },
            })
        });
        let after: Vec<_> = bounds
            .iter()
            .map(|bound| {
                let count = if bound.ordinal < inputs.len() {
                    number(&inputs[bound.ordinal]["count"])
                } else {
                    producers[bound.ordinal - inputs.len()]
                };
                json!({"upper_bound": format!("{:016x}", bound.upper_bound.to_bits()),
                "count": format!("{:016x}", count.to_bits())})
            })
            .collect();
        let after = if request["buckets"].is_null() {
            serde_json::Value::Null
        } else {
            json!(after)
        };
        steps.push(json!({"panicked": outcome.is_err(), "result": result, "input_after": after}));
    }
    let observations = account.snapshot();
    let producer_count = producers.len();
    drop((bounds, producers));
    assert_eq!(
        account.snapshot().memory[MemoryKind::Workspace as usize].current,
        0
    );
    json!({"steps": steps, "reads": reads, "producers": producer_count,
        "work": observations.work, "workspace_peak": observations.memory[MemoryKind::Workspace as usize].peak})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        classic_replay::{self, Replay},
    };

    #[test]
    fn terminal_result_transfers_through_prepared_replay_without_a_duplicate_owner() {
        for q in [NAN, -1.0, 2.0] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let mut producers = Producers::new(&account);
            let mut bounds = [Bound {
                upper_bound: 1.0,
                ordinal: 0,
            }];
            let mut mutation =
                Mutation::new(q, &mut bounds, &account, || Err("no sorting")).unwrap();
            assert_eq!(account.current_samples(), 1);
            let Action::Done { length, result } = mutation
                .next(&mut bounds, &mut producers, 1, || Ok(()))
                .unwrap()
            else {
                panic!("no count request")
            };
            assert_eq!(length, 0);
            drop(mutation);
            let mut replay = Replay::from_prepared(q, &bounds[..length], result);
            assert_eq!(account.current_samples(), 1);
            let classic_replay::Action::Done(result) =
                replay.next(&bounds[..length], || Ok::<_, ()>(())).unwrap()
            else {
                panic!("no count request")
            };
            drop(replay);
            assert_eq!(account.current_samples(), 1);
            assert_eq!(account.snapshot().statistics.samples.peak_samples, 1);
            drop(result);
            assert_eq!(account.current_samples(), 0);
        }
    }

    fn evaluate(
        q: f64,
        bounds: &mut [Bound],
        producers: &mut Producers,
        raw: &[f64],
    ) -> BucketQuantileResult {
        let mut mutation = Mutation::new(q, bounds, producers.accounting(), || {
            Ok::<_, EvaluationError>(())
        })
        .unwrap();
        let (length, result) = loop {
            match mutation
                .next(bounds, producers, raw.len(), || Ok(()))
                .unwrap()
            {
                Action::Count(key) => {
                    let value = if key < raw.len() {
                        raw[key]
                    } else {
                        producers[key - raw.len()]
                    };
                    mutation.receive(
                        value,
                        Some(crate::statistics::OwnedSamples::new(
                            producers.accounting(),
                            1,
                        )),
                        bounds,
                    );
                }
                Action::Done { length, result } => break (length, result),
            }
        };
        let mut replay = Replay::from_prepared(q, &bounds[..length], result);
        loop {
            match replay
                .next(&bounds[..length], || Ok::<_, EvaluationError>(()))
                .unwrap()
            {
                classic_replay::Action::Count(key) => {
                    let value = if key < raw.len() {
                        raw[key]
                    } else {
                        producers[key - raw.len()]
                    };
                    replay.receive(
                        value,
                        Some(crate::statistics::OwnedSamples::new(
                            producers.accounting(),
                            1,
                        )),
                    );
                }
                classic_replay::Action::Done(result) => return result.result,
            }
        }
    }

    #[test]
    fn repeated_quantiles_preserve_the_original_length_and_mutated_tail() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut producers = Producers::new(&account);
        let mut bounds = [
            Bound {
                upper_bound: 1.0,
                ordinal: 0,
            },
            Bound {
                upper_bound: 1.0,
                ordinal: 1,
            },
            Bound {
                upper_bound: f64::INFINITY,
                ordinal: 2,
            },
        ];
        let raw = [1.0, 1.0, 2.0];
        assert_eq!(
            evaluate(0.25, &mut bounds, &mut producers, &raw)
                .value
                .to_bits(),
            0.25_f64.to_bits()
        );
        assert_eq!(bounds.map(|b| b.ordinal), [3, 2, 2]);
        assert_eq!(&*producers, &[2.0]);
        assert_eq!(
            evaluate(0.75, &mut bounds, &mut producers, &raw)
                .value
                .to_bits(),
            1.0_f64.to_bits()
        );
        assert_eq!(bounds.map(|b| b.ordinal), [3, 4, 2]);
        assert_eq!(&*producers, &[2.0, 4.0]);
        drop(producers);
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn singletons_are_never_cached_and_equal_zero_keeps_previous_source() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut producers = Producers::new(&account);
        let mut bounds = [
            Bound {
                upper_bound: 0.0,
                ordinal: 0,
            },
            Bound {
                upper_bound: 1.0,
                ordinal: 1,
            },
            Bound {
                upper_bound: f64::INFINITY,
                ordinal: 2,
            },
        ];
        let result = evaluate(0.5, &mut bounds, &mut producers, &[0.0, -0.0, -1.0]);
        assert!(result.forced_monotonicity);
        assert_eq!(bounds.map(|b| b.ordinal), [0, 1, 0]);
        assert_eq!(result.value.to_bits(), NAN.to_bits());
        assert!(producers.is_empty());
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].peak,
            0
        );
        drop(producers);
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn invalid_quantile_leaves_backing_and_accounting_untouched() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut producers = Producers::new(&account);
        let mut bounds = [
            Bound {
                upper_bound: f64::INFINITY,
                ordinal: 0,
            },
            Bound {
                upper_bound: 1.0,
                ordinal: 1,
            },
        ];
        let result = evaluate(-1.0, &mut bounds, &mut producers, &[2.0, 1.0]);
        assert_eq!(result.value.to_bits(), f64::NEG_INFINITY.to_bits());
        assert_eq!(bounds.map(|b| b.ordinal), [0, 1]);
        assert!(producers.is_empty());
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].peak,
            0
        );
        drop(producers);
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}
