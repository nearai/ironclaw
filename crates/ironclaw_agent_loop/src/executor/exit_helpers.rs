use ironclaw_turns::{
    LoopCancelled, LoopCancelledReasonKind, LoopCompleted, LoopCompletionKind, LoopDiagnosticRef,
    LoopExit, LoopExitId, LoopFailed, LoopFailureKind, LoopMessageRef, SanitizedFailure,
    run_profile::{AgentLoopDriverHost, LoopCancelReasonKind, LoopCancellationSignal},
};

use crate::state::LoopExecutionState;

use super::AgentLoopExecutorError;

pub(super) fn completed_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: LoopExecutionState,
    final_checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> Result<LoopExit, AgentLoopExecutorError> {
    let completion_kind = if !state.assistant_refs.is_empty() {
        LoopCompletionKind::FinalReply
    } else if !state.result_refs.is_empty() {
        LoopCompletionKind::ResultOnly
    } else {
        LoopCompletionKind::NoReply
    };
    let model_usage = state.cumulative_model_usage;
    Ok(LoopExit::Completed(LoopCompleted {
        completion_kind,
        reply_message_refs: state.assistant_refs,
        result_refs: state.result_refs,
        final_checkpoint_id,
        model_usage,
        exit_id: exit_id(host, "completed")?,
    }))
}

pub(super) fn failed_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: LoopExecutionState,
    reason_kind: LoopFailureKind,
    checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
    details: FailedExitDetails,
) -> Result<LoopExit, AgentLoopExecutorError> {
    let model_usage = state.cumulative_model_usage;
    Ok(LoopExit::Failed(LoopFailed {
        reason_kind,
        checkpoint_id,
        model_usage,
        diagnostic_ref: details.diagnostic_ref,
        exit_id: exit_id(host, "failed")?,
        explanation_message_refs: failure_message_refs(&state, details.explanation_message_ref),
        safe_summary: details.safe_summary,
    }))
}

#[derive(Debug, Clone, Default)]
pub(super) struct FailedExitDetails {
    pub(super) diagnostic_ref: Option<LoopDiagnosticRef>,
    pub(super) safe_summary: Option<SanitizedFailure>,
    pub(super) explanation_message_ref: Option<LoopMessageRef>,
}

fn failure_message_refs(
    state: &LoopExecutionState,
    explanation_message_ref: Option<LoopMessageRef>,
) -> Vec<LoopMessageRef> {
    let mut refs = Vec::new();
    for message_ref in state
        .assistant_refs
        .iter()
        .cloned()
        .chain(explanation_message_ref)
    {
        if !refs.contains(&message_ref) {
            refs.push(message_ref);
        }
    }
    refs
}

pub(super) fn cancelled_reason_from_signal(
    signal: &LoopCancellationSignal,
) -> LoopCancelledReasonKind {
    // LoopCancelReasonKind preserves host/input detail; LoopExit currently exposes
    // the coarser terminal taxonomy, so every observed signal maps explicitly here.
    //
    // Reason coarsened to HostCancellation intentionally: the loop exit taxonomy
    // does not expose raw reason_kind to the product layer at this WS boundary.
    // WS16/WS17 can map finer-grained reasons when the product adapter is wired.
    match signal.reason_kind {
        LoopCancelReasonKind::UserRequested
        | LoopCancelReasonKind::Superseded
        | LoopCancelReasonKind::Policy => LoopCancelledReasonKind::HostCancellation,
    }
}

pub(super) fn cancelled_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: LoopExecutionState,
    checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> Result<LoopExit, AgentLoopExecutorError> {
    cancelled_exit_with_reason(
        host,
        state,
        LoopCancelledReasonKind::HostCancellation,
        checkpoint_id,
    )
}

pub(super) fn cancelled_exit_with_reason(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: LoopExecutionState,
    reason_kind: LoopCancelledReasonKind,
    checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> Result<LoopExit, AgentLoopExecutorError> {
    Ok(LoopExit::Cancelled(LoopCancelled {
        reason_kind,
        checkpoint_id,
        interrupted_message_refs: state.assistant_refs,
        exit_id: exit_id(host, "cancelled")?,
    }))
}

pub(super) fn exit_id(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    suffix: &'static str,
) -> Result<LoopExitId, AgentLoopExecutorError> {
    LoopExitId::new(format!("exit:{}-{suffix}", host.run_context().run_id)).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "run id could not be represented as loop exit id",
        }
    })
}
