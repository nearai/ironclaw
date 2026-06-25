//! Filesystem-backed [`TurnStateStore`] implementation.
//!
//! Persists the lower-churn [`TurnPersistenceSnapshot`] as a JSON blob under
//! the `/turns` mount alias (alias-relative path: `/turns/state.json`) and
//! high-churn runner lease heartbeats as per-run CAS records under
//! `/turns/runner-leases`. Snapshot mutations read the snapshot, overlay
//! current runner leases, delegate to an [`InMemoryTurnStateStore`] in a
//! transient `apply` closure, and write the resulting snapshot back with
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
//! /turns/runner-leases/{run_id}.json
//! ```
//!
//! Within-tenant scoping (agent/project/thread) is encoded inside the
//! snapshot body via `TurnScope` on every persisted record; no extra path
//! segments are needed because the snapshot lives at the tenant/user level.
//! Tenant + user identity moves into the caller's `MountView` per the
//! per-tenant `MountAlias` rewriting, so neither prefix is encoded in the
//! path itself.

use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RecordKind,
    RecordVersion, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{ResourceScope, ScopedPath, UserId};

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

mod runner_lease;

/// Bound on the CAS retry loop. The per-user snapshot is intentionally written
/// with optimistic CAS instead of an in-process write gate, so bursts of
/// same-user transitions can overlap without parking unrelated turn-state
/// callers behind one wedged operation.
const FILESYSTEM_CAS_RETRIES: usize = 32;
const FILESYSTEM_APPLY_TIMEOUT: Duration = Duration::from_secs(15);
const FILESYSTEM_CAS_BACKOFF_BASE: Duration = Duration::from_millis(2);
const FILESYSTEM_CAS_BACKOFF_MAX: Duration = Duration::from_millis(50);
const SNAPSHOT_READ_CACHE_TTL: Duration = Duration::from_millis(500);

const TURNS_PREFIX: &str = "/turns";
const TURNS_SNAPSHOT_FILE: &str = "state.json";
const TURNS_SNAPSHOT_KIND: &str = "turn_state_snapshot";

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
        match overlay {
            RunnerLeaseOverlay::None => Ok(snapshot),
            RunnerLeaseOverlay::Run(run_id) => {
                let sidecar = self.runner_lease_sidecar();
                self.sidecar_with_timeout(
                    sidecar.overlay_run(snapshot, run_id),
                    "overlay run lease",
                )
                .await
            }
            RunnerLeaseOverlay::All => {
                let sidecar = self.runner_lease_sidecar();
                self.sidecar_with_timeout(sidecar.overlay_snapshot(snapshot), "overlay leases")
                    .await
            }
        }
    }

    async fn seed_runner_lease_from_snapshot_inner(
        &self,
        run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        let (snapshot, _version) = self.read_snapshot_from_filesystem().await?;
        let sidecar = self.runner_lease_sidecar();
        self.sidecar_with_timeout(
            sidecar.seed_from_snapshot(&snapshot, run_id),
            "seed runner lease",
        )
        .await?;
        self.clear_snapshot_cache();
        Ok(())
    }

    async fn cleanup_runner_lease_after_state(&self, result: &Result<TurnRunState, TurnError>) {
        let sidecar = self.runner_lease_sidecar();
        self.sidecar_unit_best_effort_with_timeout(
            sidecar.cleanup_after_state(result),
            "cleanup runner lease",
        )
        .await;
        self.clear_snapshot_cache();
    }

    async fn heartbeat_runner_lease(
        &self,
        request: HeartbeatRequest,
    ) -> Result<EventCursor, TurnError> {
        let sidecar = self.runner_lease_sidecar();
        let cursor = match self
            .sidecar_with_timeout(sidecar.heartbeat(request.clone()), "heartbeat runner lease")
            .await
        {
            Err(TurnError::ScopeNotFound) => {
                self.seed_missing_runner_lease_from_snapshot(request.run_id)
                    .await?;
                let sidecar = self.runner_lease_sidecar();
                self.sidecar_with_timeout(sidecar.heartbeat(request), "heartbeat runner lease")
                    .await?
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
        let sidecar = self.runner_lease_sidecar();
        self.sidecar_with_timeout(
            sidecar.seed_from_snapshot_if_missing(&snapshot, run_id),
            "seed missing runner lease",
        )
        .await
    }

    async fn prepare_cancel_requested_runner_lease(
        &self,
        request: &CancelRunRequest,
    ) -> Result<Option<runner_lease::RunnerLeaseRecord>, TurnError> {
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
        let sidecar = self.runner_lease_sidecar();
        self.sidecar_with_timeout(
            sidecar.mark_cancel_requested_from_snapshot(&snapshot, request.run_id),
            "mark runner lease cancel requested",
        )
        .await
    }

    async fn prepare_runner_lease_retirement(
        &self,
        run_id: TurnRunId,
        runner_id: crate::TurnRunnerId,
        lease_token: crate::TurnLeaseToken,
        retired_status: TurnStatus,
    ) -> Result<Option<runner_lease::RunnerLeaseRecord>, TurnError> {
        let (snapshot, _version) = self.read_snapshot_from_filesystem().await?;
        let sidecar = self.runner_lease_sidecar();
        self.sidecar_with_timeout(
            sidecar.retire_runner_lease_from_snapshot(
                &snapshot,
                run_id,
                runner_id,
                lease_token,
                retired_status,
            ),
            "retire runner lease",
        )
        .await
    }

    async fn restore_runner_lease_after_failed_transition(
        &self,
        previous: Option<runner_lease::RunnerLeaseRecord>,
        current_status: TurnStatus,
    ) {
        let Some(previous) = previous else {
            return;
        };
        let sidecar = self.runner_lease_sidecar();
        self.sidecar_best_effort_with_timeout(
            sidecar.restore_if_current_status(previous, current_status),
            "restore runner lease",
        )
        .await;
        self.clear_snapshot_cache();
    }

    async fn sidecar_with_timeout<T, Fut>(
        &self,
        future: Fut,
        operation: &'static str,
    ) -> Result<T, TurnError>
    where
        Fut: Future<Output = Result<T, TurnError>>,
    {
        match tokio::time::timeout(self.apply_timeout, future).await {
            Ok(result) => result,
            Err(_) => Err(TurnError::Unavailable {
                reason: format!("turn runner lease {operation} timed out"),
            }),
        }
    }

    async fn sidecar_best_effort_with_timeout<Fut>(&self, future: Fut, operation: &'static str)
    where
        Fut: Future<Output = Result<(), TurnError>>,
    {
        match tokio::time::timeout(self.apply_timeout, future).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(%error, operation, "turn runner lease best-effort operation failed");
            }
            Err(_) => {
                tracing::debug!(
                    operation,
                    "turn runner lease best-effort operation timed out"
                );
            }
        }
    }

    async fn sidecar_unit_best_effort_with_timeout<Fut>(&self, future: Fut, operation: &'static str)
    where
        Fut: Future<Output = ()>,
    {
        match tokio::time::timeout(self.apply_timeout, future).await {
            Ok(()) => {}
            Err(_) => {
                tracing::debug!(
                    operation,
                    "turn runner lease best-effort operation timed out"
                );
            }
        }
    }

    fn runner_lease_sidecar(&self) -> runner_lease::RunnerLeaseSidecar<F> {
        runner_lease::RunnerLeaseSidecar::new(
            Arc::clone(&self.filesystem),
            self.limits.runner_lease_ttl,
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
                "failed to compensate turn claim after runner lease sidecar seed failed"
            );
        }
        self.clear_snapshot_cache();
    }
}

#[derive(Clone, Copy)]
enum RunnerLeaseOverlay {
    None,
    Run(TurnRunId),
    All,
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
                let sidecar = self.runner_lease_sidecar();
                self.sidecar_unit_best_effort_with_timeout(
                    sidecar.delete_best_effort(response.run_id),
                    "delete terminal runner lease",
                )
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
        Ok(project_children_of(&snapshot, scope, run_id))
    }

    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        let (snapshot, _) = self
            .read_snapshot_with_runner_lease_overlay(RunnerLeaseOverlay::Run(run_id))
            .await?;
        Ok(project_run_record(&snapshot, scope, run_id))
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
            let sidecar = self.runner_lease_sidecar();
            for state in &response.recovered {
                self.sidecar_unit_best_effort_with_timeout(
                    sidecar.delete_best_effort(state.run_id),
                    "delete recovered runner lease",
                )
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

/// Pre-resolved run-profile resolver used to thread the resolver result
/// *into* the apply closure. The resolver future runs once per
/// `submit_turn` call outside the CAS loop because resolving may issue I/O
/// the lock-holding closure shouldn't carry; the resolution outcome is then
/// constant for the retry loop.
#[derive(Clone)]
struct PreResolvedRunProfileResolver {
    result: Result<crate::ResolvedRunProfile, crate::RunProfileResolutionError>,
}

impl PreResolvedRunProfileResolver {
    fn new(result: Result<crate::ResolvedRunProfile, crate::RunProfileResolutionError>) -> Self {
        Self { result }
    }
}

#[async_trait]
impl RunProfileResolver for PreResolvedRunProfileResolver {
    async fn resolve_run_profile(
        &self,
        _request: crate::RunProfileResolutionRequest,
    ) -> Result<crate::ResolvedRunProfile, crate::RunProfileResolutionError> {
        self.result.clone()
    }
}

fn snapshot_path() -> Result<ScopedPath, TurnError> {
    ScopedPath::new(format!("{TURNS_PREFIX}/{TURNS_SNAPSHOT_FILE}")).map_err(|error| {
        TurnError::Unavailable {
            reason: format!("invalid turn-state snapshot path: {error}"),
        }
    })
}

/// Project the children of a run directly from a snapshot without building
/// an `InMemoryTurnStateStore`. Mirrors `InMemoryTurnStateStore::children_of`
/// scope semantics: returns an empty list when the parent is missing or out of
/// scope, filters children by the parent's scope envelope (tenant/agent/project),
/// and sorts by `received_at`.
fn project_children_of(
    snapshot: &TurnPersistenceSnapshot,
    scope: &TurnScope,
    run_id: TurnRunId,
) -> Vec<TurnRunRecord> {
    let Some(parent) = snapshot.runs.iter().find(|record| record.run_id == run_id) else {
        return Vec::new();
    };
    if parent.scope != *scope {
        return Vec::new();
    }
    let mut children: Vec<TurnRunRecord> = snapshot
        .runs
        .iter()
        .filter(|record| {
            record.parent_run_id == Some(run_id)
                && record.scope.tenant_id == scope.tenant_id
                && record.scope.agent_id == scope.agent_id
                && record.scope.project_id == scope.project_id
        })
        .cloned()
        .collect();
    children.sort_by_key(|record| record.received_at);
    children
}

/// Project a run record by id directly from a snapshot, scoped exactly to
/// `scope`. Mirrors `InMemoryTurnStateStore::get_run_record` semantics.
fn project_run_record(
    snapshot: &TurnPersistenceSnapshot,
    scope: &TurnScope,
    run_id: TurnRunId,
) -> Option<TurnRunRecord> {
    snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id && record.scope == *scope)
        .cloned()
}

fn snapshot_entry(snapshot: &TurnPersistenceSnapshot) -> Result<Entry, TurnError> {
    let body = serde_json::to_vec_pretty(snapshot).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state snapshot serialization failed: {error}"),
    })?;
    let kind = RecordKind::new(TURNS_SNAPSHOT_KIND).map_err(|error| TurnError::Unavailable {
        reason: format!("invalid turn-state snapshot record kind: {error}"),
    })?;
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    entry.kind = Some(kind);
    Ok(entry)
}

fn deserialize_snapshot(bytes: &[u8]) -> Result<TurnPersistenceSnapshot, TurnError> {
    serde_json::from_slice(bytes).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state snapshot deserialization failed: {error}"),
    })
}

fn fs_error(error: FilesystemError) -> TurnError {
    tracing::debug!(%error, "turn state filesystem operation failed");
    TurnError::Unavailable {
        reason: "turn state persistence temporarily unavailable".to_string(),
    }
}

async fn cas_retry_backoff(attempt: usize) {
    let shift = attempt.min(8) as u32;
    let multiplier = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
    let base_delay = FILESYSTEM_CAS_BACKOFF_BASE
        .saturating_mul(multiplier)
        .min(FILESYSTEM_CAS_BACKOFF_MAX);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| {
            let jitter_ceiling = base_delay.as_millis().max(1);
            Duration::from_millis((elapsed.as_nanos() % jitter_ceiling) as u64)
        })
        .unwrap_or_default();
    tokio::time::sleep(base_delay.saturating_add(jitter)).await;
}

/// Local error classification for the CAS-aware put helper.
enum PutError {
    /// Backend reported `VersionMismatch` (cross-process raced us). The
    /// caller retries by re-reading the current snapshot.
    VersionMismatch,
    /// Any other backend or serialization failure; surface to caller.
    Other(TurnError),
}

/// Issue a `put` honoring the requested CAS expectation.
///
/// Turn state is a single per-user snapshot, so this store requires a backend
/// with real `Absent` / `Version` CAS. Falling back to `Any` would turn a
/// stale-snapshot race into a blind overwrite.
async fn put_with_cas<F>(
    filesystem: &ScopedFilesystem<F>,
    path: &ScopedPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<RecordVersion, PutError>
where
    F: RootFilesystem,
{
    let scope = ResourceScope::system();
    match filesystem.put(&scope, path, entry, cas).await {
        Ok(version) => Ok(version),
        Err(FilesystemError::VersionMismatch { .. }) => Err(PutError::VersionMismatch),
        Err(FilesystemError::Unsupported {
            operation: FilesystemOperation::WriteFile,
            ..
        }) => Err(PutError::Other(TurnError::Unavailable {
            reason: "turn state filesystem backend must support versioned CAS".to_string(),
        })),
        Err(error) => Err(PutError::Other(fs_error(error))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_snapshot_freshness_is_bounded() {
        let snapshot = TurnPersistenceSnapshot::default();
        let fresh = CachedSnapshot::new(snapshot.clone(), None);
        assert!(fresh.is_fresh());

        let stale = CachedSnapshot {
            snapshot,
            version: None,
            loaded_at: Instant::now() - SNAPSHOT_READ_CACHE_TTL - Duration::from_millis(1),
        };
        assert!(!stale.is_fresh());
    }

    #[tokio::test]
    async fn no_op_apply_clears_snapshot_cache_before_returning() {
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(ironclaw_filesystem::InMemoryBackend::new()),
            ironclaw_host_api::MountView::new(vec![ironclaw_host_api::MountGrant::new(
                ironclaw_host_api::MountAlias::new("/turns").unwrap(),
                ironclaw_host_api::VirtualPath::new("/engine/turns").unwrap(),
                ironclaw_host_api::MountPermissions::read_write_list_delete(),
            )])
            .unwrap(),
        ));
        let store = FilesystemTurnStateStore::new(filesystem);
        store.store_snapshot_cache((
            TurnPersistenceSnapshot::default(),
            Some(RecordVersion::from_backend(99)),
        ));

        store
            .apply(RunnerLeaseOverlay::None, |store| async move {
                (Ok::<_, TurnError>(()), store)
            })
            .await
            .unwrap();

        assert!(store.fresh_cached_snapshot().is_none());
    }
}
