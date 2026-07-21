//! Contract tests for [`FilesystemCheckpointStateStore`] against a
//! [`ScopedFilesystem`] over [`DiskFilesystem`].

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, DiskFilesystem, Entry, FileStat,
    FilesystemError, FilesystemOperation, InMemoryBackend, RecordVersion, RootFilesystem,
    ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, HostPath, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
    ThreadId, VirtualPath,
};
use ironclaw_loop_host::FilesystemCheckpointStateStore;
use ironclaw_turns::{
    CheckpointSchemaId, CheckpointStateRecord, CheckpointStateStore, GetCheckpointStateRequest,
    MAX_CHECKPOINT_STATE_PAYLOAD_BYTES, PutCheckpointStateRequest, RunProfileVersion, TurnError,
    TurnId, TurnRunId, TurnScope, run_profile::LoopCheckpointKind,
    run_profile::LoopCheckpointStateRef,
};

fn engine_filesystem() -> DiskFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = DiskFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn scoped_checkpoint_state_fs_at<F>(
    backend: Arc<F>,
    tenant: &str,
    user: &str,
) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let tenant_user_prefix = format!("/engine/tenants/{tenant}/users/{user}");
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/checkpoint-state").expect("alias"),
        VirtualPath::new(format!("{tenant_user_prefix}/checkpoint-state")).expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn scoped_checkpoint_state_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    scoped_checkpoint_state_fs_at(backend, "test-tenant", "test-user")
}

fn turn_scope(tenant: &str, thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new(tenant).unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn minimal_scope(tenant: &str, thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new(tenant).unwrap(),
        None,
        None,
        ThreadId::new(thread).unwrap(),
    )
}

fn put_request(
    scope: TurnScope,
    turn_id: TurnId,
    run_id: TurnRunId,
    payload: Vec<u8>,
) -> PutCheckpointStateRequest {
    PutCheckpointStateRequest::new(
        scope,
        turn_id,
        run_id,
        CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
        RunProfileVersion::new(7),
        LoopCheckpointKind::BeforeSideEffect,
        payload,
    )
}

fn get_request(
    record: &CheckpointStateRecord,
    scope: TurnScope,
    turn_id: TurnId,
    run_id: TurnRunId,
) -> GetCheckpointStateRequest {
    GetCheckpointStateRequest {
        scope,
        turn_id,
        run_id,
        state_ref: record.state_ref.clone(),
        schema_id: record.schema_id.clone(),
        schema_version: record.schema_version,
        kind: record.kind,
    }
}

#[derive(Default)]
struct DiskFullFilesystem {
    inner: InMemoryBackend,
}

#[async_trait]
impl RootFilesystem for DiskFullFilesystem {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::WriteFile,
            reason: "disk full".to_string(),
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
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_persists_and_reopens() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_checkpoint_state_fs(Arc::clone(&backend));
    let store = FilesystemCheckpointStateStore::new(Arc::clone(&scoped));
    let scope = turn_scope("tenant1", "thread-checkpoint-state-persist");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let payload = b"RAW_PROMPT_SENTINEL sk-secret /host/path tool_input".to_vec();

    let record = store
        .put_checkpoint_state(put_request(scope.clone(), turn_id, run_id, payload.clone()))
        .await
        .unwrap();
    assert!(record.state_ref.as_str().starts_with("checkpoint:"));
    // The state ref is an opaque handle: it must not leak run identity.
    assert!(!record.state_ref.as_str().contains(&turn_id.to_string()));
    assert!(!record.state_ref.as_str().contains(&run_id.to_string()));
    assert_eq!(record.payload.as_bytes(), payload.as_slice());

    let reopened = FilesystemCheckpointStateStore::new(scoped);
    let loaded = reopened
        .get_checkpoint_state(get_request(&record, scope, turn_id, run_id))
        .await
        .unwrap()
        .expect("checkpoint payload should survive store reconstruction");

    assert_eq!(loaded, record);
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_maps_disk_full_write_failure_to_unavailable() {
    let backend = Arc::new(DiskFullFilesystem::default());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = turn_scope("tenant1", "thread-checkpoint-state-disk-full");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();

    let error = store
        .put_checkpoint_state(put_request(scope, turn_id, run_id, b"checkpoint".to_vec()))
        .await
        .unwrap_err();

    let TurnError::Unavailable { reason } = error else {
        panic!("expected retryable unavailable error, got {error:?}");
    };
    assert_eq!(
        reason,
        "checkpoint state persistence temporarily unavailable"
    );
    assert!(!reason.contains("disk full"));
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_hides_other_tenant_mounts() {
    let backend = Arc::new(engine_filesystem());
    let scoped_a = scoped_checkpoint_state_fs_at(Arc::clone(&backend), "tenant-a", "system");
    let scoped_b = scoped_checkpoint_state_fs_at(Arc::clone(&backend), "tenant-b", "system");
    let store_a = FilesystemCheckpointStateStore::new(scoped_a);
    let store_b = FilesystemCheckpointStateStore::new(scoped_b);
    let scope_a = turn_scope("tenant-a", "thread-cross-tenant");
    let scope_b = turn_scope("tenant-b", "thread-cross-tenant");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();

    let record = store_a
        .put_checkpoint_state(put_request(
            scope_a.clone(),
            turn_id,
            run_id,
            b"tenant-a checkpoint".to_vec(),
        ))
        .await
        .unwrap();

    assert!(
        store_b
            .get_checkpoint_state(get_request(&record, scope_b, turn_id, run_id))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_rejects_cross_run_ref() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = turn_scope("tenant1", "thread-cross-run");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let record = store
        .put_checkpoint_state(put_request(
            scope.clone(),
            turn_id,
            run_id,
            b"checkpoint".to_vec(),
        ))
        .await
        .unwrap();

    assert!(
        store
            .get_checkpoint_state(get_request(&record, scope, turn_id, TurnRunId::new()))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_rejects_cross_turn_id_ref() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = turn_scope("tenant1", "thread-cross-turn");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let record = store
        .put_checkpoint_state(put_request(
            scope.clone(),
            turn_id,
            run_id,
            b"checkpoint".to_vec(),
        ))
        .await
        .unwrap();

    assert!(
        store
            .get_checkpoint_state(get_request(&record, scope.clone(), TurnId::new(), run_id))
            .await
            .unwrap()
            .is_none(),
        "checkpoint state must not be returned for a different turn_id"
    );
    assert!(
        store
            .get_checkpoint_state(get_request(
                &record,
                turn_scope("tenant1", "thread-cross-all"),
                TurnId::new(),
                TurnRunId::new(),
            ))
            .await
            .unwrap()
            .is_none(),
        "checkpoint state must not be returned when scope, turn_id, and run_id all differ"
    );
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_rejects_same_tenant_cross_thread_scope() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = turn_scope("tenant1", "thread-scope-a");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let record = store
        .put_checkpoint_state(put_request(scope, turn_id, run_id, b"checkpoint".to_vec()))
        .await
        .unwrap();

    assert!(
        store
            .get_checkpoint_state(get_request(
                &record,
                turn_scope("tenant1", "thread-scope-b"),
                turn_id,
                run_id,
            ))
            .await
            .unwrap()
            .is_none(),
        "a state ref minted in one thread scope must not resolve in another"
    );
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_rejects_schema_or_kind_mismatch() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = turn_scope("tenant1", "thread-schema-mismatch");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let record = store
        .put_checkpoint_state(put_request(
            scope.clone(),
            turn_id,
            run_id,
            b"checkpoint".to_vec(),
        ))
        .await
        .unwrap();

    let mut wrong_schema = get_request(&record, scope.clone(), turn_id, run_id);
    wrong_schema.schema_id = CheckpointSchemaId::new("other_checkpoint_v1").unwrap();
    assert!(
        store
            .get_checkpoint_state(wrong_schema)
            .await
            .unwrap()
            .is_none()
    );

    let mut wrong_version = get_request(&record, scope.clone(), turn_id, run_id);
    wrong_version.schema_version = RunProfileVersion::new(2);
    assert!(
        store
            .get_checkpoint_state(wrong_version)
            .await
            .unwrap()
            .is_none()
    );

    let mut wrong_kind = get_request(&record, scope, turn_id, run_id);
    wrong_kind.kind = LoopCheckpointKind::BeforeModel;
    assert!(
        store
            .get_checkpoint_state(wrong_kind)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_round_trips_empty_payload() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = turn_scope("tenant1", "thread-empty-payload");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();

    let record = store
        .put_checkpoint_state(put_request(scope.clone(), turn_id, run_id, Vec::new()))
        .await
        .unwrap();
    assert!(record.payload.is_empty());

    let loaded = store
        .get_checkpoint_state(get_request(&record, scope, turn_id, run_id))
        .await
        .unwrap()
        .expect("empty payload should round-trip");
    assert_eq!(loaded.payload.as_bytes(), &[] as &[u8]);
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_accepts_exact_max_size_payload() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = turn_scope("tenant1", "thread-max-size");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let payload = vec![b'A'; MAX_CHECKPOINT_STATE_PAYLOAD_BYTES];

    let record = store
        .put_checkpoint_state(put_request(scope.clone(), turn_id, run_id, payload.clone()))
        .await
        .unwrap();
    assert_eq!(record.payload.len(), MAX_CHECKPOINT_STATE_PAYLOAD_BYTES);

    let loaded = store
        .get_checkpoint_state(get_request(&record, scope, turn_id, run_id))
        .await
        .unwrap()
        .expect("exact max-size payload should round-trip");
    assert_eq!(loaded.payload.as_bytes(), payload.as_slice());
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_multiple_puts_produce_distinct_refs() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = turn_scope("tenant1", "thread-distinct-refs");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let payload = b"same".to_vec();

    let record_a = store
        .put_checkpoint_state(put_request(scope.clone(), turn_id, run_id, payload.clone()))
        .await
        .unwrap();
    let record_b = store
        .put_checkpoint_state(put_request(scope.clone(), turn_id, run_id, payload.clone()))
        .await
        .unwrap();

    assert_ne!(
        record_a.state_ref, record_b.state_ref,
        "each put must produce a unique state_ref"
    );
    let loaded_a = store
        .get_checkpoint_state(get_request(&record_a, scope.clone(), turn_id, run_id))
        .await
        .unwrap()
        .expect("first record should be independently retrievable");
    assert_eq!(loaded_a.payload.as_bytes(), payload.as_slice());
    let loaded_b = store
        .get_checkpoint_state(get_request(&record_b, scope, turn_id, run_id))
        .await
        .unwrap()
        .expect("second record should be independently retrievable");
    assert_eq!(loaded_b.payload.as_bytes(), payload.as_slice());
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_returns_none_for_unknown_state_ref() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = turn_scope("tenant1", "thread-missing-state-ref");

    let missing = GetCheckpointStateRequest {
        scope,
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        state_ref: LoopCheckpointStateRef::new("checkpoint:missing-state").unwrap(),
        schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
        schema_version: RunProfileVersion::new(7),
        kind: LoopCheckpointKind::BeforeSideEffect,
    };

    assert!(store.get_checkpoint_state(missing).await.unwrap().is_none());
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_round_trips_minimal_scope() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let scope = minimal_scope("tenant1", "thread-minimal-scope");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let payload = b"minimal scope checkpoint".to_vec();

    let record = store
        .put_checkpoint_state(put_request(scope.clone(), turn_id, run_id, payload.clone()))
        .await
        .unwrap();
    let loaded = store
        .get_checkpoint_state(get_request(&record, scope, turn_id, run_id))
        .await
        .unwrap()
        .expect("minimal scope checkpoint should be readable");

    assert_eq!(loaded.payload.as_bytes(), payload.as_slice());
    assert_eq!(loaded, record);
}

#[tokio::test]
async fn filesystem_checkpoint_state_store_rejects_oversized_payload() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemCheckpointStateStore::new(scoped_checkpoint_state_fs(backend));
    let error = store
        .put_checkpoint_state(put_request(
            turn_scope("tenant1", "thread-oversized"),
            TurnId::new(),
            TurnRunId::new(),
            vec![b'x'; MAX_CHECKPOINT_STATE_PAYLOAD_BYTES + 1],
        ))
        .await
        .unwrap_err();

    assert!(matches!(error, TurnError::InvalidRequest { .. }));
    assert!(!format!("{error:?}").contains("xxxx"));
}
