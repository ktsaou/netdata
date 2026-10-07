// SPDX-License-Identifier: GPL-3.0-or-later
//! Whole-query descriptors retain labels and ordinals, never raw sample timelines.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::AnnotationBuilder,
    evaluator::EvaluationError,
    evaluator_order,
    frontend::ast::{NodeId, VectorMatching},
    label_hash,
    result::OwnedLabels,
    storage::{Control, InitialSample, Interrupted},
};

pub(crate) struct Descriptor {
    pub(crate) labels: OwnedLabels,
    pub(crate) ordinal: usize,
    pub(crate) hash: u64,
    pub(crate) drop_name: bool,
    pub(crate) first_order: usize,
    pub(crate) sources: std::ops::Range<usize>,
    pub(crate) kinds: u8,
    pub(crate) heads: HeadSummary,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct PointCounts([u8; 2]);
impl PointCounts {
    pub(crate) fn push(&mut self, kind: u8) {
        let index = usize::from(kind == 2);
        self.0[index] = self.0[index].saturating_add(1).min(2);
    }
    fn merge(&mut self, other: Self) {
        for (count, value) in self.0.iter_mut().zip(other.0) {
            *count = count.saturating_add(value).min(2);
        }
    }
    pub(crate) fn repeated(self, kind: u8) -> bool {
        self.0[usize::from(kind == 2)] == 2
    }
}

pub(crate) struct MergeGuard {
    hashes: AccountedVec<u64>,
}
impl MergeGuard {
    pub(crate) fn new(
        mut hashes: AccountedVec<u64>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, Interrupted> {
        evaluator_order::sort(&mut hashes, control, account, u64::cmp)?;
        let (mut position, mut end) = (0, 0);
        while position < hashes.len() {
            let hash = hashes[position];
            let start = position;
            while position < hashes.len() && hashes[position] == hash {
                control.check()?;
                account.work(1);
                position += 1;
            }
            if position - start > 1 {
                hashes[end] = hash;
                end += 1;
            }
        }
        hashes.truncate(end);
        Ok(Self { hashes })
    }
    pub(crate) fn checks(
        &self,
        hash: u64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        let (mut left, mut right) = (0, self.hashes.len());
        while left < right {
            control.check()?;
            account.work(1);
            let middle = left + (right - left) / 2;
            match self.hashes[middle].cmp(&hash) {
                std::cmp::Ordering::Less => left = middle + 1,
                std::cmp::Ordering::Greater => right = middle,
                std::cmp::Ordering::Equal => return Ok(true),
            }
        }
        Ok(false)
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct HeadSummary {
    times: [i64; 3],
    present: u8,
}
impl HeadSummary {
    fn time(self, index: usize) -> Option<i64> {
        (self.present & (1 << index) != 0).then_some(self.times[index])
    }
    fn record(&mut self, index: usize, time_ms: i64) {
        self.times[index] = self.time(index).map_or(time_ms, |prior| prior.min(time_ms));
        self.present |= 1 << index;
    }
    pub(crate) fn note(&mut self, counts: PointCounts, time_ms: i64) {
        if counts.0[0] == 2 {
            self.record(0, time_ms);
        }
        if counts.0[1] == 2 {
            self.record(1, time_ms);
        }
        if counts.0[0] != 0 && counts.0[1] != 0 {
            self.record(2, time_ms);
        }
    }
    pub(crate) fn merge(&mut self, other: Self) {
        for index in 0..3 {
            if let Some(time_ms) = other.time(index) {
                self.record(index, time_ms);
            }
        }
    }
    pub(crate) fn multiple(self) -> bool {
        self.present != 0
    }
    pub(crate) fn repeated_kind(self) -> bool {
        self.present & 3 != 0
    }
    pub(crate) fn accepts(self, kind: u8, time_ms: i64) -> bool {
        if kind == 1 {
            return self.time(0).is_none_or(|duplicate| time_ms <= duplicate);
        }
        self.time(1).is_none_or(|duplicate| time_ms <= duplicate)
            && self.time(2).is_none_or(|collision| {
                self.time(0).is_some_and(|duplicate| duplicate < collision) || time_ms < collision
            })
    }
}

pub(crate) struct DescriptorIndex {
    entries: AccountedVec<(usize, usize)>,
}
impl DescriptorIndex {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            entries: AccountedVec::new(account, MemoryKind::Workspace),
        }
    }
    pub(crate) fn find(
        &self,
        ordinal: usize,
        labels: &OwnedLabels,
        descriptors: &[Descriptor],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        let (mut left, mut right) = (0, self.entries.len());
        while left < right {
            control.check()?;
            account.work(1);
            let middle = left + (right - left) / 2;
            if self.entries[middle].0 < ordinal {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        for &(prior, position) in &self.entries[left..] {
            control.check()?;
            account.work(1);
            if prior != ordinal {
                break;
            }
            if crate::binary_labels::same(&descriptors[position].labels, labels, control, account)?
            {
                return Ok(Some(position));
            }
        }
        Ok(None)
    }
    pub(crate) fn insert(
        &mut self,
        ordinal: usize,
        position: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let entry = (ordinal, position);
        let (mut left, mut right) = (0, self.entries.len());
        while left < right {
            control.check()?;
            account.work(1);
            let middle = left + (right - left) / 2;
            if self.entries[middle] < entry {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        let end = self.entries.len();
        self.entries.push(entry);
        // Preserve descriptor discovery order while making ordinal lookup independent of unrelated rows.
        for index in (left + 1..=end).rev() {
            control.check()?;
            account.work(1);
            self.entries.swap(index - 1, index);
        }
        Ok(())
    }
}

pub(crate) struct StepKinds {
    entries: AccountedVec<(u64, usize, PointCounts)>,
    unary_merge: bool,
}
impl StepKinds {
    pub(crate) fn new(unary_merge: bool, account: &QueryAccounting) -> Self {
        Self {
            entries: AccountedVec::new(account, MemoryKind::Workspace),
            unary_merge,
        }
    }
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
    pub(crate) fn push(&mut self, hash: u64, ordinal: usize, kind: u8) {
        let mut counts = PointCounts::default();
        counts.push(kind);
        self.entries
            .push((hash, if self.unary_merge { 0 } else { ordinal }, counts));
    }
    pub(crate) fn finish(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        evaluator_order::sort(&mut self.entries, control, account, |left, right| {
            (left.0, left.1).cmp(&(right.0, right.1))
        })?;
        let mut end = 0;
        for index in 0..self.entries.len() {
            control.check()?;
            account.work(1);
            let entry = self.entries[index];
            if end > 0 && (self.entries[end - 1].0, self.entries[end - 1].1) == (entry.0, entry.1) {
                self.entries[end - 1].2.merge(entry.2);
            } else {
                self.entries[end] = entry;
                end += 1;
            }
        }
        self.entries.truncate(end);
        Ok(())
    }
    pub(crate) fn counts(
        &self,
        descriptor: &Descriptor,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<PointCounts, Interrupted> {
        let key = (
            descriptor.hash,
            if self.unary_merge {
                0
            } else {
                descriptor.ordinal
            },
        );
        let (mut left, mut right) = (0, self.entries.len());
        while left < right {
            control.check()?;
            account.work(1);
            let middle = left + (right - left) / 2;
            let entry = self.entries[middle];
            match (entry.0, entry.1).cmp(&key) {
                std::cmp::Ordering::Less => left = middle + 1,
                std::cmp::Ordering::Greater => right = middle,
                std::cmp::Ordering::Equal => return Ok(entry.2),
            }
        }
        Ok(PointCounts::default())
    }
}

pub(crate) struct GroupDescriptor {
    pub(crate) hash: u64,
    pub(crate) labels: OwnedLabels,
    pub(crate) whole_drop_name: Option<bool>,
}
pub(crate) struct Projection<'a> {
    pub(crate) descriptors: &'a [Descriptor],
    pub(crate) bindings: &'a [(usize, usize)],
}
#[derive(Default)]
struct NodeRoster {
    output: Option<AccountedVec<Descriptor>>,
    sources: Option<AccountedVec<usize>>,
    groups: Option<AccountedVec<GroupDescriptor>>,
    annotations: Option<AnnotationBuilder>,
    projection: Option<AccountedVec<(usize, usize)>>,
    binary: Option<AccountedBox<crate::vector_matching::Plan>>,
    histogram: Option<AccountedBox<crate::histogram_call::Plan>>,
    info: Option<AccountedBox<crate::info::Plan>>,
    count: Option<AccountedBox<crate::count_values::Plan>>,
    selection: Option<AccountedBox<crate::topk::Plan>>,
    quantile: Option<AccountedBox<crate::quantile_aggregate::Plan>>,
    clock_initial: Option<AccountedVec<InitialSample>>,
    merge_guard: Option<MergeGuard>,
}
pub(crate) struct Rosters {
    nodes: AccountedVec<NodeRoster>,
}
impl Rosters {
    pub(crate) fn prepare_aggregate_parameter(
        &mut self,
        node: NodeId,
        operator: crate::frontend::token::TokenKind,
        parameters: crate::topk::Parameters,
        account: &QueryAccounting,
    ) {
        if operator == crate::frontend::token::TokenKind::QUANTILE {
            self.nodes[node.0].quantile = Some(AccountedBox::new(
                crate::quantile_aggregate::Plan::new(parameters, account),
                &AllocationContext::new(account, MemoryKind::Workspace),
            ));
            return;
        }
        self.nodes[node.0].selection = Some(AccountedBox::new(
            crate::topk::Plan::new(operator, parameters, account),
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
    }
    pub(crate) fn prepare_selection(
        &mut self,
        node: NodeId,
        child: NodeId,
        expression: &crate::frontend::ast::ParsedExpression,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let rows = if self.nodes[child.0].sources.is_some() {
            self.projection_input(child, control, account)?
        } else {
            self.take_output(child)
                .unwrap_or_else(|| AccountedVec::new(account, MemoryKind::Workspace))
        };
        self.nodes[node.0]
            .selection
            .as_deref_mut()
            .expect("prepared selection parameter")
            .prepare_rows(rows, expression, node, control, account)
    }
    pub(crate) fn selection(&self, node: NodeId) -> Option<&crate::topk::Plan> {
        self.nodes[node.0].selection.as_deref()
    }
    pub(crate) fn prepare_quantile(
        &mut self,
        node: NodeId,
        child: NodeId,
        expression: &crate::frontend::ast::ParsedExpression,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let rows = self.projection_input(child, control, account)?;
        let groups = self.nodes[node.0]
            .quantile
            .as_deref_mut()
            .expect("prepared quantile parameter")
            .prepare_rows(rows, expression, node, control, account)?;
        self.nodes[node.0].groups = Some(groups);
        Ok(())
    }
    pub(crate) fn quantile(&self, node: NodeId) -> Option<&crate::quantile_aggregate::Plan> {
        self.nodes[node.0].quantile.as_deref()
    }
    pub(crate) fn finish_selection_projection(
        &mut self,
        node: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let roster = &mut self.nodes[node.0];
        if let (Some(plan), Some(descriptors), Some(sources)) = (
            roster.selection.as_deref_mut(),
            roster.output.as_deref(),
            roster.sources.as_deref(),
        ) {
            plan.project(descriptors, sources, control, account)?;
        }
        Ok(())
    }
    pub(crate) fn prepare_merge_guard(
        &mut self,
        node: NodeId,
        hashes: AccountedVec<u64>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        self.nodes[node.0].merge_guard = Some(MergeGuard::new(hashes, control, account)?);
        Ok(())
    }
    pub(crate) fn checks_merge(
        &self,
        node: NodeId,
        hash: u64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        self.nodes[node.0]
            .merge_guard
            .as_ref()
            .map_or(Ok(false), |guard| guard.checks(hash, control, account))
    }
    pub(crate) fn prepare_count(
        &mut self,
        node: NodeId,
        plan: crate::count_values::Plan,
        account: &QueryAccounting,
    ) {
        self.nodes[node.0].count = Some(AccountedBox::new(
            plan,
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
    }
    pub(crate) fn count(&self, node: NodeId) -> Option<&crate::count_values::Plan> {
        self.nodes[node.0].count.as_deref()
    }
    pub(crate) fn finish_count_cache(&mut self, node: NodeId, cache: crate::count_values::Cache) {
        self.nodes[node.0]
            .count
            .as_deref_mut()
            .expect("count_values plan")
            .finish(cache);
    }
    pub(crate) fn finish_count_projection(
        &mut self,
        node: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let roster = &mut self.nodes[node.0];
        if let (Some(plan), Some(descriptors), Some(sources)) = (
            roster.count.as_deref_mut(),
            roster.output.as_deref(),
            roster.sources.as_deref(),
        ) {
            plan.project(descriptors, sources, control, account)?;
        }
        Ok(())
    }
    pub(crate) fn prepare_clock(
        &mut self,
        selector: NodeId,
        failures: AccountedVec<InitialSample>,
    ) {
        self.nodes[selector.0].clock_initial = Some(failures);
    }
    pub(crate) fn clock_initial(&self, selector: NodeId, ordinal: usize) -> Option<&InitialSample> {
        self.nodes[selector.0].clock_initial.as_ref()?.get(ordinal)
    }
    pub(crate) fn prepare_info(
        &mut self,
        node: NodeId,
        plan: crate::info::Plan,
        account: &QueryAccounting,
    ) {
        self.nodes[node.0].info = Some(AccountedBox::new(
            plan,
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
    }
    pub(crate) fn info(&self, node: NodeId) -> Option<&crate::info::Plan> {
        self.nodes[node.0].info.as_deref()
    }
    pub(crate) fn finish_info_cache(&mut self, node: NodeId, cache: crate::info::Cache) {
        self.nodes[node.0]
            .info
            .as_deref_mut()
            .expect("INFO plan")
            .finish(cache);
    }
    pub(crate) fn prepare_histogram(
        &mut self,
        node: NodeId,
        child: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let rows = self.projection_input(child, control, account)?;
        self.nodes[node.0].histogram = Some(AccountedBox::new(
            crate::histogram_call::Plan::new(rows, control, account)?,
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
        Ok(())
    }
    pub(crate) fn histogram(&self, node: NodeId) -> Option<&crate::histogram_call::Plan> {
        self.nodes[node.0].histogram.as_deref()
    }
    pub(crate) fn finish_histogram_cache(
        &mut self,
        node: NodeId,
        cache: crate::histogram_call::Cache,
    ) {
        self.nodes[node.0]
            .histogram
            .as_deref_mut()
            .expect("histogram plan")
            .finish(cache);
    }
    pub(crate) fn prepare_binary(
        &mut self,
        node: NodeId,
        children: [NodeId; 2],
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let left = self.projection_input(children[0], control, account)?;
        let right = self.projection_input(children[1], control, account)?;
        self.nodes[node.0].binary = Some(AccountedBox::new(
            crate::vector_matching::Plan::new(left, right, matching, control, account)?,
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
        Ok(())
    }
    pub(crate) fn binary(&self, node: NodeId) -> Option<&crate::vector_matching::Plan> {
        self.nodes[node.0].binary.as_deref()
    }
    pub(crate) fn finish_binary_cache(
        &mut self,
        node: NodeId,
        cache: AccountedVec<crate::vector_matching::CacheEntry>,
    ) {
        self.nodes[node.0]
            .binary
            .as_deref_mut()
            .expect("binary plan")
            .cache = cache;
    }
    pub(crate) fn new(account: &QueryAccounting, nodes: usize) -> Self {
        let mut values = AccountedVec::new(account, MemoryKind::Workspace);
        for _ in 0..nodes {
            values.push(NodeRoster::default());
        }
        Self { nodes: values }
    }
    pub(crate) fn take_output(&mut self, node: NodeId) -> Option<AccountedVec<Descriptor>> {
        self.nodes[node.0].output.take()
    }
    pub(crate) fn release_output(&mut self, node: NodeId) {
        if self.nodes[node.0].sources.is_none() {
            self.nodes[node.0].output = None;
        }
    }
    pub(crate) fn descriptors(&self, node: NodeId) -> &[Descriptor] {
        self.nodes[node.0].output.as_deref().unwrap_or(&[])
    }
    pub(crate) fn output(
        &mut self,
        node: NodeId,
        mut descriptors: AccountedVec<Descriptor>,
        account: &QueryAccounting,
    ) {
        descriptors.sort_unstable_by_key(|descriptor| descriptor.ordinal);
        if descriptors
            .iter()
            .any(|descriptor| descriptor.heads.multiple())
        {
            let mut sources = AccountedVec::new(account, MemoryKind::Workspace);
            for descriptor in descriptors.iter_mut() {
                descriptor.sources = sources.len()..sources.len() + 1;
                sources.push(descriptor.ordinal);
            }
            self.nodes[node.0].sources = Some(sources);
        }
        self.nodes[node.0].output = Some(descriptors);
    }
    pub(crate) fn ordered(&self, node: NodeId) -> Option<(&[Descriptor], &[usize])> {
        let roster = &self.nodes[node.0];
        Some((roster.output.as_deref()?, roster.sources.as_deref()?))
    }
    pub(crate) fn selection_output(
        &mut self,
        node: NodeId,
        mut descriptors: AccountedVec<Descriptor>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut sources = AccountedVec::new(account, MemoryKind::Workspace);
        for (ordinal, descriptor) in descriptors.iter_mut().enumerate() {
            control.check()?;
            account.work(1);
            descriptor.sources = sources.len()..sources.len() + 1;
            sources.push(descriptor.ordinal);
            descriptor.ordinal = ordinal;
        }
        self.nodes[node.0].output = Some(descriptors);
        self.nodes[node.0].sources = Some(sources);
        Ok(())
    }
    pub(crate) fn ordered_output(
        &mut self,
        node: NodeId,
        mut descriptors: AccountedVec<Descriptor>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        evaluator_order::sort(&mut descriptors, control, account, |left, right| {
            left.hash
                .cmp(&right.hash)
                .then_with(|| left.first_order.cmp(&right.first_order))
        })?;
        let mut output = AccountedVec::<Descriptor>::new(account, MemoryKind::Workspace);
        let mut sources = AccountedVec::<usize>::new(account, MemoryKind::Workspace);
        for mut descriptor in descriptors {
            control.check()?;
            account.work(1);
            if let Some(previous) = output.last_mut()
                && previous.hash == descriptor.hash
            {
                previous.kinds |= descriptor.kinds;
                previous.heads.merge(descriptor.heads);
                if !sources[previous.sources.clone()].contains(&descriptor.ordinal) {
                    sources.push(descriptor.ordinal);
                    previous.sources.end = sources.len();
                }
            } else {
                descriptor.sources = sources.len()..sources.len() + 1;
                sources.push(descriptor.ordinal);
                output.push(descriptor);
            }
        }
        evaluator_order::sort_by_with(&mut output, Some(control), Some(account), |left, right| {
            left.labels.compare_checked(&right.labels, account, control)
        })?;
        for (ordinal, descriptor) in output.iter_mut().enumerate() {
            descriptor.ordinal = ordinal;
        }
        self.nodes[node.0].output = Some(output);
        self.nodes[node.0].sources = Some(sources);
        Ok(())
    }
    pub(crate) fn projection_input(
        &mut self,
        child: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<AccountedVec<Descriptor>, Interrupted> {
        if self.nodes[child.0].sources.is_none() {
            return Ok(self
                .take_output(child)
                .unwrap_or_else(|| AccountedVec::new(account, MemoryKind::Workspace)));
        }
        let mut output = AccountedVec::new(account, MemoryKind::Workspace);
        for descriptor in self.descriptors(child) {
            control.check()?;
            account.work(1);
            output.push(Descriptor {
                labels: descriptor
                    .labels
                    .copy_checked(account, MemoryKind::Workspace, control)?,
                ordinal: descriptor.ordinal,
                hash: descriptor.hash,
                drop_name: descriptor.drop_name,
                first_order: descriptor.first_order,
                sources: 0..0,
                kinds: descriptor.kinds,
                heads: descriptor.heads,
            });
        }
        Ok(output)
    }
    pub(crate) fn projection_output(
        &mut self,
        node: NodeId,
        mut descriptors: AccountedVec<Descriptor>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut hashes = AccountedVec::new(account, MemoryKind::Workspace);
        for descriptor in &*descriptors {
            control.check()?;
            account.work(1);
            hashes.push(descriptor.hash);
        }
        self.prepare_merge_guard(node, hashes, control, account)?;
        // Check hashes in place; unique matrices recover their original child order.
        evaluator_order::sort(&mut descriptors, control, account, |left, right| {
            left.hash
                .cmp(&right.hash)
                .then_with(|| left.first_order.cmp(&right.first_order))
        })?;
        let mut duplicates = false;
        for pair in descriptors.windows(2) {
            control.check()?;
            account.work(1);
            duplicates |= pair[0].hash == pair[1].hash;
        }
        if duplicates {
            self.ordered_output(node, descriptors, control, account)?;
        } else {
            evaluator_order::sort(&mut descriptors, control, account, |left, right| {
                left.first_order.cmp(&right.first_order)
            })?;
            let mut sources = AccountedVec::new(account, MemoryKind::Workspace);
            for (ordinal, descriptor) in descriptors.iter_mut().enumerate() {
                control.check()?;
                account.work(1);
                descriptor.sources = sources.len()..sources.len() + 1;
                sources.push(descriptor.ordinal);
                descriptor.ordinal = ordinal;
            }
            self.nodes[node.0].output = Some(descriptors);
            self.nodes[node.0].sources = Some(sources);
        }
        let mut bindings = AccountedVec::new(account, MemoryKind::Workspace);
        let (descriptors, sources) = self.ordered(node).expect("projected roster");
        for (position, descriptor) in descriptors.iter().enumerate() {
            for source in &sources[descriptor.sources.clone()] {
                control.check()?;
                account.work(1);
                bindings.push((*source, position));
            }
        }
        evaluator_order::sort(&mut bindings, control, account, |left, right| {
            left.0.cmp(&right.0)
        })?;
        self.nodes[node.0].projection = Some(bindings);
        Ok(())
    }
    pub(crate) fn projection(&self, node: NodeId) -> Option<Projection<'_>> {
        let roster = &self.nodes[node.0];
        Some(Projection {
            descriptors: roster.output.as_deref()?,
            bindings: roster.projection.as_deref()?,
        })
    }
    pub(crate) fn projection_collisions(
        &mut self,
        node: NodeId,
        kinds: &[PointCounts],
        time_ms: i64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let descriptors = self.nodes[node.0]
            .output
            .as_deref_mut()
            .expect("projected roster");
        assert_eq!(descriptors.len(), kinds.len());
        for (descriptor, kind) in descriptors.iter_mut().zip(kinds) {
            control.check()?;
            account.work(1);
            descriptor.heads.note(*kind, time_ms);
        }
        Ok(())
    }
    pub(crate) fn groups(&self, node: NodeId) -> Option<&[GroupDescriptor]> {
        self.nodes[node.0].groups.as_deref()
    }
    pub(crate) fn finish_group_flags(
        &mut self,
        node: NodeId,
        descriptors: &[Descriptor],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if let Some(groups) = &mut self.nodes[node.0].groups {
            for descriptor in descriptors {
                control.check()?;
                account.work(1);
                groups[descriptor.ordinal].whole_drop_name = Some(descriptor.drop_name);
            }
        }
        Ok(())
    }
    pub(crate) fn take_annotations(&mut self, node: NodeId) -> Option<AnnotationBuilder> {
        self.nodes[node.0].annotations.take()
    }
    pub(crate) fn annotations(&mut self, node: NodeId, annotations: AnnotationBuilder) {
        self.nodes[node.0].annotations = Some(annotations);
    }
    pub(crate) fn prepare_groups(
        &mut self,
        node: NodeId,
        child: NodeId,
        grouping: Option<&[AccountedVec<u8>]>,
        without: bool,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<(), Interrupted> {
        let mut groups = AccountedVec::<GroupDescriptor>::new(account, MemoryKind::Workspace);
        if self.nodes[child.0].sources.is_some() {
            for descriptor in self.nodes[child.0].output.as_deref().unwrap_or(&[]) {
                control.check()?;
                account.work(1);
                let mut labels = OwnedLabels::copy_from(
                    &descriptor.labels.views(account),
                    account,
                    MemoryKind::Workspace,
                );
                group_key(&mut labels, grouping, without);
                let hash = if !without && grouping.is_none_or(|names| names.is_empty()) {
                    0
                } else {
                    label_hash::grouping(&labels, control, account)?
                };
                if !groups.iter().any(|group| group.hash == hash) {
                    groups.push(GroupDescriptor {
                        hash,
                        labels,
                        whole_drop_name: None,
                    });
                }
            }
        } else if let Some(mut descriptors) = self.take_output(child) {
            descriptors.reverse();
            while let Some(mut descriptor) = descriptors.pop() {
                group_key(&mut descriptor.labels, grouping, without);
                let hash = if !without && grouping.is_none_or(|names| names.is_empty()) {
                    0
                } else {
                    label_hash::grouping(&descriptor.labels, control, account)?
                };
                if !groups.iter().any(|group| group.hash == hash) {
                    groups.push(GroupDescriptor {
                        hash,
                        labels: descriptor.labels,
                        whole_drop_name: None,
                    });
                }
            }
        }
        self.nodes[node.0].groups = Some(groups);
        Ok(())
    }
}
pub(crate) fn projection_index(
    bindings: &[(usize, usize)],
    ordinal: usize,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<usize>, Interrupted> {
    let (mut left, mut right) = (0, bindings.len());
    while left < right {
        control.check()?;
        account.work(1);
        let middle = left + (right - left) / 2;
        match bindings[middle].0.cmp(&ordinal) {
            std::cmp::Ordering::Less => left = middle + 1,
            std::cmp::Ordering::Greater => right = middle,
            std::cmp::Ordering::Equal => return Ok(Some(bindings[middle].1)),
        }
    }
    Ok(None)
}
pub(crate) fn group_key(
    labels: &mut OwnedLabels,
    grouping: Option<&[AccountedVec<u8>]>,
    without: bool,
) {
    labels.retain(|name| {
        let found = grouping.is_some_and(|labels| labels.iter().any(|label| label == name));
        if without {
            name != b"__name__" && !found
        } else {
            found
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::EngineAccounting;
    use std::collections::VecDeque;

    #[test]
    fn compact_heads_match_source_destructive_heads_for_every_four_step_multiplicity() {
        // Pinned engine.go nextValues keeps an unmatched old head rather than skipping it.
        for mut encoding in 0..9usize.pow(4) {
            let mut counts = [PointCounts::default(); 4];
            let mut floats = VecDeque::new();
            let mut histograms = VecDeque::new();
            let mut summary = HeadSummary::default();
            for (time, count) in counts.iter_mut().enumerate() {
                *count = PointCounts([(encoding % 3) as u8, ((encoding / 3) % 3) as u8]);
                encoding /= 9;
                for _ in 0..count.0[0] {
                    floats.push_back(time);
                }
                for _ in 0..count.0[1] {
                    histograms.push_back(time);
                }
                summary.note(*count, time as i64);
            }
            for (time, count) in counts.into_iter().enumerate() {
                let expected = if floats.front() == Some(&time) {
                    floats.pop_front();
                    Some(1)
                } else if histograms.front() == Some(&time) {
                    histograms.pop_front();
                    Some(2)
                } else {
                    None
                };
                let actual = if count.0[0] > 0 && summary.accepts(1, time as i64) {
                    Some(1)
                } else if count.0[1] > 0 && summary.accepts(2, time as i64) {
                    Some(2)
                } else {
                    None
                };
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn cleanup_guard_counts_matrix_rows_and_balances_its_metadata_owner() {
        let account = EngineAccounting::default().query();
        let control = Control::default();
        let mut hashes = AccountedVec::new(&account, MemoryKind::Workspace);
        hashes.extend([7, 3, 9, 3]);
        let guard = MergeGuard::new(hashes, &control, &account).unwrap();
        assert!(guard.checks(3, &control, &account).unwrap());
        for hash in [0, 7, 9, u64::MAX] {
            assert!(!guard.checks(hash, &control, &account).unwrap());
        }
        drop(guard);
        account.snapshot().assert_memory_released();
    }
}
