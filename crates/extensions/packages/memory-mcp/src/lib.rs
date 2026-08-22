//! Generic MCP-backed memory provider for IronClaw Reborn.
//!
//! This crate is the point at which memory stops being a fixed menu. The native
//! and mem0 providers are each a Rust crate plus a compiled arm in the
//! composition factory, so adding a memory system meant changing IronClaw. This
//! provider is bound to a memory system by CONFIGURATION — an MCP server, a
//! credential, and two tool names — so a system that speaks the memory-over-MCP
//! contract plugs in without a new crate and without a new factory arm.
//!
//! ## Shape
//!
//! - [`McpMemoryTransport`] is the MCP seam. This crate owns no MCP client and
//!   no HTTP client; composition injects an adapter over `ironclaw_mcp`'s
//!   host-mediated client, so the provider inherits session handling, protocol
//!   validation, and the host-mediated egress boundary without depending on a
//!   `runtimes`-layer crate from `substrates`.
//! - [`McpMemoryConfig`] carries the vendor's tool names. They are configuration
//!   rather than constants precisely so the second vendor costs no code.
//! - [`McpMemoryService`] maps the IronClaw memory lanes onto tool calls. See
//!   its module docs for the per-lane fidelity table.
//!
//! ## Lanes
//!
//! Two are implemented: `read_long_term` and `record_interaction` — retrieve
//! before the run, record after the turn, which is the loop that makes memory
//! feel like memory. `read_short_term` and `profile_read` fall through to the
//! trait defaults, and a provider's manifest simply must not declare those
//! hooks: the host only calls the lifecycle hooks a manifest declares, so
//! shipping a subset is a supported shape rather than a partial implementation.
//!
//! ## Boundary
//!
//! Provider-neutral-contract-conformant: among internal IronClaw crates this
//! depends only on the memory contract and the host-api id/scope substrate. It
//! never reaches into host composition, dispatch, filesystem, or runtime.

mod config;
mod service;
mod transport;

/// Reserved extension id under which a deployment binds this provider.
///
/// A deployment names its own provider through the `[memory]` binding; this id
/// is the generic MCP lane itself, not any one vendor.
pub const MCP_MEMORY_EXTENSION_ID: &str = "memory.mcp";

pub use config::{DEFAULT_RECORD_TOOL, DEFAULT_SEARCH_TOOL, McpMemoryConfig};
pub use service::McpMemoryService;
#[cfg(any(test, feature = "test-support"))]
pub use transport::MockMcpMemoryTransport;
pub use transport::{McpMemoryToolCall, McpMemoryTransport, McpMemoryTransportError};
