//! Exact authority cleanup for lifecycle-managed capabilities.

use ironclaw_host_api::{CapabilityId, Principal, ResourceScope};
use thiserror::Error;

use crate::{
    CapabilityPermissionOverrideKey, CapabilityPermissionOverrideStore,
    CapabilityPermissionStoreError, PersistentApprovalAction, PersistentApprovalPolicyError,
    PersistentApprovalPolicyKey, PersistentApprovalPolicyStore,
};

#[derive(Debug, Error)]
pub enum CapabilityAuthorityCleanupError {
    #[error("persistent approval policy cleanup failed: {0}")]
    Policy(#[source] PersistentApprovalPolicyError),
    #[error("capability permission override cleanup failed: {0}")]
    Override(#[source] CapabilityPermissionStoreError),
}

/// Revoke reusable dispatch authority and clear explicit permission overrides
/// for an exact set of lifecycle-managed capabilities.
///
/// Authority created by the operator tools surface is tenant-user scoped, so
/// this helper deliberately normalizes away invocation, thread, agent, project,
/// and mission fields before constructing store keys. For every capability the
/// dispatch policy is revoked first; only then is its override cleared. Missing
/// records are successful, while every other store error stops cleanup before
/// the caller may remove its durable retry intent.
pub async fn cleanup_capability_authority(
    persistent_policies: &dyn PersistentApprovalPolicyStore,
    overrides: &dyn CapabilityPermissionOverrideStore,
    scope: &ResourceScope,
    grantee: &Principal,
    capability_ids: &[CapabilityId],
) -> Result<(), CapabilityAuthorityCleanupError> {
    let tenant_user_scope = scope.tenant_user_settings_scope();
    for capability_id in capability_ids {
        let policy_key = PersistentApprovalPolicyKey::new(
            &tenant_user_scope,
            PersistentApprovalAction::Dispatch,
            capability_id.clone(),
            grantee.clone(),
        );
        match persistent_policies.revoke(&policy_key).await {
            Ok(_) | Err(PersistentApprovalPolicyError::UnknownPolicy) => {}
            Err(error) => return Err(CapabilityAuthorityCleanupError::Policy(error)),
        }

        overrides
            .clear(&CapabilityPermissionOverrideKey::new(
                &tenant_user_scope,
                capability_id.clone(),
            ))
            .await
            .map_err(CapabilityAuthorityCleanupError::Override)?;
    }
    Ok(())
}
