// arch-exempt: large_file, filesystem turn-state contract suite decomposition, plan #5662
//! Contract tests for [`FilesystemTurnStateStore`] against a
//! [`ScopedFilesystem`] over a CAS-capable filesystem backend. The persistent
//! shape is a lower-churn `/turns/state.json` snapshot; active runner leases
//! are memory-backed and fall back to the snapshot after restart.

use std::{
    sync::{
        Arc, Condvar, Mutex as StdMutex,
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
    SeqNo, StorageClass, VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, HostPath, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, ScopedPath,
    TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_turns::{
    AcceptedMessageRef, AdmissionRejection, AllowAllTurnAdmissionPolicy, BlockedReason,
    CheckpointSchemaId, FilesystemTurnStateRowStore, FilesystemTurnStateStore, GateRef,
    GetLoopCheckpointRequest, GetRunStateRequest, IdempotencyKey, InMemoryRunProfileResolver,
    InMemoryTurnStateStoreLimits, LoopCheckpointStore, LoopExitMapping, ProductTurnContext,
    PutLoopCheckpointRequest, ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest,
    RunOriginAdapter, RunProfileRequest, RunProfileVersion, SanitizedCancelReason,
    SanitizedFailure, SourceBindingRef, SubmitChildRunRequest, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnAdmissionPolicy, TurnCheckpointId, TurnError, TurnEventKind,
    TurnEventProjectionSource, TurnId, TurnLeaseToken, TurnOriginKind, TurnOwner,
    TurnPersistenceSnapshot, TurnRunId, TurnRunnerId, TurnScope, TurnSpawnTreeStateStore,
    TurnStateStore, TurnStatus,
    run_profile::{LoopCheckpointKind, LoopCheckpointStateRef},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, ClaimRunRequest, CompleteRunRequest,
        FailRunRequest, HeartbeatRequest, RecoverExpiredLeasesRequest, TurnRunTransitionPort,
        TurnRunnerOutcome,
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

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        self.inner.append(path, payload).await
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        self.inner.append_batch(path, payloads).await
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail(path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail_bounded(path, from, max_records).await
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

fn strict_row_store<F>(scoped: Arc<ScopedFilesystem<F>>) -> FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem + 'static,
{
    FilesystemTurnStateRowStore::new(scoped).with_preappend_row_reservations()
}

async fn retry_get_run_state<F>(
    store: &FilesystemTurnStateRowStore<F>,
    scope: TurnScope,
    run_id: TurnRunId,
) -> ironclaw_turns::TurnRunState
where
    F: RootFilesystem,
{
    let mut last_error = None;
    for _ in 0..8 {
        match store
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
        {
            Ok(state) => return state,
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    panic!("run state replay did not recover after materialization retry: {last_error:?}");
}

async fn retry_read_turn_events<F>(
    store: &FilesystemTurnStateRowStore<F>,
    scope: &TurnScope,
) -> ironclaw_turns::TurnEventPage
where
    F: RootFilesystem,
{
    let mut last_error = None;
    for _ in 0..8 {
        match store.read_turn_events_after(scope, None, None, 100).await {
            Ok(events) => return events,
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    panic!("event replay did not recover after materialization retry: {last_error:?}");
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

fn row_delta_log_virtual_path() -> VirtualPath {
    VirtualPath::new("/engine/tenants/test-tenant/users/test-user/turns/rows/v1/deltas/log")
        .unwrap()
}

fn row_run_virtual_path(run_id: TurnRunId) -> VirtualPath {
    VirtualPath::new(format!(
        "/engine/tenants/test-tenant/users/test-user/turns/rows/v1/runs/{run_id}.json"
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

struct BlockingAppendFilesystem<F> {
    inner: F,
    block_next_append: AtomicBool,
    append_blocked: AtomicBool,
    append_started: tokio::sync::Notify,
    release_append: tokio::sync::Notify,
}

impl<F> BlockingAppendFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            block_next_append: AtomicBool::new(false),
            append_blocked: AtomicBool::new(false),
            append_started: tokio::sync::Notify::new(),
            release_append: tokio::sync::Notify::new(),
        }
    }

    fn block_next_append(&self) {
        self.block_next_append.store(true, Ordering::SeqCst);
    }

    async fn wait_for_blocked_append(&self) {
        while !self.append_blocked.load(Ordering::SeqCst) {
            self.append_started.notified().await;
        }
    }

    fn release_blocked_append(&self) {
        self.release_append.notify_one();
    }

    async fn maybe_block_append(&self) {
        if self.block_next_append.swap(false, Ordering::SeqCst) {
            self.append_blocked.store(true, Ordering::SeqCst);
            self.append_started.notify_one();
            self.release_append.notified().await;
            self.append_blocked.store(false, Ordering::SeqCst);
        }
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

struct BlockingAdmissionPolicy {
    state: StdMutex<BlockingAdmissionState>,
    entered: Condvar,
    released: Condvar,
}

struct BlockingAdmissionState {
    entered: bool,
    released: bool,
}

impl BlockingAdmissionPolicy {
    fn new() -> Self {
        Self {
            state: StdMutex::new(BlockingAdmissionState {
                entered: false,
                released: false,
            }),
            entered: Condvar::new(),
            released: Condvar::new(),
        }
    }

    fn wait_until_entered(&self) {
        let mut state = self.state.lock().expect("blocking admission mutex");
        while !state.entered {
            state = self
                .entered
                .wait(state)
                .expect("blocking admission condvar");
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().expect("blocking admission mutex");
        state.released = true;
        self.released.notify_all();
    }
}

impl TurnAdmissionPolicy for BlockingAdmissionPolicy {
    fn check_submit(&self, _request: &SubmitTurnRequest) -> Result<(), AdmissionRejection> {
        let mut state = self.state.lock().expect("blocking admission mutex");
        state.entered = true;
        self.entered.notify_all();
        while !state.released {
            state = self
                .released
                .wait(state)
                .expect("blocking admission condvar");
        }
        Ok(())
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

struct RejectingAppendFilesystem<F> {
    inner: F,
    append_calls: AtomicUsize,
}

struct FailOncePutFilesystem<F> {
    inner: F,
    fail_next_put: AtomicBool,
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

impl<F> RejectingAppendFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            append_calls: AtomicUsize::new(0),
        }
    }

    fn append_calls(&self) -> usize {
        self.append_calls.load(Ordering::SeqCst)
    }
}

impl<F> FailOncePutFilesystem<F> {
    fn new(inner: F) -> Self {
        Self {
            inner,
            fail_next_put: AtomicBool::new(true),
        }
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

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        self.inner.append(path, payload).await
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        self.inner.append_batch(path, payloads).await
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail(path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail_bounded(path, from, max_records).await
    }
}

#[async_trait]
impl<F> RootFilesystem for RejectingAppendFilesystem<F>
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

    async fn append(
        &self,
        path: &VirtualPath,
        _payload: Vec<u8>,
    ) -> Result<SeqNo, FilesystemError> {
        self.append_calls.fetch_add(1, Ordering::SeqCst);
        Err(FilesystemError::PermissionDenied {
            path: ScopedPath::new(path.as_str().to_string()).expect("scoped path"),
            operation: FilesystemOperation::Append,
        })
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        _payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        self.append_calls.fetch_add(1, Ordering::SeqCst);
        Err(FilesystemError::PermissionDenied {
            path: ScopedPath::new(path.as_str().to_string()).expect("scoped path"),
            operation: FilesystemOperation::Append,
        })
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail(path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail_bounded(path, from, max_records).await
    }
}

#[async_trait]
impl<F> RootFilesystem for FailOncePutFilesystem<F>
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
        if path.as_str().ends_with("/turns/rows/v1/meta/state.json")
            && self.fail_next_put.swap(false, Ordering::SeqCst)
        {
            return Err(FilesystemError::PermissionDenied {
                path: ScopedPath::new(path.as_str().to_string()).expect("scoped path"),
                operation: FilesystemOperation::WriteFile,
            });
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

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        self.inner.append(path, payload).await
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        self.inner.append_batch(path, payloads).await
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail(path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail_bounded(path, from, max_records).await
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

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.delete(path).await
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        self.inner.append(path, payload).await
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        self.inner.append_batch(path, payloads).await
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail(path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail_bounded(path, from, max_records).await
    }
}

#[async_trait]
impl<F> RootFilesystem for BlockingAppendFilesystem<F>
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

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        self.maybe_block_append().await;
        self.inner.append(path, payload).await
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        self.maybe_block_append().await;
        self.inner.append_batch(path, payloads).await
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail(path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<ironclaw_filesystem::EventRecord>, FilesystemError> {
        self.inner.tail_bounded(path, from, max_records).await
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

fn accepted_turn_id(response: &SubmitTurnResponse) -> TurnId {
    let SubmitTurnResponse::Accepted { turn_id, .. } = response;
    *turn_id
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
async fn filesystem_turn_state_row_store_persists_rows_without_state_blob() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(turn_scope("thread-fs-row-persist"), "idem-fs-row-persist");
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
            scope_filter: Some(request.scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.status, TurnStatus::Running);
    let checkpoint_id = TurnCheckpointId::new();
    let gate_ref = GateRef::new("gate-fs-row-block").unwrap();
    let blocked = store
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id,
            state_ref: LoopCheckpointStateRef::new("checkpoint:fs-row-block").unwrap(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();
    assert_eq!(blocked.status, TurnStatus::BlockedApproval);
    assert_eq!(blocked.checkpoint_id, Some(checkpoint_id));

    let reopened_blocked = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let blocked_state = reopened_blocked
        .get_run_state(GetRunStateRequest {
            scope: request.scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(blocked_state.status, TurnStatus::BlockedApproval);
    let blocked_snapshot = reopened_blocked.persistence_snapshot().await.unwrap();
    assert!(
        blocked_snapshot
            .checkpoints
            .iter()
            .any(|record| record.checkpoint_id == checkpoint_id),
        "row store must persist block-created checkpoints as row deltas"
    );

    reopened_blocked
        .resume_turn(ResumeTurnRequest {
            scope: request.scope.clone(),
            actor: turn_actor(),
            run_id,
            gate_resolution_ref: gate_ref,
            source_binding_ref: SourceBindingRef::new("source-resume").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-resume").unwrap(),
            idempotency_key: IdempotencyKey::new("idem-fs-row-resume").unwrap(),
            precondition: ResumeTurnPrecondition::BlockedApprovalGate,
            resume_disposition: None,
        })
        .await
        .unwrap();
    let resume_snapshot = reopened_blocked.persistence_snapshot().await.unwrap();
    assert!(
        resume_snapshot
            .idempotency_records
            .iter()
            .any(|record| record.operation == ironclaw_turns::TurnIdempotencyOperationKind::Resume),
        "row store must persist resume idempotency as a targeted delta"
    );
    let (runner_id, lease_token) = {
        let runner_id = TurnRunnerId::new();
        let lease_token = TurnLeaseToken::new();
        reopened_blocked
            .claim_next_run(ClaimRunRequest {
                runner_id,
                lease_token,
                scope_filter: None,
            })
            .await
            .unwrap()
            .unwrap();
        (runner_id, lease_token)
    };
    reopened_blocked
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    assert!(
        backend
            .get(&snapshot_virtual_path())
            .await
            .unwrap()
            .is_none(),
        "row store must not write the blob-shaped state.json snapshot"
    );
    assert!(
        !backend
            .tail(&row_delta_log_virtual_path(), SeqNo::ZERO)
            .await
            .unwrap()
            .is_empty(),
        "row store should persist typed transition deltas in the append log"
    );

    let reopened = FilesystemTurnStateRowStore::new(scoped);
    let state = reopened
        .get_run_state(GetRunStateRequest {
            scope: request.scope,
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::Completed);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_rejects_stale_cross_store_active_lock_create() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let first_store = strict_row_store(Arc::clone(&scoped));
    let second_store = strict_row_store(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let policy = AllowAllTurnAdmissionPolicy;
    let scope = turn_scope("thread-fs-row-stale-active-lock");

    first_store.persistence_snapshot().await.unwrap();
    second_store.persistence_snapshot().await.unwrap();

    let first = first_store.submit_turn(
        submit_request_for(scope.clone(), "idem-row-stale-active-lock-a"),
        &policy,
        &resolver,
    );
    let second = second_store.submit_turn(
        submit_request_for(scope, "idem-row-stale-active-lock-b"),
        &policy,
        &resolver,
    );
    let (first, second) = tokio::join!(first, second);
    let successes = [first.as_ref(), second.as_ref()]
        .into_iter()
        .filter(|result| result.is_ok())
        .count();
    let conflicts = [first, second]
        .into_iter()
        .filter(|result| matches!(result, Err(TurnError::Conflict { .. })))
        .count();

    assert_eq!(successes, 1);
    assert_eq!(conflicts, 1);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_rejects_stale_cross_store_claim() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let first_store = strict_row_store(Arc::clone(&scoped));
    let second_store = strict_row_store(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let policy = AllowAllTurnAdmissionPolicy;
    let scope = turn_scope("thread-fs-row-stale-claim");

    let submitted = first_store
        .submit_turn(
            submit_request_for(scope, "idem-row-stale-claim"),
            &policy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id = accepted_run_id(&submitted);

    first_store.persistence_snapshot().await.unwrap();
    second_store.persistence_snapshot().await.unwrap();

    let first = first_store.claim_next_run(ClaimRunRequest {
        runner_id: TurnRunnerId::new(),
        lease_token: TurnLeaseToken::new(),
        scope_filter: None,
    });
    let second = second_store.claim_next_run(ClaimRunRequest {
        runner_id: TurnRunnerId::new(),
        lease_token: TurnLeaseToken::new(),
        scope_filter: None,
    });
    let (first, second) = tokio::join!(first, second);
    let successes = [&first, &second]
        .into_iter()
        .filter(|result| matches!(result, Ok(Some(claimed)) if claimed.state.run_id == run_id))
        .count();
    let conflicts = [first, second]
        .into_iter()
        .filter(|result| matches!(result, Err(TurnError::Conflict { .. })))
        .count();

    assert_eq!(successes, 1);
    assert_eq!(conflicts, 1);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_rejects_preappend_cross_store_claim() {
    let backend = Arc::new(BlockingAppendFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let first_store = Arc::new(strict_row_store(Arc::clone(&scoped)));
    let resolver = InMemoryRunProfileResolver::default();
    let policy = AllowAllTurnAdmissionPolicy;
    let scope = turn_scope("thread-fs-row-preappend-claim");

    let submitted = first_store
        .submit_turn(
            submit_request_for(scope.clone(), "idem-row-preappend-claim"),
            &policy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id = accepted_run_id(&submitted);

    backend.block_next_append();
    let first_claim = {
        let first_store = Arc::clone(&first_store);
        tokio::spawn(async move {
            first_store
                .claim_next_run(ClaimRunRequest {
                    runner_id: TurnRunnerId::new(),
                    lease_token: TurnLeaseToken::new(),
                    scope_filter: None,
                })
                .await
        })
    };
    backend.wait_for_blocked_append().await;

    let second_store = strict_row_store(Arc::clone(&scoped));
    let preappend_snapshot = second_store.persistence_snapshot().await.unwrap();
    assert!(
        preappend_snapshot
            .runs
            .iter()
            .any(|record| record.run_id == run_id && record.status == TurnStatus::Running)
    );
    assert!(
        preappend_snapshot
            .active_locks
            .iter()
            .any(|record| record.run_id == run_id && record.status == TurnStatus::Running)
    );
    let second = second_store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await;
    assert!(
        matches!(second, Ok(None) | Err(TurnError::Conflict { .. })),
        "fresh store must not double-claim a run while another claim's append is blocked"
    );

    backend.release_blocked_append();
    let first = first_claim
        .await
        .expect("first claim task joins")
        .unwrap()
        .expect("first claim succeeds");
    assert_eq!(first.state.run_id, run_id);
    assert_eq!(first.state.status, TurnStatus::Running);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_refreshes_stale_active_lock_cache_before_submit() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let first_store = strict_row_store(Arc::clone(&scoped));
    let second_store = strict_row_store(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let policy = AllowAllTurnAdmissionPolicy;
    let scope = turn_scope("thread-fs-row-stale-active-lock-cache");
    let first_run_id = TurnRunId::new();
    let second_run_id = TurnRunId::new();
    let mut first_request = submit_request_for(scope.clone(), "idem-row-stale-cache-a");
    first_request.requested_run_id = Some(first_run_id);

    first_store
        .submit_turn(first_request, &policy, &resolver)
        .await
        .unwrap();
    second_store.persistence_snapshot().await.unwrap();

    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    first_store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    first_store
        .complete_run(CompleteRunRequest {
            run_id: first_run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let mut second_request = submit_request_for(scope, "idem-row-stale-cache-b");
    second_request.requested_run_id = Some(second_run_id);
    let submitted = second_store
        .submit_turn(second_request, &policy, &resolver)
        .await
        .unwrap();

    assert_eq!(accepted_run_id(&submitted), second_run_id);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_get_run_state_refreshes_stale_cached_run() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let first_store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let second_store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let policy = AllowAllTurnAdmissionPolicy;
    let scope = turn_scope("thread-fs-row-stale-get-run-state");

    let submitted = first_store
        .submit_turn(
            submit_request_for(scope.clone(), "idem-row-stale-get-run-state"),
            &policy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id = accepted_run_id(&submitted);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();

    let cached = second_store
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(cached.status, TurnStatus::Queued);

    first_store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .expect("run claimed");

    let refreshed = second_store
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .unwrap();
    assert_eq!(refreshed.status, TurnStatus::Running);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_rejects_uncommitted_active_lock_reservation() {
    let backend = Arc::new(BlockingAppendFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let first_store = Arc::new(strict_row_store(Arc::clone(&scoped)));
    let second_store = strict_row_store(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let policy = AllowAllTurnAdmissionPolicy;
    let scope = turn_scope("thread-fs-row-uncommitted-active-lock");
    let first_run_id = TurnRunId::new();
    let second_run_id = TurnRunId::new();
    let mut first_request = submit_request_for(scope.clone(), "idem-row-uncommitted-active-lock-a");
    first_request.requested_run_id = Some(first_run_id);

    backend.block_next_append();
    let first_submit = {
        let first_store = Arc::clone(&first_store);
        tokio::spawn(async move {
            let resolver = InMemoryRunProfileResolver::default();
            let policy = AllowAllTurnAdmissionPolicy;
            first_store
                .submit_turn(first_request, &policy, &resolver)
                .await
        })
    };
    backend.wait_for_blocked_append().await;

    let snapshot = second_store.persistence_snapshot().await.unwrap();
    assert_eq!(snapshot.turns.len(), 1);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.active_locks.len(), 1);

    let mut second_request =
        submit_request_for(scope.clone(), "idem-row-uncommitted-active-lock-b");
    second_request.requested_run_id = Some(second_run_id);
    let second = second_store
        .submit_turn(second_request, &policy, &resolver)
        .await;
    assert!(
        matches!(
            second,
            Err(TurnError::Conflict { .. }) | Err(TurnError::ThreadBusy(_))
        ),
        "fresh store must not accept a same-thread run when it only sees another writer's pre-append active-lock reservation"
    );

    backend.release_blocked_append();
    let first = first_submit
        .await
        .expect("first submit task joins")
        .unwrap();
    assert_eq!(accepted_run_id(&first), first_run_id);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_recovers_preappend_active_lock_reservation() {
    let backend = Arc::new(BlockingAppendFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let first_store = Arc::new(strict_row_store(Arc::clone(&scoped)));
    let scope = turn_scope("thread-fs-row-recover-preappend-active-lock");
    let run_id = TurnRunId::new();
    let mut request = submit_request_for(scope, "idem-row-recover-preappend-active-lock");
    request.requested_run_id = Some(run_id);

    backend.block_next_append();
    let first_submit = {
        let first_store = Arc::clone(&first_store);
        tokio::spawn(async move {
            let resolver = InMemoryRunProfileResolver::default();
            let policy = AllowAllTurnAdmissionPolicy;
            first_store.submit_turn(request, &policy, &resolver).await
        })
    };
    backend.wait_for_blocked_append().await;
    first_submit.abort();

    let recovered_store = strict_row_store(Arc::clone(&scoped));
    let snapshot = recovered_store.persistence_snapshot().await.unwrap();
    assert_eq!(snapshot.turns.len(), 1);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.active_locks.len(), 1);

    let claimed = recovered_store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap()
        .expect("pre-append reservation recovers as queued run");
    assert_eq!(claimed.state.run_id, run_id);
    assert_eq!(claimed.state.status, TurnStatus::Running);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_migrates_legacy_state_blob() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let legacy_store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let scope = turn_scope("thread-fs-row-migrate-legacy");
    let run_id = TurnRunId::new();
    let mut request = submit_request_for(scope.clone(), "idem-fs-row-migrate-legacy");
    request.requested_run_id = Some(run_id);

    legacy_store
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let legacy_snapshot = legacy_store.persistence_snapshot().await.unwrap();
    assert!(
        legacy_snapshot
            .runs
            .iter()
            .any(|record| record.run_id == run_id),
        "legacy blob fixture must contain the submitted run"
    );
    assert!(
        backend
            .get(&snapshot_virtual_path())
            .await
            .unwrap()
            .is_some(),
        "legacy store must write the blob-shaped state snapshot"
    );

    let row_store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let state = row_store
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::Queued);
    assert!(
        backend
            .get(&row_run_virtual_path(run_id))
            .await
            .unwrap()
            .is_some(),
        "legacy snapshot migration must materialize a durable run row"
    );
    assert!(
        !backend
            .tail(&row_delta_log_virtual_path(), SeqNo::ZERO)
            .await
            .unwrap()
            .is_empty(),
        "legacy snapshot migration must enter through the delta journal"
    );
    assert!(
        backend
            .get(&snapshot_virtual_path())
            .await
            .unwrap()
            .is_some(),
        "migration keeps the legacy blob as rollback evidence"
    );

    let reopened = FilesystemTurnStateRowStore::new(scoped);
    let reopened_state = reopened
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(reopened_state.status, TurnStatus::Queued);
    let events = reopened
        .read_turn_events_after(&scope, None, None, 100)
        .await
        .unwrap();
    assert!(
        events
            .entries
            .iter()
            .any(|event| event.run_id == run_id && event.kind == TurnEventKind::Submitted),
        "migrated event rows must remain queryable after reopening the row store"
    );
}

#[tokio::test]
async fn filesystem_turn_state_row_store_does_not_remigrate_stale_blob_after_rows_exist() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let row_store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let row_scope = turn_scope("thread-fs-row-existing-wins");
    let row_run_id = TurnRunId::new();
    let mut row_request = submit_request_for(row_scope.clone(), "idem-fs-row-existing-wins");
    row_request.requested_run_id = Some(row_run_id);

    row_store
        .submit_turn(row_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let head_after_row_write = backend
        .head_seq(&row_delta_log_virtual_path(), SeqNo::ZERO)
        .await
        .unwrap();

    let stale_scope = turn_scope("thread-fs-row-stale-legacy");
    let stale_run_id = TurnRunId::new();
    let mut stale_request = submit_request_for(stale_scope.clone(), "idem-fs-row-stale-legacy");
    stale_request.requested_run_id = Some(stale_run_id);
    let legacy_store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    legacy_store
        .submit_turn(stale_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    assert!(
        backend
            .get(&snapshot_virtual_path())
            .await
            .unwrap()
            .is_some(),
        "stale legacy fixture must write a blob next to existing rows"
    );

    let reopened = FilesystemTurnStateRowStore::new(scoped);
    let state = reopened
        .get_run_state(GetRunStateRequest {
            scope: row_scope,
            run_id: row_run_id,
        })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::Queued);
    let stale = reopened
        .get_run_state(GetRunStateRequest {
            scope: stale_scope,
            run_id: stale_run_id,
        })
        .await;
    assert!(
        matches!(stale, Err(TurnError::ScopeNotFound)),
        "row data must remain authoritative once the row store has materialized rows"
    );
    let head_after_reopen = backend
        .head_seq(&row_delta_log_virtual_path(), SeqNo::ZERO)
        .await
        .unwrap();
    assert_eq!(
        head_after_reopen, head_after_row_write,
        "opening a row store with existing rows must not append a stale blob migration"
    );
}

#[tokio::test]
async fn filesystem_turn_state_row_store_concurrent_submits_preserve_all_runs() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(strict_row_store(Arc::clone(&scoped)));
    let resolver = Arc::new(InMemoryRunProfileResolver::default());
    let workers = 32;
    let barrier = Arc::new(tokio::sync::Barrier::new(workers));
    let mut handles = Vec::new();

    for idx in 0..workers {
        let store = Arc::clone(&store);
        let resolver = Arc::clone(&resolver);
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let scope = turn_scope(&format!("thread-fs-row-concurrent-{idx}"));
            let request =
                submit_request_for(scope.clone(), &format!("idem-fs-row-concurrent-{idx}"));
            barrier.wait().await;
            let response = store
                .submit_turn(request, &AllowAllTurnAdmissionPolicy, resolver.as_ref())
                .await
                .unwrap();
            (scope, accepted_run_id(&response))
        }));
    }

    let mut accepted = Vec::new();
    for handle in handles {
        accepted.push(handle.await.expect("submit task joins"));
    }
    assert_eq!(accepted.len(), workers);

    let reopened = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    for (scope, run_id) in accepted {
        let state = reopened
            .get_run_state(GetRunStateRequest { scope, run_id })
            .await
            .unwrap();
        assert_eq!(state.run_id, run_id);
        assert_eq!(state.status, TurnStatus::Queued);
    }
}

#[tokio::test]
async fn filesystem_turn_state_row_store_publish_is_optimistic_but_waits_for_append_ack() {
    let backend = Arc::new(BlockingAppendFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(strict_row_store(Arc::clone(&scoped)));
    let scope = turn_scope("thread-fs-row-blocked-materialize");
    let run_id = TurnRunId::new();
    let mut request = submit_request_for(scope.clone(), "idem-fs-row-blocked-materialize");
    request.requested_run_id = Some(run_id);
    let second_scope = turn_scope("thread-fs-row-blocked-materialize-second");
    let second_run_id = TurnRunId::new();
    let mut second_request = submit_request_for(
        second_scope.clone(),
        "idem-fs-row-blocked-materialize-second",
    );
    second_request.requested_run_id = Some(second_run_id);

    backend.block_next_append();
    let submit_store = Arc::clone(&store);
    let submit = tokio::spawn(async move {
        let resolver = InMemoryRunProfileResolver::default();
        let admission = AllowAllTurnAdmissionPolicy;
        submit_store
            .submit_turn(request, &admission, &resolver)
            .await
    });
    backend.wait_for_blocked_append().await;

    let visible = store
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(visible.status, TurnStatus::Queued);
    assert!(
        !submit.is_finished(),
        "writer must still wait for the durable append ack before returning"
    );

    let second_store = Arc::clone(&store);
    let second_submit = tokio::spawn(async move {
        let resolver = InMemoryRunProfileResolver::default();
        let admission = AllowAllTurnAdmissionPolicy;
        second_store
            .submit_turn(second_request, &admission, &resolver)
            .await
    });
    let second_visible = tokio::time::timeout(
        Duration::from_secs(1),
        retry_get_run_state(&store, second_scope.clone(), second_run_id),
    )
    .await
    .expect("commit gate must be released before the first durable append ack");
    assert_eq!(second_visible.status, TurnStatus::Queued);
    assert!(
        !second_submit.is_finished(),
        "second writer also waits for the single flusher's durable append ack"
    );

    backend.release_blocked_append();
    let response = submit.await.expect("submit task joins").unwrap();
    assert_eq!(accepted_run_id(&response), run_id);
    let second_response = second_submit
        .await
        .expect("second submit task joins")
        .unwrap();
    assert_eq!(accepted_run_id(&second_response), second_run_id);

    let state = store
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .unwrap();
    assert_eq!(state.status, TurnStatus::Queued);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_loop_checkpoint_releases_gate_before_append_ack() {
    let backend = Arc::new(BlockingAppendFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(strict_row_store(Arc::clone(&scoped)));
    let resolver = InMemoryRunProfileResolver::default();
    let parent_scope = turn_scope("thread-fs-row-checkpoint-blocked-append");
    let parent_response = store
        .submit_turn(
            submit_request_for(
                parent_scope.clone(),
                "idem-fs-row-checkpoint-blocked-append",
            ),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let parent_run_id = accepted_run_id(&parent_response);
    let parent_turn_id = accepted_turn_id(&parent_response);

    backend.block_next_append();
    let checkpoint_store = Arc::clone(&store);
    let checkpoint_scope = parent_scope.clone();
    let checkpoint = tokio::spawn(async move {
        checkpoint_store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: checkpoint_scope,
                turn_id: parent_turn_id,
                run_id: parent_run_id,
                state_ref: LoopCheckpointStateRef::new("checkpoint:blocked-append").unwrap(),
                schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
                schema_version: RunProfileVersion::new(1),
                kind: LoopCheckpointKind::BeforeModel,
                gate_ref: None,
            })
            .await
    });
    backend.wait_for_blocked_append().await;
    assert!(
        !checkpoint.is_finished(),
        "checkpoint writer must still wait for the durable append ack"
    );

    let second_scope = turn_scope("thread-fs-row-checkpoint-blocked-append-second");
    let second_run_id = TurnRunId::new();
    let mut second_request = submit_request_for(
        second_scope.clone(),
        "idem-fs-row-checkpoint-blocked-append-second",
    );
    second_request.requested_run_id = Some(second_run_id);
    let second_store = Arc::clone(&store);
    let second_submit = tokio::spawn(async move {
        let resolver = InMemoryRunProfileResolver::default();
        let admission = AllowAllTurnAdmissionPolicy;
        second_store
            .submit_turn(second_request, &admission, &resolver)
            .await
    });
    let second_visible = tokio::time::timeout(
        Duration::from_secs(1),
        retry_get_run_state(&store, second_scope.clone(), second_run_id),
    )
    .await
    .expect("loop checkpoint must release the commit gate before append ack");
    assert_eq!(second_visible.status, TurnStatus::Queued);
    assert!(
        !second_submit.is_finished(),
        "later writer still waits for the flusher's durable append ack"
    );

    backend.release_blocked_append();
    let checkpoint = checkpoint
        .await
        .expect("checkpoint task joins")
        .expect("checkpoint write succeeds");
    let second_response = second_submit
        .await
        .expect("second submit task joins")
        .unwrap();
    assert_eq!(accepted_run_id(&second_response), second_run_id);

    let loaded = store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: parent_scope,
            turn_id: parent_turn_id,
            run_id: parent_run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap();
    assert_eq!(
        loaded.as_ref().map(|record| record.checkpoint_id),
        Some(checkpoint.checkpoint_id)
    );
}

#[tokio::test]
async fn filesystem_turn_state_row_store_loop_checkpoint_times_out_without_losing_unknown_commit() {
    let backend = Arc::new(BlockingAppendFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(
        FilesystemTurnStateRowStore::new(Arc::clone(&scoped))
            .with_apply_timeout(Duration::from_millis(100)),
    );
    let resolver = InMemoryRunProfileResolver::default();
    let parent_scope = turn_scope("thread-fs-row-checkpoint-timeout");
    let parent_response = store
        .submit_turn(
            submit_request_for(parent_scope.clone(), "idem-fs-row-checkpoint-timeout"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let parent_run_id = accepted_run_id(&parent_response);
    let parent_turn_id = accepted_turn_id(&parent_response);

    backend.block_next_append();
    let checkpoint_store = Arc::clone(&store);
    let checkpoint_scope = parent_scope.clone();
    let checkpoint = tokio::spawn(async move {
        checkpoint_store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: checkpoint_scope,
                turn_id: parent_turn_id,
                run_id: parent_run_id,
                state_ref: LoopCheckpointStateRef::new("checkpoint:timeout").unwrap(),
                schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
                schema_version: RunProfileVersion::new(1),
                kind: LoopCheckpointKind::BeforeModel,
                gate_ref: None,
            })
            .await
    });
    backend.wait_for_blocked_append().await;

    let result = tokio::time::timeout(Duration::from_secs(1), checkpoint)
        .await
        .expect("checkpoint write must hit the bounded row-store append timeout")
        .expect("checkpoint task joins");
    assert!(
        matches!(result, Err(TurnError::Unavailable { reason }) if reason == "timed out waiting for loop checkpoint row-store append")
    );

    backend.release_blocked_append();
    for _ in 0..8 {
        let reopened = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
        let snapshot = reopened.persistence_snapshot().await.unwrap();
        if snapshot
            .loop_checkpoints
            .iter()
            .any(|record| record.run_id == parent_run_id)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed-out checkpoint append should still materialize after the blocked append commits");
}

#[tokio::test]
async fn filesystem_turn_state_row_store_get_loop_checkpoint_refreshes_stale_cached_rows() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let writer = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let reader = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let scope = turn_scope("thread-fs-row-checkpoint-stale-cache");
    let response = writer
        .submit_turn(
            submit_request_for(scope.clone(), "idem-fs-row-checkpoint-stale-cache"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let turn_id = accepted_turn_id(&response);

    let _stale_snapshot = reader.persistence_snapshot().await.unwrap();
    let checkpoint = writer
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: scope.clone(),
            turn_id,
            run_id,
            state_ref: LoopCheckpointStateRef::new("checkpoint:stale-cache").unwrap(),
            schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            schema_version: RunProfileVersion::new(1),
            kind: LoopCheckpointKind::BeforeModel,
            gate_ref: None,
        })
        .await
        .unwrap();

    let loaded = reader
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope,
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap();
    assert_eq!(
        loaded.as_ref().map(|record| record.checkpoint_id),
        Some(checkpoint.checkpoint_id)
    );
}

#[tokio::test]
async fn filesystem_turn_state_row_store_append_failure_clears_hot_cache() {
    let backend = Arc::new(RejectingAppendFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let scope = turn_scope("thread-fs-row-reject-append");
    let run_id = TurnRunId::new();
    let mut request = submit_request_for(scope.clone(), "idem-fs-row-reject-append");
    request.requested_run_id = Some(run_id);
    let resolver = InMemoryRunProfileResolver::default();

    let error = store
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap_err();
    assert!(
        matches!(error, TurnError::Unavailable { .. }),
        "durable delta append failure should fail the writer, got {error:?}"
    );
    assert_eq!(backend.append_calls(), 1);

    let hidden = store
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await;
    assert!(
        matches!(hidden, Err(TurnError::ScopeNotFound)),
        "failed durable append should not publish hot cache state: {hidden:?}"
    );
}

#[tokio::test]
async fn filesystem_turn_state_row_store_materializes_unadvanced_prior_delta_before_cursor() {
    let backend = Arc::new(FailOncePutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let first_scope = turn_scope("thread-fs-row-recover-prior-1");
    let first_run_id = TurnRunId::new();
    let mut first_request = submit_request_for(first_scope.clone(), "idem-fs-row-recover-prior-1");
    first_request.requested_run_id = Some(first_run_id);

    store
        .submit_turn(first_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();

    let second_scope = turn_scope("thread-fs-row-recover-prior-2");
    let second_run_id = TurnRunId::new();
    let mut second_request =
        submit_request_for(second_scope.clone(), "idem-fs-row-recover-prior-2");
    second_request.requested_run_id = Some(second_run_id);
    store
        .submit_turn(second_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();

    let reopened = FilesystemTurnStateRowStore::new(scoped);
    let first = retry_get_run_state(&reopened, first_scope, first_run_id).await;
    assert_eq!(first.status, TurnStatus::Queued);
    let second = retry_get_run_state(&reopened, second_scope, second_run_id).await;
    assert_eq!(second.status, TurnStatus::Queued);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_event_projection_replays_unmaterialized_journal() {
    let backend = Arc::new(FailOncePutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let scope = turn_scope("thread-fs-row-event-replay");
    let run_id = TurnRunId::new();
    let mut request = submit_request_for(scope.clone(), "idem-fs-row-event-replay");
    request.requested_run_id = Some(run_id);

    store
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();

    let reopened = FilesystemTurnStateRowStore::new(scoped);
    let events = retry_read_turn_events(&reopened, &scope).await;
    assert!(
        events
            .entries
            .iter()
            .any(|event| event.run_id == run_id && event.kind == TurnEventKind::Submitted),
        "event projection must replay durable delta journal tails before reading event rows"
    );
}

#[tokio::test]
async fn filesystem_turn_state_row_store_loop_checkpoint_survives_concurrent_full_snapshot_apply() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(FilesystemTurnStateRowStore::new(Arc::clone(&scoped)));
    let resolver = InMemoryRunProfileResolver::default();
    let parent_scope = turn_scope("thread-fs-row-checkpoint-race-parent");
    let parent_response = store
        .submit_turn(
            submit_request_for(parent_scope.clone(), "idem-fs-row-checkpoint-race-parent"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let parent_run_id = accepted_run_id(&parent_response);
    let parent_turn_id = accepted_turn_id(&parent_response);

    let admission = Arc::new(BlockingAdmissionPolicy::new());
    let child_store = Arc::clone(&store);
    let child_admission = Arc::clone(&admission);
    let child_parent_scope = parent_scope.clone();
    let child_scope = turn_scope("thread-fs-row-checkpoint-race-child");
    let child = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let resolver = InMemoryRunProfileResolver::default();
            child_store
                .submit_child_turn(
                    child_run_request(
                        child_parent_scope,
                        parent_run_id,
                        child_scope,
                        "idem-fs-row-checkpoint-race-child",
                        1,
                    ),
                    child_admission.as_ref(),
                    &resolver,
                )
                .await
        })
    });
    let wait_admission = Arc::clone(&admission);
    tokio::task::spawn_blocking(move || wait_admission.wait_until_entered())
        .await
        .unwrap();

    let checkpoint_store = Arc::clone(&store);
    let checkpoint_scope = parent_scope.clone();
    let checkpoint = tokio::spawn(async move {
        checkpoint_store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: checkpoint_scope,
                turn_id: parent_turn_id,
                run_id: parent_run_id,
                state_ref: LoopCheckpointStateRef::new("checkpoint:row-race").unwrap(),
                schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
                schema_version: RunProfileVersion::new(1),
                kind: LoopCheckpointKind::BeforeModel,
                gate_ref: None,
            })
            .await
    });
    tokio::task::yield_now().await;

    admission.release();
    tokio::task::spawn_blocking(move || child.join().expect("child submit thread joins"))
        .await
        .unwrap()
        .unwrap();
    let checkpoint = checkpoint
        .await
        .expect("checkpoint task joins")
        .expect("checkpoint write succeeds");
    let loaded = store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: parent_scope,
            turn_id: parent_turn_id,
            run_id: parent_run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap();
    assert_eq!(
        loaded.as_ref().map(|record| record.checkpoint_id),
        Some(checkpoint.checkpoint_id),
        "full-snapshot row-store publication must not overwrite a concurrent loop checkpoint"
    );
}

#[tokio::test]
async fn filesystem_turn_state_row_store_evicted_terminal_run_remains_queryable() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let limits = InMemoryTurnStateStoreLimits {
        max_events: 2,
        max_terminal_records: 1,
        max_idempotency_records: 1,
        ..InMemoryTurnStateStoreLimits::default()
    };
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped)).with_limits(limits);
    let resolver = InMemoryRunProfileResolver::default();

    let first_scope = turn_scope("thread-fs-row-evicted-terminal-1");
    let first_request = submit_request_for(first_scope.clone(), "idem-fs-row-evicted-1");
    let first_response = store
        .submit_turn(first_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let first_run_id = accepted_run_id(&first_response);
    let first_runner_id = TurnRunnerId::new();
    let first_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: first_runner_id,
            lease_token: first_lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    store
        .fail_run(FailRunRequest {
            run_id: first_run_id,
            runner_id: first_runner_id,
            lease_token: first_lease_token,
            failure: SanitizedFailure::new("test_failure").unwrap(),
        })
        .await
        .unwrap();

    let second_scope = turn_scope("thread-fs-row-evicted-terminal-2");
    let second_request = submit_request_for(second_scope, "idem-fs-row-evicted-2");
    let second_response = store
        .submit_turn(second_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let second_run_id = accepted_run_id(&second_response);
    let second_runner_id = TurnRunnerId::new();
    let second_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: second_runner_id,
            lease_token: second_lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    store
        .complete_run(CompleteRunRequest {
            run_id: second_run_id,
            runner_id: second_runner_id,
            lease_token: second_lease_token,
        })
        .await
        .unwrap();

    let hot_snapshot = store.persistence_snapshot().await.unwrap();
    assert!(
        !hot_snapshot
            .runs
            .iter()
            .any(|record| record.run_id == first_run_id),
        "terminal cache limit should evict the old failed run from the hot snapshot"
    );
    assert!(
        hot_snapshot.events.len() <= limits.max_events,
        "event cache limit should bound the hot snapshot without deleting durable events"
    );

    let failed = store
        .get_run_state(GetRunStateRequest {
            scope: first_scope.clone(),
            run_id: first_run_id,
        })
        .await
        .unwrap();
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(
        failed.failure.as_ref().map(SanitizedFailure::category),
        Some("test_failure")
    );

    let first_events = store
        .read_turn_events_after(&first_scope, None, None, 100)
        .await
        .unwrap();
    assert_eq!(first_events.rebase_required, None);
    assert!(
        first_events
            .entries
            .iter()
            .any(|event| event.run_id == first_run_id && event.kind == TurnEventKind::Failed),
        "events are Tier 2 run-record rows and must survive cache event eviction"
    );

    let reopened = FilesystemTurnStateRowStore::new(scoped).with_limits(limits);
    let reopened_hot_snapshot = reopened.persistence_snapshot().await.unwrap();
    assert!(
        !reopened_hot_snapshot
            .runs
            .iter()
            .any(|record| record.run_id == first_run_id),
        "startup should apply the same terminal cache window instead of hydrating all history"
    );
    let reopened_failed = reopened
        .get_run_state(GetRunStateRequest {
            scope: first_scope.clone(),
            run_id: first_run_id,
        })
        .await
        .unwrap();
    assert_eq!(reopened_failed.status, TurnStatus::Failed);
    assert_eq!(
        reopened_failed
            .failure
            .as_ref()
            .map(SanitizedFailure::category),
        Some("test_failure")
    );
    let reopened_events = reopened
        .read_turn_events_after(&first_scope, None, None, 100)
        .await
        .unwrap();
    assert_eq!(reopened_events.rebase_required, None);
    assert!(
        reopened_events
            .entries
            .iter()
            .any(|event| event.run_id == first_run_id && event.kind == TurnEventKind::Failed),
        "durable event rows should remain queryable after restart"
    );
}

#[tokio::test]
async fn filesystem_turn_state_row_store_validated_loop_exit_completion_remains_queryable() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let limits = InMemoryTurnStateStoreLimits {
        max_events: 2,
        max_terminal_records: 1,
        max_idempotency_records: 1,
        ..InMemoryTurnStateStoreLimits::default()
    };
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped)).with_limits(limits);
    let resolver = InMemoryRunProfileResolver::default();

    let first_scope = turn_scope("thread-fs-row-loop-exit-terminal-1");
    let first_response = store
        .submit_turn(
            submit_request_for(first_scope.clone(), "idem-fs-row-loop-exit-1"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let first_run_id = accepted_run_id(&first_response);
    let first_runner_id = TurnRunnerId::new();
    let first_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: first_runner_id,
            lease_token: first_lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    store
        .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
            run_id: first_run_id,
            runner_id: first_runner_id,
            lease_token: first_lease_token,
            mapping: LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed),
        })
        .await
        .unwrap();

    let second_scope = turn_scope("thread-fs-row-loop-exit-terminal-2");
    let second_response = store
        .submit_turn(
            submit_request_for(second_scope, "idem-fs-row-loop-exit-2"),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let second_run_id = accepted_run_id(&second_response);
    let second_runner_id = TurnRunnerId::new();
    let second_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: second_runner_id,
            lease_token: second_lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();
    store
        .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
            run_id: second_run_id,
            runner_id: second_runner_id,
            lease_token: second_lease_token,
            mapping: LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed),
        })
        .await
        .unwrap();

    let hot_snapshot = store.persistence_snapshot().await.unwrap();
    assert!(
        !hot_snapshot
            .runs
            .iter()
            .any(|record| record.run_id == first_run_id),
        "validated loop-exit completion should still evict old terminal runs from the hot snapshot"
    );

    let completed = store
        .get_run_state(GetRunStateRequest {
            scope: first_scope.clone(),
            run_id: first_run_id,
        })
        .await
        .unwrap();
    assert_eq!(completed.status, TurnStatus::Completed);

    let reopened = FilesystemTurnStateRowStore::new(scoped).with_limits(limits);
    let reopened_completed = reopened
        .get_run_state(GetRunStateRequest {
            scope: first_scope.clone(),
            run_id: first_run_id,
        })
        .await
        .unwrap();
    assert_eq!(reopened_completed.status, TurnStatus::Completed);
    let reopened_events = reopened
        .read_turn_events_after(&first_scope, None, None, 100)
        .await
        .unwrap();
    assert_eq!(reopened_events.rebase_required, None);
    assert!(
        reopened_events
            .entries
            .iter()
            .any(|event| event.run_id == first_run_id && event.kind == TurnEventKind::Completed),
        "validated loop-exit completion events should remain durable after cache eviction and restart"
    );
}

#[tokio::test]
async fn filesystem_turn_state_row_store_heartbeat_does_not_rewrite_run_row() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-row-heartbeat-memory"),
        "idem-fs-row-heartbeat-memory",
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
    let head_after_claim = backend
        .head_seq(&row_delta_log_virtual_path(), SeqNo::ZERO)
        .await
        .unwrap();
    let first_snapshot = store.persistence_snapshot().await.unwrap();
    let first_heartbeat_at = first_snapshot
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
        .unwrap();

    let head_after_heartbeat = backend
        .head_seq(&row_delta_log_virtual_path(), SeqNo::ZERO)
        .await
        .unwrap();
    assert_eq!(
        head_after_heartbeat, head_after_claim,
        "heartbeat must refresh the runner lease without appending a durable delta"
    );
    let heartbeat_snapshot = store.persistence_snapshot().await.unwrap();
    let heartbeat_at = heartbeat_snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .and_then(|record| record.last_heartbeat_at)
        .expect("heartbeat timestamp");
    assert!(
        heartbeat_at > first_heartbeat_at,
        "row-store read model should expose the refreshed memory lease timestamp"
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

/// Regression: a no-op apply that runs under a non-`None` runner-lease overlay
/// (`Run`/`All`) must NOT rewrite `state.json`.
///
/// The overlay patches time-varying lease fields (`last_heartbeat_at`,
/// `lease_expires_at`) from the per-run sidecar into the snapshot the apply
/// closure sees, so the overlaid snapshot diverges from the raw backend body
/// (whose lease fields are frozen at claim time once heartbeats only touch the
/// sidecar). The no-op baseline must therefore be the OVERLAID snapshot — if it
/// is taken from the raw body, an inert transition is misread as a real
/// mutation and the snapshot is rewritten on every call (version churn + CAS
/// retries under load). `recover_expired_leases` with nothing expired is a true
/// no-op apply under the `All` overlay, so it exercises exactly this path.
#[tokio::test]
async fn filesystem_turn_state_store_no_op_under_active_lease_overlay_does_not_rewrite_snapshot() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-noop-active-lease"),
        "idem-fs-noop-active-lease",
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

    // Heartbeat updates only the sidecar lease, leaving state.json's lease
    // fields frozen at claim time. This is the divergence the overlay bridges
    // and the exact condition under which the no-op baseline matters.
    tokio::time::sleep(Duration::from_millis(5)).await;
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let version_before = backend
        .get(&snapshot_virtual_path())
        .await
        .unwrap()
        .expect("snapshot after heartbeat")
        .version;

    // `now` well before the (heartbeat-refreshed) lease expiry, so nothing is
    // recovered: the apply closure leaves the overlaid snapshot unchanged.
    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc.with_ymd_and_hms(2026, 5, 17, 12, 0, 0).unwrap(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(
        recovered.recovered.is_empty(),
        "active lease must not be recovered before its expiry"
    );

    let version_after = backend
        .get(&snapshot_virtual_path())
        .await
        .unwrap()
        .expect("snapshot after no-op recover")
        .version;
    assert_eq!(
        version_after, version_before,
        "a no-op apply under an active-lease overlay must not rewrite state.json \
         (the no-op baseline is the overlaid snapshot, not the raw backend body)"
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
        .release_tree_descendants(&child_b_scope, parent, 1, parent)
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
