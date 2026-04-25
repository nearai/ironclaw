//! Run-state contracts for IronClaw Reborn.
//!
//! `ironclaw_run_state` stores the current lifecycle state for host-managed
//! invocations. It is separate from runtime events: events are append-only
//! history, while run state answers "what is this invocation waiting on now?".

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    ApprovalRequest, ApprovalRequestId, CapabilityId, ErrorKind, HostApiError, InvocationId,
    ResourceScope, TenantId, UserId, VirtualPath,
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
    pub error_kind: Option<ErrorKind>,
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
    invocation_id: InvocationId,
}

impl RunStateKey {
    fn new(scope: &ResourceScope, invocation_id: InvocationId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            invocation_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ApprovalKey {
    tenant_id: TenantId,
    user_id: UserId,
    request_id: ApprovalRequestId,
}

impl ApprovalKey {
    fn new(scope: &ResourceScope, request_id: ApprovalRequestId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
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
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
    #[error("filesystem error: {0}")]
    Filesystem(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("deserialization error: {0}")]
    Deserialization(String),
}

impl From<HostApiError> for RunStateError {
    fn from(error: HostApiError) -> Self {
        Self::InvalidPath(error.to_string())
    }
}

impl From<FilesystemError> for RunStateError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error.to_string())
    }
}

/// Current-state store for invocation lifecycle.
#[async_trait]
pub trait RunStateStore: Send + Sync {
    async fn start(&self, start: RunStart) -> Result<RunRecord, RunStateError>;
    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError>;
    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError>;
    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError>;
    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError>;
    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError>;
    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError>;
}

/// Store for approval requests emitted by authorization decisions.
#[async_trait]
pub trait ApprovalRequestStore: Send + Sync {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ApprovalRequest,
    ) -> Result<ApprovalRecord, RunStateError>;
    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ApprovalRecord>, RunStateError>;
    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError>;
    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ApprovalRecord, RunStateError>;
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
            record.error_kind = Some(ErrorKind::new(error_kind));
        })
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        self.update(scope, invocation_id, |record| {
            record.status = RunStatus::Completed;
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
            record.error_kind = Some(ErrorKind::new(error_kind));
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
            .filter(|record| same_tenant_user(&record.scope, scope))
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
        self.records_guard().insert(
            ApprovalKey::new(&record.scope, record.request.id),
            record.clone(),
        );
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

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ApprovalRecord>, RunStateError> {
        let mut records = self
            .records_guard()
            .values()
            .filter(|record| same_tenant_user(&record.scope, scope))
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| record.request.id.as_uuid());
        Ok(records)
    }
}

/// Filesystem-backed run-state store under tenant/user-scoped `/engine/tenants/.../runs`.
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
        let mut record = self
            .get(scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        record.status = RunStatus::BlockedAuth;
        record.error_kind = Some(ErrorKind::new(error_kind));
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        let mut record = self
            .get(scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        record.status = RunStatus::Completed;
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
        let mut record = self
            .get(scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        record.status = RunStatus::Failed;
        record.error_kind = Some(ErrorKind::new(error_kind));
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
        if same_tenant_user(&record.scope, scope) {
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
                let bytes = self.filesystem.read_file(&entry.path).await?;
                let record = deserialize::<RunRecord>(&bytes)?;
                if same_tenant_user(&record.scope, scope) {
                    records.push(record);
                }
            }
        }
        records.sort_by_key(|record| record.invocation_id.as_uuid());
        Ok(records)
    }
}

/// Filesystem-backed approval request store under tenant/user-scoped `/engine/tenants/.../approvals`.
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
        let mut record = self
            .get(scope, request_id)
            .await?
            .ok_or(RunStateError::UnknownApprovalRequest { request_id })?;
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
        if same_tenant_user(&record.scope, scope) {
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
                let bytes = self.filesystem.read_file(&entry.path).await?;
                let record = deserialize::<ApprovalRecord>(&bytes)?;
                if same_tenant_user(&record.scope, scope) {
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
    .map_err(Into::into)
}

fn run_records_root(scope: &ResourceScope) -> Result<VirtualPath, RunStateError> {
    VirtualPath::new(format!("{}/runs", tenant_user_root(scope))).map_err(Into::into)
}

fn approval_record_path(
    scope: &ResourceScope,
    request_id: ApprovalRequestId,
) -> Result<VirtualPath, RunStateError> {
    VirtualPath::new(format!(
        "{}/{request_id}.json",
        approval_records_root(scope)?.as_str()
    ))
    .map_err(Into::into)
}

fn approval_records_root(scope: &ResourceScope) -> Result<VirtualPath, RunStateError> {
    VirtualPath::new(format!("{}/approvals", tenant_user_root(scope))).map_err(Into::into)
}

fn tenant_user_root(scope: &ResourceScope) -> String {
    format!(
        "/engine/tenants/{}/users/{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    )
}

fn same_tenant_user(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id && left.user_id == right.user_id
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
    match error {
        FilesystemError::Backend { reason, .. } => {
            reason.contains("No such file")
                || reason.contains("not found")
                || reason.contains("os error 2")
        }
        _ => false,
    }
}
