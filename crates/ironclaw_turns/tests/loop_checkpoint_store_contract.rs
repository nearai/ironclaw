#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;

use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
use ironclaw_turns::{
    CheckpointSchemaId, GetLoopCheckpointRequest, InMemoryLoopCheckpointStore,
    InMemoryTurnStateStore, LoopCheckpointStateRef, LoopCheckpointStore, PutLoopCheckpointRequest,
    RunProfileVersion, TurnId, TurnRunId, TurnScope, run_profile::LoopCheckpointKind,
};

#[cfg(feature = "libsql")]
use ironclaw_turns::LibSqlTurnStateStore;
#[cfg(feature = "postgres")]
use ironclaw_turns::PostgresTurnStateStore;

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
        snapshot.checkpoints.is_empty(),
        "loop checkpoint mappings must not use turn_checkpoints"
    );

    let reopened = InMemoryTurnStateStore::from_persistence_snapshot(
        snapshot,
        ironclaw_turns::InMemoryTurnStateStoreLimits::default(),
    )
    .unwrap();
    assert_loop_checkpoint_store_cross_scope_and_run_miss(&reopened).await;
}

#[cfg(feature = "libsql")]
async fn libsql_store() -> (Arc<LibSqlTurnStateStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("turns.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let store = Arc::new(LibSqlTurnStateStore::new(db));
    store.run_migrations().await.unwrap();
    (store, dir)
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_loop_checkpoint_roundtrip_uses_loop_mapping_table() {
    let (store, _dir) = libsql_store().await;
    assert_loop_checkpoint_store_roundtrip(store.as_ref()).await;
    assert_loop_checkpoint_store_cross_scope_and_run_miss(store.as_ref()).await;

    let snapshot = store.persistence_snapshot().await.unwrap();
    assert_eq!(snapshot.loop_checkpoints.len(), 2);
    assert!(
        snapshot.checkpoints.is_empty(),
        "libSQL loop mappings must not be written to turn_checkpoints"
    );
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_loop_checkpoint_roundtrip_uses_loop_mapping_table() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let store = Arc::new(PostgresTurnStateStore::new(pool));
    store.run_migrations().await.unwrap();
    assert_loop_checkpoint_store_roundtrip(store.as_ref()).await;
    assert_loop_checkpoint_store_cross_scope_and_run_miss(store.as_ref()).await;

    let snapshot = store.persistence_snapshot().await.unwrap();
    assert!(
        snapshot
            .loop_checkpoints
            .iter()
            .any(|record| record.state_ref.as_str() == "checkpoint:test-state"),
        "Postgres loop mappings must be written to turn_loop_checkpoints"
    );
    assert!(
        snapshot
            .checkpoints
            .iter()
            .all(|record| record.state_ref.as_str() != "checkpoint:test-state"),
        "Postgres loop mappings must not be written to turn_checkpoints"
    );
}

#[cfg(feature = "postgres")]
async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
    let Ok(url) = std::env::var("IRONCLAW_TURNS_POSTGRES_URL") else {
        eprintln!(
            "skipping postgres loop checkpoint contract: IRONCLAW_TURNS_POSTGRES_URL not set"
        );
        return None;
    };
    let config: tokio_postgres::Config = match url.parse() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("skipping postgres loop checkpoint contract: invalid url ({error})");
            return None;
        }
    };
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .unwrap();
    if let Err(error) = pool.get().await {
        eprintln!("skipping postgres loop checkpoint contract: database unavailable ({error})");
        return None;
    }
    Some(pool)
}
