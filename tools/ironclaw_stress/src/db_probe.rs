use std::{io::ErrorKind, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::redaction::redact_postgres_url;
use crate::{Args, Backend};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DbProbeSummary {
    pub(crate) before: DbProbeSnapshot,
    pub(crate) after: DbProbeSnapshot,
    pub(crate) delta: DbProbeDelta,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
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
}

pub(crate) async fn capture(args: &Args) -> DbProbeSnapshot {
    match args.backend {
        Backend::Libsql => capture_libsql(args).await,
        Backend::Postgres => capture_postgres(args).await,
    }
}

pub(crate) fn summarize(before: DbProbeSnapshot, after: DbProbeSnapshot) -> DbProbeSummary {
    let delta = DbProbeDelta {
        libsql_file_bytes: delta(before.libsql_file_bytes, after.libsql_file_bytes),
        libsql_wal_bytes: delta(before.libsql_wal_bytes, after.libsql_wal_bytes),
        libsql_shm_bytes: delta(before.libsql_shm_bytes, after.libsql_shm_bytes),
        postgres_database_size_bytes: delta(
            before.postgres_database_size_bytes,
            after.postgres_database_size_bytes,
        ),
    };
    DbProbeSummary {
        before,
        after,
        delta,
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

async fn capture_postgres(args: &Args) -> DbProbeSnapshot {
    let url = match crate::resolve_postgres_url(args) {
        Ok(url) => url,
        Err(error) => {
            return DbProbeSnapshot {
                error: Some(format!("postgres probe failed: {error}")),
                ..DbProbeSnapshot::default()
            };
        }
    };

    match try_capture_postgres(&url).await {
        Ok(snapshot) => snapshot,
        Err(error) => DbProbeSnapshot {
            error: Some(sanitize_postgres_error(&url, error)),
            ..DbProbeSnapshot::default()
        },
    }
}

async fn try_capture_postgres(
    url: &str,
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
    drop(client);
    let _ = connection_handle.await;

    Ok(DbProbeSnapshot {
        postgres_database_size_bytes: i64_to_u64(row.get(0)),
        postgres_active_connections: i64_to_u64(row.get(1)),
        postgres_idle_connections: i64_to_u64(row.get(2)),
        postgres_waiting_connections: i64_to_u64(row.get(3)),
        ..DbProbeSnapshot::default()
    })
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
    Some(i128::from(after?) - i128::from(before?))
}
