use ironclaw_host_api::turn::LoopGateRef;
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CheckpointSchemaId, LoopCheckpointRequest,
    LoopProgressEvent, LoopRecoveryClass, LoopRecoveryDisposition, LoopRecoveryStage,
    LoopSafeSummary, StageCheckpointPayloadRequest, sanitize_model_visible_text,
};

use crate::state::{CheckpointKind, LoopExecutionState};

#[cfg(test)]
use crate::executor::CanonicalAgentLoopExecutor;

#[cfg(test)]
use ironclaw_loop_contracts::AgentLoopDriverHost;

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointWrite, HostStage, StageContext,
    cancelled_exit_with_reason, cancelled_reason_from_signal, checkpoint_kind_to_host,
    debug_host_unavailable,
};

#[cfg(test)]
use super::{DrainedInputs, InputStage};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CheckpointStage;

impl CheckpointStage {
    pub(super) async fn write(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
        kind: CheckpointKind,
    ) -> Result<CheckpointWrite, AgentLoopExecutorError> {
        self.write_with_gate_ref(ctx, state, kind, None).await
    }

    /// Iteration-batched `BeforeModel` write for the canonical per-iteration
    /// boundary.
    ///
    /// Writing the full serialized state before every model call costs one
    /// checkpoint row plus one process-row update per iteration. Flushing only
    /// every `before_model_checkpoint_interval`-th iteration trades that for a
    /// bounded replay cost — but only where the trade is actually safe, which
    /// is narrower than it first appears.
    ///
    /// # Why the previous checkpoint kind gates the skip
    ///
    /// The scheduler's lease-expiry recovery does not just resume from the
    /// newest checkpoint, it decides WHETHER to resume from that checkpoint's
    /// kind (`ironclaw_processes`' `replays_side_effect`, read in
    /// `apply_recover_expired`). A run whose newest checkpoint is
    /// `BeforeSideEffect` is failed closed rather than requeued, because
    /// nothing durable proves the effect did not land.
    ///
    /// So the `BeforeModel` written after a tool-calling iteration is not
    /// bookkeeping: it is what clears the fail-closed marker the preceding
    /// `BeforeSideEffect` left on the process row. Skipping it would convert
    /// an auto-recoverable crash into a user-visible `lease_expired` failure,
    /// which is a durability regression, not a replay cost. Batching is
    /// therefore allowed only when the previous durable checkpoint is itself
    /// resumable — pinned by
    /// `run_interrupted_on_a_batched_away_iteration_resumes_without_repeating_side_effects`.
    ///
    /// # What a skip does cost
    ///
    /// Only replayed MODEL calls. `state.last_checkpoint` keeps pointing at
    /// the previous durable checkpoint, which is what resume reads, and every
    /// exit path — gate, cancel, budget, terminal — still writes its own
    /// `BeforeBlock` or `Final` checkpoint before returning.
    ///
    /// The other `BeforeModel` writers (input drain, compaction, model-retry
    /// recovery) call [`write`] directly and stay unbatched: each persists
    /// consumed one-shot budget that a replay must not re-grant.
    pub(super) async fn write_before_model_batched(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
    ) -> Result<LoopExecutionState, AgentLoopExecutorError> {
        let policy = &ctx
            .host
            .run_context()
            .resolved_run_profile
            .checkpoint_policy;
        // Fail-closed on the first iteration too: no previous checkpoint means
        // nothing to fall back to, so it always flushes.
        let previous_is_resumable = state
            .last_checkpoint
            .as_ref()
            .is_some_and(|marker| !marker.kind.replays_side_effect());
        if previous_is_resumable && !policy.flushes_before_model_at(state.iteration) {
            tracing::debug!(
                iteration = state.iteration,
                interval = policy.before_model_flush_interval(),
                "skipping batched BeforeModel checkpoint between flush points"
            );
            return Ok(state);
        }
        Ok(self
            .write(ctx, state, CheckpointKind::BeforeModel)
            .await?
            .state)
    }

    /// Variant of [`write`] for `BeforeBlock` checkpoints. Stores the
    /// triggering gate ref in the checkpoint record so that
    /// `verify_blocked_evidence` can cross-check gate identity and reject
    /// a rogue driver that reuses a legitimate checkpoint from a different gate.
    pub(super) async fn write_before_block(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
        gate_ref: &LoopGateRef,
    ) -> Result<CheckpointWrite, AgentLoopExecutorError> {
        self.write_with_gate_ref(
            ctx,
            state,
            CheckpointKind::BeforeBlock,
            Some(gate_ref.clone()),
        )
        .await
    }

    async fn write_with_gate_ref(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        kind: CheckpointKind,
        gate_ref: Option<LoopGateRef>,
    ) -> Result<CheckpointWrite, AgentLoopExecutorError> {
        state.last_checkpoint = Some(crate::state::CheckpointMarker {
            kind,
            iteration_at_checkpoint: state.iteration,
        });
        let payload = serde_json::to_vec(&state)
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        let host_kind = checkpoint_kind_to_host(kind);
        let schema_id = CheckpointSchemaId::new(crate::state::CHECKPOINT_SCHEMA_ID)
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        let state_ref = ctx
            .host
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: host_kind,
                schema_id,
                payload,
            })
            .await
            .map_err(|error| checkpoint_host_error(kind, error))?;
        let checkpoint_id = ctx
            .host
            .checkpoint(LoopCheckpointRequest {
                kind: host_kind,
                state_ref: state_ref.clone(),
                gate_ref,
            })
            .await
            .map_err(|error| checkpoint_host_error(kind, error))?;
        self.emit_progress(
            ctx,
            LoopProgressEvent::CheckpointWritten {
                iteration: state.iteration,
                kind: host_kind,
            },
        )
        .await;
        Ok(CheckpointWrite {
            state,
            checkpoint_id,
            state_ref,
        })
    }

    pub(super) async fn emit_progress(&self, ctx: StageContext<'_>, event: LoopProgressEvent) {
        let _ = ctx.host.emit_loop_progress(event).await;
    }

    /// Append recovery evidence before the recovery transition can continue.
    ///
    /// Unlike observational progress, this event is a durable metric input.
    /// Its sequence advances only after the host accepts the append. A worker
    /// replaying the pre-transition checkpoint therefore reuses the same
    /// sequence and logical event identity.
    pub(super) async fn emit_recovery(
        &self,
        ctx: StageContext<'_>,
        state: &mut LoopExecutionState,
        stage: LoopRecoveryStage,
        class: LoopRecoveryClass,
        disposition: LoopRecoveryDisposition,
    ) -> Result<(), AgentLoopExecutorError> {
        let sequence = state
            .recovery_event_sequence
            .checked_add(1)
            .ok_or(AgentLoopExecutorError::RecoverySequenceExhausted)?;
        ctx.host
            .emit_loop_progress(LoopProgressEvent::FailureRecovered {
                sequence,
                stage,
                class,
                disposition,
            })
            .await
            .map_err(recovery_event_host_error)?;
        state.recovery_event_sequence = sequence;
        Ok(())
    }

    // Cancellation is checked cooperatively at N boundary points between external calls.
    // A macro refactor was considered but deferred; the explicit sites are self-documenting.
    pub(super) async fn cancel_if_requested(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
    ) -> Result<CancelCheck, AgentLoopExecutorError> {
        let Some(signal) = ctx.host.observe_cancellation() else {
            return Ok(CancelCheck::Continue(Box::new(state)));
        };

        let fallback_state = state.clone();
        match self.write(ctx, state, CheckpointKind::Final).await {
            Ok(checked) => Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                ctx.host,
                checked.state,
                cancelled_reason_from_signal(&signal),
                Some(checked.checkpoint_id),
            )?)),
            // Permissive profile: only checkpoint-write failures are absorbed
            // into a checkpoint-free `Cancelled` exit. Other variants (e.g.
            // `HostUnavailable`) must propagate so the runner can apply its
            // recovery policy.
            Err(
                AgentLoopExecutorError::CheckpointFailed { .. }
                | AgentLoopExecutorError::CheckpointRejected { .. },
            ) if !ctx
                .host
                .run_context()
                .resolved_run_profile
                .checkpoint_policy
                .require_final_checkpoint =>
            {
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    ctx.host,
                    fallback_state,
                    cancelled_reason_from_signal(&signal),
                    None,
                )?))
            }
            Err(error) => Err(error),
        }
    }
}

fn checkpoint_host_error(
    kind: CheckpointKind,
    error: AgentLoopHostError,
) -> AgentLoopExecutorError {
    if error.kind == AgentLoopHostErrorKind::Cancelled {
        return AgentLoopExecutorError::Cancelled;
    }
    debug_host_unavailable(HostStage::Checkpoint, &error);
    if error.kind == AgentLoopHostErrorKind::CheckpointRejected {
        let safe_summary = LoopSafeSummary::new(error.safe_summary).unwrap_or_else(|error| {
            tracing::debug!(
                checkpoint_kind = ?kind,
                validation_error = %error,
                "checkpoint rejection summary rejected; using fixed fallback"
            );
            LoopSafeSummary::checkpoint_rejected()
        });
        return AgentLoopExecutorError::CheckpointRejected {
            stage: kind,
            safe_summary,
        };
    }
    if matches!(
        error.kind,
        AgentLoopHostErrorKind::Unavailable
            | AgentLoopHostErrorKind::Internal
            | AgentLoopHostErrorKind::BudgetAccountingFailed
    ) {
        let raw_summary = error.safe_summary;
        let (safe_summary, rejected_summary_detail) =
            match LoopSafeSummary::new(raw_summary.clone()) {
                Ok(summary) => (summary, None),
                Err(validation_error) => {
                    tracing::debug!(
                        checkpoint_kind = ?kind,
                        validation_error = %validation_error,
                        "checkpoint error summary rejected; using fallback"
                    );
                    (
                        LoopSafeSummary::model_gateway_failed(),
                        Some(sanitize_model_visible_text(raw_summary)),
                    )
                }
            };
        let detail = error.detail.or(rejected_summary_detail);
        return AgentLoopExecutorError::HostUnavailableWithDiagnostics {
            stage: HostStage::Checkpoint,
            kind: error.kind,
            safe_summary,
            reason_kind: error.reason_kind,
            detail,
        };
    }
    AgentLoopExecutorError::CheckpointFailed { stage: kind }
}

fn recovery_event_host_error(error: AgentLoopHostError) -> AgentLoopExecutorError {
    if error.kind == AgentLoopHostErrorKind::Cancelled {
        return AgentLoopExecutorError::Cancelled;
    }
    debug_host_unavailable(HostStage::Checkpoint, &error);
    let raw_summary = error.safe_summary;
    let (safe_summary, rejected_summary_detail) = match LoopSafeSummary::new(raw_summary.clone()) {
        Ok(summary) => (summary, None),
        Err(validation_error) => {
            tracing::debug!(
                validation_error = %validation_error,
                "recovery event error summary rejected; using fallback"
            );
            (
                LoopSafeSummary::model_gateway_failed(),
                Some(sanitize_model_visible_text(raw_summary)),
            )
        }
    };
    AgentLoopExecutorError::HostUnavailableWithDiagnostics {
        stage: HostStage::Checkpoint,
        kind: error.kind,
        safe_summary,
        reason_kind: error.reason_kind,
        detail: error.detail.or(rejected_summary_detail),
    }
}

#[cfg(test)]
impl CanonicalAgentLoopExecutor {
    pub(super) async fn drain_user_inputs(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        let family = crate::families::default();
        let ctx = StageContext {
            planner: family.planner(),
            host,
        };
        InputStage.drain_user_inputs(ctx, state).await
    }

    pub(super) async fn drain_followup(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        let family = crate::families::default();
        let ctx = StageContext {
            planner: family.planner(),
            host,
        };
        InputStage.drain_followup(ctx, state).await
    }
}

#[cfg(test)]
mod checkpoint_host_error_tests {
    use super::*;

    #[test]
    fn invalid_checkpoint_rejection_summary_uses_cause_neutral_fallback() {
        let error = checkpoint_host_error(
            CheckpointKind::BeforeModel,
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "api_key marker must not escape",
            ),
        );

        assert_eq!(
            error,
            AgentLoopExecutorError::CheckpointRejected {
                stage: CheckpointKind::BeforeModel,
                safe_summary: LoopSafeSummary::checkpoint_rejected(),
            }
        );
        assert_eq!(
            LoopSafeSummary::checkpoint_rejected().as_str(),
            "checkpoint was rejected and no safe explanation was available"
        );
    }
}
