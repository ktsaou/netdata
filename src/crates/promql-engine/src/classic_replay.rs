// SPDX-License-Identifier: GPL-3.0-or-later
// Single-call replay derives from Prometheus promql/quantile.go; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    go_sort::sort_by_with,
    histogram_classic::{
        BucketQuantileResult, ClassicBucket, FractionRanks, NAN, bound_order, coalesced_count,
        corrected_count, quantile_interpolate,
    },
    numerical::source_mul,
    statistics::OwnedSamples,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Bound {
    pub(crate) upper_bound: f64,
    pub(crate) ordinal: usize,
}

#[derive(Clone, Copy)]
pub(crate) enum Operation {
    Quantile(f64),
    Fraction(f64, f64),
}

pub(crate) enum Action {
    Count(usize),
    Done(ResultParts),
}

pub(crate) struct ResultParts {
    pub(crate) result: BucketQuantileResult,
    pub(crate) samples: OwnedSamples,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Done,
    QuantileTotal,
    QuantileProbe,
    QuantileEndpoint,
    FractionTotal,
    FractionRanks,
}

pub(crate) struct Replay {
    operation: Operation,
    phase: Phase,
    result: BucketQuantileResult,
    result_samples: OwnedSamples,
    groups: usize,
    total: f64,
    rank: f64,
    penultimate_bound: f64,
    left: usize,
    right: usize,
    target: usize,
    position: usize,
    run_start: usize,
    group: usize,
    count: Option<f64>,
    count_samples: Option<crate::statistics::OwnedSamples>,
    correction_previous: Option<f64>,
    correction_samples: Option<crate::statistics::OwnedSamples>,
    previous_bucket: Option<ClassicBucket>,
    previous_samples: Option<crate::statistics::OwnedSamples>,
    total_samples: Option<crate::statistics::OwnedSamples>,
    correction: BucketQuantileResult,
    ranks: FractionRanks,
    waiting: bool,
    prepared: bool,
}

impl Replay {
    fn initial(operation: Operation, account: &QueryAccounting) -> Self {
        Self {
            operation,
            phase: Phase::Done,
            result: BucketQuantileResult::new(NAN),
            result_samples: OwnedSamples::new(account, 0),
            groups: 0,
            total: 0.0,
            rank: 0.0,
            penultimate_bound: 0.0,
            left: 0,
            right: 0,
            target: 0,
            position: 0,
            run_start: 0,
            group: 0,
            count: None,
            count_samples: None,
            correction_previous: None,
            correction_samples: None,
            previous_bucket: None,
            previous_samples: None,
            total_samples: None,
            correction: BucketQuantileResult::new(NAN),
            ranks: match operation {
                Operation::Fraction(lower, upper) => FractionRanks::new(lower, upper),
                Operation::Quantile(_) => FractionRanks::new(0.0, 0.0),
            },
            waiting: false,
            prepared: false,
        }
    }

    pub(crate) fn new<E>(
        operation: Operation,
        bounds: &mut [Bound],
        account: &QueryAccounting,
        mut poll: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, E> {
        let mut replay = Self::initial(operation, account);
        if let Operation::Quantile(q) = operation {
            if q.is_nan() {
                replay.complete();
                return Ok(replay);
            }
            if q < 0.0 {
                replay.result.value = f64::NEG_INFINITY;
                replay.complete();
                return Ok(replay);
            }
            if q > 1.0 {
                replay.result.value = f64::INFINITY;
                replay.complete();
                return Ok(replay);
            }
        }
        sort_by_with(
            bounds,
            |a, b| bound_order(a.upper_bound, b.upper_bound),
            &mut poll,
        )?;
        // Source tests exercise empty helper panics; actual query groups are nonempty.
        if bounds[bounds.len() - 1].upper_bound != f64::INFINITY {
            replay.complete();
            return Ok(replay);
        }
        replay.groups = 1;
        for index in 1..bounds.len() {
            poll()?;
            if bounds[index].upper_bound != bounds[index - 1].upper_bound {
                replay.groups += 1;
            }
        }
        replay.phase = match operation {
            Operation::Quantile(_) => Phase::QuantileTotal,
            Operation::Fraction(_, _) => Phase::FractionTotal,
        };
        replay.reset_sweep();
        Ok(replay)
    }

    pub(crate) fn from_prepared(q: f64, bounds: &[Bound], parts: ResultParts) -> Self {
        let mut replay = Self::initial(Operation::Quantile(q), parts.samples.accounting());
        replay.prepared = true;
        replay.result = parts.result;
        replay.result_samples = parts.samples;
        replay.groups = bounds.len();
        if bounds.len() >= 2 {
            replay.penultimate_bound = bounds[bounds.len() - 2].upper_bound;
            replay.phase = Phase::QuantileTotal;
        }
        replay
    }

    fn complete(&mut self) {
        self.phase = Phase::Done;
        self.result_samples.resize(1);
    }

    fn take_result(&mut self) -> ResultParts {
        let empty = OwnedSamples::new(self.result_samples.accounting(), 0);
        ResultParts {
            result: self.result,
            samples: std::mem::replace(&mut self.result_samples, empty),
        }
    }

    fn reset_sweep(&mut self) {
        self.position = 0;
        self.run_start = 0;
        self.group = 0;
        self.count = None;
        self.count_samples = None;
        self.correction_previous = None;
        self.correction_samples = None;
        self.previous_bucket = None;
        self.previous_samples = None;
        self.correction = BucketQuantileResult::new(NAN);
        self.correction.min_bound = f64::INFINITY;
        self.correction.max_bound = f64::NEG_INFINITY;
    }

    /// Resume through the evaluator's frame loop, rather than a recursive count callback.
    pub(crate) fn next<E>(
        &mut self,
        bounds: &[Bound],
        mut poll: impl FnMut() -> Result<(), E>,
    ) -> Result<Action, E> {
        assert!(!self.waiting, "count request has not completed");
        loop {
            if self.prepared {
                return self.next_prepared(bounds, poll);
            }
            poll()?;
            if self.phase == Phase::Done {
                return Ok(Action::Done(self.take_result()));
            }
            if self.count.is_some()
                && (self.position == bounds.len()
                    || bounds[self.position].upper_bound != bounds[self.run_start].upper_bound)
            {
                let bucket = ClassicBucket {
                    upper_bound: bounds[self.run_start].upper_bound,
                    count: self.count.take().unwrap(),
                };
                self.run_start = self.position;
                let samples = self.count_samples.take();
                let last = self.position == bounds.len();
                let complete_total = last && self.phase == Phase::QuantileTotal;
                self.finish_run(bucket, samples, last);
                // A completed unrepaired sweep makes raw bucket positions directly addressable.
                if complete_total
                    && self.phase != Phase::Done
                    && self.groups == bounds.len()
                    && !self.result.fixed_precision
                    && !self.result.forced_monotonicity
                {
                    self.prepared = true;
                }
                continue;
            }
            self.waiting = true;
            return Ok(Action::Count(bounds[self.position].ordinal));
        }
    }

    pub(crate) fn receive(&mut self, count: f64, samples: Option<crate::statistics::OwnedSamples>) {
        assert!(self.waiting, "unsolicited bucket count");
        self.waiting = false;
        if self.prepared {
            self.receive_prepared(count, samples);
            return;
        }
        if self.count.is_none() {
            self.count_samples = samples;
        }
        self.count = Some(
            self.count
                .map_or(count, |previous| coalesced_count(previous, count)),
        );
        self.position += 1;
    }

    fn next_prepared<E>(
        &mut self,
        bounds: &[Bound],
        mut poll: impl FnMut() -> Result<(), E>,
    ) -> Result<Action, E> {
        loop {
            poll()?;
            self.position = match self.phase {
                Phase::Done => return Ok(Action::Done(self.take_result())),
                Phase::QuantileTotal => self.groups - 1,
                Phase::QuantileProbe => self.target,
                Phase::QuantileEndpoint => {
                    if self.target == self.groups - 1 {
                        self.result.value = self.penultimate_bound;
                        self.complete();
                        continue;
                    }
                    if self.target == 0 && bounds[0].upper_bound <= 0.0 {
                        self.result.value = bounds[0].upper_bound;
                        self.complete();
                        continue;
                    }
                    if let Some(count) = self.count {
                        if self.target == 0 || self.previous_bucket.is_some() {
                            let previous = self.previous_bucket.map(|mut bucket| {
                                bucket.upper_bound = bounds[self.target - 1].upper_bound;
                                bucket
                            });
                            self.result.value = quantile_interpolate(
                                self.rank,
                                ClassicBucket {
                                    upper_bound: bounds[self.target].upper_bound,
                                    count,
                                },
                                previous,
                            );
                            self.complete();
                            continue;
                        }
                        self.target - 1
                    } else {
                        self.target
                    }
                }
                Phase::FractionTotal | Phase::FractionRanks => unreachable!("prepared quantile"),
            };
            self.waiting = true;
            return Ok(Action::Count(bounds[self.position].ordinal));
        }
    }

    fn receive_prepared(&mut self, count: f64, samples: Option<crate::statistics::OwnedSamples>) {
        match self.phase {
            Phase::QuantileTotal => {
                self.total = count;
                self.total_samples = samples;
                if count == 0.0 {
                    self.complete();
                } else {
                    let Operation::Quantile(q) = self.operation else {
                        unreachable!("prepared quantile")
                    };
                    self.rank = source_mul(q, count);
                    self.right = self.groups - 1;
                    self.search();
                }
            }
            Phase::QuantileProbe => {
                if count >= self.rank {
                    self.right = self.target;
                } else {
                    self.left = self.target + 1;
                }
                self.search();
            }
            Phase::QuantileEndpoint => {
                if self.position == self.target {
                    self.count = Some(count);
                    self.count_samples = samples;
                } else {
                    self.previous_bucket = Some(ClassicBucket {
                        upper_bound: 0.0,
                        count,
                    });
                    self.previous_samples = samples;
                }
            }
            _ => unreachable!("prepared count request"),
        }
    }

    fn finish_run(
        &mut self,
        mut bucket: ClassicBucket,
        samples: Option<crate::statistics::OwnedSamples>,
        last: bool,
    ) {
        if matches!(self.operation, Operation::Quantile(_)) {
            bucket.count = if let Some(previous) = self.correction_previous.as_mut() {
                corrected_count(
                    bucket.count,
                    bucket.upper_bound,
                    previous,
                    &mut self.correction,
                )
            } else {
                self.correction_previous = Some(bucket.count);
                self.correction_samples =
                    samples.as_ref().map(crate::statistics::OwnedSamples::copy);
                bucket.count
            };
        }
        let index = self.group;
        self.group += 1;
        let previous = self.previous_bucket.replace(bucket);
        let _previous_samples = std::mem::replace(&mut self.previous_samples, samples);
        match self.phase {
            Phase::QuantileTotal => {
                if index + 2 == self.groups {
                    self.penultimate_bound = bucket.upper_bound;
                }
                if last {
                    self.result = self.correction;
                    self.total = bucket.count;
                    self.total_samples = self
                        .previous_samples
                        .as_ref()
                        .map(crate::statistics::OwnedSamples::copy);
                    if self.groups < 2 || self.total == 0.0 {
                        self.complete();
                    } else {
                        let Operation::Quantile(q) = self.operation else {
                            unreachable!()
                        };
                        self.rank = source_mul(q, self.total);
                        self.right = self.groups - 1;
                        self.search();
                    }
                }
            }
            Phase::QuantileProbe => {
                if index == self.target {
                    if bucket.count >= self.rank {
                        self.right = self.target;
                    } else {
                        self.left = self.target + 1;
                    }
                    self.search();
                }
            }
            Phase::QuantileEndpoint => {
                if index == self.target {
                    self.result.value = if index == self.groups - 1 {
                        self.penultimate_bound
                    } else if index == 0 && bucket.upper_bound <= 0.0 {
                        bucket.upper_bound
                    } else {
                        quantile_interpolate(self.rank, bucket, previous)
                    };
                    self.complete();
                }
            }
            Phase::FractionTotal => {
                if last {
                    self.total = bucket.count;
                    self.total_samples = self
                        .previous_samples
                        .as_ref()
                        .map(crate::statistics::OwnedSamples::copy);
                    let Operation::Fraction(lower, upper) = self.operation else {
                        unreachable!()
                    };
                    if self.total == 0.0 || lower.is_nan() || upper.is_nan() {
                        self.complete();
                    } else if lower >= upper {
                        self.result.value = 0.0;
                        self.complete();
                    } else {
                        self.phase = Phase::FractionRanks;
                        self.reset_sweep();
                    }
                }
            }
            Phase::FractionRanks => {
                if self.ranks.push(bucket) || last {
                    self.result.value = self.ranks.finish(self.total);
                    self.complete();
                }
            }
            Phase::Done => unreachable!(),
        }
    }

    fn search(&mut self) {
        if self.left < self.right {
            self.target = self.left.wrapping_add(self.right) >> 1;
            self.phase = Phase::QuantileProbe;
        } else {
            self.target = self.left;
            self.phase = Phase::QuantileEndpoint;
        }
        if self.prepared {
            // Completed prefix producers make the source predicate directly addressable.
            self.count = None;
            self.count_samples = None;
            self.previous_bucket = None;
            self.previous_samples = None;
        } else {
            self.reset_sweep();
        }
    }
}

#[cfg(feature = "fixtures")]
pub fn fixture(request: &serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    let f = |value: &serde_json::Value| {
        f64::from_bits(u64::from_str_radix(value.as_str().expect("expected bits"), 16).unwrap())
    };
    let buckets = request["buckets"].as_array();
    let mut bounds: Vec<_> = buckets
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(ordinal, bucket)| Bound {
            upper_bound: f(&bucket["upper_bound"]),
            ordinal,
        })
        .collect();
    let op = &request["operation"];
    let operation = match op["kind"].as_str().unwrap() {
        "quantile" => Operation::Quantile(f(&op["quantile"])),
        "fraction" => Operation::Fraction(f(&op["lower"]), f(&op["upper"])),
        _ => panic!("unknown replay operation"),
    };
    let engine = crate::accounting::EngineAccounting::default();
    let account = engine.query();
    let mut reads = 0usize;
    let mut polls = 0usize;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut poll = || {
            polls += 1;
            Ok::<_, std::convert::Infallible>(())
        };
        let mut replay = Replay::new(operation, &mut bounds, &account, &mut poll).unwrap();
        loop {
            match replay.next(&bounds, &mut poll).unwrap() {
                Action::Count(ordinal) => {
                    reads += 1;
                    replay.receive(
                        f(&request["buckets"][ordinal]["count"]),
                        Some(crate::statistics::OwnedSamples::new(&account, 1)),
                    );
                }
                Action::Done(result) => break result.result,
            }
        }
    }));
    let result = outcome.as_ref().ok().map(|result| {
        let value = result.value;
        let reciprocal = 1.0 / value;
        let quantile = matches!(operation, Operation::Quantile(_)).then(|| json!({
            "forced_monotonicity": result.forced_monotonicity,
            "fixed_precision": result.fixed_precision,
            "min_bound": format!("{:016x}", result.min_bound.to_bits()),
            "max_bound": format!("{:016x}", result.max_bound.to_bits()),
            "max_decrease": format!("{:016x}", result.max_decrease.to_bits()),
        }));
        json!({"value":format!("{:016x}", value.to_bits()),"reciprocal":format!("{:016x}", reciprocal.to_bits()),
            "negative_reciprocal_infinite":reciprocal == f64::NEG_INFINITY,
            "positive_reciprocal_infinite":reciprocal == f64::INFINITY,
            "less_than_positive_infinity":value < f64::INFINITY,"quantile":quantile})
    });
    json!({"panicked":outcome.is_err(),"result":result,"reads":reads,"polls":polls})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{AccountedBox, AccountedVec, AllocationContext, EngineAccounting, MemoryKind},
        storage::{Control, Interrupted},
    };

    fn result_bits(result: &BucketQuantileResult) -> (u64, bool, bool, u64, u64, u64) {
        (
            result.value.to_bits(),
            result.forced_monotonicity,
            result.fixed_precision,
            result.min_bound.to_bits(),
            result.max_bound.to_bits(),
            result.max_decrease.to_bits(),
        )
    }

    #[test]
    fn unrepaired_quantiles_keep_complete_results_with_direct_probe_read_bounds() {
        let mut measurements = Vec::new();
        for length in [3usize, 30, 128] {
            for q in [0.0, 0.05, 0.5, 0.95, 1.0] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let buckets: Vec<_> = (1..=length)
                    .rev()
                    .map(|index| ClassicBucket {
                        upper_bound: if index == length {
                            f64::INFINITY
                        } else {
                            index as f64
                        },
                        count: index as f64,
                    })
                    .collect();
                let expected = crate::histogram_classic::bucket_quantile(q, &mut buckets.clone());
                let mut bounds: Vec<_> = buckets
                    .iter()
                    .enumerate()
                    .map(|(ordinal, bucket)| Bound {
                        upper_bound: bucket.upper_bound,
                        ordinal,
                    })
                    .collect();
                let mut replay = Replay::new(Operation::Quantile(q), &mut bounds, &account, || {
                    Ok::<_, ()>(())
                })
                .unwrap();
                let mut reads = 0;
                let result = loop {
                    match replay.next(&bounds, || Ok::<_, ()>(())).unwrap() {
                        Action::Count(ordinal) => {
                            reads += 1;
                            replay.receive(
                                buckets[ordinal].count,
                                Some(OwnedSamples::new(&account, 1)),
                            );
                        }
                        Action::Done(result) => break result,
                    }
                };
                assert_eq!(result_bits(&result.result), result_bits(&expected));
                assert_eq!(result.samples.units(), 1);
                drop((result, replay));
                assert_eq!(account.current_samples(), 0);
                account.snapshot().assert_memory_released();
                measurements.push((length, q, reads));
            }
        }
        println!("classic quantile Count requests: {measurements:?}");
        for &(length, q, reads) in &measurements {
            assert!(
                reads <= length + length.next_power_of_two().ilog2() as usize + 2,
                "repeated prefix reads: length={length}, q={q}, reads={reads}"
            );
        }
        assert_eq!(measurements[8], (30, 0.95, 37));
    }

    #[test]
    fn repaired_and_coalesced_quantiles_retain_generic_prefix_replay() {
        for (upper, counts) in [
            (vec![1.0, 2.0, f64::INFINITY], vec![2.0, 1.0, 4.0]),
            (vec![1.0, 2.0, f64::INFINITY], vec![2.0, 2.0 - 1e-13, 4.0]),
            (vec![1.0, 1.0, 2.0, f64::INFINITY], vec![1.0, 1.0, 3.0, 4.0]),
            (vec![1.0, 2.0, f64::INFINITY], vec![1.0, 3.0, 2.0]),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let mut original: Vec<_> = upper
                .iter()
                .zip(&counts)
                .map(|(&upper_bound, &count)| ClassicBucket { upper_bound, count })
                .collect();
            let expected = crate::histogram_classic::bucket_quantile(0.75, &mut original);
            let mut bounds: Vec<_> = upper
                .iter()
                .enumerate()
                .map(|(ordinal, &upper_bound)| Bound {
                    upper_bound,
                    ordinal,
                })
                .collect();
            let mut replay = Replay::new(Operation::Quantile(0.75), &mut bounds, &account, || {
                Ok::<_, ()>(())
            })
            .unwrap();
            let result = loop {
                match replay.next(&bounds, || Ok::<_, ()>(())).unwrap() {
                    Action::Count(ordinal) => {
                        replay.receive(counts[ordinal], Some(OwnedSamples::new(&account, 1)))
                    }
                    Action::Done(result) => break result,
                }
            };
            assert_eq!(result_bits(&result.result), result_bits(&expected));
            assert!(!replay.prepared);
            drop((replay, result));
            assert_eq!(account.current_samples(), 0);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn promotion_boundary_polls_after_releasing_completed_correction_scratch() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut bounds: Vec<_> = (0..30)
            .map(|ordinal| Bound {
                upper_bound: if ordinal == 29 {
                    f64::INFINITY
                } else {
                    ordinal as f64 + 1.0
                },
                ordinal,
            })
            .collect();
        let mut replay = Replay::new(Operation::Quantile(0.95), &mut bounds, &account, || {
            Ok::<_, ()>(())
        })
        .unwrap();
        for ordinal in 0..30 {
            assert!(matches!(
                replay.next(&bounds, || Ok::<_, ()>(())).unwrap(),
                Action::Count(value) if value == ordinal
            ));
            replay.receive(ordinal as f64 + 1.0, Some(OwnedSamples::new(&account, 1)));
        }
        let mut polls = 0;
        assert!(matches!(
            replay.next(&bounds, || {
                polls += 1;
                if polls == 2 {
                    Err(Interrupted::Cancelled)
                } else {
                    Ok(())
                }
            }),
            Err(Interrupted::Cancelled)
        ));
        assert_eq!(polls, 2);
        assert_eq!(account.current_samples(), 1);
        assert!(replay.correction_samples.is_none());
        assert!(replay.previous_samples.is_none());
        assert!(replay.count_samples.is_none());
        assert!(replay.prepared);
        drop(replay);
        assert_eq!(account.current_samples(), 0);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn invalid_quantiles_leave_metadata_untouched_and_never_request_counts() {
        for (q, expected) in [(NAN, NAN), (-1.0, f64::NEG_INFINITY), (2.0, f64::INFINITY)] {
            let engine = EngineAccounting::default();
            let query = engine.query();
            let mut bounds = [
                Bound {
                    upper_bound: 2.0,
                    ordinal: 1,
                },
                Bound {
                    upper_bound: 1.0,
                    ordinal: 0,
                },
            ];
            let mut replay = Replay::new(Operation::Quantile(q), &mut bounds, &query, || {
                Err("must not sort")
            })
            .unwrap();
            let Action::Done(result) = replay.next(&bounds, || Ok::<_, ()>(())).unwrap() else {
                panic!("unexpected read")
            };
            assert_eq!(result.result.value.to_bits(), expected.to_bits());
            assert_eq!(result.samples.units(), 1);
            assert_eq!(query.current_samples(), 1);
            assert_eq!(query.snapshot().statistics.samples.peak_samples, 1);
            assert_eq!(result.result.min_bound.to_bits(), 0);
            drop(result);
            assert_eq!(query.current_samples(), 0);
            assert_eq!(bounds.map(|b| b.ordinal), [1, 0]);
        }
    }

    #[test]
    fn completed_results_overlap_endpoints_and_transfer_without_an_extra_copy() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let bounds = [
            Bound {
                upper_bound: 1.0,
                ordinal: 0,
            },
            Bound {
                upper_bound: f64::INFINITY,
                ordinal: 1,
            },
        ];
        let mut replay = Replay::from_prepared(
            0.5,
            &bounds,
            ResultParts {
                result: BucketQuantileResult::new(NAN),
                samples: OwnedSamples::new(&account, 0),
            },
        );
        let result = loop {
            match replay.next(&bounds, || Ok::<_, ()>(())).unwrap() {
                Action::Count(ordinal) => {
                    replay.receive([2.0, 4.0][ordinal], Some(OwnedSamples::new(&account, 1)))
                }
                Action::Done(result) => break result,
            }
        };
        assert_eq!(result.result.value, 1.0);
        assert_eq!(account.current_samples(), 3);
        assert_eq!(account.snapshot().statistics.samples.peak_samples, 3);
        drop(replay);
        assert_eq!(account.current_samples(), 1);
        drop(result);
        assert_eq!(account.current_samples(), 0);
    }

    #[test]
    fn cancellation_releases_a_result_completed_by_the_last_received_count() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let bounds = [
            Bound {
                upper_bound: 1.0,
                ordinal: 0,
            },
            Bound {
                upper_bound: f64::INFINITY,
                ordinal: 1,
            },
        ];
        let mut replay = Replay::from_prepared(
            0.5,
            &bounds,
            ResultParts {
                result: BucketQuantileResult::new(NAN),
                samples: OwnedSamples::new(&account, 0),
            },
        );
        assert!(matches!(
            replay.next(&bounds, || Ok::<_, ()>(())).unwrap(),
            Action::Count(1)
        ));
        replay.receive(0.0, Some(OwnedSamples::new(&account, 1)));
        assert_eq!(account.current_samples(), 2);
        assert_eq!(account.snapshot().statistics.samples.peak_samples, 2);
        let control = Control::default();
        control.cancellation.cancel();
        assert!(matches!(
            replay.next(&bounds, || control.check()),
            Err(Interrupted::Cancelled)
        ));
        drop(replay);
        assert_eq!(account.current_samples(), 0);
    }

    #[test]
    fn cancellation_interrupts_all_count_sweeps_without_new_buffers() {
        for phase in [
            Phase::QuantileTotal,
            Phase::QuantileProbe,
            Phase::QuantileEndpoint,
            Phase::FractionTotal,
            Phase::FractionRanks,
        ] {
            let engine = EngineAccounting::default();
            let query = engine.query();
            let workspace = AllocationContext::new(&query, MemoryKind::Workspace);
            let mut bounds = AccountedVec::new_in(Some(&workspace));
            for ordinal in 1..=80 {
                bounds.push(Bound {
                    upper_bound: ordinal as f64,
                    ordinal,
                });
            }
            bounds.push(Bound {
                upper_bound: f64::INFINITY,
                ordinal: 80,
            });
            let operation = if matches!(phase, Phase::FractionTotal | Phase::FractionRanks) {
                Operation::Fraction(25.0, 75.0)
            } else {
                Operation::Quantile(0.55)
            };
            let mut replay = AccountedBox::new(
                Replay::new(operation, &mut bounds, &query, || Ok::<_, ()>(())).unwrap(),
                &workspace,
            );
            let retained = query.snapshot().memory;
            loop {
                if replay.phase == phase {
                    break;
                }
                match replay.next(&bounds, || Ok::<_, ()>(())).unwrap() {
                    Action::Count(ordinal) => replay.receive(
                        ordinal as f64,
                        Some(crate::statistics::OwnedSamples::new(&query, 1)),
                    ),
                    Action::Done(_) => panic!("phase was never reached: {phase:?}"),
                }
            }
            let control = Control::default();
            control.cancellation.cancel();
            assert!(matches!(
                replay.next(&bounds, || control.check()),
                Err(Interrupted::Cancelled)
            ));
            assert_eq!(query.snapshot().memory, retained);
            drop(replay);
            drop(bounds);
            assert_eq!(
                query.snapshot().memory[MemoryKind::Workspace as usize].current,
                0
            );
        }
    }

    #[test]
    fn large_metadata_replay_uses_bounded_reads_and_a_small_stack_without_count_arrays() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let engine = EngineAccounting::default();
                let query = engine.query();
                let workspace = AllocationContext::new(&query, MemoryKind::Workspace);
                let mut bounds = AccountedVec::new_in(Some(&workspace));
                for ordinal in (1..=100_000).rev() {
                    bounds.push(Bound {
                        upper_bound: ordinal as f64,
                        ordinal,
                    });
                }
                bounds.push(Bound {
                    upper_bound: f64::INFINITY,
                    ordinal: 100_001,
                });
                let control = Control::default();
                let poll = || {
                    control.check()?;
                    query.work(1);
                    Ok::<_, Interrupted>(())
                };
                for operation in [
                    Operation::Quantile(0.5),
                    Operation::Fraction(25_000.0, 75_000.0),
                ] {
                    let mut replay = AccountedBox::new(
                        Replay::new(operation, &mut bounds, &query, poll).unwrap(),
                        &workspace,
                    );
                    let retained = query.snapshot().memory;
                    let mut reads = 0usize;
                    let result = loop {
                        match replay.next(&bounds, poll).unwrap() {
                            Action::Count(ordinal) => {
                                reads += 1;
                                replay.receive(
                                    ordinal.min(100_000) as f64,
                                    Some(crate::statistics::OwnedSamples::new(&query, 1)),
                                );
                            }
                            Action::Done(result) => break result.result,
                        }
                    };
                    let (expected, bound) = match operation {
                        Operation::Quantile(_) => (
                            50_000.0,
                            bounds.len() * (2 + bounds.len().ilog2() as usize + 1),
                        ),
                        Operation::Fraction(_, _) => (0.5, 2 * bounds.len()),
                    };
                    assert_eq!(result.value, expected);
                    assert!(reads <= bound, "{reads} > {bound}");
                    assert_eq!(query.snapshot().memory, retained);
                    drop(replay);
                }
                drop(bounds);
                assert_eq!(
                    query.snapshot().memory[MemoryKind::Workspace as usize].current,
                    0
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
