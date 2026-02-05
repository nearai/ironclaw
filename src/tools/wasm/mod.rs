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

mod allowlist;
mod capabilities;
mod capabilities_schema;
mod credential_injector;
mod error;
mod host;
mod limits;
mod loader;
mod rate_limiter;
mod runtime;
mod storage;
mod wrapper;

// Core types
pub use error::{TrapCode, TrapInfo, WasmError};
pub use host::{HostState, LogEntry, LogLevel};
pub use limits::{
    DEFAULT_FUEL_LIMIT, DEFAULT_MEMORY_LIMIT, DEFAULT_TIMEOUT, FuelConfig, ResourceLimits,
    WasmResourceLimiter,
};
pub use runtime::{PreparedModule, WasmRuntimeConfig, WasmToolRuntime};
pub use wrapper::WasmToolWrapper;

// Capabilities (V2)
pub use capabilities::{
    Capabilities, EndpointPattern, HttpCapability, RateLimitConfig, SecretsCapability,
    ToolInvokeCapability, WorkspaceCapability, WorkspaceReader,
};

// Security components (V2)
pub use allowlist::{AllowlistResult, AllowlistValidator, DenyReason};
pub use credential_injector::{CredentialInjector, InjectedCredentials, InjectionError};
pub use rate_limiter::{LimitType, RateLimitError, RateLimitResult, RateLimiter};

// Storage (V2)
pub use storage::{
    PostgresWasmToolStore, StoreToolParams, StoredCapabilities, StoredWasmTool,
    StoredWasmToolWithBinary, ToolStatus, TrustLevel, WasmStorageError, WasmToolStore,
    compute_binary_hash, verify_binary_integrity,
};

// Loader
pub use loader::{DiscoveredTool, LoadResults, WasmLoadError, WasmToolLoader, discover_tools};

// Capabilities schema (for parsing *.capabilities.json files)
pub use capabilities_schema::{CapabilitiesFile, RateLimitSchema};
