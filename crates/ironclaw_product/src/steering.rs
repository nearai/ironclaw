//! Shared busy-run steering enqueue gateway.
//!
//! Both the product inbound-turn path ([`crate::inbound_turn`]) and the WebUI
//! facade ([`crate::reborn_services`]) enqueue a user message as steering input
//! when the target run is busy. This module owns the single enqueue sequence so
//! the two callers cannot drift on ordering, idempotency, or error fidelity.

use ironclaw_host_api::ids::ThreadId;
use ironclaw_loop_contracts::LoopInput;
use ironclaw_loop_host::{EnqueueQueuedMessageRequest, HostInputEnqueuePort, HostInputQueueError};
use ironclaw_threads::{ThreadMessageId, ThreadScope};
use ironclaw_turns::{
    AcceptedMessageRef, GetRunStateRequest, LoopMessageRef, TurnCoordinator, TurnError, TurnRunId,
    TurnScope,
};

/// Failure surface of [`enqueue_busy_steering`].
///
/// Each variant maps to a distinct caller-facing error so neither the inbound
/// path nor the WebUI facade collapses an enqueue failure into a generic,
/// cause-less error.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SteeringEnqueueError {
    /// The accepted message ref could not be re-expressed as a loop message ref.
    #[error("invalid steering message ref: {0}")]
    InvalidMessageRef(String),
    /// Reading the active run state failed.
    #[error("active run state read failed: {0}")]
    RunState(#[source] TurnError),
    /// The named active run no longer exists in scope or is already terminal —
    /// nothing will ever drain the queue, so callers fall back to the
    /// `RejectedBusy` outcome instead of stranding the message `Queued`.
    #[error("active run is terminal or gone; steering input has no consumer")]
    ActiveRunGone,
    /// The active run's resolved profile forbids mid-run steering
    /// (`SteeringPolicy::allow_steering = false`); callers fall back to the
    /// `RejectedBusy` outcome exactly like the no-queue mode.
    #[error("steering is disallowed for the active run's profile")]
    SteeringDisallowed,
    /// The host input queue rejected the enqueue.
    #[error("steering enqueue failed: {0}")]
    Enqueue(#[source] HostInputQueueError),
}

/// Enqueue `accepted_message_ref` as steering input for the busy `active_run_id`.
///
/// Resolves the active run's turn id, builds the loop message ref, and hands the
/// queued-message request (carrying the originating thread message identity) to
/// the host input queue. The queue is responsible for transitioning that thread
/// message to `submitted` once the input is consumed; this gateway does not
/// touch transcript status, leaving the queued/replay reconciliation to the
/// caller that owns the message-resolution strategy.
// arch-exempt: too_many_args, leaf gateway passes through a caller-owned scope/identity tuple, plan #5347
#[allow(clippy::too_many_arguments)]
pub(crate) async fn enqueue_busy_steering<C>(
    turn_coordinator: &C,
    input_enqueue: &dyn HostInputEnqueuePort,
    turn_scope: TurnScope,
    thread_scope: ThreadScope,
    thread_id: ThreadId,
    message_id: ThreadMessageId,
    accepted_message_ref: &AcceptedMessageRef,
    active_run_id: TurnRunId,
) -> Result<(), SteeringEnqueueError>
where
    C: TurnCoordinator + ?Sized,
{
    let active_run = match turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope: turn_scope,
            run_id: active_run_id,
        })
        .await
    {
        Ok(state) => state,
        // The run named by the busy rejection (or the queued replay row) is
        // gone from this scope: nothing can drain the queue for it.
        Err(TurnError::ScopeNotFound) => return Err(SteeringEnqueueError::ActiveRunGone),
        Err(error) => return Err(SteeringEnqueueError::RunState(error)),
    };
    if active_run.status.is_terminal() {
        return Err(SteeringEnqueueError::ActiveRunGone);
    }
    if !active_run.allow_steering {
        return Err(SteeringEnqueueError::SteeringDisallowed);
    }
    let message_ref = LoopMessageRef::new(accepted_message_ref.as_str().to_string())
        .map_err(|e| SteeringEnqueueError::InvalidMessageRef(e.to_string()))?;
    input_enqueue
        .enqueue_queued_message(EnqueueQueuedMessageRequest {
            run_id: active_run_id,
            turn_id: active_run.turn_id,
            scope: thread_scope,
            thread_id,
            message_id,
            input: LoopInput::Steering { message_ref },
        })
        .await
        .map_err(SteeringEnqueueError::Enqueue)?;
    Ok(())
}
