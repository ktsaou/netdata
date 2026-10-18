// SPDX-License-Identifier: GPL-3.0-or-later
// Value-dependent grouping follows pinned promql/engine.go; see NOTICE.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    binary_labels,
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_order,
    evaluator_roster::Descriptor,
    frontend::{
        ast::{Expr, NodeId, ParsedExpression},
        token::TokenKind,
    },
    label_functions, label_hash,
    position_index::PositionIndex,
    query_preparation::Prepared,
    result::{OwnedLabels, OwnedValue, append_bytes_checked},
    storage::{Control, Interrupted},
};
use std::cell::RefCell;

const LINEAR_ENTRIES: usize = 8;

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
    bindings: Cache,
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
            bindings: Cache::new(account),
        })
    }
    pub(crate) fn finish(&mut self, cache: Cache) {
        self.bindings = cache;
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
            let binding =
                self.bindings
                    .entries
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
                    .entries
                    .get_mut(*raw)
                    .ok_or(EvaluationError::Invalid(
                        "count_values raw binding is missing",
                    ))?
                    .canonical = Some((descriptor.ordinal, representative));
            }
        }
        Ok(())
    }
    fn hash(
        &self,
        labels: &OwnedLabels,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<u64, Interrupted> {
        if !self.without && self.grouping.is_empty() {
            Ok(0)
        } else {
            label_hash::grouping(labels, control, account)
        }
    }
}

pub(crate) struct Cache {
    entries: AccountedVec<Binding>,
    lookup: Option<AccountedBox<PositionIndex>>,
}
impl Cache {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            entries: AccountedVec::new(account, MemoryKind::Workspace),
            lookup: None,
        }
    }
    fn find(
        &self,
        hash: u64,
        labels: &OwnedLabels,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        if let Some(lookup) = &self.lookup {
            return lookup.find(
                hash,
                |prior| binary_labels::same(&self.entries[prior].labels, labels, control, account),
                control,
                account,
            );
        }
        for (ordinal, candidate) in self.entries.iter().enumerate() {
            control.check()?;
            account.work(1);
            if binary_labels::same(&candidate.labels, labels, control, account)? {
                return Ok(Some(ordinal));
            }
        }
        Ok(None)
    }
    fn insert(
        &mut self,
        hash: u64,
        labels: &OwnedLabels,
        plan: &Plan,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<usize, EvaluationError> {
        let labels = labels.copy_checked(account, MemoryKind::Workspace, control)?;
        let ordinal = self.entries.len();
        let binding = Binding {
            labels,
            canonical: None,
        };
        if ordinal < LINEAR_ENTRIES {
            self.entries.push(binding);
            return Ok(ordinal);
        }
        control.check()?;
        self.entries.reserve(1);
        control.check()?;
        insert_position(
            &mut self.lookup,
            hash,
            ordinal,
            |prior| plan.hash(&self.entries[prior].labels, control, account),
            control,
            account,
        )?;
        // The index and reserved vector become visible together, before another fallible check.
        self.entries.push(binding);
        control.check()?;
        Ok(ordinal)
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
    lookup: Option<AccountedBox<PositionIndex>>,
    position: usize,
    project: bool,
}
impl<'a> Step<'a> {
    pub(crate) fn new(plan: &'a Plan, project: bool, account: &QueryAccounting) -> Self {
        Self {
            plan,
            groups: AccountedVec::new(account, MemoryKind::Workspace),
            lookup: None,
            position: 0,
            project,
        }
    }
    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let mut labels = sample.labels.into_owned(account, control)?;
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
        let hash = self.plan.hash(&labels, control, account)?;
        if let Some(lookup) = &self.lookup {
            if let Some(position) = lookup.find(
                hash,
                |prior| Ok(self.groups[prior].hash == hash),
                control,
                account,
            )? {
                self.groups[position].count = self.groups[position].count.wrapping_add(1);
                return Ok(());
            }
        } else {
            for group in &mut *self.groups {
                control.check()?;
                account.work(1);
                if group.hash == hash {
                    group.count = group.count.wrapping_add(1);
                    return Ok(());
                }
            }
        }
        let group = Group {
            hash,
            full_hash: 0,
            labels,
            count: 1,
            ordinal: 0,
        };
        if self.groups.len() < LINEAR_ENTRIES {
            self.groups.push(group);
            return Ok(());
        }
        control.check()?;
        self.groups.reserve(1);
        control.check()?;
        insert_position(
            &mut self.lookup,
            hash,
            self.groups.len(),
            |prior| Ok(self.groups[prior].hash),
            control,
            account,
        )?;
        self.groups.push(group);
        control.check()?;
        Ok(())
    }
    pub(crate) fn finish(
        &mut self,
        options: EvaluationOptions,
        preparation: Option<Preparation<'_>>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        // Finish reorders groups; their collection positions must not survive that boundary.
        self.lookup = None;
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
                group.ordinal = match cache.find(group.hash, &group.labels, control, account)? {
                    Some(ordinal) => ordinal,
                    None => cache.insert(group.hash, &group.labels, self.plan, control, account)?,
                };
            } else {
                let raw = self
                    .plan
                    .bindings
                    .find(group.hash, &group.labels, control, account)?
                    .ok_or(EvaluationError::Invalid(
                        "frozen count_values label binding is missing",
                    ))?;
                group.ordinal = raw;
                if self.project
                    && let Some((ordinal, representative)) =
                        self.plan.bindings.entries[raw].canonical
                {
                    group.ordinal = ordinal;
                    let labels = &self.plan.bindings.entries[representative].labels;
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
    ) -> Option<EvaluatedSample<'a>> {
        let group = self.groups.get_mut(self.position)?;
        self.position += 1;
        Some(EvaluatedSample {
            ordinal: group.ordinal,
            labels: std::mem::replace(
                &mut group.labels,
                OwnedLabels::new(account, MemoryKind::Workspace),
            )
            .into(),
            float_samples: Some(crate::statistics::OwnedSamples::new(account, 1)),
            value: OwnedValue::Float(group.count as f64),
            source_time_ms: time_ms,
            source_start_time_ms: 0,
            drop_name: false,
        })
    }
}

fn insert_position(
    lookup: &mut Option<AccountedBox<PositionIndex>>,
    hash: u64,
    position: usize,
    mut hash_of: impl FnMut(usize) -> Result<u64, Interrupted>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    if let Some(lookup) = lookup {
        lookup.insert_new(hash, position, hash_of, control, account)?;
    } else if position == LINEAR_ENTRIES {
        // Tiny owners retain their original scan; publish the complete index only after growth.
        let mut index = PositionIndex::new(account);
        for prior in 0..position {
            control.check()?;
            account.work(1);
            let prior_hash = hash_of(prior)?;
            index.insert_new(prior_hash, prior, &mut hash_of, control, account)?;
        }
        index.insert_new(hash, position, hash_of, control, account)?;
        *lookup = Some(AccountedBox::new(
            index,
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounting::EngineAccounting, test_storage::group_preparation};

    fn plan(account: &QueryAccounting) -> Plan {
        let mut destination = AccountedVec::new(account, MemoryKind::Workspace);
        destination.extend_from_slice(b"v");
        let mut grouping = AccountedVec::new(account, MemoryKind::Workspace);
        for name in [b"id".as_slice(), b"v"] {
            let mut value = AccountedVec::new(account, MemoryKind::Workspace);
            value.extend_from_slice(name);
            grouping.push(value);
        }
        Plan {
            destination,
            grouping,
            without: true,
            bindings: Cache::new(account),
        }
    }

    fn labels(fields: &[(&[u8], &[u8])], account: &QueryAccounting) -> OwnedLabels {
        let mut result = OwnedLabels::new(account, MemoryKind::Workspace);
        for (name, value) in fields {
            result.push(name, value, account, MemoryKind::Workspace);
        }
        result
    }

    fn sample(labels: OwnedLabels, value: OwnedValue) -> EvaluatedSample<'static> {
        EvaluatedSample {
            ordinal: 0,
            labels: labels.into(),
            value,
            float_samples: None,
            source_time_ms: 0,
            source_start_time_ms: 0,
            drop_name: false,
        }
    }

    fn options() -> EvaluationOptions {
        EvaluationOptions {
            time_ms: 0,
            start_ms: 0,
            end_ms: 1000,
            step_ms: 1000,
            lookback_ms: 1,
            start_timestamps: false,
            delayed_name_removal: false,
        }
    }

    #[test]
    fn count_values_lookup_keeps_group_collisions_and_transfers_exact_bindings() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        {
            let mut plan = plan(&account);
            let cache = RefCell::new(Cache::new(&account));
            let first = labels(&[(b"a", b"b\xffx\xffy")], &account);
            let second = labels(&[(b"a", b"b"), (b"x", b"y")], &account);
            let hash = plan.hash(&first, &control, &account).unwrap();
            assert_eq!(hash, plan.hash(&second, &control, &account).unwrap());
            let mut step = Step::new(&plan, false, &account);
            for index in 0..16 {
                step.receive(
                    sample(
                        labels(&[(b"a", index.to_string().as_bytes())], &account),
                        OwnedValue::Float(7.0),
                    ),
                    &control,
                    &account,
                )
                .unwrap();
            }
            for key in [&first, &second] {
                step.receive(
                    sample(
                        key.copy_checked(&account, MemoryKind::Workspace, &control)
                            .unwrap(),
                        OwnedValue::Float(7.0),
                    ),
                    &control,
                    &account,
                )
                .unwrap();
            }
            assert!(step.lookup.is_some());
            assert_eq!(step.groups.len(), 17);
            let collided = step.groups.iter().find(|group| group.hash == hash).unwrap();
            assert_eq!(collided.count, 2);
            assert!(binary_labels::same(&collided.labels, &first, &control, &account).unwrap());
            step.finish(
                options(),
                Some(Preparation {
                    node: NodeId(0),
                    cache: &cache,
                }),
                &control,
                &account,
            )
            .unwrap();
            assert!(step.lookup.is_none());
            drop(step);
            let mut step = Step::new(&plan, false, &account);
            step.receive(
                sample(
                    second
                        .copy_checked(&account, MemoryKind::Workspace, &control)
                        .unwrap(),
                    OwnedValue::Float(7.0),
                ),
                &control,
                &account,
            )
            .unwrap();
            step.finish(
                options(),
                Some(Preparation {
                    node: NodeId(0),
                    cache: &cache,
                }),
                &control,
                &account,
            )
            .unwrap();
            drop(step);
            let mut cache = cache.into_inner();
            assert_eq!(cache.entries.len(), 18);
            for key in [labels(&[], &account), labels(&[(b"a", b"")], &account)] {
                let hash = plan.hash(&key, &control, &account).unwrap();
                assert_eq!(cache.find(hash, &key, &control, &account).unwrap(), None);
                cache.insert(hash, &key, &plan, &control, &account).unwrap();
            }
            let keys: Vec<_> = cache
                .entries
                .iter()
                .map(|binding| group_preparation::raw(&binding.labels))
                .collect();
            let pointer = cache.lookup.as_deref().unwrap() as *const PositionIndex;
            plan.finish(cache);
            assert_eq!(
                pointer,
                plan.bindings.lookup.as_deref().unwrap() as *const PositionIndex
            );
            let mut descriptors = group_preparation::rows(keys.clone(), &account, &control);
            let sources: Vec<_> = (0..keys.len()).collect();
            for (index, descriptor) in descriptors.iter_mut().enumerate() {
                descriptor.ordinal = index + 3;
                descriptor.sources = index..index + 1;
            }
            plan.project(&descriptors, &sources, &control, &account)
                .unwrap();
            for (ordinal, key) in keys.iter().enumerate() {
                let fields: Vec<_> = key
                    .iter()
                    .map(|(name, value)| (name.as_slice(), value.as_slice()))
                    .collect();
                let key = labels(&fields, &account);
                let hash = plan.hash(&key, &control, &account).unwrap();
                assert_eq!(
                    plan.bindings.find(hash, &key, &control, &account).unwrap(),
                    Some(ordinal)
                );
                assert_eq!(
                    plan.bindings.entries[ordinal].canonical,
                    Some((ordinal + 3, ordinal))
                );
            }
            println!(
                "count-values lookup:17hash groups,20exact bindings,allocation-preserving transfer/project"
            );
        }
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn count_values_lookup_growth_cancellation_keeps_consistent_owners() {
        for prior in [8, 10] {
            let successful_work = {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let plan = plan(&account);
                let mut cache = Cache::new(&account);
                for index in 0..prior {
                    let key = labels(&[(b"a", index.to_string().as_bytes())], &account);
                    let hash = plan.hash(&key, &control, &account).unwrap();
                    cache.insert(hash, &key, &plan, &control, &account).unwrap();
                }
                let key = labels(&[(b"a", b"new")], &account);
                let hash = plan.hash(&key, &control, &account).unwrap();
                let before = account.snapshot().work;
                cache.insert(hash, &key, &plan, &control, &account).unwrap();
                account.snapshot().work - before
            };
            let mut after_append = 0;
            for budget in 1..=successful_work {
                let engine = EngineAccounting::default();
                let account = engine.query();
                {
                    let control = Control::default();
                    let plan = plan(&account);
                    let mut cache = Cache::new(&account);
                    for index in 0..prior {
                        let key = labels(&[(b"a", index.to_string().as_bytes())], &account);
                        let hash = plan.hash(&key, &control, &account).unwrap();
                        cache.insert(hash, &key, &plan, &control, &account).unwrap();
                    }
                    let key = labels(&[(b"a", b"new")], &account);
                    let hash = plan.hash(&key, &control, &account).unwrap();
                    let result = crate::test_storage::interrupt_after_work(
                        &account,
                        &control,
                        budget,
                        || cache.insert(hash, &key, &plan, &control, &account),
                    );
                    assert!(matches!(
                        result,
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    ));
                    after_append += usize::from(cache.entries.len() == prior + 1);
                    let fresh = Control::default();
                    for (ordinal, binding) in cache.entries.iter().enumerate() {
                        let hash = plan.hash(&binding.labels, &fresh, &account).unwrap();
                        assert_eq!(
                            cache.find(hash, &binding.labels, &fresh, &account).unwrap(),
                            Some(ordinal)
                        );
                    }
                    assert!(cache.entries.len() >= prior);
                }
                account.snapshot().assert_memory_released();
            }
            assert!(after_append != 0);
            println!(
                "count-values lookup cancellation prior={prior} budgets={successful_work} consistent_appended_errors={after_append}"
            );
        }
        for prior in [8, 10] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut lookup = None;
            if prior == 10 {
                for position in 8..prior {
                    insert_position(
                        &mut lookup,
                        position as u64,
                        position,
                        |prior| Ok(prior as u64),
                        &control,
                        &account,
                    )
                    .unwrap();
                }
            }
            let held = account.snapshot().memory[MemoryKind::Workspace as usize].current;
            let mut calls = 0;
            let result = insert_position(
                &mut lookup,
                prior as u64,
                prior,
                |_| {
                    calls += 1;
                    if prior == 10 || calls == 3 {
                        control.cancellation.cancel();
                    }
                    Ok(0)
                },
                &control,
                &account,
            );
            assert!(matches!(result, Err(Interrupted::Cancelled)));
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].current,
                held
            );
            assert!(account.snapshot().memory[MemoryKind::Workspace as usize].peak > held);
            assert_eq!(lookup.is_some(), prior == 10);
            drop(lookup);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn count_values_lookup_long_copy_and_rehash_are_interruptible() {
        for rehash in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            {
                let control = Control::default();
                let plan = plan(&account);
                let mut cache = Cache::new(&account);
                for index in 0..if rehash { 10 } else { 1 } {
                    let mut text = vec![b'x'; if rehash { 32769 } else { 1 }];
                    text[0] = b'a' + index;
                    let key = labels(&[(b"a", &text)], &account);
                    let hash = plan.hash(&key, &control, &account).unwrap();
                    cache.insert(hash, &key, &plan, &control, &account).unwrap();
                }
                let text = vec![b'z'; if rehash { 1 } else { 32769 }];
                let key = labels(&[(b"a", &text)], &account);
                let hash = plan.hash(&key, &control, &account).unwrap();
                let before = account.snapshot().work;
                drop(
                    key.copy_checked(&account, MemoryKind::Workspace, &control)
                        .unwrap(),
                );
                let copy_work = account.snapshot().work - before;
                let held = account.snapshot().memory[MemoryKind::Workspace as usize].current;
                let before = account.snapshot().work;
                let result = crate::test_storage::interrupt_after_work(
                    &account,
                    &control,
                    if rehash { copy_work + 64 } else { 64 },
                    || cache.insert(hash, &key, &plan, &control, &account),
                );
                assert!(matches!(
                    result,
                    Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                ));
                let reached = account.snapshot().work - before;
                if rehash {
                    assert!(reached >= copy_work + 32768);
                } else {
                    assert!(reached >= 1024 && reached < copy_work);
                }
                assert_eq!(cache.entries.len(), if rehash { 10 } else { 1 });
                assert_eq!(cache.lookup.is_some(), rehash);
                assert_eq!(
                    account.snapshot().memory[MemoryKind::Workspace as usize].current,
                    held
                );
                let fresh = Control::default();
                for (ordinal, binding) in cache.entries.iter().enumerate() {
                    let hash = plan.hash(&binding.labels, &fresh, &account).unwrap();
                    assert_eq!(
                        cache.find(hash, &binding.labels, &fresh, &account).unwrap(),
                        Some(ordinal)
                    );
                }
                println!(
                    "count-values long-key cancellation rehash={rehash} copy_work={copy_work}"
                );
            }
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn count_values_lookup_hits_preserve_late_validation_errors() {
        use crate::{histogram::FloatHistogram, value::SampleValueRef};
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        {
            let plan = plan(&account);
            let mut step = Step::new(&plan, false, &account);
            for index in 0..9 {
                let key = index.to_string();
                step.receive(
                    sample(
                        labels(&[(b"a", key.as_bytes())], &account),
                        OwnedValue::Float(7.0),
                    ),
                    &control,
                    &account,
                )
                .unwrap();
            }
            assert!(step.lookup.is_some());
            let histogram = FloatHistogram {
                schema: crate::histogram::CUSTOM_BUCKETS_SCHEMA,
                zero_count: 1.0,
                ..FloatHistogram::default()
            };
            let value = OwnedValue::from_sample_checked(
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                &control,
                &account,
                MemoryKind::Workspace,
            )
            .unwrap();
            let crate::value::SampleValueRef::FloatHistogram(raw) = value.as_ref() else {
                panic!("raw float histogram")
            };
            assert_eq!(raw.zero_count, 1.0);
            let error = step
                .receive(
                    sample(labels(&[(b"a", b"0")], &account), value),
                    &control,
                    &account,
                )
                .unwrap_err();
            let EvaluationError::Storage(crate::storage::StorageError::Runtime {
                wrapped: false,
                message,
            }) = error
            else {
                panic!("native source error")
            };
            assert_eq!(
                &*message,
                b"histograms with custom buckets have no zero bucket"
            );
            let oversized = vec![b'x'; crate::label_encoding::FIELD_BOUNDARY];
            let error = step
                .receive(
                    sample(
                        labels(&[(b"a", b"0"), (b"id", &oversized)], &account),
                        OwnedValue::Float(7.0),
                    ),
                    &control,
                    &account,
                )
                .unwrap_err();
            let EvaluationError::Runtime(message) = error else {
                panic!("field boundary error")
            };
            assert_eq!(&*message, crate::label_encoding::TOO_LONG.as_bytes());
            assert_eq!(step.groups.len(), 9);
            assert!(step.groups.iter().all(|group| group.count == 1));
        }
        account.snapshot().assert_memory_released();
    }
}
