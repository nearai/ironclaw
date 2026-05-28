use std::sync::Arc;

use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityDescriptor, Decision, ExecutionContext,
    Principal, ResourceEstimate,
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

#[derive(Clone, Copy)]
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
        ApprovalPolicy::Minimal => Arc::new(GrantAuthorizer::new()),
        ApprovalPolicy::AskAlways
        | ApprovalPolicy::AskWrites
        | ApprovalPolicy::AskDestructive
        | ApprovalPolicy::OrgPolicy
        | _ => Arc::new(LocalDevApprovalPolicyAuthorizer::new(
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
    match decision {
        Decision::Allow { .. }
            if capability_policy.effects_require_approval(approval_policy, &descriptor.effects)
                && !has_matching_one_shot_approval_grant(
                    context,
                    descriptor,
                    approval_policy,
                    capability_policy,
                ) =>
        {
            Decision::RequireApproval {
                request: approval_request(
                    context,
                    descriptor,
                    estimate,
                    action_kind,
                    approval_policy,
                ),
            }
        }
        other => other,
    }
}

fn has_matching_one_shot_approval_grant(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    approval_policy: ApprovalPolicy,
    capability_policy: &LocalDevCapabilityPolicy,
) -> bool {
    context.grants.grants.iter().any(|grant| {
        grant.capability == descriptor.id
            && grant.constraints.max_invocations == Some(1)
            && grant.grantee == Principal::Extension(context.extension_id.clone())
            && descriptor
                .effects
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
    approval_policy: ApprovalPolicy,
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
        reason: format!("local-dev {approval_policy} policy requires approval"),
        reusable_scope: None,
    }
}
