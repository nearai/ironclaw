//! Process lifecycle contracts for IronClaw Reborn.
//!
//! `ironclaw_processes` stores and manages host-tracked background capability
//! processes. It owns lifecycle mechanics, not capability authorization or
//! runtime dispatch policy.

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use futures::FutureExt;
use ironclaw_events::{EventSink, RuntimeEvent};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    AgentId, CapabilityId, CapabilitySet, ExtensionId, HostApiError, InvocationId, MountView,
    ProcessId, ResourceEstimate, ResourceReservationId, ResourceScope, ResourceUsage, RuntimeKind,
    TenantId, UserId, VirtualPath,
};
use ironclaw_resources::{ResourceError, ResourceGovernor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::{
    sync::{Mutex as AsyncMutex, Notify},
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

impl ProcessStatus {
    pub fn is_terminal(self) -> bool {
        self != Self::Running
    }
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessResultRecord {
    pub process_id: ProcessId,
    pub scope: ResourceScope,
    pub status: ProcessStatus,
    pub output: Option<Value>,
    pub output_ref: Option<VirtualPath>,
    pub error_kind: Option<String>,
}

impl ProcessExit {
    fn from_terminal(record: ProcessRecord) -> Self {
        debug_assert!(record.status.is_terminal());
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
    #[error(
        "process {process_id} start cannot supply pre-existing resource reservation {reservation_id}"
    )]
    ResourceReservationAlreadyAssigned {
        process_id: ProcessId,
        reservation_id: ResourceReservationId,
    },
    #[error(
        "process {process_id} resource reservation {reservation_id:?} is not owned by this store"
    )]
    ResourceReservationNotOwned {
        process_id: ProcessId,
        reservation_id: Option<ResourceReservationId>,
    },
    #[error("resource lifecycle error: {0}")]
    Resource(ResourceError),
    #[error("process result store is not configured")]
    ProcessResultStoreUnavailable,
    #[error("process result is unavailable for {process_id}")]
    ProcessResultUnavailable { process_id: ProcessId },
    #[error("invalid stored process record: {reason}")]
    InvalidStoredRecord { reason: String },
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
    pub cancellation: ProcessCancellationToken,
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
    /// Runs one background process request and must observe cooperative cancellation where possible.
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError>;
}

#[async_trait]
pub trait ProcessManager: Send + Sync {
    /// Starts process lifecycle tracking before detached execution begins.
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError>;
}

#[async_trait]
pub trait ProcessResultStore: Send + Sync {
    /// Stores successful process output separately from the lifecycle record.
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: Value,
    ) -> Result<ProcessResultRecord, ProcessError>;

    /// Stores a classified process failure without raw backend detail strings.
    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError>;

    /// Stores killed process result metadata without implying executor preemption succeeded.
    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError>;

    /// Loads scoped result metadata; wrong-scope lookups must look unknown.
    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError>;

    /// Loads scoped process output, keeping large/sensitive output outside lifecycle records.
    async fn output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<Value>, ProcessError> {
        Ok(self
            .get(scope, process_id)
            .await?
            .and_then(|record| record.output))
    }
}

#[derive(Clone)]
pub struct ProcessCancellationToken {
    inner: Arc<ProcessCancellationState>,
}

struct ProcessCancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

impl Default for ProcessCancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessCancellationToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ProcessCancellationState {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

impl fmt::Debug for ProcessCancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessCancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl PartialEq for ProcessCancellationToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

#[derive(Debug, Default)]
pub struct ProcessCancellationRegistry {
    tokens: Mutex<HashMap<ProcessKey, ProcessCancellationToken>>,
}

impl ProcessCancellationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> ProcessCancellationToken {
        let token = ProcessCancellationToken::new();
        self.tokens_guard()
            .insert(ProcessKey::new(scope, process_id), token.clone());
        token
    }

    pub fn cancel(&self, scope: &ResourceScope, process_id: ProcessId) -> bool {
        let token = self
            .tokens_guard()
            .remove(&ProcessKey::new(scope, process_id));
        if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub fn unregister(&self, scope: &ResourceScope, process_id: ProcessId) {
        self.tokens_guard()
            .remove(&ProcessKey::new(scope, process_id));
    }

    fn tokens_guard(&self) -> MutexGuard<'_, HashMap<ProcessKey, ProcessCancellationToken>> {
        self.tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Host-facing lifecycle API over process current state.
pub struct ProcessHost<'a> {
    store: &'a dyn ProcessStore,
    poll_interval: Duration,
    cancellation_registry: Option<Arc<ProcessCancellationRegistry>>,
    result_store: Option<Arc<dyn ProcessResultStore>>,
}

impl<'a> ProcessHost<'a> {
    pub fn new(store: &'a dyn ProcessStore) -> Self {
        Self {
            store,
            poll_interval: Duration::from_millis(10),
            cancellation_registry: None,
            result_store: None,
        }
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub fn with_cancellation_registry(
        mut self,
        registry: Arc<ProcessCancellationRegistry>,
    ) -> Self {
        self.cancellation_registry = Some(registry);
        self
    }

    pub fn with_result_store<S>(mut self, store: Arc<S>) -> Self
    where
        S: ProcessResultStore + 'static,
    {
        self.result_store = Some(store);
        self
    }

    fn result_store(&self) -> Result<&dyn ProcessResultStore, ProcessError> {
        self.result_store
            .as_deref()
            .ok_or(ProcessError::ProcessResultStoreUnavailable)
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
        let record = self.store.kill(scope, process_id).await?;
        if let Some(registry) = &self.cancellation_registry {
            registry.cancel(scope, process_id);
        }
        if let Some(result_store) = &self.result_store {
            result_store.kill(&record.scope, record.process_id).await?;
        }
        Ok(record)
    }

    pub async fn result(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        self.result_store()?.get(scope, process_id).await
    }

    pub async fn output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<Value>, ProcessError> {
        self.result_store()?.output(scope, process_id).await
    }

    pub async fn await_result(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        let mut terminal_without_result_seen = false;
        loop {
            if let Some(result) = self.result(scope, process_id).await? {
                return Ok(result);
            }
            let record = self
                .store
                .get(scope, process_id)
                .await?
                .ok_or(ProcessError::UnknownProcess { process_id })?;
            if record.status.is_terminal() {
                if self.result_store.is_none() || terminal_without_result_seen {
                    return Err(ProcessError::ProcessResultUnavailable { process_id });
                }
                terminal_without_result_seen = true;
            } else {
                terminal_without_result_seen = false;
            }
            sleep(self.poll_interval).await;
        }
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
            if record.status.is_terminal() {
                return Ok(ProcessExit::from_terminal(record));
            }
            sleep(self.poll_interval).await;
        }
    }

    pub async fn subscribe(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessSubscription<'a>, ProcessError> {
        let initial_record = self
            .store
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        Ok(ProcessSubscription {
            store: self.store,
            scope: scope.clone(),
            process_id,
            poll_interval: self.poll_interval,
            last_status: Some(initial_record.status),
            pending_initial: Some(initial_record),
            finished: false,
        })
    }
}

/// Scoped subscription over process lifecycle status changes.
pub struct ProcessSubscription<'a> {
    store: &'a dyn ProcessStore,
    scope: ResourceScope,
    process_id: ProcessId,
    poll_interval: Duration,
    last_status: Option<ProcessStatus>,
    pending_initial: Option<ProcessRecord>,
    finished: bool,
}

impl fmt::Debug for ProcessSubscription<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessSubscription")
            .field("scope", &self.scope)
            .field("process_id", &self.process_id)
            .field("last_status", &self.last_status)
            .field(
                "pending_initial_status",
                &self.pending_initial.as_ref().map(|record| record.status),
            )
            .field("finished", &self.finished)
            .finish()
    }
}

impl ProcessSubscription<'_> {
    pub async fn next(&mut self) -> Result<Option<ProcessRecord>, ProcessError> {
        if let Some(record) = self.pending_initial.take() {
            if record.status.is_terminal() {
                self.finished = true;
            }
            return Ok(Some(record));
        }

        if self.finished {
            return Ok(None);
        }

        loop {
            let record = self.store.get(&self.scope, self.process_id).await?.ok_or(
                ProcessError::UnknownProcess {
                    process_id: self.process_id,
                },
            )?;
            if Some(record.status) != self.last_status {
                self.last_status = Some(record.status);
                if record.status.is_terminal() {
                    self.finished = true;
                }
                return Ok(Some(record));
            }
            sleep(self.poll_interval).await;
        }
    }
}

#[derive(Debug, Default)]
pub struct InMemoryProcessResultStore {
    records: Mutex<HashMap<ProcessKey, ProcessResultRecord>>,
}

impl InMemoryProcessResultStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn records_guard(&self) -> MutexGuard<'_, HashMap<ProcessKey, ProcessResultRecord>> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn store_result(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        status: ProcessStatus,
        output: Option<Value>,
        error_kind: Option<String>,
    ) -> ProcessResultRecord {
        let record = ProcessResultRecord {
            process_id,
            scope: scope.clone(),
            status,
            output,
            output_ref: None,
            error_kind,
        };
        self.records_guard()
            .insert(ProcessKey::new(scope, process_id), record.clone());
        record
    }
}

#[async_trait]
impl ProcessResultStore for InMemoryProcessResultStore {
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: Value,
    ) -> Result<ProcessResultRecord, ProcessError> {
        Ok(self.store_result(
            scope,
            process_id,
            ProcessStatus::Completed,
            Some(output),
            None,
        ))
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError> {
        Ok(self.store_result(
            scope,
            process_id,
            ProcessStatus::Failed,
            None,
            Some(error_kind),
        ))
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        Ok(self.store_result(scope, process_id, ProcessStatus::Killed, None, None))
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        Ok(self
            .records_guard()
            .get(&ProcessKey::new(scope, process_id))
            .cloned())
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
    owned_reservations: Mutex<HashMap<ProcessKey, ResourceReservationId>>,
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
            owned_reservations: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_completion_usage(mut self, usage: ResourceUsage) -> Self {
        self.completion_usage = usage;
        self
    }

    fn owned_reservations_guard(
        &self,
    ) -> MutexGuard<'_, HashMap<ProcessKey, ResourceReservationId>> {
        self.owned_reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn record_owned_reservation(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        reservation_id: ResourceReservationId,
    ) {
        self.owned_reservations_guard()
            .insert(ProcessKey::new(scope, process_id), reservation_id);
    }

    fn take_owned_reservation(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        record_reservation_id: Option<ResourceReservationId>,
    ) -> Result<ResourceReservationId, ProcessError> {
        let reservation_id = self
            .owned_reservations_guard()
            .remove(&ProcessKey::new(scope, process_id))
            .ok_or(ProcessError::ResourceReservationNotOwned {
                process_id,
                reservation_id: record_reservation_id,
            })?;
        if Some(reservation_id) != record_reservation_id {
            self.owned_reservations_guard()
                .insert(ProcessKey::new(scope, process_id), reservation_id);
            return Err(ProcessError::ResourceReservationMismatch {
                process_id,
                expected: reservation_id,
                actual: record_reservation_id,
            });
        }
        Ok(reservation_id)
    }

    fn release_reservation(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<(), ProcessError> {
        self.governor.release(reservation_id)?;
        Ok(())
    }

    fn reconcile_reservation(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<(), ProcessError> {
        self.governor
            .reconcile(reservation_id, self.completion_usage.clone())?;
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
        if let Some(reservation_id) = start.resource_reservation_id {
            return Err(ProcessError::ResourceReservationAlreadyAssigned {
                process_id: start.process_id,
                reservation_id,
            });
        }

        let reservation = self
            .governor
            .reserve(start.scope.clone(), start.estimated_resources.clone())?;
        start.resource_reservation_id = Some(reservation.id);
        match self.inner.start(start).await {
            Ok(record) if record.resource_reservation_id == Some(reservation.id) => {
                self.record_owned_reservation(&record.scope, record.process_id, reservation.id);
                Ok(record)
            }
            Ok(record) => {
                self.release_reservation(reservation.id)?;
                Err(ProcessError::ResourceReservationMismatch {
                    process_id: record.process_id,
                    expected: reservation.id,
                    actual: record.resource_reservation_id,
                })
            }
            Err(error) => {
                self.release_reservation(reservation.id)?;
                Err(error)
            }
        }
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let current = self
            .inner
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        let reservation_id =
            self.take_owned_reservation(scope, process_id, current.resource_reservation_id)?;
        let record = match self.inner.complete(scope, process_id).await {
            Ok(record) => record,
            Err(error) => {
                self.record_owned_reservation(scope, process_id, reservation_id);
                return Err(error);
            }
        };
        if record.resource_reservation_id != Some(reservation_id) {
            self.reconcile_reservation(reservation_id)?;
            return Err(ProcessError::ResourceReservationMismatch {
                process_id: record.process_id,
                expected: reservation_id,
                actual: record.resource_reservation_id,
            });
        }
        self.reconcile_reservation(reservation_id)?;
        Ok(record)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        let current = self
            .inner
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        let reservation_id =
            self.take_owned_reservation(scope, process_id, current.resource_reservation_id)?;
        let record = match self.inner.fail(scope, process_id, error_kind).await {
            Ok(record) => record,
            Err(error) => {
                self.record_owned_reservation(scope, process_id, reservation_id);
                return Err(error);
            }
        };
        if record.resource_reservation_id != Some(reservation_id) {
            self.release_reservation(reservation_id)?;
            return Err(ProcessError::ResourceReservationMismatch {
                process_id: record.process_id,
                expected: reservation_id,
                actual: record.resource_reservation_id,
            });
        }
        self.release_reservation(reservation_id)?;
        Ok(record)
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let current = self
            .inner
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        let reservation_id =
            self.take_owned_reservation(scope, process_id, current.resource_reservation_id)?;
        let record = match self.inner.kill(scope, process_id).await {
            Ok(record) => record,
            Err(error) => {
                self.record_owned_reservation(scope, process_id, reservation_id);
                return Err(error);
            }
        };
        if record.resource_reservation_id != Some(reservation_id) {
            self.release_reservation(reservation_id)?;
            return Err(ProcessError::ResourceReservationMismatch {
                process_id: record.process_id,
                expected: reservation_id,
                actual: record.resource_reservation_id,
            });
        }
        self.release_reservation(reservation_id)?;
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

pub struct ProcessServices<S, R>
where
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
{
    process_store: Arc<S>,
    result_store: Arc<R>,
    cancellation_registry: Arc<ProcessCancellationRegistry>,
}

impl<S, R> Clone for ProcessServices<S, R>
where
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
{
    fn clone(&self) -> Self {
        Self {
            process_store: Arc::clone(&self.process_store),
            result_store: Arc::clone(&self.result_store),
            cancellation_registry: Arc::clone(&self.cancellation_registry),
        }
    }
}

impl<S, R> ProcessServices<S, R>
where
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
{
    pub fn new(process_store: Arc<S>, result_store: Arc<R>) -> Self {
        Self::from_parts(
            process_store,
            result_store,
            Arc::new(ProcessCancellationRegistry::new()),
        )
    }

    pub fn from_parts(
        process_store: Arc<S>,
        result_store: Arc<R>,
        cancellation_registry: Arc<ProcessCancellationRegistry>,
    ) -> Self {
        Self {
            process_store,
            result_store,
            cancellation_registry,
        }
    }

    pub fn process_store(&self) -> Arc<S> {
        Arc::clone(&self.process_store)
    }

    pub fn result_store(&self) -> Arc<R> {
        Arc::clone(&self.result_store)
    }

    pub fn cancellation_registry(&self) -> Arc<ProcessCancellationRegistry> {
        Arc::clone(&self.cancellation_registry)
    }

    pub fn host(&self) -> ProcessHost<'_> {
        ProcessHost::new(self.process_store.as_ref())
            .with_cancellation_registry(Arc::clone(&self.cancellation_registry))
            .with_result_store(Arc::clone(&self.result_store))
    }

    pub fn background_manager<E>(&self, executor: Arc<E>) -> BackgroundProcessManager
    where
        E: ProcessExecutor + 'static,
    {
        BackgroundProcessManager::new(Arc::clone(&self.process_store), executor)
            .with_cancellation_registry(Arc::clone(&self.cancellation_registry))
            .with_result_store(Arc::clone(&self.result_store))
    }
}

impl ProcessServices<InMemoryProcessStore, InMemoryProcessResultStore> {
    pub fn in_memory() -> Self {
        Self::new(
            Arc::new(InMemoryProcessStore::new()),
            Arc::new(InMemoryProcessResultStore::new()),
        )
    }
}

impl<F>
    ProcessServices<FilesystemProcessStore<'static, F>, FilesystemProcessResultStore<'static, F>>
where
    F: RootFilesystem + 'static,
{
    pub fn filesystem(filesystem: Arc<F>) -> Self {
        Self::new(
            Arc::new(FilesystemProcessStore::from_arc(Arc::clone(&filesystem))),
            Arc::new(FilesystemProcessResultStore::from_arc(filesystem)),
        )
    }
}

pub struct BackgroundProcessManager {
    store: Arc<dyn ProcessStore>,
    executor: Arc<dyn ProcessExecutor + 'static>,
    cancellation_registry: Option<Arc<ProcessCancellationRegistry>>,
    result_store: Option<Arc<dyn ProcessResultStore>>,
}

impl BackgroundProcessManager {
    pub fn new<S, E>(store: Arc<S>, executor: Arc<E>) -> Self
    where
        S: ProcessStore + 'static,
        E: ProcessExecutor + 'static,
    {
        Self {
            store,
            executor,
            cancellation_registry: None,
            result_store: None,
        }
    }

    pub fn with_cancellation_registry(
        mut self,
        registry: Arc<ProcessCancellationRegistry>,
    ) -> Self {
        self.cancellation_registry = Some(registry);
        self
    }

    pub fn with_result_store<S>(mut self, store: Arc<S>) -> Self
    where
        S: ProcessResultStore + 'static,
    {
        self.result_store = Some(store);
        self
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
        let cancellation_registry = self.cancellation_registry.clone();
        let result_store = self.result_store.clone();
        let cancellation = cancellation_registry
            .as_ref()
            .map(|registry| registry.register(&record.scope, record.process_id))
            .unwrap_or_default();
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
            cancellation,
        };
        tokio::spawn(async move {
            match std::panic::AssertUnwindSafe(executor.execute(request))
                .catch_unwind()
                .await
            {
                Ok(Ok(result)) => {
                    if let Ok(record) = store.complete(&scope, process_id).await
                        && let Some(result_store) = &result_store
                    {
                        let _ = result_store
                            .complete(&record.scope, record.process_id, result.output)
                            .await;
                    }
                }
                Ok(Err(error)) => {
                    if let Ok(record) = store.fail(&scope, process_id, error.kind).await
                        && let Some(result_store) = &result_store
                        && let Some(error_kind) = record.error_kind.clone()
                    {
                        let _ = result_store
                            .fail(&record.scope, record.process_id, error_kind)
                            .await;
                    }
                }
                Err(_) => {
                    if let Ok(record) = store
                        .fail(&scope, process_id, "runtime_panic".to_string())
                        .await
                        && let Some(result_store) = &result_store
                    {
                        let _ = result_store
                            .fail(
                                &record.scope,
                                record.process_id,
                                "runtime_panic".to_string(),
                            )
                            .await;
                    }
                }
            }
            if let Some(registry) = cancellation_registry {
                registry.unregister(&scope, process_id);
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
    /// Persists a running process record without storing raw input.
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError>;

    /// Transitions a scoped running process to completed.
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError>;

    /// Transitions a scoped running process to failed with a classified error kind.
    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError>;

    /// Marks a scoped process killed and must not reveal cross-tenant process existence.
    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError>;

    /// Loads scoped process lifecycle metadata; wrong-scope lookups must look unknown.
    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError>;

    /// Lists process lifecycle records visible to the tenant/user/agent scope only.
    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProcessKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    process_id: ProcessId,
}

impl ProcessKey {
    fn new(scope: &ResourceScope, process_id: ProcessId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
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
            .filter(|record| same_scope_owner(&record.scope, scope))
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
        ensure_process_record_matches(&record, process_id)?;
        if same_scope_owner(&record.scope, scope) {
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
                if same_scope_owner(&record.scope, scope) {
                    records.push(record);
                }
            }
        }
        records.sort_by_key(|record| record.process_id.as_uuid());
        Ok(records)
    }
}

pub struct FilesystemProcessResultStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: FilesystemHandle<'a, F>,
}

impl<'a, F> FilesystemProcessResultStore<'a, F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: &'a F) -> Self {
        Self {
            filesystem: FilesystemHandle::Borrowed(filesystem),
        }
    }

    pub fn from_arc(filesystem: Arc<F>) -> FilesystemProcessResultStore<'static, F> {
        FilesystemProcessResultStore {
            filesystem: FilesystemHandle::Shared(filesystem),
        }
    }

    async fn write_result(&self, record: &ProcessResultRecord) -> Result<(), ProcessError> {
        let path = process_result_path(&record.scope, record.process_id)?;
        let bytes = serialize_pretty(record)?;
        self.filesystem.as_ref().write_file(&path, &bytes).await?;
        Ok(())
    }

    async fn write_output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: &Value,
    ) -> Result<VirtualPath, ProcessError> {
        let path = process_output_path(scope, process_id)?;
        let bytes = serialize_pretty(output)?;
        self.filesystem.as_ref().write_file(&path, &bytes).await?;
        Ok(path)
    }

    async fn store_result(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        status: ProcessStatus,
        output: Option<Value>,
        output_ref: Option<VirtualPath>,
        error_kind: Option<String>,
    ) -> Result<ProcessResultRecord, ProcessError> {
        let record = ProcessResultRecord {
            process_id,
            scope: scope.clone(),
            status,
            output,
            output_ref,
            error_kind,
        };
        self.write_result(&record).await?;
        Ok(record)
    }
}

#[async_trait]
impl<F> ProcessResultStore for FilesystemProcessResultStore<'_, F>
where
    F: RootFilesystem,
{
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: Value,
    ) -> Result<ProcessResultRecord, ProcessError> {
        let output_ref = self.write_output(scope, process_id, &output).await?;
        self.store_result(
            scope,
            process_id,
            ProcessStatus::Completed,
            None,
            Some(output_ref),
            None,
        )
        .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.store_result(
            scope,
            process_id,
            ProcessStatus::Failed,
            None,
            None,
            Some(error_kind),
        )
        .await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.store_result(scope, process_id, ProcessStatus::Killed, None, None, None)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        let path = process_result_path(scope, process_id)?;
        let bytes = match self.filesystem.as_ref().read_file(&path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record = deserialize::<ProcessResultRecord>(&bytes)?;
        ensure_result_record_matches(&record, process_id)?;
        if same_scope_owner(&record.scope, scope) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    async fn output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<Value>, ProcessError> {
        let Some(record) = self.get(scope, process_id).await? else {
            return Ok(None);
        };
        if let Some(output) = record.output {
            return Ok(Some(output));
        }
        let Some(output_ref) = record.output_ref else {
            return Ok(None);
        };
        let expected_output_ref = process_output_path(scope, process_id)?;
        if output_ref != expected_output_ref {
            return Err(invalid_stored_record(format!(
                "process result output ref {} does not match expected {}",
                output_ref.as_str(),
                expected_output_ref.as_str()
            )));
        }
        let bytes = match self.filesystem.as_ref().read_file(&output_ref).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        deserialize::<Value>(&bytes).map(Some)
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

fn process_result_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!(
        "{}/{process_id}.json",
        process_results_root(scope)?.as_str()
    ))
    .map_err(Into::into)
}

fn process_results_root(scope: &ResourceScope) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!("{}/process-results", tenant_user_root(scope))).map_err(Into::into)
}

fn process_output_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!(
        "{}/output.json",
        process_outputs_root(scope, process_id)?.as_str()
    ))
    .map_err(Into::into)
}

fn process_outputs_root(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!(
        "{}/process-outputs/{process_id}",
        tenant_user_root(scope)
    ))
    .map_err(Into::into)
}

fn tenant_user_root(scope: &ResourceScope) -> String {
    let base = format!(
        "/engine/tenants/{}/users/{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    );
    match &scope.agent_id {
        Some(agent_id) => format!("{base}/agents/{}", agent_id.as_str()),
        None => base,
    }
}

fn same_scope_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
}

fn ensure_process_record_matches(
    record: &ProcessRecord,
    process_id: ProcessId,
) -> Result<(), ProcessError> {
    if record.process_id != process_id {
        return Err(invalid_stored_record(format!(
            "stored process id {} does not match requested {}",
            record.process_id, process_id
        )));
    }
    Ok(())
}

fn ensure_result_record_matches(
    record: &ProcessResultRecord,
    process_id: ProcessId,
) -> Result<(), ProcessError> {
    if record.process_id != process_id {
        return Err(invalid_stored_record(format!(
            "stored process result id {} does not match requested {}",
            record.process_id, process_id
        )));
    }
    Ok(())
}

fn invalid_stored_record(reason: impl Into<String>) -> ProcessError {
    ProcessError::InvalidStoredRecord {
        reason: reason.into(),
    }
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
        FilesystemError::NotFound { .. } => true,
        FilesystemError::Backend { reason, .. } => {
            reason.contains("No such file")
                || reason.contains("not found")
                || reason.contains("os error 2")
        }
        _ => false,
    }
}
