// arch-exempt: large_file, filesystem turn-state contract suite decomposition, plan #5662
//! Contract tests for [`FilesystemTurnStateRowStore`] against a
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
    ContentKind, ContentType, DirEntry, DiskFilesystem, Entry, FileStat, FilesystemError,
    FilesystemOperation, InMemoryBackend, IndexPolicy, MountDescriptor, RecordVersion,
    RootFilesystem, ScopedFilesystem, SeqNo, StorageClass, VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, HostPath, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
    ResourceScope, ScopedPath, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_turns::{
    AcceptedMessageRef, AdmissionRejection, AllowAllTurnAdmissionPolicy, BlockedReason,
    CheckpointSchemaId, EventCursor, FilesystemTurnStateBlockPersistence,
    FilesystemTurnStateRowStore, GateRef, GetLoopCheckpointRequest, GetRunStateRequest,
    IdempotencyKey, InMemoryRunProfileResolver, LoopCheckpointStore, LoopExitMapping,
    ProductTurnContext, PutLoopCheckpointRequest, ReplyTargetBindingRef, ResumeTurnPrecondition,
    ResumeTurnRequest, RunOriginAdapter, RunProfileRequest, RunProfileVersion,
    SanitizedCancelReason, SanitizedFailure, SourceBindingRef, SubmitChildRunRequest,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnAdmissionPolicy, TurnCheckpointId,
    TurnError, TurnEventKind, TurnEventProjectionSource, TurnId, TurnLeaseToken,
    TurnLifecycleEvent, TurnOriginKind, TurnOwner, TurnRunId, TurnRunnerId, TurnScope,
    TurnSpawnTreeStateStore, TurnStateBlockPersistence, TurnStateStore, TurnStateStoreLimits,
    TurnStatus,
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

fn byte_only_filesystem() -> DiskFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = DiskFilesystem::new();
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
    FilesystemTurnStateRowStore::new(scoped)
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

struct RejectingAppendFilesystem<F> {
    inner: F,
    append_calls: AtomicUsize,
}

struct FailOncePutFilesystem<F> {
    inner: F,
    fail_next_put: AtomicBool,
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
        requested_model: None,
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
    let store = FilesystemTurnStateRowStore::new(scoped);

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

/// #6263 Step 5b: `get_run_state` prefers the process-local hot cache under
/// (now-unconditional) write-behind — the single-writer authority model, where
/// a live SECOND instance over the same backend is not a supported production
/// shape (each scope has exactly one authoritative writer/reader instance;
/// see `filesystem_store/row_store.rs`). The refresh-from-durable-rows path
/// this test locks is REOPEN after a drop (crash/restart), not a second live
/// instance staying open alongside the first.
#[tokio::test]
async fn filesystem_turn_state_row_store_get_run_state_refreshes_stale_cached_run() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let policy = AllowAllTurnAdmissionPolicy;
    let scope = turn_scope("thread-fs-row-stale-get-run-state");

    let submitted = store
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

    let cached = store
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(cached.status, TurnStatus::Queued);

    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .expect("run claimed");
    // The claim (Queued -> Running) is non-critical claim churn — drain before
    // the drop so the crash-free reopen below sees it (this test's point is
    // the durable-rows refresh on reopen, not the write's own crash-loss
    // window, which the crash-consistency suite covers separately).
    store.drain().await.expect("drain claim");
    drop(store);

    let reopened = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let refreshed = reopened
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .unwrap();
    assert_eq!(refreshed.status, TurnStatus::Running);
}

#[tokio::test]
async fn filesystem_turn_state_row_store_migrates_legacy_state_blob() {
    // Build a legacy `/turns/state.json` blob the way a pre-#6263
    // `inmemory-turn-state` deployment did: produce a Queued run on a throwaway
    // store, snapshot it, and write that snapshot through the block-persistence
    // sink (the row store's legacy-blob importer reads exactly this artifact).
    let source_backend = Arc::new(engine_filesystem());
    let source_scoped = scoped_turns_fs(Arc::clone(&source_backend));
    let source = FilesystemTurnStateRowStore::new(Arc::clone(&source_scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let scope = turn_scope("thread-fs-row-migrate-legacy");
    let run_id = TurnRunId::new();
    let mut request = submit_request_for(scope.clone(), "idem-fs-row-migrate-legacy");
    request.requested_run_id = Some(run_id);
    source
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let legacy_snapshot = source.persistence_snapshot().await.unwrap();
    assert!(
        legacy_snapshot
            .runs
            .iter()
            .any(|record| record.run_id == run_id),
        "legacy blob fixture must contain the submitted run"
    );

    // Fresh backend with no row data (first boot after the flip): seed only the
    // legacy blob via the block-persistence sink.
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    FilesystemTurnStateBlockPersistence::new(Arc::clone(&scoped))
        .persist(&legacy_snapshot)
        .await;
    assert!(
        backend
            .get(&snapshot_virtual_path())
            .await
            .unwrap()
            .is_some(),
        "block-persistence must write the blob-shaped state snapshot"
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

/// #6263 Step 4 migration proof. An existing `inmemory-turn-state` deployment
/// persisted its gate-parked (approval/auth) turns through
/// [`FilesystemTurnStateBlockPersistence`], which writes the full
/// [`TurnPersistenceSnapshot`] to `/turns/state.json`. After the profile flips
/// onto the row store, first boot MUST import that exact artifact so no
/// approval-parked turn is silently dropped. The block-persistence sink and the
/// row store's legacy-blob importer share `io::snapshot_path()` +
/// `io::snapshot_entry`, so the migration is automatic — this pins it for the
/// real gate-parked artifact, not just a generic Queued-run blob.
#[tokio::test]
async fn filesystem_turn_state_row_store_migrates_block_persistence_gate_park_snapshot() {
    // 1) Produce a genuine BlockedApproval run and snapshot it exactly as the old
    //    in-memory authority did when a run parked on a gate.
    let source_backend = Arc::new(engine_filesystem());
    let source_scoped = scoped_turns_fs(Arc::clone(&source_backend));
    let source = FilesystemTurnStateRowStore::new(Arc::clone(&source_scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let scope = turn_scope("thread-block-persist-migrate");
    let request = submit_request_for(scope.clone(), "idem-block-persist-migrate");
    let response = source
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    source
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    let gate_ref = GateRef::new("gate-block-persist-migrate").unwrap();
    let checkpoint_id = TurnCheckpointId::new();
    let blocked = source
        .block_run(BlockRunRequest {
            run_id,
            runner_id,
            lease_token,
            checkpoint_id,
            state_ref: LoopCheckpointStateRef::new("checkpoint:block-persist-migrate").unwrap(),
            reason: BlockedReason::Approval {
                gate_ref: gate_ref.clone(),
            },
        })
        .await
        .unwrap();
    assert_eq!(blocked.status, TurnStatus::BlockedApproval);
    let gate_parked_snapshot = source.persistence_snapshot().await.unwrap();
    assert!(
        gate_parked_snapshot
            .runs
            .iter()
            .any(|record| record.run_id == run_id && record.status == TurnStatus::BlockedApproval),
        "fixture snapshot must carry the gate-parked run"
    );

    // 2) Write that snapshot the way the OLD deployment's block-persistence sink
    //    did, onto a FRESH backend that has no row data (a first boot after the
    //    flip).
    let target_backend = Arc::new(engine_filesystem());
    let target_scoped = scoped_turns_fs(Arc::clone(&target_backend));
    let block_persistence = FilesystemTurnStateBlockPersistence::new(Arc::clone(&target_scoped));
    block_persistence.persist(&gate_parked_snapshot).await;
    assert!(
        target_backend
            .get(&snapshot_virtual_path())
            .await
            .unwrap()
            .is_some(),
        "block-persistence must leave the legacy blob the row store imports from"
    );

    // 3) Open the row store over the same fresh backend; the first read triggers
    //    the automatic legacy-blob import, recovering the gate-parked turn.
    let row_store = FilesystemTurnStateRowStore::new(Arc::clone(&target_scoped));
    let migrated = row_store
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(
        migrated.status,
        TurnStatus::BlockedApproval,
        "block-persistence gate-parked turn must survive the row-store migration"
    );

    // 4) The gate-parked run is durable rows now: it rehydrates on a fresh reopen.
    let reopened = FilesystemTurnStateRowStore::new(target_scoped);
    let reopened_state = reopened
        .get_run_state(GetRunStateRequest { scope, run_id })
        .await
        .unwrap();
    assert_eq!(reopened_state.status, TurnStatus::BlockedApproval);
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

    // Build a stale legacy `/turns/state.json` blob (as a pre-#6263 deployment
    // would have) carrying a run the row store has never seen, and drop it next
    // to the existing rows via the block-persistence sink. The fixture snapshot
    // is produced on a throwaway store over its OWN backend, then persisted onto
    // this store's backend.
    let stale_scope = turn_scope("thread-fs-row-stale-legacy");
    let stale_run_id = TurnRunId::new();
    let mut stale_request = submit_request_for(stale_scope.clone(), "idem-fs-row-stale-legacy");
    stale_request.requested_run_id = Some(stale_run_id);
    let stale_source_backend = Arc::new(engine_filesystem());
    let stale_source_scoped = scoped_turns_fs(Arc::clone(&stale_source_backend));
    let stale_source = FilesystemTurnStateRowStore::new(Arc::clone(&stale_source_scoped));
    stale_source
        .submit_turn(stale_request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let stale_snapshot = stale_source.persistence_snapshot().await.unwrap();
    FilesystemTurnStateBlockPersistence::new(Arc::clone(&scoped))
        .persist(&stale_snapshot)
        .await;
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

/// #6263 Step 5b: a `BeforeSideEffect` checkpoint is critical (it is a
/// durability barrier — expired-lease recovery treats its ABSENCE as proof no
/// side effect ran, so it must be durable before the caller proceeds), so its
/// writer genuinely awaits the durable append ack, unlike a `BeforeModel`
/// checkpoint (non-critical, returns before its append). This locks two
/// things: the critical writer really does wait on a blocked append (not just
/// return immediately), and a SECOND concurrent critical writer's row becomes
/// visible via the hot cache before its OWN ack resolves (the cache updates
/// synchronously under the commit lock, before the ack-await) — so unrelated
/// concurrent writers are not needlessly serialized on each other's visibility,
/// only on the shared journal's append order.
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
                kind: LoopCheckpointKind::BeforeSideEffect,
                gate_ref: None,
            })
            .await
    });
    backend.wait_for_blocked_append().await;
    assert!(
        !checkpoint.is_finished(),
        "a critical BeforeSideEffect checkpoint writer must still wait for the durable append ack"
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

/// #6263 Step 5b: only a `BeforeSideEffect` checkpoint is critical (a
/// durability barrier), so only it awaits its append and can hit the bounded
/// `apply_timeout`; a `BeforeModel` checkpoint is non-critical and would
/// return immediately without ever touching this timeout.
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
                kind: LoopCheckpointKind::BeforeSideEffect,
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
    // A `BeforeModel` checkpoint is non-critical (only `BeforeSideEffect` is a
    // durability barrier) — drain `writer` so it is durable before `reader` (a
    // different cached instance over the same backend) reads it. This test's
    // point is the stale-cache refresh from durable rows, not the write's own
    // crash-loss window.
    writer.drain().await.expect("drain checkpoint");

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
    let limits = TurnStateStoreLimits::default()
        .set_max_events(2)
        .set_max_terminal_records(1)
        .set_max_idempotency_records(1);
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
    let limits = TurnStateStoreLimits {
        max_events: 2,
        max_terminal_records: 1,
        max_idempotency_records: 1,
        ..TurnStateStoreLimits::default()
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
            model_usage: None,
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
            model_usage: None,
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
    // The claim (Queued -> Running) is non-critical claim churn — drain it so
    // its append has landed before the baseline read below. Otherwise the
    // claim's own async append could land DURING the heartbeat window and be
    // mistaken for a durable write heartbeat caused.
    store.drain().await.expect("drain claim");
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

// DELETED: filesystem_turn_state_store_heartbeat_updates_lease_without_rewriting_snapshot
// asserted the deleted blob store's `state.json` version staying unchanged
// across a heartbeat. The row store's equivalent — a heartbeat refreshes the
// memory-backed runner lease WITHOUT appending a durable delta (and never
// materializes a runner-lease sidecar) — is covered by
// `filesystem_turn_state_row_store_heartbeat_does_not_rewrite_run_row` and
// `filesystem_turn_state_store_heartbeat_does_not_write_runner_lease_sidecar`
// (both in this file).

/// Regression: a no-op apply that runs under a non-`None` runner-lease overlay
/// (`Run`/`All`) must NOT append a durable delta.
///
/// The overlay patches time-varying lease fields (`last_heartbeat_at`,
/// `lease_expires_at`) from the memory-backed lease into the snapshot the apply
/// closure sees, so the overlaid snapshot diverges from the durable rows (whose
/// lease fields are frozen at claim time once heartbeats only touch memory). The
/// no-op baseline must therefore be the OVERLAID snapshot — if it is taken from
/// the raw durable rows, an inert transition is misread as a real mutation and a
/// spurious delta is appended on every call (journal churn under load).
/// `recover_expired_leases` with nothing expired is a true no-op apply under the
/// `All` overlay, so it exercises exactly this path. This asserts through the
/// row store's durable delta journal (the row-store analog of the deleted blob
/// store's `state.json` version check).
#[tokio::test]
async fn filesystem_turn_state_store_no_op_under_active_lease_overlay_does_not_append_delta() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(scoped);
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

    // Heartbeat updates only the memory lease, leaving the durable run row's
    // lease fields frozen at claim time. This is the divergence the overlay
    // bridges and the exact condition under which the no-op baseline matters.
    tokio::time::sleep(Duration::from_millis(5)).await;
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();
    // Drain the (non-critical) claim's async append so the baseline head below
    // is stable and cannot be advanced by claim churn landing mid-window.
    store.drain().await.expect("drain claim");

    let head_before = backend
        .head_seq(&row_delta_log_virtual_path(), SeqNo::ZERO)
        .await
        .unwrap();

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

    let head_after = backend
        .head_seq(&row_delta_log_virtual_path(), SeqNo::ZERO)
        .await
        .unwrap();
    assert_eq!(
        head_after, head_before,
        "a no-op apply under an active-lease overlay must not append a durable delta \
         (the no-op baseline is the overlaid snapshot, not the raw durable rows)"
    );
}

#[tokio::test]
async fn filesystem_turn_state_store_heartbeat_seeds_memory_lease_from_snapshot_after_reopen() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
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

    let reopened = FilesystemTurnStateRowStore::new(scoped);
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
    let store = FilesystemTurnStateRowStore::new(scoped);
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
    // #6284: a checkpoint-less run (no loop checkpoint reached) that crashed
    // before any side effect is re-queued to a claimable state, not stranded
    // terminal `Failed`. The point of this test — recovery firing on the
    // heartbeat-refreshed memory lease, not the stale state.json expiry — is
    // unchanged; the run is still recovered exactly once.
    assert_eq!(recovered.recovered[0].status, TurnStatus::Queued);
}

/// The terminal `complete_run` transition validates the runner lease against
/// the memory-backed lease store (runner leases are no longer durable). This
/// exercises that authority directly through the public API: a completion with a
/// mismatched lease is rejected, and only the live memory lease (refreshed by a
/// heartbeat) authorizes the terminal transition. (Replaces the deleted blob
/// store's variant that overwrote a stale `state.json` lease expiry to prove the
/// memory lease overlay won.)
#[tokio::test]
async fn filesystem_turn_state_store_complete_run_uses_memory_runner_lease() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(scoped);
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

    // A completion whose lease does not match the memory-backed runner lease is
    // rejected — proving the terminal transition is gated on the memory lease,
    // not merely on the durable run row.
    let mismatch = store
        .complete_run(CompleteRunRequest {
            run_id,
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
        })
        .await
        .expect_err("mismatched lease must not complete the run");
    assert_eq!(mismatch, TurnError::LeaseMismatch);

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
    let store = FilesystemTurnStateRowStore::new(scoped);
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

// DELETED: filesystem_turn_state_store_heartbeat_does_not_read_snapshot asserted
// the deleted blob store never touched the `state.json` GET during a heartbeat
// (via a filesystem that rejected snapshot reads). The row store services a
// heartbeat entirely from the in-memory runner lease — it appends no durable
// delta and writes no sidecar — which is covered by
// `filesystem_turn_state_row_store_heartbeat_does_not_rewrite_run_row` and
// `filesystem_turn_state_store_heartbeat_does_not_write_runner_lease_sidecar`
// (both in this file).

/// The memory-backed runner lease carries the run's current lifecycle status, so
/// a heartbeat on a run that has moved to `CancelRequested` is rejected as an
/// invalid `-> Running` transition without consulting durable state. (Replaces
/// the deleted blob store's variant that additionally rejected `state.json`
/// reads to prove the status came from memory.)
#[tokio::test]
async fn filesystem_turn_state_store_cancel_requested_heartbeat_uses_memory_lease_status() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(scoped);
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

/// A heartbeat is serviced entirely from the memory-backed runner lease, so it
/// must not be serialized behind a concurrent writer's blocked durable append.
/// (Replaces the deleted blob store's variant that blocked the `state.json`
/// put; the row store's durable write is the delta-journal append.)
#[tokio::test]
async fn filesystem_turn_state_store_heartbeat_succeeds_while_durable_append_is_blocked() {
    let backend = Arc::new(BlockingAppendFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(FilesystemTurnStateRowStore::new(scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(
        turn_scope("thread-fs-heartbeat-blocked-append"),
        "idem-fs-heartbeat-blocked-append",
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
    // Drain the claim's async append so the next blocked append is the
    // concurrent writer's, not leftover claim churn.
    store.drain().await.expect("drain claim");

    backend.block_next_append();
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

    tokio::time::timeout(Duration::from_secs(1), backend.wait_for_blocked_append())
        .await
        .expect("writer should reach the blocked durable append");

    tokio::time::timeout(
        Duration::from_secs(1),
        store.heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        }),
    )
    .await
    .expect("heartbeat must not wait behind a blocked durable append")
    .unwrap();

    backend.release_blocked_append();
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
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();

    let request = submit_request_for(turn_scope("thread-fs-persist"), "idem-fs-persist");
    let response = store
        .submit_turn(request.clone(), &AllowAllTurnAdmissionPolicy, &resolver)
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);

    // Re-construct the store over the same scoped filesystem; the on-disk
    // snapshot must rehydrate the queued run.
    let reopened = FilesystemTurnStateRowStore::new(scoped);
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
    let store = FilesystemTurnStateRowStore::new(scoped);
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

// DELETED: filesystem_turn_state_store_clears_stale_snapshot_cache_after_version_mismatch
// asserted the deleted blob store's single-document snapshot cache being
// invalidated on a CAS version mismatch before retry. The row store has no
// single-document snapshot cache; its stale-cache-refresh-from-durable-rows
// behavior is covered by
// `filesystem_turn_state_row_store_get_run_state_refreshes_stale_cached_run`
// (this file) and `row_store_crash_consistency::live_reads_are_read_your_writes_consistent`.

#[tokio::test]
async fn filesystem_turn_state_store_snapshot_reads_overlap_apply_write() {
    let backend = Arc::new(BlockingPutFilesystem::new(engine_filesystem()));
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = Arc::new(FilesystemTurnStateRowStore::new(Arc::clone(&scoped)));
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
    let store = Arc::new(FilesystemTurnStateRowStore::new(Arc::clone(&scoped)));
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

// DELETED: filesystem_turn_state_store_cas_storm_preserves_all_submits exercised
// the deleted blob store's single-document CAS-retry storm (all writers
// contending on one `state.json` document). The row store's per-run/append-log
// durability preserves all concurrent submits without a single-document CAS
// funnel; that is covered by
// `filesystem_turn_state_row_store_concurrent_submits_preserve_all_runs`
// (this file) and
// `row_store_crash_consistency::write_behind_concurrent_writers_under_cap_stay_consistent`.

// DELETED: filesystem_turn_state_store_timed_out_apply_does_not_wedge_subsequent_writes
// and filesystem_turn_state_store_timed_out_claim_does_not_wedge_scheduler_writes
// asserted the deleted blob store's bounded single-document apply timeout
// ("turn state filesystem apply timed out") not wedging later writers. The row
// store has its own bounded apply timeout on the critical (durable) path; that a
// timed-out critical write surfaces `Unavailable` without losing the eventual
// commit is covered by
// `filesystem_turn_state_row_store_loop_checkpoint_times_out_without_losing_unknown_commit`
// (this file).

// DELETED: filesystem_turn_state_store_returns_unavailable_after_persistent_version_mismatches
// asserted the deleted blob store's single-document CAS-retry-exhaustion path
// ("turn state filesystem CAS retries exhausted"). The row store's durable
// append/CAS-budget exhaustion is covered by
// `row_store_crash_consistency::lease_expiry_crash_retry_bound_fails_with_crash_retry_exhausted`
// and `write_behind_append_failure_halts_degrades_and_recovers_consistently`.

// DELETED: filesystem_turn_state_store_returns_unavailable_on_non_version_mismatch_put_error
// asserted the deleted blob store's single-put-no-retry error surface. The row
// store's equivalent — a durable delta append failure surfaces as
// `TurnError::Unavailable` and does not publish hot-cache state — is covered by
// `filesystem_turn_state_row_store_append_failure_clears_hot_cache` in this file.

#[tokio::test]
async fn filesystem_turn_state_store_rejects_byte_only_backend_before_persisting_rows() {
    // A byte-only backend (no versioned CAS / structured-record append) cannot
    // back the row store's durable delta journal. A submit must fail loudly with
    // a retryable `TurnError::Unavailable` and leave no durable row artifacts
    // behind, rather than silently accepting state it can never persist
    // consistently. (The old blob store surfaced a specific
    // "backend must support versioned CAS" string; the row store rejects the
    // same class of backend when the durable journal append fails.)
    let backend = Arc::new(byte_only_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let resolver = InMemoryRunProfileResolver::default();
    let request = submit_request_for(turn_scope("thread-fs-byte-only"), "idem-byte-only");

    let error = match store
        .submit_turn(request, &AllowAllTurnAdmissionPolicy, &resolver)
        .await
    {
        Ok(_) => panic!("byte-only backend must not accept durable turn-state rows"),
        Err(error) => error,
    };
    assert!(
        matches!(error, TurnError::Unavailable { .. }),
        "byte-only backend rejection must surface as retryable Unavailable: {error:?}"
    );
    assert!(
        backend
            .tail(&row_delta_log_virtual_path(), SeqNo::ZERO)
            .await
            .unwrap_or_default()
            .is_empty(),
        "a rejected byte-only backend must not commit durable delta rows"
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

    let store_a = FilesystemTurnStateRowStore::new(Arc::clone(&scoped_a));
    let store_b = FilesystemTurnStateRowStore::new(Arc::clone(&scoped_b));
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
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
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

    let reopened = FilesystemTurnStateRowStore::new(scoped);
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
    // Build a parent + child spawn tree on a throwaway row store, then snapshot
    // it and splice in a SHADOW parent — a clone of the parent run under a
    // different tenant scope but sharing the parent's run_id. Persist the spliced
    // snapshot as a legacy `/turns/state.json` blob on a FRESH backend via the
    // block-persistence sink; opening the row store there migrates all three
    // runs into durable rows. The adversarial shadow (same run_id, foreign
    // scope) proves spawn-tree reads filter by scope, not run_id alone. (The
    // deleted blob store spliced the shadow into its live `state.json`; the row
    // store never writes that blob, so the shadow is injected through the
    // one-shot legacy-blob migration instead.)
    let source_backend = Arc::new(engine_filesystem());
    let source_scoped = scoped_turns_fs(Arc::clone(&source_backend));
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&source_scoped));
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

    let mut snapshot = store.persistence_snapshot().await.unwrap();
    let mut shadow_parent = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == parent && record.scope == parent_scope)
        .expect("parent run in fixture snapshot")
        .clone();
    shadow_parent.scope = TurnScope::new(
        TenantId::new("shadow-tenant").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-fs-scope-shadow").unwrap(),
    );
    snapshot.runs.insert(0, shadow_parent);

    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    FilesystemTurnStateBlockPersistence::new(Arc::clone(&scoped))
        .persist(&snapshot)
        .await;

    let reopened = FilesystemTurnStateRowStore::new(scoped);
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
    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
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
    let reopened = FilesystemTurnStateRowStore::new(scoped);
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

    let reopened2 = FilesystemTurnStateRowStore::new(scoped_turns_fs(Arc::clone(&backend)));
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

// ---------------------------------------------------------------------------
// Durable turn-event indexed-query read path (#6382 follow-up).
//
// `read_turn_events_after` serves a scoped `cursor > after` range via the
// backend `query` over an `Entry::indexed` projection, instead of listing the
// whole events collection and reading every cross-thread row after the cursor.
// These tests pin the three behaviors that path introduces: scope pruning,
// query-only visibility (an unprojected row is invisible), and the one-time
// backfill that re-projects pre-upgrade rows so historical events survive.
// ---------------------------------------------------------------------------

/// A bare (unwrapped) `TurnLifecycleEvent` row body — the `StoredRow::Raw`
/// shape an event row persisted before the indexed-projection change would
/// have on disk (no `journal_seq` wrapper, no `indexed` projection).
fn raw_submitted_event(scope: &TurnScope, cursor: u64, run_id: TurnRunId) -> TurnLifecycleEvent {
    TurnLifecycleEvent {
        cursor: EventCursor(cursor),
        scope: scope.clone(),
        occurred_at: None,
        owner_user_id: None,
        run_id,
        status: TurnStatus::Queued,
        kind: TurnEventKind::Submitted,
        blocked_gate: None,
        sanitized_reason: None,
        retryable: None,
        detail: None,
    }
}

/// Write a pre-upgrade event row (bare body, no `indexed` projection) at the
/// exact scoped row path the store reads, through the same scoped filesystem.
async fn write_unprojected_event_row(
    scoped: &ScopedFilesystem<CompositeRootFilesystem>,
    event: &TurnLifecycleEvent,
) {
    let path =
        ScopedPath::new(format!("/turns/rows/v1/events/{:020}.json", event.cursor.0)).unwrap();
    let entry =
        Entry::bytes(serde_json::to_vec(event).unwrap()).with_content_type(ContentType::json());
    scoped
        .put(
            &ResourceScope::system(),
            &path,
            entry,
            CasExpectation::Absent,
        )
        .await
        .expect("write unprojected event row");
}

/// Pre-mark the durable event-index backfill complete so the store does NOT
/// re-project on read.
async fn mark_events_index_backfilled(scoped: &ScopedFilesystem<CompositeRootFilesystem>) {
    let path = ScopedPath::new("/turns/rows/v1/meta/events-index.json").unwrap();
    let entry =
        Entry::bytes(br#"{"backfilled":true}"#.to_vec()).with_content_type(ContentType::json());
    scoped
        .put(&ResourceScope::system(), &path, entry, CasExpectation::Any)
        .await
        .expect("write backfill marker");
}

#[tokio::test]
async fn filesystem_turn_state_events_query_isolates_scopes_across_threads() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let store = FilesystemTurnStateRowStore::new(scoped);
    let resolver = InMemoryRunProfileResolver::default();

    let scope_a = turn_scope("thread-events-a");
    let scope_b = turn_scope("thread-events-b");
    let run_a = accepted_run_id(
        &store
            .submit_turn(
                submit_request_for(scope_a.clone(), "idem-events-a"),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
            .unwrap(),
    );
    let run_b = accepted_run_id(
        &store
            .submit_turn(
                submit_request_for(scope_b.clone(), "idem-events-b"),
                &AllowAllTurnAdmissionPolicy,
                &resolver,
            )
            .await
            .unwrap(),
    );

    let page_a = retry_read_turn_events(&store, &scope_a).await;
    assert!(
        page_a.entries.iter().all(|event| event.scope == scope_a),
        "the scoped indexed query must return only the requested scope's events"
    );
    assert!(
        page_a
            .entries
            .iter()
            .any(|event| event.run_id == run_a && event.kind == TurnEventKind::Submitted),
        "thread-a's own Submitted event must be returned"
    );
    assert!(
        !page_a.entries.iter().any(|event| event.run_id == run_b),
        "thread-b's events must not leak into thread-a's timeline"
    );

    // The cursor range is exclusive-after: replaying past the newest cursor
    // yields an empty page rather than re-serving the same events.
    let newest = page_a
        .entries
        .iter()
        .map(|event| event.cursor)
        .max()
        .expect("thread-a has at least one event");
    let after_newest = store
        .read_turn_events_after(&scope_a, None, Some(newest), 100)
        .await
        .unwrap();
    assert!(
        after_newest.entries.is_empty(),
        "reading after the newest cursor must return no further events"
    );
}

#[tokio::test]
async fn filesystem_turn_state_events_query_hides_rows_missing_index_projection() {
    // A pre-upgrade event row carries no `indexed` projection. With the backfill
    // marker pre-set (so no re-projection runs), the indexed query must NOT
    // surface it — this is what proves the read path is the indexed query and
    // not the legacy directory scan (which ignores `indexed` and would find it).
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let scope = turn_scope("thread-events-unprojected");
    write_unprojected_event_row(&scoped, &raw_submitted_event(&scope, 1, TurnRunId::new())).await;
    mark_events_index_backfilled(&scoped).await;

    let store = FilesystemTurnStateRowStore::new(scoped);
    let page = store
        .read_turn_events_after(&scope, None, None, 100)
        .await
        .unwrap();
    assert!(
        page.entries.is_empty(),
        "the indexed query must not surface an event row that carries no scope_key projection"
    );
}

#[tokio::test]
async fn filesystem_turn_state_events_backfill_reprojects_preexisting_rows() {
    // Same pre-upgrade row, but WITHOUT the backfill marker: first read must
    // re-project it so the indexed query surfaces the historical event, and the
    // projection must survive a reopen.
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let scope = turn_scope("thread-events-backfill");
    let run_id = TurnRunId::new();
    write_unprojected_event_row(&scoped, &raw_submitted_event(&scope, 1, run_id)).await;

    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let page = store
        .read_turn_events_after(&scope, None, None, 100)
        .await
        .unwrap();
    assert!(
        page.entries
            .iter()
            .any(|event| event.run_id == run_id && event.cursor == EventCursor(1)),
        "backfill must re-project a pre-upgrade event row so the indexed query surfaces it"
    );

    let reopened = FilesystemTurnStateRowStore::new(scoped);
    let reopened_page = reopened
        .read_turn_events_after(&scope, None, None, 100)
        .await
        .unwrap();
    assert!(
        reopened_page
            .entries
            .iter()
            .any(|event| event.run_id == run_id),
        "the durable re-projection must persist across a row-store reopen"
    );
}

/// Write a pre-upgrade **tombstone** event row (materialized shape with a null
/// `value` and no `indexed` projection) — the on-disk residue of a pruned event
/// that the one-time backfill must read, recognize as a tombstone, and skip
/// (never re-project, never surface) without failing or stalling the migration.
async fn write_events_tombstone_row(
    scoped: &ScopedFilesystem<CompositeRootFilesystem>,
    cursor: u64,
) {
    let path = ScopedPath::new(format!("/turns/rows/v1/events/{cursor:020}.json")).unwrap();
    let entry = Entry::bytes(br#"{"journal_seq":1,"value":null}"#.to_vec())
        .with_content_type(ContentType::json());
    scoped
        .put(
            &ResourceScope::system(),
            &path,
            entry,
            CasExpectation::Absent,
        )
        .await
        .expect("write tombstone event row");
}

#[tokio::test]
async fn filesystem_turn_state_events_backfill_skips_tombstones_and_reprojects_live_rows() {
    // Regression for the durable-read backfill over a tombstone-inclusive events
    // collection (#6390): the events collection retains a durable tombstone for
    // every pruned event, so the one-time backfill scans a set NOT bounded by
    // the live-event count. It must re-project every live pre-upgrade row (now
    // at bounded concurrency, not serially), skip tombstones without surfacing
    // or failing on them, and still write the completion marker so a reopen does
    // not repeat the work.
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(Arc::clone(&backend));
    let scope = turn_scope("thread-events-backfill-tombstones");
    let other = turn_scope("thread-events-backfill-other");

    // Live pre-upgrade rows for the target scope, interleaved (by cursor) with
    // tombstones and another scope's rows across the shared cursor space.
    let mut live_run_ids = Vec::new();
    for cursor in 1..=20u64 {
        match cursor % 4 {
            0 => write_events_tombstone_row(&scoped, cursor).await,
            1 => {
                let run_id = TurnRunId::new();
                live_run_ids.push(run_id);
                write_unprojected_event_row(&scoped, &raw_submitted_event(&scope, cursor, run_id))
                    .await;
            }
            _ => {
                write_unprojected_event_row(
                    &scoped,
                    &raw_submitted_event(&other, cursor, TurnRunId::new()),
                )
                .await;
            }
        }
    }

    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let page = store
        .read_turn_events_after(&scope, None, None, 100)
        .await
        .unwrap();

    for run_id in &live_run_ids {
        assert!(
            page.entries.iter().any(|event| event.run_id == *run_id),
            "backfill must re-project every live pre-upgrade row for the target scope"
        );
    }
    assert!(
        page.entries.iter().all(|event| event.scope == scope),
        "tombstones and other scopes' rows must never surface in the scoped query"
    );

    // The completion marker is written even over a tombstone-heavy collection,
    // so a reopen serves from the index without re-running the backfill scan.
    let marker = scoped
        .get(
            &ResourceScope::system(),
            &ScopedPath::new("/turns/rows/v1/meta/events-index.json").unwrap(),
        )
        .await
        .unwrap()
        .expect("backfill completion marker present");
    let marker: serde_json::Value = serde_json::from_slice(&marker.entry.body).unwrap();
    assert_eq!(
        marker.get("backfilled").and_then(|value| value.as_bool()),
        Some(true),
        "backfill must record completion even when the events collection is tombstone-heavy"
    );
}
