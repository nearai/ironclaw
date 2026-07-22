//! Managed-user data-plane adapter.
//!
//! Every operation preserves the authenticated actor and target subject as
//! distinct `UserId` values and crosses the identity domain's canonical
//! admin-managed-target authorization decision before touching user resources.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{SecretHandle, TenantId, UserId};
use ironclaw_product_workflow::{AdminManagedResourceService, AdminUserError, AdminUserSecretMeta};
use ironclaw_reborn_identity::{
    AdminManagedUserOperation, RebornIdentityError, RebornUserDirectory,
};
use ironclaw_secrets::{SecretMetadata, SecretStoreError};
use secrecy::SecretString;

use crate::admin_secrets::AdminSecretProvisioner;

pub(crate) struct RebornAdminManagedResources {
    directory: Arc<dyn RebornUserDirectory>,
    secrets: Arc<dyn AdminSecretProvisioner>,
}

impl RebornAdminManagedResources {
    pub(crate) fn new(
        directory: Arc<dyn RebornUserDirectory>,
        secrets: Arc<dyn AdminSecretProvisioner>,
    ) -> Self {
        Self { directory, secrets }
    }

    async fn authorize(
        &self,
        tenant: &TenantId,
        actor_user_id: &UserId,
        subject_user_id: &UserId,
    ) -> Result<(), AdminUserError> {
        let allowed = self
            .directory
            .authorize_admin_managed_target(
                tenant,
                actor_user_id,
                subject_user_id,
                AdminManagedUserOperation::ManageSecrets,
            )
            .await
            .map_err(map_identity_error)?;
        if allowed {
            Ok(())
        } else {
            Err(AdminUserError::Forbidden)
        }
    }
}

#[async_trait]
impl AdminManagedResourceService for RebornAdminManagedResources {
    async fn list_secrets(
        &self,
        tenant: &TenantId,
        actor_user_id: &UserId,
        subject_user_id: &UserId,
    ) -> Result<Vec<AdminUserSecretMeta>, AdminUserError> {
        self.authorize(tenant, actor_user_id, subject_user_id)
            .await?;
        let secrets = self
            .secrets
            .list(tenant, subject_user_id)
            .await
            .map_err(map_secret_error)?;
        Ok(secrets.into_iter().map(to_secret_meta).collect())
    }

    async fn put_secret(
        &self,
        tenant: &TenantId,
        actor_user_id: &UserId,
        subject_user_id: &UserId,
        handle: SecretHandle,
        material: SecretString,
    ) -> Result<AdminUserSecretMeta, AdminUserError> {
        self.authorize(tenant, actor_user_id, subject_user_id)
            .await?;
        self.secrets
            .put(tenant, subject_user_id, handle, material)
            .await
            .map(to_secret_meta)
            .map_err(map_secret_error)
    }

    async fn delete_secret(
        &self,
        tenant: &TenantId,
        actor_user_id: &UserId,
        subject_user_id: &UserId,
        handle: SecretHandle,
    ) -> Result<bool, AdminUserError> {
        self.authorize(tenant, actor_user_id, subject_user_id)
            .await?;
        self.secrets
            .delete(tenant, subject_user_id, &handle)
            .await
            .map_err(map_secret_error)
    }
}

fn to_secret_meta(meta: SecretMetadata) -> AdminUserSecretMeta {
    AdminUserSecretMeta {
        handle: meta.handle.as_str().to_string(),
        created_at: None,
        updated_at: None,
    }
}

fn map_identity_error(error: RebornIdentityError) -> AdminUserError {
    match error {
        RebornIdentityError::Backend(_) => AdminUserError::Unavailable,
        RebornIdentityError::UserNotFound(_) => AdminUserError::NotFound,
        RebornIdentityError::UserPolicyViolation(_) => AdminUserError::InvalidInput,
        RebornIdentityError::InvalidUserId(_)
        | RebornIdentityError::ChannelActorNotMintable
        | RebornIdentityError::UserSuspended(_)
        | RebornIdentityError::ManagedUserLoginDisabled(_) => AdminUserError::Internal,
    }
}

fn map_secret_error(error: SecretStoreError) -> AdminUserError {
    match error {
        SecretStoreError::UnknownSecret { .. } | SecretStoreError::UnknownLease { .. } => {
            AdminUserError::NotFound
        }
        SecretStoreError::StoreUnavailable { .. } => AdminUserError::Unavailable,
        _ => AdminUserError::Internal,
    }
}
