// SPDX-License-Identifier: GPL-3.0-or-later
// Sort semantics derive from pinned PromQL, Go sort and facette/natsort; see NOTICE.
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    binary_labels,
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_roster::Descriptor,
    frontend::ast::{Expr, NodeId, ParsedExpression},
    go_sort_program::{Operation, Program, physical_index},
    position_index::PositionIndex,
    query_preparation::Prepared,
    result::{OwnedLabels, OwnedValue},
    shared::Strong,
    storage::{Control, Interrupted},
};
use std::{
    cell::RefCell,
    cmp::Ordering,
    sync::atomic::{AtomicUsize, Ordering as AtomicOrdering},
};

#[derive(Clone, Copy)]
enum Function {
    Numeric(bool),
    Labels(bool),
}
impl Function {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "sort" => Self::Numeric(false),
            "sort_desc" => Self::Numeric(true),
            "sort_by_label" => Self::Labels(false),
            "sort_by_label_desc" => Self::Labels(true),
            _ => return None,
        })
    }
}
pub(crate) fn is_sort(expression: &Expr) -> bool {
    matches!(expression, Expr::Call { function: Some(function), .. }
        if Function::parse(function.name).is_some())
}

pub(crate) struct Plan {
    rows: AccountedVec<Descriptor>,
    labels: AccountedVec<NodeId>,
    function: Function,
    pub(crate) singleton: bool,
    emit_certified: bool,
    canonical: Option<AccountedVec<Option<usize>>>,
    cache: RefCell<Option<Strong<Snapshot>>>,
}
impl Plan {
    #[cfg(test)]
    pub(crate) fn emission_state(&self) -> (bool, bool) {
        (
            self.emit_certified,
            self.cache
                .borrow()
                .as_ref()
                .is_some_and(|snapshot| snapshot.fully_emitted()),
        )
    }
    #[cfg(test)]
    pub(crate) fn emission_progress(&self) -> Option<usize> {
        self.cache
            .borrow()
            .as_ref()
            .map(|snapshot| snapshot.emitted.load(AtomicOrdering::Relaxed))
    }
    pub(crate) fn clear_emission(&self) {
        if let Some(snapshot) = &*self.cache.borrow() {
            snapshot.emitted.store(0, AtomicOrdering::Relaxed);
        }
    }
    pub(crate) fn new(
        rows: AccountedVec<Descriptor>,
        expression: &ParsedExpression,
        prepared: &Prepared,
        node: NodeId,
        singleton: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, Interrupted> {
        let Expr::Call {
            function: Some(function),
            arguments,
        } = &expression.arena[node].expression
        else {
            unreachable!("typed sorting call")
        };
        let mut labels = AccountedVec::new(account, MemoryKind::Workspace);
        for &argument in &arguments[1..] {
            control.check()?;
            account.work(1);
            labels.push(
                prepared
                    .preprocessing
                    .child(expression, node, argument, control, account)?,
            );
        }
        Ok(Self {
            rows,
            labels,
            function: Function::parse(function.name).expect("sorting function"),
            singleton,
            emit_certified: false,
            canonical: None,
            cache: RefCell::new(None),
        })
    }
    pub(crate) fn project(
        &mut self,
        descriptors: &[Descriptor],
        sources: &[usize],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let mut canonical = AccountedVec::new(account, MemoryKind::Workspace);
        for _ in &*self.rows {
            control.check()?;
            account.work(1);
            canonical.push(None);
        }
        for descriptor in descriptors {
            for &source in &sources[descriptor.sources.clone()] {
                control.check()?;
                account.work(1);
                *canonical.get_mut(source).ok_or(EvaluationError::Invalid(
                    "sort projection source is missing",
                ))? = Some(descriptor.ordinal);
            }
        }
        self.canonical = Some(canonical);
        Ok(())
    }
    pub(crate) fn certify_emit(
        &mut self,
        descriptors: &[Descriptor],
        sources: &[usize],
        catalog_len: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let Some(canonical) = &self.canonical else {
            return Ok(());
        };
        if self.rows.len() < 2
            || descriptors.len() != self.rows.len()
            || canonical.len() != self.rows.len()
        {
            return Ok(());
        }
        let mut prior = None;
        for row in &*self.rows {
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
            let Some(row) = self.rows.get(input) else {
                return Ok(());
            };
            if canonical[input] != Some(descriptor.ordinal) || row.hash != descriptor.hash {
                return Ok(());
            }
        }
        control.check()?;
        self.emit_certified = true;
        Ok(())
    }
    fn row(
        &self,
        sample: &EvaluatedSample,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<usize, EvaluationError> {
        let (mut left, mut right) = (0, self.rows.len());
        while left < right {
            control.check()?;
            account.work(1);
            let middle = left + (right - left) / 2;
            if self.rows[middle].ordinal < sample.ordinal {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        for (position, row) in self.rows.iter().enumerate().skip(left) {
            control.check()?;
            account.work(1);
            if row.ordinal != sample.ordinal {
                break;
            }
            if binary_labels::same(&row.labels, &sample.labels, control, account)? {
                return Ok(position);
            }
        }
        Err(EvaluationError::Invalid("sort child replay row is missing"))
    }
    fn validate_strings(
        &self,
        expression: &ParsedExpression,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        for &node in &*self.labels {
            control.check()?;
            account.work(1);
            if !matches!(expression.arena[node].expression, Expr::String(_)) {
                return Err(crate::label_functions::string_literal_error(
                    control, account,
                )?);
            }
        }
        Ok(())
    }
    fn compare_labels(
        &self,
        expression: &ParsedExpression,
        left: usize,
        right: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Ordering, Interrupted> {
        let Function::Labels(descending) = self.function else {
            unreachable!()
        };
        let left = &self.rows[left].labels;
        let right = &self.rows[right].labels;
        for &node in &*self.labels {
            control.check()?;
            account.work(1);
            let Expr::String(name) = &expression.arena[node].expression else {
                unreachable!("validated sort label")
            };
            let a = left.get_checked(name, account, control)?;
            let b = right.get_checked(name, account, control)?;
            if binary_labels::equal(a, b, control, account)? {
                continue;
            }
            let before = natural_before(a, b, control, account)?;
            return Ok(if before != descending {
                Ordering::Less
            } else {
                Ordering::Greater
            });
        }
        let order = compare_full_labels(left, right, control, account)?;
        Ok(if descending { order.reverse() } else { order })
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
struct Snapshot {
    key: Key,
    indices: AccountedVec<usize>,
    // Full Done is distinct from a prefix ending at the final sample.
    emitted: AtomicUsize,
}
impl Snapshot {
    #[cfg(test)]
    fn fully_emitted(&self) -> bool {
        self.emitted.load(AtomicOrdering::Relaxed) == usize::MAX
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Target {
    Complete(usize),
    Prefix(usize),
}
impl Target {
    fn input(self) -> usize {
        match self {
            Self::Complete(input) | Self::Prefix(input) => input,
        }
    }
}
#[derive(Clone, Copy)]
enum Stage {
    Scan,
    Sort,
    Program,
    Emit,
    Done,
}
struct Comparison {
    inputs: [usize; 2],
    values: [f64; 2],
    position: usize,
    samples: [Option<crate::statistics::OwnedSamples>; 2],
}
pub(crate) enum Action<'a> {
    Continue,
    Child(Option<&'a usize>),
    Sample(EvaluatedSample),
    Done,
}
pub(crate) struct Step<'a> {
    plan: &'a Plan,
    options: EvaluationOptions,
    step_index: i64,
    project: bool,
    stage: Stage,
    indices: AccountedVec<usize>,
    snapshot: Option<Strong<Snapshot>>,
    program: Option<Program>,
    comparison: Option<Comparison>,
    reply: Option<bool>,
    waiting: bool,
    expected: Option<usize>,
    value: Option<f64>,
    value_samples: Option<crate::statistics::OwnedSamples>,
    emitted: Option<EvaluatedSample>,
    position: usize,
    range_warning: bool,
}
impl<'a> Step<'a> {
    pub(crate) fn new(
        plan: &'a Plan,
        options: EvaluationOptions,
        step_index: i64,
        project: bool,
        account: &QueryAccounting,
    ) -> Self {
        let snapshot = {
            let mut cache = plan.cache.borrow_mut();
            if cache
                .as_ref()
                .is_some_and(|snapshot| snapshot.key != (options, step_index).into())
            {
                *cache = None;
            }
            cache.as_ref().cloned()
        };
        let stage = if snapshot.is_some() {
            Stage::Emit
        } else {
            Stage::Scan
        };
        Self {
            plan,
            options,
            step_index,
            project,
            stage,
            indices: AccountedVec::new(account, MemoryKind::Workspace),
            snapshot,
            program: None,
            comparison: None,
            reply: None,
            waiting: false,
            expected: None,
            value: None,
            value_samples: None,
            emitted: None,
            position: 0,
            range_warning: false,
        }
    }
    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        control.check()?;
        account.work(1);
        if matches!(self.stage, Stage::Scan) {
            if matches!(self.plan.function, Function::Labels(_))
                || matches!(sample.value, OwnedValue::Float(_))
            {
                self.indices.push(self.plan.row(&sample, control, account)?);
            }
            return Ok(());
        }
        let row = &self.plan.rows[self.expected.expect("requested sort operand")];
        if row.ordinal != sample.ordinal
            || !binary_labels::same(&row.labels, &sample.labels, control, account)?
        {
            return Ok(());
        }
        if matches!(self.stage, Stage::Emit) {
            if self.emitted.is_none() {
                self.emitted = Some(sample);
            }
        } else if self.value.is_none()
            && let OwnedValue::Float(value) = sample.value
        {
            self.value = Some(value);
            self.value_samples = sample.float_samples;
        }
        Ok(())
    }
    fn request(&mut self, input: usize) -> Action<'a> {
        self.waiting = true;
        self.expected = Some(input);
        self.value = None;
        self.value_samples = None;
        self.emitted = None;
        Action::Child(Some(&self.plan.rows[input].ordinal))
    }
    fn publish(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        self.range_warning = self.options.start_ms != self.options.end_ms;
        // The complete callback runs before the source's duplicate-label check.
        if !self.options.delayed_name_removal || self.options.start_ms != self.options.end_ms {
            self.validate_hashes(control, account)?;
        }
        control.check()?;
        let indices = std::mem::replace(
            &mut self.indices,
            AccountedVec::new(account, MemoryKind::Workspace),
        );
        let snapshot = Strong::new_in(
            Snapshot {
                key: (self.options, self.step_index).into(),
                indices,
                emitted: AtomicUsize::new(0),
            },
            Some(&AllocationContext::new(account, MemoryKind::Workspace)),
        );
        *self.plan.cache.borrow_mut() = Some(snapshot.clone());
        self.snapshot = Some(snapshot);
        self.stage = Stage::Emit;
        Ok(())
    }
    fn validate_hashes(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        if self.indices.len() <= 32 {
            for (position, &input) in self.indices.iter().enumerate() {
                for &prior in &self.indices[..position] {
                    control.check()?;
                    account.work(1);
                    if self.plan.rows[input].hash == self.plan.rows[prior].hash {
                        return Err(EvaluationError::Invalid(
                            "vector cannot contain metrics with the same labelset",
                        ));
                    }
                }
            }
        } else {
            let mut seen = PositionIndex::new(account);
            for &input in &*self.indices {
                control.check()?;
                account.work(1);
                let hash = self.plan.rows[input].hash;
                if seen
                    .find(
                        hash,
                        |prior| Ok(self.plan.rows[prior].hash == hash),
                        control,
                        account,
                    )?
                    .is_some()
                {
                    return Err(EvaluationError::Invalid(
                        "vector cannot contain metrics with the same labelset",
                    ));
                }
                control.check()?;
                seen.insert_new(
                    hash,
                    input,
                    |prior| Ok(self.plan.rows[prior].hash),
                    control,
                    account,
                )?;
                control.check()?;
            }
        }
        Ok(())
    }
    pub(crate) fn take_range_warning(&mut self) -> bool {
        std::mem::take(&mut self.range_warning)
    }
    pub(crate) fn complete_prefix(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if !self.plan.emit_certified || self.project {
            return Ok(());
        }
        control.check()?;
        account.work(1);
        control.check()?;
        let snapshot = self.snapshot.as_ref().expect("sorted snapshot");
        let prior = snapshot.emitted.load(AtomicOrdering::Relaxed);
        // Plan deliveries are serial; a suspended owner must never revoke later credit.
        if self.position > prior {
            snapshot
                .emitted
                .store(self.position, AtomicOrdering::Relaxed);
        }
        Ok(())
    }
    pub(crate) fn advance(
        &mut self,
        expression: &ParsedExpression,
        target: Option<Target>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action<'a>, EvaluationError> {
        control.check()?;
        account.work(1);
        match self.stage {
            Stage::Scan => {
                if !self.waiting {
                    self.waiting = true;
                    return Ok(Action::Child(None));
                }
                self.waiting = false;
                self.plan.validate_strings(expression, control, account)?;
                self.stage = Stage::Sort;
            }
            Stage::Sort => match self.plan.function {
                Function::Labels(_) => {
                    crate::go_sort::try_sort_by_with(
                        &mut self.indices,
                        |a, b| {
                            self.plan
                                .compare_labels(expression, *a, *b, control, account)
                        },
                        || {
                            control.check()?;
                            account.work(1);
                            Ok::<_, Interrupted>(())
                        },
                    )?;
                    self.publish(control, account)?;
                }
                Function::Numeric(_) => {
                    self.program = Some(Program::sort(self.indices.len() as u64));
                    self.stage = Stage::Program;
                }
            },
            Stage::Program => {
                if let Some(comparison) = &mut self.comparison {
                    if self.waiting {
                        self.waiting = false;
                        comparison.values[comparison.position] = self.value.take().ok_or(
                            EvaluationError::Invalid("sort operand changed during replay"),
                        )?;
                        comparison.samples[comparison.position] = self.value_samples.take();
                        comparison.position += 1;
                    }
                    if comparison.position < 2 {
                        let input = comparison.inputs[comparison.position];
                        return Ok(self.request(input));
                    }
                    let Function::Numeric(descending) = self.plan.function else {
                        unreachable!()
                    };
                    let [left, right] = comparison.values;
                    self.reply = Some(
                        right.is_nan()
                            || if descending {
                                right < left
                            } else {
                                right > left
                            },
                    );
                    self.comparison = None;
                }
                match self
                    .program
                    .as_mut()
                    .expect("sort program")
                    .next(self.reply.take())
                {
                    Operation::Step => (),
                    Operation::Less(a, b) => {
                        let (a, b) = (
                            physical_index(a, self.indices.len()),
                            physical_index(b, self.indices.len()),
                        );
                        self.comparison = Some(Comparison {
                            inputs: [self.indices[a], self.indices[b]],
                            values: [0.0; 2],
                            position: 0,
                            samples: [None, None],
                        })
                    }
                    Operation::Swap(a, b) => {
                        let (a, b) = (
                            physical_index(a, self.indices.len()),
                            physical_index(b, self.indices.len()),
                        );
                        self.indices.swap(a, b);
                    }
                    Operation::Done => {
                        self.program = None;
                        self.publish(control, account)?;
                    }
                }
            }
            Stage::Emit => {
                if self.waiting {
                    self.waiting = false;
                    let input = self.expected.expect("requested sorted row");
                    let mut sample = self
                        .emitted
                        .take()
                        .ok_or(EvaluationError::Invalid("sort row changed during replay"))?;
                    if matches!(self.plan.function, Function::Numeric(_))
                        && !matches!(sample.value, OwnedValue::Float(_))
                    {
                        return Err(EvaluationError::Invalid("sort row changed during replay"));
                    }
                    sample.ordinal = if self.project
                        && let Some(canonical) = &self.plan.canonical
                    {
                        canonical[input]
                            .ok_or(EvaluationError::Invalid("sort canonical row is missing"))?
                    } else {
                        input
                    };
                    sample.source_time_ms = self.options.time_ms;
                    sample.source_start_time_ms = 0;
                    self.position += 1;
                    return Ok(Action::Sample(sample));
                }
                let target = target.filter(|_| self.plan.emit_certified && !self.project);
                if let Some(&input) = self
                    .snapshot
                    .as_ref()
                    .expect("sorted snapshot")
                    .indices
                    .get(self.position)
                {
                    if let Some(target) = target
                        && target.input() != input
                    {
                        let progress = self
                            .snapshot
                            .as_ref()
                            .expect("sorted snapshot")
                            .emitted
                            .load(AtomicOrdering::Relaxed);
                        if progress == usize::MAX
                            || matches!(target, Target::Prefix(_)) && self.position < progress
                        {
                            self.position += 1;
                            return Ok(Action::Continue);
                        }
                    }
                    return Ok(self.request(input));
                }
                self.stage = Stage::Done;
            }
            Stage::Done => {
                if target.is_some() && self.plan.emit_certified && !self.project {
                    control.check()?;
                    // Order alone is insufficient: the original full delivery must succeed.
                    self.snapshot
                        .as_ref()
                        .expect("sorted snapshot")
                        .emitted
                        .store(usize::MAX, AtomicOrdering::Relaxed);
                }
                return Ok(Action::Done);
            }
        }
        Ok(Action::Continue)
    }
}

fn compare_full_labels(
    left: &OwnedLabels,
    right: &OwnedLabels,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Ordering, Interrupted> {
    for (a, b) in left.0.iter().zip(right.0.iter()) {
        control.check()?;
        account.work(1);
        let order = binary_labels::compare(&a.name, &b.name, control, account)?;
        if !order.is_eq() {
            return Ok(order);
        }
        let order = binary_labels::compare(&a.value, &b.value, control, account)?;
        if !order.is_eq() {
            return Ok(order);
        }
    }
    Ok(left.0.len().cmp(&right.0.len()))
}
fn chunk_end(
    value: &[u8],
    start: usize,
    control: &Control,
    account: &QueryAccounting,
) -> Result<usize, Interrupted> {
    let digits = value[start].is_ascii_digit();
    let mut end = start;
    while end < value.len() && value[end].is_ascii_digit() == digits {
        control.check()?;
        account.work(1);
        end += 1;
    }
    Ok(end)
}
fn chunk_integer(
    value: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<i64>, Interrupted> {
    let mut number = 0_i64;
    for &byte in value {
        control.check()?;
        account.work(1);
        if !byte.is_ascii_digit() {
            return Ok(None);
        }
        let Some(next) = number
            .checked_mul(10)
            .and_then(|n| n.checked_add(i64::from(byte - b'0')))
        else {
            return Ok(None);
        };
        number = next;
    }
    Ok(Some(number))
}
fn natural_before(
    a: &[u8],
    b: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    let (mut left, mut right) = (0, 0);
    while left < a.len() {
        control.check()?;
        account.work(1);
        if right == b.len() {
            return Ok(false);
        }
        let next_left = chunk_end(a, left, control, account)?;
        let next_right = chunk_end(b, right, control, account)?;
        let a_chunk = &a[left..next_left];
        let b_chunk = &b[right..next_right];
        let order = match (
            chunk_integer(a_chunk, control, account)?,
            chunk_integer(b_chunk, control, account)?,
        ) {
            (Some(a), Some(b)) => a.cmp(&b),
            _ => binary_labels::compare(a_chunk, b_chunk, control, account)?,
        };
        if !order.is_eq() {
            return Ok(order.is_lt());
        }
        if next_left == a.len() {
            return Ok(true);
        }
        if next_right == b.len() {
            return Ok(false);
        }
        left = next_left;
        right = next_right;
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::EngineAccounting;

    fn owner_plan(count: usize, account: &QueryAccounting) -> Plan {
        let control = Control::default();
        let mut rows = AccountedVec::new(account, MemoryKind::Workspace);
        for index in 0..count {
            let mut labels = OwnedLabels::new(account, MemoryKind::Workspace);
            labels.push(b"__name__", b"m", account, MemoryKind::Workspace);
            labels.push(
                b"id",
                format!("i{index:08}").as_bytes(),
                account,
                MemoryKind::Workspace,
            );
            let hash = crate::label_hash::full(&labels, &control, account).unwrap();
            rows.push(Descriptor {
                labels,
                ordinal: index,
                hash,
                drop_name: false,
                first_order: index,
                sources: index..index + 1,
                kinds: 1,
                heads: Default::default(),
            });
        }
        Plan {
            rows,
            labels: AccountedVec::new(account, MemoryKind::Workspace),
            function: Function::Numeric(false),
            singleton: false,
            emit_certified: false,
            canonical: None,
            cache: RefCell::new(None),
        }
    }

    fn owner_options() -> EvaluationOptions {
        EvaluationOptions {
            time_ms: 4000,
            start_ms: 2000,
            end_ms: 4000,
            step_ms: 1000,
            lookback_ms: 300_000,
            start_timestamps: false,
            delayed_name_removal: false,
        }
    }

    fn inert_expression(account: &QueryAccounting) -> ParsedExpression {
        let mut arena = crate::frontend::ast::Arena::new_in(Some(&AllocationContext::new(
            account,
            MemoryKind::Compiled,
        )));
        let root = arena.push(
            Expr::String(AccountedVec::new(account, MemoryKind::Compiled)),
            Default::default(),
        );
        ParsedExpression {
            source: AccountedVec::new(account, MemoryKind::Compiled),
            arena,
            root,
        }
    }

    type EmissionTrace = (Vec<usize>, Vec<(usize, u64)>);

    fn drive_emission(
        step: &mut Step<'_>,
        expression: &ParsedExpression,
        target: Option<usize>,
        control: &Control,
        account: &QueryAccounting,
        value: f64,
    ) -> Result<EmissionTrace, EvaluationError> {
        let mut requested = Vec::new();
        let mut output = Vec::new();
        loop {
            control.check()?;
            match step.advance(expression, target.map(Target::Complete), control, account)? {
                Action::Child(Some(ordinal)) => {
                    let ordinal = *ordinal;
                    control.check()?;
                    requested.push(ordinal);
                    let row = step
                        .plan
                        .rows
                        .iter()
                        .find(|row| row.ordinal == ordinal)
                        .unwrap();
                    step.receive(
                        EvaluatedSample {
                            ordinal,
                            labels: row.labels.copy_checked(
                                account,
                                MemoryKind::Workspace,
                                control,
                            )?,
                            value: OwnedValue::Float(value),
                            float_samples: None,
                            source_time_ms: -1,
                            source_start_time_ms: 17,
                            drop_name: false,
                        },
                        control,
                        account,
                    )?;
                }
                Action::Sample(sample) => {
                    assert_eq!(sample.source_time_ms, step.options.time_ms);
                    assert_eq!(sample.source_start_time_ms, 0);
                    let OwnedValue::Float(value) = sample.value else {
                        panic!("float emission")
                    };
                    output.push((sample.ordinal, value.to_bits()));
                }
                Action::Continue => (),
                Action::Done => return Ok((requested, output)),
                _ => panic!("published emission must request a single catalog row"),
            }
        }
    }

    fn drive_prefix(
        step: &mut Step<'_>,
        expression: &ParsedExpression,
        target: usize,
        control: &Control,
        account: &QueryAccounting,
        round: u32,
        delivered: &mut Vec<usize>,
    ) -> Result<EmissionTrace, EvaluationError> {
        let mut requested = Vec::new();
        loop {
            match step.advance(expression, Some(Target::Prefix(target)), control, account)? {
                Action::Child(Some(ordinal)) => {
                    let ordinal = *ordinal;
                    requested.push(ordinal);
                    let row = step
                        .plan
                        .rows
                        .iter()
                        .find(|row| row.ordinal == ordinal)
                        .unwrap();
                    step.receive(
                        EvaluatedSample {
                            ordinal,
                            labels: row.labels.copy_checked(
                                account,
                                MemoryKind::Workspace,
                                control,
                            )?,
                            value: OwnedValue::Float(f64::from(round) * 1000.0 + ordinal as f64),
                            float_samples: None,
                            source_time_ms: -1,
                            source_start_time_ms: 17,
                            drop_name: false,
                        },
                        control,
                        account,
                    )?;
                }
                Action::Sample(sample) => {
                    control.check()?;
                    step.complete_prefix(control, account)?;
                    delivered.push(sample.ordinal);
                    assert_eq!(sample.source_time_ms, step.options.time_ms);
                    assert_eq!(sample.source_start_time_ms, 0);
                    let OwnedValue::Float(value) = sample.value else {
                        panic!("float emission")
                    };
                    if sample.ordinal == target {
                        return Ok((requested, vec![(sample.ordinal, value.to_bits())]));
                    }
                }
                Action::Continue => (),
                Action::Done => return Ok((requested, vec![])),
                _ => panic!("published emission must request one catalog row"),
            }
        }
    }

    #[test]
    fn nested_prefix_uses_permutation_positions_and_fresh_sparse_values() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let expression = inert_expression(&account);
        let mut plan = owner_plan(3, &account);
        for (row, ordinal) in plan.rows.iter_mut().zip([7, 23, 99]) {
            row.ordinal = ordinal;
        }
        plan.canonical = Some(AccountedVec::from_vec_in(
            vec![Some(2), Some(0), Some(1)],
            Some(&AllocationContext::new(&account, MemoryKind::Workspace)),
        ));
        plan.emit_certified = true;
        let mut seed = Step::new(&plan, owner_options(), 2, false, &account);
        seed.indices.extend_from_slice(&[2, 0, 1]);
        seed.publish(&control, &account).unwrap();
        let snapshot = seed.snapshot.as_ref().unwrap().clone();
        drop(seed);
        for (round, target, expected_reads, ordinal, progress) in [
            (1, 2, vec![99], 99, 1),
            (2, 1, vec![7, 23], 23, 3),
            (3, 2, vec![99], 99, 3),
        ] {
            let mut step = Step::new(&plan, owner_options(), 2, false, &account);
            let result = drive_prefix(
                &mut step,
                &expression,
                target,
                &control,
                &account,
                round,
                &mut vec![],
            )
            .unwrap();
            assert_eq!(result.0, expected_reads);
            assert_eq!(
                result.1,
                vec![(
                    target,
                    (f64::from(round) * 1000.0 + f64::from(ordinal)).to_bits()
                )]
            );
            assert_eq!(plan.emission_progress(), Some(progress));
            assert!(!snapshot.fully_emitted());
        }
        let mut root = Step::new(&plan, owner_options(), 2, false, &account);
        assert_eq!(
            drive_emission(&mut root, &expression, Some(2), &control, &account, 4.0)
                .unwrap()
                .0,
            vec![99, 7, 23]
        );
        assert!(snapshot.fully_emitted());
        drop(root);
        plan.clear_emission();
        assert_eq!(plan.emission_progress(), Some(0));
        assert!(snapshot.same_allocation(plan.cache.borrow().as_ref().unwrap()));
        let mut absent = Step::new(&plan, owner_options(), 2, false, &account);
        assert_eq!(
            drive_prefix(
                &mut absent,
                &expression,
                3,
                &control,
                &account,
                5,
                &mut vec![]
            )
            .unwrap(),
            (vec![99, 7, 23], vec![])
        );
        assert!(snapshot.fully_emitted());
        drop(absent);
        let mut selected = Step::new(&plan, owner_options(), 2, false, &account);
        assert_eq!(
            drive_prefix(
                &mut selected,
                &expression,
                0,
                &control,
                &account,
                6,
                &mut vec![]
            )
            .unwrap(),
            (vec![7], vec![(0, 6007.0f64.to_bits())])
        );
        assert!(snapshot.fully_emitted());
        drop(selected);
        for (certified, project, reads, value) in [
            (false, false, vec![99, 7, 23], 7023.0_f64),
            (true, true, vec![99], 7099.0),
        ] {
            plan.clear_emission();
            plan.emit_certified = certified;
            let mut ordinary = Step::new(&plan, owner_options(), 2, project, &account);
            assert_eq!(
                drive_prefix(
                    &mut ordinary,
                    &expression,
                    1,
                    &control,
                    &account,
                    7,
                    &mut vec![]
                )
                .unwrap(),
                (reads, vec![(1, value.to_bits())])
            );
            assert_eq!(plan.emission_progress(), Some(0));
        }
        drop((snapshot, plan, expression));
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn nested_prefix_belongs_to_each_exact_snapshot_owner() {
        for change in 1..=5 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let expression = inert_expression(&account);
            let mut plan = owner_plan(3, &account);
            plan.emit_certified = true;
            let options = owner_options();
            let mut old = Step::new(&plan, options, 2, false, &account);
            old.indices.extend_from_slice(&[2, 0, 1]);
            old.publish(&control, &account).unwrap();
            drive_prefix(&mut old, &expression, 2, &control, &account, 1, &mut vec![]).unwrap();
            let old_snapshot = old.snapshot.as_ref().unwrap().clone();
            assert_eq!(old_snapshot.emitted.load(AtomicOrdering::Relaxed), 1);
            let mut changed = options;
            let mut position = 2;
            match change {
                1 => changed.time_ms += 1,
                2 => changed.start_ms += 1,
                3 => changed.end_ms += 1,
                4 => changed.step_ms += 1,
                5 => position += 1,
                _ => unreachable!(),
            }
            let mut other = Step::new(&plan, changed, position, false, &account);
            assert!(other.snapshot.is_none());
            other.indices.extend_from_slice(&[1, 2, 0]);
            other.publish(&control, &account).unwrap();
            let other_snapshot = other.snapshot.as_ref().unwrap().clone();
            let mut current = Step::new(&plan, options, 2, false, &account);
            assert!(current.snapshot.is_none());
            current.indices.extend_from_slice(&[0, 1, 2]);
            current.publish(&control, &account).unwrap();
            let current_snapshot = current.snapshot.as_ref().unwrap().clone();
            assert!(!old_snapshot.same_allocation(&current_snapshot));
            assert_eq!(
                drive_prefix(&mut old, &expression, 0, &control, &account, 2, &mut vec![])
                    .unwrap()
                    .0,
                vec![0]
            );
            assert_eq!(old_snapshot.emitted.load(AtomicOrdering::Relaxed), 2);
            assert_eq!(other_snapshot.emitted.load(AtomicOrdering::Relaxed), 0);
            assert_eq!(current_snapshot.emitted.load(AtomicOrdering::Relaxed), 0);
            assert_eq!(
                drive_prefix(
                    &mut current,
                    &expression,
                    1,
                    &control,
                    &account,
                    3,
                    &mut vec![]
                )
                .unwrap()
                .0,
                vec![0, 1]
            );
            assert_eq!(current_snapshot.emitted.load(AtomicOrdering::Relaxed), 2);
            drop((old, other, current));
            plan.clear_emission();
            assert_eq!(current_snapshot.emitted.load(AtomicOrdering::Relaxed), 0);
            assert_eq!(old_snapshot.emitted.load(AtomicOrdering::Relaxed), 2);
            drop((
                old_snapshot,
                other_snapshot,
                current_snapshot,
                plan,
                expression,
            ));
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn nested_prefix_interruption_grants_only_completed_handoffs() {
        for warm in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let expression = inert_expression(&account);
            let mut plan = owner_plan(3, &account);
            plan.emit_certified = true;
            let mut seed = Step::new(&plan, owner_options(), 2, false, &account);
            seed.indices.extend_from_slice(&[2, 0, 1]);
            seed.publish(&control, &account).unwrap();
            let snapshot = seed.snapshot.as_ref().unwrap().clone();
            if warm {
                drive_prefix(
                    &mut seed,
                    &expression,
                    2,
                    &control,
                    &account,
                    1,
                    &mut vec![],
                )
                .unwrap();
            }
            let prior = snapshot.emitted.load(AtomicOrdering::Relaxed);
            drop(seed);
            let mut measured = Step::new(&plan, owner_options(), 2, false, &account);
            let before = account.snapshot().work;
            drive_prefix(
                &mut measured,
                &expression,
                1,
                &control,
                &account,
                2,
                &mut vec![],
            )
            .unwrap();
            let work = account.snapshot().work - before;
            drop(measured);
            for charge in 1..=work {
                snapshot.emitted.store(prior, AtomicOrdering::Relaxed);
                let control = Control::default();
                let mut step = Step::new(&plan, owner_options(), 2, false, &account);
                let held = account.snapshot().memory[MemoryKind::Workspace as usize].current;
                let mut delivered = Vec::new();
                let result =
                    crate::test_storage::interrupt_after_work(&account, &control, charge, || {
                        drive_prefix(
                            &mut step,
                            &expression,
                            1,
                            &control,
                            &account,
                            3,
                            &mut delivered,
                        )
                    });
                assert!(
                    matches!(
                        result,
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    ),
                    "warm={warm},charge={charge}/{work}"
                );
                let expected = delivered
                    .iter()
                    .map(|input| [2, 0, 1].iter().position(|value| value == input).unwrap() + 1)
                    .max()
                    .unwrap_or(prior)
                    .max(prior);
                assert_eq!(
                    snapshot.emitted.load(AtomicOrdering::Relaxed),
                    expected,
                    "warm={warm},charge={charge}/{work}"
                );
                drop(step);
                assert_eq!(
                    account.snapshot().memory[MemoryKind::Workspace as usize].current,
                    held
                );
            }
            println!("{{\"nested_prefix_warm\":{warm},\"charged_boundaries\":{work}}}");
            drop((snapshot, plan, expression));
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn emission_reuse_keeps_sparse_domains_fresh_values_and_ordinary_gates() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let expression = inert_expression(&account);
        let mut plan = owner_plan(3, &account);
        for (row, ordinal) in plan.rows.iter_mut().zip([7, 23, 99]) {
            row.ordinal = ordinal;
        }
        plan.canonical = Some(AccountedVec::from_vec_in(
            vec![Some(2), Some(0), Some(1)],
            Some(&AllocationContext::new(&account, MemoryKind::Workspace)),
        ));
        plan.emit_certified = true;
        let mut first = Step::new(&plan, owner_options(), 2, false, &account);
        first.indices.extend_from_slice(&[2, 0]);
        first.publish(&control, &account).unwrap();
        assert_eq!(
            drive_emission(&mut first, &expression, Some(2), &control, &account, 1.0).unwrap(),
            (
                vec![99, 7],
                vec![(2, 1.0f64.to_bits()), (0, 1.0f64.to_bits())]
            )
        );
        drop(first);
        for (target, project, expected_reads, expected_ordinals) in [
            (Some(2), false, vec![99], vec![2]),
            (Some(1), false, vec![], vec![]),
            (None, false, vec![99, 7], vec![2, 0]),
            (Some(2), true, vec![99, 7], vec![1, 2]),
        ] {
            let mut step = Step::new(&plan, owner_options(), 2, project, &account);
            let (reads, output) =
                drive_emission(&mut step, &expression, target, &control, &account, 2.0).unwrap();
            assert_eq!(reads, expected_reads);
            assert_eq!(
                output,
                expected_ordinals
                    .into_iter()
                    .map(|ordinal| (ordinal, 2.0f64.to_bits()))
                    .collect::<Vec<_>>()
            );
        }
        plan.emit_certified = false;
        let mut ordinary = Step::new(&plan, owner_options(), 2, false, &account);
        assert_eq!(
            drive_emission(&mut ordinary, &expression, Some(2), &control, &account, 3.0)
                .unwrap()
                .0,
            vec![99, 7]
        );
        drop(ordinary);
        drop((plan, expression));
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn emission_completion_belongs_to_each_exact_snapshot_owner() {
        for change in 1..=5 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let expression = inert_expression(&account);
            let mut plan = owner_plan(3, &account);
            plan.emit_certified = true;
            let options = owner_options();
            let mut completed = Step::new(&plan, options, 2, false, &account);
            completed.indices.extend_from_slice(&[2, 0, 1]);
            completed.publish(&control, &account).unwrap();
            drive_emission(
                &mut completed,
                &expression,
                Some(0),
                &control,
                &account,
                0.5,
            )
            .unwrap();
            drop(completed);
            let hit = Step::new(&plan, options, 2, false, &account);
            assert!(hit.snapshot.as_ref().unwrap().fully_emitted());
            drop(hit);
            let mut changed = options;
            let mut position = 2;
            match change {
                1 => changed.time_ms += 1,
                2 => changed.start_ms += 1,
                3 => changed.end_ms += 1,
                4 => changed.step_ms += 1,
                5 => position += 1,
                _ => unreachable!(),
            }
            let changed_owner = Step::new(&plan, changed, position, false, &account);
            assert!(matches!(changed_owner.stage, Stage::Scan));
            assert!(changed_owner.snapshot.is_none());
            drop(changed_owner);
            let mut old = Step::new(&plan, options, 2, false, &account);
            old.indices.extend_from_slice(&[2, 0, 1]);
            old.publish(&control, &account).unwrap();
            let old_snapshot = old.snapshot.as_ref().unwrap().clone();
            let mut other = Step::new(&plan, changed, position, false, &account);
            assert!(matches!(other.stage, Stage::Scan));
            other.indices.extend_from_slice(&[1, 2, 0]);
            other.publish(&control, &account).unwrap();
            let other_snapshot = other.snapshot.as_ref().unwrap().clone();
            let mut current = Step::new(&plan, options, 2, false, &account);
            assert!(matches!(current.stage, Stage::Scan));
            current.indices.extend_from_slice(&[0, 1, 2]);
            current.publish(&control, &account).unwrap();
            let current_snapshot = current.snapshot.as_ref().unwrap().clone();
            assert!(!old_snapshot.same_allocation(&current_snapshot));
            assert_eq!(
                drive_emission(&mut old, &expression, Some(0), &control, &account, 1.0)
                    .unwrap()
                    .0,
                vec![2, 0, 1]
            );
            assert!(old_snapshot.fully_emitted());
            assert!(!other_snapshot.fully_emitted());
            assert!(!current_snapshot.fully_emitted());
            assert_eq!(
                drive_emission(&mut current, &expression, Some(0), &control, &account, 2.0)
                    .unwrap()
                    .0,
                vec![0, 1, 2]
            );
            drop(current);
            let mut hit = Step::new(&plan, options, 2, false, &account);
            assert!(
                hit.snapshot
                    .as_ref()
                    .unwrap()
                    .same_allocation(&current_snapshot)
            );
            assert_eq!(
                drive_emission(&mut hit, &expression, Some(2), &control, &account, 3.0)
                    .unwrap()
                    .0,
                vec![2]
            );
            drop((
                hit,
                old,
                other,
                old_snapshot,
                other_snapshot,
                current_snapshot,
            ));
            drop((plan, expression));
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn emission_every_work_boundary_preserves_prior_credit_and_releases() {
        for warm in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let expression = inert_expression(&account);
            let mut plan = owner_plan(3, &account);
            plan.emit_certified = true;
            let control = Control::default();
            let mut seed = Step::new(&plan, owner_options(), 2, false, &account);
            seed.indices.extend_from_slice(&[2, 0, 1]);
            seed.publish(&control, &account).unwrap();
            let snapshot = seed.snapshot.as_ref().unwrap().clone();
            if warm {
                drive_emission(&mut seed, &expression, Some(0), &control, &account, 1.0).unwrap();
            }
            drop(seed);
            let mut complete = Step::new(&plan, owner_options(), 2, false, &account);
            let before = account.snapshot().work;
            drive_emission(&mut complete, &expression, Some(0), &control, &account, 2.0).unwrap();
            let work = account.snapshot().work - before;
            drop(complete);
            snapshot
                .emitted
                .store(if warm { usize::MAX } else { 0 }, AtomicOrdering::Relaxed);
            for charge in 1..=work {
                let control = Control::default();
                let mut step = Step::new(&plan, owner_options(), 2, false, &account);
                let held = account.snapshot().memory[MemoryKind::Workspace as usize].current;
                let result =
                    crate::test_storage::interrupt_after_work(&account, &control, charge, || {
                        drive_emission(&mut step, &expression, Some(0), &control, &account, 3.0)
                    });
                assert!(
                    matches!(
                        result,
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    ),
                    "warm={warm}, charge={charge}/{work}"
                );
                assert_eq!(snapshot.fully_emitted(), warm);
                drop(step);
                assert_eq!(
                    account.snapshot().memory[MemoryKind::Workspace as usize].current,
                    held
                );
            }
            println!("{{\"sort_emit_warm\":{warm},\"charged_boundaries\":{work}}}");
            drop(snapshot);
            drop((plan, expression));
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn emission_snapshot_traits_and_accounted_layouts_remain_explicit() {
        fn send<T: Send>() {}
        fn shared<T: Send + Sync>() {}
        send::<Plan>();
        shared::<Snapshot>();
        shared::<Strong<Snapshot>>();
        println!(
            "{{\"sort_plan\":{},\"sort_plan_allocation\":{},\"sort_snapshot\":{},\"sort_snapshot_allocation\":{},\"sort_step\":{},\"sort_step_allocation\":{}}}",
            size_of::<Plan>(),
            crate::accounting::AccountedBox::<Plan>::allocation_bytes(),
            size_of::<Snapshot>(),
            Strong::<Snapshot>::allocation_bytes(),
            size_of::<Step<'static>>(),
            crate::accounting::AccountedBox::<Step<'static>>::allocation_bytes()
        );
    }

    #[test]
    fn emission_fresh_selected_missing_and_wrong_kind_errors_remain() {
        for wrong_kind in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let expression = inert_expression(&account);
            let mut plan = owner_plan(3, &account);
            plan.emit_certified = true;
            let mut seed = Step::new(&plan, owner_options(), 2, false, &account);
            seed.indices.extend_from_slice(&[2, 0, 1]);
            seed.publish(&control, &account).unwrap();
            drive_emission(&mut seed, &expression, Some(0), &control, &account, 1.0).unwrap();
            drop(seed);
            let mut step = Step::new(&plan, owner_options(), 2, false, &account);
            assert!(matches!(
                step.advance(&expression, Some(Target::Complete(0)), &control, &account)
                    .unwrap(),
                Action::Continue
            ));
            assert!(matches!(
                step.advance(&expression, Some(Target::Complete(0)), &control, &account)
                    .unwrap(),
                Action::Child(Some(0))
            ));
            if wrong_kind {
                let histogram = crate::histogram::FloatHistogram::default();
                step.receive(
                    EvaluatedSample {
                        ordinal: 0,
                        labels: plan.rows[0]
                            .labels
                            .copy_checked(&account, MemoryKind::Workspace, &control)
                            .unwrap(),
                        value: OwnedValue::from_sample_checked(
                            crate::value::SampleValueRef::FloatHistogram(histogram.as_ref()),
                            &control,
                            &account,
                            MemoryKind::Workspace,
                        )
                        .unwrap(),
                        float_samples: None,
                        source_time_ms: 4000,
                        source_start_time_ms: 0,
                        drop_name: false,
                    },
                    &control,
                    &account,
                )
                .unwrap();
            }
            let result = step.advance(&expression, Some(Target::Complete(0)), &control, &account);
            assert!(matches!(
                result,
                Err(EvaluationError::Invalid("sort row changed during replay"))
            ));
            drop(result);
            drop(step);
            drop((plan, expression));
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn emission_certificate_rejects_inconsistent_metadata_and_every_charge() {
        for invalid in 0..17 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut plan = owner_plan(3, &account);
            let mut descriptors = owner_plan(3, &account).rows;
            let mut sources = vec![2, 0, 1];
            for (row, ordinal) in plan.rows.iter_mut().zip([7, 23, 99]) {
                row.ordinal = ordinal;
                row.sources = 0..0;
            }
            for (descriptor, input) in descriptors.iter_mut().zip(&sources) {
                descriptor.hash = plan.rows[*input].hash;
            }
            plan.project(&descriptors, &sources, &control, &account)
                .unwrap();
            let mut catalog_len = 100;
            match invalid {
                0 => (),
                1 => plan.canonical = None,
                2 => plan.rows.truncate(1),
                3 => {
                    descriptors.pop();
                }
                4 => {
                    plan.canonical.as_mut().unwrap().pop();
                }
                5 => plan.rows[1].kinds = 2,
                6 => {
                    let mut counts = crate::evaluator_roster::PointCounts::default();
                    counts.push(1);
                    counts.push(1);
                    plan.rows[1].heads.note(counts, 4000);
                }
                7 => catalog_len = 99,
                8 => plan.rows[1].ordinal = 7,
                9 => descriptors[1].kinds = 3,
                10 => {
                    let mut counts = crate::evaluator_roster::PointCounts::default();
                    counts.push(1);
                    counts.push(1);
                    descriptors[1].heads.note(counts, 4000);
                }
                11 => descriptors[1].sources = 0..2,
                12 => descriptors[1].ordinal = 7,
                13 => sources[1] = 3,
                14 => descriptors[1].hash ^= 1,
                15 => plan.canonical.as_mut().unwrap()[sources[1]] = Some(2),
                16 => sources.clear(),
                _ => unreachable!(),
            }
            let before = account.snapshot().work;
            plan.certify_emit(&descriptors, &sources, catalog_len, &control, &account)
                .unwrap();
            assert_eq!(plan.emit_certified, invalid == 0, "invalid={invalid}");
            if invalid == 0 {
                let work = account.snapshot().work - before;
                assert!(work > 0);
                for charge in 1..=work {
                    plan.emit_certified = false;
                    let control = Control::default();
                    let result = crate::test_storage::interrupt_after_work(
                        &account,
                        &control,
                        charge,
                        || {
                            plan.certify_emit(
                                &descriptors,
                                &sources,
                                catalog_len,
                                &control,
                                &account,
                            )
                        },
                    );
                    assert!(matches!(result, Err(Interrupted::Cancelled)));
                    assert!(!plan.emit_certified);
                }
            }
            drop((plan, descriptors));
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn emission_completion_waits_for_successful_done_handoff() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut plan = owner_plan(3, &account);
        plan.emit_certified = true;
        let expression = inert_expression(&account);
        let mut step = Step::new(&plan, owner_options(), 2, false, &account);
        step.indices.extend_from_slice(&[0, 1, 2]);
        step.publish(&control, &account).unwrap();
        let snapshot = step.snapshot.as_ref().unwrap().clone();
        let mut delivered = 0;
        while !matches!(step.stage, Stage::Done) {
            match step
                .advance(&expression, Some(Target::Complete(0)), &control, &account)
                .unwrap()
            {
                Action::Child(Some(ordinal)) => {
                    let ordinal = *ordinal;
                    step.receive(
                        EvaluatedSample {
                            ordinal,
                            labels: plan.rows[ordinal]
                                .labels
                                .copy_checked(&account, MemoryKind::Workspace, &control)
                                .unwrap(),
                            value: OwnedValue::Float(1.0),
                            float_samples: None,
                            source_time_ms: 4000,
                            source_start_time_ms: 0,
                            drop_name: false,
                        },
                        &control,
                        &account,
                    )
                    .unwrap();
                }
                Action::Sample(_) => delivered += 1,
                Action::Continue => (),
                _ => panic!("original full emission before Done"),
            }
        }
        assert_eq!(delivered, 3);
        assert!(
            !snapshot.fully_emitted(),
            "abandonment before the Done handoff must grant no completed credit"
        );
        let interrupted = Control::default();
        let result = crate::test_storage::interrupt_after_work(&account, &interrupted, 1, || {
            step.advance(
                &expression,
                Some(Target::Complete(0)),
                &interrupted,
                &account,
            )
        });
        assert!(matches!(
            result,
            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
        ));
        assert!(!snapshot.fully_emitted());
        assert!(matches!(
            step.advance(
                &expression,
                Some(Target::Complete(0)),
                &Control::default(),
                &account
            )
            .unwrap(),
            Action::Done
        ));
        assert!(snapshot.fully_emitted());
        drop(snapshot);
        drop(step);
        drop(plan);
        drop(expression);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn publication_hash_identity_preserves_sparse_order_and_delayed_gate() {
        for count in [32, 33, 43] {
            for identity in ["unique", "repeat", "collision", "zero"] {
                for (delayed, range) in [(false, false), (true, false), (true, true)] {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let mut plan = owner_plan(count * 3 + 4, &account);
                    let mut indices: Vec<_> = (0..count).rev().map(|i| i * 3 + 1).collect();
                    if identity == "repeat" {
                        indices[count - 1] = indices[0];
                    } else if identity == "collision" {
                        for (input, value) in [indices[0], indices[count - 1]]
                            .into_iter()
                            .zip(crate::test_storage::FULL_LABEL_HASH_COLLISION_VALUES)
                        {
                            let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
                            labels.push(b"a", value, &account, MemoryKind::Workspace);
                            plan.rows[input].hash =
                                crate::label_hash::full(&labels, &control, &account).unwrap();
                            plan.rows[input].labels = labels;
                        }
                        assert_ne!(
                            &*plan.rows[indices[0]].labels.0[0].value,
                            &*plan.rows[indices[count - 1]].labels.0[0].value
                        );
                        assert_eq!(plan.rows[indices[0]].hash, 0x9c71_0b2e_3747_e3bd);
                        assert_eq!(plan.rows[indices[count - 1]].hash, 0x9c71_0b2e_3747_e3bd);
                    } else if identity == "zero" {
                        plan.rows[indices[0]].hash = 0;
                        plan.rows[indices[count - 1]].hash = 0;
                    }
                    let options = EvaluationOptions {
                        start_ms: if range { 2000 } else { 4000 },
                        delayed_name_removal: delayed,
                        ..owner_options()
                    };
                    let mut step = Step::new(&plan, options, 2, false, &account);
                    step.indices.extend_from_slice(&indices);
                    let before = account.snapshot();
                    let result = step.publish(&control, &account);
                    let bypass = delayed && !range;
                    if identity == "unique" || bypass {
                        result.unwrap();
                        assert!(matches!(step.stage, Stage::Emit));
                        assert!(step.snapshot.as_ref().unwrap().key == (options, 2).into());
                        assert_eq!(&*step.snapshot.as_ref().unwrap().indices, &*indices);
                        if bypass {
                            assert_eq!(account.snapshot().work, before.work);
                        }
                    } else {
                        assert!(matches!(
                            result,
                            Err(EvaluationError::Invalid(
                                "vector cannot contain metrics with the same labelset"
                            ))
                        ));
                        assert!(plan.cache.borrow().is_none() && step.snapshot.is_none());
                        assert_eq!(&*step.indices, &*indices);
                        assert_eq!(
                            account.snapshot().memory[MemoryKind::Workspace as usize].current,
                            before.memory[MemoryKind::Workspace as usize].current
                        );
                    }
                    assert_eq!(step.take_range_warning(), range);
                    assert!(!step.take_range_warning());
                    drop(step);
                    drop(plan);
                    account.snapshot().assert_memory_released();
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn publication_snapshot_hits_and_all_real_key_changes_invalidate() {
        for change in 0..6 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let plan = owner_plan(43, &account);
            let options = owner_options();
            let expected: Vec<_> = (0..43).map(|i| i * 37 % 43).collect();
            let mut step = Step::new(&plan, options, 2, false, &account);
            step.indices.extend_from_slice(&expected);
            step.publish(&control, &account).unwrap();
            let before = account.snapshot().memory;
            drop(step);
            let mut changed = options;
            let mut position = 2;
            match change {
                0 => (),
                1 => changed.time_ms += 1,
                2 => changed.start_ms += 1,
                3 => changed.end_ms += 1,
                4 => changed.step_ms += 1,
                5 => position += 1,
                _ => unreachable!(),
            }
            let step = Step::new(&plan, changed, position, true, &account);
            if change == 0 {
                assert!(matches!(step.stage, Stage::Emit));
                assert_eq!(&*step.snapshot.as_ref().unwrap().indices, &*expected);
                assert_eq!(account.snapshot().memory, before);
            } else {
                assert!(matches!(step.stage, Stage::Scan));
                assert!(step.snapshot.is_none() && plan.cache.borrow().is_none());
                assert!(
                    account.snapshot().memory[MemoryKind::Workspace as usize].current
                        < before[MemoryKind::Workspace as usize].current
                );
            }
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn publication_every_work_boundary_cleans_up_without_a_snapshot() {
        for count in [32, 33, 43] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let plan = owner_plan(count, &account);
            let indices: Vec<_> = (0..count).map(|i| i * 37 % count).collect();
            let mut step = Step::new(&plan, owner_options(), 2, false, &account);
            step.indices.extend_from_slice(&indices);
            let before = account.snapshot().work;
            step.validate_hashes(&Control::default(), &account).unwrap();
            let total = account.snapshot().work - before;
            let held = account.snapshot().memory[MemoryKind::Workspace as usize].current;
            for charge in 1..=total {
                let control = Control::default();
                let before = account.snapshot().work;
                let result =
                    crate::test_storage::interrupt_after_work(&account, &control, charge, || {
                        step.publish(&control, &account)
                    });
                assert!(matches!(
                    result,
                    Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                ));
                assert!(account.snapshot().work - before >= charge);
                assert!(plan.cache.borrow().is_none() && step.snapshot.is_none());
                assert_eq!(&*step.indices, &*indices);
                assert_eq!(
                    account.snapshot().memory[MemoryKind::Workspace as usize].current,
                    held
                );
            }
            step.publish(&Control::default(), &account).unwrap();
            assert_eq!(&*step.snapshot.as_ref().unwrap().indices, &*indices);
            println!("{{\"sort_cancel_rows\":{count},\"charged_boundaries\":{total}}}");
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn publication_duplicate_error_precedes_its_final_charge_cancellation() {
        for count in [32, 33, 43] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let plan = owner_plan(count, &account);
            let mut step = Step::new(&plan, owner_options(), 2, false, &account);
            step.indices
                .extend_from_slice(&(0..count).collect::<Vec<_>>());
            step.indices[count - 1] = 0;
            let before = account.snapshot().work;
            let result = step.publish(&Control::default(), &account);
            assert!(matches!(result, Err(EvaluationError::Invalid(_))));
            let work = account.snapshot().work - before;
            let control = Control::default();
            let result =
                crate::test_storage::interrupt_after_work(&account, &control, work, || {
                    step.publish(&control, &account)
                });
            assert!(control.cancellation.cancelled());
            assert!(matches!(
                result,
                Err(EvaluationError::Invalid(
                    "vector cannot contain metrics with the same labelset"
                ))
            ));
            assert!(plan.cache.borrow().is_none() && step.snapshot.is_none());
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn publication_completed_operand_membership_filters_only_numeric_histograms() {
        for (count, native_only) in [(33, false), (34, false), (33, true)] {
            for labels in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let mut plan = owner_plan(count, &account);
                if labels {
                    plan.function = Function::Labels(false);
                }
                let mut step = Step::new(&plan, owner_options(), 2, false, &account);
                for index in (0..count).rev() {
                    let native = native_only || index == count - 1;
                    let value = if native {
                        let histogram = crate::histogram::FloatHistogram::default();
                        OwnedValue::from_sample_checked(
                            crate::value::SampleValueRef::FloatHistogram(histogram.as_ref()),
                            &control,
                            &account,
                            MemoryKind::Workspace,
                        )
                        .unwrap()
                    } else {
                        OwnedValue::Float(index as f64)
                    };
                    step.receive(
                        EvaluatedSample {
                            ordinal: index,
                            labels: plan.rows[index]
                                .labels
                                .copy_checked(&account, MemoryKind::Workspace, &control)
                                .unwrap(),
                            value,
                            float_samples: None,
                            source_time_ms: 4000,
                            source_start_time_ms: 0,
                            drop_name: false,
                        },
                        &control,
                        &account,
                    )
                    .unwrap();
                }
                let expected: Vec<_> = (0..count)
                    .rev()
                    .filter(|&i| labels || (!native_only && i != count - 1))
                    .collect();
                assert_eq!(&*step.indices, &*expected);
                step.publish(&control, &account).unwrap();
                assert_eq!(&*step.snapshot.as_ref().unwrap().indices, &*expected);
                drop(step);
                drop(plan);
                account.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn publication_pending_duplicates_wait_for_string_and_numeric_replay_errors() {
        for numeric in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut plan = owner_plan(43, &account);
            let mut expression = inert_expression(&account);
            let paren = expression
                .arena
                .push(Expr::Paren(expression.root), Default::default());
            if !numeric {
                plan.function = Function::Labels(false);
                plan.labels.push(paren);
            }
            let mut step = Step::new(&plan, owner_options(), 2, false, &account);
            step.indices.extend_from_slice(&(0..43).collect::<Vec<_>>());
            step.indices[42] = 0;
            step.waiting = true;
            if numeric {
                step.stage = Stage::Program;
                step.comparison = Some(Comparison {
                    inputs: [0, 1],
                    values: [0.0; 2],
                    position: 0,
                    samples: [None, None],
                });
            }
            let result = step.advance(&expression, None, &control, &account);
            if numeric {
                assert!(matches!(
                    result,
                    Err(EvaluationError::Invalid(
                        "sort operand changed during replay"
                    ))
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(EvaluationError::WrappedRuntime(ref bytes))
                    if bytes.starts_with(b"unexpected error: interface conversion: parser.Expr is *parser.ParenExpr")
                ));
            }
            drop(result);
            assert!(plan.cache.borrow().is_none() && step.snapshot.is_none());
            drop(step);
            drop(plan);
            drop(expression);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn publication_duplicate_lookup_avoids_all_pairs() {
        let mut observations = Vec::new();
        for count in [128, 512] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let plan = owner_plan(count, &account);
            let options = owner_options();
            let mut step = Step::new(&plan, options, 2, false, &account);
            let expected: Vec<_> = (0..count).map(|position| position * 37 % count).collect();
            step.indices.extend_from_slice(&expected);
            let before = account.snapshot().work;
            step.publish(&control, &account).unwrap();
            let work = account.snapshot().work - before;
            assert!(matches!(step.stage, Stage::Emit));
            assert!(step.indices.is_empty());
            assert!(step.take_range_warning());
            {
                let snapshot = plan.cache.borrow();
                let snapshot = snapshot.as_ref().unwrap();
                assert!(snapshot.key == Key::from((options, 2)));
                assert_eq!(&*snapshot.indices, expected.as_slice());
            }
            assert_eq!(
                &*step.snapshot.as_ref().unwrap().indices,
                expected.as_slice()
            );
            observations.push(work);
            println!("{{\"sort_publication_rows\":{count},\"work\":{work}}}");
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
        assert!(
            observations[1] <= observations[0] * 6,
            "publication duplicate lookup must avoid all-pairs growth: {observations:?}"
        );
    }

    #[test]
    fn natural_order_preserves_source_nonstrict_and_overflow_cases() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        for (left, right, before) in [
            (b"".as_slice(), b"v1".as_slice(), false),
            (b"v1", b"", false),
            (b"v1", b"v01", true),
            (b"v01", b"v1", true),
            (b"v2", b"v10", true),
            (b"v9223372036854775807", b"v9223372036854775808", true),
            (b"v9223372036854775808", b"v9223372036854775807", false),
        ] {
            assert_eq!(
                natural_before(left, right, &control, &account).unwrap(),
                before
            );
        }
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn natural_chunk_walk_interrupts_without_allocating() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let name = vec![b'a'; 100_000];
        let before = account.snapshot().memory;
        let result = crate::test_storage::interrupt_during(&account, &control, || {
            natural_before(&name, b"b", &control, &account)
        });
        assert!(matches!(result, Err(Interrupted::Cancelled)));
        assert_eq!(account.snapshot().memory, before);
        account.snapshot().assert_memory_released();
    }
}
