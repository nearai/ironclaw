//! libSQL/Turso backend for the Database trait.
//!
//! Provides an embedded SQLite-compatible database using Turso's libSQL fork.
//! Supports three modes:
//! - Local embedded (file-based, no server needed)
//! - Turso cloud with embedded replica (sync to cloud)
//! - In-memory (for testing)

mod conversations;
mod identities;
mod jobs;
mod pairing;
mod routines;
mod sandbox;
mod settings;
mod tool_failures;
mod users;
mod workspace;

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use libsql::{Connection, Database as LibSqlDatabase};
use rust_decimal::Decimal;

use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
};
use crate::context::JobState;
use crate::db::Database;
use crate::error::DatabaseError;
use crate::workspace::MemoryDocument;

use crate::db::libsql_migrations;

/// Explicit column list for routines table (matches positional access in `row_to_routine_libsql`).
pub(crate) const ROUTINE_COLUMNS: &str = "\
    id, name, description, user_id, enabled, \
    trigger_type, trigger_config, action_type, action_config, \
    cooldown_secs, max_concurrent, dedup_window_secs, \
    notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention, \
    state, last_run_at, next_fire_at, run_count, consecutive_failures, \
    created_at, updated_at, \
    agent_review_on_success, agent_review_on_failure, agent_review_on_attention";

/// Explicit column list for routine_runs table (matches positional access in `row_to_routine_run_libsql`).
pub(crate) const ROUTINE_RUN_COLUMNS: &str = "\
    id, routine_id, trigger_type, trigger_detail, started_at, \
    status, completed_at, result_summary, tokens_used, job_id, created_at";

/// libSQL/Turso database backend.
///
/// Stores the `Database` handle in an `Arc` so that the same underlying
/// database can be shared with stores (SecretsStore, WasmToolStore) that
/// create their own connections per-operation.
pub struct LibSqlBackend {
    db: Arc<LibSqlDatabase>,
}

impl LibSqlBackend {
    /// Create a new local embedded database.
    pub async fn new_local(path: &Path) -> Result<Self, DatabaseError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DatabaseError::Pool(format!("Failed to create database directory: {}", e))
            })?;
        }

        let db = libsql::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to open libSQL database: {}", e)))?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Create a new in-memory database (for testing).
    pub async fn new_memory() -> Result<Self, DatabaseError> {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .map_err(|e| {
                DatabaseError::Pool(format!("Failed to create in-memory database: {}", e))
            })?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Create with Turso cloud sync (embedded replica).
    pub async fn new_remote_replica(
        path: &Path,
        url: &str,
        auth_token: &str,
    ) -> Result<Self, DatabaseError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DatabaseError::Pool(format!("Failed to create database directory: {}", e))
            })?;
        }

        let db = libsql::Builder::new_remote_replica(path, url.to_string(), auth_token.to_string())
            .build()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to open remote replica: {}", e)))?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Get a shared reference to the underlying database handle.
    ///
    /// Use this to pass the database to stores (SecretsStore, WasmToolStore)
    /// that need to create their own connections per-operation.
    pub fn shared_db(&self) -> Arc<LibSqlDatabase> {
        Arc::clone(&self.db)
    }

    /// Create a new connection to the database.
    ///
    /// Sets `PRAGMA busy_timeout = 5000` on every connection so concurrent
    /// writers wait up to 5 seconds instead of failing instantly with
    /// "database is locked".
    ///
    /// Retries up to 3 times with exponential backoff to handle transient
    /// "unable to open database file" errors from concurrent connection
    /// creation (e.g. cron ticker vs main thread).
    pub async fn connect(&self) -> Result<Connection, DatabaseError> {
        let mut last_err = None;
        for attempt in 0..3u32 {
            match self.db.connect() {
                Ok(conn) => {
                    conn.query("PRAGMA busy_timeout = 5000", ())
                        .await
                        .map_err(|e| {
                            DatabaseError::Pool(format!("Failed to set busy_timeout: {}", e))
                        })?;
                    return Ok(conn);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            50 * 2u64.pow(attempt),
                        ))
                        .await;
                    }
                }
            }
        }
        Err(DatabaseError::Pool(format!(
            "Failed to create connection after 3 attempts: {}",
            last_err.map(|e| e.to_string()).unwrap_or_default()
        )))
    }
}

// ==================== Helper functions ====================

/// Parse an ISO-8601 timestamp string from SQLite into DateTime<Utc>.
///
/// Tries multiple formats in order:
/// 1. RFC 3339 with timezone (e.g. `2024-01-15T10:30:00.123Z`)
/// 2. Naive datetime with fractional seconds (e.g. `2024-01-15 10:30:00.123`)
/// 3. Naive datetime without fractional seconds (e.g. `2024-01-15 10:30:00`)
///
/// Returns an error if none of the formats match.
pub(crate) fn parse_timestamp(s: &str) -> Result<DateTime<Utc>, String> {
    let log_naive_timestamp = || {
        tracing::warn!(
            timestamp = %s,
            "parsed naive timestamp, assuming UTC — consider migrating to RFC 3339"
        );
    };

    // RFC 3339 (our canonical write format)
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Naive with fractional seconds (legacy or SQLite datetime() output)
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        log_naive_timestamp();
        return Ok(ndt.and_utc());
    }
    // Naive without fractional seconds (legacy format)
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        log_naive_timestamp();
        return Ok(ndt.and_utc());
    }
    Err(format!("unparseable timestamp: {:?}", s))
}

/// Format a DateTime<Utc> for SQLite storage (RFC 3339 with millisecond precision).
pub(crate) fn fmt_ts(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Format an optional DateTime<Utc>.
pub(crate) fn fmt_opt_ts(dt: &Option<DateTime<Utc>>) -> libsql::Value {
    match dt {
        Some(dt) => libsql::Value::Text(fmt_ts(dt)),
        None => libsql::Value::Null,
    }
}

pub(crate) fn parse_job_state(s: &str) -> JobState {
    match s {
        "pending" => JobState::Pending,
        "in_progress" => JobState::InProgress,
        "completed" => JobState::Completed,
        "submitted" => JobState::Submitted,
        "accepted" => JobState::Accepted,
        "failed" => JobState::Failed,
        "stuck" => JobState::Stuck,
        "cancelled" => JobState::Cancelled,
        _ => JobState::Pending,
    }
}

/// Extract a text column from a libsql Row, returning empty string for NULL.
pub(crate) fn get_text(row: &libsql::Row, idx: i32) -> String {
    row.get::<String>(idx).unwrap_or_default()
}

/// Extract an optional text column.
/// Returns None for SQL NULL, preserves empty strings as Some("").
pub(crate) fn get_opt_text(row: &libsql::Row, idx: i32) -> Option<String> {
    row.get::<String>(idx).ok()
}

/// Convert an `Option<&str>` to a `libsql::Value` (Text or Null).
/// Use this instead of `.unwrap_or("")` to preserve NULL semantics.
pub(crate) fn opt_text(s: Option<&str>) -> libsql::Value {
    match s {
        Some(s) => libsql::Value::Text(s.to_string()),
        None => libsql::Value::Null,
    }
}

/// Convert an `Option<String>` to a `libsql::Value` (Text or Null).
pub(crate) fn opt_text_owned(s: Option<String>) -> libsql::Value {
    match s {
        Some(s) => libsql::Value::Text(s),
        None => libsql::Value::Null,
    }
}

pub(crate) fn normalize_notify_user(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed == "default" {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Extract an i64 column, defaulting to 0.
pub(crate) fn get_i64(row: &libsql::Row, idx: i32) -> i64 {
    row.get::<i64>(idx).unwrap_or(0)
}

/// Extract an optional bool from an integer column.
pub(crate) fn get_opt_bool(row: &libsql::Row, idx: i32) -> Option<bool> {
    row.get::<i64>(idx).ok().map(|v| v != 0)
}

/// Parse a Decimal from a text column.
pub(crate) fn get_decimal(row: &libsql::Row, idx: i32) -> Decimal {
    row.get::<String>(idx)
        .ok()
        .and_then(|s| s.parse::<Decimal>().ok())
        .unwrap_or_default()
}

/// Parse an optional Decimal from a text column.
pub(crate) fn get_opt_decimal(row: &libsql::Row, idx: i32) -> Option<Decimal> {
    row.get::<String>(idx)
        .ok()
        .and_then(|s| s.parse::<Decimal>().ok())
}

/// Parse a JSON value from a text column.
pub(crate) fn get_json(row: &libsql::Row, idx: i32) -> serde_json::Value {
    row.get::<String>(idx)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Parse a timestamp from a text column.
///
/// If the column is NULL or the value cannot be parsed, logs a warning and
/// returns the Unix epoch (1970-01-01T00:00:00Z) so the error is detectable
/// rather than silently replaced by the current time.
pub(crate) fn get_ts(row: &libsql::Row, idx: i32) -> DateTime<Utc> {
    match row.get::<String>(idx) {
        Ok(s) => match parse_timestamp(&s) {
            Ok(dt) => dt,
            Err(e) => {
                tracing::warn!("Timestamp parse failure at column {}: {}", idx, e);
                DateTime::UNIX_EPOCH
            }
        },
        Err(_) => DateTime::UNIX_EPOCH,
    }
}

/// Parse an optional timestamp from a text column.
///
/// Returns None if the column is NULL. Logs a warning and returns None if the
/// value is present but cannot be parsed.
pub(crate) fn get_opt_ts(row: &libsql::Row, idx: i32) -> Option<DateTime<Utc>> {
    match row.get::<String>(idx) {
        Ok(s) if s.is_empty() => None,
        Ok(s) => match parse_timestamp(&s) {
            Ok(dt) => Some(dt),
            Err(e) => {
                tracing::warn!("Timestamp parse failure at column {}: {}", idx, e);
                None
            }
        },
        Err(_) => None,
    }
}

#[async_trait]
impl Database for LibSqlBackend {
    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        // WAL mode persists in the database file: all future connections benefit.
        // Readers no longer block writers and vice versa.
        conn.query("PRAGMA journal_mode=WAL", ())
            .await
            .map_err(|e| DatabaseError::Migration(format!("Failed to enable WAL mode: {}", e)))?;
        conn.execute_batch(libsql_migrations::SCHEMA)
            .await
            .map_err(|e| DatabaseError::Migration(format!("libSQL migration failed: {}", e)))?;
        // Apply incremental migrations (V9+) tracked in _migrations table.
        libsql_migrations::run_incremental(&conn).await?;

        // Set up vector index if embeddings are configured.
        // This dynamically creates a libsql_vector_idx on memory_chunks.embedding
        // with the correct F32_BLOB(N) dimension inferred from env vars.
        if let Some(dimension) = workspace::resolve_embedding_dimension() {
            self.ensure_vector_index(dimension).await?;
        }

        Ok(())
    }

    async fn migrate_default_owner(&self, owner_id: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            // Only tables with a real `user_id` column participate in the legacy
            // 'default' -> owner rewrite. `dynamic_tools` is intentionally excluded:
            // it is ownerless today and scoped by `scope`, not `user_id`.
            let tables = [
                "conversations",
                "memory_documents",
                "heartbeat_state",
                "secrets",
                "wasm_tools",
                "routines",
                "settings",
                "agent_jobs",
                "api_tokens",
            ];
            for table in &tables {
                conn.execute(
                    &format!(
                        "UPDATE {} SET user_id = ?1 WHERE user_id = 'default'",
                        table
                    ),
                    libsql::params![owner_id],
                )
                .await
                .map_err(|e| DatabaseError::Query(format!("migrate_default_owner {table}: {e}")))?;
            }
            Ok::<(), DatabaseError>(())
        }
        .await;

        match result {
            Ok(()) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(e)
            }
        }
    }
}

// ==================== Row conversion helpers ====================

pub(crate) fn row_to_memory_document(row: &libsql::Row) -> MemoryDocument {
    MemoryDocument {
        id: get_text(row, 0).parse().unwrap_or_default(),
        user_id: get_text(row, 1),
        agent_id: get_opt_text(row, 2).and_then(|s| s.parse().ok()),
        path: get_text(row, 3),
        content: get_text(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
        metadata: get_json(row, 7),
    }
}

pub(crate) fn row_to_routine_libsql(row: &libsql::Row) -> Result<Routine, DatabaseError> {
    let trigger_type = get_text(row, 5);
    let trigger_config = get_json(row, 6);
    let action_type = get_text(row, 7);
    let action_config = get_json(row, 8);
    let cooldown_secs = get_i64(row, 9);
    let max_concurrent = get_i64(row, 10);
    let dedup_window_secs: Option<i64> = row.get::<i64>(11).ok();

    let trigger = Trigger::from_db(&trigger_type, trigger_config)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let action = RoutineAction::from_db(&action_type, action_config)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

    Ok(Routine {
        id: get_text(row, 0).parse().unwrap_or_default(),
        name: get_text(row, 1),
        description: get_text(row, 2),
        user_id: get_text(row, 3),
        enabled: get_i64(row, 4) != 0,
        trigger,
        action,
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(cooldown_secs as u64),
            max_concurrent: max_concurrent as u32,
            dedup_window: dedup_window_secs.map(|s| std::time::Duration::from_secs(s as u64)),
        },
        notify: NotifyConfig {
            channel: get_opt_text(row, 12),
            user: normalize_notify_user(get_opt_text(row, 13)),
            on_success: get_i64(row, 14) != 0,
            on_failure: get_i64(row, 15) != 0,
            on_attention: get_i64(row, 16) != 0,
            agent_review_on_success: get_i64(row, 24) != 0,
            agent_review_on_failure: get_i64(row, 25) != 0,
            agent_review_on_attention: get_i64(row, 26) != 0,
        },
        state: get_json(row, 17),
        last_run_at: get_opt_ts(row, 18),
        next_fire_at: get_opt_ts(row, 19),
        run_count: get_i64(row, 20) as u64,
        consecutive_failures: get_i64(row, 21) as u32,
        created_at: get_ts(row, 22),
        updated_at: get_ts(row, 23),
    })
}

pub(crate) fn row_to_routine_run_libsql(row: &libsql::Row) -> Result<RoutineRun, DatabaseError> {
    let status_str = get_text(row, 5);
    let status: RunStatus = status_str
        .parse()
        .map_err(|e: crate::error::RoutineError| DatabaseError::Serialization(e.to_string()))?;

    Ok(RoutineRun {
        id: get_text(row, 0).parse().unwrap_or_default(),
        routine_id: get_text(row, 1).parse().unwrap_or_default(),
        trigger_type: get_text(row, 2),
        trigger_detail: get_opt_text(row, 3),
        started_at: get_ts(row, 4),
        completed_at: get_opt_ts(row, 6),
        status,
        result_summary: get_opt_text(row, 7),
        tokens_used: row.get::<i64>(8).ok().map(|v| v as i32),
        job_id: get_opt_text(row, 9).and_then(|s| s.parse().ok()),
        created_at: get_ts(row, 10),
    })
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use crate::db::Database;
    use crate::db::libsql::{LibSqlBackend, fmt_ts, normalize_notify_user, parse_timestamp};

    #[test]
    fn test_normalize_notify_user_treats_legacy_default_as_missing() {
        assert_eq!(normalize_notify_user(None), None); // safety: test-only assertion
        assert_eq!(normalize_notify_user(Some(String::new())), None); // safety: test-only assertion
        assert_eq!(normalize_notify_user(Some("   ".to_string())), None); // safety: test-only assertion
        assert_eq!(normalize_notify_user(Some("default".to_string())), None); // safety: test-only assertion
        let normalized = normalize_notify_user(Some("123456789".to_string()));
        assert_eq!(normalized, Some("123456789".to_string())); // safety: test-only assertion
    }

    #[test]
    fn test_parse_timestamp_accepts_rfc3339_and_legacy_naive_formats() {
        let expected = Utc.with_ymd_and_hms(2026, 3, 7, 12, 34, 56).unwrap();

        let with_millis = parse_timestamp("2026-03-07T12:34:56.789Z").unwrap();
        assert_eq!(with_millis, expected + chrono::Duration::milliseconds(789));

        let naive_with_millis = parse_timestamp("2026-03-07 12:34:56.789").unwrap();
        assert_eq!(
            naive_with_millis,
            expected + chrono::Duration::milliseconds(789)
        );

        let naive_without_millis = parse_timestamp("2026-03-07 12:34:56").unwrap();
        assert_eq!(naive_without_millis, expected);
    }

    #[test]
    fn test_fmt_ts_roundtrips_through_parse_timestamp() {
        let original = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 45).unwrap()
            + chrono::Duration::milliseconds(123);
        let formatted = fmt_ts(&original);
        let parsed = parse_timestamp(&formatted).unwrap();
        assert_eq!(parsed, original);
    }

    #[tokio::test]
    async fn test_libsql_now_format_is_rfc3339_and_parseable() {
        let backend = LibSqlBackend::new_memory().await.unwrap();
        backend.run_migrations().await.unwrap();

        let conn = backend.connect().await.unwrap();
        let mut rows = conn
            .query("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let ts: String = row.get(0).unwrap();

        let parsed = parse_timestamp(&ts).unwrap();
        assert_eq!(
            ts,
            parsed.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );
    }

    #[tokio::test]
    async fn test_wal_mode_after_migrations() {
        let backend = LibSqlBackend::new_memory().await.unwrap();
        backend.run_migrations().await.unwrap();

        let conn = backend.connect().await.unwrap();
        let mut rows = conn.query("PRAGMA journal_mode", ()).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let mode: String = row.get(0).unwrap();
        // In-memory databases use "memory" journal mode (WAL doesn't apply to :memory:),
        // but the PRAGMA still executes without error. For file-based databases it returns "wal".
        assert!(
            mode == "wal" || mode == "memory",
            "expected wal or memory, got: {}",
            mode,
        );
    }

    #[tokio::test]
    async fn test_busy_timeout_set_on_connect() {
        let backend = LibSqlBackend::new_memory().await.unwrap();
        backend.run_migrations().await.unwrap();

        let conn = backend.connect().await.unwrap();
        let mut rows = conn.query("PRAGMA busy_timeout", ()).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let timeout: i64 = row.get(0).unwrap();
        assert_eq!(timeout, 5000);
    }

    /// Regression test: save_job must persist user_id and get_job must return it.
    #[tokio::test]
    async fn test_save_job_persists_user_id() {
        use crate::context::JobContext;
        use crate::db::JobStore;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_user_id.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let ctx = JobContext::with_user("test-user-42", "Test Job", "A test job");
        backend.save_job(&ctx).await.unwrap();

        let loaded = backend.get_job(ctx.job_id).await.unwrap().unwrap();
        assert_eq!(loaded.user_id, "test-user-42");
    }

    #[tokio::test]
    async fn test_concurrent_writes_succeed() {
        // Use a temp file so connections share state (in-memory DBs are connection-local)
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_concurrent.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Spawn 20 concurrent inserts into the conversations table
        let mut handles = Vec::new();
        for i in 0..20 {
            let conn = backend.connect().await.unwrap();
            let handle = tokio::spawn(async move {
                let id = uuid::Uuid::new_v4().to_string();
                let val = format!("ch_{}", i);
                conn.execute(
                    "INSERT INTO conversations (id, channel, user_id) VALUES (?1, ?2, ?3)",
                    libsql::params![id, val, "test_user"],
                )
                .await
            });
            handles.push(handle);
        }

        for handle in handles {
            let result = handle.await.unwrap();
            assert!(
                result.is_ok(),
                "concurrent write failed: {:?}",
                result.err()
            );
        }

        // Verify all 20 rows landed
        let conn = backend.connect().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM conversations WHERE user_id = ?1",
                libsql::params!["test_user"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 20);
    }

    #[tokio::test]
    async fn test_connect_retry_succeeds_on_valid_db() {
        // Verify connect() works with retry logic on a file-backed DB
        // (exercises the retry path even though transient failures are hard
        // to reproduce deterministically).
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_retry.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Multiple concurrent connect() calls should all succeed
        let mut handles = Vec::new();
        for _ in 0..10 {
            let b = LibSqlBackend {
                db: backend.shared_db(),
            };
            handles.push(tokio::spawn(async move { b.connect().await }));
        }

        for handle in handles {
            let result = handle.await.unwrap();
            assert!(
                result.is_ok(),
                "concurrent connect failed: {:?}",
                result.err()
            );
        }
    }

    #[tokio::test]
    async fn test_ownership_model_tables_created() {
        // Use a file-backed DB so that multiple connections share state.
        // In-memory databases do not share state between connections in libSQL.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_ownership_model.db");
        let db = LibSqlBackend::new_local(&db_path).await.unwrap();
        db.run_migrations().await.unwrap();

        let conn = db.connect().await.unwrap();
        // channel_identities exists with expected columns
        conn.execute(
            "SELECT id, owner_id, channel, external_id FROM channel_identities LIMIT 0",
            (),
        )
        .await
        .unwrap();
        // pairing_requests exists with expected columns
        conn.execute(
            "SELECT id, channel, code, owner_id FROM pairing_requests LIMIT 0",
            (),
        )
        .await
        .unwrap();
        // scope column exists on wasm_tools
        conn.execute("SELECT scope FROM wasm_tools LIMIT 0", ())
            .await
            .unwrap();
        // scope column exists on dynamic_tools
        conn.execute("SELECT scope FROM dynamic_tools LIMIT 0", ())
            .await
            .unwrap();
    }
}
