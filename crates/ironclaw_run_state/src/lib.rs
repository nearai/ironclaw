//! Run-state contracts for IronClaw Reborn.
//!
//! `ironclaw_run_state` stores the current lifecycle state for host-managed
//! invocations. It is separate from runtime events: events are append-only
//! history, while run state answers "what is this invocation waiting on now?".

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    AgentId, ApprovalRequest, ApprovalRequestId, CapabilityId, HostApiError, InvocationId,
    MissionId, ProjectId, ResourceScope, TenantId, ThreadId, UserId, VirtualPath,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
}

/// Approval request lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

/// Durable approval request record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub scope: ResourceScope,
    pub request: ApprovalRequest,
    pub status: ApprovalStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RunStateKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    mission_id: Option<MissionId>,
    thread_id: Option<ThreadId>,
    invocation_id: InvocationId,
}

impl RunStateKey {
    fn new(scope: &ResourceScope, invocation_id: InvocationId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: scope.mission_id.clone(),
            thread_id: scope.thread_id.clone(),
            invocation_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ApprovalKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    mission_id: Option<MissionId>,
    thread_id: Option<ThreadId>,
    request_id: ApprovalRequestId,
}

impl ApprovalKey {
    fn new(scope: &ResourceScope, request_id: ApprovalRequestId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: scope.mission_id.clone(),
            thread_id: scope.thread_id.clone(),
            request_id,
        }
    }
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

/// In-memory run-state store for tests and early host wiring.
#[derive(Debug, Default)]
pub struct InMemoryRunStateStore {
    records: Mutex<HashMap<RunStateKey, RunRecord>>,
}

impl InMemoryRunStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn update(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        update: impl FnOnce(&mut RunRecord),
    ) -> Result<RunRecord, RunStateError> {
        let key = RunStateKey::new(scope, invocation_id);
        let mut records = self.records_guard();
        let record = records
            .get_mut(&key)
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        update(record);
        Ok(record.clone())
    }

    fn records_guard(&self) -> MutexGuard<'_, HashMap<RunStateKey, RunRecord>> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl RunStateStore for InMemoryRunStateStore {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        let record = RunRecord {
            invocation_id: start.invocation_id,
            capability_id: start.capability_id,
            scope: start.scope,
            status: RunStatus::Running,
            approval_request_id: None,
            error_kind: None,
        };
        let key = RunStateKey::new(&record.scope, record.invocation_id);
        let mut records = self.records_guard();
        if records.contains_key(&key) {
            return Err(RunStateError::InvocationAlreadyExists {
                invocation_id: record.invocation_id,
            });
        }
        records.insert(key, record.clone());
        Ok(record)
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.update(scope, invocation_id, |record| {
            record.status = RunStatus::BlockedApproval;
            record.approval_request_id = Some(approval.id);
            record.error_kind = None;
        })
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.update(scope, invocation_id, |record| {
            record.status = RunStatus::BlockedAuth;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind);
        })
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.update(scope, invocation_id, |record| {
            record.status = RunStatus::Completed;
            record.approval_request_id = None;
            record.error_kind = None;
        })
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.update(scope, invocation_id, |record| {
            record.status = RunStatus::Failed;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind);
        })
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        Ok(self
            .records_guard()
            .get(&RunStateKey::new(scope, invocation_id))
            .cloned())
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        let mut records = self
            .records_guard()
            .values()
            .filter(|record| same_scope_owner(&record.scope, scope))
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| record.invocation_id.as_uuid());
        Ok(records)
    }
}

/// In-memory approval request store for tests and early host wiring.
#[derive(Debug, Default)]
pub struct InMemoryApprovalRequestStore {
    records: Mutex<HashMap<ApprovalKey, ApprovalRecord>>,
}

impl InMemoryApprovalRequestStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn update_status(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        status: ApprovalStatus,
    ) -> Result<ApprovalRecord, RunStateError> {
        let mut records = self.records_guard();
        let record = records
            .get_mut(&ApprovalKey::new(scope, request_id))
            .ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
        if record.status != ApprovalStatus::Pending {
            return Err(RunStateError::ApprovalNotPending {
                request_id,
                status: record.status,
            });
        }
        record.status = status;
        Ok(record.clone())
    }

    fn records_guard(&self) -> MutexGuard<'_, HashMap<ApprovalKey, ApprovalRecord>> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl ApprovalRequestStore for InMemoryApprovalRequestStore {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        let record = ApprovalRecord {
            scope,
            request,
            status: ApprovalStatus::Pending,
        };
        let key = ApprovalKey::new(&record.scope, record.request.id);
        let mut records = self.records_guard();
        if records.contains_key(&key) {
            return Err(RunStateError::ApprovalRequestAlreadyExists {
                request_id: record.request.id,
            });
        }
        records.insert(key, record.clone());
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        Ok(self
            .records_guard()
            .get(&ApprovalKey::new(scope, request_id))
            .cloned())
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.update_status(scope, request_id, ApprovalStatus::Approved)
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        self.update_status(scope, request_id, ApprovalStatus::Denied)
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError> {
        let mut records = self.records_guard();
        let key = ApprovalKey::new(scope, request_id);
        let record = records
            .get(&key)
            .ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
        if record.status != ApprovalStatus::Pending {
            return Err(RunStateError::ApprovalNotPending {
                request_id,
                status: record.status,
            });
        }
        records
            .remove(&key)
            .ok_or(RunStateError::UnknownApprovalRequest { request_id })
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        let mut records = self
            .records_guard()
            .values()
            .filter(|record| same_scope_owner(&record.scope, scope))
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| record.request.id.as_uuid());
        Ok(records)
    }
}

/// Filesystem-backed run-state store under resource-owner-scoped `/engine` paths.
pub struct FilesystemRunStateStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: &'a F,
}

impl<'a, F> FilesystemRunStateStore<'a, F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: &'a F) -> Self {
        Self { filesystem }
    }

    async fn write_record(&self, record: &RunRecord) -> Result<(), RunStateError> {
        let path = run_record_path(&record.scope, record.invocation_id)?;
        let bytes = serialize_pretty(record)?;
        self.filesystem.write_file(&path, &bytes).await?;
        Ok(())
    }
}

#[async_trait]
impl<F> RunStateStore for FilesystemRunStateStore<'_, F>
where
    F: RootFilesystem,
{
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError> {
        let path = run_record_path(&start.scope, start.invocation_id)?;
        let record_lock = filesystem_record_lock(&path);
        let _guard = record_lock.lock().await;
        if self.get(&start.scope, start.invocation_id).await?.is_some() {
            return Err(RunStateError::InvocationAlreadyExists {
                invocation_id: start.invocation_id,
            });
        }
        let record = RunRecord {
            invocation_id: start.invocation_id,
            capability_id: start.capability_id,
            scope: start.scope,
            status: RunStatus::Running,
            approval_request_id: None,
            error_kind: None,
        };
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        let path = run_record_path(scope, invocation_id)?;
        let record_lock = filesystem_record_lock(&path);
        let _guard = record_lock.lock().await;
        let mut record = self
            .get(scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        record.status = RunStatus::BlockedApproval;
        record.approval_request_id = Some(approval.id);
        record.error_kind = None;
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        let path = run_record_path(scope, invocation_id)?;
        let record_lock = filesystem_record_lock(&path);
        let _guard = record_lock.lock().await;
        let mut record = self
            .get(scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        record.status = RunStatus::BlockedAuth;
        record.approval_request_id = None;
        record.error_kind = Some(error_kind);
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        let path = run_record_path(scope, invocation_id)?;
        let record_lock = filesystem_record_lock(&path);
        let _guard = record_lock.lock().await;
        let mut record = self
            .get(scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        record.status = RunStatus::Completed;
        record.approval_request_id = None;
        record.error_kind = None;
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        let path = run_record_path(scope, invocation_id)?;
        let record_lock = filesystem_record_lock(&path);
        let _guard = record_lock.lock().await;
        let mut record = self
            .get(scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        record.status = RunStatus::Failed;
        record.approval_request_id = None;
        record.error_kind = Some(error_kind);
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        let path = run_record_path(scope, invocation_id)?;
        let bytes = match self.filesystem.read_file(&path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record = deserialize::<RunRecord>(&bytes)?;
        if same_scope_owner(&record.scope, scope) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        let root = run_records_root(scope)?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            if entry.name.ends_with(".json") {
                let bytes = match self.filesystem.read_file(&entry.path).await {
                    Ok(bytes) => bytes,
                    Err(error) if is_not_found(&error) => continue,
                    Err(error) => return Err(error.into()),
                };
                let record = deserialize::<RunRecord>(&bytes)?;
                if same_scope_owner(&record.scope, scope) {
                    records.push(record);
                }
            }
        }
        records.sort_by_key(|record| record.invocation_id.as_uuid());
        Ok(records)
    }
}

/// Filesystem-backed approval request store under resource-owner-scoped `/engine` paths.
pub struct FilesystemApprovalRequestStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: &'a F,
}

impl<'a, F> FilesystemApprovalRequestStore<'a, F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: &'a F) -> Self {
        Self { filesystem }
    }

    async fn update_status(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        status: ApprovalStatus,
    ) -> Result<ApprovalRecord, RunStateError> {
        let path = approval_record_path(scope, request_id)?;
        let record_lock = filesystem_record_lock(&path);
        let _guard = record_lock.lock().await;
        let mut record = self
            .get(scope, request_id)
            .await?
            .ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
        if record.status != ApprovalStatus::Pending {
            return Err(RunStateError::ApprovalNotPending {
                request_id,
                status: record.status,
            });
        }
        record.status = status;
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn write_record(&self, record: &ApprovalRecord) -> Result<(), RunStateError> {
        let path = approval_record_path(&record.scope, record.request.id)?;
        let bytes = serialize_pretty(record)?;
        self.filesystem.write_file(&path, &bytes).await?;
        Ok(())
    }
}

#[async_trait]
impl<F> ApprovalRequestStore for FilesystemApprovalRequestStore<'_, F>
where
    F: RootFilesystem,
{
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError> {
        let path = approval_record_path(&scope, request.id)?;
        let record_lock = filesystem_record_lock(&path);
        let _guard = record_lock.lock().await;
        if self.get(&scope, request.id).await?.is_some() {
            return Err(RunStateError::ApprovalRequestAlreadyExists {
                request_id: request.id,
            });
        }
        let record = ApprovalRecord {
            scope,
            request,
            status: ApprovalStatus::Pending,
        };
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError> {
        let path = approval_record_path(scope, request_id)?;
        let bytes = match self.filesystem.read_file(&path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record = deserialize::<ApprovalRecord>(&bytes)?;
        if same_scope_owner(&record.scope, scope) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
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
        let record_lock = filesystem_record_lock(&path);
        let _guard = record_lock.lock().await;
        let record = self
            .get(scope, request_id)
            .await?
            .ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
        if record.status != ApprovalStatus::Pending {
            return Err(RunStateError::ApprovalNotPending {
                request_id,
                status: record.status,
            });
        }
        let path = approval_record_path(scope, request_id)?;
        self.filesystem.delete(&path).await?;
        Ok(record)
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        let root = approval_records_root(scope)?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            if entry.name.ends_with(".json") {
                let bytes = match self.filesystem.read_file(&entry.path).await {
                    Ok(bytes) => bytes,
                    Err(error) if is_not_found(&error) => continue,
                    Err(error) => return Err(error.into()),
                };
                let record = deserialize::<ApprovalRecord>(&bytes)?;
                if same_scope_owner(&record.scope, scope) {
                    records.push(record);
                }
            }
        }
        records.sort_by_key(|record| record.request.id.as_uuid());
        Ok(records)
    }
}

fn run_record_path(
    scope: &ResourceScope,
    invocation_id: InvocationId,
) -> Result<VirtualPath, RunStateError> {
    VirtualPath::new(format!(
        "{}/{invocation_id}.json",
        run_records_root(scope)?.as_str()
    ))
    .map_err(invalid_path)
}

fn run_records_root(scope: &ResourceScope) -> Result<VirtualPath, RunStateError> {
    VirtualPath::new(format!("{}/runs", tenant_user_root(scope))).map_err(invalid_path)
}

fn approval_record_path(
    scope: &ResourceScope,
    request_id: ApprovalRequestId,
) -> Result<VirtualPath, RunStateError> {
    VirtualPath::new(format!(
        "{}/{request_id}.json",
        approval_records_root(scope)?.as_str()
    ))
    .map_err(invalid_path)
}

fn approval_records_root(scope: &ResourceScope) -> Result<VirtualPath, RunStateError> {
    VirtualPath::new(format!("{}/approvals", tenant_user_root(scope))).map_err(invalid_path)
}

type FilesystemRecordLock = Arc<tokio::sync::Mutex<()>>;

static FILESYSTEM_RECORD_LOCKS: OnceLock<Mutex<HashMap<String, FilesystemRecordLock>>> =
    OnceLock::new();

fn filesystem_record_lock(path: &VirtualPath) -> FilesystemRecordLock {
    let locks = FILESYSTEM_RECORD_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        guard
            .entry(path.as_str().to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

fn tenant_user_root(scope: &ResourceScope) -> String {
    let mut base = format!(
        "/engine/tenants/{}/users/{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    );
    if let Some(agent_id) = &scope.agent_id {
        base = format!("{base}/agents/{}", agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base = format!("{base}/projects/{}", project_id.as_str());
    }
    if let Some(mission_id) = &scope.mission_id {
        base = format!("{base}/missions/{}", mission_id.as_str());
    }
    if let Some(thread_id) = &scope.thread_id {
        base = format!("{base}/threads/{}", thread_id.as_str());
    }
    base
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
