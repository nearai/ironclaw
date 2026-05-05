//! Reborn WASM component runtime lane.
//!
//! This crate owns the Reborn-only WASM runtime surface. It intentionally uses
//! the canonical WIT/component-model contract in `wit/tool.wit` instead of the
//! temporary JSON pointer/length ABI that was abandoned before landing.

mod bindings;
mod config;
mod error;
mod host;
mod limiter;
mod runtime;
mod store;
mod types;

pub use config::{WIT_TOOL_VERSION, WitToolLimits, WitToolRuntimeConfig};
pub use error::{WasmError, WasmHostError};
pub use host::{
    DenyWasmHostHttp, DenyWasmHostSecrets, DenyWasmHostTools, DenyWasmHostWorkspace,
    EmptyWasmRuntimeCredentials, RecordingWasmHostHttp, SystemWasmHostClock, WasmHostClock,
    WasmHostHttp, WasmHostSecrets, WasmHostTools, WasmHostWorkspace, WasmHttpRequest,
    WasmHttpResponse, WasmRuntimeCredentialProvider, WasmRuntimeCredentialRequest,
    WasmRuntimeHttpAdapter, WasmStagedRuntimeCredential, WasmStagedRuntimeCredentials, WitToolHost,
};
pub use runtime::WitToolRuntime;
pub use types::{PreparedWitTool, WasmLogLevel, WasmLogRecord, WitToolExecution, WitToolRequest};
