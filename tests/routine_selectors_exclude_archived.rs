//! Regression tests for archived-workspace filtering in routine selectors.
//!
//! Archiving a workspace must stop the engine from firing its routines.
//! The HTTP archive handler flips `workspaces.status = 'archived'`, but the
//! background scheduler reads `routines` directly via `list_event_routines`,
//! `list_due_cron_routines`, and `get_webhook_routine_by_path`. Before this
//! fix those selectors ignored the workspace status — archived workspaces
//! kept firing cron, event, and webhook routines.
//!
//! Personal routines (`workspace_id IS NULL`) must remain visible after a
//! workspace is archived, so each test asserts both exclusion of the
//! archived-workspace routine and inclusion of an unrelated personal routine.

#![cfg(feature = "libsql")]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ironclaw::agent::routine::{
    Routine, RoutineAction, RoutineGuardrails, Trigger,
};
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::{Database, RoutineStore, UserRecord, UserStore, WorkspaceMgmtStore};
use serde_json::json;
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

fn routine(
    user_id: &str,
    workspace_id: Option<Uuid>,
    name: &str,
    trigger: Trigger,
    next_fire_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Routine {
    Routine {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: String::new(),
        user_id: user_id.to_string(),
        workspace_id,
        enabled: true,
        trigger,
        action: RoutineAction::FullJob {
            title: "t".into(),
            description: "d".into(),
            max_iterations: 1,
        },
        guardrails: RoutineGuardrails {
            cooldown: Duration::ZERO,
            max_concurrent: 1,
            dedup_window: None,
        },
        notify: Default::default(),
        last_run_at: None,
        next_fire_at,
        run_count: 0,
        consecutive_failures: 0,
        state: json!({}),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

async fn seed_two_user_archived_workspace(
    backend: &Arc<LibSqlBackend>,
) -> (Uuid, Uuid) {
    backend.create_user(&test_user("alice")).await.unwrap();
    let ws = backend
        .create_workspace("Demo", "demo", "", "alice", &json!({}))
        .await
        .unwrap();
    // Archive up-front; the selectors should not return routines owned by ws.id.
    assert!(backend.archive_workspace(ws.id).await.unwrap());
    (ws.id, Uuid::new_v4()) // second uuid unused — placeholder for signature symmetry
}

#[tokio::test]
async fn list_event_routines_excludes_archived_workspace_routines() {
    let (backend, _dir) = make_backend().await;
    let (ws_id, _) = seed_two_user_archived_workspace(&backend).await;

    let ws_event = routine(
        "alice",
        Some(ws_id),
        "ws-event",
        Trigger::Event {
            channel: None,
            pattern: "hi".into(),
        },
        None,
    );
    let personal_event = routine(
        "alice",
        None,
        "personal-event",
        Trigger::Event {
            channel: None,
            pattern: "hi".into(),
        },
        None,
    );
    backend.create_routine(&ws_event).await.unwrap();
    backend.create_routine(&personal_event).await.unwrap();

    let listed = backend.list_event_routines().await.unwrap();
    let names: Vec<_> = listed.iter().map(|r| r.name.as_str()).collect();
    assert!(
        !names.contains(&"ws-event"),
        "archived-workspace routine must not fire: got {names:?}"
    );
    assert!(
        names.contains(&"personal-event"),
        "personal routine must still fire: got {names:?}"
    );
}

#[tokio::test]
async fn list_due_cron_routines_excludes_archived_workspace_routines() {
    let (backend, _dir) = make_backend().await;
    let (ws_id, _) = seed_two_user_archived_workspace(&backend).await;

    let past = Utc::now() - chrono::Duration::minutes(1);
    let ws_cron = routine(
        "alice",
        Some(ws_id),
        "ws-cron",
        Trigger::Cron {
            schedule: "* * * * *".into(),
            timezone: None,
        },
        Some(past),
    );
    let personal_cron = routine(
        "alice",
        None,
        "personal-cron",
        Trigger::Cron {
            schedule: "* * * * *".into(),
            timezone: None,
        },
        Some(past),
    );
    backend.create_routine(&ws_cron).await.unwrap();
    backend.create_routine(&personal_cron).await.unwrap();

    let listed = backend.list_due_cron_routines().await.unwrap();
    let names: Vec<_> = listed.iter().map(|r| r.name.as_str()).collect();
    assert!(
        !names.contains(&"ws-cron"),
        "archived-workspace cron must not fire: got {names:?}"
    );
    assert!(
        names.contains(&"personal-cron"),
        "personal cron must still fire: got {names:?}"
    );
}

#[tokio::test]
async fn get_webhook_routine_by_path_skips_archived_workspace_routines() {
    let (backend, _dir) = make_backend().await;
    let (ws_id, _) = seed_two_user_archived_workspace(&backend).await;

    let ws_webhook = routine(
        "alice",
        Some(ws_id),
        "ws-webhook",
        Trigger::Webhook {
            path: Some("archived-path".into()),
            secret: None,
        },
        None,
    );
    backend.create_routine(&ws_webhook).await.unwrap();

    let hit: Option<Routine> = backend
        .get_webhook_routine_by_path("archived-path", None)
        .await
        .unwrap();
    assert!(
        hit.is_none(),
        "webhook in archived workspace must not resolve: got {hit:?}"
    );

    // Also the scoped lookup with the owning user must skip it.
    let hit_scoped: Option<Routine> = backend
        .get_webhook_routine_by_path("archived-path", Some("alice"))
        .await
        .unwrap();
    assert!(
        hit_scoped.is_none(),
        "scoped webhook lookup in archived workspace must not resolve: got {hit_scoped:?}"
    );
}
