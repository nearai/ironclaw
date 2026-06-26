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
    async fn enqueue_input(
        &self,
        run_id: TurnRunId,
        input: LoopInput,
    ) -> Result<HostInputEnvelope, HostInputQueueError>;

    async fn enqueue_queued_message(
        &self,
        request: EnqueueQueuedMessageRequest,
    ) -> Result<HostInputEnvelope, HostInputQueueError> {
        self.enqueue_input(request.run_id, request.input).await
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

#[derive(Default)]
pub struct InMemoryHostInputQueue {
    state: Arc<Mutex<InMemoryHostInputQueueState>>,
    thread_service: Option<Arc<dyn SessionThreadService>>,
}

impl std::fmt::Debug for InMemoryHostInputQueue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InMemoryHostInputQueue")
            .field("state", &self.state)
            .field("thread_service", &self.thread_service.is_some())
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_thread_service(thread_service: Arc<dyn SessionThreadService>) -> Self {
        Self {
            state: Arc::new(Mutex::new(InMemoryHostInputQueueState::default())),
            thread_service: Some(thread_service),
        }
    }
}

#[async_trait]
impl HostInputEnqueuePort for InMemoryHostInputQueue {
    async fn enqueue_input(
        &self,
        run_id: TurnRunId,
        input: LoopInput,
    ) -> Result<HostInputEnvelope, HostInputQueueError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| HostInputQueueError::Internal)?;
        let queue = state.runs.entry(run_id).or_default();
        if let Some(existing) = queue
            .entries
            .iter()
            .find(|entry| entry.envelope.input == input)
        {
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
            queued_message: None,
        });
        Ok(envelope)
    }

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
            if existing.queued_message.is_none() {
                existing.queued_message = Some(QueuedMessageStatusUpdate {
                    turn_id,
                    scope,
                    thread_id,
                    message_id,
                });
            }
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
            queued_message: Some(QueuedMessageStatusUpdate {
                turn_id,
                scope,
                thread_id,
                message_id,
            }),
        });
        Ok(envelope)
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
        let updates = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| HostInputQueueError::Internal)?;
            let queue = state.runs.entry(run_id).or_default();
            let mut updates = Vec::new();
            for token in tokens {
                let newly_acked = queue.acked.insert(token.clone());
                if !newly_acked {
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
            }
            updates
        };
        if let Some(thread_service) = &self.thread_service {
            for update in updates {
                thread_service
                    .mark_message_submitted(
                        &update.scope,
                        &update.thread_id,
                        update.message_id,
                        update.turn_id.to_string(),
                        run_id.to_string(),
                    )
                    .await
                    .map_err(|_| HostInputQueueError::Internal)?;
            }
        }
        Ok(())
    }
}

fn cursor_sequence(token: &LoopInputCursorToken) -> Result<u64, HostInputQueueError> {
    if token.as_str() == "input-cursor:origin" {
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

fn cursor_token(sequence: u64) -> Result<LoopInputCursorToken, HostInputQueueError> {
    LoopInputCursorToken::new(format!("input-cursor:{sequence}"))
        .map_err(|_| HostInputQueueError::Internal)
}

fn ack_token(sequence: u64) -> Result<LoopInputAckToken, HostInputQueueError> {
    LoopInputAckToken::new(format!("input-ack:{sequence}"))
        .map_err(|_| HostInputQueueError::Internal)
}
