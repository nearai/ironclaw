//! Effective trust + authority-ceiling output of a trust policy evaluation.
//!
//! [`EffectiveTrustClass`] wraps [`ironclaw_host_api::TrustClass`] so the
//! privileged variants (`FirstParty`, `System`) are only constructible from
//! inside this crate. Downstream authorization code that requires a
//! policy-validated trust ceiling consumes `EffectiveTrustClass`, not
//! `TrustClass` — host_api's `#[serde(skip_deserializing)]` guards the wire
//! boundary, this newtype guards the in-process construction boundary.

use ironclaw_host_api::{EffectKind, ResourceCeiling, SandboxQuota, Timestamp, TrustClass};
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
    #[allow(dead_code)]
    pub(crate) fn first_party() -> Self {
        Self {
            inner: TrustClass::FirstParty,
        }
    }

    /// System privilege. Constructible only from inside the trust crate.
    #[allow(dead_code)]
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

    /// Authority level for ordering trust classes.
    ///
    /// Higher value ⇒ more authority. Used by [`crate::TrustChange`] to
    /// distinguish downgrades, upgrades, and sideways "kind changes":
    ///
    /// | Class         | Level |
    /// |---------------|-------|
    /// | `Sandbox`     |   0   |
    /// | `UserTrusted` |   1   |
    /// | `FirstParty`  |   2   |
    /// | `System`      |   2   |
    ///
    /// `FirstParty` and `System` share level `2` because both are
    /// privileged but represent *different kinds* of privilege
    /// (host-blessed extension vs host-owned service). A change between
    /// them is neither a downgrade nor an upgrade — it's a kind change,
    /// and listeners that scope grants to a specific privilege kind must
    /// still revoke. See [`crate::TrustChange::is_kind_change`].
    pub fn authority_level(&self) -> u8 {
        match self.inner {
            TrustClass::Sandbox => 0,
            TrustClass::UserTrusted => 1,
            TrustClass::FirstParty | TrustClass::System => 2,
        }
    }
}

/// Host-controlled trust assignment used to seed policy-source entries.
///
/// This is deliberately distinct from [`EffectiveTrustClass`]: downstream
/// host wiring may build bundled/admin policy entries from trusted bundle
/// metadata or operator config, but authorization still consumes only the
/// `EffectiveTrustClass` that comes out of [`crate::TrustPolicy::evaluate`].
/// The type does not implement `Deserialize`, so untrusted manifests cannot
/// deserialize directly into a privileged assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostTrustAssignment {
    inner: TrustClass,
}

impl HostTrustAssignment {
    pub fn sandbox() -> Self {
        Self {
            inner: TrustClass::Sandbox,
        }
    }

    pub fn user_trusted() -> Self {
        Self {
            inner: TrustClass::UserTrusted,
        }
    }

    pub fn first_party() -> Self {
        Self {
            inner: TrustClass::FirstParty,
        }
    }

    pub fn system() -> Self {
        Self {
            inner: TrustClass::System,
        }
    }

    pub fn class(&self) -> TrustClass {
        self.inner
    }

    pub(crate) fn into_effective(self) -> EffectiveTrustClass {
        EffectiveTrustClass { inner: self.inner }
    }
}

impl From<EffectiveTrustClass> for HostTrustAssignment {
    fn from(value: EffectiveTrustClass) -> Self {
        Self {
            inner: value.class(),
        }
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

    /// Returns true when `self` is a stricter ceiling than `previous`.
    ///
    /// This is the invalidation-relevant direction: grants issued under
    /// `previous` may now exceed what `self` allows. Expansions are not
    /// reductions and do not require revoking existing grants.
    pub fn is_reduction_from(&self, previous: &Self) -> bool {
        previous
            .allowed_effects
            .iter()
            .any(|effect| !self.allowed_effects.contains(effect))
            || resource_ceiling_reduced(&previous.max_resource_ceiling, &self.max_resource_ceiling)
    }
}

fn resource_ceiling_reduced(
    previous: &Option<ResourceCeiling>,
    current: &Option<ResourceCeiling>,
) -> bool {
    match (previous, current) {
        (None, Some(_)) => true,
        (None, None) | (Some(_), None) => false,
        (Some(previous), Some(current)) => {
            limit_reduced(&previous.max_usd, &current.max_usd)
                || limit_reduced(&previous.max_input_tokens, &current.max_input_tokens)
                || limit_reduced(&previous.max_output_tokens, &current.max_output_tokens)
                || limit_reduced(&previous.max_wall_clock_ms, &current.max_wall_clock_ms)
                || limit_reduced(&previous.max_output_bytes, &current.max_output_bytes)
                || sandbox_quota_reduced(&previous.sandbox, &current.sandbox)
        }
    }
}

fn sandbox_quota_reduced(previous: &Option<SandboxQuota>, current: &Option<SandboxQuota>) -> bool {
    match (previous, current) {
        (None, Some(_)) => true,
        (None, None) | (Some(_), None) => false,
        (Some(previous), Some(current)) => {
            limit_reduced(&previous.cpu_time_ms, &current.cpu_time_ms)
                || limit_reduced(&previous.memory_bytes, &current.memory_bytes)
                || limit_reduced(&previous.disk_bytes, &current.disk_bytes)
                || limit_reduced(
                    &previous.network_egress_bytes,
                    &current.network_egress_bytes,
                )
                || limit_reduced(&previous.process_count, &current.process_count)
        }
    }
}

fn limit_reduced<T: PartialOrd>(previous: &Option<T>, current: &Option<T>) -> bool {
    match (previous, current) {
        (None, Some(_)) => true,
        (Some(previous), Some(current)) => current < previous,
        (None, None) | (Some(_), None) => false,
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
