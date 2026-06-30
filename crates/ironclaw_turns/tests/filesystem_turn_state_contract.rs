//! Contract tests for [`FilesystemTurnStateStore`] against a
//! [`ScopedFilesystem`] over a CAS-capable filesystem backend. The persistent
//! shape is a lower-churn `/turns/state.json` snapshot; active runner leases
//! are memory-backed and fall back to the snapshot after restart.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CasExpectation, CompositeRootFilesystem,
    ContentKind, DirEntry, Entry, FileStat, FilesystemError, FilesystemOperation, InMemoryBackend,
    IndexPolicy, LocalFilesystem, MountDescriptor, RecordVersion, RootFilesystem, ScopedFilesystem,
    StorageClass, VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, HostPath, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, ScopedPath,
    TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, FilesystemTurnStateStore, GetRunStateRequest,
    IdempotencyKey, InMemoryRunProfileResolver, ProductTurnContext, ReplyTargetBindingRef,
    RunOriginAdapter, RunProfileRequest, SanitizedCancelReason, SourceBindingRef,
    SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnError,
    TurnLeaseToken, TurnOriginKind, TurnOwner, TurnPersistenceSnapshot, TurnRunId, TurnRunnerId,
    TurnScope, TurnSpawnTreeStateStore, TurnStateStore, TurnStatus,
    runner::{
        ClaimRunRequest, CompleteRunRequest, HeartbeatRequest, RecoverExpiredLeasesRequest,
        TurnRunTransitionPort,
    },
};

/// Build a CAS-capable backend; local-dev and production mount `/turns` under
/// the structured `/tenants` root, not the byte-only local workspace root.
fn engine_filesystem() -> InMemoryBackend {
    InMemoryBackend::new()
}

fn byte_only_filesystem() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn engine_mount_descriptor<F>(backend: &F) -> MountDescriptor
where
    F: RootFilesystem,
{
    MountDescriptor {
        virtual_root: VirtualPath::new("/engine").unwrap(),
        backend_id: BackendId::new("test-turn-state").unwrap(),
        backend_kind: BackendKind::MemoryDocuments,
        storage_class: StorageClass::StructuredRecords,
        content_kind: ContentKind::StructuredRecord,
        index_policy: IndexPolicy::NotIndexed,
        capabilities: backend.capabilities(),
    }
}

fn catalog_root<F>(backend: Arc<F>) -> Arc<CompositeRootFilesystem>
where
    F: RootFilesystem + 'static,
{
    let mut root = CompositeRootFilesystem::new();
    root.mount(engine_mount_descriptor(backend.as_ref()), backend)
        .unwrap();
    Arc::new(root)
}

struct CountingFilesystem {
    inner: InMemoryBackend,
    get_calls: AtomicUsize,
}

impl CountingFilesystem {
    fn new(inner: InMemoryBackend) -> Self {
        Self {
            inner,
            get_calls: AtomicUsize::new(0),
        }
    }

    fn reset_get_calls(&self) {
        self.get_calls.store(0, Ordering::SeqCst);
    }

    fn get_calls(&self) -> usize {
        self.get_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl RootFilesystem for CountingFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.get_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

/// Wrap a [`RootFilesystem`] in a [`ScopedFilesystem`] that exposes the
/// `/turns` mount alias under a tenant/user-scoped subtree of the underlying
/// mount target.
fn scoped_turns_fs_at<F>(
    backend: Arc<F>,
    tenant: &str,
    user: &str,
) -> Arc<ScopedFilesystem<CompositeRootFilesystem>>
where
    F: RootFilesystem + 'static,
{
    let tenant_user_prefix = format!("/engine/tenants/{tenant}/users/{user}");
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("alias"),
        VirtualPath::new(format!("{tenant_user_prefix}/turns")).expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(
        catalog_root(backend),
        mounts,
    ))
}

fn scoped_turns_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<CompositeRootFilesystem>>
where
    F: RootFilesystem + 'static,
{
    scoped_turns_fs_at(backend, "test-tenant", "test-user")
}

fn snapshot_virtual_path() -> VirtualPath {
    VirtualPath::new("/engine/tenants/test-tenant/users/test-user/turns/state.json").unwrap()
}

fn runner_lease_virtual_path(run_id: TurnRunId) -> VirtualPath {
    VirtualPath::new(format!(
        "/engine/tenants/test-tenant/users/test-user/turns/runner-leases/{run_id}.json"
    ))
    .unwrap()
}

async fn overwrite_snapshot_lease_expiry(
    backend: &InMemoryBackend,
    run_id: TurnRunId,
    lease_expires_at: chrono::DateTime<Utc>,
) {
    let versioned = backend
        .get(&snapshot_virtual_path())
        .await
        .unwrap()
        .expect("snapshot");
    let mut snapshot: TurnPersistenceSnapshot =
        serde_json::from_slice(&versioned.entry.body).unwrap();
    let run = snapshot
        .runs
        .iter_mut()
        .find(|record| record.run_id == run_id)
        .expect("run in snapshot");
    run.lease_expires_at = Some(lease_expires_at);
    let mut entry = versioned.entry;
    entry.body = serde_json::to_vec_pretty(&snapshot).unwrap();
    backend
        .put(
            &snapshot_virtual_path(),
            entry,
            CasExpectation::Version(versioned.version),
        )
        .await
        .unwrap();
}

struct BlockingPutFilesystem<F> {
    inner: F,
    block_next_put: AtomicBool,
    put_blocked: AtomicBool,
    put_started: tokio::sync::Notify,
    release_put: tokio::sync::Notify,
}

impl<F> BlockingPutFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            block_next_put: AtomicBool::new(false),
            put_blocked: AtomicBool::new(false),
            put_started: tokio::sync::Notify::new(),
            release_put: tokio::sync::Notify::new(),
        }
    }

    fn block_next_put(&self) {
        self.block_next_put.store(true, Ordering::SeqCst);
    }

    async fn wait_for_blocked_put(&self) {
        while !self.put_blocked.load(Ordering::SeqCst) {
            self.put_started.notified().await;
        }
    }

    fn release_blocked_put(&self) {
        self.release_put.notify_one();
    }
}

struct BlockingSnapshotPutFilesystem<F> {
    inner: F,
    block_snapshot_puts: AtomicBool,
    snapshot_put_blocked: AtomicBool,
    snapshot_put_started: tokio::sync::Notify,
    release_snapshot_puts: tokio::sync::Notify,
}

struct RejectSnapshotGetFilesystem<F> {
    inner: F,
    reject_snapshot_gets: AtomicBool,
}

impl<F> BlockingSnapshotPutFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            block_snapshot_puts: AtomicBool::new(false),
            snapshot_put_blocked: AtomicBool::new(false),
            snapshot_put_started: tokio::sync::Notify::new(),
            release_snapshot_puts: tokio::sync::Notify::new(),
        }
    }

    fn block_snapshot_puts(&self) {
        self.block_snapshot_puts.store(true, Ordering::SeqCst);
    }

    async fn wait_for_blocked_snapshot_put(&self) {
        while !self.snapshot_put_blocked.load(Ordering::SeqCst) {
            self.snapshot_put_started.notified().await;
        }
    }

    fn release_snapshot_puts(&self) {
        self.block_snapshot_puts.store(false, Ordering::SeqCst);
        self.release_snapshot_puts.notify_waiters();
    }
}

impl<F> RejectSnapshotGetFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            reject_snapshot_gets: AtomicBool::new(false),
        }
    }

    fn reject_snapshot_gets(&self) {
        self.reject_snapshot_gets.store(true, Ordering::SeqCst);
    }
}

struct FirstWaveBlockingPutFilesystem<F> {
    inner: F,
    expected_first_wave_puts: AtomicUsize,
    first_wave_arrivals: AtomicUsize,
    version_mismatches: AtomicUsize,
    reject_puts: AtomicBool,
    first_wave_released: AtomicBool,
    first_wave_ready: tokio::sync::Notify,
    release_first_wave: tokio::sync::Notify,
    mismatch_retry_seen: AtomicBool,
    mismatch_retry_ready: tokio::sync::Notify,
}

impl<F> FirstWaveBlockingPutFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            expected_first_wave_puts: AtomicUsize::new(0),
            first_wave_arrivals: AtomicUsize::new(0),
            version_mismatches: AtomicUsize::new(0),
            reject_puts: AtomicBool::new(false),
            first_wave_released: AtomicBool::new(false),
            first_wave_ready: tokio::sync::Notify::new(),
            release_first_wave: tokio::sync::Notify::new(),
            mismatch_retry_seen: AtomicBool::new(false),
            mismatch_retry_ready: tokio::sync::Notify::new(),
        }
    }

    fn block_first_put_wave(&self, expected_puts: usize) {
        self.first_wave_arrivals.store(0, Ordering::SeqCst);
        self.expected_first_wave_puts
            .store(expected_puts, Ordering::SeqCst);
        self.first_wave_released.store(false, Ordering::SeqCst);
        self.mismatch_retry_seen.store(false, Ordering::SeqCst);
    }

    async fn wait_for_first_wave(&self) {
        let expected = self.expected_first_wave_puts.load(Ordering::SeqCst);
        while self.first_wave_arrivals.load(Ordering::SeqCst) < expected {
            self.first_wave_ready.notified().await;
        }
    }

    fn release_first_wave(&self) {
        self.first_wave_released.store(true, Ordering::SeqCst);
        self.release_first_wave.notify_waiters();
    }

    async fn wait_for_mismatch_retry_read(&self) {
        while !self.mismatch_retry_seen.load(Ordering::SeqCst) {
            self.mismatch_retry_ready.notified().await;
        }
    }

    fn version_mismatches(&self) -> usize {
        self.version_mismatches.load(Ordering::SeqCst)
    }

    fn set_reject_puts(&self, reject_puts: bool) {
        self.reject_puts.store(reject_puts, Ordering::SeqCst);
    }
}

struct VersionMismatchFilesystem<F> {
    inner: F,
}

impl<F> VersionMismatchFilesystem<F> {
    fn new(inner: F) -> Self {
        Self { inner }
    }
}

struct RejectingPutFilesystem<F> {
    inner: F,
    put_calls: AtomicUsize,
}

impl<F> RejectingPutFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            put_calls: AtomicUsize::new(0),
        }
    }

    fn put_calls(&self) -> usize {
        self.put_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl<F> RootFilesystem for RejectingPutFilesystem<F>
where
    F: RootFilesystem,
{
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.put_calls.fetch_add(1, Ordering::SeqCst);
        Err(FilesystemError::PermissionDenied {
            path: ScopedPath::new(path.as_str().to_string()).expect("scoped path"),
            operation: FilesystemOperation::WriteFile,
        })
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

#[async_trait]
impl<F> RootFilesystem for VersionMismatchFilesystem<F>
where
    F: RootFilesystem,
{
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        _entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let expected = match cas {
            CasExpectation::Any => None,
            CasExpectation::Absent => None,
            CasExpectation::Version(version) => Some(version),
        };
        Err(FilesystemError::VersionMismatch {
            path: path.clone(),
            expected,
            found: None,
        })
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

#[async_trait]
impl<F> RootFilesystem for BlockingPutFilesystem<F>
where
    F: RootFilesystem,
{
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if self.block_next_put.swap(false, Ordering::SeqCst) {
            self.put_blocked.store(true, Ordering::SeqCst);
            self.put_started.notify_one();
            self.release_put.notified().await;
            self.put_blocked.store(false, Ordering::SeqCst);
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }
}

#[async_trait]
impl<F> RootFilesystem for BlockingSnapshotPutFilesystem<F>
where
    F: RootFilesystem,
{
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if path == &snapshot_virtual_path() && self.block_snapshot_puts.load(Ordering::SeqCst) {
            self.snapshot_put_blocked.store(true, Ordering::SeqCst);
            self.snapshot_put_started.notify_one();
            while self.block_snapshot_puts.load(Ordering::SeqCst) {
                self.release_snapshot_puts.notified().await;
            }
            self.snapshot_put_blocked.store(false, Ordering::SeqCst);
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

#[async_trait]
impl<F> RootFilesystem for RejectSnapshotGetFilesystem<F>
where
    F: RootFilesystem,
{
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        if path == &snapshot_virtual_path() && self.reject_snapshot_gets.load(Ordering::SeqCst) {
            return Err(FilesystemError::PermissionDenied {
                path: ScopedPath::new(path.as_str().to_string()).expect("scoped path"),
                operation: FilesystemOperation::ReadFile,
            });
        }
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

#[async_trait]
impl<F> RootFilesystem for FirstWaveBlockingPutFilesystem<F>
where
    F: RootFilesystem,
{
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let expected = self.expected_first_wave_puts.load(Ordering::SeqCst);
        if expected > 0 {
            let arrival = self.first_wave_arrivals.fetch_add(1, Ordering::SeqCst) + 1;
            if arrival <= expected {
                if arrival == expected {
                    self.first_wave_ready.notify_one();
                }
                while !self.first_wave_released.load(Ordering::SeqCst) {
                    self.release_first_wave.notified().await;
                }
            }
        }
        if self.reject_puts.load(Ordering::SeqCst) {
            self.version_mismatches.fetch_add(1, Ordering::SeqCst);
            return Err(FilesystemError::VersionMismatch {
                path: path.clone(),
                expected: match cas {
                    CasExpectation::Any => None,
                    CasExpectation::Absent => None,
                    CasExpectation::Version(version) => Some(version),
                },
                found: None,
            });
        }
        let result = self.inner.put(path, entry, cas).await;
        if matches!(result, Err(FilesystemError::VersionMismatch { .. })) {
            self.version_mismatches.fetch_add(1, Ordering::SeqCst);
        }
        result
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let result = self.inner.get(path).await;
        if self.version_mismatches.load(Ordering::SeqCst) > 0
            && !self.mismatch_retry_seen.swap(true, Ordering::SeqCst)
        {
            self.mismatch_retry_ready.notify_waiters();
        }
        result
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }
}

fn turn_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn turn_actor() -> TurnActor {
    TurnActor::new(UserId::new("user1").unwrap())
}

fn submit_request_for(scope: TurnScope, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope,
        actor: turn_actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{idempotency_key}"))
            .unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

fn accepted_run_id(response: &SubmitTurnResponse) -> TurnRunId {
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    *run_id
}

#[tokio::test]
async fn filesystem_turn_state_store_does_not_write_unchanged_idle_runner_snapshot() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);

    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(claimed.is_none());

    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc.with_ymd_and_hms(2026, 5, 27, 0, 12, 0).unwrap(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(recovered.recovered.is_empty());

    let err = backend
        .read_file(&snapshot_virtual_path())
        .await
        .unwrap_err();
    assert!(
        matches!(err, FilesystemError::NotFound { .. }),
        "idle no-op runner polling must not create or rewrite the snapshot: {err:?}"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_heartbeat_updates_lease_without_rewriting_snapshot() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-heartbeat-memory"),
        "idem-fs-heartbeat-memory",
    );
    let response = store
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, run_id);

    let version_after_claim = backend
        .get(&snapshot_virtual_path())
        .await
        .unwrap()
        .expect("snapshot after claim")
        .version;
    let claimed_snapshot = store.persistence_snapshot().await.unwrap();
    let claimed_run = claimed_snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .expect("claimed run");
    let first_heartbeat_at = claimed_run.last_heartbeat_at.expect("heartbeat timestamp");
    let first_expiry = claimed_run.lease_expires_at.expect("lease expiry");

    tokio::time::sleep(Duration::from_millis(5)).await;
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let version_after_heartbeat = backend
        .get(&snapshot_virtual_path())
        .await
        .unwrap()
        .expect("snapshot after heartbeat")
        .version;
    assert_eq!(
        version_after_heartbeat, version_after_claim,
        "heartbeat must refresh the runner lease without rewriting state.json"
    );
    let heartbeat_snapshot = store.persistence_snapshot().await.unwrap();
    let heartbeat_run = heartbeat_snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .expect("heartbeat run");
    assert!(
        heartbeat_run
            .last_heartbeat_at
            .expect("heartbeat timestamp")
            > first_heartbeat_at,
        "heartbeat read model should expose the refreshed memory lease timestamp"
    );
    assert!(
        heartbeat_run.lease_expires_at.expect("lease expiry") > first_expiry,
        "heartbeat read model should expose the refreshed memory lease expiry"
    );
    assert!(
        backend
            .get(&runner_lease_virtual_path(run_id))
            .await
            .unwrap()
            .is_none(),
        "runner leases are memory-backed and must not materialize durable sidecar records"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_heartbeat_seeds_memory_lease_from_snapshot_after_reopen() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-heartbeat-memory-backfill"),
        "idem-fs-heartbeat-memory-backfill",
    );
    let response = store
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    let claimed_snapshot = store.persistence_snapshot().await.unwrap();
    let first_heartbeat_at = claimed_snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .and_then(|record| record.last_heartbeat_at)
        .expect("claimed heartbeat timestamp");

    let reopened = FilesystemTurnStateStore::new(scoped);
    tokio::time::sleep(Duration::from_millis(5)).await;
    reopened
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .expect("heartbeat should lazily seed a missing memory lease from state.json");

    let heartbeat_snapshot = reopened.persistence_snapshot().await.unwrap();
    let heartbeat_run = heartbeat_snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .expect("heartbeat run");
    assert!(
        heartbeat_run
            .last_heartbeat_at
            .expect("heartbeat timestamp")
            > first_heartbeat_at,
        "lazy memory lease backfill must still expose the refreshed heartbeat"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_recover_expired_leases_uses_memory_runner_lease() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-recover-memory-lease"),
        "idem-fs-recover-memory-lease",
    );
    let response = store
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    let claimed_snapshot = store.persistence_snapshot().await.unwrap();
    let first_expiry = claimed_snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .and_then(|record| record.lease_expires_at)
        .expect("claimed lease expiry");

    tokio::time::sleep(Duration::from_millis(5)).await;
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    let heartbeat_snapshot = store.persistence_snapshot().await.unwrap();
    let refreshed_expiry = heartbeat_snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .and_then(|record| record.lease_expires_at)
        .expect("refreshed lease expiry");
    assert!(refreshed_expiry > first_expiry);

    let not_yet_recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: first_expiry + chrono::Duration::milliseconds(1),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(
        not_yet_recovered.recovered.is_empty(),
        "recovery must use memory lease expiry instead of stale state.json expiry"
    );

    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: refreshed_expiry + chrono::Duration::milliseconds(1),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 1);
    assert_eq!(recovered.recovered[0].run_id, run_id);
    assert_eq!(recovered.recovered[0].status, TurnStatus::Failed);
}

#[tokio::test]
async fn filesystem_turn_state_store_complete_run_uses_memory_runner_lease() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-complete-memory-lease"),
        "idem-fs-complete-memory-lease",
    );
    let response = store
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    overwrite_snapshot_lease_expiry(&backend, run_id, Utc::now() - chrono::Duration::seconds(1))
        .await;

    let completed = store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .expect("terminal transition must validate against memory lease metadata");
    assert_eq!(completed.status, TurnStatus::Completed);
}

#[tokio::test]
async fn filesystem_turn_state_store_heartbeat_does_not_write_runner_lease_sidecar() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-heartbeat-memory-no-sidecar"),
        "idem-fs-heartbeat-memory-no-sidecar",
    );
    let response = store
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    let claimed_snapshot = store.persistence_snapshot().await.unwrap();
    let first_heartbeat_at = claimed_snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .and_then(|record| record.last_heartbeat_at)
        .expect("claimed heartbeat timestamp");

    tokio::time::sleep(Duration::from_millis(5)).await;
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .expect("heartbeat should update the memory-backed runner lease");

    let heartbeat_snapshot = store.persistence_snapshot().await.unwrap();
    let heartbeat_at = heartbeat_snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .and_then(|record| record.last_heartbeat_at)
        .expect("heartbeat timestamp");
    assert!(heartbeat_at > first_heartbeat_at);
    assert!(
        backend
            .get(&runner_lease_virtual_path(run_id))
            .await
            .unwrap()
            .is_none(),
        "memory-backed heartbeat must not write a durable runner lease sidecar"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_heartbeat_does_not_read_snapshot() {
    let backend = Arc::new(RejectSnapshotGetFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let response = store
        .submit_turn(
            submit_request_for(
                turn_scope("thread-fs-heartbeat-memory-only"),
                "idem-fs-heartbeat-memory-only",
            ),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, run_id);

    backend.reject_snapshot_gets();
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .expect("heartbeat must use only the memory runner lease");
}

#[tokio::test]
async fn filesystem_turn_state_store_cancel_requested_heartbeat_uses_memory_lease_status() {
    let backend = Arc::new(RejectSnapshotGetFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-heartbeat-cancel-memory"),
        "idem-fs-heartbeat-cancel-memory",
    );
    let response = store
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();

    let cancel = store
        .request_cancel(ironclaw_turns::CancelRunRequest {
            scope: request.scope,
            actor: turn_actor(),
            run_id,
            reason: SanitizedCancelReason::UserRequested,
            idempotency_key: IdempotencyKey::new("idem-fs-heartbeat-cancel-request").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(cancel.status, TurnStatus::CancelRequested);

    backend.reject_snapshot_gets();
    let heartbeat = store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap_err();
    assert_eq!(
        heartbeat,
        TurnError::InvalidTransition {
            from: TurnStatus::CancelRequested,
            to: TurnStatus::Running,
        }
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_heartbeat_succeeds_while_snapshot_put_is_blocked() {
    let backend = Arc::new(BlockingSnapshotPutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(FilesystemTurnStateStore::new(scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-heartbeat-blocked-snapshot"),
        "idem-fs-heartbeat-blocked-snapshot",
    );
    let response = store
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, run_id);

    backend.block_snapshot_puts();
    let blocked_store = Arc::clone(&store);
    let blocked_writer = tokio::spawn(async move {
        let resolver = InMemoryRunProfileResolver::default();
        blocked_store
            .submit_turn(
                submit_request_for(
                    turn_scope("thread-fs-heartbeat-blocked-writer"),
                    "idem-fs-heartbeat-blocked-writer",
                ),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
    });

    tokio::time::timeout(
        Duration::from_secs(1),
        backend.wait_for_blocked_snapshot_put(),
    )
    .await
    .expect("writer should reach the blocked state.json put");

    tokio::time::timeout(
        Duration::from_secs(1),
        store.heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        }),
    )
    .await
    .expect("heartbeat must not wait behind a blocked state.json put")
    .unwrap();

    backend.release_snapshot_puts();
    blocked_writer.await.unwrap().unwrap();
}

fn child_run_request(
    parent_scope: TurnScope,
    parent_run_id: TurnRunId,
    child_scope: TurnScope,
    idempotency_key: &str,
    cap: u32,
) -> SubmitChildRunRequest {
    SubmitChildRunRequest {
        parent_scope,
        parent_run_id,
        child_scope,
        actor: turn_actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{idempotency_key}"))
            .unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap(),
        requested_run_id: Some(TurnRunId::new()),
        spawn_tree_descendant_cap: cap,
    }
}

#[tokio::test]
async fn filesystem_turn_state_store_persists_submit_and_reopens() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(turn_scope("thread-fs-persist"), "idem-fs-persist");
    let response = store
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);

    // Re-construct the store over the same scoped filesystem; the on-disk
    // snapshot must rehydrate the queued run.
    let reopened = FilesystemTurnStateStore::new(scoped);
    let state = reopened
        .get_run_state(GetRunStateRequest {
            scope: request.scope,
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.run_id, run_id);
    assert_eq!(state.status, TurnStatus::Queued);
}

#[tokio::test]
async fn filesystem_turn_state_store_reuses_fresh_snapshot_for_read_only_lookup() {
    let backend = Arc::new(CountingFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(turn_scope("thread-fs-read-cache"), "idem-fs-read-cache");
    let response = store
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);

    backend.reset_get_calls();
    let state = store
        .get_run_state(GetRunStateRequest {
            scope: request.scope,
            run_id,
        })
        .await
        .unwrap();

    assert_eq!(state.run_id, run_id);
    assert_eq!(
        backend.get_calls(),
        0,
        "fresh read-only turn-state lookups should reuse the in-process snapshot cache"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_clears_stale_snapshot_cache_after_version_mismatch() {
    let backend = Arc::new(FirstWaveBlockingPutFilesystem::new(
        CountingFilesystem::new(engine_filesystem()),
    ));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(FilesystemTurnStateStore::new(Arc::clone(&scoped)));
    let external_store = FilesystemTurnStateStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let seed_scope = turn_scope("thread-fs-vm-cache-seed");
    let seed_request = submit_request_for(seed_scope, "idem-fs-vm-cache-seed");
    store
        .submit_turn(seed_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();

    let external_request = submit_request_for(
        turn_scope("thread-fs-vm-cache-external"),
        "idem-fs-vm-cache-ext",
    );
    external_store
        .submit_turn(external_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();

    backend.block_first_put_wave(1);

    let raced_scope = turn_scope("thread-fs-vm-cache-raced");
    let raced_request = submit_request_for(raced_scope, "idem-fs-vm-cache-raced");
    let raced_store = Arc::clone(&store);
    let raced = tokio::spawn(async move {
        let resolver = InMemoryRunProfileResolver::default();
        raced_store
            .submit_turn(raced_request, &AllowAllTurnAdmissionPolicy, &resolver)
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), backend.wait_for_first_wave())
        .await
        .expect("first version-mismatching writer should block on its initial put");

    let competing_scope = turn_scope("thread-fs-vm-cache-competing");
    let competing_request =
        submit_request_for(competing_scope.clone(), "idem-fs-vm-cache-competing");
    let competing_response = external_store
        .submit_turn(competing_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let competing_run_id = accepted_run_id(&competing_response);

    backend.set_reject_puts(true);
    backend.release_first_wave();

    tokio::time::timeout(Duration::from_secs(1), async {
        while backend.version_mismatches() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first writer should observe a version mismatch before retrying");

    tokio::time::timeout(
        Duration::from_secs(1),
        backend.wait_for_mismatch_retry_read(),
    )
    .await
    .expect("store should retry with a fresh snapshot after clearing stale cache");

    raced.abort();
    let _ = raced.await;

    backend.inner.reset_get_calls();
    let state = store
        .get_run_state(GetRunStateRequest {
            scope: competing_scope,
            run_id: competing_run_id,
        })
        .await
        .unwrap();

    assert_eq!(state.run_id, competing_run_id);
    assert_eq!(
        backend.inner.get_calls(),
        1,
        "version mismatch must clear stale snapshot cache before retry/backoff"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_snapshot_reads_overlap_apply_write() {
    let backend = Arc::new(BlockingPutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(FilesystemTurnStateStore::new(Arc::clone(&scoped)));
    let resolver = InMemoryRunProfileResolver::default();

    let existing_request = submit_request_for(turn_scope("thread-fs-overlap-a"), "idem-overlap-a");
    let existing_response = store
        .submit_turn(
            existing_request.clone(),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let existing_run_id = accepted_run_id(&existing_response);

    backend.block_next_put();
    let writer_store = Arc::clone(&store);
    let writer = tokio::spawn(async move {
        let resolver = InMemoryRunProfileResolver::default();
        writer_store
            .submit_turn(
                submit_request_for(turn_scope("thread-fs-overlap-b"), "idem-overlap-b"),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), backend.wait_for_blocked_put())
        .await
        .expect("writer should reach the delayed snapshot write");

    let read = tokio::time::timeout(
        Duration::from_millis(100),
        store.get_run_state(GetRunStateRequest {
            scope: existing_request.scope,
            run_id: existing_run_id,
        }),
    )
    .await
    .expect("snapshot read must not wait behind a blocked writer")
    .unwrap();
    assert_eq!(read.run_id, existing_run_id);
    assert_eq!(read.status, TurnStatus::Queued);

    backend.release_blocked_put();
    writer.await.unwrap().unwrap();
}

#[tokio::test]
async fn filesystem_turn_state_store_cas_writers_overlap_blocked_snapshot_write() {
    let backend = Arc::new(BlockingPutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(FilesystemTurnStateStore::new(Arc::clone(&scoped)));
    let resolver = InMemoryRunProfileResolver::default();

    store
        .submit_turn(
            submit_request_for(turn_scope("thread-fs-overlap-seed"), "idem-overlap-seed"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();

    backend.block_next_put();
    let blocked_store = Arc::clone(&store);
    let blocked_writer = tokio::spawn(async move {
        let resolver = InMemoryRunProfileResolver::default();
        blocked_store
            .submit_turn(
                submit_request_for(
                    turn_scope("thread-fs-overlap-blocked"),
                    "idem-overlap-blocked",
                ),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), backend.wait_for_blocked_put())
        .await
        .expect("first writer should reach the delayed snapshot write");

    let next_store = Arc::clone(&store);
    let next_writer = tokio::spawn(async move {
        let resolver = InMemoryRunProfileResolver::default();
        next_store
            .submit_turn(
                submit_request_for(turn_scope("thread-fs-overlap-next"), "idem-overlap-next"),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
    });

    let next_response = tokio::time::timeout(Duration::from_secs(1), next_writer)
        .await
        .expect("CAS-backed writer must not queue behind a blocked writer")
        .unwrap()
        .unwrap();
    let next_run_id = accepted_run_id(&next_response);

    backend.release_blocked_put();
    let blocked_response = blocked_writer.await.unwrap().unwrap();
    let blocked_run_id = accepted_run_id(&blocked_response);

    let blocked_state = store
        .get_run_state(GetRunStateRequest {
            scope: turn_scope("thread-fs-overlap-blocked"),
            run_id: blocked_run_id,
        })
        .await
        .unwrap();
    let next_state = store
        .get_run_state(GetRunStateRequest {
            scope: turn_scope("thread-fs-overlap-next"),
            run_id: next_run_id,
        })
        .await
        .unwrap();

    assert_eq!(blocked_state.run_id, blocked_run_id);
    assert_eq!(blocked_state.status, TurnStatus::Queued);
    assert_eq!(next_state.run_id, next_run_id);
    assert_eq!(next_state.status, TurnStatus::Queued);
}

#[tokio::test]
async fn filesystem_turn_state_store_cas_storm_preserves_all_submits() {
    const CONCURRENT_SUBMITS: usize = 24;

    let backend = Arc::new(FirstWaveBlockingPutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(FilesystemTurnStateStore::new(Arc::clone(&scoped)));
    let resolver = InMemoryRunProfileResolver::default();

    store
        .submit_turn(
            submit_request_for(
                turn_scope("thread-fs-cas-storm-seed"),
                "idem-cas-storm-seed",
            ),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();

    backend.block_first_put_wave(CONCURRENT_SUBMITS);
    let start = Arc::new(tokio::sync::Barrier::new(CONCURRENT_SUBMITS + 1));

    let mut tasks = Vec::new();
    for index in 0..CONCURRENT_SUBMITS {
        let task_store = Arc::clone(&store);
        let task_start = Arc::clone(&start);
        tasks.push(tokio::spawn(async move {
            task_start.wait().await;
            let resolver = InMemoryRunProfileResolver::default();
            let request = submit_request_for(
                turn_scope(&format!("thread-fs-cas-storm-{index}")),
                &format!("idem-cas-storm-{index}"),
            );
            let response = task_store
                .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
                .await?;
            Ok::<_, TurnError>((request.scope, accepted_run_id(&response)))
        }));
    }

    start.wait().await;
    tokio::time::timeout(Duration::from_secs(1), backend.wait_for_first_wave())
        .await
        .expect("all first-wave writers must reach the CAS write together");
    backend.release_first_wave();

    let mut accepted = Vec::new();
    for task in tasks {
        accepted.push(
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .expect("concurrent submit must not exhaust CAS retries")
                .unwrap()
                .unwrap(),
        );
    }
    assert!(
        backend.version_mismatches() > 0,
        "test must exercise real CAS retry path, not just serialized writes"
    );

    for (scope, run_id) in accepted {
        let state = store
            .get_run_state(GetRunStateRequest { scope, run_id })
            .await
            .unwrap();
        assert_eq!(state.run_id, run_id);
        assert_eq!(state.status, TurnStatus::Queued);
    }
}

#[tokio::test]
async fn filesystem_turn_state_store_timed_out_apply_does_not_wedge_subsequent_writes() {
    let backend = Arc::new(BlockingPutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(
        FilesystemTurnStateStore::new(Arc::clone(&scoped))
            .with_apply_timeout(Duration::from_millis(100)),
    );
    let resolver = InMemoryRunProfileResolver::default();

    store
        .submit_turn(
            submit_request_for(turn_scope("thread-fs-timeout-seed"), "idem-timeout-seed"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();

    backend.block_next_put();
    let blocked_store = Arc::clone(&store);
    let blocked_writer = tokio::spawn(async move {
        let resolver = InMemoryRunProfileResolver::default();
        blocked_store
            .submit_turn(
                submit_request_for(
                    turn_scope("thread-fs-timeout-blocked"),
                    "idem-timeout-blocked",
                ),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), backend.wait_for_blocked_put())
        .await
        .expect("first writer should reach the delayed snapshot write");

    let blocked_result = tokio::time::timeout(Duration::from_secs(1), blocked_writer)
        .await
        .expect("blocked snapshot write must hit the bounded apply timeout")
        .unwrap();
    assert!(
        matches!(blocked_result, Err(TurnError::Unavailable { reason }) if reason == "turn state filesystem apply timed out")
    );

    backend.release_blocked_put();

    let next_request =
        submit_request_for(turn_scope("thread-fs-timeout-next"), "idem-timeout-next");
    let next_response = tokio::time::timeout(
        Duration::from_secs(1),
        store.submit_turn(
            next_request.clone(),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        ),
    )
    .await
    .expect("turn state must be usable after the timed-out writer")
    .unwrap();
    let next_run_id = accepted_run_id(&next_response);
    let next_state = store
        .get_run_state(GetRunStateRequest {
            scope: next_request.scope,
            run_id: next_run_id,
        })
        .await
        .unwrap();
    assert_eq!(next_state.status, TurnStatus::Queued);
}

#[tokio::test]
async fn filesystem_turn_state_store_timed_out_claim_does_not_wedge_scheduler_writes() {
    let backend = Arc::new(BlockingPutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(
        FilesystemTurnStateStore::new(Arc::clone(&scoped))
            .with_apply_timeout(Duration::from_millis(100)),
    );
    let resolver = InMemoryRunProfileResolver::default();
    let request = submit_request_for(turn_scope("thread-fs-timeout-claim"), "idem-timeout-claim");
    let response = store
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);

    backend.block_next_put();
    let blocked_store = Arc::clone(&store);
    let blocked_claim = tokio::spawn(async move {
        blocked_store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: None,
            })
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), backend.wait_for_blocked_put())
        .await
        .expect("scheduler claim should reach the delayed snapshot write");

    let blocked_result = tokio::time::timeout(Duration::from_secs(1), blocked_claim)
        .await
        .expect("blocked scheduler claim must hit the bounded apply timeout")
        .unwrap();
    assert!(
        matches!(blocked_result, Err(TurnError::Unavailable { reason }) if reason == "turn state filesystem apply timed out")
    );

    backend.release_blocked_put();

    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap()
        .expect("queued run should still be claimable after timed-out claim");
    assert_eq!(claimed.state.run_id, run_id);
    assert_eq!(claimed.state.scope, request.scope);
}

#[tokio::test]
async fn filesystem_turn_state_store_returns_unavailable_after_persistent_version_mismatches() {
    let backend = Arc::new(VersionMismatchFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let error = match store
        .submit_turn(
            submit_request_for(turn_scope("thread-fs-cas-exhausted"), "idem-cas-exhausted"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
    {
        Ok(_) => panic!("persistent version mismatch should exhaust CAS retries"),
        Err(error) => error,
    };

    assert!(
        matches!(error, TurnError::Unavailable { reason } if reason == "turn state filesystem CAS retries exhausted")
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_returns_unavailable_on_non_version_mismatch_put_error() {
    let backend = Arc::new(RejectingPutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let error = match store
        .submit_turn(
            submit_request_for(turn_scope("thread-fs-put-error"), "idem-put-error"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
    {
        Ok(_) => panic!("put failure should surface as unavailable"),
        Err(error) => error,
    };

    assert!(matches!(error, TurnError::Unavailable { .. }));
    assert_eq!(
        backend.put_calls(),
        1,
        "non-version-mismatch put errors must not retry"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_rejects_byte_only_backend_before_snapshot_write() {
    let backend = Arc::new(byte_only_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let request = submit_request_for(turn_scope("thread-fs-byte-only"), "idem-byte-only");

    let error = match store
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
    {
        Ok(_) => panic!("byte-only backend must not accept turn-state snapshots"),
        Err(error) => error,
    };
    assert!(
        matches!(error, TurnError::Unavailable { reason } if reason == "turn state filesystem backend must support versioned CAS")
    );
    assert!(
        backend
            .get(&snapshot_virtual_path())
            .await
            .unwrap()
            .is_none(),
        "non-CAS backend rejection must happen before writing state.json"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_hides_records_from_other_tenants_via_mount_view() {
    // Regression for the ScopedFilesystem migration: two stores share one
    // underlying RootFilesystem but each is constructed with a MountView
    // whose `/turns` alias resolves to a different tenant-scoped VirtualPath
    // subtree. Writing the same (thread, idempotency_key) on tenant A's
    // store must NOT make the snapshot visible from tenant B's store. The
    // structural fix routes every op through ScopedFilesystem; two
    // MountViews over the same backend cannot see each other's snapshots.
    let backend = Arc::new(engine_filesystem());
    let scoped_a = scoped_turns_fs_at(Arc::clone(&backend), "tenant-a", "alice");
    let scoped_b = scoped_turns_fs_at(Arc::clone(&backend), "tenant-b", "alice");

    let store_a = FilesystemTurnStateStore::new(Arc::clone(&scoped_a));
    let store_b = FilesystemTurnStateStore::new(Arc::clone(&scoped_b));
    let resolver = InMemoryRunProfileResolver::default();

    let scope_a = TurnScope::new(
        TenantId::new("tenant-a").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-cross-tenant").unwrap(),
    );
    let scope_b = TurnScope::new(
        TenantId::new("tenant-b").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-cross-tenant").unwrap(),
    );

    let response_a = store_a
        .submit_turn(
            submit_request_for(scope_a.clone(), "idem-cross-tenant"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id_a = accepted_run_id(&response_a);

    // Tenant A sees its own run.
    let state_a = store_a
        .get_run_state(GetRunStateRequest {
            scope: scope_a.clone(),
            run_id: run_id_a,
        })
        .await
        .unwrap();
    assert_eq!(state_a.run_id, run_id_a);

    // Tenant B does NOT see tenant A's run id, despite the identical
    // (thread, idempotency_key). The mount target prefix in tenant B's
    // ScopedFilesystem resolves to a disjoint VirtualPath, so the snapshot
    // is absent and `get_run_state` reports `ScopeNotFound`.
    let err = store_b
        .get_run_state(GetRunStateRequest {
            scope: scope_b.clone(),
            run_id: run_id_a,
        })
        .await
        .expect_err("tenant B must NOT see tenant A's run (cross-tenant snapshot leak)");
    assert!(matches!(err, ironclaw_turns::TurnError::ScopeNotFound));

    // Tenant B can independently submit with the same idempotency_key and
    // observe its own run id, distinct from tenant A's.
    let response_b = store_b
        .submit_turn(
            submit_request_for(scope_b.clone(), "idem-cross-tenant"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id_b = accepted_run_id(&response_b);
    assert_ne!(
        run_id_a, run_id_b,
        "each tenant snapshot must mint its own run id; collision implies leakage"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_persists_lineage_and_tree_reservations() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let parent_scope = turn_scope("thread-fs-parent");
    let parent = accepted_run_id(
        &store
            .submit_turn(
                submit_request_for(parent_scope.clone(), "idem-fs-parent"),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
            .unwrap(),
    );

    let child_scope = turn_scope("thread-fs-child");
    let child_run_id = accepted_run_id(
        &store
            .submit_child_turn(
                child_run_request(
                    parent_scope.clone(),
                    parent,
                    child_scope.clone(),
                    "idem-fs-child",
                    3,
                ),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
            .unwrap(),
    );

    let child_b_scope = turn_scope("thread-fs-child-b");
    let reservation = store
        .reserve_tree_descendants(&child_scope, parent, 1, 3)
        .await
        .unwrap();
    assert_eq!(reservation.descendant_count, 2);
    assert!(matches!(
        store
            .reserve_tree_descendants(&child_b_scope, parent, 2, 3)
            .await,
        Err(TurnError::CapacityExceeded { .. })
    ));
    store
        .release_tree_descendants(&child_b_scope, parent, 1)
        .await
        .unwrap();

    let reopened = FilesystemTurnStateStore::new(scoped);
    let children = reopened.children_of(&parent_scope, parent).await.unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].run_id, child_run_id);
    assert_eq!(children[0].parent_run_id, Some(parent));
    assert_eq!(
        reopened
            .get_run_record(&child_scope, child_run_id)
            .await
            .unwrap()
            .unwrap()
            .spawn_tree_root_run_id,
        Some(parent)
    );
    assert_eq!(
        reopened
            .reserve_tree_descendants(&child_b_scope, parent, 1, 3)
            .await
            .unwrap()
            .descendant_count,
        2
    );
}

#[tokio::test]
async fn filesystem_spawn_tree_reads_are_scope_checked() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let parent_scope = turn_scope("thread-fs-scope-parent");
    let parent = accepted_run_id(
        &store
            .submit_turn(
                submit_request_for(parent_scope.clone(), "idem-fs-scope-parent"),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
            .unwrap(),
    );
    let child_scope = turn_scope("thread-fs-scope-child");
    let child = accepted_run_id(
        &store
            .submit_child_turn(
                child_run_request(
                    parent_scope.clone(),
                    parent,
                    child_scope.clone(),
                    "idem-fs-scope-child",
                    4,
                ),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
            .unwrap(),
    );

    let versioned = backend
        .get(&snapshot_virtual_path())
        .await
        .unwrap()
        .expect("snapshot after child submit");
    let mut snapshot: TurnPersistenceSnapshot =
        serde_json::from_slice(&versioned.entry.body).unwrap();
    let mut shadow_parent = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == parent && record.scope == parent_scope)
        .expect("parent run in snapshot")
        .clone();
    shadow_parent.scope = TurnScope::new(
        TenantId::new("shadow-tenant").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-fs-scope-shadow").unwrap(),
    );
    snapshot.runs.insert(0, shadow_parent);
    let mut entry = versioned.entry;
    entry.body = serde_json::to_vec_pretty(&snapshot).unwrap();
    backend
        .put(
            &snapshot_virtual_path(),
            entry,
            CasExpectation::Version(versioned.version),
        )
        .await
        .unwrap();

    let reopened = FilesystemTurnStateStore::new(scoped);
    assert_eq!(
        reopened
            .children_of(&parent_scope, parent)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        reopened
            .children_of(&child_scope, parent)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        reopened
            .children_of(&parent_scope, TurnRunId::new())
            .await
            .unwrap()
            .is_empty()
    );

    let foreign_scope = TurnScope::new(
        TenantId::new("foreign-tenant").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-fs-scope-parent").unwrap(),
    );
    assert!(
        reopened
            .children_of(&foreign_scope, parent)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        reopened
            .get_run_record(&foreign_scope, parent)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        reopened
            .get_run_record(&parent_scope, child)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reopened
            .get_run_record(&child_scope, child)
            .await
            .unwrap()
            .unwrap()
            .run_id,
        child
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_persists_product_context_through_snapshot_round_trip() {
    // Regression for item-6 persistence: product_context must survive the
    // snapshot write → read cycle so the model-visible runtime context
    // section renders the correct origin after a restart.
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    // Submit with a non-None product context.
    let mut request = submit_request_for(turn_scope("thread-origin-rt"), "idem-origin-rt");
    let expected_ctx = ProductTurnContext::new(
        TurnOriginKind::Inbound,
        None,
        Some(RunOriginAdapter::new("telegram_v2").unwrap()),
        TurnOwner::Personal {
            user: ironclaw_host_api::UserId::new("user-rt").unwrap(),
        },
    );
    request.product_context = Some(expected_ctx.clone());
    let response = store
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);

    // Re-open the store — this forces a full deserialize from the snapshot.
    let reopened = FilesystemTurnStateStore::new(scoped);
    let state = reopened
        .get_run_state(GetRunStateRequest {
            scope: request.scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(
        state.product_context,
        Some(expected_ctx),
        "product_context must survive snapshot round-trip"
    );

    // Also verify that None product_context is preserved as None (separate thread to
    // avoid ThreadBusy on the already-queued run above).
    let mut request_none =
        submit_request_for(turn_scope("thread-origin-rt-none"), "idem-origin-none");
    request_none.product_context = None;
    let response_none = reopened
        .submit_turn(
            request_none.clone(),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id_none = accepted_run_id(&response_none);

    let reopened2 = FilesystemTurnStateStore::new(scoped_turns_fs(Arc::clone(&backend)));
    let state_none = reopened2
        .get_run_state(GetRunStateRequest {
            scope: request_none.scope,
            run_id: run_id_none,
        })
        .await
        .unwrap();
    assert!(
        state_none.product_context.is_none(),
        "None product_context must remain None after snapshot round-trip"
    );
}
