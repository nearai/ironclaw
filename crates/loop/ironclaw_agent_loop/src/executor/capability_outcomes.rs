use ironclaw_host_api::resolution::{Resolution, Suspension};
use ironclaw_host_api::turn::LoopGateRef;
use ironclaw_loop_contracts::{
    CapabilityCallCandidate, CapabilityProgress, CapabilityResultMessage, LoopExit,
};

use crate::{
    state::{
        CapabilityCallSignature, CapabilityOutputObservation, CheckpointKind, LoopExecutionState,
    },
    strategies::{CapabilityBatchTurnSummary, TurnSummary},
};

use super::capabilities::CapabilityStage;
use super::capability_helpers::{append_capability_safe_summary_ref, gate_tool_result_summary};
use super::capability_records::{ChildResultAppendInput, dependent_run_result_message};
use super::gates::gate_outcome_kind;
use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, FailedExitDetails, StageContext,
    TurnCompletedStep, append_capability_result_ref, cancelled_exit_with_reason,
    capability_host_error, clear_matching_pending_approval_resume,
    clear_matching_pending_auth_resume, clear_matching_pending_external_tool_resume, failed_exit,
    push_completed_result, sanitized_strategy_summary_or_fallback,
};

/// Rebuild a selected terminal sibling exit against a freshly staged Final
/// checkpoint so its checkpoint carries every processed sibling's mutations
/// (siblings after the exit were processed and merged after the exit's own
/// checkpoint was staged). The original reason/kind metadata and safe summary
/// are preserved; explanation refs are re-derived from the merged state's
/// assistant refs — the exit's own explanation was already appended to them
/// during its outcome processing.
fn rebuild_terminal_exit_against_checkpoint(
    ctx: StageContext<'_>,
    exit: LoopExit,
    state: LoopExecutionState,
    checkpoint_id: ironclaw_host_api::turn::TurnCheckpointId,
) -> Result<LoopExit, AgentLoopExecutorError> {
    match exit {
        LoopExit::Failed(failed) => failed_exit(
            ctx.host,
            state,
            failed.reason_kind,
            Some(checkpoint_id),
            FailedExitDetails {
                safe_summary: failed.safe_summary,
                explanation_message_ref: None,
            },
        ),
        LoopExit::Cancelled(cancelled) => {
            cancelled_exit_with_reason(ctx.host, state, cancelled.reason_kind, Some(checkpoint_id))
        }
        LoopExit::Completed(_) | LoopExit::Blocked(_) => {
            Err(AgentLoopExecutorError::PlannerContract {
                detail: "selected sibling exit was not terminal failure or cancellation",
            })
        }
    }
}

pub(super) enum SelectedParallelTerminal {
    Loop(LoopExit),
    Host(ironclaw_loop_contracts::AgentLoopHostError),
}

pub(super) async fn finish_selected_parallel_terminal(
    ctx: StageContext<'_>,
    state: LoopExecutionState,
    terminal: SelectedParallelTerminal,
) -> Result<TurnCompletedStep, AgentLoopExecutorError> {
    let checked = CheckpointStage
        .write(ctx, state, CheckpointKind::Final)
        .await?;
    match terminal {
        SelectedParallelTerminal::Loop(exit) => {
            let exit = rebuild_terminal_exit_against_checkpoint(
                ctx,
                exit,
                checked.state,
                checked.checkpoint_id,
            )?;
            Ok(TurnCompletedStep::Exit(exit))
        }
        SelectedParallelTerminal::Host(error) => Err(capability_host_error(error)),
    }
}

impl CapabilityStage {
    pub(super) async fn completed_turn(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
        result_refs_start: usize,
        capability_batch: CapabilityBatchTurnSummary,
    ) -> Result<TurnCompletedStep, AgentLoopExecutorError> {
        let state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
        };
        let summary = TurnSummary::after_capability_batch(
            state.result_refs[result_refs_start..].to_vec(),
            capability_batch,
        );
        Ok(TurnCompletedStep::Continue {
            state: Box::new(state),
            summary,
        })
    }
}

pub(super) async fn append_spawned_child_result(
    host: &(dyn ironclaw_loop_contracts::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    signature: CapabilityCallSignature,
    input: ChildResultAppendInput,
    capability_batch: &mut CapabilityBatchTurnSummary,
) -> Result<(), AgentLoopExecutorError> {
    // Keep this write seam fail-soft even though `Resolution` normally arrives
    // with a validated `SafeSummary`: a malformed adapter value must degrade the
    // label, not end the run.
    let (safe_summary, _) =
        sanitized_strategy_summary_or_fallback(input.safe_summary, "spawned a child run");
    let safe_summary = safe_summary.into_inner();
    let result = CapabilityResultMessage {
        result_ref: input.result_ref,
        safe_summary,
        progress: CapabilityProgress::MadeProgress,
        terminate_hint: false,
        byte_len: input.byte_len,
        output_digest: None,
        model_observation: input.model_observation,
    };
    append_completed_capability_result(host, state, call, signature, result, capability_batch).await
}

pub(super) async fn append_completed_capability_result(
    host: &(dyn ironclaw_loop_contracts::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    // Computed once before dispatch and carried through the indexed outcome.
    signature: CapabilityCallSignature,
    result: CapabilityResultMessage,
    capability_batch: &mut CapabilityBatchTurnSummary,
) -> Result<(), AgentLoopExecutorError> {
    append_capability_result_ref(host, call, &result).await?;
    // #7531 made repeated-call detection advisory-only by deleting this
    // ring's only producer; dominant_repeated_observation (strategies/stop.rs)
    // needs a (signature, output_digest) trail — real OUTPUT repetition, not
    // just repeated CALLS. A result with no digest (synthetic results, older
    // hosts, failures) never counts.
    if let Some(output_digest) = result.output_digest {
        state
            .seen_capability_output_digests
            .push(CapabilityOutputObservation {
                signature: signature.clone(),
                output_digest,
            });
    }
    // NOT (re-)promoted to host-reported CapabilityProgress — that stays
    // retired; the advisory keys off consecutive signatures, the terminating
    // check above keys off the digest ring, neither reads this.
    capability_batch.record_result(signature, result.terminate_hint);
    push_completed_result(state, &call.capability_id, result);
    Ok(())
}
/// Complete a later sibling gate model-visibly without allocating another
/// resumable gate slot. The first gate in input order owns the batch's single
/// BeforeBlock checkpoint; later calls return a durable "pending" observation
/// so no provider call is left without a result and the model can retry after
/// the first gate resolves. A dependent run already has a concrete result, so
/// preserve that result instead of replacing it with a pending summary.
pub(super) async fn persist_later_gate_outcome(
    ctx: StageContext<'_>,
    state: &mut LoopExecutionState,
    call: CapabilityCallCandidate,
    resolution: Resolution,
) -> Result<(), AgentLoopExecutorError> {
    // A later sibling may itself be a resumed call. Its new pending/concrete
    // result consumes the prior resume token; only the first gate may remain
    // resumable in the checkpoint this batch returns.
    clear_matching_pending_approval_resume(state, &call);
    clear_matching_pending_auth_resume(state, &call);
    clear_matching_pending_external_tool_resume(state, &call);
    // A dependent run already has a concrete result; persist that result
    // instead of a pending summary.
    if let Resolution::Suspended(Suspension::DependentRun { result, .. }) = &resolution {
        let result = dependent_run_result_message(result)?;
        append_capability_result_ref(ctx.host, &call, &result).await?;
        push_completed_result(state, &call.capability_id, result);
        return Ok(());
    }
    let Some(kind) = gate_outcome_kind(&resolution) else {
        return Err(AgentLoopExecutorError::PlannerContract {
            detail: "persist_later_gate_outcome called for a non-gate outcome",
        });
    };
    append_capability_safe_summary_ref(
        ctx.host,
        state,
        &call,
        gate_tool_result_summary(kind, "pending"),
    )
    .await
}

pub(super) fn shared_await_dependent_gate(
    calls: &[CapabilityCallCandidate],
    resolutions: &[Resolution],
) -> Option<(
    ironclaw_host_api::turn::LoopGateRef,
    CapabilityCallCandidate,
)> {
    let mut shared_gate: Option<ironclaw_host_api::turn::LoopGateRef> = None;
    let mut first_call: Option<CapabilityCallCandidate> = None;
    let mut count = 0_usize;
    for (call, resolution) in calls.iter().zip(resolutions.iter()) {
        match resolution {
            Resolution::Suspended(Suspension::DependentRun { waypoint, .. }) => {
                // Coalesce on the preserved originating loop gate ref; a missing
                // origin (never produced by the mapping) can't be coalesced.
                let gate_ref = waypoint
                    .origin
                    .as_ref()
                    .and_then(|origin| LoopGateRef::new(origin.as_str()).ok())?;
                if let Some(existing) = shared_gate.as_ref() {
                    if existing != &gate_ref {
                        return None;
                    }
                } else {
                    shared_gate = Some(gate_ref);
                    first_call = Some(call.clone());
                }
                count += 1;
            }
            // Any other parked work — a re-entrant gate (`Blocked`) or a
            // non-dependent-run suspension (process/external-tool) — means the
            // batch cannot coalesce into a single dependent-run gate. `parks()`,
            // not `is_suspension()`, to also catch `Blocked` (H1).
            resolution if resolution.parks() => {
                return None;
            }
            _ => {}
        }
    }
    // Only coalesce when at least two AwaitDependentRun outcomes share the
    // same gate — that is the case the fast path exists for. A single
    // AwaitDependentRun (with or without sibling completed outcomes) has no
    // coalescing benefit, and routing through this path would diverge the
    // completed-first durability ordering the non-suspended branch
    // guarantees. Fall back to the per-outcome path for single-await batches.
    if count <= 1 {
        return None;
    }
    shared_gate.zip(first_call)
}
