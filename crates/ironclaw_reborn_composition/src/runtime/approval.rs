use async_trait::async_trait;
use ironclaw_host_api::MountView;
use ironclaw_product_workflow::{
    ApprovalGateRecord, ApprovalInteractionRejectionKind, ApprovalLeaseTermsProvider,
    ProductWorkflowError,
};

use crate::local_dev_capability_policy::{
    LocalDevApprovalPolicyAction, local_dev_capability_policy,
};

pub(super) struct LocalDevApprovalLeaseTermsProvider {
    workspace_mounts: MountView,
    skill_mounts: MountView,
}

impl LocalDevApprovalLeaseTermsProvider {
    pub(super) fn new(workspace_mounts: MountView, skill_mounts: MountView) -> Self {
        Self {
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
        let policy = local_dev_capability_policy().map_err(|_| {
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::LeaseTermsUnavailable,
            }
        })?;
        policy
            .lease_approval_for(action, &self.workspace_mounts, &self.skill_mounts)
            .map_err(|_| ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::LeaseTermsUnavailable,
            })
    }
}
