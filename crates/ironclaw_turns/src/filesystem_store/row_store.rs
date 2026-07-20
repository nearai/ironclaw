// arch-exempt: large_file, WAL row store (apply/reservation/journal + delta commit paths) predates decomposition; #6263 no-op-delta durability fix adds a small guard, plan #6263
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use futures_util::stream::{self, StreamExt, TryStreamExt};
use ironclaw_filesystem::{
    FILESYSTEM_APPLY_TIMEOUT, FileType, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem, SeqNo,
};
use ironclaw_host_api::{ResourceScope, UserId};
use serde::de::DeserializeOwned;
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use tracing::{Instrument, field};

use crate::{
    AllowAllTurnAdmissionLimitProvider, CancelRunRequest, EventCursor, GetLoopCheckpointRequest,
    GetRunStateRequest, LoopCheckpointRecord, TurnAdmissionLimitProvider, TurnError, TurnEventPage,
    TurnPersistenceSnapshot, TurnRecord, TurnRunId, TurnRunRecord, TurnRunState, TurnScope,
    TurnStateStoreLimits, TurnStatus,
    events::project_turn_events,
    runner::{ClaimedTurnRun, HeartbeatRequest, RelinquishRunRequest, TurnRunTransitionPort},
};

use super::{
    io as legacy_blob_io, projection,
    runner_lease::{RunnerLeaseMemory, RunnerLeaseOverlay, RunnerLeaseRecord, RunnerLeaseStore},
    turn_state_engine::TurnStateEngine,
};

mod delta;
mod io;
mod journal;
mod traits;

use delta::{
    RowPersistError, RowSnapshotState, RowStoreMeta, SnapshotDelta, active_lock_record_key,
    event_record_key, keyed_records, preserve_loop_checkpoints, row_store_durable_delta,
    row_store_hot_cache_snapshot, snapshot_delta,
};
use io::{
    delta_log_path, deserialize_materialized_row, deserialize_row, fs_error, meta_path, row_dir,
    row_path,
};
use journal::{DeltaAck, DeltaJournal, materialize_delta_log};

const TURN_ROWS: &str = "turns";
const RUN_ROWS: &str = "runs";
const ACTIVE_LOCK_ROWS: &str = "active-locks";
const CHECKPOINT_ROWS: &str = "checkpoints";
const LOOP_CHECKPOINT_ROWS: &str = "loop-checkpoints";
const IDEMPOTENCY_ROWS: &str = "idempotency";
const EVENT_ROWS: &str = "events";
const ADMISSION_RESERVATION_ROWS: &str = "admission-reservations";
const SPAWN_TREE_RESERVATION_ROWS: &str = "spawn-tree-reservations";
const ROW_COLLECTION_READ_CONCURRENCY: usize = 32;

// #6263 Step 5b: `FilesystemTurnStateRowStore` no longer has a durability-mode
// choice. A mutation whose resulting run status is NOT
// [`is_recoverability_critical`](crate::is_recoverability_critical) returns
// `Ok` immediately after enqueue, WITHOUT awaiting the durable ack; the
// flusher persists it in the background (memory-speed non-critical writes, at
// the cost of a bounded crash-loss window for trailing non-critical
// transitions). Recoverability-critical transitions (gate-park, terminal, and
// brand-new run creation) still await synchronously, and because the journal
// is a strictly sequential single-writer, awaiting a critical op's ack
// flushes its entire preceding async tail — critical ops are natural
// durability barriers. There is exactly one mode; the former
// `TurnStateDurabilityPolicy::WriteThrough` variant (and its opt-in
// `with_durability_policy` setter) is gone.

/// Filesystem-backed turn-state store using typed append-log deltas.
///
/// This is intentionally separate from [`super::FilesystemTurnStateStore`].
/// When the row projection is empty, first load imports a legacy
/// `/turns/state.json` blob by appending a full-snapshot row delta and then
/// replaying the normal delta journal. Once any row data exists, rows are
/// authoritative and the legacy blob is left untouched as rollback evidence.
/// Transitions still delegate to [`TurnStateEngine`]; only the durable
/// representation changes from whole-snapshot CAS to a typed append log plus a
/// process-local hot snapshot cache.
// arch-exempt: large_file, long-lived embedded-authority refactor adds a shared
// overlay-store acquisition helper (deduplicating the apply / targeted-delta
// paths) rather than a second store-acquisition pipeline, plan #6263
pub struct FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    limits: TurnStateStoreLimits,
    admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    snapshot_state: AsyncMutex<Option<RowSnapshotState>>,
    legacy_migration_gate: Arc<AsyncMutex<()>>,
    materialize_gate: Arc<AsyncMutex<()>>,
    runner_leases: RunnerLeaseMemory,
    delta_journal: DeltaJournal,
    apply_timeout: Duration,
    /// Backpressure window: the enqueued-but-un-acked non-critical delta
    /// acks, oldest first. Bounded by
    /// [`TurnStateStoreLimits::max_pending_write_behind_deltas`]; at the cap
    /// the next non-critical op awaits the oldest before enqueuing.
    pending_write_behind: AsyncMutex<VecDeque<DeltaAck>>,
}

struct PendingRowCommit<T> {
    value: T,
    ack: Option<DeltaAck>,
    /// Whether the resulting run status is recoverability-critical (gate-park,
    /// terminal, or brand-new run creation). A critical commit awaits its
    /// durable ack synchronously (a barrier); a non-critical one registers for
    /// backpressure and returns without awaiting.
    critical: bool,
}

enum RowApplyOutcome<T> {
    Ready(T),
    Pending(PendingRowCommit<T>),
}

struct RunStateTransitionTarget {
    run_id: TurnRunId,
    runner_id: crate::TurnRunnerId,
    lease_token: crate::TurnLeaseToken,
    retired_status: TurnStatus,
}

impl<F> FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self
    where
        F: 'static,
    {
        let materialize_gate = Arc::new(AsyncMutex::new(()));
        Self {
            filesystem: Arc::clone(&filesystem),
            limits: TurnStateStoreLimits::default(),
            admission_limit_provider: Arc::new(AllowAllTurnAdmissionLimitProvider),
            snapshot_state: AsyncMutex::new(None),
            legacy_migration_gate: Arc::new(AsyncMutex::new(())),
            materialize_gate: Arc::clone(&materialize_gate),
            runner_leases: Arc::new(RwLock::new(HashMap::new())),
            delta_journal: DeltaJournal::new(filesystem, materialize_gate),
            apply_timeout: FILESYSTEM_APPLY_TIMEOUT,
            pending_write_behind: AsyncMutex::new(VecDeque::new()),
        }
    }

    pub fn with_limits(mut self, limits: TurnStateStoreLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_admission_limit_provider(
        mut self,
        admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    ) -> Self {
        self.admission_limit_provider = admission_limit_provider;
        self
    }

    pub fn with_apply_timeout(mut self, apply_timeout: Duration) -> Self {
        self.apply_timeout = apply_timeout;
        self
    }

    pub async fn persistence_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let (snapshot, _) = self
            .read_snapshot_with_runner_lease_overlay(RunnerLeaseOverlay::All)
            .await?;
        Ok(snapshot)
    }

    /// Materialize the embedded engine over the current durable state (runner
    /// leases overlaid) for a read-only inspection. Shared by the observability
    /// accessors below, which mirror the engine's own inherent accessors.
    ///
    /// Uses a non-blocking snapshot read: a `TurnAdmissionPolicy` may call these
    /// observability accessors reentrantly from inside `submit_turn`'s
    /// `check_submit`, which runs while the mutation holds `snapshot_state`.
    /// Blocking on that lock from the reentrant read would deadlock (the reenter
    /// waits for a lock the same logical operation holds), so when the hot cache
    /// is busy this reads the committed durable rows directly — the pre-mutation
    /// state a concurrent reader must observe anyway (#6263).
    async fn read_engine(&self) -> Result<TurnStateEngine, TurnError> {
        let snapshot = self.read_snapshot_for_observability().await?;
        let (snapshot, _) = self
            .runner_lease_store()
            .overlay((snapshot, None), RunnerLeaseOverlay::All)
            .await?;
        self.build_in_memory_store(snapshot)
    }

    /// Read the persistence snapshot without blocking on an in-flight mutation.
    /// If `snapshot_state` is held (a mutation is in progress), read the committed
    /// durable rows directly instead of waiting — see [`Self::read_engine`].
    async fn read_snapshot_for_observability(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        match self.snapshot_state.try_lock() {
            Ok(mut guard) => {
                self.drop_cache_if_degraded(&mut guard);
                if guard.is_none() {
                    *guard = Some(self.load_snapshot_from_rows().await?);
                }
                Ok(guard
                    .as_ref()
                    .map(|state| state.snapshot.clone())
                    .unwrap_or_default())
            }
            Err(_) => {
                materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None)
                    .await?;
                self.read_materialized_row_snapshot().await
            }
        }
    }

    /// Admission reservations currently outstanding. Testing/observability read.
    pub async fn active_admission_reservations(
        &self,
    ) -> Result<Vec<crate::TurnAdmissionReservationRecord>, TurnError> {
        Ok(self.read_engine().await?.active_admission_reservations())
    }

    /// The full redacted lifecycle-event log. Testing/observability read.
    pub async fn events(&self) -> Result<Vec<crate::TurnLifecycleEvent>, TurnError> {
        Ok(self.read_engine().await?.events())
    }

    /// Count of running-slot-holding runs for a user. Testing/observability read.
    pub async fn running_count_for_user(
        &self,
        tenant: &ironclaw_host_api::TenantId,
        user: &UserId,
    ) -> Result<u32, TurnError> {
        Ok(self
            .read_engine()
            .await?
            .running_count_for_user(tenant, user))
    }

    /// Count of running trigger-origin runs for a tenant. Testing/observability read.
    pub async fn running_trigger_count(
        &self,
        tenant: &ironclaw_host_api::TenantId,
    ) -> Result<u32, TurnError> {
        Ok(self.read_engine().await?.running_trigger_count(tenant))
    }

    /// Count of running conversation-origin runs for a tenant. Testing/observability read.
    pub async fn running_conversation_count(
        &self,
        tenant: &ironclaw_host_api::TenantId,
    ) -> Result<u32, TurnError> {
        Ok(self.read_engine().await?.running_conversation_count(tenant))
    }

    /// Flush the write-behind tail: await every enqueued-but-un-acked
    /// non-critical delta ack so nothing un-durable remains when the process
    /// exits.
    ///
    /// Awaits the oldest-first backpressure window populated by
    /// [`track_write_behind_ack_if_async`](Self::track_write_behind_ack_if_async),
    /// giving a planned/graceful restart a clean durable tail. A hard crash
    /// (SIGKILL/OOM) still loses only the un-acked non-critical (non-critical =
    /// not gate-park/terminal/brand-new-run) tail; gate-park, terminal, and
    /// brand-new-run transitions are synchronously durable and never wait on
    /// this drain.
    ///
    /// Idempotent. On an append failure the flusher has halted and latched the
    /// store degraded; the error is surfaced and the hot cache is rolled back to
    /// the last consistent durable point (mirroring the backpressure path).
    pub async fn drain(&self) -> Result<(), TurnError> {
        let mut window = self.pending_write_behind.lock().await;
        while let Some(ack) = window.pop_front() {
            if let Err(error) = DeltaJournal::await_ack(Some(ack)).await {
                drop(window);
                self.clear_snapshot_cache().await;
                return Err(error);
            }
        }
        Ok(())
    }

    async fn read_snapshot(
        &self,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        let mut guard = self.snapshot_state.lock().await;
        self.drop_cache_if_degraded(&mut guard);
        if guard.is_none() {
            *guard = Some(self.load_snapshot_from_rows().await?);
        }
        let snapshot = guard
            .as_ref()
            .map(|state| state.snapshot.clone())
            .unwrap_or_default();
        Ok((snapshot, None))
    }

    async fn read_snapshot_with_runner_lease_overlay(
        &self,
        overlay: RunnerLeaseOverlay,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        let snapshot = self.read_snapshot().await?;
        self.runner_lease_store().overlay(snapshot, overlay).await
    }

    async fn with_cached_snapshot<T, R>(&self, read: R) -> Result<T, TurnError>
    where
        R: FnOnce(&TurnPersistenceSnapshot) -> T,
    {
        let mut guard = self.snapshot_state.lock().await;
        self.drop_cache_if_degraded(&mut guard);
        if guard.is_none() {
            *guard = Some(self.load_snapshot_from_rows().await?);
        }
        let snapshot = &guard
            .as_ref()
            .ok_or_else(|| TurnError::Unavailable {
                reason: "row snapshot cache was not initialized".to_string(),
            })?
            .snapshot;
        Ok(read(snapshot))
    }

    async fn clear_snapshot_cache(&self) {
        *self.snapshot_state.lock().await = None;
    }

    /// If the store degraded after a write-behind append failure, drop the hot
    /// cache so the next read reloads from the last consistent durable point.
    /// A pure atomic check off the hot path when not degraded.
    fn drop_cache_if_degraded(&self, guard: &mut Option<RowSnapshotState>) {
        if self.delta_journal.is_degraded() {
            *guard = None;
        }
    }

    /// Whether this commit takes the async (non-awaited) write-behind path:
    /// only non-critical transitions (write-behind is unconditional; every
    /// commit takes this path unless it is recoverability-critical).
    fn write_behind_async(&self, critical: bool) -> bool {
        !critical
    }

    /// Whether reads must serve from the process-local hot snapshot to honor
    /// read-your-writes. A non-critical mutation returns `Ok` after updating
    /// the hot cache but before its durable append, so a durable-row read
    /// could miss it. The hot cache is the single-writer authority. Once the
    /// journal has degraded the cache is cleared and reads must fall back to
    /// the last consistent durable point.
    fn is_write_behind_healthy(&self) -> bool {
        !self.delta_journal.is_degraded()
    }

    /// Reset the hot cache after a mutation the embedded engine REJECTED (a
    /// domain error such as `ThreadBusy` / `InvalidTransition`).
    ///
    /// The op ran against the shared cached engine, so it may have left a
    /// partial mutation; that must be discarded. Durable LAGS the cache
    /// (un-acked non-critical ops), so a reload-from-durable would silently
    /// drop acked-but-unflushed ops the caller was told succeeded. Instead,
    /// rebuild the embedded engine from the cached snapshot (which still
    /// includes those un-flushed ops), discarding only the failed op's
    /// partial mutation.
    fn reset_cache_after_rejected_mutation(
        &self,
        guard: &mut Option<RowSnapshotState>,
    ) -> Result<(), TurnError> {
        if let Some(state) = guard.as_mut() {
            state.store = Arc::new(self.build_in_memory_store(state.snapshot.clone())?);
        }
        Ok(())
    }

    /// Fail a mutation fast when the store has degraded after a write-behind
    /// append failure. Clears the diverged hot cache (reads then reload from the
    /// last consistent durable point) and returns a retryable error.
    async fn ensure_not_degraded(&self) -> Result<(), TurnError> {
        if self.delta_journal.is_degraded() {
            self.clear_snapshot_cache().await;
            return Err(TurnError::Unavailable {
                reason: "turn-state row store degraded after a write-behind durable append failure"
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Commit a prepared [`PendingRowCommit`].
    ///
    /// * Critical — await the ack (a barrier: awaiting a critical op's ack
    ///   implies the whole preceding async tail is durable).
    /// * Non-critical — already enqueued and tracked in the bounded pending
    ///   window by `apply` (returns `ack: None`), so nothing is awaited here.
    async fn commit_pending<T>(
        &self,
        pending: PendingRowCommit<T>,
        timeout_reason: &'static str,
    ) -> Result<T, TurnError> {
        // `ack` is `None` for a no-op/empty commit AND for a non-critical
        // write-behind commit (enqueued + tracked in `apply` under the
        // `snapshot_state` lock): either way there is nothing to flush here.
        if pending.ack.is_none() {
            return Ok(pending.value);
        }
        // A durable ack is present ⇒ a critical write-behind barrier: await
        // it. A non-critical write-behind commit never reaches here —
        // `track_write_behind_ack_if_async` returned `ack: None` above.
        debug_assert!(
            !self.write_behind_async(pending.critical),
            "non-critical write-behind commit must be tracked in apply, not awaited",
        );
        self.await_pending_commit(pending, timeout_reason).await
    }

    /// Reserve a slot in the bounded write-behind pending window BEFORE the
    /// journal enqueue, so a stalled flusher cannot let concurrent callers grow
    /// the unbounded journal channel without bound (#6263 Step 3). Called under
    /// the `snapshot_state` lock, which serializes enqueue; the flusher drains
    /// the journal independently of that lock, so awaiting the oldest pending
    /// ack here is backpressure, not deadlock. At the cap, await (and drop) the
    /// OLDEST pending ack first, bounding both memory and the crash-loss window.
    /// A degraded (append-failure) ack propagates; the caller clears the hot
    /// cache and fails fast.
    async fn reserve_write_behind_slot(&self) -> Result<(), TurnError> {
        let cap = self.limits.max_pending_write_behind_deltas.max(1);
        let mut window = self.pending_write_behind.lock().await;
        while window.len() >= cap {
            // Await the OLDEST ack IN PLACE — peek, don't pop first. This runs
            // under `apply`'s outer timeout, and a cancelled await MUST leave the
            // ack tracked in the window; popping first would drop it on
            // cancellation, letting later writes see an empty window and enqueue
            // behind a stalled append (unbounded again), and letting a read-side
            // drain falsely succeed while the acknowledged write is still in
            // flight (#6298 IronLoop f7). Remove it only once it has resolved.
            let Some(front) = window.front_mut() else {
                break;
            };
            let result = DeltaJournal::await_ack_ref(front).await;
            window.pop_front();
            result?;
        }
        Ok(())
    }

    /// For a non-critical write-behind commit, move the durable ack into the
    /// bounded pending window (its slot was reserved by
    /// [`reserve_write_behind_slot`](Self::reserve_write_behind_slot) before the
    /// enqueue) and return `None` so [`commit_pending`](Self::commit_pending)
    /// returns without awaiting. Otherwise return the ack unchanged for the
    /// caller to await (a critical barrier). Runs under
    /// `snapshot_state`, so the reserve→enqueue→track sequence is serialized and
    /// the window can never exceed the cap.
    async fn track_write_behind_ack_if_async(
        &self,
        critical: bool,
        ack: Option<DeltaAck>,
    ) -> Option<DeltaAck> {
        if self.write_behind_async(critical) {
            if let Some(ack) = ack {
                self.pending_write_behind.lock().await.push_back(ack);
            }
            return None;
        }
        ack
    }

    /// Read-side write-behind barrier (#6298).
    ///
    /// The durable-read query methods (`get_run_state`, `read_turn_events_after`,
    /// `get_loop_checkpoint`) read materialized durable rows so they preserve the
    /// contracts a bounded hot cache cannot: cross-writer freshness and
    /// queryability of terminal runs / events the cache has EVICTED but the
    /// durable store retains. However, a non-critical mutation returns `Ok`
    /// after merely ENQUEUEing its delta — before the flusher appends it and
    /// the materializer writes its rows. A durable read issued in that window
    /// would miss the just-acked write (`get_run_state` → `Ok(None)` →
    /// `ScopeNotFound`, an empty event page, a missing checkpoint), failing
    /// every runtime `submit_turn` → `get_run_state`.
    ///
    /// Draining the enqueued-but-un-acked non-critical window here — bounded by
    /// [`TurnStateStoreLimits::max_pending_write_behind_deltas`], typically
    /// just the caller's own trailing submit — awaits those durable appends, so
    /// the subsequent durable read (which force-materializes the journal tail)
    /// observes them. This is a read-side barrier symmetric to a critical
    /// transition's write-side barrier, and keeps the durable read's exact
    /// semantics stable (only WHEN it reads changes, never WHAT it reads). On a
    /// drained ack failure the flusher has halted and latched the store
    /// degraded; clear the diverged hot cache and surface the retryable error,
    /// mirroring the backpressure path.
    async fn flush_pending_write_behind_for_read(&self) -> Result<(), TurnError> {
        // Await each pending ack IN PLACE under the window lock (peek-await-pop),
        // removing it only once it resolves. Holding the lock across the awaits
        // bounds the set to the writes present when the flush began — no later
        // write can register mid-flush, so this still only covers read-your-writes
        // writes — AND keeps un-awaited acks tracked if this read is cancelled. A
        // drain-into-`Vec` would drop the un-awaited acks on cancellation, losing
        // acknowledged-but-unflushed writes and letting this flush falsely report
        // success (#6298 IronLoop f7). The journal flusher does not take this
        // lock, so awaiting under it cannot deadlock; the set is bounded by the
        // pending cap.
        let mut window = self.pending_write_behind.lock().await;
        while let Some(front) = window.front_mut() {
            let result = DeltaJournal::await_ack_ref(front).await;
            window.pop_front();
            if let Err(error) = result {
                drop(window);
                self.clear_snapshot_cache().await;
                return Err(error);
            }
        }
        Ok(())
    }

    async fn ensure_snapshot_cache_for_mutation(
        &self,
        guard: &mut Option<RowSnapshotState>,
    ) -> Result<(), TurnError> {
        if guard.is_none() {
            *guard = Some(self.load_snapshot_from_rows().await?);
        }
        Ok(())
    }

    async fn refresh_snapshot_cache_after_stale_mutation_error(
        &self,
        guard: &mut Option<RowSnapshotState>,
        error: &TurnError,
    ) -> Result<bool, TurnError> {
        if !matches!(error, TurnError::ThreadBusy(_)) {
            return Ok(false);
        }
        let head_seq = self.delta_log_head_seq().await?;
        if guard
            .as_ref()
            .is_none_or(|state| state.journal_seq < head_seq)
        {
            *guard = Some(self.load_snapshot_from_rows().await?);
            return Ok(true);
        }
        Ok(false)
    }

    async fn load_snapshot_from_rows(&self) -> Result<RowSnapshotState, TurnError> {
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        let snapshot = self.read_materialized_row_snapshot().await?;
        let snapshot = self.remove_orphan_active_locks(snapshot).await?;
        let snapshot = self.migrate_legacy_blob_if_needed(snapshot).await?;
        let snapshot = row_store_hot_cache_snapshot(snapshot, self.limits);
        let store = self.build_in_memory_store(snapshot)?;
        let snapshot = store.persistence_snapshot();
        let journal_seq = self.read_meta().await?.journal_seq;
        RowSnapshotState::new(snapshot, Arc::new(store), journal_seq)
    }

    async fn read_materialized_row_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let meta = self.read_meta().await?;
        let turns = self.read_row_collection(TURN_ROWS).await?;
        let runs = self.read_row_collection(RUN_ROWS).await?;
        let active_locks = self.read_row_collection(ACTIVE_LOCK_ROWS).await?;
        let checkpoints = self.read_row_collection(CHECKPOINT_ROWS).await?;
        let loop_checkpoints = self.read_row_collection(LOOP_CHECKPOINT_ROWS).await?;
        let idempotency_records = self.read_row_collection(IDEMPOTENCY_ROWS).await?;
        let events = self.read_row_collection(EVENT_ROWS).await?;
        let admission_reservations = self.read_row_collection(ADMISSION_RESERVATION_ROWS).await?;
        let spawn_tree_reservations = self
            .read_row_collection(SPAWN_TREE_RESERVATION_ROWS)
            .await?;

        Ok(TurnPersistenceSnapshot {
            turns,
            runs,
            active_locks,
            checkpoints,
            loop_checkpoints,
            idempotency_records,
            events,
            event_retention_floor: meta.event_retention_floor,
            admission_reservations,
            spawn_tree_reservations,
        })
    }

    async fn remove_orphan_active_locks(
        &self,
        mut snapshot: TurnPersistenceSnapshot,
    ) -> Result<TurnPersistenceSnapshot, TurnError> {
        let mut retained = Vec::with_capacity(snapshot.active_locks.len());
        for lock in snapshot.active_locks {
            if snapshot.runs.iter().any(|run| run.run_id == lock.run_id) {
                retained.push(lock);
                continue;
            }
            let key = active_lock_record_key(&lock)?;
            let path = row_path(ACTIVE_LOCK_ROWS, &key)?;
            match self
                .filesystem
                .delete(&ResourceScope::system(), &path)
                .await
            {
                Ok(()) => {
                    tracing::debug!(
                        active_lock_key = %key,
                        run_id = %lock.run_id,
                        "removed orphan turn-state active-lock row without a durable run row",
                    );
                }
                Err(FilesystemError::NotFound { .. }) => {
                    tracing::debug!(
                        active_lock_key = %key,
                        run_id = %lock.run_id,
                        "orphan turn-state active-lock row disappeared during cleanup",
                    );
                }
                Err(error) => {
                    tracing::debug!(
                        active_lock_key = %key,
                        run_id = %lock.run_id,
                        %error,
                        "failed to remove orphan turn-state active-lock row; continuing with filtered snapshot",
                    );
                }
            }
        }
        snapshot.active_locks = retained;
        Ok(snapshot)
    }

    async fn migrate_legacy_blob_if_needed(
        &self,
        materialized: TurnPersistenceSnapshot,
    ) -> Result<TurnPersistenceSnapshot, TurnError> {
        if materialized != TurnPersistenceSnapshot::default() {
            return Ok(materialized);
        }
        if self.read_meta().await?.journal_seq > SeqNo::ZERO {
            return Ok(materialized);
        }

        let _migration_guard = self.legacy_migration_gate.lock().await;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        let current = self.read_materialized_row_snapshot().await?;
        if current != TurnPersistenceSnapshot::default() {
            return Ok(current);
        }
        if self.read_meta().await?.journal_seq > SeqNo::ZERO {
            return Ok(current);
        }

        let Some(legacy) = self.read_legacy_blob_snapshot().await? else {
            return Ok(current);
        };
        if legacy == TurnPersistenceSnapshot::default() {
            return Ok(current);
        }

        let delta = snapshot_delta(&TurnPersistenceSnapshot::default(), &legacy)
            .map_err(RowPersistError::into_turn)?;
        if delta.is_empty() {
            return Ok(current);
        }

        tracing::debug!(
            turns = legacy.turns.len(),
            runs = legacy.runs.len(),
            events = legacy.events.len(),
            active_locks = legacy.active_locks.len(),
            checkpoints = legacy.checkpoints.len(),
            loop_checkpoints = legacy.loop_checkpoints.len(),
            idempotency_records = legacy.idempotency_records.len(),
            "migrating legacy turn-state blob into row store"
        );
        let ack = self
            .enqueue_delta(delta)
            .map_err(RowPersistError::into_turn)?;
        self.await_delta_ack(ack)
            .await
            .map_err(RowPersistError::into_turn)?;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        let migrated = self.read_materialized_row_snapshot().await?;
        tracing::debug!(
            turns = migrated.turns.len(),
            runs = migrated.runs.len(),
            events = migrated.events.len(),
            active_locks = migrated.active_locks.len(),
            checkpoints = migrated.checkpoints.len(),
            loop_checkpoints = migrated.loop_checkpoints.len(),
            idempotency_records = migrated.idempotency_records.len(),
            "legacy turn-state blob migration completed"
        );
        Ok(migrated)
    }

    async fn read_legacy_blob_snapshot(
        &self,
    ) -> Result<Option<TurnPersistenceSnapshot>, TurnError> {
        let path = legacy_blob_io::snapshot_path()?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => {
                legacy_blob_io::deserialize_snapshot(&versioned.entry.body).map(Some)
            }
            Ok(None) => Ok(None),
            Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(fs_error(error)),
        }
    }

    async fn read_meta(&self) -> Result<RowStoreMeta, TurnError> {
        let path = meta_path()?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => deserialize_row(&versioned.entry.body, "turn-state row meta"),
            Ok(None) => Ok(RowStoreMeta::default()),
            Err(error) => Err(fs_error(error)),
        }
    }

    async fn delta_log_head_seq(&self) -> Result<SeqNo, TurnError> {
        let path = delta_log_path()?;
        let head = self
            .filesystem
            .head_seq(&ResourceScope::system(), &path, SeqNo::ZERO)
            .await
            .map_err(fs_error)?;
        Ok(head.unwrap_or(SeqNo::ZERO))
    }

    async fn read_row_collection<T>(&self, collection: &'static str) -> Result<Vec<T>, TurnError>
    where
        T: DeserializeOwned,
    {
        self.read_row_collection_filtered(collection, |_| true)
            .await
    }

    async fn read_row_collection_filtered<T, P>(
        &self,
        collection: &'static str,
        include_key: P,
    ) -> Result<Vec<T>, TurnError>
    where
        T: DeserializeOwned,
        P: Fn(&str) -> bool,
    {
        let dir = row_dir(collection)?;
        let entries = match self
            .filesystem
            .list_dir(&ResourceScope::system(), &dir)
            .await
        {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => Vec::new(),
            Err(error) => return Err(fs_error(error)),
        };
        let paths = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::File)
            .filter_map(|entry| entry.name.strip_suffix(".json").map(ToString::to_string))
            .filter(|key| include_key(key))
            .map(|key| row_path(collection, &key))
            .collect::<Result<Vec<_>, _>>()?;
        let records = stream::iter(paths)
            .map(|path| async move {
                match self.filesystem.get(&ResourceScope::system(), &path).await {
                    Ok(Some(versioned)) => {
                        deserialize_materialized_row(&versioned.entry.body, collection)
                    }
                    Ok(None) | Err(FilesystemError::NotFound { .. }) => Ok(None),
                    Err(error) => Err(fs_error(error)),
                }
            })
            .buffer_unordered(ROW_COLLECTION_READ_CONCURRENCY)
            .try_collect::<Vec<_>>()
            .await?;
        Ok(records.into_iter().flatten().collect())
    }

    async fn read_row_by_key<T>(
        &self,
        collection: &'static str,
        key: &str,
    ) -> Result<Option<T>, TurnError>
    where
        T: DeserializeOwned,
    {
        let path = row_path(collection, key)?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => deserialize_materialized_row(&versioned.entry.body, collection),
            Ok(None) | Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(fs_error(error)),
        }
    }

    async fn read_run_state_from_durable_rows(
        &self,
        request: &GetRunStateRequest,
    ) -> Result<Option<TurnRunState>, TurnError> {
        self.flush_pending_write_behind_for_read().await?;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        self.ensure_legacy_blob_migrated_for_direct_row_read()
            .await?;
        let run = self
            .read_row_by_key::<TurnRunRecord>(RUN_ROWS, &request.run_id.to_string())
            .await?;

        let Some(run) = run.filter(|record| record.scope == request.scope) else {
            return Ok(None);
        };
        let turn_key = run.turn_id.to_string();
        let turn = self
            .read_row_by_key::<TurnRecord>(TURN_ROWS, &turn_key)
            .await?
            .ok_or_else(|| TurnError::Unavailable {
                reason: "turn run references missing durable turn row".to_string(),
            })?;
        let run = self.runner_lease_store().overlay_run_record(run).await?;
        Ok(Some(projection::run_state_from_record(run, turn.actor)))
    }

    /// Read run state from the process-local hot snapshot (the write-behind
    /// authority). Used by cancellation reads and, under healthy write-behind,
    /// by [`get_run_state`](crate::TurnStateStore::get_run_state) to honor
    /// read-your-writes for a not-yet-flushed non-critical mutation.
    pub(crate) async fn read_run_state_from_hot_cache(
        &self,
        request: &GetRunStateRequest,
    ) -> Result<Option<TurnRunState>, TurnError> {
        let (run, turn) = {
            let mut guard = self.snapshot_state.lock().await;
            self.drop_cache_if_degraded(&mut guard);
            if guard.is_none() {
                *guard = Some(self.load_snapshot_from_rows().await?);
            }
            let Some(state) = guard.as_ref() else {
                return Ok(None);
            };
            let Some(run) = state.run_record(&request.scope, request.run_id) else {
                return Ok(None);
            };
            let turn = state.turn_record_for_run(&request.scope, &run)?;
            (run, turn)
        };
        let run = self.runner_lease_store().overlay_run_record(run).await?;
        Ok(Some(projection::run_state_from_record(run, turn.actor)))
    }

    async fn read_turn_events_from_durable_rows(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        self.flush_pending_write_behind_for_read().await?;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        self.ensure_legacy_blob_migrated_for_direct_row_read()
            .await?;
        let after_key = after.map(|cursor| format!("{:020}", cursor.0));
        let events = keyed_records(
            &self
                .read_row_collection_filtered(EVENT_ROWS, |key| {
                    after_key
                        .as_ref()
                        .is_none_or(|after_key| key > after_key.as_str())
                })
                .await?,
            &event_record_key,
        )
        .map_err(RowPersistError::into_turn)?;
        let retention_floor = self.read_meta().await?.event_retention_floor;
        let mut events = events.into_values().collect::<Vec<_>>();
        events.sort_by_key(|event| event.cursor);
        Ok(project_turn_events(
            &events,
            scope,
            owner_user_id,
            after,
            limit,
            retention_floor,
        ))
    }

    async fn read_loop_checkpoint_from_durable_rows(
        &self,
        request: &GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        self.flush_pending_write_behind_for_read().await?;
        materialize_delta_log(self.filesystem.as_ref(), &self.materialize_gate, None).await?;
        self.ensure_legacy_blob_migrated_for_direct_row_read()
            .await?;
        let key = request.checkpoint_id.as_uuid().to_string();
        let checkpoint = self
            .read_row_by_key::<LoopCheckpointRecord>(LOOP_CHECKPOINT_ROWS, &key)
            .await?;
        Ok(checkpoint.filter(|record| {
            record.scope == request.scope
                && record.turn_id == request.turn_id
                && record.run_id == request.run_id
                && record.checkpoint_id == request.checkpoint_id
        }))
    }

    async fn ensure_legacy_blob_migrated_for_direct_row_read(&self) -> Result<(), TurnError> {
        if self.read_meta().await?.journal_seq > SeqNo::ZERO {
            return Ok(());
        }
        if self.read_legacy_blob_snapshot().await?.is_none() {
            return Ok(());
        }
        let mut guard = self.snapshot_state.lock().await;
        if guard.is_none() {
            *guard = Some(self.load_snapshot_from_rows().await?);
        }
        Ok(())
    }

    async fn seed_runner_lease_from_cached_run(&self, run_id: TurnRunId) -> Result<(), TurnError> {
        let run = self
            .with_cached_snapshot(|snapshot| {
                snapshot
                    .runs
                    .iter()
                    .find(|record| record.run_id == run_id)
                    .cloned()
            })
            .await?
            .ok_or(TurnError::ScopeNotFound)?;
        self.runner_lease_store().seed_from_run_record(run).await
    }

    async fn cleanup_runner_lease_after_state(&self, result: &Result<TurnRunState, TurnError>) {
        self.runner_lease_store().cleanup_after_state(result).await;
    }

    async fn heartbeat_runner_lease(
        &self,
        request: HeartbeatRequest,
    ) -> Result<EventCursor, TurnError> {
        let lease_store = self.runner_lease_store();
        match lease_store.heartbeat(request.clone()).await {
            Err(TurnError::ScopeNotFound) => {
                self.seed_missing_runner_lease_from_snapshot(request.run_id)
                    .await?;
                self.runner_lease_store().heartbeat(request).await
            }
            result => result,
        }
    }

    async fn seed_missing_runner_lease_from_snapshot(
        &self,
        run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        let (snapshot, _version) = self.read_snapshot().await?;
        self.runner_lease_store()
            .seed_from_snapshot_if_missing(&snapshot, run_id)
            .await
    }

    async fn prepare_cancel_requested_runner_lease(
        &self,
        request: &CancelRunRequest,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        let (snapshot, _version) = self.read_snapshot().await?;
        let Some(run) = snapshot
            .runs
            .iter()
            .find(|record| record.run_id == request.run_id && record.scope == request.scope)
        else {
            return Ok(None);
        };
        if !matches!(
            run.status,
            TurnStatus::Running | TurnStatus::CancelRequested
        ) {
            return Ok(None);
        }
        self.runner_lease_store()
            .mark_cancel_requested_from_snapshot(&snapshot, request.run_id)
            .await
    }

    async fn prepare_runner_lease_retirement(
        &self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        retired_status: TurnStatus,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        let run = self
            .with_cached_snapshot(|snapshot| {
                snapshot
                    .runs
                    .iter()
                    .find(|record| record.run_id == run_id)
                    .cloned()
            })
            .await?
            .ok_or(TurnError::ScopeNotFound)?;
        self.runner_lease_store()
            .retire_runner_lease_from_run_record(run, runner_id, lease_token, retired_status)
            .await
    }

    async fn restore_runner_lease_after_failed_transition(
        &self,
        previous: Option<RunnerLeaseRecord>,
        current_status: TurnStatus,
    ) {
        let Some(previous) = previous else {
            return;
        };
        self.runner_lease_store()
            .restore_if_current_status(previous, current_status)
            .await;
    }

    fn runner_lease_store(&self) -> RunnerLeaseStore {
        RunnerLeaseStore::new(
            Arc::clone(&self.runner_leases),
            self.limits.runner_lease_ttl,
            self.apply_timeout,
        )
    }

    fn build_in_memory_store(
        &self,
        snapshot: TurnPersistenceSnapshot,
    ) -> Result<TurnStateEngine, TurnError> {
        TurnStateEngine::from_persistence_snapshot_with_admission_limit_provider(
            snapshot,
            self.limits,
            self.admission_limit_provider.clone(),
        )
    }

    /// Acquire the long-lived embedded authority with the runner-lease overlay
    /// applied, without rebuilding it from a full-snapshot clone.
    ///
    /// For [`RunnerLeaseOverlay::None`] and [`RunnerLeaseOverlay::Run`] this
    /// reuses the cached authority in place (the `Run` case overlays a single
    /// run record's live lease heartbeat onto it) — an O(1) reuse rather than a
    /// per-op O(store-size) `from_persistence_snapshot` rebuild. Only
    /// [`RunnerLeaseOverlay::All`] (expired-lease recovery) still rebuilds from
    /// an overlaid snapshot clone, because it must overlay every run's lease.
    async fn acquire_overlaid_store(
        &self,
        cached_store: Arc<TurnStateEngine>,
        overlay_baseline: Option<TurnPersistenceSnapshot>,
        overlay_run: Option<TurnRunRecord>,
        overlay: RunnerLeaseOverlay,
    ) -> Result<Arc<TurnStateEngine>, TurnError> {
        if let Some(baseline) = overlay_baseline {
            let (overlaid_snapshot, _) = self
                .runner_lease_store()
                .overlay((baseline, None), overlay)
                .await?;
            Ok(Arc::new(self.build_in_memory_store(overlaid_snapshot)?))
        } else {
            if let Some(run) = overlay_run {
                let overlaid = self.runner_lease_store().overlay_run_record(run).await?;
                cached_store.overlay_runner_lease_record(overlaid)?;
            }
            Ok(cached_store)
        }
    }

    async fn apply<T, A, Fut>(
        &self,
        overlay: RunnerLeaseOverlay,
        mut apply: A,
    ) -> Result<T, TurnError>
    where
        A: FnMut(Arc<TurnStateEngine>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, TurnError>> + Send,
        T: Send,
    {
        self.ensure_not_degraded().await?;
        let critical = async {
            let mut guard = self.snapshot_state.lock().await;
            let mut refreshed_after_stale_error = false;
            loop {
                self.ensure_snapshot_cache_for_mutation(&mut guard).await?;
                let (baseline, current_journal_seq, cached_store, overlay_baseline, overlay_run) = {
                    let state = guard.as_ref().ok_or_else(|| TurnError::Unavailable {
                        reason: "row snapshot cache was not initialized".to_string(),
                    })?;
                    let overlay_baseline =
                        matches!(overlay, RunnerLeaseOverlay::All).then(|| state.snapshot.clone());
                    let overlay_run = match overlay {
                        RunnerLeaseOverlay::Run(run_id) => state.run_record_by_id(run_id),
                        RunnerLeaseOverlay::None | RunnerLeaseOverlay::All => None,
                    };
                    (
                        state.snapshot.clone(),
                        state.journal_seq,
                        Arc::clone(&state.store),
                        overlay_baseline,
                        overlay_run,
                    )
                };
                let store = self
                    .acquire_overlaid_store(cached_store, overlay_baseline, overlay_run, overlay)
                    .await?;
                let outcome = apply(Arc::clone(&store)).await;
                let mut new_snapshot = store.persistence_snapshot();
                preserve_loop_checkpoints(&baseline, &mut new_snapshot);
                let value = match outcome {
                    Ok(value) => value,
                    Err(error) => {
                        if !refreshed_after_stale_error
                            && self
                                .refresh_snapshot_cache_after_stale_mutation_error(
                                    &mut guard, &error,
                                )
                                .await?
                        {
                            refreshed_after_stale_error = true;
                            continue;
                        }
                        self.reset_cache_after_rejected_mutation(&mut guard)?;
                        return Err(error);
                    }
                };
                if new_snapshot == baseline {
                    return Ok(RowApplyOutcome::Ready(value));
                }

                let delta = match snapshot_delta(&baseline, &new_snapshot) {
                    Ok(delta) => delta,
                    Err(RowPersistError::Turn(error)) => {
                        *guard = None;
                        return Err(error);
                    }
                };
                let persist_delta = row_store_durable_delta(delta.clone());
                // A durable-empty delta (the snapshot differs only in categories
                // the durable delta strips — run/turn tombstones, admission
                // scaffolding, retention floor — all rebuilt on load) must NOT
                // consume a reservation sequence: `enqueue_delta` skips an empty
                // delta so no backend append happens. Advancing the hot-cache
                // journal seq without a matching append desyncs it from the
                // backend append log, and a later mutation's rows/reservations
                // land one seq ahead of the real append — a subsequent active-lock
                // DELETE then collides with a stale reserved row and is dropped on
                // crash recovery (#6263). Keep the hot cache current at the SAME
                // seq and return.
                if persist_delta.is_empty() {
                    let next_state =
                        RowSnapshotState::new(new_snapshot, store, current_journal_seq)?;
                    *guard = Some(next_state);
                    return Ok(RowApplyOutcome::Ready(value));
                }
                let delta_critical = delta_is_recoverability_critical(&baseline, &persist_delta);
                let reservation_seq = current_journal_seq.next();
                let next_state = RowSnapshotState::new(new_snapshot, store, reservation_seq)?;
                // Bound the pending window BEFORE enqueue (#6263 Step 3): a
                // non-critical write-behind op reserves a slot here, under
                // `snapshot_state`, so concurrent callers can never grow the
                // journal channel past the cap while the flusher is stalled.
                // A degraded reservation → clear the hot cache (next read
                // reloads from durable) and fail fast.
                if self.write_behind_async(delta_critical)
                    && let Err(error) = self.reserve_write_behind_slot().await
                {
                    *guard = None;
                    return Err(error);
                }
                let ack = match self.enqueue_delta(persist_delta) {
                    Ok(ack) => ack,
                    Err(RowPersistError::Turn(error)) => {
                        *guard = None;
                        return Err(error);
                    }
                };
                *guard = Some(next_state);
                let ack = self
                    .track_write_behind_ack_if_async(delta_critical, ack)
                    .await;
                return Ok(RowApplyOutcome::Pending(PendingRowCommit {
                    value,
                    ack,
                    critical: delta_critical,
                }));
            }
        };

        let outcome = match tokio::time::timeout(self.apply_timeout, critical).await {
            Ok(result) => result?,
            Err(_) => {
                self.clear_snapshot_cache().await;
                return Err(TurnError::Unavailable {
                    reason: "turn state row-store apply timed out".to_string(),
                });
            }
        };
        match outcome {
            RowApplyOutcome::Ready(value) => Ok(value),
            RowApplyOutcome::Pending(pending) => {
                self.commit_pending(pending, "turn state row-store append ack timed out")
                    .await
            }
        }
    }

    fn enqueue_delta(&self, delta: SnapshotDelta) -> Result<Option<DeltaAck>, RowPersistError> {
        self.delta_journal
            .enqueue(delta)
            .map_err(RowPersistError::Turn)
    }

    async fn await_delta_ack(&self, ack: Option<DeltaAck>) -> Result<(), RowPersistError> {
        DeltaJournal::await_ack(ack)
            .await
            .map_err(RowPersistError::Turn)
    }

    async fn await_pending_commit<T>(
        &self,
        pending: PendingRowCommit<T>,
        timeout_reason: &'static str,
    ) -> Result<T, TurnError> {
        match tokio::time::timeout(self.apply_timeout, self.await_delta_ack(pending.ack)).await {
            Ok(Ok(())) => Ok(pending.value),
            Ok(Err(error)) => {
                self.clear_snapshot_cache().await;
                Err(error.into_turn())
            }
            Err(_) => {
                self.clear_snapshot_cache().await;
                Err(TurnError::Unavailable {
                    reason: timeout_reason.to_string(),
                })
            }
        }
    }

    async fn apply_with_targeted_delta<T, A, Fut, D>(
        &self,
        overlay: RunnerLeaseOverlay,
        mut apply: A,
        build_delta: D,
    ) -> Result<T, TurnError>
    where
        A: FnMut(Arc<TurnStateEngine>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, TurnError>> + Send,
        D: FnOnce(
                &TurnPersistenceSnapshot,
                EventCursor,
                &TurnStateEngine,
                &T,
            ) -> Result<SnapshotDelta, TurnError>
            + Send,
        T: Send,
    {
        self.ensure_not_degraded().await?;
        let critical = async {
            let mut guard = self.snapshot_state.lock().await;
            let mut build_delta = Some(build_delta);
            let mut refreshed_after_stale_error = false;
            loop {
                self.ensure_snapshot_cache_for_mutation(&mut guard).await?;
                let (latest_event_cursor, cached_store, overlay_baseline, overlay_run) = {
                    let state = guard.as_ref().ok_or_else(|| TurnError::Unavailable {
                        reason: "row snapshot cache was not initialized".to_string(),
                    })?;
                    let overlay_baseline =
                        matches!(overlay, RunnerLeaseOverlay::All).then(|| state.snapshot.clone());
                    let overlay_run = match overlay {
                        RunnerLeaseOverlay::Run(run_id) => state.run_record_by_id(run_id),
                        RunnerLeaseOverlay::None | RunnerLeaseOverlay::All => None,
                    };
                    (
                        state.latest_event_cursor(),
                        Arc::clone(&state.store),
                        overlay_baseline,
                        overlay_run,
                    )
                };
                let store = self
                    .acquire_overlaid_store(cached_store, overlay_baseline, overlay_run, overlay)
                    .await?;
                let outcome = apply(Arc::clone(&store)).await;
                let value = match outcome {
                    Ok(value) => value,
                    Err(error) => {
                        if !refreshed_after_stale_error
                            && self
                                .refresh_snapshot_cache_after_stale_mutation_error(
                                    &mut guard, &error,
                                )
                                .await?
                        {
                            refreshed_after_stale_error = true;
                            continue;
                        }
                        self.reset_cache_after_rejected_mutation(&mut guard)?;
                        return Err(error);
                    }
                };
                let build_delta = build_delta.take().ok_or_else(|| TurnError::Unavailable {
                    reason: "turn state row-store targeted delta builder was reused".to_string(),
                })?;
                let (delta, persist_delta, delta_critical, reservation_seq) = {
                    let state = guard.as_ref().ok_or_else(|| TurnError::Unavailable {
                        reason: "row snapshot cache was not initialized".to_string(),
                    })?;
                    let delta =
                        build_delta(&state.snapshot, latest_event_cursor, store.as_ref(), &value)?;
                    let persist_delta = row_store_durable_delta(delta.clone());
                    let delta_critical =
                        delta_is_recoverability_critical(&state.snapshot, &persist_delta);
                    (
                        delta,
                        persist_delta,
                        delta_critical,
                        state.journal_seq.next(),
                    )
                };
                // A no-op or in-memory-only mutation whose DURABLE delta is empty
                // must NOT consume a reservation sequence. `enqueue_delta` skips an
                // empty delta (no backend append happens), so advancing the
                // hot-cache journal seq here would desync it from the backend
                // append log: the next mutation's rows would be written one
                // sequence ahead of the real append, and a later active-lock
                // DELETE (materialized at the real seq) would then collide with
                // the stale row and be dropped, leaking the lock across a crash
                // (#6263). Apply the hot-cache delta at the CURRENT seq (no
                // advance), enqueue nothing, and return.
                if persist_delta.is_empty() {
                    if let Some(state) = guard.as_mut() {
                        let current_seq = state.journal_seq;
                        if let Err(error) = state.apply_delta(delta, current_seq) {
                            *guard = None;
                            return Err(error);
                        }
                        state.store = store;
                    }
                    return Ok(PendingRowCommit {
                        value,
                        ack: None,
                        critical: delta_critical,
                    });
                }
                if let Some(state) = guard.as_mut() {
                    if let Err(error) = state.apply_delta(delta, reservation_seq) {
                        *guard = None;
                        return Err(error);
                    }
                    state.store = store;
                } else {
                    let mut snapshot = store.persistence_snapshot();
                    snapshot = row_store_hot_cache_snapshot(snapshot, self.limits);
                    let next_state = match RowSnapshotState::new(snapshot, store, reservation_seq) {
                        Ok(state) => state,
                        Err(error) => {
                            *guard = None;
                            return Err(error);
                        }
                    };
                    *guard = Some(next_state);
                }
                // Bound the pending window BEFORE enqueue (#6263 Step 3): see the
                // twin reservation in the whole-snapshot apply path above.
                if self.write_behind_async(delta_critical)
                    && let Err(error) = self.reserve_write_behind_slot().await
                {
                    *guard = None;
                    return Err(error);
                }
                let ack = match self.enqueue_delta(persist_delta) {
                    Ok(ack) => ack,
                    Err(RowPersistError::Turn(error)) => {
                        *guard = None;
                        return Err(error);
                    }
                };
                let ack = self
                    .track_write_behind_ack_if_async(delta_critical, ack)
                    .await;
                return Ok(PendingRowCommit {
                    value,
                    ack,
                    critical: delta_critical,
                });
            }
        };

        let pending = match tokio::time::timeout(self.apply_timeout, critical).await {
            Ok(result) => result?,
            Err(_) => {
                self.clear_snapshot_cache().await;
                return Err(TurnError::Unavailable {
                    reason: "turn state row-store targeted apply timed out".to_string(),
                });
            }
        };
        self.commit_pending(
            pending,
            "turn state row-store targeted append ack timed out",
        )
        .await
    }

    async fn apply_run_state_transition<A, Fut>(
        &self,
        operation: &'static str,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        retired_status: TurnStatus,
        apply: A,
    ) -> Result<TurnRunState, TurnError>
    where
        A: FnMut(Arc<TurnStateEngine>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<TurnRunState, TurnError>> + Send,
    {
        let span = turn_state_write_span(operation, None, Some(&run_id));
        async move {
            let previous = self
                .prepare_runner_lease_retirement(run_id, runner_id, lease_token, retired_status)
                .await?;
            let result = self.apply(RunnerLeaseOverlay::Run(run_id), apply).await;
            if result.is_err() {
                self.restore_runner_lease_after_failed_transition(previous, retired_status)
                    .await;
            }
            self.cleanup_runner_lease_after_state(&result).await;
            result
        }
        .instrument(span)
        .await
    }

    async fn apply_run_state_transition_with_targeted_delta<A, Fut, D>(
        &self,
        operation: &'static str,
        target: RunStateTransitionTarget,
        apply: A,
        build_delta: D,
    ) -> Result<TurnRunState, TurnError>
    where
        A: FnMut(Arc<TurnStateEngine>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<TurnRunState, TurnError>> + Send,
        D: FnOnce(
                &TurnPersistenceSnapshot,
                EventCursor,
                &TurnStateEngine,
                &TurnRunState,
            ) -> Result<SnapshotDelta, TurnError>
            + Send,
    {
        let RunStateTransitionTarget {
            run_id,
            runner_id,
            lease_token,
            retired_status,
        } = target;
        let span = turn_state_write_span(operation, None, Some(&run_id));
        async move {
            let previous = self
                .prepare_runner_lease_retirement(run_id, runner_id, lease_token, retired_status)
                .await?;
            let result = self
                .apply_with_targeted_delta(RunnerLeaseOverlay::Run(run_id), apply, build_delta)
                .await;
            if result.is_err() {
                self.restore_runner_lease_after_failed_transition(previous, retired_status)
                    .await;
            }
            self.cleanup_runner_lease_after_state(&result).await;
            result
        }
        .instrument(span)
        .await
    }

    async fn compensate_failed_claim(&self, claimed: &ClaimedTurnRun) {
        let run_id = claimed.state.run_id;
        let result = self
            .apply(RunnerLeaseOverlay::Run(run_id), |store| async move {
                let outcome = store
                    .relinquish_run(RelinquishRunRequest {
                        run_id,
                        runner_id: claimed.runner_id,
                        lease_token: claimed.lease_token,
                    })
                    .await;
                outcome.map(|_| ())
            })
            .instrument(turn_state_write_span(
                "compensate_failed_claim",
                Some(&claimed.state.scope),
                Some(&run_id),
            ))
            .await;
        if let Err(error) = result {
            tracing::debug!(
                run_id = %run_id,
                error = %error,
                "failed to compensate turn claim after memory runner lease seed failed"
            );
        }
    }
}

/// Whether a durable delta carries a recoverability-critical run transition:
/// gate-park, terminal (the production [`crate::is_recoverability_critical`]
/// boundary), OR the first durable row for a run `baseline` has never seen.
/// This decides the sync-durable barrier vs. the async write-behind path; a
/// delta touching only ALREADY-durable runs (claim/relinquish churn, loop
/// checkpoints, tree reservations) is non-critical — losing its trailing async
/// tail on crash still leaves the run recoverable from its last durable row.
/// A brand-new run (`submit_turn`, `submit_child_turn`, and the runs
/// `resume_turn`/`retry_turn` spawn) has no such fallback: if its creation
/// never reaches the durable log, the run has no trace at all and the caller's
/// `Ok` response describes a run that crash recovery can never find. Keep
/// every new-run creation on the synchronous barrier.
fn delta_is_recoverability_critical(
    baseline: &TurnPersistenceSnapshot,
    delta: &SnapshotDelta,
) -> bool {
    delta.runs_upsert.iter().any(|run| {
        crate::is_recoverability_critical(run.status)
            || !baseline
                .runs
                .iter()
                .any(|existing| existing.run_id == run.run_id)
    })
}

fn turn_state_write_span(
    operation: &'static str,
    scope: Option<&TurnScope>,
    run_id: Option<&TurnRunId>,
) -> tracing::Span {
    let span = tracing::trace_span!(
        target: "ironclaw_latency",
        "turn_state_write",
        turn_state_op = operation,
        tenant_id = field::Empty,
        thread_id = field::Empty,
        owner_user_id = field::Empty,
        run_id = field::Empty,
    );

    if let Some(scope) = scope {
        span.record("tenant_id", field::display(&scope.tenant_id));
        span.record("thread_id", field::display(&scope.thread_id));
        if let Some(owner_user_id) = scope.explicit_owner_user_id() {
            span.record("owner_user_id", field::display(owner_user_id));
        }
    }

    if let Some(run_id) = run_id {
        span.record("run_id", field::display(&run_id));
    }

    span
}
