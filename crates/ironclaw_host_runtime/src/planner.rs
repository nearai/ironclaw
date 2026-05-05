//! Runtime planner: `EffectiveRuntimePolicy + CapabilityDescriptor → ExecutionPlan`.
//!
//! The planner is the seam between the resolver-level policy
//! (deployment + profile + tenant/org constraints, owned by
//! `ironclaw_runtime_policy`) and the concrete backend selection the
//! host runtime carries through dispatch. Examples:
//!
//! ```text
//! LocalDev + filesystem.read   -> HostWorkspace read under selected root
//! HostedDev + shell.run        -> tenant-sandbox process, never provider-host
//! EnterpriseDev + process.run  -> org-dedicated runner if org policy permits
//! Experiment + package install -> disposable SmolVm/Docker workspace
//! ```
//!
//! The planner is a pure function over the resolver output and the
//! capability descriptor. Authority decisions (whether the principal
//! holds the grant, whether approval is required, etc.) remain in
//! `ironclaw_authorization` and `ironclaw_capabilities`. The planner
//! only answers "which backend kinds will we use for this capability,
//! and is this combination structurally possible at all?"
//!
//! Fail-closed rules locked in here:
//!
//! - A capability that requires `SpawnProcess` / `ExecuteCode` against
//!   a policy whose `process_backend` is `None` is rejected.
//! - A capability that requires `Network` against `NetworkMode::Deny`
//!   is rejected.
//! - A capability that requires `UseSecret` against `SecretMode::Deny`
//!   is rejected.
//!
//! These are *runtime* fail-closed checks. The visibility filter (in
//! `runtime_filter`) hides profile-impossible capabilities before the
//! model call; the planner is the second line of defence at execution
//! time. Both must agree: if the visibility filter would have hidden a
//! capability, the planner must also refuse to plan it.

use ironclaw_host_api::runtime_policy::{
    EffectiveRuntimePolicy, FilesystemBackendKind, NetworkMode, ProcessBackendKind, SecretMode,
};
use ironclaw_host_api::{CapabilityDescriptor, CapabilityId, EffectKind};
use thiserror::Error;

/// Concrete plan derived from an `EffectiveRuntimePolicy` for a single
/// capability invocation.
///
/// The plan names the backend kinds the host runtime will dispatch
/// against. It does not own backend instances or resource leases —
/// those are still composed by `HostRuntimeServices` and gated by
/// authorization. The plan is the audit-friendly summary of which
/// substrate the capability *will* run on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    pub capability: CapabilityId,
    pub filesystem_backend: FilesystemBackendKind,
    pub process_backend: ProcessBackendKind,
    pub network_mode: NetworkMode,
    pub secret_mode: SecretMode,
}

/// Reasons the planner refuses to produce an `ExecutionPlan`.
///
/// Each variant carries enough context for an audit log entry to
/// explain the refusal without leaking principal/secret material.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlannerError {
    /// Capability declared a `SpawnProcess`/`ExecuteCode` effect but
    /// the resolved policy disables process effects entirely.
    #[error(
        "capability {capability} requires process effects but policy resolves to ProcessBackendKind::None"
    )]
    ProcessEffectsRequiredButProcessBackendIsNone { capability: CapabilityId },

    /// Capability declared a `Network` effect but the resolved policy
    /// denies all network egress.
    #[error(
        "capability {capability} requires network effects but policy resolves to NetworkMode::Deny"
    )]
    NetworkRequiredButNetworkModeIsDeny { capability: CapabilityId },

    /// Capability declared a `UseSecret` effect but the resolved policy
    /// denies all secret access.
    #[error(
        "capability {capability} requires secret access but policy resolves to SecretMode::Deny"
    )]
    SecretAccessRequiredButSecretModeIsDeny { capability: CapabilityId },
}

/// Plan a capability invocation against an `EffectiveRuntimePolicy`.
///
/// Returns the concrete `ExecutionPlan` or a `PlannerError` if the
/// capability's declared effects are structurally incompatible with
/// the policy's backend selection. This is the second line of defence
/// behind the visibility filter — a capability that survived the
/// filter but later turned out to be impossible (e.g. because the
/// policy was reduced after the model call) still fails closed here.
pub fn plan_capability(
    descriptor: &CapabilityDescriptor,
    policy: &EffectiveRuntimePolicy,
) -> Result<ExecutionPlan, PlannerError> {
    let effects = &descriptor.effects;
    let needs_process = effects
        .iter()
        .any(|e| matches!(e, EffectKind::SpawnProcess | EffectKind::ExecuteCode));
    if needs_process && matches!(policy.process_backend, ProcessBackendKind::None) {
        return Err(
            PlannerError::ProcessEffectsRequiredButProcessBackendIsNone {
                capability: descriptor.id.clone(),
            },
        );
    }

    let needs_network = effects.iter().any(|e| matches!(e, EffectKind::Network));
    if needs_network && matches!(policy.network_mode, NetworkMode::Deny) {
        return Err(PlannerError::NetworkRequiredButNetworkModeIsDeny {
            capability: descriptor.id.clone(),
        });
    }

    let needs_secret = effects.iter().any(|e| matches!(e, EffectKind::UseSecret));
    if needs_secret && matches!(policy.secret_mode, SecretMode::Deny) {
        return Err(PlannerError::SecretAccessRequiredButSecretModeIsDeny {
            capability: descriptor.id.clone(),
        });
    }

    Ok(ExecutionPlan {
        capability: descriptor.id.clone(),
        filesystem_backend: policy.filesystem_backend,
        process_backend: policy.process_backend,
        network_mode: policy.network_mode,
        secret_mode: policy.secret_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::runtime_policy::{
        ApprovalPolicy, AuditMode, DeploymentMode, RuntimeProfile,
    };
    use ironclaw_host_api::{ExtensionId, PermissionMode, RuntimeKind, TrustClass};

    fn descriptor(effects: Vec<EffectKind>) -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: CapabilityId::new("test.capability".to_string()).unwrap(),
            provider: ExtensionId::new("test_extension".to_string()).unwrap(),
            runtime: RuntimeKind::Script,
            trust_ceiling: TrustClass::UserTrusted,
            description: "test".to_string(),
            parameters_schema: serde_json::Value::Null,
            effects,
            default_permission: PermissionMode::Allow,
            resource_profile: None,
        }
    }

    fn policy_with(
        filesystem: FilesystemBackendKind,
        process: ProcessBackendKind,
        network: NetworkMode,
        secret: SecretMode,
    ) -> EffectiveRuntimePolicy {
        EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::LocalDev,
            filesystem_backend: filesystem,
            process_backend: process,
            network_mode: network,
            secret_mode: secret,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        }
    }

    #[test]
    fn plans_local_dev_filesystem_read_against_host_workspace() {
        // Issue example: `LocalDev + filesystem.read -> HostWorkspace
        // read under selected root`. The planner forwards the
        // resolved filesystem backend; downstream composition picks
        // the actual root.
        let desc = descriptor(vec![EffectKind::ReadFilesystem]);
        let policy = policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::None,
            NetworkMode::Brokered,
            SecretMode::ScrubbedEnv,
        );
        let plan = plan_capability(&desc, &policy).unwrap();
        assert_eq!(
            plan.filesystem_backend,
            FilesystemBackendKind::HostWorkspace
        );
    }

    #[test]
    fn plans_hosted_dev_shell_run_against_tenant_sandbox_never_local_host() {
        // Issue example: `HostedDev + shell.run -> tenant-sandbox
        // process, never provider-host shell`. The resolver guarantees
        // the policy carries `TenantSandbox`; the planner forwards it.
        // A regression that swapped to `LocalHost` here would defeat
        // the resolver's hosted-multi-tenant fail-closed contract.
        let desc = descriptor(vec![EffectKind::SpawnProcess, EffectKind::ExecuteCode]);
        let policy = policy_with(
            FilesystemBackendKind::TenantWorkspace,
            ProcessBackendKind::TenantSandbox,
            NetworkMode::Allowlist,
            SecretMode::TenantBroker,
        );
        let plan = plan_capability(&desc, &policy).unwrap();
        assert_eq!(plan.process_backend, ProcessBackendKind::TenantSandbox);
        assert_ne!(plan.process_backend, ProcessBackendKind::LocalHost);
    }

    #[test]
    fn plans_enterprise_dev_process_run_against_org_dedicated_runner() {
        // Issue example: `EnterpriseDev + process.run -> org-dedicated
        // runner if org policy permits`. Org policy admission is the
        // resolver's job; by the time we get here, the policy already
        // carries `OrgDedicatedRunner`.
        let desc = descriptor(vec![EffectKind::SpawnProcess]);
        let policy = policy_with(
            FilesystemBackendKind::OrgDedicatedWorkspace,
            ProcessBackendKind::OrgDedicatedRunner,
            NetworkMode::Allowlist,
            SecretMode::OrgBroker,
        );
        let plan = plan_capability(&desc, &policy).unwrap();
        assert_eq!(plan.process_backend, ProcessBackendKind::OrgDedicatedRunner);
    }

    #[test]
    fn rejects_process_capability_when_policy_disables_processes() {
        // Process effects against `ProcessBackendKind::None` (e.g. a
        // SecureDefault profile that turned off processes entirely)
        // must fail closed at plan time. The visibility filter should
        // already have hidden this capability, but the planner is the
        // belt-and-braces check at execution.
        let desc = descriptor(vec![EffectKind::SpawnProcess]);
        let policy = policy_with(
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::None,
            NetworkMode::Brokered,
            SecretMode::BrokeredHandles,
        );
        let err = plan_capability(&desc, &policy).unwrap_err();
        assert!(matches!(
            err,
            PlannerError::ProcessEffectsRequiredButProcessBackendIsNone { .. }
        ));
    }

    #[test]
    fn rejects_network_capability_when_policy_denies_network() {
        let desc = descriptor(vec![EffectKind::Network]);
        let policy = policy_with(
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::None,
            NetworkMode::Deny,
            SecretMode::BrokeredHandles,
        );
        let err = plan_capability(&desc, &policy).unwrap_err();
        assert!(matches!(
            err,
            PlannerError::NetworkRequiredButNetworkModeIsDeny { .. }
        ));
    }

    #[test]
    fn rejects_secret_capability_when_policy_denies_secrets() {
        let desc = descriptor(vec![EffectKind::UseSecret]);
        let policy = policy_with(
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::None,
            NetworkMode::Brokered,
            SecretMode::Deny,
        );
        let err = plan_capability(&desc, &policy).unwrap_err();
        assert!(matches!(
            err,
            PlannerError::SecretAccessRequiredButSecretModeIsDeny { .. }
        ));
    }

    #[test]
    fn plans_capability_with_no_effects_against_any_policy() {
        // A capability with empty `effects` (e.g. a pure
        // dispatch/observability capability) doesn't trigger any
        // fail-closed branches and just gets the policy's defaults.
        let desc = descriptor(vec![]);
        let policy = policy_with(
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::None,
            NetworkMode::Deny,
            SecretMode::Deny,
        );
        let plan = plan_capability(&desc, &policy).unwrap();
        assert_eq!(
            plan.filesystem_backend,
            FilesystemBackendKind::ScopedVirtual
        );
    }
}
