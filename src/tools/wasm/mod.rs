//! WASM sandbox for untrusted tool execution.
//!
//! Provides Wasmtime-based sandboxed execution for tools. See individual
//! submodules for details.
//!
//! Data types (Capabilities, ResourceLimits, OAuthRefreshConfig, storage
//! data) live in `ironclaw_common` so they can be used without the
//! `wasm-sandbox` feature. Wasmtime-dependent code (runtime, wrapper,
//! host, loader) lives in this module which is gated at the `pub mod wasm;`
//! declaration in `src/tools/mod.rs`.

/// Host WIT version for tool extensions.
pub const WIT_TOOL_VERSION: &str = "0.3.0";

/// Host WIT version for channel extensions.
pub const WIT_CHANNEL_VERSION: &str = "0.3.0";

#[cfg(feature = "wasm-sandbox")]
mod capabilities;
#[cfg(feature = "wasm-sandbox")]
mod capabilities_schema;
#[cfg(feature = "wasm-sandbox")]
mod error;
#[cfg(feature = "wasm-sandbox")]
mod host;
#[cfg(feature = "wasm-sandbox")]
mod http_security;
#[cfg(feature = "wasm-sandbox")]
mod limits;
#[cfg(feature = "wasm-sandbox")]
pub(crate) mod loader;
#[cfg(feature = "wasm-sandbox")]
mod rate_limiter;
#[cfg(feature = "wasm-sandbox")]
mod runtime;
#[cfg(feature = "wasm-sandbox")]
pub(crate) mod storage;
#[cfg(feature = "wasm-sandbox")]
mod wrapper;

// Core types
#[cfg(feature = "wasm-sandbox")]
pub use error::WasmError;
#[cfg(feature = "wasm-sandbox")]
pub use host::{HostState, LogEntry, LogLevel};
#[cfg(feature = "wasm-sandbox")]
pub use limits::{
    DEFAULT_FUEL_LIMIT, DEFAULT_MEMORY_LIMIT, DEFAULT_TIMEOUT, FuelConfig, ResourceLimits,
    WasmResourceLimiter,
};
#[cfg(feature = "wasm-sandbox")]
pub use runtime::{PreparedModule, WasmRuntimeConfig, WasmToolRuntime, enable_compilation_cache};
#[cfg(feature = "wasm-sandbox")]
pub use wrapper::{OAuthRefreshConfig, WasmToolWrapper};

// Capabilities (V2)
#[cfg(feature = "wasm-sandbox")]
pub use capabilities::{
    Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
    ToolInvokeCapability, WebhookCapability, WorkspaceCapability, WorkspaceReader,
};

// Security components — re-export from ungated locations for backward compat.
pub use crate::tools::allowlist::{AllowlistResult, AllowlistValidator, DenyReason};
pub use crate::tools::credentials::{
    CredentialInjector, InjectedCredentials, InjectionError, SharedCredentialRegistry,
};
#[cfg(all(test, feature = "wasm-sandbox"))]
pub(crate) use crate::tools::http_security::is_private_ip;
#[cfg(feature = "wasm-sandbox")]
pub(crate) use crate::tools::http_security::{
    ssrf_safe_client_builder, ssrf_safe_client_builder_for_target, validate_and_resolve_http_target,
};
#[cfg(feature = "wasm-sandbox")]
pub(crate) use http_security::reject_private_ip;
#[cfg(feature = "wasm-sandbox")]
pub use rate_limiter::{LimitType, RateLimitError, RateLimitResult, RateLimiter};

// Storage
#[cfg(all(feature = "wasm-sandbox", feature = "libsql"))]
pub use storage::LibSqlWasmToolStore;
#[cfg(all(feature = "wasm-sandbox", feature = "postgres"))]
pub use storage::PostgresWasmToolStore;
#[cfg(feature = "wasm-sandbox")]
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
#[cfg(feature = "wasm-sandbox")]
pub use capabilities_schema::{
    AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema, RateLimitSchema,
    ToolFieldSetupSchema, ToolSetupFieldInputType, ToolSetupSchema, ValidationEndpointSchema,
};

// ---------------------------------------------------------------------------
// Stub types when `wasm-sandbox` feature is disabled (e.g. armv7 builds).
// ---------------------------------------------------------------------------

#[cfg(not(feature = "wasm-sandbox"))]
mod stubs {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use thiserror::Error;

    pub use ironclaw_common::capabilities::{
        Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
        ToolInvokeCapability, WebhookCapability, WorkspaceCapability, WorkspaceReader,
    };
    pub use ironclaw_common::capabilities_schema::{
        AuthCapabilitySchema, CapabilitiesFile, OAuthConfigSchema, RateLimitSchema,
        ToolFieldSetupSchema, ToolSetupFieldInputType, ToolSetupSchema, ValidationEndpointSchema,
    };

    #[derive(Debug, Error)]
    pub enum WasmError {
        #[error("wasm-sandbox feature not enabled")]
        NotEnabled,
    }

    #[derive(Debug, Error)]
    pub enum WasmLoadError {
        #[error("wasm-sandbox feature not enabled")]
        NotEnabled,
        #[error("{0}")]
        Other(String),
    }

    /// Stub resource limits matching the real type's fields used by callers.
    #[derive(Debug, Clone, Default)]
    pub struct ResourceLimits;

    /// Stub runtime config.
    #[derive(Debug, Clone, Default)]
    pub struct WasmRuntimeConfig;

    /// Stub WASM tool runtime.
    #[derive(Debug)]
    pub struct WasmToolRuntime;

    impl WasmToolRuntime {
        pub fn new(_config: WasmRuntimeConfig) -> Result<Arc<Self>, WasmError> {
            Err(WasmError::NotEnabled)
        }
        pub async fn remove(&self, _name: &str) {}
    }

    /// Stub WASM tool wrapper.
    pub struct WasmToolWrapper;

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

        pub async fn load_from_dir(&self, _dir: &Path) -> Result<Vec<String>, WasmLoadError> {
            Ok(Vec::new())
        }

        pub async fn load_tool(
            &self,
            _name: &str,
            _wasm_path: &Path,
            _cap_path: Option<&Path>,
        ) -> Result<(), WasmLoadError> {
            Err(WasmLoadError::NotEnabled)
        }

        pub fn with_role_lookup(self, _role_lookup: Arc<dyn crate::db::UserStore>) -> Self {
            self
        }

        pub async fn load_from_files(
            &self,
            _name: &str,
            _wasm_path: &Path,
            _cap_path: Option<&Path>,
        ) -> Result<(), WasmLoadError> {
            Err(WasmLoadError::NotEnabled)
        }
    }

    /// Stub discovered tool.
    #[derive(Debug)]
    pub struct DiscoveredTool {
        pub name: String,
        pub wasm_path: PathBuf,
        pub capabilities_path: Option<PathBuf>,
    }

    pub async fn discover_tools(
        _dir: &Path,
    ) -> Result<std::collections::HashMap<String, DiscoveredTool>, std::io::Error> {
        Ok(std::collections::HashMap::new())
    }

    pub async fn discover_dev_tools()
    -> Result<std::collections::HashMap<String, DiscoveredTool>, std::io::Error> {
        Ok(std::collections::HashMap::new())
    }

    pub fn wasm_artifact_path(_crate_dir: &Path, _binary_name: &str) -> PathBuf {
        PathBuf::new()
    }

    pub async fn load_dev_tools(
        _runtime: Arc<WasmToolRuntime>,
        _dir: &Path,
    ) -> Result<Vec<String>, WasmLoadError> {
        Ok(Vec::new())
    }

    pub fn resolve_wasm_target_dir() -> PathBuf {
        PathBuf::new()
    }

    pub fn check_wit_version_compat(
        _name: &str,
        _declared: Option<&str>,
        _host_version: &str,
    ) -> Result<(), WasmLoadError> {
        Ok(())
    }
}

#[cfg(not(feature = "wasm-sandbox"))]
pub use stubs::*;
