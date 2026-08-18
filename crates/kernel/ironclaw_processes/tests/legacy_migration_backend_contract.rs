//! Durable-backend conformance fixtures for process-journal persistence.

use chrono::Utc;
use std::sync::Arc;

use ironclaw_filesystem::{
    CasExpectation, Entry, LibSqlRootFilesystem, PostgresRootFilesystem, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProcessId, ProjectId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, ScopedPath, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_processes::{
    ClaimProcessesRequest, GetProcessSnapshotRequest, ProcessJournalCursor, ProcessJournalKind,
    ProcessJournalSource, ProcessJournalStore, ProcessKind, ProcessLeaseRequest,
    ProcessSubmissionPort, ProcessTransitionPort, ProcessWorkerId, SubmitProcessRequest,
};
use serde_json::json;

#[tokio::test]
async fn heartbeat_persistence_is_durable_on_libsql() {
    let storage = tempfile::tempdir().expect("temporary heartbeat database");
    let database = Arc::new(
        libsql::Builder::new_local(storage.path().join("heartbeat.db"))
            .build()
            .await
            .expect("build libsql database"),
    );
    let backend = Arc::new(LibSqlRootFilesystem::new(database).expect("libSQL filesystem runtime"));
    backend
        .run_migrations()
        .await
        .expect("migrate libsql filesystem");
    assert_durable_heartbeat(backend, format!("libsql-{}", uuid::Uuid::new_v4())).await;
}

#[tokio::test]
async fn heartbeat_persistence_is_durable_on_postgres() {
    let Some(backend) = postgres_backend()
        .await
        .expect("configure process heartbeat PostgreSQL backend")
    else {
        eprintln!(
            "skipping process heartbeat Postgres contract: \
             IRONCLAW_FILESYSTEM_POSTGRES_URL / DATABASE_URL unavailable"
        );
        return;
    };
    assert_durable_heartbeat(
        Arc::new(backend),
        format!("postgres-{}", uuid::Uuid::new_v4()),
    )
    .await;
}

async fn assert_durable_heartbeat<F>(backend: Arc<F>, fixture: String)
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        backend,
        MountView::new(vec![mount(
            "/processes",
            &format!("/engine/process-heartbeat/{fixture}/processes"),
        )])
        .expect("heartbeat mount view"),
    ));
    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    let scope = heartbeat_scope();
    let process_id = ProcessId::new();
    store
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit durable heartbeat process");
    let claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted(ProcessId::new().as_uuid().to_string()),
            scope_filter: Some(scope.clone()),
            process_id_filter: Some(process_id),
            process_kind_filter: Some(ProcessKind::AgentTurn),
            max_processes: 1,
        })
        .await
        .expect("claim durable heartbeat process")
        .pop()
        .expect("one durable heartbeat claim");
    let claimed_lease = claim.state.lease.clone().expect("claimed lease");
    let lease = ProcessLeaseRequest {
        process_id,
        worker_id: claim.worker_id,
        lease_token: claim.lease_token,
    };

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let first_cursor = store
        .heartbeat_process(lease.clone())
        .await
        .expect("first durable heartbeat");
    let first_snapshot = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: scope.clone(),
            process_id,
        })
        .await
        .expect("snapshot after first durable heartbeat");
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let second_cursor = store
        .heartbeat_process(lease)
        .await
        .expect("second durable heartbeat");
    assert_eq!(first_cursor, claim.state.journal_cursor);
    assert_eq!(second_cursor, claim.state.journal_cursor);

    drop(store);
    let reloaded = ProcessJournalStore::new(filesystem);
    let reloaded_snapshot = reloaded
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: scope.clone(),
            process_id,
        })
        .await
        .expect("reload durable heartbeat process");
    let first_lease = first_snapshot
        .lease
        .as_ref()
        .expect("first heartbeat lease");
    let reloaded_lease = reloaded_snapshot
        .lease
        .as_ref()
        .expect("reloaded heartbeat lease");
    assert!(
        first_lease.last_heartbeat_at > claimed_lease.last_heartbeat_at,
        "first heartbeat must refresh last_heartbeat_at"
    );
    assert!(
        reloaded_lease.last_heartbeat_at > first_lease.last_heartbeat_at,
        "repeated heartbeat must durably refresh last_heartbeat_at"
    );
    assert!(
        reloaded_lease.lease_expires_at > first_lease.lease_expires_at,
        "repeated heartbeat must durably refresh lease_expires_at"
    );
    assert_eq!(reloaded_snapshot.journal_cursor, claim.state.journal_cursor);

    let page = reloaded
        .read_process_journal_after(&scope, None, Some(ProcessJournalCursor(0)), 10)
        .await
        .expect("read durable heartbeat journal after reload");
    assert_eq!(page.entries.len(), 2);
    assert!(
        page.entries
            .iter()
            .all(|entry| entry.kind != ProcessJournalKind::Heartbeat),
        "durable heartbeats must not append journal rows"
    );
    assert_eq!(
        page.entries
            .iter()
            .map(|entry| entry.cursor)
            .collect::<Vec<_>>(),
        vec![ProcessJournalCursor(1), ProcessJournalCursor(2)],
        "durable heartbeats must not reserve journal cursors"
    );
}

fn heartbeat_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-heartbeat").expect("tenant"),
        user_id: UserId::new("user-heartbeat").expect("user"),
        agent_id: Some(AgentId::new("agent-heartbeat").expect("agent")),
        project_id: Some(ProjectId::new("project-heartbeat").expect("project")),
        mission_id: None,
        thread_id: Some(ThreadId::new("thread-heartbeat").expect("thread")),
        invocation_id: InvocationId::new(),
    }
}

#[tokio::test]
async fn deployed_legacy_layouts_import_on_libsql() {
    let storage = tempfile::tempdir().expect("temporary migration database");
    let database = Arc::new(
        libsql::Builder::new_local(storage.path().join("legacy-migration.db"))
            .build()
            .await
            .expect("build libsql database"),
    );
    let backend = Arc::new(LibSqlRootFilesystem::new(database).expect("libSQL filesystem runtime"));
    backend
        .run_migrations()
        .await
        .expect("migrate libsql filesystem");
    assert_durable_legacy_import(backend, format!("libsql-{}", uuid::Uuid::new_v4())).await;
}

#[tokio::test]
async fn deployed_legacy_layouts_import_on_postgres() {
    let Some(backend) = postgres_backend()
        .await
        .expect("configure process migration PostgreSQL backend")
    else {
        eprintln!(
            "skipping process legacy-migration Postgres contract: \
             IRONCLAW_FILESYSTEM_POSTGRES_URL / DATABASE_URL unavailable"
        );
        return;
    };
    assert_durable_legacy_import(
        Arc::new(backend),
        format!("postgres-{}", uuid::Uuid::new_v4()),
    )
    .await;
}

async fn assert_durable_legacy_import<F>(backend: Arc<F>, fixture: String)
where
    F: RootFilesystem + 'static,
{
    let prefix = format!("/engine/process-migration/{fixture}");
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        backend,
        MountView::new(vec![
            mount("/processes", &format!("{prefix}/processes")),
            mount("/turns", &format!("{prefix}/turns")),
            mount("/run-state", &format!("{prefix}/run-state")),
        ])
        .expect("migration mount view"),
    ));
    let blob_run = uuid::Uuid::new_v4();
    let row_run = uuid::Uuid::new_v4();
    let capability_run = uuid::Uuid::new_v4();
    seed_json(
        filesystem.as_ref(),
        "/turns/state.json",
        json!({
            "runs": [turn_run(blob_run, "thread-blob", 11)],
            "turns": [],
            "loop_checkpoints": [],
            "idempotency_records": [],
            "spawn_tree_reservations": []
        }),
    )
    .await;
    seed_json(
        filesystem.as_ref(),
        "/turns/rows/v1/meta/state.json",
        json!({"journal_seq": 12, "event_retention_floor": 0}),
    )
    .await;
    seed_json(
        filesystem.as_ref(),
        &format!("/turns/rows/v1/runs/{row_run}.json"),
        json!({"journal_seq": 12, "value": turn_run(row_run, "thread-row", 12)}),
    )
    .await;
    seed_json(
        filesystem.as_ref(),
        &format!("/run-state/agents/fixture/runs/{capability_run}.json"),
        json!({
            "invocation_id": capability_run,
            "capability_id": "builtin.echo",
            "scope": resource_scope("thread-capability", capability_run),
            "authenticated_actor_user_id": "migration-user",
            "status": "Completed",
            "approval_request_id": null,
            "error_kind": null
        }),
    )
    .await;

    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    assert_eq!(
        store
            .migrate_legacy_journal()
            .await
            .expect("import deployed durable fixtures"),
        3
    );
    let page = store
        .read_process_journal_log_after(None, 16)
        .await
        .expect("read imported durable journal");
    assert_eq!(page.entries.len(), 3);

    let restarted = ProcessJournalStore::new(filesystem);
    assert_eq!(
        restarted
            .migrate_legacy_journal()
            .await
            .expect("durable migration rerun"),
        0
    );
    assert_eq!(
        restarted
            .read_process_journal_log_after(None, 16)
            .await
            .expect("read journal after restart")
            .entries
            .len(),
        3
    );
}

fn turn_run(run_id: uuid::Uuid, thread_id: &str, cursor: u64) -> serde_json::Value {
    json!({
        "run_id": run_id,
        "turn_id": uuid::Uuid::new_v4(),
        "scope": {
            "tenant_id": "migration-tenant",
            "agent_id": "migration-agent",
            "project_id": "migration-project",
            "thread_id": thread_id,
            "thread_owner": {
                "mode": "explicit_user",
                "owner_user_id": "migration-user"
            }
        },
        "accepted_message_ref": format!("accepted:{run_id}"),
        "source_binding_ref": format!("source:{run_id}"),
        "reply_target_binding_ref": format!("reply:{run_id}"),
        "status": "Completed",
        "profile": {
            "id": "default",
            "version": 1,
            "allow_steering": false,
            "auto_queue_followups": false
        },
        "checkpoint_id": null,
        "gate_ref": null,
        "credential_requirements": [],
        "failure": null,
        "event_cursor": cursor,
        "runner_id": null,
        "lease_token": null,
        "lease_expires_at": null,
        "last_heartbeat_at": null,
        "claim_count": 0,
        "received_at": "2026-01-03T00:00:00Z",
        "parent_run_id": null,
        "subagent_depth": 0,
        "spawn_tree_root_run_id": null,
        "product_context": null
    })
}

fn resource_scope(thread_id: &str, invocation_id: uuid::Uuid) -> serde_json::Value {
    json!({
        "tenant_id": "migration-tenant",
        "user_id": "migration-user",
        "agent_id": "migration-agent",
        "project_id": "migration-project",
        "mission_id": null,
        "thread_id": thread_id,
        "invocation_id": invocation_id
    })
}

fn mount(alias: &str, target: &str) -> MountGrant {
    MountGrant::new(
        MountAlias::new(alias).expect("mount alias"),
        VirtualPath::new(target).expect("mount target"),
        MountPermissions::read_write_list_delete(),
    )
}

async fn seed_json<F>(filesystem: &ScopedFilesystem<F>, path: &str, value: serde_json::Value)
where
    F: RootFilesystem,
{
    filesystem
        .put(
            &ResourceScope::system(),
            &ScopedPath::new(path).expect("fixture path"),
            Entry::bytes(serde_json::to_vec(&value).expect("serialize fixture")),
            CasExpectation::Absent,
        )
        .await
        .expect("seed durable fixture");
}

async fn postgres_backend() -> Result<Option<PostgresRootFilesystem>, String> {
    if std::env::var_os("IRONCLAW_SKIP_POSTGRES_TESTS").is_some() {
        return Ok(None);
    }
    let Some(url) = std::env::var_os("IRONCLAW_FILESYSTEM_POSTGRES_URL")
        .or_else(|| std::env::var_os("DATABASE_URL"))
    else {
        return Ok(None);
    };
    let url = url
        .into_string()
        .map_err(|_| "PostgreSQL URL is not valid UTF-8".to_string())?;
    let config = url
        .parse::<tokio_postgres::Config>()
        .map_err(|error| format!("parse PostgreSQL URL: {error}"))?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .map_err(|error| format!("build PostgreSQL pool: {error}"))?;
    let backend = PostgresRootFilesystem::new(pool);
    backend
        .run_migrations()
        .await
        .map_err(|error| format!("run PostgreSQL migrations: {error}"))?;
    Ok(Some(backend))
}
