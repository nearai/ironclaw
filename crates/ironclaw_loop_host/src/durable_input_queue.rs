//! Durable, filesystem-backed host input queue.
//!
//! The queue SEMANTICS live in [`RunQueueModel`](crate::input_queue) — the one
//! model both backends share. This file is only the durability shell: it
//! persists each run's model verbatim as a single CAS-guarded JSON document
//! under the run-scoped filesystem, so the queue survives restart and the
//! resumed run drains it exactly as before. Cursor/ack tokens are
//! reconstructed deterministically from persisted sequences via the shared
//! wire helpers, so the loop's persisted input cursor stays valid across a
//! restart. (The in-memory backend holds the same model in a per-run map; a
//! daemon restart drops it, which is why production composes THIS backend.)
//!
//! Scope preservation: the queue document is written through a
//! [`ScopedFilesystem`] under the owner [`ResourceScope`] the composition
//! passes at construction (built from the run's tenant / user / agent /
//! project). In multi-tenant composition the mount-view resolver rewrites that
//! scope into the virtual path prefix (`/tenants/<tenant>/users/<user>/…`), so
//! the record *is* tenant/user-partitioned at the storage boundary — the scope
//! is not dropped. The path itself is then keyed by the globally-unique
//! `run_id` (a UUID), which guarantees no cross-run or cross-tenant collision
//! and lets the resumed run find its own queue.
//!
//! What is *deferred*: finer per-run path granularity inside that owner scope
//! (e.g. a per-thread subtree). The `HostInputQueue` trait methods receive
//! only `run_id`, not a scope, so per-run path partitioning would need either
//! a `run_id → scope` map or a trait change. `run_id` uniqueness makes that
//! unnecessary for correctness or isolation, so it is intentionally left out
//! here.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};
use ironclaw_loop_contracts::{LoopInputAckToken, LoopInputCursorToken};
use ironclaw_threads::{SessionThreadService, ThreadMessageId};
use ironclaw_turns::TurnRunId;

use crate::input_queue::{
    EnqueueQueuedMessageRequest, HostInputBatch, HostInputEnqueuePort, HostInputEnvelope,
    HostInputQueue, HostInputQueueError, HostInputQueueReconcile, QueuedMessageStatusUpdate,
    RunQueueModel, cursor_sequence, envelope_for, flip_rejected_busy, flip_submitted,
};

/// Bounds the CAS retry loop so persistent contention surfaces as a host error
/// instead of spinning forever. Per-run contention is low (one producer thread
/// enqueuing, one loop thread acking), so a handful of retries is ample.
const MAX_CAS_RETRIES: usize = 8;

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

/// Outcome of a CAS-guarded durable write.
enum StorePutError {
    /// The CAS precondition failed — a concurrent writer won; retry.
    Conflict,
    /// A non-retryable failure (serialization, backend IO, bad path).
    Fatal(HostInputQueueError),
}

impl<F> FilesystemHostInputQueue<F>
where
    F: RootFilesystem + ?Sized + 'static,
{
    /// Build a durable queue over `filesystem`, persisting under `owner_scope`.
    /// `thread_service` performs the queued-message status flips.
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
    ) -> Result<(RunQueueModel, Option<RecordVersion>), HostInputQueueError> {
        let path = queue_path(run_id)?;
        match self.filesystem.get(&self.owner_scope, &path).await {
            Ok(Some(versioned)) => {
                let model = serde_json::from_slice(&versioned.entry.body).map_err(|error| {
                    HostInputQueueError::Unavailable {
                        reason: format!("durable input queue is corrupt: {error}"),
                    }
                })?;
                Ok((model, Some(versioned.version)))
            }
            Ok(None) => Ok((RunQueueModel::default(), None)),
            Err(error) => Err(fs_error(error)),
        }
    }

    /// Persist `model`, asserting the expected CAS precondition. `version` is
    /// `None` for a first write (`Absent`) and `Some` for an update.
    async fn store(
        &self,
        run_id: TurnRunId,
        model: &RunQueueModel,
        version: Option<RecordVersion>,
    ) -> Result<(), StorePutError> {
        let body = serde_json::to_vec(model).map_err(|error| {
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

#[async_trait]
impl<F> HostInputEnqueuePort for FilesystemHostInputQueue<F>
where
    F: RootFilesystem + ?Sized + 'static,
{
    async fn enqueue_queued_message(
        &self,
        request: EnqueueQueuedMessageRequest,
    ) -> Result<HostInputEnvelope, HostInputQueueError> {
        let status = QueuedMessageStatusUpdate {
            turn_id: request.turn_id,
            scope: request.scope,
            thread_id: request.thread_id,
            message_id: request.message_id,
        };
        for _ in 0..MAX_CAS_RETRIES {
            let (mut model, version) = self.load(request.run_id).await?;
            let enqueued = model.enqueue_dedup(request.input.clone(), status.clone());
            if !enqueued.inserted {
                return envelope_for(enqueued.sequence, request.input);
            }
            match self.store(request.run_id, &model, version).await {
                Ok(()) => return envelope_for(enqueued.sequence, request.input),
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
        let (model, version) = self.load(run_id).await?;
        if version.is_none() {
            return Ok(HostInputBatch {
                inputs: Vec::new(),
                next_cursor: after,
            });
        }
        model.scan_after(after_sequence, limit)
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
            let (mut model, version) = self.load(run_id).await?;
            let Some(version) = version else {
                // No durable queue for this run: nothing to ack.
                return Ok(());
            };
            status_updates = model.validate_and_ack(&tokens)?;
            if status_updates.is_empty() {
                // Every token was a redelivered duplicate: idempotent no-op.
                return Ok(());
            }
            match self.store(run_id, &model, Some(version)).await {
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
        // Phase 2: best-effort transcript flip (see the shared helper's doc).
        flip_submitted(self.thread_service.as_ref(), run_id, status_updates).await;
        Ok(())
    }
}

#[async_trait]
impl<F> HostInputQueueReconcile for FilesystemHostInputQueue<F>
where
    F: RootFilesystem + ?Sized + 'static,
{
    async fn reject_unconsumed(
        &self,
        run_id: TurnRunId,
    ) -> Result<Vec<ThreadMessageId>, HostInputQueueError> {
        // Phase 1: durably claim every live entry by DELETING the run's queue
        // document under its CAS version — the run is terminal, so nothing may
        // enqueue for it again, a racing drain can no longer load the claimed
        // entries, and a late duplicate ack finds no document (a no-op). This
        // is also the durable lifetime bound: terminal queues do not
        // accumulate documents forever.
        let mut status_updates = Vec::new();
        let mut committed = false;
        for _ in 0..MAX_CAS_RETRIES {
            let (model, version) = self.load(run_id).await?;
            let Some(version) = version else {
                // No durable queue for this run: nothing to reconcile.
                return Ok(Vec::new());
            };
            status_updates = model.unacked_status_updates();
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
        // Phase 2: best-effort transcript flip (see the shared helper's doc).
        Ok(flip_rejected_busy(self.thread_service.as_ref(), run_id, status_updates).await)
    }
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

fn cas_exhausted(operation: &'static str) -> HostInputQueueError {
    HostInputQueueError::Unavailable {
        reason: format!("durable input queue {operation} exhausted CAS retries"),
    }
}

#[cfg(test)]
#[path = "durable_input_queue/tests.rs"]
mod tests;
