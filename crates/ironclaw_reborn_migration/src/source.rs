//! v1 / engine-v2 read side.
//!
//! Opens the legacy database through the frozen [`crate::legacy_snapshot`]
//! read path (independent of `ironclaw_legacy`, see that module's docs) and
//! exposes it as a [`legacy_snapshot::LegacyDb`] plus the backend-specific
//! handles that satellite v1 stores (secrets, wasm tools, identities) need.
//! Engine-v2 mission/project state is not a separate connection — it lives as
//! JSON blobs inside the `memory_documents` table and is read through the
//! same handle (see [`crate::convert::automations`] and [`crate::v2_model`]).

use crate::error::MigrationError;
use crate::legacy_snapshot::{self, LegacyDb, LegacyHandles};
use crate::options::SourceDb;

/// True when a PostgreSQL error is the "table/relation does not exist" class,
/// the one case the read paths tolerate as "nothing here" (see
/// [`is_missing_table_error`] for the string-based libSQL counterpart). Shared
/// across every read site in the crate (source discovery, the frozen legacy
/// queries, the wasm stores, and the identity converter).
pub(crate) fn is_missing_postgres_table_error(error: &tokio_postgres::Error) -> bool {
    error
        .as_db_error()
        .is_some_and(|db| db.code() == &tokio_postgres::error::SqlState::UNDEFINED_TABLE)
}

/// A live handle to the v1 source database.
///
/// Crate-internal: the only public entry point is [`crate::run_migration`], and
/// this handle is consumed exclusively by the in-crate converters (mirrors the
/// symmetric `RebornTarget` visibility).
pub(crate) struct V1Source {
    pub(crate) db: LegacyDb,
    /// Backend-specific handles for satellite v1 stores (secrets) and raw
    /// distinct-user / channel-identity discovery.
    pub(crate) handles: LegacyHandles,
}

/// Tables a v1 user_id can appear in. Queried independently so a DB missing one
/// (e.g. a minimal libSQL install without `settings`) still discovers users
/// from the others.
const USER_ID_TABLES: [&str; 4] = ["conversations", "routines", "memory_documents", "settings"];

impl V1Source {
    pub(crate) async fn open(source: &SourceDb) -> Result<Self, MigrationError> {
        let (db, handles) = legacy_snapshot::connect(source)
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
            for uid in self.distinct_user_ids_in(table, "user_id").await? {
                if !uid.is_empty() {
                    users.insert(uid);
                }
            }
        }
        Ok(users.into_iter().collect())
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
        if let Some(pool) = self.handles.pg_pool.as_ref() {
            let client = pool.get().await.map_err(|e| read_err(&e))?;
            let stmt_rows = match client.query(sql.as_str(), &[]).await {
                Ok(rows) => rows,
                Err(e) if is_missing_postgres_table_error(&e) => return Ok(Vec::new()),
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
