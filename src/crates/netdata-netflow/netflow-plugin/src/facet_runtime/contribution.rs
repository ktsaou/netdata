use super::store::FacetStore;
use crate::facet_catalog::facet_field_spec;
use crate::flow::FlowFields;
use crate::presentation;
use crate::query::{payload_value, split_payload_bytes};
use std::collections::{BTreeMap, BTreeSet};
use std::mem::size_of;

#[derive(Debug, Clone, Default)]
pub(crate) struct FacetFileContribution {
    fields: BTreeMap<&'static str, FacetStore>,
}

impl FacetFileContribution {
    pub(super) fn field(&self, field: &str) -> Option<&FacetStore> {
        self.fields.get(field)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&'static str, &FacetStore)> + '_ {
        self.fields.iter().map(|(field, store)| (*field, store))
    }

    pub(super) fn from_scanned_values(values: BTreeMap<String, BTreeSet<String>>) -> Self {
        let mut contribution = Self::default();

        for (field, stored_values) in values {
            let Some(spec) = facet_field_spec(field.as_str()) else {
                continue;
            };

            let store = contribution
                .fields
                .entry(spec.name)
                .or_insert_with(|| FacetStore::new(spec.kind));
            for value in stored_values {
                let _ = store.insert_raw(&value);
            }
        }

        contribution
    }

    pub(super) fn insert_raw(&mut self, field: &'static str, value: &str) {
        let Some(spec) = facet_field_spec(field) else {
            return;
        };
        let store = self
            .fields
            .entry(spec.name)
            .or_insert_with(|| FacetStore::new(spec.kind));
        let _ = store.insert_raw(value);
    }

    pub(super) fn estimated_heap_bytes(&self) -> usize {
        self.fields.len() * (size_of::<&'static str>() + size_of::<FacetStore>())
            + self
                .fields
                .values()
                .map(FacetStore::estimated_heap_bytes)
                .sum::<usize>()
    }
}

pub(crate) fn facet_contribution_from_flow_fields(fields: &FlowFields) -> FacetFileContribution {
    let mut contribution = FacetFileContribution::default();
    let protocol = fields.get("PROTOCOL").map(String::as_str);
    let icmpv4_type = fields.get("ICMPV4_TYPE").map(String::as_str);
    let icmpv4_code = fields.get("ICMPV4_CODE").map(String::as_str);
    let icmpv6_type = fields.get("ICMPV6_TYPE").map(String::as_str);
    let icmpv6_code = fields.get("ICMPV6_CODE").map(String::as_str);

    for (field, value) in fields {
        if facet_field_spec(field).is_some() && !value.is_empty() {
            contribution.insert_raw(field, value);
        }
    }

    if let Some(value) =
        presentation::icmp_virtual_value("ICMPV4", protocol, icmpv4_type, icmpv4_code)
    {
        contribution.insert_raw("ICMPV4", &value);
    }
    if let Some(value) =
        presentation::icmp_virtual_value("ICMPV6", protocol, icmpv6_type, icmpv6_code)
    {
        contribution.insert_raw("ICMPV6", &value);
    }

    contribution
}

pub(crate) fn facet_contribution_from_encoded_fields<'a, I>(fields: I) -> FacetFileContribution
where
    I: IntoIterator<Item = &'a [u8]>,
{
    let mut contribution = FacetFileContribution::default();
    let mut protocol = None;
    let mut icmpv4_type = None;
    let mut icmpv4_code = None;
    let mut icmpv6_type = None;
    let mut icmpv6_code = None;

    for payload in fields {
        let Some((key_bytes, value_bytes)) = split_payload_bytes(payload) else {
            continue;
        };
        let Ok(field) = std::str::from_utf8(key_bytes) else {
            continue;
        };
        let value = payload_value(value_bytes);
        if value.is_empty() {
            continue;
        }

        match field {
            "PROTOCOL" => protocol = Some(value.into_owned()),
            "ICMPV4_TYPE" => icmpv4_type = Some(value.into_owned()),
            "ICMPV4_CODE" => icmpv4_code = Some(value.into_owned()),
            "ICMPV6_TYPE" => icmpv6_type = Some(value.into_owned()),
            "ICMPV6_CODE" => icmpv6_code = Some(value.into_owned()),
            _ => {
                if let Some(spec) = facet_field_spec(field) {
                    contribution.insert_raw(spec.name, value.as_ref());
                }
            }
        }
    }

    if let Some(value) = presentation::icmp_virtual_value(
        "ICMPV4",
        protocol.as_deref(),
        icmpv4_type.as_deref(),
        icmpv4_code.as_deref(),
    ) {
        contribution.insert_raw("ICMPV4", &value);
    }
    if let Some(value) = presentation::icmp_virtual_value(
        "ICMPV6",
        protocol.as_deref(),
        icmpv6_type.as_deref(),
        icmpv6_code.as_deref(),
    ) {
        contribution.insert_raw("ICMPV6", &value);
    }

    contribution
}
