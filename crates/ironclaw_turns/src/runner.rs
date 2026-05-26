use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    BlockedReason, LoopExitMapping, ResolvedRunProfile, SanitizedFailure, TurnCheckpointId,
    TurnError, TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId, TurnScope, TurnTimestamp,
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
pub struct RecordRecoveryRequiredRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
    pub failure: SanitizedFailure,
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

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError>;

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError>;

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

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError>;

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError>;
}
