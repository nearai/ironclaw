#![cfg(any(feature = "libsql", feature = "postgres"))]

use chrono::{TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
use ironclaw_turns::{
    GateRef, LoopCheckpointStore, TurnCheckpointId, TurnCheckpointRecord, TurnRunId, TurnScope,
    TurnStatus,
    run_profile::{LoopCheckpointKind, LoopCheckpointStateRef},
};
use std::sync::Arc;

#[cfg(feature = "libsql")]
use ironclaw_turns::LibSqlTurnStateStore;

fn test_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn make_checkpoint(
    run_id: TurnRunId,
    sequence: u64,
    kind: LoopCheckpointKind,
) -> TurnCheckpointRecord {
    TurnCheckpointRecord {
        checkpoint_id: TurnCheckpointId::new(),
        run_id,
        scope: Some(test_scope("thread-a")),
        sequence,
        status: TurnStatus::BlockedApproval,
        gate_ref: GateRef::new("gate:test-gate").unwrap(),
        kind,
        state_ref: LoopCheckpointStateRef::new("checkpoint:test-state").unwrap(),
        created_at: Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap(),
    }
}

// ── Tests against in-memory backend ──────────────────────────────────────────

#[tokio::test]
async fn inmemory_put_get_roundtrip() {
    let store = ironclaw_turns::InMemoryTurnStateStore::default();
    let run_id = TurnRunId::new();
    let record = make_checkpoint(run_id, 1, LoopCheckpointKind::BeforeModel);
    let checkpoint_id = record.checkpoint_id;

    store.put_loop_checkpoint(record.clone()).await.unwrap();
    let fetched = store
        .get_loop_checkpoint(checkpoint_id, run_id)
        .await
        .unwrap();

    let fetched = fetched.expect("should find checkpoint");
    assert_eq!(fetched.checkpoint_id, checkpoint_id);
    assert_eq!(fetched.run_id, run_id);
    assert_eq!(fetched.kind, LoopCheckpointKind::BeforeModel);
    assert_eq!(
        fetched.state_ref,
        LoopCheckpointStateRef::new("checkpoint:test-state").unwrap()
    );
    assert_eq!(fetched.scope, Some(test_scope("thread-a")));
}

#[tokio::test]
async fn inmemory_cross_run_rejection() {
    let store = ironclaw_turns::InMemoryTurnStateStore::default();
    let run_a = TurnRunId::new();
    let run_b = TurnRunId::new();
    let record = make_checkpoint(run_a, 1, LoopCheckpointKind::BeforeBlock);
    let checkpoint_id = record.checkpoint_id;

    store.put_loop_checkpoint(record).await.unwrap();
    let fetched = store
        .get_loop_checkpoint(checkpoint_id, run_b)
        .await
        .unwrap();
    assert!(fetched.is_none(), "cross-run lookup should return None");
}

#[tokio::test]
async fn inmemory_multiple_checkpoints_per_run() {
    let store = ironclaw_turns::InMemoryTurnStateStore::default();
    let run_id = TurnRunId::new();

    let r1 = make_checkpoint(run_id, 1, LoopCheckpointKind::BeforeModel);
    let r2 = make_checkpoint(run_id, 2, LoopCheckpointKind::BeforeSideEffect);
    let r3 = make_checkpoint(run_id, 3, LoopCheckpointKind::Final);
    let id1 = r1.checkpoint_id;
    let id2 = r2.checkpoint_id;
    let id3 = r3.checkpoint_id;

    store.put_loop_checkpoint(r1).await.unwrap();
    store.put_loop_checkpoint(r2).await.unwrap();
    store.put_loop_checkpoint(r3).await.unwrap();

    assert!(store.get_loop_checkpoint(id1, run_id).await.unwrap().is_some());
    assert!(store.get_loop_checkpoint(id2, run_id).await.unwrap().is_some());
    assert!(store.get_loop_checkpoint(id3, run_id).await.unwrap().is_some());
}

#[tokio::test]
async fn inmemory_idempotent_put() {
    let store = ironclaw_turns::InMemoryTurnStateStore::default();
    let run_id = TurnRunId::new();
    let record = make_checkpoint(run_id, 1, LoopCheckpointKind::BeforeBlock);

    store.put_loop_checkpoint(record.clone()).await.unwrap();
    // Second put with same checkpoint_id should succeed without error.
    store.put_loop_checkpoint(record).await.unwrap();
}

#[tokio::test]
async fn serde_backward_compat_missing_fields() {
    // Simulate old persisted JSON that lacks kind, state_ref, and scope.
    let json = r#"{
        "checkpoint_id": "00000000-0000-0000-0000-000000000001",
        "run_id": "00000000-0000-0000-0000-000000000002",
        "sequence": 1,
        "status": "BlockedApproval",
        "gate_ref": "gate:test",
        "created_at": "2026-05-11T12:00:00Z"
    }"#;
    let record: TurnCheckpointRecord = serde_json::from_str(json).unwrap();
    assert_eq!(record.kind, LoopCheckpointKind::BeforeBlock);
    assert_eq!(
        record.state_ref,
        LoopCheckpointStateRef::new("checkpoint:unknown").unwrap()
    );
    assert_eq!(record.scope, None);
}

// ── Tests against libSQL backend ─────────────────────────────────────────────

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
async fn libsql_put_get_roundtrip() {
    let (store, _dir) = libsql_store().await;
    let run_id = TurnRunId::new();
    let record = make_checkpoint(run_id, 1, LoopCheckpointKind::BeforeModel);
    let checkpoint_id = record.checkpoint_id;

    store.put_loop_checkpoint(record.clone()).await.unwrap();
    let fetched = store
        .get_loop_checkpoint(checkpoint_id, run_id)
        .await
        .unwrap()
        .expect("should find checkpoint");

    assert_eq!(fetched.checkpoint_id, checkpoint_id);
    assert_eq!(fetched.run_id, run_id);
    assert_eq!(fetched.kind, LoopCheckpointKind::BeforeModel);
    assert_eq!(
        fetched.state_ref,
        LoopCheckpointStateRef::new("checkpoint:test-state").unwrap()
    );
    assert_eq!(fetched.scope, Some(test_scope("thread-a")));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_cross_run_rejection() {
    let (store, _dir) = libsql_store().await;
    let run_a = TurnRunId::new();
    let run_b = TurnRunId::new();
    let record = make_checkpoint(run_a, 1, LoopCheckpointKind::BeforeBlock);
    let checkpoint_id = record.checkpoint_id;

    store.put_loop_checkpoint(record).await.unwrap();
    let fetched = store
        .get_loop_checkpoint(checkpoint_id, run_b)
        .await
        .unwrap();
    assert!(fetched.is_none(), "cross-run lookup should return None");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_multiple_checkpoints_per_run() {
    let (store, _dir) = libsql_store().await;
    let run_id = TurnRunId::new();

    let r1 = make_checkpoint(run_id, 1, LoopCheckpointKind::BeforeModel);
    let r2 = make_checkpoint(run_id, 2, LoopCheckpointKind::BeforeSideEffect);
    let r3 = make_checkpoint(run_id, 3, LoopCheckpointKind::Final);
    let id1 = r1.checkpoint_id;
    let id2 = r2.checkpoint_id;
    let id3 = r3.checkpoint_id;

    store.put_loop_checkpoint(r1).await.unwrap();
    store.put_loop_checkpoint(r2).await.unwrap();
    store.put_loop_checkpoint(r3).await.unwrap();

    assert!(store.get_loop_checkpoint(id1, run_id).await.unwrap().is_some());
    assert!(store.get_loop_checkpoint(id2, run_id).await.unwrap().is_some());
    assert!(store.get_loop_checkpoint(id3, run_id).await.unwrap().is_some());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_idempotent_put() {
    let (store, _dir) = libsql_store().await;
    let run_id = TurnRunId::new();
    let record = make_checkpoint(run_id, 1, LoopCheckpointKind::BeforeBlock);

    store.put_loop_checkpoint(record.clone()).await.unwrap();
    store.put_loop_checkpoint(record).await.unwrap();
}
