use std::{
    collections::{BTreeMap, BTreeSet},
    io::ErrorKind,
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use tokio_postgres::Client;

use crate::redaction::redact_postgres_url;
use crate::{Args, Backend};

const MEASUREMENT_TABLES: &[&str] = &[
    "root_filesystem_entries",
    "root_filesystem_events",
    "root_filesystem_index_specs",
    "root_filesystem_ordered_index_rows",
    "root_filesystem_sequences",
    "trigger_records",
    "trigger_run_history",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DbProbeSummary {
    pub(crate) before: DbProbeSnapshot,
    pub(crate) after: DbProbeSnapshot,
    pub(crate) delta: DbProbeDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) idle_after: Option<DbProbeSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) idle_delta: Option<DbProbeDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) measurement: Option<DbWriteMeasurement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DbWriteMeasurement {
    pub(crate) workload: String,
    pub(crate) tool_calls_per_turn: usize,
    pub(crate) idle_observation_seconds: u64,
    pub(crate) reset_stats: bool,
    pub(crate) stats_scope: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DbProbeSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) libsql_file_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) libsql_wal_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) libsql_shm_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) postgres_database_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) postgres_active_connections: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) postgres_idle_connections: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) postgres_waiting_connections: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) postgres_table_writes: Vec<PostgresTableWrites>,
    #[serde(default)]
    pub(crate) postgres_table_writes_total: PostgresWriteCounts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) postgres_statement_calls: Vec<PostgresStatementCalls>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) postgres_statement_calls_by_table: Vec<PostgresTableStatementCalls>,
    #[serde(default)]
    pub(crate) postgres_statement_calls_total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PostgresWriteCounts {
    pub(crate) inserts: u64,
    pub(crate) updates: u64,
    pub(crate) deletes: u64,
}

impl PostgresWriteCounts {
    pub(crate) fn total(&self) -> u64 {
        self.inserts
            .saturating_add(self.updates)
            .saturating_add(self.deletes)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PostgresTableWrites {
    pub(crate) table: String,
    pub(crate) inserts: u64,
    pub(crate) updates: u64,
    pub(crate) deletes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PostgresStatementCalls {
    pub(crate) query_id: String,
    pub(crate) operation: String,
    pub(crate) tables: Vec<String>,
    pub(crate) calls: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PostgresTableStatementCalls {
    pub(crate) table: String,
    pub(crate) calls: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DbProbeDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) libsql_file_bytes: Option<i128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) libsql_wal_bytes: Option<i128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) libsql_shm_bytes: Option<i128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) postgres_database_size_bytes: Option<i128>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) postgres_table_writes: Vec<PostgresTableWriteDelta>,
    #[serde(default)]
    pub(crate) postgres_table_writes_total: PostgresWriteDelta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) postgres_statement_calls: Vec<PostgresStatementCallDelta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) postgres_statement_calls_by_table: Vec<PostgresTableStatementCallDelta>,
    #[serde(default)]
    pub(crate) postgres_statement_calls_total: i128,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PostgresWriteDelta {
    pub(crate) inserts: i128,
    pub(crate) updates: i128,
    pub(crate) deletes: i128,
}

impl PostgresWriteDelta {
    pub(crate) fn total(&self) -> i128 {
        self.inserts + self.updates + self.deletes
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PostgresTableWriteDelta {
    pub(crate) table: String,
    pub(crate) inserts: i128,
    pub(crate) updates: i128,
    pub(crate) deletes: i128,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PostgresStatementCallDelta {
    pub(crate) query_id: String,
    pub(crate) operation: String,
    pub(crate) tables: Vec<String>,
    pub(crate) calls: i128,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PostgresTableStatementCallDelta {
    pub(crate) table: String,
    pub(crate) calls: i128,
}

pub(crate) async fn capture(args: &Args) -> DbProbeSnapshot {
    match args.backend {
        Backend::Libsql => capture_libsql(args).await,
        Backend::Postgres => capture_postgres(args, false).await,
    }
}

pub(crate) async fn begin_measurement(args: &Args) -> Result<DbProbeSnapshot, String> {
    let url = crate::resolve_postgres_url(args)?;
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .map_err(|error| sanitize_postgres_error(&url, error))?;
    let connection_handle = tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("[ironclaw-stress] postgres probe connection error: {error}");
        }
    });
    ensure_pg_stat_statements(&client, &url).await?;
    if args.db_write_reset_stats {
        reset_measurement_stats(&client, &url).await?;
    }
    drop(client);
    let _ = connection_handle.await;
    capture_measurement(args).await
}

pub(crate) async fn capture_measurement(args: &Args) -> Result<DbProbeSnapshot, String> {
    let url = crate::resolve_postgres_url(args)?;
    try_capture_postgres(&url, true)
        .await
        .map_err(|error| sanitize_postgres_error(&url, error))
}

pub(crate) fn summarize(before: DbProbeSnapshot, after: DbProbeSnapshot) -> DbProbeSummary {
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

pub(crate) fn summarize_measurement(
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

async fn capture_libsql(args: &Args) -> DbProbeSnapshot {
    let path = args
        .libsql_path
        .clone()
        .unwrap_or_else(crate::default_libsql_path);
    match try_capture_libsql(path).await {
        Ok(snapshot) => snapshot,
        Err(error) => DbProbeSnapshot {
            error: Some(format!("libsql probe failed: {error}")),
            ..DbProbeSnapshot::default()
        },
    }
}

async fn try_capture_libsql(path: PathBuf) -> Result<DbProbeSnapshot, std::io::Error> {
    Ok(DbProbeSnapshot {
        libsql_file_bytes: Some(file_size(&path).await?),
        libsql_wal_bytes: Some(file_size(&sidecar_path(&path, "-wal")).await?),
        libsql_shm_bytes: Some(file_size(&sidecar_path(&path, "-shm")).await?),
        ..DbProbeSnapshot::default()
    })
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

async fn capture_postgres(args: &Args, include_write_stats: bool) -> DbProbeSnapshot {
    let url = match crate::resolve_postgres_url(args) {
        Ok(url) => url,
        Err(error) => {
            return DbProbeSnapshot {
                error: Some(format!("postgres probe failed: {error}")),
                ..DbProbeSnapshot::default()
            };
        }
    };

    match try_capture_postgres(&url, include_write_stats).await {
        Ok(snapshot) => snapshot,
        Err(error) => DbProbeSnapshot {
            error: Some(sanitize_postgres_error(&url, error)),
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

async fn ensure_pg_stat_statements(client: &Client, url: &str) -> Result<(), String> {
    let installed: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_stat_statements')",
            &[],
        )
        .await
        .map_err(|error| sanitize_postgres_error(url, error))?
        .get(0);
    if !installed {
        return Err(pg_stat_statements_unavailable(
            url,
            "the pg_stat_statements extension is not installed in this database",
        ));
    }
    client
        .query("SELECT calls FROM pg_stat_statements LIMIT 1", &[])
        .await
        .map_err(|error| pg_stat_statements_unavailable(url, error))?;
    Ok(())
}

async fn reset_measurement_stats(client: &Client, url: &str) -> Result<(), String> {
    client
        .query(
            "SELECT pg_stat_statements_reset(0::oid, (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database()), 0::bigint)",
            &[],
        )
        .await
        .map_err(|error| {
            format!(
                "{}; --db-write-reset-stats explicitly requested a current-database reset. \
                 Omit the flag to use non-destructive snapshot deltas",
                sanitize_postgres_error(url, error)
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
        .map_err(|error| {
            format!(
                "{}; --db-write-reset-stats explicitly requested per-table counter resets. \
                 Omit the flag to use non-destructive snapshot deltas",
                sanitize_postgres_error(url, error)
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
            "SELECT relname, n_tup_ins::bigint, n_tup_upd::bigint, n_tup_del::bigint \
             FROM pg_stat_user_tables \
             WHERE relname = ANY($1::text[]) \
             ORDER BY relname",
            &[&table_names],
        )
        .await?;
    let mut table_writes = MEASUREMENT_TABLES
        .iter()
        .map(|table| {
            (
                (*table).to_string(),
                PostgresTableWrites {
                    table: (*table).to_string(),
                    ..PostgresTableWrites::default()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    for row in table_rows {
        let table: String = row.get(0);
        table_writes.insert(
            table.clone(),
            PostgresTableWrites {
                table,
                inserts: i64_to_u64(row.get(1)).unwrap_or(0),
                updates: i64_to_u64(row.get(2)).unwrap_or(0),
                deletes: i64_to_u64(row.get(3)).unwrap_or(0),
            },
        );
    }
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
        .filter_map(|row| {
            let query_id = row.try_get::<_, i64>(0).ok()?;
            let query = row.try_get::<_, String>(1).ok()?;
            let calls = i64_to_u64(row.try_get::<_, i64>(2).ok()?)?;
            Some((query_id, query, calls))
        })
        .collect::<Vec<_>>();
    snapshot.postgres_statement_calls = aggregate_statement_calls(
        raw_statements
            .iter()
            .map(|(query_id, query, calls)| (*query_id, query.as_str(), *calls)),
    );
    Ok(())
}

pub(crate) fn aggregate_statement_calls<'a>(
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

pub(crate) fn pg_stat_statements_unavailable(url: &str, detail: impl std::fmt::Display) -> String {
    format!(
        "postgres DB write measurement requires a loaded pg_stat_statements extension: {detail}. \
         Run CREATE EXTENSION pg_stat_statements in the target database, add pg_stat_statements \
         to shared_preload_libraries, and restart PostgreSQL. target={}",
        redact_postgres_url(url)
    )
}

pub(crate) fn sanitize_postgres_error(resolved_url: &str, error: impl std::fmt::Display) -> String {
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
