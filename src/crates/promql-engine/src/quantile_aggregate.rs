// SPDX-License-Identifier: GPL-3.0-or-later
// Ordinal replay follows pinned promql/engine.go and quantile.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::AnnotationKind,
    binary_labels,
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_roster::{Descriptor, GroupDescriptor, copy_group_key},
    frontend::ast::{Expr, NodeId, ParsedExpression},
    go_sort_program::{Operation, Program, physical_index},
    label_hash,
    numerical::{floor, source_add, source_mul},
    position_index::PositionIndex,
    result::OwnedValue,
    shared::Strong,
    storage::{Control, Interrupted},
    topk::Parameters,
};
use std::cell::RefCell;

const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

pub(crate) struct Plan {
    pub(crate) parameters: Parameters,
    inputs: AccountedVec<Descriptor>,
    bindings: AccountedVec<usize>,
    cache: RefCell<Option<Strong<Snapshot>>>,
}
impl Plan {
    pub(crate) fn new(parameters: Parameters, account: &QueryAccounting) -> Self {
        Self {
            parameters,
            inputs: AccountedVec::new(account, MemoryKind::Workspace),
            bindings: AccountedVec::new(account, MemoryKind::Workspace),
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
    ) -> Result<AccountedVec<GroupDescriptor>, EvaluationError> {
        let Expr::Aggregate {
            grouping, without, ..
        } = &expression.arena[node].expression
        else {
            unreachable!("typed quantile aggregate")
        };
        let mut groups = AccountedVec::<GroupDescriptor>::new(account, MemoryKind::Workspace);
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
            let position = if let Some(position) = index.find(
                hash,
                |position| Ok(groups[position].hash == hash),
                control,
                account,
            )? {
                position
            } else {
                let position = groups.len();
                groups.push(GroupDescriptor {
                    hash,
                    labels,
                    whole_drop_name: None,
                });
                index.insert_new(
                    hash,
                    position,
                    |prior| Ok(groups[prior].hash),
                    control,
                    account,
                )?;
                position
            };
            bindings.push(position);
        }
        self.inputs = rows;
        self.bindings = bindings;
        Ok(groups)
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
struct Group {
    indices: AccountedVec<usize>,
    drop_name: bool,
}
struct Snapshot {
    _parameter_samples: Option<crate::statistics::OwnedSamples>,
    key: Key,
    parameter: f64,
    groups: AccountedVec<Group>,
    ignored: bool,
}
#[derive(Clone, Copy)]
enum Stage {
    Parameter,
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
    descriptors: &'a [GroupDescriptor],
    options: EvaluationOptions,
    step_index: i64,
    stage: Stage,
    groups: AccountedVec<Group>,
    snapshot: Option<Strong<Snapshot>>,
    parameter: f64,
    input: usize,
    group: usize,
    waiting: bool,
    expected: Option<usize>,
    pending: Pending,
    pending_samples: Option<crate::statistics::OwnedSamples>,
    parameter_samples: Option<crate::statistics::OwnedSamples>,
    program: Option<Program>,
    comparison: Option<Comparison>,
    reply: Option<bool>,
    weight: f64,
    ignored: bool,
}
impl<'a> Step<'a> {
    pub(crate) fn new(
        plan: &'a Plan,
        descriptors: &'a [GroupDescriptor],
        options: EvaluationOptions,
        step_index: i64,
        account: &QueryAccounting,
    ) -> Self {
        let snapshot = {
            let mut cache = plan.cache.borrow_mut();
            if cache
                .as_ref()
                .is_some_and(|snapshot| snapshot.key != Key::from((options, step_index)))
            {
                *cache = None;
            }
            cache.as_ref().cloned()
        };
        let (stage, parameter) = if let Some(snapshot) = &snapshot {
            (Stage::Emit, snapshot.parameter)
        } else if let Some(value) = plan.parameters.fixed_value() {
            (Stage::Scan, value)
        } else {
            (Stage::Parameter, 0.0)
        };
        Self {
            plan,
            descriptors,
            options,
            step_index,
            stage,
            groups: AccountedVec::new(account, MemoryKind::Workspace),
            snapshot,
            parameter,
            input: 0,
            group: 0,
            waiting: false,
            expected: None,
            pending: Pending::Empty,
            pending_samples: None,
            parameter_samples: (!matches!(stage, Stage::Parameter | Stage::Done))
                .then(|| crate::statistics::OwnedSamples::new(account, 1)),
            program: None,
            comparison: None,
            reply: None,
            weight: 0.0,
            ignored: false,
        }
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
        sample: EvaluatedSample<'_>,
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
        if matches!(self.pending, Pending::Empty) {
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
                "quantile operand changed during replay",
            )),
        }
    }
    fn publish(&mut self, control: &Control, account: &QueryAccounting) -> Result<(), Interrupted> {
        control.check()?;
        let groups = std::mem::replace(
            &mut self.groups,
            AccountedVec::new(account, MemoryKind::Workspace),
        );
        let snapshot = Strong::new_in(
            Snapshot {
                _parameter_samples: self
                    .parameter_samples
                    .as_ref()
                    .map(|_| crate::statistics::OwnedSamples::new(account, 1)),
                key: (self.options, self.step_index).into(),
                parameter: self.parameter,
                groups,
                ignored: self.ignored,
            },
            Some(&AllocationContext::new(account, MemoryKind::Workspace)),
        );
        *self.plan.cache.borrow_mut() = Some(snapshot.clone());
        self.snapshot = Some(snapshot);
        self.group = 0;
        self.stage = Stage::Emit;
        Ok(())
    }
    fn emit(
        &mut self,
        value: f64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action<'a>, EvaluationError> {
        let ordinal = self.group;
        self.group += 1;
        let descriptor = &self.descriptors[ordinal];
        let step_flag =
            self.snapshot.as_ref().expect("quantile snapshot").groups[ordinal].drop_name;
        let labels =
            crate::sample_labels::SampleLabels::aggregate(&descriptor.labels, account, control)?;
        Ok(Action::Sample(EvaluatedSample {
            ordinal,
            labels,
            float_samples: Some(crate::statistics::OwnedSamples::new(account, 1)),
            value: OwnedValue::Float(value),
            source_time_ms: self.options.time_ms,
            source_start_time_ms: 0,
            // Discovery-time snapshots precede final whole-matrix flag preparation.
            drop_name: descriptor.whole_drop_name.unwrap_or(step_flag),
        }))
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
                self.stage = Stage::Scan;
            }
            Stage::Scan => {
                if self.groups.is_empty() {
                    for _ in self.descriptors {
                        control.check()?;
                        account.work(1);
                        self.groups.push(Group {
                            indices: AccountedVec::new(account, MemoryKind::Workspace),
                            drop_name: false,
                        });
                    }
                }
                if self.input == self.plan.inputs.len() {
                    self.stage = Stage::Sort;
                    return Ok(Action::Continue);
                }
                if !self.waiting {
                    return Ok(self.request(self.input));
                }
                self.waiting = false;
                let group = &mut self.groups[self.plan.bindings[self.input]];
                let _pending_samples = self.pending_samples.take();
                match std::mem::replace(&mut self.pending, Pending::Empty) {
                    Pending::Empty => (),
                    value => {
                        let flag = self.plan.inputs[self.input].drop_name;
                        if group.indices.is_empty() {
                            group.drop_name = flag;
                        } else {
                            group.drop_name |= flag;
                        }
                        match value {
                            Pending::Float(_) => group.indices.push(self.input),
                            Pending::Histogram => self.ignored = true,
                            Pending::Empty => unreachable!(),
                        }
                    }
                }
                self.input += 1;
            }
            Stage::Sort => {
                // Invalid q still consumes every body head, flag and ignored histogram first.
                if self.parameter.is_nan()
                    || !(0.0..=1.0).contains(&self.parameter)
                    || self.group == self.groups.len()
                {
                    self.publish(control, account)?;
                } else if self.groups[self.group].indices.len() > 1 {
                    self.program =
                        Some(Program::sort(self.groups[self.group].indices.len() as u64));
                    self.stage = Stage::Program;
                } else {
                    self.group += 1;
                }
            }
            Stage::Program => {
                if self.comparison.is_some() {
                    if self.waiting {
                        self.waiting = false;
                        let value = self.float()?;
                        let comparison = self.comparison.as_mut().expect("quantile comparison");
                        comparison.values[comparison.position] = value;
                        comparison.samples[comparison.position] = self.pending_samples.take();
                        comparison.position += 1;
                    }
                    let comparison = self.comparison.as_ref().expect("quantile comparison");
                    if comparison.position < 2 {
                        return Ok(self.request(comparison.inputs[comparison.position]));
                    }
                    let [left, right] = comparison.values;
                    self.reply = Some(left.is_nan() || left < right);
                    self.comparison = None;
                    return Ok(Action::Continue);
                }
                match self
                    .program
                    .as_mut()
                    .expect("quantile sort program")
                    .next(self.reply.take())
                {
                    Operation::Step => (),
                    Operation::Swap(left, right) => {
                        let indices = &mut self.groups[self.group].indices;
                        let (left, right) = (
                            physical_index(left, indices.len()),
                            physical_index(right, indices.len()),
                        );
                        indices.swap(left, right);
                    }
                    Operation::Less(left, right) => {
                        let indices = &self.groups[self.group].indices;
                        let (left, right) = (
                            physical_index(left, indices.len()),
                            physical_index(right, indices.len()),
                        );
                        self.comparison = Some(Comparison {
                            inputs: [indices[left], indices[right]],
                            values: [0.0; 2],
                            samples: [None, None],
                            position: 0,
                        });
                    }
                    Operation::Done => {
                        self.program = None;
                        self.group += 1;
                        self.stage = Stage::Sort;
                    }
                }
            }
            Stage::Emit => {
                let Some(group) = self
                    .snapshot
                    .as_ref()
                    .expect("quantile snapshot")
                    .groups
                    .get(self.group)
                else {
                    self.stage = Stage::Done;
                    return Ok(Action::Continue);
                };
                if group.indices.is_empty() {
                    self.group += 1;
                    return Ok(Action::Continue);
                }
                if self.parameter.is_nan() {
                    return self.emit(NAN, control, account);
                }
                if self.parameter < 0.0 {
                    return self.emit(f64::NEG_INFINITY, control, account);
                }
                if self.parameter > 1.0 {
                    return self.emit(f64::INFINITY, control, account);
                }
                if self.comparison.is_none() {
                    let last = (group.indices.len() - 1) as f64;
                    let rank = self.parameter * last;
                    let lower = floor(rank).max(0.0);
                    let upper = (lower + 1.0).min(last);
                    self.weight = rank - floor(rank);
                    self.comparison = Some(Comparison {
                        inputs: [group.indices[lower as usize], group.indices[upper as usize]],
                        values: [0.0; 2],
                        samples: [None, None],
                        position: 0,
                    });
                }
                if self.waiting {
                    self.waiting = false;
                    let value = self.float()?;
                    let comparison = self.comparison.as_mut().expect("quantile endpoints");
                    comparison.values[comparison.position] = value;
                    comparison.samples[comparison.position] = self.pending_samples.take();
                    comparison.position += 1;
                }
                let comparison = self.comparison.as_ref().expect("quantile endpoints");
                if comparison.position < 2 {
                    return Ok(self.request(comparison.inputs[comparison.position]));
                }
                let [lower, upper] = self.comparison.take().expect("quantile endpoints").values;
                let lower_product = source_mul(1.0 - self.weight, lower);
                let upper_product = source_mul(upper, self.weight);
                return self.emit(source_add(upper_product, lower_product), control, account);
            }
            Stage::Done => {
                let ignored = self
                    .snapshot
                    .as_ref()
                    .map_or(self.ignored, |snapshot| snapshot.ignored);
                return Ok(Action::Done(ignored.then_some(
                    AnnotationKind::HistogramIgnoredInAggregation(
                        crate::frontend::token::TokenKind::QUANTILE,
                    ),
                )));
            }
        }
        Ok(Action::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting, result::OwnedLabels, sample_labels::SampleLabels,
        storage::LabelRef,
    };

    fn published<'a>(
        plan: &'a Plan,
        descriptors: &'a [GroupDescriptor],
        drop_name: bool,
        account: &QueryAccounting,
    ) -> Step<'a> {
        let mut step = Step::new(
            plan,
            descriptors,
            EvaluationOptions {
                time_ms: 3000,
                start_ms: 1000,
                end_ms: 3000,
                step_ms: 1000,
                lookback_ms: 1,
                start_timestamps: false,
                delayed_name_removal: false,
            },
            2,
            account,
        );
        let mut indices = AccountedVec::new(account, MemoryKind::Workspace);
        indices.push(0);
        step.groups.push(Group { indices, drop_name });
        step.publish(&Control::default(), account).unwrap();
        step
    }

    #[test]
    fn quantile_emission_borrows_frozen_groups_and_materializes_retained_labels() {
        for shape in 0..3 {
            for whole_drop_name in [None, Some(false), Some(true)] {
                for drop_name in [false, true] {
                    let account = EngineAccounting::default().query();
                    let control = Control::default();
                    let bytes = vec![0xff; 65_536];
                    let mut refs = vec![
                        LabelRef {
                            name: b"__name__",
                            value: b"m",
                        },
                        LabelRef {
                            name: b"id",
                            value: &bytes,
                        },
                    ];
                    if shape == 1 {
                        refs.insert(
                            1,
                            LabelRef {
                                name: b"empty",
                                value: b"",
                            },
                        );
                    } else if shape == 2 {
                        refs.clear();
                    }
                    let descriptors = [GroupDescriptor {
                        hash: 0,
                        labels: OwnedLabels::copy_from(&refs, &account, MemoryKind::Workspace),
                        whole_drop_name,
                    }];
                    let mut parameters = Parameters::new(true);
                    parameters.push(-1.0, &account);
                    let plan = Plan::new(parameters, &account);
                    let mut step = published(&plan, &descriptors, drop_name, &account);
                    let before = account.snapshot().memory;
                    let Action::Sample(sample) = step.advance(&control, &account).unwrap() else {
                        panic!("published quantile group must emit");
                    };
                    assert_eq!(sample.ordinal, 0);
                    assert_eq!(sample.source_time_ms, 3000);
                    assert_eq!(sample.source_start_time_ms, 0);
                    assert_eq!(sample.drop_name, whole_drop_name.unwrap_or(drop_name));
                    let OwnedValue::Float(value) = sample.value else {
                        panic!("quantile emits a float");
                    };
                    assert_eq!(value.to_bits(), f64::NEG_INFINITY.to_bits());
                    if shape == 1 {
                        assert!(matches!(sample.labels, SampleLabels::Owned(_)));
                        assert_eq!(sample.labels.0.len(), 2);
                        assert_eq!(descriptors[0].labels.0.len(), 3);
                        assert!(descriptors[0].labels.0[1].value.is_empty());
                    } else {
                        assert!(matches!(sample.labels, SampleLabels::Borrowed(labels)
                            if std::ptr::eq(labels, &descriptors[0].labels)));
                        assert_eq!(account.snapshot().memory, before);
                    }
                    let retained = sample.labels.into_owned(&account, &control).unwrap();
                    drop(sample.float_samples);
                    let mut replay = Step::new(&plan, &descriptors, step.options, 2, &account);
                    let Action::Sample(mut sample) = replay.advance(&control, &account).unwrap()
                    else {
                        panic!("completed snapshot must replay");
                    };
                    sample
                        .labels
                        .make_owned(&account, &control)
                        .unwrap()
                        .set(b"id", b"changed", &account, MemoryKind::Workspace, &control)
                        .unwrap();
                    assert_eq!(descriptors[0].labels.0.len(), refs.len());
                    if shape != 2 {
                        assert_eq!(
                            descriptors[0].labels.0.last().unwrap().value.as_ref(),
                            &bytes
                        );
                        assert_eq!(retained.0.last().unwrap().value.as_ref(), &bytes);
                    }
                    drop((sample, replay, step));
                    drop((plan, descriptors));
                    assert_eq!(retained.0.len(), if shape == 2 { 0 } else { 2 });
                    drop(retained);
                    account.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn quantile_emission_controls_cover_complete_label_walks_and_release() {
        for empty in [false, true] {
            let account = EngineAccounting::default().query();
            let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
            for index in 0..10_000 {
                labels.push(
                    format!("label_{index:05}").as_bytes(),
                    if empty && index == 9999 {
                        b""
                    } else {
                        b"value"
                    },
                    &account,
                    MemoryKind::Workspace,
                );
            }
            let descriptors = [GroupDescriptor {
                hash: 0,
                labels,
                whole_drop_name: None,
            }];
            let mut parameters = Parameters::new(true);
            parameters.push(-1.0, &account);
            let plan = Plan::new(parameters, &account);
            let mut step = published(&plan, &descriptors, false, &account);
            let before = account.snapshot().work;
            let sample = step.advance(&Control::default(), &account).unwrap();
            let work = account.snapshot().work - before;
            assert!(work > 8192);
            drop((sample, step));
            for budget in [0, 1, 8192, work] {
                let mut step = Step::new(
                    &plan,
                    &descriptors,
                    EvaluationOptions {
                        time_ms: 3000,
                        start_ms: 1000,
                        end_ms: 3000,
                        step_ms: 1000,
                        lookback_ms: 1,
                        start_timestamps: false,
                        delayed_name_removal: false,
                    },
                    2,
                    &account,
                );
                let control = Control::default();
                if budget == 0 {
                    control.cancellation.cancel();
                }
                let before = account.snapshot().memory;
                let result =
                    crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                        step.advance(&control, &account)
                    });
                assert!(matches!(
                    result,
                    Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                ));
                assert!(
                    account
                        .snapshot()
                        .memory
                        .iter()
                        .zip(before)
                        .all(|(now, old)| now.current == old.current)
                );
            }
            drop((plan, descriptors));
            account.snapshot().assert_memory_released();
        }
    }
}
