//! Host-owned input queue contract for Reborn loop input ports.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::ids::ThreadId;
use ironclaw_loop_contracts::{LoopInput, LoopInputAckToken, LoopInputCursorToken};
use ironclaw_threads::{SessionThreadService, ThreadMessageId, ThreadScope};
use ironclaw_turns::{TurnId, TurnRunId};
use thiserror::Error;

/// Host-owned input queue surface.
///
/// The host runtime exposes one implementation backed by its actual
/// user-input, steering, and followup substrate. `HostQueueLoopInputPort`
/// adapts this surface to the `LoopInputPort` contract the loop calls.
///
/// Cursor semantics:
///
/// - Tokens are opaque to the loop. Implementations may use a monotonic
///   sequence, generation token, or compound key. `next_after` must return the
///   first input strictly after `after`, or an equivalent origin point for a
///   run-start cursor. Implementations must reject malformed, foreign, or
///   unissued future cursor tokens for the bound run instead of treating them
///   as empty positions.
/// - Cursors are read positions, not ack identities. Acking is by exact
///   per-input token so control inputs cannot be skipped by cursor-through ack.
/// - `ack_consumed` is at-most-once. Acking the same token twice is a no-op.
/// - Polled but unacked inputs are redeliverable when the caller polls again
///   from the same prior cursor.
///
/// Implementations are per host process. Each adapter binds to one run at host
/// build time; cross-run polls are rejected by the adapter before reaching the
/// queue.
#[async_trait]
pub trait HostInputQueue: Send + Sync {
    async fn next_after(
        &self,
        run_id: TurnRunId,
        after: LoopInputCursorToken,
        limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError>;

    async fn ack_consumed(
        &self,
        run_id: TurnRunId,
        tokens: Vec<LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError>;

    /// Terminal reconciliation: consume every input still queued for `run_id`
    /// and flip each bound transcript row `Queued` → `RejectedBusy` (resend
    /// affordance) — never auto-resubmitting. Called after the run reaches a
    /// terminal state with the queue undrained (e.g. a cancel before the loop's
    /// next drain). Consumed entries are marked acked first, so a still-running
    /// loop that races this call can no longer drain them; a row the loop
    /// already consumed (`Submitted`) is left untouched. Idempotent.
    ///
    /// Returns the message ids whose rows were flipped.
    async fn reject_unconsumed(
        &self,
        run_id: TurnRunId,
    ) -> Result<Vec<ThreadMessageId>, HostInputQueueError>;
}

/// Raw queue batch returned by a host queue implementation.
///
/// The adapter wraps `next_cursor` into a `LoopInputCursor` scoped to the
/// bound run context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInputBatch {
    pub inputs: Vec<HostInputEnvelope>,
    pub next_cursor: LoopInputCursorToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInputEnvelope {
    pub input: LoopInput,
    pub cursor: LoopInputCursorToken,
    pub ack_token: LoopInputAckToken,
}

#[derive(Debug, Error)]
pub enum HostInputQueueError {
    #[error("input queue unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("cursor invalid for run: {reason}")]
    InvalidCursor { reason: String },
    /// Steering is deliberately not wired for this runtime (the
    /// [`RejectingInputEnqueue`] null port). Distinct from [`Self::Unavailable`]
    /// so callers can fall back to the rejected-busy outcome ONLY for the
    /// disabled mode, while genuine durable-I/O failures surface as retryable
    /// errors instead of masquerading as successful rejections.
    #[error("input queue is not wired for this runtime")]
    Disabled,
    #[error("input queue internal error")]
    Internal,
}

#[async_trait]
pub trait HostInputEnqueuePort: Send + Sync {
    /// Enqueue a user message as steering/followup input for an active run.
    ///
    /// The request carries the originating thread message identity so the queue
    /// can transition that message to `submitted` once the input is consumed.
    /// That transition is BEST-EFFORT transcript bookkeeping: input
    /// consumption/ack is never rolled back when the status write fails (a
    /// stale `Queued` badge is reconcilable; a dead run is not), and the
    /// failure is logged at debug level. There is deliberately no
    /// metadata-free variant: every enqueued input is backed by a thread
    /// message, so the transition can never be silently *omitted*.
    async fn enqueue_queued_message(
        &self,
        request: EnqueueQueuedMessageRequest,
    ) -> Result<HostInputEnvelope, HostInputQueueError>;
}

/// Null-object enqueue port used as the default when a host has not wired a
/// real input queue. Every enqueue fails closed with the distinct
/// [`HostInputQueueError::Disabled`] rather than silently dropping the
/// message (or conflating "steering off" with an operational failure). Production runtimes always replace this with
/// the host-owned queue; it exists so callers can hold a non-optional
/// `Arc<dyn HostInputEnqueuePort>` instead of an `Option` that production never
/// leaves unset.
#[derive(Debug, Default, Clone, Copy)]
pub struct RejectingInputEnqueue;

#[async_trait]
impl HostInputEnqueuePort for RejectingInputEnqueue {
    async fn enqueue_queued_message(
        &self,
        _request: EnqueueQueuedMessageRequest,
    ) -> Result<HostInputEnvelope, HostInputQueueError> {
        Err(HostInputQueueError::Disabled)
    }
}

#[derive(Debug, Clone)]
pub struct EnqueueQueuedMessageRequest {
    pub run_id: TurnRunId,
    pub turn_id: TurnId,
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub message_id: ThreadMessageId,
    pub input: LoopInput,
}

fn poisoned_lock(operation: &'static str) -> HostInputQueueError {
    // Carry the diagnosis to the log before collapsing to the sanitized
    // variant (`.claude/rules/error-handling.md`: never a bare `map_err(|_|)`).
    tracing::debug!(
        component = "host_input_queue",
        operation,
        "input queue state lock poisoned"
    );
    HostInputQueueError::Internal
}

#[derive(Clone)]
struct QueuedMessageStatusUpdate {
    turn_id: TurnId,
    scope: ThreadScope,
    thread_id: ThreadId,
    message_id: ThreadMessageId,
}

pub struct InMemoryHostInputQueue {
    state: Arc<Mutex<InMemoryHostInputQueueState>>,
    thread_service: Arc<dyn SessionThreadService>,
}

impl std::fmt::Debug for InMemoryHostInputQueue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InMemoryHostInputQueue")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Default)]
struct InMemoryHostInputQueueState {
    runs: HashMap<TurnRunId, InMemoryRunInputQueue>,
}

impl std::fmt::Debug for InMemoryHostInputQueueState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InMemoryHostInputQueueState")
            .finish()
    }
}

struct InMemoryRunInputQueue {
    entries: Vec<InMemoryInputEntry>,
    /// Compact ack state: every sequence `<= acked_watermark` is acked, plus
    /// the sparse out-of-order set above it. Bounded by in-flight inputs, not
    /// by queue lifetime — the tombstone-per-ack shape grew without bound.
    acked_watermark: u64,
    acked_above: std::collections::BTreeSet<u64>,
    /// Next sequence to issue. Starts at 1 so the origin cursor (sequence 0)
    /// stays the unique run-start position and every real input is strictly
    /// after it.
    next_sequence: u64,
}

impl Default for InMemoryRunInputQueue {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            acked_watermark: 0,
            acked_above: std::collections::BTreeSet::new(),
            next_sequence: 1,
        }
    }
}

impl InMemoryRunInputQueue {
    fn is_acked(&self, sequence: u64) -> bool {
        sequence <= self.acked_watermark || self.acked_above.contains(&sequence)
    }

    fn record_ack(&mut self, sequence: u64) {
        if sequence <= self.acked_watermark {
            return;
        }
        self.acked_above.insert(sequence);
        while self.acked_above.remove(&(self.acked_watermark + 1)) {
            self.acked_watermark += 1;
        }
    }
}

#[derive(Clone)]
struct InMemoryInputEntry {
    sequence: u64,
    envelope: HostInputEnvelope,
    queued_message: Option<QueuedMessageStatusUpdate>,
}

impl InMemoryHostInputQueue {
    pub fn new(thread_service: Arc<dyn SessionThreadService>) -> Self {
        Self {
            state: Arc::new(Mutex::new(InMemoryHostInputQueueState::default())),
            thread_service,
        }
    }

    /// Enqueue `input` for `run_id`, attaching `queued_message` status metadata.
    ///
    /// Identical inputs already queued for the run are deduplicated; the first
    /// status binding for an entry wins.
    fn enqueue_with(
        &self,
        run_id: TurnRunId,
        input: LoopInput,
        queued_message: QueuedMessageStatusUpdate,
    ) -> Result<HostInputEnvelope, HostInputQueueError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| HostInputQueueError::Internal)?;
        let queue = state.runs.entry(run_id).or_default();
        if let Some(existing) = queue
            .entries
            .iter_mut()
            .find(|entry| entry.envelope.input == input)
        {
            existing.queued_message.get_or_insert(queued_message);
            return Ok(existing.envelope.clone());
        }
        let sequence = queue.next_sequence;
        queue.next_sequence = queue.next_sequence.saturating_add(1);
        let envelope = HostInputEnvelope {
            input,
            cursor: cursor_token(sequence)?,
            ack_token: ack_token(sequence)?,
        };
        queue.entries.push(InMemoryInputEntry {
            sequence,
            envelope: envelope.clone(),
            queued_message: Some(queued_message),
        });
        Ok(envelope)
    }
}

#[async_trait]
impl HostInputEnqueuePort for InMemoryHostInputQueue {
    async fn enqueue_queued_message(
        &self,
        request: EnqueueQueuedMessageRequest,
    ) -> Result<HostInputEnvelope, HostInputQueueError> {
        let EnqueueQueuedMessageRequest {
            run_id,
            turn_id,
            scope,
            thread_id,
            message_id,
            input,
        } = request;
        self.enqueue_with(
            run_id,
            input,
            QueuedMessageStatusUpdate {
                turn_id,
                scope,
                thread_id,
                message_id,
            },
        )
    }
}

#[async_trait]
impl HostInputQueue for InMemoryHostInputQueue {
    async fn next_after(
        &self,
        run_id: TurnRunId,
        after: LoopInputCursorToken,
        limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        let after_sequence = cursor_sequence(&after)?;
        let state = self.state.lock().map_err(|_| poisoned_lock("next_after"))?;
        let Some(queue) = state.runs.get(&run_id) else {
            return Ok(HostInputBatch {
                inputs: Vec::new(),
                next_cursor: after,
            });
        };
        // `next_sequence` is the next-to-issue value, so the max issued cursor
        // is `next_sequence - 1`; anything at or past `next_sequence` is an
        // unissued FUTURE cursor and is rejected instead of read as empty.
        if after_sequence >= queue.next_sequence {
            return Err(HostInputQueueError::InvalidCursor {
                reason: "input cursor is ahead of the run input queue".to_string(),
            });
        }
        // Strictly-after semantics: an entry's own cursor is its sequence, so
        // polling from that cursor never returns the entry again.
        let mut inputs = Vec::new();
        let mut cursor_sequence_out = after_sequence;
        for entry in queue
            .entries
            .iter()
            .filter(|entry| entry.sequence > after_sequence)
        {
            if queue.is_acked(entry.sequence) {
                cursor_sequence_out = entry.sequence;
                continue;
            }
            if inputs.len() >= limit {
                break;
            }
            inputs.push(entry.envelope.clone());
            cursor_sequence_out = entry.sequence;
        }
        let next_cursor = if cursor_sequence_out == 0 {
            LoopInputCursorToken::origin()
        } else {
            cursor_token(cursor_sequence_out)?
        };
        Ok(HostInputBatch {
            inputs,
            next_cursor,
        })
    }

    async fn ack_consumed(
        &self,
        run_id: TurnRunId,
        tokens: Vec<LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        // Record acks and prune consumed entries UNDER the lock, before any
        // await: a concurrent `next_after` must never observe a consumed input
        // as unacked and redeliver it while the (async) transcript flip below
        // is still in flight.
        let updates = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| poisoned_lock("ack_consumed"))?;
            let Some(queue) = state.runs.get_mut(&run_id) else {
                return Ok(());
            };
            let mut updates = Vec::new();
            let mut sequences_to_ack = Vec::new();
            for token in tokens {
                let sequence = ack_sequence(&token)?;
                if queue.is_acked(sequence) {
                    continue;
                }
                // Fail loud on a token that matches no live entry and is not
                // already acked. Committing an unknown sequence would poison
                // state: a later entry minted for that sequence would be
                // skipped forever by `next_after`. A redelivered ack lands in
                // `is_acked` above; anything else is a genuine fault
                // (stale/forged token), not a no-op.
                let Some(entry) = queue
                    .entries
                    .iter()
                    .find(|entry| entry.envelope.ack_token == token)
                else {
                    return Err(HostInputQueueError::InvalidCursor {
                        reason: "ack token references an input that is neither live nor already \
                                 acked for this run"
                            .to_string(),
                    });
                };
                if let Some(update) = &entry.queued_message {
                    updates.push(update.clone());
                }
                sequences_to_ack.push(sequence);
            }
            for sequence in &sequences_to_ack {
                queue.record_ack(*sequence);
            }
            // Drop the consumed entries' payloads (`LoopInput` + `ThreadScope`
            // binding) to bound per-run memory over a long-lived run; the
            // compact watermark/sparse-set ack state keeps duplicate acks
            // idempotent without a tombstone per input.
            queue
                .entries
                .retain(|entry| !sequences_to_ack.contains(&entry.sequence));
            updates
        };
        // The queued-message status flip (`Queued` → `Submitted`) is
        // BEST-EFFORT bookkeeping for the transcript badge, NOT part of
        // consuming the input (see the `HostInputEnqueuePort` contract). The
        // input has already been drained and delivered to the model by the
        // time we ack; failing the ack here would map to a terminal
        // `HostUnavailable` and kill the whole run for a cosmetic status write
        // (see `.claude/rules/agent-loop-capabilities.md`, Invariant 1). So a
        // status-update failure is logged with its cause and swallowed — the
        // ack has already advanced so the input is never redelivered. A stale
        // "queued" badge is reconcilable; a dead run is not.
        for update in updates {
            if let Err(source) = self
                .thread_service
                .mark_message_submitted(
                    &update.scope,
                    &update.thread_id,
                    update.message_id,
                    update.turn_id.to_string(),
                    run_id.to_string(),
                )
                .await
            {
                tracing::debug!(
                    component = "host_input_queue",
                    operation = "mark_message_submitted",
                    %run_id,
                    thread_id = %update.thread_id,
                    message_id = %update.message_id,
                    error = %source,
                    "queued-message status flip failed after the input was consumed; \
                     already acked so the run continues (transcript badge may lag)"
                );
            }
        }
        Ok(())
    }

    async fn reject_unconsumed(
        &self,
        run_id: TurnRunId,
    ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
        // Claim every live entry under the lock FIRST (mark acked + prune), so
        // a racing drain can no longer deliver them; then flip the transcript
        // rows outside the lock. Whoever claims an entry first wins — a row the
        // loop already consumed rejects the `RejectedBusy` transition and is
        // skipped best-effort.
        let updates = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| poisoned_lock("reject_unconsumed"))?;
            // The run is terminal: remove its whole queue entry. A late
            // duplicate ack for a claimed input finds no queue and is a no-op;
            // nothing else may enqueue for a terminal run. This is also the
            // in-memory lifetime bound — completed queues do not accumulate
            // for the daemon's lifetime.
            let Some(queue) = state.runs.remove(&run_id) else {
                return Ok(Vec::new());
            };
            queue
                .entries
                .iter()
                .filter(|entry| !queue.is_acked(entry.sequence))
                .filter_map(|entry| entry.queued_message.clone())
                .collect::<Vec<_>>()
        };
        let mut rejected = Vec::new();
        for update in updates {
            match self
                .thread_service
                .mark_message_rejected_busy(&update.scope, &update.thread_id, update.message_id)
                .await
            {
                Ok(_) => rejected.push(update.message_id),
                Err(source) => {
                    // Best-effort: the run is already terminal; a row the loop
                    // consumed first (`Submitted`) legitimately rejects this
                    // transition, and any other failure must not fail the
                    // caller's terminal transition.
                    tracing::debug!(
                        component = "host_input_queue",
                        operation = "reject_unconsumed",
                        %run_id,
                        error = %source,
                        "queued-message reject skipped during terminal reconciliation"
                    );
                }
            }
        }
        Ok(rejected)
    }
}

// The cursor/ack token helpers below are shared with the durable queue
// (`durable_input_queue.rs`) so both backends speak the identical
// `input-cursor:{n}` / `input-ack:{n}` token wire format. A durable queue
// rehydrated after restart must mint the same tokens the loop's persisted
// input cursor already references, so this format is the single source of truth.
pub(crate) fn cursor_sequence(token: &LoopInputCursorToken) -> Result<u64, HostInputQueueError> {
    if token.is_origin() {
        return Ok(0);
    }
    token
        .as_str()
        .strip_prefix("input-cursor:")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| HostInputQueueError::InvalidCursor {
            reason: "input cursor token is malformed".to_string(),
        })
}

pub(crate) fn cursor_token(sequence: u64) -> Result<LoopInputCursorToken, HostInputQueueError> {
    LoopInputCursorToken::new(format!("input-cursor:{sequence}"))
        .map_err(|_| HostInputQueueError::Internal)
}

pub(crate) fn ack_token(sequence: u64) -> Result<LoopInputAckToken, HostInputQueueError> {
    LoopInputAckToken::new(format!("input-ack:{sequence}"))
        .map_err(|_| HostInputQueueError::Internal)
}

pub(crate) fn ack_sequence(token: &LoopInputAckToken) -> Result<u64, HostInputQueueError> {
    token
        .as_str()
        .strip_prefix("input-ack:")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| HostInputQueueError::InvalidCursor {
            reason: "input ack token is malformed".to_string(),
        })
}
