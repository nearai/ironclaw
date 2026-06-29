//! Host-owned input queue contract for Reborn loop input ports.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::ThreadId;
use ironclaw_threads::{SessionThreadService, ThreadMessageId, ThreadScope};
use ironclaw_turns::{
    TurnId, TurnRunId,
    run_profile::{LoopInput, LoopInputAckToken, LoopInputCursorToken},
};
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
    #[error("input queue internal error")]
    Internal,
}

#[async_trait]
pub trait HostInputEnqueuePort: Send + Sync {
    /// Enqueue a user message as steering/followup input for an active run.
    ///
    /// The request carries the originating thread message identity so the queue
    /// can transition that message to `submitted` once the input is consumed.
    /// There is deliberately no metadata-free variant: every enqueued input is
    /// backed by a thread message, so the status transition can never be
    /// silently dropped.
    async fn enqueue_queued_message(
        &self,
        request: EnqueueQueuedMessageRequest,
    ) -> Result<HostInputEnvelope, HostInputQueueError>;
}

/// Null-object enqueue port used as the default when a host has not wired a
/// real input queue. Every enqueue fails closed with `Unavailable` rather than
/// silently dropping the message. Production runtimes always replace this with
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
        Err(HostInputQueueError::Unavailable {
            reason: "input queue is not wired for this runtime".to_string(),
        })
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

#[derive(Default)]
struct InMemoryRunInputQueue {
    entries: Vec<InMemoryInputEntry>,
    acked: HashSet<LoopInputAckToken>,
    next_sequence: u64,
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
        let state = self
            .state
            .lock()
            .map_err(|_| HostInputQueueError::Internal)?;
        let Some(queue) = state.runs.get(&run_id) else {
            return Ok(HostInputBatch {
                inputs: Vec::new(),
                next_cursor: after,
            });
        };
        if after_sequence > queue.next_sequence {
            return Err(HostInputQueueError::InvalidCursor {
                reason: "input cursor is ahead of the run input queue".to_string(),
            });
        }
        let mut inputs = Vec::new();
        let mut next_sequence = after_sequence;
        for entry in queue
            .entries
            .iter()
            .filter(|entry| entry.sequence >= after_sequence)
        {
            next_sequence = entry.sequence.saturating_add(1);
            if queue.acked.contains(&entry.envelope.ack_token) {
                continue;
            }
            if inputs.len() >= limit {
                next_sequence = entry.sequence;
                break;
            }
            inputs.push(entry.envelope.clone());
        }
        Ok(HostInputBatch {
            inputs,
            next_cursor: cursor_token(next_sequence)?,
        })
    }

    async fn ack_consumed(
        &self,
        run_id: TurnRunId,
        tokens: Vec<LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        let (tokens_to_ack, updates) = {
            let state = self
                .state
                .lock()
                .map_err(|_| HostInputQueueError::Internal)?;
            let Some(queue) = state.runs.get(&run_id) else {
                return Ok(());
            };
            let mut tokens_to_ack = Vec::new();
            let mut updates = Vec::new();
            for token in tokens {
                if queue.acked.contains(&token) {
                    continue;
                }
                if let Some(entry) = queue
                    .entries
                    .iter()
                    .find(|entry| entry.envelope.ack_token == token)
                    && let Some(update) = &entry.queued_message
                {
                    updates.push(update.clone());
                }
                tokens_to_ack.push(token);
            }
            (tokens_to_ack, updates)
        };
        // The queued-message status flip (`Queued` → `Submitted`) is best-effort
        // bookkeeping for the transcript badge, NOT part of consuming the input.
        // The input has already been drained and delivered to the model by the
        // time we ack; failing the ack here would map to a terminal
        // `HostUnavailable` and kill the whole run for a cosmetic status write
        // (see `.claude/rules/agent-loop-capabilities.md`, Invariant 1). So a
        // status-update failure is logged with its cause and swallowed — the ack
        // still advances so the input is never redelivered. A stale "queued"
        // badge is reconcilable; a dead run is not.
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
                tracing::warn!(
                    component = "host_input_queue",
                    operation = "mark_message_submitted",
                    %run_id,
                    error = %source,
                    "queued-message status flip failed after the input was consumed; \
                     acking anyway so the run continues (transcript badge may lag)"
                );
            }
        }
        let acked_now: HashSet<LoopInputAckToken> = tokens_to_ack.iter().cloned().collect();
        let mut state = self
            .state
            .lock()
            .map_err(|_| HostInputQueueError::Internal)?;
        let queue = state.runs.entry(run_id).or_default();
        for token in tokens_to_ack {
            queue.acked.insert(token);
        }
        // Drop the consumed entries' payloads (`LoopInput` + `ThreadScope`
        // binding) to bound per-run memory over a long-lived run. The ack token
        // stays in `acked` so a duplicate/redelivered ack is still skipped
        // idempotently by the guard above; `next_sequence` is a separate
        // high-water mark, so removing entries never lets a stale cursor look
        // "ahead of the queue".
        queue
            .entries
            .retain(|entry| !acked_now.contains(&entry.envelope.ack_token));
        Ok(())
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
