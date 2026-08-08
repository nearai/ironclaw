use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_approvals::{LeaseApproval, permission_mode_allows_persistent_approval};
use ironclaw_assistant::{
    ApprovalGateRecord, ApprovalInteractionRejectionKind, ApprovalLeaseTermsProvider,
    ProductSurfaceFailure,
};
use ironclaw_extension_registry::ExtensionRegistry;
use ironclaw_host_api::{capability::EffectKind, mount::MountView, scope::Principal};

use crate::builtin_capability_policy::{
    BuiltinApprovalPolicyAction, BuiltinCapabilityPolicy, BuiltinCapabilityPolicyError,
    builtin_one_shot_lease_approval,
};
use crate::runtime_mounts::WorkspaceMountPolicy;
use ironclaw_assistant::OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID;

use ironclaw_extension_host::capability_surface::ExtensionCapabilitySurfaceSource;

pub(super) struct PolicyApprovalLeaseTermsProvider {
    policy: Arc<BuiltinCapabilityPolicy>,
    registry: Arc<ExtensionRegistry>,
    /// Resolved per gate from the gate's own `ResourceScope`, so a lease minted
    /// for one caller can never grant another caller's workspace subtree.
    workspace_mounts: WorkspaceMountPolicy,
    memory_mounts: MountView,
    system_extensions_lifecycle_mounts: MountView,
    extension_surface_source: ExtensionCapabilitySurfaceSource,
}

impl PolicyApprovalLeaseTermsProvider {
    pub(super) fn new(
        policy: Arc<BuiltinCapabilityPolicy>,
        registry: Arc<ExtensionRegistry>,
        workspace_mounts: WorkspaceMountPolicy,
        memory_mounts: MountView,
        system_extensions_lifecycle_mounts: MountView,
        extension_surface_source: ExtensionCapabilitySurfaceSource,
    ) -> Self {
        Self {
            policy,
            registry,
            workspace_mounts,
            memory_mounts,
            system_extensions_lifecycle_mounts,
            extension_surface_source,
        }
    }

    /// The skill view this gate's lease terms are minted from.
    ///
    /// Per gate, from the gate's own scope, for the same reason the workspace view is: the terms
    /// have to name the paths the capability will touch. A fixed, scope-free view named
    /// `/projects/skills` minted leases describing a tree the install never writes.
    fn skill_mounts_for(
        &self,
        gate: &ApprovalGateRecord,
    ) -> Result<MountView, ProductSurfaceFailure> {
        crate::runtime_mounts::db_backed_skill_management_mount_view(gate.resource_scope()).map_err(
            |error| {
                tracing::error!(%error, "approval lease skill mounts could not be scoped");
                lease_terms_unavailable()
            },
        )
    }

    /// The workspace view this gate's lease terms are minted from.
    ///
    /// Fails closed: a per-caller deployment whose gate scope cannot key a
    /// subtree yields no lease rather than the shared workspace root.
    fn workspace_mounts_for(
        &self,
        gate: &ApprovalGateRecord,
    ) -> Result<MountView, ProductSurfaceFailure> {
        self.workspace_mounts
            .capability_grant_view(gate.resource_scope())
            .map_err(|error| {
                tracing::error!(%error, "approval lease workspace mounts could not be scoped");
                lease_terms_unavailable()
            })
    }

    async fn extension_lease_terms_for(
        &self,
        gate: &ApprovalGateRecord,
        action: BuiltinApprovalPolicyAction<'_>,
    ) -> Result<LeaseApproval, ProductSurfaceFailure> {
        self.extension_lease_terms_for_active_capability(gate, action)
            .await?
            .ok_or_else(lease_terms_unavailable)
    }

    async fn extension_lease_terms_for_active_capability(
        &self,
        gate: &ApprovalGateRecord,
        action: BuiltinApprovalPolicyAction<'_>,
    ) -> Result<Option<LeaseApproval>, ProductSurfaceFailure> {
        let capability = action.capability();
        let Principal::Extension(extension_id) = &gate.request().requested_by else {
            return Ok(None);
        };
        let surface = self
            .extension_surface_source
            .snapshot()
            .await
            .map_err(|error| {
                tracing::error!(%error, "standalone extension approval lease terms are unavailable");
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
                "standalone extension spawn approval lease lacks SpawnProcess"
            );
            return Err(lease_terms_unavailable());
        }
        Ok(Some(builtin_one_shot_lease_approval(grant.constraints)))
    }

    async fn active_extension_persistent_approval_allowed(
        &self,
        action: BuiltinApprovalPolicyAction<'_>,
    ) -> Result<bool, ProductSurfaceFailure> {
        let surface = self
            .extension_surface_source
            .snapshot()
            .await
            .map_err(|error| {
                tracing::error!(%error, "standalone extension approval surface is unavailable");
                lease_terms_unavailable()
            })?;
        let Some(capability) = surface.capability(action.capability()) else {
            return Ok(false);
        };
        if action.is_spawn_capability() && !capability.effects.contains(&EffectKind::SpawnProcess) {
            tracing::error!(
                capability = %action.capability(),
                "standalone extension spawn persistent approval lacks SpawnProcess"
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
    ) -> Result<ironclaw_approvals::LeaseApproval, ProductSurfaceFailure> {
        let action = BuiltinApprovalPolicyAction::from_host_action(gate.request().action.as_ref())
            .ok_or(ProductSurfaceFailure::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::UnsupportedAction,
            })?;
        if action.is_spawn_capability()
            && let Some(approval) = self
                .extension_lease_terms_for_active_capability(gate, action)
                .await?
        {
            return Ok(approval);
        }
        let workspace_mounts = self.workspace_mounts_for(gate)?;
        let skill_mounts = self.skill_mounts_for(gate)?;
        match self.policy.lease_approval_for(
            action,
            &workspace_mounts,
            &skill_mounts,
            &self.memory_mounts,
            &self.system_extensions_lifecycle_mounts,
        ) {
            Ok(approval) => Ok(approval),
            Err(BuiltinCapabilityPolicyError::MissingGrant { .. }) => {
                self.extension_lease_terms_for(gate, action).await
            }
            Err(error) => {
                tracing::error!(%error, "standalone approval lease terms are unavailable");
                Err(lease_terms_unavailable())
            }
        }
    }

    async fn persistent_approval_allowed(
        &self,
        gate: &ApprovalGateRecord,
    ) -> Result<(), ProductSurfaceFailure> {
        let action = BuiltinApprovalPolicyAction::from_host_action(gate.request().action.as_ref())
            .ok_or(ProductSurfaceFailure::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::UnsupportedAction,
            })?;
        if let Some(descriptor) = self.registry.get_capability(action.capability_id()) {
            if permission_mode_allows_persistent_approval(descriptor.default_permission) {
                return Ok(());
            }
            return Err(ProductSurfaceFailure::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::AlwaysAllowUnsupported,
            });
        }
        if action.capability_id().as_str() == OUTBOUND_NOTIFICATION_CHANNELS_SET_CAPABILITY_ID {
            let workspace_mounts = self.workspace_mounts_for(gate)?;
            let skill_mounts = self.skill_mounts_for(gate)?;
            match self.policy.lease_approval_for(
                action,
                &workspace_mounts,
                &skill_mounts,
                &self.memory_mounts,
                &self.system_extensions_lifecycle_mounts,
            ) {
                Ok(_) => return Ok(()),
                Err(BuiltinCapabilityPolicyError::MissingGrant { .. }) => {}
                Err(error) => {
                    tracing::error!(
                        %error,
                        "standalone persistent approval terms are unavailable"
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
            Err(ProductSurfaceFailure::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::AlwaysAllowUnsupported,
            })
        }
    }
}

fn lease_terms_unavailable() -> ProductSurfaceFailure {
    ProductSurfaceFailure::ApprovalInteractionRejected {
        kind: ApprovalInteractionRejectionKind::LeaseTermsUnavailable,
    }
}

#[cfg(test)]
mod tests;
