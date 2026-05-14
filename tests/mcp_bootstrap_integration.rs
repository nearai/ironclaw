//! Regression coverage for `bootstrap_t3n_mcp_server` and
//! `bootstrap_nearai_mcp_server`.
//!
//! Background: on staging the t3n-mcp entry was present in the on-disk
//! `~/.t3claw/mcp-servers.json` (the legacy single-user store) but
//! absent from the `settings.mcp_servers` row that the gateway reads. The
//! web UI showed no custom MCP server. Root cause was that
//! `bootstrap_t3n_mcp_server` consulted `load_mcp_servers_from_db`, whose
//! silent disk fallback returned the disk-resident entry as if it were
//! persisted, short-circuiting the DB write.
//!
//! This test drives the public caller (`load_mcp_servers_ready`) rather
//! than the helper in isolation, per the "test through the caller, not
//! just the helper" rule in `.claude/rules/testing.md`.

#![cfg(feature = "libsql")]

use t3claw::db::libsql::LibSqlBackend;
use t3claw::db::{Database, SettingsStore};
use t3claw::tools::mcp::config;
use t3claw::tools::mcp::{McpServerConfig, McpServersFile};
use tempfile::TempDir;

const TEST_USER_ID: &str = "test-owner";

fn write_disk_mcp_servers_with_t3n(base_dir: &std::path::Path, socket_path: &str) {
    let server = McpServerConfig::new_unix("t3n-mcp", socket_path)
        .with_description("pre-existing on-disk entry");
    let file = McpServersFile {
        servers: vec![server],
        schema_version: 1,
    };
    let path = base_dir.join("mcp-servers.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&file).expect("serialize"))
        .expect("write mcp-servers.json");
}

#[tokio::test]
async fn load_mcp_servers_ready_persists_disk_only_t3n_entry_to_db() {
    // Pin the t3claw base dir to a tempdir BEFORE any code reads it,
    // because `t3claw_base_dir()` caches in a `LazyLock`.
    let base_dir = TempDir::new().expect("base dir");
    // SAFETY: tests run in their own process; this env-var write happens
    // before any other code reads the LazyLock-cached base dir.
    unsafe {
        std::env::set_var("T3CLAW_BASE_DIR", base_dir.path());
    }

    let socket_path = base_dir.path().join("t3n-mcp.sock");
    let socket_str = socket_path.to_string_lossy().into_owned();

    // Mirror the staging condition: mcp-servers.json contains t3n-mcp, but
    // the DB has no `mcp_servers` setting row.
    write_disk_mcp_servers_with_t3n(base_dir.path(), &socket_str);

    // SAFETY: same justification as above; single-threaded test setup.
    unsafe {
        std::env::set_var("T3N_MCP_SOCKET_PATH", &socket_str);
    }

    let db_path = base_dir.path().join("t3claw.db");
    let db = LibSqlBackend::new_local(&db_path)
        .await
        .expect("open libsql backend");
    db.run_migrations().await.expect("run migrations");

    // Precondition: DB has no row yet.
    let before = db
        .get_setting(TEST_USER_ID, "mcp_servers")
        .await
        .expect("get_setting");
    assert!(
        before.is_none(),
        "expected no DB row before bootstrap, got {:?}",
        before
    );

    // Drive the public caller (the path agent startup actually uses).
    let servers =
        config::load_mcp_servers_ready(Some(&db as &dyn t3claw::db::Database), TEST_USER_ID)
            .await
            .expect("load_mcp_servers_ready");

    assert!(
        servers.get("t3n-mcp").is_some(),
        "returned config should contain t3n-mcp; got {:?}",
        servers.servers.iter().map(|s| &s.name).collect::<Vec<_>>()
    );

    // The real regression assertion: the DB row must now exist, so the
    // gateway (which reads the DB directly with no disk fallback) sees it.
    let after = db
        .get_setting(TEST_USER_ID, "mcp_servers")
        .await
        .expect("get_setting after bootstrap")
        .expect("DB row must be present after bootstrap");

    let cfg: t3claw::tools::mcp::McpServersFile =
        serde_json::from_value(after).expect("parse mcp_servers JSON from DB");
    assert!(
        cfg.get("t3n-mcp").is_some(),
        "DB row must contain t3n-mcp; got {:?}",
        cfg.servers.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}
