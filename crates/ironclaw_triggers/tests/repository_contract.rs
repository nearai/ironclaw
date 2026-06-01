#![cfg(any(feature = "libsql", feature = "postgres"))]

use chrono::{TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, Timestamp, UserId};
use ironclaw_triggers::{
    InMemoryTriggerRepository, TriggerCompletionPolicy, TriggerError, TriggerId, TriggerRecord,
    TriggerRepository, TriggerRunStatus, TriggerSchedule, TriggerSourceKind, TriggerState,
};
use ironclaw_turns::TurnRunId;

#[cfg(feature = "libsql")]
use {
    ironclaw_triggers::LibSqlTriggerRepository, libsql::params, std::sync::Arc, tempfile::tempdir,
};

#[cfg(feature = "postgres")]
use ironclaw_triggers::PostgresTriggerRepository;

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

async fn assert_round_trip_and_scoped_isolation(repo: &impl TriggerRepository) {
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

async fn assert_round_trip_preserves_optional_run_metadata_and_completion_policy(
    repo: &impl TriggerRepository,
) {
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

async fn assert_round_trip_preserves_null_optional_scope_fields(repo: &impl TriggerRepository) {
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

async fn assert_upsert_preserves_original_created_at(repo: &impl TriggerRepository) {
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

async fn assert_due_query_clamps_limit_and_respects_state_gate(repo: &impl TriggerRepository) {
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
    let completed = {
        let mut record = sample_record(
            TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZX").expect("ulid"),
            tenant("tenant-completed"),
            due_slot,
        );
        record.state = TriggerState::Completed;
        record
    };
    let active_claim = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000004").expect("ulid"),
            tenant("tenant-active-claim"),
            due_slot,
        );
        record.active_fire_slot = Some(due_slot);
        record
    };
    let active_run_claim = {
        let mut record = sample_record(
            TriggerId::parse("01J00000000000000000000005").expect("ulid"),
            tenant("tenant-active-run"),
            due_slot,
        );
        record.active_run_ref =
            Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run"));
        record
    };
    repo.upsert_trigger(paused.clone())
        .await
        .expect("insert paused");
    repo.upsert_trigger(future.clone())
        .await
        .expect("insert future");
    repo.upsert_trigger(completed.clone())
        .await
        .expect("insert completed");
    repo.upsert_trigger(active_claim.clone())
        .await
        .expect("insert active claim");
    repo.upsert_trigger(active_run_claim.clone())
        .await
        .expect("insert active run claim");

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
    let same_tenant_low = sample_record(
        TriggerId::parse("01J00000000000000000000001").expect("ulid"),
        tenant("tenant-d"),
        due_slot,
    );
    let same_tenant_high = sample_record(
        TriggerId::parse("01J00000000000000000000003").expect("ulid"),
        tenant("tenant-d"),
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
    repo.upsert_trigger(same_tenant_high.clone())
        .await
        .expect("insert same_tenant_high");
    repo.upsert_trigger(same_tenant_low.clone())
        .await
        .expect("insert same_tenant_low");

    let ordered_due_records = repo
        .list_due_triggers(due_slot, 5)
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
            (
                same_tenant_low.tenant_id.clone(),
                same_tenant_low.trigger_id
            ),
            (
                same_tenant_high.tenant_id.clone(),
                same_tenant_high.trigger_id
            ),
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
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == active_claim.tenant_id),
        "active fire claim must not be returned as due"
    );
    assert!(
        !due_records
            .iter()
            .any(|record| record.tenant_id == active_run_claim.tenant_id),
        "active run claim must not be returned as due"
    );
}

async fn assert_rejects_validation_failures_before_persistence(repo: &impl TriggerRepository) {
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

async fn assert_persists_trigger_state_fire_gate(repo: &impl TriggerRepository) {
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

#[cfg(feature = "libsql")]
async fn build_libsql_repo_with_db() -> (
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

#[cfg(feature = "libsql")]
async fn build_libsql_repo() -> (tempfile::TempDir, LibSqlTriggerRepository) {
    let (dir, _db, repo) = build_libsql_repo_with_db().await;
    (dir, repo)
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_contract_parity() {
    let (_dir, repo) = build_libsql_repo().await;
    assert_round_trip_and_scoped_isolation(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_round_trip_preserves_optional_run_metadata_and_completion_policy(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_round_trip_preserves_null_optional_scope_fields(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_upsert_preserves_original_created_at(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_due_query_clamps_limit_and_respects_state_gate(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_rejects_validation_failures_before_persistence(&repo).await;

    let (_dir, repo) = build_libsql_repo().await;
    assert_persists_trigger_state_fire_gate(&repo).await;
}

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_rejects_malformed_persisted_rows() {
    let (_dir, db, repo) = build_libsql_repo_with_db().await;
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let record = sample_record(trigger_id, tenant_id.clone(), ts(1_704_067_260));

    repo.upsert_trigger(record).await.expect("insert record");

    let conn = db.connect().expect("connect raw libsql");
    for (column, value, expected_field, read_mode) in malformed_row_cases() {
        conn.execute(
            &format!(
                "UPDATE trigger_records SET {column} = ?1 WHERE tenant_id = ?2 AND trigger_id = ?3"
            ),
            params![value, tenant_id.as_str(), trigger_id.to_string()],
        )
        .await
        .expect("corrupt persisted row");

        assert_malformed_row_error(
            &repo,
            tenant_id.clone(),
            trigger_id,
            expected_field,
            read_mode,
        )
        .await;

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

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_repository_contract_parity() {
    let Some((_container, pool)) = postgres_pool_or_skip().await else {
        return;
    };
    let repo = PostgresTriggerRepository::new(pool.clone());
    repo.run_migrations().await.expect("run migrations");
    assert_round_trip_and_scoped_isolation(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_round_trip_preserves_optional_run_metadata_and_completion_policy(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_round_trip_preserves_null_optional_scope_fields(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_upsert_preserves_original_created_at(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_due_query_clamps_limit_and_respects_state_gate(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_rejects_validation_failures_before_persistence(&repo).await;

    clear_postgres_triggers(&pool).await;
    assert_persists_trigger_state_fire_gate(&repo).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_repository_run_migrations_is_idempotent() {
    let Some((_container, pool)) = postgres_pool_or_skip().await else {
        return;
    };
    let repo = PostgresTriggerRepository::new(pool);

    repo.run_migrations().await.expect("first run migrations");
    repo.run_migrations().await.expect("second run migrations");
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_repository_rejects_malformed_persisted_rows() {
    let Some((_container, pool)) = postgres_pool_or_skip().await else {
        return;
    };
    let repo = PostgresTriggerRepository::new(pool.clone());
    repo.run_migrations().await.expect("run migrations");
    let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
    let tenant_id = tenant("tenant-a");
    let record = sample_record(trigger_id, tenant_id.clone(), ts(1_704_067_260));

    repo.upsert_trigger(record).await.expect("insert record");

    let client = pool.get().await.expect("postgres connection");
    for (column, value, expected_field, read_mode) in malformed_row_cases() {
        client
            .execute(
                &format!(
                    "UPDATE trigger_records SET {column} = $1 WHERE tenant_id = $2 AND trigger_id = $3"
                ),
                &[&value, &tenant_id.as_str(), &trigger_id.to_string()],
            )
            .await
            .expect("corrupt persisted row");

        assert_malformed_row_error(
            &repo,
            tenant_id.clone(),
            trigger_id,
            expected_field,
            read_mode,
        )
        .await;

        client
            .execute("DELETE FROM trigger_records", &[])
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

#[derive(Clone, Copy)]
enum ReadMode {
    Get,
    List,
    Due,
    Remove,
}

fn malformed_row_cases() -> Vec<(&'static str, &'static str, &'static str, ReadMode)> {
    use ReadMode::{Due, Get, List, Remove};

    [
        ("trigger_id", "not-a-ulid", "invalid length", List),
        ("tenant_id", "", "tenant_id", Due),
        ("creator_user_id", "", "creator_user_id", Remove),
        ("creator_user_id", "", "creator_user_id", Get),
        ("agent_id", "", "agent_id", Get),
        ("project_id", "", "project_id", Get),
        ("name", "", "name", Get),
        ("name", "   ", "name", Get),
        ("source", "webhook", "source", Get),
        ("schedule_expression", "*/30 * * * * *", "schedule", Get),
        ("state", "unknown", "state", Get),
        ("completion_policy", "once", "completion_policy", Get),
        ("prompt", "", "prompt", Get),
        ("prompt", "\t  ", "prompt", Get),
        ("next_run_at", "not-a-timestamp", "next_run_at", Get),
        ("last_run_at", "not-a-timestamp", "last_run_at", Get),
        ("last_fired_slot", "not-a-timestamp", "last_fired_slot", Get),
        (
            "active_fire_slot",
            "not-a-timestamp",
            "active_fire_slot",
            Get,
        ),
        ("active_run_ref", "not-a-uuid", "active_run_ref", Get),
        ("last_status", "timed_out", "last_status", Get),
        ("created_at", "not-a-timestamp", "created_at", Get),
    ]
    .into()
}

async fn assert_malformed_row_error(
    repo: &impl TriggerRepository,
    tenant_id: TenantId,
    trigger_id: TriggerId,
    expected_field: &str,
    read_mode: ReadMode,
) {
    let error = match read_mode {
        ReadMode::Get => repo.get_trigger(tenant_id.clone(), trigger_id).await,
        ReadMode::List => repo
            .list_triggers(tenant_id.clone())
            .await
            .map(|records| records.first().cloned()),
        ReadMode::Due => repo
            .list_due_triggers(ts(1_704_067_260), 10)
            .await
            .map(|records| records.first().cloned()),
        ReadMode::Remove => repo.remove_trigger(tenant_id.clone(), trigger_id).await,
    }
    .expect_err("malformed row must fail hydration");
    assert!(
        if expected_field == "invalid length" {
            matches!(
                error,
                TriggerError::InvalidTriggerId { ref reason } if reason.contains(expected_field)
            )
        } else if expected_field == "schedule" {
            matches!(error, TriggerError::InvalidSchedule { .. })
        } else {
            matches!(
                error,
                TriggerError::InvalidRecord { ref reason } if reason.contains(expected_field)
            )
        },
        "expected malformed row to report {expected_field}, got {error:?}"
    );
}

#[cfg(feature = "postgres")]
async fn postgres_pool_or_skip() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    deadpool_postgres::Pool,
)> {
    if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
        eprintln!(
            "skipping Postgres trigger repository tests: IRONCLAW_SKIP_POSTGRES_TESTS is set"
        );
        return None;
    }

    // Test-only bootstrap: production composition must pass a constructed pool
    // into PostgresTriggerRepository and keep URL parsing out of this crate.
    let (container, database_url) = start_postgres_container().await?;
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("testcontainer database URL must parse");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("Postgres pool must build");
    if let Err(error) = pool.get().await {
        eprintln!("skipping Postgres trigger repository tests: database unavailable ({error})");
        return None;
    }
    Some((container, pool))
}

#[cfg(feature = "postgres")]
async fn start_postgres_container() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<
        testcontainers_modules::postgres::Postgres,
    >,
    String,
)> {
    use testcontainers_modules::testcontainers::{ImageExt, runners::AsyncRunner};

    let image = testcontainers_modules::postgres::Postgres::default()
        .with_db_name("ironclaw_test")
        .with_user("postgres")
        .with_password("postgres")
        .with_tag("16-alpine");

    let container = match image.start().await {
        Ok(container) => container,
        Err(error) => {
            eprintln!(
                "skipping Postgres trigger repository tests: docker/testcontainers unavailable ({error})"
            );
            return None;
        }
    };
    let host = match container.get_host().await {
        Ok(host) => host,
        Err(error) => {
            eprintln!(
                "skipping Postgres trigger repository tests: could not resolve container host ({error})"
            );
            return None;
        }
    };
    let port = match container.get_host_port_ipv4(5432).await {
        Ok(port) => port,
        Err(error) => {
            eprintln!(
                "skipping Postgres trigger repository tests: could not resolve container port ({error})"
            );
            return None;
        }
    };
    Some((
        container,
        format!("postgres://postgres:postgres@{host}:{port}/ironclaw_test"),
    ))
}

#[cfg(feature = "postgres")]
async fn clear_postgres_triggers(pool: &deadpool_postgres::Pool) {
    pool.get()
        .await
        .expect("postgres connection")
        .execute("DELETE FROM trigger_records", &[])
        .await
        .expect("clear trigger records");
}

mod fire_claim_contract {
    use super::*;

    use ironclaw_triggers::{
        ClaimDueFireOutcome, ClaimDueFireRequest, FireAcceptedRequest, FirePermanentFailedRequest,
        FireReplayedRequest, FireRetryableFailedRequest,
    };

    async fn assert_fire_claim_and_update_contract(repo: &impl TriggerRepository) {
        let trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let tenant_id = tenant("tenant-a");
        let fire_slot = ts(1_704_067_200);
        let accepted_at = ts(1_704_067_205);
        let mut record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        let expected_next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next slot calculation")
            .expect("future slot");
        record.last_status = Some(TriggerRunStatus::Error);
        repo.upsert_trigger(record.clone())
            .await
            .expect("insert record");

        let claimed = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim due fire");
        let ClaimDueFireOutcome::Claimed(claimed) = claimed else {
            panic!("record should be claimable, got {claimed:?}");
        };
        assert_eq!(claimed.record.active_fire_slot, Some(fire_slot));
        assert_eq!(claimed.record.active_run_ref, None);

        let persisted = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("reload claimed record")
            .expect("record present");
        assert_eq!(persisted.active_fire_slot, Some(fire_slot));
        assert_eq!(persisted.active_run_ref, None);
        assert_eq!(persisted.last_status, Some(TriggerRunStatus::Error));

        let accepted_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run");
        let accepted = repo
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                run_id: accepted_run_id,
                submitted_at: accepted_at,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("mark accepted")
            .expect("accepted fire should persist");
        assert_eq!(accepted.last_run_at, Some(accepted_at));
        assert_eq!(accepted.last_fired_slot, Some(fire_slot));
        assert_eq!(accepted.last_status, Some(TriggerRunStatus::Ok));
        assert_eq!(accepted.active_fire_slot, Some(fire_slot));
        assert_eq!(accepted.active_run_ref, Some(accepted_run_id));
        assert_eq!(accepted.next_run_at, expected_next_run_at);

        let accepted_again = repo
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                run_id: accepted_run_id,
                submitted_at: ts(1_704_067_206),
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("idempotent accepted result")
            .expect("accepted result returns existing record");
        assert_eq!(accepted_again, accepted);

        let different_accepted_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5c").expect("valid run");
        let error = repo
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                run_id: different_accepted_run_id,
                submitted_at: accepted_at,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect_err("different accepted run id must not rewrite active_run_ref");
        assert_error_contains(error, "must not rewrite an existing active_run_ref");

        let error = repo
            .mark_fire_retryable_failed(FireRetryableFailedRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
            })
            .await
            .expect_err("stale retryable failure must not clear accepted run ref");
        assert_error_contains(error, "must not clear an accepted active_run_ref");

        let replayed_trigger_id = TriggerId::parse("01J00000000000000000000006").expect("ulid");
        let replayed_tenant_id = tenant("tenant-replayed");
        let replayed_record =
            sample_record(replayed_trigger_id, replayed_tenant_id.clone(), fire_slot);
        repo.upsert_trigger(replayed_record)
            .await
            .expect("insert replayed record");
        let replayed_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: replayed_tenant_id.clone(),
                trigger_id: replayed_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim replayed record");
        assert!(matches!(replayed_claim, ClaimDueFireOutcome::Claimed(_)));

        let replayed_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5b").expect("valid run");
        let replayed = repo
            .mark_fire_replayed(FireReplayedRequest {
                tenant_id: replayed_tenant_id.clone(),
                trigger_id: replayed_trigger_id,
                fire_slot,
                original_run_id: replayed_run_id,
                replayed_at: accepted_at,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("mark replayed")
            .expect("replayed fire should persist");
        assert_eq!(replayed.last_run_at, Some(accepted_at));
        assert_eq!(replayed.last_fired_slot, Some(fire_slot));
        assert_eq!(replayed.last_status, Some(TriggerRunStatus::Ok));
        assert_eq!(replayed.active_fire_slot, Some(fire_slot));
        assert_eq!(replayed.active_run_ref, Some(replayed_run_id));
        assert_eq!(replayed.next_run_at, expected_next_run_at);

        let replayed_again = repo
            .mark_fire_replayed(FireReplayedRequest {
                tenant_id: replayed_tenant_id.clone(),
                trigger_id: replayed_trigger_id,
                fire_slot,
                original_run_id: replayed_run_id,
                replayed_at: ts(1_704_067_207),
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("idempotent replayed result")
            .expect("replayed result returns existing record");
        assert_eq!(replayed_again, replayed);

        let different_replayed_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5d").expect("valid run");
        let error = repo
            .mark_fire_replayed(FireReplayedRequest {
                tenant_id: replayed_tenant_id.clone(),
                trigger_id: replayed_trigger_id,
                fire_slot,
                original_run_id: different_replayed_run_id,
                replayed_at: accepted_at,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect_err("different replayed run id must not rewrite active_run_ref");
        assert_error_contains(error, "must not rewrite an existing active_run_ref");

        let error = repo
            .mark_fire_permanently_failed(FirePermanentFailedRequest {
                tenant_id: replayed_tenant_id,
                trigger_id: replayed_trigger_id,
                fire_slot,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect_err("stale permanent failure must not clear replayed run ref");
        assert_error_contains(error, "must not clear an accepted active_run_ref");

        let failure_previous_run_at = ts(1_704_066_900);
        let failure_previous_slot = ts(1_704_066_840);
        let retryable_trigger_id = TriggerId::parse("01J00000000000000000000004").expect("ulid");
        let retryable_tenant_id = tenant("tenant-retryable");
        let mut retryable_record =
            sample_record(retryable_trigger_id, retryable_tenant_id.clone(), fire_slot);
        retryable_record.last_run_at = Some(failure_previous_run_at);
        retryable_record.last_fired_slot = Some(failure_previous_slot);
        repo.upsert_trigger(retryable_record)
            .await
            .expect("insert retryable record");
        let retryable_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: retryable_tenant_id.clone(),
                trigger_id: retryable_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim retryable record");
        assert!(matches!(retryable_claim, ClaimDueFireOutcome::Claimed(_)));

        let retryable_failed = repo
            .mark_fire_retryable_failed(FireRetryableFailedRequest {
                tenant_id: retryable_tenant_id,
                trigger_id: retryable_trigger_id,
                fire_slot,
            })
            .await
            .expect("mark retryable failed")
            .expect("retryable failure should persist");
        assert_eq!(retryable_failed.last_run_at, Some(failure_previous_run_at));
        assert_eq!(
            retryable_failed.last_fired_slot,
            Some(failure_previous_slot)
        );
        assert_eq!(retryable_failed.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(retryable_failed.active_fire_slot, None);
        assert_eq!(retryable_failed.active_run_ref, None);
        assert_eq!(retryable_failed.next_run_at, fire_slot);

        let permanent_trigger_id = TriggerId::parse("01J00000000000000000000005").expect("ulid");
        let permanent_tenant_id = tenant("tenant-permanent");
        let mut permanent_record =
            sample_record(permanent_trigger_id, permanent_tenant_id.clone(), fire_slot);
        permanent_record.last_run_at = Some(failure_previous_run_at);
        permanent_record.last_fired_slot = Some(failure_previous_slot);
        repo.upsert_trigger(permanent_record)
            .await
            .expect("insert permanent-failure record");
        let permanent_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: permanent_tenant_id.clone(),
                trigger_id: permanent_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim permanent-failure record");
        assert!(matches!(permanent_claim, ClaimDueFireOutcome::Claimed(_)));

        let permanent_failed = repo
            .mark_fire_permanently_failed(FirePermanentFailedRequest {
                tenant_id: permanent_tenant_id,
                trigger_id: permanent_trigger_id,
                fire_slot,
                next_run_at: expected_next_run_at,
            })
            .await
            .expect("mark permanently failed")
            .expect("permanent failure should persist");
        assert_eq!(permanent_failed.last_run_at, Some(failure_previous_run_at));
        assert_eq!(
            permanent_failed.last_fired_slot,
            Some(failure_previous_slot)
        );
        assert_eq!(permanent_failed.last_status, Some(TriggerRunStatus::Error));
        assert_eq!(permanent_failed.active_fire_slot, None);
        assert_eq!(permanent_failed.active_run_ref, None);
        assert!(permanent_failed.next_run_at > fire_slot);
    }

    async fn assert_fire_result_rejects_invalid_next_run_at(repo: &impl TriggerRepository) {
        let fire_slot = ts(1_704_067_200);
        let stale_fire_slot = ts(1_704_067_140);

        let early_claim_trigger_id = TriggerId::parse("01J0000000000000000000000D").expect("ulid");
        let early_claim_tenant_id = tenant("tenant-early-claim");
        repo.upsert_trigger(sample_record(
            early_claim_trigger_id,
            early_claim_tenant_id.clone(),
            fire_slot,
        ))
        .await
        .expect("insert early-claim record");
        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: early_claim_tenant_id,
                trigger_id: early_claim_trigger_id,
                fire_slot,
                now: ts(1_704_067_199),
            })
            .await
            .expect("early claim")
            .matches_not_due(),
            "scheduled row must not be claimable before the requested fire slot is due"
        );

        let stale_accepted_trigger_id =
            TriggerId::parse("01J00000000000000000000010").expect("ulid");
        let stale_accepted_tenant_id = tenant("tenant-stale-accepted");
        let mut stale_accepted_record = sample_record(
            stale_accepted_trigger_id,
            stale_accepted_tenant_id.clone(),
            fire_slot,
        );
        stale_accepted_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(stale_accepted_record.clone())
            .await
            .expect("insert stale accepted record");
        assert!(
            repo.mark_fire_accepted(FireAcceptedRequest {
                tenant_id: stale_accepted_tenant_id.clone(),
                trigger_id: stale_accepted_trigger_id,
                fire_slot: stale_fire_slot,
                run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f62")
                    .expect("valid run"),
                submitted_at: fire_slot,
                next_run_at: ts(1_704_067_260),
            })
            .await
            .expect("stale accepted update")
            .is_none()
        );
        let reloaded = repo
            .get_trigger(stale_accepted_tenant_id, stale_accepted_trigger_id)
            .await
            .expect("reload stale accepted")
            .expect("record present");
        assert_eq!(reloaded, stale_accepted_record);

        let stale_replayed_trigger_id =
            TriggerId::parse("01J00000000000000000000011").expect("ulid");
        let stale_replayed_tenant_id = tenant("tenant-stale-replayed");
        let mut stale_replayed_record = sample_record(
            stale_replayed_trigger_id,
            stale_replayed_tenant_id.clone(),
            fire_slot,
        );
        stale_replayed_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(stale_replayed_record.clone())
            .await
            .expect("insert stale replayed record");
        assert!(
            repo.mark_fire_replayed(FireReplayedRequest {
                tenant_id: stale_replayed_tenant_id.clone(),
                trigger_id: stale_replayed_trigger_id,
                fire_slot: stale_fire_slot,
                original_run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f63")
                    .expect("valid run"),
                replayed_at: fire_slot,
                next_run_at: ts(1_704_067_260),
            })
            .await
            .expect("stale replayed update")
            .is_none()
        );
        let reloaded = repo
            .get_trigger(stale_replayed_tenant_id, stale_replayed_trigger_id)
            .await
            .expect("reload stale replayed")
            .expect("record present");
        assert_eq!(reloaded, stale_replayed_record);

        let stale_retryable_trigger_id =
            TriggerId::parse("01J00000000000000000000012").expect("ulid");
        let stale_retryable_tenant_id = tenant("tenant-stale-retryable");
        let mut stale_retryable_record = sample_record(
            stale_retryable_trigger_id,
            stale_retryable_tenant_id.clone(),
            fire_slot,
        );
        stale_retryable_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(stale_retryable_record.clone())
            .await
            .expect("insert stale retryable record");
        assert!(
            repo.mark_fire_retryable_failed(FireRetryableFailedRequest {
                tenant_id: stale_retryable_tenant_id.clone(),
                trigger_id: stale_retryable_trigger_id,
                fire_slot: stale_fire_slot,
            })
            .await
            .expect("stale retryable update")
            .is_none()
        );
        let reloaded = repo
            .get_trigger(stale_retryable_tenant_id, stale_retryable_trigger_id)
            .await
            .expect("reload stale retryable")
            .expect("record present");
        assert_eq!(reloaded, stale_retryable_record);

        let stale_permanent_trigger_id =
            TriggerId::parse("01J00000000000000000000013").expect("ulid");
        let stale_permanent_tenant_id = tenant("tenant-stale-permanent");
        let mut stale_permanent_record = sample_record(
            stale_permanent_trigger_id,
            stale_permanent_tenant_id.clone(),
            fire_slot,
        );
        stale_permanent_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(stale_permanent_record.clone())
            .await
            .expect("insert stale permanent record");
        assert!(
            repo.mark_fire_permanently_failed(FirePermanentFailedRequest {
                tenant_id: stale_permanent_tenant_id.clone(),
                trigger_id: stale_permanent_trigger_id,
                fire_slot: stale_fire_slot,
                next_run_at: ts(1_704_067_260),
            })
            .await
            .expect("stale permanent update")
            .is_none()
        );
        let reloaded = repo
            .get_trigger(stale_permanent_tenant_id, stale_permanent_trigger_id)
            .await
            .expect("reload stale permanent")
            .expect("record present");
        assert_eq!(reloaded, stale_permanent_record);

        let accepted_trigger_id = TriggerId::parse("01J0000000000000000000000E").expect("ulid");
        let accepted_tenant_id = tenant("tenant-invalid-accepted");
        let mut accepted_record =
            sample_record(accepted_trigger_id, accepted_tenant_id.clone(), fire_slot);
        accepted_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(accepted_record)
            .await
            .expect("insert invalid accepted record");
        let error = repo
            .mark_fire_accepted(FireAcceptedRequest {
                tenant_id: accepted_tenant_id,
                trigger_id: accepted_trigger_id,
                fire_slot,
                run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f60")
                    .expect("valid run"),
                submitted_at: fire_slot,
                next_run_at: fire_slot,
            })
            .await
            .expect_err("accepted result rejects non-future next_run_at");
        assert_error_contains(error, "must be after the claimed fire slot");

        let replayed_trigger_id = TriggerId::parse("01J0000000000000000000000F").expect("ulid");
        let replayed_tenant_id = tenant("tenant-invalid-replayed");
        let mut replayed_record =
            sample_record(replayed_trigger_id, replayed_tenant_id.clone(), fire_slot);
        replayed_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(replayed_record)
            .await
            .expect("insert invalid replayed record");
        let error = repo
            .mark_fire_replayed(FireReplayedRequest {
                tenant_id: replayed_tenant_id,
                trigger_id: replayed_trigger_id,
                fire_slot,
                original_run_id: TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f61")
                    .expect("valid run"),
                replayed_at: fire_slot,
                next_run_at: fire_slot,
            })
            .await
            .expect_err("replayed result rejects non-future next_run_at");
        assert_error_contains(error, "must be after the claimed fire slot");

        let retryable_trigger_id = TriggerId::parse("01J00000000000000000000007").expect("ulid");
        let retryable_tenant_id = tenant("tenant-invalid-retryable");
        let mut retryable_record =
            sample_record(retryable_trigger_id, retryable_tenant_id.clone(), fire_slot);
        retryable_record.active_fire_slot = Some(fire_slot);
        retryable_record.next_run_at = ts(1_704_067_260);
        repo.upsert_trigger(retryable_record)
            .await
            .expect("insert invalid retryable record");
        let error = repo
            .mark_fire_retryable_failed(FireRetryableFailedRequest {
                tenant_id: retryable_tenant_id,
                trigger_id: retryable_trigger_id,
                fire_slot,
            })
            .await
            .expect_err("retryable failure rejects advanced next_run_at");
        assert_error_contains(error, "at or before the failed fire slot");

        let permanent_trigger_id = TriggerId::parse("01J00000000000000000000008").expect("ulid");
        let permanent_tenant_id = tenant("tenant-invalid-permanent");
        let mut permanent_record =
            sample_record(permanent_trigger_id, permanent_tenant_id.clone(), fire_slot);
        permanent_record.active_fire_slot = Some(fire_slot);
        repo.upsert_trigger(permanent_record)
            .await
            .expect("insert invalid permanent record");
        let error = repo
            .mark_fire_permanently_failed(FirePermanentFailedRequest {
                tenant_id: permanent_tenant_id,
                trigger_id: permanent_trigger_id,
                fire_slot,
                next_run_at: fire_slot,
            })
            .await
            .expect_err("permanent failure rejects non-future next_run_at");
        assert_error_contains(error, "must be after the claimed fire slot");
    }

    async fn assert_fire_claim_exclusions_and_active_gate_contract(repo: &impl TriggerRepository) {
        let fire_slot = ts(1_704_067_200);
        let tenant_id = tenant("tenant-a");

        let base_trigger_id = TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZZ").expect("ulid");
        let base = sample_record(base_trigger_id, tenant_id.clone(), fire_slot);
        repo.upsert_trigger(base.clone())
            .await
            .expect("insert base");

        let paused = {
            let mut record = sample_record(
                TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZY").expect("ulid"),
                tenant("tenant-paused"),
                fire_slot,
            );
            record.state = TriggerState::Paused;
            record
        };
        repo.upsert_trigger(paused.clone())
            .await
            .expect("insert paused");

        let paused_active = {
            let mut record = sample_record(
                TriggerId::parse("01J0000000000000000000000B").expect("ulid"),
                tenant("tenant-paused-active"),
                fire_slot,
            );
            record.state = TriggerState::Paused;
            record.active_run_ref =
                Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5f").expect("valid run"));
            record
        };
        repo.upsert_trigger(paused_active.clone())
            .await
            .expect("insert paused active");

        let completed = {
            let mut record = sample_record(
                TriggerId::parse("01HZZZZZZZZZZZZZZZZZZZZZZX").expect("ulid"),
                tenant("tenant-completed"),
                fire_slot,
            );
            record.state = TriggerState::Completed;
            record
        };
        repo.upsert_trigger(completed.clone())
            .await
            .expect("insert completed");

        let future = sample_record(
            TriggerId::parse("01J00000000000000000000002").expect("ulid"),
            tenant("tenant-future"),
            ts(1_704_067_320),
        );
        repo.upsert_trigger(future.clone())
            .await
            .expect("insert future");

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: base_trigger_id,
                fire_slot: ts(1_704_067_260),
                now: fire_slot,
            })
            .await
            .expect("wrong fire slot claim")
            .matches_not_due(),
            "wrong fire slot must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: TriggerId::parse("01J00000000000000000000009").expect("ulid"),
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("missing row claim")
            .matches_not_found(),
            "missing row must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: paused.tenant_id.clone(),
                trigger_id: paused.trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("paused claim")
            .matches_not_due(),
            "paused row must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: paused_active.tenant_id.clone(),
                trigger_id: paused_active.trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("paused active claim")
            .matches_not_due(),
            "paused row with stale active metadata must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: completed.tenant_id.clone(),
                trigger_id: completed.trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("completed claim")
            .matches_not_due(),
            "completed row must not be claimable"
        );

        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: future.tenant_id.clone(),
                trigger_id: future.trigger_id,
                fire_slot: future.next_run_at,
                now: fire_slot,
            })
            .await
            .expect("future claim")
            .matches_not_due(),
            "future next_run_at must not be claimable"
        );

        let claimed = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: base_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim base row");
        let ClaimDueFireOutcome::Claimed(claimed) = claimed else {
            panic!("base row should be claimable, got {claimed:?}");
        };
        assert_eq!(claimed.record.active_fire_slot, Some(fire_slot));
        assert_eq!(claimed.record.active_run_ref, None);

        let mut active_fire = claimed.record.clone();
        active_fire.last_status = Some(TriggerRunStatus::Error);
        repo.upsert_trigger(active_fire)
            .await
            .expect("persist active row with error status");
        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: base_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("active fire slot claim")
            .matches_already_active(Some(fire_slot), None),
            "active fire slot must block a second claim"
        );

        let active_run_ref =
            Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5a").expect("valid run"));
        let mut active_run = claimed.record.clone();
        active_run.active_run_ref = active_run_ref;
        repo.upsert_trigger(active_run)
            .await
            .expect("persist active row with run ref");
        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id: base_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("active run ref claim")
            .matches_already_active(Some(fire_slot), active_run_ref),
            "active run ref must block a second claim"
        );

        let run_only_trigger_id = TriggerId::parse("01J0000000000000000000000A").expect("ulid");
        let run_only_tenant_id = tenant("tenant-run-only");
        let mut run_only =
            sample_record(run_only_trigger_id, run_only_tenant_id.clone(), fire_slot);
        run_only.active_fire_slot = None;
        run_only.active_run_ref =
            Some(TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f5e").expect("valid run"));
        assert!(run_only.has_active_fire());
        let expected_run_only_ref = run_only.active_run_ref;
        repo.upsert_trigger(run_only)
            .await
            .expect("persist active row with run ref only");
        assert!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: run_only_tenant_id,
                trigger_id: run_only_trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("active run-ref-only claim")
            .matches_already_active(None, expected_run_only_ref),
            "active run ref without fire slot must block a second claim"
        );

        let mut status_only = sample_record(
            TriggerId::parse("01J00000000000000000000003").expect("ulid"),
            tenant("tenant-status"),
            fire_slot,
        );
        status_only.last_status = Some(TriggerRunStatus::Error);
        repo.upsert_trigger(status_only.clone())
            .await
            .expect("insert status-only record");
        let status_claim = repo
            .claim_due_fire(ClaimDueFireRequest {
                tenant_id: status_only.tenant_id.clone(),
                trigger_id: status_only.trigger_id,
                fire_slot,
                now: ts(1_704_067_260),
            })
            .await
            .expect("status-only claim");
        let ClaimDueFireOutcome::Claimed(status_claim) = status_claim else {
            panic!("status-only row should still be claimable, got {status_claim:?}");
        };
        assert_eq!(status_claim.record.active_fire_slot, Some(fire_slot));
        assert_eq!(status_claim.record.active_run_ref, None);
        assert_eq!(
            status_claim.record.last_status,
            Some(TriggerRunStatus::Error)
        );
    }

    async fn assert_durable_claim_is_atomic<R>(repo: std::sync::Arc<R>)
    where
        R: TriggerRepository + 'static,
    {
        let trigger_id = TriggerId::parse("01J0000000000000000000000C").expect("ulid");
        let tenant_id = tenant("tenant-atomic");
        let fire_slot = ts(1_704_067_200);
        let now = fire_slot;
        repo.upsert_trigger(sample_record(trigger_id, tenant_id.clone(), fire_slot))
            .await
            .expect("insert atomic record");

        let first_repo = repo.clone();
        let second_repo = repo.clone();
        let first_tenant_id = tenant_id.clone();
        let second_tenant_id = tenant_id;
        let first = async move {
            tokio::task::yield_now().await;
            first_repo
                .claim_due_fire(ClaimDueFireRequest {
                    tenant_id: first_tenant_id,
                    trigger_id,
                    fire_slot,
                    now,
                })
                .await
        };
        let second = async move {
            tokio::task::yield_now().await;
            second_repo
                .claim_due_fire(ClaimDueFireRequest {
                    tenant_id: second_tenant_id,
                    trigger_id,
                    fire_slot,
                    now,
                })
                .await
        };

        let (first, second) = tokio::join!(first, second);
        let outcomes = [first.expect("first claim"), second.expect("second claim")];

        let claimed = outcomes
            .iter()
            .find_map(|outcome| match outcome {
                ClaimDueFireOutcome::Claimed(claimed) => Some(claimed.clone()),
                _ => None,
            })
            .expect("one poller must claim the fire");
        let already_active_count = outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    ClaimDueFireOutcome::AlreadyActive {
                        active_fire_slot: Some(slot),
                        active_run_ref: None,
                    } if *slot == fire_slot
                )
            })
            .count();

        assert_eq!(
            already_active_count, 1,
            "one poller must observe the active claim"
        );
        assert_eq!(claimed.fire_slot, fire_slot);
        assert_eq!(claimed.record.active_fire_slot, Some(fire_slot));
        assert_eq!(claimed.record.active_run_ref, None);

        let persisted = repo
            .get_trigger(tenant("tenant-atomic"), trigger_id)
            .await
            .expect("reload atomic record")
            .expect("record present");
        assert_eq!(persisted.active_fire_slot, Some(fire_slot));
        assert_eq!(persisted.active_run_ref, None);
    }

    trait ClaimDueFireOutcomeAssertions {
        fn matches_not_found(&self) -> bool;
        fn matches_not_due(&self) -> bool;
        fn matches_already_active(
            &self,
            active_fire_slot: Option<Timestamp>,
            active_run_ref: Option<TurnRunId>,
        ) -> bool;
    }

    impl ClaimDueFireOutcomeAssertions for ClaimDueFireOutcome {
        fn matches_not_found(&self) -> bool {
            matches!(self, Self::NotFound)
        }

        fn matches_not_due(&self) -> bool {
            matches!(self, Self::NotDue { .. })
        }

        fn matches_already_active(
            &self,
            active_fire_slot: Option<Timestamp>,
            active_run_ref: Option<TurnRunId>,
        ) -> bool {
            matches!(
                self,
                Self::AlreadyActive {
                    active_fire_slot: actual_fire_slot,
                    active_run_ref: actual_run_ref,
                } if *actual_fire_slot == active_fire_slot && *actual_run_ref == active_run_ref
            )
        }
    }

    async fn assert_durable_fire_claim_contract(repo: &impl TriggerRepository) {
        assert_fire_claim_and_update_contract(repo).await;
        assert_fire_claim_exclusions_and_active_gate_contract(repo).await;
        assert_fire_result_rejects_invalid_next_run_at(repo).await;
    }

    fn assert_error_contains(error: TriggerError, expected: &str) {
        assert!(
            error.to_string().contains(expected),
            "expected error to contain {expected:?}, got {error}"
        );
    }

    #[tokio::test]
    async fn in_memory_repository_fire_claim_contract() {
        let repo = InMemoryTriggerRepository::default();
        assert_fire_claim_and_update_contract(&repo).await;
        assert_fire_claim_exclusions_and_active_gate_contract(&repo).await;
        assert_fire_result_rejects_invalid_next_run_at(&repo).await;
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_repository_fire_claim_contract() {
        let (_dir, repo) = build_libsql_repo().await;
        assert_durable_fire_claim_contract(&repo).await;
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_repository_fire_claim_is_atomic() {
        let (_dir, repo) = build_libsql_repo().await;
        assert_durable_claim_is_atomic(std::sync::Arc::new(repo)).await;
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_repository_mark_fire_accepted_is_idempotent_under_concurrency() {
        let (_dir, repo) = build_libsql_repo().await;
        let repo = std::sync::Arc::new(repo);
        let trigger_id = TriggerId::parse("01J00000000000000000000014").expect("ulid");
        let tenant_id = tenant("tenant-accepted-concurrent");
        let fire_slot = ts(1_704_067_200);
        let accepted_at = ts(1_704_067_205);
        let record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        let next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next slot calculation")
            .expect("future slot");
        repo.upsert_trigger(record).await.expect("insert record");
        assert!(matches!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim fire"),
            ClaimDueFireOutcome::Claimed(_)
        ));

        let run_id = TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f64").expect("valid run");
        let request = FireAcceptedRequest {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot,
            run_id,
            submitted_at: accepted_at,
            next_run_at,
        };
        let first_repo = repo.clone();
        let second_repo = repo.clone();
        let first_request = request.clone();
        let second_request = request;
        let first = async move {
            tokio::task::yield_now().await;
            first_repo.mark_fire_accepted(first_request).await
        };
        let second = async move {
            tokio::task::yield_now().await;
            second_repo.mark_fire_accepted(second_request).await
        };

        let (first, second) = tokio::join!(first, second);
        let first = first
            .expect("first accepted result")
            .expect("first accepted record");
        let second = second
            .expect("second accepted result")
            .expect("second accepted record");
        assert_eq!(first, second);
        assert_eq!(first.active_fire_slot, Some(fire_slot));
        assert_eq!(first.active_run_ref, Some(run_id));
        assert_eq!(first.last_run_at, Some(accepted_at));
        assert_eq!(first.last_fired_slot, Some(fire_slot));
        assert_eq!(first.last_status, Some(TriggerRunStatus::Ok));

        let persisted = repo
            .get_trigger(tenant_id, trigger_id)
            .await
            .expect("reload accepted result")
            .expect("record present");
        assert_eq!(persisted, first);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn libsql_repository_mark_fire_replayed_is_idempotent_under_concurrency() {
        let (_dir, repo) = build_libsql_repo().await;
        let repo = std::sync::Arc::new(repo);
        let trigger_id = TriggerId::parse("01J00000000000000000000015").expect("ulid");
        let tenant_id = tenant("tenant-replayed-concurrent");
        let fire_slot = ts(1_704_067_200);
        let replayed_at = ts(1_704_067_205);
        let record = sample_record(trigger_id, tenant_id.clone(), fire_slot);
        let next_run_at = record
            .schedule
            .next_slot_after(fire_slot)
            .expect("next slot calculation")
            .expect("future slot");
        repo.upsert_trigger(record).await.expect("insert record");
        assert!(matches!(
            repo.claim_due_fire(ClaimDueFireRequest {
                tenant_id: tenant_id.clone(),
                trigger_id,
                fire_slot,
                now: fire_slot,
            })
            .await
            .expect("claim fire"),
            ClaimDueFireOutcome::Claimed(_)
        ));

        let original_run_id =
            TurnRunId::parse("01890f0f-9b6f-7a85-9e5b-9f21a93c4f65").expect("valid run");
        let request = FireReplayedRequest {
            tenant_id: tenant_id.clone(),
            trigger_id,
            fire_slot,
            original_run_id,
            replayed_at,
            next_run_at,
        };
        let first_repo = repo.clone();
        let second_repo = repo.clone();
        let first_request = request.clone();
        let second_request = request;
        let first = async move {
            tokio::task::yield_now().await;
            first_repo.mark_fire_replayed(first_request).await
        };
        let second = async move {
            tokio::task::yield_now().await;
            second_repo.mark_fire_replayed(second_request).await
        };

        let (first, second) = tokio::join!(first, second);
        let first = first
            .expect("first replayed result")
            .expect("first replayed record");
        let second = second
            .expect("second replayed result")
            .expect("second replayed record");
        assert_eq!(first, second);
        assert_eq!(first.active_fire_slot, Some(fire_slot));
        assert_eq!(first.active_run_ref, Some(original_run_id));
        assert_eq!(first.last_run_at, Some(replayed_at));
        assert_eq!(first.last_fired_slot, Some(fire_slot));
        assert_eq!(first.last_status, Some(TriggerRunStatus::Ok));

        let persisted = repo
            .get_trigger(tenant_id, trigger_id)
            .await
            .expect("reload replayed result")
            .expect("record present");
        assert_eq!(persisted, first);
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_repository_fire_claim_contract() {
        let Some((_container, pool)) = postgres_pool_or_skip().await else {
            return;
        };
        let repo = PostgresTriggerRepository::new(pool.clone());
        repo.run_migrations().await.expect("run migrations");
        assert_durable_fire_claim_contract(&repo).await;
        clear_postgres_triggers(&pool).await;
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn postgres_repository_fire_claim_is_atomic() {
        let Some((_container, pool)) = postgres_pool_or_skip().await else {
            return;
        };
        let repo = PostgresTriggerRepository::new(pool.clone());
        repo.run_migrations().await.expect("run migrations");
        assert_durable_claim_is_atomic(std::sync::Arc::new(repo)).await;
        clear_postgres_triggers(&pool).await;
    }
}
