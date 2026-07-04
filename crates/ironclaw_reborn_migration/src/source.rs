//! v1 / engine-v2 read side.
//!
//! Opens the legacy database through the root `ironclaw` crate and exposes it
//! as an `Arc<dyn Database>` plus the backend-specific handles that satellite
//! v1 stores (secrets, wasm tools, identities) need. Engine-v2 mission/project
//! state is not a separate connection — it lives as JSON blobs inside the
//! `memory_documents` table and is read through the same `Database` handle
//! (see [`crate::convert::automations`] and [`crate::v2_model`]).

use std::sync::Arc;

use ironclaw::config::{DatabaseBackend, DatabaseConfig, SslMode};
use ironclaw::db::{Database, DatabaseHandles, connect_with_handles};
use secrecy::SecretString;

use crate::error::MigrationError;
use crate::options::SourceDb;

/// A live, migrations-applied handle to the v1 source database.
pub struct V1Source {
    pub db: Arc<dyn Database>,
    /// Backend-specific handles for satellite v1 stores (secrets) and raw
    /// distinct-user / channel-identity discovery.
    pub handles: DatabaseHandles,
}

/// Tables a v1 user_id can appear in. Queried independently so a DB missing one
/// (e.g. a minimal libSQL install without `settings`) still discovers users
/// from the others.
const USER_ID_TABLES: [&str; 4] = ["conversations", "routines", "memory_documents", "settings"];

impl V1Source {
    pub async fn open(source: &SourceDb) -> Result<Self, MigrationError> {
        let config = source_to_config(source);
        let (db, handles) = connect_with_handles(&config)
            .await
            .map_err(|e| MigrationError::OpenSource(e.to_string()))?;
        Ok(Self { db, handles })
    }

    /// Discover every distinct v1 `user_id` present in the source. v1 single-user
    /// installs (especially libSQL) may have no `users` table, so users are
    /// discovered from the data rows themselves, tolerating any table that does
    /// not exist.
    pub(crate) async fn distinct_users(&self) -> Result<Vec<String>, MigrationError> {
        let mut users = std::collections::BTreeSet::new();
        for table in USER_ID_TABLES {
            for uid in self.distinct_user_ids_in(table).await {
                if !uid.is_empty() {
                    users.insert(uid);
                }
            }
        }
        Ok(users.into_iter().collect())
    }

    /// Best-effort `SELECT DISTINCT user_id FROM <table>` against the raw handle.
    /// Returns an empty vec (not an error) if the table is absent — a missing
    /// table means "no users here", not a migration failure.
    pub(crate) async fn distinct_user_ids_in(&self, table: &str) -> Vec<String> {
        let sql = format!("SELECT DISTINCT user_id FROM {table}");
        #[cfg(feature = "libsql")]
        if let Some(db) = self.handles.libsql_db.as_ref() {
            let Ok(conn) = db.connect() else {
                return Vec::new();
            };
            let Ok(mut rows) = conn.query(&sql, ()).await else {
                return Vec::new();
            };
            let mut out = Vec::new();
            while let Ok(Some(row)) = rows.next().await {
                if let Ok(value) = row.get::<String>(0) {
                    out.push(value);
                }
            }
            return out;
        }
        #[cfg(feature = "postgres")]
        if let Some(pool) = self.handles.pg_pool.as_ref() {
            let Ok(client) = pool.get().await else {
                return Vec::new();
            };
            let Ok(stmt_rows) = client.query(sql.as_str(), &[]).await else {
                return Vec::new();
            };
            return stmt_rows
                .iter()
                .filter_map(|row| row.try_get::<_, String>(0).ok())
                .collect();
        }
        Vec::new()
    }
}

fn source_to_config(source: &SourceDb) -> DatabaseConfig {
    match source {
        SourceDb::LibSql { path } => DatabaseConfig {
            backend: DatabaseBackend::LibSql,
            // libSQL backend ignores `url`; the resolver uses this sentinel too.
            url: SecretString::from("unused://libsql"),
            pool_size: 4,
            ssl_mode: SslMode::default(),
            libsql_path: Some(path.clone()),
            libsql_url: None,
            libsql_auth_token: None,
        },
        SourceDb::Postgres { url } => DatabaseConfig {
            backend: DatabaseBackend::Postgres,
            url: SecretString::from(url.clone()),
            pool_size: 4,
            ssl_mode: SslMode::default(),
            libsql_path: None,
            libsql_url: None,
            libsql_auth_token: None,
        },
    }
}
