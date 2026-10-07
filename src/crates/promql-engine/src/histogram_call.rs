// SPDX-License-Identifier: GPL-3.0-or-later
// Bounded call replay follows pinned promql/{engine,functions}.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::{AnnotationBuilder, AnnotationKind, Monotonicity},
    binary_labels,
    classic_replay::{self, Bound, Operation, Replay},
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions, metadata},
    evaluator_order,
    evaluator_roster::Descriptor,
    frontend::{Span, ast::NodeId, float},
    histogram_functions::{self, NativeHistogramInfo},
    label_hash,
    result::{OwnedLabels, OwnedValue},
    storage::{Control, Interrupted},
};
use std::{cell::RefCell, cmp::Ordering, ops::Range};

#[derive(Clone, Copy)]
pub(crate) enum Function {
    Quantile,
    Fraction,
}
impl Function {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "histogram_quantile" => Self::Quantile,
            "histogram_fraction" => Self::Fraction,
            _ => return None,
        })
    }
    pub(crate) fn vector_argument(self) -> usize {
        match self {
            Self::Quantile => 1,
            Self::Fraction => 2,
        }
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
    labels: OwnedLabels,
}
pub(crate) struct Plan {
    rows: AccountedVec<Descriptor>,
    metadata: AccountedVec<Row>,
    groups: AccountedVec<Group>,
    bindings: AccountedVec<Binding>,
}
impl Plan {
    pub(crate) fn new(
        rows: AccountedVec<Descriptor>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, Interrupted> {
        let mut result = Self {
            rows,
            metadata: AccountedVec::new(account, MemoryKind::Workspace),
            groups: AccountedVec::new(account, MemoryKind::Workspace),
            bindings: AccountedVec::new(account, MemoryKind::Workspace),
        };
        let context = AllocationContext::new(account, MemoryKind::Workspace);
        let mut keys = AccountedVec::<AccountedVec<u8>>::new(account, MemoryKind::Workspace);
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
                    for (index, prior) in keys.iter().enumerate() {
                        if binary_labels::equal(&key, prior, control, account)? {
                            group = Some(index);
                            break;
                        }
                    }
                    if group.is_none() {
                        let mut labels =
                            row.labels
                                .copy_checked(account, MemoryKind::Workspace, control)?;
                        labels.reset_checked(account, control)?;
                        labels.set(b"le", b"", account, MemoryKind::Workspace, control)?;
                        group = Some(result.groups.len());
                        result.groups.push(Group { labels });
                        keys.push(key);
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
            for (group, candidate) in keys.iter().enumerate() {
                if binary_labels::equal(&key, candidate, control, account)? {
                    result.metadata[position].native_group = Some(group);
                    break;
                }
            }
        }
        Ok(result)
    }
    pub(crate) fn finish(&mut self, cache: Cache) {
        self.bindings = cache.bindings;
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
    Count { position: usize, row: usize },
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
    function: Function,
    scalar_span: Span,
    vector_span: Span,
    phase: Phase,
    parameters: [Option<f64>; 2],
    active: AccountedVec<Active>,
    groups: AccountedVec<ActiveGroup>,
    bounds: AccountedVec<Bound>,
    choice: Option<BindingKey>,
    position: usize,
    replay: Option<Replay>,
    pending: Option<OwnedValue>,
    notes: Option<AnnotationBuilder>,
    seen: AccountedVec<u64>,
    duplicate: bool,
    selected: Option<&'a [usize]>,
}
impl<'a> Step<'a> {
    pub(crate) fn new(
        plan: &'a Plan,
        function: Function,
        scalar_span: Span,
        vector_span: Span,
        logical: bool,
        selected: Option<&'a [usize]>,
        account: &QueryAccounting,
    ) -> Self {
        Self {
            plan,
            function,
            scalar_span,
            vector_span,
            phase: Phase::StartScalar(0),
            parameters: [None, None],
            active: AccountedVec::new(account, MemoryKind::Workspace),
            groups: AccountedVec::new(account, MemoryKind::Workspace),
            bounds: AccountedVec::new(account, MemoryKind::Workspace),
            choice: None,
            position: 0,
            replay: None,
            pending: None,
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
        if self.parameters[0].is_none()
            || matches!(self.function, Function::Fraction) && self.parameters[1].is_none()
        {
            self.phase = Phase::Finished;
            return Ok(());
        }
        if matches!(self.function, Function::Quantile) {
            let q = self.parameters[0].unwrap();
            if (q.is_nan() || !(0.0..=1.0).contains(&q))
                && let Some(notes) = &mut self.notes
            {
                notes.add_invalid_quantile(q, self.scalar_span, query, account);
            }
        }
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
                        self.vector_span,
                        query,
                        account,
                    );
                }
                continue;
            };
            let mut found = false;
            for prior in &*self.groups {
                control.check()?;
                account.work(1);
                found |= prior.group == group;
            }
            if found {
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
                bounds: 0..0,
                dropped,
                skip: false,
            });
        }
        for active in &mut *self.active {
            if active.kind != 2 {
                continue;
            }
            let Some(group) = self.plan.metadata[active.row].native_group else {
                continue;
            };
            for classic in &mut *self.groups {
                control.check()?;
                account.work(1);
                if classic.group != group || classic.skip {
                    continue;
                }
                classic.skip = true;
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
                        self.vector_span,
                        query,
                        account,
                    );
                }
                break;
            }
        }
        // Freeze enumeration before any emitted group mutates its retained metric.
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
        )?;
        for group in &mut *self.groups {
            group.bounds.start = self.bounds.len();
            if !group.skip {
                for active in &*self.active {
                    control.check()?;
                    account.work(1);
                    if active.kind == 1 && self.plan.metadata[active.row].group == Some(group.group)
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
        self.phase = Phase::Choose;
        Ok(())
    }
    fn choose(
        &mut self,
        logical: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        if logical {
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
    ) -> Result<OwnedLabels, Interrupted> {
        let source = match self.choice.expect("chosen histogram") {
            BindingKey::Classic(group) => &self.plan.groups[group].labels,
            BindingKey::Native(row) => &self.plan.rows[row].labels,
        };
        let mut result = source.copy_checked(account, MemoryKind::Workspace, control)?;
        if !options.delayed_name_removal {
            result.retain_labels_checked(account, control, |label| {
                Ok::<_, Interrupted>(!metadata(&label.name))
            })?;
        }
        Ok(result)
    }
    fn emit(
        &mut self,
        value: f64,
        options: EvaluationOptions,
        preparation: Option<Preparation<'a>>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<EvaluatedSample, EvaluationError> {
        let key = self.choice.expect("chosen histogram");
        let (ordinal, labels) = if let Some(preparation) = preparation {
            let mut cache = preparation.cache.borrow_mut();
            let mut found = None;
            for (position, entry) in cache.bindings.iter().enumerate() {
                control.check()?;
                account.work(1);
                if entry.key == key {
                    found = Some(position);
                    break;
                }
            }
            let ordinal = if let Some(ordinal) = found {
                ordinal
            } else {
                let labels = self.labels(options, control, account)?;
                let ordinal = cache.bindings.len();
                cache.bindings.push(Binding { key, labels });
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
                if entry.key == key {
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
            value: OwnedValue::Float(value),
            source_time_ms: options.time_ms,
            source_start_time_ms: 0,
            drop_name: true,
        })
    }
    fn drop_live(
        &self,
        preparation: Option<Preparation<'a>>,
        options: EvaluationOptions,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if !options.delayed_name_removal
            && let BindingKey::Classic(group) = self.choice.expect("chosen histogram")
            && let Some(preparation) = preparation
        {
            for entry in &mut *preparation.cache.borrow_mut().live {
                control.check()?;
                account.work(1);
                if entry.group == group {
                    entry.dropped = true;
                    break;
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
                        argument,
                        ordinal: None,
                    });
                }
                Phase::Scalar(argument) => {
                    self.phase = if argument + 1 < self.function.vector_argument() {
                        Phase::StartScalar(argument + 1)
                    } else {
                        Phase::StartVector
                    };
                }
                Phase::StartVector => {
                    self.phase = Phase::Vector;
                    return Ok(Action::Child {
                        argument: self.function.vector_argument(),
                        ordinal: None,
                    });
                }
                Phase::Vector => self.phase = Phase::Begin,
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
                                argument: self.function.vector_argument(),
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
                            if matches!(self.function, Function::Fraction) {
                                self.drop_live(preparation, options, control, account)?;
                            }
                            let operation = match self.function {
                                Function::Quantile => {
                                    Operation::Quantile(self.parameters[0].unwrap())
                                }
                                Function::Fraction => Operation::Fraction(
                                    self.parameters[0].unwrap(),
                                    self.parameters[1].unwrap(),
                                ),
                            };
                            self.replay = Some(Replay::new(
                                operation,
                                &mut self.bounds[self.groups[position].bounds.clone()],
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
                        Ok::<_, Interrupted>(())
                    };
                    let result = match self.function {
                        Function::Quantile => histogram_functions::histogram_quantile_with(
                            self.parameters[0].unwrap(),
                            histogram.as_ref(),
                            poll,
                        )?,
                        Function::Fraction => histogram_functions::histogram_fraction_with(
                            self.parameters[0].unwrap(),
                            self.parameters[1].unwrap(),
                            histogram.as_ref(),
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
                            self.vector_span,
                            query,
                            account,
                        );
                    }
                    return Ok(Action::Sample(self.emit(
                        result.value,
                        options,
                        preparation,
                        control,
                        account,
                    )?));
                }
                Phase::Classic(position) => {
                    match self
                        .replay
                        .as_mut()
                        .expect("classic count continuation")
                        .next(&self.bounds[self.groups[position].bounds.clone()], || {
                            control.check()?;
                            account.work(1);
                            Ok::<_, Interrupted>(())
                        })? {
                        classic_replay::Action::Count(row) => {
                            self.phase = Phase::Count { position, row };
                            // The immutable plan owns this reference throughout the child frame.
                            return Ok(Action::Child {
                                argument: self.function.vector_argument(),
                                ordinal: Some(&self.plan.rows[row].ordinal),
                            });
                        }
                        classic_replay::Action::Done(result) => {
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
                                    self.vector_span,
                                    Monotonicity::new(
                                        options.time_ms,
                                        result.min_bound,
                                        result.max_bound,
                                        result.max_decrease,
                                    ),
                                    query,
                                    account,
                                );
                            }
                            self.drop_live(preparation, options, control, account)?;
                            self.replay = None;
                            return Ok(Action::Sample(self.emit(
                                result.value,
                                options,
                                preparation,
                                control,
                                account,
                            )?));
                        }
                    }
                }
                Phase::Count { position, .. } => {
                    let Some(OwnedValue::Float(value)) = self.pending.take() else {
                        return Err(EvaluationError::Invalid(
                            "histogram classic replay value is missing",
                        ));
                    };
                    self.replay
                        .as_mut()
                        .expect("classic count continuation")
                        .receive(value);
                    self.phase = Phase::Classic(position);
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
