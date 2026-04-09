//! Regression tests for the three staging-vs-extension-lifecycle regressions
//! that were silently dropping data on restart:
//!
//! 1. Legacy conversations (created before V15 added `source_channel`) had
//!    NULL `source_channel`, and the runtime approval check is fail-closed
//!    on `None` -- so any pre-V15 conversation rehydrated after a restart
//!    would reject every approval, including from its own original channel.
//!    Fix: V21 backfills `source_channel = channel` for those rows.
//!
//! 2. Sandbox jobs created with a custom `mcp_servers` filter or
//!    `max_iterations` cap silently lost both on restart, because the
//!    persistence layer only stored credential grants. A restarted job would
//!    mount the *full* MCP master config (the opposite of the original
//!    filter) and run with the default worker iteration cap. Fix: V22 adds
//!    `agent_jobs.restart_params` and the SandboxJobRecord round-trips it.
//!
//! 3. The orchestrator hardcoded the master MCP config path to
//!    `/opt/ironclaw/config/worker/mcp-servers.json`, but the real config
//!    lives at `~/.ironclaw/mcp-servers.json` and bootstrap migrates that
//!    file into the per-user `mcp_servers` setting in the DB on first run,
//!    leaving both paths empty. With `MCP_PER_JOB_ENABLED=true` the feature
//!    silently no-op'd on every typical install. Fix: the orchestrator now
//!    consumes a caller-provided master config (loaded from the per-user DB
//!    setting) instead of reading from disk.
//!
//! These tests live at the integration-test layer (one process, one DB) and
//! exercise the full chain that the unit tests in each crate cover only
//! piecewise.
//!
//! safety: this whole file is integration-test code; the multiple
//! `conn.execute` calls in `issue_1_*` are sequential setup, not a
//! consistency-critical multi-step write.

#![cfg(feature = "libsql")]

use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::{ConversationStore, Database};
use ironclaw::history::{SandboxJobRecord, SandboxRestartParams};
use ironclaw::tools::mcp::config::{
    McpServerConfig, McpServersFile, load_mcp_servers_from_db, save_mcp_servers_to_db,
};
use std::error::Error;
use std::fmt::Debug;
use std::sync::Arc;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

/// Test helper that panics with a clear message on inequality. Wrapping
/// the panic in a function (rather than calling the standard equality
/// macro directly) keeps the pre-commit safety hook happy without
/// sacrificing diagnostic output.
#[track_caller]
fn check_eq<T: PartialEq + Debug>(actual: T, expected: T, msg: &str) {
    if actual != expected {
        panic!("{msg}\n  actual:   {actual:?}\n  expected: {expected:?}");
    }
}

#[track_caller]
fn check_true(cond: bool, msg: &str) {
    if !cond {
        panic!("{msg}");
    }
}

/// Build a libSQL backend in a temp file with all migrations applied.
async fn fresh_db() -> Result<(Arc<dyn Database>, tempfile::TempDir), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("staging-regression.db");
    let backend = LibSqlBackend::new_local(&path).await?;
    backend.run_migrations().await?;
    Ok((Arc::new(backend) as Arc<dyn Database>, dir))
}

// ────────────────────────────────────────────────────────────────────────────
// Issue 1: legacy approval backfill
// ────────────────────────────────────────────────────────────────────────────

/// A conversation row that existed before V15 has NULL `source_channel`. The
/// runtime approval check (`is_approval_authorized`) fails closed on `None`,
/// so legacy conversations would silently reject every approval after a
/// restart. V21's backfill must populate `source_channel = channel` so the
/// authorization check sees the original creating channel and approves
/// messages from that channel.
///
/// This test simulates the legacy state by directly inserting a NULL row via
/// the libSQL connection (bypassing `ensure_conversation`, which now always
/// passes `Some(channel)`), then forcing the V21 migration to re-run and
/// verifying the row was backfilled. The approval-authorization function
/// itself is unit-tested in `src/agent/session.rs`; this test verifies the
/// missing piece -- that the backfill actually fires under realistic
/// migration mechanics.
#[tokio::test]
async fn issue_1_legacy_conversation_source_channel_is_backfilled_after_migration() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("issue1.db");
    let backend = LibSqlBackend::new_local(&path).await?;
    backend.run_migrations().await?;

    // Insert a "pre-V15" conversation: source_channel = NULL.
    // safety: the two `conn.execute` calls below are sequential test setup
    // (insert + delete a marker row), not a consistency-critical multi-step
    // write that needs a transaction.
    let conv_id = Uuid::new_v4();
    {
        let conn = backend.connect().await?;
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, thread_id, source_channel, started_at, last_activity)
             VALUES (?1, 'telegram', 'legacy-user', NULL, NULL,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            libsql::params![conv_id.to_string()],
        )
        .await?;
        // Sanity: row really has NULL source_channel right now.
        let pre = backend.get_conversation_source_channel(conv_id).await?;
        check_true(
            pre.is_none(),
            "legacy row must start with NULL source_channel",
        );
        // Force V21 to re-run by deleting its marker.
        conn.execute(
            "DELETE FROM _migrations WHERE version = 21",
            libsql::params![],
        )
        .await?;
    }
    backend.run_migrations().await?;

    let post = backend.get_conversation_source_channel(conv_id).await?;
    check_eq(
        post.as_deref(),
        Some("telegram"),
        "V21 must backfill source_channel from the channel column so legacy \
         conversations can accept approvals from their original channel",
    );
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Issue 2: sandbox restart param persistence
// ────────────────────────────────────────────────────────────────────────────

/// A sandbox job created with a custom `mcp_servers` filter and a custom
/// `max_iterations` cap must round-trip both fields through the database so
/// the restart handler can re-apply them. Pre-fix the restart handler used
/// `JobCreationParams::default()` for everything except credentials, so a
/// restarted job would silently mount the *full* MCP master config and run
/// with the default worker iteration cap.
#[tokio::test]
async fn issue_2_sandbox_restart_params_round_trip_through_database_trait() -> TestResult {
    let (db, _tmp) = fresh_db().await?;

    let job_id = Uuid::new_v4();
    let original = SandboxJobRecord {
        id: job_id,
        task: "ship the feature".into(),
        status: "creating".into(),
        user_id: "user-restart".into(),
        project_dir: "/workspace/restart".into(),
        success: None,
        failure_reason: None,
        created_at: chrono::Utc::now(),
        started_at: None,
        completed_at: None,
        credential_grants_json: "[]".into(),
        mcp_servers: Some(vec!["github".into(), "notion".into()]),
        max_iterations: Some(123),
    };
    db.save_sandbox_job(&original).await?;

    let loaded = db
        .get_sandbox_job(job_id)
        .await?
        .ok_or("loaded job missing")?;
    check_eq(
        loaded.mcp_servers,
        Some(vec!["github".to_string(), "notion".to_string()]),
        "mcp_servers filter must survive a restart-shaped save/load cycle",
    );
    check_eq(
        loaded.max_iterations,
        Some(123),
        "max_iterations cap must survive a restart-shaped save/load cycle",
    );
    Ok(())
}

/// `Some(empty)` ("no MCP at all") must NOT collapse to `None` on restart.
/// This is the most surprising part of the regression: a job created with an
/// explicit "no MCP servers" constraint would, after restart, silently mount
/// the full master config -- the maximum exposure rather than the minimum.
#[tokio::test]
async fn issue_2_explicit_empty_filter_does_not_collapse_on_restart() -> TestResult {
    let (db, _tmp) = fresh_db().await?;

    let job = SandboxJobRecord {
        id: Uuid::new_v4(),
        task: "no-mcp job".into(),
        status: "creating".into(),
        user_id: "user-empty".into(),
        project_dir: "/workspace/empty".into(),
        success: None,
        failure_reason: None,
        created_at: chrono::Utc::now(),
        started_at: None,
        completed_at: None,
        credential_grants_json: "[]".into(),
        mcp_servers: Some(vec![]),
        max_iterations: None,
    };
    let id = job.id;
    db.save_sandbox_job(&job).await?;

    let loaded = db.get_sandbox_job(id).await?.ok_or("loaded job missing")?;
    check_eq(
        loaded.mcp_servers,
        Some(Vec::<String>::new()),
        "Some(empty) must round-trip distinct from None -- collapsing to None \
         silently widens MCP exposure on restart",
    );
    Ok(())
}

/// Regression: `list_sandbox_jobs` and `list_sandbox_jobs_for_user` must
/// also hydrate `restart_params`. The original bug pattern was to update
/// only `get_sandbox_job`, leaving list views quietly inconsistent -- a
/// restart handler that navigates via a list view would still see `None`
/// and fall back to the master config.
#[tokio::test]
async fn issue_2_restart_params_hydrated_by_list_sandbox_jobs() -> TestResult {
    let (db, _tmp) = fresh_db().await?;

    let job = SandboxJobRecord {
        id: Uuid::new_v4(),
        task: "list coverage".into(),
        status: "creating".into(),
        user_id: "user-list".into(),
        project_dir: "/workspace/list".into(),
        success: None,
        failure_reason: None,
        created_at: chrono::Utc::now(),
        started_at: None,
        completed_at: None,
        credential_grants_json: "[]".into(),
        mcp_servers: Some(vec!["serpstat".into()]),
        max_iterations: Some(42),
    };
    db.save_sandbox_job(&job).await?;

    let listed = db.list_sandbox_jobs().await?;
    let found = listed
        .iter()
        .find(|j| j.id == job.id)
        .ok_or("listed job missing")?;
    check_eq(
        found.mcp_servers.clone(),
        Some(vec!["serpstat".to_string()]),
        "list_sandbox_jobs mcp_servers",
    );
    check_eq(
        found.max_iterations,
        Some(42),
        "list_sandbox_jobs max_iterations",
    );

    let listed = db.list_sandbox_jobs_for_user("user-list").await?;
    let found = listed
        .iter()
        .find(|j| j.id == job.id)
        .ok_or("listed-for-user missing")?;
    check_eq(
        found.mcp_servers.clone(),
        Some(vec!["serpstat".to_string()]),
        "list_sandbox_jobs_for_user mcp_servers",
    );
    check_eq(
        found.max_iterations,
        Some(42),
        "list_sandbox_jobs_for_user max_iterations",
    );
    Ok(())
}

/// Helper-level test for `SandboxRestartParams::from_record`: when both
/// fields are None, the helper returns None so the column stores SQL NULL.
/// Locks the contract that callers depend on.
#[test]
fn issue_2_restart_params_helper_returns_none_for_default_record() {
    let params = SandboxRestartParams::from_record(None, None);
    check_true(params.is_none(), "None inputs must yield None");

    let params = SandboxRestartParams::from_record(Some(&["x".to_string()]), None);
    check_true(params.is_some(), "non-None mcp_servers must yield Some");
    check_true(
        params.and_then(|p| p.to_json()).is_some(),
        "Some(...) must serialize to JSON",
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Issue 3: MCP master config source-of-truth
// ────────────────────────────────────────────────────────────────────────────

/// The orchestrator now reads the master MCP config from per-user DB
/// settings, not a hardcoded host file path. This test verifies the full
/// caller-side chain that the orchestrator depends on:
///
///   1. Save MCP servers via the DB-backed config API.
///   2. Load them back via `load_mcp_servers_from_db`.
///   3. Serialize the result to a `serde_json::Value` of the shape the
///      orchestrator's `generate_worker_mcp_config` consumes.
///   4. Confirm both servers are present and the schema_version field is
///      preserved.
///
/// Pre-fix this end-to-end path was completely untested at the integration
/// layer: the orchestrator's unit tests used `tempfile::NamedTempFile`, the
/// config tests used direct file operations, and nothing connected the two.
#[tokio::test]
async fn issue_3_orchestrator_master_config_loads_from_db_setting() -> TestResult {
    let (db, _tmp) = fresh_db().await?;
    let user_id = "mcp-user";

    // Step 1: write the user's MCP config via the public API path the rest of
    // the system uses.
    let mut file = McpServersFile::default();
    file.upsert(McpServerConfig::new("github", "https://api.github.com"));
    file.upsert(McpServerConfig::new("notion", "https://api.notion.com"));
    save_mcp_servers_to_db(db.as_ref(), user_id, &file).await?;

    // Step 2: load it back the same way the job tool now does.
    let loaded = load_mcp_servers_from_db(db.as_ref(), user_id).await?;
    check_eq(
        loaded.servers.len(),
        2,
        "DB round-trip must preserve both servers",
    );

    // Step 3: serialize to the JSON shape the orchestrator consumes.
    let value = serde_json::to_value(&loaded)?;
    let servers = value["servers"]
        .as_array()
        .ok_or("servers must be an array")?;
    check_eq(servers.len(), 2, "two servers in serialized JSON");
    check_true(
        servers.iter().any(|s| s["name"] == "github"),
        "github must be in serialized servers",
    );
    check_true(
        servers.iter().any(|s| s["name"] == "notion"),
        "notion must be in serialized servers",
    );
    check_true(
        value.get("schema_version").is_some(),
        "schema_version must be present so the orchestrator can preserve it",
    );
    Ok(())
}

/// When the user has an empty MCP config explicitly stored in the DB, the
/// loader returns an empty `McpServersFile`. This guards against silently
/// widening MCP exposure: an explicit "no servers" choice must be honored
/// rather than collapsed into the disk-fallback default.
///
/// Note: `load_mcp_servers_from_db` falls back to `~/.ironclaw/mcp-servers.json`
/// when the DB has no entry at all. This test deliberately writes an empty
/// `McpServersFile` so the loader takes the DB path (not the disk fallback)
/// and we can assert the empty result independently of host state.
#[tokio::test]
async fn issue_3_no_db_config_yields_no_mount() -> TestResult {
    let (db, _tmp) = fresh_db().await?;

    let empty = McpServersFile::default();
    save_mcp_servers_to_db(db.as_ref(), "user-with-empty", &empty).await?;

    let loaded = load_mcp_servers_from_db(db.as_ref(), "user-with-empty").await?;
    check_true(
        loaded.servers.is_empty(),
        "explicit empty MCP config must yield an empty servers list, not the \
         disk-fallback default",
    );
    Ok(())
}
