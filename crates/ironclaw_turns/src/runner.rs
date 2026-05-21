use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    BlockedReason, LoopExitMapping, ResolvedRunProfile, SanitizedFailure, TurnCheckpointId,
    TurnError, TurnEventProjectionSource, TurnEventSink, TurnLeaseToken, TurnRunId, TurnRunState,
    TurnRunnerId, TurnScope, TurnTimestamp,
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

pub struct EventPublishingTurnRunTransitionPort {
    inner: Arc<dyn TurnRunTransitionPort>,
    source: Arc<dyn TurnEventProjectionSource>,
    sink: Arc<dyn TurnEventSink>,
}

impl EventPublishingTurnRunTransitionPort {
    pub fn new(
        inner: Arc<dyn TurnRunTransitionPort>,
        source: Arc<dyn TurnEventProjectionSource>,
        sink: Arc<dyn TurnEventSink>,
    ) -> Self {
        Self {
            inner,
            source,
            sink,
        }
    }

    async fn publish_state_event_best_effort(&self, state: &TurnRunState) {
        let after = EventCursor(state.event_cursor.0.saturating_sub(1));
        let page = match self
            .source
            .read_turn_events_after(&state.scope, Some(after), 8)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::debug!(error = %error, "turn transition event source read failed");
                return;
            }
        };
        let Some(event) = page
            .entries
            .into_iter()
            .find(|event| event.run_id == state.run_id && event.cursor == state.event_cursor)
        else {
            tracing::debug!(
                run_id = %state.run_id,
                cursor = state.event_cursor.0,
                "turn transition event not found after committed state"
            );
            return;
        };
        if let Err(error) = self.sink.publish(event).await {
            tracing::debug!(error = %error, "turn transition event sink publish failed");
        }
    }
}

#[async_trait]
impl TurnRunTransitionPort for EventPublishingTurnRunTransitionPort {
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let claimed = self.inner.claim_next_run(request).await?;
        if let Some(claimed) = &claimed {
            self.publish_state_event_best_effort(&claimed.state).await;
        }
        Ok(claimed)
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        self.inner.heartbeat(request).await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        let response = self.inner.recover_expired_leases(request).await?;
        for state in &response.recovered {
            self.publish_state_event_best_effort(state).await;
        }
        Ok(response)
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.inner.record_model_route_snapshot(request).await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.block_run(request).await?;
        self.publish_state_event_best_effort(&state).await;
        Ok(state)
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.complete_run(request).await?;
        self.publish_state_event_best_effort(&state).await;
        Ok(state)
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.cancel_run(request).await?;
        self.publish_state_event_best_effort(&state).await;
        Ok(state)
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.fail_run(request).await?;
        self.publish_state_event_best_effort(&state).await;
        Ok(state)
    }

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.record_recovery_required(request).await?;
        self.publish_state_event_best_effort(&state).await;
        Ok(state)
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.apply_validated_loop_exit(request).await?;
        self.publish_state_event_best_effort(&state).await;
        Ok(state)
    }
}
