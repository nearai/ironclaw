use super::*;

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
    Ok(LoopExit::Completed(LoopCompleted {
        completion_kind,
        reply_message_refs: state.assistant_refs,
        result_refs: state.result_refs,
        final_checkpoint_id,
        usage_summary_ref: None,
        exit_id: exit_id(host, "completed")?,
    }))
}

pub(super) fn failed_exit(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    _state: LoopExecutionState,
    reason_kind: LoopFailureKind,
    checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> Result<LoopExit, AgentLoopExecutorError> {
    Ok(LoopExit::Failed(LoopFailed {
        reason_kind,
        checkpoint_id,
        usage_summary_ref: None,
        diagnostic_ref: None,
        exit_id: exit_id(host, "failed")?,
    }))
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
