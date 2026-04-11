//! WASM sandbox for untrusted tool execution.
//!
//! This module provides Wasmtime-based sandboxed execution for tools,
//! following patterns from NEAR blockchain and modern WASM best practices:
//!
//! - **Compile once, instantiate fresh**: Tools are validated and compiled
//!   at registration time. Each execution creates a fresh instance.
//!
//! - **Fuel metering**: CPU usage is limited via Wasmtime's fuel system.
//!
//! - **Memory limits**: Memory growth is bounded via ResourceLimiter.
//!
//! - **Extended host API (V2)**: log, time, workspace, HTTP, tool invoke, secrets
//!
//! - **Capability-based security**: Features are opt-in via Capabilities.
//!
//! # Architecture (V2)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                              WASM Tool Execution                             │
//! │                                                                              │
//! │   WASM Tool ──▶ Host Function ──▶ Allowlist ──▶ Credential ──▶ Execute     │
//! │   (untrusted)   (boundary)        Validator     Injector       Request      │
//! │                                                                    │        │
//! │                                                                    ▼        │
//! │                              ◀────── Leak Detector ◀────── Response        │
//! │                          (sanitized, no secrets)                            │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Security Constraints
//!
//! | Threat | Mitigation |
//! |--------|------------|
//! | CPU exhaustion | Fuel metering |
//! | Memory exhaustion | ResourceLimiter, 10MB default |
//! | Infinite loops | Epoch interruption + tokio timeout |
//! | Filesystem access | No WASI FS, only host workspace_read |
//! | Network access | Allowlisted endpoints only |
//! | Credential exposure | Injection at host boundary only |
//! | Secret exfiltration | Leak detector scans all outputs |
//! | Log spam | Max 1000 entries, 4KB per message |
//! | Path traversal | Validate paths (no `..`, no `/` prefix) |
//! | Trap recovery | Discard instance, never reuse |
//! | Side channels | Fresh instance per execution |
//! | Rate abuse | Per-tool rate limiting |
//! | WASM tampering | BLAKE3 hash verification on load |
//! | Direct tool access | Tool aliasing (indirection layer) |
//!
//! # Example
//!
//! ```ignore
//! use ironclaw::tools::wasm::{WasmToolRuntime, WasmRuntimeConfig, WasmToolWrapper};
//! use ironclaw::tools::wasm::Capabilities;
//! use std::sync::Arc;
//!
//! // Create runtime
//! let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::default())?);
//!
//! // Prepare a tool from WASM bytes
//! let wasm_bytes = std::fs::read("my_tool.wasm")?;
//! let prepared = runtime.prepare("my_tool", &wasm_bytes, None).await?;
//!
//! // Create wrapper with HTTP capability
//! let capabilities = Capabilities::none()
//!     .with_http(HttpCapability::new(vec![
//!         EndpointPattern::host("api.openai.com").with_path_prefix("/v1/"),
//!     ]));
//! let tool = WasmToolWrapper::new(runtime, prepared, capabilities);
//!
//! // Execute (implements Tool trait)
//! let output = tool.execute(serde_json::json!({"input": "test"}), &ctx).await?;
//! ```

/// Host WIT version for tool extensions.
///
/// Extensions declaring a `wit_version` in their capabilities file are checked
/// against this at load time: same major, not greater than host.
pub const WIT_TOOL_VERSION: &str = "0.3.0";

/// Host WIT version for channel extensions.
pub const WIT_CHANNEL_VERSION: &str = "0.3.0";

mod allowlist;
mod capabilities;
mod capabilities_schema;
pub(crate) mod credential_injector;
mod error;
#[cfg(feature = "wasm-sandbox")]
mod host;
mod http_security;
mod limits;
#[cfg(feature = "wasm-sandbox")]
pub(crate) mod loader;
mod rate_limiter;
#[cfg(feature = "wasm-sandbox")]
mod runtime;
pub(crate) mod storage;
#[cfg(feature = "wasm-sandbox")]
mod wrapper;

// Core types
pub use error::WasmError;
#[cfg(feature = "wasm-sandbox")]
pub use host::{HostState, LogEntry, LogLevel};
#[cfg(feature = "wasm-sandbox")]
pub use limits::WasmResourceLimiter;
pub use limits::{
    DEFAULT_FUEL_LIMIT, DEFAULT_MEMORY_LIMIT, DEFAULT_TIMEOUT, FuelConfig, ResourceLimits,
};
#[cfg(feature = "wasm-sandbox")]
pub use runtime::{PreparedModule, WasmRuntimeConfig, WasmToolRuntime, enable_compilation_cache};
#[cfg(feature = "wasm-sandbox")]
pub use wrapper::{OAuthRefreshConfig, WasmToolWrapper};

// Capabilities (V2)
pub use capabilities::{
    Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
    ToolInvokeCapability, WebhookCapability, WorkspaceCapability, WorkspaceReader,
};

// Security components (V2)
pub use allowlist::{AllowlistResult, AllowlistValidator, DenyReason};
pub(crate) use credential_injector::inject_credential;
pub use credential_injector::{
    CredentialInjector, InjectedCredentials, InjectionError, SharedCredentialRegistry,
};
#[cfg(test)]
pub(crate) use http_security::is_private_ip;
pub(crate) use http_security::{
    reject_private_ip, ssrf_safe_client_builder, ssrf_safe_client_builder_for_target,
    validate_and_resolve_http_target,
};
pub use rate_limiter::{LimitType, RateLimitError, RateLimitResult, RateLimiter};

// Storage (V2)
#[cfg(feature = "libsql")]
pub use storage::LibSqlWasmToolStore;
#[cfg(feature = "postgres")]
pub use storage::PostgresWasmToolStore;
pub use storage::{
    StoreToolParams, StoredCapabilities, StoredWasmTool, StoredWasmToolWithBinary, ToolStatus,
    TrustLevel, WasmStorageError, WasmToolStore, compute_binary_hash, verify_binary_integrity,
};

// Loader
#[cfg(feature = "wasm-sandbox")]
pub use loader::{
    DiscoveredTool, LoadResults, WasmLoadError, WasmToolLoader, check_wit_version_compat,
    discover_dev_tools, discover_tools, load_dev_tools, resolve_wasm_target_dir,
    wasm_artifact_path,
};

// Capabilities schema (for parsing *.capabilities.json files)
pub use capabilities_schema::{
    AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema, RateLimitSchema,
    ToolFieldSetupSchema, ToolSetupFieldInputType, ToolSetupSchema, ValidationEndpointSchema,
};

// ---------------------------------------------------------------------------
// Stub types when `wasm-sandbox` feature is disabled (e.g. armv7 builds).
//
// These stubs let the rest of the codebase compile without wasmtime by
// providing the same public type names. Runtime code paths that attempt to
// use WASM functionality will encounter `None` or return errors.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "wasm-sandbox"))]
mod stubs {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use super::error::WasmError;
    use super::limits::{FuelConfig, ResourceLimits};

    /// Stub runtime configuration (no wasmtime available).
    #[derive(Debug, Clone, Default)]
    pub struct WasmRuntimeConfig {
        pub default_limits: ResourceLimits,
        pub fuel_config: FuelConfig,
        pub cache_compiled: bool,
        pub cache_dir: Option<PathBuf>,
    }

    impl WasmRuntimeConfig {
        pub fn for_testing() -> Self {
            Self::default()
        }
    }

    /// Stub WASM tool runtime (WASM support not compiled).
    #[derive(Debug)]
    pub struct WasmToolRuntime;

    impl WasmToolRuntime {
        pub fn new(_config: WasmRuntimeConfig) -> Result<Self, WasmError> {
            Err(WasmError::EngineCreationFailed(
                "WASM support is not compiled (wasm-sandbox feature disabled)".to_string(),
            ))
        }

        pub fn config(&self) -> &WasmRuntimeConfig {
            unreachable!("WasmToolRuntime cannot be constructed without wasm-sandbox feature")
        }

        pub async fn prepare(
            &self,
            _name: &str,
            _wasm_bytes: &[u8],
            _limits: Option<ResourceLimits>,
        ) -> Result<Arc<PreparedModule>, WasmError> {
            Err(WasmError::EngineCreationFailed(
                "WASM support is not compiled".to_string(),
            ))
        }

        pub async fn get(&self, _name: &str) -> Option<Arc<PreparedModule>> {
            None
        }

        pub async fn remove(&self, _name: &str) -> Option<Arc<PreparedModule>> {
            None
        }

        pub async fn list(&self) -> Vec<String> {
            Vec::new()
        }

        pub async fn clear(&self) {}
    }

    /// Stub compiled module.
    #[derive(Debug)]
    pub struct PreparedModule {
        pub name: String,
        pub description: String,
        pub schema: serde_json::Value,
        pub limits: ResourceLimits,
    }

    /// Stub OAuth refresh config.
    #[derive(Debug, Clone)]
    pub struct OAuthRefreshConfig {
        pub token_url: String,
        pub client_id: String,
        pub client_secret: Option<String>,
        pub exchange_proxy_url: Option<String>,
        pub gateway_token: Option<String>,
        pub secret_name: String,
        pub provider: Option<String>,
        pub extra_refresh_params: HashMap<String, String>,
    }

    impl OAuthRefreshConfig {
        pub fn oauth_proxy_auth_token(&self) -> Option<&str> {
            self.gateway_token.as_deref()
        }
    }

    /// Stub WASM tool wrapper (not constructible).
    pub struct WasmToolWrapper;

    /// Stub host state.
    #[derive(Debug)]
    pub struct HostState;

    /// Log level for WASM tools.
    #[derive(Debug, Clone, Copy)]
    pub enum LogLevel {
        Error,
        Warn,
        Info,
        Debug,
        Trace,
    }

    /// Log entry from WASM execution.
    #[derive(Debug, Clone)]
    pub struct LogEntry {
        pub level: LogLevel,
        pub message: String,
    }

    /// Stub resource limiter.
    #[derive(Debug)]
    pub struct WasmResourceLimiter;

    impl WasmResourceLimiter {
        pub fn new(_memory_limit: u64) -> Self {
            Self
        }
    }

    /// Stub discovered tool.
    #[derive(Debug, Clone)]
    pub struct DiscoveredTool {
        pub name: String,
        pub wasm_path: PathBuf,
        pub capabilities_path: Option<PathBuf>,
    }

    /// Stub load results.
    #[derive(Debug)]
    pub struct LoadResults {
        pub loaded: Vec<String>,
        pub errors: Vec<(PathBuf, WasmLoadError)>,
    }

    /// Stub load error.
    #[derive(Debug, thiserror::Error)]
    pub enum WasmLoadError {
        #[error("WASM support is not compiled (wasm-sandbox feature disabled)")]
        NotCompiled,
        #[error("IO error: {0}")]
        Io(#[from] std::io::Error),
        #[error("WIT version mismatch: {0}")]
        WitVersionMismatch(String),
    }

    /// Stub WASM tool loader.
    pub struct WasmToolLoader;

    impl WasmToolLoader {
        pub fn new(
            _runtime: Arc<WasmToolRuntime>,
            _registry: Arc<crate::tools::ToolRegistry>,
        ) -> Self {
            Self
        }

        pub fn with_secrets_store(
            self,
            _store: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
        ) -> Self {
            self
        }

        pub fn with_role_lookup(
            self,
            _store: Arc<dyn crate::db::UserStore>,
        ) -> Self {
            self
        }

        pub async fn load_from_dir(&self, _dir: &Path) -> Result<LoadResults, WasmLoadError> {
            Ok(LoadResults {
                loaded: Vec::new(),
                errors: Vec::new(),
            })
        }

        pub async fn load_from_files(
            &self,
            _name: &str,
            _wasm_path: &Path,
            _cap_path: Option<&Path>,
        ) -> Result<String, WasmLoadError> {
            Err(WasmLoadError::NotCompiled)
        }
    }

    pub fn enable_compilation_cache(
        _config: &mut (),
        _label: &str,
        _explicit_dir: Option<&Path>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn check_wit_version_compat(
        _name: &str,
        _declared: Option<&str>,
        _host: &str,
    ) -> Result<(), WasmLoadError> {
        Ok(())
    }

    pub async fn discover_tools(
        _dir: &Path,
    ) -> Result<HashMap<String, DiscoveredTool>, std::io::Error> {
        Ok(HashMap::new())
    }

    pub async fn discover_dev_tools() -> Result<HashMap<String, DiscoveredTool>, std::io::Error> {
        Ok(HashMap::new())
    }

    pub async fn load_dev_tools(
        _loader: &WasmToolLoader,
        _tools_dir: &Path,
    ) -> Result<LoadResults, WasmLoadError> {
        Ok(LoadResults {
            loaded: Vec::new(),
            errors: Vec::new(),
        })
    }

    pub fn resolve_wasm_target_dir(_tool_dir: &Path) -> PathBuf {
        PathBuf::new()
    }

    pub fn wasm_artifact_path(_crate_dir: &Path, _binary_name: &str) -> PathBuf {
        PathBuf::new()
    }
}

#[cfg(not(feature = "wasm-sandbox"))]
pub use stubs::*;
