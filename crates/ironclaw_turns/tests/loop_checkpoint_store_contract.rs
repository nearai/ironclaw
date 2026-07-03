use std::sync::Arc;

use ironclaw_filesystem::{
    BackendId, BackendKind, CompositeRootFilesystem, ContentKind, InMemoryBackend, IndexPolicy,
    MountDescriptor, RootFilesystem, ScopedFilesystem, StorageClass,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, ThreadId,
    VirtualPath,
};
use ironclaw_turns::{
    CheckpointSchemaId, FilesystemTurnStateStore, GetLoopCheckpointRequest,
    InMemoryLoopCheckpointStore, InMemoryTurnStateStore, LoopCheckpointStateRef,
    LoopCheckpointStore, PutLoopCheckpointRequest, RunProfileVersion, TurnId, TurnRunId, TurnScope,
    run_profile::LoopCheckpointKind,
};

fn test_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn put_request(scope: TurnScope, turn_id: TurnId, run_id: TurnRunId) -> PutLoopCheckpointRequest {
    PutLoopCheckpointRequest {
        scope,
        turn_id,
        run_id,
        state_ref: LoopCheckpointStateRef::new("checkpoint:test-state").unwrap(),
        schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
        schema_version: RunProfileVersion::new(1),
        kind: LoopCheckpointKind::BeforeModel,
        gate_ref: None,
    }
}

async fn assert_loop_checkpoint_store_roundtrip(store: &(impl LoopCheckpointStore + ?Sized)) {
    let scope = test_scope("thread-loop-checkpoint-roundtrip");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let checkpoint = store
        .put_loop_checkpoint(put_request(scope.clone(), turn_id, run_id))
        .await
        .unwrap();

    let loaded = store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: scope.clone(),
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap()
        .expect("checkpoint id should resolve to state ref");

    assert_eq!(loaded, checkpoint);
    assert_eq!(loaded.scope, scope);
    assert_eq!(loaded.turn_id, turn_id);
    assert_eq!(loaded.run_id, run_id);
    assert_eq!(loaded.kind, LoopCheckpointKind::BeforeModel);
}

async fn assert_loop_checkpoint_store_cross_scope_and_run_miss(
    store: &(impl LoopCheckpointStore + ?Sized),
) {
    let scope = test_scope("thread-loop-checkpoint-scope-a");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let checkpoint = store
        .put_loop_checkpoint(put_request(scope.clone(), turn_id, run_id))
        .await
        .unwrap();

    let cross_scope = store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: test_scope("thread-loop-checkpoint-scope-b"),
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap();
    assert!(cross_scope.is_none(), "cross-scope lookup must fail closed");

    let cross_run = store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope,
            turn_id,
            run_id: TurnRunId::new(),
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap();
    assert!(cross_run.is_none(), "cross-run lookup must fail closed");
}

#[tokio::test]
async fn inmemory_standalone_loop_checkpoint_roundtrip() {
    let store = InMemoryLoopCheckpointStore::default();
    assert_loop_checkpoint_store_roundtrip(&store).await;
    assert_loop_checkpoint_store_cross_scope_and_run_miss(&store).await;
}

#[tokio::test]
async fn inmemory_turn_state_loop_checkpoint_roundtrip_and_snapshot() {
    let store = InMemoryTurnStateStore::default();
    assert_loop_checkpoint_store_roundtrip(&store).await;
    assert_loop_checkpoint_store_cross_scope_and_run_miss(&store).await;

    let snapshot = store.persistence_snapshot();
    assert_eq!(snapshot.loop_checkpoints.len(), 2);
    assert!(
        snapshot
            .checkpoints
            .iter()
            .all(|record| record.state_ref.as_str() != "checkpoint:test-state"),
        "loop checkpoint mappings must not use turn_checkpoints"
    );

    let reopened = InMemoryTurnStateStore::from_persistence_snapshot(
        snapshot,
        ironclaw_turns::InMemoryTurnStateStoreLimits::default(),
    )
    .unwrap();
    assert_loop_checkpoint_store_cross_scope_and_run_miss(&reopened).await;
}

/// Build the CAS-capable backend shape used by `/turns` in local-dev and
/// production composition.
fn engine_filesystem() -> InMemoryBackend {
    InMemoryBackend::new()
}

fn engine_mount_descriptor<F>(backend: &F) -> MountDescriptor
where
    F: RootFilesystem,
{
    MountDescriptor {
        virtual_root: VirtualPath::new("/engine").unwrap(),
        backend_id: BackendId::new("test-loop-checkpoint").unwrap(),
        backend_kind: BackendKind::MemoryDocuments,
        storage_class: StorageClass::StructuredRecords,
        content_kind: ContentKind::StructuredRecord,
        index_policy: IndexPolicy::NotIndexed,
        capabilities: backend.capabilities(),
    }
}

fn scoped_turns_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<CompositeRootFilesystem>>
where
    F: RootFilesystem + 'static,
{
    let mut root = CompositeRootFilesystem::new();
    root.mount(engine_mount_descriptor(backend.as_ref()), backend)
        .unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("alias"),
        VirtualPath::new("/engine/tenants/test-tenant/users/test-user/turns").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(Arc::new(root), mounts))
}

#[tokio::test]
async fn filesystem_turn_state_loop_checkpoint_roundtrip_and_snapshot() {
    let backend = Arc::new(engine_filesystem());
    let scoped = scoped_turns_fs(backend);
    let store = FilesystemTurnStateStore::new(Arc::clone(&scoped));
    assert_loop_checkpoint_store_roundtrip(&store).await;
    assert_loop_checkpoint_store_cross_scope_and_run_miss(&store).await;

    let snapshot = store.persistence_snapshot().await.unwrap();
    assert_eq!(snapshot.loop_checkpoints.len(), 2);
    assert!(
        snapshot
            .checkpoints
            .iter()
            .all(|record| record.state_ref.as_str() != "checkpoint:test-state"),
        "filesystem loop mappings must not collide with turn_checkpoints"
    );

    // Reopen against the same scoped filesystem; the persistence snapshot must
    // rehydrate the same loop-checkpoint set without any backend-specific
    // migration step.
    let reopened = FilesystemTurnStateStore::new(scoped);
    let reopened_snapshot = reopened.persistence_snapshot().await.unwrap();
    assert_eq!(reopened_snapshot.loop_checkpoints.len(), 2);
}
