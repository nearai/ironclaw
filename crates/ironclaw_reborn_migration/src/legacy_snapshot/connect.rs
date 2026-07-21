//! Opens the legacy v1 database directly (libSQL or PostgreSQL), independent
//! of `ironclaw_legacy`.
//!
//! Frozen from `ironclaw::db::connect_with_handles` (`src/db/mod.rs`) minus
//! the `Arc<dyn Database>` trait object it built — see [`super::queries`] for
//! why: `Database` is a 9-sub-trait, ~78-method supertrait that cannot be
//! partially implemented, so [`LegacyDb`] is a concrete enum instead.
//!
//! **Does not apply schema migrations.** `connect_with_handles` ran the v1
//! migration suite (refinery for Postgres, the consolidated libSQL schema +
//! incremental migrations) as a side effect of connecting — reproducing that
//! here would mean porting the entire migration-application machinery for a
//! one-time cutover tool. Instead this reader requires the source database to
//! already be at the schema version it was frozen against (realistic: the v1
//! app was in normal use, applying its own migrations, right up until this
//! tool runs). [`ensure_schema_current`] checks this for `routines` — the
//! table with the most migrations layered onto it, so the likeliest to go
//! stale — and fails loud with a specific missing-column error there rather
//! than silently reading a partial row; it is not a general guarantee across
//! every table this reader touches (see [`super::queries`] and
//! [`super::libsql_helpers`] for how the other tables degrade on drift).

use std::path::Path;
use std::sync::Arc;

use secrecy::SecretString;

use super::error::LegacyError;
use crate::options::SourceDb;

/// The connected v1 database, dispatched on backend. Holds only what the 7
/// queries in [`super::queries`] need — not the full v1 `Database` trait
/// surface.
pub(crate) enum LegacyDb {
    LibSql(Arc<libsql::Database>),
    Postgres(deadpool_postgres::Pool),
}

/// Backend-specific handles for satellite v1 readers (secrets, wasm tool/
/// channel stores) that need their own connections rather than going through
/// [`LegacyDb`]. Same shape as the original `ironclaw::db::DatabaseHandles`
/// so call sites built around it (`convert::secrets`, `convert::extensions`,
/// `convert::identities`) needed no changes beyond their imports.
#[derive(Default, Clone)]
pub(crate) struct LegacyHandles {
    pub(crate) pg_pool: Option<deadpool_postgres::Pool>,
    pub(crate) libsql_db: Option<Arc<libsql::Database>>,
}

impl LegacyHandles {
    fn from_libsql(db: Arc<libsql::Database>) -> Self {
        Self {
            libsql_db: Some(db),
            pg_pool: None,
        }
    }

    fn from_postgres(pool: deadpool_postgres::Pool) -> Self {
        Self {
            pg_pool: Some(pool),
            libsql_db: None,
        }
    }
}

pub(crate) async fn connect(source: &SourceDb) -> Result<(LegacyDb, LegacyHandles), LegacyError> {
    match source {
        SourceDb::LibSql { path } => {
            let db = Arc::new(open_libsql(path).await?);
            let legacy_db = LegacyDb::LibSql(Arc::clone(&db));
            let handles = LegacyHandles::from_libsql(db);
            ensure_schema_current(&legacy_db).await?;
            Ok((legacy_db, handles))
        }
        SourceDb::Postgres { url } => {
            let pool = open_postgres(url)?;
            // Cheap connectivity smoke test, mirroring `Store::new`.
            let _ = pool
                .get()
                .await
                .map_err(|e| LegacyError::Connect(e.to_string()))?;
            let legacy_db = LegacyDb::Postgres(pool.clone());
            let handles = LegacyHandles::from_postgres(pool);
            ensure_schema_current(&legacy_db).await?;
            Ok((legacy_db, handles))
        }
    }
}

async fn open_libsql(path: &Path) -> Result<libsql::Database, LegacyError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            LegacyError::Connect(format!("failed to create database directory: {e}"))
        })?;
    }
    libsql::Builder::new_local(path)
        .build()
        .await
        .map_err(|e| LegacyError::Connect(format!("failed to open libSQL database: {e}")))
}

/// Open a Postgres pool via the canonical Reborn helper
/// ([`ironclaw_reborn_composition::open_reborn_postgres_pool_with_max_size`]),
/// the same one [`crate::target::open_postgres_pool`] uses for the write
/// side — so the source connection inherits the same fail-closed remote-TLS
/// policy (reject `sslmode=disable` on any non-local host, upgrade `Prefer`
/// to `Require`) without this crate hand-rolling a second TLS connector.
fn open_postgres(url: &SecretString) -> Result<deadpool_postgres::Pool, LegacyError> {
    ironclaw_reborn_composition::open_reborn_postgres_pool_with_max_size(url.clone(), 4)
        .map_err(|e| LegacyError::Connect(e.to_string()))
}

/// Columns this reader depends on that were added by later v1 migrations
/// (rather than present in the original schema) — checked only for
/// `routines`, the table with the most migrations layered onto it (notify_*,
/// dedup_window_secs, state), so a missing column there is the most likely
/// sign of an out-of-date database. This is not exhaustive: it is the one
/// canary this reader checks, not a guarantee that every table/column this
/// crate reads is present (`postgres_row_to_routine` in [`super::queries`]
/// reads exactly these 24 columns by name, so keep the two lists in sync).
/// The check is skipped entirely if `routines` itself doesn't exist — a
/// minimal v1 install legitimately has no routines table, and that is a
/// normal empty-result case elsewhere in this crate, not a schema mismatch.
async fn ensure_schema_current(db: &LegacyDb) -> Result<(), LegacyError> {
    let expected_columns = [
        "id",
        "name",
        "description",
        "user_id",
        "enabled",
        "trigger_type",
        "trigger_config",
        "action_type",
        "action_config",
        "cooldown_secs",
        "max_concurrent",
        "dedup_window_secs",
        "notify_channel",
        "notify_user",
        "notify_on_success",
        "notify_on_failure",
        "notify_on_attention",
        "state",
        "last_run_at",
        "next_fire_at",
        "run_count",
        "consecutive_failures",
        "created_at",
        "updated_at",
    ];

    let existing = routines_columns(db).await?;
    let Some(existing) = existing else {
        // No `routines` table at all — nothing to check, nothing to migrate.
        return Ok(());
    };

    for column in expected_columns {
        if !existing.iter().any(|c| c == column) {
            return Err(LegacyError::SchemaMismatch {
                table: "routines".to_string(),
                column: column.to_string(),
            });
        }
    }
    Ok(())
}

/// Column names present on the `routines` table, or `None` if the table
/// doesn't exist.
async fn routines_columns(db: &LegacyDb) -> Result<Option<Vec<String>>, LegacyError> {
    match db {
        LegacyDb::LibSql(handle) => {
            let conn = handle
                .connect()
                .map_err(|e| LegacyError::Connect(e.to_string()))?;
            let mut rows = conn
                .query("PRAGMA table_info(routines)", ())
                .await
                .map_err(|e| LegacyError::Query(e.to_string()))?;
            let mut columns = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| LegacyError::Query(e.to_string()))?
            {
                // PRAGMA table_info columns: cid(0), name(1), type(2), notnull(3), dflt_value(4), pk(5).
                columns.push(row.get::<String>(1).unwrap_or_default());
            }
            if columns.is_empty() {
                Ok(None)
            } else {
                Ok(Some(columns))
            }
        }
        LegacyDb::Postgres(pool) => {
            let client = pool
                .get()
                .await
                .map_err(|e| LegacyError::Connect(e.to_string()))?;
            let rows = client
                .query(
                    "SELECT column_name FROM information_schema.columns WHERE table_name = 'routines'",
                    &[],
                )
                .await
                .map_err(|e| LegacyError::Query(e.to_string()))?;
            if rows.is_empty() {
                Ok(None)
            } else {
                Ok(Some(rows.iter().map(|r| r.get::<_, String>(0)).collect()))
            }
        }
    }
}
