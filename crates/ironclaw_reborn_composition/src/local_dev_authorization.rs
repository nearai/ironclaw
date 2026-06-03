use std::{borrow::Cow, sync::Arc};

use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityDescriptor, Decision, EffectKind,
    ExecutionContext, Principal, ResourceEstimate,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
use ironclaw_trust::TrustDecision;

use crate::local_dev_capability_policy::LocalDevCapabilityPolicy;

struct LocalDevApprovalPolicyAuthorizer {
    inner: GrantAuthorizer,
    approval_policy: ApprovalPolicy,
    capability_policy: Arc<LocalDevCapabilityPolicy>,
}

impl LocalDevApprovalPolicyAuthorizer {
    fn new(
        approval_policy: ApprovalPolicy,
        capability_policy: Arc<LocalDevCapabilityPolicy>,
    ) -> Self {
        Self {
            inner: GrantAuthorizer::new(),
            approval_policy,
            capability_policy,
        }
    }
}

#[async_trait::async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for LocalDevApprovalPolicyAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        let decision = self
            .inner
            .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
            .await;
        require_approval_for_local_dev_policy(
            decision,
            context,
            descriptor,
            estimate,
            LocalDevApprovalActionKind::Dispatch,
            self.approval_policy,
            &self.capability_policy,
        )
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        let decision = self
            .inner
            .authorize_spawn_with_trust(context, descriptor, estimate, trust_decision)
            .await;
        require_approval_for_local_dev_policy(
            decision,
            context,
            descriptor,
            estimate,
            LocalDevApprovalActionKind::SpawnCapability,
            self.approval_policy,
            &self.capability_policy,
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum LocalDevApprovalActionKind {
    Dispatch,
    SpawnCapability,
}

pub(crate) fn local_dev_authorizer(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: Arc<LocalDevCapabilityPolicy>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    let approval_policy = runtime_policy
        .map(|policy| policy.approval_policy)
        .unwrap_or(ApprovalPolicy::AskDestructive);
    match approval_policy {
        // Minimal ~ yolo: skip approval gates entirely and delegate to
        // the grant authorizer only. This means every effectful capability
        // (shell, apply_patch, network) runs ungated. Intentional for the
        // local-dev-yolo profile only. Warn so misconfiguration is visible.
        ApprovalPolicy::Minimal => {
            tracing::warn!(
                "local-dev running with ApprovalPolicy::Minimal — all capability gates are disabled"
            );
            Arc::new(GrantAuthorizer::new())
        }
        ApprovalPolicy::AskAlways
        | ApprovalPolicy::AskWrites
        | ApprovalPolicy::AskDestructive
        | ApprovalPolicy::OrgPolicy => Arc::new(LocalDevApprovalPolicyAuthorizer::new(
            approval_policy,
            capability_policy,
        )),
        // Any future ApprovalPolicy variants default to the gating authorizer
        // (fail toward requiring approval rather than disabling gates).
        _ => Arc::new(LocalDevApprovalPolicyAuthorizer::new(
            approval_policy,
            capability_policy,
        )),
    }
}

fn require_approval_for_local_dev_policy(
    decision: Decision,
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    action_kind: LocalDevApprovalActionKind,
    approval_policy: ApprovalPolicy,
    capability_policy: &LocalDevCapabilityPolicy,
) -> Decision {
    // A spawn exercises SpawnProcess even when the capability's own descriptor
    // does not declare it: the underlying GrantAuthorizer authorizes spawns
    // against `spawn_descriptor`, which adds EffectKind::SpawnProcess. Evaluate
    // the approval gate against the same elevated effect set so a dispatch-only
    // builtin (e.g. builtin.echo) cannot be spawned as a live process without
    // an approval gate.
    let gate_effects = approval_gate_effects(action_kind, descriptor);
    if let Decision::Allow { .. } = &decision
        && capability_policy.effects_require_approval(approval_policy, &gate_effects)
        && !has_matching_one_shot_approval_grant(
            context,
            descriptor,
            &gate_effects,
            approval_policy,
            capability_policy,
        )
    {
        return Decision::RequireApproval {
            request: approval_request(context, descriptor, estimate, action_kind),
        };
    }
    // Non-Allow decisions (Deny, RequireApproval) pass through unchanged —
    // the local-dev gate only upgrades Allow to RequireApproval, never
    // downgrades a deny or re-gates an already-gated request.
    decision
}

/// Effects the local-dev approval gate evaluates for `action_kind`.
///
/// Mirrors `ironclaw_authorization::spawn_descriptor`: a spawn always exercises
/// `SpawnProcess`, so it is added to the capability's declared effects when
/// gating a spawn. Dispatch evaluates the declared effects unchanged.
fn approval_gate_effects(
    action_kind: LocalDevApprovalActionKind,
    descriptor: &CapabilityDescriptor,
) -> Cow<'_, [EffectKind]> {
    match action_kind {
        LocalDevApprovalActionKind::Dispatch => Cow::Borrowed(descriptor.effects.as_slice()),
        LocalDevApprovalActionKind::SpawnCapability => {
            if descriptor.effects.contains(&EffectKind::SpawnProcess) {
                Cow::Borrowed(descriptor.effects.as_slice())
            } else {
                let mut effects = descriptor.effects.clone();
                effects.push(EffectKind::SpawnProcess);
                Cow::Owned(effects)
            }
        }
    }
}

fn has_matching_one_shot_approval_grant(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    gate_effects: &[EffectKind],
    approval_policy: ApprovalPolicy,
    capability_policy: &LocalDevCapabilityPolicy,
) -> bool {
    // Hoist the expected grantee/issuer principals so they are not
    // re-allocated on every iteration of the any() closure.
    let expected_grantee = Principal::Extension(context.extension_id.clone());
    context.grants.grants.iter().any(|grant| {
        grant.capability == descriptor.id
            && grant.constraints.max_invocations == Some(1)
            && grant.issued_by == Principal::HostRuntime
            && grant.grantee == expected_grantee
            // Match against the spawn-elevated effect set so a one-shot lease
            // that does not cover SpawnProcess cannot satisfy a spawn gate.
            && gate_effects
                .iter()
                .all(|effect| grant.constraints.allowed_effects.contains(effect))
            && capability_policy
                .effects_require_approval(approval_policy, &grant.constraints.allowed_effects)
    })
}

fn approval_request(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    action_kind: LocalDevApprovalActionKind,
) -> ApprovalRequest {
    let action = match action_kind {
        LocalDevApprovalActionKind::Dispatch => Action::Dispatch {
            capability: descriptor.id.clone(),
            estimated_resources: estimate.clone(),
        },
        LocalDevApprovalActionKind::SpawnCapability => Action::SpawnCapability {
            capability: descriptor.id.clone(),
            estimated_resources: estimate.clone(),
        },
    };
    ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: context.correlation_id,
        requested_by: Principal::Extension(context.extension_id.clone()),
        action: Box::new(action),
        invocation_fingerprint: None,
        reason: format!(
            "approval required for {:?} of {}",
            action_kind,
            descriptor.id.as_str()
        ),
        reusable_scope: None,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{
        CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet,
        EffectKind, ExecutionContext, ExtensionId, GrantConstraints, MountView, NetworkPolicy,
        PermissionMode, Principal, ResourceEstimate, RuntimeKind, TrustClass,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use serde_json::json;

    use super::*;
    use crate::local_dev_capability_policy::local_dev_capability_policy;

    fn test_descriptor(effects: Vec<EffectKind>) -> CapabilityDescriptor {
        test_descriptor_with_id(CapabilityId::new("builtin.shell").unwrap(), effects)
    }

    fn test_descriptor_with_id(id: CapabilityId, effects: Vec<EffectKind>) -> CapabilityDescriptor {
        CapabilityDescriptor {
            id,
            provider: ExtensionId::new("builtin").unwrap(),
            runtime: RuntimeKind::FirstParty,
            trust_ceiling: TrustClass::UserTrusted,
            description: "test".to_string(),
            parameters_schema: json!({}),
            effects,
            default_permission: PermissionMode::Allow,
            runtime_credentials: Vec::new(),
            resource_profile: None,
        }
    }

    fn test_context(grants: CapabilitySet) -> ExecutionContext {
        let ctx = ExecutionContext::local_default(
            ironclaw_host_api::UserId::new("test-user").unwrap(),
            ExtensionId::new("builtin").unwrap(),
            RuntimeKind::FirstParty,
            TrustClass::UserTrusted,
            grants,
            MountView::default(),
        )
        .unwrap();
        ctx.validate().unwrap();
        ctx
    }

    fn test_trust_decision() -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: vec![EffectKind::SpawnProcess, EffectKind::DispatchCapability],
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::AdminConfig,
            evaluated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn dispatch_with_destructive_effect_requires_approval() {
        let capability_policy = Arc::new(local_dev_capability_policy().unwrap());
        let authorizer = local_dev_authorizer(None, capability_policy); // defaults to AskDestructive

        let shell_id = CapabilityId::new("builtin.shell").unwrap();
        let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "destructive dispatch should require approval, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn spawn_with_dispatch_only_capability_requires_approval() {
        let capability_policy = Arc::new(local_dev_capability_policy().unwrap());
        let authorizer = local_dev_authorizer(None, capability_policy); // defaults to AskDestructive

        // builtin.echo declares only DispatchCapability, but spawn elevates to include SpawnProcess
        let echo_id = CapabilityId::new("builtin.echo").unwrap();
        let descriptor =
            test_descriptor_with_id(echo_id.clone(), vec![EffectKind::DispatchCapability]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: echo_id,
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_spawn_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::RequireApproval { .. }),
            "spawn of dispatch-only capability should require approval via SpawnProcess elevation, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn minimal_policy_skips_approval_gate() {
        use ironclaw_host_api::runtime_policy::{
            ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy,
            FilesystemBackendKind, NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
        };

        let capability_policy = Arc::new(local_dev_capability_policy().unwrap());
        let mut runtime_policy = EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::LocalDev,
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend: ProcessBackendKind::LocalHost,
            network_mode: NetworkMode::DirectLogged,
            secret_mode: SecretMode::ScrubbedEnv,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        };
        runtime_policy.approval_policy = ApprovalPolicy::Minimal;
        let authorizer = local_dev_authorizer(Some(&runtime_policy), capability_policy);

        let descriptor = test_descriptor(vec![EffectKind::SpawnProcess]);
        let ctx = test_context(CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new("builtin.shell").unwrap(),
                grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::Allow { .. }),
            "Minimal policy should delegate to GrantAuthorizer and Allow, got {decision:?}"
        );
    }

    #[tokio::test]
    async fn deny_decision_passes_through_unchanged() {
        let capability_policy = Arc::new(local_dev_capability_policy().unwrap());
        let authorizer = local_dev_authorizer(None, capability_policy); // defaults to AskDestructive

        // Empty grant set: GrantAuthorizer returns Deny (no matching grant)
        let descriptor = test_descriptor(vec![EffectKind::DispatchCapability]);
        let ctx = test_context(CapabilitySet { grants: vec![] });
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &ctx,
                &descriptor,
                &ResourceEstimate::default(),
                &test_trust_decision(),
            )
            .await;

        assert!(
            matches!(decision, Decision::Deny { .. }),
            "ungranted capability should return Deny unchanged, got {decision:?}"
        );
    }

    #[test]
    fn approval_request_reason_includes_capability_id() {
        let descriptor = test_descriptor(vec![EffectKind::SpawnProcess]);
        let ctx = test_context(CapabilitySet { grants: vec![] });
        let req = approval_request(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            LocalDevApprovalActionKind::Dispatch,
        );

        assert!(
            req.reason.contains("builtin.shell"),
            "reason should contain capability id, got: {:?}",
            req.reason
        );
        assert!(
            req.reason.contains("Dispatch"),
            "reason should contain action kind, got: {:?}",
            req.reason
        );
    }
}
