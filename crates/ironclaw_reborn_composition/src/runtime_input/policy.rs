//! Runtime policy inputs for the assembled Reborn runtime.
//!
//! `RebornPolicyConfig` carries deployment mode, default runtime profile,
//! and approval policy. The values surface here so that:
//!
//! - The composition root can fail closed if a caller picks a runtime
//!   profile incompatible with their deployment mode (e.g.
//!   `RuntimeProfile::LocalDev` under `DeploymentMode::HostedMultiTenant`).
//! - Today's stubbed runtime can still log the chosen policy on boot
//!   ("running under DeploymentMode=LocalSingleUser,
//!   default_profile=LocalDev, approval_policy=AskDestructive") even
//!   though approval gating itself isn't wired through the loop yet.
//! - The same DTO field will become the **boot-time** view of the value
//!   once the policy repo (`RuntimePolicyRepo`, epic #3036) lands; the
//!   runtime starts using repo lookups at request time and the input
//!   here narrows to "initial value to write if no repo entry exists".
//!
//! See `docs/reborn/contracts/runtime-profiles.md` for the
//! `DeploymentMode + RuntimeProfile -> EffectiveRuntimePolicy` resolution
//! rules these values feed into.

use ironclaw_host_api::runtime_policy::{ApprovalPolicy, DeploymentMode, RuntimeProfile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornPolicyConfig {
    /// Authority ceiling for the host process. Determines which runtime
    /// profiles the resolver may accept.
    pub deployment_mode: DeploymentMode,

    /// Default runtime profile used when a submitted turn doesn't request
    /// one (and the request-time policy resolver isn't yet wired).
    pub default_profile: RuntimeProfile,

    /// Default approval policy applied to effectful capability invocations.
    /// Reborn does not yet enforce this in the composed loop driver — it's
    /// recorded so audits and boot logs surface the operator's intent.
    pub default_approval_policy: ApprovalPolicy,
}

impl RebornPolicyConfig {
    /// Single-user local dev defaults. Matches the values an operator
    /// running `ironclaw-reborn run` on a laptop would expect.
    pub fn cli_default() -> Self {
        Self {
            deployment_mode: DeploymentMode::LocalSingleUser,
            default_profile: RuntimeProfile::LocalDev,
            default_approval_policy: ApprovalPolicy::AskDestructive,
        }
    }

    pub fn with_deployment_mode(mut self, mode: DeploymentMode) -> Self {
        self.deployment_mode = mode;
        self
    }

    pub fn with_default_profile(mut self, profile: RuntimeProfile) -> Self {
        self.default_profile = profile;
        self
    }

    pub fn with_approval_policy(mut self, policy: ApprovalPolicy) -> Self {
        self.default_approval_policy = policy;
        self
    }
}

impl Default for RebornPolicyConfig {
    fn default() -> Self {
        Self::cli_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_default_is_local_single_user_localdev_askdestructive() {
        let policy = RebornPolicyConfig::cli_default();
        assert_eq!(policy.deployment_mode, DeploymentMode::LocalSingleUser);
        assert_eq!(policy.default_profile, RuntimeProfile::LocalDev);
        assert_eq!(
            policy.default_approval_policy,
            ApprovalPolicy::AskDestructive
        );
    }

    #[test]
    fn builders_compose() {
        let policy = RebornPolicyConfig::cli_default()
            .with_deployment_mode(DeploymentMode::HostedMultiTenant)
            .with_default_profile(RuntimeProfile::HostedSafe)
            .with_approval_policy(ApprovalPolicy::AskWrites);
        assert_eq!(policy.deployment_mode, DeploymentMode::HostedMultiTenant);
        assert_eq!(policy.default_profile, RuntimeProfile::HostedSafe);
        assert_eq!(policy.default_approval_policy, ApprovalPolicy::AskWrites);
    }
}
