use std::sync::Arc;

use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_host_api::{
    EffectKind,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};

use crate::{
    local_dev_capability_policy::LocalDevCapabilityPolicy,
    profile_approval_authorization::{
        ProfileApprovalAuthorizerConfig, ProfileApprovalGatePolicy, profile_approval_authorizer,
    },
};

impl ProfileApprovalGatePolicy for LocalDevCapabilityPolicy {
    fn effects_require_approval(
        &self,
        approval_policy: ApprovalPolicy,
        effects: &[EffectKind],
    ) -> bool {
        LocalDevCapabilityPolicy::effects_require_approval(self, approval_policy, effects)
    }
}

pub(crate) fn local_dev_authorizer(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: Arc<LocalDevCapabilityPolicy>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    let approval_policy = runtime_policy
        .map(|policy| policy.approval_policy)
        .unwrap_or(ApprovalPolicy::AskDestructive);
    profile_approval_authorizer(ProfileApprovalAuthorizerConfig {
        profile_label: "local-dev",
        approval_policy,
        gate_policy: capability_policy,
    })
}
