use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_turns::{
    TurnCheckpointId, TurnError, TurnEventKind, TurnEventSink, TurnId, TurnLifecycleEvent,
    TurnRunId, TurnRunState, TurnScope, TurnStatus,
    events::{TurnBlockedGateKind, TurnBlockedGateMetadata},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimRunsRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest,
        HeartbeatRequest, RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest,
        RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse, RelinquishRunRequest,
        TurnRunTransitionPort,
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
                        activity_id: state.blocked_activity_id,
                        credential_requirements: state.credential_requirements.clone(),
                    }
                })
            })
        } else {
            None
        };
        let retryable = (kind == TurnEventKind::Failed).then(|| state.checkpoint_id.is_some());
        let detail = (kind == TurnEventKind::Failed)
            .then(|| {
                state
                    .failure
                    .as_ref()
                    .and_then(|failure| failure.detail().map(str::to_string))
            })
            .flatten();
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
            retryable,
            detail,
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
            | TurnStatus::BlockedDependentRun
            | TurnStatus::BlockedExternalTool => TurnEventKind::Blocked,
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

    async fn claim_next_runs(
        &self,
        request: ClaimRunsRequest,
    ) -> Result<Vec<ClaimedTurnRun>, TurnError> {
        let claimed = self.inner.claim_next_runs(request).await?;
        for claimed_run in &claimed {
            self.publish_state_event_best_effort(
                &claimed_run.state,
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

    async fn latest_resumable_checkpoint(
        &self,
        scope: &TurnScope,
        turn_id: TurnId,
        run_id: TurnRunId,
    ) -> Result<Option<TurnCheckpointId>, TurnError> {
        self.inner
            .latest_resumable_checkpoint(scope, turn_id, run_id)
            .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use ironclaw_host_api::{AgentId, TenantId, ThreadId};

    struct CheckpointProbeTransitionPort {
        checkpoint_id: TurnCheckpointId,
        calls: Mutex<Vec<(TurnScope, TurnId, TurnRunId)>>,
    }

    impl CheckpointProbeTransitionPort {
        fn new(checkpoint_id: TurnCheckpointId) -> Self {
            Self {
                checkpoint_id,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl TurnRunTransitionPort for CheckpointProbeTransitionPort {
        async fn claim_next_run(
            &self,
            _request: ClaimRunRequest,
        ) -> Result<Option<ClaimedTurnRun>, TurnError> {
            panic!("latest_resumable_checkpoint test must not claim runs")
        }

        async fn heartbeat(
            &self,
            _request: HeartbeatRequest,
        ) -> Result<ironclaw_turns::EventCursor, TurnError> {
            panic!("latest_resumable_checkpoint test must not heartbeat")
        }

        async fn recover_expired_leases(
            &self,
            _request: RecoverExpiredLeasesRequest,
        ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
            panic!("latest_resumable_checkpoint test must not recover leases")
        }

        async fn latest_resumable_checkpoint(
            &self,
            scope: &TurnScope,
            turn_id: TurnId,
            run_id: TurnRunId,
        ) -> Result<Option<TurnCheckpointId>, TurnError> {
            self.calls
                .lock()
                .expect("calls mutex")
                .push((scope.clone(), turn_id, run_id));
            Ok(Some(self.checkpoint_id))
        }

        async fn record_model_route_snapshot(
            &self,
            _request: RecordModelRouteSnapshotRequest,
        ) -> Result<TurnRunState, TurnError> {
            panic!("latest_resumable_checkpoint test must not record model route snapshots")
        }

        async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
            panic!("latest_resumable_checkpoint test must not block runs")
        }

        async fn complete_run(
            &self,
            _request: CompleteRunRequest,
        ) -> Result<TurnRunState, TurnError> {
            panic!("latest_resumable_checkpoint test must not complete runs")
        }

        async fn cancel_run(
            &self,
            _request: CancelRunCompletionRequest,
        ) -> Result<TurnRunState, TurnError> {
            panic!("latest_resumable_checkpoint test must not cancel runs")
        }

        async fn fail_run(&self, _request: FailRunRequest) -> Result<TurnRunState, TurnError> {
            panic!("latest_resumable_checkpoint test must not fail runs")
        }

        async fn apply_validated_loop_exit(
            &self,
            _request: ApplyValidatedLoopExitRequest,
        ) -> Result<TurnRunState, TurnError> {
            panic!("latest_resumable_checkpoint test must not apply loop exits")
        }
    }

    struct NoopEventSink;

    #[async_trait]
    impl TurnEventSink for NoopEventSink {
        async fn publish(&self, _event: TurnLifecycleEvent) -> Result<(), TurnError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn event_publishing_transition_port_forwards_latest_resumable_checkpoint() {
        let checkpoint_id = TurnCheckpointId::new();
        let inner = Arc::new(CheckpointProbeTransitionPort::new(checkpoint_id));
        let port =
            EventPublishingTurnRunTransitionPort::new(inner.clone(), Arc::new(NoopEventSink));
        let scope = TurnScope::new(
            TenantId::new("tenant-forward").expect("tenant id"),
            Some(AgentId::new("agent-forward").expect("agent id")),
            None,
            ThreadId::new("thread-forward").expect("thread id"),
        );
        let turn_id = TurnId::new();
        let run_id = TurnRunId::new();

        let forwarded = port
            .latest_resumable_checkpoint(&scope, turn_id, run_id)
            .await
            .expect("forwarded checkpoint lookup");

        assert_eq!(forwarded, Some(checkpoint_id));
        assert_eq!(
            inner.calls.lock().expect("calls mutex").as_slice(),
            &[(scope, turn_id, run_id)]
        );
    }
}
