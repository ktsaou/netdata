// SPDX-License-Identifier: GPL-3.0-or-later
// Selection reducers derive from pinned promql/engine.go and value.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::{AnnotationBuilder, AnnotationKind},
    binary_labels,
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_roster::{Descriptor, group_hash},
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
    sample_labels::SampleLabels,
    shared::Strong,
    storage::{Control, Interrupted},
};
use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, Ordering},
};

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
    emit_certified: bool,
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
            emit_certified: false,
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
            let hash = group_hash(&row.labels, grouping.as_deref(), *without, control, account)?;
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
    pub(crate) fn clear_emission(&self) {
        if let Some(snapshot) = &*self.cache.borrow() {
            snapshot.emitted.store(false, Ordering::Relaxed);
        }
    }
    pub(crate) fn certify_emit(
        &mut self,
        descriptors: &[Descriptor],
        sources: &[usize],
        catalog_len: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if !matches!(self.operator, TokenKind::TOPK | TokenKind::BOTTOMK)
            || self.inputs.len() < 2
            || self.bindings.len() != self.inputs.len()
            || descriptors.len() < 2
            || descriptors.len() > self.inputs.len()
            || self.projected_rows != Some(descriptors.len())
        {
            return Ok(());
        }
        let mut prior = None;
        for (input, row) in self.inputs.iter().enumerate() {
            control.check()?;
            account.work(1);
            if row.kinds != 1
                || row.heads.multiple()
                || row.ordinal >= catalog_len
                || prior.is_some_and(|ordinal| row.ordinal <= ordinal)
            {
                return Ok(());
            }
            prior = Some(row.ordinal);
            if let Some(canonical) = self.bindings[input].canonical {
                let Some(descriptor) = descriptors.get(canonical.ordinal) else {
                    return Ok(());
                };
                if descriptor.sources.len() != 1
                    || sources.get(descriptor.sources.start) != Some(&input)
                {
                    return Ok(());
                }
            }
        }
        for (position, descriptor) in descriptors.iter().enumerate() {
            control.check()?;
            account.work(1);
            if descriptor.kinds != 1
                || descriptor.heads.multiple()
                || descriptor.sources.len() != 1
                || descriptor.ordinal != position
            {
                return Ok(());
            }
            let Some(&input) = sources.get(descriptor.sources.start) else {
                return Ok(());
            };
            let Some(row) = self.inputs.get(input) else {
                return Ok(());
            };
            let Some(canonical) = self.bindings[input].canonical else {
                return Ok(());
            };
            if canonical.ordinal != descriptor.ordinal
                || canonical.representative != input
                || row.hash != descriptor.hash
            {
                return Ok(());
            }
        }
        control.check()?;
        self.emit_certified = true;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn emission_state(&self) -> (bool, bool) {
        (
            self.emit_certified,
            self.cache
                .borrow()
                .as_ref()
                .is_some_and(|snapshot| snapshot.emitted.load(Ordering::Relaxed)),
        )
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
    emitted: AtomicBool,
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
                emitted: AtomicBool::new(false),
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
        target: Option<usize>,
        sources: Option<&[usize]>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        let snapshot = self
            .snapshot
            .as_ref()
            .expect("completed selection snapshot");
        let target = target.filter(|_| {
            self.plan.emit_certified
                && self.project
                && self.vector
                && snapshot.emitted.load(Ordering::Relaxed)
        });
        if let Some(target) = target
            && self.emit_row < target
        {
            self.emit_row = target;
            self.emit_group = 0;
            self.emit_position = 0;
        }
        let target_group = if target == Some(self.emit_row) {
            sources.and_then(|sources| {
                account.work(1);
                self.canonical_rows
                    .and_then(|rows| rows.get(self.emit_row))
                    .and_then(|row| sources.get(row.sources.start))
                    .and_then(|&input| self.plan.bindings.get(input))
                    .map(|binding| binding.group)
            })
        } else {
            None
        };
        if let Some(group) = target_group
            && self.emit_group < group
        {
            self.emit_group = group;
            self.emit_position = 0;
        }
        loop {
            control.check()?;
            account.work(1);
            if target_group.is_some_and(|group| self.emit_group > group) {
                return Ok(None);
            }
            if target.is_some_and(|target| self.emit_row > target) {
                return Ok(None);
            }
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
        target: Option<usize>,
        sources: Option<&[usize]>,
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
                        control.check()?;
                        sample.labels = SampleLabels::Borrowed(&representative.labels);
                        sample.drop_name = representative.drop_name;
                    }
                    sample.source_time_ms = self.options.time_ms;
                    sample.source_start_time_ms = 0;
                    return Ok(Action::Sample(sample));
                }
                let Some(input) = self.next_winner(target, sources, control, account)? else {
                    self.stage = Stage::Done;
                    return Ok(Action::Continue);
                };
                return Ok(self.request(input));
            }
            Stage::Done => {
                if self.plan.emit_certified && self.project && self.vector {
                    control.check()?;
                    if let Some(snapshot) = &self.snapshot {
                        // A cached selection alone does not validate its full delivery.
                        snapshot.emitted.store(true, Ordering::Relaxed);
                    }
                }
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

#[cfg(test)]
mod emission_tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting, evaluator_roster::PointCounts, histogram::FloatHistogram,
        sample_labels::SampleLabels, statistics::OwnedSamples,
        test_storage::group_preparation as fixture, value::SampleValueRef,
    };

    struct Fixture {
        plan: Plan,
        canonical: AccountedVec<Descriptor>,
        sources: Vec<usize>,
    }

    #[test]
    fn ranking_snapshot_traits_and_accounted_layouts_remain_explicit() {
        fn send<T: Send>() {}
        fn shared<T: Send + Sync>() {}
        send::<Plan>();
        shared::<Snapshot>();
        shared::<Strong<Snapshot>>();
        println!(
            "{{\"ranking_plan\":{},\"ranking_plan_allocation\":{},\"ranking_snapshot\":{},\"ranking_snapshot_allocation\":{},\"ranking_step\":{},\"ranking_step_allocation\":{}}}",
            size_of::<Plan>(),
            crate::accounting::AccountedBox::<Plan>::allocation_bytes(),
            size_of::<Snapshot>(),
            Strong::<Snapshot>::allocation_bytes(),
            size_of::<Step<'static>>(),
            crate::accounting::AccountedBox::<Step<'static>>::allocation_bytes()
        );
    }

    impl Fixture {
        fn unique(operator: TokenKind, account: &QueryAccounting) -> Self {
            let control = Control::default();
            let expression = fixture::expression(
                if operator == TokenKind::TOPK {
                    "topk by(id)(1,m)"
                } else {
                    "bottomk by(id)(1,m)"
                },
                account,
                &control,
            );
            let labels = |index: usize| {
                vec![
                    (b"__name__".to_vec(), b"m".to_vec()),
                    (b"id".to_vec(), index.to_string().into_bytes()),
                ]
            };
            let mut rows = fixture::rows((0..3).map(labels).collect(), account, &control);
            for (position, row) in rows.iter_mut().enumerate() {
                row.ordinal = 1 + 3 * position;
                row.kinds = 1;
            }
            let sources = [2, 0, 1];
            let mut canonical = fixture::rows(
                sources.iter().map(|&input| labels(input)).collect(),
                account,
                &control,
            );
            for (position, row) in canonical.iter_mut().enumerate() {
                row.ordinal = position;
                row.kinds = 1;
                row.sources = position..position + 1;
            }
            let mut parameters = Parameters::new(true);
            parameters.push(1.0, account);
            let mut plan = Plan::new(operator, parameters, account);
            plan.prepare_rows(rows, &expression, expression.root, &control, account)
                .unwrap();
            plan.project(&canonical, &sources, &control, account)
                .unwrap();
            plan.certify_emit(&canonical, &sources, 8, &control, account)
                .unwrap();
            assert_eq!(plan.emission_state(), (true, false));
            Self {
                plan,
                canonical,
                sources: sources.to_vec(),
            }
        }

        fn new(operator: TokenKind, width: Option<usize>, account: &QueryAccounting) -> Self {
            let (name, parameter) = match operator {
                TokenKind::TOPK => ("topk", 1.0),
                TokenKind::BOTTOMK => ("bottomk", 1.0),
                TokenKind::LIMITK => ("limitk", 2.0),
                TokenKind::LIMIT_RATIO => ("limit_ratio", 1.0),
                _ => unreachable!(),
            };
            let control = Control::default();
            let expression =
                fixture::expression(&format!("{name}({parameter},m)"), account, &control);
            let raw = width.map_or_else(
                || vec![vec![], vec![]],
                |width| {
                    let suffix = vec![
                        (b"zzempty".to_vec(), vec![]),
                        (b"zzpad".to_vec(), vec![b'p'; width]),
                    ];
                    let mut first = vec![
                        (b"__name__".to_vec(), b"m".to_vec()),
                        (b"a".to_vec(), b"b\xffx\xffy".to_vec()),
                    ];
                    first.extend(suffix.clone());
                    if width == 13 {
                        vec![
                            vec![(b"a".to_vec(), b"Aaaaa\0\0\0\0\0\0\0\0".to_vec())],
                            vec![(
                                b"a".to_vec(),
                                b"Bbbbb\x1d\x3c\xa5\x95\x96\x2f\x43\xdd".to_vec(),
                            )],
                        ]
                    } else {
                        vec![first.clone(), first]
                    }
                },
            );
            let rows = fixture::rows(raw, account, &control);
            assert_eq!(rows[0].hash, rows[1].hash);
            if width == Some(13) {
                assert_ne!(fixture::raw(&rows[0].labels), fixture::raw(&rows[1].labels));
                assert_eq!(rows[0].hash, 0x9c71_0b2e_3747_e3bd);
            }
            let mut canonical =
                fixture::rows(vec![fixture::raw(&rows[0].labels)], account, &control);
            canonical[0].ordinal = 0;
            canonical[0].sources = 0..2;
            let mut parameters = Parameters::new(true);
            parameters.push(parameter, account);
            let mut plan = Plan::new(operator, parameters, account);
            plan.prepare_rows(rows, &expression, expression.root, &control, account)
                .unwrap();
            plan.project(&canonical, &[0, 1], &control, account)
                .unwrap();
            Self {
                plan,
                canonical,
                sources: vec![0, 1],
            }
        }
    }

    fn options(time_ms: i64) -> EvaluationOptions {
        EvaluationOptions {
            time_ms,
            start_ms: 0,
            end_ms: 2000,
            step_ms: 1000,
            lookback_ms: 1,
            start_timestamps: true,
            delayed_name_removal: false,
        }
    }

    fn value(index: usize, flip: bool) -> f64 {
        if (index == 1) != flip { 2.0 } else { 1.0 }
    }

    fn source<'a>(
        plan: &'a Plan,
        index: usize,
        histogram: bool,
        flip: bool,
        account: &QueryAccounting,
    ) -> EvaluatedSample<'a> {
        let value = if histogram {
            let histogram = FloatHistogram {
                count: 1.0,
                sum: value(index, flip),
                ..FloatHistogram::default()
            };
            OwnedValue::copy_from(
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                account,
                MemoryKind::Workspace,
            )
        } else {
            OwnedValue::Float(value(index, flip))
        };
        EvaluatedSample {
            ordinal: plan.inputs[index].ordinal,
            labels: SampleLabels::Borrowed(&plan.inputs[index].labels),
            value,
            float_samples: (!histogram).then(|| OwnedSamples::new(account, 1)),
            source_time_ms: -123,
            source_start_time_ms: -456,
            drop_name: !plan.inputs[index].drop_name,
        }
    }

    fn prime<'a>(fixture: &'a Fixture, clock: i64, account: &QueryAccounting) -> Step<'a> {
        let control = Control::default();
        let mut step = Step::new(
            &fixture.plan,
            options(clock),
            clock / 1000,
            true,
            true,
            Some(&fixture.canonical),
            account,
        );
        while step.snapshot.is_none() {
            match step
                .advance(Some(1), Some(&fixture.sources), &control, account)
                .unwrap()
            {
                Action::Continue => (),
                Action::Child { parameter, ordinal } => {
                    assert!(!parameter);
                    let input = fixture
                        .plan
                        .inputs
                        .iter()
                        .position(|row| Some(&row.ordinal) == ordinal)
                        .unwrap();
                    step.receive(
                        source(&fixture.plan, input, false, false, account),
                        &control,
                        account,
                    )
                    .unwrap();
                }
                _ => panic!("selection must publish before delivery"),
            }
        }
        assert!(matches!(step.stage, Stage::Emit));
        step
    }

    fn deliver(
        step: &mut Step<'_>,
        target: Option<usize>,
        offset: f64,
        control: &Control,
        account: &QueryAccounting,
    ) -> (Vec<(usize, u64)>, Vec<usize>) {
        let mut output = Vec::new();
        let mut requests = Vec::new();
        loop {
            match step
                .advance(target, Some(&[2, 0, 1]), control, account)
                .unwrap()
            {
                Action::Continue => (),
                Action::Child { parameter, ordinal } => {
                    assert!(!parameter);
                    let input = step
                        .plan
                        .inputs
                        .iter()
                        .position(|row| Some(&row.ordinal) == ordinal)
                        .unwrap();
                    requests.push(input);
                    let mut sample = source(step.plan, input, false, false, account);
                    sample.value = OwnedValue::Float(offset + input as f64);
                    step.receive(sample, control, account).unwrap();
                }
                Action::Sample(sample) => {
                    let OwnedValue::Float(value) = sample.value else {
                        panic!("float winner")
                    };
                    let input = [2, 0, 1][sample.ordinal];
                    assert_eq!(
                        fixture::raw(&sample.labels),
                        fixture::raw(&step.plan.inputs[input].labels)
                    );
                    assert_eq!(value.to_bits(), (offset + input as f64).to_bits());
                    output.push((sample.ordinal, value.to_bits()));
                }
                Action::Done(annotation) => {
                    assert!(annotation.is_none());
                    return (output, requests);
                }
            }
        }
    }

    #[test]
    fn completed_ranking_delivery_uses_canonical_targets_and_fresh_selected_values() {
        for operator in [TokenKind::TOPK, TokenKind::BOTTOMK] {
            let account = EngineAccounting::default().query();
            let fixture = Fixture::unique(operator, &account);
            let control = Control::default();
            let mut cold = prime(&fixture, 0, &account);
            assert_eq!(fixture.plan.emission_state(), (true, false));
            let (output, requests) = deliver(&mut cold, Some(1), 10.0, &control, &account);
            assert_eq!(
                output.iter().map(|&(id, _)| id).collect::<Vec<_>>(),
                [0, 1, 2]
            );
            assert_eq!(requests, [2, 0, 1]);
            assert_eq!(fixture.plan.emission_state(), (true, true));
            for target in 0..4 {
                let mut warm = prime(&fixture, 0, &account);
                let (output, requests) =
                    deliver(&mut warm, Some(target), 100.0, &control, &account);
                if target < 3 {
                    assert_eq!(requests, [[2], [0], [1]][target]);
                    assert_eq!(output, [(target, (100.0 + requests[0] as f64).to_bits())]);
                } else {
                    assert!(output.is_empty() && requests.is_empty());
                }
            }
            let mut ordinary = prime(&fixture, 0, &account);
            assert_eq!(
                deliver(&mut ordinary, None, 20.0, &control, &account).1,
                [2, 0, 1]
            );
            fixture.plan.clear_emission();
            assert_eq!(fixture.plan.emission_state(), (true, false));
            let mut reset = prime(&fixture, 0, &account);
            assert_eq!(
                deliver(&mut reset, Some(1), 30.0, &control, &account).1,
                [2, 0, 1]
            );
            let mut next = prime(&fixture, 1000, &account);
            assert_eq!(fixture.plan.emission_state(), (true, false));
            assert_eq!(
                deliver(&mut next, Some(1), 40.0, &control, &account).1,
                [2, 0, 1]
            );
            drop((cold, ordinary, reset, next));
            drop(fixture);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn ranking_completion_belongs_to_the_held_snapshot_and_checks_terminal_control() {
        let account = EngineAccounting::default().query();
        let fixture = Fixture::unique(TokenKind::TOPK, &account);
        let control = Control::default();
        let mut old = prime(&fixture, 0, &account);
        let mut replacement = prime(&fixture, 1000, &account);
        deliver(&mut old, Some(1), 10.0, &control, &account);
        assert!(
            old.snapshot
                .as_ref()
                .unwrap()
                .emitted
                .load(Ordering::Relaxed)
        );
        assert_eq!(fixture.plan.emission_state(), (true, false));
        replacement.stage = Stage::Done;
        let result = crate::test_storage::interrupt_after_work(&account, &control, 1, || {
            replacement.advance(Some(1), Some(&fixture.sources), &control, &account)
        });
        assert!(matches!(
            result,
            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
        ));
        assert_eq!(fixture.plan.emission_state(), (true, false));
        drop((old, replacement));
        drop(fixture);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn completed_ranking_delivery_keeps_cold_suffix_and_warm_selected_errors() {
        for warm in [false, true] {
            for wrong_kind in [false, true] {
                let account = EngineAccounting::default().query();
                let fixture = Fixture::unique(TokenKind::TOPK, &account);
                let control = Control::default();
                let mut seed = prime(&fixture, 0, &account);
                if warm {
                    deliver(&mut seed, Some(1), 10.0, &control, &account);
                }
                let mut step = prime(&fixture, 0, &account);
                let fault = if warm { 0 } else { 1 };
                let mut delivered = 0;
                loop {
                    match step.advance(Some(1), Some(&fixture.sources), &control, &account) {
                        Ok(Action::Continue) => (),
                        Ok(Action::Child { parameter, ordinal }) => {
                            assert!(!parameter);
                            let input = fixture
                                .plan
                                .inputs
                                .iter()
                                .position(|row| Some(&row.ordinal) == ordinal)
                                .unwrap();
                            if input != fault || wrong_kind {
                                step.receive(
                                    source(&fixture.plan, input, input == fault, false, &account),
                                    &control,
                                    &account,
                                )
                                .unwrap();
                            }
                        }
                        Ok(Action::Sample(_)) => {
                            delivered += 1;
                            assert!(!warm);
                            assert_eq!(fixture.plan.emission_state(), (true, false));
                        }
                        Err(EvaluationError::Invalid("selection winner changed during replay")) => {
                            break;
                        }
                        _ => panic!("fresh replay failure must remain visible"),
                    }
                }
                assert_eq!(delivered, if warm { 0 } else { 2 });
                assert_eq!(fixture.plan.emission_state(), (true, warm));
                drop((seed, step));
                drop(fixture);
                account.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn ranking_certificate_rejects_aliases_kinds_noncanonical_and_catalog_mismatches() {
        for scenario in 0..17 {
            let account = EngineAccounting::default().query();
            let mut fixture = Fixture::unique(TokenKind::TOPK, &account);
            fixture.plan.emit_certified = false;
            let mut sources = [2, 0, 1];
            let mut catalog_len = 8;
            match scenario {
                0 => fixture.plan.operator = TokenKind::LIMITK,
                1 => fixture.plan.inputs[0].kinds = 3,
                2 => fixture.canonical[0].kinds = 2,
                3 => fixture.plan.inputs[1].ordinal = fixture.plan.inputs[0].ordinal,
                4 => catalog_len = 7,
                5 => fixture.canonical[0].sources = 0..2,
                6 => fixture.canonical[0].ordinal = 1,
                7 => sources[0] = 0,
                8 => {
                    fixture.plan.bindings[2]
                        .canonical
                        .as_mut()
                        .unwrap()
                        .representative = 0
                }
                9 => fixture.canonical[0].hash ^= 1,
                10 => fixture.plan.projected_rows = None,
                11 => fixture.plan.bindings[2].canonical = None,
                12 => sources[0] = usize::MAX,
                13 => {
                    fixture.plan.bindings.pop();
                }
                14 | 15 => {
                    let mut counts = PointCounts::default();
                    counts.push(1);
                    counts.push(1);
                    if scenario == 14 {
                        fixture.plan.inputs[0].heads.note(counts, 0);
                    } else {
                        fixture.canonical[0].heads.note(counts, 0);
                    }
                }
                _ => fixture.plan.projected_rows = Some(2),
            }
            fixture
                .plan
                .certify_emit(
                    &fixture.canonical,
                    &sources,
                    catalog_len,
                    &Control::default(),
                    &account,
                )
                .unwrap();
            assert_eq!(
                fixture.plan.emission_state(),
                (false, false),
                "scenario={scenario}"
            );
            drop(fixture);
            account.snapshot().assert_memory_released();
        }
    }

    fn subset_fixture(
        operator: TokenKind,
        keep: usize,
        account: &QueryAccounting,
    ) -> (Fixture, Vec<usize>) {
        let control = Control::default();
        let name = if operator == TokenKind::TOPK {
            "topk"
        } else {
            "bottomk"
        };
        let expression = fixture::expression(&format!("{name}({keep},m)"), account, &control);
        let labels = |i: usize| {
            vec![
                (b"__name__".to_vec(), b"m".to_vec()),
                (b"id".to_vec(), format!("{i:02}").into_bytes()),
            ]
        };
        let count = 2 * keep;
        let mut rows = fixture::rows((0..count).map(labels).collect(), account, &control);
        for (i, row) in rows.iter_mut().enumerate() {
            row.ordinal = 1 + 3 * i;
            row.kinds = 1;
        }
        let sources: Vec<_> = (keep..count).rev().collect();
        let mut canonical = fixture::rows(
            sources.iter().copied().map(labels).collect(),
            account,
            &control,
        );
        for (i, row) in canonical.iter_mut().enumerate() {
            row.ordinal = i;
            row.kinds = 1;
            row.sources = i..i + 1;
        }
        let mut parameters = Parameters::new(true);
        parameters.push(keep as f64, account);
        let mut plan = Plan::new(operator, parameters, account);
        plan.prepare_rows(rows, &expression, expression.root, &control, account)
            .unwrap();
        plan.project(&canonical, &sources, &control, account)
            .unwrap();
        plan.certify_emit(&canonical, &sources, 3 * count, &control, account)
            .unwrap();
        (
            Fixture {
                plan,
                canonical,
                sources: sources.clone(),
            },
            sources,
        )
    }

    fn deliver_subset(
        fixture: &Fixture,
        sources: &[usize],
        target: Option<usize>,
        offset: f64,
        account: &QueryAccounting,
    ) -> (Vec<(usize, u64)>, usize) {
        let control = Control::default();
        let mut step = Step::new(
            &fixture.plan,
            options(0),
            0,
            true,
            true,
            Some(&fixture.canonical),
            account,
        );
        let value = |input: usize| {
            let magnitude = (input + 1) as f64;
            if fixture.plan.operator == TokenKind::TOPK {
                magnitude
            } else {
                -magnitude
            }
        };
        let mut output = Vec::new();
        let mut requested = 0;
        loop {
            match step
                .advance(target, Some(&fixture.sources), &control, account)
                .unwrap()
            {
                Action::Continue => (),
                Action::Child { parameter, ordinal } => {
                    assert!(!parameter);
                    let input = fixture
                        .plan
                        .inputs
                        .iter()
                        .position(|row| Some(&row.ordinal) == ordinal)
                        .unwrap();
                    let mut sample = source(&fixture.plan, input, false, false, account);
                    sample.value = OwnedValue::Float(
                        value(input) + if step.snapshot.is_some() { offset } else { 0.0 },
                    );
                    requested += 1;
                    step.receive(sample, &control, account).unwrap();
                }
                Action::Sample(sample) => {
                    let input = sources[sample.ordinal];
                    assert_eq!(
                        fixture::raw(&sample.labels),
                        fixture::raw(&fixture.plan.inputs[input].labels)
                    );
                    assert_eq!(sample.drop_name, input % 2 == 1);
                    assert_eq!(sample.source_time_ms, 0);
                    assert_eq!(sample.source_start_time_ms, 0);
                    let OwnedValue::Float(actual) = sample.value else {
                        panic!("float winner")
                    };
                    assert_eq!(actual.to_bits(), (value(input) + offset).to_bits());
                    if target.is_none_or(|target| target == sample.ordinal) {
                        output.push((sample.ordinal, actual.to_bits()));
                    }
                }
                Action::Done(annotation) => {
                    assert!(annotation.is_none());
                    return (output, requested);
                }
            }
        }
    }

    #[test]
    fn proper_ranking_subsets_avoid_repeated_completed_winner_requests() {
        let mut observations = Vec::new();
        for operator in [TokenKind::TOPK, TokenKind::BOTTOMK] {
            let mut requests = Vec::new();
            for keep in [2, 4, 8, 16] {
                let account = EngineAccounting::default().query();
                let (fixture, sources) = subset_fixture(operator, keep, &account);
                let cold = deliver_subset(&fixture, &sources, None, 10.0, &account);
                assert_eq!(
                    cold.0.iter().map(|row| row.0).collect::<Vec<_>>(),
                    (0..keep).collect::<Vec<_>>()
                );
                assert!(
                    fixture.plan.bindings[..keep]
                        .iter()
                        .all(|row| row.canonical.is_none())
                );
                let mut fetched = 0;
                for target in 0..keep {
                    let offset = 100.0 + target as f64;
                    let (output, count) =
                        deliver_subset(&fixture, &sources, Some(target), offset, &account);
                    let value = (sources[target] + 1) as f64;
                    let expected = if operator == TokenKind::TOPK {
                        value
                    } else {
                        -value
                    };
                    assert_eq!(output, [(target, (expected + offset).to_bits())]);
                    fetched += count;
                }
                println!(
                    "{{\"ranking_subset\":\"{operator:?}\",\"kept\":{keep},\"warm_requests\":{fetched}}}"
                );
                requests.push(fetched);
                drop(fixture);
                account.snapshot().assert_memory_released();
            }
            observations.push(requests);
        }
        // Validate every complete cold/warm value and owner before checking repeated work.
        assert!(
            observations.iter().all(|reads| reads[3] <= 3 * reads[2]),
            "proper ranking subsets must avoid repeated completed winner requests: {observations:?}"
        );
    }

    #[test]
    fn ranking_subset_certificate_checks_inverse_bindings_and_small_unions() {
        for operator in [TokenKind::TOPK, TokenKind::BOTTOMK] {
            for keep in [0, 1] {
                let account = EngineAccounting::default().query();
                let (fixture, _) = subset_fixture(operator, keep, &account);
                assert_eq!(fixture.plan.emission_state(), (false, false));
                drop(fixture);
                account.snapshot().assert_memory_released();
            }
            for scenario in 0..13 {
                let account = EngineAccounting::default().query();
                let (mut fixture, mut sources) = subset_fixture(operator, 2, &account);
                assert_eq!(fixture.plan.emission_state(), (true, false));
                fixture.plan.emit_certified = false;
                let mut catalog_len = 12;
                match scenario {
                    0 | 1 | 11 => {
                        fixture.plan.bindings[0].canonical = Some(Canonical {
                            ordinal: if scenario == 1 { usize::MAX } else { 0 },
                            representative: if scenario == 11 { 3 } else { 0 },
                        });
                    }
                    2 => fixture.plan.bindings[3].canonical = None,
                    3 => sources[1] = sources[0],
                    4 => sources[0] = usize::MAX,
                    5 => catalog_len = 10,
                    6 => fixture.canonical[0].sources = 0..2,
                    7 => fixture.canonical[0].hash ^= 1,
                    8 => fixture.plan.inputs[0].kinds = 3,
                    9 => fixture.plan.projected_rows = None,
                    10 => fixture.canonical[0].ordinal = 1,
                    _ => {
                        fixture.plan.bindings.pop();
                    }
                }
                fixture
                    .plan
                    .certify_emit(
                        &fixture.canonical,
                        &sources,
                        catalog_len,
                        &Control::default(),
                        &account,
                    )
                    .unwrap();
                assert_eq!(
                    fixture.plan.emission_state(),
                    (false, false),
                    "scenario={scenario}"
                );
                drop(fixture);
                account.snapshot().assert_memory_released();
            }
        }
    }

    fn grouped_fixture(operator: TokenKind, groups: usize, account: &QueryAccounting) -> Fixture {
        let control = Control::default();
        let name = if operator == TokenKind::TOPK {
            "topk"
        } else {
            "bottomk"
        };
        let expression = fixture::expression(&format!("{name} by(region)(2,m)"), account, &control);
        let labels = |input: usize| {
            vec![
                (b"__name__".to_vec(), b"m".to_vec()),
                (b"id".to_vec(), input.to_string().into_bytes()),
                (
                    b"region".to_vec(),
                    (input % groups).to_string().into_bytes(),
                ),
            ]
        };
        let count = 3 * groups;
        let mut rows = fixture::rows((0..count).map(labels).collect(), account, &control);
        for (input, row) in rows.iter_mut().enumerate() {
            row.ordinal = 1 + 3 * input;
            row.kinds = 1;
        }
        let sources: Vec<_> = (0..count).rev().collect();
        let mut canonical = fixture::rows(
            sources.iter().copied().map(labels).collect(),
            account,
            &control,
        );
        for (ordinal, row) in canonical.iter_mut().enumerate() {
            row.ordinal = ordinal;
            row.kinds = 1;
            row.sources = ordinal..ordinal + 1;
        }
        let mut parameters = Parameters::new(true);
        parameters.push(2.0, account);
        let mut plan = Plan::new(operator, parameters, account);
        plan.prepare_rows(rows, &expression, expression.root, &control, account)
            .unwrap();
        plan.project(&canonical, &sources, &control, account)
            .unwrap();
        plan.certify_emit(&canonical, &sources, 3 * count, &control, account)
            .unwrap();
        assert_eq!(plan.emission_state(), (true, false));
        Fixture {
            plan,
            canonical,
            sources,
        }
    }

    fn grouped_value(plan: &Plan, input: usize) -> f64 {
        let value = (input + 1) as f64;
        if plan.operator == TokenKind::TOPK {
            value
        } else {
            -value
        }
    }

    fn grouped_step<'a>(fixture: &'a Fixture, clock: i64, account: &QueryAccounting) -> Step<'a> {
        Step::new(
            &fixture.plan,
            options(clock),
            clock / 1000,
            true,
            true,
            Some(&fixture.canonical),
            account,
        )
    }

    fn grouped_delivery(
        fixture: &Fixture,
        step: &mut Step<'_>,
        target: Option<usize>,
        mapped: bool,
        offset: f64,
        account: &QueryAccounting,
    ) -> (Vec<usize>, Vec<usize>) {
        let control = Control::default();
        let mut output = Vec::new();
        let mut requests = Vec::new();
        for _ in 0..100_000 {
            match step
                .advance(
                    target,
                    mapped.then_some(fixture.sources.as_slice()),
                    &control,
                    account,
                )
                .unwrap()
            {
                Action::Continue => (),
                Action::Child { parameter, ordinal } => {
                    assert!(!parameter);
                    let input = step
                        .plan
                        .inputs
                        .iter()
                        .position(|row| Some(&row.ordinal) == ordinal)
                        .unwrap();
                    let fresh = step.snapshot.is_some();
                    if fresh {
                        requests.push(input);
                    }
                    let mut sample = source(step.plan, input, false, false, account);
                    sample.value = OwnedValue::Float(
                        grouped_value(step.plan, input) + if fresh { offset } else { 0.0 },
                    );
                    step.receive(sample, &control, account).unwrap();
                }
                Action::Sample(sample) => {
                    let input = fixture.sources[sample.ordinal];
                    assert_eq!(
                        fixture::raw(&sample.labels),
                        fixture::raw(&fixture.plan.inputs[input].labels)
                    );
                    assert!(matches!(sample.labels, SampleLabels::Borrowed(owner)
                        if std::ptr::eq(owner, &fixture.plan.inputs[input].labels)));
                    assert_eq!(sample.drop_name, fixture.plan.inputs[input].drop_name);
                    assert_eq!(sample.source_time_ms, step.options.time_ms);
                    assert_eq!(sample.source_start_time_ms, 0);
                    let OwnedValue::Float(value) = sample.value else {
                        panic!("float winner")
                    };
                    assert_eq!(
                        value.to_bits(),
                        (grouped_value(step.plan, input) + offset).to_bits()
                    );
                    assert!(sample.float_samples.is_some());
                    output.push(sample.ordinal);
                }
                Action::Done(annotation) => {
                    assert!(annotation.is_none());
                    return (output, requests);
                }
            }
        }
        panic!("grouped ranking delivery exceeded finite progress guard");
    }

    #[test]
    fn grouped_ranking_winner_lookup_avoids_unrelated_heaps() {
        let mut all_work = Vec::new();
        for operator in [TokenKind::TOPK, TokenKind::BOTTOMK] {
            let mut work = Vec::new();
            for groups in [2, 4, 8, 16] {
                let account = EngineAccounting::default().query();
                let fixture = grouped_fixture(operator, groups, &account);
                let mut cold = grouped_step(&fixture, 0, &account);
                let (output, requests) =
                    grouped_delivery(&fixture, &mut cold, Some(1), true, 10.0, &account);
                let expected: Vec<_> = (0..2 * groups).collect();
                let inputs: Vec<_> = fixture.sources[..2 * groups].to_vec();
                assert_eq!(output, expected);
                assert_eq!(requests, inputs);
                assert_eq!(fixture.plan.emission_state(), (true, true));
                let before = account.snapshot().work;
                let mut total_requests = 0;
                for target in 0..3 * groups {
                    let mut warm = grouped_step(&fixture, 0, &account);
                    let (output, requests) = grouped_delivery(
                        &fixture,
                        &mut warm,
                        Some(target),
                        true,
                        100.0 + target as f64,
                        &account,
                    );
                    if target < 2 * groups {
                        assert_eq!(output, [target]);
                        assert_eq!(requests, [fixture.sources[target]]);
                    } else {
                        assert!(output.is_empty() && requests.is_empty());
                    }
                    total_requests += requests.len();
                }
                assert_eq!(total_requests, 2 * groups);
                let measured = account.snapshot().work - before;
                println!(
                    "{{\"ranking_groups\":\"{operator:?}\",\"groups\":{groups},\"warm_work\":{measured},\"fresh_requests\":{total_requests}}}"
                );
                work.push(measured);
                drop(cold);
                drop(fixture);
                account.snapshot().assert_memory_released();
            }
            all_work.push(work);
        }
        // Check growth only after every complete result, fresh request and release assertion.
        assert!(
            all_work.iter().all(|work| 2 * work[3] <= 5 * work[2]),
            "grouped ranking lookup must avoid unrelated heaps: {all_work:?}"
        );
    }

    #[test]
    fn grouped_ranking_routes_keep_partial_snapshots_and_fallback_targets() {
        for operator in [TokenKind::TOPK, TokenKind::BOTTOMK] {
            let account = EngineAccounting::default().query();
            let fixture = grouped_fixture(operator, 4, &account);
            let control = Control::default();
            let mut partial = grouped_step(&fixture, 0, &account);
            let first = loop {
                match partial
                    .advance(Some(0), Some(&fixture.sources), &control, &account)
                    .unwrap()
                {
                    Action::Continue => (),
                    Action::Child { parameter, ordinal } => {
                        assert!(!parameter);
                        let input = fixture
                            .plan
                            .inputs
                            .iter()
                            .position(|row| Some(&row.ordinal) == ordinal)
                            .unwrap();
                        let fresh = partial.snapshot.is_some();
                        let mut sample = source(&fixture.plan, input, false, false, &account);
                        sample.value = OwnedValue::Float(
                            grouped_value(&fixture.plan, input) + if fresh { 10.0 } else { 0.0 },
                        );
                        partial.receive(sample, &control, &account).unwrap();
                        if fresh {
                            break input;
                        }
                    }
                    _ => panic!("pause at the first pending fresh winner"),
                }
            };
            assert_eq!(first, fixture.sources[0]);
            let held = partial.snapshot.as_ref().unwrap().clone();
            assert!(!held.emitted.load(Ordering::Relaxed));
            let mut finisher = grouped_step(&fixture, 0, &account);
            assert!(std::ptr::eq::<Snapshot>(
                &**finisher.snapshot.as_ref().unwrap(),
                &*held
            ));
            assert_eq!(
                grouped_delivery(&fixture, &mut finisher, None, true, 20.0, &account).0,
                (0..8).collect::<Vec<_>>()
            );
            assert!(held.emitted.load(Ordering::Relaxed));
            let mut next_clock = grouped_step(&fixture, 1000, &account);
            grouped_delivery(&fixture, &mut next_clock, None, true, 30.0, &account);
            assert!(!std::ptr::eq::<Snapshot>(
                &**next_clock.snapshot.as_ref().unwrap(),
                &*held
            ));
            let resumed = grouped_delivery(&fixture, &mut partial, Some(0), true, 10.0, &account);
            assert_eq!(resumed, (vec![0], vec![]));
            for target in [0, 4, 7, 8, 11, 12, usize::MAX] {
                let mut mapped = grouped_step(&fixture, 1000, &account);
                let actual =
                    grouped_delivery(&fixture, &mut mapped, Some(target), true, 40.0, &account);
                let mut ordinary = grouped_step(&fixture, 1000, &account);
                assert_eq!(
                    actual,
                    grouped_delivery(&fixture, &mut ordinary, Some(target), false, 40.0, &account)
                );
                if target < 8 {
                    assert_eq!(actual, (vec![target], vec![fixture.sources[target]]));
                } else {
                    assert_eq!(actual, (vec![], vec![]));
                }
            }
            let mut backward = grouped_step(&fixture, 1000, &account);
            backward.emit_row = 6;
            assert_eq!(
                grouped_delivery(&fixture, &mut backward, Some(0), true, 50.0, &account),
                (vec![], vec![])
            );
            drop((partial, finisher, next_clock, backward, held));
            drop(fixture);
            account.snapshot().assert_memory_released();
        }
    }

    struct Collected<'a> {
        samples: Vec<EvaluatedSample<'a>>,
        acquired: u64,
        exact_borrows: bool,
        requests: usize,
    }

    fn collect<'a>(
        fixture: &'a Fixture,
        project: bool,
        vector: bool,
        histogram: bool,
        clock: i64,
        flip: bool,
        account: &QueryAccounting,
    ) -> Collected<'a> {
        let control = Control::default();
        let plan = &fixture.plan;
        let mut step = Step::new(
            plan,
            options(clock),
            clock / 1000,
            project,
            vector,
            Some(&fixture.canonical),
            account,
        );
        let mut collected = Collected {
            samples: vec![],
            acquired: 0,
            exact_borrows: true,
            requests: 0,
        };
        loop {
            let input = step.expected;
            let before = account.snapshot();
            match step
                .advance(None, Some(&fixture.sources), &control, account)
                .unwrap()
            {
                Action::Continue => (),
                Action::Child { parameter, ordinal } => {
                    assert!(!parameter);
                    let index = plan
                        .inputs
                        .iter()
                        .position(|row| Some(&row.ordinal) == ordinal)
                        .unwrap();
                    collected.requests += 1;
                    step.receive(
                        source(plan, index, histogram, flip, account),
                        &control,
                        account,
                    )
                    .unwrap();
                }
                Action::Sample(sample) => {
                    let index = input.expect("fresh selected input");
                    let representative = &plan.inputs[if project { 0 } else { index }];
                    assert_eq!(
                        fixture::raw(&sample.labels),
                        fixture::raw(&representative.labels)
                    );
                    assert_eq!(sample.ordinal, if project { 0 } else { index });
                    assert_eq!(
                        sample.drop_name,
                        if project {
                            representative.drop_name
                        } else {
                            !representative.drop_name
                        }
                    );
                    assert_eq!(
                        (sample.source_time_ms, sample.source_start_time_ms),
                        (clock, 0)
                    );
                    assert_eq!(sample.float_samples.is_some(), !histogram);
                    match &sample.value {
                        OwnedValue::Float(actual) => {
                            assert_eq!(actual.to_bits(), value(index, flip).to_bits())
                        }
                        OwnedValue::Histogram(actual) => {
                            assert_eq!(actual.as_ref().sum.to_bits(), value(index, flip).to_bits());
                            assert_eq!(actual.as_ref().count, 1.0);
                        }
                    }
                    collected.acquired += account
                        .snapshot()
                        .memory
                        .iter()
                        .zip(before.memory)
                        .map(|(new, old)| new.acquired - old.acquired)
                        .sum::<u64>();
                    collected.exact_borrows &= matches!(sample.labels,
                        SampleLabels::Borrowed(owner) if std::ptr::eq(owner, &representative.labels));
                    collected.samples.push(sample);
                }
                Action::Done(ignored) => {
                    assert!(ignored.is_none());
                    break;
                }
            }
        }
        collected
    }

    #[test]
    fn selection_emission_preserves_canonical_metadata_before_copy_guard() {
        let mut costs = vec![];
        for operator in [
            TokenKind::TOPK,
            TokenKind::BOTTOMK,
            TokenKind::LIMITK,
            TokenKind::LIMIT_RATIO,
        ] {
            for width in [None, Some(0), Some(13), Some(16), Some(4096)] {
                for project in [false, true] {
                    for vector in [false, true] {
                        for histogram in [false, true] {
                            if histogram && matches!(operator, TokenKind::TOPK | TokenKind::BOTTOMK)
                            {
                                continue;
                            }
                            let account = EngineAccounting::default().query();
                            let fixture = Fixture::new(operator, width, &account);
                            let output =
                                collect(&fixture, project, vector, histogram, 0, false, &account);
                            let count = if matches!(operator, TokenKind::TOPK | TokenKind::BOTTOMK)
                                || project && vector
                            {
                                1
                            } else {
                                2
                            };
                            assert_eq!(output.samples.len(), count);
                            assert!(output.requests > output.samples.len());
                            costs.push((
                                operator,
                                width,
                                project,
                                vector,
                                histogram,
                                output.acquired,
                                output.exact_borrows,
                            ));
                            drop(output);
                            drop(fixture);
                            account.snapshot().assert_memory_released();
                        }
                    }
                }
            }
        }
        println!(
            "complete selection emission scenarios={} {costs:?}",
            costs.len()
        );
        assert_eq!(costs.len(), 120);
        assert!(
            costs.iter().all(|case| case.5 == 0 && case.6),
            "canonical emission must borrow exact raw owners without allocation"
        );
    }

    #[test]
    fn selection_emission_keeps_snapshots_fresh_values_and_independent_mutation() {
        let account = EngineAccounting::default().query();
        let fixture = Fixture::new(TokenKind::TOPK, Some(4096), &account);
        let original = fixture::raw(&fixture.plan.inputs[0].labels);
        let mut cold = collect(&fixture, true, true, false, 0, false, &account);
        let cached = collect(&fixture, true, true, false, 0, true, &account);
        let next = collect(&fixture, true, true, false, 1000, true, &account);
        assert_eq!(
            (cold.samples.len(), cached.samples.len(), next.samples.len()),
            (1, 1, 1)
        );
        assert!(cold.requests > cached.requests && next.requests > cached.requests);
        let bits = |output: &Collected<'_>| match output.samples[0].value {
            OwnedValue::Float(value) => value.to_bits(),
            _ => unreachable!(),
        };
        assert_eq!(
            (bits(&cold), bits(&cached), bits(&next)),
            (2.0f64.to_bits(), 1.0f64.to_bits(), 2.0f64.to_bits())
        );
        assert_eq!(fixture::raw(&cold.samples[0].labels), original);
        cold.samples[0]
            .labels
            .make_owned(&account, &Control::default())
            .unwrap()
            .set(
                b"a",
                b"changed",
                &account,
                MemoryKind::Workspace,
                &Control::default(),
            )
            .unwrap();
        assert_eq!(fixture::raw(&fixture.plan.inputs[0].labels), original);
        assert_eq!(fixture::raw(&cached.samples[0].labels), original);
        assert_eq!(fixture::raw(&next.samples[0].labels), original);
        drop((cold, cached, next));
        drop(fixture);
        account.snapshot().assert_memory_released();
    }

    fn ready_emit<'a>(fixture: &'a Fixture, account: &QueryAccounting) -> Step<'a> {
        let plan = &fixture.plan;
        let control = Control::default();
        let mut step = Step::new(
            plan,
            options(0),
            0,
            true,
            true,
            Some(&fixture.canonical),
            account,
        );
        loop {
            match step
                .advance(None, Some(&fixture.sources), &control, account)
                .unwrap()
            {
                Action::Continue => (),
                Action::Child { parameter, ordinal } => {
                    assert!(!parameter);
                    let index = plan
                        .inputs
                        .iter()
                        .position(|row| Some(&row.ordinal) == ordinal)
                        .unwrap();
                    step.receive(
                        source(plan, index, false, false, account),
                        &control,
                        account,
                    )
                    .unwrap();
                    if matches!(step.stage, Stage::Emit) {
                        return step;
                    }
                }
                _ => panic!("selection must request its fresh winner before emission"),
            }
        }
    }

    #[test]
    fn selection_emission_checks_the_replacement_boundary_after_head_and_replay_errors() {
        for scenario in 0..4 {
            let account = EngineAccounting::default().query();
            let mut fixture = Fixture::new(TokenKind::TOPK, Some(4096), &account);
            if scenario == 3 {
                let mut counts = PointCounts::default();
                counts.push(1);
                counts.push(1);
                fixture.canonical[0].heads.note(counts, -1);
            }
            let mut step = ready_emit(&fixture, &account);
            if scenario == 1 {
                step.emitted = None;
            } else if scenario == 2 {
                step.emitted.as_mut().unwrap().value =
                    source(&fixture.plan, 1, true, false, &account).value;
                step.emitted.as_mut().unwrap().float_samples = None;
            }
            let control = Control::default();
            let before = account.snapshot();
            let result = crate::test_storage::interrupt_after_work(&account, &control, 1, || {
                step.advance(None, Some(&fixture.sources), &control, &account)
            });
            assert_eq!(account.snapshot().work - before.work, 1);
            match (scenario, result) {
                (0, Err(EvaluationError::Interrupted(Interrupted::Cancelled))) => (),
                (
                    1 | 2,
                    Err(EvaluationError::Invalid("selection winner changed during replay")),
                ) => (),
                (3, Ok(Action::Continue)) => (),
                _ => {
                    panic!("replacement boundary changed replay/head/error precedence: {scenario}")
                }
            }
            assert!(
                account
                    .snapshot()
                    .memory
                    .iter()
                    .zip(before.memory)
                    .all(|(new, old)| new.acquired == old.acquired)
            );
            drop(step);
            drop(fixture);
            account.snapshot().assert_memory_released();
        }
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use crate::{accounting::EngineAccounting, test_storage::group_preparation as fixture};

    #[test]
    fn preparation_preserves_full_inputs_before_discarded_key_allocation_guard() {
        let mut costs = Vec::new();
        for (operator, name) in [
            (TokenKind::TOPK, "topk"),
            (TokenKind::BOTTOMK, "bottomk"),
            (TokenKind::LIMITK, "limitk"),
            (TokenKind::LIMIT_RATIO, "limit_ratio"),
        ] {
            for count in [1, 32] {
                for unique in [false, true] {
                    for fields in [4, 63, 64, 65, 128] {
                        for without in [false, true] {
                            for width in [16, 4096] {
                                let engine = EngineAccounting::default();
                                let account = engine.query();
                                let control = Control::default();
                                let expression = fixture::expression(
                                    &fixture::query(name, fields, without),
                                    &account,
                                    &control,
                                );
                                let rows = fixture::population(
                                    count, width, fields, unique, &account, &control,
                                );
                                let expected = fixture::identities(&rows);
                                let mut parameters = Parameters::new(true);
                                parameters.push(2.0, &account);
                                let mut plan = Plan::new(operator, parameters, &account);
                                let before = account.snapshot();
                                plan.prepare_rows(
                                    rows,
                                    &expression,
                                    expression.root,
                                    &control,
                                    &account,
                                )
                                .unwrap();
                                let after = account.snapshot();
                                assert_eq!(fixture::identities(&plan.inputs), expected);
                                assert_eq!(plan.groups, if unique { count } else { 1 });
                                assert_eq!(
                                    plan.bindings
                                        .iter()
                                        .map(|binding| binding.group)
                                        .collect::<Vec<_>>(),
                                    (0..count)
                                        .map(|position| if unique { position } else { 0 })
                                        .collect::<Vec<_>>()
                                );
                                assert!(
                                    plan.bindings
                                        .iter()
                                        .all(|binding| binding.canonical.is_none())
                                );
                                assert!(
                                    plan.productive
                                        && plan.projected_rows.is_none()
                                        && plan.cache.borrow().is_none()
                                );
                                let acquired = after.memory[MemoryKind::Workspace as usize]
                                    .acquired
                                    - before.memory[MemoryKind::Workspace as usize].acquired;
                                assert!(after.memory.iter().all(|memory| !memory.overflowed));
                                costs.push((name, count, unique, fields, without, width, acquired));
                                eprintln!(
                                    "ranking-preparation operator={name} count={count} unique={unique} fields={fields} without={without} width={width} acquired={acquired}"
                                );
                                drop((plan, expression));
                                account.snapshot().assert_memory_released();
                                drop(account);
                                engine.snapshot().assert_memory_released();
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(costs.len(), 320);
        eprintln!("complete ranking-preparation constructor controls:320");
        for pair in costs.chunks_exact(2) {
            assert_eq!(
                pair[0].6, pair[1].6,
                "discarded ranking keys must not acquire selected-value width: {:?}",
                pair
            );
        }
    }

    #[test]
    fn preparation_preserves_raw_hash_groups_and_position_bindings() {
        for case in 0..6 {
            for reverse in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let (rows, groups, bindings, grouping) =
                    fixture::raw_case(case, reverse, &account, &control);
                let expected = fixture::identities(&rows);
                let expression =
                    fixture::expression(&format!("topk {grouping}(2,m)"), &account, &control);
                let mut parameters = Parameters::new(true);
                parameters.push(2.0, &account);
                let mut plan = Plan::new(TokenKind::TOPK, parameters, &account);
                plan.prepare_rows(rows, &expression, expression.root, &control, &account)
                    .unwrap();
                assert_eq!(fixture::identities(&plan.inputs), expected);
                assert_eq!(plan.groups, groups.len());
                assert_eq!(
                    plan.bindings
                        .iter()
                        .map(|binding| binding.group)
                        .collect::<Vec<_>>(),
                    bindings
                );
                assert!(
                    plan.bindings
                        .iter()
                        .all(|binding| binding.canonical.is_none())
                );
                drop((plan, expression));
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }
    #[test]
    fn preparation_checks_hash_before_parameter_errors_and_releases_consumed_rows() {
        for parameter in [0.0, f64::NAN, f64::INFINITY] {
            for boundary in 0..4 {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let expression = fixture::expression("topk by(job)(2,m)", &account, &control);
                let mut parameters = Parameters::new(true);
                parameters.push(parameter, &account);
                let mut plan = Plan::new(TokenKind::TOPK, parameters, &account);
                let inputless = account.snapshot().memory[MemoryKind::Workspace as usize].current;
                let rows = fixture::population(8, 65_536, 4, false, &account, &control);
                let Expr::Aggregate {
                    grouping, without, ..
                } = &expression.arena[expression.root].expression
                else {
                    unreachable!()
                };
                let start = account.snapshot().work;
                group_hash(
                    &rows[0].labels,
                    grouping.as_deref(),
                    *without,
                    &control,
                    &account,
                )
                .unwrap();
                let hash_work = account.snapshot().work - start;
                let before = account.snapshot();
                let operation =
                    || plan.prepare_rows(rows, &expression, expression.root, &control, &account);
                let result = match boundary {
                    0 => operation(),
                    1 => {
                        crate::test_storage::interrupt_after_work(&account, &control, 1, operation)
                    }
                    2 => crate::test_storage::interrupt_after_work(
                        &account, &control, 8192, operation,
                    ),
                    _ => crate::test_storage::interrupt_after_work(
                        &account,
                        &control,
                        1 + hash_work,
                        operation,
                    ),
                };
                if boundary != 0 {
                    assert_eq!(
                        result,
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    );
                    assert!(plan.inputs.is_empty() && plan.bindings.is_empty() && plan.groups == 0);
                    assert_eq!(
                        account.snapshot().memory[MemoryKind::Workspace as usize].current,
                        inputless
                    );
                    assert_eq!(
                        account.snapshot().memory[MemoryKind::Workspace as usize].acquired,
                        before.memory[MemoryKind::Workspace as usize].acquired
                    );
                } else if parameter == 0.0 {
                    result.unwrap();
                    assert!(!plan.productive);
                    assert_eq!(plan.inputs.len(), 8);
                    assert_eq!(plan.bindings.len(), 8);
                    assert_eq!(plan.groups, 1);
                    assert!(account.snapshot().work - before.work >= 8 * hash_work);
                } else {
                    let Err(EvaluationError::Runtime(message)) = &result else {
                        panic!("parameter error after grouping: {result:?}")
                    };
                    assert_eq!(
                        &**message,
                        if parameter.is_nan() {
                            b"Parameter value is NaN".as_slice()
                        } else {
                            b"Scalar value +Inf overflows int64".as_slice()
                        }
                    );
                    assert!(account.snapshot().work - before.work >= 8 * hash_work);
                    assert!(plan.inputs.is_empty() && plan.bindings.is_empty() && plan.groups == 0);
                    drop(result);
                    assert_eq!(
                        account.snapshot().memory[MemoryKind::Workspace as usize].current,
                        inputless
                    );
                }
                drop((plan, expression));
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn preparation_distinguishes_empty_rows_from_empty_labelsets() {
        for count in [0, 1] {
            for cancelled in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let expression = fixture::expression("topk by(missing)(2,m)", &account, &control);
                let rows = fixture::rows(vec![vec![]; count], &account, &control);
                let mut parameters = Parameters::new(true);
                parameters.push(2.0, &account);
                let mut plan = Plan::new(TokenKind::TOPK, parameters, &account);
                if cancelled {
                    control.cancellation.cancel();
                }
                let result =
                    plan.prepare_rows(rows, &expression, expression.root, &control, &account);
                if cancelled && count != 0 {
                    assert_eq!(
                        result,
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    );
                    assert!(plan.inputs.is_empty() && plan.bindings.is_empty());
                } else {
                    result.unwrap();
                    assert_eq!(plan.groups, count);
                    assert_eq!(plan.inputs.len(), count);
                    assert_eq!(plan.bindings.len(), count);
                }
                drop((plan, expression));
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }
}
