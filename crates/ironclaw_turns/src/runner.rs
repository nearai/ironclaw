use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    BlockedReason, LoopExitMapping, ResolvedRunProfile, SanitizedFailure, TurnCheckpointId,
    TurnCommittedEventObserver, TurnError, TurnEventKind, TurnEventSink, TurnLeaseToken,
    TurnLifecycleEvent, TurnRunId, TurnRunState, TurnRunnerId, TurnScope, TurnStatus,
    TurnTimestamp,
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
    sink: Option<Arc<dyn TurnEventSink>>,
    // arch-exempt: optional_arc, observer is genuinely optional for helper/contract tests that exercise the port without subagent integration; production wiring in `ironclaw_reborn::runtime` always supplies one via `with_required_observer`. Tracked under the subagent-spawn epic.
    required_observer: Option<Arc<dyn TurnCommittedEventObserver>>,
}

impl EventPublishingTurnRunTransitionPort {
    pub fn new(inner: Arc<dyn TurnRunTransitionPort>, sink: Arc<dyn TurnEventSink>) -> Self {
        Self {
            inner,
            sink: Some(sink),
            required_observer: None,
        }
    }

    pub fn new_optional_sink(
        inner: Arc<dyn TurnRunTransitionPort>,
        sink: Option<Arc<dyn TurnEventSink>>,
    ) -> Self {
        Self {
            inner,
            sink,
            required_observer: None,
        }
    }

    pub fn with_required_observer(mut self, observer: Arc<dyn TurnCommittedEventObserver>) -> Self {
        self.required_observer = Some(observer);
        self
    }

    async fn publish_state_event(
        &self,
        state: &TurnRunState,
        kind: TurnEventKind,
        sanitized_reason: Option<String>,
    ) -> Result<(), TurnError> {
        let blocked_gate = if kind == TurnEventKind::Blocked {
            state.gate_ref.clone().and_then(|gate_ref| {
                crate::events::TurnBlockedGateKind::from_status(state.status).map(|gate_kind| {
                    crate::events::TurnBlockedGateMetadata {
                        gate_ref,
                        gate_kind,
                    }
                })
            })
        } else {
            None
        };
        let required_observer = self
            .required_observer
            .as_ref()
            .filter(|observer| observer.observes_state(state));
        if let Some(observer) = required_observer {
            observer.observe_committed_state(state.clone()).await?;
        }
        if let Some(sink) = self.sink.as_ref() {
            let event = TurnLifecycleEvent {
                cursor: state.event_cursor,
                scope: state.scope.clone(),
                occurred_at: Some(Utc::now()),
                owner_user_id: state.actor.as_ref().map(|actor| actor.user_id.clone()),
                run_id: state.run_id,
                status: state.status,
                kind,
                blocked_gate,
                sanitized_reason,
            };
            if let Err(error) = sink.publish(event).await {
                tracing::debug!(error = %error, "turn transition event sink publish failed");
            }
        }
        Ok(())
    }

    fn event_kind_for_state(state: &TurnRunState) -> TurnEventKind {
        match state.status {
            TurnStatus::Running => TurnEventKind::RunnerClaimed,
            TurnStatus::BlockedApproval
            | TurnStatus::BlockedAuth
            | TurnStatus::BlockedResource
            | TurnStatus::BlockedDependentRun => TurnEventKind::Blocked,
            TurnStatus::Completed => TurnEventKind::Completed,
            TurnStatus::Cancelled => TurnEventKind::Cancelled,
            TurnStatus::Failed => TurnEventKind::Failed,
            TurnStatus::RecoveryRequired => TurnEventKind::RecoveryRequired,
            TurnStatus::Queued | TurnStatus::CancelRequested => TurnEventKind::RunnerHeartbeat,
        }
    }

    fn sanitized_reason_for_state(state: &TurnRunState) -> Option<String> {
        match state.status {
            TurnStatus::Failed | TurnStatus::RecoveryRequired => state
                .failure
                .as_ref()
                .map(|failure| failure.category().to_string()),
            _ => None,
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
        if let Some(claimed) = &claimed
            && let Err(error) = self
                .publish_state_event(&claimed.state, TurnEventKind::RunnerClaimed, None)
                .await
        {
            tracing::debug!(
                error = %error,
                run_id = %claimed.state.run_id,
                "turn transition observer failed after committed claim"
            );
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
            if let Err(error) = self
                .publish_state_event(
                    state,
                    TurnEventKind::RecoveryRequired,
                    Some("lease_expired".to_string()),
                )
                .await
            {
                tracing::debug!(
                    error = %error,
                    run_id = %state.run_id,
                    "turn transition observer failed after committed recovery"
                );
            }
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
        self.publish_state_event(&state, TurnEventKind::Blocked, None)
            .await?;
        Ok(state)
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.complete_run(request).await?;
        self.publish_state_event(&state, TurnEventKind::Completed, None)
            .await?;
        Ok(state)
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.cancel_run(request).await?;
        self.publish_state_event(&state, TurnEventKind::Cancelled, None)
            .await?;
        Ok(state)
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.fail_run(request).await?;
        self.publish_state_event(
            &state,
            TurnEventKind::Failed,
            Self::sanitized_reason_for_state(&state),
        )
        .await?;
        Ok(state)
    }

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.record_recovery_required(request).await?;
        self.publish_state_event(
            &state,
            TurnEventKind::RecoveryRequired,
            Self::sanitized_reason_for_state(&state),
        )
        .await?;
        Ok(state)
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.apply_validated_loop_exit(request).await?;
        self.publish_state_event(
            &state,
            Self::event_kind_for_state(&state),
            Self::sanitized_reason_for_state(&state),
        )
        .await?;
        Ok(state)
    }
}
