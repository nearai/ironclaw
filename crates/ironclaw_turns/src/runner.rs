use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    BlockedReason, SanitizedFailure, TurnCheckpointId, TurnError, TurnLeaseToken, TurnRunId,
    TurnRunState, TurnRunnerId, TurnScope, events::EventCursor,
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
pub struct BlockRunRequest {
    pub run_id: TurnRunId,
    pub runner_id: TurnRunnerId,
    pub lease_token: TurnLeaseToken,
    pub checkpoint_id: TurnCheckpointId,
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
pub enum TurnRunnerOutcome {
    Completed,
    Blocked {
        checkpoint_id: TurnCheckpointId,
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

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError>;

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError>;

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError>;
}
