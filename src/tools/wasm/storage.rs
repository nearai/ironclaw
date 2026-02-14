//! WASM binary storage with integrity verification.
//!
//! Stores compiled WASM tools in PostgreSQL with BLAKE3 hash verification.
//! On load, the hash is verified to detect tampering.
//!
//! # Storage Flow
//!
//! ```text
//! WASM bytes ──► BLAKE3 hash ──► Store in PostgreSQL
//!                    │               (binary + hash)
//!                    │
//!                    └──► Later: Load ──► Verify hash ──► Return bytes
//! ```

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(feature = "postgres")]
use deadpool_postgres::Pool;
use uuid::Uuid;

use crate::tools::wasm::capabilities::{
    Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
    ToolInvokeCapability,
};

/// Trust level for a WASM tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// Built-in system tool (highest trust).
    System,
    /// Audited and verified tool.
    Verified,
    /// User-uploaded tool (untrusted).
    User,
}

impl std::fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrustLevel::System => write!(f, "system"),
            TrustLevel::Verified => write!(f, "verified"),
            TrustLevel::User => write!(f, "user"),
        }
    }
}

impl std::str::FromStr for TrustLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "system" => Ok(TrustLevel::System),
            "verified" => Ok(TrustLevel::Verified),
            "user" => Ok(TrustLevel::User),
            _ => Err(format!("Unknown trust level: {}", s)),
        }
    }
}

/// Status of a WASM tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// Tool is active and can be used.
    Active,
    /// Tool is disabled (manually or due to errors).
    Disabled,
    /// Tool is quarantined (suspected malicious).
    Quarantined,
}

impl std::fmt::Display for ToolStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolStatus::Active => write!(f, "active"),
            ToolStatus::Disabled => write!(f, "disabled"),
            ToolStatus::Quarantined => write!(f, "quarantined"),
        }
    }
}

impl std::str::FromStr for ToolStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(ToolStatus::Active),
            "disabled" => Ok(ToolStatus::Disabled),
            "quarantined" => Ok(ToolStatus::Quarantined),
            _ => Err(format!("Unknown status: {}", s)),
        }
    }
}

/// A stored WASM tool.
#[derive(Debug, Clone)]
pub struct StoredWasmTool {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    pub source_url: Option<String>,
    pub trust_level: TrustLevel,
    pub status: ToolStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full tool data including binary (not returned by default for efficiency).
#[derive(Debug)]
pub struct StoredWasmToolWithBinary {
    pub tool: StoredWasmTool,
    pub wasm_binary: Vec<u8>,
    pub binary_hash: Vec<u8>,
}

/// Capabilities stored in the database.
#[derive(Debug, Clone)]
pub struct StoredCapabilities {
    pub id: Uuid,
    pub wasm_tool_id: Uuid,
    pub http_allowlist: Vec<EndpointPattern>,
    pub allowed_secrets: Vec<String>,
    pub tool_aliases: HashMap<String, String>,
    pub requests_per_minute: u32,
    pub requests_per_hour: u32,
    pub max_request_body_bytes: i64,
    pub max_response_body_bytes: i64,
    pub workspace_read_prefixes: Vec<String>,
    pub http_timeout_secs: i32,
}

impl StoredCapabilities {
    /// Convert to runtime Capabilities struct.
    pub fn to_capabilities(&self) -> Capabilities {
        let mut caps = Capabilities::default();

        // Workspace read
        if !self.workspace_read_prefixes.is_empty() {
            caps = caps.with_workspace_read(self.workspace_read_prefixes.clone());
        }

        // HTTP capability
        if !self.http_allowlist.is_empty() {
            caps.http = Some(HttpCapability {
                allowlist: self.http_allowlist.clone(),
                credentials: HashMap::new(), // Loaded separately
                rate_limit: RateLimitConfig {
                    requests_per_minute: self.requests_per_minute,
                    requests_per_hour: self.requests_per_hour,
                },
                max_request_bytes: self.max_request_body_bytes as usize,
                max_response_bytes: self.max_response_body_bytes as usize,
                timeout: std::time::Duration::from_secs(self.http_timeout_secs as u64),
            });
        }

        // Tool invoke capability
        if !self.tool_aliases.is_empty() {
            caps.tool_invoke = Some(ToolInvokeCapability {
                aliases: self.tool_aliases.clone(),
                rate_limit: RateLimitConfig {
                    requests_per_minute: self.requests_per_minute,
                    requests_per_hour: self.requests_per_hour,
                },
            });
        }

        // Secrets capability
        if !self.allowed_secrets.is_empty() {
            caps.secrets = Some(SecretsCapability {
                allowed_names: self.allowed_secrets.clone(),
            });
        }

        caps
    }
}

/// Error from WASM storage operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum WasmStorageError {
    #[error("Tool not found: {0}")]
    NotFound(String),

    #[error("Tool is disabled")]
    Disabled,

    #[error("Tool is quarantined")]
    Quarantined,

    #[error("Binary integrity check failed: hash mismatch")]
    IntegrityCheckFailed,

    #[error("Database error: {0}")]
    Database(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

/// Trait for WASM tool storage.
#[async_trait]
pub trait WasmToolStore: Send + Sync {
    /// Store a new WASM tool.
    async fn store(&self, params: StoreToolParams) -> Result<StoredWasmTool, WasmStorageError>;

    /// Get tool metadata (without binary).
    async fn get(&self, user_id: &str, name: &str) -> Result<StoredWasmTool, WasmStorageError>;

    /// Get tool with binary (verifies integrity).
    async fn get_with_binary(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmToolWithBinary, WasmStorageError>;

    /// Get tool capabilities.
    async fn get_capabilities(
        &self,
        tool_id: Uuid,
    ) -> Result<Option<StoredCapabilities>, WasmStorageError>;

    /// List all tools for a user.
    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmTool>, WasmStorageError>;

    /// Update tool status.
    async fn update_status(
        &self,
        user_id: &str,
        name: &str,
        status: ToolStatus,
    ) -> Result<(), WasmStorageError>;

    /// Delete a tool.
    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, WasmStorageError>;
}

/// Parameters for storing a new tool.
pub struct StoreToolParams {
    pub user_id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub wasm_binary: Vec<u8>,
    pub parameters_schema: serde_json::Value,
    pub source_url: Option<String>,
    pub trust_level: TrustLevel,
}

/// Compute BLAKE3 hash of WASM binary.
pub fn compute_binary_hash(binary: &[u8]) -> Vec<u8> {
    let hash = blake3::hash(binary);
    hash.as_bytes().to_vec()
}

/// Verify binary integrity against stored hash.
pub fn verify_binary_integrity(binary: &[u8], expected_hash: &[u8]) -> bool {
    let actual_hash = compute_binary_hash(binary);
    actual_hash == expected_hash
}

/// PostgreSQL implementation of WasmToolStore.
#[cfg(feature = "postgres")]
pub struct PostgresWasmToolStore {
    pool: Pool,
}

#[cfg(feature = "postgres")]
impl PostgresWasmToolStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl WasmToolStore for PostgresWasmToolStore {
    async fn store(&self, params: StoreToolParams) -> Result<StoredWasmTool, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let binary_hash = compute_binary_hash(&params.wasm_binary);
        let id = Uuid::new_v4();
        let now = Utc::now();

        let row = client
            .query_one(
                r#"
                INSERT INTO wasm_tools (
                    id, user_id, name, version, description, wasm_binary, binary_hash,
                    parameters_schema, source_url, trust_level, status, created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'active', $11, $11)
                ON CONFLICT (user_id, name, version) DO UPDATE SET
                    description = EXCLUDED.description,
                    wasm_binary = EXCLUDED.wasm_binary,
                    binary_hash = EXCLUDED.binary_hash,
                    parameters_schema = EXCLUDED.parameters_schema,
                    source_url = EXCLUDED.source_url,
                    updated_at = NOW()
                RETURNING id, user_id, name, version, description, parameters_schema,
                          source_url, trust_level, status, created_at, updated_at
                "#,
                &[
                    &id,
                    &params.user_id,
                    &params.name,
                    &params.version,
                    &params.description,
                    &params.wasm_binary,
                    &binary_hash,
                    &params.parameters_schema,
                    &params.source_url,
                    &params.trust_level.to_string(),
                    &now,
                ],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        row_to_tool(&row)
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<StoredWasmTool, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema,
                       source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = $1 AND name = $2 AND status = 'active'
                ORDER BY version DESC
                LIMIT 1
                "#,
                &[&user_id, &name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match row {
            Some(r) => {
                let tool = row_to_tool(&r)?;
                match tool.status {
                    ToolStatus::Active => Ok(tool),
                    ToolStatus::Disabled => Err(WasmStorageError::Disabled),
                    ToolStatus::Quarantined => Err(WasmStorageError::Quarantined),
                }
            }
            None => Err(WasmStorageError::NotFound(name.to_string())),
        }
    }

    async fn get_with_binary(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmToolWithBinary, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                r#"
                SELECT id, user_id, name, version, description, wasm_binary, binary_hash,
                       parameters_schema, source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = $1 AND name = $2 AND status = 'active'
                ORDER BY version DESC
                LIMIT 1
                "#,
                &[&user_id, &name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match row {
            Some(r) => {
                let wasm_binary: Vec<u8> = r.get("wasm_binary");
                let binary_hash: Vec<u8> = r.get("binary_hash");

                // Verify integrity
                if !verify_binary_integrity(&wasm_binary, &binary_hash) {
                    tracing::error!(
                        user_id = user_id,
                        name = name,
                        "WASM binary integrity check failed"
                    );
                    return Err(WasmStorageError::IntegrityCheckFailed);
                }

                let tool = row_to_tool(&r)?;

                match tool.status {
                    ToolStatus::Active => Ok(StoredWasmToolWithBinary {
                        tool,
                        wasm_binary,
                        binary_hash,
                    }),
                    ToolStatus::Disabled => Err(WasmStorageError::Disabled),
                    ToolStatus::Quarantined => Err(WasmStorageError::Quarantined),
                }
            }
            None => Err(WasmStorageError::NotFound(name.to_string())),
        }
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

        let row = client
            .query_opt(
                r#"
                SELECT id, wasm_tool_id, http_allowlist, allowed_secrets, tool_aliases,
                       requests_per_minute, requests_per_hour, max_request_body_bytes,
                       max_response_body_bytes, workspace_read_prefixes, http_timeout_secs
                FROM tool_capabilities
                WHERE wasm_tool_id = $1
                "#,
                &[&tool_id],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match row {
            Some(r) => {
                let http_allowlist_json: serde_json::Value = r.get("http_allowlist");
                let tool_aliases_json: serde_json::Value = r.get("tool_aliases");

                let http_allowlist: Vec<EndpointPattern> =
                    serde_json::from_value(http_allowlist_json).unwrap_or_default();
                let tool_aliases: HashMap<String, String> =
                    serde_json::from_value(tool_aliases_json).unwrap_or_default();

                Ok(Some(StoredCapabilities {
                    id: r.get("id"),
                    wasm_tool_id: r.get("wasm_tool_id"),
                    http_allowlist,
                    allowed_secrets: r.get("allowed_secrets"),
                    tool_aliases,
                    requests_per_minute: r.get::<_, i32>("requests_per_minute") as u32,
                    requests_per_hour: r.get::<_, i32>("requests_per_hour") as u32,
                    max_request_body_bytes: r.get("max_request_body_bytes"),
                    max_response_body_bytes: r.get("max_response_body_bytes"),
                    workspace_read_prefixes: r.get("workspace_read_prefixes"),
                    http_timeout_secs: r.get("http_timeout_secs"),
                }))
            }
            None => Ok(None),
        }
    }

    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmTool>, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let rows = client
            .query(
                r#"
                SELECT DISTINCT ON (name) id, user_id, name, version, description,
                       parameters_schema, source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = $1
                ORDER BY name, version DESC
                "#,
                &[&user_id],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        rows.into_iter().map(|r| row_to_tool(&r)).collect()
    }

    async fn update_status(
        &self,
        user_id: &str,
        name: &str,
        status: ToolStatus,
    ) -> Result<(), WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let result = client
            .execute(
                "UPDATE wasm_tools SET status = $1, updated_at = NOW() WHERE user_id = $2 AND name = $3",
                &[&status.to_string(), &user_id, &name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        if result == 0 {
            return Err(WasmStorageError::NotFound(name.to_string()));
        }

        Ok(())
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, WasmStorageError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let result = client
            .execute(
                "DELETE FROM wasm_tools WHERE user_id = $1 AND name = $2",
                &[&user_id, &name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        Ok(result > 0)
    }
}

#[cfg(feature = "postgres")]
fn row_to_tool(row: &tokio_postgres::Row) -> Result<StoredWasmTool, WasmStorageError> {
    let trust_level_str: String = row.get("trust_level");
    let status_str: String = row.get("status");

    Ok(StoredWasmTool {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        version: row.get("version"),
        description: row.get("description"),
        parameters_schema: row.get("parameters_schema"),
        source_url: row.get("source_url"),
        trust_level: trust_level_str
            .parse()
            .map_err(WasmStorageError::InvalidData)?,
        status: status_str.parse().map_err(WasmStorageError::InvalidData)?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// ==================== libSQL implementation ====================

/// libSQL/Turso implementation of WasmToolStore.
///
/// Holds an `Arc<Database>` handle and creates a fresh connection per operation,
/// matching the connection-per-request pattern used by the main `LibSqlBackend`.
#[cfg(feature = "libsql")]
pub struct LibSqlWasmToolStore {
    db: std::sync::Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlWasmToolStore {
    pub fn new(db: std::sync::Arc<libsql::Database>) -> Self {
        Self { db }
    }

    fn connect(&self) -> Result<libsql::Connection, WasmStorageError> {
        self.db
            .connect()
            .map_err(|e| WasmStorageError::Database(format!("Connection failed: {}", e)))
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl WasmToolStore for LibSqlWasmToolStore {
    async fn store(&self, params: StoreToolParams) -> Result<StoredWasmTool, WasmStorageError> {
        let binary_hash = compute_binary_hash(&params.wasm_binary);
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let schema_str = serde_json::to_string(&params.parameters_schema)
            .map_err(|e| WasmStorageError::InvalidData(e.to_string()))?;

        // Wrap INSERT + read-back in a transaction to prevent TOCTOU races
        let conn = self.connect()?;
        let tx = conn
            .transaction()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        tx.execute(
            r#"
                INSERT INTO wasm_tools (
                    id, user_id, name, version, description, wasm_binary, binary_hash,
                    parameters_schema, source_url, trust_level, status, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, ?11)
                ON CONFLICT (user_id, name, version) DO UPDATE SET
                    description = excluded.description,
                    wasm_binary = excluded.wasm_binary,
                    binary_hash = excluded.binary_hash,
                    parameters_schema = excluded.parameters_schema,
                    source_url = excluded.source_url,
                    updated_at = ?11
                "#,
            libsql::params![
                id.to_string(),
                params.user_id.as_str(),
                params.name.as_str(),
                params.version.as_str(),
                params.description.as_str(),
                libsql::Value::Blob(params.wasm_binary),
                libsql::Value::Blob(binary_hash),
                schema_str.as_str(),
                libsql_wasm_opt_text(params.source_url.as_deref()),
                params.trust_level.to_string(),
                now.as_str(),
            ],
        )
        .await
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        // Read back the row within the same transaction
        let mut rows = tx
            .query(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema,
                       source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = ?1 AND name = ?2
                ORDER BY version DESC
                LIMIT 1
                "#,
                libsql::params![params.user_id.as_str(), params.name.as_str()],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let row = rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
            .ok_or_else(|| {
                WasmStorageError::Database("Insert succeeded but row not found".into())
            })?;

        let tool = libsql_row_to_tool(&row)?;

        tx.commit()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        Ok(tool)
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<StoredWasmTool, WasmStorageError> {
        let conn = self.connect()?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema,
                       source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = ?1 AND name = ?2 AND status = 'active'
                ORDER BY version DESC
                LIMIT 1
                "#,
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            Some(row) => {
                let tool = libsql_row_to_tool(&row)?;
                match tool.status {
                    ToolStatus::Active => Ok(tool),
                    ToolStatus::Disabled => Err(WasmStorageError::Disabled),
                    ToolStatus::Quarantined => Err(WasmStorageError::Quarantined),
                }
            }
            None => Err(WasmStorageError::NotFound(name.to_string())),
        }
    }

    async fn get_with_binary(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<StoredWasmToolWithBinary, WasmStorageError> {
        let conn = self.connect()?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, version, description, wasm_binary, binary_hash,
                       parameters_schema, source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = ?1 AND name = ?2 AND status = 'active'
                ORDER BY version DESC
                LIMIT 1
                "#,
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            Some(row) => {
                let wasm_binary: Vec<u8> = row
                    .get(5)
                    .map_err(|e| WasmStorageError::Database(e.to_string()))?;
                let binary_hash: Vec<u8> = row
                    .get(6)
                    .map_err(|e| WasmStorageError::Database(e.to_string()))?;

                if !verify_binary_integrity(&wasm_binary, &binary_hash) {
                    tracing::error!(
                        user_id = user_id,
                        name = name,
                        "WASM binary integrity check failed"
                    );
                    return Err(WasmStorageError::IntegrityCheckFailed);
                }

                // Parse metadata from the row (different column offsets due to binary/hash)
                let tool = libsql_row_to_tool_with_offset(&row)?;

                match tool.status {
                    ToolStatus::Active => Ok(StoredWasmToolWithBinary {
                        tool,
                        wasm_binary,
                        binary_hash,
                    }),
                    ToolStatus::Disabled => Err(WasmStorageError::Disabled),
                    ToolStatus::Quarantined => Err(WasmStorageError::Quarantined),
                }
            }
            None => Err(WasmStorageError::NotFound(name.to_string())),
        }
    }

    async fn get_capabilities(
        &self,
        tool_id: Uuid,
    ) -> Result<Option<StoredCapabilities>, WasmStorageError> {
        let conn = self.connect()?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, wasm_tool_id, http_allowlist, allowed_secrets, tool_aliases,
                       requests_per_minute, requests_per_hour, max_request_body_bytes,
                       max_response_body_bytes, workspace_read_prefixes, http_timeout_secs
                FROM tool_capabilities
                WHERE wasm_tool_id = ?1
                "#,
                libsql::params![tool_id.to_string()],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            Some(row) => {
                let id_str: String = row
                    .get(0)
                    .map_err(|e| WasmStorageError::Database(e.to_string()))?;
                let tool_id_str: String = row
                    .get(1)
                    .map_err(|e| WasmStorageError::Database(e.to_string()))?;
                let http_allowlist_str: String = row.get::<String>(2).unwrap_or_default();
                let allowed_secrets_str: String = row.get::<String>(3).unwrap_or_default();
                let tool_aliases_str: String = row.get::<String>(4).unwrap_or_default();
                let rpm: i64 = row.get::<i64>(5).unwrap_or(60);
                let rph: i64 = row.get::<i64>(6).unwrap_or(1000);
                let max_req: i64 = row.get::<i64>(7).unwrap_or(1048576);
                let max_resp: i64 = row.get::<i64>(8).unwrap_or(10485760);
                let ws_prefixes_str: String = row.get::<String>(9).unwrap_or_default();
                let timeout: i64 = row.get::<i64>(10).unwrap_or(30);

                let http_allowlist: Vec<EndpointPattern> =
                    serde_json::from_str(&http_allowlist_str).unwrap_or_default();
                let allowed_secrets: Vec<String> =
                    serde_json::from_str(&allowed_secrets_str).unwrap_or_default();
                let tool_aliases: HashMap<String, String> =
                    serde_json::from_str(&tool_aliases_str).unwrap_or_default();
                let workspace_read_prefixes: Vec<String> =
                    serde_json::from_str(&ws_prefixes_str).unwrap_or_default();

                Ok(Some(StoredCapabilities {
                    id: id_str
                        .parse()
                        .map_err(|e: uuid::Error| WasmStorageError::InvalidData(e.to_string()))?,
                    wasm_tool_id: tool_id_str
                        .parse()
                        .map_err(|e: uuid::Error| WasmStorageError::InvalidData(e.to_string()))?,
                    http_allowlist,
                    allowed_secrets,
                    tool_aliases,
                    requests_per_minute: rpm as u32,
                    requests_per_hour: rph as u32,
                    max_request_body_bytes: max_req,
                    max_response_body_bytes: max_resp,
                    workspace_read_prefixes,
                    http_timeout_secs: timeout as i32,
                }))
            }
            None => Ok(None),
        }
    }

    async fn list(&self, user_id: &str) -> Result<Vec<StoredWasmTool>, WasmStorageError> {
        // SQLite doesn't have DISTINCT ON, so we use a subquery to get latest version per name
        let conn = self.connect()?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, version, description, parameters_schema,
                       source_url, trust_level, status, created_at, updated_at
                FROM wasm_tools
                WHERE user_id = ?1
                  AND rowid IN (
                      SELECT MAX(rowid)
                      FROM wasm_tools
                      WHERE user_id = ?1
                      GROUP BY name
                  )
                ORDER BY name
                "#,
                libsql::params![user_id],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        let mut tools = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?
        {
            tools.push(libsql_row_to_tool(&row)?);
        }
        Ok(tools)
    }

    async fn update_status(
        &self,
        user_id: &str,
        name: &str,
        status: ToolStatus,
    ) -> Result<(), WasmStorageError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let conn = self.connect()?;

        let result = conn
            .execute(
                "UPDATE wasm_tools SET status = ?1, updated_at = ?2 WHERE user_id = ?3 AND name = ?4",
                libsql::params![status.to_string(), now.as_str(), user_id, name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        if result == 0 {
            return Err(WasmStorageError::NotFound(name.to_string()));
        }

        Ok(())
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, WasmStorageError> {
        let conn = self.connect()?;
        let result = conn
            .execute(
                "DELETE FROM wasm_tools WHERE user_id = ?1 AND name = ?2",
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| WasmStorageError::Database(e.to_string()))?;

        Ok(result > 0)
    }
}

#[cfg(feature = "libsql")]
fn libsql_wasm_opt_text(s: Option<&str>) -> libsql::Value {
    match s {
        Some(s) => libsql::Value::Text(s.to_string()),
        None => libsql::Value::Null,
    }
}

#[cfg(feature = "libsql")]
fn libsql_wasm_parse_ts(s: &str) -> Result<DateTime<Utc>, WasmStorageError> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(ndt.and_utc());
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    Err(WasmStorageError::InvalidData(format!(
        "unparseable timestamp: {:?}",
        s
    )))
}

/// Parse a tool row with standard column order (no binary columns).
/// Columns: id(0), user_id(1), name(2), version(3), description(4),
///          parameters_schema(5), source_url(6), trust_level(7), status(8),
///          created_at(9), updated_at(10)
#[cfg(feature = "libsql")]
fn libsql_row_to_tool(row: &libsql::Row) -> Result<StoredWasmTool, WasmStorageError> {
    libsql_row_to_tool_at(row, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
}

/// Parse a tool row when binary columns are present (get_with_binary query).
/// Columns: id(0), user_id(1), name(2), version(3), description(4),
///          wasm_binary(5), binary_hash(6),
///          parameters_schema(7), source_url(8), trust_level(9), status(10),
///          created_at(11), updated_at(12)
#[cfg(feature = "libsql")]
fn libsql_row_to_tool_with_offset(row: &libsql::Row) -> Result<StoredWasmTool, WasmStorageError> {
    libsql_row_to_tool_at(row, 0, 1, 2, 3, 4, 7, 8, 9, 10, 11, 12)
}

#[cfg(feature = "libsql")]
#[allow(clippy::too_many_arguments)]
fn libsql_row_to_tool_at(
    row: &libsql::Row,
    id_idx: i32,
    user_id_idx: i32,
    name_idx: i32,
    version_idx: i32,
    description_idx: i32,
    schema_idx: i32,
    source_url_idx: i32,
    trust_level_idx: i32,
    status_idx: i32,
    created_at_idx: i32,
    updated_at_idx: i32,
) -> Result<StoredWasmTool, WasmStorageError> {
    let id_str: String = row
        .get(id_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let trust_level_str: String = row
        .get(trust_level_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let status_str: String = row
        .get(status_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let schema_str: String = row
        .get(schema_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let created_at_str: String = row
        .get(created_at_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;
    let updated_at_str: String = row
        .get(updated_at_idx)
        .map_err(|e| WasmStorageError::Database(e.to_string()))?;

    Ok(StoredWasmTool {
        id: id_str
            .parse()
            .map_err(|e: uuid::Error| WasmStorageError::InvalidData(e.to_string()))?,
        user_id: row
            .get(user_id_idx)
            .map_err(|e| WasmStorageError::Database(e.to_string()))?,
        name: row
            .get(name_idx)
            .map_err(|e| WasmStorageError::Database(e.to_string()))?,
        version: row
            .get(version_idx)
            .map_err(|e| WasmStorageError::Database(e.to_string()))?,
        description: row
            .get(description_idx)
            .map_err(|e| WasmStorageError::Database(e.to_string()))?,
        parameters_schema: serde_json::from_str(&schema_str).unwrap_or_default(),
        source_url: row
            .get::<String>(source_url_idx)
            .ok()
            .filter(|s| !s.is_empty()),
        trust_level: trust_level_str
            .parse()
            .map_err(WasmStorageError::InvalidData)?,
        status: status_str.parse().map_err(WasmStorageError::InvalidData)?,
        created_at: libsql_wasm_parse_ts(&created_at_str)?,
        updated_at: libsql_wasm_parse_ts(&updated_at_str)?,
    })
}

#[cfg(test)]
mod tests {
    use crate::tools::wasm::storage::{
        ToolStatus, TrustLevel, compute_binary_hash, verify_binary_integrity,
    };

    #[test]
    fn test_compute_hash() {
        let binary = b"(module)";
        let hash = compute_binary_hash(binary);
        assert_eq!(hash.len(), 32); // BLAKE3 produces 32-byte hash
    }

    #[test]
    fn test_verify_integrity_success() {
        let binary = b"test wasm binary content";
        let hash = compute_binary_hash(binary);
        assert!(verify_binary_integrity(binary, &hash));
    }

    #[test]
    fn test_verify_integrity_failure() {
        let binary = b"test wasm binary content";
        let hash = compute_binary_hash(binary);
        let tampered = b"tampered wasm binary content";
        assert!(!verify_binary_integrity(tampered, &hash));
    }

    #[test]
    fn test_trust_level_parse() {
        assert_eq!("system".parse::<TrustLevel>().unwrap(), TrustLevel::System);
        assert_eq!(
            "verified".parse::<TrustLevel>().unwrap(),
            TrustLevel::Verified
        );
        assert_eq!("user".parse::<TrustLevel>().unwrap(), TrustLevel::User);
        assert!("invalid".parse::<TrustLevel>().is_err());
    }

    #[test]
    fn test_status_parse() {
        assert_eq!("active".parse::<ToolStatus>().unwrap(), ToolStatus::Active);
        assert_eq!(
            "disabled".parse::<ToolStatus>().unwrap(),
            ToolStatus::Disabled
        );
        assert_eq!(
            "quarantined".parse::<ToolStatus>().unwrap(),
            ToolStatus::Quarantined
        );
        assert!("invalid".parse::<ToolStatus>().is_err());
    }
}
