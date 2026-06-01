use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::MountView;
use ironclaw_product_workflow::{
    ApprovalGateRecord, ApprovalInteractionRejectionKind, ApprovalLeaseTermsProvider,
    ProductWorkflowError,
};

use crate::local_dev_capability_policy::{LocalDevApprovalPolicyAction, LocalDevCapabilityPolicy};

pub(super) struct LocalDevApprovalLeaseTermsProvider {
    policy: Arc<LocalDevCapabilityPolicy>,
    workspace_mounts: MountView,
    skill_mounts: MountView,
}

impl LocalDevApprovalLeaseTermsProvider {
    pub(super) fn new(
        policy: Arc<LocalDevCapabilityPolicy>,
        workspace_mounts: MountView,
        skill_mounts: MountView,
    ) -> Self {
        Self {
            policy,
            workspace_mounts,
            skill_mounts,
        }
    }
}

#[async_trait]
impl ApprovalLeaseTermsProvider for LocalDevApprovalLeaseTermsProvider {
    async fn lease_terms_for(
        &self,
        gate: &ApprovalGateRecord,
    ) -> Result<ironclaw_approvals::LeaseApproval, ProductWorkflowError> {
        let action = LocalDevApprovalPolicyAction::from_host_action(gate.request().action.as_ref())
            .ok_or(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::UnsupportedAction,
            })?;
        self.policy
            .lease_approval_for(action, &self.workspace_mounts, &self.skill_mounts)
            .map_err(|error| {
                tracing::error!(%error, "local-dev approval lease terms are unavailable");
                ProductWorkflowError::ApprovalInteractionRejected {
                    kind: ApprovalInteractionRejectionKind::LeaseTermsUnavailable,
                }
            })
    }
}
