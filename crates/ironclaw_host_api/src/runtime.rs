//! Runtime and trust classification contracts.
//!
//! [`RuntimeKind`] identifies the execution lane required for a capability or
//! invocation: WASM, MCP, script, first-party extension, or system service.
//! [`TrustClass`] is the *effective* authority ceiling consumed by downstream
//! authorization — not a grant. Even first-party and system contexts still
//! need explicit mounts, capability grants, resource scopes, and audit
//! obligations.
//!
//! Privileged runtime/trust variants are host-assigned only. They serialize for
//! audit and durable trusted records, but plain serde deserialization rejects
//! them so untrusted manifests cannot self-assert first-party or system status.
//!
//! The *requested* counterpart — what an untrusted manifest declares — lives
//! in [`crate::trust::RequestedTrustClass`]. Conversion from requested to
//! effective trust must go through the host policy engine in `ironclaw_trust`;
//! this is the only path that can construct privileged effective variants.

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

/// Effective trust ceiling for an invocation, produced by the host trust
/// policy engine.
///
/// `Sandbox` and `UserTrusted` are constructible by any caller; `FirstParty`
/// and `System` should only be produced by `ironclaw_trust::TrustPolicy`. The
/// `#[serde(skip_deserializing)]` markers prevent untrusted JSON from forging
/// the privileged variants — but since this enum's variants are otherwise
/// public, downstream code that requires a *policy-validated* effective trust
/// must consume `ironclaw_trust::EffectiveTrustClass`, whose privileged
/// constructors are crate-private.
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
