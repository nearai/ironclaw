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
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, FILESYSTEM_APPLY_TIMEOUT, RecordVersion,
    RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{ResourceScope, UserId};
use tokio::sync::RwLock;
use tracing::{Instrument, field};

use crate::{
    AllowAllTurnAdmissionLimitProvider, CancelRunRequest, CancelRunResponse, EventCursor,
    GetLoopCheckpointRequest, GetRunStateRequest, InMemoryTurnStateStore,
    InMemoryTurnStateStoreLimits, LoopCheckpointRecord, LoopCheckpointStore,
    PutLoopCheckpointRequest, ResumeTurnRequest, ResumeTurnResponse, RetryTurnRequest,
    RetryTurnResponse, RunProfileResolver, SpawnTreeReservation, SubmitChildRunRequest,
    SubmitTurnRequest, SubmitTurnResponse, TurnAdmissionLimitProvider, TurnAdmissionPolicy,
    TurnError, TurnEventPage, TurnEventProjectionSource, TurnPersistenceSnapshot, TurnRunId,
    TurnRunRecord, TurnRunState, TurnScope, TurnSpawnTreeStateStore, TurnStateStore, TurnStatus,
    events::project_turn_events,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimRunsRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest,
        HeartbeatRequest, RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest,
        RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse, RelinquishRunRequest,
        TurnRunTransitionPort, TurnRunnerOutcome,
    },
};

mod io;
mod profile_resolver;
mod projection;
mod row_store;
mod runner_lease;

use io::{deserialize_snapshot, fs_error, snapshot_entry, snapshot_path};
use profile_resolver::PreResolvedRunProfileResolver;
use runner_lease::{RunnerLeaseMemory, RunnerLeaseOverlay, RunnerLeaseRecord, RunnerLeaseStore};

pub use row_store::FilesystemTurnStateRowStore;

#[cfg(test)]
mod tests;

const SNAPSHOT_READ_CACHE_TTL: Duration = Duration::from_millis(500);

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
        span.record("run_id", field::display(run_id));
    }

    span
}

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
    async fn apply<T, A, Fut>(&self, overlay: RunnerLeaseOverlay, apply: A) -> Result<T, TurnError>
    where
        A: FnMut(InMemoryTurnStateStore) -> Fut + Send,
        Fut: std::future::Future<Output = (Result<T, TurnError>, InMemoryTurnStateStore)> + Send,
        T: Send,
    {
        let path = snapshot_path()?;
        // Clear stale cache before entering the CAS loop so every retry reads
        // through to the backend rather than a potentially stale in-process
        // snapshot.
        self.clear_snapshot_cache();

        let scope = ResourceScope::system();
        let limits = self.limits;
        let admission_limit_provider = self.admission_limit_provider.clone();
        let runner_leases_for_overlay = Arc::clone(&self.runner_leases);
        let runner_lease_ttl = limits.runner_lease_ttl;
        let overlay_timeout = self.apply_timeout;

        // Wrap the caller's FnMut in Arc<Mutex> so the `cas_update` callback
        // can call it without capturing a mutable reference into the async block.
        // The lock is never held across an `.await` point: the FnMut is called
        // synchronously (returning a Future), the guard is dropped, and only
        // then the returned Future is awaited. CAS retries are sequential, so
        // there is no lock contention in practice.
        let apply = Arc::new(Mutex::new(apply));

        let cas_future = cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            // decode: stored body → TurnPersistenceSnapshot.
            |bytes: &[u8]| deserialize_snapshot(bytes),
            // encode: next snapshot → versioned Entry.
            |snapshot: &TurnPersistenceSnapshot| snapshot_entry(snapshot),
            // apply: per CAS-retry callback.
            //   1. Apply runner-lease overlay so the InMemoryTurnStateStore sees
            //      up-to-date `last_heartbeat_at` / `lease_expires_at` from the
            //      in-memory runner-lease map. The overlaid snapshot is both the
            //      no-op baseline and (on a real transition) what cas_update
            //      writes back — mirroring the pre-merge `apply_with_retry`,
            //      which built and persisted from the overlaid `old_snapshot`.
            //   2. Build the transient InMemoryTurnStateStore.
            //   3. Call the caller's closure (FnMut) via the mutex, then await
            //      the returned Future without holding the lock.
            move |current: Option<TurnPersistenceSnapshot>| {
                let apply = Arc::clone(&apply);
                let runner_leases_for_overlay = Arc::clone(&runner_leases_for_overlay);
                let admission_limit_provider = admission_limit_provider.clone();

                async move {
                    let raw_snapshot = current.unwrap_or_default();
                    let (overlaid_snapshot, _) = RunnerLeaseStore::new(
                        runner_leases_for_overlay,
                        runner_lease_ttl,
                        overlay_timeout,
                    )
                    .overlay((raw_snapshot, None), overlay)
                    .await?;
                    // No-op baseline is the OVERLAID snapshot, not the raw
                    // backend body. `RunnerLeaseOverlay::Run` / `::All` patch
                    // time-varying lease fields (last_heartbeat_at,
                    // lease_expires_at) from the in-memory runner-lease map into
                    // the snapshot the closure sees. Comparing the closure's result
                    // against the raw body would classify an inert transition as
                    // a real mutation and write on every call — version churn and
                    // CAS retries under load. This mirrors the pre-merge
                    // `apply_with_retry`, which diffed `new_snapshot` against the
                    // overlaid `old_snapshot`, and subsumes the absent +
                    // default-snapshot case (the overlay of an absent record is
                    // the default snapshot, so an empty store stays inert here).
                    let baseline = overlaid_snapshot.clone();

                    let store =
                        InMemoryTurnStateStore::from_persistence_snapshot_with_admission_limit_provider(
                            overlaid_snapshot,
                            limits,
                            admission_limit_provider,
                        )?;

                    // Call the FnMut via mutex without holding the lock across `.await`.
                    let apply_fut = {
                        let mut guard = apply.lock().map_err(|_| TurnError::Unavailable {
                            reason: "turn state apply closure panicked".to_string(),
                        })?;
                        (*guard)(store)
                    };
                    let (outcome, store) = apply_fut.await;
                    let new_snapshot = store.persistence_snapshot();

                    let outcome = match outcome {
                        Ok(value) => Ok(value),
                        Err(error) if new_snapshot != baseline => Err(error),
                        Err(error) => return Err(error),
                    };

                    // Inert transition: the closure left the overlaid
                    // snapshot unchanged, so there is nothing to persist.
                    // Signal no-op so `cas_update` skips the write entirely
                    // (no version bump).
                    if new_snapshot == baseline {
                        return Ok(CasApply::no_op(
                            new_snapshot.clone(),
                            (outcome, new_snapshot),
                        ));
                    }
                    // Thread the new snapshot back alongside the caller's
                    // outcome so the outer scope can populate the cache. Some
                    // failed operations still intentionally mutate durable
                    // audit state, such as retry ThreadBusy idempotency
                    // records.
                    Ok(CasApply::new(new_snapshot.clone(), (outcome, new_snapshot)))
                }
            },
        );

        // Run the CAS loop inside the apply timeout.
        //
        // Note: `cas_update` internally applies `FILESYSTEM_APPLY_TIMEOUT` (from
        // `ironclaw_filesystem`) to the whole CAS loop.  `self.apply_timeout`
        // defaults to the same shared constant but can be shortened in tests via
        // `with_apply_timeout`.  The outer `tokio::time::timeout` here enforces
        // the per-call deadline; `cas_update`'s inner timeout is an additional
        // guard inside the helper itself.
        let result: Result<
            (Result<T, TurnError>, TurnPersistenceSnapshot),
            CasUpdateError<TurnError>,
        > = match tokio::time::timeout(self.apply_timeout, cas_future).await {
            Ok(result) => result,
            Err(_) => {
                self.clear_snapshot_cache();
                return Err(TurnError::Unavailable {
                    reason: "turn state filesystem apply timed out".to_string(),
                });
            }
        };

        match result {
            Ok((outcome, written_snapshot)) => {
                // Successful write or explicit no-op (CasApply::no_op). Populate
                // the snapshot cache with the resulting snapshot so the next read
                // can skip a backend roundtrip. For a true write we cache the
                // written snapshot; for a no-op (absent+default) we cache the
                // default, which is observationally identical to re-reading an
                // absent record. We store `None` for the version; reads don't use
                // the version and writes always re-read fresh.
                self.store_snapshot_cache((written_snapshot, None));
                outcome
            }
            Err(e) => {
                self.clear_snapshot_cache();
                Err(map_cas_error(e))
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
        A: FnMut(InMemoryTurnStateStore) -> Fut + Send,
        Fut: std::future::Future<Output = (Result<TurnRunState, TurnError>, InMemoryTurnStateStore)>
            + Send,
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
        self.clear_snapshot_cache();
    }
}

/// Concrete filesystem turn-state store selected by composition.
///
/// Keeping this as a concrete enum, rather than a bundle of trait objects,
/// lets production wiring retain component type checks while allowing
/// filesystem-backed deployments to choose the row/append layout explicitly
/// while retaining the blob layout for compatibility tests and rollback seams.
pub enum FilesystemTurnStateStoreKind<F>
where
    F: RootFilesystem,
{
    // arch-exempt: large_file, boxed enum variant clippy fix in existing store facade, plan #5662
    Blob(Box<FilesystemTurnStateStore<F>>),
    Row(Box<FilesystemTurnStateRowStore<F>>),
}

impl<F> FilesystemTurnStateStoreKind<F>
where
    F: RootFilesystem,
{
    pub fn blob(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self::Blob(Box::new(FilesystemTurnStateStore::new(filesystem)))
    }

    pub fn row(filesystem: Arc<ScopedFilesystem<F>>) -> Self
    where
        F: 'static,
    {
        Self::Row(Box::new(FilesystemTurnStateRowStore::new(filesystem)))
    }

    pub fn with_limits(self, limits: InMemoryTurnStateStoreLimits) -> Self {
        match self {
            Self::Blob(store) => Self::Blob(Box::new((*store).with_limits(limits))),
            Self::Row(store) => Self::Row(Box::new((*store).with_limits(limits))),
        }
    }

    pub async fn persistence_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        match self {
            Self::Blob(store) => store.persistence_snapshot().await,
            Self::Row(store) => store.persistence_snapshot().await,
        }
    }
}

/// Map a [`CasUpdateError`] into a [`TurnError`].
fn map_cas_error(error: CasUpdateError<TurnError>) -> TurnError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout => TurnError::Unavailable {
            reason: "turn state filesystem apply timed out".to_string(),
        },
        CasUpdateError::RetriesExhausted => TurnError::Unavailable {
            reason: "turn state filesystem CAS retries exhausted".to_string(),
        },
        CasUpdateError::CasUnsupported => TurnError::Unavailable {
            reason: "turn state filesystem backend must support versioned CAS".to_string(),
        },
        CasUpdateError::Backend(fs) => fs_error(fs),
    }
}

#[async_trait]
impl<F> TurnStateStore for FilesystemTurnStateStoreKind<F>
where
    F: RootFilesystem,
{
    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        match self {
            Self::Blob(store) => {
                store
                    .submit_turn(request, admission_policy, run_profile_resolver)
                    .await
            }
            Self::Row(store) => {
                store
                    .submit_turn(request, admission_policy, run_profile_resolver)
                    .await
            }
        }
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        match self {
            Self::Blob(store) => store.resume_turn(request).await,
            Self::Row(store) => store.resume_turn(request).await,
        }
    }

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        match self {
            Self::Blob(store) => store.retry_turn(request).await,
            Self::Row(store) => store.retry_turn(request).await,
        }
    }

    async fn request_cancel(
        &self,
        request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        match self {
            Self::Blob(store) => store.request_cancel(request).await,
            Self::Row(store) => store.request_cancel(request).await,
        }
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.get_run_state(request).await,
            Self::Row(store) => store.get_run_state(request).await,
        }
    }

    async fn get_run_state_for_cancellation(
        &self,
        request: GetRunStateRequest,
    ) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.get_run_state_for_cancellation(request).await,
            Self::Row(store) => store.get_run_state_for_cancellation(request).await,
        }
    }
}

#[async_trait]
impl<F> TurnSpawnTreeStateStore for FilesystemTurnStateStoreKind<F>
where
    F: RootFilesystem,
{
    async fn submit_child_turn(
        &self,
        request: SubmitChildRunRequest,
        admission_policy: &dyn TurnAdmissionPolicy,
        run_profile_resolver: &dyn RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        match self {
            Self::Blob(store) => {
                store
                    .submit_child_turn(request, admission_policy, run_profile_resolver)
                    .await
            }
            Self::Row(store) => {
                store
                    .submit_child_turn(request, admission_policy, run_profile_resolver)
                    .await
            }
        }
    }

    async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<TurnRunRecord>, TurnError> {
        match self {
            Self::Blob(store) => store.children_of(scope, run_id).await,
            Self::Row(store) => store.children_of(scope, run_id).await,
        }
    }

    async fn get_run_record(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<Option<TurnRunRecord>, TurnError> {
        match self {
            Self::Blob(store) => store.get_run_record(scope, run_id).await,
            Self::Row(store) => store.get_run_record(scope, run_id).await,
        }
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        match self {
            Self::Blob(store) => {
                store
                    .reserve_tree_descendants(scope, root_run_id, delta, cap)
                    .await
            }
            Self::Row(store) => {
                store
                    .reserve_tree_descendants(scope, root_run_id, delta, cap)
                    .await
            }
        }
    }

    async fn release_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        match self {
            Self::Blob(store) => {
                store
                    .release_tree_descendants(scope, root_run_id, delta, idempotency_key)
                    .await
            }
            Self::Row(store) => {
                store
                    .release_tree_descendants(scope, root_run_id, delta, idempotency_key)
                    .await
            }
        }
    }

    async fn prune_released_child(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        match self {
            Self::Blob(store) => {
                store
                    .prune_released_child(scope, root_run_id, child_run_id)
                    .await
            }
            Self::Row(store) => {
                store
                    .prune_released_child(scope, root_run_id, child_run_id)
                    .await
            }
        }
    }
}

#[async_trait]
impl<F> TurnEventProjectionSource for FilesystemTurnStateStoreKind<F>
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
        match self {
            Self::Blob(store) => {
                store
                    .read_turn_events_after(scope, owner_user_id, after, limit)
                    .await
            }
            Self::Row(store) => {
                store
                    .read_turn_events_after(scope, owner_user_id, after, limit)
                    .await
            }
        }
    }
}

#[async_trait]
impl<F> LoopCheckpointStore for FilesystemTurnStateStoreKind<F>
where
    F: RootFilesystem,
{
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        match self {
            Self::Blob(store) => store.put_loop_checkpoint(request).await,
            Self::Row(store) => store.put_loop_checkpoint(request).await,
        }
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        match self {
            Self::Blob(store) => store.get_loop_checkpoint(request).await,
            Self::Row(store) => store.get_loop_checkpoint(request).await,
        }
    }
}

#[async_trait]
impl<F> TurnRunTransitionPort for FilesystemTurnStateStoreKind<F>
where
    F: RootFilesystem,
{
    async fn claim_next_run(
        &self,
        request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        match self {
            Self::Blob(store) => store.claim_next_run(request).await,
            Self::Row(store) => store.claim_next_run(request).await,
        }
    }

    async fn claim_next_runs(
        &self,
        request: ClaimRunsRequest,
    ) -> Result<Vec<ClaimedTurnRun>, TurnError> {
        match self {
            Self::Blob(store) => store.claim_next_runs(request).await,
            Self::Row(store) => store.claim_next_runs(request).await,
        }
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        match self {
            Self::Blob(store) => store.heartbeat(request).await,
            Self::Row(store) => store.heartbeat(request).await,
        }
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        match self {
            Self::Blob(store) => store.recover_expired_leases(request).await,
            Self::Row(store) => store.recover_expired_leases(request).await,
        }
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.record_model_route_snapshot(request).await,
            Self::Row(store) => store.record_model_route_snapshot(request).await,
        }
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.block_run(request).await,
            Self::Row(store) => store.block_run(request).await,
        }
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.complete_run(request).await,
            Self::Row(store) => store.complete_run(request).await,
        }
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.cancel_run(request).await,
            Self::Row(store) => store.cancel_run(request).await,
        }
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.fail_run(request).await,
            Self::Row(store) => store.fail_run(request).await,
        }
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.record_runner_failure(request).await,
            Self::Row(store) => store.record_runner_failure(request).await,
        }
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.relinquish_run(request).await,
            Self::Row(store) => store.relinquish_run(request).await,
        }
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        match self {
            Self::Blob(store) => store.apply_validated_loop_exit(request).await,
            Self::Row(store) => store.apply_validated_loop_exit(request).await,
        }
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
        self.apply(RunnerLeaseOverlay::None, |store| {
            let request = request.clone();
            async move {
                let outcome = store.resume_turn(request).await;
                (outcome, store)
            }
        })
        .instrument(turn_state_write_span(
            "resume_turn",
            Some(&request.scope),
            Some(&request.run_id),
        ))
        .await
    }

    async fn retry_turn(&self, request: RetryTurnRequest) -> Result<RetryTurnResponse, TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| {
            let request = request.clone();
            async move {
                // WS-3 implements failed-run retry semantics and idempotency.
                let outcome = store.retry_turn(request).await;
                (outcome, store)
            }
        })
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
        .instrument(span)
        .await
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
        idempotency_key: TurnRunId,
    ) -> Result<(), TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| async move {
            let outcome = store
                .release_tree_descendants(scope, root_run_id, delta, idempotency_key)
                .await;
            (outcome, store)
        })
        .instrument(turn_state_write_span(
            "release_tree_descendants",
            Some(scope),
            Some(&root_run_id),
        ))
        .await
    }

    async fn prune_released_child(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), TurnError> {
        self.apply(RunnerLeaseOverlay::None, |store| async move {
            let outcome = store
                .prune_released_child(scope, root_run_id, child_run_id)
                .await;
            (outcome, store)
        })
        .instrument(turn_state_write_span(
            "prune_released_child",
            Some(scope),
            Some(&root_run_id),
        ))
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
        .instrument(turn_state_write_span(
            "put_loop_checkpoint",
            Some(&request.scope),
            Some(&request.run_id),
        ))
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
        let span = turn_state_write_span("claim_next_run", request.scope_filter.as_ref(), None);
        async move {
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
        .instrument(span)
        .await
    }

    async fn claim_next_runs(
        &self,
        request: ClaimRunsRequest,
    ) -> Result<Vec<ClaimedTurnRun>, TurnError> {
        let span = turn_state_write_span("claim_next_runs", request.scope_filter.as_ref(), None);
        async move {
            let claimed = self
                .apply(RunnerLeaseOverlay::None, |store| {
                    let request = request.clone();
                    async move {
                        let outcome = store.claim_next_runs(request).await;
                        (outcome, store)
                    }
                })
                .await?;
            for run in &claimed {
                if let Err(error) = self
                    .seed_runner_lease_from_snapshot_inner(run.state.run_id)
                    .await
                {
                    for claimed_run in &claimed {
                        self.compensate_failed_claim(claimed_run).await;
                    }
                    return Err(error);
                }
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
                    (outcome, store)
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
            self.clear_snapshot_cache();
        }
        result
    }

    async fn latest_resumable_checkpoint(
        &self,
        scope: &TurnScope,
        turn_id: crate::TurnId,
        run_id: TurnRunId,
    ) -> Result<Option<crate::TurnCheckpointId>, TurnError> {
        let (snapshot, _) = self.read_snapshot().await?;
        self.build_in_memory_store(snapshot)?
            .latest_resumable_checkpoint(scope, turn_id, run_id)
            .await
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
        .instrument(turn_state_write_span(
            "record_model_route_snapshot",
            None,
            Some(&request.run_id),
        ))
        .await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply_run_state_transition(
            "block_run",
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
            "complete_run",
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
            "cancel_run",
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
            "fail_run",
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
            "record_runner_failure",
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
            "relinquish_run",
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
            "apply_validated_loop_exit",
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

/// Filesystem-backed durable sink for the in-memory turn-state authority's
/// blocked runs.
///
/// Writes the full [`TurnPersistenceSnapshot`] to the same `/turns/state.json`
/// alias-relative path the [`FilesystemTurnStateStore`] uses, but only when the
/// in-memory store reports a change to the set of gate-blocked runs (see
/// [`TurnStateBlockPersistence`](crate::TurnStateBlockPersistence)). This keeps
/// gate-parked turns (approval/auth) recoverable across a process restart while
/// leaving the normal claim/complete hot path untouched.
///
/// On process start, composition calls [`load`](Self::load) once and rehydrates
/// the in-memory store via
/// [`InMemoryTurnStateStore::from_persistence_snapshot`](crate::InMemoryTurnStateStore::from_persistence_snapshot).
pub struct FilesystemTurnStateBlockPersistence<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemTurnStateBlockPersistence<F>
where
    F: RootFilesystem,
{
    /// Build a sink over the same scoped filesystem the store persists to.
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    /// Load the last persisted snapshot for startup rehydration.
    ///
    /// Returns an empty snapshot when nothing has been persisted yet (fresh
    /// tenant/user, or a store that never blocked a run).
    pub async fn load(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let path = snapshot_path()?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => deserialize_snapshot(&versioned.entry.body),
            Ok(None) => Ok(TurnPersistenceSnapshot::default()),
            Err(error) => Err(fs_error(error)),
        }
    }
}

#[async_trait]
impl<F> crate::TurnStateBlockPersistence for FilesystemTurnStateBlockPersistence<F>
where
    F: RootFilesystem,
{
    async fn persist(&self, snapshot: &TurnPersistenceSnapshot) {
        // Best-effort by contract: a durable-write failure must never fail an
        // already-applied in-memory transition, so log and swallow. The
        // in-memory store remains authoritative; this snapshot only backs
        // restart recovery of gate-blocked turns.
        let path = match snapshot_path() {
            Ok(path) => path,
            Err(error) => {
                tracing::debug!(%error, "turn-state block persistence: invalid snapshot path");
                return;
            }
        };
        let entry = match snapshot_entry(snapshot) {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(%error, "turn-state block persistence: snapshot serialization failed");
                return;
            }
        };
        // Blind overwrite: the in-memory authority owns the truth, orders writes
        // by a monotonic sequence, and hands us the complete latest snapshot, so
        // last-writer-wins is correct here — there is no cross-process snapshot to
        // lose. A plain `put` with `CasExpectation::Any` (rather than the store's
        // read-modify-write `cas_update`) keeps this a single write off the hot
        // path.
        let scope = ResourceScope::system();
        if let Err(error) = self
            .filesystem
            .put(&scope, &path, entry, CasExpectation::Any)
            .await
        {
            tracing::debug!(%error, "turn-state block persistence: durable write failed");
        }
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
