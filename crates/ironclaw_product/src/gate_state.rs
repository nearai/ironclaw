use ironclaw_host_api::turn::{TurnActor, TurnGateRef, TurnRunId, TurnScope, TurnStatus};
use ironclaw_turns::{GetRunStateRequest, TurnCoordinator, TurnError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockedGateState {
    ParkedOnGate,
    NotParkedOnGate,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BlockedGateStateError {
    #[error("turn run state unavailable")]
    Turn(#[from] TurnError),
    #[error("turn run actor mismatch")]
    ActorMismatch,
}

/// Checks the parked run by actor first, then matches status and gate.
///
/// Callers that already read run-state for service routing should still use
/// this before mutating typed gate state: the second read is a freshness /
/// TOCTOU guard immediately before resume/cancel side effects.
pub(crate) async fn blocked_gate_state(
    turn_coordinator: &dyn TurnCoordinator,
    scope: &TurnScope,
    actor: &TurnActor,
    run_id: TurnRunId,
    gate_ref: &TurnGateRef,
    blocked_status: TurnStatus,
) -> Result<BlockedGateState, BlockedGateStateError> {
    let state = turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await?;
    if state.actor.as_ref() != Some(actor) {
        return Err(BlockedGateStateError::ActorMismatch);
    }
    if state.status != blocked_status || state.gate_ref.as_ref() != Some(gate_ref) {
        return Ok(BlockedGateState::NotParkedOnGate);
    }
    Ok(BlockedGateState::ParkedOnGate)
}
