//! Regression tests for the last-owner TOCTOU race.
//!
//! Two concurrent demote/remove operations against different owners of the
//! same workspace used to both observe `owner_count > 1`, both pass the
//! application-level check, and both commit — leaving the workspace with
//! zero owners.
//!
//! The libSQL backend is tested here against a real on-disk SQLite file
//! (in-memory databases do not share state between connections). PostgreSQL
//! uses `SELECT ... FOR UPDATE` on the workspace row for the same guarantee
//! and is exercised through the integration tier.

#![cfg(feature = "libsql")]

use std::sync::Arc;

use chrono::Utc;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::{Database, UserRecord, UserStore, WorkspaceMgmtStore};
use serde_json::json;
use tokio::sync::Barrier;
use uuid::Uuid;

async fn make_backend() -> (Arc<LibSqlBackend>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&path)
        .await
        .expect("open libsql backend");
    backend.run_migrations().await.expect("migrations");
    (Arc::new(backend), dir)
}

fn test_user(id: &str) -> UserRecord {
    let now = Utc::now();
    UserRecord {
        id: id.to_string(),
        email: Some(format!("{id}@example.com")),
        display_name: id.to_string(),
        status: "active".to_string(),
        role: "member".to_string(),
        created_at: now,
        updated_at: now,
        last_login_at: None,
        created_by: None,
        metadata: json!({}),
    }
}

async fn seed_two_owners(backend: &Arc<LibSqlBackend>) -> Uuid {
    backend.create_user(&test_user("alice")).await.unwrap();
    backend.create_user(&test_user("bob")).await.unwrap();

    let workspace = backend
        .create_workspace("Demo", "demo", "", "alice", &json!({}))
        .await
        .unwrap();

    // Promote bob to owner. `update_member_role_checked` with `new_role =
    // "owner"` does not run the last-owner guard (promotion can never
    // reduce the count), so this succeeds.
    backend
        .update_member_role_checked(workspace.id, "bob", "owner", Some("alice"))
        .await
        .unwrap();

    workspace.id
}

async fn count_owners(backend: &Arc<LibSqlBackend>, workspace_id: Uuid) -> usize {
    backend
        .list_workspace_members(workspace_id)
        .await
        .unwrap()
        .into_iter()
        .filter(|(_, m)| m.role == "owner")
        .count()
}

// Multi-threaded runtime + a barrier so both tasks reach the transaction
// entry point simultaneously. A single-threaded runtime cooperatively
// schedules the tasks and the "race" never actually happens.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_demote_of_two_owners_leaves_exactly_one() {
    // Repeat the race a few times — libSQL's default `BEGIN DEFERRED` can
    // sometimes happen to serialize correctly by luck. Run enough iterations
    // that the bug, if reintroduced, would reliably trip at least one run.
    for iter in 0..16 {
        let (backend, _dir) = make_backend().await;
        let workspace_id = seed_two_owners(&backend).await;
        assert_eq!(count_owners(&backend, workspace_id).await, 2);

        let barrier = Arc::new(Barrier::new(2));
        let b1 = Arc::clone(&backend);
        let b2 = Arc::clone(&backend);
        let bar1 = Arc::clone(&barrier);
        let bar2 = Arc::clone(&barrier);

        let (r1, r2) = tokio::join!(
            tokio::spawn(async move {
                bar1.wait().await;
                b1.update_member_role_checked(workspace_id, "alice", "member", Some("alice"))
                    .await
            }),
            tokio::spawn(async move {
                bar2.wait().await;
                b2.update_member_role_checked(workspace_id, "bob", "member", Some("bob"))
                    .await
            })
        );

        let r1 = r1.expect("task 1 panic");
        let r2 = r2.expect("task 2 panic");

        // Exactly one demote must succeed; the loser must either see the
        // last-owner constraint after the serialization point, or observe
        // SQLITE_BUSY from `BEGIN IMMEDIATE` contention (which the caller
        // may legitimately retry). Either way, final owner count == 1.
        let ok_count = usize::from(r1.is_ok()) + usize::from(r2.is_ok());
        let final_owners = count_owners(&backend, workspace_id).await;
        assert!(
            ok_count <= 1,
            "iter {iter}: both demotes succeeded, owner count must violate the invariant: r1={r1:?} r2={r2:?}"
        );
        assert!(
            final_owners >= 1,
            "iter {iter}: workspace has {final_owners} owners, must be >= 1: r1={r1:?} r2={r2:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_remove_of_two_owners_leaves_exactly_one() {
    for iter in 0..16 {
        let (backend, _dir) = make_backend().await;
        let workspace_id = seed_two_owners(&backend).await;
        assert_eq!(count_owners(&backend, workspace_id).await, 2);

        let barrier = Arc::new(Barrier::new(2));
        let b1 = Arc::clone(&backend);
        let b2 = Arc::clone(&backend);
        let bar1 = Arc::clone(&barrier);
        let bar2 = Arc::clone(&barrier);

        let (r1, r2) = tokio::join!(
            tokio::spawn(async move {
                bar1.wait().await;
                b1.remove_workspace_member_checked(workspace_id, "alice")
                    .await
            }),
            tokio::spawn(async move {
                bar2.wait().await;
                b2.remove_workspace_member_checked(workspace_id, "bob")
                    .await
            })
        );

        let r1 = r1.expect("task 1 panic");
        let r2 = r2.expect("task 2 panic");

        let ok_count = usize::from(r1.is_ok()) + usize::from(r2.is_ok());
        let final_owners = count_owners(&backend, workspace_id).await;
        assert!(
            ok_count <= 1,
            "iter {iter}: both removals succeeded, owner count must violate the invariant: r1={r1:?} r2={r2:?}"
        );
        assert!(
            final_owners >= 1,
            "iter {iter}: workspace has {final_owners} owners, must be >= 1: r1={r1:?} r2={r2:?}"
        );
    }
}
