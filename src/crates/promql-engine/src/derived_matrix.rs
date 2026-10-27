// SPDX-License-Identifier: GPL-3.0-or-later
// Logical matrix acquisition derives from pinned storage/buffer.go and promql/engine.go; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    evaluator::{EvaluatedSample, EvaluationError},
    histogram::{COUNTER_RESET, NOT_RESET, UNKNOWN_RESET},
    query_scopes::Grid,
    result::OwnedValue,
    storage::{Control, StorageError},
    value::SampleRef,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Window {
    pub(crate) first_end_ms: i64,
    pub(crate) end_ms: i64,
    pub(crate) range_ms: i64,
    pub(crate) step_ms: i64,
}

pub(crate) const CHECKPOINT_WORDS: usize = 17;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Checkpoint(pub(crate) [u64; CHECKPOINT_WORDS]);
impl Checkpoint {
    pub(crate) fn sample_units(&self) -> u64 {
        u64::from(self.0[0] & 3 == 3)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Fetch {
    pub(crate) time_ms: i64,
    pub(crate) kind: u8,
    pub(crate) occurrence: u64,
}
impl Fetch {
    pub(crate) fn absent_histogram(self, kinds: u8) -> bool {
        // Histogram probes follow the completed Float traversal at this timestamp.
        self.kind == 2 && kinds == 1
    }
}

pub(crate) enum Action {
    Fetch(Fetch),
    Done,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Identity {
    time_ms: i64,
    occurrence: u64,
}
struct Current {
    identity: Identity,
    kind: u8,
    value: OwnedValue,
    _float_samples: Option<crate::statistics::OwnedSamples>,
}

#[derive(Clone, Copy)]
enum Phase {
    Initial,
    Begin,
    Seek,
    Advance,
    Endpoint,
    Finish,
    Done,
}
#[derive(Clone, Copy)]
enum Landing {
    Initial,
    Seek,
    Next,
}

pub(crate) struct Reader {
    grid: Grid,
    window: Window,
    end_ms: i64,
    delta_ms: i64,
    last_time_ms: i64,
    materialized: [Option<i64>; 2],
    cutoffs: [i64; 2],
    emitted: [Option<Identity>; 2],
    phase: Phase,
    cursor: Fetch,
    exhausted: bool,
    landing: Landing,
    pending: bool,
    seen: u64,
    current: Option<Current>,
    after_child: Option<Action>,
}
impl Reader {
    pub(crate) fn checkpoint(&self) -> Option<Checkpoint> {
        if !matches!(self.phase, Phase::Done)
            || self.pending
            || self.materialized[1].is_some()
            || self.after_child.is_some()
        {
            return None;
        }
        let mut words = [0; CHECKPOINT_WORDS];
        words[0] = 1
            | (u64::from(self.exhausted) << 7)
            | (u64::from(self.cursor.kind == 2) << 8)
            | (match self.landing {
                Landing::Initial => 0,
                Landing::Seek => 1,
                Landing::Next => 2,
            } << 9);
        words[1] = self.end_ms as u64;
        words[2] = self.delta_ms as u64;
        words[3] = self.last_time_ms as u64;
        for lane in 0..2 {
            if let Some(time) = self.materialized[lane] {
                words[0] |= 1 << (3 + lane);
                words[4 + lane] = time as u64;
            }
            words[6 + lane] = self.cutoffs[lane] as u64;
            if let Some(identity) = self.emitted[lane] {
                words[0] |= 1 << (5 + lane);
                words[8 + lane * 2] = identity.time_ms as u64;
                words[9 + lane * 2] = identity.occurrence;
            }
        }
        words[12] = self.cursor.time_ms as u64;
        words[13] = self.cursor.occurrence;
        if let Some(current) = &self.current {
            let OwnedValue::Float(value) = &current.value else {
                return None;
            };
            if current.kind != 1
                || current
                    ._float_samples
                    .as_ref()
                    .is_some_and(|samples| samples.units() != 1)
            {
                return None;
            }
            words[0] |= 2 | (u64::from(current._float_samples.is_some()) << 2);
            words[14] = current.identity.time_ms as u64;
            words[15] = current.identity.occurrence;
            words[16] = value.to_bits();
        }
        Some(Checkpoint(words))
    }

    pub(crate) fn resume(
        grid: Grid,
        window: Window,
        checkpoint: Checkpoint,
        account: &QueryAccounting,
    ) -> Option<Self> {
        let words = checkpoint.0;
        let flags = words[0];
        let previous = words[1] as i64;
        if flags & 1 == 0
            || flags & !0x7ff != 0
            || (flags >> 9) == 3
            || window.step_ms <= 0
            || window.range_ms <= 0
            || !grid.forward_without_wrap()
            || previous < window.first_end_ms
            || previous > window.end_ms
            || (i128::from(window.end_ms) - i128::from(previous)) % i128::from(window.step_ms) != 0
        {
            return None;
        }
        Some(Self {
            grid,
            window,
            end_ms: previous,
            delta_ms: words[2] as i64,
            last_time_ms: words[3] as i64,
            materialized: std::array::from_fn(|lane| {
                (flags & (1 << (3 + lane)) != 0).then_some(words[4 + lane] as i64)
            }),
            cutoffs: [words[6] as i64, words[7] as i64],
            emitted: std::array::from_fn(|lane| {
                (flags & (1 << (5 + lane)) != 0).then_some(Identity {
                    time_ms: words[8 + lane * 2] as i64,
                    occurrence: words[9 + lane * 2],
                })
            }),
            phase: if previous == window.end_ms {
                Phase::Done
            } else {
                Phase::Finish
            },
            cursor: Fetch {
                time_ms: words[12] as i64,
                occurrence: words[13],
                kind: if flags & 256 == 0 { 1 } else { 2 },
            },
            exhausted: flags & 128 != 0,
            landing: match flags >> 9 {
                0 => Landing::Initial,
                1 => Landing::Seek,
                _ => Landing::Next,
            },
            pending: false,
            seen: 0,
            current: (flags & 2 != 0).then(|| Current {
                identity: Identity {
                    time_ms: words[14] as i64,
                    occurrence: words[15],
                },
                kind: 1,
                value: OwnedValue::Float(f64::from_bits(words[16])),
                _float_samples: (flags & 4 != 0)
                    .then(|| crate::statistics::OwnedSamples::new(account, 1)),
            }),
            after_child: None,
        })
    }

    pub(crate) fn new(grid: Grid, window: Window) -> Self {
        Self {
            grid,
            window,
            end_ms: window.first_end_ms,
            delta_ms: window.range_ms,
            last_time_ms: i64::MIN,
            materialized: [None; 2],
            cutoffs: [0; 2],
            emitted: [None; 2],
            phase: Phase::Initial,
            cursor: Fetch {
                time_ms: grid.start_ms,
                kind: 1,
                occurrence: 0,
            },
            exhausted: grid.end_ms < grid.start_ms,
            landing: Landing::Initial,
            pending: false,
            seen: 0,
            current: None,
            after_child: None,
        }
    }

    pub(crate) fn receive(&mut self, sample: EvaluatedSample<'_>) -> bool {
        self.receive_value(sample.into_parts())
    }

    pub(crate) fn receive_continued(
        &mut self,
        sample: EvaluatedSample<'_>,
        single_float: bool,
        control: &Control,
        account: &QueryAccounting,
        visit: impl FnMut(SampleRef<'_>) -> Result<(), EvaluationError>,
    ) -> Result<bool, EvaluationError> {
        let fetch = self.cursor;
        let window = self.window;
        let complete = single_float
            && self.pending
            && fetch.kind == 1
            && fetch.occurrence == 0
            && self.end_ms == window.first_end_ms
            && window.range_ms > 0
            && window.first_end_ms <= window.end_ms
            && (window.first_end_ms == window.end_ms || window.step_ms > 0)
            && self.grid.forward_without_wrap()
            && self.grid.start_ms <= self.grid.end_ms
            && window.end_ms.checked_sub(window.range_ms).is_some()
            && window
                .first_end_ms
                .checked_sub(window.range_ms)
                .is_some_and(|lower| lower <= fetch.time_ms && fetch.time_ms < window.first_end_ms);
        let accepted = self.receive(sample);
        if !accepted || !complete {
            return Ok(accepted);
        }
        // Visit before completing the child, preserving visitor-versus-tail failure priority.
        match self.advance(control, account, visit)? {
            Action::Fetch(next)
                if next.time_ms == fetch.time_ms && next.kind == 1 && next.occurrence == 1 =>
            {
                // Actual child Done acknowledges this proven-empty pending occurrence.
                Ok(false)
            }
            _ => Err(EvaluationError::Invalid(
                "derived Float completion did not reach its next occurrence",
            )),
        }
    }

    pub(crate) fn complete_received_child(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
        visit: impl FnMut(SampleRef<'_>) -> Result<(), EvaluationError>,
    ) -> Result<(), EvaluationError> {
        debug_assert!(self.after_child.is_none() && self.pending);
        let fetch = self.cursor;
        let next = self.advance(control, account, visit)?;
        if !matches!(&next, Action::Fetch(next)
            if next.time_ms == fetch.time_ms && next.kind == fetch.kind
                && next.occurrence == fetch.occurrence + 1)
        {
            // The next request belongs to a different child; terminal completion cannot acknowledge it.
            self.after_child = Some(next);
        }
        Ok(())
    }

    pub(crate) fn receive_value(&mut self, parts: crate::statistics::ValueParts) -> bool {
        let crate::statistics::ValueParts {
            mut value,
            float_samples,
        } = parts;
        assert!(self.pending, "requested derived child");
        if value.kind_bit() != self.cursor.kind {
            return false;
        }
        let position = self.seen;
        self.seen += 1;
        if position != self.cursor.occurrence {
            return false;
        }
        if let OwnedValue::Histogram(value) = &mut value
            && matches!(value.as_ref().reset_hint, COUNTER_RESET | NOT_RESET)
        {
            value.set_reset_hint(UNKNOWN_RESET);
        }
        self.current = Some(Current {
            identity: Identity {
                time_ms: self.cursor.time_ms,
                occurrence: self.cursor.occurrence,
            },
            kind: self.cursor.kind,
            value,
            _float_samples: float_samples,
        });
        true
    }

    fn next_lane(&mut self) -> Result<(), EvaluationError> {
        self.cursor.occurrence = 0;
        if self.cursor.kind == 1 {
            self.cursor.kind = 2;
        } else {
            if self.cursor.time_ms == self.grid.end_ms {
                self.exhausted = true;
                return Ok(());
            }
            let next = self
                .cursor
                .time_ms
                .checked_add(self.grid.interval_ms)
                .filter(|next| *next > self.cursor.time_ms)
                .ok_or(StorageError::Invalid(
                    "subquery evaluator schedule is not strictly increasing",
                ))?;
            self.exhausted = next > self.grid.end_ms;
            self.cursor.time_ms = next;
            self.cursor.kind = 1;
        }
        Ok(())
    }

    fn seek(&mut self, time_ms: i64) -> Result<(), EvaluationError> {
        self.current = None;
        if time_ms <= self.cursor.time_ms {
            return Ok(());
        }
        if time_ms > self.grid.end_ms {
            self.exhausted = true;
            return Ok(());
        }
        if self.grid.interval_ms <= 0 {
            return Err(StorageError::Invalid(
                "subquery evaluator schedule is not strictly increasing",
            )
            .into());
        }
        let distance = i128::from(time_ms) - i128::from(self.grid.start_ms);
        let interval = i128::from(self.grid.interval_ms);
        let hops = (distance + interval - 1) / interval;
        let next = i128::from(self.grid.start_ms) + hops * interval;
        if next > i128::from(self.grid.end_ms) {
            self.exhausted = true;
        } else {
            self.cursor = Fetch {
                time_ms: i64::try_from(next).map_err(|_| {
                    StorageError::Invalid("derived seek exceeds the timestamp domain")
                })?,
                kind: 1,
                occurrence: 0,
            };
        }
        Ok(())
    }

    fn access(
        &mut self,
        endpoint: bool,
        visit: &mut impl FnMut(SampleRef<'_>) -> Result<(), EvaluationError>,
    ) -> Result<(), EvaluationError> {
        let current = self.current.as_ref().expect("derived current");
        let lane = usize::from(current.kind == 2);
        let time_ms = current.identity.time_ms;
        if current.value.as_ref().is_stale() || !endpoint && time_ms <= self.cutoffs[lane] {
            return Ok(());
        }
        self.materialized[lane] = Some(time_ms);
        let target_lower = self.window.end_ms.wrapping_sub(self.window.range_ms);
        if time_ms > target_lower
            && time_ms <= self.window.end_ms
            && self.emitted[lane].is_none_or(|previous| current.identity > previous)
        {
            visit(SampleRef {
                time_ms,
                start_time_ms: 0,
                value: current.value.as_ref(),
            })?;
            self.emitted[lane] = Some(current.identity);
        }
        Ok(())
    }

    pub(crate) fn advance(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
        mut visit: impl FnMut(SampleRef<'_>) -> Result<(), EvaluationError>,
    ) -> Result<Action, EvaluationError> {
        if let Some(action) = self.after_child.take() {
            control.check()?;
            account.work(1);
            return Ok(action);
        }
        loop {
            control.check()?;
            account.work(1);
            if self.pending {
                self.pending = false;
                if let Some(current) = &self.current {
                    self.cursor.occurrence += 1;
                    match self.landing {
                        Landing::Initial => self.phase = Phase::Begin,
                        Landing::Seek | Landing::Next => {
                            self.last_time_ms = current.identity.time_ms;
                            self.phase = Phase::Advance;
                        }
                    }
                } else {
                    self.next_lane()?;
                }
            }
            let fetch = match self.phase {
                Phase::Initial => {
                    if self.exhausted {
                        self.phase = Phase::Begin;
                        continue;
                    }
                    self.landing = Landing::Initial;
                    true
                }
                Phase::Begin => {
                    let lower = self.end_ms.wrapping_sub(self.window.range_ms);
                    for (cutoff, retained) in self.cutoffs.iter_mut().zip(self.materialized) {
                        *cutoff = retained.filter(|time| *time > lower).unwrap_or(lower);
                    }
                    self.phase = if self.window.range_ms == 0 {
                        Phase::Finish
                    } else {
                        Phase::Seek
                    };
                    continue;
                }
                Phase::Seek => {
                    let lower = self.end_ms.wrapping_sub(self.delta_ms);
                    if let Some(current) = &self.current
                        && lower > self.last_time_ms
                    {
                        if current.identity.time_ms >= lower {
                            self.last_time_ms = current.identity.time_ms;
                        } else {
                            self.seek(lower)?;
                            self.landing = Landing::Seek;
                            if !self.exhausted {
                                self.pending = true;
                                self.seen = 0;
                                return Ok(Action::Fetch(self.cursor));
                            }
                        }
                    }
                    self.phase = Phase::Advance;
                    continue;
                }
                Phase::Advance => {
                    if self.current.is_some() && self.last_time_ms < self.end_ms {
                        self.access(false, &mut visit)?;
                        self.current = None;
                        self.landing = Landing::Next;
                        true
                    } else if self.current.is_none() && !self.exhausted {
                        true
                    } else {
                        self.phase = Phase::Endpoint;
                        continue;
                    }
                }
                Phase::Endpoint => {
                    if self
                        .current
                        .as_ref()
                        .is_some_and(|current| current.identity.time_ms == self.end_ms)
                    {
                        self.access(true, &mut visit)?;
                    }
                    self.phase = Phase::Finish;
                    continue;
                }
                Phase::Finish => {
                    let lower = self.end_ms.wrapping_sub(self.window.range_ms);
                    if self.materialized.iter().flatten().any(|time| *time > lower) {
                        self.delta_ms = self
                            .delta_ms
                            .min(self.window.range_ms.min(self.window.step_ms));
                    }
                    if self.end_ms == self.window.end_ms {
                        self.phase = Phase::Done;
                        continue;
                    }
                    self.end_ms = self
                        .end_ms
                        .checked_add(self.window.step_ms)
                        .filter(|next| *next > self.end_ms && *next <= self.window.end_ms)
                        .ok_or(StorageError::Invalid(
                            "derived matrix schedule is not strictly increasing",
                        ))?;
                    self.phase = Phase::Begin;
                    continue;
                }
                Phase::Done => return Ok(Action::Done),
            };
            if fetch {
                if self.exhausted {
                    self.phase = Phase::Endpoint;
                    continue;
                }
                self.pending = true;
                self.seen = 0;
                return Ok(Action::Fetch(self.cursor));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        result::OwnedLabels,
        statistics::OwnedSamples,
        storage::Interrupted,
    };

    #[test]
    fn completed_float_reader_resumes_sparse_and_stale_lookahead() {
        fn drive(
            reader: &mut Reader,
            pattern: usize,
            fail: Option<i64>,
            account: &QueryAccounting,
        ) -> Result<Vec<i64>, EvaluationError> {
            let control = Control::default();
            let mut fetched = Vec::new();
            while let Action::Fetch(fetch) = reader.advance(&control, account, |_| Ok(()))? {
                if fetch.absent_histogram(1) || fetch.occurrence != 0 {
                    continue;
                }
                fetched.push(fetch.time_ms);
                if fail == Some(fetch.time_ms) {
                    return Err(StorageError::Invalid("future point failed").into());
                }
                let present = match pattern {
                    0 => true,
                    1 | 2 => matches!(fetch.time_ms, 1 | 9),
                    3 => fetch.time_ms == 1,
                    _ => false,
                };
                if present {
                    let mut value = sample(account, fetch.time_ms);
                    if pattern == 2 && fetch.time_ms == 9 {
                        value.value =
                            OwnedValue::Float(f64::from_bits(crate::value::STALE_NAN_BITS));
                    }
                    reader.receive_continued(value, true, &control, account, |_| Ok(()))?;
                }
            }
            Ok(fetched)
        }
        fn without_emission(mut checkpoint: Checkpoint) -> Checkpoint {
            // A dry visitor has no numerical effect; only acquisition/retention state must agree.
            checkpoint.0[0] &= !((1 << 5) | (1 << 6));
            checkpoint.0[8..12].fill(0);
            checkpoint
        }
        for pattern in 0..5 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let grid = Grid {
                start_ms: 1,
                end_ms: 12,
                interval_ms: 1,
            };
            let mut window = Window {
                first_end_ms: 4,
                end_ms: 4,
                range_ms: 3,
                step_ms: 1,
            };
            let mut reader = Reader::new(grid, window);
            let first = drive(&mut reader, pattern, None, &account).unwrap();
            if pattern == 1 || pattern == 2 {
                assert_eq!(first.last(), Some(&9));
            }
            let mut checkpoint = reader.checkpoint().unwrap();
            if pattern == 2 {
                assert_eq!(checkpoint.0[16], crate::value::STALE_NAN_BITS);
                assert_eq!(checkpoint.sample_units(), 1);
            }
            drop(reader);
            for target in [4, 5, 8, 12] {
                window.end_ms = target;
                let mut resumed = Reader::resume(grid, window, checkpoint, &account).unwrap();
                let incremental = drive(&mut resumed, pattern, None, &account).unwrap();
                let mut original = Reader::new(grid, window);
                let prefix = drive(&mut original, pattern, None, &account).unwrap();
                checkpoint = resumed.checkpoint().unwrap();
                assert_eq!(
                    without_emission(checkpoint),
                    without_emission(original.checkpoint().unwrap()),
                    "pattern={pattern} target={target}"
                );
                assert!(incremental.len() <= prefix.len());
                if target == 4 || target < 9 && matches!(pattern, 1 | 2) {
                    assert!(incremental.is_empty(), "saved lookahead is retained");
                }
                drop((resumed, original));
            }
            assert!(
                Reader::resume(
                    grid,
                    Window {
                        end_ms: 8,
                        ..window
                    },
                    checkpoint,
                    &account
                )
                .is_none()
            );
            let mut failed = Reader::new(
                grid,
                Window {
                    end_ms: 4,
                    ..window
                },
            );
            assert!(matches!(
                drive(&mut failed, 1, Some(8), &account),
                Err(EvaluationError::Storage(StorageError::Invalid(
                    "future point failed"
                )))
            ));
            assert!(
                failed.checkpoint().is_none(),
                "failed lookahead cannot publish progress"
            );
            drop(failed);
            account.snapshot().assert_memory_released();
        }
    }

    fn sample(account: &QueryAccounting, time_ms: i64) -> EvaluatedSample<'static> {
        EvaluatedSample {
            ordinal: 0,
            labels: OwnedLabels::new(account, MemoryKind::Workspace).into(),
            value: OwnedValue::Float(time_ms as f64 / 1000.0),
            float_samples: Some(OwnedSamples::new(account, 1)),
            source_time_ms: time_ms,
            source_start_time_ms: 0,
            drop_name: false,
        }
    }

    #[test]
    fn drained_child_preserves_endpoint_visits_and_next_fetches() {
        for range_ms in [500, 2000] {
            let mut observations = Vec::new();
            for drained in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let mut reader = Reader::new(
                    Grid {
                        start_ms: 1000,
                        end_ms: 3000,
                        interval_ms: 1000,
                    },
                    Window {
                        first_end_ms: 1000,
                        end_ms: 3000,
                        range_ms,
                        step_ms: 1000,
                    },
                );
                let mut visits = Vec::new();
                let mut requests = Vec::new();
                let mut terminals = 0;
                loop {
                    let mut visit = |value: SampleRef<'_>| {
                        visits.push(value.time_ms);
                        Ok(())
                    };
                    let Action::Fetch(fetch) =
                        reader.advance(&control, &account, &mut visit).unwrap()
                    else {
                        break;
                    };
                    if fetch.kind != 1 || fetch.occurrence != 0 {
                        continue;
                    }
                    requests.push(fetch.time_ms);
                    assert!(reader.receive(sample(&account, fetch.time_ms)));
                    if drained {
                        reader
                            .complete_received_child(&control, &account, &mut visit)
                            .unwrap();
                        assert!(
                            reader.checkpoint().is_none(),
                            "actual terminal step is still pending"
                        );
                        terminals += 1;
                    }
                }
                assert!(reader.checkpoint().is_some());
                assert_eq!(requests, [1000, 2000, 3000]);
                assert_eq!(
                    visits,
                    if range_ms == 500 {
                        vec![3000]
                    } else {
                        vec![2000, 3000]
                    }
                );
                assert_eq!(terminals, if drained { 3 } else { 0 });
                observations.push((requests, visits));
                drop(reader);
                account.snapshot().assert_memory_released();
            }
            assert_eq!(observations[0], observations[1]);
        }
    }

    #[test]
    fn drained_child_visits_precede_terminal_and_staged_requests() {
        for end_ms in [1000, 2000] {
            for failure in 0..3 {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let mut reader = Reader::new(
                    Grid {
                        start_ms: 1000,
                        end_ms,
                        interval_ms: 1000,
                    },
                    Window {
                        first_end_ms: 1000,
                        end_ms,
                        range_ms: 500,
                        step_ms: 1000,
                    },
                );
                assert!(matches!(
                    reader
                        .advance(&control, &account, |_| unreachable!())
                        .unwrap(),
                    Action::Fetch(_)
                ));
                assert!(reader.receive(sample(&account, 1000)));
                let mut visits = 0;
                let mut terminals = 0;
                let result = (|| -> Result<(), EvaluationError> {
                    reader.complete_received_child(&control, &account, |_| {
                        visits += 1;
                        if failure == 0 {
                            return Err(EvaluationError::Invalid("drained visitor failure"));
                        }
                        Ok(())
                    })?;
                    assert!(reader.checkpoint().is_none());
                    terminals += 1;
                    if failure == 1 {
                        control.cancellation.cancel();
                    }
                    control.check()?;
                    match reader.advance(&control, &account, |_| unreachable!())? {
                        Action::Done => assert_eq!(end_ms, 1000),
                        Action::Fetch(fetch) => {
                            assert_eq!((fetch.time_ms, fetch.kind, fetch.occurrence), (2000, 1, 0))
                        }
                    }
                    Ok(())
                })();
                assert_eq!(visits, usize::from(end_ms == 1000));
                assert_eq!(terminals, usize::from(failure != 0 || end_ms != 1000));
                assert_eq!(
                    result,
                    match failure {
                        0 if end_ms == 1000 =>
                            Err(EvaluationError::Invalid("drained visitor failure")),
                        1 => Err(EvaluationError::Interrupted(Interrupted::Cancelled)),
                        _ => Ok(()),
                    }
                );
                drop(reader);
                account.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn derived_float_completion_preserves_visit_tail_and_cancellation_priority() {
        for phase in 0..5 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut reader = Reader::new(
                Grid {
                    start_ms: 1000,
                    end_ms: 2000,
                    interval_ms: 1000,
                },
                Window {
                    first_end_ms: 2000,
                    end_ms: 2000,
                    range_ms: 2000,
                    step_ms: 0,
                },
            );
            let mut visits = 0;
            let mut tails = 0;
            let result = (|| -> Result<(), EvaluationError> {
                let Action::Fetch(fetch) =
                    reader.advance(&control, &account, |_| unreachable!())?
                else {
                    panic!("initial Float request");
                };
                assert_eq!((fetch.time_ms, fetch.kind, fetch.occurrence), (1000, 1, 0));
                let truncate = reader.receive_continued(
                    sample(&account, 1000),
                    true,
                    &control,
                    &account,
                    |value| {
                        visits += 1;
                        assert_eq!(value.time_ms, 1000);
                        assert!(account.current_samples() >= 1);
                        if phase == 0 {
                            return Err(EvaluationError::Invalid("injected derived visit failure"));
                        }
                        if phase == 2 {
                            control.cancellation.cancel();
                        }
                        control.check()?;
                        Ok(())
                    },
                )?;
                assert!(!truncate, "eligible child must finish itself");
                // The driver is still inside the same child's completion path.
                tails += 1;
                if phase == 3 {
                    control.cancellation.cancel();
                }
                control.check()?;
                if phase == 1 {
                    return Err(EvaluationError::Invalid("injected derived tail failure"));
                }
                let Action::Fetch(next) = reader.advance(&control, &account, |_| unreachable!())?
                else {
                    panic!("completed Float proceeds to its histogram lane");
                };
                assert_eq!((next.time_ms, next.kind, next.occurrence), (1000, 2, 0));
                Ok(())
            })();
            assert_eq!(visits, 1);
            assert_eq!(tails, usize::from(matches!(phase, 1 | 3 | 4)));
            assert_eq!(
                result,
                match phase {
                    0 => Err(EvaluationError::Invalid("injected derived visit failure")),
                    1 => Err(EvaluationError::Invalid("injected derived tail failure")),
                    2 | 3 => Err(EvaluationError::Interrupted(Interrupted::Cancelled)),
                    _ => Ok(()),
                }
            );
            drop((result, reader));
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn derived_float_completion_keeps_endpoint_disjoint_and_unproved_requests() {
        let ordinary = Window {
            first_end_ms: 2000,
            end_ms: 2000,
            range_ms: 2000,
            step_ms: 0,
        };
        for (first, interval, window, single) in [
            (1000, 1000, ordinary, false),
            (2000, 1000, ordinary, true),
            (
                0,
                1000,
                Window {
                    range_ms: 1000,
                    ..ordinary
                },
                true,
            ),
            (1000, 0, ordinary, true),
            (
                1000,
                1000,
                Window {
                    end_ms: 3000,
                    step_ms: 0,
                    ..ordinary
                },
                true,
            ),
            (
                i64::MIN + 50,
                10,
                Window {
                    first_end_ms: i64::MIN + 100,
                    end_ms: i64::MIN + 100,
                    range_ms: 200,
                    step_ms: 0,
                },
                true,
            ),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut reader = Reader::new(
                Grid {
                    start_ms: first,
                    end_ms: window.first_end_ms,
                    interval_ms: interval,
                },
                window,
            );
            let mut visits = 0;
            assert!(matches!(
                reader
                    .advance(&control, &account, |_| unreachable!())
                    .unwrap(),
                Action::Fetch(_)
            ));
            assert!(
                reader
                    .receive_continued(sample(&account, first), single, &control, &account, |_| {
                        visits += 1;
                        Ok(())
                    })
                    .unwrap()
            );
            assert_eq!(
                visits, 0,
                "ordinary truncation must precede any later visitation"
            );
            drop(reader);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut reader = Reader::new(
            Grid {
                start_ms: 1000,
                end_ms: 3000,
                interval_ms: 1000,
            },
            Window {
                first_end_ms: 1000,
                end_ms: 3000,
                range_ms: 500,
                step_ms: 2000,
            },
        );
        let mut requests = Vec::new();
        let mut values = Vec::new();
        loop {
            let mut visit = |value: SampleRef<'_>| {
                let crate::value::SampleValueRef::Float(number) = value.value else {
                    unreachable!()
                };
                values.push((value.time_ms, number.to_bits()));
                Ok(())
            };
            match reader.advance(&control, &account, &mut visit).unwrap() {
                Action::Fetch(fetch) => {
                    requests.push((fetch.time_ms, fetch.kind, fetch.occurrence));
                    assert_eq!((fetch.kind, fetch.occurrence), (1, 0));
                    assert!(
                        reader
                            .receive_continued(
                                sample(&account, fetch.time_ms),
                                true,
                                &control,
                                &account,
                                &mut visit
                            )
                            .unwrap()
                    );
                }
                Action::Done => break,
            }
        }
        assert_eq!(requests, [(1000, 1, 0), (3000, 1, 0)]);
        assert_eq!(values, [(3000, 3.0_f64.to_bits())]);
        drop(reader);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}
