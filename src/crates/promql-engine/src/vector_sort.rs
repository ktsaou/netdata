// SPDX-License-Identifier: GPL-3.0-or-later
// Sort semantics derive from pinned PromQL, Go sort and facette/natsort; see NOTICE.
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    binary_labels,
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_roster::Descriptor,
    frontend::ast::{Expr, NodeId, ParsedExpression},
    go_sort_program::{Operation, Program, physical_index},
    query_preparation::Prepared,
    result::{OwnedLabels, OwnedValue},
    shared::Strong,
    storage::{Control, Interrupted},
};
use std::{cell::RefCell, cmp::Ordering};

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
    canonical: Option<AccountedVec<Option<usize>>>,
    cache: RefCell<Option<Strong<Snapshot>>>,
}
impl Plan {
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
        }
        Ok(())
    }
    fn request(&mut self, input: usize) -> Action<'a> {
        self.waiting = true;
        self.expected = Some(input);
        self.value = None;
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
        }
        let indices = std::mem::replace(
            &mut self.indices,
            AccountedVec::new(account, MemoryKind::Workspace),
        );
        let snapshot = Strong::new_in(
            Snapshot {
                key: (self.options, self.step_index).into(),
                indices,
            },
            Some(&AllocationContext::new(account, MemoryKind::Workspace)),
        );
        *self.plan.cache.borrow_mut() = Some(snapshot.clone());
        self.snapshot = Some(snapshot);
        self.stage = Stage::Emit;
        Ok(())
    }
    pub(crate) fn take_range_warning(&mut self) -> bool {
        std::mem::take(&mut self.range_warning)
    }
    pub(crate) fn advance(
        &mut self,
        expression: &ParsedExpression,
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
                if let Some(&input) = self
                    .snapshot
                    .as_ref()
                    .expect("sorted snapshot")
                    .indices
                    .get(self.position)
                {
                    return Ok(self.request(input));
                }
                self.stage = Stage::Done;
            }
            Stage::Done => return Ok(Action::Done),
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
