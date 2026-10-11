// SPDX-License-Identifier: GPL-3.0-or-later
// Destructive matrix heads derive from pinned promql/engine.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, QueryAccounting},
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions, RowFilter},
    query_scopes::{Grid, Traversal},
    storage::{Control, Interrupted},
};

#[derive(Clone, Copy)]
pub(crate) struct Clock {
    pub(crate) grid: Grid,
    pub(crate) shortcut: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Row {
    pub(crate) ordinal: usize,
    pub(crate) kinds: u8,
    pub(crate) sorted: bool,
}

pub(crate) struct Shape {
    pub(crate) clock: Clock,
    pub(crate) rows: AccountedVec<Row>,
}
impl Shape {
    pub(crate) fn sorted(
        &self,
        ordinal: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        for row in &*self.rows {
            control.check()?;
            account.work(1);
            if row.ordinal == ordinal {
                return Ok(row.sorted);
            }
        }
        Ok(false)
    }
}

#[derive(Clone, Copy)]
struct Origin {
    time_ms: i64,
    step_index: i64,
    kind: u8,
    occurrence: u64,
}

#[derive(Default)]
struct Lane {
    loaded: bool,
    head: Option<Origin>,
    consumed: u64,
    after_ms: Option<i64>,
}
impl Lane {
    fn consume(&mut self) {
        self.after_ms = self.head.take().map(|head| head.time_ms);
        self.consumed = self.consumed.wrapping_add(1);
        self.loaded = false;
    }
}

struct Scan {
    traversal: Traversal,
    kind: u8,
    sorted: bool,
    after_ms: Option<i64>,
    target: u64,
    seen: u64,
    occurrence: u64,
    waiting: bool,
    origin: Option<Origin>,
}
impl Scan {
    fn new(clock: Clock, row: Row, kind: u8, lane: &Lane) -> Self {
        Self {
            traversal: Traversal::new(clock.grid, clock.shortcut),
            kind,
            sorted: row.sorted,
            after_ms: lane.after_ms,
            target: lane.consumed,
            seen: 0,
            occurrence: 0,
            waiting: false,
            origin: None,
        }
    }
    fn next(&mut self) -> Option<(i64, i64)> {
        if self.waiting {
            self.waiting = false;
            if self.origin.is_some() && !self.sorted {
                return None;
            }
            self.traversal.advance();
        }
        let position = self.traversal.current()?;
        self.waiting = true;
        self.occurrence = 0;
        Some(position)
    }
    fn receive(&mut self, kind: u8) -> bool {
        if kind != self.kind {
            return false;
        }
        let (time_ms, step_index) = self.traversal.current().expect("requested matrix cell");
        let origin = Origin {
            time_ms,
            step_index,
            kind,
            occurrence: self.occurrence,
        };
        self.occurrence = self.occurrence.wrapping_add(1);
        if self.sorted {
            if self.after_ms.is_none_or(|after| time_ms > after)
                && self.origin.is_none_or(|prior| time_ms < prior.time_ms)
            {
                self.origin = Some(origin);
            }
            false
        } else {
            let selected = self.seen == self.target;
            self.seen = self.seen.wrapping_add(1);
            if selected {
                self.origin = Some(origin);
            }
            selected
        }
    }
}

pub(crate) enum Action {
    Child {
        ordinal: usize,
        options: EvaluationOptions,
        step_index: i64,
    },
    Sample(EvaluatedSample),
    Done,
}

struct Heads {
    clock: Clock,
    row: Row,
    options: EvaluationOptions,
    lanes: [Lane; 2],
    scan: Option<Scan>,
    loading_lane: usize,
    value_origin: Option<Origin>,
    value_waiting: bool,
    value_seen: u64,
    emitted: Option<EvaluatedSample>,
}
impl Heads {
    fn new(clock: Clock, row: Row, options: EvaluationOptions) -> Self {
        Self {
            clock,
            row,
            options,
            lanes: Default::default(),
            scan: None,
            loading_lane: 0,
            value_origin: None,
            value_waiting: false,
            value_seen: 0,
            emitted: None,
        }
    }
    fn child(&self, time_ms: i64, step_index: i64) -> Action {
        let grid = self.clock.grid;
        Action::Child {
            ordinal: self.row.ordinal,
            options: EvaluationOptions {
                time_ms,
                start_ms: grid.start_ms,
                end_ms: grid.end_ms,
                step_ms: grid.interval_ms,
                ..self.options
            },
            step_index,
        }
    }
    fn receive(&mut self, sample: EvaluatedSample) -> bool {
        if let Some(scan) = &mut self.scan {
            return scan.receive(sample.value.kind_bit());
        }
        let origin = self.value_origin.expect("requested matrix value");
        if sample.value.kind_bit() != origin.kind {
            return false;
        }
        let selected = self.value_seen == origin.occurrence;
        self.value_seen = self.value_seen.wrapping_add(1);
        if selected {
            self.emitted = Some(sample);
        }
        selected
    }
    fn advance(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<Action>, EvaluationError> {
        loop {
            control.check()?;
            account.work(1);
            if let Some(scan) = &mut self.scan {
                if let Some((time_ms, step_index)) = scan.next() {
                    return Ok(Some(self.child(time_ms, step_index)));
                }
                self.lanes[self.loading_lane].head = scan.origin;
                self.lanes[self.loading_lane].loaded = true;
                self.scan = None;
                continue;
            }
            if let Some(origin) = self.value_origin {
                if !self.value_waiting {
                    self.value_waiting = true;
                    self.value_seen = 0;
                    return Ok(Some(self.child(origin.time_ms, origin.step_index)));
                }
                let mut sample = self.emitted.take().ok_or(EvaluationError::Invalid(
                    "matrix head changed during replay",
                ))?;
                sample.ordinal = self.row.ordinal;
                sample.source_time_ms = origin.time_ms;
                sample.source_start_time_ms = 0;
                self.value_origin = None;
                self.value_waiting = false;
                return Ok(Some(Action::Sample(sample)));
            }
            for index in 0..2 {
                if self.lanes[index].loaded {
                    continue;
                }
                let kind = 1 << index;
                if self.row.kinds & kind == 0 {
                    self.lanes[index].loaded = true;
                } else {
                    self.scan = Some(Scan::new(self.clock, self.row, kind, &self.lanes[index]));
                    self.loading_lane = index;
                    break;
                }
            }
            if self.scan.is_some() {
                continue;
            }
            return Ok(None);
        }
    }
}

pub(crate) struct Step<'a> {
    shape: &'a Shape,
    options: EvaluationOptions,
    target_index: i64,
    filter: RowFilter<'a>,
    position: usize,
    consumer: Traversal,
    heads: Option<Heads>,
}
impl<'a> Step<'a> {
    pub(crate) fn new(
        shape: &'a Shape,
        options: EvaluationOptions,
        target_index: i64,
        filter: RowFilter<'a>,
    ) -> Self {
        Self {
            shape,
            options,
            target_index,
            filter,
            position: 0,
            consumer: Self::consumer(options),
            heads: None,
        }
    }
    fn consumer(options: EvaluationOptions) -> Traversal {
        Traversal::new(
            Grid {
                start_ms: options.start_ms,
                end_ms: options.end_ms,
                interval_ms: options.step_ms,
            },
            false,
        )
    }
    pub(crate) fn receive(&mut self, sample: EvaluatedSample) -> bool {
        self.heads
            .as_mut()
            .expect("requested matrix row")
            .receive(sample)
    }
    pub(crate) fn advance(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action, EvaluationError> {
        loop {
            control.check()?;
            account.work(1);
            let Some(row) = self.shape.rows.get(self.position).copied() else {
                return Ok(Action::Done);
            };
            if !self.filter.accepts(row.ordinal, control, account)? {
                self.position += 1;
                continue;
            }
            if self.heads.is_none() {
                self.consumer = Self::consumer(self.options);
                self.heads = Some(Heads::new(self.shape.clock, row, self.options));
            }
            let heads = self.heads.as_mut().expect("active matrix row");
            if let Some(mut action) = heads.advance(control, account)? {
                if let Action::Sample(sample) = &mut action {
                    sample.source_time_ms = self.options.time_ms;
                    self.position += 1;
                    self.heads = None;
                }
                return Ok(action);
            }
            let Some((time_ms, step_index)) = self.consumer.current() else {
                self.position += 1;
                self.heads = None;
                continue;
            };
            let matched = heads
                .lanes
                .iter()
                .position(|lane| lane.head.is_some_and(|head| head.time_ms == time_ms));
            if step_index == self.target_index {
                if let Some(index) = matched {
                    heads.value_origin = heads.lanes[index].head;
                } else {
                    self.position += 1;
                    self.heads = None;
                }
                continue;
            }
            if let Some(index) = matched {
                heads.lanes[index].consume();
            }
            self.consumer.advance();
        }
    }
}

pub(crate) struct SeriesIterator {
    heads: Heads,
    current_lane: Option<usize>,
    advance_current: bool,
}
impl SeriesIterator {
    pub(crate) fn new(clock: Clock, row: Row, options: EvaluationOptions) -> Self {
        Self {
            heads: Heads::new(clock, row, options),
            current_lane: None,
            advance_current: false,
        }
    }
    pub(crate) fn receive(&mut self, sample: EvaluatedSample) -> bool {
        self.heads.receive(sample)
    }
    pub(crate) fn advance(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action, EvaluationError> {
        loop {
            control.check()?;
            account.work(1);
            if self.advance_current {
                if let Some(index) = self.current_lane.take() {
                    self.heads.lanes[index].consume();
                }
                self.advance_current = false;
            }
            if let Some(action) = self.heads.advance(control, account)? {
                if matches!(action, Action::Sample(_)) {
                    self.advance_current = true;
                }
                return Ok(action);
            }
            let index = match (self.heads.lanes[0].head, self.heads.lanes[1].head) {
                (None, None) => return Ok(Action::Done),
                (Some(_), None) => 0,
                (None, Some(_)) => 1,
                // The source iterator advances one lane; equal timestamps select the float.
                (Some(float), Some(histogram)) => usize::from(histogram.time_ms < float.time_ms),
            };
            self.current_lane = Some(index);
            self.heads.value_origin = self.heads.lanes[index].head;
        }
    }
}
