#![cfg(feature = "libsql")]

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
use ironclaw_triggers::{
    LibSqlTriggerRepository, TriggerCompletionPolicy, TriggerError, TriggerId, TriggerRecord,
    TriggerRepository, TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use ironclaw_turns::TurnRunId;
use libsql::params;
use tempfile::tempdir;

fn ts(seconds: i64) -> Timestamp {
    Utc.timestamp_opt(seconds, 0).single().expect("valid ts")
}

fn tenant(value: &str) -> TenantId {
    TenantId::new(value).expect("valid tenant")
}

fn user(value: &str) -> UserId {
    UserId::new(value).expect("valid user")
}

fn sample_record(
    trigger_id: TriggerId,
    tenant_id: TenantId,
    next_run_at: Timestamp,
) -> TriggerRecord {
    TriggerRecord {
        trigger_id,
        tenant_id,
        creator_user_id: user("user-a"),
        agent_id: Some(AgentId::new("agent-a").expect("valid agent")),
        project_id: Some(ProjectId::new("project-a").expect("valid project")),
        name: "daily summary".to_string(),
        source: TriggerSourceKind::Schedule,
        schedule: TriggerSchedule::cron("0 8 * * *").expect("valid cron"),
        completion_policy: TriggerCompletionPolicy::Recurring,
        prompt: "summarize unread mail".to_string(),
        state: TriggerState::Scheduled,
        next_run_at,
        last_run_at: None,
        last_fired_slot: None,
        last_status: None,
        active_fire_slot: None,
        active_run_ref: None,
        created_at: ts(1_704_067_200),
    }
}

async fn build_repo_with_db() -> (
    tempfile::TempDir,
    Arc<libsql::Database>,
    LibSqlTriggerRepository,
) {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("triggers.db");
    let db = Arc::new(
        libsql::Builder::new_local(db_path.display().to_string())
            .build()
            .await
            .expect("build libsql db"),
    );
    let repo = LibSqlTriggerRepository::new(db.clone());
    repo.run_migrations().await.expect("run migrations");
    (dir, db, repo)
}

async fn build_repo() -> (tempfile::TempDir, LibSqlTriggerRepository) {
    let (dir, _db, repo) = build_repo_with_db().await;
    (dir, repo)
}

#[tokio::test]
async fn libsql_repository_round_trip_and_scoped_isolation() {
    let (_dir, repo) = build_repo().await;
    let due = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_200),
    );
    let later = sample_record(
        TriggerId::parse("01J00000000000000000000000").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_260),
    );
    let other_tenant = sample_record(
        TriggerId::parse("01J00000000000000000000001").expect("ulid"),
        tenant("tenant-b"),
        ts(1_704_067_200),
    );

    repo.upsert_trigger(due.clone()).await.expect("insert due");
    repo.upsert_trigger(later.clone())
        .await
        .expect("insert later");
    repo.upsert_trigger(other_tenant.clone())
        .await
        .expect("insert other tenant");

    let fetched = repo
        .get_trigger(tenant("tenant-a"), due.trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");
    assert_eq!(fetched, due);

    assert!(
        repo.get_trigger(tenant("tenant-b"), due.trigger_id)
            .await
            .expect("wrong-tenant lookup")
            .is_none()
    );

    let tenant_records = repo
        .list_triggers(tenant("tenant-a"))
        .await
        .expect("list tenant");
    assert_eq!(
        tenant_records
            .iter()
            .map(|record| record.trigger_id)
            .collect::<Vec<_>>(),
        vec![due.trigger_id, later.trigger_id]
    );

    let removed = repo
        .remove_trigger(tenant("tenant-a"), due.trigger_id)
        .await
        .expect("remove trigger")
        .expect("removed record");
    assert_eq!(removed.trigger_id, due.trigger_id);
    assert!(
        repo.get_trigger(tenant("tenant-a"), due.trigger_id)
            .await
            .expect("lookup removed")
            .is_none()
    );
    assert!(
        repo.get_trigger(tenant("tenant-b"), other_tenant.trigger_id)
            .await
            .expect("lookup other tenant")
            .is_some()
    );
    assert_eq!(
        repo.remove_trigger(tenant("tenant-a"), other_tenant.trigger_id)
            .await
            .expect("wrong-tenant remove"),
        None
    );
    assert!(
        repo.get_trigger(tenant("tenant-b"), other_tenant.trigger_id)
            .await
            .expect("other tenant remains")
            .is_some()
    );
    assert!(
        repo.remove_trigger(tenant("tenant-a"), due.trigger_id)
            .await
            .expect("remove missing trigger")
            .is_none()
    );
}

#[tokio::test]
async fn libsql_repository_round_trip_preserves_optional_run_metadata_and_completion_policy() {
    let (_dir, repo) = build_repo().await;
    let mut record = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_260),
    );
    record.completion_policy = TriggerCompletionPolicy::CompleteAfterFirstFire;
    record.last_run_at = Some(ts(1_704_067_200));
    record.last_fired_slot = Some(ts(1_704_067_140));
    record.last_status = Some(TriggerRunStatus::Error);
    record.active_fire_slot = Some(ts(1_704_067_260));
    record.active_run_ref = Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").unwrap());

    repo.upsert_trigger(record.clone())
        .await
        .expect("insert record with run metadata");

    let fetched = repo
        .get_trigger(tenant("tenant-a"), record.trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");

    assert_eq!(fetched, record);
}

#[tokio::test]
async fn libsql_repository_round_trip_preserves_null_optional_scope_fields() {
    let (_dir, repo) = build_repo().await;
    let mut record = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        ts(1_704_067_260),
    );
    record.agent_id = None;
    record.project_id = None;

    repo.upsert_trigger(record.clone())
        .await
        .expect("insert record with null optional fields");

    let fetched = repo
        .get_trigger(tenant("tenant-a"), record.trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");

    assert_eq!(fetched, record);
}

#[tokio::test]
async fn libsql_repository_upsert_preserves_original_created_at() {
    let (_dir, repo) = build_repo().await;
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let original_created_at = ts(1_704_067_200);
    let mut record = sample_record(trigger_id, tenant_id.clone(), ts(1_704_067_260));
    record.created_at = original_created_at;

    repo.upsert_trigger(record.clone())
        .await
        .expect("insert record");

    let mut update = record;
    update.name = "renamed trigger".to_string();
    update.created_at = ts(1_704_067_900);
    repo.upsert_trigger(update)
        .await
        .expect("update existing record");

    let fetched = repo
        .get_trigger(tenant_id, trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");

    assert_eq!(fetched.name, "renamed trigger");
    assert_eq!(fetched.created_at, original_created_at);
}

#[tokio::test]
async fn libsql_repository_rejects_malformed_persisted_rows() {
    let (_dir, db, repo) = build_repo_with_db().await;
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let record = sample_record(trigger_id, tenant_id.clone(), ts(1_704_067_260));

    repo.upsert_trigger(record).await.expect("insert record");

    let conn = db.connect().expect("connect raw libsql");
    for (column, value, expected_field, read_mode) in [
        ("trigger_id", "not-a-ulid", "invalid length", "list"),
        ("tenant_id", "", "tenant_id", "due"),
        ("creator_user_id", "", "creator_user_id", "get"),
        ("agent_id", "", "agent_id", "get"),
        ("project_id", "", "project_id", "get"),
        ("name", "", "name", "get"),
        ("name", "   ", "name", "get"),
        ("source", "webhook", "source", "get"),
        ("state", "unknown", "state", "get"),
        ("completion_policy", "once", "completion_policy", "get"),
        ("prompt", "", "prompt", "get"),
        ("prompt", "\t  ", "prompt", "get"),
        ("next_run_at", "not-a-timestamp", "next_run_at", "get"),
        ("last_run_at", "not-a-timestamp", "last_run_at", "get"),
        (
            "last_fired_slot",
            "not-a-timestamp",
            "last_fired_slot",
            "get",
        ),
        (
            "active_fire_slot",
            "not-a-timestamp",
            "active_fire_slot",
            "get",
        ),
        ("active_run_ref", "not-a-uuid", "active_run_ref", "get"),
        ("last_status", "timed_out", "last_status", "get"),
    ] {
        conn.execute(
            &format!(
                "UPDATE trigger_records SET {column} = ?1 WHERE tenant_id = ?2 AND trigger_id = ?3"
            ),
            params![value, tenant_id.as_str(), trigger_id.to_string()],
        )
        .await
        .expect("corrupt persisted row");

        let error = match read_mode {
            "get" => repo.get_trigger(tenant_id.clone(), trigger_id).await,
            "list" => repo
                .list_triggers(tenant_id.clone())
                .await
                .map(|records| records.first().cloned()),
            "due" => repo
                .list_due_triggers(ts(1_704_067_260), 10)
                .await
                .map(|records| records.first().cloned()),
            _ => unreachable!("known read mode"),
        }
        .expect_err("malformed row must fail hydration");
        assert!(
            if column == "trigger_id" {
                matches!(
                    error,
                    TriggerError::InvalidTriggerId { ref reason } if reason.contains(expected_field)
                )
            } else {
                matches!(
                    error,
                    TriggerError::InvalidRecord { ref reason } if reason.contains(expected_field)
                )
            },
            "expected malformed {column} to report {expected_field}, got {error:?}"
        );

        conn.execute("DELETE FROM trigger_records", ())
            .await
            .expect("clear malformed row");
        repo.upsert_trigger(sample_record(
            trigger_id,
            tenant_id.clone(),
            ts(1_704_067_260),
        ))
        .await
        .expect("restore valid row");
    }
}

#[tokio::test]
async fn libsql_repository_due_query_clamps_limit_and_respects_state_gate() {
    let (_dir, repo) = build_repo().await;
    let due_slot = ts(1_704_067_200);
    let future = sample_record(
        TriggerId::parse("01J00000000000000000000002").expect("ulid"),
        tenant("tenant-future"),
        ts(1_704_067_320),
    );
    let paused = {
        let mut record = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid"),
            tenant("tenant-paused"),
            due_slot,
        );
        record.state = TriggerState::Paused;
        record
    };
    repo.upsert_trigger(paused.clone())
        .await
        .expect("insert paused");
    repo.upsert_trigger(future.clone())
        .await
        .expect("insert future");
    let completed = {
        let mut record = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZX").expect("ulid"),
            tenant("tenant-completed"),
            due_slot,
        );
        record.state = TriggerState::Completed;
        record
    };
    repo.upsert_trigger(completed.clone())
        .await
        .expect("insert completed");

    let small_a = sample_record(
        TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid"),
        tenant("tenant-a"),
        due_slot,
    );
    let small_b = sample_record(
        TriggerId::parse("01J00000000000000000000000").expect("ulid"),
        tenant("tenant-b"),
        due_slot,
    );
    let small_c = sample_record(
        TriggerId::parse("01J00000000000000000000000").expect("ulid"),
        tenant("tenant-c"),
        due_slot,
    );
    repo.upsert_trigger(small_b.clone())
        .await
        .expect("insert small_b");
    repo.upsert_trigger(small_c.clone())
        .await
        .expect("insert small_c");
    repo.upsert_trigger(small_a.clone())
        .await
        .expect("insert small_a");

    let ordered_due_records = repo
        .list_due_triggers(due_slot, 3)
        .await
        .expect("list due ordered");
    assert_eq!(
        ordered_due_records
            .iter()
            .map(|record| (record.tenant_id.clone(), record.trigger_id))
            .collect::<Vec<_>>(),
        vec![
            (small_a.tenant_id.clone(), small_a.trigger_id),
            (small_b.tenant_id.clone(), small_b.trigger_id),
            (small_c.tenant_id.clone(), small_c.trigger_id),
        ]
    );

    for index in 0..127 {
        let record = sample_record(
            TriggerId::parse("01Z00000000000000000000000").expect("ulid"),
            tenant(&format!("tenant-z-{index:03}")),
            due_slot,
        );
        repo.upsert_trigger(record).await.expect("insert filler");
    }

    assert!(
        repo.list_due_triggers(due_slot, 0)
            .await
            .expect("zero limit")
            .is_empty()
    );

    let due_records = repo
        .list_due_triggers(due_slot, 128 + 10)
        .await
        .expect("list due");
    assert_eq!(due_records.len(), 128);
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == future.tenant_id),
        "future scheduled record must not be returned as due"
    );
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == paused.tenant_id),
        "paused record must not be returned as due"
    );
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == completed.tenant_id),
        "completed record must not be returned as due"
    );
}

#[tokio::test]
async fn libsql_repository_run_migrations_is_idempotent() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("triggers.db");
    let db = Arc::new(
        libsql::Builder::new_local(db_path.display().to_string())
            .build()
            .await
            .expect("build libsql db"),
    );
    let repo = LibSqlTriggerRepository::new(db);

    repo.run_migrations().await.expect("first run migrations");
    repo.run_migrations().await.expect("second run migrations");
}

#[tokio::test]
async fn libsql_repository_rejects_validation_failures_before_persistence() {
    let (_dir, repo) = build_repo().await;
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let next_run_at = ts(1_704_067_200);

    let mut name_error = sample_record(trigger_id, tenant_id.clone(), next_run_at);
    name_error.name.clear();
    assert!(matches!(
        repo.upsert_trigger(name_error).await,
        Err(TriggerError::InvalidRecord { .. })
    ));

    let mut prompt_error = sample_record(trigger_id, tenant_id.clone(), next_run_at);
    prompt_error.prompt.clear();
    assert!(matches!(
        repo.upsert_trigger(prompt_error).await,
        Err(TriggerError::InvalidRecord { .. })
    ));

    let mut schedule_error = sample_record(trigger_id, tenant_id, next_run_at);
    schedule_error.schedule = TriggerSchedule::Cron {
        expression: "*/30 * * * * *".to_string(),
    };
    assert!(matches!(
        repo.upsert_trigger(schedule_error).await,
        Err(TriggerError::InvalidSchedule { .. })
    ));

    assert!(
        repo.list_triggers(tenant("tenant-a"))
            .await
            .expect("list after failures")
            .is_empty()
    );
}

#[tokio::test]
async fn libsql_repository_persists_trigger_state_fire_gate() {
    let (_dir, repo) = build_repo().await;
    let trigger_id = TriggerId::parse("01J00000000000000000000000").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let mut record = sample_record(trigger_id, tenant_id.clone(), ts(1_704_067_200));
    record.state = TriggerState::Paused;

    repo.upsert_trigger(record.clone())
        .await
        .expect("insert paused");

    let fetched = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get paused")
        .expect("paused record");
    assert_eq!(fetched.state, TriggerState::Paused);
    assert_eq!(fetched.schedule, record.schedule);
    assert!(
        repo.list_due_triggers(ts(1_704_067_200), 10)
            .await
            .expect("list due")
            .is_empty()
    );

    record.state = TriggerState::Scheduled;
    repo.upsert_trigger(record.clone())
        .await
        .expect("reactivate");
    let due_records = repo
        .list_due_triggers(ts(1_704_067_200), 10)
        .await
        .expect("list due after reactivation");
    assert_eq!(due_records.len(), 1);
    assert_eq!(due_records[0].state, TriggerState::Scheduled);
    assert_eq!(due_records[0].trigger_id, trigger_id);
}
