//! Busy-run steering admission — the single owner of the queued-message flow.
//!
//! Both the product inbound-turn path ([`crate::inbound_turn`]) and the WebUI
//! facade ([`crate::reborn_services`]) admit a user message that hits a busy
//! thread through this module: mark the row `Queued`, enqueue it as steering
//! input for the active run, and settle it as `RejectedBusy` when steering
//! cannot serve it. The DISPOSITION policy lives here — callers receive a
//! classified [`SteeringAdmission`] and only build their surface-specific
//! response; they never enumerate queue error variants, so the two surfaces
//! cannot drift on ordering, idempotency, settle semantics, or error fidelity.
//!
//! Two entry points cover the two lifecycle moments:
//! - [`admit_busy_steering`] — a FRESH busy submit: the row is `Accepted` and
//!   must be marked `Queued` (reconciling the races a failed mark can hide)
//!   before the input becomes drainable.
//! - [`readmit_queued_steering`] — a REPLAY of an already-`Queued` row
//!   (crash-orphan recovery): the busy path persists `Queued` before
//!   enqueueing, so a crash between the two must not leave the replay
//!   treating the row as proof of enqueue; re-enqueueing is idempotent (the
//!   queue dedups by the message's own ref).

use ironclaw_host_api::ids::ThreadId;
use ironclaw_loop_contracts::LoopInput;
use ironclaw_loop_host::{EnqueueQueuedMessageRequest, HostInputEnqueuePort, HostInputQueueError};
use ironclaw_threads::{
    MessageStatus, SessionThreadError, SessionThreadService, ThreadMessageId, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, GetRunStateRequest, LoopMessageRef, TurnCoordinator, TurnError, TurnRunId,
    TurnRunState, TurnScope,
};

/// One busy-thread message targeting one active run.
pub(crate) struct SteeringAdmissionRequest {
    pub turn_scope: TurnScope,
    pub thread_scope: ThreadScope,
    pub message_id: ThreadMessageId,
    pub accepted_message_ref: AcceptedMessageRef,
    pub active_run_id: TurnRunId,
}

impl SteeringAdmissionRequest {
    /// The thread identity lives on the turn scope; re-passing it separately
    /// would be re-derived identity the compiler cannot keep in agreement.
    fn thread_id(&self) -> &ThreadId {
        &self.turn_scope.thread_id
    }
}

/// Classified admission outcome. Callers translate these into their surface's
/// response types; every settle decision has already been applied here.
pub(crate) enum SteeringAdmission {
    /// The row is `Queued` and the input is (idempotently) enqueued for the
    /// active run, whose state snapshot rides along so callers never need a
    /// second `get_run_state` to build their response.
    Deferred { run: Box<TurnRunState> },
    /// Steering cannot serve this message — the queue is disabled, the run's
    /// profile disallows steering, or the run is terminal/gone — and the row
    /// has been settled `RejectedBusy` (resend affordance).
    Rejected,
}

/// Genuinely fatal admission failures. Settle-cases are never errors: they
/// come back as [`SteeringAdmission::Rejected`], so this enum cannot express
/// "should have been a rejection" and callers need no unreachable arms.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SteeringAdmissionError {
    #[error("invalid steering message ref: {0}")]
    InvalidMessageRef(String),
    #[error("active run state read failed: {0}")]
    RunState(#[source] TurnError),
    #[error("failed to mark message queued: {0}")]
    MarkQueued(#[source] SessionThreadError),
    #[error("failed to settle message as rejected-busy: {0}")]
    SettleRejected(#[source] SessionThreadError),
    #[error("steering enqueue failed: {0}")]
    Enqueue(#[source] HostInputQueueError),
}

/// Parse a stored `turn_run_id` column back into a [`TurnRunId`]. The single
/// spelling of this reconstruction; callers map the error string into their
/// surface's error type.
pub(crate) fn parse_stored_run_id(raw: Option<&str>) -> Result<TurnRunId, String> {
    let raw = raw.ok_or_else(|| "queued replay missing turn_run_id".to_string())?;
    uuid::Uuid::parse_str(raw)
        .map(TurnRunId::from_uuid)
        .map_err(|error| format!("invalid queued turn_run_id: {error}"))
}

/// Fresh busy admission (the `ThreadBusy` submit branches): classify the run,
/// mark the row `Queued` BEFORE the input becomes drainable (so the loop's
/// consumer always observes a deterministic `Queued` → `Submitted`
/// transition), then enqueue. A failed enqueue rolls the row back to
/// `RejectedBusy` so it never sticks in `Queued` with no backing input.
pub(crate) async fn admit_busy_steering<C>(
    turn_coordinator: &C,
    input_enqueue: &dyn HostInputEnqueuePort,
    thread_service: &dyn SessionThreadService,
    request: SteeringAdmissionRequest,
) -> Result<SteeringAdmission, SteeringAdmissionError>
where
    C: TurnCoordinator + ?Sized,
{
    let Some(run) = serviceable_run(turn_coordinator, &request).await? else {
        settle_rejected(thread_service, &request).await?;
        return Ok(SteeringAdmission::Rejected);
    };
    match mark_queued_reconciling(thread_service, &request).await? {
        QueuedMark::Queued => {}
        // A concurrent duplicate lost the race (or a reconciler already
        // flipped the row): it is settled — do not queue a contradicting
        // input on top of it.
        QueuedMark::SettledRejected => return Ok(SteeringAdmission::Rejected),
    }
    match try_enqueue(input_enqueue, &request, &run).await {
        Ok(()) => Ok(SteeringAdmission::Deferred { run: Box::new(run) }),
        Err(EnqueueFailure::Disabled) => {
            settle_rejected(thread_service, &request).await?;
            Ok(SteeringAdmission::Rejected)
        }
        Err(EnqueueFailure::Fatal(error)) => {
            // Best-effort rollback: the `Queued` row has no backing input.
            if let Err(rollback) = thread_service
                .mark_message_rejected_busy(
                    &request.thread_scope,
                    request.thread_id(),
                    request.message_id,
                )
                .await
            {
                tracing::debug!(
                    %rollback,
                    thread_id = %request.thread_id(),
                    message_id = %request.message_id,
                    active_run_id = %request.active_run_id,
                    "failed to roll back queued message to rejected-busy after steering enqueue failure"
                );
            }
            Err(error)
        }
    }
}

/// Replay re-admission for an already-`Queued` row. No mark step (the row is
/// already `Queued`) and no rollback on a fatal enqueue failure — the row
/// stays `Queued` so the NEXT replay retries the re-enqueue.
pub(crate) async fn readmit_queued_steering<C>(
    turn_coordinator: &C,
    input_enqueue: &dyn HostInputEnqueuePort,
    thread_service: &dyn SessionThreadService,
    request: SteeringAdmissionRequest,
) -> Result<SteeringAdmission, SteeringAdmissionError>
where
    C: TurnCoordinator + ?Sized,
{
    let Some(run) = serviceable_run(turn_coordinator, &request).await? else {
        settle_rejected(thread_service, &request).await?;
        return Ok(SteeringAdmission::Rejected);
    };
    match try_enqueue(input_enqueue, &request, &run).await {
        Ok(()) => Ok(SteeringAdmission::Deferred { run: Box::new(run) }),
        Err(EnqueueFailure::Disabled) => {
            settle_rejected(thread_service, &request).await?;
            Ok(SteeringAdmission::Rejected)
        }
        Err(EnqueueFailure::Fatal(error)) => Err(error),
    }
}

/// The active run's state when it can still consume steering input; `None`
/// when the run is gone from scope, terminal, or its resolved profile
/// disallows steering — every case where queueing would strand the message.
async fn serviceable_run<C>(
    turn_coordinator: &C,
    request: &SteeringAdmissionRequest,
) -> Result<Option<TurnRunState>, SteeringAdmissionError>
where
    C: TurnCoordinator + ?Sized,
{
    let run = match turn_coordinator
        .get_run_state(GetRunStateRequest {
            scope: request.turn_scope.clone(),
            run_id: request.active_run_id,
        })
        .await
    {
        Ok(run) => run,
        Err(TurnError::ScopeNotFound) => return Ok(None),
        Err(error) => return Err(SteeringAdmissionError::RunState(error)),
    };
    if run.status.is_terminal() || !run.allow_steering {
        return Ok(None);
    }
    Ok(Some(run))
}

enum QueuedMark {
    Queued,
    SettledRejected,
}

/// Mark the row `Queued`, reconciling a failed transition against what it
/// actually raced with via a point read. Lenient ONLY for the confirmed
/// races: the row is already `Queued`/`Submitted` for this run (a duplicate
/// submit or a fast loop won) or already settled `RejectedBusy`. Every other
/// failure propagates — swallowing it would enqueue a drainable input whose
/// row silently stays `Accepted` (fail-loud, `.claude/rules/error-handling.md`).
async fn mark_queued_reconciling(
    thread_service: &dyn SessionThreadService,
    request: &SteeringAdmissionRequest,
) -> Result<QueuedMark, SteeringAdmissionError> {
    let run_id = request.active_run_id.to_string();
    let original_error = match thread_service
        .mark_message_queued(
            &request.thread_scope,
            request.thread_id(),
            request.message_id,
            run_id.clone(),
        )
        .await
    {
        Ok(_) => return Ok(QueuedMark::Queued),
        Err(error) => error,
    };
    let row = match thread_service
        .read_thread_message(&request.thread_scope, request.thread_id(), request.message_id)
        .await
    {
        Ok(row) => row,
        Err(read_error) => {
            tracing::debug!(
                thread_id = %request.thread_id(),
                message_id = %request.message_id,
                active_run_id = %request.active_run_id,
                %original_error,
                %read_error,
                "queued-status reconciliation read failed after mark_message_queued error"
            );
            return Err(SteeringAdmissionError::MarkQueued(original_error));
        }
    };
    match row {
        Some(row)
            if matches!(row.status, MessageStatus::Queued | MessageStatus::Submitted)
                && row.turn_run_id.as_deref() == Some(run_id.as_str()) =>
        {
            Ok(QueuedMark::Queued)
        }
        Some(row) if row.status == MessageStatus::RejectedBusy => Ok(QueuedMark::SettledRejected),
        _ => {
            tracing::debug!(
                thread_id = %request.thread_id(),
                message_id = %request.message_id,
                active_run_id = %request.active_run_id,
                %original_error,
                "queued-status transition failed and the row was not consumed by the run"
            );
            Err(SteeringAdmissionError::MarkQueued(original_error))
        }
    }
}

async fn settle_rejected(
    thread_service: &dyn SessionThreadService,
    request: &SteeringAdmissionRequest,
) -> Result<(), SteeringAdmissionError> {
    thread_service
        .mark_message_rejected_busy(&request.thread_scope, request.thread_id(), request.message_id)
        .await
        .map(|_| ())
        .map_err(SteeringAdmissionError::SettleRejected)
}

enum EnqueueFailure {
    /// Steering is deliberately not wired for this runtime.
    Disabled,
    Fatal(SteeringAdmissionError),
}

async fn try_enqueue(
    input_enqueue: &dyn HostInputEnqueuePort,
    request: &SteeringAdmissionRequest,
    run: &TurnRunState,
) -> Result<(), EnqueueFailure> {
    let message_ref = LoopMessageRef::new(request.accepted_message_ref.as_str().to_string())
        .map_err(|error| {
            EnqueueFailure::Fatal(SteeringAdmissionError::InvalidMessageRef(error.to_string()))
        })?;
    match input_enqueue
        .enqueue_queued_message(EnqueueQueuedMessageRequest {
            run_id: request.active_run_id,
            turn_id: run.turn_id,
            scope: request.thread_scope.clone(),
            thread_id: request.thread_id().clone(),
            message_id: request.message_id,
            input: LoopInput::Steering { message_ref },
        })
        .await
    {
        Ok(_) => Ok(()),
        Err(HostInputQueueError::Disabled) => Err(EnqueueFailure::Disabled),
        Err(error) => Err(EnqueueFailure::Fatal(SteeringAdmissionError::Enqueue(
            error,
        ))),
    }
}
