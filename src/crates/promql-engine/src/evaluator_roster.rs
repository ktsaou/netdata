// SPDX-License-Identifier: GPL-3.0-or-later
//! Whole-query descriptors retain labels and ordinals, never raw sample timelines.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    annotations::AnnotationBuilder,
    frontend::ast::NodeId,
    label_hash,
    result::OwnedLabels,
    storage::{Control, Interrupted},
};

pub(crate) struct Descriptor {
    pub(crate) labels: OwnedLabels,
    pub(crate) ordinal: usize,
}
pub(crate) struct GroupDescriptor {
    pub(crate) hash: u64,
    pub(crate) labels: OwnedLabels,
}
#[derive(Default)]
struct NodeRoster {
    output: Option<AccountedVec<Descriptor>>,
    groups: Option<AccountedVec<GroupDescriptor>>,
    annotations: Option<AnnotationBuilder>,
}
pub(crate) struct Rosters {
    nodes: AccountedVec<NodeRoster>,
}
impl Rosters {
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
        self.nodes[node.0].output = None;
    }
    pub(crate) fn output(&mut self, node: NodeId, mut descriptors: AccountedVec<Descriptor>) {
        descriptors.sort_unstable_by_key(|descriptor| descriptor.ordinal);
        self.nodes[node.0].output = Some(descriptors);
    }
    pub(crate) fn groups(&self, node: NodeId) -> Option<&[GroupDescriptor]> {
        self.nodes[node.0].groups.as_deref()
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
        grouping: Option<&[Vec<u8>]>,
        without: bool,
        account: &QueryAccounting,
        control: &Control,
    ) -> Result<(), Interrupted> {
        let mut groups = AccountedVec::<GroupDescriptor>::new(account, MemoryKind::Workspace);
        if let Some(mut descriptors) = self.take_output(child) {
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
                    });
                }
            }
        }
        self.nodes[node.0].groups = Some(groups);
        Ok(())
    }
}
pub(crate) fn group_key(labels: &mut OwnedLabels, grouping: Option<&[Vec<u8>]>, without: bool) {
    labels.retain(|name| {
        let found = grouping.is_some_and(|labels| labels.iter().any(|label| label == name));
        if without {
            name != b"__name__" && !found
        } else {
            found
        }
    });
}
