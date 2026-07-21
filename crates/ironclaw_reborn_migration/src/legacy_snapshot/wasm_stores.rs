//! Frozen, read-only port of the v1 installed-tool/channel stores
//! (`src/tools/wasm/storage.rs`, `src/channels/wasm/storage.rs`) — only the
//! `list`/`get_capabilities` surface [`crate::convert::extensions`] calls.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum WasmStorageError {
    #[error("Database error: {0}")]
    Database(String),
}

use crate::source::is_missing_postgres_table_error;
use crate::source::is_missing_table_error;

/// Frozen mirror of `ironclaw::tools::wasm::ToolStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolStatus {
    Active,
    Disabled,
    Quarantined,
}

impl std::str::FromStr for ToolStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(ToolStatus::Active),
            "disabled" => Ok(ToolStatus::Disabled),
            "quarantined" => Ok(ToolStatus::Quarantined),
            _ => Err(format!("Unknown status: {s}")),
        }
    }
}

/// Frozen mirror of `ironclaw::tools::wasm::StoredWasmTool` (metadata fields
/// this crate reads; trust_level/parameters_schema/source_url are parsed for
/// contract fidelity even though `convert::extensions` doesn't consume them).
#[derive(Debug, Clone)]
pub(crate) struct StoredWasmTool {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) description: String,
    pub(crate) status: ToolStatus,
    pub(crate) updated_at: DateTime<Utc>,
}

/// Frozen mirror of `ironclaw::tools::wasm::StoredCapabilities` (only
/// `allowed_secrets` is consumed downstream, the rest exists to document the
/// on-disk contract).
#[derive(Debug, Clone)]
pub(crate) struct StoredCapabilities {
    pub(crate) allowed_secrets: Vec<String>,
}

/// Frozen mirror of `ironclaw::channels::wasm::StoredWasmChannel`.
#[derive(Debug, Clone)]
pub(crate) struct StoredWasmChannel {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) description: String,
    pub(crate) status: String,
    pub(crate) updated_at: DateTime<Utc>,
}

/// Frozen mirror of `ironclaw::tools::wasm::WasmToolStore` — narrowed to the
/// two methods this crate calls (not the full store contract: `store`/`get`/
/// `get_with_binary`/`update_status`/`delete` have no migration use).
#[async_trait]
pub(crate) trait WasmToolStore: Send + Sync {
    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmTool>, WasmStorageError>;
    async fn get_capabilities(
        &self,
        tool_id: Uuid,
    ) -> Result<Option<StoredCapabilities>, WasmStorageError>;
}

/// Frozen mirror of `ironclaw::channels::wasm::WasmChannelStore` — narrowed to
/// `list`.
#[async_trait]
pub(crate) trait WasmChannelStore: Send + Sync {
    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmChannel>, WasmStorageError>;
}

// ============================== libSQL ======================================

pub(crate) struct LibSqlWasmToolStore {
    db: Arc<libsql::Database>,
}

impl LibSqlWasmToolStore {
    pub(crate) fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    async fn connect(&self) -> Result<libsql::Connection, WasmStorageError> {
        let conn = self
            .db
            .connect()
            .map_err(|e| WasmStorageError::Database(format!("Connection failed: {e}")))?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|e| WasmStorageError::Database(format!("Failed to set busy_timeout: {e}")))?;
        Ok(conn)
    }
}

#[async_trait]
impl WasmToolStore for LibSqlWasmToolStore {
    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmTool>, WasmStorageError> {
        use super::libsql_helpers::{get_text, get_ts};
        let conn = self.connect().await?;
        let mut rows = match conn
            .query(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema,
                       source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = ?1
                ORDER BY name
                "#,
                libsql::params![user_id],
            )
            .await
        {
            Ok(rows) => rows,
            Err(e) if is_missing_table_error(&e.to_string()) => return Ok(Vec::new()),
            Err(e) => return Err(WasmStorageError::Database(e.to_string())),
        };

        let mut tools = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            let id_str = get_text(&row, 0);
            let status_str = get_text(&row, 8);
            tools.push(StoredWasmTool {
                id: id_str
                    .parse()
                    .map_err(|e: uuid::Error| WasmStorageError::Database(e.to_string()))?,
                name: get_text(&row, 2),
                version: get_text(&row, 3),
                description: get_text(&row, 4),
                status: status_str.parse().map_err(WasmStorageError::Database)?,
                updated_at: get_ts(&row, 10),
            });
        }
        Ok(tools)
    }

    async fn get_capabilities(
        &self,
        tool_id: Uuid,
    ) -> Result<Option<StoredCapabilities>, WasmStorageError> {
        let conn = self.connect().await?;
        let mut rows = match conn
            .query(
                r#"
                SELECT allowed_secrets
                FROM tool_capabilities
                WHERE wasm_tool_id = ?1
                "#,
                libsql::params![tool_id.to_string()],
            )
            .await
        {
            Ok(rows) => rows,
            Err(e) if is_missing_table_error(&e.to_string()) => return Ok(None),
            Err(e) => return Err(WasmStorageError::Database(e.to_string())),
        };

        match rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            Some(row) => {
                let allowed_secrets_str: String = row.get::<String>(0).unwrap_or_default();
                let allowed_secrets: Vec<String> =
                    serde_json::from_str(&allowed_secrets_str).unwrap_or_default();
                Ok(Some(StoredCapabilities { allowed_secrets }))
            }
            None => Ok(None),
        }
    }
}

pub(crate) struct LibSqlWasmChannelStore {
    db: Arc<libsql::Database>,
}

impl LibSqlWasmChannelStore {
    pub(crate) fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    async fn connect(&self) -> Result<libsql::Connection, WasmStorageError> {
        let conn = self
            .db
            .connect()
            .map_err(|e| WasmStorageError::Database(format!("Connection failed: {e}")))?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|e| WasmStorageError::Database(format!("Failed to set busy_timeout: {e}")))?;
        Ok(conn)
    }
}

#[async_trait]
impl WasmChannelStore for LibSqlWasmChannelStore {
    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmChannel>, WasmStorageError> {
        use super::libsql_helpers::{get_text, parse_timestamp};
        let conn = self.connect().await?;
        let mut rows = match conn
            .query(
                r#"
                SELECT id, user_id, name, version, description,
                       capabilities_json, status, created_at, updated_at
                FROM wasm_channels
                WHERE user_id = ?1
                ORDER BY name
                "#,
                libsql::params![user_id],
            )
            .await
        {
            Ok(rows) => rows,
            Err(e) if is_missing_table_error(&e.to_string()) => return Ok(Vec::new()),
            Err(e) => return Err(WasmStorageError::Database(e.to_string())),
        };

        let mut channels = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            let updated_at_str = get_text(&row, 8);
            channels.push(StoredWasmChannel {
                name: get_text(&row, 2),
                version: get_text(&row, 3),
                description: get_text(&row, 4),
                status: get_text(&row, 6),
                updated_at: parse_timestamp(&updated_at_str).map_err(WasmStorageError::Database)?,
            });
        }
        Ok(channels)
    }
}

// ============================== PostgreSQL ==================================

pub(crate) struct PostgresWasmToolStore {
    pool: deadpool_postgres::Pool,
}

impl PostgresWasmToolStore {
    pub(crate) fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WasmToolStore for PostgresWasmToolStore {
    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmTool>, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;
        let rows = match client
            .query(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema, source_url,
                       trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = $1
                ORDER BY name
                "#,
                &[&user_id],
            )
            .await
        {
            Ok(rows) => rows,
            Err(e) if is_missing_postgres_table_error(&e) => return Ok(Vec::new()),
            Err(e) => return Err(WasmStorageError::Database(e.to_string())),
        };

        rows.iter()
            .map(|r| {
                let status_str: String = r.get("status");
                Ok(StoredWasmTool {
                    id: r.get("id"),
                    name: r.get("name"),
                    version: r.get("version"),
                    description: r.get("description"),
                    status: status_str.parse().map_err(WasmStorageError::Database)?,
                    updated_at: r.get("updated_at"),
                })
            })
            .collect()
    }

    async fn get_capabilities(
        &self,
        tool_id: Uuid,
    ) -> Result<Option<StoredCapabilities>, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;
        let row = match client
            .query_opt(
                "SELECT allowed_secrets FROM tool_capabilities WHERE wasm_tool_id = $1",
                &[&tool_id],
            )
            .await
        {
            Ok(row) => row,
            Err(e) if is_missing_postgres_table_error(&e) => return Ok(None),
            Err(e) => return Err(WasmStorageError::Database(e.to_string())),
        };
        Ok(row.map(|r| StoredCapabilities {
            allowed_secrets: r.get("allowed_secrets"),
        }))
    }
}

pub(crate) struct PostgresWasmChannelStore {
    pool: deadpool_postgres::Pool,
}

impl PostgresWasmChannelStore {
    pub(crate) fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WasmChannelStore for PostgresWasmChannelStore {
    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmChannel>, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;
        let rows = match client
            .query(
                r#"
                SELECT id, user_id, name, version, description, capabilities_json, status,
                       created_at, updated_at
                FROM wasm_channels
                WHERE user_id = $1
                ORDER BY name
                "#,
                &[&user_id],
            )
            .await
        {
            Ok(rows) => rows,
            Err(e) if is_missing_postgres_table_error(&e) => return Ok(Vec::new()),
            Err(e) => return Err(WasmStorageError::Database(e.to_string())),
        };

        Ok(rows
            .iter()
            .map(|r| StoredWasmChannel {
                name: r.get("name"),
                version: r.get("version"),
                description: r.get("description"),
                status: r.get("status"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }
}
