use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    BlockedReason, CapabilityActivityId, LoopExitMapping, ResolvedRunProfile, SanitizedFailure,
    TurnCheckpointId, TurnError, TurnId, TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId,
    TurnScope, TurnTimestamp,
    events::EventCursor,
    run_profile::{LoopCheckpointStateRef, LoopModelRouteSnapshot},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimRunRequest {
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
    pub scope_filter: Option<TurnScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimRunsRequest {
    pub runner_id: TurnRunnerId,
    pub scope_filter: Option<TurnScope>,
    pub max_runs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedTurnRun {
    pub state: TurnRunState,
    pub resolved_run_profile: ResolvedRunProfile,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverExpiredLeasesRequest {
    pub now: TurnTimestamp,
    pub scope_filter: Option<TurnScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverExpiredLeasesResponse {
    pub recovered: Vec<TurnRunState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordModelRouteSnapshotRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
    pub snapshot: LoopModelRouteSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRunRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
    pub checkpoint_id: TurnCheckpointId,
    pub state_ref: LoopCheckpointStateRef,
    pub reason: BlockedReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteRunRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailRunRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
    pub failure: SanitizedFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRunCompletionRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordRunnerFailureRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
    pub failure: SanitizedFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelinquishRunRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyValidatedLoopExitRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
    pub mapping: LoopExitMapping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnRunnerOutcome {
    Completed,
    Cancelled,
    Blocked {
        checkpoint_id: TurnCheckpointId,
        state_ref: LoopCheckpointStateRef,
        reason: BlockedReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocked_activity_id: Option<CapabilityActivityId>,
    },
    Failed {
        failure: SanitizedFailure,
    },
}

#[async_trait]
pub trait TurnRunTransitionPort: Send + Sync {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError>;

    async fn claim_next_runs(
        &self,
        request: ClaimRunsRequest,
    ) -> Result<Vec<ClaimedTurnRun>, TurnError> {
        let mut claimed = Vec::new();
        for _ in 0..request.max_runs {
            let next = self
                .claim_next_run(ClaimRunRequest {
                    runner_id: request.runner_id,
                    lease_token: TurnLeaseToken::new(),
                    scope_filter: request.scope_filter.clone(),
                })
                .await?;
            let Some(next) = next else {
                break;
            };
            claimed.push(next);
        }
        Ok(claimed)
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError>;

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError>;

    async fn latest_resumable_checkpoint(
        &self,
        _scope: &TurnScope,
        _turn_id: TurnId,
        _run_id: TurnRunId,
    ) -> Result<Option<TurnCheckpointId>, TurnError> {
        Ok(None)
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError>;

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError>;

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError>;

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError>;

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError>;

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.fail_run(FailRunRequest {
            run_id: request.run_id,
            runner_id: request.runner_id,
            lease_token: request.lease_token,
            failure: request.failure,
        })
        .await
    }

    /// Release the lease and re-queue the run so another worker can claim it.
    ///
    /// Use for transient worker-side events (`WorkerCancelled`, `HeartbeatStopped`) where
    /// the turn should be retried rather than permanently failed.
    /// If the run is already `CancelRequested`, the cancellation intent is honored and the
    /// run transitions to `Cancelled` instead of being re-queued.
    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        let _ = request;
        Err(TurnError::Unavailable {
            reason: "relinquish_run not implemented for this TurnRunTransitionPort".to_string(),
        })
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError>;
}
