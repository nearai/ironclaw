//! MCP adapter contracts for IronClaw Reborn.
//!
//! `ironclaw_mcp` adapts manifest-declared MCP tools into IronClaw
//! capabilities. It does not grant MCP servers ambient filesystem, secret, or
//! network authority; the host-selected client is the only integration point and
//! resource accounting still happens host-side, through the narrow
//! [`RuntimeResourceBudget`](ironclaw_host_api::resource::RuntimeResourceBudget)
//! port — this lane holds no budget authority of its own and can only reserve,
//! reconcile, and release.
//!
//! # Module charter
//!
//! PROPOSAL §6.6.3 asks this crate to split its single file into chartered
//! modules. The pipeline runs outward: a caller names the **contract**, the
//! **runtime** governs resources around it, the **client** speaks the protocol,
//! **jsonrpc** frames it, **discovery** admits what comes back, **egress** is
//! the only way bytes leave, and **diagnostics** is the only vocabulary any of
//! them may report a failure in. Each concern owns one module, and the table
//! below is the rule for where new code goes.
//!
//! The submodules are **private**: every public item is re-exported here, so
//! `ironclaw_mcp::X` stays the single import path for callers outside the
//! crate and the module names are never part of the public API.
//!
//! | Module | Owns | Never contains |
//! |---|---|---|
//! | `contract` | The vocabulary a caller names: config, invocation/request/output DTOs, the [`McpClient`] and [`McpExecutor`] traits, and the [`McpError`]/[`McpClientError`] taxonomy | Protocol framing, transport, or resource accounting |
//! | `runtime` | Resource-governed execution: reserve → call → reconcile/release, descriptor admission, and the manifest credential context an auth failure reports | JSON-RPC, HTTP, or catalog parsing |
//! | `client` | The Streamable-HTTP [`McpClient`] implementation: handshake, per-invocation session lifecycle, the `tools/list` paging loop | The wire codec (that is `jsonrpc`) or catalog admission rules (that is `discovery`) |
//! | `jsonrpc` | The JSON-RPC 2.0 codec and MCP response hygiene: encode, plain-JSON and SSE framing, id matching, session-id and protocol-version validation, auth-challenge extraction, per-method credential routing | Session *state* (that is `client`) or tool-shape rules (that is `discovery`) |
//! | `discovery` | `tools/list` catalog admission: the host ceilings, per-tool classification, input-schema bounds, description bounding, annotations, tool-name grammar | Anything that sends or receives a request |
//! | `egress` | The host-mediated HTTP seam: the [`McpHostHttp`] port, its runtime-egress adapter, and the host-owned egress plan/planner | A URL, header, or body decision that is protocol content |
//! | `diagnostics` | Every stable, bounded failure token the lane surfaces, and the cause enums behind them | A failure *decision* — modules classify, `diagnostics` only names |
//!
//! Two rules keep the charter honest:
//!
//! - **No module builds a failure string of its own.** Reasons are constructed
//!   only from `diagnostics`' cause enums, so every token the model can see is
//!   bounded and enumerable in one file. Armed by
//!   `tests/module_charter.rs`, which also carries the rule's one enumerated
//!   carve-out: `runtime.rs`'s two `McpError` descriptor/invocation reasons,
//!   which echo the manifest's own ids rather than classifying a failure. The
//!   `egress` seam was the other exception until its two reasons became
//!   `McpEgressCause`, and `impl From<String> for McpClientError` — the
//!   implicit bypass — is gone.
//! - **`discovery` owns the rules, `client` owns the loop.** The per-page caps
//!   and the running-total check read the *same* constants from `discovery`, so
//!   the two enforcement points cannot drift apart.

mod client;
mod contract;
mod diagnostics;
mod discovery;
mod egress;
mod jsonrpc;
mod runtime;

pub use client::McpHostHttpClient;
pub use contract::{
    McpClient, McpClientError, McpClientOutput, McpClientRequest, McpError, McpExecutionRequest,
    McpExecutionResult, McpExecutor, McpInvocation, McpProviderRejection, McpRuntimeConfig,
    McpToolDiscoveryOutput,
};
pub use egress::{
    McpHostHttp, McpHostHttpEgressPlan, McpHostHttpEgressPlanRequest, McpHostHttpEgressPlanner,
    McpHostHttpError, McpHostHttpResponse, McpRuntimeHttpAdapter, StaticMcpHostHttpEgressPlanner,
};
pub use runtime::McpRuntime;
