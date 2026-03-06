//! WASM channel binary storage with integrity verification.
//!
//! Stores compiled WASM channels in the database with BLAKE3 hash verification.
//! Mirrors the pattern in `crate::tools::wasm::storage` but without capabilities table.
//!
//! # Storage Flow
//!
//! ```text
//! WASM bytes ──► BLAKE3 hash ──► Store in database
//!                    │               (binary + hash)
//!                    │
//!                    └──► Later: Load ──► Verify hash ──► Return bytes
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use deadpool_postgres::Pool;
use uuid::Uuid;

use crate::tools::wasm::storage::{compute_binary_hash, verify_binary_integrity};

/// A stored WASM channel (metadata only, no binary).
#[derive(Debug, Clone)]
pub struct StoredWasmChannel {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub version: String,
    pub wit_version: String,
    pub description: String,
    pub capabilities_json: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full channel data including binary.
#[derive(Debug)]
pub struct StoredWasmChannelWithBinary {
    pub tool: StoredWasmChannel,
    pub wasm_binary: Vec<u8>,
    pub binary_hash: Vec<u8>,
}

/// Parameters for storing a new WASM channel.
pub struct StoreChannelParams {
    pub user_id: String,
    pub name: String,
    pub version: String,
    pub wit_version: String,
    pub description: String,
    pub wasm_binary: Vec<u8>,
    pub capabilities_json: String,
}

/// Error from WASM channel storage operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum WasmChannelStoreError {
    #[error("Channel not found: {0}")]
    NotFound(String),

    #[error("Binary integrity check failed: hash mismatch")]
    IntegrityCheckFailed,

    #[error("Database error: {0}")]
    Database(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

/// Trait for WASM channel storage.
#[async_trait]
pub trait WasmChannelStore: Send + Sync {
    /// Store a new WASM channel.
    async fn store(
        &self,
        params: StoreChannelParams,
    ) -> Result<StoredWasmChannel, WasmChannelStoreError>;

    /// Get channel metadata (without binary).
    async fn get(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmChannel, WasmChannelStoreError>;

    /// Get channel with binary (verifies integrity).
    async fn get_with_binary(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmChannelWithBinary, WasmChannelStoreError>;

    /// List all channels for a user.
    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmChannel>, WasmChannelStoreError>;

    /// Delete a channel.
    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, WasmChannelStoreError>;
}

// ==================== PostgreSQL implementation ====================

/// PostgreSQL implementation of WasmChannelStore.
#[cfg(feature = "postgres")]
pub struct PostgresWasmChannelStore {
    pool: Pool,
}

#[cfg(feature = "postgres")]
impl PostgresWasmChannelStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl WasmChannelStore for PostgresWasmChannelStore {
    async fn store(
        &self,
        params: StoreChannelParams,
    ) -> Result<StoredWasmChannel, WasmChannelStoreError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        let binary_hash = compute_binary_hash(&params.wasm_binary);
        let id = Uuid::new_v4();
        let now = Utc::now();

        // Delete any existing version for this (user_id, name) — upgrade-in-place
        client
            .execute(
                "DELETE FROM wasm_channels WHERE user_id = $1 AND name = $2",
                &[&params.user_id, &params.name],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        let row = client
            .query_one(
                r#"
                INSERT INTO wasm_channels (
                    id, user_id, name, version, wit_version, description, wasm_binary, binary_hash,
                    capabilities_json, status, created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active', $10, $10)
                RETURNING id, user_id, name, version, wit_version, description,
                          capabilities_json, status, created_at, updated_at
                "#,
                &[
                    &id,
                    &params.user_id,
                    &params.name,
                    &params.version,
                    &params.wit_version,
                    &params.description,
                    &params.wasm_binary,
                    &binary_hash,
                    &params.capabilities_json,
                    &now,
                ],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        pg_row_to_channel(&row)
    }

    async fn get(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmChannel, WasmChannelStoreError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                r#"
                SELECT id, user_id, name, version, wit_version, description,
                       capabilities_json, status, created_at, updated_at
                FROM wasm_channels
                WHERE user_id = $1 AND name = $2
                "#,
                &[&user_id, &name],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        match row {
            Some(r) => pg_row_to_channel(&r),
            None => Err(WasmChannelStoreError::NotFound(name.to_string())),
        }
    }

    async fn get_with_binary(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmChannelWithBinary, WasmChannelStoreError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                r#"
                SELECT id, user_id, name, version, wit_version, description,
                       wasm_binary, binary_hash,
                       capabilities_json, status, created_at, updated_at
                FROM wasm_channels
                WHERE user_id = $1 AND name = $2
                "#,
                &[&user_id, &name],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        match row {
            Some(r) => {
                let wasm_binary: Vec<u8> = r.get("wasm_binary");
                let binary_hash: Vec<u8> = r.get("binary_hash");

                if !verify_binary_integrity(&wasm_binary, &binary_hash) {
                    tracing::error!(
                        user_id = user_id,
                        name = name,
                        "WASM channel binary integrity check failed"
                    );
                    return Err(WasmChannelStoreError::IntegrityCheckFailed);
                }

                let channel = StoredWasmChannel {
                    id: r.get("id"),
                    user_id: r.get("user_id"),
                    name: r.get("name"),
                    version: r.get("version"),
                    wit_version: r.get("wit_version"),
                    description: r.get("description"),
                    capabilities_json: r.get("capabilities_json"),
                    status: r.get("status"),
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                };

                Ok(StoredWasmChannelWithBinary {
                    tool: channel,
                    wasm_binary,
                    binary_hash,
                })
            }
            None => Err(WasmChannelStoreError::NotFound(name.to_string())),
        }
    }

    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmChannel>, WasmChannelStoreError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        let rows = client
            .query(
                r#"
                SELECT id, user_id, name, version, wit_version, description,
                       capabilities_json, status, created_at, updated_at
                FROM wasm_channels
                WHERE user_id = $1
                ORDER BY name
                "#,
                &[&user_id],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        rows.into_iter().map(|r| pg_row_to_channel(&r)).collect()
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, WasmChannelStoreError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        let result = client
            .execute(
                "DELETE FROM wasm_channels WHERE user_id = $1 AND name = $2",
                &[&user_id, &name],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        Ok(result > 0)
    }
}

#[cfg(feature = "postgres")]
fn pg_row_to_channel(
    row: &tokio_postgres::Row,
) -> Result<StoredWasmChannel, WasmChannelStoreError> {
    Ok(StoredWasmChannel {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        version: row.get("version"),
        wit_version: row.get("wit_version"),
        description: row.get("description"),
        capabilities_json: row.get("capabilities_json"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// ==================== libSQL implementation ====================

/// libSQL/Turso implementation of WasmChannelStore.
///
/// Holds an `Arc<Database>` handle and creates a fresh connection per operation,
/// matching the connection-per-request pattern used by the main `LibSqlBackend`.
#[cfg(feature = "libsql")]
pub struct LibSqlWasmChannelStore {
    db: std::sync::Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlWasmChannelStore {
    pub fn new(db: std::sync::Arc<libsql::Database>) -> Self {
        Self { db }
    }

    async fn connect(&self) -> Result<libsql::Connection, WasmChannelStoreError> {
        let conn = self
            .db
            .connect()
            .map_err(|e| WasmChannelStoreError::Database(format!("Connection failed: {}", e)))?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|e| {
                WasmChannelStoreError::Database(format!("Failed to set busy_timeout: {}", e))
            })?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl WasmChannelStore for LibSqlWasmChannelStore {
    async fn store(
        &self,
        params: StoreChannelParams,
    ) -> Result<StoredWasmChannel, WasmChannelStoreError> {
        let binary_hash = compute_binary_hash(&params.wasm_binary);
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let conn = self.connect().await?;
        let tx = conn
            .transaction()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        // Delete any existing version for this (user_id, name) — upgrade-in-place
        tx.execute(
            "DELETE FROM wasm_channels WHERE user_id = ?1 AND name = ?2",
            libsql::params![params.user_id.as_str(), params.name.as_str()],
        )
        .await
        .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        tx.execute(
            r#"
                INSERT INTO wasm_channels (
                    id, user_id, name, version, wit_version, description, wasm_binary, binary_hash,
                    capabilities_json, status, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?10)
                "#,
            libsql::params![
                id.to_string(),
                params.user_id.as_str(),
                params.name.as_str(),
                params.version.as_str(),
                params.wit_version.as_str(),
                params.description.as_str(),
                libsql::Value::Blob(params.wasm_binary),
                libsql::Value::Blob(binary_hash),
                params.capabilities_json.as_str(),
                now.as_str(),
            ],
        )
        .await
        .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        // Read back the row within the same transaction
        let mut rows = tx
            .query(
                r#"
                SELECT id, user_id, name, version, wit_version, description,
                       capabilities_json, status, created_at, updated_at
                FROM wasm_channels
                WHERE user_id = ?1 AND name = ?2
                "#,
                libsql::params![params.user_id.as_str(), params.name.as_str()],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        let row = rows
            .next()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?
            .ok_or_else(|| {
                WasmChannelStoreError::Database("Insert succeeded but row not found".into())
            })?;

        let channel = libsql_row_to_channel(&row)?;

        tx.commit()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        Ok(channel)
    }

    async fn get(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmChannel, WasmChannelStoreError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, version, wit_version, description,
                       capabilities_json, status, created_at, updated_at
                FROM wasm_channels
                WHERE user_id = ?1 AND name = ?2
                "#,
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?
        {
            Some(row) => libsql_row_to_channel(&row),
            None => Err(WasmChannelStoreError::NotFound(name.to_string())),
        }
    }

    async fn get_with_binary(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmChannelWithBinary, WasmChannelStoreError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, version, wit_version, description,
                       wasm_binary, binary_hash,
                       capabilities_json, status, created_at, updated_at
                FROM wasm_channels
                WHERE user_id = ?1 AND name = ?2
                "#,
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?
        {
            Some(row) => {
                let wasm_binary: Vec<u8> = row
                    .get(6)
                    .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;
                let binary_hash: Vec<u8> = row
                    .get(7)
                    .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

                if !verify_binary_integrity(&wasm_binary, &binary_hash) {
                    tracing::error!(
                        user_id = user_id,
                        name = name,
                        "WASM channel binary integrity check failed"
                    );
                    return Err(WasmChannelStoreError::IntegrityCheckFailed);
                }

                let channel = libsql_row_to_channel_with_offset(&row)?;

                Ok(StoredWasmChannelWithBinary {
                    tool: channel,
                    wasm_binary,
                    binary_hash,
                })
            }
            None => Err(WasmChannelStoreError::NotFound(name.to_string())),
        }
    }

    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmChannel>, WasmChannelStoreError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, version, wit_version, description,
                       capabilities_json, status, created_at, updated_at
                FROM wasm_channels
                WHERE user_id = ?1
                ORDER BY name
                "#,
                libsql::params![user_id],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        let mut channels = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?
        {
            channels.push(libsql_row_to_channel(&row)?);
        }
        Ok(channels)
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, WasmChannelStoreError> {
        let conn = self.connect().await?;
        let result = conn
            .execute(
                "DELETE FROM wasm_channels WHERE user_id = ?1 AND name = ?2",
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

        Ok(result > 0)
    }
}

#[cfg(feature = "libsql")]
#[allow(dead_code)]
fn libsql_channel_opt_text(s: Option<&str>) -> libsql::Value {
    match s {
        Some(s) => libsql::Value::Text(s.to_string()),
        None => libsql::Value::Null,
    }
}

#[cfg(feature = "libsql")]
fn libsql_channel_parse_ts(s: &str) -> Result<DateTime<Utc>, WasmChannelStoreError> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(ndt.and_utc());
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    Err(WasmChannelStoreError::InvalidData(format!(
        "unparseable timestamp: {:?}",
        s
    )))
}

/// Parse a channel row with standard column order (no binary columns).
/// Columns: id(0), user_id(1), name(2), version(3), wit_version(4), description(5),
///          capabilities_json(6), status(7), created_at(8), updated_at(9)
#[cfg(feature = "libsql")]
fn libsql_row_to_channel(row: &libsql::Row) -> Result<StoredWasmChannel, WasmChannelStoreError> {
    let id_str: String = row
        .get(0)
        .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;
    let created_at_str: String = row
        .get(8)
        .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;
    let updated_at_str: String = row
        .get(9)
        .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

    Ok(StoredWasmChannel {
        id: id_str
            .parse()
            .map_err(|e: uuid::Error| WasmChannelStoreError::InvalidData(e.to_string()))?,
        user_id: row
            .get(1)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        name: row
            .get(2)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        version: row
            .get(3)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        wit_version: row
            .get(4)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        description: row
            .get(5)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        capabilities_json: row
            .get(6)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        status: row
            .get(7)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        created_at: libsql_channel_parse_ts(&created_at_str)?,
        updated_at: libsql_channel_parse_ts(&updated_at_str)?,
    })
}

/// Parse a channel row when binary columns are present (get_with_binary query).
/// Columns: id(0), user_id(1), name(2), version(3), wit_version(4), description(5),
///          wasm_binary(6), binary_hash(7),
///          capabilities_json(8), status(9), created_at(10), updated_at(11)
#[cfg(feature = "libsql")]
fn libsql_row_to_channel_with_offset(
    row: &libsql::Row,
) -> Result<StoredWasmChannel, WasmChannelStoreError> {
    let id_str: String = row
        .get(0)
        .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;
    let created_at_str: String = row
        .get(10)
        .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;
    let updated_at_str: String = row
        .get(11)
        .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?;

    Ok(StoredWasmChannel {
        id: id_str
            .parse()
            .map_err(|e: uuid::Error| WasmChannelStoreError::InvalidData(e.to_string()))?,
        user_id: row
            .get(1)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        name: row
            .get(2)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        version: row
            .get(3)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        wit_version: row
            .get(4)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        description: row
            .get(5)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        capabilities_json: row
            .get(8)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        status: row
            .get(9)
            .map_err(|e| WasmChannelStoreError::Database(e.to_string()))?,
        created_at: libsql_channel_parse_ts(&created_at_str)?,
        updated_at: libsql_channel_parse_ts(&updated_at_str)?,
    })
}
