use std::sync::Arc;

use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_host_api::runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy, RuntimeProfile};

use crate::{
    local_dev_capability_policy::LocalDevCapabilityPolicy,
    profile_approval_authorization::{ProfileApprovalGatePolicy, profile_approval_authorizer},
    runtime_profile_approval_policy::RuntimeProfileApprovalGatePolicy,
};

pub(crate) fn local_dev_authorizer(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: Arc<LocalDevCapabilityPolicy>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    let approval_policy = runtime_policy
        .map(|policy| policy.approval_policy)
        .unwrap_or(ApprovalPolicy::AskDestructive);
    let resolved_profile = runtime_policy
        .map(|policy| policy.resolved_profile)
        .unwrap_or(RuntimeProfile::LocalDev);
    let gate_effects = capability_policy.approval_gate_effects();
    let gate_policy: Arc<dyn ProfileApprovalGatePolicy> = Arc::new(
        RuntimeProfileApprovalGatePolicy::new(resolved_profile, gate_effects),
    );
    profile_approval_authorizer(approval_policy, gate_policy)
}
