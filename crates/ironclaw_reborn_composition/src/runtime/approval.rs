use async_trait::async_trait;
use ironclaw_approvals::LeaseApproval;
use ironclaw_host_api::{Action, CapabilityId, EffectKind, MountView, Principal};
use ironclaw_product_workflow::{
    ApprovalGateRecord, ApprovalInteractionRejectionKind, ApprovalLeaseTermsProvider,
    ProductWorkflowError,
};

use super::local_dev;

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

enum LocalDevApprovalAction<'a> {
    Dispatch { capability: &'a CapabilityId },
    SpawnCapability { capability: &'a CapabilityId },
}

impl<'a> LocalDevApprovalAction<'a> {
    fn from_action(action: &'a Action) -> Result<Self, ProductWorkflowError> {
        match action {
            Action::Dispatch { capability, .. } => Ok(Self::Dispatch { capability }),
            Action::SpawnCapability { capability, .. } => Ok(Self::SpawnCapability { capability }),
            _ => Err(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::UnsupportedAction,
            }),
        }
    }

    fn capability(&self) -> &CapabilityId {
        match self {
            Self::Dispatch { capability } | Self::SpawnCapability { capability } => capability,
        }
    }

    fn requires_spawn_process(&self) -> bool {
        matches!(self, Self::SpawnCapability { .. })
    }
}

#[async_trait]
impl ApprovalLeaseTermsProvider for LocalDevApprovalLeaseTermsProvider {
    async fn lease_terms_for(
        &self,
        gate: &ApprovalGateRecord,
    ) -> Result<LeaseApproval, ProductWorkflowError> {
        let action = LocalDevApprovalAction::from_action(gate.request().action.as_ref())?;
        let mut constraints = local_dev::local_dev_grant_constraints(
            action.capability().as_str(),
            &self.workspace_mounts,
            &self.skill_mounts,
        );
        if action.requires_spawn_process()
            && !constraints
                .allowed_effects
                .contains(&EffectKind::SpawnProcess)
        {
            constraints.allowed_effects.push(EffectKind::SpawnProcess);
        }
        Ok(LeaseApproval {
            issued_by: Principal::HostRuntime,
            allowed_effects: constraints.allowed_effects,
            mounts: constraints.mounts,
            network: constraints.network,
            secrets: constraints.secrets,
            resource_ceiling: constraints.resource_ceiling,
            expires_at: constraints.expires_at,
            max_invocations: Some(1),
        })
    }
}
