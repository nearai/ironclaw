//! Host-facing process query API.
//!
//! [`ProcessHost`] wraps a [`ProcessStore`](crate::types::ProcessStore) and an
//! optional [`ProcessResultStore`](crate::types::ProcessResultStore) and
//! [`ProcessCancellationRegistry`](crate::cancellation::ProcessCancellationRegistry).
//! It is the read/poll/await/cancel surface used by host runtimes; spawning
//! processes lives in [`crate::services`].

use std::{fmt, sync::Arc};

use ironclaw_host_api::{ProcessId, ResourceScope};
use serde_json::Value;
use tokio::time::{Duration, sleep};

use crate::cancellation::ProcessCancellationRegistry;
use crate::types::{
    ProcessError, ProcessExit, ProcessRecord, ProcessResultRecord, ProcessResultStore,
    ProcessStatus, ProcessStore,
};

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

    pub fn with_result_store_dyn(mut self, store: Arc<dyn ProcessResultStore>) -> Self {
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
        match self.store.kill(scope, process_id).await {
            Ok(record) => {
                self.record_kill_side_effects(&record).await?;
                Ok(record)
            }
            Err(error @ ProcessError::InvalidTransition { .. }) => Err(error),
            Err(error) => {
                if let Ok(Some(record)) = self.store.get(scope, process_id).await
                    && record.status == ProcessStatus::Killed
                {
                    self.record_kill_side_effects(&record).await?;
                }
                Err(error)
            }
        }
    }

    async fn record_kill_side_effects(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        if let Some(registry) = &self.cancellation_registry {
            registry.cancel(&record.scope, record.process_id);
        }
        if let Some(result_store) = &self.result_store {
            result_store.kill(&record.scope, record.process_id).await?;
        }
        Ok(())
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
