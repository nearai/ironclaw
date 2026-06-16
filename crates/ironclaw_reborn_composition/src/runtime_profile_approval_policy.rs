use ironclaw_host_api::{
    CapabilityId, EffectKind,
    runtime_policy::{ApprovalPolicy, RuntimeProfile},
};

use crate::profile_approval_authorization::ProfileApprovalGatePolicy;

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
    resolved_profile: RuntimeProfile,
    effects: RuntimeProfileApprovalGateEffectSets,
    exempt_capabilities: Vec<CapabilityId>,
}

impl RuntimeProfileApprovalGatePolicy {
    pub(crate) fn new(
        resolved_profile: RuntimeProfile,
        effects: RuntimeProfileApprovalGateEffectSets,
    ) -> Self {
        Self {
            resolved_profile,
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
        self.resolved_profile.allows_minimal_approval_bypass()
    }
}

impl ProfileApprovalGatePolicy for RuntimeProfileApprovalGatePolicy {
    fn capability_exempt_from_approval(&self, capability: &CapabilityId) -> bool {
        self.exempt_capabilities.contains(capability)
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
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{
        EffectKind,
        runtime_policy::{ApprovalPolicy, RuntimeProfile},
    };

    use super::*;

    fn policy(profile: RuntimeProfile) -> RuntimeProfileApprovalGatePolicy {
        RuntimeProfileApprovalGatePolicy::new(
            profile,
            RuntimeProfileApprovalGateEffectSets::new(
                vec![EffectKind::WriteFilesystem, EffectKind::SpawnProcess],
                vec![EffectKind::SpawnProcess],
            ),
        )
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
