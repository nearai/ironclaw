use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::{LeaseApproval, permission_mode_allows_persistent_approval};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_host_api::{EffectKind, MountView, Principal};
use ironclaw_product_workflow::{
    ApprovalGateRecord, ApprovalInteractionRejectionKind, ApprovalLeaseTermsProvider,
    ProductWorkflowError,
};

use crate::builtin_capability_policy::{
    BuiltinApprovalPolicyAction, BuiltinCapabilityPolicy, BuiltinCapabilityPolicyError,
    builtin_one_shot_lease_approval,
};
use crate::outbound::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID;

use super::local_dev::extension_surface::ExtensionCapabilitySurfaceSource;

pub(super) struct PolicyApprovalLeaseTermsProvider {
    policy: Arc<BuiltinCapabilityPolicy>,
    registry: Arc<ExtensionRegistry>,
    workspace_mounts: MountView,
    skill_mounts: MountView,
    memory_mounts: MountView,
    system_extensions_lifecycle_mounts: MountView,
    extension_surface_source: ExtensionCapabilitySurfaceSource,
}

impl PolicyApprovalLeaseTermsProvider {
    pub(super) fn new(
        policy: Arc<BuiltinCapabilityPolicy>,
        registry: Arc<ExtensionRegistry>,
        workspace_mounts: MountView,
        skill_mounts: MountView,
        memory_mounts: MountView,
        system_extensions_lifecycle_mounts: MountView,
        extension_surface_source: ExtensionCapabilitySurfaceSource,
    ) -> Self {
        Self {
            policy,
            registry,
            workspace_mounts,
            skill_mounts,
            memory_mounts,
            system_extensions_lifecycle_mounts,
            extension_surface_source,
        }
    }

    async fn extension_lease_terms_for(
        &self,
        gate: &ApprovalGateRecord,
        action: BuiltinApprovalPolicyAction<'_>,
    ) -> Result<LeaseApproval, ProductWorkflowError> {
        self.extension_lease_terms_for_active_capability(gate, action)
            .await?
            .ok_or_else(lease_terms_unavailable)
    }

    async fn extension_lease_terms_for_active_capability(
        &self,
        gate: &ApprovalGateRecord,
        action: BuiltinApprovalPolicyAction<'_>,
    ) -> Result<Option<LeaseApproval>, ProductWorkflowError> {
        let capability = action.capability();
        let Principal::Extension(extension_id) = &gate.request().requested_by else {
            return Ok(None);
        };
        let surface = self
            .extension_surface_source
            .snapshot()
            .await
            .map_err(|error| {
                tracing::error!(%error, "local-dev extension approval lease terms are unavailable");
                lease_terms_unavailable()
            })?;
        // Lease terms resolve for the user whose run raised the gate; the
        // owner filter in `grants` then behaves exactly like dispatch did
        // (#5459 P1): their own private capability resolves, anyone else's
        // yields no grant and the lease stays unavailable.
        let Some(grant) = surface
            .grants(extension_id, &gate.resource_scope().user_id)
            .into_iter()
            .find(|grant| grant.capability == *capability)
        else {
            return Ok(None);
        };
        if action.is_spawn_capability()
            && !grant
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
        Ok(Some(builtin_one_shot_lease_approval(grant.constraints)))
    }

    async fn active_extension_persistent_approval_allowed(
        &self,
        action: BuiltinApprovalPolicyAction<'_>,
    ) -> Result<bool, ProductWorkflowError> {
        let surface = self
            .extension_surface_source
            .snapshot()
            .await
            .map_err(|error| {
                tracing::error!(%error, "local-dev extension approval surface is unavailable");
                lease_terms_unavailable()
            })?;
        let Some(capability) = surface.capability(action.capability()) else {
            return Ok(false);
        };
        if action.is_spawn_capability() && !capability.effects.contains(&EffectKind::SpawnProcess) {
            tracing::error!(
                capability = %action.capability(),
                "local-dev extension spawn persistent approval lacks SpawnProcess"
            );
            return Ok(false);
        }
        Ok(permission_mode_allows_persistent_approval(
            capability.default_permission,
        ))
    }
}

#[async_trait]
impl ApprovalLeaseTermsProvider for PolicyApprovalLeaseTermsProvider {
    async fn lease_terms_for(
        &self,
        gate: &ApprovalGateRecord,
    ) -> Result<ironclaw_approvals::LeaseApproval, ProductWorkflowError> {
        let action = BuiltinApprovalPolicyAction::from_host_action(gate.request().action.as_ref())
            .ok_or(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::UnsupportedAction,
            })?;
        if action.is_spawn_capability()
            && let Some(approval) = self
                .extension_lease_terms_for_active_capability(gate, action)
                .await?
        {
            return Ok(approval);
        }
        match self.policy.lease_approval_for(
            action,
            &self.workspace_mounts,
            &self.skill_mounts,
            &self.memory_mounts,
            &self.system_extensions_lifecycle_mounts,
        ) {
            Ok(approval) => Ok(approval),
            Err(BuiltinCapabilityPolicyError::MissingGrant { .. }) => {
                self.extension_lease_terms_for(gate, action).await
            }
            Err(error) => {
                tracing::error!(%error, "local-dev approval lease terms are unavailable");
                Err(lease_terms_unavailable())
            }
        }
    }

    async fn persistent_approval_allowed(
        &self,
        gate: &ApprovalGateRecord,
    ) -> Result<(), ProductWorkflowError> {
        let action = BuiltinApprovalPolicyAction::from_host_action(gate.request().action.as_ref())
            .ok_or(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::UnsupportedAction,
            })?;
        if let Some(descriptor) = self.registry.get_capability(action.capability_id()) {
            if permission_mode_allows_persistent_approval(descriptor.default_permission) {
                return Ok(());
            }
            return Err(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::AlwaysAllowUnsupported,
            });
        }
        if action.capability_id().as_str() == OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID {
            match self.policy.lease_approval_for(
                action,
                &self.workspace_mounts,
                &self.skill_mounts,
                &self.memory_mounts,
                &self.system_extensions_lifecycle_mounts,
            ) {
                Ok(_) => return Ok(()),
                Err(BuiltinCapabilityPolicyError::MissingGrant { .. }) => {}
                Err(error) => {
                    tracing::error!(
                        %error,
                        "local-dev persistent approval terms are unavailable"
                    );
                    return Err(lease_terms_unavailable());
                }
            }
        }
        if self
            .active_extension_persistent_approval_allowed(action)
            .await?
        {
            Ok(())
        } else {
            Err(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::AlwaysAllowUnsupported,
            })
        }
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
        ExtensionId, InvocationId, PermissionMode, ResourceEstimate, ResourceScope, SecretHandle,
        TenantId, ThreadId, UserId,
    };
    use ironclaw_product_workflow::approval_gate_ref;
    use ironclaw_turns::{GateRef, TurnRunId};

    use crate::builtin_capability_policy::builtin_capability_policy;
    use crate::extension_host::extension_lifecycle::ActiveExtensionCapability;
    use crate::runtime::local_dev::extension_surface::{
        ExtensionCapabilitySurface, ExtensionCapabilitySurfaceSource,
    };

    use super::*;

    #[tokio::test]
    async fn extension_capability_missing_from_builtin_policy_gets_one_shot_lease_terms() {
        let capability = CapabilityId::new("gmail.send_message").expect("capability id");
        let provider = ExtensionId::new("gmail").expect("provider id");
        let caller = ExtensionId::new("caller").expect("caller id");
        let source = ExtensionCapabilitySurfaceSource::from_surface(
            ExtensionCapabilitySurface::from_active_capabilities(vec![ActiveExtensionCapability {
                id: capability.clone(),
                provider,
                effects: vec![EffectKind::Network, EffectKind::UseSecret],
                default_permission: PermissionMode::Allow,
                runtime_credentials: Vec::new(),
                network_targets: Vec::new(),
                owner: ironclaw_extensions::InstallationOwner::Tenant,
            }]),
        );
        let terms_provider = PolicyApprovalLeaseTermsProvider::new(
            Arc::new(builtin_capability_policy().expect("policy parses")),
            Arc::new(ExtensionRegistry::new()),
            MountView::default(),
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
        assert_eq!(approval.constraints.max_invocations, Some(1));
        assert_eq!(
            approval.constraints.allowed_effects,
            vec![EffectKind::Network, EffectKind::UseSecret]
        );
        assert_eq!(
            approval.constraints.secrets,
            Vec::<SecretHandle>::new(),
            "test capability has no runtime credential handles"
        );
    }

    #[tokio::test]
    async fn extension_spawn_capability_uses_extension_surface_terms_before_default_policy() {
        let capability = CapabilityId::new("gmail.send_message").expect("capability id");
        let provider = ExtensionId::new("gmail").expect("provider id");
        let caller = ExtensionId::new("caller").expect("caller id");
        let secret = SecretHandle::new("gmail_token").expect("secret handle");
        let source = ExtensionCapabilitySurfaceSource::from_surface(
            ExtensionCapabilitySurface::from_active_capabilities(vec![ActiveExtensionCapability {
                id: capability.clone(),
                provider,
                effects: vec![
                    EffectKind::SpawnProcess,
                    EffectKind::Network,
                    EffectKind::UseSecret,
                ],
                default_permission: PermissionMode::Allow,
                runtime_credentials: vec![ironclaw_host_api::RuntimeCredentialRequirement {
                    handle: secret.clone(),
                    source: ironclaw_host_api::RuntimeCredentialRequirementSource::SecretHandle,
                    provider_scopes: Vec::new(),
                    audience: ironclaw_host_api::NetworkTargetPattern {
                        scheme: Some(ironclaw_host_api::NetworkScheme::Https),
                        host_pattern: "gmail.googleapis.com".to_string(),
                        port: None,
                    },
                    target: ironclaw_host_api::RuntimeCredentialTarget::Header {
                        name: "authorization".to_string(),
                        prefix: Some("Bearer ".to_string()),
                    },
                    required: true,
                }],
                network_targets: Vec::new(),
                owner: ironclaw_extensions::InstallationOwner::Tenant,
            }]),
        );
        let terms_provider = PolicyApprovalLeaseTermsProvider::new(
            Arc::new(builtin_capability_policy().expect("policy parses")),
            Arc::new(ExtensionRegistry::new()),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            source,
        );
        let request_id = ApprovalRequestId::new();
        let gate = approval_gate_record(
            request_id,
            Principal::Extension(caller),
            Action::SpawnCapability {
                capability: capability.clone(),
                estimated_resources: ResourceEstimate::default(),
            },
        );

        let approval = terms_provider
            .lease_terms_for(&gate)
            .await
            .expect("extension spawn lease terms");

        assert_eq!(approval.issued_by, Principal::HostRuntime);
        assert_eq!(approval.constraints.max_invocations, Some(1));
        assert_eq!(
            approval.constraints.allowed_effects,
            vec![
                EffectKind::SpawnProcess,
                EffectKind::Network,
                EffectKind::UseSecret
            ]
        );
        assert_eq!(approval.constraints.secrets, vec![secret]);
    }

    #[tokio::test]
    async fn active_extension_capability_allows_persistent_approval_when_manifest_allows() {
        let capability = CapabilityId::new("gmail.send_message").expect("capability id");
        let provider = ExtensionId::new("gmail").expect("provider id");
        let caller = ExtensionId::new("caller").expect("caller id");
        let source = ExtensionCapabilitySurfaceSource::from_surface(
            ExtensionCapabilitySurface::from_active_capabilities(vec![ActiveExtensionCapability {
                id: capability.clone(),
                provider,
                effects: vec![EffectKind::Network],
                default_permission: PermissionMode::Allow,
                runtime_credentials: Vec::new(),
                network_targets: Vec::new(),
                owner: ironclaw_extensions::InstallationOwner::Tenant,
            }]),
        );
        let terms_provider = PolicyApprovalLeaseTermsProvider::new(
            Arc::new(builtin_capability_policy().expect("policy parses")),
            Arc::new(ExtensionRegistry::new()),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            source,
        );
        let gate = approval_gate_record(
            ApprovalRequestId::new(),
            Principal::Extension(caller),
            Action::Dispatch {
                capability,
                estimated_resources: ResourceEstimate::default(),
            },
        );

        terms_provider
            .persistent_approval_allowed(&gate)
            .await
            .expect("active extension persistent approval should be allowed");
    }

    #[tokio::test]
    async fn active_extension_capability_allows_persistent_approval_when_manifest_asks() {
        let capability = CapabilityId::new("gmail.send_message").expect("capability id");
        let provider = ExtensionId::new("gmail").expect("provider id");
        let caller = ExtensionId::new("caller").expect("caller id");
        let source = ExtensionCapabilitySurfaceSource::from_surface(
            ExtensionCapabilitySurface::from_active_capabilities(vec![ActiveExtensionCapability {
                id: capability.clone(),
                provider,
                effects: vec![EffectKind::Network],
                default_permission: PermissionMode::Ask,
                runtime_credentials: Vec::new(),
                network_targets: Vec::new(),
                owner: ironclaw_extensions::InstallationOwner::Tenant,
            }]),
        );
        let terms_provider = PolicyApprovalLeaseTermsProvider::new(
            Arc::new(builtin_capability_policy().expect("policy parses")),
            Arc::new(ExtensionRegistry::new()),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            source,
        );
        let gate = approval_gate_record(
            ApprovalRequestId::new(),
            Principal::Extension(caller),
            Action::Dispatch {
                capability,
                estimated_resources: ResourceEstimate::default(),
            },
        );

        terms_provider
            .persistent_approval_allowed(&gate)
            .await
            .expect("active extension default ask should allow explicit persistent approval");
    }

    #[tokio::test]
    async fn outbound_delivery_target_set_allows_persistent_approval() {
        let capability =
            CapabilityId::new(OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID).expect("capability id");
        let caller = ExtensionId::new("loop-driver").expect("caller id");
        let terms_provider = PolicyApprovalLeaseTermsProvider::new(
            Arc::new(builtin_capability_policy().expect("policy parses")),
            Arc::new(ExtensionRegistry::new()),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            ExtensionCapabilitySurfaceSource::default(),
        );
        let gate = approval_gate_record(
            ApprovalRequestId::new(),
            Principal::Extension(caller),
            Action::Dispatch {
                capability,
                estimated_resources: ResourceEstimate::default(),
            },
        );

        terms_provider
            .persistent_approval_allowed(&gate)
            .await
            .expect("outbound delivery target set should allow persistent approval");
    }

    #[tokio::test]
    async fn active_extension_capability_rejects_persistent_approval_when_manifest_denies() {
        let capability = CapabilityId::new("gmail.send_message").expect("capability id");
        let provider = ExtensionId::new("gmail").expect("provider id");
        let caller = ExtensionId::new("caller").expect("caller id");
        let source = ExtensionCapabilitySurfaceSource::from_surface(
            ExtensionCapabilitySurface::from_active_capabilities(vec![ActiveExtensionCapability {
                id: capability.clone(),
                provider,
                effects: vec![EffectKind::Network],
                default_permission: PermissionMode::Deny,
                runtime_credentials: Vec::new(),
                network_targets: Vec::new(),
                owner: ironclaw_extensions::InstallationOwner::Tenant,
            }]),
        );
        let terms_provider = PolicyApprovalLeaseTermsProvider::new(
            Arc::new(builtin_capability_policy().expect("policy parses")),
            Arc::new(ExtensionRegistry::new()),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            MountView::default(),
            source,
        );
        let gate = approval_gate_record(
            ApprovalRequestId::new(),
            Principal::Extension(caller),
            Action::Dispatch {
                capability,
                estimated_resources: ResourceEstimate::default(),
            },
        );

        let error = terms_provider
            .persistent_approval_allowed(&gate)
            .await
            .expect_err("active extension default deny should reject persistent approval");

        assert!(matches!(
            error,
            ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::AlwaysAllowUnsupported
            }
        ));
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
