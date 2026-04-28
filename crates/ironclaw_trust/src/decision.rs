//! Effective trust + authority-ceiling output of a trust policy evaluation.
//!
//! [`EffectiveTrustClass`] wraps [`ironclaw_host_api::TrustClass`] so the
//! privileged variants (`FirstParty`, `System`) are only constructible from
//! inside this crate. Downstream authorization code that requires a
//! policy-validated trust ceiling consumes `EffectiveTrustClass`, not
//! `TrustClass` — host_api's `#[serde(skip_deserializing)]` guards the wire
//! boundary, this newtype guards the in-process construction boundary.

use ironclaw_host_api::{EffectKind, ResourceCeiling, Timestamp, TrustClass};
use serde::Serialize;

/// Policy-validated trust ceiling.
///
/// Construction of `Sandbox` and `UserTrusted` is public because those carry
/// no host-controlled privilege. Construction of `FirstParty` and `System` is
/// crate-private — outside callers receive these only through
/// [`crate::TrustPolicy::evaluate`].
///
/// Serialization is supported so audit envelopes can record the effective
/// class. Deserialization is intentionally absent: a downstream service must
/// not be able to reconstruct a privileged effective trust from a wire
/// payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct EffectiveTrustClass {
    inner: TrustClass,
}

impl EffectiveTrustClass {
    /// Fully sandboxed. Public constructor — no privilege.
    pub fn sandbox() -> Self {
        Self {
            inner: TrustClass::Sandbox,
        }
    }

    /// User-trusted (third-party with normal user authority). Public
    /// constructor — no host-controlled privilege.
    pub fn user_trusted() -> Self {
        Self {
            inner: TrustClass::UserTrusted,
        }
    }

    /// First-party privilege. Constructible only from inside the trust crate;
    /// outside callers must receive this via policy evaluation.
    pub(crate) fn first_party() -> Self {
        Self {
            inner: TrustClass::FirstParty,
        }
    }

    /// System privilege. Constructible only from inside the trust crate.
    pub(crate) fn system() -> Self {
        Self {
            inner: TrustClass::System,
        }
    }

    /// Underlying host_api class for audit, wire output, or permission-mode
    /// comparisons. Read-only — does not allow privilege construction.
    pub fn class(&self) -> TrustClass {
        self.inner
    }

    /// True for `FirstParty` or `System`.
    pub fn is_privileged(&self) -> bool {
        matches!(self.inner, TrustClass::FirstParty | TrustClass::System)
    }
}

/// Where the effective trust came from. Recorded on every decision for audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TrustProvenance {
    /// Default fallback path: package matched no host policy entry.
    Default,
    /// Compiled-in or signed-bundled package recognized by the bundled
    /// registry source.
    Bundled,
    /// Operator-configured trust assignment.
    AdminConfig,
    /// Verified remote registry assignment.
    SignedRegistry { signer: String },
    /// Local user-installed manifest. Always caps below privileged.
    LocalManifest,
}

/// Maximum authority a downstream grant decision may issue.
///
/// PR1b ships a simple shape: an allowed-effects whitelist and an optional
/// resource ceiling. PR3 will compare proposed `CapabilityGrant`s against
/// this ceiling. Trust class on its own grants nothing — the ceiling is
/// purely a *cap*, not a permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorityCeiling {
    pub allowed_effects: Vec<EffectKind>,
    pub max_resource_ceiling: Option<ResourceCeiling>,
}

impl AuthorityCeiling {
    pub fn empty() -> Self {
        Self {
            allowed_effects: Vec::new(),
            max_resource_ceiling: None,
        }
    }

    pub fn allows_effect(&self, effect: &EffectKind) -> bool {
        self.allowed_effects.contains(effect)
    }
}

/// Output of [`crate::TrustPolicy::evaluate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrustDecision {
    pub effective_trust: EffectiveTrustClass,
    pub authority_ceiling: AuthorityCeiling,
    pub provenance: TrustProvenance,
    pub evaluated_at: Timestamp,
}
