use ironclaw_host_api::{
    CapabilityId, EffectKind, InvocationOrigin, OriginGateMatrix, OriginGatePolicy,
    runtime_policy::ApprovalPolicy,
};
use ironclaw_runtime_policy::MinimalApprovalBypass;

use crate::profile_approval_authorization::{OriginGateRequirement, ProfileApprovalGatePolicy};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProfileApprovalGateEffectSets {
    pub(crate) ask_writes: Vec<EffectKind>,
    pub(crate) ask_destructive: Vec<EffectKind>,
}

impl RuntimeProfileApprovalGateEffectSets {
    pub(crate) fn new(ask_writes: Vec<EffectKind>, ask_destructive: Vec<EffectKind>) -> Self {
        Self {
            ask_writes,
            ask_destructive,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProfileApprovalGatePolicy {
    /// Whether `ApprovalPolicy::Minimal` may bypass effect gates, as a
    /// resolved policy *value* — not a deployment profile this type then asks
    /// about itself (§4.4). `ironclaw_runtime_policy::minimal_approval_bypass`
    /// is the one place that classification lives.
    minimal_bypass: MinimalApprovalBypass,
    effects: RuntimeProfileApprovalGateEffectSets,
    exempt_capabilities: Vec<CapabilityId>,
}

impl RuntimeProfileApprovalGatePolicy {
    pub(crate) fn new(
        minimal_bypass: MinimalApprovalBypass,
        effects: RuntimeProfileApprovalGateEffectSets,
    ) -> Self {
        Self {
            minimal_bypass,
            effects,
            exempt_capabilities: Vec::new(),
        }
    }

    pub(crate) fn with_exempt_capabilities(
        mut self,
        exempt_capabilities: Vec<CapabilityId>,
    ) -> Self {
        self.exempt_capabilities = exempt_capabilities;
        self
    }

    fn profile_allows_minimal_bypass(&self) -> bool {
        self.minimal_bypass == MinimalApprovalBypass::Allowed
    }
}

impl ProfileApprovalGatePolicy for RuntimeProfileApprovalGatePolicy {
    fn capability_exempt_from_approval(&self, capability: &CapabilityId) -> bool {
        self.exempt_capabilities.contains(capability)
    }

    fn effects_force_approval(&self, effects: &[EffectKind]) -> bool {
        // Hard floor (#4776): the highest-risk effects always require an
        // explicit approval gate and can never be auto-approved or satisfied by
        // a stored always-allow grant — independent of profile or policy. Kept
        // deliberately narrow so the yolo/Minimal-bypass behaviour for ordinary
        // write/spawn effects is unchanged.
        effects.iter().any(|effect| {
            matches!(
                effect,
                EffectKind::Financial | EffectKind::ModifyApproval | EffectKind::ModifyBudget
            )
        })
    }

    fn effects_require_approval(
        &self,
        approval_policy: ApprovalPolicy,
        effects: &[EffectKind],
    ) -> bool {
        match approval_policy {
            ApprovalPolicy::Minimal => !self.profile_allows_minimal_bypass() && !effects.is_empty(),
            ApprovalPolicy::AskAlways => !effects.is_empty(),
            ApprovalPolicy::AskWrites => effects
                .iter()
                .any(|effect| self.effects.ask_writes.contains(effect)),
            ApprovalPolicy::AskDestructive => effects
                .iter()
                .any(|effect| self.effects.ask_destructive.contains(effect)),
            ApprovalPolicy::OrgPolicy => !effects.is_empty(),
            // Any future ApprovalPolicy variants default to fail-safe: require
            // approval for non-empty effects rather than silently disabling gates.
            _ => !effects.is_empty(),
        }
    }

    fn origin_gate_requirement(
        &self,
        approval_policy: ApprovalPolicy,
        origin: Option<&InvocationOrigin>,
        matrix: Option<&OriginGateMatrix>,
    ) -> OriginGateRequirement {
        // No resolvable origin: a test-only context that stamped neither
        // `origin` nor `run_id`. The matrix is not consulted, so it contributes
        // nothing — this is the path that keeps pre-S4 decisions neutral.
        let Some(origin) = origin else {
            return OriginGateRequirement::None;
        };
        // Production descriptors always declare a matrix (S3). A missing matrix
        // with a real origin is fail-closed: `Forbidden` for every origin. Only
        // test fixtures leave it `None`, and those never stamp an origin, so
        // this Deny arm is unreachable outside deliberate fail-closed tests.
        let policy = match matrix {
            Some(matrix) => matrix.policy_for(origin),
            None => OriginGatePolicy::Forbidden,
        };
        match policy {
            // A forbidden origin is denied regardless of profile — NOT
            // suppressed by the Minimal (yolo) bypass.
            OriginGatePolicy::Forbidden => OriginGateRequirement::Deny,
            // `AskAlways` is a hard-floor gate (§5.2.7): every invocation gates
            // and no stored auto-approve/always-allow can bypass it. Like
            // `effects_force_approval` (which fires for Financial/ModifyApproval/
            // ModifyBudget even under yolo), it is NOT suppressed by the Minimal
            // bypass — AskAlways gates even in yolo.
            OriginGatePolicy::AskAlways => OriginGateRequirement::GateHardFloor,
            // `GatedUnlessGranted` is a soft gate satisfied by a scoped
            // persistent/policy grant. Suppress it behind the SAME Minimal-bypass
            // guard `effects_require_approval` uses (see the
            // `ApprovalPolicy::Minimal` arm above), so yolo stays "no prompts"
            // and the fold is behavior-neutral under it.
            OriginGatePolicy::GatedUnlessGranted => {
                if approval_policy == ApprovalPolicy::Minimal
                    && self.profile_allows_minimal_bypass()
                {
                    OriginGateRequirement::None
                } else {
                    OriginGateRequirement::GateSoft
                }
            }
            OriginGatePolicy::ConsentSufficient | OriginGatePolicy::Ungated => {
                OriginGateRequirement::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{
        EffectKind,
        runtime_policy::{ApprovalPolicy, DeploymentMode, RuntimeProfile},
    };
    use ironclaw_runtime_policy::{OrgPolicyConstraints, ResolveRequest};

    use super::*;

    /// Build the gate policy the way production does: resolve the profile
    /// through the sanctioned resolver, then classify the resolved policy.
    /// Driving the real resolver here (rather than hand-picking a bypass
    /// value) keeps this suite honest about which profiles actually reach
    /// `MinimalApprovalBypass::Allowed`.
    fn policy(profile: RuntimeProfile) -> RuntimeProfileApprovalGatePolicy {
        RuntimeProfileApprovalGatePolicy::new(
            bypass_for(profile),
            RuntimeProfileApprovalGateEffectSets::new(
                vec![EffectKind::WriteFilesystem, EffectKind::SpawnProcess],
                vec![EffectKind::SpawnProcess],
            ),
        )
    }

    fn bypass_for(profile: RuntimeProfile) -> MinimalApprovalBypass {
        let deployment = match profile {
            RuntimeProfile::HostedSafe
            | RuntimeProfile::HostedDev
            | RuntimeProfile::HostedYoloTenantScoped => DeploymentMode::HostedMultiTenant,
            RuntimeProfile::EnterpriseSafe
            | RuntimeProfile::EnterpriseDev
            | RuntimeProfile::EnterpriseYoloDedicated => DeploymentMode::EnterpriseDedicated,
            _ => DeploymentMode::LocalSingleUser,
        };
        let resolved = ironclaw_runtime_policy::resolve(ResolveRequest {
            yolo_disclosure_acknowledged: true,
            org_policy: OrgPolicyConstraints::default().set_admin_approves_dedicated_yolo(true),
            ..ResolveRequest::new(deployment, profile)
        })
        .expect("test profile resolves");
        ironclaw_runtime_policy::minimal_approval_bypass(&resolved)
    }

    #[test]
    fn hard_floor_forces_approval_for_high_risk_effects_on_real_policy() {
        // The production gate policy must hard-floor these even under the most
        // permissive profile (yolo), so global auto-approve / always-allow can
        // never bypass them.
        let p = policy(RuntimeProfile::LocalYolo);
        for effect in [
            EffectKind::Financial,
            EffectKind::ModifyApproval,
            EffectKind::ModifyBudget,
        ] {
            assert!(
                p.effects_force_approval(&[effect]),
                "{effect:?} must be hard-floored by the production policy"
            );
        }
        // Ordinary effects and the empty set are not floored.
        assert!(!p.effects_force_approval(&[EffectKind::WriteFilesystem]));
        assert!(!p.effects_force_approval(&[EffectKind::SpawnProcess]));
        assert!(!p.effects_force_approval(&[]));
    }

    #[test]
    fn minimal_only_disables_gates_for_local_and_hosted_yolo_profiles() {
        for profile in [
            RuntimeProfile::LocalYolo,
            RuntimeProfile::HostedYoloTenantScoped,
        ] {
            assert!(
                !policy(profile)
                    .effects_require_approval(ApprovalPolicy::Minimal, &[EffectKind::SpawnProcess]),
                "{profile:?} should allow Minimal to bypass approval gates"
            );
        }

        for profile in [
            RuntimeProfile::SecureDefault,
            RuntimeProfile::LocalSafe,
            RuntimeProfile::LocalDev,
            RuntimeProfile::HostedSafe,
            RuntimeProfile::HostedDev,
            RuntimeProfile::EnterpriseSafe,
            RuntimeProfile::EnterpriseDev,
            RuntimeProfile::EnterpriseYoloDedicated,
            RuntimeProfile::Sandboxed,
            RuntimeProfile::Experiment,
        ] {
            assert!(
                policy(profile).effects_require_approval(
                    ApprovalPolicy::Minimal,
                    &[EffectKind::DispatchCapability]
                ),
                "{profile:?} should fail closed if Minimal reaches a non-minimal profile"
            );
        }
    }

    #[test]
    fn org_ceiling_narrowing_yolo_away_restores_minimal_approval_gates() {
        // Regression for the mode-as-type leak (§4.4): the gate policy used to
        // hold a `RuntimeProfile` and ask it about itself. It now consumes a
        // resolved value, so a tenant/org ceiling that narrows `LocalYolo`
        // down to `LocalDev` also re-gates `Minimal` — authority reductions
        // reach this axis instead of stopping at the requested profile.
        let narrowed = ironclaw_runtime_policy::resolve(ResolveRequest {
            yolo_disclosure_acknowledged: true,
            org_policy: OrgPolicyConstraints::default().set_max_profile(RuntimeProfile::LocalDev),
            ..ResolveRequest::new(DeploymentMode::LocalSingleUser, RuntimeProfile::LocalYolo)
        })
        .expect("narrowed local yolo resolves");
        assert!(narrowed.was_reduced());

        let gate_policy = RuntimeProfileApprovalGatePolicy::new(
            ironclaw_runtime_policy::minimal_approval_bypass(&narrowed),
            RuntimeProfileApprovalGateEffectSets::new(
                vec![EffectKind::WriteFilesystem, EffectKind::SpawnProcess],
                vec![EffectKind::SpawnProcess],
            ),
        );
        assert!(
            gate_policy
                .effects_require_approval(ApprovalPolicy::Minimal, &[EffectKind::SpawnProcess]),
            "an org ceiling that removes yolo must restore Minimal approval gates"
        );
    }

    #[test]
    fn hosted_dev_ask_destructive_gates_process_but_not_read_only_effects() {
        let policy = policy(RuntimeProfile::HostedDev);

        assert!(
            policy.effects_require_approval(
                ApprovalPolicy::AskDestructive,
                &[EffectKind::SpawnProcess]
            )
        );
        assert!(!policy.effects_require_approval(
            ApprovalPolicy::AskDestructive,
            &[EffectKind::ReadFilesystem]
        ));
    }

    #[test]
    fn hosted_safe_ask_writes_gates_writes_but_not_read_only_effects() {
        let policy = policy(RuntimeProfile::HostedSafe);

        assert!(
            policy.effects_require_approval(
                ApprovalPolicy::AskWrites,
                &[EffectKind::WriteFilesystem]
            )
        );
        assert!(
            !policy
                .effects_require_approval(ApprovalPolicy::AskWrites, &[EffectKind::ReadFilesystem])
        );
    }

    #[test]
    fn secure_default_ask_always_gates_read_only_effects() {
        let policy = policy(RuntimeProfile::SecureDefault);

        assert!(
            policy
                .effects_require_approval(ApprovalPolicy::AskAlways, &[EffectKind::ReadFilesystem])
        );
    }

    #[test]
    fn enterprise_yolo_dedicated_org_policy_still_gates_effectful_actions() {
        let policy = policy(RuntimeProfile::EnterpriseYoloDedicated);

        assert!(policy.effects_require_approval(
            ApprovalPolicy::OrgPolicy,
            &[EffectKind::DispatchCapability]
        ));
    }
}
