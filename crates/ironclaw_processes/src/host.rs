//! Host-facing process query API.
//!
//! [`ProcessHost`] wraps the unified [`ProcessServices`](crate::ProcessServices).
//! It is the read/poll/await/cancel surface used by host runtimes; spawning
//! processes lives in [`crate::services`].

use std::{fmt, sync::Arc};

use ironclaw_host_api::{ids::ProcessId, resource::ResourceScope};
use serde_json::Value;
use tokio::time::{Duration, sleep};

use crate::capability_process::{map_process_journal_error, process_record_from_snapshot};
use crate::services::ProcessServices;
use crate::types::{ProcessError, ProcessExit, ProcessRecord, ProcessResultRecord, ProcessStatus};
use crate::{GetProcessSnapshotRequest, KillProcessRequest, ProcessRuntimePort};

/// Host-facing lifecycle API over process current state.
pub struct ProcessHost {
    process_services: ProcessServices,
    poll_interval: Duration,
}

impl ProcessHost {
    pub(crate) fn new(process_services: ProcessServices) -> Self {
        Self {
            process_services,
            poll_interval: Duration::from_millis(10),
        }
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    async fn process_record(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        match self
            .process_services
            .process_runtime()
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: scope.clone(),
                process_id,
            })
            .await
        {
            Ok(snapshot) => process_record_from_snapshot(snapshot).map(Some),
            Err(error) => match map_process_journal_error(error) {
                ProcessError::UnknownProcess { .. } => Ok(None),
                error => Err(error),
            },
        }
    }

    pub async fn status(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        self.process_record(scope, process_id).await
    }

    pub async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        match self
            .process_services
            .process_runtime()
            .kill_process(KillProcessRequest {
                scope: scope.clone(),
                process_id,
                operation_id: None,
                reason: None,
            })
            .await
            .map_err(map_process_journal_error)
            .and_then(|result| process_record_from_snapshot(result.state))
        {
            Ok(record) => {
                self.record_kill_side_effects(&record).await?;
                Ok(record)
            }
            Err(error @ ProcessError::InvalidTransition { .. }) => {
                if let Ok(Some(record)) = self.process_record(scope, process_id).await
                    && record.status == ProcessStatus::Killed
                {
                    self.record_kill_side_effects(&record).await?;
                    return Ok(record);
                }
                Err(error)
            }
            Err(error) => {
                if let Ok(Some(record)) = self.process_record(scope, process_id).await
                    && record.status == ProcessStatus::Killed
                {
                    self.record_kill_side_effects(&record).await?;
                }
                Err(error)
            }
        }
    }

    async fn record_kill_side_effects(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        self.process_services
            .cancellation_registry()
            .cancel(&record.scope, record.process_id);
        self.process_services
            .result_store()
            .kill(&record.scope, record.process_id)
            .await?;
        Ok(())
    }

    pub async fn result(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        self.process_services
            .result_store()
            .get(scope, process_id)
            .await
    }

    pub async fn output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<Value>, ProcessError> {
        self.process_services
            .result_store()
            .output(scope, process_id)
            .await
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
                .process_record(scope, process_id)
                .await?
                .ok_or(ProcessError::UnknownProcess { process_id })?;
            if record.status.is_terminal() {
                if terminal_without_result_seen {
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
                .process_record(scope, process_id)
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
    ) -> Result<ProcessSubscription, ProcessError> {
        let initial_record = self
            .process_record(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        Ok(ProcessSubscription {
            runtime: self.process_services.process_runtime(),
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
pub struct ProcessSubscription {
    runtime: Arc<dyn ProcessRuntimePort>,
    scope: ResourceScope,
    process_id: ProcessId,
    poll_interval: Duration,
    last_status: Option<ProcessStatus>,
    pending_initial: Option<ProcessRecord>,
    finished: bool,
}

impl fmt::Debug for ProcessSubscription {
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

impl ProcessSubscription {
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
            let record = match self
                .runtime
                .get_process_snapshot(GetProcessSnapshotRequest {
                    scope: self.scope.clone(),
                    process_id: self.process_id,
                })
                .await
            {
                Ok(snapshot) => process_record_from_snapshot(snapshot)?,
                Err(error) => return Err(map_process_journal_error(error)),
            };
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
