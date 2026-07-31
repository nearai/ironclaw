//! Composition + spawn surface for process services.
//!
//! - [`ProcessServices`] bundles a process runtime, a result store, and a
//!   shared [`ProcessCancellationRegistry`] so the host and background manager
//!   stay coordinated through a single graph.
//! - [`BackgroundProcessManager`] is the capability [`ProcessManager`] that
//!   journals capability work and registers its executor with the generic
//!   [`ProcessSupervisor`].
//!
//! Executor persistence failures can be observed by attaching a
//! [`with_error_handler`](BackgroundProcessManager::with_error_handler)
//! callback. Without a handler, those errors are silently dropped.

use std::{
    any::{TypeId, type_name},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures::FutureExt;
use ironclaw_events::sanitize_error_kind;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    ids::ProcessId,
    resource::{ResourceReservation, ResourceScope},
};

use crate::cancellation::ProcessCancellationRegistry;
use crate::capability_process::{
    capability_process_record, complete_capability_process, fail_capability_process,
};
use crate::host::ProcessHost;
use crate::result_store::ProcessResultStore;
use crate::types::{
    ProcessError, ProcessExecutionRequest, ProcessExecutor, ProcessManager, ProcessRecord,
    ProcessResultStorePort, ProcessStart, ProcessStatus, ProcessSubmissionLifecycle,
};
use crate::{
    ClaimedProcess, GetProcessInputRequest, JournalProcessExecutor, ProcessExecutorFailure,
    ProcessJournalStore, ProcessKind, ProcessRuntimePort, ProcessSupervisor,
    ProcessSupervisorConfig, ProcessSupervisorHandle, submit_capability_process,
};

/// Stage at which a background task failed to persist state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundFailureStage {
    /// Process snapshot lookup failed during the post-execution status probe.
    StoreLookup,
    /// Journal completion failed when promoting to `Completed`.
    StoreComplete,
    /// Journal failure recording failed when promoting to `Failed`.
    StoreFail,
    /// `ProcessResultStorePort::complete` failed.
    ResultStoreComplete,
    /// `ProcessResultStorePort::fail` failed.
    ResultStoreFail,
}

/// Failure observed inside a [`BackgroundProcessManager`] spawned task.
///
/// The detached task cannot return errors to the original `spawn` caller, so
/// any failure surfaces here for an attached error handler. If no handler is
/// configured, the error is dropped — see
/// [`BackgroundProcessManager::with_error_handler`].
#[derive(Debug)]
pub struct BackgroundFailure {
    pub scope: ResourceScope,
    pub process_id: ProcessId,
    pub stage: BackgroundFailureStage,
    pub error: ProcessError,
}

/// Callback invoked for each [`BackgroundFailure`] in the spawned task.
pub type BackgroundErrorHandler = dyn Fn(BackgroundFailure) + Send + Sync;

pub struct ProcessServices {
    process_runtime: Arc<dyn ProcessRuntimePort>,
    result_store: Arc<dyn ProcessResultStorePort>,
    process_store_type: (&'static str, TypeId),
    result_store_type: (&'static str, TypeId),
    cancellation_registry: Arc<ProcessCancellationRegistry>,
}

impl Clone for ProcessServices {
    fn clone(&self) -> Self {
        Self {
            process_runtime: Arc::clone(&self.process_runtime),
            result_store: Arc::clone(&self.result_store),
            process_store_type: self.process_store_type,
            result_store_type: self.result_store_type,
            cancellation_registry: Arc::clone(&self.cancellation_registry),
        }
    }
}

impl ProcessServices {
    pub fn new<S, R>(process_runtime: Arc<S>, result_store: Arc<R>) -> Self
    where
        S: ProcessRuntimePort + 'static,
        R: ProcessResultStorePort + 'static,
    {
        Self::from_parts(
            process_runtime,
            result_store,
            Arc::new(ProcessCancellationRegistry::new()),
        )
    }

    pub fn from_parts<S, R>(
        process_runtime: Arc<S>,
        result_store: Arc<R>,
        cancellation_registry: Arc<ProcessCancellationRegistry>,
    ) -> Self
    where
        S: ProcessRuntimePort + 'static,
        R: ProcessResultStorePort + 'static,
    {
        Self {
            process_runtime,
            result_store,
            process_store_type: (type_name::<S>(), TypeId::of::<S>()),
            result_store_type: (type_name::<R>(), TypeId::of::<R>()),
            cancellation_registry,
        }
    }

    pub fn process_runtime(&self) -> Arc<dyn ProcessRuntimePort> {
        Arc::clone(&self.process_runtime)
    }

    pub fn result_store(&self) -> Arc<dyn ProcessResultStorePort> {
        Arc::clone(&self.result_store)
    }

    pub fn process_store_type(&self) -> (&'static str, TypeId) {
        self.process_store_type
    }

    pub fn result_store_type(&self) -> (&'static str, TypeId) {
        self.result_store_type
    }

    pub fn cancellation_registry(&self) -> Arc<ProcessCancellationRegistry> {
        Arc::clone(&self.cancellation_registry)
    }

    pub fn host(&self) -> ProcessHost {
        ProcessHost::new(self.clone())
    }

    pub fn background_manager<E>(&self, executor: Arc<E>) -> BackgroundProcessManager
    where
        E: ProcessExecutor + 'static,
    {
        BackgroundProcessManager::new(self.clone(), executor).start_supervisor()
    }
    pub fn filesystem<F>(filesystem: Arc<ScopedFilesystem<F>>) -> Self
    where
        F: RootFilesystem + Send + Sync + 'static,
    {
        Self::new(
            Arc::new(ProcessJournalStore::new(Arc::clone(&filesystem))),
            Arc::new(ProcessResultStore::from_arc(filesystem)),
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ProcessServices {
    /// In-memory-backed process services for tests — the production
    /// [`filesystem`](Self::filesystem) store pair over one fresh
    /// `InMemoryBackend` `/processes` mount (arch-simplification §4.3).
    /// Replaces the deleted bespoke `InMemory*Store` pair; the two stores
    /// share one backend so externalized output (`output_ref`) reads back.
    pub fn in_memory() -> Self {
        Self::filesystem(crate::test_support::in_memory_backed_processes_filesystem())
    }
}

pub struct BackgroundProcessManager {
    process_services: ProcessServices,
    executor: Arc<dyn ProcessExecutor + 'static>,
    // arch-exempt: optional_arc, host-owned obligation handoff is absent from generic process executors, plan #4539
    submission_lifecycle: Option<Arc<dyn ProcessSubmissionLifecycle>>,
    // arch-exempt: optional_arc, detached failure observation is installed only by hosts that own cleanup policy, plan #4539
    error_handler: Option<Arc<BackgroundErrorHandler>>,
    supervisor: Mutex<Option<ProcessSupervisorHandle>>,
}

impl BackgroundProcessManager {
    pub fn new<E>(process_services: ProcessServices, executor: Arc<E>) -> Self
    where
        E: ProcessExecutor + 'static,
    {
        Self {
            process_services,
            executor,
            submission_lifecycle: None,
            error_handler: None,
            supervisor: Mutex::new(None),
        }
    }

    pub fn with_submission_lifecycle(
        mut self,
        lifecycle: Arc<dyn ProcessSubmissionLifecycle>,
    ) -> Self {
        self.submission_lifecycle = Some(lifecycle);
        self
    }

    /// Attach a callback for executor-side store/result-store failures.
    pub fn with_error_handler<F>(mut self, handler: F) -> Self
    where
        F: Fn(BackgroundFailure) + Send + Sync + 'static,
    {
        self.error_handler = Some(Arc::new(handler));
        self
    }

    /// Starts polling immediately so queued work resumes after restart.
    pub fn start_supervisor(self) -> Self {
        if let Err(error) = self.wake_notifier() {
            tracing::error!(%error, "capability process supervisor failed to start");
        }
        self
    }

    fn wake_notifier(&self) -> Result<Arc<crate::ProcessWakeNotifier>, ProcessError> {
        let mut supervisor =
            self.supervisor
                .lock()
                .map_err(|_| ProcessError::InvalidStoredRecord {
                    reason: "process supervisor mutex poisoned".to_string(),
                })?;
        if supervisor.is_none() {
            let runtime = self.process_services.process_runtime();
            let executor = Arc::new(BackgroundJournalExecutor {
                runtime: Arc::clone(&runtime),
                executor: Arc::clone(&self.executor),
                cancellation_registry: self.process_services.cancellation_registry(),
                result_store: self.process_services.result_store(),
                error_handler: self.error_handler.clone(),
            });
            *supervisor = Some(
                ProcessSupervisor::new(
                    runtime,
                    executor,
                    ProcessKind::CapabilityInvocation,
                    ProcessSupervisorConfig::default(),
                )
                .start(),
            );
        }
        supervisor
            .as_ref()
            .map(ProcessSupervisorHandle::wake_notifier)
            .ok_or_else(|| ProcessError::InvalidStoredRecord {
                reason: "process supervisor failed to start".to_string(),
            })
    }
}

#[async_trait]
impl ProcessManager for BackgroundProcessManager {
    /// Journal the request, then wake the shared process supervisor.
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        if let Some(lifecycle) = &self.submission_lifecycle {
            lifecycle.before_submit(&start).await?;
        }
        let runtime = self.process_services.process_runtime();
        let record = match submit_capability_process(runtime.as_ref(), start.clone()).await {
            Ok(record) => record,
            Err(error) => {
                if let Some(lifecycle) = &self.submission_lifecycle
                    && let Err(cleanup_error) = lifecycle.submit_failed(&start).await
                {
                    return Err(ProcessError::InvalidStoredRecord {
                        reason: format!(
                            "process submit failed: {error}; lifecycle cleanup failed: {cleanup_error}"
                        ),
                    });
                }
                return Err(error);
            }
        };
        if let Some(lifecycle) = &self.submission_lifecycle {
            lifecycle.submitted(&record).await?;
        }
        let cancellation_registry = self.process_services.cancellation_registry();
        cancellation_registry.register(&record.scope, record.process_id);
        let wake_result = self.wake_notifier().and_then(|notifier| {
            notifier
                .notify_scope(record.scope.clone())
                .map_err(|error| ProcessError::InvalidStoredRecord {
                    reason: error.to_string(),
                })
        });
        if wake_result.is_err() {
            cancellation_registry.unregister(&record.scope, record.process_id);
        }
        wake_result?;
        Ok(record)
    }
}

struct BackgroundJournalExecutor {
    runtime: Arc<dyn ProcessRuntimePort>,
    executor: Arc<dyn ProcessExecutor>,
    cancellation_registry: Arc<ProcessCancellationRegistry>,
    result_store: Arc<dyn ProcessResultStorePort>,
    // arch-exempt: optional_arc, copied from the manager's optional host-owned cleanup callback, plan #4539
    error_handler: Option<Arc<BackgroundErrorHandler>>,
}

impl BackgroundJournalExecutor {
    fn report(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        stage: BackgroundFailureStage,
        error: ProcessError,
    ) {
        if let Some(handler) = &self.error_handler {
            handler(BackgroundFailure {
                scope: scope.clone(),
                process_id,
                stage,
                error,
            });
        }
    }

    async fn execution_request(
        &self,
        claimed: &ClaimedProcess,
    ) -> Result<ProcessExecutionRequest, ProcessError> {
        let process_id = claimed.state.process_id;
        let scope = &claimed.state.scope;
        let record = capability_process_record(self.runtime.as_ref(), scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        let input = self
            .runtime
            .get_process_input(GetProcessInputRequest {
                process_id,
                scope: scope.clone(),
            })
            .await
            .map_err(crate::capability_process::map_process_journal_error)?
            .ok_or_else(|| ProcessError::InvalidStoredRecord {
                reason: format!("process {process_id} has no durable input"),
            })
            .and_then(|record| {
                serde_json::from_slice(record.payload.as_bytes())
                    .map_err(|error| ProcessError::Deserialization(error.to_string()))
            })?;
        let cancellation = self.cancellation_registry.register(scope, process_id);
        let resource_reservation = record
            .resource_reservation_id
            .map(|id| ResourceReservation {
                id,
                scope: record.scope.clone(),
                estimate: record.estimated_resources.clone(),
            });
        Ok(ProcessExecutionRequest {
            process_id,
            invocation_id: record.invocation_id,
            scope: record.scope,
            authenticated_actor_user_id: record.authenticated_actor_user_id,
            extension_id: record.extension_id,
            capability_id: record.capability_id,
            runtime: record.runtime,
            estimate: record.estimated_resources,
            mounts: record.mounts,
            resource_reservation,
            authorized_continuation: record.authorized_continuation,
            input,
            cancellation,
        })
    }

    async fn persist_outcome(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        outcome: Result<
            Result<crate::ProcessExecutionResult, crate::ProcessExecutionError>,
            Box<dyn std::any::Any + Send>,
        >,
    ) {
        let still_running =
            match capability_process_record(self.runtime.as_ref(), scope, process_id).await {
                Ok(Some(record)) => record.status == ProcessStatus::Running,
                Ok(None) => false,
                Err(error) => {
                    self.report(
                        scope,
                        process_id,
                        BackgroundFailureStage::StoreLookup,
                        error,
                    );
                    false
                }
            };
        if !still_running {
            return;
        }
        match outcome {
            Ok(Ok(result)) => {
                let persisted = match self
                    .result_store
                    .complete(scope, process_id, result.output)
                    .await
                {
                    Ok(_) => true,
                    Err(error) => {
                        self.report(
                            scope,
                            process_id,
                            BackgroundFailureStage::ResultStoreComplete,
                            error,
                        );
                        false
                    }
                };
                if persisted
                    && let Err(error) =
                        complete_capability_process(self.runtime.as_ref(), scope, process_id).await
                {
                    self.report(
                        scope,
                        process_id,
                        BackgroundFailureStage::StoreComplete,
                        error,
                    );
                }
            }
            Ok(Err(error)) => {
                self.persist_failure(scope, process_id, sanitize_error_kind(error.kind))
                    .await;
            }
            Err(_) => {
                self.persist_failure(scope, process_id, "runtime_panic".to_string())
                    .await;
            }
        }
    }

    async fn persist_failure(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) {
        let persisted = match self
            .result_store
            .fail(scope, process_id, error_kind.clone())
            .await
        {
            Ok(_) => true,
            Err(error) => {
                self.report(
                    scope,
                    process_id,
                    BackgroundFailureStage::ResultStoreFail,
                    error,
                );
                false
            }
        };
        if persisted
            && let Err(error) =
                fail_capability_process(self.runtime.as_ref(), scope, process_id, error_kind).await
        {
            self.report(scope, process_id, BackgroundFailureStage::StoreFail, error);
        }
    }
}

#[async_trait]
impl JournalProcessExecutor for BackgroundJournalExecutor {
    async fn execute_claimed_process(
        &self,
        claimed: ClaimedProcess,
    ) -> Result<(), ProcessExecutorFailure> {
        let scope = claimed.state.scope.clone();
        let process_id = claimed.state.process_id;
        let request = self
            .execution_request(&claimed)
            .await
            .map_err(|_| ProcessExecutorFailure::new("process_request_invalid"))?;
        let outcome = std::panic::AssertUnwindSafe(self.executor.execute(request))
            .catch_unwind()
            .await;
        self.persist_outcome(&scope, process_id, outcome).await;
        self.cancellation_registry.unregister(&scope, process_id);
        Ok(())
    }
}
