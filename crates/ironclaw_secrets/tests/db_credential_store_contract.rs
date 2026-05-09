#![cfg(any(feature = "libsql", feature = "postgres"))]

use std::sync::Arc;

use chrono::Utc;
use ironclaw_host_api::{
    AgentId, CapabilityId, ExtensionId, InvocationId, MissionId, NetworkMethod, ProjectId,
    ResourceScope, SecretHandle, TenantId, ThreadId, UserId,
};
#[cfg(feature = "libsql")]
use ironclaw_secrets::LibSqlCredentialStore;
#[cfg(feature = "postgres")]
use ironclaw_secrets::PostgresCredentialStore;
use ironclaw_secrets::{
    CredentialAccount, CredentialAccountId, CredentialAccountStatus, CredentialAccountStore,
    CredentialPathPolicy, CredentialSessionRequest, CredentialSessionStore, CredentialTargetPolicy,
    InMemoryCredentialBroker, RedactedJson, SecretMaterial, SecretsCrypto,
};
use serde_json::json;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_persists_accounts_across_reopen_and_isolates_scope() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let store = libsql_store(&db_path).await;
    let scope = sample_scope("tenant-a", "user-a");
    let other_scope = sample_scope("tenant-b", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    let account = sample_account(scope.clone(), account_id.clone());

    store.put_account(account.clone()).await.unwrap();
    drop(store);

    let reopened = libsql_store(&db_path).await;
    assert_eq!(
        reopened.get_account(&scope, &account_id).await.unwrap(),
        Some(account)
    );
    assert_eq!(
        reopened
            .get_account(&other_scope, &account_id)
            .await
            .unwrap(),
        None
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_persists_sessions_and_enforces_use_limits_across_reopen() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let store = libsql_store(&db_path).await;
    let scope = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    store
        .put_account(sample_account(scope.clone(), account_id.clone()))
        .await
        .unwrap();
    let session = broker_session(scope.clone(), account_id, Some(1), None);
    let session_id = session.correlation_id();

    store.issue_session(session.clone()).await.unwrap();
    store
        .consume_session_use(&scope, session_id, Utc::now())
        .await
        .unwrap();
    drop(store);

    let reopened = libsql_store(&db_path).await;
    assert!(
        reopened
            .validate_session(&scope, session_id, Utc::now())
            .await
            .unwrap_err()
            .is_use_limit_exceeded()
    );
    assert_eq!(
        reopened.get_session(&scope, session_id).await.unwrap(),
        Some(session)
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_does_not_persist_sensitive_payload_plaintext() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let store = libsql_store(&db_path).await;
    let scope = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    let mut account = sample_account(scope.clone(), account_id.clone());
    account.redacted_metadata = RedactedJson::new(json!({
        "last_four": "1234",
        "refresh_token": "sk-live-sentinel-refresh-token"
    }));
    account.allowed_targets = vec![CredentialTargetPolicy {
        scheme: "https".to_string(),
        host: "sentinel-api.example.com".to_string(),
        port: Some(443),
        path: CredentialPathPolicy::Prefix("/v1/".to_string()),
        methods: vec![NetworkMethod::Get],
    }];
    let session = broker_session(scope.clone(), account_id.clone(), Some(1), None);

    store.put_account(account).await.unwrap();
    store.issue_session(session).await.unwrap();
    drop(store);

    let raw_database = std::fs::read(&db_path).unwrap();
    let raw_database = String::from_utf8_lossy(&raw_database);
    assert!(
        !raw_database.contains("sk-live-sentinel-refresh-token"),
        "credential account metadata must be encrypted at rest"
    );
    assert!(
        !raw_database.contains("sentinel-api.example.com"),
        "credential target policy payload must be encrypted at rest"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_rejects_existing_plaintext_payload_rows() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let db = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let conn = db.connect().unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE reborn_credential_accounts (
            tenant_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            project_id TEXT NOT NULL,
            account_id TEXT NOT NULL,
            status TEXT NOT NULL,
            provider_or_extension_id TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            payload TEXT NOT NULL,
            PRIMARY KEY (tenant_id, user_id, agent_id, project_id, account_id)
        );
        CREATE TABLE reborn_credential_sessions (
            tenant_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            project_id TEXT NOT NULL,
            mission_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            invocation_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            account_id TEXT NOT NULL,
            expires_at TEXT,
            max_uses INTEGER,
            uses INTEGER NOT NULL DEFAULT 0,
            payload TEXT NOT NULL,
            PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id)
        );
        "#,
    )
    .await
    .unwrap();
    let scope = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    let mut account = sample_account(scope.clone(), account_id.clone());
    account.redacted_metadata = RedactedJson::new(json!({
        "refresh_token": "legacy-sk-live-sentinel-refresh-token"
    }));
    let key = db_scope_key(&scope);
    conn.execute(
        "INSERT INTO reborn_credential_accounts (tenant_id, user_id, agent_id, project_id, account_id, status, provider_or_extension_id, updated_at, payload) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8)",
        libsql::params![
            key.tenant_id,
            key.user_id,
            key.agent_id,
            key.project_id,
            account_id.as_str(),
            account.provider_or_extension_id.as_str(),
            account.updated_at.to_rfc3339(),
            serde_json::to_string(&account).unwrap(),
        ],
    )
    .await
    .unwrap();
    drop(conn);

    let store = LibSqlCredentialStore::new(db, test_crypto());
    let error = store.run_migrations().await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unencrypted legacy payload rows"),
        "durable Reborn credential stores must fail closed instead of pretending to securely scrub prior plaintext rows: {error}"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_rejects_sql_injected_scope_values() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let store = libsql_store(&db_path).await;
    let scope = sample_scope("tenant-a", "user-a");
    let injected_scope = sample_scope("tenant-a' OR '1'='1", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    let account = sample_account(scope.clone(), account_id.clone());

    store.put_account(account).await.unwrap();

    assert_eq!(
        store
            .get_account(&injected_scope, &account_id)
            .await
            .unwrap(),
        None,
        "scope fields must be bound parameters, not SQL string fragments"
    );
    assert!(
        store
            .accounts_for_scope(&injected_scope)
            .await
            .unwrap()
            .is_empty(),
        "injected tenant ids must not escape scope isolation"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_keeps_legacy_session_table_when_fk_migration_fails() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let db = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
    let conn = db.connect().unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE reborn_credential_accounts (
            tenant_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            project_id TEXT NOT NULL,
            account_id TEXT NOT NULL,
            status TEXT NOT NULL,
            provider_or_extension_id TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            payload TEXT NOT NULL DEFAULT '{}',
            encrypted_payload BLOB NOT NULL DEFAULT X'',
            payload_key_salt BLOB NOT NULL DEFAULT X'',
            PRIMARY KEY (tenant_id, user_id, agent_id, project_id, account_id)
        );
        CREATE TABLE reborn_credential_sessions (
            tenant_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            project_id TEXT NOT NULL,
            mission_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            invocation_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            account_id TEXT NOT NULL,
            expires_at TEXT,
            max_uses INTEGER,
            uses INTEGER NOT NULL DEFAULT 0,
            payload TEXT NOT NULL DEFAULT '{}',
            encrypted_payload BLOB NOT NULL DEFAULT X'00',
            payload_key_salt BLOB NOT NULL DEFAULT X'00',
            PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id)
        );
        INSERT INTO reborn_credential_sessions (
            tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id,
            session_id, account_id, encrypted_payload, payload_key_salt
        ) VALUES (
            'tenant-a', 'user-a', 'agent-a', 'project-a', 'mission-a', 'thread-a', 'invocation-a',
            'session-orphan', 'missing-account', X'01', X'02'
        );
        "#,
    )
    .await
    .unwrap();
    drop(conn);

    let store = LibSqlCredentialStore::new(db.clone(), test_crypto());
    let error = store.run_migrations().await.unwrap_err();
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("constraint"),
        "orphan legacy sessions should fail FK backfill: {error}"
    );

    let conn = db.connect().unwrap();
    let mut rows = conn
        .query("SELECT account_id FROM reborn_credential_sessions", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    let account_id: String = row.get(0).unwrap();
    assert_eq!(account_id, "missing-account");
    let mut temp_rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'reborn_credential_sessions_with_account_fk'",
            (),
        )
        .await
        .unwrap();
    assert!(temp_rows.next().await.unwrap().is_none());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_rejects_sessions_without_matching_account_row() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let store = libsql_store(&db_path).await;
    let scope = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    let session = broker_session(scope.clone(), account_id.clone(), Some(1), None);

    let error = store.issue_session(session.clone()).await.unwrap_err();
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("constraint"),
        "sessions must be constrained to an existing credential account row: {error}"
    );

    store
        .put_account(sample_account(scope, account_id))
        .await
        .unwrap();
    store.issue_session(session).await.unwrap();
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_concurrent_session_consumption_respects_max_uses() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let store = Arc::new(libsql_store(&db_path).await);
    let scope = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    store
        .put_account(sample_account(scope.clone(), account_id.clone()))
        .await
        .unwrap();
    let session = broker_session(scope.clone(), account_id, Some(1), None);
    let session_id = session.correlation_id();
    store.issue_session(session).await.unwrap();

    let first = {
        let store = Arc::clone(&store);
        let scope = scope.clone();
        tokio::spawn(async move {
            store
                .consume_session_use(&scope, session_id, Utc::now())
                .await
        })
    };
    let second = {
        let store = Arc::clone(&store);
        let scope = scope.clone();
        tokio::spawn(async move {
            store
                .consume_session_use(&scope, session_id, Utc::now())
                .await
        })
    };

    let results = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(error) if error.is_use_limit_exceeded()))
            .count(),
        1
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_accounts_for_scope_rejects_over_limit_result_sets() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let store = libsql_store(&db_path).await;
    let scope = sample_scope("tenant-a", "user-a");

    for index in 0..1000 {
        let account_id = CredentialAccountId::new(format!("acct_{index:04}")).unwrap();
        store
            .put_account(sample_account(scope.clone(), account_id))
            .await
            .unwrap();
    }
    assert_eq!(store.accounts_for_scope(&scope).await.unwrap().len(), 1000);

    store
        .put_account(sample_account(
            scope.clone(),
            CredentialAccountId::new("acct_overflow").unwrap(),
        ))
        .await
        .unwrap();
    let error = store.accounts_for_scope(&scope).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("more than 1000 credential accounts"),
        "large account listings must fail closed before unbounded allocation: {error}"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_credential_store_persists_session_expiry_across_reopen() {
    let dir = tempfile::tempdir().unwrap().keep();
    let db_path = dir.join("credentials.db");
    let store = libsql_store(&db_path).await;
    let scope = sample_scope("tenant-a", "user-a");
    let account_id = CredentialAccountId::new("openai_prod").unwrap();
    store
        .put_account(sample_account(scope.clone(), account_id.clone()))
        .await
        .unwrap();
    let session = broker_session(
        scope.clone(),
        account_id,
        None,
        Some(Utc::now() - chrono::Duration::seconds(1)),
    );
    let session_id = session.correlation_id();

    store.issue_session(session).await.unwrap();
    drop(store);

    let reopened = libsql_store(&db_path).await;
    assert!(
        reopened
            .validate_session(&scope, session_id, Utc::now())
            .await
            .unwrap_err()
            .is_expired()
    );
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_credential_store_persists_accounts_and_sessions_when_database_url_is_set() {
    let Some(store) = postgres_store().await else {
        return;
    };
    let suffix = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let scope = sample_scope(&format!("tenant-{suffix}"), "user-a");
    let other_scope = sample_scope(&format!("tenant-other-{suffix}"), "user-a");
    let account_id = CredentialAccountId::new(format!("openai_{suffix}")).unwrap();
    let account = sample_account(scope.clone(), account_id.clone());
    let session = broker_session(scope.clone(), account_id.clone(), Some(1), None);
    let session_id = session.correlation_id();

    store.put_account(account.clone()).await.unwrap();
    store.issue_session(session.clone()).await.unwrap();
    store
        .consume_session_use(&scope, session_id, Utc::now())
        .await
        .unwrap();

    assert_eq!(
        store.get_account(&scope, &account_id).await.unwrap(),
        Some(account)
    );
    assert_eq!(
        store.get_account(&other_scope, &account_id).await.unwrap(),
        None
    );
    assert!(
        store
            .validate_session(&scope, session_id, Utc::now())
            .await
            .unwrap_err()
            .is_use_limit_exceeded()
    );
    assert_eq!(
        store.get_session(&scope, session_id).await.unwrap(),
        Some(session)
    );
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_credential_store_rejects_sessions_without_matching_account_row_when_database_url_is_set()
 {
    let Some(store) = postgres_store().await else {
        return;
    };
    let suffix = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let scope = sample_scope(&format!("tenant-missing-{suffix}"), "user-a");
    let account_id = CredentialAccountId::new(format!("openai_missing_{suffix}")).unwrap();
    let session = broker_session(scope.clone(), account_id.clone(), Some(1), None);

    let error = store.issue_session(session.clone()).await.unwrap_err();
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("constraint"),
        "sessions must be constrained to an existing credential account row: {error}"
    );

    store
        .put_account(sample_account(scope, account_id))
        .await
        .unwrap();
    store.issue_session(session).await.unwrap();
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_credential_store_accounts_for_scope_rejects_over_limit_result_sets_when_database_url_is_set()
 {
    let Some(store) = postgres_store().await else {
        return;
    };
    let suffix = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let scope = sample_scope(&format!("tenant-many-{suffix}"), "user-a");

    for index in 0..1000 {
        let account_id = CredentialAccountId::new(format!("acct_{suffix}_{index:04}")).unwrap();
        store
            .put_account(sample_account(scope.clone(), account_id))
            .await
            .unwrap();
    }
    assert_eq!(store.accounts_for_scope(&scope).await.unwrap().len(), 1000);

    store
        .put_account(sample_account(
            scope.clone(),
            CredentialAccountId::new(format!("acct_{suffix}_overflow")).unwrap(),
        ))
        .await
        .unwrap();
    let error = store.accounts_for_scope(&scope).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("more than 1000 credential accounts"),
        "large account listings must fail closed before unbounded allocation: {error}"
    );
}

#[cfg(feature = "libsql")]
async fn libsql_store(path: &std::path::Path) -> LibSqlCredentialStore {
    let db = Arc::new(libsql::Builder::new_local(path).build().await.unwrap());
    let store = LibSqlCredentialStore::new(db, test_crypto());
    store.run_migrations().await.unwrap();
    store
}

#[cfg(feature = "postgres")]
async fn postgres_store() -> Option<PostgresCredentialStore> {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping Postgres credential store contract: DATABASE_URL is not set");
        return None;
    };
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("DATABASE_URL must parse as a Postgres connection string");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("Postgres pool must build");
    let store = PostgresCredentialStore::new(pool, test_crypto());
    store
        .run_migrations()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres test database");
    Some(store)
}

fn test_crypto() -> Arc<SecretsCrypto> {
    Arc::new(
        SecretsCrypto::new(SecretMaterial::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        ))
        .unwrap(),
    )
}

fn broker_session(
    scope: ResourceScope,
    account_id: CredentialAccountId,
    max_uses: Option<u64>,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> ironclaw_secrets::CredentialSession {
    let broker = InMemoryCredentialBroker::new();
    broker
        .put_account(sample_account(scope.clone(), account_id.clone()))
        .unwrap();
    broker
        .create_session(CredentialSessionRequest {
            invocation_id: scope.invocation_id,
            scope,
            capability_id: CapabilityId::new("openai.chat").unwrap(),
            extension_id: ExtensionId::new("openai").unwrap(),
            account_id,
            method: NetworkMethod::Get,
            url: "https://api.example.com/v1/models".to_string(),
            expires_at,
            max_uses,
        })
        .unwrap()
}

#[cfg(feature = "libsql")]
struct TestDbScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: String,
    project_id: String,
}

#[cfg(feature = "libsql")]
fn db_scope_key(scope: &ResourceScope) -> TestDbScopeKey {
    TestDbScopeKey {
        tenant_id: scope.tenant_id.to_string(),
        user_id: scope.user_id.to_string(),
        agent_id: scope
            .agent_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        project_id: scope
            .project_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    }
}

fn sample_account(scope: ResourceScope, id: CredentialAccountId) -> CredentialAccount {
    CredentialAccount {
        scope,
        id,
        provider_or_extension_id: ExtensionId::new("openai").unwrap(),
        label: "Production".to_string(),
        status: CredentialAccountStatus::Active,
        secret_handles: vec![SecretHandle::new("openai_key").unwrap()],
        allowed_targets: vec![CredentialTargetPolicy {
            scheme: "https".to_string(),
            host: "api.example.com".to_string(),
            port: Some(443),
            path: CredentialPathPolicy::Prefix("/v1/".to_string()),
            methods: vec![NetworkMethod::Get],
        }],
        redacted_metadata: RedactedJson::new(json!({ "last_four": "1234" })),
        updated_at: Utc::now(),
    }
}

fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: Some(AgentId::new("agent-a").unwrap()),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}
