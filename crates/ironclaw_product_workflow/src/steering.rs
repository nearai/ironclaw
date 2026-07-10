//! Shared busy-run steering enqueue gateway.
//!
//! Both the product inbound-turn path ([`crate::inbound_turn`]) and the WebUI
//! facade ([`crate::reborn_services`]) enqueue a user message as steering input
//! when the target run is busy. This module owns the single enqueue sequence so
//! the two callers cannot drift on ordering, idempotency, or error fidelity.

use ironclaw_host_api::ThreadId;
use ironclaw_loop_support::{
    EnqueueQueuedMessageRequest, HostInputEnqueuePort, HostInputQueueError,
};
use ironclaw_threads::{ThreadMessageId, ThreadScope};
use ironclaw_turns::{
    AcceptedMessageRef, GetRunStateRequest, LoopMessageRef, TurnCoordinator, TurnError, TurnRunId,
    TurnScope, run_profile::LoopInput,
};

/// Failure surface of [`enqueue_busy_steering`].
///
/// Each variant maps to a distinct caller-facing error so neither the inbound
/// path nor the WebUI facade collapses an enqueue failure into a generic,
/// cause-less error.
#[derive(Debug)]
pub(crate) enum SteeringEnqueueError {
    /// The accepted message ref could not be re-expressed as a loop message ref.
    InvalidMessageRef(String),
    /// Reading the active run state failed.
    RunState(TurnError),
    /// The host input queue rejected the enqueue.
    Enqueue(HostInputQueueError),
}

/// Enqueue `accepted_message_ref` as steering input for the busy `active_run_id`.
///
/// Resolves the active run's turn id, builds the loop message ref, and hands the
/// queued-message request (carrying the originating thread message identity) to
/// the host input queue. The queue is responsible for transitioning that thread
/// message to `submitted` once the input is consumed; this gateway does not
/// touch transcript status, leaving the queued/replay reconciliation to the
/// caller that owns the message-resolution strategy.
#[allow(clippy::too_many_arguments)]
// arch-exempt: too_many_args, leaf gateway passes through caller-owned scope +
// identity tuple with no natural aggregate type to bundle, plan #5347
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
    let active_run = turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope: turn_scope,
            run_id: active_run_id,
        })
        .await
        .map_err(SteeringEnqueueError::RunState)?;
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
