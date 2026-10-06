// SPDX-License-Identifier: GPL-3.0-or-later
//! Whole-query descriptors retain labels and ordinals, never raw sample timelines.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::AnnotationBuilder,
    evaluator_order,
    frontend::ast::{NodeId, VectorMatching},
    label_hash,
    result::OwnedLabels,
    storage::{Control, Interrupted},
};

pub(crate) struct Descriptor {
    pub(crate) labels: OwnedLabels,
    pub(crate) ordinal: usize,
    pub(crate) hash: u64,
    pub(crate) drop_name: bool,
    pub(crate) first_order: usize,
    pub(crate) sources: std::ops::Range<usize>,
    pub(crate) kinds: u8,
    // Only the earliest dual-kind logical step is needed to replay monotone vector reads.
    pub(crate) collision_time_ms: Option<i64>,
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
}
pub(crate) struct Rosters {
    nodes: AccountedVec<NodeRoster>,
}
impl Rosters {
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
            .any(|descriptor| descriptor.collision_time_ms.is_some())
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
                previous.collision_time_ms =
                    earliest(previous.collision_time_ms, descriptor.collision_time_ms);
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
        evaluator_order::sort(&mut output, control, account, |left, right| {
            left.labels.compare(&right.labels)
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
                collision_time_ms: descriptor.collision_time_ms,
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
        kinds: &[u8],
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
            if *kind == 3 {
                descriptor.collision_time_ms =
                    earliest(descriptor.collision_time_ms, Some(time_ms));
            }
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
pub(crate) fn earliest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
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
