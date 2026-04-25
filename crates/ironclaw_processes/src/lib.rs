//! Process lifecycle contracts for IronClaw Reborn.
//!
//! `ironclaw_processes` stores and manages host-tracked background capability
//! processes. It owns lifecycle mechanics, not capability authorization or
//! runtime dispatch policy.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityId, CapabilitySet, ExtensionId, HostApiError, InvocationId, MountView, ProcessId,
    ResourceEstimate, ResourceReservationId, ResourceScope, RuntimeKind, TenantId, UserId,
    VirtualPath,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessRecord {
    pub process_id: ProcessId,
    pub parent_process_id: Option<ProcessId>,
    pub invocation_id: InvocationId,
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub status: ProcessStatus,
    pub grants: CapabilitySet,
    pub mounts: MountView,
    pub estimated_resources: ResourceEstimate,
    pub resource_reservation_id: Option<ResourceReservationId>,
    pub error_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessStart {
    pub process_id: ProcessId,
    pub parent_process_id: Option<ProcessId>,
    pub invocation_id: InvocationId,
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub grants: CapabilitySet,
    pub mounts: MountView,
    pub estimated_resources: ResourceEstimate,
    pub resource_reservation_id: Option<ResourceReservationId>,
    pub input: Value,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("unknown process {process_id}")]
    UnknownProcess { process_id: ProcessId },
    #[error("process {process_id} already exists")]
    ProcessAlreadyExists { process_id: ProcessId },
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
    #[error("filesystem error: {0}")]
    Filesystem(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("deserialization error: {0}")]
    Deserialization(String),
}

impl From<HostApiError> for ProcessError {
    fn from(error: HostApiError) -> Self {
        Self::InvalidPath(error.to_string())
    }
}

impl From<FilesystemError> for ProcessError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(error.to_string())
    }
}

#[async_trait]
pub trait ProcessManager: Send + Sync {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError>;
}

#[async_trait]
impl<T> ProcessManager for T
where
    T: ProcessStore + ?Sized,
{
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        self.start(start).await
    }
}

#[async_trait]
pub trait ProcessStore: Send + Sync {
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError>;
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError>;
    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError>;
    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError>;
    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError>;
    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProcessKey {
    tenant_id: TenantId,
    user_id: UserId,
    process_id: ProcessId,
}

impl ProcessKey {
    fn new(scope: &ResourceScope, process_id: ProcessId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            process_id,
        }
    }
}

#[derive(Debug, Default)]
pub struct InMemoryProcessStore {
    records: Mutex<HashMap<ProcessKey, ProcessRecord>>,
}

impl InMemoryProcessStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn records_guard(&self) -> MutexGuard<'_, HashMap<ProcessKey, ProcessRecord>> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        update: impl FnOnce(&mut ProcessRecord),
    ) -> Result<ProcessRecord, ProcessError> {
        let key = ProcessKey::new(scope, process_id);
        let mut records = self.records_guard();
        let record = records
            .get_mut(&key)
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        update(record);
        Ok(record.clone())
    }
}

#[async_trait]
impl ProcessStore for InMemoryProcessStore {
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let record = ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            error_kind: None,
        };
        let key = ProcessKey::new(&record.scope, record.process_id);
        let mut records = self.records_guard();
        if records.contains_key(&key) {
            return Err(ProcessError::ProcessAlreadyExists {
                process_id: record.process_id,
            });
        }
        records.insert(key, record.clone());
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update(scope, process_id, |record| {
            record.status = ProcessStatus::Completed;
            record.error_kind = None;
        })
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update(scope, process_id, |record| {
            record.status = ProcessStatus::Failed;
            record.error_kind = Some(error_kind);
        })
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update(scope, process_id, |record| {
            record.status = ProcessStatus::Killed;
            record.error_kind = None;
        })
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        Ok(self
            .records_guard()
            .get(&ProcessKey::new(scope, process_id))
            .cloned())
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        let mut records = self
            .records_guard()
            .values()
            .filter(|record| same_tenant_user(&record.scope, scope))
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| record.process_id.as_uuid());
        Ok(records)
    }
}

pub struct FilesystemProcessStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: &'a F,
}

impl<'a, F> FilesystemProcessStore<'a, F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: &'a F) -> Self {
        Self { filesystem }
    }

    async fn write_record(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        let path = process_record_path(&record.scope, record.process_id)?;
        let bytes = serialize_pretty(record)?;
        self.filesystem.write_file(&path, &bytes).await?;
        Ok(())
    }
}

#[async_trait]
impl<F> ProcessStore for FilesystemProcessStore<'_, F>
where
    F: RootFilesystem,
{
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        if self.get(&start.scope, start.process_id).await?.is_some() {
            return Err(ProcessError::ProcessAlreadyExists {
                process_id: start.process_id,
            });
        }
        let record = ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            error_kind: None,
        };
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let mut record = self
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        record.status = ProcessStatus::Completed;
        record.error_kind = None;
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        let mut record = self
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        record.status = ProcessStatus::Failed;
        record.error_kind = Some(error_kind);
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let mut record = self
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        record.status = ProcessStatus::Killed;
        record.error_kind = None;
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        let path = process_record_path(scope, process_id)?;
        let bytes = match self.filesystem.read_file(&path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record = deserialize::<ProcessRecord>(&bytes)?;
        if same_tenant_user(&record.scope, scope) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        let root = process_records_root(scope)?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            if entry.name.ends_with(".json") {
                let bytes = self.filesystem.read_file(&entry.path).await?;
                let record = deserialize::<ProcessRecord>(&bytes)?;
                if same_tenant_user(&record.scope, scope) {
                    records.push(record);
                }
            }
        }
        records.sort_by_key(|record| record.process_id.as_uuid());
        Ok(records)
    }
}

fn process_record_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!(
        "{}/{process_id}.json",
        process_records_root(scope)?.as_str()
    ))
    .map_err(Into::into)
}

fn process_records_root(scope: &ResourceScope) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!("{}/processes", tenant_user_root(scope))).map_err(Into::into)
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

fn serialize_pretty<T>(value: &T) -> Result<Vec<u8>, ProcessError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value).map_err(|error| ProcessError::Serialization(error.to_string()))
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, ProcessError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| ProcessError::Deserialization(error.to_string()))
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
