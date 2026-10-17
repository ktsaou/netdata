// SPDX-License-Identifier: GPL-3.0-or-later
// Selection reducers derive from pinned promql/engine.go and value.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::{AnnotationBuilder, AnnotationKind},
    binary_labels,
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_roster::{Descriptor, copy_group_key},
    float_text::{FloatTextStyle, float_text},
    frontend::{
        Span,
        ast::{Expr, NodeId, ParsedExpression},
        token::TokenKind,
    },
    go_sort_program::{Operation, Program, physical_index},
    label_hash, numerical,
    position_index::PositionIndex,
    result::OwnedValue,
    shared::Strong,
    storage::{Control, Interrupted},
};
use std::cell::RefCell;

fn sample_offset(hash: u64) -> f64 {
    (hash as f64) / (u64::MAX as f64)
}
fn ratio_contains(ratio: f64, offset: f64) -> bool {
    (ratio >= 0.0 && offset < ratio) || (ratio < 0.0 && offset >= 1.0 + ratio)
}
#[cfg(feature = "fixtures")]
pub fn ratio_sampler_fixture(hash: u64, ratio: f64) -> (f64, bool) {
    let offset = sample_offset(hash);
    (offset, ratio_contains(ratio, offset))
}

pub(crate) fn is_selection(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::Aggregate {
            operator: TokenKind::TOPK
                | TokenKind::BOTTOMK
                | TokenKind::LIMITK
                | TokenKind::LIMIT_RATIO,
            ..
        }
    )
}

pub(crate) fn has_step_parameter(expression: &Expr) -> bool {
    is_selection(expression)
        || matches!(
            expression,
            Expr::Aggregate {
                operator: TokenKind::QUANTILE,
                ..
            }
        )
}

pub(crate) struct Parameters {
    constant: bool,
    value: f64,
    minimum: f64,
    maximum: f64,
    nan: bool,
    present: bool,
    samples: Option<crate::statistics::OwnedSamples>,
}
impl Parameters {
    pub(crate) fn new(constant: bool) -> Self {
        Self {
            constant,
            value: 0.0,
            minimum: f64::MAX,
            maximum: -f64::MAX,
            nan: false,
            present: false,
            samples: None,
        }
    }
    pub(crate) fn push(&mut self, value: f64, account: &QueryAccounting) {
        self.samples.get_or_insert_with(|| {
            crate::statistics::OwnedSamples::new(account, if self.constant { 3 } else { 2 })
        });
        if self.constant {
            self.value = value;
            self.minimum = value;
            self.maximum = value;
        } else {
            self.minimum = numerical::min(self.minimum, value);
            self.maximum = numerical::max(self.maximum, value);
        }
        self.nan |= value.is_nan();
        self.present = true;
    }
    pub(crate) fn fixed_value(&self) -> Option<f64> {
        self.constant.then_some(self.value)
    }
    pub(crate) fn add_quantile_annotations(
        &self,
        annotations: &mut AnnotationBuilder,
        span: Span,
        source: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if self.nan {
            annotations.add_invalid_quantile(
                f64::from_bits(0x7ff8_0000_0000_0001),
                span,
                source,
                control,
                account,
            )?;
        }
        if self.maximum > 1.0 && self.present {
            annotations.add_invalid_quantile(self.maximum, span, source, control, account)?;
        }
        if self.minimum < 0.0 && self.present {
            annotations.add_invalid_quantile(self.minimum, span, source, control, account)?;
        }
        Ok(())
    }
    fn validate(
        &self,
        operator: TokenKind,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, EvaluationError> {
        let minimum = if self.present { self.minimum } else { 0.0 };
        let maximum = if self.present { self.maximum } else { 0.0 };
        if operator == TokenKind::LIMIT_RATIO {
            if maximum == 0.0 && minimum == 0.0 {
                return Ok(false);
            }
            if self.nan {
                return Err(EvaluationError::Runtime(binary_labels::formatted(
                    format_args!("Ratio value is NaN"),
                    control,
                    account,
                )?));
            }
            return Ok(true);
        }
        if maximum < 1.0 {
            return Ok(false);
        }
        let message = if self.nan {
            binary_labels::formatted(format_args!("Parameter value is NaN"), control, account)?
        } else if minimum <= -9_223_372_036_854_775_808.0 {
            binary_labels::formatted(
                format_args!(
                    "Scalar value {} underflows int64",
                    float_text(minimum, FloatTextStyle::Shortest)
                ),
                control,
                account,
            )?
        } else if maximum >= 9_223_372_036_854_774_784.0 {
            binary_labels::formatted(
                format_args!(
                    "Scalar value {} overflows int64",
                    float_text(maximum, FloatTextStyle::Shortest)
                ),
                control,
                account,
            )?
        } else {
            return Ok(true);
        };
        Err(EvaluationError::Runtime(message))
    }
}

#[derive(Clone, Copy)]
struct Canonical {
    ordinal: usize,
    representative: usize,
}
struct Binding {
    group: usize,
    canonical: Option<Canonical>,
}
pub(crate) struct Plan {
    operator: TokenKind,
    parameters: Parameters,
    productive: bool,
    inputs: AccountedVec<Descriptor>,
    bindings: AccountedVec<Binding>,
    groups: usize,
    projected_rows: Option<usize>,
    cache: RefCell<Option<Strong<Snapshot>>>,
}
impl Plan {
    pub(crate) fn new(
        operator: TokenKind,
        parameters: Parameters,
        account: &QueryAccounting,
    ) -> Self {
        Self {
            operator,
            parameters,
            productive: false,
            inputs: AccountedVec::new(account, MemoryKind::Workspace),
            bindings: AccountedVec::new(account, MemoryKind::Workspace),
            groups: 0,
            projected_rows: None,
            cache: RefCell::new(None),
        }
    }
    pub(crate) fn prepare_rows(
        &mut self,
        rows: AccountedVec<Descriptor>,
        expression: &ParsedExpression,
        node: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let Expr::Aggregate {
            grouping, without, ..
        } = &expression.arena[node].expression
        else {
            unreachable!("typed selection aggregate")
        };
        let mut hashes = AccountedVec::<u64>::new(account, MemoryKind::Workspace);
        let mut index = PositionIndex::new(account);
        let mut bindings = AccountedVec::new(account, MemoryKind::Workspace);
        for row in &*rows {
            control.check()?;
            account.work(1);
            let labels =
                copy_group_key(&row.labels, grouping.as_deref(), *without, control, account)?;
            let hash = if !without && grouping.as_ref().is_none_or(|names| names.is_empty()) {
                0
            } else {
                label_hash::grouping(&labels, control, account)?
            };
            drop(labels);
            let group = if let Some(group) = index.find(
                hash,
                |position| Ok(hashes[position] == hash),
                control,
                account,
            )? {
                group
            } else {
                let position = hashes.len();
                hashes.push(hash);
                index.insert_new(hash, position, |prior| Ok(hashes[prior]), control, account)?;
                position
            };
            bindings.push(Binding {
                group,
                canonical: None,
            });
        }
        // The complete parameter and body precede this global validation in the source.
        self.productive = self.parameters.validate(self.operator, control, account)?;
        self.inputs = rows;
        self.bindings = bindings;
        self.groups = hashes.len();
        Ok(())
    }
    pub(crate) fn add_annotations(
        &self,
        annotations: &mut AnnotationBuilder,
        span: Span,
        source: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if self.operator != TokenKind::LIMIT_RATIO || !self.productive {
            return Ok(());
        }
        if self.parameters.maximum > 1.0 {
            annotations.add_invalid_ratio(
                self.parameters.maximum,
                1.0,
                span,
                source,
                control,
                account,
            )?;
        }
        if self.parameters.minimum < -1.0 {
            annotations.add_invalid_ratio(
                self.parameters.minimum,
                -1.0,
                span,
                source,
                control,
                account,
            )?;
        }
        Ok(())
    }
    pub(crate) fn project(
        &mut self,
        descriptors: &[Descriptor],
        sources: &[usize],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        for descriptor in descriptors {
            let representative = sources[descriptor.sources.start];
            for &source in &sources[descriptor.sources.clone()] {
                control.check()?;
                account.work(1);
                let input = self
                    .bindings
                    .get_mut(source)
                    .ok_or(EvaluationError::Invalid(
                        "selection projection source is missing",
                    ))?;
                input.canonical = Some(Canonical {
                    ordinal: descriptor.ordinal,
                    representative,
                });
            }
        }
        self.projected_rows = Some(descriptors.len());
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    time: i64,
    start: i64,
    end: i64,
    step: i64,
    position: i64,
}
impl From<(EvaluationOptions, i64)> for Key {
    fn from((options, position): (EvaluationOptions, i64)) -> Self {
        Self {
            time: options.time_ms,
            start: options.start_ms,
            end: options.end_ms,
            step: options.step_ms,
            position,
        }
    }
}
struct Heap {
    indices: AccountedVec<usize>,
}
struct Snapshot {
    _parameter_samples: Option<crate::statistics::OwnedSamples>,
    key: Key,
    k: usize,
    parameter: f64,
    heaps: AccountedVec<Heap>,
    ignored: bool,
}
#[derive(Clone, Copy)]
enum Stage {
    Parameter,
    Scan,
    Root,
    Program,
    Sort,
    Emit,
    Done,
}
#[derive(Clone, Copy)]
enum Resume {
    Scan,
    Sort,
}
struct Comparison {
    inputs: [usize; 2],
    values: [f64; 2],
    position: usize,
    samples: [Option<crate::statistics::OwnedSamples>; 2],
}
enum Pending {
    Empty,
    Float(f64),
    Histogram,
}
pub(crate) enum Action<'a> {
    Continue,
    Child {
        parameter: bool,
        ordinal: Option<&'a usize>,
    },
    Sample(EvaluatedSample<'a>),
    Done(Option<AnnotationKind>),
}
pub(crate) struct Step<'a> {
    plan: &'a Plan,
    options: EvaluationOptions,
    step_index: i64,
    stage: Stage,
    project: bool,
    vector: bool,
    canonical_rows: Option<&'a [Descriptor]>,
    heaps: AccountedVec<Heap>,
    snapshot: Option<Strong<Snapshot>>,
    k: usize,
    parameter: f64,
    input: usize,
    group: usize,
    waiting: bool,
    expected: Option<usize>,
    pending: Pending,
    pending_samples: Option<crate::statistics::OwnedSamples>,
    parameter_samples: Option<crate::statistics::OwnedSamples>,
    candidate: f64,
    candidate_samples: Option<crate::statistics::OwnedSamples>,
    program: Option<Program>,
    resume: Resume,
    comparison: Option<Comparison>,
    reply: Option<bool>,
    ignored: bool,
    emit_row: usize,
    emit_group: usize,
    emit_position: usize,
    emitted: Option<EvaluatedSample<'a>>,
}
impl<'a> Step<'a> {
    pub(crate) fn new(
        plan: &'a Plan,
        options: EvaluationOptions,
        step_index: i64,
        project: bool,
        vector: bool,
        canonical_rows: Option<&'a [Descriptor]>,
        account: &QueryAccounting,
    ) -> Self {
        let snapshot = {
            let cache = plan.cache.borrow();
            cache
                .as_ref()
                .filter(|snapshot| snapshot.key == Key::from((options, step_index)))
                .cloned()
        };
        let (stage, parameter) = if !plan.productive {
            (Stage::Done, 0.0)
        } else if let Some(snapshot) = &snapshot {
            (Stage::Emit, snapshot.parameter)
        } else if plan.parameters.constant {
            (Stage::Scan, plan.parameters.value)
        } else {
            (Stage::Parameter, 0.0)
        };
        let k = snapshot
            .as_ref()
            .map_or_else(|| Self::capacity(plan, parameter), |snapshot| snapshot.k);
        Self {
            plan,
            options,
            step_index,
            stage,
            project,
            vector,
            canonical_rows,
            snapshot,
            k,
            parameter,
            heaps: AccountedVec::new(account, MemoryKind::Workspace),
            input: 0,
            group: 0,
            waiting: false,
            expected: None,
            pending: Pending::Empty,
            pending_samples: None,
            parameter_samples: (!matches!(stage, Stage::Parameter | Stage::Done))
                .then(|| crate::statistics::OwnedSamples::new(account, 1)),
            candidate: 0.0,
            candidate_samples: None,
            program: None,
            resume: Resume::Scan,
            comparison: None,
            reply: None,
            ignored: false,
            emit_row: 0,
            emit_group: 0,
            emit_position: 0,
            emitted: None,
        }
    }
    fn capacity(plan: &Plan, parameter: f64) -> usize {
        if plan.operator == TokenKind::LIMIT_RATIO {
            if parameter == 0.0 {
                0
            } else {
                plan.inputs.len()
            }
        } else {
            (parameter as usize).min(plan.inputs.len())
        }
    }
    fn native(&self) -> bool {
        matches!(
            self.plan.operator,
            TokenKind::LIMITK | TokenKind::LIMIT_RATIO
        )
    }
    fn request(&mut self, input: usize) -> Action<'a> {
        self.waiting = true;
        self.expected = Some(input);
        self.pending = Pending::Empty;
        self.pending_samples = None;
        Action::Child {
            parameter: false,
            ordinal: Some(&self.plan.inputs[input].ordinal),
        }
    }
    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample<'a>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        control.check()?;
        account.work(1);
        if let Some(input) = self.expected {
            let row = &self.plan.inputs[input];
            if row.ordinal != sample.ordinal
                || !binary_labels::same(&row.labels, &sample.labels, control, account)?
            {
                return Ok(());
            }
        }
        if matches!(self.stage, Stage::Emit) {
            if self.emitted.is_none() {
                self.emitted = Some(sample);
            }
        } else if matches!(self.pending, Pending::Empty) {
            self.pending = match sample.value {
                OwnedValue::Float(value) => Pending::Float(value),
                OwnedValue::Histogram(_) => Pending::Histogram,
            };
            self.pending_samples = sample.float_samples;
        }
        Ok(())
    }
    fn float(&mut self) -> Result<f64, EvaluationError> {
        match std::mem::replace(&mut self.pending, Pending::Empty) {
            Pending::Float(value) => Ok(value),
            _ => Err(EvaluationError::Invalid(
                "selection operand changed during replay",
            )),
        }
    }
    fn comparison_value(&mut self) -> Result<f64, EvaluationError> {
        if self.native() && matches!(self.pending, Pending::Histogram) {
            self.pending = Pending::Empty;
            self.pending_samples = None;
            Ok(0.0)
        } else {
            self.float()
        }
    }
    fn start_program(&mut self, program: Program, group: usize, resume: Resume) {
        self.program = Some(program);
        self.group = group;
        self.resume = resume;
        self.stage = Stage::Program;
    }
    fn publish(&mut self, control: &Control, account: &QueryAccounting) -> Result<(), Interrupted> {
        control.check()?;
        let heaps = std::mem::replace(
            &mut self.heaps,
            AccountedVec::new(account, MemoryKind::Workspace),
        );
        let snapshot = Strong::new_in(
            Snapshot {
                _parameter_samples: self
                    .parameter_samples
                    .as_ref()
                    .map(|_| crate::statistics::OwnedSamples::new(account, 1)),
                key: (self.options, self.step_index).into(),
                k: self.k,
                parameter: self.parameter,
                heaps,
                ignored: self.ignored,
            },
            Some(&AllocationContext::new(account, MemoryKind::Workspace)),
        );
        *self.plan.cache.borrow_mut() = Some(snapshot.clone());
        self.snapshot = Some(snapshot);
        self.stage = Stage::Emit;
        Ok(())
    }
    fn next_winner(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        let snapshot = self
            .snapshot
            .as_ref()
            .expect("completed selection snapshot");
        loop {
            control.check()?;
            account.work(1);
            if self.project
                && let Some(rows) = self.plan.projected_rows
                && self.emit_row >= rows
            {
                return Ok(None);
            }
            let Some(heap) = snapshot.heaps.get(self.emit_group) else {
                if self.project && self.plan.projected_rows.is_some() {
                    self.emit_row += 1;
                    self.emit_group = 0;
                    self.emit_position = 0;
                    continue;
                }
                return Ok(None);
            };
            let Some(&input) = heap.indices.get(self.emit_position) else {
                self.emit_group += 1;
                self.emit_position = 0;
                continue;
            };
            self.emit_position += 1;
            if self.project && self.plan.projected_rows.is_some() {
                let canonical = self.plan.bindings[input]
                    .canonical
                    .expect("productive canonical binding");
                if canonical.ordinal != self.emit_row {
                    continue;
                }
            }
            return Ok(Some(input));
        }
    }
    pub(crate) fn advance(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action<'a>, EvaluationError> {
        control.check()?;
        account.work(1);
        match self.stage {
            Stage::Parameter => {
                if !self.waiting {
                    self.waiting = true;
                    self.expected = None;
                    return Ok(Action::Child {
                        parameter: true,
                        ordinal: None,
                    });
                }
                self.waiting = false;
                self.parameter = self.float()?;
                self.parameter_samples = self.pending_samples.take();
                self.k = Self::capacity(self.plan, self.parameter);
                self.stage = Stage::Scan;
            }
            Stage::Scan => {
                if self.k == 0 {
                    self.publish(control, account)?;
                    return Ok(Action::Continue);
                }
                if self.heaps.is_empty() {
                    for _ in 0..self.plan.groups {
                        control.check()?;
                        account.work(1);
                        self.heaps.push(Heap {
                            indices: AccountedVec::new(account, MemoryKind::Workspace),
                        });
                    }
                }
                if self.input == self.plan.inputs.len() {
                    self.group = 0;
                    self.stage = Stage::Sort;
                    return Ok(Action::Continue);
                }
                if !self.waiting {
                    return Ok(self.request(self.input));
                }
                self.waiting = false;
                let group = self.plan.bindings[self.input].group;
                let pending_samples = self.pending_samples.take();
                match std::mem::replace(&mut self.pending, Pending::Empty) {
                    Pending::Empty => self.input += 1,
                    Pending::Histogram if !self.native() => {
                        self.ignored = true;
                        self.input += 1;
                    }
                    value => {
                        let value = match value {
                            Pending::Float(value) => value,
                            Pending::Histogram => 0.0,
                            Pending::Empty => unreachable!(),
                        };
                        if self.plan.operator == TokenKind::LIMIT_RATIO {
                            let hash = label_hash::full(
                                &self.plan.inputs[self.input].labels,
                                control,
                                account,
                            )?;
                            let offset = sample_offset(hash);
                            let ratio = self.parameter.clamp(-1.0, 1.0);
                            if !ratio_contains(ratio, offset) {
                                self.input += 1;
                                return Ok(Action::Continue);
                            }
                        }
                        let heap = &mut self.heaps[group].indices;
                        if heap.len() < self.k {
                            heap.push(self.input);
                            let program = Program::heap_push(heap.len() as u64);
                            self.input += 1;
                            self.start_program(program, group, Resume::Scan);
                        } else if self.native() {
                            self.input += 1;
                        } else {
                            self.candidate = value;
                            self.candidate_samples = pending_samples;
                            self.group = group;
                            self.stage = Stage::Root;
                        }
                    }
                }
            }
            Stage::Root => {
                if !self.waiting {
                    return Ok(self.request(self.heaps[self.group].indices[0]));
                }
                self.waiting = false;
                let root = self.float()?;
                let _root_samples = self.pending_samples.take();
                let replace = (if self.plan.operator == TokenKind::TOPK {
                    root < self.candidate
                } else {
                    root > self.candidate
                }) || root.is_nan() && !self.candidate.is_nan();
                self.candidate_samples = None;
                if replace {
                    self.heaps[self.group].indices[0] = self.input;
                }
                self.input += 1;
                if replace && self.k > 1 {
                    self.start_program(
                        Program::heap_fix(0, self.heaps[self.group].indices.len() as u64),
                        self.group,
                        Resume::Scan,
                    );
                } else {
                    self.stage = Stage::Scan;
                }
            }
            Stage::Program => {
                if let Some(comparison) = &mut self.comparison {
                    if self.waiting {
                        self.waiting = false;
                        let position = comparison.position;
                        let value = self.comparison_value()?;
                        let comparison = self.comparison.as_mut().expect("comparison continuation");
                        comparison.values[position] = value;
                        comparison.samples[position] = self.pending_samples.take();
                        comparison.position += 1;
                    }
                    let comparison = self.comparison.as_ref().expect("comparison continuation");
                    if comparison.position < 2 {
                        return Ok(self.request(comparison.inputs[comparison.position]));
                    }
                    let [left, right] = comparison.values;
                    self.reply = Some(
                        left.is_nan()
                            || if self.plan.operator == TokenKind::BOTTOMK {
                                left > right
                            } else {
                                left < right
                            },
                    );
                    self.comparison = None;
                    return Ok(Action::Continue);
                }
                match self
                    .program
                    .as_mut()
                    .expect("active ordering program")
                    .next(self.reply.take())
                {
                    Operation::Step => (),
                    Operation::Swap(left, right) => {
                        let heap = &mut self.heaps[self.group].indices;
                        let (left, right) = (
                            physical_index(left, heap.len()),
                            physical_index(right, heap.len()),
                        );
                        heap.swap(left, right);
                    }
                    Operation::Less(left, right) => {
                        let heap = &self.heaps[self.group].indices;
                        let (left, right) = (
                            physical_index(left, heap.len()),
                            physical_index(right, heap.len()),
                        );
                        let mut inputs = [heap[left], heap[right]];
                        if matches!(self.resume, Resume::Sort) {
                            inputs.swap(0, 1);
                        }
                        self.comparison = Some(Comparison {
                            inputs,
                            values: [0.0; 2],
                            samples: [None, None],
                            position: 0,
                        });
                    }
                    Operation::Done => {
                        self.program = None;
                        self.stage = match self.resume {
                            Resume::Scan => Stage::Scan,
                            Resume::Sort => {
                                self.group += 1;
                                Stage::Sort
                            }
                        };
                    }
                }
            }
            Stage::Sort => {
                if self.native() || self.group == self.heaps.len() {
                    self.publish(control, account)?;
                } else if self.heaps[self.group].indices.len() > 1 {
                    self.start_program(
                        Program::sort(self.heaps[self.group].indices.len() as u64),
                        self.group,
                        Resume::Sort,
                    );
                } else {
                    self.group += 1;
                }
            }
            Stage::Emit => {
                if self.waiting {
                    self.waiting = false;
                    let input = self.expected.expect("requested selection winner");
                    let mut sample = self.emitted.take().ok_or(EvaluationError::Invalid(
                        "selection winner changed during replay",
                    ))?;
                    if !self.native() && !matches!(sample.value, OwnedValue::Float(_)) {
                        return Err(EvaluationError::Invalid(
                            "selection winner changed during replay",
                        ));
                    }
                    sample.ordinal = input;
                    if self.project
                        && let Some(canonical) = self.plan.bindings[input].canonical
                    {
                        if self.vector {
                            let kind = if matches!(sample.value, OwnedValue::Float(_)) {
                                1
                            } else {
                                2
                            };
                            if !self
                                .canonical_rows
                                .expect("frozen canonical selection rows")[canonical.ordinal]
                                .heads
                                .accepts(kind, self.options.time_ms)
                            {
                                return Ok(Action::Continue);
                            }
                            self.emit_row += 1;
                            self.emit_group = 0;
                            self.emit_position = 0;
                        }
                        let representative = &self.plan.inputs[canonical.representative];
                        sample.ordinal = canonical.ordinal;
                        sample.labels = representative
                            .labels
                            .copy_checked(account, MemoryKind::Workspace, control)?
                            .into();
                        sample.drop_name = representative.drop_name;
                    }
                    sample.source_time_ms = self.options.time_ms;
                    sample.source_start_time_ms = 0;
                    return Ok(Action::Sample(sample));
                }
                let Some(input) = self.next_winner(control, account)? else {
                    self.stage = Stage::Done;
                    return Ok(Action::Continue);
                };
                return Ok(self.request(input));
            }
            Stage::Done => {
                let ignored = self
                    .snapshot
                    .as_ref()
                    .map_or(self.ignored, |snapshot| snapshot.ignored);
                return Ok(Action::Done(ignored.then_some(
                    AnnotationKind::HistogramIgnoredInAggregation(self.plan.operator),
                )));
            }
        }
        Ok(Action::Continue)
    }
}
