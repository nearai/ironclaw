//! Runtime and trust classification contracts.
//!
//! [`RuntimeKind`] identifies the execution lane required for a capability or
//! invocation: WASM, MCP, script, first-party extension, or system service.
//! [`TrustClass`] is an authority ceiling, not a grant. Even first-party and
//! system contexts still need explicit mounts, capability grants, resource
//! scopes, and audit obligations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Wasm,
    Mcp,
    Script,
    FirstParty,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustClass {
    Sandbox,
    UserTrusted,
    FirstParty,
    System,
}
