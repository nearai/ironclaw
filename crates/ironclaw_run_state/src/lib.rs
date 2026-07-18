//! Run-state contracts for IronClaw Reborn.
//!
//! `ironclaw_run_state` stores the current lifecycle state for host-managed
//! invocations. It is separate from runtime events: events are append-only
//! history, while run state answers "what is this invocation waiting on now?".
//!
//! Durable persistence is provided by [`FilesystemRunStateStore`] and
//! [`FilesystemApprovalRequestStore`] over a
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
    ApprovalRequest, ApprovalRequestId, CapabilityId, HostApiError, InvocationId, ResourceScope,
    ScopedPath, UserId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(any(test, feature = "test-support"))]
mod test_support;
#[cfg(any(test, feature = "test-support"))]
pub use test_support::{
    in_memory_backed_approval_request_store, in_memory_backed_run_state_filesystem,
    in_memory_backed_run_state_store,
};

/// Current lifecycle state for one invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    BlockedApproval,
    BlockedAuth,
    Completed,
    Failed,
}

/// State record keyed by invocation ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticated_actor_user_id: Option<UserId>,
    pub status: RunStatus,
    pub approval_request_id: Option<ApprovalRequestId>,
    pub error_kind: Option<String>,
}

/// Start metadata for a capability invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStart {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub authenticated_actor_user_id: Option<UserId>,
}

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

/// Run-state and approval persistence errors.
#[derive(Debug, Error)]
pub enum RunStateError {
    #[error("unknown invocation {invocation_id}")]
    UnknownInvocation { invocation_id: InvocationId },
    #[error("invocation {invocation_id} already exists")]
    InvocationAlreadyExists { invocation_id: InvocationId },
    #[error("unknown approval request {request_id}")]
    UnknownApprovalRequest { request_id: ApprovalRequestId },
    #[error("approval request {request_id} already exists")]
    ApprovalRequestAlreadyExists { request_id: ApprovalRequestId },
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
    #[error("run-state backend error: {0}")]
    Backend(String),
}

impl From<FilesystemError> for RunStateError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error.to_string())
    }
}

/// Current-state store for invocation lifecycle.
#[async_trait]
pub trait RunStateStore: Send + Sync {
    /// Creates a running invocation record in the exact resource-owner scope.
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError>;

    /// Marks an invocation blocked on an approval request without granting authority by itself.
    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError>;

    /// Marks an invocation blocked on external auth/secret resolution without exposing secret material.
    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError>;

    /// Marks an invocation completed only within the matching scope.
    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError>;

    /// Marks an invocation failed with a classified error kind, not raw runtime details.
    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError>;

    /// Loads one scoped invocation record; wrong-scope lookups must look unknown.
    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError>;

    /// Lists invocation records visible to the exact resource-owner scope only.
    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError>;
}

/// Store for approval requests emitted by authorization decisions.
#[async_trait]
pub trait ApprovalRequestStore: Send + Sync {
    /// Persists a pending approval request in the exact resource-owner scope without resolving it.
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError>;

    /// Loads one scoped approval record; wrong-scope lookups must look unknown.
    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError>;

    /// Marks a pending approval request approved only within the matching scope.
    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError>;

    /// Marks a pending approval request denied only within the matching scope.
    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError>;

    /// Discards a still-pending approval request during rollback before it becomes user-actionable.
    ///
    /// Stores that can delete pending records should override this method. The default is a
    /// fail-closed tombstone fallback that marks the record denied rather than leaving a
    /// user-actionable pending approval behind.
    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.deny(scope, request_id).await
    }

    /// Lists approval records visible to the exact resource-owner scope only.
    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError>;
}

/// Combined run-state + approval request store with an atomic approval-block transition.
///
/// Production composition should prefer this interface when the same durable backend
/// owns both invocation state and approval records. It prevents a crash between
/// `ApprovalRequestStore::save_pending` and `RunStateStore::block_approval` from
/// leaving a user-actionable approval disconnected from a blocked run.
#[async_trait]
pub trait RunStateApprovalStore: RunStateStore + ApprovalRequestStore {
    async fn save_pending_and_block_approval(
        &self,
        scope: ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError>;
}

/// `RecordKind` tag written on every run-state entry so byte-only backends
/// (e.g. `DiskFilesystem`) are rejected with `Unsupported{WriteFile}` on
/// first put, which `cas_update` maps to `CasUnsupported` (fail-closed).
const RUN_STATE_RECORD_KIND: &str = "run_state_record";

/// `RecordKind` tag written on every approval-request entry for the same
/// fail-closed CAS gate as [`RUN_STATE_RECORD_KIND`].
const APPROVAL_RECORD_KIND: &str = "approval_record";

/// Filesystem-backed run-state store under the `/run-state` mount alias.
///
/// Construct with a [`ScopedFilesystem`] over any [`RootFilesystem`]. The
/// [`ScopedFilesystem`] resolves the `/run-state` alias to a
/// tenant/user-scoped [`VirtualPath`](ironclaw_host_api::VirtualPath) per
/// its [`MountView`](ironclaw_host_api::MountView) and enforces per-op ACL
/// before any backend dispatch — so tenant isolation is structural rather
/// than something this crate has to re-derive from `ResourceScope.tenant_id`
/// / `user_id`. Within-tenant axes (agent/project/mission/thread) remain in
/// the alias-relative path because they are not covered by the per-tenant
/// `MountAlias`.
pub struct FilesystemRunStateStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemRunStateStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn record_entry(record: &RunRecord) -> Result<Entry, RunStateError> {
        let body = serialize_pretty(record)?;
        let kind = RecordKind::new(RUN_STATE_RECORD_KIND)
            .map_err(|e| RunStateError::Backend(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry)
    }

    async fn read_versioned(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<(RunRecord, ironclaw_filesystem::RecordVersion)>, RunStateError> {
        let path = run_record_path(scope, invocation_id)?;
        let Some(versioned) = self.filesystem.get(scope, &path).await? else {
            return Ok(None);
        };
        let record = deserialize::<RunRecord>(&versioned.entry.body)?;
        if same_scope_owner(&record.scope, scope) {
            Ok(Some((record, versioned.version)))
        } else {
            Ok(None)
        }
    }

    /// Read-modify-write a run record using the shared lock-free CAS helper.
    ///
    /// `mutate` projects the staged record onto its new shape. The helper's
    /// optimistic CAS-retry loop handles cross-process contention without
    /// holding any lock across `.await`.
    async fn apply_update<M>(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        mut mutate: M,
    ) -> Result<RunRecord, RunStateError>
    where
        M: FnMut(&mut RunRecord),
    {
        let path = run_record_path(scope, invocation_id)?;
        let scope_clone = scope.clone();
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            |bytes: &[u8]| deserialize::<RunRecord>(bytes),
            |record: &RunRecord| Self::record_entry(record),
            |current: Option<RunRecord>| {
                // Compute the outcome synchronously so the async block only
                // captures an already-resolved `Result` (mirrors cas_snapshot.rs).
                let outcome = (|| {
                    let mut record =
                        current.ok_or(RunStateError::UnknownInvocation { invocation_id })?;
                    // Enforce scope ownership on each retry against a freshly read record.
                    if !same_scope_owner(&record.scope, &scope_clone) {
                        return Err(RunStateError::UnknownInvocation { invocation_id });
                    }
                    mutate(&mut record);
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
impl<F> RunStateStore for FilesystemRunStateStore<F>
where
    F: RootFilesystem,
{
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        let path = run_record_path(&start.scope, start.invocation_id)?;
        let invocation_id = start.invocation_id;
        let record = RunRecord {
            invocation_id: start.invocation_id,
            capability_id: start.capability_id,
            scope: start.scope,
            authenticated_actor_user_id: start.authenticated_actor_user_id,
            status: RunStatus::Running,
            approval_request_id: None,
            error_kind: None,
        };
        let scope = record.scope.clone();
        cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            |bytes: &[u8]| deserialize::<RunRecord>(bytes),
            |r: &RunRecord| Self::record_entry(r),
            |current: Option<RunRecord>| {
                let fresh = record.clone();
                let outcome = if current.is_some() {
                    Err(RunStateError::InvocationAlreadyExists { invocation_id })
                } else {
                    Ok(CasApply::new(fresh.clone(), fresh))
                };
                async move { outcome }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.apply_update(scope, invocation_id, |record| {
            record.status = RunStatus::BlockedApproval;
            record.approval_request_id = Some(approval.id);
            record.error_kind = None;
        })
        .await
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.apply_update(scope, invocation_id, |record| {
            record.status = RunStatus::BlockedAuth;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind.clone());
        })
        .await
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.apply_update(scope, invocation_id, |record| {
            record.status = RunStatus::Completed;
            record.approval_request_id = None;
            record.error_kind = None;
        })
        .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.apply_update(scope, invocation_id, |record| {
            record.status = RunStatus::Failed;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind.clone());
        })
        .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        Ok(self
            .read_versioned(scope, invocation_id)
            .await?
            .map(|(record, _)| record))
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        let root = run_records_root(scope)?;
        let entries = match self.filesystem.list_dir(scope, &root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            if entry.name.ends_with(".json") {
                // `list_dir` returns post-resolution `VirtualPath`s; reconstruct
                // the alias-relative `ScopedPath` so the follow-up `get` still
                // runs through the per-op ACL.
                let child = join_scoped(&root, &entry.name)?;
                let Some(versioned) = self.filesystem.get(scope, &child).await? else {
                    continue;
                };
                let record = deserialize::<RunRecord>(&versioned.entry.body)?;
                if same_scope_owner(&record.scope, scope) {
                    records.push(record);
                }
            }
        }
        records.sort_by_key(|record| record.invocation_id.as_uuid());
        Ok(records)
    }
}

/// Filesystem-backed approval request store under the `/approvals` mount alias.
///
/// See [`FilesystemRunStateStore`] for the structural-tenant-isolation
/// rationale; this store applies the same shape to approval-request records
/// under a sibling mount alias so a single composition can wire run state
/// and approvals to distinct alias targets while sharing one backend.
pub struct FilesystemApprovalRequestStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemApprovalRequestStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn record_entry(record: &ApprovalRecord) -> Result<Entry, RunStateError> {
        let body = serialize_pretty(record)?;
        let kind = RecordKind::new(APPROVAL_RECORD_KIND)
            .map_err(|e| RunStateError::Backend(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry)
    }

    async fn read_versioned(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<(ApprovalRecord, ironclaw_filesystem::RecordVersion)>, RunStateError> {
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
    /// Mirrors `FilesystemRunStateStore::apply_update` — no per-record mutex,
    /// no lock held across `.await`.
    async fn update_status(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        status: ApprovalStatus,
    ) -> Result<ApprovalRecord, RunStateError> {
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
                        current.ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
                    // Enforce scope ownership on each retry against a freshly read record.
                    if !same_scope_owner(&record.scope, &scope_clone) {
                        return Err(RunStateError::UnknownApprovalRequest { request_id });
                    }
                    if record.status != ApprovalStatus::Pending {
                        return Err(RunStateError::ApprovalNotPending {
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
impl<F> ApprovalRequestStore for FilesystemApprovalRequestStore<F>
where
    F: RootFilesystem,
{
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
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
                    Err(RunStateError::ApprovalRequestAlreadyExists { request_id })
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
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
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
    ) -> Result<ApprovalRecord, RunStateError> {
        self.update_status(scope, request_id, ApprovalStatus::Approved)
            .await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.update_status(scope, request_id, ApprovalStatus::Denied)
            .await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
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
                        current.ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
                    // Enforce scope ownership on each retry against a freshly read record.
                    if !same_scope_owner(&record.scope, &scope_clone) {
                        return Err(RunStateError::UnknownApprovalRequest { request_id });
                    }
                    if record.status != ApprovalStatus::Pending {
                        return Err(RunStateError::ApprovalNotPending {
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
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        let root = approval_records_root(scope)?;
        let entries = match self.filesystem.list_dir(scope, &root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            if entry.name.ends_with(".json") {
                // See `FilesystemRunStateStore::records_for_scope` — `list_dir`
                // returns post-resolution `VirtualPath`s; rebuild the
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

// Path layout under the `/run-state` and `/approvals` mount aliases:
//
//     /run-state[/agents/<agent>][/projects/<project>][/missions/<mission>][/threads/<thread>]/runs/<invocation_id>.json
//     /approvals[/agents/<agent>][/projects/<project>][/missions/<mission>][/threads/<thread>]/<request_id>.json
//
// Tenant + user identity moves into the caller's `MountView` per the
// per-tenant `MountAlias` rewriting, so neither prefix is encoded in the
// path itself. Within-tenant sub-scope axes (agent/project/mission/thread)
// stay in the alias-relative path because they are within-tenant scoping
// not covered by the per-tenant `MountAlias`.

const RUN_STATE_PREFIX: &str = "/run-state";
const APPROVALS_PREFIX: &str = "/approvals";

fn run_record_path(
    scope: &ResourceScope,
    invocation_id: InvocationId,
) -> Result<ScopedPath, RunStateError> {
    scoped_path(&format!(
        "{}/{invocation_id}.json",
        run_records_root_string(scope)
    ))
}

fn run_records_root(scope: &ResourceScope) -> Result<ScopedPath, RunStateError> {
    scoped_path(&run_records_root_string(scope))
}

fn run_records_root_string(scope: &ResourceScope) -> String {
    format!("{}/runs", scope_owner_alias_string(RUN_STATE_PREFIX, scope))
}

fn approval_record_path(
    scope: &ResourceScope,
    request_id: ApprovalRequestId,
) -> Result<ScopedPath, RunStateError> {
    scoped_path(&format!(
        "{}/{request_id}.json",
        approval_records_root_string(scope)
    ))
}

fn approval_records_root(scope: &ResourceScope) -> Result<ScopedPath, RunStateError> {
    scoped_path(&approval_records_root_string(scope))
}

fn approval_records_root_string(scope: &ResourceScope) -> String {
    scope_owner_alias_string(APPROVALS_PREFIX, scope)
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

fn scoped_path(raw: &str) -> Result<ScopedPath, RunStateError> {
    ScopedPath::new(raw).map_err(invalid_path)
}

/// Join a leaf segment onto a [`ScopedPath`] prefix. Mirrors the engine /
/// processes / secrets / outbound stores' `join_scoped` helper: `list_dir`
/// returns post-resolution [`VirtualPath`](ironclaw_host_api::VirtualPath)s,
/// but the follow-up `get` must run through the `ScopedFilesystem` so the
/// per-op ACL is enforced — so callers strip the leaf name and rejoin it
/// onto the original `ScopedPath` prefix.
fn join_scoped(prefix: &ScopedPath, leaf: &str) -> Result<ScopedPath, RunStateError> {
    scoped_path(&format!(
        "{}/{}",
        prefix.as_str().trim_end_matches('/'),
        leaf
    ))
}

fn invalid_path(error: HostApiError) -> RunStateError {
    RunStateError::InvalidPath(error.to_string())
}

fn same_scope_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.mission_id == right.mission_id
        && left.thread_id == right.thread_id
}

fn serialize_pretty<T>(value: &T) -> Result<Vec<u8>, RunStateError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value)
        .map_err(|error| RunStateError::Serialization(error.to_string()))
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, RunStateError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| RunStateError::Deserialization(error.to_string()))
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

/// Map the shared CAS helper's [`CasUpdateError`] into a [`RunStateError`].
///
/// [`CasUpdateError::Apply`] carries the caller's own error straight through;
/// all other variants are storage-layer failures. Fail-closed: a backend that
/// cannot honor versioned CAS surfaces as a [`RunStateError::Backend`] rather
/// than a silent blind overwrite.
fn map_cas_error(error: CasUpdateError<RunStateError>) -> RunStateError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout | CasUpdateError::RetriesExhausted => {
            RunStateError::Backend("filesystem CAS retries exhausted".to_string())
        }
        CasUpdateError::CasUnsupported => RunStateError::Backend(
            "backend does not support versioned compare-and-swap".to_string(),
        ),
        CasUpdateError::Backend(fs_err) => RunStateError::Filesystem(fs_err.to_string()),
    }
}
