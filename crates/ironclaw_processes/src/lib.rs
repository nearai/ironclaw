//! Process lifecycle contracts for IronClaw Reborn.
//!
//! `ironclaw_processes` stores and manages host-tracked background capability
//! processes. It owns lifecycle mechanics, not capability authorization or
//! runtime dispatch policy.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_events::{EventSink, RuntimeEvent};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityId, CapabilitySet, ExtensionId, HostApiError, InvocationId, MountView, ProcessId,
    ResourceEstimate, ResourceReservationId, ResourceScope, ResourceUsage, RuntimeKind, TenantId,
    UserId, VirtualPath,
};
use ironclaw_resources::{ResourceError, ResourceGovernor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::{
    sync::Mutex as AsyncMutex,
    time::{Duration, sleep},
};

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

/// Terminal process state returned by host-facing await operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessExit {
    pub process_id: ProcessId,
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub status: ProcessStatus,
    pub error_kind: Option<String>,
}

impl ProcessExit {
    fn from_terminal(record: ProcessRecord) -> Self {
        debug_assert!(record.status != ProcessStatus::Running);
        Self {
            process_id: record.process_id,
            scope: record.scope,
            extension_id: record.extension_id,
            capability_id: record.capability_id,
            runtime: record.runtime,
            status: record.status,
            error_kind: record.error_kind,
        }
    }
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("unknown process {process_id}")]
    UnknownProcess { process_id: ProcessId },
    #[error("process {process_id} already exists")]
    ProcessAlreadyExists { process_id: ProcessId },
    #[error("process {process_id} cannot transition from {from:?} to {to:?}")]
    InvalidTransition {
        process_id: ProcessId,
        from: ProcessStatus,
        to: ProcessStatus,
    },
    #[error("process {process_id} returned reservation {actual:?}, expected {expected}")]
    ResourceReservationMismatch {
        process_id: ProcessId,
        expected: ResourceReservationId,
        actual: Option<ResourceReservationId>,
    },
    #[error("resource lifecycle error: {0}")]
    Resource(ResourceError),
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

impl From<ResourceError> for ProcessError {
    fn from(error: ResourceError) -> Self {
        Self::Resource(error)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessExecutionRequest {
    pub process_id: ProcessId,
    pub invocation_id: InvocationId,
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub capability_id: CapabilityId,
    pub runtime: RuntimeKind,
    pub estimate: ResourceEstimate,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessExecutionResult {
    pub output: Value,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("process execution failed: {kind}")]
pub struct ProcessExecutionError {
    pub kind: String,
}

impl ProcessExecutionError {
    pub fn new(kind: impl Into<String>) -> Self {
        Self { kind: kind.into() }
    }
}

#[async_trait]
pub trait ProcessExecutor: Send + Sync {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError>;
}

#[async_trait]
pub trait ProcessManager: Send + Sync {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError>;
}

/// Host-facing lifecycle API over process current state.
pub struct ProcessHost<'a> {
    store: &'a dyn ProcessStore,
    poll_interval: Duration,
}

impl<'a> ProcessHost<'a> {
    pub fn new(store: &'a dyn ProcessStore) -> Self {
        Self {
            store,
            poll_interval: Duration::from_millis(10),
        }
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub async fn status(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        self.store.get(scope, process_id).await
    }

    pub async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.store.kill(scope, process_id).await
    }

    pub async fn await_process(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessExit, ProcessError> {
        loop {
            let record = self
                .store
                .get(scope, process_id)
                .await?
                .ok_or(ProcessError::UnknownProcess { process_id })?;
            if record.status != ProcessStatus::Running {
                return Ok(ProcessExit::from_terminal(record));
            }
            sleep(self.poll_interval).await;
        }
    }
}

pub struct EventingProcessStore<S>
where
    S: ProcessStore,
{
    inner: S,
    event_sink: Arc<dyn EventSink>,
}

impl<S> EventingProcessStore<S>
where
    S: ProcessStore,
{
    pub fn new(inner: S, event_sink: Arc<dyn EventSink>) -> Self {
        Self { inner, event_sink }
    }

    async fn emit_best_effort(&self, event: RuntimeEvent) {
        let _ = self.event_sink.emit(event).await;
    }
}

#[async_trait]
impl<S> ProcessStore for EventingProcessStore<S>
where
    S: ProcessStore,
{
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.start(start).await?;
        self.emit_best_effort(RuntimeEvent::process_started(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
        ))
        .await;
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.complete(scope, process_id).await?;
        self.emit_best_effort(RuntimeEvent::process_completed(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
        ))
        .await;
        Ok(record)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.fail(scope, process_id, error_kind).await?;
        self.emit_best_effort(RuntimeEvent::process_failed(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
            record
                .error_kind
                .clone()
                .unwrap_or_else(|| "Unknown".to_string()),
        ))
        .await;
        Ok(record)
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.kill(scope, process_id).await?;
        self.emit_best_effort(RuntimeEvent::process_killed(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
        ))
        .await;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        self.inner.get(scope, process_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        self.inner.records_for_scope(scope).await
    }
}

pub struct ResourceManagedProcessStore<S, G>
where
    S: ProcessStore,
    G: ResourceGovernor + ?Sized,
{
    inner: S,
    governor: Arc<G>,
    completion_usage: ResourceUsage,
}

impl<S, G> ResourceManagedProcessStore<S, G>
where
    S: ProcessStore,
    G: ResourceGovernor + ?Sized,
{
    pub fn new(inner: S, governor: Arc<G>) -> Self {
        Self {
            inner,
            governor,
            completion_usage: ResourceUsage::default(),
        }
    }

    pub fn with_completion_usage(mut self, usage: ResourceUsage) -> Self {
        self.completion_usage = usage;
        self
    }

    fn release_reservation(
        &self,
        reservation_id: Option<ResourceReservationId>,
    ) -> Result<(), ProcessError> {
        if let Some(reservation_id) = reservation_id {
            self.governor.release(reservation_id)?;
        }
        Ok(())
    }

    fn reconcile_reservation(
        &self,
        reservation_id: Option<ResourceReservationId>,
    ) -> Result<(), ProcessError> {
        if let Some(reservation_id) = reservation_id {
            self.governor
                .reconcile(reservation_id, self.completion_usage.clone())?;
        }
        Ok(())
    }
}

#[async_trait]
impl<S, G> ProcessStore for ResourceManagedProcessStore<S, G>
where
    S: ProcessStore,
    G: ResourceGovernor + ?Sized,
{
    async fn start(&self, mut start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        if start.resource_reservation_id.is_some() {
            return self.inner.start(start).await;
        }

        let reservation = self
            .governor
            .reserve(start.scope.clone(), start.estimated_resources.clone())?;
        start.resource_reservation_id = Some(reservation.id);
        match self.inner.start(start).await {
            Ok(record) if record.resource_reservation_id == Some(reservation.id) => Ok(record),
            Ok(record) => {
                self.release_reservation(Some(reservation.id))?;
                Err(ProcessError::ResourceReservationMismatch {
                    process_id: record.process_id,
                    expected: reservation.id,
                    actual: record.resource_reservation_id,
                })
            }
            Err(error) => {
                self.release_reservation(Some(reservation.id))?;
                Err(error)
            }
        }
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.complete(scope, process_id).await?;
        self.reconcile_reservation(record.resource_reservation_id)?;
        Ok(record)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.fail(scope, process_id, error_kind).await?;
        self.release_reservation(record.resource_reservation_id)?;
        Ok(record)
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.kill(scope, process_id).await?;
        self.release_reservation(record.resource_reservation_id)?;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        self.inner.get(scope, process_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        self.inner.records_for_scope(scope).await
    }
}

pub struct BackgroundProcessManager {
    store: Arc<dyn ProcessStore>,
    executor: Arc<dyn ProcessExecutor + 'static>,
}

impl BackgroundProcessManager {
    pub fn new<S, E>(store: Arc<S>, executor: Arc<E>) -> Self
    where
        S: ProcessStore + 'static,
        E: ProcessExecutor + 'static,
    {
        Self { store, executor }
    }
}

#[async_trait]
impl ProcessManager for BackgroundProcessManager {
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let input = start.input.clone();
        let record = self.store.start(start).await?;
        let store = Arc::clone(&self.store);
        let executor = Arc::clone(&self.executor);
        let scope = record.scope.clone();
        let process_id = record.process_id;
        let dispatch_estimate = if record.resource_reservation_id.is_some() {
            ResourceEstimate::default()
        } else {
            record.estimated_resources.clone()
        };
        let request = ProcessExecutionRequest {
            process_id: record.process_id,
            invocation_id: record.invocation_id,
            scope: record.scope.clone(),
            extension_id: record.extension_id.clone(),
            capability_id: record.capability_id.clone(),
            runtime: record.runtime,
            estimate: dispatch_estimate,
            input,
        };
        tokio::spawn(async move {
            match executor.execute(request).await {
                Ok(_result) => {
                    let _ = store.complete(&scope, process_id).await;
                }
                Err(error) => {
                    let _ = store.fail(&scope, process_id, error.kind).await;
                }
            }
        });
        Ok(record)
    }
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

    fn update_status(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        to: ProcessStatus,
        error_kind: Option<String>,
    ) -> Result<ProcessRecord, ProcessError> {
        let key = ProcessKey::new(scope, process_id);
        let mut records = self.records_guard();
        let record = records
            .get_mut(&key)
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        ensure_status_transition(process_id, record.status, to)?;
        record.status = to;
        record.error_kind = error_kind;
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
        self.update_status(scope, process_id, ProcessStatus::Completed, None)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Failed, Some(error_kind))
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Killed, None)
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

enum FilesystemHandle<'a, F>
where
    F: RootFilesystem,
{
    Borrowed(&'a F),
    Shared(Arc<F>),
}

impl<F> FilesystemHandle<'_, F>
where
    F: RootFilesystem,
{
    fn as_ref(&self) -> &F {
        match self {
            Self::Borrowed(filesystem) => filesystem,
            Self::Shared(filesystem) => filesystem.as_ref(),
        }
    }
}

pub struct FilesystemProcessStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: FilesystemHandle<'a, F>,
    transition_lock: AsyncMutex<()>,
}

impl<'a, F> FilesystemProcessStore<'a, F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: &'a F) -> Self {
        Self {
            filesystem: FilesystemHandle::Borrowed(filesystem),
            transition_lock: AsyncMutex::new(()),
        }
    }

    pub fn from_arc(filesystem: Arc<F>) -> FilesystemProcessStore<'static, F> {
        FilesystemProcessStore {
            filesystem: FilesystemHandle::Shared(filesystem),
            transition_lock: AsyncMutex::new(()),
        }
    }

    async fn write_record(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        let path = process_record_path(&record.scope, record.process_id)?;
        let bytes = serialize_pretty(record)?;
        self.filesystem.as_ref().write_file(&path, &bytes).await?;
        Ok(())
    }

    async fn update_status(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        to: ProcessStatus,
        error_kind: Option<String>,
    ) -> Result<ProcessRecord, ProcessError> {
        let _guard = self.transition_lock.lock().await;
        let mut record = self
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        ensure_status_transition(process_id, record.status, to)?;
        record.status = to;
        record.error_kind = error_kind;
        self.write_record(&record).await?;
        Ok(record)
    }
}

#[async_trait]
impl<F> ProcessStore for FilesystemProcessStore<'_, F>
where
    F: RootFilesystem,
{
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let _guard = self.transition_lock.lock().await;
        let path = process_record_path(&start.scope, start.process_id)?;
        let existing = match self.filesystem.as_ref().read_file(&path).await {
            Ok(_) => true,
            Err(error) if is_not_found(&error) => false,
            Err(error) => return Err(error.into()),
        };
        if existing {
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
        self.update_status(scope, process_id, ProcessStatus::Completed, None)
            .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Failed, Some(error_kind))
            .await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Killed, None)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        let path = process_record_path(scope, process_id)?;
        let bytes = match self.filesystem.as_ref().read_file(&path).await {
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
        let entries = match self.filesystem.as_ref().list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            if entry.name.ends_with(".json") {
                let bytes = self.filesystem.as_ref().read_file(&entry.path).await?;
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

fn ensure_status_transition(
    process_id: ProcessId,
    from: ProcessStatus,
    to: ProcessStatus,
) -> Result<(), ProcessError> {
    if from != ProcessStatus::Running {
        return Err(ProcessError::InvalidTransition {
            process_id,
            from,
            to,
        });
    }
    Ok(())
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
