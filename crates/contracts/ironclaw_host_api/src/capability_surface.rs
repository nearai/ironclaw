//! Neutral policy vocabulary for model-visible capability surfaces.
//!
//! A surface policy narrows visibility only. It does not grant invocation
//! authority or bypass authorization, approvals, obligations, or dispatch-time
//! checks.

use std::collections::BTreeSet;

use crate::{capability::EffectKind, ids::CapabilityId, runtime::RuntimeKind};

const ALL_RUNTIME_KINDS: &[RuntimeKind] = &[
    RuntimeKind::Wasm,
    RuntimeKind::Mcp,
    RuntimeKind::Script,
    RuntimeKind::Sandbox,
    RuntimeKind::FirstParty,
    RuntimeKind::System,
];

const ALL_EFFECT_KINDS: &[EffectKind] = &[
    EffectKind::ReadFilesystem,
    EffectKind::WriteFilesystem,
    EffectKind::DeleteFilesystem,
    EffectKind::Network,
    EffectKind::UseSecret,
    EffectKind::ExecuteCode,
    EffectKind::SpawnProcess,
    EffectKind::DispatchCapability,
    EffectKind::ModifyExtension,
    EffectKind::ModifyApproval,
    EffectKind::ModifyBudget,
    EffectKind::ExternalWrite,
    EffectKind::Financial,
];

/// Capability-id dimension of a [`CapabilitySurfacePolicy`].
///
/// The default is fail-closed: no capability ids are visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityIdScope {
    Only(BTreeSet<CapabilityId>),
    AllExcept(BTreeSet<CapabilityId>),
}

impl Default for CapabilityIdScope {
    fn default() -> Self {
        Self::Only(BTreeSet::new())
    }
}

impl CapabilityIdScope {
    pub fn only(ids: impl IntoIterator<Item = CapabilityId>) -> Self {
        Self::Only(ids.into_iter().collect())
    }

    pub fn permits(&self, id: &CapabilityId) -> bool {
        match self {
            Self::Only(allowed) => allowed.contains(id),
            Self::AllExcept(denied) => !denied.contains(id),
        }
    }

    pub fn intersect(self, other: Self) -> Self {
        match (self, other) {
            (Self::Only(left), Self::Only(right)) => {
                Self::Only(left.intersection(&right).cloned().collect())
            }
            (Self::Only(mut allowed), Self::AllExcept(denied))
            | (Self::AllExcept(denied), Self::Only(mut allowed)) => {
                allowed.retain(|id| !denied.contains(id));
                Self::Only(allowed)
            }
            (Self::AllExcept(mut left), Self::AllExcept(right)) => {
                left.extend(right);
                Self::AllExcept(left)
            }
        }
    }

    pub fn without(self, denied: impl IntoIterator<Item = CapabilityId>) -> Self {
        let denied: BTreeSet<_> = denied.into_iter().collect();
        match self {
            Self::Only(mut allowed) => {
                allowed.retain(|id| !denied.contains(id));
                Self::Only(allowed)
            }
            Self::AllExcept(mut existing) => {
                existing.extend(denied);
                Self::AllExcept(existing)
            }
        }
    }
}

/// Visibility-only ceiling applied before capability metadata reaches a model.
///
/// This is neutral policy vocabulary, not an authority source. Missing grants,
/// denied trust ceilings, authorization decisions, approvals, and action-time
/// checks may narrow the resulting surface further.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySurfacePolicy {
    /// Capability ids that may appear on the surface. The default allows none.
    pub capability_ids: CapabilityIdScope,
    /// Runtime kinds that may appear on the surface. Empty allows none.
    pub allowed_runtimes: Vec<RuntimeKind>,
    /// Every effect declared by a visible capability must appear here. Empty
    /// allows none.
    pub allowed_effects: Vec<EffectKind>,
    /// Whether capabilities requiring approval may be rendered as askable.
    pub include_requires_approval: bool,
    /// Maximum returned capabilities in registry order.
    pub max_capabilities: Option<usize>,
}

impl CapabilitySurfacePolicy {
    pub fn allow_all() -> Self {
        Self {
            capability_ids: CapabilityIdScope::AllExcept(BTreeSet::new()),
            allowed_runtimes: ALL_RUNTIME_KINDS.to_vec(),
            allowed_effects: ALL_EFFECT_KINDS.to_vec(),
            include_requires_approval: true,
            max_capabilities: None,
        }
    }

    pub fn allow_only(ids: impl IntoIterator<Item = CapabilityId>) -> Self {
        Self::allow_all().with_capability_ids(CapabilityIdScope::only(ids))
    }

    pub fn with_capability_ids(mut self, capability_ids: CapabilityIdScope) -> Self {
        self.capability_ids = capability_ids;
        self
    }

    pub fn permits_capability_id(&self, id: &CapabilityId) -> bool {
        self.capability_ids.permits(id)
    }

    pub fn allows_runtime(&self, runtime: RuntimeKind) -> bool {
        self.allowed_runtimes.contains(&runtime)
    }

    pub fn allows_effects(&self, effects: &[EffectKind]) -> bool {
        effects
            .iter()
            .all(|effect| self.allowed_effects.contains(effect))
    }

    pub fn narrow_to_capability_ids(
        mut self,
        allowed: impl IntoIterator<Item = CapabilityId>,
    ) -> Self {
        self.capability_ids = self
            .capability_ids
            .intersect(CapabilityIdScope::only(allowed));
        self
    }

    pub fn deny_capability_ids(mut self, denied: impl IntoIterator<Item = CapabilityId>) -> Self {
        self.capability_ids = self.capability_ids.without(denied);
        self
    }

    /// Drop capabilities whose authorization resolves to `RequireApproval`
    /// from the surface, instead of rendering them as askable.
    ///
    /// For an unattended run there is nobody to answer the prompt, so an
    /// askable capability is a run that parks rather than one that asks.
    pub fn without_approval_gated(mut self) -> Self {
        self.include_requires_approval = false;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability_id(value: &str) -> CapabilityId {
        CapabilityId::new(value).expect("test capability id is valid")
    }

    #[test]
    fn default_policy_is_fail_closed() {
        assert!(
            !CapabilitySurfacePolicy::default()
                .permits_capability_id(&capability_id("demo.capability"))
        );
    }

    #[test]
    fn allow_only_then_deny_resolves_to_one_scope() {
        let allowed = capability_id("demo.allowed");
        let denied = capability_id("demo.denied");
        let policy = CapabilitySurfacePolicy::allow_only([allowed.clone(), denied.clone()])
            .deny_capability_ids([denied.clone()]);

        assert!(policy.permits_capability_id(&allowed));
        assert!(!policy.permits_capability_id(&denied));
        assert_eq!(policy.capability_ids, CapabilityIdScope::only([allowed]));
    }

    #[test]
    fn all_minus_denies_is_fail_closed_for_denied_ids() {
        let denied = capability_id("demo.denied");
        let allowed = capability_id("demo.allowed");
        let policy = CapabilitySurfacePolicy::allow_all().deny_capability_ids([denied.clone()]);

        assert!(!policy.permits_capability_id(&denied));
        assert!(policy.permits_capability_id(&allowed));
    }

    #[test]
    fn narrowing_two_only_scopes_intersects_them() {
        let shared = capability_id("demo.shared");
        let base_only = capability_id("demo.base-only");
        let child_only = capability_id("demo.child-only");
        let policy = CapabilitySurfacePolicy::allow_only([shared.clone(), base_only])
            .narrow_to_capability_ids([shared.clone(), child_only]);

        assert_eq!(policy.capability_ids, CapabilityIdScope::only([shared]));
    }

    #[test]
    fn without_approval_gated_stops_rendering_askable_capabilities() {
        let policy = CapabilitySurfacePolicy::allow_all();
        assert!(policy.include_requires_approval);

        let policy = policy.without_approval_gated();

        assert!(!policy.include_requires_approval);
    }
}
