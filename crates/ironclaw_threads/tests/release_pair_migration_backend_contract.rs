//! Durable-backend coverage for the rc1 thread and append-log migration.

use std::sync::Arc;

use ironclaw_filesystem::{
    CasExpectation, Entry, LibSqlRootFilesystem, PostgresRootFilesystem, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, ScopedPath, VirtualPath},
};
use ironclaw_threads::{
    EnsureThreadRequest, FilesystemSessionThreadService, ListThreadsForScopeRequest,
    SessionThreadService, ThreadHistoryRequest, ThreadScope, migrate_all_thread_scopes,
};

const RC1_APPEND_ONLY_MESSAGE: &[u8] = br#"{
  "message_id": "11111111-1111-4111-8111-111111111111",
  "thread_id": "thread-rc1-backend",
  "sequence": 1,
  "kind": "assistant",
  "status": "finalized",
  "created_at": "2026-07-01T12:00:00Z",
  "updated_at": "2026-07-01T12:00:00Z",
  "actor_id": null,
  "source_binding_id": null,
  "reply_target_binding_id": null,
  "turn_id": null,
  "turn_run_id": "run-rc1-backend",
  "content": "durable rc1 append-only reply",
  "redaction_ref": null
}"#;

#[tokio::test]
async fn rc1_thread_upgrade_is_durable_on_libsql() {
    let storage = tempfile::tempdir().expect("temporary migration database");
    let database = Arc::new(
        libsql::Builder::new_local(storage.path().join("thread-migration.db"))
            .build()
            .await
            .expect("build libsql database"),
    );
    let backend = Arc::new(LibSqlRootFilesystem::new(database).expect("libSQL filesystem"));
    backend
        .run_migrations()
        .await
        .expect("run libSQL migrations");
    assert_rc1_upgrade(backend, format!("libsql-{}", uuid::Uuid::new_v4())).await;
}

#[tokio::test]
async fn rc1_thread_upgrade_is_durable_on_postgres() {
    let Some(backend) = postgres_backend().await else {
        eprintln!(
            "skipping thread release-pair Postgres contract: \
             IRONCLAW_FILESYSTEM_POSTGRES_URL / DATABASE_URL unavailable"
        );
        return;
    };
    assert_rc1_upgrade(
        Arc::new(backend),
        format!("postgres-{}", uuid::Uuid::new_v4()),
    )
    .await;
}

async fn assert_rc1_upgrade<F>(backend: Arc<F>, fixture: String)
where
    F: RootFilesystem + 'static,
{
    let tenant = TenantId::new(format!("migration-{fixture}")).expect("tenant");
    let user = UserId::new("migration-user").expect("user");
    let scope = ThreadScope {
        tenant_id: tenant.clone(),
        agent_id: AgentId::new("migration-agent").expect("agent"),
        project_id: Some(ProjectId::new("migration-project").expect("project")),
        owner_user_id: Some(user.clone()),
        mission_id: None,
    };
    let thread_id = ThreadId::new("thread-rc1-backend").expect("thread");
    let scoped = fixed_scoped(Arc::clone(&backend), &tenant, &user);
    let writer = FilesystemSessionThreadService::new(Arc::clone(&scoped));
    writer
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: "migration-user".to_string(),
            title: Some("rc1 durable thread".to_string()),
            metadata_json: None,
        })
        .await
        .expect("seed thread");

    remove_current_marker(scoped.as_ref(), &scope, "thread-index-v2.complete").await;
    remove_current_marker(scoped.as_ref(), &scope, "transcript-index-v2.complete").await;
    scoped
        .put(
            &scope.to_resource_scope(),
            &migration_marker(&scope, "thread-index-v1.complete"),
            Entry::bytes(b"thread-index-v1".to_vec()),
            CasExpectation::Any,
        )
        .await
        .expect("seed rc1 thread marker");
    scoped
        .put(
            &scope.to_resource_scope(),
            &migration_marker(&scope, "transcript-index-v1.complete"),
            Entry::bytes(b"transcript-index-v1".to_vec()),
            CasExpectation::Any,
        )
        .await
        .expect("seed rc1 transcript marker");

    let index_path = thread_index_path(&scope, &thread_id);
    let versioned = scoped
        .get(&scope.to_resource_scope(), &index_path)
        .await
        .expect("read current index")
        .expect("index exists");
    let mut body: serde_json::Value =
        serde_json::from_slice(&versioned.entry.body).expect("decode index");
    body.as_object_mut()
        .expect("index object")
        .remove("projection_schema_version");
    let mut rc1_entry = versioned.entry;
    rc1_entry.body = serde_json::to_vec(&body).expect("encode rc1 index");
    rc1_entry.indexed.clear();
    scoped
        .put(
            &scope.to_resource_scope(),
            &index_path,
            rc1_entry,
            CasExpectation::Version(versioned.version),
        )
        .await
        .expect("replace with rc1 index wire");
    scoped
        .append(
            &scope.to_resource_scope(),
            &append_path(&scope, &thread_id),
            RC1_APPEND_ONLY_MESSAGE.to_vec(),
        )
        .await
        .expect("seed rc1 append event");

    let first =
        migrate_all_thread_scopes(Arc::clone(&backend), dynamic_scoped(Arc::clone(&backend)))
            .await
            .expect("migrate rc1 state");
    assert_eq!(first.thread_rows, 1);
    assert_eq!(first.append_messages_materialized, 1);

    let reopened =
        FilesystemSessionThreadService::new(fixed_scoped(Arc::clone(&backend), &tenant, &user));
    let listed = reopened
        .list_threads_for_scope(ListThreadsForScopeRequest {
            scope: scope.clone(),
            limit: None,
            cursor: None,
        })
        .await
        .expect("list migrated thread");
    assert_eq!(listed.threads.len(), 1);
    assert_eq!(listed.threads[0].thread_id, thread_id);
    let history = reopened
        .list_thread_history(ThreadHistoryRequest {
            scope: scope.clone(),
            thread_id: thread_id.clone(),
        })
        .await
        .expect("read migrated history");
    assert_eq!(history.messages.len(), 1);
    assert_eq!(
        history.messages[0].content.as_deref(),
        Some("durable rc1 append-only reply")
    );

    let second =
        migrate_all_thread_scopes(Arc::clone(&backend), dynamic_scoped(Arc::clone(&backend)))
            .await
            .expect("rerun migration");
    assert_eq!(second.append_messages_materialized, 0);
    assert_eq!(second.transcript_scopes_migrated, 0);
}

fn fixed_scoped<F>(backend: Arc<F>, tenant: &TenantId, user: &UserId) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    Arc::new(ScopedFilesystem::with_fixed_view(
        backend,
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/threads").expect("alias"),
            VirtualPath::new(format!(
                "/tenants/{}/users/{}/threads",
                tenant.as_str(),
                user.as_str()
            ))
            .expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view"),
    ))
}

fn dynamic_scoped<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    Arc::new(ScopedFilesystem::new(backend, |scope| {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/threads")?,
            VirtualPath::new(format!(
                "/tenants/{}/users/{}/threads",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            ))?,
            MountPermissions::read_write_list_delete(),
        )])
    }))
}

fn scope_root(scope: &ThreadScope) -> String {
    format!(
        "/threads/agents/{}/projects/{}/owners/{}",
        scope.agent_id.as_str(),
        scope.project_id.as_ref().expect("project").as_str(),
        scope.owner_user_id.as_ref().expect("owner").as_str()
    )
}

fn migration_marker(scope: &ThreadScope, name: &str) -> ScopedPath {
    ScopedPath::new(format!("{}/index-migrations/{name}", scope_root(scope))).expect("marker path")
}

fn thread_index_path(scope: &ThreadScope, thread_id: &ThreadId) -> ScopedPath {
    ScopedPath::new(format!(
        "{}/thread_index/{}.json",
        scope_root(scope),
        thread_id.as_str()
    ))
    .expect("index path")
}

fn append_path(scope: &ThreadScope, thread_id: &ThreadId) -> ScopedPath {
    ScopedPath::new(format!(
        "{}/threads/{}/message_appends",
        scope_root(scope),
        thread_id.as_str()
    ))
    .expect("append path")
}

async fn remove_current_marker<F>(scoped: &ScopedFilesystem<F>, scope: &ThreadScope, name: &str)
where
    F: RootFilesystem,
{
    let path = migration_marker(scope, name);
    if scoped
        .get(&scope.to_resource_scope(), &path)
        .await
        .expect("read marker")
        .is_some()
    {
        scoped
            .delete(&scope.to_resource_scope(), &path)
            .await
            .expect("delete marker");
    }
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
