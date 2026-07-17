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

impl RuntimeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wasm => "wasm",
            Self::Mcp => "mcp",
            Self::Script => "script",
            Self::FirstParty => "first_party",
            Self::System => "system",
        }
    }
}

impl std::fmt::Display for RuntimeKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Trusted deserialization for [`RuntimeKind`] on **durable host-written
/// records** (e.g. the process store's `ProcessRecord`). It accepts **every**
/// variant — including the host-assigned `FirstParty` / `System` that the
/// derived `Deserialize` intentionally rejects (module docs) so untrusted
/// manifests/worker output cannot self-assert privileged status.
///
/// Use ONLY via `#[serde(deserialize_with = ...)]` on a field of a record the
/// host itself wrote and re-reads. Never wire it into a path that parses
/// untrusted input — that would reopen the forgery hole the `skip_deserializing`
/// markers close. (A store round-trips its own trusted bytes; a manifest does
/// not.)
pub fn deserialize_trusted_runtime_kind<'de, D>(deserializer: D) -> Result<RuntimeKind, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    match raw.as_str() {
        // Only the `#[serde(skip_deserializing)]` host-assigned variants are
        // hand-mapped; everything else delegates to the derived `Deserialize`,
        // so a future non-privileged variant round-trips here without edits
        // (hand-listing all names would silently reject it — the same durable
        // round-trip failure this helper exists to prevent).
        "first_party" => Ok(RuntimeKind::FirstParty),
        "system" => Ok(RuntimeKind::System),
        other => RuntimeKind::deserialize(serde::de::value::StrDeserializer::new(other)),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct DurableHolder {
        #[serde(deserialize_with = "deserialize_trusted_runtime_kind")]
        runtime: RuntimeKind,
    }

    // Regression (arch-simplification §4.3): a durable host-written record must
    // round-trip EVERY runtime kind, including the host-assigned `System`/
    // `FirstParty` that the InMemory process store never serialized but the
    // filesystem store does. Before `deserialize_trusted_runtime_kind`, reading
    // back a `ProcessRecord { runtime: System }` failed with `unknown variant`.
    #[test]
    fn trusted_runtime_kind_round_trips_every_variant() {
        for kind in [
            RuntimeKind::Wasm,
            RuntimeKind::Mcp,
            RuntimeKind::Script,
            RuntimeKind::FirstParty,
            RuntimeKind::System,
        ] {
            let json = serde_json::to_string(&DurableHolder { runtime: kind }).unwrap();
            let back: DurableHolder = serde_json::from_str(&json).unwrap();
            assert_eq!(back.runtime, kind, "trusted path must round-trip {kind}");
        }
    }

    // The security boundary the trusted path must NOT weaken: the *derived*
    // `Deserialize` (used for untrusted manifest/worker input) still rejects the
    // host-assigned privileged kinds, so untrusted JSON cannot forge them.
    #[test]
    fn default_deserialize_still_rejects_privileged_variants() {
        assert!(serde_json::from_str::<RuntimeKind>("\"system\"").is_err());
        assert!(serde_json::from_str::<RuntimeKind>("\"first_party\"").is_err());
        assert!(serde_json::from_str::<RuntimeKind>("\"wasm\"").is_ok());
        assert!(serde_json::from_str::<RuntimeKind>("\"mcp\"").is_ok());
        assert!(serde_json::from_str::<RuntimeKind>("\"script\"").is_ok());
    }
}
