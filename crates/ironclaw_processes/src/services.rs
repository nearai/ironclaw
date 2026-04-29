//! Composition + spawn surface for process services.
//!
//! - [`ProcessServices`] bundles a process store, a result store, and a
//!   shared [`ProcessCancellationRegistry`] so the host and background manager
//!   stay coordinated through a single graph.
//! - [`BackgroundProcessManager`] is the production [`ProcessManager`] that
//!   spawns a detached tokio task per `spawn` and writes terminal status +
//!   result records when the executor finishes (or panics).
//!
//! Any `T: ProcessStore` also satisfies [`ProcessManager`] via blanket
//! `spawn = start` for ergonomics in tests/composition.

use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::ResourceEstimate;

use crate::cancellation::ProcessCancellationRegistry;
use crate::filesystem_store::{FilesystemProcessResultStore, FilesystemProcessStore};
use crate::host::ProcessHost;
use crate::memory_store::{InMemoryProcessResultStore, InMemoryProcessStore};
use crate::types::{
    ProcessError, ProcessExecutionRequest, ProcessExecutor, ProcessManager, ProcessRecord,
    ProcessResultStore, ProcessStart, ProcessStore,
};

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
