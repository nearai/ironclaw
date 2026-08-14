//! Durable approval and gate records.
//!
//! Process and capability-invocation lifecycle state belongs to
//! `ironclaw_processes`; this crate owns only approval persistence and gates.
//!
//! Durable approval persistence is provided by [`ApprovalRequestStore`] and
//! [`GateRecordStore`] over a
//! [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem). The
//! `RootFilesystem` choice (libSQL-backed, PostgreSQL-backed, in-memory, or
//! local-disk) is made at the filesystem layer — the consumer-store level no
//! longer carries per-backend impls.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, FilesystemError, RecordKind, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    approval::ApprovalRequest,
    error::HostApiError,
    gate_record::GateRecord,
    ids::{ApprovalRequestId, GateRef},
    path::ScopedPath,
    resource::ResourceScope,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Approval request lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
    Discarded,
}

/// Durable approval request record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub scope: ResourceScope,
    pub request: ApprovalRequest,
    pub status: ApprovalStatus,
}

/// Approval and gate persistence errors.
#[derive(Debug, Error)]
pub enum ApprovalStoreError {
    #[error("unknown approval request {request_id}")]
    UnknownApprovalRequest { request_id: ApprovalRequestId },
    #[error("approval request {request_id} already exists")]
    ApprovalRequestAlreadyExists { request_id: ApprovalRequestId },
    #[error("gate record {gate_ref} already exists")]
    GateRecordAlreadyExists { gate_ref: GateRef },
    #[error("approval request {request_id} is not pending (status: {status:?})")]
    ApprovalNotPending {
        request_id: ApprovalRequestId,
        status: ApprovalStatus,
    },
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
    #[error("filesystem error: {0}")]
    Filesystem(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("deserialization error: {0}")]
    Deserialization(String),
    #[error("approval backend error: {0}")]
    Backend(String),
}

impl From<FilesystemError> for ApprovalStoreError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error.to_string())
    }
}

/// Store for approval requests emitted by authorization decisions.
#[async_trait]
pub trait ApprovalRequestStorePort: Send + Sync {
    /// Persists a pending approval request in the exact resource-owner scope without resolving it.
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, ApprovalStoreError>;

    /// Loads one scoped approval record; wrong-scope lookups must look unknown.
    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, ApprovalStoreError>;

    /// Marks a pending approval request approved only within the matching scope.
    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, ApprovalStoreError>;

    /// Marks a pending approval request denied only within the matching scope.
    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, ApprovalStoreError>;

    /// Discards a still-pending approval request during rollback before it becomes user-actionable.
    ///
    /// Stores that can delete pending records should override this method. The default is a
    /// fail-closed tombstone fallback that marks the record denied rather than leaving a
    /// user-actionable pending approval behind.
    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        self.deny(scope, request_id).await
    }

    /// Lists approval records visible to the exact resource-owner scope only.
    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, ApprovalStoreError>;
}

/// Durable store for the model-visible [`GateRecord`] a pending gate renders
/// from (arch-simplification §5.2.9).
///
/// A [`Resolution`](ironclaw_host_api::resolution::Resolution) control-plane arm carries only
/// an opaque [`GateRef`]; the content the loop renders the pending gate from —
/// the auth gate's credential requirements (G3), the dependent-run staged result
/// handle (G2), the redacted summary — lives in the referenced [`GateRecord`].
/// Because a gate blocks on one turn and resumes on a **later** turn, that record
/// must outlive the turn that produced it, so it needs persistence keyed by
/// `GateRef`. (The sibling terminal `DenyRecord` is same-turn and needs no store.)
///
/// This port mirrors [`ApprovalRequestStorePort`]: resource-owner scoped, wrong-scope
/// lookups look unknown. It intentionally exposes no removal method — a
/// `GateRecord` is host-owned, write-once, model-visible content with no status
/// field to tombstone, so (unlike `ApprovalRequestStorePort::discard_pending`, which
/// tombstones a lifecycle *status*) there is no scope-safe soft-delete to mirror,
/// and hard deletion of retained model-visible data would need an explicit
/// product/retention contract (`database.md` "Data safety").
#[async_trait]
pub trait GateRecordStorePort: Send + Sync {
    /// Persists the gate record for `gate_ref` in the exact resource-owner scope.
    ///
    /// Write-once: a `gate_ref` that already has a record is a
    /// [`ApprovalStoreError::GateRecordAlreadyExists`], mirroring
    /// [`ApprovalRequestStorePort::save_pending`]. `GateRef`s are freshly minted per
    /// gate, so a collision is a caller invariant violation, not an update path.
    async fn save(
        &self,
        scope: ResourceScope,
        gate_ref: GateRef,
        record: GateRecord,
    ) -> Result<(), ApprovalStoreError>;

    /// Loads the gate record for `gate_ref`; a wrong-scope lookup must look
    /// unknown (`Ok(None)`), never leak another owner's record.
    async fn load(
        &self,
        scope: &ResourceScope,
        gate_ref: GateRef,
    ) -> Result<Option<GateRecord>, ApprovalStoreError>;
}

/// `RecordKind` tag written on every approval-request entry for the same
/// fail-closed CAS gate as other typed records.
const APPROVAL_RECORD_KIND: &str = "approval_record";

/// `RecordKind` tag written on every gate-record entry for the same
/// fail-closed CAS gate as other typed records.
const GATE_RECORD_KIND: &str = "gate_record";

/// Filesystem-backed approval request store under the `/approvals` mount alias.
///
/// Tenant/user isolation is supplied by the scoped mount view; the remaining
/// resource-owner dimensions are encoded structurally in record paths.
pub struct ApprovalRequestStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> ApprovalRequestStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn record_entry(record: &ApprovalRecord) -> Result<Entry, ApprovalStoreError> {
        let body = serialize_pretty(record)?;
        let kind = RecordKind::new(APPROVAL_RECORD_KIND)
            .map_err(|e| ApprovalStoreError::Backend(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry)
    }

    async fn read_versioned(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<(ApprovalRecord, ironclaw_filesystem::RecordVersion)>, ApprovalStoreError>
    {
        let path = approval_record_path(scope, request_id)?;
        let Some(versioned) = self.filesystem.get(scope, &path).await? else {
            return Ok(None);
        };
        let record = deserialize::<ApprovalRecord>(&versioned.entry.body)?;
        if same_scope_owner(&record.scope, scope) {
            Ok(Some((record, versioned.version)))
        } else {
            Ok(None)
        }
    }

    /// Read-modify-write an approval record using the shared lock-free CAS helper.
    ///
    /// Uses the shared bounded CAS helper, with no lock held across `.await`.
    async fn update_status(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        status: ApprovalStatus,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        let path = approval_record_path(scope, request_id)?;
        let scope_clone = scope.clone();
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            |bytes: &[u8]| deserialize::<ApprovalRecord>(bytes),
            |record: &ApprovalRecord| Self::record_entry(record),
            |current: Option<ApprovalRecord>| {
                // Compute the outcome synchronously so the async block only
                // captures an already-resolved `Result` (mirrors cas_snapshot.rs).
                let outcome = (|| {
                    let mut record =
                        current.ok_or(ApprovalStoreError::UnknownApprovalRequest { request_id })?;
                    // Enforce scope ownership on each retry against a freshly read record.
                    if !same_scope_owner(&record.scope, &scope_clone) {
                        return Err(ApprovalStoreError::UnknownApprovalRequest { request_id });
                    }
                    if record.status != ApprovalStatus::Pending {
                        return Err(ApprovalStoreError::ApprovalNotPending {
                            request_id,
                            status: record.status,
                        });
                    }
                    record.status = status;
                    Ok(CasApply::new(record.clone(), record))
                })();
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)
    }
}

#[async_trait]
impl<F> ApprovalRequestStorePort for ApprovalRequestStore<F>
where
    F: RootFilesystem,
{
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        let path = approval_record_path(&scope, request.id)?;
        let request_id = request.id;
        let record = ApprovalRecord {
            scope: scope.clone(),
            request,
            status: ApprovalStatus::Pending,
        };
        cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            |bytes: &[u8]| deserialize::<ApprovalRecord>(bytes),
            |r: &ApprovalRecord| Self::record_entry(r),
            |current: Option<ApprovalRecord>| {
                let fresh = record.clone();
                let outcome = if current.is_some() {
                    Err(ApprovalStoreError::ApprovalRequestAlreadyExists { request_id })
                } else {
                    Ok(CasApply::new(fresh.clone(), fresh))
                };
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, ApprovalStoreError> {
        Ok(self
            .read_versioned(scope, request_id)
            .await?
            .map(|(record, _)| record)
            .filter(|record| record.status != ApprovalStatus::Discarded))
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        self.update_status(scope, request_id, ApprovalStatus::Approved)
            .await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        self.update_status(scope, request_id, ApprovalStatus::Denied)
            .await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, ApprovalStoreError> {
        let path = approval_record_path(scope, request_id)?;
        let scope_clone = scope.clone();
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            |bytes: &[u8]| deserialize::<ApprovalRecord>(bytes),
            |record: &ApprovalRecord| Self::record_entry(record),
            |current: Option<ApprovalRecord>| {
                // Compute the outcome synchronously so the async block only
                // captures an already-resolved `Result` (mirrors cas_snapshot.rs).
                let outcome = (|| {
                    let record =
                        current.ok_or(ApprovalStoreError::UnknownApprovalRequest { request_id })?;
                    // Enforce scope ownership on each retry against a freshly read record.
                    if !same_scope_owner(&record.scope, &scope_clone) {
                        return Err(ApprovalStoreError::UnknownApprovalRequest { request_id });
                    }
                    if record.status != ApprovalStatus::Pending {
                        return Err(ApprovalStoreError::ApprovalNotPending {
                            request_id,
                            status: record.status,
                        });
                    }
                    // Write a Discarded tombstone so the file still exists (preventing
                    // a subsequent save_pending from re-using the same ID), but return
                    // the original Pending record as the caller-visible outcome.
                    // If approve()/deny() raced and won, the CAS put fails with
                    // VersionMismatch → retry re-reads → sees non-Pending → returns
                    // ApprovalNotPending without clobbering the resolved record.
                    let original = record.clone();
                    let mut discarded = record;
                    discarded.status = ApprovalStatus::Discarded;
                    Ok(CasApply::new(discarded, original))
                })();
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, ApprovalStoreError> {
        let root = approval_records_root(scope)?;
        let entries = match self.filesystem.list_dir(scope, &root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            if entry.name.ends_with(".json") {
                // `list_dir` returns post-resolution `VirtualPath`s; rebuild the
                // alias-relative `ScopedPath` so the follow-up `get` runs
                // through the per-op ACL.
                let child = join_scoped(&root, &entry.name)?;
                let Some(versioned) = self.filesystem.get(scope, &child).await? else {
                    continue;
                };
                let record = deserialize::<ApprovalRecord>(&versioned.entry.body)?;
                if same_scope_owner(&record.scope, scope)
                    && record.status != ApprovalStatus::Discarded
                {
                    records.push(record);
                }
            }
        }
        records.sort_by_key(|record| record.request.id.as_uuid());
        Ok(records)
    }
}

/// Durable wrapper carrying the resource-owner scope alongside the host-owned
/// [`GateRecord`]. `GateRecord` is a `host_api` vocabulary type with no scope
/// field; persisting the scope beside it lets [`GateRecordStore::load`]
/// apply the same `same_scope_owner` defense-in-depth check the sibling
/// [`ApprovalRecord`] does, so a wrong-scope read looks unknown. The scope is
/// storage metadata only — `load` returns the bare [`GateRecord`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredGateRecord {
    scope: ResourceScope,
    record: GateRecord,
}

/// Filesystem-backed gate-record store under the `/gate-records` mount alias.
///
/// Tenant/user isolation is supplied by the scoped mount view; the remaining
/// resource-owner dimensions are encoded structurally in record paths.
pub struct GateRecordStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> GateRecordStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn record_entry(record: &StoredGateRecord) -> Result<Entry, ApprovalStoreError> {
        let body = serialize_pretty(record)?;
        let kind = RecordKind::new(GATE_RECORD_KIND)
            .map_err(|e| ApprovalStoreError::Backend(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry)
    }
}

#[async_trait]
impl<F> GateRecordStorePort for GateRecordStore<F>
where
    F: RootFilesystem,
{
    async fn save(
        &self,
        scope: ResourceScope,
        gate_ref: GateRef,
        record: GateRecord,
    ) -> Result<(), ApprovalStoreError> {
        let path = gate_record_path(&scope, gate_ref)?;
        let stored = StoredGateRecord {
            scope: scope.clone(),
            record,
        };
        cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            |bytes: &[u8]| deserialize::<StoredGateRecord>(bytes),
            |r: &StoredGateRecord| Self::record_entry(r),
            |current: Option<StoredGateRecord>| {
                let fresh = stored.clone();
                // Write-once: reject a duplicate ref rather than clobbering the
                // host-owned record a later resume turn still needs.
                let outcome = if current.is_some() {
                    Err(ApprovalStoreError::GateRecordAlreadyExists { gate_ref })
                } else {
                    Ok(CasApply::new(fresh, ()))
                };
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn load(
        &self,
        scope: &ResourceScope,
        gate_ref: GateRef,
    ) -> Result<Option<GateRecord>, ApprovalStoreError> {
        let path = gate_record_path(scope, gate_ref)?;
        let Some(versioned) = self.filesystem.get(scope, &path).await? else {
            return Ok(None);
        };
        let stored = deserialize::<StoredGateRecord>(&versioned.entry.body)?;
        // Defense-in-depth against a shared-path read; wrong scope looks unknown.
        if same_scope_owner(&stored.scope, scope) {
            Ok(Some(stored.record))
        } else {
            Ok(None)
        }
    }
}

// Path layout under the `/approvals` and `/gate-records` mount aliases:
//
//     /approvals[/agents/<agent>][/projects/<project>][/missions/<mission>][/threads/<thread>]/<request_id>.json
//     /gate-records[/agents/<agent>][/projects/<project>][/missions/<mission>][/threads/<thread>]/<gate_ref>.json
//
// Tenant + user identity moves into the caller's `MountView` per the
// per-tenant `MountAlias` rewriting, so neither prefix is encoded in the
// path itself. Within-tenant sub-scope axes (agent/project/mission/thread)
// stay in the alias-relative path because they are within-tenant scoping
// not covered by the per-tenant `MountAlias`.

const APPROVALS_PREFIX: &str = "/approvals";
const GATE_RECORDS_PREFIX: &str = "/gate-records";

fn approval_record_path(
    scope: &ResourceScope,
    request_id: ApprovalRequestId,
) -> Result<ScopedPath, ApprovalStoreError> {
    scoped_path(&format!(
        "{}/{request_id}.json",
        approval_records_root_string(scope)
    ))
}

fn approval_records_root(scope: &ResourceScope) -> Result<ScopedPath, ApprovalStoreError> {
    scoped_path(&approval_records_root_string(scope))
}

fn approval_records_root_string(scope: &ResourceScope) -> String {
    scope_owner_alias_string(APPROVALS_PREFIX, scope)
}

fn gate_record_path(
    scope: &ResourceScope,
    gate_ref: GateRef,
) -> Result<ScopedPath, ApprovalStoreError> {
    scoped_path(&format!(
        "{}/{gate_ref}.json",
        scope_owner_alias_string(GATE_RECORDS_PREFIX, scope)
    ))
}

/// Build the alias-relative owner prefix for a scope under the given mount
/// alias. Tenant and user are intentionally absent — they live in the
/// `MountView` the caller supplied. Sub-scope axes (agent/project/mission/
/// thread) stay in the path so within-tenant cross-scope isolation still
/// works for stores sharing one alias target.
fn scope_owner_alias_string(prefix: &'static str, scope: &ResourceScope) -> String {
    let mut base = String::from(prefix);
    if let Some(agent_id) = &scope.agent_id {
        base.push_str("/agents/");
        base.push_str(agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    if let Some(mission_id) = &scope.mission_id {
        base.push_str("/missions/");
        base.push_str(mission_id.as_str());
    }
    if let Some(thread_id) = &scope.thread_id {
        base.push_str("/threads/");
        base.push_str(thread_id.as_str());
    }
    base
}

fn scoped_path(raw: &str) -> Result<ScopedPath, ApprovalStoreError> {
    ScopedPath::new(raw).map_err(invalid_path)
}

/// Join a leaf segment onto a [`ScopedPath`] prefix. Mirrors the engine /
/// processes / secrets / outbound stores' `join_scoped` helper: `list_dir`
/// returns post-resolution [`VirtualPath`](ironclaw_host_api::path::VirtualPath)s,
/// but the follow-up `get` must run through the `ScopedFilesystem` so the
/// per-op ACL is enforced — so callers strip the leaf name and rejoin it
/// onto the original `ScopedPath` prefix.
fn join_scoped(prefix: &ScopedPath, leaf: &str) -> Result<ScopedPath, ApprovalStoreError> {
    scoped_path(&format!(
        "{}/{}",
        prefix.as_str().trim_end_matches('/'),
        leaf
    ))
}

fn invalid_path(error: HostApiError) -> ApprovalStoreError {
    ApprovalStoreError::InvalidPath(error.to_string())
}

fn same_scope_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.mission_id == right.mission_id
        && left.thread_id == right.thread_id
}

fn serialize_pretty<T>(value: &T) -> Result<Vec<u8>, ApprovalStoreError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value)
        .map_err(|error| ApprovalStoreError::Serialization(error.to_string()))
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, ApprovalStoreError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes)
        .map_err(|error| ApprovalStoreError::Deserialization(error.to_string()))
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

/// Map the shared CAS helper's [`CasUpdateError`] into a [`ApprovalStoreError`].
///
/// [`CasUpdateError::Apply`] carries the caller's own error straight through;
/// all other variants are storage-layer failures. Fail-closed: a backend that
/// cannot honor versioned CAS surfaces as a [`ApprovalStoreError::Backend`] rather
/// than a silent blind overwrite.
fn map_cas_error(error: CasUpdateError<ApprovalStoreError>) -> ApprovalStoreError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout | CasUpdateError::RetriesExhausted => {
            ApprovalStoreError::Backend("filesystem CAS retries exhausted".to_string())
        }
        CasUpdateError::CasUnsupported => ApprovalStoreError::Backend(
            "backend does not support versioned compare-and-swap".to_string(),
        ),
        CasUpdateError::Backend(fs_err) => ApprovalStoreError::Filesystem(fs_err.to_string()),
    }
}
