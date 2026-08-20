use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_processes::{
    CancelProcessRequest, ClaimProcessesRequest, ClaimedProcess, FailProcessRequest,
    GetProcessCheckpointRequest, GetProcessInputRequest, GetProcessSnapshotRequest,
    JournaledProcessSnapshot, KillProcessRequest, ProcessCheckpointPort, ProcessCheckpointRecord,
    ProcessControlPort, ProcessControlResult, ProcessGateQuery, ProcessGateQuerySource,
    ProcessGateRecord, ProcessInputPort, ProcessInputRecord, ProcessJournalCursor,
    ProcessJournalPage, ProcessJournalSource, ProcessJournalStoreError, ProcessLeaseRequest,
    ProcessLifecycleLookupBatchRequest, ProcessLifecycleLookupResult, ProcessLifecycleLookupSource,
    ProcessRuntimePort, ProcessSnapshotSource, ProcessStateTransitionRequest,
    ProcessSubmissionPort, ProcessTransitionPort, ProcessTreePort, ProcessTreeReservation,
    PruneReleasedProcessRequest, RecordProcessCheckpointRequest,
    RecoverExpiredProcessLeasesRequest, RecoverExpiredProcessLeasesResponse,
    ReleaseProcessTreeRequest, ReserveProcessTreeRequest, ResumeProcessRequest, StopProcessRequest,
    SubmitProcessAtEdgeRequest, SubmitProcessRequest, SubmitProcessWithCheckpointRequest,
    SuspendProcessRequest,
};

use crate::TurnError;

#[derive(Clone)]
pub struct ProcessJournalStoreTurnAdapter {
    runtime: Arc<dyn ProcessRuntimePort>,
}
impl ProcessJournalStoreTurnAdapter {
    pub fn new(runtime: Arc<dyn ProcessRuntimePort>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl ProcessSnapshotSource for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn process_snapshots(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<JournaledProcessSnapshot>, Self::Error> {
        self.runtime
            .process_snapshots(scope)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn recent_agent_turn_snapshots(
        &self,
        scope: &ResourceScope,
        limit: u32,
    ) -> Result<Vec<JournaledProcessSnapshot>, Self::Error> {
        self.runtime
            .recent_agent_turn_snapshots(scope, limit)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }
}

#[async_trait]
impl ProcessTreePort for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn child_processes(
        &self,
        scope: &ResourceScope,
        parent_process_id: ironclaw_host_api::ids::ProcessId,
    ) -> Result<Vec<JournaledProcessSnapshot>, Self::Error> {
        self.runtime
            .child_processes(scope, parent_process_id)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn reserve_process_tree(
        &self,
        request: ReserveProcessTreeRequest,
    ) -> Result<ProcessTreeReservation, Self::Error> {
        self.runtime
            .reserve_process_tree(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn release_process_tree(
        &self,
        request: ReleaseProcessTreeRequest,
    ) -> Result<(), Self::Error> {
        self.runtime
            .release_process_tree(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn prune_released_process(
        &self,
        request: PruneReleasedProcessRequest,
    ) -> Result<(), Self::Error> {
        self.runtime
            .prune_released_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }
}

#[async_trait]
impl ProcessCheckpointPort for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn record_process_checkpoint(
        &self,
        request: RecordProcessCheckpointRequest,
    ) -> Result<ProcessCheckpointRecord, Self::Error> {
        self.runtime
            .record_process_checkpoint(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn get_process_checkpoint(
        &self,
        request: GetProcessCheckpointRequest,
    ) -> Result<Option<ProcessCheckpointRecord>, Self::Error> {
        self.runtime
            .get_process_checkpoint(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }
}

#[async_trait]
impl ProcessInputPort for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn get_process_input(
        &self,
        request: GetProcessInputRequest,
    ) -> Result<Option<ProcessInputRecord>, Self::Error> {
        self.runtime
            .get_process_input(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }
}

#[async_trait]
impl ProcessSubmissionPort for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn submit_process(
        &self,
        request: SubmitProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.runtime
            .submit_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn submit_process_at_edge(
        &self,
        request: SubmitProcessAtEdgeRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.runtime
            .submit_process_at_edge(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn submit_process_with_checkpoint(
        &self,
        request: SubmitProcessWithCheckpointRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.runtime
            .submit_process_with_checkpoint(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }
}

#[async_trait]
impl ProcessControlPort for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn resume_process(
        &self,
        request: ResumeProcessRequest,
    ) -> Result<ProcessControlResult, Self::Error> {
        self.runtime
            .resume_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn stop_process(
        &self,
        request: StopProcessRequest,
    ) -> Result<ProcessControlResult, Self::Error> {
        self.runtime
            .stop_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn request_cancel_process(
        &self,
        request: CancelProcessRequest,
    ) -> Result<ProcessControlResult, Self::Error> {
        self.runtime
            .request_cancel_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn kill_process(
        &self,
        request: KillProcessRequest,
    ) -> Result<ProcessControlResult, Self::Error> {
        self.runtime
            .kill_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }
}

#[async_trait]
impl ProcessTransitionPort for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn claim_next_processes(
        &self,
        request: ClaimProcessesRequest,
    ) -> Result<Vec<ClaimedProcess>, Self::Error> {
        self.runtime
            .claim_next_processes(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn heartbeat_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<ProcessJournalCursor, Self::Error> {
        self.runtime
            .heartbeat_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn recover_expired_process_leases(
        &self,
        request: RecoverExpiredProcessLeasesRequest,
    ) -> Result<RecoverExpiredProcessLeasesResponse, Self::Error> {
        self.runtime
            .recover_expired_process_leases(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn suspend_process(
        &self,
        request: SuspendProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.runtime
            .suspend_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn complete_process(
        &self,
        request: ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.runtime
            .complete_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn cancel_process(
        &self,
        request: ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.runtime
            .cancel_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn fail_process(
        &self,
        request: FailProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.runtime
            .fail_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn relinquish_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.runtime
            .relinquish_process(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }
}

#[async_trait]
impl ProcessJournalSource for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn get_process_snapshot(
        &self,
        request: GetProcessSnapshotRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.runtime
            .get_process_snapshot(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn read_process_journal_after(
        &self,
        scope: &ResourceScope,
        owner_user_id: Option<&ironclaw_host_api::ids::UserId>,
        after: Option<ProcessJournalCursor>,
        limit: usize,
    ) -> Result<ProcessJournalPage, Self::Error> {
        self.runtime
            .read_process_journal_after(scope, owner_user_id, after, limit)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }

    async fn read_process_journal_log_after(
        &self,
        after: Option<ProcessJournalCursor>,
        limit: usize,
    ) -> Result<ProcessJournalPage, Self::Error> {
        self.runtime
            .read_process_journal_log_after(after, limit)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }
}

#[async_trait]
impl ProcessLifecycleLookupSource for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn process_lifecycle_states(
        &self,
        request: ProcessLifecycleLookupBatchRequest,
    ) -> Vec<Result<ProcessLifecycleLookupResult, Self::Error>> {
        self.runtime
            .process_lifecycle_states(request)
            .await
            .into_iter()
            .map(|result| result.map_err(turn_error_from_process_journal_store_error))
            .collect()
    }
}

#[async_trait]
impl ProcessGateQuerySource for ProcessJournalStoreTurnAdapter {
    type Error = TurnError;

    async fn query_process_gates(
        &self,
        request: ProcessGateQuery,
    ) -> Result<Vec<ProcessGateRecord>, Self::Error> {
        self.runtime
            .query_process_gates(request)
            .await
            .map_err(turn_error_from_process_journal_store_error)
    }
}

pub fn turn_error_from_process_journal_store_error(error: ProcessJournalStoreError) -> TurnError {
    match error {
        ProcessJournalStoreError::UnknownProcess { .. } => TurnError::ScopeNotFound,
        ProcessJournalStoreError::ActiveProcessConflict {
            process_id,
            status,
            suspension,
            cursor,
            ..
        } => TurnError::ThreadBusy(crate::ThreadBusy {
            active_run_id: crate::TurnRunId::from_uuid(process_id.as_uuid()),
            status: crate::process_projection::turn_status_from_process_status(
                status,
                suspension.as_deref(),
            )
            .unwrap_or(crate::TurnStatus::RecoveryRequired),
            event_cursor: crate::EventCursor(cursor.0),
        }),
        ProcessJournalStoreError::ProcessAlreadyExists { .. }
        | ProcessJournalStoreError::StaleSnapshot { .. } => TurnError::Conflict {
            reason: error.to_string(),
        },
        ProcessJournalStoreError::InvalidTransition { from, to, .. } => TurnError::InvalidRequest {
            reason: format!("invalid process journal transition from {from:?} to {to:?}"),
        },
        ProcessJournalStoreError::UnauthorizedScope => TurnError::Unauthorized,
        ProcessJournalStoreError::InvalidRequest(reason) => TurnError::InvalidRequest { reason },
        ProcessJournalStoreError::ProcessTreeCapacityExceeded { cap } => {
            TurnError::capacity_exceeded(
                crate::TurnCapacityResource::SpawnTreeDescendants,
                cap.into(),
            )
        }
        // A group commit that rolled back is a transient storage-side failure
        // from the caller's perspective: nothing committed, and re-issuing the
        // command is the correct response.
        ProcessJournalStoreError::GroupCommitFailed { .. }
        | ProcessJournalStoreError::InvalidLease { .. }
        | ProcessJournalStoreError::InvalidPath(_)
        | ProcessJournalStoreError::Filesystem(_)
        | ProcessJournalStoreError::Serialization(_)
        | ProcessJournalStoreError::Deserialization(_)
        | ProcessJournalStoreError::Observer(_)
        | ProcessJournalStoreError::MigrationRequired => TurnError::Unavailable {
            reason: error.to_string(),
        },
    }
}
