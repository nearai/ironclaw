use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::LeaseApproval;
use ironclaw_host_api::{Action, CapabilityId, EffectKind, MountView, Principal};
use ironclaw_product_workflow::{
    ApprovalGateRecord, ApprovalInteractionRejectionKind, ApprovalLeaseTermsProvider,
    ProductWorkflowError,
};

use crate::local_dev_capability_policy::{
    LocalDevApprovalPolicyAction, LocalDevCapabilityPolicy, LocalDevCapabilityPolicyError,
};

use super::local_dev::extension_surface::LocalDevExtensionSurfaceSource;

pub(super) struct LocalDevApprovalLeaseTermsProvider {
    policy: Arc<LocalDevCapabilityPolicy>,
    workspace_mounts: MountView,
    skill_mounts: MountView,
    memory_mounts: MountView,
    extension_surface_source: LocalDevExtensionSurfaceSource,
}

impl LocalDevApprovalLeaseTermsProvider {
    pub(super) fn new(
        policy: Arc<LocalDevCapabilityPolicy>,
        workspace_mounts: MountView,
        skill_mounts: MountView,
        memory_mounts: MountView,
        extension_surface_source: LocalDevExtensionSurfaceSource,
    ) -> Self {
        Self {
            policy,
            workspace_mounts,
            skill_mounts,
            memory_mounts,
            extension_surface_source,
        }
    }

    async fn extension_lease_terms_for(
        &self,
        gate: &ApprovalGateRecord,
    ) -> Result<LeaseApproval, ProductWorkflowError> {
        let capability = capability_for_action(gate.request().action.as_ref())?;
        let Principal::Extension(extension_id) = &gate.request().requested_by else {
            return Err(lease_terms_unavailable());
        };
        let surface = self
            .extension_surface_source
            .snapshot()
            .await
            .map_err(|error| {
                tracing::error!(%error, "local-dev extension approval lease terms are unavailable");
                lease_terms_unavailable()
            })?;
        let grant = surface
            .grants(extension_id)
            .into_iter()
            .find(|grant| grant.capability == *capability)
            .ok_or_else(lease_terms_unavailable)?;
        if matches!(
            gate.request().action.as_ref(),
            Action::SpawnCapability { .. }
        ) && !grant
            .constraints
            .allowed_effects
            .contains(&EffectKind::SpawnProcess)
        {
            tracing::error!(
                capability = %capability,
                "local-dev extension spawn approval lease lacks SpawnProcess"
            );
            return Err(lease_terms_unavailable());
        }
        Ok(LeaseApproval {
            issued_by: Principal::HostRuntime,
            allowed_effects: grant.constraints.allowed_effects,
            mounts: grant.constraints.mounts,
            network: grant.constraints.network,
            secrets: grant.constraints.secrets,
            resource_ceiling: grant.constraints.resource_ceiling,
            expires_at: grant.constraints.expires_at,
            max_invocations: Some(1),
        })
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
        match self.policy.lease_approval_for(
            action,
            &self.workspace_mounts,
            &self.skill_mounts,
            &self.memory_mounts,
        ) {
            Ok(approval) => Ok(approval),
            Err(LocalDevCapabilityPolicyError::MissingGrant { .. }) => {
                self.extension_lease_terms_for(gate).await
            }
            Err(error) => {
                tracing::error!(%error, "local-dev approval lease terms are unavailable");
                Err(lease_terms_unavailable())
            }
        }
    }
}

fn capability_for_action(action: &Action) -> Result<&CapabilityId, ProductWorkflowError> {
    match action {
        Action::Dispatch { capability, .. } | Action::SpawnCapability { capability, .. } => {
            Ok(capability)
        }
        _ => Err(ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::UnsupportedAction,
        }),
    }
}

fn lease_terms_unavailable() -> ProductWorkflowError {
    ProductWorkflowError::ApprovalInteractionRejected {
        kind: ApprovalInteractionRejectionKind::LeaseTermsUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_host_api::{
        Action, ApprovalRequest, ApprovalRequestId, CapabilityId, CorrelationId, EffectKind,
        ExtensionId, InvocationId, ResourceEstimate, ResourceScope, SecretHandle, TenantId,
        ThreadId, UserId,
    };
    use ironclaw_product_workflow::approval_gate_ref;
    use ironclaw_turns::{GateRef, TurnRunId};

    use crate::{
        extension_lifecycle::ActiveExtensionCapability,
        local_dev_capability_policy::local_dev_capability_policy,
        runtime::local_dev::extension_surface::{
            LocalDevExtensionSurface, LocalDevExtensionSurfaceSource,
        },
    };

    use super::*;

    #[tokio::test]
    async fn extension_capability_missing_from_builtin_policy_gets_one_shot_lease_terms() {
        let capability = CapabilityId::new("gmail.send_message").expect("capability id");
        let provider = ExtensionId::new("gmail").expect("provider id");
        let caller = ExtensionId::new("caller").expect("caller id");
        let source = LocalDevExtensionSurfaceSource::from_surface(
            LocalDevExtensionSurface::from_active_capabilities(vec![ActiveExtensionCapability {
                id: capability.clone(),
                provider,
                effects: vec![EffectKind::Network, EffectKind::UseSecret],
                runtime_credentials: Vec::new(),
            }]),
        );
        let terms_provider = LocalDevApprovalLeaseTermsProvider::new(
            Arc::new(local_dev_capability_policy().expect("policy parses")),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            source,
        );
        let request_id = ApprovalRequestId::new();
        let gate = approval_gate_record(
            request_id,
            Principal::Extension(caller),
            Action::Dispatch {
                capability: capability.clone(),
                estimated_resources: ResourceEstimate::default(),
            },
        );

        let approval = terms_provider
            .lease_terms_for(&gate)
            .await
            .expect("extension lease terms");

        assert_eq!(approval.issued_by, Principal::HostRuntime);
        assert_eq!(approval.max_invocations, Some(1));
        assert_eq!(
            approval.allowed_effects,
            vec![EffectKind::Network, EffectKind::UseSecret]
        );
        assert_eq!(
            approval.secrets,
            Vec::<SecretHandle>::new(),
            "test capability has no runtime credential handles"
        );
    }

    fn approval_gate_record(
        request_id: ApprovalRequestId,
        requested_by: Principal,
        action: Action,
    ) -> ApprovalGateRecord {
        let resource_scope = ResourceScope {
            tenant_id: TenantId::new("tenant").expect("tenant id"),
            user_id: UserId::new("user").expect("user id"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: Some(ThreadId::new("thread").expect("thread id")),
            invocation_id: InvocationId::new(),
        };
        let gate_ref: GateRef = approval_gate_ref(request_id).expect("approval gate ref");
        ApprovalGateRecord::new(
            resource_scope,
            TurnRunId::new(),
            gate_ref,
            ApprovalRequest {
                id: request_id,
                correlation_id: CorrelationId::new(),
                requested_by,
                action: Box::new(action),
                invocation_fingerprint: None,
                reason: "approval required".to_string(),
                reusable_scope: None,
            },
        )
        .expect("approval gate record")
    }
}
