#![cfg(all(feature = "migrate", feature = "libsql"))]

use std::path::Path;
use std::sync::Arc;

use ironclaw::bridge::HybridStore;
use ironclaw::cli::{MigrateCommand, run_migrate_command_with_services};
use ironclaw::db::Database;
use ironclaw::db::SettingsStore;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::migrate::MigrationServices;
use ironclaw::secrets::{LibSqlSecretsStore, SecretsCrypto, SecretsStore};
use ironclaw::workspace::{Workspace, WorkspaceSettingsAdapter};
use ironclaw_engine::{Project, Store};
use secrecy::SecretString;
use tempfile::TempDir;
use uuid::Uuid;

async fn make_services() -> (MigrationServices, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("target.db");
    let backend = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("new_local"));
    backend.run_migrations().await.expect("migrations");

    let db: Arc<dyn Database> = backend.clone();
    let workspace = Arc::new(Workspace::new_with_db("alice", Arc::clone(&db)));
    let settings_adapter = Arc::new(WorkspaceSettingsAdapter::new(
        Arc::clone(&workspace),
        Arc::clone(&db),
    ));
    settings_adapter
        .ensure_system_config()
        .await
        .expect("seed system config");
    let settings_store: Arc<dyn SettingsStore + Send + Sync> = settings_adapter;

    let crypto = Arc::new(
        SecretsCrypto::new(SecretString::from(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        ))
        .expect("crypto"),
    );
    let secrets_store: Arc<dyn SecretsStore + Send + Sync> =
        Arc::new(LibSqlSecretsStore::new(backend.shared_db(), crypto));

    let engine_store = Arc::new(HybridStore::new(Some(Arc::clone(&workspace))));
    engine_store.load_state_from_workspace().await;
    let project = Project::new("alice", "default", "test project");
    let project_id = project.id;
    Store::save_project(engine_store.as_ref(), &project)
        .await
        .expect("save project");

    (
        MigrationServices {
            user_id: "alice".to_string(),
            db,
            workspace,
            settings_store,
            secrets_store,
            engine_store,
            project_id,
        },
        dir,
    )
}

async fn create_openclaw_source(root: &Path) {
    std::fs::write(
        root.join("openclaw.json"),
        r#"{
            llm: {
                provider: "openai",
                model: "gpt-4o",
                api_key: "sk-openclaw-test"
            },
            embeddings: {
                provider: "openai",
                model: "text-embedding-3-small",
                api_key: "sk-openclaw-embed"
            }
        }"#,
    )
    .expect("write config");

    let workspace_dir = root.join("workspace");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
    std::fs::write(
        workspace_dir.join("notes.md"),
        "# Imported\n\nOpenClaw workspace doc",
    )
    .expect("workspace doc");

    let agents_dir = root.join("agents");
    std::fs::create_dir_all(&agents_dir).expect("agents dir");
    let db_path = agents_dir.join("assistant.sqlite");
    let db = libsql::Builder::new_local(&db_path)
        .build()
        .await
        .expect("source db");
    let conn = db.connect().expect("source conn");
    conn.execute_batch(
        "CREATE TABLE chunks (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL,
            content TEXT NOT NULL,
            embedding BLOB,
            chunk_index INTEGER NOT NULL
        );
        CREATE TABLE conversations (
            id TEXT PRIMARY KEY,
            channel TEXT,
            created_at TEXT
        );
        CREATE TABLE messages (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT
        );",
    )
    .await
    .expect("schema");

    let conversation_id = "openclaw-conv-1";
    conn.execute(
        "INSERT INTO chunks (id, path, content, embedding, chunk_index) VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params!["chunk-1", "memory/topic.md", "Remember the migration plan", libsql::Value::Null, 0i64],
    )
    .await
    .expect("chunk");
    conn.execute(
        "INSERT INTO conversations (id, channel, created_at) VALUES (?1, ?2, ?3)",
        libsql::params![conversation_id, "slack", "2026-01-01T10:00:00Z"],
    )
    .await
    .expect("conversation");
    conn.execute(
        "INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params!["msg-1", conversation_id, "user", "hello from openclaw", "2026-01-01T10:00:00Z"],
    )
    .await
    .expect("message1");
    conn.execute(
        "INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params!["msg-2", conversation_id, "assistant", "hi from ironclaw migration", "2026-01-01T10:00:05Z"],
    )
    .await
    .expect("message2");
}

async fn create_hermes_source(root: &Path) {
    std::fs::write(
        root.join("config.yaml"),
        "model:\n  provider: local\n  default: hermes-model\n  base_url: http://localhost:1234/v1\nproviders:\n  local:\n    name: Local Provider\n    base_url: http://localhost:1234/v1\n    key_env: LOCAL_MODEL_API_KEY\n    transport: openai_chat\n    model: hermes-model\n",
    )
    .expect("config.yaml");
    std::fs::write(root.join(".env"), "LOCAL_MODEL_API_KEY=sk-hermes-local\n").expect("env");
    std::fs::write(root.join("auth.json"), "{\"active_provider\":\"local\"}").expect("auth");
    std::fs::write(root.join("SOUL.md"), "# Soul\n\nHermes remembers this.").expect("soul");

    let db_path = root.join("state.db");
    let db = libsql::Builder::new_local(&db_path)
        .build()
        .await
        .expect("state db");
    let conn = db.connect().expect("state conn");
    conn.execute_batch(
        "CREATE TABLE sessions (
            id TEXT PRIMARY KEY,
            source TEXT,
            user_id TEXT,
            model TEXT,
            title TEXT,
            started_at TEXT
        );
        CREATE TABLE messages (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp TEXT,
            tool_name TEXT
        );",
    )
    .await
    .expect("schema");
    conn.execute(
        "INSERT INTO sessions (id, source, user_id, model, title, started_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        libsql::params!["session-1", "cli", "hermes-user", "hermes-model", "Hermes Session", "2026-02-02T12:00:00Z"],
    )
    .await
    .expect("session");
    conn.execute(
        "INSERT INTO messages (id, session_id, role, content, timestamp, tool_name) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        libsql::params!["hermes-msg-1", "session-1", "user", "hello from hermes", "2026-02-02T12:00:00Z", libsql::Value::Null],
    )
    .await
    .expect("message1");
    conn.execute(
        "INSERT INTO messages (id, session_id, role, content, timestamp, tool_name) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        libsql::params!["hermes-msg-2", "session-1", "assistant", "Hermes answer", "2026-02-02T12:00:10Z", libsql::Value::Null],
    )
    .await
    .expect("message2");
}

fn migration_uuid(kind: &str, source: &str, namespace: &str, external_id: &str) -> Uuid {
    let seed = format!("ironclaw-migrate::{kind}::{source}::{namespace}::{external_id}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, seed.as_bytes())
}

#[tokio::test]
async fn openclaw_command_migrates_into_engine_and_legacy_history() {
    let (services, _tmp) = make_services().await;
    let source = TempDir::new().expect("source tempdir");
    create_openclaw_source(source.path()).await;

    let cmd = MigrateCommand::Openclaw {
        path: Some(source.path().to_path_buf()),
        dry_run: false,
        user_id: None,
    };
    let stats = run_migrate_command_with_services(&cmd, &services)
        .await
        .expect("migrate openclaw");

    assert!(
        stats.workspace_documents >= 2,
        "workspace + chunk docs should be written"
    );
    assert_eq!(stats.engine_threads, 1);
    assert_eq!(stats.engine_conversations, 1);
    assert_eq!(stats.legacy_conversations, 1);
    assert_eq!(stats.messages, 2);
    assert!(
        stats.settings >= 4,
        "llm + embeddings settings should be persisted"
    );
    assert!(
        services
            .secrets_store
            .exists("alice", "llm_builtin_openai_api_key")
            .await
            .expect("secret exists"),
        "OpenAI key should be migrated into persistent secrets"
    );

    let threads = Store::list_threads(services.engine_store.as_ref(), services.project_id, "alice")
        .await
        .expect("list threads");
    assert_eq!(
        threads.len(),
        1,
        "imported engine thread should be visible in default project"
    );

    let imported_doc = services
        .workspace
        .read("imports/openclaw/root/workspace/notes.md")
        .await
        .expect("imported workspace doc");
    assert!(imported_doc.content.contains("OpenClaw workspace doc"));

    let legacy_id = migration_uuid(
        "legacy-conversation",
        "openclaw",
        "assistant",
        "openclaw-conv-1",
    );
    let metadata = services
        .db
        .get_conversation_metadata(legacy_id)
        .await
        .expect("metadata lookup")
        .expect("metadata present");
    assert_eq!(
        metadata.get("thread_type").and_then(|v| v.as_str()),
        Some("migration")
    );
}

#[tokio::test]
async fn hermes_command_migrates_custom_provider_and_session_history() {
    let (services, _tmp) = make_services().await;
    let source = TempDir::new().expect("source tempdir");
    create_hermes_source(source.path()).await;

    let cmd = MigrateCommand::Hermes {
        path: Some(source.path().to_path_buf()),
        profiles: Vec::new(),
        all_profiles: false,
        dry_run: false,
        user_id: None,
    };
    let stats = run_migrate_command_with_services(&cmd, &services)
        .await
        .expect("migrate hermes");

    assert_eq!(stats.engine_threads, 1);
    assert_eq!(stats.engine_conversations, 1);
    assert_eq!(stats.legacy_conversations, 1);
    assert_eq!(stats.messages, 2);
    assert!(stats.settings >= 2, "Hermes settings should be written");
    assert!(
        services
            .secrets_store
            .exists("alice", "llm_custom_local_api_key")
            .await
            .expect("custom provider secret"),
        "Hermes custom provider secret should be migrated"
    );
    assert!(
        services
            .secrets_store
            .exists("alice", "migrate_hermes_default_auth_json")
            .await
            .expect("auth backup secret"),
        "auth.json should be preserved as encrypted backup"
    );

    let settings = ironclaw::settings::Settings::from_db_map(
        &services
            .settings_store
            .get_all_settings("alice")
            .await
            .expect("all settings"),
    );
    assert_eq!(settings.llm_backend.as_deref(), Some("local"));
    assert_eq!(settings.selected_model.as_deref(), Some("hermes-model"));
    assert_eq!(settings.llm_custom_providers.len(), 1);
    assert_eq!(settings.llm_custom_providers[0].id, "local");

    let imported_doc = services
        .workspace
        .read("imports/hermes/default/SOUL.md")
        .await
        .expect("soul doc");
    assert!(imported_doc.content.contains("Hermes remembers this"));
}

#[tokio::test]
async fn rerun_migration_is_idempotent() {
    let (services, _tmp) = make_services().await;
    let source = TempDir::new().expect("source tempdir");
    create_openclaw_source(source.path()).await;

    let cmd = MigrateCommand::Openclaw {
        path: Some(source.path().to_path_buf()),
        dry_run: false,
        user_id: None,
    };

    // First run
    let stats1 = run_migrate_command_with_services(&cmd, &services)
        .await
        .expect("first migration");
    assert!(stats1.engine_threads >= 1);
    assert!(stats1.legacy_conversations >= 1);

    // Second run — must not error (idempotency)
    let stats2 = run_migrate_command_with_services(&cmd, &services)
        .await
        .expect("re-run migration should succeed");

    // On re-run, unchanged documents should be skipped
    assert!(
        stats2.skipped > 0 || stats2.workspace_documents == 0,
        "re-run should skip unchanged documents or write zero new ones"
    );
}
