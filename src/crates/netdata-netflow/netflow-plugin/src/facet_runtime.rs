mod contribution;
mod sidecar;
mod store;

use crate::facet_catalog::{FACET_ALLOWED_OPTIONS, FACET_FIELD_SPECS, facet_field_spec};
use crate::query::{
    FACET_CACHE_JOURNAL_WINDOW_SIZE, FACET_VALUE_LIMIT, accumulate_simple_closed_file_facet_values,
    accumulate_targeted_facet_values, facet_field_requires_protocol_scan,
    virtual_flow_field_dependencies,
};
use anyhow::{Context, Result};
use journal_core::file::JournalFileMap;
use journal_registry::FileInfo;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::Notify;

pub(crate) use contribution::{
    FacetFileContribution, facet_contribution_from_encoded_fields,
    facet_contribution_from_flow_fields,
};
use sidecar::{delete_sidecar_files, search_sidecar, write_sidecar_files};
use store::{FacetStore, PersistedFacetStore};

const FACET_STATE_VERSION: u32 = 4;
const FACET_STATE_FILE_NAME: &str = "facet-state.bin";
const FACET_AUTOCOMPLETE_LIMIT: usize = 100;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct FacetPublishedField {
    pub(crate) total_values: usize,
    pub(crate) autocomplete: bool,
    pub(crate) values: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FacetPublishedSnapshot {
    pub(crate) fields: BTreeMap<String, FacetPublishedField>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FacetMemoryBreakdown {
    pub(crate) archived_bytes: u64,
    pub(crate) active_bytes: u64,
    pub(crate) active_contributions_bytes: u64,
    pub(crate) published_bytes: u64,
    pub(crate) archived_path_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedFacetState {
    version: u32,
    indexed_archived_paths: BTreeSet<String>,
    archived_fields: BTreeMap<String, PersistedFacetStore>,
    published: BTreeMap<String, FacetPublishedField>,
}

#[derive(Debug, Clone)]
struct FacetState {
    indexed_archived_paths: BTreeSet<String>,
    archived_fields: BTreeMap<String, FacetStore>,
    active_fields: BTreeMap<String, FacetStore>,
    active_contributions: BTreeMap<String, FacetFileContribution>,
    published: FacetPublishedSnapshot,
    dirty: bool,
    rebuild_archived: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FacetReconcilePlan {
    pub(crate) current_archived_paths: BTreeSet<String>,
    pub(crate) archived_files_to_scan: Vec<FileInfo>,
    pub(crate) active_files_to_scan: Vec<FileInfo>,
    pub(crate) rebuild_archived: bool,
}

pub(crate) struct FacetRuntime {
    state: Mutex<FacetState>,
    snapshot: Arc<RwLock<Arc<FacetPublishedSnapshot>>>,
    ready: AtomicBool,
    ready_notify: Notify,
    state_path: PathBuf,
}

impl FacetRuntime {
    pub(crate) fn new(base_dir: &Path) -> Self {
        let state_path = base_dir.join(FACET_STATE_FILE_NAME);
        let loaded = load_persisted_state(&state_path);
        let ready = loaded.is_some();
        let state = loaded
            .map(FacetState::from_persisted)
            .unwrap_or_else(FacetState::new);
        let snapshot = Arc::new(RwLock::new(Arc::new(state.published.clone())));

        Self {
            state: Mutex::new(state),
            snapshot,
            ready: AtomicBool::new(ready),
            ready_notify: Notify::new(),
            state_path,
        }
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub(crate) fn snapshot(&self) -> Arc<FacetPublishedSnapshot> {
        self.snapshot
            .read()
            .map(|guard| Arc::clone(&guard))
            .unwrap_or_else(|_| Arc::new(FacetPublishedSnapshot::default()))
    }

    pub(crate) fn estimated_memory_breakdown(&self) -> FacetMemoryBreakdown {
        let Ok(state) = self.state.lock() else {
            return FacetMemoryBreakdown::default();
        };

        FacetMemoryBreakdown {
            archived_bytes: estimate_store_map_bytes(&state.archived_fields) as u64,
            active_bytes: estimate_store_map_bytes(&state.active_fields) as u64,
            active_contributions_bytes: state
                .active_contributions
                .iter()
                .map(|(path, contribution)| path.capacity() + contribution.estimated_heap_bytes())
                .sum::<usize>() as u64,
            published_bytes: estimate_published_snapshot_bytes(&state.published) as u64,
            archived_path_bytes: state
                .indexed_archived_paths
                .iter()
                .map(|path| path.capacity())
                .sum::<usize>() as u64,
        }
    }

    pub(crate) fn build_reconcile_plan(&self, registry_files: &[FileInfo]) -> FacetReconcilePlan {
        let current_archived_paths = registry_files
            .iter()
            .filter(|file_info| file_info.file.is_archived())
            .map(|file_info| file_info.file.path().to_string())
            .collect::<BTreeSet<_>>();
        let _current_active_paths = registry_files
            .iter()
            .filter(|file_info| file_info.file.is_active())
            .map(|file_info| file_info.file.path().to_string())
            .collect::<BTreeSet<_>>();

        let Some(state) = self.state.lock().ok() else {
            return FacetReconcilePlan {
                current_archived_paths,
                archived_files_to_scan: registry_files
                    .iter()
                    .filter(|file_info| file_info.file.is_archived())
                    .cloned()
                    .collect(),
                active_files_to_scan: registry_files
                    .iter()
                    .filter(|file_info| file_info.file.is_active())
                    .cloned()
                    .collect(),
                rebuild_archived: true,
            };
        };

        let rebuild_archived = state.rebuild_archived
            || !state
                .indexed_archived_paths
                .is_subset(&current_archived_paths);
        let archived_files_to_scan = registry_files
            .iter()
            .filter(|file_info| {
                file_info.file.is_archived()
                    && (rebuild_archived
                        || !state.indexed_archived_paths.contains(file_info.file.path()))
            })
            .cloned()
            .collect::<Vec<_>>();
        let active_files_to_scan = registry_files
            .iter()
            .filter(|file_info| file_info.file.is_active())
            .cloned()
            .collect::<Vec<_>>();

        FacetReconcilePlan {
            current_archived_paths,
            archived_files_to_scan,
            active_files_to_scan,
            rebuild_archived,
        }
    }

    pub(crate) fn apply_reconcile_plan(
        &self,
        plan: FacetReconcilePlan,
        archived_scans: BTreeMap<String, FacetFileContribution>,
        active_scans: BTreeMap<String, FacetFileContribution>,
    ) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("facet runtime lock poisoned"))?;

        let removed_archived = state
            .indexed_archived_paths
            .iter()
            .filter(|path| !plan.current_archived_paths.contains(*path))
            .cloned()
            .collect::<Vec<_>>();
        for path in removed_archived {
            state.indexed_archived_paths.remove(&path);
            delete_sidecar_files(Path::new(&path));
            state.rebuild_archived = true;
            state.dirty = true;
        }
        if plan.rebuild_archived {
            state.archived_fields = empty_field_stores();
            state.indexed_archived_paths.clear();
            state.dirty = true;
        }

        for (path, contribution) in archived_scans {
            merge_global_contribution(&mut state.archived_fields, &contribution);
            write_sidecar_files(Path::new(&path), &contribution)?;
            if state.indexed_archived_paths.insert(path) {
                state.dirty = true;
            }
        }

        state.active_contributions.clear();
        for (path, contribution) in active_scans {
            state.active_contributions.insert(path, contribution);
            state.dirty = true;
        }
        rebuild_active_fields(&mut state);
        rebuild_published_fields(&mut state);
        state.rebuild_archived = false;

        publish_locked(&self.snapshot, &state);
        persist_state_locked(&self.state_path, &mut state)?;
        drop(state);
        self.mark_ready();
        Ok(())
    }

    pub(crate) fn observe_active_contribution(
        &self,
        path: &Path,
        contribution: FacetFileContribution,
    ) -> Result<()> {
        let Some(path_str) = path.to_str() else {
            return Ok(());
        };

        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("facet runtime lock poisoned"))?;
        let changed = apply_active_contribution(&mut state, path_str, contribution);
        state.dirty = true;
        if changed {
            publish_locked(&self.snapshot, &state);
        }

        Ok(())
    }

    pub(crate) fn observe_rotation(&self, archived_path: &Path, active_path: &Path) -> Result<()> {
        let archived_path_str = archived_path.to_string_lossy().to_string();
        let active_path_str = active_path.to_string_lossy().to_string();
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("facet runtime lock poisoned"))?;

        let contribution = state
            .active_contributions
            .remove(&archived_path_str)
            .or_else(|| state.active_contributions.remove(&active_path_str));
        if let Some(contribution) = contribution {
            merge_global_contribution(&mut state.archived_fields, &contribution);
            rebuild_active_fields(&mut state);
            rebuild_published_fields(&mut state);
            write_sidecar_files(archived_path, &contribution)?;
            state.indexed_archived_paths.insert(archived_path_str);
            state.dirty = true;
            publish_locked(&self.snapshot, &state);
            persist_state_locked(&self.state_path, &mut state)?;
        }

        Ok(())
    }

    pub(crate) fn observe_deleted_paths(&self, deleted_paths: &[PathBuf]) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("facet runtime lock poisoned"))?;
        let mut changed = false;

        for path in deleted_paths {
            let Some(path_str) = path.to_str() else {
                continue;
            };
            if state.active_contributions.remove(path_str).is_some() {
                changed = true;
            }
            if state.indexed_archived_paths.remove(path_str) {
                state.rebuild_archived = true;
                changed = true;
            }
            delete_sidecar_files(path);
        }

        if changed {
            if !state.rebuild_archived {
                rebuild_active_fields(&mut state);
                rebuild_published_fields(&mut state);
                publish_locked(&self.snapshot, &state);
            }
            state.dirty = true;
            persist_state_locked(&self.state_path, &mut state)?;
        }

        Ok(())
    }

    pub(crate) fn autocomplete(&self, field: &str, term: &str) -> Result<Vec<String>> {
        let normalized = field.trim().to_ascii_uppercase();
        let Some(spec) = facet_field_spec(&normalized) else {
            return Ok(Vec::new());
        };

        let (promoted, mut matches, archived_paths) = {
            let state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("facet runtime lock poisoned"))?;
            let Some(published) = state.published.fields.get(normalized.as_str()) else {
                return Ok(Vec::new());
            };
            let active_matches = state
                .active_fields
                .get(normalized.as_str())
                .map(|store| store.prefix_matches(term, FACET_AUTOCOMPLETE_LIMIT))
                .unwrap_or_default();
            let archived_matches = if !spec.uses_sidecar || !published.autocomplete {
                state
                    .archived_fields
                    .get(normalized.as_str())
                    .map(|store| store.prefix_matches(term, FACET_AUTOCOMPLETE_LIMIT))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let archived_paths = state
                .indexed_archived_paths
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            (
                published.autocomplete,
                merge_autocomplete_values(active_matches, archived_matches),
                archived_paths,
            )
        };

        if spec.uses_sidecar && promoted && matches.len() < FACET_AUTOCOMPLETE_LIMIT {
            for path in archived_paths {
                let needed = FACET_AUTOCOMPLETE_LIMIT.saturating_sub(matches.len());
                if needed == 0 {
                    break;
                }
                let sidecar_matches =
                    search_sidecar(Path::new(&path), normalized.as_str(), term, needed)?;
                matches = merge_autocomplete_values(matches, sidecar_matches);
                if matches.len() >= FACET_AUTOCOMPLETE_LIMIT {
                    break;
                }
            }
        }

        Ok(matches)
    }

    pub(crate) fn persist_if_dirty(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("facet runtime lock poisoned"))?;
        persist_state_locked(&self.state_path, &mut state)
    }

    fn mark_ready(&self) {
        let was_ready = self.ready.swap(true, Ordering::AcqRel);
        if !was_ready {
            self.ready_notify.notify_waiters();
        }
    }
}

impl FacetState {
    fn new() -> Self {
        let archived_fields = empty_field_stores();
        let active_fields = empty_field_stores();
        Self {
            indexed_archived_paths: BTreeSet::new(),
            published: published_snapshot_from_field_stores(&archived_fields, &active_fields),
            archived_fields,
            active_fields,
            active_contributions: BTreeMap::new(),
            dirty: false,
            rebuild_archived: false,
        }
    }

    fn from_persisted(persisted: PersistedFacetState) -> Self {
        let mut archived_fields = empty_field_stores();
        for spec in FACET_FIELD_SPECS.iter() {
            if let Some(saved) = persisted.archived_fields.get(spec.name) {
                archived_fields.insert(
                    spec.name.to_string(),
                    FacetStore::from_persisted(spec.kind, saved.clone()),
                );
            }
        }

        let active_fields = empty_field_stores();

        Self {
            indexed_archived_paths: persisted.indexed_archived_paths,
            published: if persisted.published.is_empty() {
                published_snapshot_from_field_stores(&archived_fields, &active_fields)
            } else {
                FacetPublishedSnapshot {
                    fields: persisted.published,
                }
            },
            archived_fields,
            active_fields,
            active_contributions: BTreeMap::new(),
            dirty: false,
            rebuild_archived: false,
        }
    }
}

pub(crate) fn scan_registry_file_contribution(
    file_info: &FileInfo,
) -> Result<FacetFileContribution> {
    let requested_fields = FACET_ALLOWED_OPTIONS.clone();
    let simple_fields = requested_fields
        .iter()
        .filter(|field| !facet_field_requires_protocol_scan(field))
        .cloned()
        .collect::<Vec<_>>();
    let mut values = BTreeMap::new();
    let file_path = PathBuf::from(file_info.file.path());
    let journal = JournalFileMap::open(&file_info.file, FACET_CACHE_JOURNAL_WINDOW_SIZE)
        .with_context(|| {
            format!(
                "failed to open netflow journal {} for facet contribution scan",
                file_info.file.path()
            )
        })?;
    accumulate_simple_closed_file_facet_values(&journal, &simple_fields, &mut values)
        .with_context(|| {
            format!(
                "failed to enumerate simple facet values from {}",
                file_info.file.path()
            )
        })?;
    if requested_fields.iter().any(|field| field == "ICMPV4") {
        accumulate_targeted_facet_values(
            std::slice::from_ref(&file_path),
            "ICMPV4",
            &[("PROTOCOL".to_string(), "1".to_string())],
            virtual_flow_field_dependencies("ICMPV4"),
            &mut values,
        )
        .with_context(|| {
            format!(
                "failed to enumerate ICMPv4 facet values from {}",
                file_path.display()
            )
        })?;
    }
    if requested_fields.iter().any(|field| field == "ICMPV6") {
        accumulate_targeted_facet_values(
            std::slice::from_ref(&file_path),
            "ICMPV6",
            &[("PROTOCOL".to_string(), "58".to_string())],
            virtual_flow_field_dependencies("ICMPV6"),
            &mut values,
        )
        .with_context(|| {
            format!(
                "failed to enumerate ICMPv6 facet values from {}",
                file_path.display()
            )
        })?;
    }
    Ok(FacetFileContribution::from_scanned_values(values))
}

fn empty_field_stores() -> BTreeMap<String, FacetStore> {
    FACET_FIELD_SPECS
        .iter()
        .map(|spec| (spec.name.to_string(), FacetStore::new(spec.kind)))
        .collect()
}

fn merge_global_contribution(
    fields: &mut BTreeMap<String, FacetStore>,
    contribution: &FacetFileContribution,
) -> bool {
    let mut changed = false;
    for (field, store) in contribution.iter() {
        match fields.entry(field.to_string()) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                changed |= entry.get_mut().merge_from(store);
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(store.clone());
                changed = true;
            }
        }
    }
    changed
}

fn rebuild_active_fields(state: &mut FacetState) {
    state.active_fields = empty_field_stores();
    for contribution in state.active_contributions.values() {
        merge_global_contribution(&mut state.active_fields, contribution);
    }
}

fn published_snapshot_from_field_stores(
    archived_fields: &BTreeMap<String, FacetStore>,
    active_fields: &BTreeMap<String, FacetStore>,
) -> FacetPublishedSnapshot {
    let mut fields = BTreeMap::new();

    for spec in FACET_FIELD_SPECS.iter() {
        let mut combined = archived_fields
            .get(spec.name)
            .cloned()
            .unwrap_or_else(|| FacetStore::new(spec.kind));
        if let Some(active_store) = active_fields.get(spec.name) {
            let _ = combined.merge_from(active_store);
        }
        let total_values = combined.len();
        let autocomplete = spec.supports_autocomplete && total_values > FACET_VALUE_LIMIT;
        let values = if autocomplete {
            Vec::new()
        } else {
            combined.collect_strings(None)
        };
        fields.insert(
            spec.name.to_string(),
            FacetPublishedField {
                total_values,
                autocomplete,
                values,
            },
        );
    }

    FacetPublishedSnapshot { fields }
}

fn rebuild_published_fields(state: &mut FacetState) {
    state.published =
        published_snapshot_from_field_stores(&state.archived_fields, &state.active_fields);
}

fn apply_new_active_value_to_published(
    published: &mut FacetPublishedSnapshot,
    field: &str,
    value: &str,
) {
    let Some(spec) = facet_field_spec(field) else {
        return;
    };
    let entry = published
        .fields
        .entry(spec.name.to_string())
        .or_insert_with(FacetPublishedField::default);

    entry.total_values = entry.total_values.saturating_add(1);
    if spec.supports_autocomplete && entry.total_values > FACET_VALUE_LIMIT {
        entry.autocomplete = true;
        entry.values.clear();
        return;
    }

    entry.values.push(value.to_string());
    entry.values.sort_unstable();
}

fn apply_active_contribution(
    state: &mut FacetState,
    path_str: &str,
    contribution: FacetFileContribution,
) -> bool {
    let entry = state
        .active_contributions
        .entry(path_str.to_string())
        .or_default();
    let mut changed = false;

    for (field, store) in contribution.iter() {
        let Some(spec) = facet_field_spec(field) else {
            continue;
        };

        let active_store = state
            .active_fields
            .entry(spec.name.to_string())
            .or_insert_with(|| FacetStore::new(spec.kind));
        for value in store.collect_strings(None) {
            if entry
                .field(field)
                .is_some_and(|existing| existing.contains_raw(&value))
            {
                continue;
            }

            entry.insert_raw(field, &value);
            if !active_store.insert_raw(&value) {
                continue;
            }
            if state
                .archived_fields
                .get(spec.name)
                .is_some_and(|archived| archived.contains_raw(&value))
            {
                continue;
            }

            apply_new_active_value_to_published(&mut state.published, spec.name, &value);
            changed = true;
        }
    }

    changed
}

fn publish_locked(snapshot: &Arc<RwLock<Arc<FacetPublishedSnapshot>>>, state: &FacetState) {
    let published = Arc::new(state.published.clone());
    if let Ok(mut guard) = snapshot.write() {
        *guard = published;
    }
}

fn persist_state_locked(state_path: &Path, state: &mut FacetState) -> Result<()> {
    if !state.dirty {
        return Ok(());
    }

    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to prepare facet state directory {}",
                parent.display()
            )
        })?;
    }

    let persisted = PersistedFacetState {
        version: FACET_STATE_VERSION,
        indexed_archived_paths: state.indexed_archived_paths.clone(),
        archived_fields: state
            .archived_fields
            .iter()
            .filter(|(_, store)| store.len() > 0)
            .map(|(field, store)| (field.clone(), store.persist()))
            .collect(),
        published: state.published.fields.clone(),
    };
    let payload = bincode::serialize(&persisted).context("failed to serialize facet state")?;
    let tmp_path = state_path.with_extension("bin.tmp");
    fs::write(&tmp_path, &payload).with_context(|| {
        format!(
            "failed to write temporary facet state {}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, state_path).with_context(|| {
        format!(
            "failed to move temporary facet state {} to {}",
            tmp_path.display(),
            state_path.display()
        )
    })?;
    state.dirty = false;
    Ok(())
}

fn estimate_store_map_bytes(fields: &BTreeMap<String, FacetStore>) -> usize {
    fields
        .iter()
        .map(|(field, store)| field.capacity() + size_of::<String>() + store.estimated_heap_bytes())
        .sum()
}

fn estimate_published_snapshot_bytes(snapshot: &FacetPublishedSnapshot) -> usize {
    snapshot
        .fields
        .iter()
        .map(|(field, published)| {
            field.capacity()
                + size_of::<String>()
                + published
                    .values
                    .iter()
                    .map(|value| value.capacity() + size_of::<String>())
                    .sum::<usize>()
        })
        .sum()
}

fn load_persisted_state(state_path: &Path) -> Option<PersistedFacetState> {
    let payload = match fs::read(state_path) {
        Ok(payload) => payload,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!(
                "failed to read persisted netflow facet state {}: {}",
                state_path.display(),
                err
            );
            return None;
        }
    };
    let persisted = match bincode::deserialize::<PersistedFacetState>(&payload) {
        Ok(persisted) => persisted,
        Err(err) => {
            tracing::warn!(
                "failed to decode persisted netflow facet state {}: {}",
                state_path.display(),
                err
            );
            return None;
        }
    };
    if persisted.version != FACET_STATE_VERSION {
        tracing::warn!(
            "ignoring persisted netflow facet state {} due to version mismatch {} != {}",
            state_path.display(),
            persisted.version,
            FACET_STATE_VERSION
        );
        return None;
    }
    Some(persisted)
}

fn merge_autocomplete_values(left: Vec<String>, right: Vec<String>) -> Vec<String> {
    let mut merged = left.into_iter().collect::<BTreeSet<_>>();
    merged.extend(right);
    let mut out = merged.into_iter().collect::<Vec<_>>();
    out.truncate(FACET_AUTOCOMPLETE_LIMIT);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::FlowFields;

    fn fields_with_protocol(protocol: &str) -> FlowFields {
        let mut fields = FlowFields::new();
        fields.insert("PROTOCOL", protocol.to_string());
        fields
    }

    #[test]
    fn runtime_rebuilds_global_values_after_archived_deletion() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let runtime = FacetRuntime::new(tmp.path());
        let retained = facet_contribution_from_flow_fields(&fields_with_protocol("17"));

        runtime
            .observe_active_contribution(
                Path::new("/tmp/flows-a.journal"),
                facet_contribution_from_flow_fields(&fields_with_protocol("6")),
            )
            .expect("observe first contribution");
        runtime
            .observe_active_contribution(Path::new("/tmp/flows-b.journal"), retained.clone())
            .expect("observe second contribution");
        runtime
            .observe_rotation(
                Path::new("/tmp/flows-a.journal"),
                Path::new("/tmp/flows-a-next.journal"),
            )
            .expect("rotate first file");
        runtime
            .observe_deleted_paths(&[PathBuf::from("/tmp/flows-a.journal")])
            .expect("delete first file");
        runtime
            .apply_reconcile_plan(
                FacetReconcilePlan {
                    current_archived_paths: BTreeSet::new(),
                    archived_files_to_scan: Vec::new(),
                    active_files_to_scan: Vec::new(),
                    rebuild_archived: true,
                },
                BTreeMap::new(),
                BTreeMap::from([(String::from("/tmp/flows-b.journal"), retained)]),
            )
            .expect("rebuild after deletion");

        let snapshot = runtime.snapshot();
        let protocol = snapshot.fields.get("PROTOCOL").expect("protocol field");
        assert_eq!(protocol.total_values, 1);
        assert!(
            !protocol.autocomplete,
            "small protocol domains should stay inline"
        );
    }

    #[test]
    fn runtime_persists_and_reloads_compact_field_stores() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let runtime = FacetRuntime::new(tmp.path());
        let mut fields = fields_with_protocol("6");
        fields.insert("SRC_ADDR", "192.0.2.10".to_string());
        fields.insert("SRC_AS_NAME", "AS15169 GOOGLE".to_string());

        runtime
            .observe_active_contribution(
                Path::new("/tmp/flows-a.journal"),
                facet_contribution_from_flow_fields(&fields),
            )
            .expect("observe active contribution");
        runtime.persist_if_dirty().expect("persist runtime");

        let reloaded = FacetRuntime::new(tmp.path());
        let snapshot = reloaded.snapshot();

        assert_eq!(
            snapshot
                .fields
                .get("PROTOCOL")
                .expect("protocol")
                .total_values,
            1
        );
        assert_eq!(
            snapshot
                .fields
                .get("SRC_ADDR")
                .expect("src addr")
                .total_values,
            1
        );
        assert_eq!(
            snapshot
                .fields
                .get("SRC_AS_NAME")
                .expect("src as")
                .total_values,
            1
        );
    }

    #[test]
    fn runtime_autocomplete_reads_promoted_archived_sidecar_values() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let runtime = FacetRuntime::new(tmp.path());
        let archived_path = Path::new("/tmp/flows-promoted.journal");

        for value in 0..120 {
            let mut fields = FlowFields::new();
            fields.insert("SRC_AS_NAME", format!("AS{value:03} EXAMPLE"));
            runtime
                .observe_active_contribution(
                    archived_path,
                    facet_contribution_from_flow_fields(&fields),
                )
                .expect("observe contribution");
        }

        runtime
            .observe_rotation(archived_path, Path::new("/tmp/flows-promoted-next.journal"))
            .expect("rotate file");

        let results = runtime
            .autocomplete("SRC_AS_NAME", "AS11")
            .expect("autocomplete values");

        assert!(
            results.iter().any(|value| value == "AS110 EXAMPLE"),
            "expected promoted sidecar autocomplete to return archived values"
        );
    }
}
