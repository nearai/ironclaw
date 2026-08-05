//! Durable-backend coverage for the rc1 thread and append-log migration.

use std::sync::Arc;

use ironclaw_filesystem::{
    CasExpectation, Entry, LibSqlRootFilesystem, PostgresRootFilesystem, RecordKind,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    ids::{AgentId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
};
use ironclaw_threads::{
    FilesystemSessionThreadService, ListThreadsForScopeRequest, SessionThreadService,
    ThreadHistoryRequest, ThreadScope, migrate_all_thread_scopes,
};
use serde::Deserialize;

const RC1_ACTUAL_FIXTURE: &str = include_str!("fixtures/rc1_actual_thread_state.json");

#[derive(Debug, Deserialize)]
struct Rc1ActualFixture {
    source: Rc1FixtureSource,
    scope: Rc1FixtureScope,
    thread_id: String,
    records: Vec<Rc1FixtureRecord>,
    append_events: Vec<Rc1FixtureAppend>,
    expected_messages: Vec<(u64, String, String)>,
}

#[derive(Debug, Deserialize)]
struct Rc1FixtureSource {
    binary_tag: String,
    commit: String,
    capture: String,
}

#[derive(Debug, Deserialize)]
struct Rc1FixtureScope {
    tenant_id: String,
    agent_id: String,
    owner_user_id: String,
}

#[derive(Debug, Deserialize)]
struct Rc1FixtureRecord {
    path: String,
    kind: String,
    body: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct Rc1FixtureAppend {
    path: String,
    body: serde_json::Value,
}

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
    let mut rc1: Rc1ActualFixture =
        serde_json::from_str(RC1_ACTUAL_FIXTURE).expect("frozen actual rc1 fixture");
    assert_eq!(rc1.source.binary_tag, "ironclaw-v1.0.0-rc.1");
    assert_eq!(
        rc1.source.commit,
        "8257215700fd75a3636338e969605f5dee8f99c4"
    );
    assert!(rc1.source.capture.contains("rc1 WebChat API"));
    // Preserve the exact captured wire as the checked-in fixture, then scope
    // each backend invocation to a unique tenant so a developer can rerun the
    // live PostgreSQL contract without colliding with its prior derived rows.
    let captured_tenant = rc1.scope.tenant_id.clone();
    let migrated_tenant = format!("{captured_tenant}-{fixture}");
    for record in &mut rc1.records {
        record.path = record.path.replacen(
            &format!("/tenants/{captured_tenant}/"),
            &format!("/tenants/{migrated_tenant}/"),
            1,
        );
        if record.kind == "session_thread"
            && let Some(scope) = record
                .body
                .get_mut("scope")
                .and_then(|scope| scope.as_object_mut())
        {
            scope.insert(
                "tenant_id".to_string(),
                serde_json::Value::String(migrated_tenant.clone()),
            );
        }
    }
    for append in &mut rc1.append_events {
        append.path = append.path.replacen(
            &format!("/tenants/{captured_tenant}/"),
            &format!("/tenants/{migrated_tenant}/"),
            1,
        );
    }
    let tenant = TenantId::new(migrated_tenant).expect("tenant");
    let user = UserId::new(rc1.scope.owner_user_id.clone()).expect("user");
    let scope = ThreadScope {
        tenant_id: tenant.clone(),
        agent_id: AgentId::new(rc1.scope.agent_id.clone()).expect("agent"),
        project_id: None,
        owner_user_id: Some(user.clone()),
        mission_id: None,
    };
    let thread_id = ThreadId::new(rc1.thread_id.clone()).expect("thread");
    for record in rc1.records {
        backend
            .put(
                &VirtualPath::new(record.path).expect("released record path"),
                Entry::record(
                    RecordKind::new(record.kind).expect("released record kind"),
                    &record.body,
                )
                .expect("released record entry"),
                CasExpectation::Absent,
            )
            .await
            .expect("seed exact released record");
    }
    for append in rc1.append_events {
        backend
            .append(
                &VirtualPath::new(append.path).expect("released append path"),
                serde_json::to_vec_pretty(&append.body).expect("released append wire"),
            )
            .await
            .expect("seed exact released append event");
    }
    let first =
        migrate_all_thread_scopes(Arc::clone(&backend), dynamic_scoped(Arc::clone(&backend)))
            .await
            .expect("migrate rc1 state");
    assert!(first.thread_rows >= 1, "{first:?}");
    assert!(first.append_messages_materialized >= 1, "{first:?}");

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
    let actual = history
        .messages
        .iter()
        .map(|message| {
            (
                message.sequence,
                format!("{:?}", message.kind).to_ascii_lowercase(),
                message.content.clone().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, rc1.expected_messages);

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
