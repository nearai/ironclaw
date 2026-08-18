use std::{
    collections::{BTreeMap, BTreeSet},
    io::ErrorKind,
    path::PathBuf,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio_postgres::Client;

use crate::redaction::redact_postgres_url;

const MEASUREMENT_TABLES: &[&str] = &[
    "root_filesystem_entries",
    "root_filesystem_events",
    "root_filesystem_index_specs",
    "root_filesystem_ordered_index_rows",
    "root_filesystem_sequences",
    "trigger_records",
    "trigger_run_history",
];

// PostgreSQL normally flushes cumulative table statistics at most once per second.
const POSTGRES_STATS_SETTLE_DURATION: Duration = Duration::from_millis(1_100);

/// Database backend measured by the probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbProbeTarget {
    LibSql { path: PathBuf },
    Postgres { url: String },
}

/// Backend-neutral configuration for one database-write measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbProbeConfig {
    target: DbProbeTarget,
    reset_stats: bool,
}

impl DbProbeConfig {
    pub fn libsql(path: impl Into<PathBuf>, reset_stats: bool) -> Self {
        Self {
            target: DbProbeTarget::LibSql { path: path.into() },
            reset_stats,
        }
    }

    pub fn postgres(url: impl Into<String>, reset_stats: bool) -> Self {
        Self {
            target: DbProbeTarget::Postgres { url: url.into() },
            reset_stats,
        }
    }

    pub fn target(&self) -> &DbProbeTarget {
        &self.target
    }

    pub fn reset_stats(&self) -> bool {
        self.reset_stats
    }
}

/// Failure from a measured database probe operation.
#[derive(Debug, thiserror::Error)]
pub enum DbProbeError {
    #[error("{message}")]
    Operation {
        backend: &'static str,
        operation: &'static str,
        message: String,
    },
    #[error("{message}")]
    OperationWithSource {
        backend: &'static str,
        operation: &'static str,
        message: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("{source}")]
    CleanupAfterBaseline {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("{primary}; libsql measurement cleanup after baseline failure also failed: {cleanup}")]
    BaselineCleanup {
        #[source]
        primary: Box<DbProbeError>,
        cleanup: Box<DbProbeError>,
    },
}

impl DbProbeError {
    pub fn operation(
        backend: &'static str,
        operation: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::Operation {
            backend,
            operation,
            message: message.into(),
        }
    }

    fn with_source(
        backend: &'static str,
        operation: &'static str,
        message: impl Into<String>,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::OperationWithSource {
            backend,
            operation,
            message: message.into(),
            source,
        }
    }

    pub fn backend(&self) -> &'static str {
        match self {
            Self::Operation { backend, .. } | Self::OperationWithSource { backend, .. } => backend,
            Self::CleanupAfterBaseline { .. } | Self::BaselineCleanup { .. } => "libsql",
        }
    }

    pub fn operation_name(&self) -> &'static str {
        match self {
            Self::Operation { operation, .. } | Self::OperationWithSource { operation, .. } => {
                operation
            }
            Self::CleanupAfterBaseline { .. } | Self::BaselineCleanup { .. } => "baseline cleanup",
        }
    }

    pub fn cleanup_error(&self) -> Option<&DbProbeError> {
        match self {
            Self::BaselineCleanup { cleanup, .. } => Some(cleanup),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatsScope {
    ExplicitResetCurrentDatabase,
    #[default]
    SnapshotDeltaCurrentDatabase,
}

impl StatsScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitResetCurrentDatabase => "explicit-reset-current-database",
            Self::SnapshotDeltaCurrentDatabase => "snapshot-delta-current-database",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbProbeSummary {
    pub before: DbProbeSnapshot,
    pub after: DbProbeSnapshot,
    pub delta: DbProbeDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_after: Option<DbProbeSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_delta: Option<DbProbeDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurement: Option<DbWriteMeasurement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbWriteMeasurement {
    pub workload: String,
    pub tool_calls_per_turn: usize,
    pub idle_observation_seconds: u64,
    pub reset_stats: bool,
    pub stats_scope: StatsScope,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbProbeSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libsql_file_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libsql_wal_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libsql_shm_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libsql_table_writes: Vec<LibSqlTableWrites>,
    #[serde(default)]
    pub libsql_table_writes_total: LibSqlWriteCounts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postgres_database_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postgres_active_connections: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postgres_idle_connections: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postgres_waiting_connections: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub postgres_table_writes: Vec<PostgresTableWrites>,
    #[serde(default)]
    pub postgres_table_writes_total: PostgresWriteCounts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub postgres_statement_calls: Vec<PostgresStatementCalls>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub postgres_statement_calls_by_table: Vec<PostgresTableStatementCalls>,
    #[serde(default)]
    pub postgres_statement_calls_total: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uninstrumented_tables: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibSqlWriteCounts {
    pub inserts: u64,
    pub updates: u64,
    pub deletes: u64,
}

impl LibSqlWriteCounts {
    pub fn total(&self) -> u64 {
        self.inserts
            .saturating_add(self.updates)
            .saturating_add(self.deletes)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibSqlTableWrites {
    pub table: String,
    pub inserts: u64,
    pub updates: u64,
    pub deletes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresWriteCounts {
    pub inserts: u64,
    pub updates: u64,
    pub deletes: u64,
}

impl PostgresWriteCounts {
    pub fn total(&self) -> u64 {
        self.inserts
            .saturating_add(self.updates)
            .saturating_add(self.deletes)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresTableWrites {
    pub table: String,
    pub inserts: u64,
    pub updates: u64,
    pub deletes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresStatementCalls {
    pub query_id: String,
    pub operation: String,
    pub tables: Vec<String>,
    pub calls: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresTableStatementCalls {
    pub table: String,
    pub calls: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbProbeDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libsql_file_bytes: Option<i128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libsql_wal_bytes: Option<i128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libsql_shm_bytes: Option<i128>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libsql_table_writes: Vec<LibSqlTableWriteDelta>,
    #[serde(default)]
    pub libsql_table_writes_total: LibSqlWriteDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postgres_database_size_bytes: Option<i128>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub postgres_table_writes: Vec<PostgresTableWriteDelta>,
    #[serde(default)]
    pub postgres_table_writes_total: PostgresWriteDelta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub postgres_statement_calls: Vec<PostgresStatementCallDelta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub postgres_statement_calls_by_table: Vec<PostgresTableStatementCallDelta>,
    #[serde(default)]
    pub postgres_statement_calls_total: i128,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibSqlWriteDelta {
    pub inserts: i128,
    pub updates: i128,
    pub deletes: i128,
}

impl LibSqlWriteDelta {
    pub fn total(&self) -> i128 {
        self.inserts + self.updates + self.deletes
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibSqlTableWriteDelta {
    pub table: String,
    pub inserts: i128,
    pub updates: i128,
    pub deletes: i128,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresWriteDelta {
    pub inserts: i128,
    pub updates: i128,
    pub deletes: i128,
}

impl PostgresWriteDelta {
    pub fn total(&self) -> i128 {
        self.inserts + self.updates + self.deletes
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresTableWriteDelta {
    pub table: String,
    pub inserts: i128,
    pub updates: i128,
    pub deletes: i128,
}
/// Aggregate PostgreSQL write deltas by unqualified relation name.
///
/// PostgreSQL statistics include the schema in `table`. Multiple schemas may
/// contain the same measured relation, so matching rows are summed rather than
/// overwritten.
pub fn postgres_relation_write_totals(rows: &[PostgresTableWriteDelta]) -> BTreeMap<String, i128> {
    let mut totals = BTreeMap::new();
    for row in rows {
        let relation = row
            .table
            .rsplit_once('.')
            .map_or(row.table.as_str(), |(_, relation)| relation);
        let writes = row.inserts + row.updates + row.deletes;
        totals
            .entry(relation.to_string())
            .and_modify(|total| *total += writes)
            .or_insert(writes);
    }
    totals
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresStatementCallDelta {
    pub query_id: String,
    pub operation: String,
    pub tables: Vec<String>,
    pub calls: i128,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresTableStatementCallDelta {
    pub table: String,
    pub calls: i128,
}

#[doc(hidden)]
pub async fn capture_unmeasured(config: &DbProbeConfig) -> DbProbeSnapshot {
    match config.target() {
        DbProbeTarget::LibSql { path } => capture_libsql(path).await,
        DbProbeTarget::Postgres { url } => capture_postgres(url, false).await,
    }
}

/// Installs or resets backend instrumentation and captures the starting counters.
pub async fn begin(config: &DbProbeConfig) -> Result<DbProbeSnapshot, DbProbeError> {
    match config.target() {
        DbProbeTarget::LibSql { path } => {
            install_libsql_write_counters(path, config.reset_stats())
                .await
                .map_err(|source| {
                    DbProbeError::with_source(
                        "libsql",
                        "begin",
                        format!("libsql measurement setup failed: {source}"),
                        source,
                    )
                })?;
            retain_libsql_snapshot_or_cleanup(path, capture(config).await).await
        }
        DbProbeTarget::Postgres { url } => {
            let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
                .await
                .map_err(|source| {
                    DbProbeError::with_source(
                        "postgres",
                        "begin",
                        sanitize_postgres_error(url, &source),
                        Box::new(source),
                    )
                })?;
            let connection_handle = tokio::spawn(async move {
                if let Err(error) = connection.await {
                    eprintln!("[ironclaw-stress] postgres probe connection error: {error}");
                }
            });
            ensure_pg_stat_statements(&client, url).await?;
            if config.reset_stats() {
                reset_measurement_stats(&client, url).await?;
            }
            drop(client);
            let _ = connection_handle.await;
            capture(config).await
        }
    }
}

async fn retain_libsql_snapshot_or_cleanup(
    path: &std::path::Path,
    capture_result: Result<DbProbeSnapshot, DbProbeError>,
) -> Result<DbProbeSnapshot, DbProbeError> {
    match capture_result {
        Ok(snapshot) => Ok(snapshot),
        Err(primary) => match remove_libsql_write_counters(path).await {
            Ok(()) => Err(primary),
            Err(source) => {
                let cleanup = DbProbeError::CleanupAfterBaseline { source };
                Err(DbProbeError::BaselineCleanup {
                    primary: Box::new(primary),
                    cleanup: Box::new(cleanup),
                })
            }
        },
    }
}

/// Captures backend write counters for the configured target.
pub async fn capture(config: &DbProbeConfig) -> Result<DbProbeSnapshot, DbProbeError> {
    match config.target() {
        DbProbeTarget::LibSql { path } => {
            try_capture_libsql(path.clone()).await.map_err(|source| {
                DbProbeError::with_source(
                    "libsql",
                    "capture",
                    format!("libsql probe failed: {source}"),
                    source,
                )
            })
        }
        DbProbeTarget::Postgres { url } => {
            try_capture_postgres(url, true).await.map_err(|source| {
                let message = sanitize_postgres_error(url, &source);
                DbProbeError::with_source("postgres", "capture", message, source)
            })
        }
    }
}

/// Waits for PostgreSQL cumulative statistics to flush, then captures write counters.
///
/// libSQL counters are transactionally visible, so libSQL capture remains immediate.
pub async fn capture_settled(config: &DbProbeConfig) -> Result<DbProbeSnapshot, DbProbeError> {
    if let Some(delay) = settlement_delay(config.target()) {
        tokio::time::sleep(delay).await;
    }
    capture(config).await
}

fn settlement_delay(target: &DbProbeTarget) -> Option<Duration> {
    matches!(target, DbProbeTarget::Postgres { .. }).then_some(POSTGRES_STATS_SETTLE_DURATION)
}

/// Removes temporary backend instrumentation installed by [`begin`].
pub async fn finish(config: &DbProbeConfig) -> Result<(), DbProbeError> {
    let DbProbeTarget::LibSql { path } = config.target() else {
        return Ok(());
    };
    remove_libsql_write_counters(path).await.map_err(|source| {
        DbProbeError::with_source(
            "libsql",
            "finish",
            format!("libsql measurement cleanup failed: {source}"),
            source,
        )
    })
}

#[doc(hidden)]
pub fn summarize(before: DbProbeSnapshot, after: DbProbeSnapshot) -> DbProbeSummary {
    let before = normalize_snapshot(before);
    let after = normalize_snapshot(after);
    let delta = snapshot_delta(&before, &after);
    DbProbeSummary {
        before,
        after,
        delta,
        idle_after: None,
        idle_delta: None,
        measurement: None,
    }
}

#[doc(hidden)]
pub fn summarize_measurement(
    before: DbProbeSnapshot,
    after: DbProbeSnapshot,
    idle_after: Option<DbProbeSnapshot>,
    measurement: DbWriteMeasurement,
) -> DbProbeSummary {
    let before = normalize_snapshot(before);
    let after = normalize_snapshot(after);
    let idle_after = idle_after.map(normalize_snapshot);
    let delta = snapshot_delta(&before, &after);
    let idle_delta = idle_after
        .as_ref()
        .map(|idle_after| snapshot_delta(&after, idle_after));
    DbProbeSummary {
        before,
        after,
        delta,
        idle_after,
        idle_delta,
        measurement: Some(measurement),
    }
}

async fn capture_libsql(path: &std::path::Path) -> DbProbeSnapshot {
    match try_capture_libsql(path.to_path_buf()).await {
        Ok(snapshot) => snapshot,
        Err(error) => DbProbeSnapshot {
            error: Some(format!("libsql probe failed: {error}")),
            ..DbProbeSnapshot::default()
        },
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid libSQL write counter row_count={value} for table={table} operation={operation}")]
struct InvalidLibSqlCounter {
    table: String,
    operation: String,
    value: String,
    #[source]
    source: std::num::ParseIntError,
}

#[doc(hidden)]
pub async fn try_capture_libsql(
    path: PathBuf,
) -> Result<DbProbeSnapshot, Box<dyn std::error::Error + Send + Sync>> {
    let file_bytes = file_size(&path).await?;
    let wal_bytes = file_size(&sidecar_path(&path, "-wal")).await?;
    let shm_bytes = file_size(&sidecar_path(&path, "-shm")).await?;
    let db = libsql::Builder::new_local(&path).build().await?;
    let connection = db.connect()?;
    let mut counter_table = connection
        .query(
            "SELECT 1 FROM sqlite_schema \
             WHERE type = 'table' AND name = 'ironclaw_stress_write_counters'",
            (),
        )
        .await?;
    if counter_table.next().await?.is_none() {
        return Ok(DbProbeSnapshot {
            libsql_file_bytes: Some(file_bytes),
            libsql_wal_bytes: Some(wal_bytes),
            libsql_shm_bytes: Some(shm_bytes),
            ..DbProbeSnapshot::default()
        });
    }
    let mut table_writes = BTreeMap::new();
    let mut uninstrumented_tables = Vec::new();
    for table in MEASUREMENT_TABLES {
        let mut present = connection
            .query(
                "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [*table],
            )
            .await?;
        if present.next().await?.is_some() {
            table_writes.insert(
                (*table).to_string(),
                LibSqlTableWrites {
                    table: (*table).to_string(),
                    ..LibSqlTableWrites::default()
                },
            );
        } else {
            uninstrumented_tables.push((*table).to_string());
        }
    }
    let mut rows = connection
        .query(
            "SELECT table_name, operation, CAST(row_count AS TEXT) \
             FROM ironclaw_stress_write_counters \
             ORDER BY table_name, operation",
            (),
        )
        .await?;
    while let Some(row) = rows.next().await? {
        let table: String = row.get(0)?;
        let operation: String = row.get(1)?;
        let count: String = row.get(2)?;
        let count = count
            .parse::<u64>()
            .map_err(|source| InvalidLibSqlCounter {
                table: table.clone(),
                operation: operation.clone(),
                value: count,
                source,
            })?;
        let Some(table_writes) = table_writes.get_mut(&table) else {
            continue;
        };
        match operation.as_str() {
            "insert" => table_writes.inserts = count,
            "update" => table_writes.updates = count,
            "delete" => table_writes.deletes = count,
            _ => {}
        }
    }
    Ok(DbProbeSnapshot {
        libsql_file_bytes: Some(file_bytes),
        libsql_wal_bytes: Some(wal_bytes),
        libsql_shm_bytes: Some(shm_bytes),
        libsql_table_writes: table_writes.into_values().collect(),
        uninstrumented_tables,
        ..DbProbeSnapshot::default()
    })
}

#[doc(hidden)]
pub async fn install_libsql_write_counters(
    path: &std::path::Path,
    reset: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let db = libsql::Builder::new_local(path).build().await?;
    let connection = db.connect()?;

    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS ironclaw_stress_write_counters (\
                table_name TEXT NOT NULL,\
                operation TEXT NOT NULL CHECK (operation IN ('insert', 'update', 'delete')),\
                row_count INTEGER NOT NULL DEFAULT 0,\
                PRIMARY KEY (table_name, operation)\
             );",
        )
        .await?;
    if reset {
        connection
            .execute("DELETE FROM ironclaw_stress_write_counters", ())
            .await?;
    }
    for table in MEASUREMENT_TABLES {
        let mut rows = connection
            .query(
                "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [*table],
            )
            .await?;
        if rows.next().await?.is_none() {
            continue;
        }
        for (suffix, timing, operation) in [
            ("ai", "AFTER INSERT", "insert"),
            ("au", "AFTER UPDATE", "update"),
            ("ad", "AFTER DELETE", "delete"),
        ] {
            let sql = format!(
                "CREATE TRIGGER IF NOT EXISTS ironclaw_stress_{table}_{suffix} \
                 {timing} ON {table} BEGIN \
                   INSERT INTO ironclaw_stress_write_counters(table_name, operation, row_count) \
                   VALUES ('{table}', '{operation}', 1) \
                   ON CONFLICT(table_name, operation) DO UPDATE \
                   SET row_count = row_count + 1; \
                 END;"
            );
            connection.execute_batch(&sql).await?;
        }
    }
    Ok(())
}

async fn remove_libsql_write_counters(
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let db = libsql::Builder::new_local(path).build().await?;
    let connection = db.connect()?;
    for table in MEASUREMENT_TABLES {
        for suffix in ["ai", "au", "ad"] {
            connection
                .execute_batch(&format!(
                    "DROP TRIGGER IF EXISTS ironclaw_stress_{table}_{suffix};"
                ))
                .await?;
        }
    }
    connection
        .execute_batch("DROP TABLE IF EXISTS ironclaw_stress_write_counters;")
        .await?;
    Ok(())
}

fn sidecar_path(path: &std::path::Path, suffix: &str) -> PathBuf {
    let mut sidecar = path.to_path_buf();
    let Some(file_name) = path.file_name() else {
        return path.with_extension(suffix.trim_start_matches('-'));
    };
    sidecar.set_file_name(format!("{}{}", file_name.to_string_lossy(), suffix));
    sidecar
}

async fn file_size(path: &std::path::Path) -> Result<u64, std::io::Error> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

async fn capture_postgres(url: &str, include_write_stats: bool) -> DbProbeSnapshot {
    match try_capture_postgres(url, include_write_stats).await {
        Ok(snapshot) => snapshot,
        Err(error) => DbProbeSnapshot {
            error: Some(sanitize_postgres_error(url, error)),
            ..DbProbeSnapshot::default()
        },
    }
}

async fn try_capture_postgres(
    url: &str,
    include_write_stats: bool,
) -> Result<DbProbeSnapshot, Box<dyn std::error::Error + Send + Sync>> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls).await?;
    let connection_handle = tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("[ironclaw-stress] postgres probe connection error: {error}");
        }
    });
    let row = client
        .query_one(
            "SELECT \
                pg_database_size(current_database())::bigint, \
                COUNT(*) FILTER (WHERE state = 'active' AND pid <> pg_backend_pid())::bigint, \
                COUNT(*) FILTER (WHERE state = 'idle')::bigint, \
                COUNT(*) FILTER (WHERE wait_event_type IS NOT NULL AND pid <> pg_backend_pid())::bigint \
             FROM pg_stat_activity \
             WHERE datname = current_database()",
            &[],
        )
        .await?;
    let mut snapshot = DbProbeSnapshot {
        postgres_database_size_bytes: i64_to_u64(row.get(0)),
        postgres_active_connections: i64_to_u64(row.get(1)),
        postgres_idle_connections: i64_to_u64(row.get(2)),
        postgres_waiting_connections: i64_to_u64(row.get(3)),
        ..DbProbeSnapshot::default()
    };
    if include_write_stats {
        capture_postgres_write_stats(&client, &mut snapshot).await?;
    }
    drop(client);
    let _ = connection_handle.await;
    Ok(normalize_snapshot(snapshot))
}

async fn ensure_pg_stat_statements(client: &Client, url: &str) -> Result<(), DbProbeError> {
    let installed: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_stat_statements')",
            &[],
        )
        .await
        .map_err(|source| {
            DbProbeError::with_source(
                "postgres",
                "verify pg_stat_statements",
                sanitize_postgres_error(url, &source),
                Box::new(source),
            )
        })?
        .get(0);
    if !installed {
        return Err(DbProbeError::operation(
            "postgres",
            "verify pg_stat_statements",
            pg_stat_statements_unavailable(
                url,
                "the pg_stat_statements extension is not installed in this database",
            ),
        ));
    }
    client
        .query("SELECT calls FROM pg_stat_statements LIMIT 1", &[])
        .await
        .map_err(|source| {
            DbProbeError::with_source(
                "postgres",
                "verify pg_stat_statements",
                pg_stat_statements_unavailable(url, &source),
                Box::new(source),
            )
        })?;
    Ok(())
}

async fn reset_measurement_stats(client: &Client, url: &str) -> Result<(), DbProbeError> {
    let extension_version: String = client
        .query_one(
            "SELECT extversion FROM pg_catalog.pg_extension WHERE extname = 'pg_stat_statements'",
            &[],
        )
        .await
        .map_err(|source| {
            DbProbeError::with_source(
                "postgres",
                "read pg_stat_statements version",
                sanitize_postgres_error(url, &source),
                Box::new(source),
            )
        })?
        .try_get(0)
        .map_err(|source| {
            DbProbeError::with_source(
                "postgres",
                "decode pg_stat_statements version",
                sanitize_postgres_error(url, &source),
                Box::new(source),
            )
        })?;
    if !pg_stat_statements_reset_supported(&extension_version) {
        return Err(DbProbeError::operation(
            "postgres",
            "reset statistics",
            format!(
                "--db-write-reset-stats requires pg_stat_statements 1.7 or newer for the scoped \
                 three-argument reset; found {extension_version}. Upgrade the extension or omit the \
                 flag to use non-destructive snapshot deltas"
            ),
        ));
    }
    client
        .query(
            "SELECT pg_stat_statements_reset(0::oid, (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database()), 0::bigint)",
            &[],
        )
        .await
        .map_err(|source| {
            let message = format!(
                "{}; --db-write-reset-stats explicitly requested a current-database reset. \
                 Omit the flag to use non-destructive snapshot deltas",
                sanitize_postgres_error(url, &source)
            );
            DbProbeError::with_source(
                "postgres",
                "reset pg_stat_statements",
                message,
                Box::new(source),
            )
        })?;
    let table_names = MEASUREMENT_TABLES.to_vec();
    client
        .query(
            "SELECT pg_stat_reset_single_table_counters(relid) \
             FROM pg_stat_user_tables WHERE relname = ANY($1::text[])",
            &[&table_names],
        )
        .await
        .map_err(|source| {
            let message = format!(
                "{}; --db-write-reset-stats explicitly requested per-table counter resets. \
                 Omit the flag to use non-destructive snapshot deltas",
                sanitize_postgres_error(url, &source)
            );
            DbProbeError::with_source(
                "postgres",
                "reset table counters",
                message,
                Box::new(source),
            )
        })?;
    Ok(())
}

async fn capture_postgres_write_stats(
    client: &Client,
    snapshot: &mut DbProbeSnapshot,
) -> Result<(), tokio_postgres::Error> {
    client.query("SELECT pg_stat_clear_snapshot()", &[]).await?;
    let table_names = MEASUREMENT_TABLES.to_vec();
    let table_rows = client
        .query(
            "SELECT schemaname, relname, n_tup_ins::bigint, n_tup_upd::bigint, n_tup_del::bigint \
             FROM pg_stat_user_tables \
             WHERE relname = ANY($1::text[]) \
             ORDER BY schemaname, relname",
            &[&table_names],
        )
        .await?;
    let mut table_writes = BTreeMap::new();
    let mut present_relations = BTreeSet::new();
    for row in table_rows {
        let schema: String = row.try_get(0)?;
        let relation: String = row.try_get(1)?;
        let table = format!("{schema}.{relation}");
        present_relations.insert(relation.clone());
        table_writes.insert(
            table.clone(),
            PostgresTableWrites {
                table,
                inserts: i64_to_u64(row.try_get(2)?).unwrap_or(0),
                updates: i64_to_u64(row.try_get(3)?).unwrap_or(0),
                deletes: i64_to_u64(row.try_get(4)?).unwrap_or(0),
            },
        );
    }
    snapshot.uninstrumented_tables = MEASUREMENT_TABLES
        .iter()
        .filter(|table| !present_relations.contains(**table))
        .map(|table| (*table).to_string())
        .collect();
    snapshot.postgres_table_writes = table_writes.into_values().collect();

    let statement_rows = client
        .query(
            "SELECT queryid::bigint, query, SUM(calls)::bigint \
             FROM pg_stat_statements \
             WHERE dbid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database()) \
               AND queryid IS NOT NULL \
             GROUP BY queryid, query",
            &[],
        )
        .await?;
    let raw_statements = statement_rows
        .into_iter()
        .map(
            |row| -> Result<Option<(i64, String, u64)>, tokio_postgres::Error> {
                let query_id = row.try_get::<_, i64>(0)?;
                let query = row.try_get::<_, String>(1)?;
                let calls = i64_to_u64(row.try_get::<_, i64>(2)?);
                Ok(calls.map(|calls| (query_id, query, calls)))
            },
        )
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    snapshot.postgres_statement_calls = aggregate_statement_calls(
        raw_statements
            .iter()
            .map(|(query_id, query, calls)| (*query_id, query.as_str(), *calls)),
    );
    Ok(())
}

#[doc(hidden)]
pub fn aggregate_statement_calls<'a>(
    rows: impl IntoIterator<Item = (i64, &'a str, u64)>,
) -> Vec<PostgresStatementCalls> {
    let mut grouped = BTreeMap::<String, PostgresStatementCalls>::new();
    for (query_id, query, calls) in rows {
        let tables = referenced_measurement_tables(query);
        if tables.is_empty() {
            continue;
        }
        let query_id = format!("{:016x}", query_id as u64);
        let entry = grouped
            .entry(query_id.clone())
            .or_insert_with(|| PostgresStatementCalls {
                query_id,
                operation: statement_operation(query),
                tables,
                calls: 0,
            });
        entry.calls = entry.calls.saturating_add(calls);
    }
    grouped.into_values().collect()
}

fn referenced_measurement_tables(query: &str) -> Vec<String> {
    let query = query.to_ascii_lowercase();
    MEASUREMENT_TABLES
        .iter()
        .filter(|table| contains_identifier(&query, table))
        .map(|table| (*table).to_string())
        .collect()
}

fn contains_identifier(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(start, _)| {
        let before = haystack[..start].chars().next_back();
        let after = haystack[start + needle.len()..].chars().next();
        !before.is_some_and(is_identifier_char) && !after.is_some_and(is_identifier_char)
    })
}

fn is_identifier_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn statement_operation(query: &str) -> String {
    let words = query
        .split(|character: char| !character.is_ascii_alphabetic())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    for operation in ["insert", "update", "delete", "select"] {
        if words.iter().any(|word| word == operation) {
            return operation.to_string();
        }
    }
    words
        .first()
        .cloned()
        .unwrap_or_else(|| "unknown".to_string())
}

fn normalize_snapshot(mut snapshot: DbProbeSnapshot) -> DbProbeSnapshot {
    snapshot
        .libsql_table_writes
        .sort_by(|left, right| left.table.cmp(&right.table));
    snapshot.libsql_table_writes_total = snapshot.libsql_table_writes.iter().fold(
        LibSqlWriteCounts::default(),
        |mut total, table| {
            total.inserts = total.inserts.saturating_add(table.inserts);
            total.updates = total.updates.saturating_add(table.updates);
            total.deletes = total.deletes.saturating_add(table.deletes);
            total
        },
    );
    snapshot
        .postgres_table_writes
        .sort_by(|left, right| left.table.cmp(&right.table));
    snapshot.postgres_table_writes_total = snapshot.postgres_table_writes.iter().fold(
        PostgresWriteCounts::default(),
        |mut total, table| {
            total.inserts = total.inserts.saturating_add(table.inserts);
            total.updates = total.updates.saturating_add(table.updates);
            total.deletes = total.deletes.saturating_add(table.deletes);
            total
        },
    );
    snapshot
        .postgres_statement_calls
        .sort_by(|left, right| left.query_id.cmp(&right.query_id));
    snapshot.postgres_statement_calls_total = snapshot
        .postgres_statement_calls
        .iter()
        .map(|statement| statement.calls)
        .sum();
    let mut by_table = BTreeMap::<String, u64>::new();
    for statement in &snapshot.postgres_statement_calls {
        for table in &statement.tables {
            let calls = by_table.entry(table.clone()).or_default();
            *calls = calls.saturating_add(statement.calls);
        }
    }
    snapshot.postgres_statement_calls_by_table = by_table
        .into_iter()
        .map(|(table, calls)| PostgresTableStatementCalls { table, calls })
        .collect();
    snapshot
}

fn snapshot_delta(before: &DbProbeSnapshot, after: &DbProbeSnapshot) -> DbProbeDelta {
    let libsql_table_writes = libsql_table_write_delta(before, after);
    let libsql_table_writes_total =
        libsql_table_writes
            .iter()
            .fold(LibSqlWriteDelta::default(), |mut total, table| {
                total.inserts += table.inserts;
                total.updates += table.updates;
                total.deletes += table.deletes;
                total
            });
    let postgres_table_writes = table_write_delta(before, after);
    let postgres_table_writes_total =
        postgres_table_writes
            .iter()
            .fold(PostgresWriteDelta::default(), |mut total, table| {
                total.inserts += table.inserts;
                total.updates += table.updates;
                total.deletes += table.deletes;
                total
            });
    let postgres_statement_calls = statement_call_delta(before, after);
    let mut by_table = BTreeMap::<String, i128>::new();
    for statement in &postgres_statement_calls {
        for table in &statement.tables {
            *by_table.entry(table.clone()).or_default() += statement.calls;
        }
    }
    let postgres_statement_calls_by_table = by_table
        .into_iter()
        .map(|(table, calls)| PostgresTableStatementCallDelta { table, calls })
        .collect();
    let postgres_statement_calls_total = postgres_statement_calls
        .iter()
        .map(|statement| statement.calls)
        .sum();

    DbProbeDelta {
        libsql_file_bytes: delta(before.libsql_file_bytes, after.libsql_file_bytes),
        libsql_wal_bytes: delta(before.libsql_wal_bytes, after.libsql_wal_bytes),
        libsql_shm_bytes: delta(before.libsql_shm_bytes, after.libsql_shm_bytes),
        libsql_table_writes,
        libsql_table_writes_total,
        postgres_database_size_bytes: delta(
            before.postgres_database_size_bytes,
            after.postgres_database_size_bytes,
        ),
        postgres_table_writes,
        postgres_table_writes_total,
        postgres_statement_calls,
        postgres_statement_calls_by_table,
        postgres_statement_calls_total,
    }
}

fn libsql_table_write_delta(
    before: &DbProbeSnapshot,
    after: &DbProbeSnapshot,
) -> Vec<LibSqlTableWriteDelta> {
    let before = before
        .libsql_table_writes
        .iter()
        .map(|table| (table.table.as_str(), table))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .libsql_table_writes
        .iter()
        .map(|table| (table.table.as_str(), table))
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|table| {
            let before = before.get(table).copied();
            let after = after.get(table).copied();
            LibSqlTableWriteDelta {
                table: table.to_string(),
                inserts: counter_delta(
                    before.map_or(0, |value| value.inserts),
                    after.map_or(0, |value| value.inserts),
                ),
                updates: counter_delta(
                    before.map_or(0, |value| value.updates),
                    after.map_or(0, |value| value.updates),
                ),
                deletes: counter_delta(
                    before.map_or(0, |value| value.deletes),
                    after.map_or(0, |value| value.deletes),
                ),
            }
        })
        .collect()
}

fn table_write_delta(
    before: &DbProbeSnapshot,
    after: &DbProbeSnapshot,
) -> Vec<PostgresTableWriteDelta> {
    let before = before
        .postgres_table_writes
        .iter()
        .map(|table| (table.table.as_str(), table))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .postgres_table_writes
        .iter()
        .map(|table| (table.table.as_str(), table))
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|table| {
            let before = before.get(table).copied();
            let after = after.get(table).copied();
            PostgresTableWriteDelta {
                table: table.to_string(),
                inserts: counter_delta(
                    before.map_or(0, |value| value.inserts),
                    after.map_or(0, |value| value.inserts),
                ),
                updates: counter_delta(
                    before.map_or(0, |value| value.updates),
                    after.map_or(0, |value| value.updates),
                ),
                deletes: counter_delta(
                    before.map_or(0, |value| value.deletes),
                    after.map_or(0, |value| value.deletes),
                ),
            }
        })
        .collect()
}

fn statement_call_delta(
    before: &DbProbeSnapshot,
    after: &DbProbeSnapshot,
) -> Vec<PostgresStatementCallDelta> {
    let before = before
        .postgres_statement_calls
        .iter()
        .map(|statement| (statement.query_id.as_str(), statement))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .postgres_statement_calls
        .iter()
        .map(|statement| (statement.query_id.as_str(), statement))
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|query_id| {
            let before = before.get(query_id).copied();
            let after = after.get(query_id).copied();
            let metadata = after.or(before);
            PostgresStatementCallDelta {
                query_id: query_id.to_string(),
                operation: metadata
                    .map(|statement| statement.operation.clone())
                    .unwrap_or_default(),
                tables: metadata
                    .map(|statement| statement.tables.clone())
                    .unwrap_or_default(),
                calls: counter_delta(
                    before.map_or(0, |statement| statement.calls),
                    after.map_or(0, |statement| statement.calls),
                ),
            }
        })
        .collect()
}

#[doc(hidden)]
pub fn pg_stat_statements_unavailable(url: &str, detail: impl std::fmt::Display) -> String {
    format!(
        "postgres DB write measurement requires a loaded pg_stat_statements extension: {detail}. \
         Run CREATE EXTENSION pg_stat_statements in the target database, add pg_stat_statements \
         to shared_preload_libraries, and restart PostgreSQL. target={}",
        redact_postgres_url(url)
    )
}

#[doc(hidden)]
pub fn pg_stat_statements_reset_supported(version: &str) -> bool {
    let mut components = version.split('.');
    let Some(major) = components
        .next()
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return false;
    };
    let Some(minor) = components
        .next()
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return false;
    };
    major > 1 || (major == 1 && minor >= 7)
}

#[doc(hidden)]
pub fn sanitize_postgres_error(resolved_url: &str, error: impl std::fmt::Display) -> String {
    let mut message = format!("postgres probe failed: {error}");
    message = message.replace(resolved_url, &redact_postgres_url(resolved_url));
    message
}

fn i64_to_u64(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
}

fn delta(before: Option<u64>, after: Option<u64>) -> Option<i128> {
    Some(counter_delta(before?, after?))
}
fn counter_delta(before: u64, after: u64) -> i128 {
    i128::from(after) - i128::from(before)
}

#[cfg(test)]
mod tests {
    use super::{
        DbProbeConfig, DbProbeError, DbProbeTarget, POSTGRES_STATS_SETTLE_DURATION,
        PostgresTableWriteDelta, begin, postgres_relation_write_totals,
        retain_libsql_snapshot_or_cleanup, settlement_delay,
    };

    #[test]
    fn settled_capture_waits_only_for_postgres_stats() {
        assert_eq!(
            settlement_delay(&DbProbeTarget::Postgres {
                url: "postgresql://localhost/ironclaw".to_string(),
            }),
            Some(POSTGRES_STATS_SETTLE_DURATION)
        );
        assert_eq!(
            settlement_delay(&DbProbeTarget::LibSql {
                path: "measurement.db".into(),
            }),
            None
        );
    }

    #[tokio::test]
    async fn failed_baseline_capture_removes_libsql_instrumentation() {
        let path = std::env::temp_dir().join(format!(
            "ironclaw-stress-baseline-failure-{}.db",
            uuid::Uuid::new_v4().simple()
        ));
        let database = libsql::Builder::new_local(&path)
            .build()
            .await
            .expect("build local libSQL");
        let connection = database.connect().expect("connect local libSQL");
        connection
            .execute_batch(
                "CREATE TABLE root_filesystem_entries (id INTEGER PRIMARY KEY);\
                 CREATE TABLE root_filesystem_events (id INTEGER PRIMARY KEY);\
                 CREATE TABLE root_filesystem_index_specs (id INTEGER PRIMARY KEY);\
                 CREATE TABLE root_filesystem_ordered_index_rows (id INTEGER PRIMARY KEY);\
                 CREATE TABLE root_filesystem_sequences (id INTEGER PRIMARY KEY);\
                 CREATE TABLE trigger_records (id INTEGER PRIMARY KEY);\
                 CREATE TABLE trigger_run_history (id INTEGER PRIMARY KEY);\
                 CREATE TABLE ironclaw_stress_write_counters (\
                   table_name TEXT NOT NULL,\
                   operation TEXT NOT NULL,\
                   row_count INTEGER NOT NULL,\
                   PRIMARY KEY (table_name, operation)\
                 );\
                 INSERT INTO ironclaw_stress_write_counters(table_name, operation, row_count)\
                 VALUES ('root_filesystem_entries', 'insert', -1);",
            )
            .await
            .expect("create measured tables and invalid counter");
        drop(connection);
        drop(database);

        let config = DbProbeConfig::libsql(&path, false);
        let error = begin(&config)
            .await
            .expect_err("invalid baseline counter must fail");
        assert!(error.to_string().contains("invalid libSQL write counter"));

        let database = libsql::Builder::new_local(&path)
            .build()
            .await
            .expect("reopen local libSQL");
        let connection = database.connect().expect("reconnect local libSQL");
        let remaining = connection
            .query(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name LIKE 'ironclaw_stress_%'",
                (),
            )
            .await
            .expect("query instrumentation objects")
            .next()
            .await
            .expect("read instrumentation count")
            .expect("instrumentation count row")
            .get::<i64>(0)
            .expect("instrumentation count");
        assert_eq!(remaining, 0);
        drop(connection);
        drop(database);
        tokio::fs::remove_file(path)
            .await
            .expect("remove measurement database");
    }

    #[tokio::test]
    async fn baseline_and_cleanup_failures_retain_both_sources() {
        let path = std::env::temp_dir()
            .join(format!(
                "ironclaw-stress-missing-parent-{}",
                uuid::Uuid::new_v4().simple()
            ))
            .join("measurement.db");
        let primary = DbProbeError::with_source(
            "libsql",
            "capture",
            "forced baseline capture failure",
            Box::new(std::io::Error::other("baseline source")),
        );

        let error = retain_libsql_snapshot_or_cleanup(&path, Err(primary))
            .await
            .expect_err("cleanup must fail for a database in a missing directory");
        let DbProbeError::BaselineCleanup {
            primary, cleanup, ..
        } = &error
        else {
            panic!("expected combined baseline and cleanup failure");
        };
        assert!(std::error::Error::source(primary.as_ref()).is_some());
        assert!(std::error::Error::source(cleanup.as_ref()).is_some());
        assert!(
            error
                .to_string()
                .contains("forced baseline capture failure")
        );
        assert!(error.to_string().contains("cleanup after baseline failure"));
    }
    #[test]
    fn postgres_relation_writes_sum_duplicate_schema_relations() {
        let rows = [
            PostgresTableWriteDelta {
                table: "public.root_filesystem_entries".to_string(),
                inserts: 2,
                updates: 3,
                deletes: 1,
            },
            PostgresTableWriteDelta {
                table: "tenant.root_filesystem_entries".to_string(),
                inserts: 5,
                updates: 7,
                deletes: 0,
            },
            PostgresTableWriteDelta {
                table: "root_filesystem_events".to_string(),
                inserts: 11,
                updates: 0,
                deletes: 0,
            },
        ];

        let totals = postgres_relation_write_totals(&rows);

        assert_eq!(totals.get("root_filesystem_entries"), Some(&18));
        assert_eq!(totals.get("root_filesystem_events"), Some(&11));
    }
}
