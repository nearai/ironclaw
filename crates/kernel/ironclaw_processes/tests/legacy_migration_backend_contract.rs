//! Durable-backend fixtures for the deployed turn/run-state migration layouts.

use std::sync::Arc;

use ironclaw_filesystem::{
    CasExpectation, Entry, LibSqlRootFilesystem, PostgresRootFilesystem, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, ScopedPath, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_processes::{ProcessJournalSource, ProcessJournalStore};
use serde_json::json;

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
    let Some(backend) = postgres_backend().await else {
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

async fn postgres_backend() -> Option<PostgresRootFilesystem> {
    if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
        return None;
    }
    let url = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let config = url.parse::<tokio_postgres::Config>().ok()?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .ok()?;
    let backend = PostgresRootFilesystem::new(pool);
    backend.run_migrations().await.ok()?;
    Some(backend)
}
