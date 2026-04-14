//! WASM binary storage data types.
//!
//! These are the pure-data types shared across the codebase. The actual
//! PostgreSQL/libSQL `WasmToolStore` implementations live in
//! `src/tools/wasm/storage.rs` because they depend on backend-specific
//! database crates.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::capabilities::{
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
    pub wit_version: String,
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

        if !self.workspace_read_prefixes.is_empty() {
            caps = caps.with_workspace_read(self.workspace_read_prefixes.clone());
        }

        if !self.http_allowlist.is_empty() {
            caps.http = Some(HttpCapability {
                allowlist: self.http_allowlist.clone(),
                credentials: HashMap::new(),
                rate_limit: RateLimitConfig {
                    requests_per_minute: self.requests_per_minute,
                    requests_per_hour: self.requests_per_hour,
                },
                max_request_bytes: self.max_request_body_bytes as usize,
                max_response_bytes: self.max_response_body_bytes as usize,
                timeout: std::time::Duration::from_secs(self.http_timeout_secs as u64),
            });
        }

        if !self.tool_aliases.is_empty() {
            caps.tool_invoke = Some(ToolInvokeCapability {
                aliases: self.tool_aliases.clone(),
                rate_limit: RateLimitConfig {
                    requests_per_minute: self.requests_per_minute,
                    requests_per_hour: self.requests_per_hour,
                },
            });
        }

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

/// Parameters for storing a new tool.
pub struct StoreToolParams {
    pub user_id: String,
    pub name: String,
    pub version: String,
    pub wit_version: String,
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
