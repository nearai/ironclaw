//! Filesystem-backed [`TurnStateStore`] implementation.
//!
//! Persists the lower-churn [`TurnPersistenceSnapshot`] as a JSON blob under
//! the `/turns` mount alias (alias-relative path: `/turns/state.json`).
//! High-churn runner lease heartbeats are memory-backed per
//! [`FilesystemTurnStateStore`] instance and are overlaid onto the durable
//! snapshot while the process is alive. Snapshot mutations read the snapshot,
//! overlay current runner leases, delegate to an [`InMemoryTurnStateStore`] in
//! a transient `apply` closure, and write the resulting snapshot back with
//! optimistic CAS + bounded retry. Reads load the snapshot, overlay current
//! runner leases, and project through the in-memory store without writing
//! back.
//!
//! This mirrors the load-snapshot / replace-snapshot pattern the legacy
//! [`LibSqlTurnStateStore`] / [`PostgresTurnStateStore`] used internally —
//! their migration is in
//! `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
//!
//! Tenant/user isolation is structural: the [`MountView`] the composition
//! layer hands the [`ScopedFilesystem`] resolves `/turns/state.json` to a
//! tenant/user-scoped [`VirtualPath`](ironclaw_host_api::VirtualPath) before
//! any backend dispatch. The on-disk layout under the alias is fixed:
//!
//! ```text
//! /turns/state.json
//! ```
//!
//! Within-tenant scoping (agent/project/thread) is encoded inside the
//! snapshot body via `TurnScope` on every persisted record; no extra path
//! segments are needed because the snapshot lives at the tenant/user level.
//! Tenant + user identity moves into the caller's `MountView` per the
//! per-tenant `MountAlias` rewriting, so neither prefix is encoded in the
//! path itself.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use ironclaw_filesystem::{CasExpectation, RecordVersion, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{ResourceScope, ScopedPath, UserId};
use tokio::sync::RwLock;

use crate::{
    AllowAllTurnAdmissionLimitProvider, CancelRunRequest, CancelRunResponse, EventCursor,
    GetLoopCheckpointRequest, GetRunStateRequest, InMemoryTurnStateStore,
    InMemoryTurnStateStoreLimits, LoopCheckpointRecord, LoopCheckpointStore,
    PutLoopCheckpointRequest, ResumeTurnRequest, ResumeTurnResponse, RunProfileResolver,
    SpawnTreeReservation, SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse,
    TurnAdmissionLimitProvider, TurnAdmissionPolicy, TurnError, TurnEventPage,
    TurnEventProjectionSource, TurnPersistenceSnapshot, TurnRunId, TurnRunRecord, TurnRunState,
    TurnScope, TurnSpawnTreeStateStore, TurnStateStore, TurnStatus,
    events::project_turn_events,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest, RecoverExpiredLeasesRequest,
        RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
        TurnRunnerOutcome,
    },
};

mod io;
mod profile_resolver;
mod projection;
mod runner_lease;

use io::{
    FILESYSTEM_CAS_RETRIES, PutError, cas_retry_backoff, deserialize_snapshot, fs_error,
    put_with_cas, snapshot_entry, snapshot_path,
};
use profile_resolver::PreResolvedRunProfileResolver;
use runner_lease::{RunnerLeaseMemory, RunnerLeaseOverlay, RunnerLeaseRecord, RunnerLeaseStore};

#[cfg(test)]
mod tests;

const FILESYSTEM_APPLY_TIMEOUT: Duration = Duration::from_secs(15);
const SNAPSHOT_READ_CACHE_TTL: Duration = Duration::from_millis(500);

#[derive(Clone)]
struct CachedSnapshot {
    snapshot: TurnPersistenceSnapshot,
    version: Option<RecordVersion>,
    loaded_at: Instant,
}

impl CachedSnapshot {
    fn new(snapshot: TurnPersistenceSnapshot, version: Option<RecordVersion>) -> Self {
        Self {
            snapshot,
            version,
            loaded_at: Instant::now(),
        }
    }

    fn is_fresh(&self) -> bool {
        self.loaded_at.elapsed() <= SNAPSHOT_READ_CACHE_TTL
    }

    fn parts(&self) -> (TurnPersistenceSnapshot, Option<RecordVersion>) {
        (self.snapshot.clone(), self.version)
    }
}

/// Filesystem-backed turn-state store under the `/turns` mount alias.
///
/// Construct with a [`ScopedFilesystem`] over a [`RootFilesystem`]. The
/// [`ScopedFilesystem`] resolves the `/turns` alias to a tenant/user-scoped
/// [`VirtualPath`](ironclaw_host_api::VirtualPath) per its
/// [`MountView`](ironclaw_host_api::MountView) and enforces per-op ACL before
/// any backend dispatch — so tenant isolation is structural rather than
/// something this crate has to re-derive from `TurnScope.tenant_id`.
/// Within-tenant axes (agent/project/thread) stay in the persisted snapshot
/// records because they are not covered by the per-tenant `MountAlias`. The
/// backend must honor `Absent` / `Version` CAS for writes; unsupported CAS
/// fails closed in the canonical write path instead of falling back to blind
/// overwrites.
pub struct FilesystemTurnStateStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    limits: InMemoryTurnStateStoreLimits,
    admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
    snapshot_cache: Mutex<Option<CachedSnapshot>>,
    runner_leases: RunnerLeaseMemory,
    apply_timeout: Duration,
}

impl<F> FilesystemTurnStateStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            limits: InMemoryTurnStateStoreLimits::default(),
            admission_limit_provider: Arc::new(AllowAllTurnAdmissionLimitProvider),
            snapshot_cache: Mutex::new(None),
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

    pub fn with_apply_timeout(mut self, apply_timeout: Duration) -> Self {
        self.apply_timeout = apply_timeout;
        self
    }

    /// Read the persistence snapshot from `/turns/state.json`. Returns an
    /// empty snapshot if the blob is missing — `start` semantics for a fresh
    /// tenant/user mount.
    pub async fn persistence_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let (snapshot, _) = self
            .read_snapshot_with_runner_lease_overlay(RunnerLeaseOverlay::All)
            .await?;
        Ok(snapshot)
    }

    async fn read_snapshot(
        &self,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        if let Some(snapshot) = self.fresh_cached_snapshot() {
            return Ok(snapshot);
        }
        // Pure reads are lock-free. CAS-capable backends expose only committed
        // snapshot versions, so a reader racing a write observes either the
        // previous committed snapshot or the next one. Taking a process-local
        // writer lock here would force `get_run_state`, host construction,
        // cancellation polling, claims, heartbeats, and terminal transitions
        // behind one in-flight write on the single per-user snapshot.
        let snapshot = self.read_snapshot_from_filesystem().await?;
        self.store_snapshot_cache(snapshot.clone());
        Ok(snapshot)
    }

    async fn read_snapshot_from_filesystem(
        &self,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        let path = snapshot_path()?;
        // Turn persistence is a single alias-relative snapshot for this
        // scoped filesystem. Tenant/user isolation comes from the mount view
        // that resolves `/turns/state.json` to the backend virtual path; the
        // snapshot body then scopes records by agent/project/thread.
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => {
                let snapshot = deserialize_snapshot(&versioned.entry.body)?;
                Ok((snapshot, Some(versioned.version)))
            }
            Ok(None) => Ok((TurnPersistenceSnapshot::default(), None)),
            Err(error) => Err(fs_error(error)),
        }
    }

    async fn read_snapshot_with_runner_lease_overlay(
        &self,
        overlay: RunnerLeaseOverlay,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        let snapshot = self.read_snapshot().await?;
        self.overlay_runner_leases(snapshot, overlay).await
    }

    async fn read_snapshot_from_filesystem_with_runner_lease_overlay(
        &self,
        overlay: RunnerLeaseOverlay,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        let snapshot = self.read_snapshot_from_filesystem().await?;
        self.overlay_runner_leases(snapshot, overlay).await
    }

    async fn overlay_runner_leases(
        &self,
        snapshot: (TurnPersistenceSnapshot, Option<RecordVersion>),
        overlay: RunnerLeaseOverlay,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        self.runner_lease_store().overlay(snapshot, overlay).await
    }

    async fn seed_runner_lease_from_snapshot_inner(
        &self,
        run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        let (snapshot, _version) = self.read_snapshot_from_filesystem().await?;
        self.runner_lease_store()
            .seed_from_snapshot(&snapshot, run_id)
            .await?;
        self.clear_snapshot_cache();
        Ok(())
    }

    async fn cleanup_runner_lease_after_state(&self, result: &Result<TurnRunState, TurnError>) {
        self.runner_lease_store().cleanup_after_state(result).await;
        self.clear_snapshot_cache();
    }

    async fn heartbeat_runner_lease(
        &self,
        request: HeartbeatRequest,
    ) -> Result<EventCursor, TurnError> {
        let lease_store = self.runner_lease_store();
        let cursor = match lease_store.heartbeat(request.clone()).await {
            Err(TurnError::ScopeNotFound) => {
                self.seed_missing_runner_lease_from_snapshot(request.run_id)
                    .await?;
                self.runner_lease_store().heartbeat(request).await?
            }
            result => result?,
        };
        self.clear_snapshot_cache();
        Ok(cursor)
    }

    async fn seed_missing_runner_lease_from_snapshot(
        &self,
        run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        let (snapshot, _version) = self.read_snapshot_from_filesystem().await?;
        self.runner_lease_store()
            .seed_from_snapshot_if_missing(&snapshot, run_id)
            .await
    }

    async fn prepare_cancel_requested_runner_lease(
        &self,
        request: &CancelRunRequest,
    ) -> Result<Option<RunnerLeaseRecord>, TurnError> {
        let (snapshot, _version) = self.read_snapshot_from_filesystem().await?;
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
        let (snapshot, _version) = self.read_snapshot_from_filesystem().await?;
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
        self.clear_snapshot_cache();
    }

    fn runner_lease_store(&self) -> RunnerLeaseStore {
        RunnerLeaseStore::new(
            Arc::clone(&self.runner_leases),
            self.limits.runner_lease_ttl,
            self.apply_timeout,
        )
    }

    fn fresh_cached_snapshot(&self) -> Option<(TurnPersistenceSnapshot, Option<RecordVersion>)> {
        match self.snapshot_cache.lock() {
            Ok(guard) => guard
                .as_ref()
                .filter(|snapshot| snapshot.is_fresh())
                .map(CachedSnapshot::parts),
            Err(poisoned) => poisoned
                .into_inner()
                .as_ref()
                .filter(|snapshot| snapshot.is_fresh())
                .map(CachedSnapshot::parts),
        }
    }

    fn store_snapshot_cache(&self, snapshot: (TurnPersistenceSnapshot, Option<RecordVersion>)) {
        let cached = CachedSnapshot::new(snapshot.0, snapshot.1);
        match self.snapshot_cache.lock() {
            Ok(mut guard) => *guard = Some(cached),
            Err(poisoned) => *poisoned.into_inner() = Some(cached),
        }
    }

    fn clear_snapshot_cache(&self) {
        match self.snapshot_cache.lock() {
            Ok(mut guard) => *guard = None,
            Err(poisoned) => *poisoned.into_inner() = None,
        }
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

    /// Read-modify-write the snapshot with optimistic CAS and bounded retry.
    ///
    /// `apply` materializes a transient [`InMemoryTurnStateStore`] from the
    /// loaded snapshot, runs the supplied async closure against it, and the
    /// resulting snapshot is written back. On `VersionMismatch` the loop
    /// re-reads and reapplies the closure against the latest snapshot. The
    /// guarded read/modify/write is deadline-bounded so one wedged filesystem
    /// operation only consumes this caller's apply attempt until the deadline
    /// returns `TurnError::Unavailable`.
    async fn apply<T, A, Fut>(
        &self,
        overlay: RunnerLeaseOverlay,
        mut apply: A,
    ) -> Result<T, TurnError>
    where
        A: FnMut(InMemoryTurnStateStore) -> Fut,
        Fut: std::future::Future<Output = (Result<T, TurnError>, InMemoryTurnStateStore)>,
    {
        let path = snapshot_path()?;
        match tokio::time::timeout(
            self.apply_timeout,
            self.apply_with_retry(&path, overlay, &mut apply),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                self.clear_snapshot_cache();
                Err(TurnError::Unavailable {
                    reason: "turn state filesystem apply timed out".to_string(),
                })
            }
        }
    }

    async fn apply_with_retry<T, A, Fut>(
        &self,
        path: &ScopedPath,
        overlay: RunnerLeaseOverlay,
        apply: &mut A,
    ) -> Result<T, TurnError>
    where
        A: FnMut(InMemoryTurnStateStore) -> Fut,
        Fut: std::future::Future<Output = (Result<T, TurnError>, InMemoryTurnStateStore)>,
    {
        for attempt in 0..FILESYSTEM_CAS_RETRIES {
            let (snapshot, version) = self
                .read_snapshot_from_filesystem_with_runner_lease_overlay(overlay)
                .await?;
            let old_snapshot = snapshot.clone();
            let store = self.build_in_memory_store(snapshot)?;
            let (outcome, store) = apply(store).await;
            let new_snapshot = store.persistence_snapshot();

            if new_snapshot == old_snapshot {
                // This apply path read the latest snapshot directly from the
                // backend, so any previously cached snapshot may now be stale.
                self.clear_snapshot_cache();
                return outcome;
            }
            let entry = snapshot_entry(&new_snapshot)?;
            let cas = match version {
                Some(version) => CasExpectation::Version(version),
                None => CasExpectation::Absent,
            };
            match put_with_cas(self.filesystem.as_ref(), path, entry, cas).await {
                Ok(version) => {
                    self.store_snapshot_cache((new_snapshot, Some(version)));
                    return outcome;
                }
                Err(PutError::VersionMismatch) => {
                    self.clear_snapshot_cache();
                    cas_retry_backoff(attempt).await;
                }
                Err(PutError::Other(error)) => return Err(error),
            }
        }
        Err(TurnError::Unavailable {
            reason: "turn state filesystem CAS retries exhausted".to_string(),
        })
    }

    async fn apply_run_state_transition<A, Fut>(
        &self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        retired_status: TurnStatus,
        apply: A,
    ) -> Result<TurnRunState, TurnError>
    where
        A: FnMut(InMemoryTurnStateStore) -> Fut,
        Fut:
            std::future::Future<Output = (Result<TurnRunState, TurnError>, InMemoryTurnStateStore)>,
    {
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
                (outcome.map(|_| ()), store)
            })
            .await;
        if let Err(error) = result {
            tracing::debug!(
                run_id = %run_id,
                error = %error,
                "failed to compensate turn claim after memory runner lease seed failed"
            );
        }
        self.clear_snapshot_cache();
    }
}

#[async_trait]
impl<F> TurnStateStore for FilesystemTurnStateStore<F>
where
    F: RootFilesystem,
{
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        // Run the resolver outside the apply closure once so we don't hold
        // the per-path async lock across the resolver future. The in-memory
        // store delegates to a pre-resolved resolver inside the CAS loop.
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
                    .submit_turn(request, admission_policy, &pre_resolved)
                    .await;
                (outcome, store)
            }
        })
        .await
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| {
            let request = request.clone();
            async move {
                let outcome = store.resume_turn(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        let previous = self.prepare_cancel_requested_runner_lease(&request).await?;
        let result = self
            .apply(RunnerLeaseOverlay::Run(request.run_id), |store| {
                let request = request.clone();
                async move {
                    let outcome = store.request_cancel(request).await;
                    (outcome, store)
                }
            })
            .await;
        if result.is_err() {
            self.restore_runner_lease_after_failed_transition(
                previous,
                TurnStatus::CancelRequested,
            )
            .await;
        }
        let response = result?;
        match response.status {
            status if status.is_terminal() => {
                self.runner_lease_store()
                    .delete_best_effort(response.run_id)
                    .await;
            }
            _ => {}
        }
        Ok(response)
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        let (snapshot, _) = self
            .read_snapshot_with_runner_lease_overlay(RunnerLeaseOverlay::Run(request.run_id))
            .await?;
        self.build_in_memory_store(snapshot)?
            .get_run_state(request)
            .await
    }
}

#[async_trait]
impl<F> TurnSpawnTreeStateStore for FilesystemTurnStateStore<F>
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
                (outcome, store)
            }
        })
        .await
    }

    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        let (snapshot, _) = self.read_snapshot().await?;
        // Walk the snapshot directly instead of rebuilding the in-memory store
        // (which constructs every index for every record) just to answer a
        // single parent→children lookup.
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
            (outcome, store)
        })
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
            (outcome, store)
        })
        .await
    }
}

#[async_trait]
impl<F> TurnEventProjectionSource for FilesystemTurnStateStore<F>
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
impl<F> LoopCheckpointStore for FilesystemTurnStateStore<F>
where
    F: RootFilesystem,
{
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| {
            let request = request.clone();
            async move {
                let outcome = store.put_loop_checkpoint(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        let (snapshot, _) = self.read_snapshot().await?;
        self.build_in_memory_store(snapshot)?
            .get_loop_checkpoint(request)
            .await
    }
}

#[async_trait]
impl<F> TurnRunTransitionPort for FilesystemTurnStateStore<F>
where
    F: RootFilesystem,
{
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        let claimed = self
            .apply(RunnerLeaseOverlay::None, |store| {
                let request = request.clone();
                async move {
                    let outcome = store.claim_next_run(request).await;
                    (outcome, store)
                }
            })
            .await?;
        if let Some(claimed) = &claimed
            && let Err(error) = self
                .seed_runner_lease_from_snapshot_inner(claimed.state.run_id)
                .await
        {
            self.compensate_failed_claim(claimed).await;
            return Err(error);
        }
        Ok(claimed)
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
                    (outcome, store)
                }
            })
            .await;
        if let Ok(response) = &result {
            for state in &response.recovered {
                self.runner_lease_store()
                    .delete_best_effort(state.run_id)
                    .await;
            }
            self.clear_snapshot_cache();
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
                (outcome, store)
            }
        })
        .await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            request.run_id,
            request.runner_id,
            request.lease_token,
            request.reason.status(),
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.block_run(request).await;
                    (outcome, store)
                }
            },
        )
        .await
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Completed,
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.complete_run(request).await;
                    (outcome, store)
                }
            },
        )
        .await
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Cancelled,
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.cancel_run(request).await;
                    (outcome, store)
                }
            },
        )
        .await
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Failed,
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.fail_run(request).await;
                    (outcome, store)
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
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Failed,
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.record_runner_failure(request).await;
                    (outcome, store)
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
            request.run_id,
            request.runner_id,
            request.lease_token,
            TurnStatus::Queued,
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.relinquish_run(request).await;
                    (outcome, store)
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
            request.run_id,
            request.runner_id,
            request.lease_token,
            retired_status_for_loop_exit(&request.mapping),
            |store| {
                let request = request.clone();
                async move {
                    let outcome = store.apply_validated_loop_exit(request).await;
                    (outcome, store)
                }
            },
        )
        .await
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
