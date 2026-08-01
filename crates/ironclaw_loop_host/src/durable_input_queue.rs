//! Durable, filesystem-backed host input queue.
//!
//! [`InMemoryHostInputQueue`](crate::InMemoryHostInputQueue) keeps queued
//! steering inputs in a process-local map, so a daemon restart drops any
//! message that was queued-but-not-yet-consumed while the owning run is resumed
//! from its durable checkpoint — the message stays `Queued` in the transcript
//! forever and is never delivered.
//!
//! [`FilesystemHostInputQueue`] persists each run's queue as a single
//! CAS-guarded JSON document under the run-scoped filesystem, so the queue
//! survives restart and the resumed run drains it exactly as before. The
//! document stores per-entry *sequences* (not the opaque cursor/ack tokens);
//! the tokens are reconstructed deterministically from the sequence via the
//! shared helpers in [`crate::input_queue`], so the loop's persisted input
//! cursor stays valid across a restart.
//!
//! Scope preservation: the queue document is written through a
//! [`ScopedFilesystem`] under the owner [`ResourceScope`] the composition
//! passes at construction (built from the run's tenant / user / agent /
//! project). In multi-tenant composition the mount-view resolver rewrites that
//! scope into the virtual path prefix (`/tenants/<tenant>/users/<user>/…`), so
//! the record *is* tenant/user-partitioned at the storage boundary — the scope
//! is not dropped. The path itself is then keyed by the globally-unique
//! `run_id` (a UUID), which guarantees no cross-run or cross-tenant collision
//! and lets the resumed run find its own queue. The per-message [`ThreadScope`]
//! that drives the `Queued → Submitted` status flip travels in the record
//! payload ([`DurableStatusUpdate`]).
//!
//! What is *deferred*: finer per-run path granularity inside that owner scope
//! (e.g. a per-thread subtree). The `HostInputQueue` trait methods
//! (`next_after`, `ack_consumed`) receive only `run_id`, not a scope, so
//! per-run path partitioning would need either a `run_id → scope` map or a
//! trait change. `run_id` uniqueness makes that unnecessary for correctness or
//! isolation, so it is intentionally left out here.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};
use ironclaw_loop_contracts::{LoopInput, LoopInputAckToken, LoopInputCursorToken};
use ironclaw_threads::{SessionThreadService, ThreadMessageId, ThreadScope};
use ironclaw_turns::{TurnId, TurnRunId};
use serde::{Deserialize, Serialize};

use crate::input_queue::{
    EnqueueQueuedMessageRequest, HostInputBatch, HostInputEnqueuePort, HostInputEnvelope,
    HostInputQueue, HostInputQueueError, ack_sequence, ack_token, cursor_sequence, cursor_token,
};

/// Bounds the CAS retry loop so persistent contention surfaces as a host error
/// instead of spinning forever. Per-run contention is low (one producer thread
/// enqueuing, one loop thread acking), so a handful of retries is ample.
const MAX_CAS_RETRIES: usize = 8;

/// Durable per-run queue document persisted as JSON at the run's queue path.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DurableRunQueue {
    /// Next sequence to issue. Starts at 1 so the origin cursor (sequence 0)
    /// stays the unique run-start position and every real input is strictly
    /// after it.
    #[serde(default = "first_sequence")]
    next_sequence: u64,
    entries: Vec<DurableEntry>,
    /// Compact ack state: every sequence `<= acked_watermark` is acked, plus
    /// the sparse out-of-order set above it. Keeps duplicate/redelivered acks
    /// idempotent without a tombstone per input, so the document does not grow
    /// (and get reparsed/rewritten) without bound over a long-lived run.
    #[serde(default)]
    acked_watermark: u64,
    #[serde(default)]
    acked_above: Vec<u64>,
}

fn first_sequence() -> u64 {
    1
}

impl Default for DurableRunQueue {
    fn default() -> Self {
        Self {
            next_sequence: first_sequence(),
            entries: Vec::new(),
            acked_watermark: 0,
            acked_above: Vec::new(),
        }
    }
}

impl DurableRunQueue {
    fn is_acked(&self, sequence: u64) -> bool {
        sequence <= self.acked_watermark || self.acked_above.contains(&sequence)
    }

    fn record_ack(&mut self, sequence: u64) {
        if sequence <= self.acked_watermark {
            return;
        }
        if !self.acked_above.contains(&sequence) {
            self.acked_above.push(sequence);
        }
        self.acked_above.sort_unstable();
        while self.acked_above.first() == Some(&(self.acked_watermark + 1)) {
            self.acked_above.remove(0);
            self.acked_watermark += 1;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DurableEntry {
    sequence: u64,
    input: LoopInput,
    status: DurableStatusUpdate,
}

/// The transcript message bound to a queued input, used to flip its status to
/// `Submitted` once the input is consumed.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DurableStatusUpdate {
    turn_id: TurnId,
    scope: ThreadScope,
    thread_id: ironclaw_host_api::ids::ThreadId,
    message_id: ThreadMessageId,
}

/// Filesystem-backed [`HostInputQueue`] / [`HostInputEnqueuePort`].
pub struct FilesystemHostInputQueue<F>
where
    F: RootFilesystem + ?Sized,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    owner_scope: ResourceScope,
    thread_service: Arc<dyn SessionThreadService>,
}

impl<F> std::fmt::Debug for FilesystemHostInputQueue<F>
where
    F: RootFilesystem + ?Sized,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemHostInputQueue")
            .field("owner_scope", &self.owner_scope)
            .finish_non_exhaustive()
    }
}

impl<F> FilesystemHostInputQueue<F>
where
    F: RootFilesystem + ?Sized + 'static,
{
    /// Build a durable queue over `filesystem`, persisting under `owner_scope`.
    /// `thread_service` performs the queued-message status flip on ack.
    pub fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        owner_scope: ResourceScope,
        thread_service: Arc<dyn SessionThreadService>,
    ) -> Self {
        Self {
            filesystem,
            owner_scope,
            thread_service,
        }
    }

    async fn load(
        &self,
        run_id: TurnRunId,
    ) -> Result<(DurableRunQueue, Option<RecordVersion>), HostInputQueueError> {
        let path = queue_path(run_id)?;
        match self.filesystem.get(&self.owner_scope, &path).await {
            Ok(Some(versioned)) => {
                let queue = serde_json::from_slice(&versioned.entry.body).map_err(|error| {
                    HostInputQueueError::Unavailable {
                        reason: format!("durable input queue is corrupt: {error}"),
                    }
                })?;
                Ok((queue, Some(versioned.version)))
            }
            Ok(None) => Ok((DurableRunQueue::default(), None)),
            Err(error) => Err(fs_error(error)),
        }
    }

    /// Persist `queue`, asserting the expected CAS precondition. `version` is
    /// `None` for a first write (`Absent`) and `Some` for an update. A CAS
    /// conflict is reported as [`StorePutError::Conflict`] so callers can retry;
    /// every other failure is [`StorePutError::Fatal`].
    async fn store(
        &self,
        run_id: TurnRunId,
        queue: &DurableRunQueue,
        version: Option<RecordVersion>,
    ) -> Result<(), StorePutError> {
        let body = serde_json::to_vec(queue).map_err(|error| {
            StorePutError::Fatal(HostInputQueueError::Unavailable {
                reason: format!("durable input queue serialization failed: {error}"),
            })
        })?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        let cas = match version {
            Some(version) => CasExpectation::Version(version),
            None => CasExpectation::Absent,
        };
        let path = queue_path(run_id).map_err(StorePutError::Fatal)?;
        match self
            .filesystem
            .put(&self.owner_scope, &path, entry, cas)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(StorePutError::Conflict),
            Err(error) => Err(StorePutError::Fatal(fs_error(error))),
        }
    }
}

/// Outcome of a CAS-guarded durable write.
enum StorePutError {
    /// The CAS precondition failed — a concurrent writer won; retry.
    Conflict,
    /// A non-retryable failure (serialization, backend IO, bad path).
    Fatal(HostInputQueueError),
}

#[async_trait]
impl<F> HostInputEnqueuePort for FilesystemHostInputQueue<F>
where
    F: RootFilesystem + ?Sized + 'static,
{
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
        for _ in 0..MAX_CAS_RETRIES {
            let (mut queue, version) = self.load(run_id).await?;
            // Dedup by input so a retried enqueue of the same message reuses its
            // entry rather than queuing it twice.
            if let Some(existing) = queue.entries.iter().find(|entry| entry.input == input) {
                return envelope_for(existing.sequence, input.clone());
            }
            let sequence = queue.next_sequence;
            queue.next_sequence = queue.next_sequence.saturating_add(1);
            queue.entries.push(DurableEntry {
                sequence,
                input: input.clone(),
                status: DurableStatusUpdate {
                    turn_id,
                    scope: scope.clone(),
                    thread_id: thread_id.clone(),
                    message_id,
                },
            });
            match self.store(run_id, &queue, version).await {
                Ok(()) => return envelope_for(sequence, input),
                Err(StorePutError::Conflict) => continue,
                Err(StorePutError::Fatal(error)) => return Err(error),
            }
        }
        Err(cas_exhausted("enqueue"))
    }
}

#[async_trait]
impl<F> HostInputQueue for FilesystemHostInputQueue<F>
where
    F: RootFilesystem + ?Sized + 'static,
{
    async fn next_after(
        &self,
        run_id: TurnRunId,
        after: LoopInputCursorToken,
        limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        let after_sequence = cursor_sequence(&after)?;
        let (queue, version) = self.load(run_id).await?;
        if version.is_none() {
            return Ok(HostInputBatch {
                inputs: Vec::new(),
                next_cursor: after,
            });
        }
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
        let mut ordered: Vec<&DurableEntry> = queue
            .entries
            .iter()
            .filter(|entry| entry.sequence > after_sequence)
            .collect();
        ordered.sort_by_key(|entry| entry.sequence);
        for entry in ordered {
            if queue.is_acked(entry.sequence) {
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

    async fn ack_consumed(
        &self,
        run_id: TurnRunId,
        tokens: Vec<LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        // Phase 1: durably record the acks (CAS retry). The cursor ack is the
        // load-bearing transition — its failure is a genuine durable-IO fault
        // and is surfaced, so the run does not silently drop a consumed input.
        let mut status_updates = Vec::new();
        let mut committed = false;
        for _ in 0..MAX_CAS_RETRIES {
            let (mut queue, version) = self.load(run_id).await?;
            let Some(version) = version else {
                // No durable queue for this run: nothing to ack.
                return Ok(());
            };
            let mut newly_acked = Vec::new();
            status_updates.clear();
            for token in &tokens {
                let sequence = ack_sequence(token)?;
                if queue.is_acked(sequence) {
                    continue;
                }
                // Fail loud on a token for a sequence that is neither live nor
                // already acked. Committing an unknown sequence into `acked`
                // would poison durable state: when that sequence is eventually
                // enqueued, its (now pre-acked) entry would be skipped forever
                // by `next_after`. A stale/forged token is a genuine fault, not
                // a redelivered ack (which lands in `already` above).
                let Some(entry) = queue.entries.iter().find(|e| e.sequence == sequence) else {
                    return Err(HostInputQueueError::InvalidCursor {
                        reason: format!(
                            "ack token references sequence {sequence} that is neither live \
                             nor already acked for this run"
                        ),
                    });
                };
                status_updates.push(entry.status.clone());
                newly_acked.push(sequence);
            }
            if newly_acked.is_empty() {
                return Ok(());
            }
            for sequence in &newly_acked {
                queue.record_ack(*sequence);
            }
            // Prune consumed entry payloads to bound the document size; the
            // compact watermark/sparse-set ack state keeps duplicate acks
            // idempotent, and `next_sequence` is the high-water mark so a
            // stale cursor never looks "ahead".
            queue
                .entries
                .retain(|entry| !newly_acked.contains(&entry.sequence));
            match self.store(run_id, &queue, Some(version)).await {
                Ok(()) => {
                    committed = true;
                    break;
                }
                Err(StorePutError::Conflict) => continue,
                Err(StorePutError::Fatal(error)) => return Err(error),
            }
        }
        if !committed {
            return Err(cas_exhausted("ack_consumed"));
        }

        // Phase 2: best-effort transcript status flip. The input is already
        // durably acked; a status-write failure must NOT fail the ack (it would
        // map to a terminal HostUnavailable and kill the run — see
        // `.claude/rules/agent-loop-capabilities.md`, Invariant 1). Log and move
        // on; the transcript badge may lag but the run continues.
        for update in status_updates {
            if let Err(error) = self
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
                    component = "durable_host_input_queue",
                    operation = "mark_message_submitted",
                    %run_id,
                    error = %error,
                    "queued-message status flip failed after the input was durably acked; \
                     run continues (transcript badge may lag)"
                );
            }
        }
        Ok(())
    }

    async fn reject_unconsumed(
        &self,
        run_id: TurnRunId,
    ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
        // Phase 1: durably claim every live entry by DELETING the run's queue
        // document under its CAS version — the run is terminal, so nothing may
        // enqueue for it again, a racing drain can no longer load the claimed
        // entries, and a late duplicate ack finds no document (a no-op). This
        // is also the durable lifetime bound: terminal queues do not
        // accumulate documents (or ack state) forever.
        let mut status_updates = Vec::new();
        let mut committed = false;
        for _ in 0..MAX_CAS_RETRIES {
            let (queue, version) = self.load(run_id).await?;
            let Some(version) = version else {
                // No durable queue for this run: nothing to reconcile.
                return Ok(Vec::new());
            };
            status_updates = queue
                .entries
                .iter()
                .filter(|entry| !queue.is_acked(entry.sequence))
                .map(|entry| entry.status.clone())
                .collect();
            let path = queue_path(run_id)?;
            match self
                .filesystem
                .delete_if_version(&self.owner_scope, &path, version)
                .await
            {
                Ok(()) => {
                    committed = true;
                    break;
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_error(error)),
            }
        }
        if !committed {
            return Err(cas_exhausted("reject_unconsumed"));
        }
        if status_updates.is_empty() {
            return Ok(Vec::new());
        }

        // Phase 2: best-effort transcript flip `Queued` → `RejectedBusy`. The
        // entries are already durably claimed; a row the loop consumed first
        // (`Submitted`) legitimately rejects this transition and is skipped,
        // and any other failure must not fail the caller's terminal
        // transition.
        let mut rejected = Vec::new();
        for update in status_updates {
            match self
                .thread_service
                .mark_message_rejected_busy(&update.scope, &update.thread_id, update.message_id)
                .await
            {
                Ok(_) => rejected.push(update.message_id),
                Err(error) => {
                    tracing::debug!(
                        component = "host_input_queue",
                        operation = "reject_unconsumed",
                        %run_id,
                        error = %error,
                        "queued-message reject skipped during terminal reconciliation"
                    );
                }
            }
        }
        Ok(rejected)
    }
}

fn envelope_for(sequence: u64, input: LoopInput) -> Result<HostInputEnvelope, HostInputQueueError> {
    Ok(HostInputEnvelope {
        input,
        cursor: cursor_token(sequence)?,
        ack_token: ack_token(sequence)?,
    })
}

fn queue_path(run_id: TurnRunId) -> Result<ScopedPath, HostInputQueueError> {
    ScopedPath::new(format!("/turns/input-queue/{}.json", run_id.as_uuid())).map_err(|error| {
        HostInputQueueError::Unavailable {
            reason: format!("invalid input queue path: {error}"),
        }
    })
}

fn fs_error(error: FilesystemError) -> HostInputQueueError {
    HostInputQueueError::Unavailable {
        reason: error.to_string(),
    }
}

fn cas_exhausted(operation: &str) -> HostInputQueueError {
    HostInputQueueError::Unavailable {
        reason: format!("durable input queue {operation} contended past retry budget"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        ids::{AgentId, ProjectId, TenantId, ThreadId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };
    use ironclaw_threads::{
        AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
        MessageContent, MessageStatus, ThreadHistoryRequest,
    };
    use ironclaw_turns::{LoopMessageRef, TurnScope};

    fn make_fs(backend: Arc<InMemoryBackend>) -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").unwrap(),
            VirtualPath::new("/turns").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    fn owner_scope() -> ResourceScope {
        TurnScope::new(
            TenantId::new("tenant-iq").unwrap(),
            Some(AgentId::new("agent-iq").unwrap()),
            Some(ProjectId::new("project-iq").unwrap()),
            ThreadId::new("thread-iq").unwrap(),
        )
        .to_resource_scope()
    }

    // ThreadScope carries no thread_id (the thread is addressed separately), so
    // one fixed scope serves both the real-message and ghost-message tests.
    fn ghost_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-iq").unwrap(),
            agent_id: AgentId::new("agent-iq").unwrap(),
            project_id: None,
            owner_user_id: None,
            mission_id: None,
        }
    }

    fn steering(message_ref: &str) -> LoopInput {
        LoopInput::Steering {
            message_ref: LoopMessageRef::new(message_ref).unwrap(),
        }
    }

    fn origin() -> LoopInputCursorToken {
        LoopInputCursorToken::new("input-cursor:origin".to_string()).unwrap()
    }

    #[tokio::test]
    async fn durable_queue_survives_store_reconstruction() {
        // The core durability guarantee: a message queued before a restart is
        // still drainable after, and the reconstructed cursor/ack tokens match
        // the ones the loop's persisted input cursor references.
        let backend = Arc::new(InMemoryBackend::new());
        let thread_service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let run_id = TurnRunId::new();
        let input = steering("msg:restart");

        // First "process": enqueue, then drop the queue object.
        let envelope = {
            let queue = FilesystemHostInputQueue::new(
                make_fs(Arc::clone(&backend)),
                owner_scope(),
                Arc::clone(&thread_service),
            );
            queue
                .enqueue_queued_message(EnqueueQueuedMessageRequest {
                    run_id,
                    turn_id: TurnId::new(),
                    scope: ghost_scope(),
                    thread_id: ThreadId::new("ghost").unwrap(),
                    message_id: ThreadMessageId::new(),
                    input: input.clone(),
                })
                .await
                .expect("enqueue")
        };

        // Second "process" (restart): a brand-new queue object over the SAME
        // durable backend must surface the queued input.
        let queue = FilesystemHostInputQueue::new(
            make_fs(Arc::clone(&backend)),
            owner_scope(),
            thread_service,
        );
        let batch = queue
            .next_after(run_id, origin(), 8)
            .await
            .expect("poll after restart");
        assert_eq!(batch.inputs.len(), 1);
        assert_eq!(batch.inputs[0].input, input);
        assert_eq!(batch.inputs[0].ack_token, envelope.ack_token);
        assert_eq!(batch.inputs[0].cursor, envelope.cursor);
    }

    #[tokio::test]
    async fn enqueue_poll_ack_flips_status_and_stops_redelivery() {
        let backend = Arc::new(InMemoryBackend::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let scope = ghost_scope();
        let thread = thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: None,
                created_by_actor_id: "actor-iq".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let accepted = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-iq".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text("queued steering"),
            })
            .await
            .unwrap();
        let run_id = TurnRunId::new();
        thread_service
            .mark_message_queued(
                &scope,
                &thread.thread_id,
                accepted.message_id,
                run_id.to_string(),
            )
            .await
            .unwrap();

        let queue = FilesystemHostInputQueue::new(
            make_fs(backend),
            owner_scope(),
            Arc::clone(&thread_service) as Arc<dyn SessionThreadService>,
        );
        queue
            .enqueue_queued_message(EnqueueQueuedMessageRequest {
                run_id,
                turn_id: TurnId::new(),
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                message_id: accepted.message_id,
                input: steering(&format!("msg:{}", accepted.message_id)),
            })
            .await
            .expect("enqueue");

        let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
        assert_eq!(batch.inputs.len(), 1);

        queue
            .ack_consumed(run_id, vec![batch.inputs[0].ack_token.clone()])
            .await
            .expect("ack");

        // Status durably flipped to Submitted...
        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope,
                thread_id: thread.thread_id,
            })
            .await
            .unwrap();
        assert_eq!(history.messages[0].status, MessageStatus::Submitted);
        // ...and the consumed input is not redelivered.
        let after = queue
            .next_after(run_id, batch.next_cursor, 8)
            .await
            .expect("poll after ack");
        assert!(after.inputs.is_empty());
    }

    /// Terminal reconciliation (both backends: the durable queue here, the
    /// in-memory queue below shares the helpers): `reject_unconsumed` claims
    /// every undrained entry — flipping its row `Queued` → `RejectedBusy` and
    /// stopping redelivery — while an entry the loop already consumed keeps
    /// its `Submitted` row. Idempotent: a second call reconciles nothing.
    #[tokio::test]
    async fn reject_unconsumed_flips_stranded_rows_and_stops_redelivery() {
        let backend = Arc::new(InMemoryBackend::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let scope = ghost_scope();
        let thread = thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: None,
                created_by_actor_id: "actor-iq".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let run_id = TurnRunId::new();
        let mut message_ids = Vec::new();
        for text in ["consumed before cancel", "stranded by cancel"] {
            let accepted = thread_service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope: scope.clone(),
                    thread_id: thread.thread_id.clone(),
                    actor_id: "actor-iq".into(),
                    source_binding_id: None,
                    reply_target_binding_id: None,
                    external_event_id: None,
                    content: MessageContent::text(text),
                })
                .await
                .unwrap();
            thread_service
                .mark_message_queued(
                    &scope,
                    &thread.thread_id,
                    accepted.message_id,
                    run_id.to_string(),
                )
                .await
                .unwrap();
            message_ids.push(accepted.message_id);
        }

        let queue = FilesystemHostInputQueue::new(
            make_fs(backend),
            owner_scope(),
            Arc::clone(&thread_service) as Arc<dyn SessionThreadService>,
        );
        for message_id in &message_ids {
            queue
                .enqueue_queued_message(EnqueueQueuedMessageRequest {
                    run_id,
                    turn_id: TurnId::new(),
                    scope: scope.clone(),
                    thread_id: thread.thread_id.clone(),
                    message_id: *message_id,
                    input: steering(&format!("msg:{message_id}")),
                })
                .await
                .expect("enqueue");
        }

        // The loop consumes the FIRST input, then the run is cancelled.
        let batch = queue.next_after(run_id, origin(), 1).await.expect("poll");
        assert_eq!(batch.inputs.len(), 1);
        queue
            .ack_consumed(run_id, vec![batch.inputs[0].ack_token.clone()])
            .await
            .expect("ack first");

        let rejected = queue.reject_unconsumed(run_id).await.expect("reconcile");
        assert_eq!(rejected, vec![message_ids[1]]);

        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
            })
            .await
            .unwrap();
        let status_of = |message_id| {
            history
                .messages
                .iter()
                .find(|message| message.message_id == message_id)
                .expect("row in history")
                .status
        };
        assert_eq!(
            status_of(message_ids[0]),
            MessageStatus::Submitted,
            "the consumed row keeps its Submitted status"
        );
        assert_eq!(
            status_of(message_ids[1]),
            MessageStatus::RejectedBusy,
            "the stranded row flips to the resend affordance"
        );

        // The claimed entry is never redelivered, and reconciling again is a
        // no-op.
        let after = queue
            .next_after(run_id, origin(), 8)
            .await
            .expect("poll after reconcile");
        assert!(after.inputs.is_empty());
        let again = queue.reject_unconsumed(run_id).await.expect("idempotent");
        assert!(again.is_empty());
    }

    /// In-memory sibling of the durable reconciliation test above — same
    /// claim-then-flip semantics on the process-local queue.
    #[tokio::test]
    async fn in_memory_reject_unconsumed_flips_stranded_rows_and_stops_redelivery() {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let scope = ghost_scope();
        let thread = thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: None,
                created_by_actor_id: "actor-iq".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let run_id = TurnRunId::new();
        let accepted = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                actor_id: "actor-iq".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text("stranded by cancel"),
            })
            .await
            .unwrap();
        thread_service
            .mark_message_queued(
                &scope,
                &thread.thread_id,
                accepted.message_id,
                run_id.to_string(),
            )
            .await
            .unwrap();

        let queue = crate::InMemoryHostInputQueue::new(
            Arc::clone(&thread_service) as Arc<dyn SessionThreadService>
        );
        queue
            .enqueue_queued_message(EnqueueQueuedMessageRequest {
                run_id,
                turn_id: TurnId::new(),
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                message_id: accepted.message_id,
                input: steering(&format!("msg:{}", accepted.message_id)),
            })
            .await
            .expect("enqueue");

        let rejected = queue.reject_unconsumed(run_id).await.expect("reconcile");
        assert_eq!(rejected, vec![accepted.message_id]);
        let history = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope,
                thread_id: thread.thread_id,
            })
            .await
            .unwrap();
        assert_eq!(history.messages[0].status, MessageStatus::RejectedBusy);
        let after = queue
            .next_after(run_id, origin(), 8)
            .await
            .expect("poll after reconcile");
        assert!(after.inputs.is_empty());
        let again = queue.reject_unconsumed(run_id).await.expect("idempotent");
        assert!(again.is_empty());
    }

    /// The origin cursor is the unique run-start position: the first enqueued
    /// input's cursor is sequence 1, strictly after origin (sequence 0), and a
    /// cursor at or past the next-to-issue sequence is rejected as unissued
    /// instead of read as an empty position.
    #[tokio::test]
    async fn origin_cursor_is_unique_and_future_cursors_are_rejected() {
        let backend = Arc::new(InMemoryBackend::new());
        let thread_service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let queue = FilesystemHostInputQueue::new(make_fs(backend), owner_scope(), thread_service);
        let run_id = TurnRunId::new();
        let envelope = queue
            .enqueue_queued_message(EnqueueQueuedMessageRequest {
                run_id,
                turn_id: TurnId::new(),
                scope: ghost_scope(),
                thread_id: ThreadId::new("thread-origin").unwrap(),
                message_id: ThreadMessageId::new(),
                input: steering("msg:origin-unique"),
            })
            .await
            .expect("enqueue");
        assert_ne!(
            envelope.cursor,
            origin(),
            "the first input's cursor must be strictly after the origin cursor"
        );

        let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
        assert_eq!(batch.inputs.len(), 1);
        // Polling from the entry's own cursor returns nothing (strict-after).
        let after = queue
            .next_after(run_id, batch.inputs[0].cursor.clone(), 8)
            .await
            .expect("poll strictly after");
        assert!(after.inputs.is_empty());
        // A cursor for a sequence that was never issued is rejected.
        let future = queue
            .next_after(
                run_id,
                LoopInputCursorToken::new("input-cursor:2".to_string()).expect("token"),
                8,
            )
            .await;
        assert!(
            matches!(future, Err(HostInputQueueError::InvalidCursor { .. })),
            "unissued future cursor must be rejected, got {future:?}"
        );
    }

    /// Regression: a corrupt persisted queue document surfaces as
    /// `Unavailable` from BOTH `next_after` and `enqueue_queued_message`, and
    /// the corrupt bytes are preserved for diagnosis — never silently
    /// overwritten with a fresh document.
    #[tokio::test]
    async fn corrupt_durable_document_surfaces_unavailable_and_is_preserved() {
        let backend = Arc::new(InMemoryBackend::new());
        let filesystem = make_fs(Arc::clone(&backend));
        let thread_service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let queue =
            FilesystemHostInputQueue::new(Arc::clone(&filesystem), owner_scope(), thread_service);
        let run_id = TurnRunId::new();
        let path = queue_path(run_id).expect("path");
        let corrupt = b"{not json".to_vec();
        filesystem
            .put(
                &owner_scope(),
                &path,
                Entry::bytes(corrupt.clone()).with_content_type(ContentType::json()),
                CasExpectation::Absent,
            )
            .await
            .expect("seed corrupt document");

        let poll = queue.next_after(run_id, origin(), 8).await;
        assert!(
            matches!(poll, Err(HostInputQueueError::Unavailable { .. })),
            "corrupt document must surface Unavailable from next_after, got {poll:?}"
        );
        let enqueue = queue
            .enqueue_queued_message(EnqueueQueuedMessageRequest {
                run_id,
                turn_id: TurnId::new(),
                scope: ghost_scope(),
                thread_id: ThreadId::new("thread-corrupt").unwrap(),
                message_id: ThreadMessageId::new(),
                input: steering("msg:corrupt"),
            })
            .await;
        assert!(
            matches!(enqueue, Err(HostInputQueueError::Unavailable { .. })),
            "corrupt document must surface Unavailable from enqueue, got {enqueue:?}"
        );
        let preserved = filesystem
            .get(&owner_scope(), &path)
            .await
            .expect("read back")
            .expect("document still present");
        assert_eq!(
            preserved.entry.body, corrupt,
            "the corrupt document must be preserved, not overwritten"
        );
    }

    /// CAS retry under real contention: concurrent enqueues (producer side)
    /// against the same run document must each land exactly once — a lost
    /// update would drop a queued message; a resurrected ack would redeliver.
    #[tokio::test]
    async fn concurrent_enqueues_and_acks_survive_cas_contention() {
        let backend = Arc::new(InMemoryBackend::new());
        let filesystem = make_fs(backend);
        let thread_service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let queue = Arc::new(FilesystemHostInputQueue::new(
            filesystem,
            owner_scope(),
            thread_service,
        ));
        let run_id = TurnRunId::new();

        let mut joins = Vec::new();
        for index in 0..8 {
            let queue = Arc::clone(&queue);
            joins.push(tokio::spawn(async move {
                queue
                    .enqueue_queued_message(EnqueueQueuedMessageRequest {
                        run_id,
                        turn_id: TurnId::new(),
                        scope: ghost_scope(),
                        thread_id: ThreadId::new("thread-contention").unwrap(),
                        message_id: ThreadMessageId::new(),
                        input: steering(&format!("msg:contended-{index}")),
                    })
                    .await
            }));
        }
        for join in joins {
            join.await.expect("task").expect("enqueue under contention");
        }

        let batch = queue.next_after(run_id, origin(), 16).await.expect("poll");
        assert_eq!(
            batch.inputs.len(),
            8,
            "every contended enqueue must land exactly once"
        );

        // Concurrent acks of disjoint halves: every input acked exactly once,
        // nothing redelivered afterward.
        let (first_half, second_half) = batch.inputs.split_at(4);
        let tokens_a: Vec<_> = first_half.iter().map(|e| e.ack_token.clone()).collect();
        let tokens_b: Vec<_> = second_half.iter().map(|e| e.ack_token.clone()).collect();
        let queue_a = Arc::clone(&queue);
        let queue_b = Arc::clone(&queue);
        let (result_a, result_b) = tokio::join!(
            queue_a.ack_consumed(run_id, tokens_a),
            queue_b.ack_consumed(run_id, tokens_b)
        );
        result_a.expect("ack half A under contention");
        result_b.expect("ack half B under contention");
        let after = queue
            .next_after(run_id, origin(), 16)
            .await
            .expect("poll after acks");
        assert!(
            after.inputs.is_empty(),
            "no input may be redelivered after concurrent acks, got {}",
            after.inputs.len()
        );
    }

    #[tokio::test]
    async fn ack_is_non_fatal_and_idempotent_when_status_flip_fails() {
        let backend = Arc::new(InMemoryBackend::new());
        let thread_service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let queue = FilesystemHostInputQueue::new(make_fs(backend), owner_scope(), thread_service);
        let run_id = TurnRunId::new();
        let envelope = queue
            .enqueue_queued_message(EnqueueQueuedMessageRequest {
                run_id,
                turn_id: TurnId::new(),
                scope: ghost_scope(),
                thread_id: ThreadId::new("ghost").unwrap(),
                message_id: ThreadMessageId::new(),
                input: steering("msg:ghost"),
            })
            .await
            .expect("enqueue");

        // Status flip fails (ghost thread) but the ack still commits durably.
        queue
            .ack_consumed(run_id, vec![envelope.ack_token.clone()])
            .await
            .expect("ack must be non-fatal when the status flip fails");
        // A redelivered ack for the same token is an idempotent no-op.
        queue
            .ack_consumed(run_id, vec![envelope.ack_token])
            .await
            .expect("idempotent ack");

        let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
        assert!(
            batch.inputs.is_empty(),
            "acked input must not be redelivered"
        );
    }

    #[tokio::test]
    async fn enqueue_dedups_identical_input() {
        let backend = Arc::new(InMemoryBackend::new());
        let thread_service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let queue = FilesystemHostInputQueue::new(make_fs(backend), owner_scope(), thread_service);
        let run_id = TurnRunId::new();
        let request = || EnqueueQueuedMessageRequest {
            run_id,
            turn_id: TurnId::new(),
            scope: ghost_scope(),
            thread_id: ThreadId::new("ghost").unwrap(),
            message_id: ThreadMessageId::new(),
            input: steering("msg:dup"),
        };
        let first = queue
            .enqueue_queued_message(request())
            .await
            .expect("first");
        let second = queue
            .enqueue_queued_message(request())
            .await
            .expect("second");
        assert_eq!(first.ack_token, second.ack_token, "identical input dedups");

        let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
        assert_eq!(batch.inputs.len(), 1, "dedup keeps a single queue entry");
    }

    #[tokio::test]
    async fn ack_rejects_unknown_sequence_instead_of_poisoning_state() {
        // An ack token for a sequence that is neither live nor already acked
        // must fail loud rather than be committed into `acked`. Committing it
        // would poison durable state: when that sequence is later enqueued, its
        // now-pre-acked entry would be skipped forever by `next_after`.
        let backend = Arc::new(InMemoryBackend::new());
        let thread_service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let queue = FilesystemHostInputQueue::new(
            make_fs(Arc::clone(&backend)),
            owner_scope(),
            thread_service,
        );
        let run_id = TurnRunId::new();
        // Create the queue document with a single live entry at sequence 0.
        queue
            .enqueue_queued_message(EnqueueQueuedMessageRequest {
                run_id,
                turn_id: TurnId::new(),
                scope: ghost_scope(),
                thread_id: ThreadId::new("ghost").unwrap(),
                message_id: ThreadMessageId::new(),
                input: steering("msg:live"),
            })
            .await
            .expect("enqueue");

        // Ack a forged token for a sequence that was never enqueued.
        let forged = LoopInputAckToken::new("input-ack:999".to_string()).unwrap();
        let result = queue.ack_consumed(run_id, vec![forged]).await;
        assert!(
            matches!(result, Err(HostInputQueueError::InvalidCursor { .. })),
            "unknown ack sequence must be rejected, got {result:?}"
        );

        // State is untouched: sequence 999 was NOT recorded as acked, so a
        // later real entry at that sequence would still be delivered.
        let batch = queue.next_after(run_id, origin(), 8).await.expect("poll");
        assert_eq!(
            batch.inputs.len(),
            1,
            "the live entry remains deliverable after a rejected forged ack"
        );
    }
}
