use ironclaw_host_api::{
    execution_policy::ResultDeliveryPolicy,
    turn::{LoopExitId, LoopMessageRef, SanitizedFailure, TurnOriginKind},
};
use ironclaw_loop_contracts::{
    AgentLoopDriverHost, LoopCancelReasonKind, LoopCancellationSignal, LoopCancelled,
    LoopCancelledReasonKind, LoopCompleted, LoopCompletionKind, LoopExit, LoopFailed,
    LoopFailureKind,
};

use crate::state::LoopExecutionState;

use super::AgentLoopExecutorError;

pub(super) fn completed_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: LoopExecutionState,
    final_checkpoint_id: Option<ironclaw_host_api::turn::TurnCheckpointId>,
) -> Result<LoopExit, AgentLoopExecutorError> {
    let typed_nothing_to_report = is_typed_nothing_to_report(host, &state);
    let completion_kind = if typed_nothing_to_report {
        LoopCompletionKind::NothingToReport
    } else if !state.assistant_refs.is_empty() {
        LoopCompletionKind::FinalReply
    } else if !state.result_refs.is_empty() {
        LoopCompletionKind::ResultOnly
    } else {
        LoopCompletionKind::NoReply
    };
    let model_usage = state.cumulative_model_usage;
    // Earlier model iterations may have emitted progress text and ordinary
    // tool results before the terminal structured result. Retain those
    // transcript rows, but do not expose any of their refs as deliverable
    // completion output. Settlement independently verifies the exact typed
    // terminal call and arguments from the durable transcript.
    let reply_message_refs = if typed_nothing_to_report {
        Vec::new()
    } else {
        state.assistant_refs
    };
    let result_refs = if typed_nothing_to_report {
        Vec::new()
    } else {
        state.result_refs
    };
    Ok(LoopExit::Completed(LoopCompleted {
        completion_kind,
        reply_message_refs,
        result_refs,
        final_checkpoint_id,
        model_usage,
        exit_id: exit_id(host, "completed")?,
    }))
}

pub(super) fn is_typed_nothing_to_report(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &LoopExecutionState,
) -> bool {
    !state.result_refs.is_empty()
        && state.stop_state.structured_result_recorded
        && host
            .run_context()
            .product_context
            .as_ref()
            .filter(|context| context.origin == TurnOriginKind::ScheduledTrigger)
            .and_then(|context| context.execution_policy.as_ref())
            .is_some_and(|policy| {
                policy.result_delivery == ResultDeliveryPolicy::SuppressWhenNothingToReport
            })
}

pub(super) fn failed_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: LoopExecutionState,
    reason_kind: LoopFailureKind,
    checkpoint_id: Option<ironclaw_host_api::turn::TurnCheckpointId>,
    details: FailedExitDetails,
) -> Result<LoopExit, AgentLoopExecutorError> {
    let model_usage = state.cumulative_model_usage;
    Ok(LoopExit::Failed(LoopFailed {
        reason_kind,
        checkpoint_id,
        model_usage,
        exit_id: exit_id(host, "failed")?,
        explanation_message_refs: failure_message_refs(&state, details.explanation_message_ref),
        safe_summary: details.safe_summary,
    }))
}

#[derive(Debug, Clone, Default)]
pub(super) struct FailedExitDetails {
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
    checkpoint_id: Option<ironclaw_host_api::turn::TurnCheckpointId>,
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
    checkpoint_id: Option<ironclaw_host_api::turn::TurnCheckpointId>,
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
