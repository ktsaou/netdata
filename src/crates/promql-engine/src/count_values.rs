// SPDX-License-Identifier: GPL-3.0-or-later
// Value-dependent grouping follows pinned promql/engine.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    binary_labels,
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_order,
    evaluator_roster::Descriptor,
    frontend::{
        ast::{Expr, NodeId, ParsedExpression},
        token::TokenKind,
    },
    label_functions, label_hash,
    query_preparation::Prepared,
    result::{OwnedLabels, OwnedValue, append_bytes_checked},
    storage::{Control, Interrupted},
};
use std::cell::RefCell;

pub(crate) fn is_count(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::Aggregate {
            operator: TokenKind::COUNT_VALUES,
            ..
        }
    )
}
struct Binding {
    labels: OwnedLabels,
    canonical: Option<(usize, usize)>,
}
pub(crate) struct Plan {
    destination: AccountedVec<u8>,
    grouping: AccountedVec<AccountedVec<u8>>,
    without: bool,
    bindings: AccountedVec<Binding>,
}
impl Plan {
    pub(crate) fn prepare(
        expression: &ParsedExpression,
        node: NodeId,
        prepared: &Prepared,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, EvaluationError> {
        let Expr::Aggregate {
            parameter: Some(parameter),
            grouping,
            without,
            ..
        } = &expression.arena[node].expression
        else {
            unreachable!("typed count_values parameter")
        };
        control.check()?;
        account.work(1);
        let parameter = prepared
            .preprocessing
            .child(expression, node, *parameter, control, account)?;
        let Expr::String(name) = &expression.arena[parameter].expression else {
            assert!(matches!(
                expression.arena[parameter].expression,
                Expr::Paren(_)
            ));
            return Err(label_functions::string_literal_error(control, account)?);
        };
        if !label_functions::valid_name(name, account, control)? {
            let mut message = AccountedVec::new(account, MemoryKind::Workspace);
            append_bytes_checked(&mut message, b"invalid label name ", account, control)?;
            binary_labels::quoted(&mut message, name, control, account)?;
            return Err(EvaluationError::Runtime(message));
        }
        let mut destination = AccountedVec::new(account, MemoryKind::Workspace);
        append_bytes_checked(&mut destination, name, account, control)?;
        let mut names = AccountedVec::new(account, MemoryKind::Workspace);
        for name in grouping.as_deref().unwrap_or(&[]) {
            let mut copy = AccountedVec::new(account, MemoryKind::Workspace);
            append_bytes_checked(&mut copy, name, account, control)?;
            names.push(copy);
        }
        if !without {
            let mut copy = AccountedVec::new(account, MemoryKind::Workspace);
            append_bytes_checked(&mut copy, &destination, account, control)?;
            names.push(copy);
        }
        evaluator_order::sort_by_with(&mut names, Some(control), Some(account), |left, right| {
            binary_labels::compare(left, right, control, account)
        })?;
        Ok(Self {
            destination,
            grouping: names,
            without: *without,
            bindings: AccountedVec::new(account, MemoryKind::Workspace),
        })
    }
    pub(crate) fn finish(&mut self, cache: Cache) {
        self.bindings = cache.bindings;
    }
    pub(crate) fn project(
        &mut self,
        descriptors: &[Descriptor],
        sources: &[usize],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        for descriptor in descriptors {
            control.check()?;
            account.work(1);
            let representative = sources[descriptor.sources.start];
            let binding = self
                .bindings
                .get(representative)
                .ok_or(EvaluationError::Invalid(
                    "count_values canonical binding is missing",
                ))?;
            if !binary_labels::same(&binding.labels, &descriptor.labels, control, account)? {
                return Err(EvaluationError::Invalid(
                    "count_values canonical labels changed",
                ));
            }
            for raw in &sources[descriptor.sources.clone()] {
                control.check()?;
                account.work(1);
                self.bindings
                    .get_mut(*raw)
                    .ok_or(EvaluationError::Invalid(
                        "count_values raw binding is missing",
                    ))?
                    .canonical = Some((descriptor.ordinal, representative));
            }
        }
        Ok(())
    }
}

pub(crate) struct Cache {
    bindings: AccountedVec<Binding>,
}
impl Cache {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            bindings: AccountedVec::new(account, MemoryKind::Workspace),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Preparation<'a> {
    pub(crate) node: NodeId,
    pub(crate) cache: &'a RefCell<Cache>,
}

struct Group {
    hash: u64,
    full_hash: u64,
    labels: OwnedLabels,
    count: i64,
    ordinal: usize,
}
pub(crate) struct Step<'a> {
    plan: &'a Plan,
    groups: AccountedVec<Group>,
    position: usize,
    project: bool,
}
impl<'a> Step<'a> {
    pub(crate) fn new(plan: &'a Plan, project: bool, account: &QueryAccounting) -> Self {
        Self {
            plan,
            groups: AccountedVec::new(account, MemoryKind::Workspace),
            position: 0,
            project,
        }
    }
    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let mut labels = sample.labels;
        labels.reset_checked(account, control)?;
        let mut value = AccountedVec::new(account, MemoryKind::Workspace);
        match &sample.value {
            OwnedValue::Float(value_float) => {
                crate::float_text::append_fixed(&mut value, *value_float, control, account)?
            }
            OwnedValue::Histogram(histogram) => {
                crate::histogram_text::append(&mut value, histogram.as_ref(), control, account)?
            }
        }
        labels.set(
            &self.plan.destination,
            &value,
            account,
            MemoryKind::Workspace,
            control,
        )?;
        // Builder encodes the modified metric before grouping can remove a destination.
        crate::label_encoding::finalized(&labels, control, account)?;
        labels.retain_labels_checked(account, control, |label| {
            let found = binary_labels::named(&label.name, &self.plan.grouping, control, account)?;
            Ok::<_, Interrupted>(if self.plan.without {
                &*label.name != b"__name__" && !found
            } else {
                found
            })
        })?;
        let hash = if !self.plan.without && self.plan.grouping.is_empty() {
            0
        } else {
            label_hash::grouping(&labels, control, account)?
        };
        for group in &mut *self.groups {
            control.check()?;
            account.work(1);
            if group.hash == hash {
                group.count = group.count.wrapping_add(1);
                return Ok(());
            }
        }
        self.groups.push(Group {
            hash,
            full_hash: 0,
            labels,
            count: 1,
            ordinal: 0,
        });
        Ok(())
    }
    pub(crate) fn finish(
        &mut self,
        options: EvaluationOptions,
        preparation: Option<Preparation<'_>>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        if !options.delayed_name_removal || options.start_ms != options.end_ms {
            for group in &mut *self.groups {
                group.full_hash = label_hash::full(&group.labels, control, account)?;
            }
            evaluator_order::sort(&mut self.groups, control, account, |left, right| {
                left.full_hash.cmp(&right.full_hash)
            })?;
            for pair in self.groups.windows(2) {
                control.check()?;
                account.work(1);
                if pair[0].full_hash == pair[1].full_hash {
                    return Err(EvaluationError::Invalid(
                        "vector cannot contain metrics with the same labelset",
                    ));
                }
            }
        }
        self.sort(control, account)?;
        for group in &mut *self.groups {
            if let Some(preparation) = preparation {
                let mut cache = preparation.cache.borrow_mut();
                group.ordinal = match binding(&cache.bindings, &group.labels, control, account)? {
                    Some(ordinal) => ordinal,
                    None => {
                        let ordinal = cache.bindings.len();
                        cache.bindings.push(Binding {
                            labels: group.labels.copy_checked(
                                account,
                                MemoryKind::Workspace,
                                control,
                            )?,
                            canonical: None,
                        });
                        ordinal
                    }
                };
            } else {
                let raw = binding(&self.plan.bindings, &group.labels, control, account)?.ok_or(
                    EvaluationError::Invalid("frozen count_values label binding is missing"),
                )?;
                group.ordinal = raw;
                if self.project
                    && let Some((ordinal, representative)) = self.plan.bindings[raw].canonical
                {
                    group.ordinal = ordinal;
                    let labels = &self.plan.bindings[representative].labels;
                    if !binary_labels::same(&group.labels, labels, control, account)? {
                        group.labels =
                            labels.copy_checked(account, MemoryKind::Workspace, control)?;
                    }
                }
            }
        }
        if self.project && preparation.is_none() {
            self.sort(control, account)?;
        }
        Ok(())
    }
    fn sort(&mut self, control: &Control, account: &QueryAccounting) -> Result<(), Interrupted> {
        evaluator_order::sort_by_with(
            &mut self.groups,
            Some(control),
            Some(account),
            |left, right| left.labels.compare_checked(&right.labels, account, control),
        )
    }
    pub(crate) fn next(
        &mut self,
        time_ms: i64,
        account: &QueryAccounting,
    ) -> Option<EvaluatedSample> {
        let group = self.groups.get_mut(self.position)?;
        self.position += 1;
        Some(EvaluatedSample {
            ordinal: group.ordinal,
            labels: std::mem::replace(
                &mut group.labels,
                OwnedLabels::new(account, MemoryKind::Workspace),
            ),
            value: OwnedValue::Float(group.count as f64),
            source_time_ms: time_ms,
            source_start_time_ms: 0,
            drop_name: false,
        })
    }
}

fn binding(
    bindings: &[Binding],
    labels: &OwnedLabels,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<usize>, Interrupted> {
    for (ordinal, candidate) in bindings.iter().enumerate() {
        control.check()?;
        account.work(1);
        if binary_labels::same(&candidate.labels, labels, control, account)? {
            return Ok(Some(ordinal));
        }
    }
    Ok(None)
}
