//! v1 / engine-v2 read side.
//!
//! Opens the legacy database through the root `ironclaw` crate and exposes it
//! as an `Arc<dyn Database>` plus the backend-specific handles that satellite
//! v1 stores (secrets, wasm tools, identities) need. Engine-v2 mission/project
//! state is not a separate connection — it lives as JSON blobs inside the
//! `memory_documents` table and is read through the same `Database` handle
//! (see [`crate::convert::automations`] and [`crate::v2_model`]).

use std::sync::Arc;

#[cfg(feature = "postgres")]
use ironclaw::config::{DatabaseBackend, DatabaseConfig, SslMode};
use ironclaw::db::{Database, DatabaseHandles};
#[cfg(feature = "postgres")]
use secrecy::SecretString;

use crate::error::MigrationError;
use crate::inventory::RawTableInventory;
use crate::manifest::SourceFingerprint;
use crate::options::SourceDb;

/// A read-only-by-contract handle to a v1 source snapshot.
///
/// Crate-internal: the only public entry point is [`crate::run_migration`], and
/// this handle is consumed exclusively by the in-crate converters (mirrors the
/// symmetric `RebornTarget` visibility).
pub(crate) struct V1Source {
    pub(crate) db: Arc<dyn Database>,
    /// Backend-specific handles for satellite v1 stores (secrets) and raw
    /// distinct-user / channel-identity discovery.
    pub(crate) handles: DatabaseHandles,
}

pub(crate) struct ProjectDocument {
    pub(crate) user_id: String,
    pub(crate) path: String,
    pub(crate) content: String,
}

/// Tables a v1 user_id can appear in. Queried independently so a DB missing one
/// (e.g. a minimal libSQL install without `settings`) still discovers users
/// from the others.
const USER_ID_TABLES: [&str; 4] = ["conversations", "routines", "memory_documents", "settings"];

impl V1Source {
    pub(crate) async fn open(source: &SourceDb) -> Result<Self, MigrationError> {
        if let SourceDb::LibSql { path } = source {
            let metadata = std::fs::metadata(path).map_err(|error| {
                MigrationError::OpenSource(format!(
                    "snapshot {} must already exist and be readable: {error}",
                    path.display()
                ))
            })?;
            if !metadata.is_file() {
                return Err(MigrationError::OpenSource(format!(
                    "snapshot {} is not a regular file",
                    path.display()
                )));
            }
        }
        // This constructor intentionally skips every v1 schema migration. The
        // migration tool reads historical schemas; it must never upgrade the
        // operator's source as a side effect of inspection.
        let (db, handles) = match source {
            SourceDb::LibSql { path: source_path } => {
                #[cfg(feature = "libsql")]
                {
                    let backend =
                        ironclaw::db::libsql::LibSqlBackend::new_local_read_only(source_path)
                            .await
                            .map_err(source_open_error)?;
                    let handles = handles_with_libsql(backend.shared_db());
                    (Arc::new(backend) as Arc<dyn Database>, handles)
                }
                #[cfg(not(feature = "libsql"))]
                {
                    let _ = source_path;
                    return Err(MigrationError::OpenSource(
                        "libSQL support is not compiled into this migrator".to_string(),
                    ));
                }
            }
            SourceDb::Postgres { .. } => {
                #[cfg(feature = "postgres")]
                {
                    let config = source_to_config(source)?;
                    let backend = ironclaw::db::postgres::PgBackend::new(&config)
                        .await
                        .map_err(|_| {
                            MigrationError::OpenSource(
                                "PostgreSQL source connection failed (connection details redacted)"
                                    .to_string(),
                            )
                        })?;
                    let handles = handles_with_postgres(backend.pool());
                    (Arc::new(backend) as Arc<dyn Database>, handles)
                }
                #[cfg(not(feature = "postgres"))]
                {
                    return Err(MigrationError::OpenSource(
                        "PostgreSQL support is not compiled into this migrator".to_string(),
                    ));
                }
            }
        };

        #[cfg(feature = "libsql")]
        if let Some(database) = handles.libsql_db.as_ref() {
            let connection = database
                .connect()
                .map_err(|error| MigrationError::OpenSource(error.to_string()))?;
            connection
                .execute("PRAGMA query_only = ON", ())
                .await
                .map_err(|error| MigrationError::OpenSource(error.to_string()))?;
        }
        Ok(Self { db, handles })
    }

    pub(crate) async fn fingerprint(
        &self,
        source: &SourceDb,
    ) -> Result<SourceFingerprint, MigrationError> {
        match source {
            SourceDb::LibSql { path } => fingerprint_local_snapshot(path).await,
            SourceDb::Postgres { .. } => {
                let tables = self.table_inventory().await?;
                let mut material = String::from("ironclaw-v1-postgres-v1\n");
                for table in tables {
                    material.push_str(&table.name);
                    material.push(':');
                    material.push_str(&table.count.to_string());
                    material.push(':');
                    material.push_str(&table.checksum);
                    material.push('\n');
                }
                Ok(SourceFingerprint {
                    algorithm: "sha256-table-inventory-v1".to_string(),
                    value: ironclaw_common::hashing::sha256_hex(material.as_bytes()),
                })
            }
        }
    }

    pub(crate) async fn schema_version(&self) -> Result<Option<String>, MigrationError> {
        #[cfg(feature = "libsql")]
        if let Some(database) = self.handles.libsql_db.as_ref() {
            let connection = database.connect().map_err(source_open_error)?;
            connection
                .execute("PRAGMA query_only = ON", ())
                .await
                .map_err(source_open_error)?;
            let mut rows = match connection
                .query("SELECT MAX(version) FROM _migrations", ())
                .await
            {
                Ok(rows) => rows,
                Err(error) if is_missing_table_error(&error.to_string()) => return Ok(None),
                Err(error) => return Err(source_read_error("schema", error)),
            };
            let Some(row) = rows
                .next()
                .await
                .map_err(|error| source_read_error("schema", error))?
            else {
                return Ok(None);
            };
            return match row.get::<Option<i64>>(0) {
                Ok(version) => Ok(version.map(|value| value.to_string())),
                Err(error) => Err(source_read_error("schema", error)),
            };
        }
        #[cfg(feature = "postgres")]
        if let Some(pool) = self.handles.pg_pool.as_ref() {
            let client = pool.get().await.map_err(source_open_error)?;
            let row = match client
                .query_opt(
                    "SELECT MAX(version)::text FROM refinery_schema_history",
                    &[],
                )
                .await
            {
                Ok(row) => row,
                Err(error) if is_missing_table_error(&error.to_string()) => return Ok(None),
                Err(error) => return Err(source_read_error("schema", error)),
            };
            return Ok(row.and_then(|row| row.try_get::<_, Option<String>>(0).ok().flatten()));
        }
        Ok(None)
    }

    pub(crate) async fn table_inventory(&self) -> Result<Vec<RawTableInventory>, MigrationError> {
        #[cfg(feature = "libsql")]
        if let Some(database) = self.handles.libsql_db.as_ref() {
            let connection = database.connect().map_err(source_open_error)?;
            connection
                .execute("PRAGMA query_only = ON", ())
                .await
                .map_err(source_open_error)?;
            let mut rows = connection
                .query(
                    "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                    (),
                )
                .await
                .map_err(|error| source_read_error("inventory", error))?;
            let mut names = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|error| source_read_error("inventory", error))?
            {
                names.push(
                    row.get::<String>(0)
                        .map_err(|error| source_read_error("inventory", error))?,
                );
            }
            let mut inventory = Vec::with_capacity(names.len());
            for name in names {
                let sql = format!("SELECT COUNT(*) FROM {}", quote_identifier(&name));
                let mut rows = connection
                    .query(&sql, ())
                    .await
                    .map_err(|error| source_read_error(&name, error))?;
                let row = rows
                    .next()
                    .await
                    .map_err(|error| source_read_error(&name, error))?
                    .ok_or_else(|| MigrationError::ReadSource {
                        domain: name.clone(),
                        reason: "COUNT(*) returned no row".to_string(),
                    })?;
                let count = row
                    .get::<i64>(0)
                    .map_err(|error| source_read_error(&name, error))?
                    .try_into()
                    .map_err(|_| MigrationError::ReadSource {
                        domain: name.clone(),
                        reason: "negative row count".to_string(),
                    })?;
                inventory.push(RawTableInventory {
                    name: name.clone(),
                    count,
                    checksum: ironclaw_common::hashing::sha256_hex(
                        format!("libsql-table-v1:{name}:{count}").as_bytes(),
                    ),
                });
            }
            return Ok(inventory);
        }
        #[cfg(feature = "postgres")]
        if let Some(pool) = self.handles.pg_pool.as_ref() {
            let client = pool.get().await.map_err(source_open_error)?;
            let rows = client
                .query(
                    "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY table_name",
                    &[],
                )
                .await
                .map_err(|error| source_read_error("inventory", error))?;
            let mut inventory = Vec::with_capacity(rows.len());
            for row in rows {
                let name: String = row
                    .try_get(0)
                    .map_err(|error| source_read_error("inventory", error))?;
                // PostgreSQL's MVCC transaction ids let us detect inserts and
                // updates without hashing row contents. Some v1 tables contain
                // bearer-token hashes, provider configuration, or encrypted
                // secret material; manifest fingerprints must not be derived
                // from those values.
                let sql = format!(
                    "SELECT COUNT(*)::bigint, COALESCE(SUM((xmin::text)::numeric), 0)::text FROM {}",
                    quote_identifier(&name)
                );
                let row = client
                    .query_one(&sql, &[])
                    .await
                    .map_err(|error| source_read_error(&name, error))?;
                let count: i64 = row
                    .try_get(0)
                    .map_err(|error| source_read_error(&name, error))?;
                let row_checksum: String = row
                    .try_get(1)
                    .map_err(|error| source_read_error(&name, error))?;
                inventory.push(RawTableInventory {
                    name: name.clone(),
                    count: count.try_into().map_err(|_| MigrationError::ReadSource {
                        domain: name.clone(),
                        reason: "negative row count".to_string(),
                    })?,
                    checksum: ironclaw_common::hashing::sha256_hex(
                        format!("postgres-table-mvcc-v1:{name}:{count}:{row_checksum}").as_bytes(),
                    ),
                });
            }
            return Ok(inventory);
        }
        Ok(Vec::new())
    }

    /// Discover every distinct v1 `user_id` present in the source. v1 single-user
    /// installs (especially libSQL) may have no `users` table, so users are
    /// discovered from the data rows themselves, tolerating any table that does
    /// not exist.
    pub(crate) async fn distinct_users(&self) -> Result<Vec<String>, MigrationError> {
        let mut users = std::collections::BTreeSet::new();
        for table in USER_ID_TABLES {
            for uid in self.distinct_user_ids_in(table, "user_id").await? {
                if !uid.is_empty() {
                    users.insert(uid);
                }
            }
        }
        for uid in self.distinct_user_ids_in("users", "id").await? {
            if !uid.is_empty() {
                users.insert(uid);
            }
        }
        Ok(users.into_iter().collect())
    }

    #[allow(dead_code, reason = "staged historical-user converter read port")]
    pub(crate) async fn users(&self) -> Result<Vec<ironclaw::db::UserRecord>, MigrationError> {
        self.db.list_users(None).await.or_else(|error| {
            if is_missing_table_error(&error.to_string()) {
                Ok(Vec::new())
            } else {
                Err(source_read_error("users", error))
            }
        })
    }

    #[allow(dead_code, reason = "staged API-token disposition converter read port")]
    pub(crate) async fn api_token_count(&self) -> Result<u64, MigrationError> {
        let mut count = 0_u64;
        for user in self.distinct_users().await? {
            let tokens = self.db.list_api_tokens(&user).await.or_else(|error| {
                if is_missing_table_error(&error.to_string()) {
                    Ok(Vec::new())
                } else {
                    Err(source_read_error("api_tokens", error))
                }
            })?;
            count = count.saturating_add(tokens.len() as u64);
        }
        Ok(count)
    }

    #[allow(dead_code, reason = "staged typed-settings converter read port")]
    pub(crate) async fn settings(
        &self,
        user_id: &str,
    ) -> Result<Vec<ironclaw::history::SettingRow>, MigrationError> {
        self.db.list_settings(user_id).await.or_else(|error| {
            if is_missing_table_error(&error.to_string()) {
                Ok(Vec::new())
            } else {
                Err(source_read_error("settings", error))
            }
        })
    }

    #[allow(dead_code, reason = "staged projects and memory converter read port")]
    pub(crate) async fn memory_documents(
        &self,
        user_id: &str,
        agent_id: Option<uuid::Uuid>,
    ) -> Result<Vec<ironclaw::workspace::MemoryDocument>, MigrationError> {
        self.db
            .list_documents(user_id, agent_id)
            .await
            .map_err(|error| source_read_error("memory_documents", error))
    }

    /// Read every engine-v2 project document regardless of its optional
    /// `agent_id`. The v1 `list_documents(user, None)` API means
    /// `agent_id IS NULL`, not "all agents", so project discovery needs this
    /// narrow raw read to avoid silently omitting agent-scoped metadata.
    pub(crate) async fn project_documents(&self) -> Result<Vec<ProjectDocument>, MigrationError> {
        #[cfg(feature = "libsql")]
        if let Some(database) = self.handles.libsql_db.as_ref() {
            let connection = database.connect().map_err(source_open_error)?;
            connection
                .execute("PRAGMA query_only = ON", ())
                .await
                .map_err(source_open_error)?;
            let mut rows = match connection
                .query(
                    "SELECT user_id, path, content FROM memory_documents
                     WHERE path LIKE 'projects/%/.project.json'
                        OR path LIKE '.system/engine/projects/%/project.json'
                        OR path LIKE 'engine/projects/%/project.json'
                     ORDER BY path, user_id, COALESCE(agent_id, '')",
                    (),
                )
                .await
            {
                Ok(rows) => rows,
                Err(error) if is_missing_table_error(&error.to_string()) => return Ok(Vec::new()),
                Err(error) => return Err(source_read_error("projects", error)),
            };
            let mut documents = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|error| source_read_error("projects", error))?
            {
                documents.push(ProjectDocument {
                    user_id: row
                        .get(0)
                        .map_err(|error| source_read_error("projects", error))?,
                    path: row
                        .get(1)
                        .map_err(|error| source_read_error("projects", error))?,
                    content: row
                        .get(2)
                        .map_err(|error| source_read_error("projects", error))?,
                });
            }
            return Ok(documents);
        }

        #[cfg(feature = "postgres")]
        if let Some(pool) = self.handles.pg_pool.as_ref() {
            let client = pool.get().await.map_err(source_open_error)?;
            let rows = match client
                .query(
                    "SELECT user_id, path, content FROM memory_documents
                     WHERE path LIKE 'projects/%/.project.json'
                        OR path LIKE '.system/engine/projects/%/project.json'
                        OR path LIKE 'engine/projects/%/project.json'
                     ORDER BY path, user_id, COALESCE(agent_id::text, '')",
                    &[],
                )
                .await
            {
                Ok(rows) => rows,
                Err(error) if is_missing_table_error(&error.to_string()) => return Ok(Vec::new()),
                Err(error) => return Err(source_read_error("projects", error)),
            };
            return rows
                .into_iter()
                .map(|row| {
                    Ok(ProjectDocument {
                        user_id: row
                            .try_get(0)
                            .map_err(|error| source_read_error("projects", error))?,
                        path: row
                            .try_get(1)
                            .map_err(|error| source_read_error("projects", error))?,
                        content: row
                            .try_get(2)
                            .map_err(|error| source_read_error("projects", error))?,
                    })
                })
                .collect();
        }

        Ok(Vec::new())
    }

    /// `SELECT DISTINCT <column> FROM <table>` against the raw handle. `column`
    /// is the user-id column, which is `user_id` on data tables but `id` on the
    /// `users` table.
    ///
    /// A **missing table** is tolerated (returns an empty vec) — minimal v1
    /// installs legitimately lack some tables (e.g. libSQL without `settings`),
    /// and "table absent" means "no users here". Every *other* failure —
    /// connect, query, or row decode — is a real infrastructure error and
    /// propagates, so a transient pool/permission/connection fault can never be
    /// silently mistaken for "0 users" and drop everything keyed to them.
    ///
    /// `table`/`column` are always internal constants, never user input.
    pub(crate) async fn distinct_user_ids_in(
        &self,
        table: &str,
        column: &str,
    ) -> Result<Vec<String>, MigrationError> {
        let read_err = |e: &dyn std::fmt::Display| MigrationError::ReadSource {
            domain: table.to_string(),
            reason: e.to_string(),
        };
        let sql = format!("SELECT DISTINCT {column} FROM {table}");
        #[cfg(feature = "libsql")]
        if let Some(db) = self.handles.libsql_db.as_ref() {
            let conn = db.connect().map_err(|e| read_err(&e))?;
            let mut rows = match conn.query(&sql, ()).await {
                Ok(rows) => rows,
                Err(e) if is_missing_table_error(&e.to_string()) => return Ok(Vec::new()),
                Err(e) => return Err(read_err(&e)),
            };
            let mut out = Vec::new();
            while let Some(row) = rows.next().await.map_err(|e| read_err(&e))? {
                out.push(row.get::<String>(0).map_err(|e| read_err(&e))?);
            }
            return Ok(out);
        }
        #[cfg(feature = "postgres")]
        if let Some(pool) = self.handles.pg_pool.as_ref() {
            let client = pool.get().await.map_err(|e| read_err(&e))?;
            let stmt_rows = match client.query(sql.as_str(), &[]).await {
                Ok(rows) => rows,
                Err(e) if is_missing_table_error(&e.to_string()) => return Ok(Vec::new()),
                Err(e) => return Err(read_err(&e)),
            };
            return stmt_rows
                .iter()
                .map(|row| row.try_get::<_, String>(0).map_err(|e| read_err(&e)))
                .collect();
        }
        Ok(Vec::new())
    }
}

/// True when a DB error string denotes an absent table/relation, the one case
/// [`V1Source::distinct_user_ids_in`] tolerates. Covers SQLite/libSQL
/// (`no such table`) and PostgreSQL (`relation "…" does not exist`).
///
/// Deliberately narrow: a bare `does not exist` also matches PostgreSQL's
/// *column*-not-found message (`column "…" does not exist`), so requiring
/// `relation` keeps a schema drift on a real table from being downgraded to an
/// empty user set — exactly the silent-drop class this converter guards against.
pub(crate) fn is_missing_table_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("no such table")
        || (lower.contains("relation") && lower.contains("does not exist"))
}

#[cfg(feature = "postgres")]
fn source_to_config(source: &SourceDb) -> Result<DatabaseConfig, MigrationError> {
    match source {
        SourceDb::LibSql { path } => Ok(DatabaseConfig {
            backend: DatabaseBackend::LibSql,
            // libSQL backend ignores `url`; the resolver uses this sentinel too.
            url: SecretString::from("unused://libsql"),
            pool_size: 4,
            ssl_mode: SslMode::default(),
            libsql_path: Some(path.clone()),
            libsql_url: None,
            libsql_auth_token: None,
        }),
        SourceDb::Postgres { url } => {
            use secrecy::ExposeSecret as _;

            let parsed = url
                .expose_secret()
                .parse::<tokio_postgres::Config>()
                .map_err(|_| {
                    MigrationError::OpenSource(
                        "invalid PostgreSQL source connection URL (details redacted)".to_string(),
                    )
                })?;
            let remote = !is_local_postgres_config(&parsed);
            let ssl_mode = match parsed.get_ssl_mode() {
                tokio_postgres::config::SslMode::Disable if remote => {
                    return Err(MigrationError::OpenSource(
                        "remote PostgreSQL source requires TLS; sslmode=disable is rejected"
                            .to_string(),
                    ));
                }
                tokio_postgres::config::SslMode::Disable => SslMode::Disable,
                _ if remote => SslMode::Require,
                _ => SslMode::Prefer,
            };
            Ok(DatabaseConfig {
                backend: DatabaseBackend::Postgres,
                url: url.clone(),
                pool_size: 4,
                ssl_mode,
                libsql_path: None,
                libsql_url: None,
                libsql_auth_token: None,
            })
        }
    }
}

#[cfg(feature = "postgres")]
fn is_local_postgres_config(config: &tokio_postgres::Config) -> bool {
    use tokio_postgres::config::Host;

    let hosts = config.get_hosts();
    let hostaddrs = config.get_hostaddrs();
    if hosts.is_empty() && hostaddrs.is_empty() {
        return true;
    }
    for host in hosts {
        match host {
            #[cfg(unix)]
            Host::Unix(_) => continue,
            Host::Tcp(name) => {
                if !matches!(
                    name.as_str(),
                    "localhost" | "127.0.0.1" | "::1" | "[::1]" | "0.0.0.0"
                ) {
                    return false;
                }
            }
        }
    }
    for address in hostaddrs {
        if !address.is_loopback() && !address.is_unspecified() {
            return false;
        }
    }
    true
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(all(feature = "libsql", feature = "postgres"))]
fn handles_with_libsql(db: Arc<libsql::Database>) -> DatabaseHandles {
    DatabaseHandles {
        libsql_db: Some(db),
        pg_pool: None,
    }
}

#[cfg(all(feature = "libsql", not(feature = "postgres")))]
fn handles_with_libsql(db: Arc<libsql::Database>) -> DatabaseHandles {
    DatabaseHandles {
        libsql_db: Some(db),
    }
}

#[cfg(all(feature = "postgres", feature = "libsql"))]
fn handles_with_postgres(pool: deadpool_postgres::Pool) -> DatabaseHandles {
    DatabaseHandles {
        pg_pool: Some(pool),
        libsql_db: None,
    }
}

#[cfg(all(feature = "postgres", not(feature = "libsql")))]
fn handles_with_postgres(pool: deadpool_postgres::Pool) -> DatabaseHandles {
    DatabaseHandles {
        pg_pool: Some(pool),
    }
}

fn source_open_error(error: impl std::fmt::Display) -> MigrationError {
    MigrationError::OpenSource(error.to_string())
}

fn source_read_error(domain: &str, error: impl std::fmt::Display) -> MigrationError {
    MigrationError::ReadSource {
        domain: domain.to_string(),
        reason: error.to_string(),
    }
}

async fn fingerprint_local_snapshot(
    path: &std::path::Path,
) -> Result<SourceFingerprint, MigrationError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut hash = 0xcbf29ce484222325_u64;
        for candidate in [
            path.clone(),
            std::path::PathBuf::from(format!("{}-wal", path.display())),
        ] {
            if !candidate.exists() {
                continue;
            }
            for byte in candidate
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_bytes()
            {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
            let metadata = std::fs::metadata(&candidate)?;
            for byte in metadata.len().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
            if let Ok(modified) = metadata.modified()
                && let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH)
            {
                for byte in since_epoch.as_secs().to_le_bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x100000001b3);
                }
                for byte in since_epoch.subsec_nanos().to_le_bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x100000001b3);
                }
            }
        }
        Ok(SourceFingerprint {
            algorithm: "fnv1a64-file-metadata-set-v1".to_string(),
            value: format!("{hash:016x}"),
        })
    })
    .await
    .map_err(|error| MigrationError::OpenSource(format!("snapshot fingerprint task: {error}")))?
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use secrecy::SecretString;

    use super::source_to_config;
    use crate::options::SourceDb;

    #[test]
    fn remote_postgres_source_rejects_disabled_tls() {
        let source = SourceDb::Postgres {
            url: SecretString::from(
                "postgresql://user:password@database.example/ironclaw?sslmode=disable",
            ),
        };
        let error = source_to_config(&source).expect_err("remote plaintext source must fail");
        let rendered = error.to_string();
        assert!(rendered.contains("requires TLS"));
        assert!(!rendered.contains("password"));
        assert!(!rendered.contains("database.example"));
    }

    #[test]
    fn local_postgres_source_can_explicitly_disable_tls() {
        let source = SourceDb::Postgres {
            url: SecretString::from(
                "postgresql://user:password@localhost/ironclaw?sslmode=disable",
            ),
        };
        let config = source_to_config(&source).expect("local plaintext source");
        assert_eq!(config.ssl_mode, ironclaw::config::SslMode::Disable);
    }
}
