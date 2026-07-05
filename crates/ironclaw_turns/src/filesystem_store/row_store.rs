use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    FILESYSTEM_APPLY_TIMEOUT, FileType, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem, SeqNo,
};
use ironclaw_host_api::{ResourceScope, ScopedPath, UserId};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use tracing::{Instrument, field};

use crate::{
    AllowAllTurnAdmissionLimitProvider, CancelRunRequest, CancelRunResponse, EventCursor,
    GetLoopCheckpointRequest, GetRunStateRequest, InMemoryTurnStateStore,
    InMemoryTurnStateStoreLimits, LoopCheckpointRecord, LoopCheckpointStore,
    PutLoopCheckpointRequest, ResumeTurnRequest, ResumeTurnResponse, RunProfileResolver,
    SpawnTreeReservation, SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse,
    TurnActiveLockRecord, TurnAdmissionLimitProvider, TurnAdmissionPolicy,
    TurnAdmissionReservationRecord, TurnCheckpointId, TurnCheckpointRecord, TurnError,
    TurnEventPage, TurnEventProjectionSource, TurnIdempotencyRecord, TurnLifecycleEvent,
    TurnPersistenceSnapshot, TurnRecord, TurnRunId, TurnRunRecord, TurnRunState, TurnScope,
    TurnSpawnTreeStateStore, TurnStateStore, TurnStatus,
    events::project_turn_events,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest, RecoverExpiredLeasesRequest,
        RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
        TurnRunnerOutcome,
    },
};

use super::{
    profile_resolver::PreResolvedRunProfileResolver,
    projection,
    runner_lease::{RunnerLeaseMemory, RunnerLeaseOverlay, RunnerLeaseRecord, RunnerLeaseStore},
};
const ROW_ROOT: &str = "/turns/rows/v1";
const META_DIR: &str = "meta";
const META_FILE: &str = "state.json";
const TURN_ROWS: &str = "turns";
const RUN_ROWS: &str = "runs";
const ACTIVE_LOCK_ROWS: &str = "active-locks";
const CHECKPOINT_ROWS: &str = "checkpoints";
const LOOP_CHECKPOINT_ROWS: &str = "loop-checkpoints";
const IDEMPOTENCY_ROWS: &str = "idempotency";
const EVENT_ROWS: &str = "events";
const ADMISSION_RESERVATION_ROWS: &str = "admission-reservations";
const SPAWN_TREE_RESERVATION_ROWS: &str = "spawn-tree-reservations";
const DELTA_LOG: &str = "deltas/log";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
struct RowStoreMeta {
    event_retention_floor: EventCursor,
}

impl Default for RowStoreMeta {
    fn default() -> Self {
        Self {
            event_retention_floor: EventCursor::default(),
        }
    }
}

struct RowSnapshotState {
    snapshot: TurnPersistenceSnapshot,
    store: Arc<InMemoryTurnStateStore>,
    latest_event_cursor: EventCursor,
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
struct SnapshotDelta {
    turns_upsert: Vec<TurnRecord>,
    turns_delete: Vec<String>,
    runs_upsert: Vec<TurnRunRecord>,
    runs_delete: Vec<String>,
    active_locks_upsert: Vec<TurnActiveLockRecord>,
    active_locks_delete: Vec<String>,
    checkpoints_upsert: Vec<TurnCheckpointRecord>,
    checkpoints_delete: Vec<String>,
    loop_checkpoints_upsert: Vec<LoopCheckpointRecord>,
    loop_checkpoints_delete: Vec<String>,
    idempotency_upsert: Vec<TurnIdempotencyRecord>,
    idempotency_delete: Vec<String>,
    events_upsert: Vec<TurnLifecycleEvent>,
    events_delete: Vec<String>,
    admission_reservations_upsert: Vec<TurnAdmissionReservationRecord>,
    admission_reservations_delete: Vec<String>,
    spawn_tree_reservations_upsert: Vec<SpawnTreeReservation>,
    spawn_tree_reservations_delete: Vec<String>,
    event_retention_floor: Option<EventCursor>,
}

impl SnapshotDelta {
    fn is_empty(&self) -> bool {
        self.turns_upsert.is_empty()
            && self.turns_delete.is_empty()
            && self.runs_upsert.is_empty()
            && self.runs_delete.is_empty()
            && self.active_locks_upsert.is_empty()
            && self.active_locks_delete.is_empty()
            && self.checkpoints_upsert.is_empty()
            && self.checkpoints_delete.is_empty()
            && self.loop_checkpoints_upsert.is_empty()
            && self.loop_checkpoints_delete.is_empty()
            && self.idempotency_upsert.is_empty()
            && self.idempotency_delete.is_empty()
            && self.events_upsert.is_empty()
            && self.events_delete.is_empty()
            && self.admission_reservations_upsert.is_empty()
            && self.admission_reservations_delete.is_empty()
            && self.spawn_tree_reservations_upsert.is_empty()
            && self.spawn_tree_reservations_delete.is_empty()
            && self.event_retention_floor.is_none()
    }
}

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
    runner_leases: RunnerLeaseMemory,
    apply_timeout: Duration,
}

impl<F> FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            limits: InMemoryTurnStateStoreLimits::default(),
            admission_limit_provider: Arc::new(AllowAllTurnAdmissionLimitProvider),
            snapshot_state: AsyncMutex::new(None),
            runner_leases: Arc::new(RwLock::new(HashMap::new())),
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

    #[cfg(test)]
    pub(crate) fn with_apply_timeout(mut self, apply_timeout: Duration) -> Self {
        self.apply_timeout = apply_timeout;
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

        let mut snapshot = TurnPersistenceSnapshot {
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
        self.replay_deltas(&mut snapshot).await?;
        let store = self.build_in_memory_store(snapshot)?;
        let snapshot = store.persistence_snapshot();
        let latest_event_cursor = latest_event_cursor(&snapshot);
        Ok(RowSnapshotState {
            snapshot,
            store: Arc::new(store),
            latest_event_cursor,
        })
    }

    async fn replay_deltas(&self, snapshot: &mut TurnPersistenceSnapshot) -> Result<(), TurnError> {
        let path = delta_log_path()?;
        let records = match self
            .filesystem
            .tail(&ResourceScope::system(), &path, SeqNo::ZERO)
            .await
        {
            Ok(records) => records,
            Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::Unsupported { .. }) => {
                Vec::new()
            }
            Err(error) => return Err(fs_error(error)),
        };
        for record in records {
            let delta: SnapshotDelta = deserialize_row(&record.payload, "turn-state delta")?;
            apply_delta(snapshot, delta)?;
        }
        Ok(())
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
        let operation = async {
            let mut guard = self.snapshot_state.lock().await;
            if guard.is_none() {
                *guard = Some(self.load_snapshot_from_rows().await?);
            }
            let store = match (overlay, guard.as_ref()) {
                (RunnerLeaseOverlay::None, Some(state)) => Arc::clone(&state.store),
                (_, Some(state)) => {
                    let snapshot = state.snapshot.clone();
                    let (overlaid_snapshot, _) = self
                        .runner_lease_store()
                        .overlay((snapshot, None), overlay)
                        .await?;
                    Arc::new(self.build_in_memory_store(overlaid_snapshot)?)
                }
                (_, None) => unreachable!("row snapshot cache is initialized above"),
            };
            let baseline = guard
                .as_ref()
                .map(|state| state.snapshot.clone())
                .unwrap_or_default();
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
                return Ok(value);
            }

            match self.persist_snapshot_diff(&baseline, &new_snapshot).await {
                Ok(()) => {
                    let latest_event_cursor = latest_event_cursor(&new_snapshot);
                    *guard = Some(RowSnapshotState {
                        snapshot: new_snapshot,
                        store,
                        latest_event_cursor,
                    });
                    Ok(value)
                }
                Err(RowPersistError::Turn(error)) => {
                    *guard = None;
                    Err(error)
                }
            }
        };

        match tokio::time::timeout(self.apply_timeout, operation).await {
            Ok(result) => result,
            Err(_) => {
                self.clear_snapshot_cache().await;
                Err(TurnError::Unavailable {
                    reason: "turn state row-store apply timed out".to_string(),
                })
            }
        }
    }

    async fn persist_snapshot_diff(
        &self,
        old: &TurnPersistenceSnapshot,
        new: &TurnPersistenceSnapshot,
    ) -> Result<(), RowPersistError> {
        let delta = snapshot_delta(old, new)?;
        self.persist_delta(&delta).await
    }

    async fn persist_delta(&self, delta: &SnapshotDelta) -> Result<(), RowPersistError> {
        if delta.is_empty() {
            return Ok(());
        }
        let payload = serde_json::to_vec(&delta).map_err(|error| {
            RowPersistError::Turn(TurnError::Unavailable {
                reason: format!("turn-state delta serialization failed: {error}"),
            })
        })?;
        let path = delta_log_path().map_err(RowPersistError::Turn)?;
        match self
            .filesystem
            .append(&ResourceScope::system(), &path, payload)
            .await
        {
            Ok(_seq) => {}
            Err(error) => return Err(RowPersistError::Turn(fs_error(error))),
        }
        Ok(())
    }

    async fn apply_cached_delta(&self, delta: SnapshotDelta) -> Result<(), TurnError> {
        if delta.is_empty() {
            return Ok(());
        }
        let mut guard = self.snapshot_state.lock().await;
        if let Some(state) = guard.as_mut() {
            let latest_event_cursor =
                latest_event_cursor_after_delta(state.latest_event_cursor, &delta);
            apply_delta(&mut state.snapshot, delta)?;
            state.latest_event_cursor = latest_event_cursor;
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
        let operation = async {
            let mut guard = self.snapshot_state.lock().await;
            if guard.is_none() {
                *guard = Some(self.load_snapshot_from_rows().await?);
            }
            let state = guard
                .as_mut()
                .expect("row snapshot cache is initialized above");
            let store = match overlay {
                RunnerLeaseOverlay::None => Arc::clone(&state.store),
                RunnerLeaseOverlay::Run(run_id) => {
                    let store = Arc::clone(&state.store);
                    if let Some(run) = state
                        .snapshot
                        .runs
                        .iter()
                        .find(|record| record.run_id == run_id)
                        .cloned()
                    {
                        let overlaid = self.runner_lease_store().overlay_run_record(run).await?;
                        store.overlay_runner_lease_record(overlaid)?;
                    }
                    store
                }
                _ => {
                    let (overlaid_snapshot, _) = self
                        .runner_lease_store()
                        .overlay((state.snapshot.clone(), None), overlay)
                        .await?;
                    Arc::new(self.build_in_memory_store(overlaid_snapshot)?)
                }
            };
            let outcome = apply(Arc::clone(&store)).await;
            let value = match outcome {
                Ok(value) => value,
                Err(error) => {
                    *guard = None;
                    return Err(error);
                }
            };
            let delta = build_delta(
                &state.snapshot,
                state.latest_event_cursor,
                store.as_ref(),
                &value,
            )?;
            let latest_event_cursor =
                latest_event_cursor_after_delta(state.latest_event_cursor, &delta);
            match self.persist_delta(&delta).await {
                Ok(()) => {
                    apply_delta(&mut state.snapshot, delta)?;
                    state.latest_event_cursor = latest_event_cursor;
                    state.store = store;
                    Ok(value)
                }
                Err(RowPersistError::Turn(error)) => {
                    *guard = None;
                    Err(error)
                }
            }
        };

        match tokio::time::timeout(self.apply_timeout, operation).await {
            Ok(result) => result,
            Err(_) => {
                self.clear_snapshot_cache().await;
                Err(TurnError::Unavailable {
                    reason: "turn state row-store targeted apply timed out".to_string(),
                })
            }
        }
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
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        retired_status: TurnStatus,
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

#[async_trait]
impl<F> TurnStateStore for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let profile_resolution = run_profile_resolver
            .resolve_run_profile(crate::RunProfileResolutionRequest {
                requested_run_profile: request.requested_run_profile.clone(),
                ..crate::RunProfileResolutionRequest::interactive_default()
            })
            .await;
        let pre_resolved = PreResolvedRunProfileResolver::new(profile_resolution);
        let max_idempotency_records = self.limits.max_idempotency_records;
        self.apply_with_targeted_delta(
            RunnerLeaseOverlay::None,
            |store| {
                let request = request.clone();
                let pre_resolved = pre_resolved.clone();
                async move {
                    store
                        .submit_turn(request, admission_policy, &pre_resolved)
                        .await
                }
            },
            move |snapshot, latest_event_cursor, store, response| {
                if snapshot.idempotency_records.len() >= max_idempotency_records {
                    return full_snapshot_delta(snapshot, store);
                }
                submit_turn_targeted_delta(snapshot, latest_event_cursor, store, response)
            },
        )
        .instrument(turn_state_write_span(
            "submit_turn",
            Some(&request.scope),
            request.requested_run_id.as_ref(),
        ))
        .await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let max_idempotency_records = self.limits.max_idempotency_records;
        let scope = request.scope.clone();
        let run_id = request.run_id;
        self.apply_with_targeted_delta(
            RunnerLeaseOverlay::None,
            |store| {
                let request = request.clone();
                async move { store.resume_turn(request).await }
            },
            move |snapshot, latest_event_cursor, store, response| {
                if snapshot.idempotency_records.len() >= max_idempotency_records {
                    return full_snapshot_delta(snapshot, store);
                }
                run_state_with_idempotency_targeted_delta(
                    snapshot,
                    latest_event_cursor,
                    store,
                    response.run_id,
                    &scope,
                    crate::TurnIdempotencyOperationKind::Resume,
                )
            },
        )
        .instrument(turn_state_write_span(
            "resume_turn",
            Some(&request.scope),
            Some(&run_id),
        ))
        .await
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let span = turn_state_write_span(
            "request_cancel",
            Some(&request.scope),
            Some(&request.run_id),
        );
        async move {
            let previous = self.prepare_cancel_requested_runner_lease(&request).await?;
            let max_idempotency_records = self.limits.max_idempotency_records;
            let max_terminal_records = self.limits.max_terminal_records;
            let scope = request.scope.clone();
            let result = self
                .apply_with_targeted_delta(
                    RunnerLeaseOverlay::Run(request.run_id),
                    |store| {
                        let request = request.clone();
                        async move { store.request_cancel(request).await }
                    },
                    move |snapshot, latest_event_cursor, store, response| {
                        if snapshot.idempotency_records.len() >= max_idempotency_records {
                            return full_snapshot_delta(snapshot, store);
                        }
                        let terminal_records = snapshot
                            .runs
                            .iter()
                            .filter(|record| record.status.is_terminal())
                            .count();
                        if response.status.is_terminal() && terminal_records >= max_terminal_records
                        {
                            return full_snapshot_delta(snapshot, store);
                        }
                        run_state_with_idempotency_targeted_delta(
                            snapshot,
                            latest_event_cursor,
                            store,
                            response.run_id,
                            &scope,
                            crate::TurnIdempotencyOperationKind::Cancel,
                        )
                    },
                )
                .await;
            if result.is_err() {
                self.restore_runner_lease_after_failed_transition(
                    previous,
                    TurnStatus::CancelRequested,
                )
                .await;
            }
            let response = result?;
            if response.status.is_terminal() {
                self.runner_lease_store()
                    .delete_best_effort(response.run_id)
                    .await;
            }
            Ok(response)
        }
        .instrument(span)
        .await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        let Some((run, actor)) = self
            .with_cached_snapshot(|snapshot| projection::run_state_parts(snapshot, &request))
            .await??
        else {
            return Err(TurnError::ScopeNotFound);
        };
        let run = self.runner_lease_store().overlay_run_record(run).await?;
        Ok(projection::run_state_from_record(run, actor))
    }
}

#[async_trait]
impl<F> TurnSpawnTreeStateStore for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        let profile_resolution = run_profile_resolver
            .resolve_run_profile(crate::RunProfileResolutionRequest {
                requested_run_profile: request.requested_run_profile.clone(),
                ..crate::RunProfileResolutionRequest::interactive_default()
            })
            .await;
        let pre_resolved = PreResolvedRunProfileResolver::new(profile_resolution);
        self.apply(RunnerLeaseOverlay::None, |store| {
            let request = request.clone();
            let pre_resolved = pre_resolved.clone();
            async move {
                let outcome = store
                    .submit_child_turn(request, admission_policy, &pre_resolved)
                    .await;
                outcome
            }
        })
        .instrument(turn_state_write_span(
            "submit_child_turn",
            Some(&request.child_scope),
            request.requested_run_id.as_ref(),
        ))
        .await
    }

    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        let (snapshot, _) = self.read_snapshot().await?;
        Ok(projection::children_of(&snapshot, scope, run_id))
    }

    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        let (snapshot, _) = self
            .read_snapshot_with_runner_lease_overlay(RunnerLeaseOverlay::Run(run_id))
            .await?;
        Ok(projection::run_record(&snapshot, scope, run_id))
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| async move {
            let outcome = store
                .reserve_tree_descendants(scope, root_run_id, delta, cap)
                .await;
            outcome
        })
        .instrument(turn_state_write_span(
            "reserve_tree_descendants",
            Some(scope),
            Some(&root_run_id),
        ))
        .await
    }

    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
    ) -> Result<(), TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| async move {
            let outcome = store
                .release_tree_descendants(scope, root_run_id, delta)
                .await;
            outcome
        })
        .instrument(turn_state_write_span(
            "release_tree_descendants",
            Some(scope),
            Some(&root_run_id),
        ))
        .await
    }
}

#[async_trait]
impl<F> TurnEventProjectionSource for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn read_turn_events_after(
        &self,
        scope: &TurnScope,
        owner_user_id: Option<&UserId>,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<TurnEventPage, TurnError> {
        let (snapshot, _) = self.read_snapshot().await?;
        Ok(project_turn_events(
            &snapshot.events,
            scope,
            owner_user_id,
            after,
            limit,
            snapshot.event_retention_floor,
        ))
    }
}

#[async_trait]
impl<F> LoopCheckpointStore for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        let span = turn_state_write_span(
            "put_loop_checkpoint",
            Some(&request.scope),
            Some(&request.run_id),
        );
        async move {
            let record = loop_checkpoint_record_from_request(request);
            let delta = SnapshotDelta {
                loop_checkpoints_upsert: vec![record.clone()],
                ..SnapshotDelta::default()
            };
            self.persist_delta(&delta)
                .await
                .map_err(|error| match error {
                    RowPersistError::Turn(error) => error,
                })?;
            self.apply_cached_delta(delta).await?;
            Ok(record)
        }
        .instrument(span)
        .await
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        self.with_cached_snapshot(|snapshot| projection::loop_checkpoint(snapshot, &request))
            .await
    }
}

#[async_trait]
impl<F> TurnRunTransitionPort for FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem,
{
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let span = turn_state_write_span("claim_next_run", request.scope_filter.as_ref(), None);
        async move {
            let claimed = self
                .apply_with_targeted_delta(
                    RunnerLeaseOverlay::None,
                    |store| {
                        let request = request.clone();
                        async move { store.claim_next_run(request).await }
                    },
                    claimed_run_targeted_delta,
                )
                .await?;
            if let Some(claimed) = &claimed
                && let Err(error) = self
                    .seed_runner_lease_from_cached_run(claimed.state.run_id)
                    .await
            {
                self.compensate_failed_claim(claimed).await;
                return Err(error);
            }
            Ok(claimed)
        }
        .instrument(span)
        .await
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        self.heartbeat_runner_lease(request).await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        let result = self
            .apply(RunnerLeaseOverlay::All, |store| {
                let request = request.clone();
                async move {
                    let outcome = store.recover_expired_leases(request).await;
                    outcome
                }
            })
            .instrument(turn_state_write_span(
                "recover_expired_leases",
                request.scope_filter.as_ref(),
                None,
            ))
            .await;
        if let Ok(response) = &result {
            for state in &response.recovered {
                self.runner_lease_store()
                    .delete_best_effort(state.run_id)
                    .await;
            }
        }
        result
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply(RunnerLeaseOverlay::Run(request.run_id), |store| {
            let request = request.clone();
            async move {
                let outcome = store.record_model_route_snapshot(request).await;
                outcome
            }
        })
        .instrument(turn_state_write_span(
            "record_model_route_snapshot",
            None,
            Some(&request.run_id),
        ))
        .await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition_with_targeted_delta(
            "block_run",
            request.run_id,
            request.runner_id,
            request.lease_token,
            request.reason.status(),
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.block_run(request).await;
                    outcome
                }
            },
            blocked_run_targeted_delta,
        )
        .await
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        let span = turn_state_write_span("complete_run", None, Some(&request.run_id));
        async move {
            let previous = self
                .prepare_runner_lease_retirement(
                    request.run_id,
                    request.runner_id,
                    request.lease_token,
                    TurnStatus::Completed,
                )
                .await?;
            let max_terminal_records = self.limits.max_terminal_records;
            let result = self
                .apply_with_targeted_delta(
                    RunnerLeaseOverlay::Run(request.run_id),
                    |store| {
                        let request = request.clone();
                        async move { store.complete_run(request).await }
                    },
                    move |snapshot, latest_event_cursor, store, state| {
                        let terminal_records = snapshot
                            .runs
                            .iter()
                            .filter(|record| record.status.is_terminal())
                            .count();
                        if terminal_records >= max_terminal_records {
                            return full_snapshot_delta(snapshot, store);
                        }
                        run_state_targeted_delta(
                            snapshot,
                            latest_event_cursor,
                            store,
                            state.run_id,
                            &state.scope,
                        )
                    },
                )
                .await;
            if result.is_err() {
                self.restore_runner_lease_after_failed_transition(previous, TurnStatus::Completed)
                    .await;
            }
            self.cleanup_runner_lease_after_state(&result).await;
            result
        }
        .instrument(span)
        .await
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        let max_terminal_records = self.limits.max_terminal_records;
        self.apply_run_state_transition_with_targeted_delta(
            "cancel_run",
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Cancelled,
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.cancel_run(request).await;
                    outcome
                }
            },
            move |snapshot, latest_event_cursor, store, state| {
                let terminal_records = snapshot
                    .runs
                    .iter()
                    .filter(|record| record.status.is_terminal())
                    .count();
                if terminal_records >= max_terminal_records {
                    return full_snapshot_delta(snapshot, store);
                }
                run_state_targeted_delta(
                    snapshot,
                    latest_event_cursor,
                    store,
                    state.run_id,
                    &state.scope,
                )
            },
        )
        .await
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            "fail_run",
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Failed,
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.fail_run(request).await;
                    outcome
                }
            },
        )
        .await
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            "record_runner_failure",
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Failed,
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.record_runner_failure(request).await;
                    outcome
                }
            },
        )
        .await
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            "relinquish_run",
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Queued,
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.relinquish_run(request).await;
                    outcome
                }
            },
        )
        .await
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            "apply_validated_loop_exit",
            request.run_id,
            request.runner_id,
            request.lease_token,
            retired_status_for_loop_exit(&request.mapping),
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.apply_validated_loop_exit(request).await;
                    outcome
                }
            },
        )
        .await
    }
}

#[derive(Serialize)]
struct SpawnTreeReservationKeyForPath<'a> {
    scope: &'a TurnScope,
    root_run_id: TurnRunId,
}

enum RowPersistError {
    Turn(TurnError),
}

impl From<TurnError> for RowPersistError {
    fn from(error: TurnError) -> Self {
        Self::Turn(error)
    }
}

fn snapshot_delta(
    old: &TurnPersistenceSnapshot,
    new: &TurnPersistenceSnapshot,
) -> Result<SnapshotDelta, RowPersistError> {
    let (turns_upsert, turns_delete) = delta_collection(&old.turns, &new.turns, |record| {
        Ok(record.turn_id.to_string())
    })?;
    let (runs_upsert, runs_delete) =
        delta_collection(&old.runs, &new.runs, |record| Ok(record.run_id.to_string()))?;
    let (active_locks_upsert, active_locks_delete) =
        delta_collection(&old.active_locks, &new.active_locks, |record| {
            hash_key(&record.key)
        })?;
    let (checkpoints_upsert, checkpoints_delete) =
        delta_collection(&old.checkpoints, &new.checkpoints, |record| {
            Ok(record.checkpoint_id.as_uuid().to_string())
        })?;
    let (loop_checkpoints_upsert, loop_checkpoints_delete) =
        delta_collection(&old.loop_checkpoints, &new.loop_checkpoints, |record| {
            Ok(record.checkpoint_id.as_uuid().to_string())
        })?;
    let (idempotency_upsert, idempotency_delete) =
        delta_collection(&old.idempotency_records, &new.idempotency_records, hash_key)?;
    let (events_upsert, events_delete) = delta_collection(&old.events, &new.events, |record| {
        Ok(format!("{:020}", record.cursor.0))
    })?;
    let (admission_reservations_upsert, admission_reservations_delete) = delta_collection(
        &old.admission_reservations,
        &new.admission_reservations,
        |record| Ok(record.run_id.to_string()),
    )?;
    let (spawn_tree_reservations_upsert, spawn_tree_reservations_delete) = delta_collection(
        &old.spawn_tree_reservations,
        &new.spawn_tree_reservations,
        |record| {
            hash_key(&SpawnTreeReservationKeyForPath {
                scope: &record.scope,
                root_run_id: record.root_run_id,
            })
        },
    )?;

    Ok(SnapshotDelta {
        turns_upsert,
        turns_delete,
        runs_upsert,
        runs_delete,
        active_locks_upsert,
        active_locks_delete,
        checkpoints_upsert,
        checkpoints_delete,
        loop_checkpoints_upsert,
        loop_checkpoints_delete,
        idempotency_upsert,
        idempotency_delete,
        events_upsert,
        events_delete,
        admission_reservations_upsert,
        admission_reservations_delete,
        spawn_tree_reservations_upsert,
        spawn_tree_reservations_delete,
        event_retention_floor: (old.event_retention_floor != new.event_retention_floor)
            .then_some(new.event_retention_floor),
    })
}

fn apply_delta(
    snapshot: &mut TurnPersistenceSnapshot,
    delta: SnapshotDelta,
) -> Result<(), TurnError> {
    if !delta.turns_upsert.is_empty() || !delta.turns_delete.is_empty() {
        apply_delta_collection(
            &mut snapshot.turns,
            delta.turns_upsert,
            delta.turns_delete,
            |record| Ok(record.turn_id.to_string()),
        )?;
    }
    if !delta.runs_upsert.is_empty() || !delta.runs_delete.is_empty() {
        apply_delta_collection(
            &mut snapshot.runs,
            delta.runs_upsert,
            delta.runs_delete,
            |record| Ok(record.run_id.to_string()),
        )?;
    }
    if !delta.active_locks_upsert.is_empty() || !delta.active_locks_delete.is_empty() {
        apply_delta_collection(
            &mut snapshot.active_locks,
            delta.active_locks_upsert,
            delta.active_locks_delete,
            |record| hash_key(&record.key),
        )?;
    }
    if !delta.checkpoints_upsert.is_empty() || !delta.checkpoints_delete.is_empty() {
        apply_delta_collection(
            &mut snapshot.checkpoints,
            delta.checkpoints_upsert,
            delta.checkpoints_delete,
            |record| Ok(record.checkpoint_id.as_uuid().to_string()),
        )?;
    }
    if !delta.loop_checkpoints_upsert.is_empty() || !delta.loop_checkpoints_delete.is_empty() {
        apply_delta_collection(
            &mut snapshot.loop_checkpoints,
            delta.loop_checkpoints_upsert,
            delta.loop_checkpoints_delete,
            |record| Ok(record.checkpoint_id.as_uuid().to_string()),
        )?;
    }
    if !delta.idempotency_upsert.is_empty() || !delta.idempotency_delete.is_empty() {
        apply_delta_collection(
            &mut snapshot.idempotency_records,
            delta.idempotency_upsert,
            delta.idempotency_delete,
            hash_key,
        )?;
    }
    if !delta.events_upsert.is_empty() || !delta.events_delete.is_empty() {
        apply_delta_collection(
            &mut snapshot.events,
            delta.events_upsert,
            delta.events_delete,
            |record| Ok(format!("{:020}", record.cursor.0)),
        )?;
    }
    if !delta.admission_reservations_upsert.is_empty()
        || !delta.admission_reservations_delete.is_empty()
    {
        apply_delta_collection(
            &mut snapshot.admission_reservations,
            delta.admission_reservations_upsert,
            delta.admission_reservations_delete,
            |record| Ok(record.run_id.to_string()),
        )?;
    }
    if !delta.spawn_tree_reservations_upsert.is_empty()
        || !delta.spawn_tree_reservations_delete.is_empty()
    {
        apply_delta_collection(
            &mut snapshot.spawn_tree_reservations,
            delta.spawn_tree_reservations_upsert,
            delta.spawn_tree_reservations_delete,
            |record| {
                hash_key(&SpawnTreeReservationKeyForPath {
                    scope: &record.scope,
                    root_run_id: record.root_run_id,
                })
            },
        )?;
    }
    if let Some(event_retention_floor) = delta.event_retention_floor {
        snapshot.event_retention_floor = event_retention_floor;
    }
    Ok(())
}

fn delta_collection<T, K>(
    old: &[T],
    new: &[T],
    key_fn: K,
) -> Result<(Vec<T>, Vec<String>), RowPersistError>
where
    T: Clone + PartialEq,
    K: Fn(&T) -> Result<String, TurnError>,
{
    let old_map = keyed_records(old, &key_fn)?;
    let new_map = keyed_records(new, &key_fn)?;
    let upsert = new_map
        .iter()
        .filter(|(key, record)| old_map.get(*key) != Some(*record))
        .map(|(_key, record)| record.clone())
        .collect();
    let new_keys = new_map.keys().cloned().collect::<HashSet<_>>();
    let delete = old_map
        .keys()
        .filter(|key| !new_keys.contains(*key))
        .cloned()
        .collect();
    Ok((upsert, delete))
}

fn apply_delta_collection<T, K>(
    records: &mut Vec<T>,
    upsert: Vec<T>,
    delete: Vec<String>,
    key_fn: K,
) -> Result<(), TurnError>
where
    K: Fn(&T) -> Result<String, TurnError>,
{
    if !delete.is_empty() {
        let deleted = delete.into_iter().collect::<HashSet<_>>();
        let mut retained = Vec::with_capacity(records.len());
        for record in records.drain(..) {
            if !deleted.contains(&key_fn(&record)?) {
                retained.push(record);
            }
        }
        *records = retained;
    }

    for record in upsert {
        let key = key_fn(&record)?;
        if let Some(index) = record_index(records, &key, &key_fn)? {
            records[index] = record;
        } else {
            records.push(record);
        }
    }
    Ok(())
}

fn record_index<T, K>(records: &[T], key: &str, key_fn: &K) -> Result<Option<usize>, TurnError>
where
    K: Fn(&T) -> Result<String, TurnError>,
{
    for (index, record) in records.iter().enumerate() {
        if key_fn(record)? == key {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn keyed_records<T, K>(records: &[T], key_fn: &K) -> Result<HashMap<String, T>, RowPersistError>
where
    T: Clone,
    K: Fn(&T) -> Result<String, TurnError>,
{
    records
        .iter()
        .map(|record| Ok((key_fn(record)?, record.clone())))
        .collect()
}

fn submit_turn_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    response: &SubmitTurnResponse,
) -> Result<SnapshotDelta, TurnError> {
    let SubmitTurnResponse::Accepted {
        turn_id, run_id, ..
    } = response;
    let turn = store
        .turn_record(*turn_id)
        .ok_or_else(|| TurnError::Unavailable {
            reason: "accepted turn missing from row-store hot state".to_string(),
        })?;
    let run = store
        .run_record(*run_id)
        .ok_or_else(|| TurnError::Unavailable {
            reason: "accepted run missing from row-store hot state".to_string(),
        })?;
    let mut delta = SnapshotDelta {
        turns_upsert: vec![turn.clone()],
        runs_upsert: vec![run],
        ..SnapshotDelta::default()
    };
    if let Some(lock) = store.active_lock_record(&turn.scope) {
        delta.active_locks_upsert.push(lock);
    }
    if let Some(reservation) = store.admission_reservation(*run_id) {
        delta.admission_reservations_upsert.push(reservation);
    }
    delta.idempotency_upsert.extend(
        store
            .idempotency_records_after(turn.created_at)
            .into_iter()
            .filter(|record| {
                record.operation == crate::TurnIdempotencyOperationKind::Submit
                    && record.run_id == Some(*run_id)
            }),
    );
    add_event_delta(snapshot, latest_event_cursor, store, &mut delta)?;
    Ok(delta)
}

fn full_snapshot_delta(
    snapshot: &TurnPersistenceSnapshot,
    store: &InMemoryTurnStateStore,
) -> Result<SnapshotDelta, TurnError> {
    let mut new_snapshot = store.persistence_snapshot();
    preserve_loop_checkpoints(snapshot, &mut new_snapshot);
    snapshot_delta(snapshot, &new_snapshot).map_err(|error| match error {
        RowPersistError::Turn(error) => error,
    })
}

fn preserve_loop_checkpoints(
    baseline: &TurnPersistenceSnapshot,
    new_snapshot: &mut TurnPersistenceSnapshot,
) {
    new_snapshot.loop_checkpoints = baseline.loop_checkpoints.clone();
}

fn loop_checkpoint_record_from_request(request: PutLoopCheckpointRequest) -> LoopCheckpointRecord {
    LoopCheckpointRecord {
        checkpoint_id: TurnCheckpointId::new(),
        scope: request.scope,
        turn_id: request.turn_id,
        run_id: request.run_id,
        state_ref: request.state_ref,
        schema_id: request.schema_id,
        schema_version: request.schema_version,
        kind: request.kind,
        gate_ref: request.gate_ref,
        created_at: Utc::now(),
    }
}

fn claimed_run_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    claimed: &Option<ClaimedTurnRun>,
) -> Result<SnapshotDelta, TurnError> {
    let Some(claimed) = claimed else {
        return Ok(SnapshotDelta::default());
    };
    run_state_targeted_delta(
        snapshot,
        latest_event_cursor,
        store,
        claimed.state.run_id,
        &claimed.state.scope,
    )
}

fn run_state_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    run_id: TurnRunId,
    scope: &TurnScope,
) -> Result<SnapshotDelta, TurnError> {
    let mut delta = SnapshotDelta::default();
    match store.run_record(run_id) {
        Some(run) => {
            delta.runs_upsert.push(run);
        }
        None => {
            delta.runs_delete.push(run_id.to_string());
            if let Some(old_run) = snapshot.runs.iter().find(|record| record.run_id == run_id)
                && store.turn_record(old_run.turn_id).is_none()
            {
                delta.turns_delete.push(old_run.turn_id.to_string());
            }
        }
    }

    if let Some(lock) = store.active_lock_record(scope) {
        delta.active_locks_upsert.push(lock);
    } else if let Some(old_lock) = snapshot
        .active_locks
        .iter()
        .find(|record| record.key.scope == *scope)
    {
        delta.active_locks_delete.push(hash_key(&old_lock.key)?);
    }

    if let Some(reservation) = store.admission_reservation(run_id) {
        delta.admission_reservations_upsert.push(reservation);
    } else if snapshot
        .admission_reservations
        .iter()
        .any(|record| record.run_id == run_id)
    {
        delta.admission_reservations_delete.push(run_id.to_string());
    }

    add_event_delta(snapshot, latest_event_cursor, store, &mut delta)?;
    Ok(delta)
}

fn run_state_with_idempotency_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    run_id: TurnRunId,
    scope: &TurnScope,
    operation: crate::TurnIdempotencyOperationKind,
) -> Result<SnapshotDelta, TurnError> {
    let mut delta = run_state_targeted_delta(snapshot, latest_event_cursor, store, run_id, scope)?;
    add_run_idempotency_delta(snapshot, store, &mut delta, run_id, operation);
    Ok(delta)
}

fn blocked_run_targeted_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    state: &TurnRunState,
) -> Result<SnapshotDelta, TurnError> {
    let mut delta = run_state_targeted_delta(
        snapshot,
        latest_event_cursor,
        store,
        state.run_id,
        &state.scope,
    )?;
    if let Some(checkpoint_id) = state.checkpoint_id {
        let checkpoint =
            store
                .checkpoint_record(checkpoint_id)
                .ok_or_else(|| TurnError::Unavailable {
                    reason: "blocked run checkpoint missing from row-store hot state".to_string(),
                })?;
        delta.checkpoints_upsert.push(checkpoint);
    }
    Ok(delta)
}

fn add_run_idempotency_delta(
    snapshot: &TurnPersistenceSnapshot,
    store: &InMemoryTurnStateStore,
    delta: &mut SnapshotDelta,
    run_id: TurnRunId,
    operation: crate::TurnIdempotencyOperationKind,
) {
    delta.idempotency_upsert.extend(
        store
            .idempotency_records_for_run_operation(run_id, operation)
            .into_iter()
            .filter(|record| !snapshot.idempotency_records.contains(record)),
    );
}

fn latest_event_cursor(snapshot: &TurnPersistenceSnapshot) -> EventCursor {
    snapshot
        .events
        .iter()
        .map(|event| event.cursor)
        .max()
        .unwrap_or(snapshot.event_retention_floor)
        .max(snapshot.event_retention_floor)
}

fn latest_event_cursor_after_delta(current: EventCursor, delta: &SnapshotDelta) -> EventCursor {
    let event_cursor = delta
        .events_upsert
        .iter()
        .map(|event| event.cursor)
        .max()
        .unwrap_or(current);
    let retention_floor = delta.event_retention_floor.unwrap_or(current);
    current.max(event_cursor).max(retention_floor)
}

fn add_event_delta(
    snapshot: &TurnPersistenceSnapshot,
    latest_event_cursor: EventCursor,
    store: &InMemoryTurnStateStore,
    delta: &mut SnapshotDelta,
) -> Result<(), TurnError> {
    delta
        .events_upsert
        .extend(store.events_after(latest_event_cursor));
    let event_retention_floor = store.event_retention_floor();
    if event_retention_floor != snapshot.event_retention_floor {
        delta.event_retention_floor = Some(event_retention_floor);
        for event in snapshot
            .events
            .iter()
            .filter(|event| event.cursor <= event_retention_floor)
        {
            delta.events_delete.push(format!("{:020}", event.cursor.0));
        }
    }
    Ok(())
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

fn row_dir(collection: &str) -> Result<ScopedPath, TurnError> {
    scoped_row_path(format!("{ROW_ROOT}/{collection}"))
}

fn row_path(collection: &str, key: &str) -> Result<ScopedPath, TurnError> {
    scoped_row_path(format!("{ROW_ROOT}/{collection}/{key}.json"))
}

fn meta_path() -> Result<ScopedPath, TurnError> {
    scoped_row_path(format!("{ROW_ROOT}/{META_DIR}/{META_FILE}"))
}

fn delta_log_path() -> Result<ScopedPath, TurnError> {
    scoped_row_path(format!("{ROW_ROOT}/{DELTA_LOG}"))
}

fn scoped_row_path(path: String) -> Result<ScopedPath, TurnError> {
    ScopedPath::new(path).map_err(|error| TurnError::Unavailable {
        reason: format!("invalid turn-state row path: {error}"),
    })
}

fn deserialize_row<T>(bytes: &[u8], collection: &'static str) -> Result<T, TurnError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state {collection} row deserialization failed: {error}"),
    })
}

fn hash_key<T>(record: &T) -> Result<String, TurnError>
where
    T: Serialize,
{
    let bytes = serde_jcs::to_vec(record).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state row key serialization failed: {error}"),
    })?;
    Ok(hex::encode(blake3::hash(&bytes).as_bytes()))
}

fn fs_error(error: FilesystemError) -> TurnError {
    tracing::debug!(%error, "turn state row-store filesystem operation failed");
    TurnError::Unavailable {
        reason: "turn state row-store persistence temporarily unavailable".to_string(),
    }
}

fn retired_status_for_loop_exit(mapping: &crate::LoopExitMapping) -> TurnStatus {
    match mapping {
        crate::LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed) => {
            TurnStatus::Completed
        }
        crate::LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled) => {
            TurnStatus::Cancelled
        }
        crate::LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Blocked { reason, .. }) => {
            reason.status()
        }
        crate::LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed { .. })
        | crate::LoopExitMapping::RecoveryRequired { .. } => TurnStatus::Failed,
    }
}
