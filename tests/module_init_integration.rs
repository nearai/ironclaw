//! Integration test for module-owned initialization factories.
//!
//! Verifies that the refactored factory functions in `db`, `secrets`,
//! `orchestrator`, and `extensions` modules wire up correctly end-to-end,
//! ensuring nothing was lost when initialization logic was moved out of
//! `main.rs` and `app.rs` into owning modules.

use std::sync::Arc;

use ironclaw::db::DatabaseHandles;
use ironclaw::secrets::{CreateSecretParams, SecretsCrypto, SecretsStore};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a libsql DatabaseConfig pointing at a temp file.
#[cfg(feature = "libsql")]
fn libsql_config(path: &std::path::Path) -> ironclaw::config::DatabaseConfig {
    ironclaw::config::DatabaseConfig {
        backend: ironclaw::config::DatabaseBackend::LibSql,
        url: secrecy::SecretString::from(String::new()),
        pool_size: 1,
        ssl_mode: ironclaw::config::SslMode::Prefer,
        libsql_path: Some(path.to_path_buf()),
        libsql_url: None,
        libsql_auth_token: None,
    }
}

/// Build a master-key crypto instance for tests.
fn test_crypto() -> Arc<SecretsCrypto> {
    let key = secrecy::SecretString::from(ironclaw::secrets::keychain::generate_master_key_hex());
    Arc::new(SecretsCrypto::new(key).expect("test crypto"))
}

// ---------------------------------------------------------------------------
// connect_with_handles: returns Database + populated handles
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
#[tokio::test]
async fn connect_with_handles_returns_db_and_libsql_handle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let config = libsql_config(&db_path);

    let (db, handles) = ironclaw::db::connect_with_handles(&config)
        .await
        .expect("connect_with_handles");

    // Database trait object works — run a trivial operation.
    db.run_migrations().await.expect("migrations");

    // Handle is populated.
    assert!(
        handles.libsql_db.is_some(),
        "libsql handle should be Some after connect_with_handles"
    );
}

// ---------------------------------------------------------------------------
// connect_from_config delegates to connect_with_handles
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
#[tokio::test]
async fn connect_from_config_produces_working_db() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let config = libsql_config(&db_path);

    // connect_from_config delegates to connect_with_handles internally.
    let db = ironclaw::db::connect_from_config(&config)
        .await
        .expect("connect_from_config");

    // Verify usable — migrations should be idempotent.
    db.run_migrations().await.expect("migrations");
}

// ---------------------------------------------------------------------------
// secrets::create_secrets_store from DatabaseHandles
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
#[tokio::test]
async fn secrets_store_from_handles_round_trips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let config = libsql_config(&db_path);

    let (_db, handles) = ironclaw::db::connect_with_handles(&config)
        .await
        .expect("connect");

    let crypto = test_crypto();
    let store = ironclaw::secrets::create_secrets_store(crypto, &handles)
        .expect("create_secrets_store should return Some for libsql");

    // Round-trip a secret to prove the store works.
    store
        .create("test", CreateSecretParams::new("test_key", "test_value"))
        .await
        .expect("create secret");

    let decrypted = store
        .get_decrypted("test", "test_key")
        .await
        .expect("get_decrypted");
    assert_eq!(decrypted.expose(), "test_value");
}

// ---------------------------------------------------------------------------
// db::create_secrets_store (standalone CLI factory)
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
#[tokio::test]
async fn db_create_secrets_store_standalone_round_trips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let config = libsql_config(&db_path);
    let crypto = test_crypto();

    let store = ironclaw::db::create_secrets_store(&config, crypto)
        .await
        .expect("db::create_secrets_store");

    store
        .create(
            "test",
            CreateSecretParams::new("standalone_key", "standalone_value"),
        )
        .await
        .expect("create secret");

    let decrypted = store
        .get_decrypted("test", "standalone_key")
        .await
        .expect("get_decrypted");
    assert_eq!(decrypted.expose(), "standalone_value");
}

// ---------------------------------------------------------------------------
// Both secrets factories produce equivalent stores
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
#[tokio::test]
async fn both_secrets_factories_produce_compatible_stores() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let config = libsql_config(&db_path);
    let crypto = test_crypto();

    // Factory 1: connect_with_handles + secrets::create_secrets_store
    let (_db, handles) = ironclaw::db::connect_with_handles(&config)
        .await
        .expect("connect");
    let store_a = ironclaw::secrets::create_secrets_store(Arc::clone(&crypto), &handles)
        .expect("store from handles");

    // Factory 2: db::create_secrets_store (standalone)
    let store_b = ironclaw::db::create_secrets_store(&config, crypto)
        .await
        .expect("standalone store");

    // Write with factory 1, read with factory 2.
    store_a
        .create(
            "test",
            CreateSecretParams::new("cross_factory", "shared_secret"),
        )
        .await
        .expect("create via store_a");

    let decrypted = store_b
        .get_decrypted("test", "cross_factory")
        .await
        .expect("read via store_b");
    assert_eq!(decrypted.expose(), "shared_secret");
}

// ---------------------------------------------------------------------------
// ExtensionManager constructs with McpProcessManager
// ---------------------------------------------------------------------------

#[tokio::test]
async fn extension_manager_with_process_manager_constructs() {
    use ironclaw::extensions::ExtensionManager;
    use ironclaw::secrets::InMemorySecretsStore;
    use ironclaw::tools::ToolRegistry;
    use ironclaw::tools::mcp::McpProcessManager;
    use ironclaw::tools::mcp::McpSessionManager;

    let crypto = test_crypto();
    let secrets: Arc<dyn SecretsStore + Send + Sync> =
        Arc::new(InMemorySecretsStore::new(crypto));
    let tools = Arc::new(ToolRegistry::new());
    let tools_dir = tempfile::tempdir().expect("tools_dir");
    let channels_dir = tempfile::tempdir().expect("channels_dir");

    let manager = ExtensionManager::new(
        Arc::new(McpSessionManager::new()),
        Arc::new(McpProcessManager::new()),
        secrets,
        tools,
        None,
        None,
        tools_dir.path().to_path_buf(),
        channels_dir.path().to_path_buf(),
        None,
        "test".to_string(),
        None,
        Vec::new(),
    );

    // Verify the manager is functional — list returns Ok.
    let result = manager.list(None, false).await;
    assert!(result.is_ok(), "list should succeed on empty manager");
    assert!(result.unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// DatabaseHandles: default is empty
// ---------------------------------------------------------------------------

#[test]
fn database_handles_default_is_empty() {
    let handles = DatabaseHandles::default();

    #[cfg(feature = "postgres")]
    assert!(handles.pg_pool.is_none());

    #[cfg(feature = "libsql")]
    assert!(handles.libsql_db.is_none());
}
