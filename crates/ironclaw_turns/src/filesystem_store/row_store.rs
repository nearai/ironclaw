use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use ironclaw_filesystem::{
    FILESYSTEM_APPLY_TIMEOUT, RecordVersion, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::UserId;
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use tracing::field;

use crate::{
    AllowAllTurnAdmissionLimitProvider, CancelRunRequest, EventCursor, TurnAdmissionLimitProvider,
    TurnError, TurnPersistenceSnapshot, TurnRunId, TurnRunState, TurnScope, TurnStateStoreLimits,
    TurnStatus, runner::HeartbeatRequest,
};

use super::{
    runner_lease::{RunnerLeaseMemory, RunnerLeaseOverlay, RunnerLeaseRecord, RunnerLeaseStore},
    turn_state_engine::TurnStateEngine,
};

mod commit;
mod delta;
mod events_index;
mod io;
mod journal;
mod load;
mod traits;
mod write_behind;

use delta::{RowSnapshotState, SnapshotDelta};
use journal::{DeltaAck, DeltaJournal, materialize_delta_log};

/// The nine persisted row collections. A fixed dispatch set is an enum, not
/// stringly-typed `&'static str` constants (`.claude/rules/types.md`).
///
/// [`as_str`](RowCollection::as_str) returns the on-disk path segment for each
/// variant. Those strings are part of the durable layout — the row directory
/// name and legacy-migration compatibility depend on them — and MUST stay
/// byte-identical to the historical constant values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowCollection {
    Turns,
    Runs,
    ActiveLocks,
    Checkpoints,
    LoopCheckpoints,
    Idempotency,
    Events,
    AdmissionReservations,
    SpawnTreeReservations,
}

impl RowCollection {
    /// The on-disk path segment for this collection. These strings are durable
    /// layout and MUST NOT change (see the type-level note).
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Turns => "turns",
            Self::Runs => "runs",
            Self::ActiveLocks => "active-locks",
            Self::Checkpoints => "checkpoints",
            Self::LoopCheckpoints => "loop-checkpoints",
            Self::Idempotency => "idempotency",
            Self::Events => "events",
            Self::AdmissionReservations => "admission-reservations",
            Self::SpawnTreeReservations => "spawn-tree-reservations",
        }
    }
}

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
/// This is the one production turn-state store.
/// When the row projection is empty, first load imports a legacy
/// `/turns/state.json` blob by appending a full-snapshot row delta and then
/// replaying the normal delta journal. Once any row data exists, rows are
/// authoritative and the legacy blob is left untouched as rollback evidence.
/// Transitions still delegate to [`TurnStateEngine`]; only the durable
/// representation changes from whole-snapshot CAS to a typed append log plus a
/// process-local hot snapshot cache.
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
    /// One-time-per-process readiness of the durable event-row index used by
    /// the query-backed `read_turn_events_after` path: declares the event
    /// indexes and runs the pre-index-projection backfill exactly once. Caches
    /// `true` when the query path is usable and `false` when the mount does not
    /// support `query`/`ensure_index` (byte-only fallback backends), so the
    /// read path degrades to the legacy directory scan without re-probing.
    events_index_ready: tokio::sync::OnceCell<bool>,
}

struct PendingRowCommit<T> {
    value: T,
    /// `Some` only for a critical write-behind barrier that `commit_pending`
    /// must await; `None` for a no-op commit or a non-critical write-behind op
    /// already tracked in the bounded pending window by `apply`. Criticality is
    /// decided upstream by `track_write_behind_ack_if_async`, which nulls the
    /// ack on the non-critical path — so this field alone drives the await.
    ack: Option<DeltaAck>,
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
            events_index_ready: tokio::sync::OnceCell::new(),
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

#[cfg(test)]
mod tests {
    use super::RowCollection;

    /// The on-disk path segment for every collection is durable layout: it names
    /// the row directory and legacy-blob migration depends on it. These strings
    /// MUST stay byte-identical to the historical `&'static str` constants the
    /// enum replaced. Pin each one.
    #[test]
    fn row_collection_as_str_matches_historical_path_segments() {
        assert_eq!(RowCollection::Turns.as_str(), "turns");
        assert_eq!(RowCollection::Runs.as_str(), "runs");
        assert_eq!(RowCollection::ActiveLocks.as_str(), "active-locks");
        assert_eq!(RowCollection::Checkpoints.as_str(), "checkpoints");
        assert_eq!(RowCollection::LoopCheckpoints.as_str(), "loop-checkpoints");
        assert_eq!(RowCollection::Idempotency.as_str(), "idempotency");
        assert_eq!(RowCollection::Events.as_str(), "events");
        assert_eq!(
            RowCollection::AdmissionReservations.as_str(),
            "admission-reservations"
        );
        assert_eq!(
            RowCollection::SpawnTreeReservations.as_str(),
            "spawn-tree-reservations"
        );
    }
}
