// SPDX-License-Identifier: GPL-3.0-or-later
// Repeated backing mutations derive from pinned promql/quantile.go; see NOTICE.
use crate::{
    accounting::AccountedVec,
    classic_replay::Bound,
    evaluator::EvaluationError,
    go_sort::sort_by_with,
    histogram_classic::{
        BucketQuantileResult, CountCorrection, NAN, bound_order, coalesced_count, count_correction,
    },
};

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
    Done {
        length: usize,
        result: BucketQuantileResult,
    },
}

pub(crate) struct Mutation {
    phase: Phase,
    result: BucketQuantileResult,
    run: Option<Bound>,
    position: usize,
    length: usize,
    sum: Option<f64>,
    previous: Option<(usize, f64)>,
    waiting: bool,
}

impl Mutation {
    pub(crate) fn new<E>(
        q: f64,
        bounds: &mut [Bound],
        mut poll: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, E> {
        let mut state = Self {
            phase: Phase::Done,
            result: BucketQuantileResult::new(NAN),
            run: None,
            position: 0,
            length: 0,
            sum: None,
            previous: None,
            waiting: false,
        };
        if q.is_nan() {
            return Ok(state);
        }
        if q < 0.0 {
            state.result.value = f64::NEG_INFINITY;
            return Ok(state);
        }
        if q > 1.0 {
            state.result.value = f64::INFINITY;
            return Ok(state);
        }
        sort_by_with(
            bounds,
            |a, b| bound_order(a.upper_bound, b.upper_bound),
            &mut poll,
        )?;
        if bounds[bounds.len() - 1].upper_bound != f64::INFINITY {
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
        producers: &mut AccountedVec<f64>,
        row_base: usize,
        mut poll: impl FnMut() -> Result<(), EvaluationError>,
    ) -> Result<Action, EvaluationError> {
        assert!(!self.waiting, "mutation count request has not completed");
        loop {
            poll()?;
            match self.phase {
                Phase::Done => {
                    return Ok(Action::Done {
                        length: self.length,
                        result: self.result,
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
                        producers.push(sum);
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

    pub(crate) fn receive(&mut self, value: f64, bounds: &mut [Bound]) {
        assert!(self.waiting, "unsolicited mutation count");
        self.waiting = false;
        match self.phase {
            Phase::First => {
                self.sum = Some(value);
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
    let mut producers = AccountedVec::new(&account, MemoryKind::Workspace);
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
            let mut mutation = Mutation::new(q, &mut bounds, &mut poll).unwrap();
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
                        mutation.receive(value, &mut bounds);
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
                        replay.receive(value);
                    }
                    classic_replay::Action::Done(result) => break result,
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

    fn evaluate(
        q: f64,
        bounds: &mut [Bound],
        producers: &mut AccountedVec<f64>,
        raw: &[f64],
    ) -> BucketQuantileResult {
        let mut mutation = Mutation::new(q, bounds, || Ok::<_, EvaluationError>(())).unwrap();
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
                    mutation.receive(value, bounds);
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
                    replay.receive(value);
                }
                classic_replay::Action::Done(result) => return result,
            }
        }
    }

    #[test]
    fn repeated_quantiles_preserve_the_original_length_and_mutated_tail() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut producers = AccountedVec::new(&account, MemoryKind::Workspace);
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
        let mut producers = AccountedVec::new(&account, MemoryKind::Workspace);
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
        let mut producers = AccountedVec::new(&account, MemoryKind::Workspace);
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
