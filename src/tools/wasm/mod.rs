//! WASM sandbox for untrusted tool execution.
//!
//! Provides Wasmtime-based sandboxed execution for tools. See individual
//! submodules for details.
//!
//! Data types (Capabilities, ResourceLimits, OAuthRefreshConfig, storage
//! data) live in `ironclaw_common` so they can be used without the
//! `wasm-sandbox` feature. Wasmtime-dependent code (runtime, wrapper,
//! host, loader) lives in this module and is gated at the `pub mod wasm;`
//! declaration in `src/tools/mod.rs`.

/// Host WIT version for tool extensions.
pub const WIT_TOOL_VERSION: &str = "0.3.0";

/// Host WIT version for channel extensions.
pub const WIT_CHANNEL_VERSION: &str = "0.3.0";

mod capabilities;
mod capabilities_schema;
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
pub use ironclaw_common::oauth_refresh::OAuthRefreshConfig;
#[cfg(feature = "wasm-sandbox")]
pub use wrapper::WasmToolWrapper;

// Capabilities (V2)
pub use capabilities::{
    Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
    ToolInvokeCapability, WebhookCapability, WorkspaceCapability, WorkspaceReader,
};

// Security components (V2) — re-export from ungated locations for backward compat.
pub use crate::tools::allowlist::{AllowlistResult, AllowlistValidator, DenyReason};
pub(crate) use crate::tools::credentials::inject_credential;
pub use crate::tools::credentials::{
    CredentialInjector, InjectedCredentials, InjectionError, SharedCredentialRegistry,
};
#[cfg(test)]
pub(crate) use crate::tools::http_security::is_private_ip;
#[cfg(feature = "wasm-sandbox")]
pub(crate) use http_security::reject_private_ip;
#[cfg(feature = "wasm-sandbox")]
pub(crate) use crate::tools::http_security::ssrf_safe_client_builder;
pub(crate) use crate::tools::http_security::{
    ssrf_safe_client_builder_for_target, validate_and_resolve_http_target,
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
