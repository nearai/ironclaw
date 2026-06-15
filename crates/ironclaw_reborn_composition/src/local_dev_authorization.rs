use std::sync::Arc;

use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_host_api::{
    EffectKind,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy, RuntimeProfile},
};

use crate::{
    local_dev_capability_policy::LocalDevCapabilityPolicy,
    profile_approval_authorization::{ProfileApprovalGatePolicy, profile_approval_authorizer},
    runtime_profile_approval_policy::RuntimeProfileApprovalGatePolicy,
};

pub(crate) fn local_dev_authorizer(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: Arc<LocalDevCapabilityPolicy>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    let (approval_policy, resolved_profile) = local_dev_approval_policy(runtime_policy);
    let gate_effects = capability_policy.approval_gate_effects();
    let gate_policy: Arc<dyn ProfileApprovalGatePolicy> = Arc::new(
        RuntimeProfileApprovalGatePolicy::new(resolved_profile, gate_effects),
    );
    profile_approval_authorizer(approval_policy, gate_policy)
}

pub(crate) fn local_dev_effects_require_approval(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: &LocalDevCapabilityPolicy,
    effects: &[EffectKind],
) -> bool {
    let (approval_policy, resolved_profile) = local_dev_approval_policy(runtime_policy);
    RuntimeProfileApprovalGatePolicy::new(
        resolved_profile,
        capability_policy.approval_gate_effects(),
    )
    .effects_require_approval(approval_policy, effects)
}

fn local_dev_approval_policy(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
) -> (ApprovalPolicy, RuntimeProfile) {
    let approval_policy = runtime_policy
        .map(|policy| policy.approval_policy)
        .unwrap_or(ApprovalPolicy::AskDestructive);
    let resolved_profile = runtime_policy
        .map(|policy| policy.resolved_profile)
        .unwrap_or(RuntimeProfile::LocalDev);
    (approval_policy, resolved_profile)
}
