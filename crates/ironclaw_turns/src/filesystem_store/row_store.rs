use std::{collections::HashMap, sync::Arc, time::Duration};

use ironclaw_filesystem::{
    FILESYSTEM_APPLY_TIMEOUT, FileType, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{ResourceScope, UserId};
use serde::de::DeserializeOwned;
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use tracing::{Instrument, field};

use crate::{
    AllowAllTurnAdmissionLimitProvider, CancelRunRequest, EventCursor, GetRunStateRequest,
    InMemoryTurnStateStore, InMemoryTurnStateStoreLimits, TurnAdmissionLimitProvider, TurnError,
    TurnEventPage, TurnPersistenceSnapshot, TurnRecord, TurnRunId, TurnRunRecord, TurnRunState,
    TurnScope, TurnStatus,
    events::project_turn_events,
    runner::{ClaimedTurnRun, HeartbeatRequest, RelinquishRunRequest, TurnRunTransitionPort},
};

use super::{
    projection,
    runner_lease::{RunnerLeaseMemory, RunnerLeaseOverlay, RunnerLeaseRecord, RunnerLeaseStore},
};

mod delta;
mod io;
mod journal;
mod traits;

use delta::{
    RowPersistError, RowSnapshotState, RowStoreMeta, SnapshotDelta, event_record_key,
    keyed_records, preserve_loop_checkpoints, row_store_durable_delta,
    row_store_hot_cache_snapshot, snapshot_delta,
};
use io::{deserialize_row, fs_error, meta_path, row_dir, row_path};
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

/// Filesystem-backed turn-state store using typed append-log deltas.
///
/// This is intentionally separate from [`super::FilesystemTurnStateStore`].
/// The blob store preserves the current `/turns/state.json` contract while this
/// store lets stress compare a narrower persistence layout before production
/// wiring changes. Transitions still delegate to [`InMemoryTurnStateStore`];
/// only the durable representation changes from whole-snapshot CAS to a typed
/// append log plus a process-local hot snapshot cache.
pub struct FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    limits: InMemoryTurnStateStoreLimits,
    admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    snapshot_state: AsyncMutex<Option<RowSnapshotState>>,
    commit_gate: AsyncMutex<()>,
    runner_leases: RunnerLeaseMemory,
    delta_journal: DeltaJournal,
    apply_timeout: Duration,
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
        Self {
            filesystem: Arc::clone(&filesystem),
            limits: InMemoryTurnStateStoreLimits::default(),
            admission_limit_provider: Arc::new(AllowAllTurnAdmissionLimitProvider),
            snapshot_state: AsyncMutex::new(None),
            commit_gate: AsyncMutex::new(()),
            runner_leases: Arc::new(RwLock::new(HashMap::new())),
            delta_journal: DeltaJournal::new(filesystem),
            apply_timeout: FILESYSTEM_APPLY_TIMEOUT,
        }
    }

    pub fn with_limits(mut self, limits: InMemoryTurnStateStoreLimits) -> Self {
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

    pub async fn persistence_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let (snapshot, _) = self
            .read_snapshot_with_runner_lease_overlay(RunnerLeaseOverlay::All)
            .await?;
        Ok(snapshot)
    }

    async fn read_snapshot(
        &self,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        let mut guard = self.snapshot_state.lock().await;
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
        if guard.is_none() {
            *guard = Some(self.load_snapshot_from_rows().await?);
        }
        let snapshot = &guard
            .as_ref()
            .expect("row snapshot cache is initialized above")
            .snapshot;
        Ok(read(snapshot))
    }

    async fn clear_snapshot_cache(&self) {
        *self.snapshot_state.lock().await = None;
    }

    async fn load_snapshot_from_rows(&self) -> Result<RowSnapshotState, TurnError> {
        materialize_delta_log(self.filesystem.as_ref(), None).await?;
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

        let snapshot = TurnPersistenceSnapshot {
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
        };
        let snapshot = row_store_hot_cache_snapshot(snapshot, self.limits);
        let store = self.build_in_memory_store(snapshot)?;
        let snapshot = store.persistence_snapshot();
        RowSnapshotState::new(snapshot, Arc::new(store))
    }

    async fn read_meta(&self) -> Result<RowStoreMeta, TurnError> {
        let path = meta_path()?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => deserialize_row(&versioned.entry.body, "turn-state row meta"),
            Ok(None) => Ok(RowStoreMeta::default()),
            Err(error) => Err(fs_error(error)),
        }
    }

    async fn read_row_collection<T>(&self, collection: &'static str) -> Result<Vec<T>, TurnError>
    where
        T: DeserializeOwned,
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
        let mut records = Vec::with_capacity(entries.len());
        for entry in entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::File)
            .filter(|entry| entry.name.ends_with(".json"))
        {
            let key = entry.name.trim_end_matches(".json").to_string();
            let path = row_path(collection, &key)?;
            let Some(versioned) = self
                .filesystem
                .get(&ResourceScope::system(), &path)
                .await
                .map_err(fs_error)?
            else {
                continue;
            };
            records.push(deserialize_row(&versioned.entry.body, collection)?);
        }
        Ok(records)
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
        let Some(versioned) = self
            .filesystem
            .get(&ResourceScope::system(), &path)
            .await
            .map_err(fs_error)?
        else {
            return Ok(None);
        };
        deserialize_row(&versioned.entry.body, collection).map(Some)
    }

    async fn read_run_state_from_durable_rows(
        &self,
        request: &GetRunStateRequest,
    ) -> Result<Option<TurnRunState>, TurnError> {
        materialize_delta_log(self.filesystem.as_ref(), None).await?;
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

    async fn read_turn_events_from_durable_rows(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        materialize_delta_log(self.filesystem.as_ref(), None).await?;
        let events = keyed_records(
            &self.read_row_collection(EVENT_ROWS).await?,
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
        let (snapshot, _version) = self.read_snapshot().await?;
        self.runner_lease_store()
            .retire_runner_lease_from_snapshot(
                &snapshot,
                run_id,
                runner_id,
                lease_token,
                retired_status,
            )
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
    ) -> Result<InMemoryTurnStateStore, TurnError> {
        InMemoryTurnStateStore::from_persistence_snapshot_with_admission_limit_provider(
            snapshot,
            self.limits,
            self.admission_limit_provider.clone(),
        )
    }

    async fn apply<T, A, Fut>(
        &self,
        overlay: RunnerLeaseOverlay,
        mut apply: A,
    ) -> Result<T, TurnError>
    where
        A: FnMut(Arc<InMemoryTurnStateStore>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, TurnError>> + Send,
        T: Send,
    {
        let critical = async {
            let _commit_guard = self.commit_gate.lock().await;
            let mut guard = self.snapshot_state.lock().await;
            if guard.is_none() {
                *guard = Some(self.load_snapshot_from_rows().await?);
            }
            let baseline = guard
                .as_ref()
                .map(|state| state.snapshot.clone())
                .unwrap_or_default();
            let (overlaid_snapshot, _) = self
                .runner_lease_store()
                .overlay((baseline.clone(), None), overlay)
                .await?;
            let store = Arc::new(self.build_in_memory_store(overlaid_snapshot)?);
            let outcome = apply(Arc::clone(&store)).await;
            let mut new_snapshot = store.persistence_snapshot();
            preserve_loop_checkpoints(&baseline, &mut new_snapshot);
            let value = match outcome {
                Ok(value) => value,
                Err(error) => {
                    *guard = None;
                    return Err(error);
                }
            };
            if new_snapshot == baseline {
                return Ok((None, value));
            }

            let delta = match snapshot_delta(&baseline, &new_snapshot) {
                Ok(delta) => delta,
                Err(RowPersistError::Turn(error)) => {
                    *guard = None;
                    return Err(error);
                }
            };
            let persist_delta = row_store_durable_delta(delta.clone());
            let ack = match self.enqueue_delta(persist_delta) {
                Ok(ack) => ack,
                Err(RowPersistError::Turn(error)) => {
                    *guard = None;
                    return Err(error);
                }
            };
            *guard = Some(RowSnapshotState::new(new_snapshot, store)?);
            Ok((ack, value))
        };

        let (ack, value) = match tokio::time::timeout(self.apply_timeout, critical).await {
            Ok(result) => result?,
            Err(_) => {
                self.clear_snapshot_cache().await;
                return Err(TurnError::Unavailable {
                    reason: "turn state row-store apply timed out".to_string(),
                });
            }
        };
        if let Err(error) = self.await_delta_ack(ack).await {
            self.clear_snapshot_cache().await;
            return Err(error.into_turn());
        }
        Ok(value)
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

    async fn apply_cached_delta(&self, delta: SnapshotDelta) -> Result<(), TurnError> {
        if delta.is_empty() {
            return Ok(());
        }
        let mut guard = self.snapshot_state.lock().await;
        if let Some(state) = guard.as_mut() {
            state.apply_delta(delta)?;
        }
        Ok(())
    }

    async fn apply_with_targeted_delta<T, A, Fut, D>(
        &self,
        overlay: RunnerLeaseOverlay,
        mut apply: A,
        build_delta: D,
    ) -> Result<T, TurnError>
    where
        A: FnMut(Arc<InMemoryTurnStateStore>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<T, TurnError>> + Send,
        D: FnOnce(
                &TurnPersistenceSnapshot,
                EventCursor,
                &InMemoryTurnStateStore,
                &T,
            ) -> Result<SnapshotDelta, TurnError>
            + Send,
        T: Send,
    {
        let critical = async {
            let _commit_guard = self.commit_gate.lock().await;
            let mut guard = self.snapshot_state.lock().await;
            if guard.is_none() {
                *guard = Some(self.load_snapshot_from_rows().await?);
            }
            let state = guard
                .as_ref()
                .expect("row snapshot cache is initialized above");
            let baseline = state.snapshot.clone();
            let latest_event_cursor = state.latest_event_cursor();
            let (overlaid_snapshot, _) = self
                .runner_lease_store()
                .overlay((baseline.clone(), None), overlay)
                .await?;
            let store = Arc::new(self.build_in_memory_store(overlaid_snapshot)?);
            let outcome = apply(Arc::clone(&store)).await;
            let value = match outcome {
                Ok(value) => value,
                Err(error) => {
                    *guard = None;
                    return Err(error);
                }
            };
            let delta = build_delta(&baseline, latest_event_cursor, store.as_ref(), &value)?;
            let persist_delta = row_store_durable_delta(delta.clone());
            let ack = match self.enqueue_delta(persist_delta) {
                Ok(ack) => ack,
                Err(RowPersistError::Turn(error)) => {
                    *guard = None;
                    return Err(error);
                }
            };
            if let Some(state) = guard.as_mut() {
                if let Err(error) = state.apply_delta(delta) {
                    *guard = None;
                    return Err(error);
                }
                state.store = store;
            } else {
                let mut snapshot = store.persistence_snapshot();
                snapshot = row_store_hot_cache_snapshot(snapshot, self.limits);
                *guard = Some(RowSnapshotState::new(snapshot, store)?);
            }
            Ok((ack, value))
        };

        let (ack, value) = match tokio::time::timeout(self.apply_timeout, critical).await {
            Ok(result) => result?,
            Err(_) => {
                self.clear_snapshot_cache().await;
                return Err(TurnError::Unavailable {
                    reason: "turn state row-store targeted apply timed out".to_string(),
                });
            }
        };
        if let Err(error) = self.await_delta_ack(ack).await {
            self.clear_snapshot_cache().await;
            return Err(error.into_turn());
        }
        Ok(value)
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
        A: FnMut(Arc<InMemoryTurnStateStore>) -> Fut + Send,
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
        A: FnMut(Arc<InMemoryTurnStateStore>) -> Fut + Send,
        Fut: std::future::Future<Output = Result<TurnRunState, TurnError>> + Send,
        D: FnOnce(
                &TurnPersistenceSnapshot,
                EventCursor,
                &InMemoryTurnStateStore,
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
