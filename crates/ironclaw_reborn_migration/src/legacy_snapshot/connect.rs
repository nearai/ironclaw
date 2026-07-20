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
//! tool runs) and [`ensure_schema_current`] fails loud with a specific
//! missing-column error rather than silently reading a partial row.

#[cfg(feature = "libsql")]
use std::path::Path;
#[cfg(feature = "libsql")]
use std::sync::Arc;

#[cfg(feature = "postgres")]
use secrecy::{ExposeSecret, SecretString};

use super::error::LegacyError;
use crate::options::SourceDb;

/// The connected v1 database, dispatched on backend. Holds only what the 7
/// queries in [`super::queries`] need — not the full v1 `Database` trait
/// surface.
pub(crate) enum LegacyDb {
    #[cfg(feature = "libsql")]
    LibSql(Arc<libsql::Database>),
    #[cfg(feature = "postgres")]
    Postgres(deadpool_postgres::Pool),
}

/// Backend-specific handles for satellite v1 readers (secrets, wasm tool/
/// channel stores) that need their own connections rather than going through
/// [`LegacyDb`]. Same shape as the original `ironclaw::db::DatabaseHandles`
/// so call sites built around it (`convert::secrets`, `convert::extensions`,
/// `convert::identities`) needed no changes beyond their imports.
#[derive(Default, Clone)]
pub(crate) struct LegacyHandles {
    #[cfg(feature = "postgres")]
    pub(crate) pg_pool: Option<deadpool_postgres::Pool>,
    #[cfg(feature = "libsql")]
    pub(crate) libsql_db: Option<Arc<libsql::Database>>,
}

impl LegacyHandles {
    #[cfg(feature = "libsql")]
    fn from_libsql(db: Arc<libsql::Database>) -> Self {
        Self {
            libsql_db: Some(db),
            #[cfg(feature = "postgres")]
            pg_pool: None,
        }
    }

    #[cfg(feature = "postgres")]
    fn from_postgres(pool: deadpool_postgres::Pool) -> Self {
        Self {
            pg_pool: Some(pool),
            #[cfg(feature = "libsql")]
            libsql_db: None,
        }
    }
}

pub(crate) async fn connect(source: &SourceDb) -> Result<(LegacyDb, LegacyHandles), LegacyError> {
    match source {
        #[cfg(feature = "libsql")]
        SourceDb::LibSql { path } => {
            let db = Arc::new(open_libsql(path).await?);
            let legacy_db = LegacyDb::LibSql(Arc::clone(&db));
            let handles = LegacyHandles::from_libsql(db);
            ensure_schema_current(&legacy_db).await?;
            Ok((legacy_db, handles))
        }
        #[cfg(not(feature = "libsql"))]
        SourceDb::LibSql { .. } => Err(LegacyError::Connect(
            "libsql feature not enabled in this build".to_string(),
        )),
        #[cfg(feature = "postgres")]
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
        #[cfg(not(feature = "postgres"))]
        SourceDb::Postgres { .. } => Err(LegacyError::Connect(
            "postgres feature not enabled in this build".to_string(),
        )),
    }
}

#[cfg(feature = "libsql")]
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

/// Build a rustls-based TLS connector, frozen from `src/db/tls.rs`. Tries the
/// platform's native certificate store first, falling back to bundled
/// Mozilla roots (`webpki-roots`) when the system store yields none (minimal
/// container images without `ca-certificates`). No certificate-verification
/// override — this must stay at least as strict as the original.
#[cfg(feature = "postgres")]
fn make_rustls_connector() -> Result<tokio_postgres_rustls::MakeRustlsConnect, rustls::Error> {
    let mut root_store = rustls::RootCertStore::empty();

    let native = rustls_native_certs::load_native_certs();
    for e in &native.errors {
        tracing::warn!("error loading system root certs: {e}");
    }
    for cert in native.certs {
        if let Err(e) = root_store.add(cert) {
            tracing::warn!("skipping invalid system root cert: {e}");
        }
    }

    if root_store.is_empty() {
        tracing::info!("no system root certificates found, using bundled Mozilla roots");
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }

    let config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()?
    .with_root_certificates(root_store)
    .with_no_client_auth();
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

/// Open a Postgres pool. Frozen from `Store::new` + `src/db/tls::create_pool`
/// — always uses a TLS-capable connector (mirroring the original's `Prefer`/
/// `Require` path); this migration tool is an operator-run, short-lived
/// process against a database an operator supplies, so there is no
/// `DATABASE_SSLMODE=disable` local-dev case to preserve here the way the
/// long-running `ironclaw` service has.
#[cfg(feature = "postgres")]
fn open_postgres(url: &SecretString) -> Result<deadpool_postgres::Pool, LegacyError> {
    let mut cfg = deadpool_postgres::Config::new();
    cfg.url = Some(url.expose_secret().to_string());
    cfg.pool = Some(deadpool_postgres::PoolConfig {
        max_size: 4,
        ..Default::default()
    });

    let tls = make_rustls_connector().map_err(|e| LegacyError::Connect(e.to_string()))?;
    cfg.create_pool(Some(deadpool_postgres::Runtime::Tokio1), tls)
        .map_err(|e| LegacyError::Connect(e.to_string()))
}

/// Table/column pairs this reader depends on that were added by later v1
/// migrations (rather than present in the original schema) — a good canary
/// for "this source database predates the schema this reader was frozen
/// against". Only `routines` columns are checked: it is the table with the
/// most migrations layered onto it (notify_*, dedup_window_secs, state), so a
/// missing column there is the most likely sign of an out-of-date database.
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
        #[cfg(feature = "libsql")]
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
        #[cfg(feature = "postgres")]
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
