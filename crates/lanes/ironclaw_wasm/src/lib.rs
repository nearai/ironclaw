//! Reborn WASM component runtime lane.
//!
//! This crate owns the Reborn-only WASM runtime surface. It intentionally uses
//! the canonical WIT/component-model contract in this crate's own
//! `wit/tool.wit` instead of the temporary JSON pointer/length ABI that was
//! abandoned before landing.

mod bindings;
mod config;
mod error;
mod host;
mod nostr_relay;
mod nostr_signer;
mod runtime;
mod store;
mod types;
pub mod wasm_sandbox_core;

pub use config::{TOOL_WIT, WIT_TOOL_VERSION, WitToolRuntimeConfig};
pub use error::{WasmError, WasmHostError};
pub use host::{
    DenyWasmHostHttp, DenyWasmHostNostr, DenyWasmHostSecrets, DenyWasmHostTools,
    DenyWasmHostWorkspace, EmptyWasmRuntimeCredentials, RecordingWasmHostHttp,
    SystemWasmHostClock, WasmHostClock, WasmHostHttp, WasmHostNostr, WasmHostSecrets,
    WasmHostTools, WasmHostWorkspace, WasmHttpRequest, WasmHttpResponse,
    WasmRuntimeCredentialProvider, WasmRuntimeCredentialRequest, WasmRuntimeHttpAdapter,
    WasmRuntimePolicyDiscarder, WasmStagedRuntimeCredential, WasmStagedRuntimeCredentialScope,
    WasmStagedRuntimeCredentials, WitToolHost,
};
pub use nostr_relay::{publish_nostr_event, subscribe_nostr_events, validate_relay_url};
pub use nostr_signer::{decode_nostr_private_key, sign_nostr_event, NostrSignError};
pub use runtime::WitToolRuntime;
pub use types::{PreparedWitTool, WasmLogLevel, WasmLogRecord, WitToolExecution, WitToolRequest};
