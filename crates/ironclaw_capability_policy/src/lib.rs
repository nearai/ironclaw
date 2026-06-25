//! Capability policy vocabulary for IronClaw Reborn — the continuation of
//! [nearai/ironclaw#4628] on the Reborn stack.
//!
//! This crate defines the **four-dimension capability policy** model from
//! `docs/plans/2026-06-24-capability-policy-architecture.md` and the
//! **precedence cascade** that resolves a per-capability default plus sparse
//! scope deltas into an [`EffectivePolicy`]:
//!
//! ```text
//! capability default -> tenant baseline -> (project) -> user   (most specific wins)
//! ```
//!
//! The four dimensions are:
//!
//! - **Availability** — can a principal see/invoke it ([`Availability`]).
//! - **Configuration** — admin-owned settings, deep-merged down the cascade.
//! - **Identity** — whose credential the call uses ([`IdentityMode`]:
//!   user-keyed vs admin-keyed).
//! - **Approval** — prompt / always-allow / deny, carried as the existing
//!   [`PermissionMode`] so there is no third approval vocabulary. (The WebUI's
//!   `CapabilityPermissionState` is the surface representation; the
//!   `PermissionMode -> CapabilityPermissionState` mapping belongs at that
//!   boundary, not here.)
//!
//! ## Scope of this crate
//!
//! It is intentionally **storage-free and dependency-light** (only
//! `ironclaw_host_api`): the pure [`resolve_effective_policy`] fold has no I/O,
//! and the [`PolicyResolver`] port is the seam a store-backed adapter fills.
//! That adapter — which sources availability from #4544's effective scoped
//! lifecycle installations, identity from credential ownership, and approval
//! from the approvals store, then applies this fold — is the integration step
//! that finalizes #4544. Resolution must be **live** (never cached across a
//! request; see architecture doc §8).
//!
//! [nearai/ironclaw#4628]: https://github.com/nearai/ironclaw/issues/4628

use std::collections::HashMap;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, PermissionMode, ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Availability dimension: can a principal see and invoke a capability?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Hidden,
}

impl Availability {
    /// `true` only for [`Availability::Available`].
    pub fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Identity dimension: who provides the credential a capability runs under.
///
/// - `None` — no credential needed.
/// - `UserKeyed` — the user supplies their own key ("introduce yourself"); a
///   missing key triggers an auth gate.
/// - `AdminKeyed` — an admin supplies a shared key; the user uses it but cannot
///   set it. A missing key resolves to *unavailable*.
///
/// Maps to `ironclaw_auth::CredentialOwnership` (`UserReusable` /
/// `SharedAdminManaged`) in the credential-binding layer; kept independent here
/// so this crate carries no auth dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityMode {
    None,
    UserKeyed,
    AdminKeyed,
}

/// The per-capability default policy, declared by the capability manifest
/// (architecture doc §7). `approval` mirrors the descriptor's existing
/// `default_permission`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDefaultPolicy {
    pub availability: Availability,
    pub identity: IdentityMode,
    pub approval: PermissionMode,
    /// Default configuration the capability runs with; admin deltas deep-merge
    /// on top.
    #[serde(default)]
    pub config: Value,
}

/// Precedence scope of a [`CapabilityPolicyDelta`]. Higher rank overrides lower.
/// For v1 the `Tenant` scope is the default project (architecture doc §1, §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyScope {
    Tenant,
    Project { project_id: ProjectId },
    User { user_id: UserId },
}

impl PolicyScope {
    /// Precedence rank; larger wins. The capability default is the implicit
    /// rank 0.
    fn rank(&self) -> u8 {
        match self {
            Self::Tenant => 1,
            Self::Project { .. } => 2,
            Self::User { .. } => 3,
        }
    }
}

/// A sparse override at one scope. Every dimension field is optional; an absent
/// field inherits the layer above.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityPolicyDelta {
    pub scope: PolicyScope,
    pub capability: CapabilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<Availability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<IdentityMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<PermissionMode>,
    /// Deep-merged into the accumulated config in precedence order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_patch: Option<Value>,
}

/// The resolved policy for one `(subject, capability)` pair at dispatch time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectivePolicy {
    /// Availability resolved to a single boolean for the dispatch allow-set.
    pub available: bool,
    pub identity: IdentityMode,
    pub approval: PermissionMode,
    pub config: Value,
}

/// Pure precedence fold (architecture doc §6, §8). Applies the capability
/// default, then the `deltas` in scope-rank order (`Tenant -> Project ->
/// User`, most specific wins). `config` deep-merges; the other dimensions
/// replace. No I/O.
///
/// Deltas are expected to be **pre-filtered to the subject** (the store selects
/// the relevant tenant/project/user rows before calling this); the fold is
/// order-independent because it sorts by scope rank.
pub fn resolve_effective_policy(
    default: &CapabilityDefaultPolicy,
    deltas: &[CapabilityPolicyDelta],
) -> EffectivePolicy {
    let mut availability = default.availability;
    let mut identity = default.identity;
    let mut approval = default.approval;
    let mut config = default.config.clone();

    let mut ordered: Vec<&CapabilityPolicyDelta> = deltas.iter().collect();
    ordered.sort_by_key(|delta| delta.scope.rank());

    for delta in ordered {
        if let Some(value) = delta.availability {
            availability = value;
        }
        if let Some(value) = delta.identity {
            identity = value;
        }
        if let Some(value) = delta.approval {
            approval = value;
        }
        if let Some(patch) = &delta.config_patch {
            deep_merge(&mut config, patch);
        }
    }

    EffectivePolicy {
        available: availability.is_available(),
        identity,
        approval,
        config,
    }
}

/// Deep-merge `patch` into `base`: objects merge key-by-key (recursively); any
/// non-object value (including arrays and `null`) replaces. Mirrors the
/// "admin owns the keys, lower scopes tweak" semantics (architecture doc §6).
fn deep_merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (key, value) in patch_map {
                deep_merge(base_map.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (base_slot, patch_value) => {
            *base_slot = patch_value.clone();
        }
    }
}

/// The subject a policy resolves for: a `(tenant, user)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySubject {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

/// Errors a [`PolicyResolver`] may return. Sanitized — no backend internals.
#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("capability policy backend unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("capability policy resolution failed: {reason}")]
    Internal { reason: String },
}

/// Store-backed resolver port (architecture doc §11).
///
/// The implementation sources availability from #4544's effective scoped
/// lifecycle installations, layers tenant/project/user [`CapabilityPolicyDelta`]
/// rows via [`resolve_effective_policy`], and is consulted once per capability
/// dispatch behind the `CapabilitySurfaceProfileResolver` seam
/// (`crates/ironclaw_loop_support/src/capability_allow_set.rs`). Implementations
/// must resolve live and fail closed in production.
#[async_trait]
pub trait PolicyResolver: Send + Sync {
    async fn resolve(
        &self,
        subject: &PolicySubject,
        capability: &CapabilityId,
    ) -> Result<EffectivePolicy, PolicyError>;
}

impl CapabilityDefaultPolicy {
    /// A conservative global fallback for capabilities that declare no policy:
    /// hidden, no credential, ask before running. Fail-closed on availability.
    pub fn conservative_fallback() -> Self {
        Self {
            availability: Availability::Hidden,
            identity: IdentityMode::None,
            approval: PermissionMode::Ask,
            config: Value::Null,
        }
    }
}

/// Source of per-capability default policy (architecture doc §7).
///
/// The capability manifest is the source of truth; this trait decouples lookup
/// from where manifests are registered, so a per-capability default can be
/// sourced without adding a field to the 49-construction-site
/// `CapabilityDescriptor`. Capabilities that declare no policy fall back to a
/// conservative global default.
pub trait CapabilityDefaultPolicySource: Send + Sync {
    /// The default policy for `capability`, or the global fallback when the
    /// capability declares none.
    fn default_for(&self, capability: &CapabilityId) -> CapabilityDefaultPolicy;
}

/// In-memory [`CapabilityDefaultPolicySource`]: explicit per-capability entries
/// over a global fallback. Seeded from manifest declarations at composition
/// time.
#[derive(Debug, Clone)]
pub struct StaticCapabilityDefaultPolicySource {
    fallback: CapabilityDefaultPolicy,
    entries: HashMap<CapabilityId, CapabilityDefaultPolicy>,
}

impl StaticCapabilityDefaultPolicySource {
    /// Create a source with the given global fallback and no entries.
    pub fn new(fallback: CapabilityDefaultPolicy) -> Self {
        Self {
            fallback,
            entries: HashMap::new(),
        }
    }

    /// Builder-style insert of a per-capability default.
    #[must_use]
    pub fn with_entry(mut self, capability: CapabilityId, policy: CapabilityDefaultPolicy) -> Self {
        self.entries.insert(capability, policy);
        self
    }

    /// Insert or replace a per-capability default.
    pub fn insert(&mut self, capability: CapabilityId, policy: CapabilityDefaultPolicy) {
        self.entries.insert(capability, policy);
    }
}

impl CapabilityDefaultPolicySource for StaticCapabilityDefaultPolicySource {
    fn default_for(&self, capability: &CapabilityId) -> CapabilityDefaultPolicy {
        self.entries
            .get(capability)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cap() -> CapabilityId {
        CapabilityId::new("web.search").expect("valid capability id")
    }
    fn uid(value: &str) -> UserId {
        UserId::new(value).expect("valid user id")
    }
    fn pid(value: &str) -> ProjectId {
        ProjectId::new(value).expect("valid project id")
    }

    fn default_policy() -> CapabilityDefaultPolicy {
        CapabilityDefaultPolicy {
            availability: Availability::Available,
            identity: IdentityMode::None,
            approval: PermissionMode::Ask,
            config: json!({}),
        }
    }

    fn delta(scope: PolicyScope) -> CapabilityPolicyDelta {
        CapabilityPolicyDelta {
            scope,
            capability: cap(),
            availability: None,
            identity: None,
            approval: None,
            config_patch: None,
        }
    }

    #[test]
    fn default_source_returns_entry_then_fallback() {
        let entry = CapabilityDefaultPolicy {
            availability: Availability::Available,
            identity: IdentityMode::None,
            approval: PermissionMode::Allow,
            config: json!({}),
        };
        let source = StaticCapabilityDefaultPolicySource::new(
            CapabilityDefaultPolicy::conservative_fallback(),
        )
        .with_entry(cap(), entry.clone());
        assert_eq!(source.default_for(&cap()), entry);

        let unknown = CapabilityId::new("shell.exec").expect("valid capability id");
        assert_eq!(
            source.default_for(&unknown),
            CapabilityDefaultPolicy::conservative_fallback()
        );
    }

    #[test]
    fn conservative_fallback_is_hidden_and_ask() {
        let fallback = CapabilityDefaultPolicy::conservative_fallback();
        assert_eq!(fallback.availability, Availability::Hidden);
        assert_eq!(fallback.approval, PermissionMode::Ask);
        assert_eq!(fallback.identity, IdentityMode::None);
    }

    #[test]
    fn source_default_feeds_the_fold() {
        // An admin per-user grant flips the conservative hidden default to
        // available for one user.
        let source = StaticCapabilityDefaultPolicySource::new(
            CapabilityDefaultPolicy::conservative_fallback(),
        );
        let mut user = delta(PolicyScope::User {
            user_id: uid("bob"),
        });
        user.availability = Some(Availability::Available);
        let eff = resolve_effective_policy(&source.default_for(&cap()), &[user]);
        assert!(eff.available);
    }

    #[test]
    fn default_only_resolves_to_default() {
        let eff = resolve_effective_policy(&default_policy(), &[]);
        assert!(eff.available);
        assert_eq!(eff.identity, IdentityMode::None);
        assert_eq!(eff.approval, PermissionMode::Ask);
    }

    #[test]
    fn tenant_hides_then_user_grants_back() {
        let mut tenant = delta(PolicyScope::Tenant);
        tenant.availability = Some(Availability::Hidden);
        let mut user = delta(PolicyScope::User {
            user_id: uid("bob"),
        });
        user.availability = Some(Availability::Available);
        // User (rank 3) overrides Tenant (rank 1).
        let eff = resolve_effective_policy(&default_policy(), &[tenant, user]);
        assert!(eff.available);
    }

    #[test]
    fn most_specific_wins_is_order_independent() {
        let mut tenant = delta(PolicyScope::Tenant);
        tenant.availability = Some(Availability::Hidden);
        tenant.approval = Some(PermissionMode::Deny);
        let mut user = delta(PolicyScope::User {
            user_id: uid("bob"),
        });
        user.availability = Some(Availability::Available);
        user.approval = Some(PermissionMode::Allow);

        let forward = resolve_effective_policy(&default_policy(), &[tenant.clone(), user.clone()]);
        let reverse = resolve_effective_policy(&default_policy(), &[user, tenant]);

        assert_eq!(forward, reverse);
        assert!(forward.available);
        assert_eq!(forward.approval, PermissionMode::Allow);
    }

    #[test]
    fn project_sits_between_tenant_and_user() {
        let mut tenant = delta(PolicyScope::Tenant);
        tenant.availability = Some(Availability::Hidden);
        let mut project = delta(PolicyScope::Project {
            project_id: pid("eng"),
        });
        project.availability = Some(Availability::Available);
        // Project (rank 2) overrides Tenant (rank 1); no user delta present.
        let eff = resolve_effective_policy(&default_policy(), &[tenant, project]);
        assert!(eff.available);
    }

    #[test]
    fn config_deep_merges_in_precedence_order() {
        let mut def = default_policy();
        def.config = json!({ "workspace": "base", "nested": { "a": 1 } });

        let mut tenant = delta(PolicyScope::Tenant);
        tenant.config_patch = Some(json!({ "workspace": "acme", "nested": { "b": 2 } }));
        let mut user = delta(PolicyScope::User {
            user_id: uid("bob"),
        });
        user.config_patch = Some(json!({ "nested": { "a": 9 } }));

        let eff = resolve_effective_policy(&def, &[tenant, user]);
        assert_eq!(
            eff.config,
            json!({ "workspace": "acme", "nested": { "a": 9, "b": 2 } })
        );
    }

    #[test]
    fn identity_mode_is_overridden() {
        let mut tenant = delta(PolicyScope::Tenant);
        tenant.identity = Some(IdentityMode::AdminKeyed);
        let eff = resolve_effective_policy(&default_policy(), &[tenant]);
        assert_eq!(eff.identity, IdentityMode::AdminKeyed);
    }

    #[test]
    fn admin_keyed_default_stays_when_no_delta() {
        let mut def = default_policy();
        def.identity = IdentityMode::AdminKeyed;
        def.availability = Availability::Hidden;
        let eff = resolve_effective_policy(&def, &[]);
        assert_eq!(eff.identity, IdentityMode::AdminKeyed);
        assert!(!eff.available);
    }

    #[test]
    fn delta_wire_roundtrip_is_snake_case() {
        let mut d = delta(PolicyScope::User {
            user_id: uid("bob"),
        });
        d.availability = Some(Availability::Hidden);
        d.identity = Some(IdentityMode::UserKeyed);
        d.approval = Some(PermissionMode::Ask);
        d.config_patch = Some(json!({ "k": "v" }));

        let serialized = serde_json::to_string(&d).expect("serialize");
        assert!(serialized.contains("user_keyed"), "{serialized}");
        assert!(serialized.contains("hidden"), "{serialized}");

        let back: CapabilityPolicyDelta = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(back, d);
    }
}
