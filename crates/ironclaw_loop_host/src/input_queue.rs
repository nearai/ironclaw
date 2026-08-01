//! Host-owned input queue contract for Reborn loop input ports, plus the ONE
//! queue model both backends share.
//!
//! The cursor/ack semantics live in [`RunQueueModel`], a pure serde-able value
//! type: the in-memory backend keeps it in a per-run map under a mutex, the
//! durable backend persists it verbatim as the per-run JSON document behind a
//! CAS loop. Fixing a queue-semantics bug therefore happens in exactly one
//! place, and the two backends cannot drift.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::ids::ThreadId;
use ironclaw_loop_contracts::{LoopInput, LoopInputAckToken, LoopInputCursorToken};
use ironclaw_threads::{SessionThreadService, ThreadMessageId, ThreadScope};
use ironclaw_turns::{TurnId, TurnRunId};
use serde::{Deserialize, Serialize};
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

/// Host-side terminal reconciliation surface, separate from the loop-facing
/// drain contract: its only caller is the runner's cancel-time reconciler, so
/// the loop port never gains reject authority and drain-side test doubles need
/// no dead stubs.
#[async_trait]
pub trait HostInputQueueReconcile: Send + Sync {
    /// Terminal reconciliation: consume every input still queued for `run_id`
    /// and flip each bound transcript row `Queued` → `RejectedBusy` (resend
    /// affordance) — never auto-resubmitting. Called after the run reaches a
    /// terminal state with the queue undrained (e.g. a cancel before the
    /// loop's next drain). Consumed entries are claimed first, so a
    /// still-running loop that races this call can no longer drain them; a
    /// row the loop already consumed (`Submitted`) is left untouched.
    /// Idempotent.
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
/// message (or conflating "steering off" with an operational failure).
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

// ---------------------------------------------------------------------------
// The shared queue model
// ---------------------------------------------------------------------------

/// The transcript message bound to a queued input, used to flip its status
/// once the input is consumed (`Submitted`) or terminally reconciled
/// (`RejectedBusy`). One type for both backends; the in-memory backend simply
/// never serializes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct QueuedMessageStatusUpdate {
    pub(crate) turn_id: TurnId,
    pub(crate) scope: ThreadScope,
    pub(crate) thread_id: ThreadId,
    pub(crate) message_id: ThreadMessageId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct QueueEntry {
    pub(crate) sequence: u64,
    pub(crate) input: LoopInput,
    pub(crate) status: QueuedMessageStatusUpdate,
}

fn first_sequence() -> u64 {
    1
}

/// One run's queue state — the single implementation of the cursor/ack
/// semantics documented on [`HostInputQueue`]. Serialized verbatim as the
/// durable backend's per-run JSON document; held in a per-run map by the
/// in-memory backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RunQueueModel {
    /// Next sequence to issue. Starts at 1 so the origin cursor (sequence 0)
    /// stays the unique run-start position and every real input is strictly
    /// after it.
    #[serde(default = "first_sequence")]
    next_sequence: u64,
    entries: Vec<QueueEntry>,
    /// Compact ack state: every sequence `<= acked_watermark` is acked, plus
    /// the sparse out-of-order set above it. Keeps duplicate/redelivered acks
    /// idempotent without a tombstone per input, so the state does not grow
    /// (or, durably, get reparsed and rewritten) without bound over a
    /// long-lived run. `BTreeSet` serializes as a sorted array.
    #[serde(default)]
    acked_watermark: u64,
    #[serde(default)]
    acked_above: BTreeSet<u64>,
}

impl Default for RunQueueModel {
    fn default() -> Self {
        Self {
            next_sequence: first_sequence(),
            entries: Vec::new(),
            acked_watermark: 0,
            acked_above: BTreeSet::new(),
        }
    }
}

/// Outcome of [`RunQueueModel::enqueue_dedup`]: callers only persist when the
/// model actually changed.
pub(crate) struct Enqueued {
    pub(crate) sequence: u64,
    pub(crate) inserted: bool,
}

impl RunQueueModel {
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

    /// Enqueue `input`, deduplicating a retried enqueue of the same message
    /// (steering refs derive from unique message ids, so distinct messages can
    /// never collide). The first status binding for an entry wins.
    pub(crate) fn enqueue_dedup(
        &mut self,
        input: LoopInput,
        status: QueuedMessageStatusUpdate,
    ) -> Enqueued {
        if let Some(existing) = self.entries.iter().find(|entry| entry.input == input) {
            return Enqueued {
                sequence: existing.sequence,
                inserted: false,
            };
        }
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        // Entries stay sequence-ordered by construction: sequences are issued
        // monotonically and only ever appended.
        self.entries.push(QueueEntry {
            sequence,
            input,
            status,
        });
        Enqueued {
            sequence,
            inserted: true,
        }
    }

    /// Strictly-after scan: an entry's own cursor is its sequence, so polling
    /// from that cursor never returns the entry again. `next_sequence` is the
    /// next-to-issue value, so the max issued cursor is `next_sequence - 1`;
    /// anything at or past `next_sequence` is an unissued FUTURE cursor and is
    /// rejected instead of read as an empty position.
    pub(crate) fn scan_after(
        &self,
        after_sequence: u64,
        limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        if after_sequence >= self.next_sequence {
            return Err(HostInputQueueError::InvalidCursor {
                reason: "input cursor is ahead of the run input queue".to_string(),
            });
        }
        let mut inputs = Vec::new();
        let mut cursor_sequence_out = after_sequence;
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.sequence > after_sequence)
        {
            if self.is_acked(entry.sequence) {
                cursor_sequence_out = entry.sequence;
                continue;
            }
            if inputs.len() >= limit {
                break;
            }
            inputs.push(envelope_for(entry.sequence, entry.input.clone())?);
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

    /// Validate `tokens` and record their acks, pruning the consumed entries'
    /// payloads to bound state size. Returns the status bindings of the newly
    /// acked entries (empty when every token was a redelivered duplicate).
    ///
    /// Fails loud on a token that matches no live entry and is not already
    /// acked: committing an unknown sequence would poison state — a later
    /// entry minted for that sequence would be skipped forever by the scan. A
    /// redelivered ack lands in `is_acked`; anything else is a genuine fault
    /// (stale/forged token), not a no-op.
    pub(crate) fn validate_and_ack(
        &mut self,
        tokens: &[LoopInputAckToken],
    ) -> Result<Vec<QueuedMessageStatusUpdate>, HostInputQueueError> {
        let mut updates = Vec::new();
        let mut newly_acked = Vec::new();
        for token in tokens {
            let sequence = ack_sequence(token)?;
            if self.is_acked(sequence) {
                continue;
            }
            let Some(entry) = self.entries.iter().find(|entry| entry.sequence == sequence)
            else {
                return Err(HostInputQueueError::InvalidCursor {
                    reason: format!(
                        "ack token references sequence {sequence} that is neither live nor \
                         already acked for this run"
                    ),
                });
            };
            updates.push(entry.status.clone());
            newly_acked.push(sequence);
        }
        for sequence in &newly_acked {
            self.record_ack(*sequence);
        }
        self.entries
            .retain(|entry| !newly_acked.contains(&entry.sequence));
        Ok(updates)
    }

    /// The status bindings of every entry not yet acked — the set a terminal
    /// reconciliation flips to `RejectedBusy`. Read-only: the caller claims
    /// the whole queue (removes the map entry / deletes the document) rather
    /// than acking piecemeal.
    pub(crate) fn unacked_status_updates(&self) -> Vec<QueuedMessageStatusUpdate> {
        self.entries
            .iter()
            .filter(|entry| !self.is_acked(entry.sequence))
            .map(|entry| entry.status.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Shared transcript-flip helpers (best-effort by contract; see
// `HostInputEnqueuePort::enqueue_queued_message`)
// ---------------------------------------------------------------------------

/// Flip each consumed row `Queued` → `Submitted`. Best-effort: the inputs are
/// already durably acked by the time this runs; failing here would map to a
/// terminal `HostUnavailable` and kill the whole run for a cosmetic status
/// write (see `.claude/rules/agent-loop-capabilities.md`, Invariant 1). A
/// stale "queued" badge is reconcilable; a dead run is not.
pub(crate) async fn flip_submitted(
    thread_service: &dyn SessionThreadService,
    run_id: TurnRunId,
    updates: Vec<QueuedMessageStatusUpdate>,
) {
    for update in updates {
        if let Err(error) = thread_service
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
                %error,
                "queued-message status flip failed after the input was consumed; \
                 already acked so the run continues (transcript badge may lag)"
            );
        }
    }
}

/// Flip each claimed row `Queued` → `RejectedBusy` during terminal
/// reconciliation. Best-effort: a row the loop consumed first (`Submitted`)
/// legitimately rejects this transition and is skipped, and any other failure
/// must not fail the caller's terminal transition. Returns the ids that
/// flipped.
pub(crate) async fn flip_rejected_busy(
    thread_service: &dyn SessionThreadService,
    run_id: TurnRunId,
    updates: Vec<QueuedMessageStatusUpdate>,
) -> Vec<ThreadMessageId> {
    let mut rejected = Vec::new();
    for update in updates {
        match thread_service
            .mark_message_rejected_busy(&update.scope, &update.thread_id, update.message_id)
            .await
        {
            Ok(_) => rejected.push(update.message_id),
            Err(error) => {
                tracing::debug!(
                    component = "host_input_queue",
                    operation = "reject_unconsumed",
                    %run_id,
                    thread_id = %update.thread_id,
                    message_id = %update.message_id,
                    %error,
                    "queued-message reject skipped during terminal reconciliation"
                );
            }
        }
    }
    rejected
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

// ---------------------------------------------------------------------------
// In-memory backend: the shared model in a per-run map under a mutex
// ---------------------------------------------------------------------------

pub struct InMemoryHostInputQueue {
    state: Arc<Mutex<HashMap<TurnRunId, RunQueueModel>>>,
    thread_service: Arc<dyn SessionThreadService>,
}

impl std::fmt::Debug for InMemoryHostInputQueue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InMemoryHostInputQueue")
            .finish_non_exhaustive()
    }
}

impl InMemoryHostInputQueue {
    pub fn new(thread_service: Arc<dyn SessionThreadService>) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            thread_service,
        }
    }
}

#[async_trait]
impl HostInputEnqueuePort for InMemoryHostInputQueue {
    async fn enqueue_queued_message(
        &self,
        request: EnqueueQueuedMessageRequest,
    ) -> Result<HostInputEnvelope, HostInputQueueError> {
        let mut state = self.state.lock().map_err(|_| poisoned_lock("enqueue"))?;
        let enqueued = state.entry(request.run_id).or_default().enqueue_dedup(
            request.input.clone(),
            QueuedMessageStatusUpdate {
                turn_id: request.turn_id,
                scope: request.scope,
                thread_id: request.thread_id,
                message_id: request.message_id,
            },
        );
        envelope_for(enqueued.sequence, request.input)
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
        let Some(model) = state.get(&run_id) else {
            return Ok(HostInputBatch {
                inputs: Vec::new(),
                next_cursor: after,
            });
        };
        model.scan_after(after_sequence, limit)
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
            let Some(model) = state.get_mut(&run_id) else {
                return Ok(());
            };
            model.validate_and_ack(&tokens)?
        };
        flip_submitted(self.thread_service.as_ref(), run_id, updates).await;
        Ok(())
    }
}

#[async_trait]
impl HostInputQueueReconcile for InMemoryHostInputQueue {
    async fn reject_unconsumed(
        &self,
        run_id: TurnRunId,
    ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
        // The run is terminal: remove its whole queue entry. A late duplicate
        // ack for a claimed input finds no queue and is a no-op; nothing else
        // may enqueue for a terminal run. This is also the in-memory lifetime
        // bound — completed queues do not accumulate for the daemon's
        // lifetime.
        let updates = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| poisoned_lock("reject_unconsumed"))?;
            let Some(model) = state.remove(&run_id) else {
                return Ok(Vec::new());
            };
            model.unacked_status_updates()
        };
        Ok(flip_rejected_busy(self.thread_service.as_ref(), run_id, updates).await)
    }
}

// ---------------------------------------------------------------------------
// Cursor/ack token wire format — shared with the durable queue so both
// backends speak the identical `input-cursor:{n}` / `input-ack:{n}` tokens. A
// durable queue rehydrated after restart must mint the same tokens the loop's
// persisted input cursor already references.
// ---------------------------------------------------------------------------

pub(crate) fn envelope_for(
    sequence: u64,
    input: LoopInput,
) -> Result<HostInputEnvelope, HostInputQueueError> {
    Ok(HostInputEnvelope {
        input,
        cursor: cursor_token(sequence)?,
        ack_token: ack_token(sequence)?,
    })
}

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
    LoopInputAckToken::new(format!("input-ack:{sequence}")).map_err(|_| HostInputQueueError::Internal)
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
