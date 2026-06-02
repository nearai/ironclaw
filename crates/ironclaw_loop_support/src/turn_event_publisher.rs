use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_turns::{
    TurnError, TurnEventKind, TurnEventSink, TurnLifecycleEvent, TurnRunState, TurnStatus,
    events::{TurnBlockedGateKind, TurnBlockedGateMetadata},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest, RecoverExpiredLeasesRequest,
        RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
    },
};

pub struct EventPublishingTurnRunTransitionPort {
    inner: Arc<dyn TurnRunTransitionPort>,
    sink: Arc<dyn TurnEventSink>,
}

impl EventPublishingTurnRunTransitionPort {
    pub fn new(inner: Arc<dyn TurnRunTransitionPort>, sink: Arc<dyn TurnEventSink>) -> Self {
        Self { inner, sink }
    }

    async fn publish_state_event_best_effort(
        &self,
        state: &TurnRunState,
        kind: TurnEventKind,
        sanitized_reason: Option<String>,
    ) {
        let blocked_gate = if kind == TurnEventKind::Blocked {
            state.gate_ref.clone().and_then(|gate_ref| {
                TurnBlockedGateKind::from_status(state.status).map(|gate_kind| {
                    TurnBlockedGateMetadata {
                        gate_ref,
                        gate_kind,
                        credential_requirements: state.credential_requirements.clone(),
                    }
                })
            })
        } else {
            None
        };
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
        if let Err(error) = self.sink.publish(event).await {
            tracing::debug!(error = %error, "turn transition event sink publish failed");
        }
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
        if let Some(claimed) = &claimed {
            self.publish_state_event_best_effort(
                &claimed.state,
                TurnEventKind::RunnerClaimed,
                None,
            )
            .await;
        }
        Ok(claimed)
    }

    async fn heartbeat(
        &self,
        request: HeartbeatRequest,
    ) -> Result<ironclaw_turns::EventCursor, TurnError> {
        self.inner.heartbeat(request).await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        let response = self.inner.recover_expired_leases(request).await?;
        for state in &response.recovered {
            self.publish_state_event_best_effort(
                state,
                Self::event_kind_for_state(state),
                Self::sanitized_reason_for_state(state),
            )
            .await;
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
        self.publish_state_event_best_effort(&state, TurnEventKind::Blocked, None)
            .await;
        Ok(state)
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.complete_run(request).await?;
        self.publish_state_event_best_effort(&state, TurnEventKind::Completed, None)
            .await;
        Ok(state)
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.cancel_run(request).await?;
        self.publish_state_event_best_effort(&state, TurnEventKind::Cancelled, None)
            .await;
        Ok(state)
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        let state = self.inner.fail_run(request).await?;
        self.publish_state_event_best_effort(
            &state,
            TurnEventKind::Failed,
            Self::sanitized_reason_for_state(&state),
        )
        .await;
        Ok(state)
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.record_runner_failure(request).await?;
        self.publish_state_event_best_effort(
            &state,
            Self::event_kind_for_state(&state),
            Self::sanitized_reason_for_state(&state),
        )
        .await;
        Ok(state)
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.relinquish_run(request).await?;
        self.publish_state_event_best_effort(
            &state,
            Self::event_kind_for_state(&state),
            Self::sanitized_reason_for_state(&state),
        )
        .await;
        Ok(state)
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        let state = self.inner.apply_validated_loop_exit(request).await?;
        self.publish_state_event_best_effort(
            &state,
            Self::event_kind_for_state(&state),
            Self::sanitized_reason_for_state(&state),
        )
        .await;
        Ok(state)
    }
}
