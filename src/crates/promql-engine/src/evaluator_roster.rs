// SPDX-License-Identifier: GPL-3.0-or-later
//! Whole-query descriptors retain labels and ordinals, never raw sample timelines.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::AnnotationBuilder,
    evaluator::EvaluationError,
    evaluator_order,
    frontend::ast::{Cardinality, NodeId, VectorMatching},
    label_hash,
    position_index::PositionIndex,
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

pub(crate) fn has_multiple(
    descriptors: &[Descriptor],
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    control.check()?;
    for descriptor in descriptors {
        control.check()?;
        account.work(1);
        if descriptor.heads.multiple() {
            control.check()?;
            return Ok(true);
        }
    }
    control.check()?;
    Ok(false)
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
    sorted: bool,
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
        self.sorted |= other.sorted;
        for index in 0..3 {
            if let Some(time_ms) = other.time(index) {
                self.record(index, time_ms);
            }
        }
    }
    pub(crate) fn multiple(self) -> bool {
        self.present != 0
    }
    pub(crate) fn sorted(self) -> bool {
        self.sorted
    }
    pub(crate) fn inherit_order(&mut self, sorted: bool) {
        self.sorted = sorted;
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

#[derive(Clone, Copy)]
pub(crate) struct ScalarShape {
    pub(crate) rows: i64,
    pub(crate) floats: i64,
    pub(crate) grid: crate::query_scopes::Grid,
}

#[derive(Default)]
struct NodeRoster {
    sample_units: u64,
    primary_statistics: bool,
    scheduled_selector: Option<NodeId>,
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
    sorting: Option<AccountedBox<crate::vector_sort::Plan>>,
    clock_initial: Option<AccountedVec<InitialSample>>,
    merge_guard: Option<MergeGuard>,
    scalar: Option<AccountedBox<ScalarShape>>,
    producer: Option<crate::matrix_heads::Clock>,
    matrix: Option<AccountedBox<crate::matrix_heads::Shape>>,
}
pub(crate) struct Rosters {
    nodes: AccountedVec<NodeRoster>,
    result_node: Option<NodeId>,
    published_samples: u64,
}
impl Rosters {
    pub(crate) fn set_result_node(&mut self, node: NodeId) {
        self.result_node = Some(node);
    }
    pub(crate) fn sample_units(&self, node: NodeId) -> u64 {
        self.nodes[node.0].sample_units
    }
    pub(crate) fn set_statistics_role(&mut self, node: NodeId, primary: bool) {
        self.nodes[node.0].primary_statistics = primary;
    }
    pub(crate) fn publishes_statistics(&self, node: NodeId) -> bool {
        self.nodes[node.0].primary_statistics
    }
    pub(crate) fn published_samples(&self) -> u64 {
        self.published_samples
    }
    pub(crate) fn finish_statistics(
        &mut self,
        node: NodeId,
        units: u64,
        children: u64,
        account: &QueryAccounting,
    ) {
        self.nodes[node.0].sample_units = units;
        if self.publishes_statistics(node) {
            // A completed parent replaces the contributions its children already published.
            self.published_samples = self
                .published_samples
                .checked_sub(children)
                .and_then(|remaining| remaining.checked_add(units))
                .unwrap_or_else(|| {
                    account.statistics_overflow();
                    u64::MAX
                });
            account.queryable_samples(self.published_samples);
        }
    }
    pub(crate) fn scheduled_selector(&self, node: NodeId) -> Option<NodeId> {
        self.nodes[node.0].scheduled_selector
    }
    pub(crate) fn certify_schedule(&mut self, node: NodeId, selector: NodeId) {
        self.nodes[node.0].scheduled_selector = Some(selector);
    }
    pub(crate) fn prepare_matrix(
        &mut self,
        node: NodeId,
        clock: crate::matrix_heads::Clock,
        copied: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        self.nodes[node.0].producer = Some(clock);
        if copied && let Some(output) = &mut self.nodes[node.0].output {
            for descriptor in output.iter_mut() {
                control.check()?;
                account.work(1);
                // Invariant duplication appends on the outer clock, even after an inner sort.
                descriptor.heads.inherit_order(false);
            }
        }
        if clock.grid.forward_without_wrap() {
            return Ok(());
        }
        let mut rows = AccountedVec::new(account, MemoryKind::Workspace);
        for descriptor in self.descriptors(node) {
            control.check()?;
            account.work(1);
            rows.push(crate::matrix_heads::Row {
                ordinal: descriptor.ordinal,
                kinds: descriptor.kinds,
                sorted: descriptor.heads.sorted(),
            });
        }
        self.nodes[node.0].matrix = Some(AccountedBox::new(
            crate::matrix_heads::Shape { clock, rows },
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
        Ok(())
    }

    pub(crate) fn matrix(&self, node: NodeId) -> Option<&crate::matrix_heads::Shape> {
        self.nodes[node.0].matrix.as_deref()
    }

    pub(crate) fn producer_clock(&self, node: NodeId) -> Option<crate::matrix_heads::Clock> {
        self.nodes[node.0].producer
    }

    pub(crate) fn prepare_scalar(
        &mut self,
        node: NodeId,
        shape: ScalarShape,
        account: &QueryAccounting,
    ) {
        self.nodes[node.0].scalar = Some(AccountedBox::new(
            shape,
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
    }

    pub(crate) fn scalar(&self, node: NodeId) -> Option<ScalarShape> {
        self.nodes[node.0].scalar.as_deref().copied()
    }

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
    pub(crate) fn prepare_sort(
        &mut self,
        node: NodeId,
        child: NodeId,
        context: (
            &crate::frontend::ast::ParsedExpression,
            &crate::query_preparation::Prepared,
        ),
        singleton: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let rows = self.projection_input(child, control, account)?;
        let (expression, prepared) = context;
        self.nodes[node.0].sorting = Some(AccountedBox::new(
            crate::vector_sort::Plan::new(
                rows, expression, prepared, node, singleton, control, account,
            )?,
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
        Ok(())
    }
    pub(crate) fn sorting(&self, node: NodeId) -> Option<&crate::vector_sort::Plan> {
        self.nodes[node.0].sorting.as_deref()
    }
    pub(crate) fn finish_sort_projection(
        &mut self,
        node: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let roster = &mut self.nodes[node.0];
        if let (Some(plan), Some(descriptors), Some(sources)) = (
            roster.sorting.as_deref_mut(),
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
    ) -> Result<(), EvaluationError> {
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
        mut set_discovery: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        if set_discovery {
            // Ordered copies lose their source ranges, so inspect aliases before copying.
            for child in children {
                let Some(rows) = &self.nodes[child.0].output else {
                    set_discovery = false;
                    break;
                };
                if !crate::scheduled_expression::unique_rows(rows, control, account)? {
                    set_discovery = false;
                    break;
                }
            }
        }
        let left = self.projection_input(children[0], control, account)?;
        let right = self.projection_input(children[1], control, account)?;
        let mut plan = crate::vector_matching::Plan::new(left, right, matching, control, account)?;
        if set_discovery {
            plan.certify_set_discovery(control, account)?;
        }
        self.nodes[node.0].binary = Some(AccountedBox::new(
            plan,
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
        Ok(())
    }
    pub(crate) fn binary(&self, node: NodeId) -> Option<&crate::vector_matching::Plan> {
        self.nodes[node.0].binary.as_deref()
    }
    pub(crate) fn clear_replay_progress(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        for roster in &*self.nodes {
            control.check()?;
            account.work(1);
            if let Some(plan) = &roster.binary {
                plan.clear_set_discovery();
            }
            if let Some(plan) = &roster.sorting {
                plan.clear_emission();
            }
        }
        control.check()?;
        Ok(())
    }
    pub(crate) fn certify_binary_replay(
        &mut self,
        node: NodeId,
        cardinality: Cardinality,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let roster = &mut self.nodes[node.0];
        let (Some(rows), Some(sources), Some(plan)) =
            (&roster.output, &roster.sources, &mut roster.binary)
        else {
            return Ok(());
        };
        if rows.len() < 2 || !crate::scheduled_expression::unique_rows(rows, control, account)? {
            return Ok(());
        }
        for row in &**rows {
            control.check()?;
            account.work(1);
            if row.kinds != 1 || row.sources.len() != 1 {
                return Ok(());
            }
        }
        plan.certify_replay(sources, cardinality, control, account)
    }
    pub(crate) fn certify_sort_emit(
        &mut self,
        node: NodeId,
        selector: NodeId,
        catalog_len: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if self.scheduled_selector(selector) != Some(selector) {
            return Ok(());
        }
        let roster = &mut self.nodes[node.0];
        if let (Some(rows), Some(sources), Some(plan)) =
            (&roster.output, &roster.sources, &mut roster.sorting)
        {
            plan.certify_emit(rows, sources, catalog_len, control, account)?;
        }
        Ok(())
    }
    pub(crate) fn finish_binary_cache(
        &mut self,
        node: NodeId,
        cache: crate::vector_matching::Cache,
    ) {
        self.nodes[node.0]
            .binary
            .as_deref_mut()
            .expect("binary plan")
            .cache = cache;
    }
    pub(crate) fn new(
        account: &QueryAccounting,
        nodes: usize,
        control: &Control,
    ) -> Result<Self, Interrupted> {
        let mut values = AccountedVec::new(account, MemoryKind::Workspace);
        for _ in 0..nodes {
            control.check()?;
            account.work(1);
            values.push(NodeRoster::default());
        }
        control.check()?;
        Ok(Self {
            nodes: values,
            result_node: None,
            published_samples: 0,
        })
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
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        // Production identities are unique here; identity-changing operators use ordered output.
        let _sort_phase =
            (self.result_node == Some(node)).then(|| account.phase(crate::statistics::Phase::Sort));
        evaluator_order::sort(&mut descriptors, control, account, |left, right| {
            left.ordinal.cmp(&right.ordinal)
        })?;
        if has_multiple(&descriptors, control, account)? {
            let mut sources = AccountedVec::new(account, MemoryKind::Workspace);
            for descriptor in descriptors.iter_mut() {
                control.check()?;
                account.work(1);
                descriptor.sources = sources.len()..sources.len() + 1;
                sources.push(descriptor.ordinal);
            }
            self.nodes[node.0].sources = Some(sources);
        }
        control.check()?;
        self.nodes[node.0].output = Some(descriptors);
        Ok(())
    }
    pub(crate) fn has_merged_unary(
        &self,
        node: NodeId,
        descriptors: &[Descriptor],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        let mut identities = AccountedVec::new(account, MemoryKind::Workspace);
        for descriptor in descriptors {
            control.check()?;
            account.work(1);
            if self.checks_merge(node, descriptor.hash, control, account)? {
                identities.push((descriptor.hash, descriptor.ordinal));
            }
        }
        // The prepared guard excludes unrelated rows; actual output still decides whether a merge occurred.
        evaluator_order::sort(&mut identities, control, account, <(u64, usize)>::cmp)?;
        for pair in identities.windows(2) {
            control.check()?;
            account.work(1);
            if pair[0].0 == pair[1].0 && pair[0].1 != pair[1].1 {
                control.check()?;
                return Ok(true);
            }
        }
        control.check()?;
        Ok(false)
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
        descriptors: AccountedVec<Descriptor>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        self.assemble_output(node, descriptors, false, control, account)
    }
    fn assemble_output(
        &mut self,
        node: NodeId,
        mut descriptors: AccountedVec<Descriptor>,
        sort_collisions: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let _sort_phase =
            (self.result_node == Some(node)).then(|| account.phase(crate::statistics::Phase::Sort));
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
                if sort_collisions {
                    previous.heads.inherit_order(true);
                }
                if !evaluator_order::contains(
                    &sources[previous.sources.clone()],
                    &descriptor.ordinal,
                    control,
                    account,
                )? {
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
            control.check()?;
            account.work(1);
            descriptor.ordinal = ordinal;
        }
        self.nodes[node.0].output = Some(output);
        self.nodes[node.0].sources = Some(sources);
        Ok(())
    }
    pub(crate) fn merged_output(
        &mut self,
        node: NodeId,
        descriptors: AccountedVec<Descriptor>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        self.assemble_output(node, descriptors, true, control, account)
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
            self.merged_output(node, descriptors, control, account)?;
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
        let mut index = PositionIndex::new(account);
        if self.nodes[child.0].sources.is_some() {
            for descriptor in self.nodes[child.0].output.as_deref().unwrap_or(&[]) {
                control.check()?;
                account.work(1);
                let labels =
                    copy_group_key(&descriptor.labels, grouping, without, control, account)?;
                let hash = if !without && grouping.is_none_or(|names| names.is_empty()) {
                    0
                } else {
                    label_hash::grouping(&labels, control, account)?
                };
                if index
                    .find(
                        hash,
                        |position| Ok(groups[position].hash == hash),
                        control,
                        account,
                    )?
                    .is_none()
                {
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
                }
            }
        } else if let Some(mut descriptors) = self.take_output(child) {
            for left in 0..descriptors.len() / 2 {
                control.check()?;
                account.work(1);
                let right = descriptors.len() - 1 - left;
                descriptors.swap(left, right);
            }
            while let Some(mut descriptor) = descriptors.pop() {
                control.check()?;
                account.work(1);
                group_key(&mut descriptor.labels, grouping, without, control, account)?;
                let hash = if !without && grouping.is_none_or(|names| names.is_empty()) {
                    0
                } else {
                    label_hash::grouping(&descriptor.labels, control, account)?
                };
                if index
                    .find(
                        hash,
                        |position| Ok(groups[position].hash == hash),
                        control,
                        account,
                    )?
                    .is_none()
                {
                    let position = groups.len();
                    groups.push(GroupDescriptor {
                        hash,
                        labels: descriptor.labels,
                        whole_drop_name: None,
                    });
                    index.insert_new(
                        hash,
                        position,
                        |prior| Ok(groups[prior].hash),
                        control,
                        account,
                    )?;
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
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    labels.retain_labels_checked(account, control, |label| {
        keep_group_label(&label.name, grouping, without, control, account)
    })
}

pub(crate) fn copy_group_key(
    labels: &OwnedLabels,
    grouping: Option<&[AccountedVec<u8>]>,
    without: bool,
    control: &Control,
    account: &QueryAccounting,
) -> Result<OwnedLabels, Interrupted> {
    let result =
        labels.copy_filtered_checked(account, MemoryKind::Workspace, control, |label| {
            keep_group_label(&label.name, grouping, without, control, account)
        })?;
    control.check()?;
    Ok(result)
}

fn keep_group_label(
    name: &[u8],
    grouping: Option<&[AccountedVec<u8>]>,
    without: bool,
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    let found = crate::binary_labels::named(name, grouping.unwrap_or(&[]), control, account)?;
    Ok(if without {
        name != b"__name__" && !found
    } else {
        found
    })
}

pub(crate) fn group_position<T>(
    groups: &[T],
    hash: u64,
    hash_of: impl Fn(&T) -> u64,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<usize>, Interrupted> {
    control.check()?;
    for (index, group) in groups.iter().enumerate() {
        control.check()?;
        account.work(1);
        if hash_of(group) == hash {
            return Ok(Some(index));
        }
    }
    control.check()?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    #[test]
    fn set_discovery_checks_original_aliases_and_descriptor_presence() {
        for case in 0..4 {
            let engine = crate::accounting::EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut rosters = Rosters::new(&account, 3, &control).unwrap();
            for side in 0..2 {
                let mut rows = AccountedVec::new(&account, MemoryKind::Workspace);
                let count = if side == 0 && case >= 2 { 0 } else { 2 };
                for position in 0..count {
                    let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
                    labels.push(
                        b"id",
                        format!("{side}-{position}").as_bytes(),
                        &account,
                        MemoryKind::Workspace,
                    );
                    let hash = label_hash::full(&labels, &control, &account).unwrap();
                    rows.push(Descriptor {
                        labels,
                        ordinal: position,
                        hash,
                        drop_name: false,
                        first_order: position,
                        sources: position..position + 1,
                        kinds: 1,
                        heads: HeadSummary::default(),
                    });
                }
                let mut sources = AccountedVec::new(&account, MemoryKind::Workspace);
                for position in 0..count {
                    sources.push(position + 30);
                }
                if side == 0 && case == 1 {
                    rows[0].sources = 0..2;
                }
                rosters.nodes[side].output = Some(rows);
                rosters.nodes[side].sources = Some(sources);
            }
            if case == 3 {
                rosters.nodes[0].output = None;
            }
            rosters
                .prepare_binary(
                    NodeId(2),
                    [NodeId(0), NodeId(1)],
                    &VectorMatching::default(),
                    true,
                    &control,
                    &account,
                )
                .unwrap();
            let plan = rosters.binary(NodeId(2)).unwrap();
            assert_eq!(
                plan.has_set_discovery(),
                case == 0 || case == 2,
                "case {case}"
            );
            assert!(
                plan.sides
                    .iter()
                    .flat_map(|side| side.rows.iter())
                    .all(|row| row.sources.is_empty())
            );
            drop(rosters);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn unary_metadata_detection_observes_cancellation_and_avoids_all_pairs() {
        let engine = crate::accounting::EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut rosters = Rosters::new(&account, 1, &control).unwrap();
        let mut descriptors = AccountedVec::new(&account, MemoryKind::Workspace);
        let mut hashes = AccountedVec::new(&account, MemoryKind::Workspace);
        for ordinal in 0..10_000 {
            let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
            labels.push(
                b"id",
                ordinal.to_string().as_bytes(),
                &account,
                MemoryKind::Workspace,
            );
            let hash = label_hash::full(&labels, &control, &account).unwrap();
            hashes.push(hash);
            descriptors.push(Descriptor {
                labels,
                ordinal,
                hash,
                drop_name: false,
                first_order: ordinal,
                sources: 0..0,
                kinds: 1,
                heads: HeadSummary::default(),
            });
        }
        rosters
            .prepare_merge_guard(NodeId(0), hashes, &control, &account)
            .unwrap();
        let before = account.snapshot().work;
        let result = crate::test_storage::interrupt_during(&account, &control, || {
            rosters.has_merged_unary(NodeId(0), &descriptors, &control, &account)
        });
        assert_eq!(result, Err(Interrupted::Cancelled));
        assert!(
            account.snapshot().work - before < 20_000,
            "distinct projected hashes must avoid the all-pairs scan"
        );
        drop((descriptors, rosters));
        account.snapshot().assert_memory_released();
    }

    use super::*;
    use crate::accounting::EngineAccounting;
    use std::collections::VecDeque;

    #[test]
    fn empty_grouping_observes_outer_metadata_work_and_cancellation() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
        for index in 0..10_000 {
            labels.push(
                format!("label_{index:05}").as_bytes(),
                b"value",
                &account,
                MemoryKind::Workspace,
            );
        }
        // Empty grouping still traverses and releases the complete owned label set.
        let result = crate::test_storage::interrupt_during(&account, &control, || {
            group_key(&mut labels, None, false, &control, &account)?;
            control.check()
        });
        assert_eq!(result, Err(Interrupted::Cancelled));
        drop(labels);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn grouping_keeps_raw_empty_keys_and_first_hash_match() {
        let account = EngineAccounting::default().query();
        let control = Control::default();
        for (without, expected) in [
            (false, &[b"a".as_slice(), b"b".as_slice()][..]),
            (true, &[b"c".as_slice()][..]),
        ] {
            let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
            for (name, value) in [
                (b"__name__".as_slice(), b"m".as_slice()),
                (b"a", b""),
                (b"b", b"1"),
                (b"c", b"2"),
            ] {
                labels.push(name, value, &account, MemoryKind::Workspace);
            }
            let names = [b"a", b"b"].map(|name| {
                let mut owned = AccountedVec::new(&account, MemoryKind::Workspace);
                owned.extend_from_slice(name);
                owned
            });
            let copied =
                copy_group_key(&labels, Some(&names), without, &control, &account).unwrap();
            group_key(&mut labels, Some(&names), without, &control, &account).unwrap();
            assert_eq!(copied.compare(&labels), std::cmp::Ordering::Equal);
            assert_eq!(
                label_hash::grouping(&copied, &control, &account).unwrap(),
                label_hash::grouping(&labels, &control, &account).unwrap()
            );
            assert_eq!(
                labels
                    .0
                    .iter()
                    .map(|label| &*label.name)
                    .collect::<Vec<_>>(),
                expected
            );
            if !without {
                assert!(labels.0[0].value.is_empty());
                let raw = label_hash::grouping(&labels, &control, &account).unwrap();
                labels.reset_checked(&account, &control).unwrap();
                assert_eq!(labels.0.len(), 1);
                assert_ne!(
                    label_hash::grouping(&labels, &control, &account).unwrap(),
                    raw
                );
            }
        }
        assert_eq!(
            group_position(&[7, 7, 9], 7, |hash| *hash, &control, &account).unwrap(),
            Some(0)
        );
        assert_eq!(
            group_position(&[7, 7, 9], 8, |hash| *hash, &control, &account).unwrap(),
            None
        );
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn copied_group_keys_keep_named_metadata_empty_values_and_source_order() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut source = OwnedLabels::new(&account, MemoryKind::Workspace);
        for (name, value) in [
            (b"__name__".as_slice(), b"m".as_slice()),
            (b"__type__", b"counter"),
            (b"__unit__", b"bytes"),
            (b"a", b""),
            (b"b", b"value"),
        ] {
            source.push(name, value, &account, MemoryKind::Workspace);
        }
        let names = [b"a".as_slice(), b"__name__", b"__type__", b"__unit__"].map(|name| {
            let mut result = AccountedVec::new(&account, MemoryKind::Workspace);
            result.extend_from_slice(name);
            result
        });
        for (grouping, without, expected) in [
            (
                Some(names.as_slice()),
                false,
                vec![b"__name__".as_slice(), b"__type__", b"__unit__", b"a"],
            ),
            (Some(names.as_slice()), true, vec![b"b".as_slice()]),
            (None, false, vec![]),
            (
                None,
                true,
                vec![b"__type__".as_slice(), b"__unit__", b"a", b"b"],
            ),
        ] {
            let copied = copy_group_key(&source, grouping, without, &control, &account).unwrap();
            assert_eq!(
                copied
                    .0
                    .iter()
                    .map(|label| &*label.name)
                    .collect::<Vec<_>>(),
                expected
            );
            if let Some(label) = copied.0.iter().find(|label| &*label.name == b"a") {
                assert!(label.value.is_empty());
            }
            let mut transferred = source
                .copy_checked(&account, MemoryKind::Workspace, &control)
                .unwrap();
            group_key(&mut transferred, grouping, without, &control, &account).unwrap();
            assert_eq!(copied.compare(&transferred), std::cmp::Ordering::Equal);
        }
        assert_eq!(source.0.len(), 5);
        drop((source, names));
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn copied_group_key_cancellation_releases_omitted_and_partial_byte_walks() {
        for case in 0..3 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut source = OwnedLabels::new(&account, MemoryKind::Workspace);
            let mut names = AccountedVec::new(&account, MemoryKind::Workspace);
            match case {
                0 => {
                    for index in 0..10_000 {
                        source.push(
                            format!("tag_{index:05}").as_bytes(),
                            b"omitted",
                            &account,
                            MemoryKind::Workspace,
                        );
                    }
                }
                1 => {
                    source.push(b"a", &vec![b'x'; 65_536], &account, MemoryKind::Workspace);
                    let mut name = AccountedVec::new(&account, MemoryKind::Workspace);
                    name.extend_from_slice(b"a");
                    names.push(name);
                }
                _ => {
                    let value = vec![b'a'; 16_384];
                    source.push(&value, b"value", &account, MemoryKind::Workspace);
                    let mut name = AccountedVec::new(&account, MemoryKind::Workspace);
                    name.extend_from_slice(&value);
                    names.push(name);
                }
            }
            let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
            let budget = if case == 0 { 64 } else { 8192 };
            let result =
                crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                    copy_group_key(&source, Some(&names), false, &control, &account)
                });
            assert!(matches!(result, Err(Interrupted::Cancelled)), "case {case}");
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].current,
                before
            );
            drop((source, names));
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn copied_group_key_polls_after_its_final_accounted_charge() {
        let mut terminal_work = 0;
        for boundary in 0..3 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut source = OwnedLabels::new(&account, MemoryKind::Workspace);
            source.push(b"a", b"kept", &account, MemoryKind::Workspace);
            source.push(b"b", &vec![b'x'; 65_536], &account, MemoryKind::Workspace);
            let mut name = AccountedVec::new(&account, MemoryKind::Workspace);
            name.extend_from_slice(b"a");
            let before = account.snapshot();
            let operation = || {
                copy_group_key(
                    &source,
                    Some(std::slice::from_ref(&name)),
                    false,
                    &control,
                    &account,
                )
            };
            let result = if boundary == 0 {
                operation()
            } else {
                let budget = if boundary == 1 { 1 } else { terminal_work };
                crate::test_storage::interrupt_after_work(&account, &control, budget, operation)
            };
            if boundary == 0 {
                let labels = result.unwrap();
                assert_eq!(labels.0.len(), 1);
                assert_eq!(&*labels.0[0].name, b"a");
                assert_eq!(&*labels.0[0].value, b"kept");
                terminal_work = account.snapshot().work - before.work;
                assert!(terminal_work > 1 && terminal_work < 65_536);
                drop(labels);
            } else {
                assert!(
                    matches!(result, Err(Interrupted::Cancelled)),
                    "boundary {boundary}"
                );
                if boundary == 2 {
                    assert_eq!(account.snapshot().work - before.work, terminal_work);
                }
            }
            assert_eq!(
                account.snapshot().memory[MemoryKind::Workspace as usize].current,
                before.memory[MemoryKind::Workspace as usize].current
            );
            println!("{{\"group_copy_boundary\":{boundary},\"terminal_work\":{terminal_work}}}");
            drop((source, name));
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn long_grouping_names_and_group_lookup_are_observed_and_cancellable() {
        for names in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let result = if names {
                let name = vec![b'a'; 16_384];
                let mut labels = OwnedLabels::new(&account, MemoryKind::Workspace);
                labels.push(&name, b"value", &account, MemoryKind::Workspace);
                let mut grouping = AccountedVec::new(&account, MemoryKind::Workspace);
                grouping.extend_from_slice(&name);
                crate::test_storage::interrupt_during(&account, &control, || {
                    group_key(&mut labels, Some(&[grouping]), false, &control, &account)
                })
            } else {
                let groups = vec![7u64; 10_000];
                crate::test_storage::interrupt_during(&account, &control, || {
                    group_position(&groups, 8, |hash| *hash, &control, &account).map(|_| ())
                })
            };
            assert_eq!(result, Err(Interrupted::Cancelled));
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

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
