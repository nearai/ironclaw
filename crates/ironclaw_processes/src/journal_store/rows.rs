use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

use ironclaw_filesystem::{
    CasExpectation, Entry, Filter, IndexKind, IndexName, IndexSpec, IndexValue, Page, RecordKind,
    RecordVersion, RootFilesystem, ScopedFilesystem, SortDirection, StorageTxn, VersionedEntry,
};
use ironclaw_host_api::{ProcessId, ResourceScope, ScopedPath, UserId, VirtualPath};
use serde::{Deserialize, Serialize};

use super::{ProcessJournalMaterializedState, ProcessJournalStoreError};
use crate::{
    ClaimProcessesRequest, JournaledProcessSnapshot, ProcessCheckpointId, ProcessCheckpointRecord,
    ProcessConcurrencyLimits, ProcessControlResult, ProcessDependencyRecord, ProcessInputRecord,
    ProcessJournalEntry, ProcessKind, ProcessLifecycleStatus, ProcessTreeReservation,
    RecoverExpiredProcessLeasesRequest,
};

mod keys;
use keys::{
    index_key, index_name, lineage_scope_key, ordered_index, owner_scope_key, process_kind_key,
    process_status_key, scope_owner_key, scoped_path,
};

const MATERIALIZED_PREFIX: &str = "/processes/materialized";
const MATERIALIZED_KIND: &str = "process_materialized";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "row_type", rename_all = "snake_case")]
pub(super) enum MaterializedRow {
    Metadata {
        next_cursor: u64,
        control_order: VecDeque<String>,
        submission_order: VecDeque<String>,
        legacy_imported: bool,
    },
    Process(JournaledProcessSnapshot),
    Input(ProcessInputRecord),
    Tree {
        root_process_id: ProcessId,
        reservation: ProcessTreeReservation,
    },
    Dependency(ProcessDependencyRecord),
    Checkpoint {
        checkpoint_id: ProcessCheckpointId,
        record: ProcessCheckpointRecord,
    },
    ControlIdempotency {
        key: String,
        result: ProcessControlResult,
    },
    SubmissionIdempotency {
        key: String,
        snapshot: JournaledProcessSnapshot,
    },
    Journal(ProcessJournalEntry),
    Tombstone,
}

pub(super) struct LoadedRows {
    pub(super) state: ProcessJournalMaterializedState,
    rows: HashMap<VirtualPath, MaterializedRow>,
    versions: HashMap<VirtualPath, RecordVersion>,
    pub(super) initialized: bool,
}

#[derive(Default)]
pub(super) struct LoadReferences {
    pub(super) process_ids: Vec<ProcessId>,
    pub(super) tree_roots: Vec<ProcessId>,
    pub(super) dependencies: Vec<(ProcessId, ProcessId)>,
    pub(super) checkpoints: Vec<ProcessCheckpointId>,
    pub(super) submission_idempotency_key: Option<String>,
    pub(super) control_idempotency_key: Option<String>,
    pub(super) active_conflict: Option<(ResourceScope, ProcessKind)>,
    pub(super) claim: Option<(ClaimProcessesRequest, ProcessConcurrencyLimits)>,
    pub(super) recover_expired: Option<RecoverExpiredProcessLeasesRequest>,
}

pub(super) fn retryable_transaction_error(error: &ironclaw_filesystem::FilesystemError) -> bool {
    matches!(
        error,
        ironclaw_filesystem::FilesystemError::BackendBusy { .. }
            | ironclaw_filesystem::FilesystemError::VersionMismatch { .. }
    )
}

pub(super) async fn retry_transaction(attempt: usize) {
    tokio::time::sleep(Duration::from_millis(1_u64 << attempt.min(6))).await;
}

pub(super) async fn ensure_indexes<F>(
    filesystem: &ScopedFilesystem<F>,
) -> Result<(), ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let prefix = scoped_path(&format!("{MATERIALIZED_PREFIX}/journal"))?;
    for spec in [
        IndexSpec::new(
            index_name("process_journal_cursor_v2")?,
            vec![index_key("cursor")?, index_key("process_id")?],
            IndexKind::Exact,
        ),
        IndexSpec::new(
            index_name("process_journal_scope_cursor_v2")?,
            vec![
                index_key("scope_key")?,
                index_key("cursor")?,
                index_key("process_id")?,
            ],
            IndexKind::Exact,
        ),
        IndexSpec::new(
            index_name("process_journal_owner_cursor_v2")?,
            vec![
                index_key("scope_key")?,
                index_key("owner_user_id")?,
                index_key("cursor")?,
                index_key("process_id")?,
            ],
            IndexKind::Exact,
        ),
    ] {
        filesystem
            .ensure_index(&ResourceScope::system(), &prefix, &spec)
            .await?;
    }
    let process_prefix = scoped_path(&format!("{MATERIALIZED_PREFIX}/process"))?;
    for spec in [
        ordered_index(
            "process_queue_v4",
            &["queue_status", "created_at", "process_id"],
        )?,
        ordered_index(
            "process_queue_scope_v4",
            &["queue_status", "scope_key", "created_at", "process_id"],
        )?,
        ordered_index(
            "process_queue_kind_v4",
            &["queue_status", "process_kind", "created_at", "process_id"],
        )?,
        ordered_index(
            "process_queue_scope_kind_v4",
            &[
                "queue_status",
                "scope_key",
                "process_kind",
                "created_at",
                "process_id",
            ],
        )?,
        ordered_index(
            "process_active_conflict_v4",
            &[
                "active_lock",
                "scope_key",
                "process_kind",
                "created_at",
                "process_id",
            ],
        )?,
        ordered_index(
            "process_running_owner_v4",
            &[
                "running",
                "tenant_id",
                "owner_user_id",
                "created_at",
                "process_id",
            ],
        )?,
        ordered_index(
            "process_running_class_v4",
            &[
                "running",
                "tenant_id",
                "concurrency_class",
                "created_at",
                "process_id",
            ],
        )?,
        ordered_index(
            "process_expiry_v4",
            &["expiry_status", "lease_expires_at", "process_id"],
        )?,
        ordered_index(
            "process_expiry_scope_v4",
            &[
                "expiry_status",
                "scope_key",
                "lease_expires_at",
                "process_id",
            ],
        )?,
        ordered_index(
            "process_expiry_kind_v4",
            &[
                "expiry_status",
                "process_kind",
                "lease_expires_at",
                "process_id",
            ],
        )?,
        ordered_index(
            "process_expiry_scope_kind_v4",
            &[
                "expiry_status",
                "scope_key",
                "process_kind",
                "lease_expires_at",
                "process_id",
            ],
        )?,
        ordered_index(
            "process_scope_v3",
            &["scope_key", "created_at", "process_id"],
        )?,
        ordered_index(
            "process_children_v3",
            &["parent_process_id", "created_at", "process_id"],
        )?,
        ordered_index(
            "process_gate_scope_v4",
            &["gate_status", "scope_key", "created_at", "process_id"],
        )?,
        ordered_index(
            "process_gate_owner_scope_v5",
            &["gate_status", "owner_scope_key", "created_at", "process_id"],
        )?,
    ] {
        filesystem
            .ensure_index(&ResourceScope::system(), &process_prefix, &spec)
            .await?;
    }
    let dependency_prefix = scoped_path(&format!("{MATERIALIZED_PREFIX}/dependency"))?;
    for spec in [
        ordered_index(
            "process_dependency_scope_v3",
            &["lineage_scope_key", "created_at", "dependency_id"],
        )?,
        ordered_index(
            "process_dependency_dependent_v3",
            &[
                "lineage_scope_key",
                "dependent_id",
                "created_at",
                "dependency_id",
            ],
        )?,
        ordered_index(
            "process_dependency_unresolved_v3",
            &["closed", "created_at", "dependency_id"],
        )?,
    ] {
        filesystem
            .ensure_index(&ResourceScope::system(), &dependency_prefix, &spec)
            .await?;
    }
    Ok(())
}

pub(super) async fn load<F>(
    filesystem: &ScopedFilesystem<F>,
    references: &LoadReferences,
) -> Result<LoadedRows, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let mut rows_by_path = HashMap::new();
    let mut versions = HashMap::new();
    let metadata_path = scoped_path(&format!("{MATERIALIZED_PREFIX}/metadata"))?;
    let mut records = filesystem
        .get(&ResourceScope::system(), &metadata_path)
        .await?
        .into_iter()
        .collect::<Vec<_>>();
    let (oldest_control_key, oldest_submission_key) = records
        .first()
        .map(decode)
        .transpose()?
        .and_then(|row| match row {
            MaterializedRow::Metadata {
                control_order,
                submission_order,
                ..
            } => Some((
                control_order.front().cloned(),
                submission_order.front().cloned(),
            )),
            _ => None,
        })
        .unwrap_or_default();
    for (collection, key) in [
        ("control", references.control_idempotency_key.as_ref()),
        ("control", oldest_control_key.as_ref()),
        ("submission", references.submission_idempotency_key.as_ref()),
        ("submission", oldest_submission_key.as_ref()),
    ] {
        if let Some(key) = key {
            let path = hashed_scoped_path(collection, key)?;
            push_if_present(filesystem, &mut records, &path).await?;
        }
    }
    for process_id in &references.process_ids {
        let path = process_scoped_path(*process_id)?;
        push_if_present(filesystem, &mut records, &path).await?;
    }
    if let Some((scope, process_kind)) = &references.active_conflict {
        records.extend(query_active_conflict(filesystem, scope, process_kind, 1).await?);
    }
    if let Some((request, limits)) = &references.claim
        && request.process_id_filter.is_none()
    {
        let candidates = query_claim_candidates(filesystem, request).await?;
        records.extend(candidates.clone());
        let snapshots = candidates
            .iter()
            .map(decode_process)
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>, _>>()?;
        let mut queried_owners = HashSet::new();
        let mut queried_classes = HashSet::new();
        for snapshot in snapshots {
            let query_owner = snapshot.owner_user_id.as_ref().is_some_and(|owner| {
                queried_owners.insert((
                    snapshot.scope.tenant_id.as_str().to_string(),
                    owner.as_str().to_string(),
                ))
            });
            let query_class = snapshot.concurrency_class.as_ref().is_some_and(|class| {
                queried_classes.insert((
                    snapshot.scope.tenant_id.as_str().to_string(),
                    class.as_str().to_string(),
                ))
            });
            records.extend(
                query_running_quota_rows(filesystem, &snapshot, limits, query_owner, query_class)
                    .await?,
            );
        }
    }
    if let Some(request) = &references.recover_expired {
        records.extend(query_expired_processes(filesystem, request).await?);
    }
    for root_process_id in &references.tree_roots {
        let path = scoped_path(&format!(
            "{MATERIALIZED_PREFIX}/tree/{}",
            root_process_id.as_uuid()
        ))?;
        push_if_present(filesystem, &mut records, &path).await?;
    }
    for (dependent, dependency) in &references.dependencies {
        let path = scoped_path(&format!(
            "{MATERIALIZED_PREFIX}/dependency/{}/{}",
            dependent.as_uuid(),
            dependency.as_uuid()
        ))?;
        push_if_present(filesystem, &mut records, &path).await?;
    }
    for checkpoint_id in &references.checkpoints {
        let path = checkpoint_scoped_path(checkpoint_id)?;
        push_if_present(filesystem, &mut records, &path).await?;
    }
    let dependency_roots = records
        .iter()
        .map(decode)
        .filter_map(|row| match row {
            Ok(MaterializedRow::Dependency(record)) => Some(Ok(record.root_process_id)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<HashSet<_>, _>>()?;
    for root_process_id in dependency_roots {
        let path = scoped_path(&format!(
            "{MATERIALIZED_PREFIX}/tree/{}",
            root_process_id.as_uuid()
        ))?;
        push_if_present(filesystem, &mut records, &path).await?;
    }
    for versioned in records {
        let row = decode(&versioned)?;
        versions.insert(versioned.path.clone(), versioned.version);
        rows_by_path.insert(versioned.path, row);
    }

    let mut state = ProcessJournalMaterializedState::default();
    let mut initialized = false;
    for row in rows_by_path.values() {
        match row {
            MaterializedRow::Metadata {
                next_cursor,
                control_order,
                submission_order,
                legacy_imported,
            } => {
                initialized = true;
                state.next_cursor = *next_cursor;
                state.control_idempotency_order = control_order.clone();
                state.submission_idempotency_order = submission_order.clone();
                state.legacy_imported = *legacy_imported;
            }
            MaterializedRow::Process(snapshot) => {
                state
                    .processes
                    .insert(snapshot.process_id, snapshot.clone());
            }
            MaterializedRow::Input(record) => {
                state.inputs.insert(record.process_id, record.clone());
            }
            MaterializedRow::Tree {
                root_process_id,
                reservation,
            } => {
                state
                    .tree_reservations
                    .insert(*root_process_id, reservation.clone());
            }
            MaterializedRow::Dependency(record) => {
                state.dependencies.insert(
                    (record.dependent_process_id, record.dependency_process_id),
                    record.clone(),
                );
            }
            MaterializedRow::Checkpoint {
                checkpoint_id,
                record,
            } => {
                state
                    .checkpoints
                    .insert(checkpoint_id.clone(), record.clone());
            }
            MaterializedRow::ControlIdempotency { key, result } => {
                state
                    .control_idempotency
                    .insert(key.clone(), result.clone());
            }
            MaterializedRow::SubmissionIdempotency { key, snapshot } => {
                state
                    .submission_idempotency
                    .insert(key.clone(), snapshot.clone());
            }
            MaterializedRow::Journal(_) => {}
            MaterializedRow::Tombstone => {}
        }
    }
    Ok(LoadedRows {
        state,
        rows: rows_by_path,
        versions,
        initialized,
    })
}

pub(super) async fn is_initialized<F>(
    filesystem: &ScopedFilesystem<F>,
) -> Result<bool, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let metadata_path = scoped_path(&format!("{MATERIALIZED_PREFIX}/metadata"))?;
    let Some(versioned) = filesystem
        .get(&ResourceScope::system(), &metadata_path)
        .await?
    else {
        return Ok(false);
    };
    Ok(matches!(
        decode(&versioned)?,
        MaterializedRow::Metadata { .. }
    ))
}

async fn push_if_present<F>(
    filesystem: &ScopedFilesystem<F>,
    records: &mut Vec<VersionedEntry>,
    path: &ScopedPath,
) -> Result<(), ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    if let Some(versioned) = filesystem.get(&ResourceScope::system(), path).await?
        && !records.iter().any(|record| record.path == versioned.path)
    {
        records.push(versioned);
    }
    Ok(())
}

pub(super) async fn persist_journal(
    filesystem: &impl ProcessFilesystemResolver,
    txn: &mut dyn StorageTxn,
    entries: &[ProcessJournalEntry],
) -> Result<(), ProcessJournalStoreError> {
    for entry in entries {
        let scoped = scoped_path(&format!(
            "{MATERIALIZED_PREFIX}/journal/{:020}",
            entry.cursor.0
        ))?;
        let path = filesystem.resolve_process_path(&scoped)?;
        let row = MaterializedRow::Journal(entry.clone());
        txn.put(&path, encode(&path, &row)?, CasExpectation::Absent)
            .await?;
    }
    Ok(())
}

pub(super) async fn persist(
    filesystem: &impl ProcessFilesystemResolver,
    txn: &mut dyn StorageTxn,
    loaded: &LoadedRows,
    state: &ProcessJournalMaterializedState,
) -> Result<(), ProcessJournalStoreError> {
    let desired = rows_for_state(filesystem, state)?;
    let paths = loaded
        .rows
        .keys()
        .chain(desired.keys())
        .cloned()
        .collect::<HashSet<_>>();
    for path in paths {
        if loaded.initialized && path.as_str().ends_with("/processes/materialized/metadata") {
            // Initialization state is immutable. Runtime cursors come from the
            // backend's transactional sequence allocator, and idempotency
            // records are independently keyed, so unrelated mutations must not
            // contend on one global metadata CAS.
            continue;
        }
        let previous = loaded.rows.get(&path);
        let next = desired.get(&path);
        if previous == next || matches!((previous, next), (Some(MaterializedRow::Tombstone), None))
        {
            continue;
        }
        let row = next.cloned().unwrap_or(MaterializedRow::Tombstone);
        let cas = loaded
            .versions
            .get(&path)
            .copied()
            .map(CasExpectation::Version)
            .unwrap_or(CasExpectation::Absent);
        txn.put(&path, encode(&path, &row)?, cas).await?;
    }
    Ok(())
}

pub(super) fn process_scoped_path(
    process_id: ProcessId,
) -> Result<ScopedPath, ProcessJournalStoreError> {
    scoped_path(&format!(
        "{MATERIALIZED_PREFIX}/process/{}",
        process_id.as_uuid()
    ))
}

pub(super) fn input_scoped_path(
    process_id: ProcessId,
) -> Result<ScopedPath, ProcessJournalStoreError> {
    scoped_path(&format!(
        "{MATERIALIZED_PREFIX}/input/{}",
        process_id.as_uuid()
    ))
}

pub(super) fn checkpoint_scoped_path(
    checkpoint_id: &ProcessCheckpointId,
) -> Result<ScopedPath, ProcessJournalStoreError> {
    let digest = blake3::hash(checkpoint_id.as_str().as_bytes()).to_hex();
    scoped_path(&format!("{MATERIALIZED_PREFIX}/checkpoint/{digest}"))
}

fn hashed_scoped_path(collection: &str, key: &str) -> Result<ScopedPath, ProcessJournalStoreError> {
    let digest = blake3::hash(key.as_bytes()).to_hex();
    scoped_path(&format!("{MATERIALIZED_PREFIX}/{collection}/{digest}"))
}

pub(super) fn decode_process(
    versioned: &VersionedEntry,
) -> Result<Option<JournaledProcessSnapshot>, ProcessJournalStoreError> {
    match decode(versioned)? {
        MaterializedRow::Process(snapshot) => Ok(Some(snapshot)),
        MaterializedRow::Tombstone => Ok(None),
        _ => Err(ProcessJournalStoreError::Deserialization(
            "process row contained another materialized row type".to_string(),
        )),
    }
}

pub(super) fn decode_input(
    versioned: &VersionedEntry,
) -> Result<Option<ProcessInputRecord>, ProcessJournalStoreError> {
    match decode(versioned)? {
        MaterializedRow::Input(record) => Ok(Some(record)),
        MaterializedRow::Tombstone => Ok(None),
        _ => Err(ProcessJournalStoreError::Deserialization(
            "process input row contained another materialized row type".to_string(),
        )),
    }
}

pub(super) fn decode_checkpoint(
    versioned: &VersionedEntry,
) -> Result<Option<ProcessCheckpointRecord>, ProcessJournalStoreError> {
    match decode(versioned)? {
        MaterializedRow::Checkpoint { record, .. } => Ok(Some(record)),
        MaterializedRow::Tombstone => Ok(None),
        _ => Err(ProcessJournalStoreError::Deserialization(
            "process checkpoint row contained another materialized row type".to_string(),
        )),
    }
}

pub(super) async fn processes_for_scope<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
) -> Result<Vec<JournaledProcessSnapshot>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let rows = query_indexed_collection(
        filesystem,
        "process",
        index_name("process_scope_v3")?,
        vec![eq_value(
            "scope_key",
            IndexValue::Text(scope_owner_key(scope)?),
        )?],
        "created_at",
        "process_id",
    )
    .await?;
    decode_process_rows(&rows)
}

pub(super) async fn child_processes<F>(
    filesystem: &ScopedFilesystem<F>,
    parent_process_id: ProcessId,
) -> Result<Vec<JournaledProcessSnapshot>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let rows = query_indexed_collection(
        filesystem,
        "process",
        index_name("process_children_v3")?,
        vec![eq_text(
            "parent_process_id",
            &parent_process_id.as_uuid().to_string(),
        )?],
        "created_at",
        "process_id",
    )
    .await?;
    decode_process_rows(&rows)
}

pub(super) async fn gate_processes<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    owner_scope: bool,
) -> Result<Vec<JournaledProcessSnapshot>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let (index, scope_key, scope_value) = if owner_scope {
        (
            index_name("process_gate_owner_scope_v5")?,
            "owner_scope_key",
            owner_scope_key(scope)?,
        )
    } else {
        (
            index_name("process_gate_scope_v4")?,
            "scope_key",
            scope_owner_key(scope)?,
        )
    };
    let rows = query_indexed_collection(
        filesystem,
        "process",
        index,
        vec![
            eq_text("gate_status", "suspended")?,
            eq_text(scope_key, &scope_value)?,
        ],
        "created_at",
        "process_id",
    )
    .await?;
    decode_process_rows(&rows)
}

pub(super) async fn dependencies_for_scope<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    dependent_process_id: Option<ProcessId>,
) -> Result<Vec<ProcessDependencyRecord>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let mut filters = vec![eq_value(
        "lineage_scope_key",
        IndexValue::Text(lineage_scope_key(scope)?),
    )?];
    let index = if let Some(dependent_process_id) = dependent_process_id {
        filters.push(eq_text(
            "dependent_id",
            &dependent_process_id.as_uuid().to_string(),
        )?);
        index_name("process_dependency_dependent_v3")?
    } else {
        index_name("process_dependency_scope_v3")?
    };
    let rows = query_indexed_collection(
        filesystem,
        "dependency",
        index,
        filters,
        "created_at",
        "dependency_id",
    )
    .await?;
    decode_dependency_rows(&rows)
}

pub(super) async fn unresolved_dependencies<F>(
    filesystem: &ScopedFilesystem<F>,
) -> Result<Vec<ProcessDependencyRecord>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let rows = query_indexed_collection(
        filesystem,
        "dependency",
        index_name("process_dependency_unresolved_v3")?,
        vec![eq_value("closed", IndexValue::Bool(false))?],
        "created_at",
        "dependency_id",
    )
    .await?;
    decode_dependency_rows(&rows)
}

fn decode_process_rows(
    rows: &[VersionedEntry],
) -> Result<Vec<JournaledProcessSnapshot>, ProcessJournalStoreError> {
    rows.iter()
        .map(decode_process)
        .filter_map(Result::transpose)
        .collect()
}

fn decode_dependency_rows(
    rows: &[VersionedEntry],
) -> Result<Vec<ProcessDependencyRecord>, ProcessJournalStoreError> {
    rows.iter()
        .map(|versioned| match decode(versioned)? {
            MaterializedRow::Dependency(record) => Ok(Some(record)),
            MaterializedRow::Tombstone => Ok(None),
            _ => Err(ProcessJournalStoreError::Deserialization(
                "dependency row contained another materialized row type".to_string(),
            )),
        })
        .filter_map(Result::transpose)
        .collect()
}

pub(super) async fn journal_page<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: Option<&ResourceScope>,
    owner_user_id: Option<&UserId>,
    after: u64,
    limit: usize,
) -> Result<Vec<ProcessJournalEntry>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let lower = i64::try_from(after.saturating_add(1)).map_err(|_| {
        ProcessJournalStoreError::InvalidRequest(
            "process journal cursor exceeds indexed integer range".to_string(),
        )
    })?;
    let prefix = scoped_path(&format!("{MATERIALIZED_PREFIX}/journal"))?;
    let (index, filter) = match (scope, owner_user_id) {
        (Some(scope), Some(owner_user_id)) => (
            index_name("process_journal_owner_cursor_v2")?,
            Filter::And(vec![
                Filter::Eq {
                    key: index_key("scope_key")?,
                    value: IndexValue::Text(scope_owner_key(scope)?),
                },
                Filter::Eq {
                    key: index_key("owner_user_id")?,
                    value: IndexValue::Text(owner_user_id.as_str().to_string()),
                },
            ]),
        ),
        (Some(scope), None) => (
            index_name("process_journal_scope_cursor_v2")?,
            Filter::Eq {
                key: index_key("scope_key")?,
                value: IndexValue::Text(scope_owner_key(scope)?),
            },
        ),
        (None, None) => (index_name("process_journal_cursor_v2")?, Filter::All),
        (None, Some(_)) => {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "owner-filtered journal reads require a resource scope".to_string(),
            ));
        }
    };
    let mut ordered_page = ironclaw_filesystem::OrderedPage::new(
        index,
        index_key("cursor")?,
        index_key("process_id")?,
        SortDirection::Ascending,
        u32::try_from(limit).unwrap_or(Page::MAX_LIMIT),
    );
    if after > 0 {
        ordered_page = ordered_page.after(ironclaw_filesystem::OrderedQueryCursor {
            value: IndexValue::I64(lower.saturating_sub(1)),
            tie_breaker: IndexValue::Text("~".to_string()),
        });
    }
    let rows = filesystem
        .query_ordered(&ResourceScope::system(), &prefix, &filter, &ordered_page)
        .await?;
    rows.iter()
        .map(|versioned| match decode(versioned)? {
            MaterializedRow::Journal(entry) => Ok(Some(entry)),
            MaterializedRow::Tombstone => Ok(None),
            _ => Err(ProcessJournalStoreError::Deserialization(
                "journal row contained another materialized row type".to_string(),
            )),
        })
        .filter_map(Result::transpose)
        .collect()
}

const RECOVERY_BATCH_LIMIT: u32 = Page::MAX_LIMIT;
const CLAIM_QUOTA_SKIP_ALLOWANCE: usize = 64;

async fn query_claim_candidates<F>(
    filesystem: &ScopedFilesystem<F>,
    request: &ClaimProcessesRequest,
) -> Result<Vec<VersionedEntry>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    if request.max_processes == 0 {
        return Ok(Vec::new());
    }
    let mut filters = vec![eq_text("queue_status", "queued")?];
    let index = match (&request.scope_filter, &request.process_kind_filter) {
        (Some(scope), Some(kind)) => {
            filters.push(eq_value(
                "scope_key",
                IndexValue::Text(scope_owner_key(scope)?),
            )?);
            filters.push(eq_text("process_kind", &process_kind_key(kind)?)?);
            index_name("process_queue_scope_kind_v4")?
        }
        (Some(scope), None) => {
            filters.push(eq_value(
                "scope_key",
                IndexValue::Text(scope_owner_key(scope)?),
            )?);
            index_name("process_queue_scope_v4")?
        }
        (None, Some(kind)) => {
            filters.push(eq_text("process_kind", &process_kind_key(kind)?)?);
            index_name("process_queue_kind_v4")?
        }
        (None, None) => index_name("process_queue_v4")?,
    };
    let candidate_limit = request
        .max_processes
        .saturating_add(CLAIM_QUOTA_SKIP_ALLOWANCE)
        .min(Page::MAX_LIMIT as usize);
    let candidate_limit = u32::try_from(candidate_limit).unwrap_or(Page::MAX_LIMIT);
    ordered_process_query(filesystem, index, filters, "created_at", candidate_limit).await
}

async fn query_active_conflict<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    process_kind: &ProcessKind,
    limit: u32,
) -> Result<Vec<VersionedEntry>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    ordered_process_query(
        filesystem,
        index_name("process_active_conflict_v4")?,
        vec![
            eq_value("active_lock", IndexValue::Bool(true))?,
            eq_value("scope_key", IndexValue::Text(scope_owner_key(scope)?))?,
            eq_text("process_kind", &process_kind_key(process_kind)?)?,
        ],
        "created_at",
        limit,
    )
    .await
}

async fn query_running_quota_rows<F>(
    filesystem: &ScopedFilesystem<F>,
    candidate: &JournaledProcessSnapshot,
    limits: &ProcessConcurrencyLimits,
    query_owner: bool,
    query_class: bool,
) -> Result<Vec<VersionedEntry>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let mut rows = Vec::new();
    if query_owner
        && let (Some(cap), Some(owner)) = (limits.max_running_per_owner, &candidate.owner_user_id)
    {
        rows.extend(
            ordered_process_query(
                filesystem,
                index_name("process_running_owner_v4")?,
                vec![
                    eq_value("running", IndexValue::Bool(true))?,
                    eq_text("tenant_id", candidate.scope.tenant_id.as_str())?,
                    eq_text("owner_user_id", owner.as_str())?,
                ],
                "created_at",
                cap,
            )
            .await?,
        );
    }
    if query_class
        && let Some(class) = &candidate.concurrency_class
        && let Some(cap) = limits.max_running_by_class.get(class)
    {
        rows.extend(
            ordered_process_query(
                filesystem,
                index_name("process_running_class_v4")?,
                vec![
                    eq_value("running", IndexValue::Bool(true))?,
                    eq_text("tenant_id", candidate.scope.tenant_id.as_str())?,
                    eq_text("concurrency_class", class.as_str())?,
                ],
                "created_at",
                *cap,
            )
            .await?,
        );
    }
    Ok(rows)
}

async fn query_expired_processes<F>(
    filesystem: &ScopedFilesystem<F>,
    request: &RecoverExpiredProcessLeasesRequest,
) -> Result<Vec<VersionedEntry>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let mut rows = Vec::new();
    for status in ["running", "cancel_requested"] {
        let mut filters = vec![eq_text("expiry_status", status)?];
        let index = match (&request.scope_filter, &request.process_kind_filter) {
            (Some(scope), Some(kind)) => {
                filters.push(eq_value(
                    "scope_key",
                    IndexValue::Text(scope_owner_key(scope)?),
                )?);
                filters.push(eq_text("process_kind", &process_kind_key(kind)?)?);
                index_name("process_expiry_scope_kind_v4")?
            }
            (Some(scope), None) => {
                filters.push(eq_value(
                    "scope_key",
                    IndexValue::Text(scope_owner_key(scope)?),
                )?);
                index_name("process_expiry_scope_v4")?
            }
            (None, Some(kind)) => {
                filters.push(eq_text("process_kind", &process_kind_key(kind)?)?);
                index_name("process_expiry_kind_v4")?
            }
            (None, None) => index_name("process_expiry_v4")?,
        };
        let candidates = ordered_process_query(
            filesystem,
            index,
            filters,
            "lease_expires_at",
            RECOVERY_BATCH_LIMIT,
        )
        .await?;
        for candidate in candidates {
            let Some(snapshot) = decode_process(&candidate)? else {
                continue;
            };
            let expired = snapshot
                .lease
                .as_ref()
                .and_then(|lease| lease.lease_expires_at)
                .is_some_and(|expires_at| expires_at <= request.now);
            if !expired {
                break;
            }
            rows.push(candidate);
        }
    }
    Ok(rows)
}

async fn ordered_process_query<F>(
    filesystem: &ScopedFilesystem<F>,
    index: IndexName,
    filters: Vec<Filter>,
    sort_key: &str,
    limit: u32,
) -> Result<Vec<VersionedEntry>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    if limit == 0 {
        return Ok(Vec::new());
    }
    let prefix = scoped_path(&format!("{MATERIALIZED_PREFIX}/process"))?;
    let filter = if filters.len() == 1 {
        filters.into_iter().next().ok_or_else(|| {
            ProcessJournalStoreError::InvalidRequest(
                "ordered process query lost its required filter".to_string(),
            )
        })?
    } else {
        Filter::And(filters)
    };
    let page = ironclaw_filesystem::OrderedPage::new(
        index,
        index_key(sort_key)?,
        index_key("process_id")?,
        SortDirection::Ascending,
        limit,
    );
    filesystem
        .query_ordered(&ResourceScope::system(), &prefix, &filter, &page)
        .await
        .map_err(Into::into)
}

fn eq_text(key: &str, value: &str) -> Result<Filter, ProcessJournalStoreError> {
    eq_value(key, IndexValue::Text(value.to_string()))
}

fn eq_value(key: &str, value: IndexValue) -> Result<Filter, ProcessJournalStoreError> {
    Ok(Filter::Eq {
        key: index_key(key)?,
        value,
    })
}

pub(super) trait ProcessFilesystemResolver {
    fn resolve_process_path(
        &self,
        path: &ScopedPath,
    ) -> Result<VirtualPath, ironclaw_filesystem::FilesystemError>;
}

impl<F> ProcessFilesystemResolver for ScopedFilesystem<F>
where
    F: RootFilesystem,
{
    fn resolve_process_path(
        &self,
        path: &ScopedPath,
    ) -> Result<VirtualPath, ironclaw_filesystem::FilesystemError> {
        self.resolve(&ResourceScope::system(), path)
    }
}

async fn query_indexed_collection<F>(
    filesystem: &ScopedFilesystem<F>,
    collection: &str,
    index: IndexName,
    filters: Vec<Filter>,
    sort_key: &str,
    tie_breaker: &str,
) -> Result<Vec<VersionedEntry>, ProcessJournalStoreError>
where
    F: RootFilesystem,
{
    let prefix = scoped_path(&format!("{MATERIALIZED_PREFIX}/{collection}"))?;
    let filter = if filters.len() == 1 {
        filters.into_iter().next().ok_or_else(|| {
            ProcessJournalStoreError::InvalidRequest(
                "indexed collection query lost its required filter".to_string(),
            )
        })?
    } else {
        Filter::And(filters)
    };
    let sort_key = index_key(sort_key)?;
    let tie_breaker = index_key(tie_breaker)?;
    let mut cursor = None;
    let mut records = Vec::new();
    loop {
        let mut page = ironclaw_filesystem::OrderedPage::new(
            index.clone(),
            sort_key.clone(),
            tie_breaker.clone(),
            SortDirection::Ascending,
            Page::MAX_LIMIT,
        );
        if let Some(after) = cursor.take() {
            page = page.after(after);
        }
        let batch = filesystem
            .query_ordered(&ResourceScope::system(), &prefix, &filter, &page)
            .await?;
        let count = batch.len();
        cursor = batch.last().and_then(|row| {
            Some(ironclaw_filesystem::OrderedQueryCursor {
                value: row.entry.indexed.get(&sort_key)?.clone(),
                tie_breaker: row.entry.indexed.get(&tie_breaker)?.clone(),
            })
        });
        records.extend(batch);
        if count < Page::MAX_LIMIT as usize {
            break;
        }
        if cursor.is_none() {
            return Err(ProcessJournalStoreError::Deserialization(
                "indexed collection row omitted pagination keys".to_string(),
            ));
        }
    }
    Ok(records)
}

fn rows_for_state(
    filesystem: &impl ProcessFilesystemResolver,
    state: &ProcessJournalMaterializedState,
) -> Result<HashMap<VirtualPath, MaterializedRow>, ProcessJournalStoreError> {
    let mut rows = HashMap::new();
    insert_scoped(
        &mut rows,
        filesystem,
        &format!("{MATERIALIZED_PREFIX}/metadata"),
        MaterializedRow::Metadata {
            next_cursor: state.next_cursor,
            control_order: state.control_idempotency_order.clone(),
            submission_order: state.submission_idempotency_order.clone(),
            legacy_imported: state.legacy_imported,
        },
    )?;
    for snapshot in state.processes.values() {
        insert_scoped(
            &mut rows,
            filesystem,
            &format!(
                "{MATERIALIZED_PREFIX}/process/{}",
                snapshot.process_id.as_uuid()
            ),
            MaterializedRow::Process(snapshot.clone()),
        )?;
    }
    for record in state.inputs.values() {
        insert_scoped(
            &mut rows,
            filesystem,
            &format!(
                "{MATERIALIZED_PREFIX}/input/{}",
                record.process_id.as_uuid()
            ),
            MaterializedRow::Input(record.clone()),
        )?;
    }
    for (root, reservation) in &state.tree_reservations {
        insert_scoped(
            &mut rows,
            filesystem,
            &format!("{MATERIALIZED_PREFIX}/tree/{}", root.as_uuid()),
            MaterializedRow::Tree {
                root_process_id: *root,
                reservation: reservation.clone(),
            },
        )?;
    }
    for ((dependent, dependency), record) in &state.dependencies {
        insert_scoped(
            &mut rows,
            filesystem,
            &format!(
                "{MATERIALIZED_PREFIX}/dependency/{}/{}",
                dependent.as_uuid(),
                dependency.as_uuid()
            ),
            MaterializedRow::Dependency(record.clone()),
        )?;
    }
    for (checkpoint_id, record) in &state.checkpoints {
        insert_hashed(
            &mut rows,
            filesystem,
            "checkpoint",
            checkpoint_id.as_str(),
            MaterializedRow::Checkpoint {
                checkpoint_id: checkpoint_id.clone(),
                record: record.clone(),
            },
        )?;
    }
    for (key, result) in &state.control_idempotency {
        insert_hashed(
            &mut rows,
            filesystem,
            "control",
            key,
            MaterializedRow::ControlIdempotency {
                key: key.clone(),
                result: result.clone(),
            },
        )?;
    }
    for (key, snapshot) in &state.submission_idempotency {
        insert_hashed(
            &mut rows,
            filesystem,
            "submission",
            key,
            MaterializedRow::SubmissionIdempotency {
                key: key.clone(),
                snapshot: snapshot.clone(),
            },
        )?;
    }
    Ok(rows)
}

fn insert_hashed(
    rows: &mut HashMap<VirtualPath, MaterializedRow>,
    filesystem: &impl ProcessFilesystemResolver,
    collection: &str,
    key: &str,
    row: MaterializedRow,
) -> Result<(), ProcessJournalStoreError> {
    let digest = blake3::hash(key.as_bytes()).to_hex();
    insert_scoped(
        rows,
        filesystem,
        &format!("{MATERIALIZED_PREFIX}/{collection}/{digest}"),
        row,
    )
}

fn insert_scoped(
    rows: &mut HashMap<VirtualPath, MaterializedRow>,
    filesystem: &impl ProcessFilesystemResolver,
    path: &str,
    row: MaterializedRow,
) -> Result<(), ProcessJournalStoreError> {
    let scoped = ScopedPath::new(path).map_err(|error| {
        ProcessJournalStoreError::InvalidPath(super::invalid_path(error).to_string())
    })?;
    rows.insert(filesystem.resolve_process_path(&scoped)?, row);
    Ok(())
}

pub(super) fn encode(
    path: &VirtualPath,
    row: &MaterializedRow,
) -> Result<Entry, ProcessJournalStoreError> {
    let kind = RecordKind::new(MATERIALIZED_KIND).map_err(|error| {
        ProcessJournalStoreError::InvalidPath(super::invalid_path(error).to_string())
    })?;
    let value = serde_json::to_value(row)
        .map_err(|error| ProcessJournalStoreError::Serialization(error.to_string()))?;
    let mut entry = Entry::record(kind, &value)
        .map_err(|error| ProcessJournalStoreError::Serialization(error.to_string()))?;
    let row_type = value
        .get("row_type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ProcessJournalStoreError::Serialization(format!(
                "materialized row at {path} has no row_type"
            ))
        })?;
    entry = entry.with_indexed(
        index_key("row_type")?,
        IndexValue::Text(row_type.to_string()),
    );
    if let MaterializedRow::Process(snapshot) = row {
        let created_at = snapshot.created_at.timestamp_micros();
        entry = entry
            .with_indexed(
                index_key("process_id")?,
                IndexValue::Text(snapshot.process_id.as_uuid().to_string()),
            )
            .with_indexed(
                index_key("tenant_id")?,
                IndexValue::Text(snapshot.scope.tenant_id.as_str().to_string()),
            )
            .with_indexed(
                index_key("scope_key")?,
                IndexValue::Text(scope_owner_key(&snapshot.scope)?),
            )
            .with_indexed(
                index_key("owner_scope_key")?,
                IndexValue::Text(owner_scope_key(&snapshot.scope)?),
            )
            .with_indexed(
                index_key("process_kind")?,
                IndexValue::Text(process_kind_key(&snapshot.process_kind)?),
            )
            .with_indexed(index_key("created_at")?, IndexValue::I64(created_at));
        if let Some(owner_user_id) = &snapshot.owner_user_id {
            entry = entry.with_indexed(
                index_key("owner_user_id")?,
                IndexValue::Text(owner_user_id.as_str().to_string()),
            );
        }
        if let Some(concurrency_class) = &snapshot.concurrency_class {
            entry = entry.with_indexed(
                index_key("concurrency_class")?,
                IndexValue::Text(concurrency_class.as_str().to_string()),
            );
        }
        if let Some(parent_process_id) = snapshot.parent_process_id {
            entry = entry.with_indexed(
                index_key("parent_process_id")?,
                IndexValue::Text(parent_process_id.as_uuid().to_string()),
            );
        }
        if snapshot.status == ProcessLifecycleStatus::Queued {
            entry = entry.with_indexed(
                index_key("queue_status")?,
                IndexValue::Text("queued".to_string()),
            );
        }
        if snapshot.status.keeps_active_lock() {
            entry = entry.with_indexed(index_key("active_lock")?, IndexValue::Bool(true));
        }
        if snapshot.status == ProcessLifecycleStatus::Running {
            entry = entry.with_indexed(index_key("running")?, IndexValue::Bool(true));
        }
        if matches!(
            snapshot.status,
            ProcessLifecycleStatus::Running | ProcessLifecycleStatus::CancelRequested
        ) && let Some(lease_expires_at) = snapshot
            .lease
            .as_ref()
            .and_then(|lease| lease.lease_expires_at)
        {
            entry = entry
                .with_indexed(
                    index_key("expiry_status")?,
                    IndexValue::Text(process_status_key(snapshot.status)?),
                )
                .with_indexed(
                    index_key("lease_expires_at")?,
                    IndexValue::I64(lease_expires_at.timestamp_micros()),
                );
        }
        if snapshot.status == ProcessLifecycleStatus::Suspended {
            entry = entry.with_indexed(
                index_key("gate_status")?,
                IndexValue::Text("suspended".to_string()),
            );
        }
    }
    if let MaterializedRow::Dependency(record) = row {
        entry = entry
            .with_indexed(
                index_key("lineage_scope_key")?,
                IndexValue::Text(lineage_scope_key(&record.scope)?),
            )
            .with_indexed(
                index_key("dependent_id")?,
                IndexValue::Text(record.dependent_process_id.as_uuid().to_string()),
            )
            .with_indexed(
                index_key("dependency_id")?,
                IndexValue::Text(record.dependency_process_id.as_uuid().to_string()),
            )
            .with_indexed(
                index_key("created_at")?,
                IndexValue::I64(record.created_at.timestamp_micros()),
            )
            .with_indexed(
                index_key("closed")?,
                IndexValue::Bool(matches!(
                    record.state,
                    crate::ProcessDependencyState::Consumed
                        | crate::ProcessDependencyState::Abandoned
                )),
            );
    }
    if let MaterializedRow::Journal(journal_entry) = row {
        let cursor = i64::try_from(journal_entry.cursor.0).map_err(|_| {
            ProcessJournalStoreError::Serialization(format!(
                "process journal cursor {} exceeds indexed integer range",
                journal_entry.cursor.0
            ))
        })?;
        entry = entry
            .with_indexed(index_key("cursor")?, IndexValue::I64(cursor))
            .with_indexed(
                index_key("process_id")?,
                IndexValue::Text(journal_entry.process_id.as_uuid().to_string()),
            )
            .with_indexed(
                index_key("scope_key")?,
                IndexValue::Text(scope_owner_key(&journal_entry.scope)?),
            )
            .with_indexed(
                index_key("owner_user_id")?,
                IndexValue::Text(
                    journal_entry
                        .owner_user_id
                        .as_ref()
                        .map(|owner| owner.as_str().to_string())
                        .unwrap_or_default(),
                ),
            );
    }
    Ok(entry)
}

pub(super) fn decode(
    versioned: &VersionedEntry,
) -> Result<MaterializedRow, ProcessJournalStoreError> {
    serde_json::from_slice(&versioned.entry.body)
        .map_err(|error| ProcessJournalStoreError::Deserialization(error.to_string()))
}
