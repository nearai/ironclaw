//! Filesystem-backed [`TurnStateStore`] implementation.
//!
//! Persists the entire [`TurnPersistenceSnapshot`] as a single JSON blob under
//! the `/turns` mount alias (alias-relative path: `/turns/state.json`). Every
//! mutation reads the snapshot, delegates to an [`InMemoryTurnStateStore`] in
//! a transient `apply` closure, and writes the resulting snapshot back with
//! optimistic CAS + bounded retry. Reads load the snapshot and project
//! through the in-memory store without writing back.
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
    sync::{Arc, Mutex, MutexGuard, OnceLock, Weak},
};

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RecordVersion,
    RootFilesystem, ScopedFilesystem,
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
    TurnScope, TurnSpawnTreeStateStore, TurnStateStore,
    events::project_turn_events,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest, RecoverExpiredLeasesRequest,
        RecoverExpiredLeasesResponse, RelinquishRunRequest, TurnRunTransitionPort,
    },
};

/// Bound on the CAS retry loop. Picked deliberately small: the in-process
/// per-path lock map collapses contention to one writer at a time, and
/// cross-process contention on filesystem mounts is what the
/// [`TurnError::Unavailable`] return shape is meant to surface.
const FILESYSTEM_CAS_RETRIES: usize = 8;

const TURNS_PREFIX: &str = "/turns";
const TURNS_SNAPSHOT_FILE: &str = "state.json";

/// Filesystem-backed turn-state store under the `/turns` mount alias.
///
/// Construct with a [`ScopedFilesystem`] over any [`RootFilesystem`]. The
/// [`ScopedFilesystem`] resolves the `/turns` alias to a tenant/user-scoped
/// [`VirtualPath`](ironclaw_host_api::VirtualPath) per its
/// [`MountView`](ironclaw_host_api::MountView) and enforces per-op ACL before
/// any backend dispatch — so tenant isolation is structural rather than
/// something this crate has to re-derive from `TurnScope.tenant_id`.
/// Within-tenant axes (agent/project/thread) stay in the persisted snapshot
/// records because they are not covered by the per-tenant `MountAlias`.
pub struct FilesystemTurnStateStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    limits: InMemoryTurnStateStoreLimits,
    admission_limit_provider: Arc<dyn TurnAdmissionLimitProvider>,
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

    /// Read the persistence snapshot from `/turns/state.json`. Returns an
    /// empty snapshot if the blob is missing — `start` semantics for a fresh
    /// tenant/user mount.
    pub async fn persistence_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let (snapshot, _) = self.read_snapshot().await?;
        Ok(snapshot)
    }

    async fn read_snapshot(
        &self,
    ) -> Result<(TurnPersistenceSnapshot, Option<RecordVersion>), TurnError> {
        let path = snapshot_path()?;
        let record_lock = filesystem_record_lock(self.filesystem.as_ref(), &path);
        let _guard = record_lock.lock().await;
        self.read_snapshot_unlocked().await
    }

    async fn read_snapshot_unlocked(
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
    /// re-reads (cross-process contention); on `Unsupported` it falls back
    /// to `CasExpectation::Any` so the byte-only `LocalFilesystem` path
    /// still works through the per-record `FILESYSTEM_RECORD_LOCKS` map.
    async fn apply<T, A, Fut>(&self, mut apply: A) -> Result<T, TurnError>
    where
        A: FnMut(InMemoryTurnStateStore) -> Fut,
        Fut: std::future::Future<Output = (Result<T, TurnError>, InMemoryTurnStateStore)>,
    {
        let path = snapshot_path()?;
        let record_lock = filesystem_record_lock(self.filesystem.as_ref(), &path);
        let _guard = record_lock.lock().await;
        for _ in 0..FILESYSTEM_CAS_RETRIES {
            let (snapshot, version) = self.read_snapshot_unlocked().await?;
            let old_snapshot = snapshot.clone();
            let store = self.build_in_memory_store(snapshot)?;
            let (outcome, store) = apply(store).await;
            let new_snapshot = store.persistence_snapshot();
            if new_snapshot == old_snapshot {
                return outcome;
            }
            let entry = snapshot_entry(&new_snapshot)?;
            let cas = match version {
                Some(v) => CasExpectation::Version(v),
                None => CasExpectation::Absent,
            };
            match put_with_cas(self.filesystem.as_ref(), &path, entry, cas).await {
                Ok(()) => return outcome,
                Err(PutError::VersionMismatch) => continue,
                Err(PutError::Other(error)) => return Err(error),
            }
        }
        Err(TurnError::Unavailable {
            reason: "turn state filesystem CAS retries exhausted".to_string(),
        })
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
        self.apply(|store| {
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
        self.apply(|store| {
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
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.request_cancel(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        let (snapshot, _) = self.read_snapshot().await?;
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
        self.apply(|store| {
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
        let (snapshot, _) = self.read_snapshot().await?;
        Ok(project_run_record(&snapshot, scope, run_id))
    }

    async fn reserve_tree_descendants(
        &self,
        scope: &TurnScope,
        root_run_id: TurnRunId,
        delta: u32,
        cap: u32,
    ) -> Result<SpawnTreeReservation, TurnError> {
        self.apply(|store| async move {
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
        self.apply(|store| async move {
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
        self.apply(|store| {
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
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.claim_next_run(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn heartbeat(&self, request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.heartbeat(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn recover_expired_leases(
        &self,
        request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.recover_expired_leases(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn record_model_route_snapshot(
        &self,
        request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.record_model_route_snapshot(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.block_run(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.complete_run(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.cancel_run(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.fail_run(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.record_runner_failure(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn relinquish_run(
        &self,
        request: RelinquishRunRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.relinquish_run(request).await;
                (outcome, store)
            }
        })
        .await
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.apply(|store| {
            let request = request.clone();
            async move {
                let outcome = store.apply_validated_loop_exit(request).await;
                (outcome, store)
            }
        })
        .await
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
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
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

type FilesystemRecordLock = Arc<tokio::sync::Mutex<()>>;

// Per-resolved-record async serialization for the filesystem-backed turn store.
//
// Values are stored as `Weak<Mutex<()>>` so the map does not pin lock entries
// alive once all in-flight operations on a path have released their `Arc`
// clones. The key is the backend virtual path when the scoped filesystem can
// resolve it, not the alias-relative path shared by every mount. Mirrors the
// per-record lock map shape used by
// `ironclaw_run_state::FilesystemRunStateStore`; only one snapshot path lives
// in this map per tenant/user, so churn is even lower here than there.
static FILESYSTEM_RECORD_LOCKS: OnceLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();

fn filesystem_record_lock<F>(
    filesystem: &ScopedFilesystem<F>,
    path: &ScopedPath,
) -> FilesystemRecordLock
where
    F: RootFilesystem,
{
    let key = filesystem
        .resolve(&ResourceScope::system(), path)
        .map(|virtual_path| virtual_path.as_str().to_string())
        .unwrap_or_else(|_| path.as_str().to_string());
    filesystem_record_lock_for_key(&key)
}

fn filesystem_record_lock_for_key(key: &str) -> FilesystemRecordLock {
    let locks = FILESYSTEM_RECORD_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard: MutexGuard<'_, HashMap<String, Weak<tokio::sync::Mutex<()>>>> = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.retain(|_, weak| weak.strong_count() > 0);
    if let Some(existing) = guard.get(key).and_then(Weak::upgrade) {
        return existing;
    }
    let fresh: FilesystemRecordLock = Arc::new(tokio::sync::Mutex::new(()));
    guard.insert(key.to_string(), Arc::downgrade(&fresh));
    fresh
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
/// Falls back to `CasExpectation::Any` when the backend reports `Unsupported`
/// for the request — `LocalFilesystem` is byte-only and only accepts `Any`.
/// On a byte-only backend the in-process record-lock map provides
/// intra-process serialization; cross-process safety on those backends is a
/// documented process-local limitation (matches
/// `ironclaw_run_state::put_with_cas`).
async fn put_with_cas<F>(
    filesystem: &ScopedFilesystem<F>,
    path: &ScopedPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<(), PutError>
where
    F: RootFilesystem,
{
    let fallback_entry = entry.clone();
    let scope = ResourceScope::system();
    match filesystem.put(&scope, path, entry, cas).await {
        Ok(_) => Ok(()),
        Err(FilesystemError::VersionMismatch { .. }) => Err(PutError::VersionMismatch),
        Err(FilesystemError::Unsupported {
            operation: FilesystemOperation::WriteFile,
            ..
        }) => filesystem
            .put(&scope, path, fallback_entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(|error| PutError::Other(fs_error(error))),
        Err(error) => Err(PutError::Other(fs_error(error))),
    }
}
