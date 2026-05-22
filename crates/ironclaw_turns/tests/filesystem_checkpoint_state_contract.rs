//! Contract tests for [`FilesystemCheckpointStateStore`] against a
//! [`ScopedFilesystem`] over [`LocalFilesystem`].

use std::sync::Arc;

use ironclaw_filesystem::{LocalFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, HostPath, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
    ThreadId, VirtualPath,
};
use ironclaw_turns::{
    CheckpointSchemaId, CheckpointStateRecord, CheckpointStateStore,
    FilesystemCheckpointStateStore, GetCheckpointStateRequest, MAX_CHECKPOINT_STATE_PAYLOAD_BYTES,
    PutCheckpointStateRequest, RunProfileVersion, TurnError, TurnId, TurnRunId, TurnScope,
    run_profile::LoopCheckpointKind,
};

fn engine_filesystem() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
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
