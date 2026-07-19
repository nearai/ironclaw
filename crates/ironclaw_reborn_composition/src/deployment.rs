//! Deployment configuration: a deployment mode is policy *data* resolved at
//! the composition edge, never a type the kernel or a substrate names.
//!
//! This is the Slice B artifact of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`
//! (§4.4 "Eliminate `Local*`", §5.6 "The deployment interface — modes are
//! data"). Each deployment target is one [`DeploymentConfig`] value built by a
//! named constructor; the difference between local-dev, local-dev-yolo, and
//! the hosted volume preview is readable on this one page as data.
//!
//! Two deliberate boundaries:
//!
//! - The sanctioned resolver in `ironclaw_runtime_policy` stays the **only**
//!   producer of [`EffectiveRuntimePolicy`]; [`DeploymentConfig::resolve`] is
//!   a thin adapter over [`ResolveRequest`], not a second policy engine.
//! - Storage roots, workspace paths, and connection pools are runtime
//!   *handles*, not deployment policy — they continue to ride
//!   `RebornStorageInput`. This value carries only the policy request.

use ironclaw_host_api::runtime_policy::{DeploymentMode, RuntimeProfile};
use ironclaw_runtime_policy::{
    EffectiveRuntimePolicy, OrgPolicyConstraints, ResolveError, ResolveRequest,
};

/// The runtime-policy request one deployment target makes, expressed as data.
///
/// Consumed at the composition edge (`local_runtime_profile`): a
/// `RebornCompositionProfile` maps to one of the named constructors below and
/// everything downstream consumes the resolved policy values — no code past
/// this edge branches on a deployment mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeploymentConfig {
    /// Where IronClaw is running and who owns the machine boundary.
    pub(crate) deployment: DeploymentMode,
    /// The operator-requested runtime preset for this deployment.
    pub(crate) requested_profile: RuntimeProfile,
    /// Operator acknowledgement required before a `*Yolo*` profile resolves.
    pub(crate) yolo_disclosure_acknowledged: bool,
    /// Tenant/org ceiling constraints applied by the resolver.
    pub(crate) org_policy: OrgPolicyConstraints,
}

impl DeploymentConfig {
    /// Standalone local development on a single-user machine.
    pub(crate) fn local_dev() -> Self {
        Self {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            yolo_disclosure_acknowledged: false,
            org_policy: OrgPolicyConstraints::default(),
        }
    }

    /// Trusted-laptop local development with minimal approvals. Requires the
    /// operator's explicit host-access confirmation; without it the resolver
    /// fails closed with [`ResolveError::YoloRequiresDisclosure`].
    pub(crate) fn local_dev_yolo(confirm_host_access: bool) -> Self {
        Self {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalYolo,
            yolo_disclosure_acknowledged: confirm_host_access,
            org_policy: OrgPolicyConstraints::default(),
        }
    }

    /// Hosted single-tenant preview backed by the local runtime substrate:
    /// process execution disabled, scoped virtual filesystem, brokered
    /// network/secrets, ask-always approvals (the resolver-owned secure
    /// default under a hosted deployment boundary).
    pub(crate) fn hosted_single_tenant_volume() -> Self {
        Self {
            deployment: DeploymentMode::HostedMultiTenant,
            requested_profile: RuntimeProfile::SecureDefault,
            yolo_disclosure_acknowledged: false,
            org_policy: OrgPolicyConstraints::default(),
        }
    }

    /// Resolve this deployment request through the sanctioned resolver.
    pub(crate) fn resolve(&self) -> Result<EffectiveRuntimePolicy, ResolveError> {
        ironclaw_runtime_policy::resolve(ResolveRequest {
            deployment: self.deployment,
            requested_profile: self.requested_profile,
            org_policy: self.org_policy.clone(),
            yolo_disclosure_acknowledged: self.yolo_disclosure_acknowledged,
        })
    }

    /// The deployment's capability-policy *data* (§4.4.1 category 1): the
    /// embedded `builtin_capability_policy.toml` — provider policy, approval
    /// gates/defaults, and capability grants — parsed once through the
    /// `OnceLock`-cached loader. Like [`DeploymentConfig::resolve`], this is a
    /// thin adapter over the owning module's loader, not a second policy
    /// source.
    // dead_code allow: config-scoped accessor added by the §4.4 relocation;
    // existing composition call sites still reach the module loader directly
    // and migrate here separately.
    #[allow(dead_code)]
    pub(crate) fn builtin_capability_policy(
        &self,
    ) -> Result<
        crate::builtin_capability_policy::BuiltinCapabilityPolicy,
        crate::builtin_capability_policy::BuiltinCapabilityPolicyError,
    > {
        crate::builtin_capability_policy::builtin_capability_policy()
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::runtime_policy::{ApprovalPolicy, ProcessBackendKind};

    use super::*;

    #[test]
    fn local_dev_resolves_to_local_host_policy() {
        let policy = DeploymentConfig::local_dev().resolve().expect("resolves");
        assert_eq!(policy.deployment, DeploymentMode::LocalSingleUser);
        assert_eq!(policy.resolved_profile, RuntimeProfile::LocalDev);
        assert_eq!(policy.process_backend, ProcessBackendKind::LocalHost);
        assert_eq!(policy.approval_policy, ApprovalPolicy::AskDestructive);
    }

    #[test]
    fn local_dev_yolo_without_disclosure_fails_closed() {
        let error = DeploymentConfig::local_dev_yolo(false)
            .resolve()
            .expect_err("yolo without disclosure must fail");
        assert!(matches!(error, ResolveError::YoloRequiresDisclosure { .. }));
    }

    #[test]
    fn local_dev_yolo_with_disclosure_resolves_minimal_approvals() {
        let policy = DeploymentConfig::local_dev_yolo(true)
            .resolve()
            .expect("resolves");
        assert_eq!(policy.resolved_profile, RuntimeProfile::LocalYolo);
        assert_eq!(policy.approval_policy, ApprovalPolicy::Minimal);
    }

    #[test]
    fn hosted_single_tenant_volume_resolves_secure_default_without_processes() {
        let policy = DeploymentConfig::hosted_single_tenant_volume()
            .resolve()
            .expect("resolves");
        assert_eq!(policy.deployment, DeploymentMode::HostedMultiTenant);
        assert_eq!(policy.resolved_profile, RuntimeProfile::SecureDefault);
        assert_eq!(policy.process_backend, ProcessBackendKind::None);
        assert_eq!(policy.approval_policy, ApprovalPolicy::AskAlways);
    }

    #[test]
    fn deployment_targets_differ_only_as_data() {
        // The whole local/hosted diff is field values on one struct — the
        // §4.4 claim this module exists to make true.
        assert_ne!(
            DeploymentConfig::local_dev(),
            DeploymentConfig::hosted_single_tenant_volume()
        );
        assert_eq!(DeploymentConfig::local_dev(), DeploymentConfig::local_dev());
    }
}
