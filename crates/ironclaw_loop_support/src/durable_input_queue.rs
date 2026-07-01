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

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use ironclaw_threads::{SessionThreadService, ThreadMessageId, ThreadScope};
use ironclaw_turns::{
    TurnId, TurnRunId,
    run_profile::{LoopInput, LoopInputAckToken, LoopInputCursorToken},
};
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
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct DurableRunQueue {
    next_sequence: u64,
    entries: Vec<DurableEntry>,
    /// Sequences whose inputs have been consumed and acked. Retained (even
    /// after the entry payload is pruned) so a duplicate/redelivered ack is
    /// skipped idempotently.
    acked: Vec<u64>,
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
    thread_id: ironclaw_host_api::ThreadId,
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
        if after_sequence > queue.next_sequence {
            return Err(HostInputQueueError::InvalidCursor {
                reason: "input cursor is ahead of the run input queue".to_string(),
            });
        }
        let acked: HashSet<u64> = queue.acked.iter().copied().collect();
        let mut inputs = Vec::new();
        let mut next_sequence = after_sequence;
        let mut ordered: Vec<&DurableEntry> = queue
            .entries
            .iter()
            .filter(|entry| entry.sequence >= after_sequence)
            .collect();
        ordered.sort_by_key(|entry| entry.sequence);
        for entry in ordered {
            next_sequence = entry.sequence.saturating_add(1);
            if acked.contains(&entry.sequence) {
                continue;
            }
            if inputs.len() >= limit {
                next_sequence = entry.sequence;
                break;
            }
            inputs.push(envelope_for(entry.sequence, entry.input.clone())?);
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
            let already: HashSet<u64> = queue.acked.iter().copied().collect();
            let mut newly_acked = Vec::new();
            status_updates.clear();
            for token in &tokens {
                let sequence = ack_sequence(token)?;
                if already.contains(&sequence) {
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
            queue.acked.extend(newly_acked);
            // Prune consumed entry payloads to bound the document size; the
            // sequence stays in `acked` for idempotency, and `next_sequence`
            // is the high-water mark so a stale cursor never looks "ahead".
            let acked_now: HashSet<u64> = queue.acked.iter().copied().collect();
            queue
                .entries
                .retain(|entry| !acked_now.contains(&entry.sequence));
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
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
        ThreadId, VirtualPath,
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
