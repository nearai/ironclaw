#![cfg(any(feature = "libsql", feature = "postgres"))]

#[cfg(feature = "libsql")]
use std::sync::Arc;

use ironclaw_host_api::*;
use ironclaw_run_state::{
    ApprovalRecord, ApprovalRequestStore, ApprovalStatus, RunRecord, RunStart,
    RunStateApprovalStore, RunStateError, RunStateStore,
};

#[cfg(feature = "libsql")]
use ironclaw_run_state::{
    LibSqlApprovalRequestStore, LibSqlRunStateApprovalStore, LibSqlRunStateStore,
};
#[cfg(feature = "postgres")]
use ironclaw_run_state::{
    PostgresApprovalRequestStore, PostgresRunStateApprovalStore, PostgresRunStateStore,
};

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_run_state_and_approval_stores_persist_across_database_reopen() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(
        libsql::Builder::new_local(db_path.clone())
            .build()
            .await
            .unwrap(),
    );
    let runs = LibSqlRunStateStore::new(Arc::clone(&db));
    runs.run_migrations().await.unwrap();
    let approvals = LibSqlApprovalRequestStore::new(db);
    approvals.run_migrations().await.unwrap();

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant-libsql", "user-libsql");
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    let running = runs
        .start(RunStart {
            invocation_id,
            capability_id: capability_id.clone(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    assert_eq!(running.status, ironclaw_run_state::RunStatus::Running);
    let blocked = runs
        .block_approval(&scope, invocation_id, approval.clone())
        .await
        .unwrap();
    assert_eq!(blocked.approval_request_id, Some(request_id));
    approvals
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();
    approvals.approve(&scope, request_id).await.unwrap();

    drop(runs);
    drop(approvals);

    let reopened_db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let reopened_runs = LibSqlRunStateStore::new(Arc::clone(&reopened_db));
    let reopened_approvals = LibSqlApprovalRequestStore::new(reopened_db);

    let reloaded_run = reopened_runs
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reloaded_run.status,
        ironclaw_run_state::RunStatus::BlockedApproval
    );
    assert_eq!(reloaded_run.capability_id, capability_id);
    assert_eq!(reloaded_run.approval_request_id, Some(request_id));
    assert_eq!(
        reopened_runs.records_for_scope(&scope).await.unwrap(),
        vec![reloaded_run]
    );

    let reloaded_approval = reopened_approvals
        .get(&scope, request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded_approval.status, ApprovalStatus::Approved);
    assert_eq!(reloaded_approval.request, approval);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_stores_serialize_duplicate_creates_across_instances() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let runs_a = LibSqlRunStateStore::new(Arc::clone(&db));
    runs_a.run_migrations().await.unwrap();
    let runs_b = LibSqlRunStateStore::new(Arc::clone(&db));
    let approvals_a = LibSqlApprovalRequestStore::new(Arc::clone(&db));
    let approvals_b = LibSqlApprovalRequestStore::new(db);

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant-libsql-duplicates", "user-a");
    let start = RunStart {
        invocation_id,
        capability_id: CapabilityId::new("echo.say").unwrap(),
        scope: scope.clone(),
    };
    let (first_run, second_run) = tokio::join!(runs_a.start(start.clone()), runs_b.start(start));
    assert_eq!(
        [&first_run, &second_run]
            .into_iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        [&first_run, &second_run]
            .into_iter()
            .filter(|result| matches!(result, Err(ironclaw_run_state::RunStateError::InvocationAlreadyExists { invocation_id: id }) if *id == invocation_id))
            .count(),
        1
    );

    let approval = approval_request(invocation_id);
    let request_id = approval.id;
    let (first_approval, second_approval) = tokio::join!(
        approvals_a.save_pending(scope.clone(), approval.clone()),
        approvals_b.save_pending(scope.clone(), approval),
    );
    assert_eq!(
        [&first_approval, &second_approval]
            .into_iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        [&first_approval, &second_approval]
            .into_iter()
            .filter(|result| matches!(result, Err(ironclaw_run_state::RunStateError::ApprovalRequestAlreadyExists { request_id: id }) if *id == request_id))
            .count(),
        1
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_combined_store_blocks_with_pending_approval_atomically() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let store = LibSqlRunStateApprovalStore::new(db);
    store.run_migrations().await.unwrap();

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant-libsql-combined", "user-a");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;
    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();

    let blocked = store
        .save_pending_and_block_approval(scope.clone(), invocation_id, approval.clone())
        .await
        .unwrap();

    assert_eq!(
        blocked.status,
        ironclaw_run_state::RunStatus::BlockedApproval
    );
    assert_eq!(blocked.approval_request_id, Some(request_id));
    let saved = ApprovalRequestStore::get(&store, &scope, request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, ApprovalStatus::Pending);
    assert_eq!(saved.request, approval);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_stores_preserve_scope_isolation_and_pending_discard() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let runs = LibSqlRunStateStore::new(Arc::clone(&db));
    runs.run_migrations().await.unwrap();
    let approvals = LibSqlApprovalRequestStore::new(db);
    approvals.run_migrations().await.unwrap();

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant-libsql-isolated", "user-a");
    let other_user = sample_scope(invocation_id, "tenant-libsql-isolated", "user-b");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    runs.start(RunStart {
        invocation_id,
        capability_id: CapabilityId::new("echo.say").unwrap(),
        scope: scope.clone(),
    })
    .await
    .unwrap();
    approvals
        .save_pending(scope.clone(), approval)
        .await
        .unwrap();

    assert!(
        runs.get(&other_user, invocation_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        runs.records_for_scope(&other_user).await.unwrap(),
        Vec::new()
    );
    assert!(
        approvals
            .get(&other_user, request_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        approvals.records_for_scope(&other_user).await.unwrap(),
        Vec::new()
    );

    let discarded = approvals.discard_pending(&scope, request_id).await.unwrap();
    assert_eq!(discarded.status, ApprovalStatus::Pending);
    assert!(approvals.get(&scope, request_id).await.unwrap().is_none());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_rejects_run_rows_when_payload_invocation_id_does_not_match_row_key() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let runs = LibSqlRunStateStore::new(Arc::clone(&db));
    runs.run_migrations().await.unwrap();

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant-libsql-mismatch", "user-a");
    let record = runs
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    let mismatched_payload = RunRecord {
        invocation_id: InvocationId::new(),
        ..record
    };
    let payload = serde_json::to_string(&mismatched_payload).unwrap();
    let conn = db.connect().unwrap();
    conn.execute(
        "UPDATE reborn_run_state_records SET payload = ?1 WHERE invocation_id = ?2",
        libsql::params![payload, invocation_id.to_string()],
    )
    .await
    .unwrap();

    let err = runs.get(&scope, invocation_id).await.unwrap_err();
    assert!(
        matches!(err, RunStateError::Deserialization(_)),
        "expected deserialization integrity error, got {err:?}"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_rejects_approval_rows_when_payload_request_id_does_not_match_row_key() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let approvals = LibSqlApprovalRequestStore::new(Arc::clone(&db));
    approvals.run_migrations().await.unwrap();

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, "tenant-libsql-approval-mismatch", "user-a");
    let approval = approval_request(invocation_id);
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();
    let mut mismatched_request = approval;
    mismatched_request.id = ApprovalRequestId::new();
    let mismatched_payload = ApprovalRecord {
        scope: scope.clone(),
        request: mismatched_request,
        status: ApprovalStatus::Pending,
    };
    let payload = serde_json::to_string(&mismatched_payload).unwrap();
    let conn = db.connect().unwrap();
    conn.execute(
        "UPDATE reborn_approval_request_records SET payload = ?1 WHERE request_id = ?2",
        libsql::params![payload, request_id.to_string()],
    )
    .await
    .unwrap();

    let err = approvals.get(&scope, request_id).await.unwrap_err();
    assert!(
        matches!(err, RunStateError::Deserialization(_)),
        "expected deserialization integrity error, got {err:?}"
    );
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_migrations_are_serialized_when_called_concurrently() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let store_a = PostgresRunStateStore::new(pool.clone());
    let store_b = PostgresRunStateStore::new(pool.clone());
    let store_c = PostgresApprovalRequestStore::new(pool.clone());
    let store_d = PostgresRunStateApprovalStore::new(pool);

    let (a, b, c, d) = tokio::join!(
        store_a.run_migrations(),
        store_b.run_migrations(),
        store_c.run_migrations(),
        store_d.run_migrations(),
    );

    a.unwrap();
    b.unwrap();
    c.unwrap();
    d.unwrap();
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_run_state_and_approval_stores_persist_across_instances_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let tenant = format!("tenant-pg-{suffix}");
    let user = format!("user-pg-{suffix}");
    let runs = PostgresRunStateStore::new(pool.clone());
    runs.run_migrations().await.unwrap();
    let approvals = PostgresApprovalRequestStore::new(pool.clone());
    approvals.run_migrations().await.unwrap();

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, &tenant, &user);
    let capability_id = CapabilityId::new("echo.say").unwrap();
    let approval = approval_request(invocation_id);
    let request_id = approval.id;

    runs.start(RunStart {
        invocation_id,
        capability_id: capability_id.clone(),
        scope: scope.clone(),
    })
    .await
    .unwrap();
    runs.block_auth(&scope, invocation_id, "ExternalAuth".to_string())
        .await
        .unwrap();
    approvals
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();
    approvals.deny(&scope, request_id).await.unwrap();

    let reopened_runs = PostgresRunStateStore::new(pool.clone());
    let reopened_approvals = PostgresApprovalRequestStore::new(pool);
    let reloaded_run = reopened_runs
        .get(&scope, invocation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reloaded_run.status,
        ironclaw_run_state::RunStatus::BlockedAuth
    );
    assert_eq!(reloaded_run.error_kind.as_deref(), Some("ExternalAuth"));
    assert_eq!(reloaded_run.capability_id, capability_id);
    assert_eq!(
        reopened_runs.records_for_scope(&scope).await.unwrap(),
        vec![reloaded_run]
    );

    let reloaded_approval = reopened_approvals
        .get(&scope, request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reloaded_approval.status, ApprovalStatus::Denied);
    assert_eq!(reloaded_approval.request, approval);
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_stores_reject_duplicate_creates_across_instances_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let tenant = format!("tenant-pg-duplicates-{suffix}");
    let user = format!("user-pg-duplicates-{suffix}");
    let runs_a = PostgresRunStateStore::new(pool.clone());
    runs_a.run_migrations().await.unwrap();
    let runs_b = PostgresRunStateStore::new(pool.clone());
    let approvals_a = PostgresApprovalRequestStore::new(pool.clone());
    let approvals_b = PostgresApprovalRequestStore::new(pool);

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, &tenant, &user);
    let start = RunStart {
        invocation_id,
        capability_id: CapabilityId::new("echo.say").unwrap(),
        scope: scope.clone(),
    };
    let (first_run, second_run) = tokio::join!(runs_a.start(start.clone()), runs_b.start(start));
    assert_eq!(
        [&first_run, &second_run]
            .into_iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        [&first_run, &second_run]
            .into_iter()
            .filter(|result| matches!(result, Err(ironclaw_run_state::RunStateError::InvocationAlreadyExists { invocation_id: id }) if *id == invocation_id))
            .count(),
        1
    );

    let approval = approval_request(invocation_id);
    let request_id = approval.id;
    let (first_approval, second_approval) = tokio::join!(
        approvals_a.save_pending(scope.clone(), approval.clone()),
        approvals_b.save_pending(scope.clone(), approval),
    );
    assert_eq!(
        [&first_approval, &second_approval]
            .into_iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        [&first_approval, &second_approval]
            .into_iter()
            .filter(|result| matches!(result, Err(ironclaw_run_state::RunStateError::ApprovalRequestAlreadyExists { request_id: id }) if *id == request_id))
            .count(),
        1
    );
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_combined_store_blocks_with_pending_approval_atomically_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let tenant = format!("tenant-pg-combined-{suffix}");
    let user = format!("user-pg-combined-{suffix}");
    let store = PostgresRunStateApprovalStore::new(pool);
    store.run_migrations().await.unwrap();

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, &tenant, &user);
    let approval = approval_request(invocation_id);
    let request_id = approval.id;
    store
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();

    let blocked = store
        .save_pending_and_block_approval(scope.clone(), invocation_id, approval.clone())
        .await
        .unwrap();

    assert_eq!(
        blocked.status,
        ironclaw_run_state::RunStatus::BlockedApproval
    );
    assert_eq!(blocked.approval_request_id, Some(request_id));
    let saved = ApprovalRequestStore::get(&store, &scope, request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status, ApprovalStatus::Pending);
    assert_eq!(saved.request, approval);
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_rejects_run_rows_when_payload_invocation_id_does_not_match_row_key() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let tenant = format!("tenant-pg-run-mismatch-{suffix}");
    let user = format!("user-pg-run-mismatch-{suffix}");
    let runs = PostgresRunStateStore::new(pool.clone());
    runs.run_migrations().await.unwrap();

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, &tenant, &user);
    let record = runs
        .start(RunStart {
            invocation_id,
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope: scope.clone(),
        })
        .await
        .unwrap();
    let mismatched_payload = RunRecord {
        invocation_id: InvocationId::new(),
        ..record
    };
    let payload = serde_json::to_string(&mismatched_payload).unwrap();
    let client = pool.get().await.unwrap();
    client
        .execute(
            "UPDATE reborn_run_state_records SET payload = $1::text::jsonb WHERE invocation_id = $2",
            &[&payload, &invocation_id.to_string()],
        )
        .await
        .unwrap();

    let err = runs.get(&scope, invocation_id).await.unwrap_err();
    assert!(
        matches!(err, RunStateError::Deserialization(_)),
        "expected deserialization integrity error, got {err:?}"
    );
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_rejects_approval_rows_when_payload_request_id_does_not_match_row_key() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let tenant = format!("tenant-pg-approval-mismatch-{suffix}");
    let user = format!("user-pg-approval-mismatch-{suffix}");
    let approvals = PostgresApprovalRequestStore::new(pool.clone());
    approvals.run_migrations().await.unwrap();

    let invocation_id = InvocationId::new();
    let scope = sample_scope(invocation_id, &tenant, &user);
    let approval = approval_request(invocation_id);
    let request_id = approval.id;
    approvals
        .save_pending(scope.clone(), approval.clone())
        .await
        .unwrap();
    let mut mismatched_request = approval;
    mismatched_request.id = ApprovalRequestId::new();
    let mismatched_payload = ApprovalRecord {
        scope: scope.clone(),
        request: mismatched_request,
        status: ApprovalStatus::Pending,
    };
    let payload = serde_json::to_string(&mismatched_payload).unwrap();
    let client = pool.get().await.unwrap();
    client
        .execute(
            "UPDATE reborn_approval_request_records SET payload = $1::text::jsonb WHERE request_id = $2",
            &[&payload, &request_id.to_string()],
        )
        .await
        .unwrap();

    let err = approvals.get(&scope, request_id).await.unwrap_err();
    assert!(
        matches!(err, RunStateError::Deserialization(_)),
        "expected deserialization integrity error, got {err:?}"
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_stores_implement_run_state_contract_traits() {
    fn assert_run_state<T: RunStateStore>() {}
    fn assert_approval<T: ApprovalRequestStore>() {}
    fn assert_combined<T: RunStateApprovalStore>() {}
    assert_run_state::<PostgresRunStateStore>();
    assert_approval::<PostgresApprovalRequestStore>();
    assert_combined::<PostgresRunStateApprovalStore>();
}

#[cfg(feature = "libsql")]
#[test]
fn libsql_stores_implement_run_state_contract_traits() {
    fn assert_run_state<T: RunStateStore>() {}
    fn assert_approval<T: ApprovalRequestStore>() {}
    fn assert_combined<T: RunStateApprovalStore>() {}
    assert_run_state::<LibSqlRunStateStore>();
    assert_approval::<LibSqlApprovalRequestStore>();
    assert_combined::<LibSqlRunStateApprovalStore>();
}

#[cfg(feature = "libsql")]
fn libsql_db_path() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("run-state.db");
    (db_path.to_string_lossy().into_owned(), dir)
}

#[cfg(feature = "postgres")]
async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
    let url = std::env::var("IRONCLAW_RUN_STATE_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://localhost/ironclaw_test".to_string());
    let config: tokio_postgres::Config = url
        .parse()
        .expect("run-state postgres test URL must be valid");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .unwrap();
    match pool.get().await {
        Ok(_) => Some(pool),
        Err(error) if skip_postgres_requested() => {
            eprintln!(
                "skipping postgres run-state contract (IRONCLAW_SKIP_POSTGRES_TESTS=1): {error}"
            );
            None
        }
        Err(error) => panic!(
            "postgres run-state contract could not reach Postgres ({error}); set \
             IRONCLAW_RUN_STATE_POSTGRES_URL or DATABASE_URL, or set \
             IRONCLAW_SKIP_POSTGRES_TESTS=1 to explicitly skip."
        ),
    }
}

#[cfg(feature = "postgres")]
fn skip_postgres_requested() -> bool {
    std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok_and(|value| value == "1" || value == "true")
}

#[cfg(feature = "postgres")]
fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos()
}

fn sample_scope(invocation_id: InvocationId, tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: Some(MissionId::new("mission1").unwrap()),
        thread_id: Some(ThreadId::new("thread1").unwrap()),
        invocation_id,
    }
}

fn approval_request(invocation_id: InvocationId) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: CorrelationId::new(),
        requested_by: Principal::Extension(ExtensionId::new("caller").unwrap()),
        action: Box::new(Action::Dispatch {
            capability: CapabilityId::new("echo.say").unwrap(),
            estimated_resources: ResourceEstimate::default(),
        }),
        invocation_fingerprint: None,
        reason: format!("approval for {invocation_id}"),
        reusable_scope: None,
    }
}
