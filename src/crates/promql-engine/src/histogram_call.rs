// SPDX-License-Identifier: GPL-3.0-or-later
// Bounded call replay follows pinned promql/{engine,functions}.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::{AnnotationBuilder, AnnotationKind, Monotonicity},
    binary_labels,
    classic_mutation::{self, Mutation},
    classic_replay::{self, Bound, Operation, Replay},
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions, drop_reserved, metadata},
    evaluator_order,
    evaluator_roster::Descriptor,
    float_text::{FloatTextStyle, float_text},
    frontend::{
        Span,
        ast::{Expr, NodeId, ParsedExpression},
        float,
    },
    histogram_functions::{self, NativeHistogramInfo},
    label_hash,
    position_index::{PositionIndex, SignatureIndex},
    query_preparation::Prepared,
    result::{OwnedLabels, OwnedValue},
    storage::{Control, Interrupted, StorageError},
};
use std::{cell::RefCell, cmp::Ordering, ops::Range};

#[derive(Clone, Copy)]
pub(crate) enum Function {
    Quantile,
    Quantiles,
    Fraction,
}
impl Function {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "histogram_quantile" => Self::Quantile,
            "histogram_quantiles" => Self::Quantiles,
            "histogram_fraction" => Self::Fraction,
            _ => return None,
        })
    }
    pub(crate) fn vector_argument(self) -> usize {
        match self {
            Self::Quantile => 1,
            Self::Quantiles => 0,
            Self::Fraction => 2,
        }
    }
    fn scalar_argument(self, index: usize) -> usize {
        if matches!(self, Self::Quantiles) {
            index + 2
        } else {
            index
        }
    }
}

pub(crate) struct Arguments<'a> {
    function: Function,
    scalar_spans: [Span; 10],
    scalar_count: usize,
    vector_span: Span,
    destination: Option<&'a Expr>,
}
impl<'a> Arguments<'a> {
    pub(crate) fn new(
        function: Function,
        node: NodeId,
        expression: &'a ParsedExpression,
        prepared: &Prepared,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, Interrupted> {
        let Expr::Call { arguments, .. } = &expression.arena[node].expression else {
            unreachable!("typed histogram call")
        };
        let child = |index| {
            prepared
                .preprocessing
                .child(expression, node, arguments[index], control, account)
        };
        let mut result = Self {
            function,
            scalar_spans: [Span::default(); 10],
            scalar_count: if matches!(function, Function::Quantiles) {
                arguments.len() - 2
            } else {
                function.vector_argument()
            },
            vector_span: expression.arena[child(function.vector_argument())?].span,
            destination: if matches!(function, Function::Quantiles) {
                Some(&expression.arena[child(1)?].expression)
            } else {
                None
            },
        };
        for index in 0..result.scalar_count {
            control.check()?;
            account.work(1);
            result.scalar_spans[index] =
                expression.arena[child(function.scalar_argument(index))?].span;
        }
        Ok(result)
    }
}
struct Row {
    bound: Option<f64>,
    group: Option<usize>,
    native_group: Option<usize>,
}
struct Group {
    labels: OwnedLabels,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum BindingKey {
    Classic(usize),
    Native(usize),
}
pub(crate) struct Binding {
    key: BindingKey,
    occurrence: usize,
    labels: OwnedLabels,
}
pub(crate) struct Plan {
    rows: AccountedVec<Descriptor>,
    metadata: AccountedVec<Row>,
    groups: AccountedVec<Group>,
    bindings: AccountedVec<Binding>,
    ordered_streaming: bool,
}

fn lookup_group(
    keys: &[AccountedVec<u8>],
    index: &PositionIndex,
    key: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(Option<usize>, u64), Interrupted> {
    // Tiny sets avoid a full hash walk and a table allocation.
    if keys.len() <= 2 {
        for (position, prior) in keys.iter().enumerate() {
            if binary_labels::equal(key, prior, control, account)? {
                return Ok((Some(position), 0));
            }
        }
        return Ok((None, 0));
    }
    let hash = label_hash::bytes(key, control, account)?;
    let group = index.find(
        hash,
        |position| binary_labels::equal(key, &keys[position], control, account),
        control,
        account,
    )?;
    Ok((group, hash))
}

impl Plan {
    pub(crate) fn new(
        rows: AccountedVec<Descriptor>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, EvaluationError> {
        let mut result = Self {
            rows,
            metadata: AccountedVec::new(account, MemoryKind::Workspace),
            groups: AccountedVec::new(account, MemoryKind::Workspace),
            bindings: AccountedVec::new(account, MemoryKind::Workspace),
            ordered_streaming: false,
        };
        let context = AllocationContext::new(account, MemoryKind::Workspace);
        let mut keys = AccountedVec::<AccountedVec<u8>>::new(account, MemoryKind::Workspace);
        let mut key_index = PositionIndex::new(account);
        for row in &*result.rows {
            control.check()?;
            account.work(1);
            let mut bound = None;
            let mut group = None;
            if row.kinds & 1 != 0 {
                let parsed = float::parse_checked(
                    row.labels.get_checked(b"le", account, control)?,
                    Some(&context),
                    Some(account),
                    Some(control),
                )?;
                if parsed.error.is_none() {
                    bound = Some(parsed.value);
                    let mut key = AccountedVec::new(account, MemoryKind::Workspace);
                    binary_labels::append_without(&mut key, &row.labels, b"le", control, account)?;
                    let (found, hash) = lookup_group(&keys, &key_index, &key, control, account)?;
                    group = found;
                    control.check()?;
                    if group.is_none() {
                        let mut labels =
                            row.labels
                                .copy_checked(account, MemoryKind::Workspace, control)?;
                        labels.reset_checked(account, control)?;
                        labels.set(b"le", b"", account, MemoryKind::Workspace, control)?;
                        let position = result.groups.len();
                        group = Some(position);
                        result.groups.push(Group { labels });
                        keys.push(key);
                        if keys.len() >= 3 {
                            let first = if keys.len() == 3 { 0 } else { position };
                            for inserted in first..keys.len() {
                                let hash = if keys.len() == 3 {
                                    label_hash::bytes(&keys[inserted], control, account)?
                                } else {
                                    hash
                                };
                                key_index.insert_new(
                                    hash,
                                    inserted,
                                    |prior| label_hash::bytes(&keys[prior], control, account),
                                    control,
                                    account,
                                )?;
                                control.check()?;
                            }
                        }
                    }
                }
            }
            result.metadata.push(Row {
                bound,
                group,
                native_group: None,
            });
        }
        for (position, row) in result.rows.iter().enumerate() {
            if row.kinds & 2 == 0 {
                continue;
            }
            let mut key = AccountedVec::new(account, MemoryKind::Workspace);
            binary_labels::append(&mut key, &row.labels, None, control, account)?;
            let (group, _) = lookup_group(&keys, &key_index, &key, control, account)?;
            control.check()?;
            result.metadata[position].native_group = group;
        }
        control.check()?;
        Ok(result)
    }
    pub(crate) fn finish(&mut self, cache: Cache) {
        self.bindings = cache.bindings;
    }
    pub(crate) fn certify_ordered_streaming(
        &mut self,
        rows: &[Descriptor],
        sources: &[usize],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        control.check()?;
        if rows.is_empty() || rows.len() != self.bindings.len() {
            return Ok(());
        }
        for (ordinal, (row, binding)) in rows.iter().zip(&*self.bindings).enumerate() {
            control.check()?;
            account.work(1);
            if row.ordinal != ordinal
                || row.kinds != 1
                || !row.drop_name
                || row.heads.multiple()
                || !matches!(sources.get(row.sources.clone()), Some([source]) if *source == ordinal)
                || row.hash != label_hash::full(&binding.labels, control, account)?
                || !binary_labels::same(&row.labels, &binding.labels, control, account)?
            {
                return Ok(());
            }
        }
        control.check()?;
        self.ordered_streaming = true;
        Ok(())
    }
    pub(crate) fn streams_ordered(&self) -> bool {
        self.ordered_streaming
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
            if row.ordinal == sample.ordinal
                && binary_labels::same(&row.labels, &sample.labels, control, account)?
            {
                return Ok(position);
            }
        }
        Err(EvaluationError::Invalid(
            "histogram child replay row is missing",
        ))
    }
}
struct Live {
    group: usize,
    dropped: bool,
}
pub(crate) struct Cache {
    live: AccountedVec<Live>,
    bindings: AccountedVec<Binding>,
    pub(crate) callbacks: crate::annotations::CallbackAnnotations,
}
impl Cache {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            live: AccountedVec::new(account, MemoryKind::Workspace),
            bindings: AccountedVec::new(account, MemoryKind::Workspace),
            callbacks: crate::annotations::CallbackAnnotations::new(account),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Preparation<'a> {
    pub(crate) node: NodeId,
    pub(crate) cache: &'a RefCell<Cache>,
}
#[derive(Clone, Copy)]
struct Active {
    row: usize,
    kind: u8,
    skip: bool,
}
struct ActiveGroup {
    group: usize,
    bounds: Range<usize>,
    dropped: bool,
    skip: bool,
}

fn lookup_active_group(
    groups: &[ActiveGroup],
    index: &SignatureIndex,
    group: usize,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<usize>, Interrupted> {
    if groups.len() <= 4 {
        for (position, prior) in groups.iter().enumerate() {
            control.check()?;
            account.work(1);
            if prior.group == group {
                return Ok(Some(position));
            }
        }
        return Ok(None);
    }
    index.find(group, |position| groups[position].group, control, account)
}
#[derive(Clone, Copy)]
enum Phase {
    StartScalar(usize),
    Scalar(usize),
    StartVector,
    Vector,
    Begin,
    Choose,
    Native(usize),
    Classic(usize),
    Mutate(usize),
    Count {
        position: usize,
        row: usize,
        mutating: bool,
    },
    Finished,
}
pub(crate) enum Action<'a> {
    Child {
        argument: usize,
        ordinal: Option<&'a usize>,
    },
    Sample(EvaluatedSample),
    Done(Option<AnnotationBuilder>),
}
pub(crate) struct Step<'a> {
    plan: &'a Plan,
    arguments: Arguments<'a>,
    phase: Phase,
    parameters: [Option<f64>; 10],
    quantile: usize,
    active: AccountedVec<Active>,
    groups: AccountedVec<ActiveGroup>,
    bounds: AccountedVec<Bound>,
    choice: Option<BindingKey>,
    position: usize,
    replay: Option<Replay>,
    mutation: Option<Mutation>,
    prepared_length: usize,
    producers: crate::classic_mutation::Producers,
    pending: Option<OwnedValue>,
    pending_samples: Option<crate::statistics::OwnedSamples>,
    parameter_samples: [Option<crate::statistics::OwnedSamples>; 10],
    notes: Option<AnnotationBuilder>,
    seen: AccountedVec<u64>,
    duplicate: bool,
    selected: Option<&'a [usize]>,
}
impl<'a> Step<'a> {
    pub(crate) fn new(
        plan: &'a Plan,
        arguments: Arguments<'a>,
        logical: bool,
        selected: Option<&'a [usize]>,
        account: &QueryAccounting,
    ) -> Self {
        let phase = if matches!(arguments.function, Function::Quantiles) {
            Phase::StartVector
        } else {
            Phase::StartScalar(0)
        };
        Self {
            plan,
            arguments,
            phase,
            parameters: [None; 10],
            quantile: 0,
            active: AccountedVec::new(account, MemoryKind::Workspace),
            groups: AccountedVec::new(account, MemoryKind::Workspace),
            bounds: AccountedVec::new(account, MemoryKind::Workspace),
            choice: None,
            position: 0,
            replay: None,
            mutation: None,
            prepared_length: 0,
            producers: crate::classic_mutation::Producers::new(account),
            pending: None,
            pending_samples: None,
            parameter_samples: std::array::from_fn(|_| None),
            notes: logical.then(|| AnnotationBuilder::new(account)),
            seen: AccountedVec::new(account, MemoryKind::Workspace),
            duplicate: false,
            selected,
        }
    }
    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        match self.phase {
            Phase::Scalar(index) => {
                if self.parameters[index].is_none()
                    && let OwnedValue::Float(value) = sample.value
                {
                    self.parameters[index] = Some(value);
                    self.parameter_samples[index] = sample.float_samples;
                }
            }
            Phase::Vector => {
                self.active.push(Active {
                    row: self.plan.row(&sample, control, account)?,
                    kind: sample.value.kind_bit(),
                    skip: false,
                });
            }
            Phase::Native(row) | Phase::Count { row, .. } => {
                let kind = if matches!(self.phase, Phase::Native(_)) {
                    2
                } else {
                    1
                };
                if self.pending.is_none()
                    && sample.value.kind_bit() == kind
                    && self.plan.row(&sample, control, account)? == row
                {
                    self.pending = Some(sample.value);
                    self.pending_samples = sample.float_samples;
                }
            }
            _ => unreachable!("histogram phase consumes child"),
        }
        Ok(())
    }
    fn begin(
        &mut self,
        options: EvaluationOptions,
        preparation: Option<Preparation<'a>>,
        query: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        if matches!(self.arguments.function, Function::Quantiles)
            && !matches!(self.arguments.destination, Some(Expr::String(_)))
        {
            return Err(crate::label_functions::string_literal_error(
                control, account,
            )?);
        }
        for index in 0..self.arguments.scalar_count {
            let Some(q) = self.parameters[index] else {
                self.phase = Phase::Finished;
                return Ok(());
            };
            if !matches!(self.arguments.function, Function::Fraction)
                && (q.is_nan() || !(0.0..=1.0).contains(&q))
                && let Some(notes) = &mut self.notes
            {
                notes.add_invalid_quantile(
                    q,
                    self.arguments.scalar_spans[index],
                    query,
                    control,
                    account,
                )?;
            }
        }
        let mut group_index = SignatureIndex::new(account);
        for active in &*self.active {
            control.check()?;
            account.work(1);
            if active.kind != 1 {
                continue;
            }
            let row = &self.plan.rows[active.row];
            let Some(group) = self.plan.metadata[active.row].group else {
                if let Some(notes) = &mut self.notes {
                    let metric = metric_name(&row.labels, options, control, account)?;
                    notes.add_bad_bucket(
                        metric,
                        row.labels.get_checked(b"le", account, control)?,
                        self.arguments.vector_span,
                        query,
                        control,
                        account,
                    )?;
                }
                continue;
            };
            let found = lookup_active_group(&self.groups, &group_index, group, control, account)?;
            control.check()?;
            if let Some(position) = found {
                // Counts become ordered offsets after native suppression and enumeration.
                self.groups[position].bounds.end = self.groups[position]
                    .bounds
                    .end
                    .checked_add(1)
                    .expect("histogram bound count overflow");
                continue;
            }
            let mut dropped = false;
            if let Some(preparation) = preparation {
                let mut cache = preparation.cache.borrow_mut();
                let mut found = None;
                for (index, entry) in cache.live.iter().enumerate() {
                    control.check()?;
                    account.work(1);
                    if entry.group == group {
                        found = Some(index);
                        break;
                    }
                }
                if let Some(index) = found {
                    dropped = cache.live[index].dropped;
                } else {
                    cache.live.push(Live {
                        group,
                        dropped: false,
                    });
                }
            }
            self.groups.push(ActiveGroup {
                group,
                bounds: 0..1,
                dropped,
                skip: false,
            });
            if self.groups.len() == 5 {
                for position in 0..5 {
                    group_index.insert_new(
                        self.groups[position].group,
                        position,
                        self.plan.groups.len(),
                        |prior| self.groups[prior].group,
                        control,
                        account,
                    )?;
                    control.check()?;
                }
            } else if self.groups.len() > 5 {
                group_index.insert_new(
                    group,
                    self.groups.len() - 1,
                    self.plan.groups.len(),
                    |prior| self.groups[prior].group,
                    control,
                    account,
                )?;
                control.check()?;
            }
        }
        let mut skipped = 0;
        for active in &mut *self.active {
            if active.kind != 2 {
                continue;
            }
            let Some(group) = self.plan.metadata[active.row].native_group else {
                continue;
            };
            let found = lookup_active_group(&self.groups, &group_index, group, control, account)?;
            control.check()?;
            if let Some(position) = found
                && !self.groups[position].skip
            {
                self.groups[position].skip = true;
                skipped += 1;
                active.skip = true;
                if let Some(preparation) = preparation {
                    let mut cache = preparation.cache.borrow_mut();
                    for index in 0..cache.live.len() {
                        control.check()?;
                        account.work(1);
                        if cache.live[index].group == group {
                            cache.live.remove(index);
                            break;
                        }
                    }
                }
                if let Some(notes) = &mut self.notes {
                    notes.add(
                        AnnotationKind::MixedClassicNativeHistograms,
                        metric_name(
                            &self.plan.rows[active.row].labels,
                            options,
                            control,
                            account,
                        )?,
                        self.arguments.vector_span,
                        query,
                        control,
                        account,
                    )?;
                }
            }
        }
        // Enumeration changes positions; its lookup must be rebuilt afterward.
        drop(group_index);
        self.enumerate_groups(control, account)?;
        if self.groups.len() - skipped <= 4 {
            for group in &mut *self.groups {
                control.check()?;
                group.bounds.start = self.bounds.len();
                if !group.skip {
                    for active in &*self.active {
                        control.check()?;
                        account.work(1);
                        if active.kind == 1
                            && self.plan.metadata[active.row].group == Some(group.group)
                        {
                            self.bounds.push(Bound {
                                upper_bound: self.plan.metadata[active.row]
                                    .bound
                                    .expect("parsed classic bound"),
                                ordinal: active.row,
                            });
                        }
                    }
                }
                group.bounds.end = self.bounds.len();
            }
        } else {
            let mut group_index = SignatureIndex::new(account);
            let mut total = 0usize;
            for position in 0..self.groups.len() {
                control.check()?;
                account.work(1);
                let count = if self.groups[position].skip {
                    0
                } else {
                    group_index.insert_new(
                        self.groups[position].group,
                        position,
                        self.plan.groups.len(),
                        |prior| self.groups[prior].group,
                        control,
                        account,
                    )?;
                    control.check()?;
                    self.groups[position].bounds.end
                };
                let end = total
                    .checked_add(count)
                    .expect("histogram bound count overflow");
                self.groups[position].bounds = total..end;
                total = end;
            }
            assert!(
                total <= self.active.len(),
                "histogram bound count exceeds active rows"
            );
            self.bounds.reserve(total);
            let empty = [Bound {
                upper_bound: 0.0,
                ordinal: 0,
            }; 64];
            while self.bounds.len() < total {
                control.check()?;
                let count = (total - self.bounds.len()).min(empty.len());
                account.work(count as u64);
                self.bounds.extend_from_slice(&empty[..count]);
            }
            for active in &*self.active {
                control.check()?;
                account.work(1);
                if active.kind != 1 {
                    continue;
                }
                let Some(group) = self.plan.metadata[active.row].group else {
                    continue;
                };
                let found = group_index.find(
                    group,
                    |position| self.groups[position].group,
                    control,
                    account,
                )?;
                control.check()?;
                if let Some(position) = found {
                    let cursor = &mut self.groups[position].bounds;
                    assert!(cursor.start < cursor.end, "histogram bound cursor overflow");
                    // Preserve source order for the subsequent pinned numerical sort.
                    self.bounds[cursor.start] = Bound {
                        upper_bound: self.plan.metadata[active.row]
                            .bound
                            .expect("parsed classic bound"),
                        ordinal: active.row,
                    };
                    cursor.start += 1;
                }
            }
            let mut start = 0;
            for group in &mut *self.groups {
                control.check()?;
                account.work(1);
                assert_eq!(
                    group.bounds.start, group.bounds.end,
                    "histogram bound count mismatch"
                );
                group.bounds.start = start;
                start = group.bounds.end;
            }
        }
        control.check()?;
        self.phase = Phase::Choose;
        Ok(())
    }
    fn enumerate_groups(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        // Freeze each source enumeration before its emitted groups drop metadata.
        evaluator_order::sort_by_with(
            &mut self.groups,
            Some(control),
            Some(account),
            |left, right| {
                compare_stored(
                    &self.plan.groups[left.group].labels,
                    left.dropped,
                    &self.plan.groups[right.group].labels,
                    right.dropped,
                    control,
                    account,
                )
                .map(|order| order.then_with(|| left.group.cmp(&right.group)))
            },
        )
    }
    fn choose(
        &mut self,
        logical: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        if logical || matches!(self.arguments.function, Function::Quantiles) {
            loop {
                while self.position < self.active.len() + self.groups.len() {
                    control.check()?;
                    account.work(1);
                    let position = self.position;
                    self.position += 1;
                    if let Some(active) = self.active.get(position) {
                        if active.kind == 2 && !active.skip {
                            self.choice = Some(BindingKey::Native(active.row));
                            return Ok(true);
                        }
                    } else {
                        let group = &self.groups[position - self.active.len()];
                        if !group.skip {
                            self.choice = Some(BindingKey::Classic(group.group));
                            return Ok(true);
                        }
                    }
                }
                if !matches!(self.arguments.function, Function::Quantiles)
                    || self.quantile + 1 == self.arguments.scalar_count
                {
                    break;
                }
                self.quantile += 1;
                self.position = 0;
                self.enumerate_groups(control, account)?;
            }
        } else {
            // Unwrapped single-step calls expose this frozen native-first order to parents.
            while let Some(binding) = self.plan.bindings.get(self.position) {
                control.check()?;
                account.work(1);
                let ordinal = self.position;
                self.position += 1;
                if let Some(selected) = self.selected {
                    let mut accepted = false;
                    for selected in selected {
                        control.check()?;
                        account.work(1);
                        accepted |= *selected == ordinal;
                    }
                    if !accepted {
                        continue;
                    }
                }
                let mut present = false;
                match binding.key {
                    BindingKey::Native(row) => {
                        for active in &*self.active {
                            control.check()?;
                            account.work(1);
                            present |= active.kind == 2 && active.row == row && !active.skip;
                        }
                    }
                    BindingKey::Classic(group) => {
                        for active in &*self.groups {
                            control.check()?;
                            account.work(1);
                            present |= active.group == group && !active.skip;
                        }
                    }
                }
                if present {
                    self.choice = Some(binding.key);
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    fn labels(
        &self,
        options: EvaluationOptions,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<OwnedLabels, EvaluationError> {
        let source = match self.choice.expect("chosen histogram") {
            BindingKey::Classic(group) => &self.plan.groups[group].labels,
            BindingKey::Native(row) => &self.plan.rows[row].labels,
        };
        let mut result = source.copy_checked(account, MemoryKind::Workspace, control)?;
        if !options.delayed_name_removal {
            drop_reserved(&mut result, false, account, control)?;
        }
        if let Some(destination) = self.destination() {
            result.reset_checked(account, control)?;
            let text = float_text(
                self.parameters[self.quantile].unwrap(),
                FloatTextStyle::OpenMetrics,
            );
            result.set(
                destination,
                text.as_bytes(),
                account,
                MemoryKind::Workspace,
                control,
            )?;
            crate::label_encoding::finalized(&result, control, account)?;
        }
        Ok(result)
    }
    fn destination(&self) -> Option<&[u8]> {
        match self.arguments.destination {
            Some(Expr::String(name)) => Some(name),
            _ => None,
        }
    }
    fn binding_matches(
        &self,
        entry: &Binding,
        key: BindingKey,
        occurrence: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        if entry.key != key || entry.occurrence != occurrence {
            return Ok(false);
        }
        if let Some(destination) = self.destination() {
            let text = float_text(
                self.parameters[self.quantile].unwrap(),
                FloatTextStyle::OpenMetrics,
            );
            return binary_labels::equal(
                entry.labels.get_checked(destination, account, control)?,
                text.as_bytes(),
                control,
                account,
            );
        }
        Ok(true)
    }
    fn occurrence(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<usize, Interrupted> {
        let mut count = 0;
        let current = float_text(
            self.parameters[self.quantile].unwrap(),
            FloatTextStyle::OpenMetrics,
        );
        for value in &self.parameters[..self.quantile] {
            control.check()?;
            account.work(1);
            count +=
                usize::from(float_text(value.unwrap(), FloatTextStyle::OpenMetrics) == current);
        }
        Ok(count)
    }
    fn emit(
        &mut self,
        value: f64,
        samples: Option<crate::statistics::OwnedSamples>,
        options: EvaluationOptions,
        preparation: Option<Preparation<'a>>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<EvaluatedSample, EvaluationError> {
        let key = self.choice.expect("chosen histogram");
        // Delayed instant parents may consume repeated labels before final cleanup.
        let occurrence = self.occurrence(control, account)?;
        let (ordinal, labels) = if let Some(preparation) = preparation {
            let mut cache = preparation.cache.borrow_mut();
            let mut found = None;
            for (position, entry) in cache.bindings.iter().enumerate() {
                control.check()?;
                account.work(1);
                if self.binding_matches(entry, key, occurrence, control, account)? {
                    found = Some(position);
                    break;
                }
            }
            let ordinal = if let Some(ordinal) = found {
                ordinal
            } else {
                let labels = self.labels(options, control, account)?;
                let ordinal = cache.bindings.len();
                cache.bindings.push(Binding {
                    key,
                    occurrence,
                    labels,
                });
                ordinal
            };
            (
                ordinal,
                cache.bindings[ordinal].labels.copy_checked(
                    account,
                    MemoryKind::Workspace,
                    control,
                )?,
            )
        } else {
            let mut found = None;
            for (position, entry) in self.plan.bindings.iter().enumerate() {
                control.check()?;
                account.work(1);
                if self.binding_matches(entry, key, occurrence, control, account)? {
                    found = Some(position);
                    break;
                }
            }
            let ordinal = found.ok_or(EvaluationError::Invalid(
                "frozen histogram label binding is missing",
            ))?;
            (
                ordinal,
                self.plan.bindings[ordinal].labels.copy_checked(
                    account,
                    MemoryKind::Workspace,
                    control,
                )?,
            )
        };
        let hash = label_hash::full(&labels, control, account)?;
        for previous in &*self.seen {
            control.check()?;
            account.work(1);
            self.duplicate |= *previous == hash;
        }
        self.seen.push(hash);
        self.phase = Phase::Choose;
        Ok(EvaluatedSample {
            ordinal,
            labels,
            float_samples: Some(
                samples.unwrap_or_else(|| crate::statistics::OwnedSamples::new(account, 1)),
            ),
            value: OwnedValue::Float(value),
            source_time_ms: options.time_ms,
            source_start_time_ms: 0,
            drop_name: true,
        })
    }
    fn drop_live(
        &mut self,
        preparation: Option<Preparation<'a>>,
        options: EvaluationOptions,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if !options.delayed_name_removal
            && let BindingKey::Classic(group) = self.choice.expect("chosen histogram")
        {
            for entry in &mut *self.groups {
                control.check()?;
                account.work(1);
                if entry.group == group {
                    entry.dropped = true;
                    break;
                }
            }
            if let Some(preparation) = preparation {
                for entry in &mut *preparation.cache.borrow_mut().live {
                    control.check()?;
                    account.work(1);
                    if entry.group == group {
                        entry.dropped = true;
                        break;
                    }
                }
            }
        }
        Ok(())
    }
    pub(crate) fn advance(
        &mut self,
        node: NodeId,
        options: EvaluationOptions,
        preparation: Option<Preparation<'a>>,
        query: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action<'a>, EvaluationError> {
        let preparation = preparation.filter(|value| value.node == node);
        loop {
            control.check()?;
            account.work(1);
            match self.phase {
                Phase::StartScalar(argument) => {
                    self.phase = Phase::Scalar(argument);
                    return Ok(Action::Child {
                        argument: self.arguments.function.scalar_argument(argument),
                        ordinal: None,
                    });
                }
                Phase::Scalar(argument) => {
                    self.phase = if argument + 1 < self.arguments.scalar_count {
                        Phase::StartScalar(argument + 1)
                    } else {
                        if matches!(self.arguments.function, Function::Quantiles) {
                            Phase::Begin
                        } else {
                            Phase::StartVector
                        }
                    };
                }
                Phase::StartVector => {
                    self.phase = Phase::Vector;
                    return Ok(Action::Child {
                        argument: self.arguments.function.vector_argument(),
                        ordinal: None,
                    });
                }
                Phase::Vector => {
                    self.phase = if matches!(self.arguments.function, Function::Quantiles) {
                        Phase::StartScalar(0)
                    } else {
                        Phase::Begin
                    }
                }
                Phase::Begin => self.begin(options, preparation, query, control, account)?,
                Phase::Choose => {
                    if !self.choose(preparation.is_some(), control, account)? {
                        self.phase = Phase::Finished;
                        continue;
                    }
                    match self.choice.expect("chosen histogram") {
                        BindingKey::Native(row) => {
                            self.phase = Phase::Native(row);
                            return Ok(Action::Child {
                                argument: self.arguments.function.vector_argument(),
                                ordinal: Some(&self.plan.rows[row].ordinal),
                            });
                        }
                        BindingKey::Classic(group) => {
                            let mut position = None;
                            for (index, active) in self.groups.iter().enumerate() {
                                control.check()?;
                                account.work(1);
                                if active.group == group {
                                    position = Some(index);
                                    break;
                                }
                            }
                            let position = position.expect("active classic group");
                            if matches!(self.arguments.function, Function::Quantiles) {
                                self.mutation = Some(Mutation::new(
                                    self.parameters[self.quantile].unwrap(),
                                    &mut self.bounds[self.groups[position].bounds.clone()],
                                    account,
                                    || {
                                        control.check()?;
                                        account.work(1);
                                        Ok::<_, Interrupted>(())
                                    },
                                )?);
                                self.phase = Phase::Mutate(position);
                                continue;
                            }
                            if matches!(self.arguments.function, Function::Fraction) {
                                self.drop_live(preparation, options, control, account)?;
                            }
                            let operation = match self.arguments.function {
                                Function::Quantile => {
                                    Operation::Quantile(self.parameters[0].unwrap())
                                }
                                Function::Fraction => Operation::Fraction(
                                    self.parameters[0].unwrap(),
                                    self.parameters[1].unwrap(),
                                ),
                                Function::Quantiles => unreachable!("plural mutation continuation"),
                            };
                            self.replay = Some(Replay::new(
                                operation,
                                &mut self.bounds[self.groups[position].bounds.clone()],
                                account,
                                || {
                                    control.check()?;
                                    account.work(1);
                                    Ok::<_, Interrupted>(())
                                },
                            )?);
                            self.phase = Phase::Classic(position);
                        }
                    }
                }
                Phase::Native(row) => {
                    let Some(OwnedValue::Histogram(histogram)) = self.pending.take() else {
                        return Err(EvaluationError::Invalid(
                            "histogram native replay value is missing",
                        ));
                    };
                    let poll = || {
                        control.check()?;
                        account.work(1);
                        Ok::<_, StorageError>(())
                    };
                    let result = match self.arguments.function {
                        Function::Quantile | Function::Quantiles => {
                            histogram_functions::histogram_quantile_with(
                                self.parameters[self.quantile].unwrap(),
                                histogram.as_ref(),
                                crate::histogram_work::HistogramWork::new(control, account),
                                poll,
                            )?
                        }
                        Function::Fraction => histogram_functions::histogram_fraction_with(
                            self.parameters[0].unwrap(),
                            self.parameters[1].unwrap(),
                            histogram.as_ref(),
                            crate::histogram_work::HistogramWork::new(control, account),
                            poll,
                        )?,
                    };
                    if let Some(info) = result.info
                        && let Some(notes) = &mut self.notes
                    {
                        let kind = match info {
                            NativeHistogramInfo::QuantileNaNResult => {
                                AnnotationKind::NativeHistogramQuantileNaNResult
                            }
                            NativeHistogramInfo::QuantileNaNSkew => {
                                AnnotationKind::NativeHistogramQuantileNaNSkew
                            }
                            NativeHistogramInfo::FractionNaNs => {
                                AnnotationKind::NativeHistogramFractionNaNs
                            }
                        };
                        notes.add(
                            kind,
                            metric_name(&self.plan.rows[row].labels, options, control, account)?,
                            self.arguments.vector_span,
                            query,
                            control,
                            account,
                        )?;
                    }
                    return Ok(Action::Sample(self.emit(
                        result.value,
                        None,
                        options,
                        preparation,
                        control,
                        account,
                    )?));
                }
                Phase::Classic(position) => {
                    let mut bounds = self.groups[position].bounds.clone();
                    if matches!(self.arguments.function, Function::Quantiles) {
                        bounds.end = bounds.start + self.prepared_length;
                    }
                    match self
                        .replay
                        .as_mut()
                        .expect("classic count continuation")
                        .next(&self.bounds[bounds], || {
                            control.check()?;
                            account.work(1);
                            Ok::<_, Interrupted>(())
                        })? {
                        classic_replay::Action::Count(row) => {
                            if row >= self.plan.rows.len() {
                                let value = *self.producers.get(row - self.plan.rows.len()).ok_or(
                                    EvaluationError::Invalid(
                                        "classic producer reference is missing",
                                    ),
                                )?;
                                self.replay.as_mut().expect("classic replay").receive(
                                    value,
                                    Some(crate::statistics::OwnedSamples::new(account, 1)),
                                );
                                continue;
                            }
                            self.phase = Phase::Count {
                                position,
                                row,
                                mutating: false,
                            };
                            // The immutable plan owns this reference throughout the child frame.
                            return Ok(Action::Child {
                                argument: self.arguments.function.vector_argument(),
                                ordinal: Some(&self.plan.rows[row].ordinal),
                            });
                        }
                        classic_replay::Action::Done(parts) => {
                            let result = parts.result;
                            if result.forced_monotonicity
                                && let Some(notes) = &mut self.notes
                            {
                                notes.add_monotonicity(
                                    metric_name(
                                        &self.plan.groups[self.groups[position].group].labels,
                                        options,
                                        control,
                                        account,
                                    )?,
                                    self.arguments.vector_span,
                                    Monotonicity::new(
                                        options.time_ms,
                                        result.min_bound,
                                        result.max_bound,
                                        result.max_decrease,
                                    ),
                                    query,
                                    control,
                                    account,
                                )?;
                            }
                            self.drop_live(preparation, options, control, account)?;
                            self.replay = None;
                            return Ok(Action::Sample(self.emit(
                                result.value,
                                Some(parts.samples),
                                options,
                                preparation,
                                control,
                                account,
                            )?));
                        }
                    }
                }
                Phase::Mutate(position) => {
                    let bounds = self.groups[position].bounds.clone();
                    match self.mutation.as_mut().expect("classic mutation").next(
                        &mut self.bounds[bounds.clone()],
                        &mut self.producers,
                        self.plan.rows.len(),
                        || {
                            control.check()?;
                            account.work(1);
                            Ok(())
                        },
                    )? {
                        classic_mutation::Action::Count(row) => {
                            if row >= self.plan.rows.len() {
                                let value = *self.producers.get(row - self.plan.rows.len()).ok_or(
                                    EvaluationError::Invalid(
                                        "classic producer reference is missing",
                                    ),
                                )?;
                                self.mutation.as_mut().expect("classic mutation").receive(
                                    value,
                                    Some(crate::statistics::OwnedSamples::new(account, 1)),
                                    &mut self.bounds[bounds],
                                );
                                continue;
                            }
                            self.phase = Phase::Count {
                                position,
                                row,
                                mutating: true,
                            };
                            return Ok(Action::Child {
                                argument: self.arguments.function.vector_argument(),
                                ordinal: Some(&self.plan.rows[row].ordinal),
                            });
                        }
                        classic_mutation::Action::Done { length, result } => {
                            self.prepared_length = length;
                            self.replay = Some(Replay::from_prepared(
                                self.parameters[self.quantile].unwrap(),
                                &self.bounds[bounds.start..bounds.start + length],
                                result,
                            ));
                            self.mutation = None;
                            self.phase = Phase::Classic(position);
                        }
                    }
                }
                Phase::Count {
                    position, mutating, ..
                } => {
                    let Some(OwnedValue::Float(value)) = self.pending.take() else {
                        return Err(EvaluationError::Invalid(
                            "histogram classic replay value is missing",
                        ));
                    };
                    if mutating {
                        self.mutation.as_mut().expect("classic mutation").receive(
                            value,
                            self.pending_samples.take(),
                            &mut self.bounds[self.groups[position].bounds.clone()],
                        );
                        self.phase = Phase::Mutate(position);
                    } else {
                        self.replay
                            .as_mut()
                            .expect("classic count continuation")
                            .receive(value, self.pending_samples.take());
                        self.phase = Phase::Classic(position);
                    }
                }
                Phase::Finished => {
                    if self.duplicate
                        && (!options.delayed_name_removal || options.start_ms != options.end_ms)
                    {
                        return Err(EvaluationError::Invalid(
                            "vector cannot contain metrics with the same labelset",
                        ));
                    }
                    return Ok(Action::Done(self.notes.take()));
                }
            }
        }
    }
}

fn metric_name<'a>(
    labels: &'a OwnedLabels,
    options: EvaluationOptions,
    control: &Control,
    account: &QueryAccounting,
) -> Result<&'a [u8], Interrupted> {
    if options.delayed_name_removal {
        labels.get_checked(b"__name__", account, control)
    } else {
        Ok(b"")
    }
}

fn compare_stored(
    left: &OwnedLabels,
    left_dropped: bool,
    right: &OwnedLabels,
    right_dropped: bool,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Ordering, Interrupted> {
    let mut left = left
        .0
        .iter()
        .filter(|label| !left_dropped || !metadata(&label.name));
    let mut right = right
        .0
        .iter()
        .filter(|label| !right_dropped || !metadata(&label.name));
    loop {
        control.check()?;
        account.work(1);
        match (left.next(), right.next()) {
            (Some(left), Some(right)) => {
                for (left, right) in [(&left.name, &right.name), (&left.value, &right.value)] {
                    let order = binary_labels::compare(left, right, control, account)?;
                    if !order.is_eq() {
                        return Ok(order);
                    }
                }
            }
            (None, None) => return Ok(Ordering::Equal),
            (None, _) => return Ok(Ordering::Less),
            (_, None) => return Ok(Ordering::Greater),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounting::EngineAccounting, evaluator_roster::HeadSummary};

    type Labels<'a> = &'a [(&'a [u8], &'a [u8])];
    type Spec<'a> = (u8, Labels<'a>, Option<usize>, Option<usize>);

    fn row(
        rows: &mut AccountedVec<Descriptor>,
        kinds: u8,
        labels: Labels<'_>,
        account: &QueryAccounting,
    ) {
        let mut labels = labels.to_vec();
        labels.sort_by(|left, right| left.0.cmp(right.0));
        let mut owned = OwnedLabels::new(account, MemoryKind::Workspace);
        for (name, value) in labels {
            owned.push(name, value, account, MemoryKind::Workspace);
        }
        let position = rows.len();
        rows.push(Descriptor {
            labels: owned,
            ordinal: position,
            hash: 0,
            drop_name: false,
            first_order: position,
            sources: position..position + 1,
            kinds,
            heads: HeadSummary::default(),
        });
    }

    #[test]
    fn constructor_collisions_keep_first_groups_and_exact_native_correspondence() {
        const FIRST: &[u8] = b"Aaaaa\x00\x00\x00\x00\x00\x00\x00\x00";
        const SECOND: &[u8] = b"Bbbbb\x1d\x3c\xa5\x95\x96\x2f\x43\xdd";
        for prefix in [0, 2] {
            for native_first in [false, true] {
                for second_first in [false, true] {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let mut encoded = Vec::new();
                    for value in [FIRST, SECOND] {
                        let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
                        labels.push(b"a", value, &account, MemoryKind::Workspace);
                        let mut key = AccountedVec::new(&account, MemoryKind::Workspace);
                        binary_labels::append(&mut key, &labels, None, &control, &account).unwrap();
                        assert_eq!(
                            label_hash::bytes(&key, &control, &account).unwrap(),
                            0x9c71_0b2e_3747_e3bd
                        );
                        encoded.push(key);
                    }
                    assert_ne!(&*encoded[0], &*encoded[1]);
                    drop(encoded);
                    let values = if second_first {
                        [SECOND, FIRST]
                    } else {
                        [FIRST, SECOND]
                    };
                    let mut rows = AccountedVec::new(&account, MemoryKind::Workspace);
                    let mut expected = Vec::new();
                    {
                        let mut push = |kinds, labels: Labels<'_>, group, native| {
                            row(&mut rows, kinds, labels, &account);
                            expected.push((group, native));
                        };
                        let native_ids = [
                            prefix + usize::from(second_first),
                            prefix + usize::from(!second_first),
                        ];
                        if native_first {
                            for (value, group) in [FIRST, SECOND].into_iter().zip(native_ids) {
                                push(2, &[(b"a", value)], None, Some(group));
                            }
                        }
                        for group in 0..prefix {
                            let value = format!("prior-{group}");
                            for bound in [b"1".as_slice(), b"+Inf"] {
                                push(
                                    1,
                                    &[(b"a", value.as_bytes()), (b"le", bound)],
                                    Some(group),
                                    None,
                                );
                            }
                        }
                        for (index, value) in values.into_iter().enumerate() {
                            for bound in [b"1".as_slice(), b"+Inf", b"2"] {
                                push(
                                    1,
                                    &[(b"a", value), (b"le", bound)],
                                    Some(prefix + index),
                                    None,
                                );
                            }
                        }
                        if !native_first {
                            for (value, group) in [FIRST, SECOND].into_iter().zip(native_ids) {
                                push(2, &[(b"a", value)], None, Some(group));
                            }
                        }
                    }
                    let plan = Plan::new(rows, &control, &account).unwrap();
                    assert_eq!(plan.groups.len(), prefix + 2);
                    assert_eq!(plan.metadata.len(), expected.len());
                    for (metadata, (group, native)) in plan.metadata.iter().zip(expected) {
                        assert_eq!((metadata.group, metadata.native_group), (group, native));
                    }
                    for (index, value) in values.into_iter().enumerate() {
                        assert_eq!(
                            plan.groups[prefix + index]
                                .labels
                                .get_checked(b"a", &account, &control)
                                .unwrap(),
                            value
                        );
                    }
                    drop(plan);
                    account.snapshot().assert_memory_released();
                }
            }
        }
        println!("{{\"histogram_constructor_collisions\":8}}");
    }

    #[test]
    fn constructor_keys_keep_empty_reserved_and_native_le_identity() {
        let cases: &[Spec<'_>] = &[
            (2, &[], None, Some(0)),
            (2, &[(b"a", b"")], None, Some(1)),
            (2, &[(b"le", b"")], None, None),
            (1, &[(b"le", b"1")], Some(0), None),
            (1, &[(b"a", b""), (b"le", b"+Inf")], Some(1), None),
            (
                1,
                &[
                    (b"__type__", b"counter"),
                    (b"__unit__", b"seconds"),
                    (b"le", b"2"),
                ],
                Some(2),
                None,
            ),
            (
                1,
                &[
                    (b"__type__", b"gauge"),
                    (b"__unit__", b"seconds"),
                    (b"le", b"3"),
                ],
                Some(3),
                None,
            ),
            (
                2,
                &[(b"__type__", b"counter"), (b"__unit__", b"seconds")],
                None,
                Some(2),
            ),
            (
                3,
                &[
                    (b"__type__", b"counter"),
                    (b"__unit__", b"seconds"),
                    (b"le", b"4"),
                ],
                Some(2),
                None,
            ),
            (3, &[(b"a", b""), (b"le", b"")], None, None),
            (1, &[(b"a", b""), (b"le", b"5")], Some(1), None),
            (0, &[(b"a", b"ignored")], None, None),
        ];
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut rows = AccountedVec::new(&account, MemoryKind::Workspace);
        for &(kind, labels, _, _) in cases {
            row(&mut rows, kind, labels, &account);
        }
        let plan = Plan::new(rows, &control, &account).unwrap();
        assert_eq!(plan.groups.len(), 4);
        assert!(plan.groups[0].labels.0.is_empty());
        assert!(plan.groups[1].labels.0.is_empty());
        for (metadata, &(_, _, group, native)) in plan.metadata.iter().zip(cases) {
            assert_eq!((metadata.group, metadata.native_group), (group, native));
        }
        assert_eq!(
            plan.groups[2]
                .labels
                .get_checked(b"__type__", &account, &control)
                .unwrap(),
            b"counter"
        );
        assert_eq!(
            plan.groups[3]
                .labels
                .get_checked(b"__type__", &account, &control)
                .unwrap(),
            b"gauge"
        );
        drop(plan);
        account.snapshot().assert_memory_released();
        println!("{{\"histogram_constructor_identity_rows\":12}}");
    }

    #[test]
    fn constructor_bound_parsing_keeps_skips_nan_infinities_and_signed_zero() {
        let cases: &[(Option<&[u8]>, Option<f64>)] = &[
            (None, None),
            (Some(b""), None),
            (Some(b"invalid"), None),
            (Some(b"NaN"), Some(f64::NAN)),
            (Some(b"+Inf"), Some(f64::INFINITY)),
            (Some(b"-Inf"), Some(f64::NEG_INFINITY)),
            (Some(b"-0"), Some(-0.0)),
            (Some(b"1"), Some(1.0)),
        ];
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut rows = AccountedVec::new(&account, MemoryKind::Workspace);
        for &(bound, _) in cases {
            let mut labels = vec![(b"a".as_slice(), b"same".as_slice())];
            if let Some(bound) = bound {
                labels.push((b"le", bound));
            }
            row(&mut rows, 1, &labels, &account);
        }
        let plan = Plan::new(rows, &control, &account).unwrap();
        assert_eq!(plan.groups.len(), 1);
        for (metadata, &(_, expected)) in plan.metadata.iter().zip(cases) {
            match (metadata.bound, expected) {
                (None, None) => assert_eq!(metadata.group, None),
                (Some(value), Some(expected)) => {
                    assert_eq!(metadata.group, Some(0));
                    if expected.is_nan() {
                        assert!(value.is_nan());
                    } else {
                        assert_eq!(value.to_bits(), expected.to_bits());
                    }
                }
                _ => panic!("bound parsing changed"),
            }
            assert_eq!(metadata.native_group, None);
        }
        drop(plan);
        account.snapshot().assert_memory_released();
        println!("{{\"histogram_constructor_bound_rows\":8}}");
    }

    #[test]
    fn constructor_tiny_lookup_avoids_hashing_long_keys() {
        for count in 0usize..=2 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let index = PositionIndex::new(&account);
            let mut keys = AccountedVec::new(&account, MemoryKind::Workspace);
            for position in 0..count {
                let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
                labels.push(
                    b"a",
                    &vec![b'x' + position as u8; 65536],
                    &account,
                    MemoryKind::Workspace,
                );
                let mut key = AccountedVec::new(&account, MemoryKind::Workspace);
                binary_labels::append(&mut key, &labels, None, &control, &account).unwrap();
                keys.push(key);
            }
            let key = keys.last().map_or(b"".as_slice(), |key| &**key);
            let before = account.snapshot();
            let (group, hash) = lookup_group(&keys, &index, key, &control, &account).unwrap();
            let after = account.snapshot();
            assert_eq!(group, count.checked_sub(1));
            assert_eq!(hash, 0);
            assert_eq!(
                after.work - before.work,
                key.len() as u64 + if count == 2 { 1024 } else { 0 }
            );
            assert_eq!(
                after.memory[MemoryKind::Workspace as usize].current,
                before.memory[MemoryKind::Workspace as usize].current
            );
            drop((index, keys));
            account.snapshot().assert_memory_released();
        }
        println!("{{\"histogram_constructor_tiny_cases\":3}}");
    }

    fn interrupt_rows(
        count: usize,
        last_kind: u8,
        account: &QueryAccounting,
    ) -> AccountedVec<Descriptor> {
        let mut rows = AccountedVec::new(account, MemoryKind::Workspace);
        for group in 0..count {
            let value = format!("{group:08}");
            for bound in [b"1".as_slice(), b"+Inf"] {
                row(
                    &mut rows,
                    1,
                    &[(b"a", value.as_bytes()), (b"le", bound)],
                    account,
                );
            }
        }
        let labels: Labels<'_> = match last_kind {
            0 => &[(b"a", b"ignored")],
            1 | 3 => &[(b"a", b"last"), (b"le", b"1")],
            2 => &[(b"a", b"00000000")],
            _ => unreachable!(),
        };
        row(&mut rows, last_kind, labels, account);
        rows
    }

    fn distribution_step<'a>(plan: &'a Plan, account: &QueryAccounting) -> Step<'a> {
        let mut step = Step::new(
            plan,
            Arguments {
                function: Function::Quantile,
                scalar_spans: [Span::default(); 10],
                scalar_count: 1,
                vector_span: Span::default(),
                destination: None,
            },
            false,
            None,
            account,
        );
        step.parameters[0] = Some(0.5);
        step.phase = Phase::Begin;
        step
    }

    fn distribution_options() -> EvaluationOptions {
        EvaluationOptions {
            time_ms: 2000,
            start_ms: 2000,
            end_ms: 2000,
            step_ms: 1000,
            lookback_ms: 300000,
            start_timestamps: false,
            delayed_name_removal: false,
        }
    }

    #[test]
    fn active_distribution_work_avoids_all_pairs() {
        let mut measured = Vec::new();
        for count in [128, 512] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let plan = Plan::new(interrupt_rows(count, 0, &account), &control, &account).unwrap();
            let mut step = distribution_step(&plan, &account);
            for row in 0..2 * count {
                step.active.push(Active {
                    row,
                    kind: 1,
                    skip: false,
                });
            }
            let before = account.snapshot().work;
            step.begin(distribution_options(), None, b"", &control, &account)
                .unwrap();
            let work = account.snapshot().work - before;
            assert!(matches!(step.phase, Phase::Choose));
            assert_eq!(step.groups.len(), count);
            assert_eq!(step.bounds.len(), 2 * count);
            for (position, group) in step.groups.iter().enumerate() {
                assert_eq!(group.group, position);
                assert_eq!(group.bounds, 2 * position..2 * position + 2);
                assert!(!group.skip && !group.dropped);
                let bounds = &step.bounds[group.bounds.clone()];
                assert_eq!(bounds[0].upper_bound.to_bits(), 1.0f64.to_bits());
                assert_eq!(bounds[1].upper_bound.to_bits(), f64::INFINITY.to_bits());
                assert_eq!(
                    (bounds[0].ordinal, bounds[1].ordinal),
                    (2 * position, 2 * position + 1)
                );
            }
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
            measured.push((count, work));
            println!("{{\"histogram_active_groups\":{count},\"begin_work\":{work}}}");
        }
        for (count, work) in measured {
            assert!(
                work <= (250 * count) as u64,
                "{count} groups used Begin work {work}"
            );
        }
    }

    #[test]
    fn active_distribution_keeps_sparse_order_skipped_ranges_and_native_phases() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut rows = AccountedVec::new(&account, MemoryKind::Workspace);
        for group in 0..18 {
            let value = format!("{:08}", 18 - group);
            for bound in [b"1".as_slice(), b"+Inf", b"-0", b"NaN"] {
                row(
                    &mut rows,
                    1,
                    &[
                        (b"__name__", b"h"),
                        (b"a", value.as_bytes()),
                        (b"le", bound),
                    ],
                    &account,
                );
            }
        }
        for value in [b"00000011".as_slice(), b"00000011", b"00000005", b"absent"] {
            row(
                &mut rows,
                2,
                &[(b"__name__", b"h"), (b"a", value)],
                &account,
            );
        }
        row(
            &mut rows,
            1,
            &[(b"__name__", b"h"), (b"a", b"bad")],
            &account,
        );
        let plan = Plan::new(rows, &control, &account).unwrap();
        assert_eq!(plan.groups.len(), 18);
        let cache = RefCell::new(Cache::new(&account));
        for group in [7, 4, 13, 0] {
            cache.borrow_mut().live.push(Live {
                group,
                dropped: group == 4,
            });
        }
        let mut step = distribution_step(&plan, &account);
        step.notes = Some(AnnotationBuilder::new(&account));
        for (row, kind) in [
            (72, 2),
            (65, 1),
            (16, 1),
            (28, 1),
            (6, 1),
            (40, 1),
            (53, 1),
            (64, 1),
            (19, 1),
            (41, 1),
            (29, 1),
            (5, 1),
            (52, 1),
            (40, 1),
            (4, 1),
            (73, 2),
            (74, 2),
            (75, 2),
            (68, 1),
            (69, 1),
            (76, 1),
        ] {
            step.active.push(Active {
                row,
                kind,
                skip: false,
            });
        }
        step.begin(
            distribution_options(),
            Some(Preparation {
                node: NodeId(0),
                cache: &cache,
            }),
            b"",
            &control,
            &account,
        )
        .unwrap();
        assert!(matches!(step.phase, Phase::Choose));
        let expected = [
            (17, 0..2, false, false),
            (16, 2..4, false, false),
            (13, 4..4, true, false),
            (10, 4..7, false, false),
            (7, 7..7, true, false),
            (1, 7..10, false, false),
            (4, 10..12, false, true),
        ];
        assert_eq!(step.groups.len(), expected.len());
        for (group, (id, bounds, skip, dropped)) in step.groups.iter().zip(expected) {
            assert_eq!(
                (group.group, group.bounds.clone(), group.skip, group.dropped),
                (id, bounds, skip, dropped)
            );
        }
        let expected_bounds = [
            (1.0f64.to_bits(), 68),
            (f64::INFINITY.to_bits(), 69),
            (f64::INFINITY.to_bits(), 65),
            (1.0f64.to_bits(), 64),
            (1.0f64.to_bits(), 40),
            (f64::INFINITY.to_bits(), 41),
            (1.0f64.to_bits(), 40),
            ((-0.0f64).to_bits(), 6),
            (f64::INFINITY.to_bits(), 5),
            (1.0f64.to_bits(), 4),
            (1.0f64.to_bits(), 16),
            (0x7ff8_0000_0000_0001, 19),
        ];
        assert_eq!(step.bounds.len(), expected_bounds.len());
        for (bound, expected) in step.bounds.iter().zip(expected_bounds) {
            assert_eq!((bound.upper_bound.to_bits(), bound.ordinal), expected);
        }
        assert!(step.active[0].skip && step.active[16].skip);
        assert!(!step.active[15].skip && !step.active[17].skip);
        let live: Vec<_> = cache
            .borrow()
            .live
            .iter()
            .map(|entry| (entry.group, entry.dropped))
            .collect();
        assert_eq!(
            live,
            [
                (4, true),
                (0, false),
                (16, false),
                (1, false),
                (10, false),
                (17, false)
            ]
        );
        let notes = step.notes.take().unwrap().finish();
        let kinds: Vec<_> = notes.iter().map(|note| note.kind).collect();
        assert_eq!(
            kinds,
            [
                AnnotationKind::BadBucketLabel,
                AnnotationKind::MixedClassicNativeHistograms
            ]
        );
        drop(notes);
        drop(step);
        drop((plan, cache));
        account.snapshot().assert_memory_released();
        println!("{{\"histogram_active_sparse_groups\":7,\"plan_groups\":18,\"exact_bounds\":12}}");
    }

    #[test]
    fn active_distribution_interruption_discards_partial_ranges_before_choose() {
        let mut interruptions = 0;
        for count in [1, 2, 128, 512] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let plan = Plan::new(interrupt_rows(count, 0, &account), &control, &account).unwrap();
            let mut step = distribution_step(&plan, &account);
            for row in 0..2 * count {
                step.active.push(Active {
                    row,
                    kind: 1,
                    skip: false,
                });
            }
            let before = account.snapshot().work;
            step.begin(distribution_options(), None, b"", &control, &account)
                .unwrap();
            let total = account.snapshot().work - before;
            drop(step);
            let mut budgets = vec![
                1,
                (total / 4).max(1),
                (total / 2).max(1),
                (total - 1).max(1),
                total,
            ];
            if total >= 8192 {
                budgets.push(8192);
            }
            budgets.sort_unstable();
            budgets.dedup();
            for budget in budgets {
                let control = Control::default();
                let mut step = distribution_step(&plan, &account);
                for row in 0..2 * count {
                    step.active.push(Active {
                        row,
                        kind: 1,
                        skip: false,
                    });
                }
                let before = account.snapshot().work;
                let outcome =
                    crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                        step.begin(distribution_options(), None, b"", &control, &account)
                    });
                assert!(
                    matches!(
                        outcome,
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    ),
                    "count={count},budget={budget}"
                );
                assert!(!matches!(step.phase, Phase::Choose));
                assert!(account.snapshot().work - before >= budget);
                drop(step);
                interruptions += 1;
            }
            drop(plan);
            account.snapshot().assert_memory_released();
        }
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let plan = Plan::new(
            AccountedVec::new(&account, MemoryKind::Workspace),
            &control,
            &account,
        )
        .unwrap();
        let mut step = distribution_step(&plan, &account);
        control.cancellation.cancel();
        assert!(matches!(
            step.begin(distribution_options(), None, b"", &control, &account),
            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
        ));
        assert!(!matches!(step.phase, Phase::Choose));
        drop(step);
        drop(plan);
        account.snapshot().assert_memory_released();
        println!(
            "{{\"histogram_active_interruptions\":{},\"empty_terminal_control\":true}}",
            interruptions
        );
    }

    #[test]
    fn constructor_interruption_releases_growth_and_prevents_terminal_publication() {
        let mut interruptions = 0;
        for count in [2, 128, 512] {
            for last_kind in 0..=3 {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let rows = interrupt_rows(count, last_kind, &account);
                let before = account.snapshot().work;
                let plan = Plan::new(rows, &control, &account).unwrap();
                let total = account.snapshot().work - before;
                assert_eq!(
                    plan.groups.len(),
                    count + usize::from(last_kind == 1 || last_kind == 3)
                );
                assert_eq!(
                    plan.metadata.last().unwrap().native_group,
                    (last_kind == 2).then_some(0)
                );
                drop(plan);
                account.snapshot().assert_memory_released();
                let mut budgets = vec![1, total / 4, total / 2, total - 1, total];
                if total >= 8192 {
                    budgets.push(8192);
                }
                budgets.sort_unstable();
                budgets.dedup();
                for budget in budgets {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let rows = interrupt_rows(count, last_kind, &account);
                    let before = account.snapshot().work;
                    let outcome = crate::test_storage::interrupt_after_work(
                        &account,
                        &control,
                        budget,
                        || Plan::new(rows, &control, &account),
                    );
                    assert!(
                        matches!(
                            &outcome,
                            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                        ),
                        "count={count},kind={last_kind},budget={budget}"
                    );
                    assert!(account.snapshot().work - before >= budget);
                    drop(outcome);
                    account.snapshot().assert_memory_released();
                    interruptions += 1;
                }
            }
        }
        println!("{{\"histogram_constructor_interruptions\":{interruptions}}}");
    }
}
