//! Composition + spawn surface for process services.
//!
//! - [`ProcessServices`] bundles a process store, a result store, and a
//!   shared [`ProcessCancellationRegistry`] so the host and background manager
//!   stay coordinated through a single graph.
//! - [`BackgroundProcessManager`] is the production [`ProcessManager`] that
//!   spawns a detached tokio task per `spawn` and writes terminal status +
//!   result records when the executor finishes (or panics).
//!
//! Background-task failures (store/result-store errors during the spawned
//! task) can be observed by attaching a
//! [`with_error_handler`](BackgroundProcessManager::with_error_handler)
//! callback. Without a handler, those errors are silently dropped.

use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{ProcessId, ResourceEstimate, ResourceScope};

use crate::cancellation::ProcessCancellationRegistry;
use crate::filesystem_store::{FilesystemProcessResultStore, FilesystemProcessStore};
use crate::host::ProcessHost;
use crate::memory_store::{InMemoryProcessResultStore, InMemoryProcessStore};
use crate::types::{
    ProcessError, ProcessExecutionRequest, ProcessExecutor, ProcessManager, ProcessRecord,
    ProcessResultStore, ProcessStart, ProcessStatus, ProcessStore,
};

/// Stage at which a background task failed to persist state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundFailureStage {
    /// `ProcessStore::get` failed during the post-execution status probe.
    StoreLookup,
    /// `ProcessStore::complete` failed when promoting to `Completed`.
    StoreComplete,
    /// `ProcessStore::fail` failed when promoting to `Failed`.
    StoreFail,
    /// `ProcessResultStore::complete` failed.
    ResultStoreComplete,
    /// `ProcessResultStore::fail` failed.
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
    error_handler: Option<Arc<BackgroundErrorHandler>>,
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
            error_handler: None,
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

    /// Attach a callback for store/result-store failures that occur inside
    /// the spawned task. Without a handler, those failures are silently
    /// dropped — they cannot be propagated to the original `spawn` caller
    /// because the task is detached.
    pub fn with_error_handler<F>(mut self, handler: F) -> Self
    where
        F: Fn(BackgroundFailure) + Send + Sync + 'static,
    {
        self.error_handler = Some(Arc::new(handler));
        self
    }
}

#[async_trait]
impl ProcessManager for BackgroundProcessManager {
    /// Persist `Running`, then spawn a detached tokio task that drives the
    /// executor and writes the terminal record(s).
    ///
    /// Write order on success/failure is **result store first, status store
    /// second**: result records are persisted before the lifecycle status
    /// flips to a terminal value. This makes the contract "if a process is
    /// observed at a terminal status, its result record is already on disk"
    /// — `ProcessHost::await_result` relies on this ordering for its
    /// notification path.
    ///
    /// The spawned task is detached: if the tokio runtime is shut down or
    /// the process exits before the executor finishes, the in-flight work
    /// is orphaned and the record will remain stuck at `Running`. Callers
    /// that need crash-safety should perform startup reconciliation by
    /// listing `Running` records on launch and deciding policy (mark
    /// failed, retry, etc.). TODO: provide a built-in reconciler.
    async fn spawn(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let input = start.input.clone();
        let record = self.store.start(start).await?;
        let store = Arc::clone(&self.store);
        let executor = Arc::clone(&self.executor);
        let scope = record.scope.clone();
        let process_id = record.process_id;
        let cancellation_registry = self.cancellation_registry.clone();
        let result_store = self.result_store.clone();
        let error_handler = self.error_handler.clone();
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
            let report = |stage: BackgroundFailureStage, error: ProcessError| {
                if let Some(handler) = &error_handler {
                    handler(BackgroundFailure {
                        scope: scope.clone(),
                        process_id,
                        stage,
                        error,
                    });
                }
            };
            let outcome = std::panic::AssertUnwindSafe(executor.execute(request))
                .catch_unwind()
                .await;

            // Skip writes if the process was already terminalized externally
            // (typically by `ProcessHost::kill`). Without this guard, the
            // result-store-first ordering below would overwrite a kill
            // record with the executor's late-arriving result.
            let still_running = match store.get(&scope, process_id).await {
                Ok(Some(record)) => record.status == ProcessStatus::Running,
                Ok(None) => false,
                Err(error) => {
                    report(BackgroundFailureStage::StoreLookup, error);
                    false
                }
            };

            if still_running {
                match outcome {
                    Ok(Ok(result)) => {
                        let result_persisted = if let Some(result_store) = &result_store {
                            match result_store
                                .complete(&scope, process_id, result.output)
                                .await
                            {
                                Ok(_) => true,
                                Err(error) => {
                                    report(BackgroundFailureStage::ResultStoreComplete, error);
                                    false
                                }
                            }
                        } else {
                            true
                        };
                        if result_persisted
                            && let Err(error) = store.complete(&scope, process_id).await
                        {
                            report(BackgroundFailureStage::StoreComplete, error);
                        }
                    }
                    Ok(Err(error)) => {
                        let error_kind = error.kind;
                        let result_persisted = if let Some(result_store) = &result_store {
                            match result_store
                                .fail(&scope, process_id, error_kind.clone())
                                .await
                            {
                                Ok(_) => true,
                                Err(error) => {
                                    report(BackgroundFailureStage::ResultStoreFail, error);
                                    false
                                }
                            }
                        } else {
                            true
                        };
                        if result_persisted
                            && let Err(error) = store.fail(&scope, process_id, error_kind).await
                        {
                            report(BackgroundFailureStage::StoreFail, error);
                        }
                    }
                    Err(_) => {
                        let panic_kind = "runtime_panic".to_string();
                        let result_persisted = if let Some(result_store) = &result_store {
                            match result_store
                                .fail(&scope, process_id, panic_kind.clone())
                                .await
                            {
                                Ok(_) => true,
                                Err(error) => {
                                    report(BackgroundFailureStage::ResultStoreFail, error);
                                    false
                                }
                            }
                        } else {
                            true
                        };
                        if result_persisted
                            && let Err(error) = store.fail(&scope, process_id, panic_kind).await
                        {
                            report(BackgroundFailureStage::StoreFail, error);
                        }
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
