//! Loop lifecycle helpers: checkpointing, stop-outcome handling, gate
//! processing, and no-progress detection.

use ironclaw_turns::{
    LoopFailureKind, LoopGateRef,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostErrorKind, AssistantReply, FinalizeAssistantMessage,
        LoopCheckpointKind, LoopCheckpointRequest, LoopCheckpointStateRef,
        StoreLoopCheckpointPayload,
    },
};

use crate::{
    planner::AgentLoopPlanner,
    state::{CHECKPOINT_SCHEMA_ID, CheckpointKind, CheckpointMarker, LoopExecutionState},
    strategies::{
        GateKind, GateOutcome, GateSummary, StopKind, StopOutcome, TurnEndKind, TurnSummary,
    },
};

use super::util::{NO_PROGRESS_THRESHOLD, NO_PROGRESS_WINDOW};
use super::{
    AgentLoopExecutorError, CanonicalAgentLoopExecutor, CompletionKind, FailureKind, HostStage,
    LoopExit, canonical::Step,
};

impl CanonicalAgentLoopExecutor {
    pub(super) async fn finalize_reply_and_check_stop(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        reply: AssistantReply,
    ) -> Result<(LoopExecutionState, StopOutcome), AgentLoopExecutorError> {
        let assistant_ref = host
            .finalize_assistant_message(FinalizeAssistantMessage { reply })
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Transcript,
            })?;
        state.assistant_refs.push(assistant_ref.clone());

        let summary = TurnSummary {
            kind: TurnEndKind::ReplyOnly,
            assistant_message_ref: Some(assistant_ref),
            batch_result_refs: Vec::new(),
        };
        let stop = planner
            .stop()
            .should_stop_after_turn(&state, &summary)
            .await;
        match &stop {
            StopOutcome::Continue { control } | StopOutcome::Stop { control, .. } => {
                state.control_state = control.clone();
            }
        }
        Ok((state, stop))
    }

    pub(super) async fn handle_gate(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        kind: GateKind,
        gate_ref: LoopGateRef,
    ) -> Result<Step, AgentLoopExecutorError> {
        let summary = GateSummary {
            kind,
            gate_ref: gate_ref.clone(),
        };
        match planner.gate().handle(&state, &summary).await {
            GateOutcome::Block { control } => {
                state.control_state = control;
                state.last_gate = Some(gate_ref.clone());
                state = self
                    .checkpoint(host, state, CheckpointKind::BeforeBlock)
                    .await?;
                Ok(Step::Exit(state, LoopExit::Blocked { gate_ref }))
            }
            GateOutcome::SkipAndContinue { control } => {
                state.control_state = control;
                Ok(Step::Continue(state))
            }
            GateOutcome::Abort {
                control,
                failure_kind,
            } => {
                state.control_state = control;
                Ok(Step::Exit(
                    state,
                    LoopExit::Failed {
                        kind: failure_kind_to_exit(failure_kind),
                    },
                ))
            }
        }
    }

    pub(super) async fn checkpoint(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        kind: CheckpointKind,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        // Master spec §10: the checkpoint payload MUST be persisted before
        // the checkpoint marker is recorded. `HostManagedLoopCheckpointPort`
        // rejects unknown state refs by design — store first, then checkpoint
        // with the returned ref.
        //
        // Legacy hosts that have not migrated to the
        // `store_checkpoint_payload`-then-`checkpoint` contract return
        // `Unavailable` from the default trait impl; we fall back to the
        // legacy `checkpoint()`-only path using the `legacy_unknown`
        // sentinel ref. Any other error bubbles up as `CheckpointFailed`
        // so retries can re-poll.
        let payload = serde_json::to_vec(&serde_json::json!({
            "schema_id": CHECKPOINT_SCHEMA_ID,
            "state": &state,
        }))
        .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        let host_kind = host_checkpoint_kind(kind);
        let state_ref = match host
            .store_checkpoint_payload(StoreLoopCheckpointPayload {
                kind: host_kind,
                payload,
            })
            .await
        {
            Ok(state_ref) => state_ref,
            Err(err) if matches!(err.kind, AgentLoopHostErrorKind::Unavailable) => {
                LoopCheckpointStateRef::legacy_unknown()
            }
            Err(_) => return Err(AgentLoopExecutorError::CheckpointFailed { stage: kind }),
        };
        host.checkpoint(LoopCheckpointRequest {
            kind: host_kind,
            state_ref,
        })
        .await
        .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        state.last_checkpoint = Some(CheckpointMarker {
            kind,
            iteration_at_checkpoint: state.iteration,
        });
        Ok(state)
    }

    pub(super) async fn exit_for_stop_kind(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        kind: StopKind,
    ) -> Result<(LoopExecutionState, LoopExit), AgentLoopExecutorError> {
        // Every terminal exit path takes `Final` just before returning
        // so profiles with `require_final_checkpoint = true` don't
        // reject the exit as `MissingFinalCheckpoint`. The helper
        // uniformly checkpoints and returns the checked state alongside
        // the exit so the caller can commit it to `*state`.
        match kind {
            StopKind::GracefulStop => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok((checked, LoopExit::Completed(CompletionKind::GracefulStop)))
            }
            StopKind::NoProgressDetected => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok((
                    checked,
                    LoopExit::Failed {
                        kind: FailureKind::NoProgressDetected,
                    },
                ))
            }
            StopKind::Aborted(failure) => {
                let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
                Ok((
                    checked,
                    LoopExit::Failed {
                        kind: failure_kind_to_exit(failure),
                    },
                ))
            }
        }
    }

    /// When a sub-routine returns `LoopExit::Failed`, take a `Final`
    /// checkpoint before propagating it. `Completed` / `Blocked` /
    /// `Cancelled` exits already carry their own checkpoint discipline
    /// (Final / BeforeBlock / Final respectively) inside the sub-routine,
    /// so this helper is a no-op for them.
    pub(super) async fn final_checkpoint_for_failure(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        exit: LoopExit,
    ) -> Result<(LoopExecutionState, LoopExit), AgentLoopExecutorError> {
        if matches!(exit, LoopExit::Failed { .. }) {
            let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
            Ok((checked, exit))
        } else {
            Ok((state, exit))
        }
    }

    pub(super) fn no_progress_exit(&self, state: &LoopExecutionState) -> Option<FailureKind> {
        if state
            .recent_call_signatures
            .most_common_count_in(NO_PROGRESS_WINDOW)
            >= NO_PROGRESS_THRESHOLD
        {
            Some(FailureKind::NoProgressDetected)
        } else {
            None
        }
    }
}

pub(super) fn failure_kind_to_exit(kind: LoopFailureKind) -> FailureKind {
    match kind {
        LoopFailureKind::IterationLimit => FailureKind::IterationLimitReached,
        LoopFailureKind::NoProgressDetected => FailureKind::NoProgressDetected,
        LoopFailureKind::WallClockLimit => FailureKind::WallClockLimitReached,
        other => FailureKind::Other(other),
    }
}

pub(super) fn host_checkpoint_kind(kind: CheckpointKind) -> LoopCheckpointKind {
    match kind {
        CheckpointKind::BeforeModel => LoopCheckpointKind::BeforeModel,
        CheckpointKind::BeforeSideEffect => LoopCheckpointKind::BeforeSideEffect,
        CheckpointKind::BeforeBlock => LoopCheckpointKind::BeforeBlock,
        CheckpointKind::Final => LoopCheckpointKind::Final,
    }
}
