//! Runtime and trust classification contracts.
//!
//! [`RuntimeKind`] identifies the execution lane required for a capability or
//! invocation: WASM, MCP, script, first-party extension, or system service.
//! [`TrustClass`] is an authority ceiling, not a grant. Even first-party and
//! system contexts still need explicit mounts, capability grants, resource
//! scopes, and audit obligations.
//!
//! Privileged runtime/trust variants are host-assigned only. They serialize for
//! audit and durable trusted records, but plain serde deserialization rejects
//! them so untrusted manifests cannot self-assert first-party or system status.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Wasm,
    Mcp,
    Script,
    #[serde(skip_deserializing)]
    FirstParty,
    #[serde(skip_deserializing)]
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustClass {
    Sandbox,
    UserTrusted,
    #[serde(skip_deserializing)]
    FirstParty,
    #[serde(skip_deserializing)]
    System,
}
