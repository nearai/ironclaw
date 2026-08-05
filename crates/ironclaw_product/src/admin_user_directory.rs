//! Product-tier implementation of the
//! [`AdminUserService`](ironclaw_product_contracts::admin_users::AdminUserService)
//! port over the Reborn identity user-directory, an admin secret provisioner,
//! and a token minter.
//!
//! Admin user management is product workflow — tenant scoping, role/status
//! transitions, one-time bearer issuance — so the adapter lives beside the rest
//! of the product surface rather than in the composition root (PROPOSAL
//! §6.10.1, WS6). Composition keeps only the *deployment* half: which secret
//! backend implements [`AdminSecretProvisioner`], and which minter is wired.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::{SecretHandle, TenantId, UserId};
use ironclaw_product_contracts::admin_users::{
    AdminApiTokenMinter, AdminCreateUserFields, AdminCreatedUser, AdminUserError, AdminUserRecord,
    AdminUserRole, AdminUserSecretMeta, AdminUserService, AdminUserStatus,
};
use ironclaw_reborn_identity::{
    RebornIdentityError, RebornUser, RebornUserDirectory, RebornUserProfileUpdate, RebornUserRole,
    RebornUserStatus,
};
use ironclaw_secrets::{SecretMaterial, SecretMetadata, SecretStoreError};
use secrecy::SecretString;

/// Admin provisioning of per-user secrets for an arbitrary target `(tenant,
/// user)`.
///
/// The `ironclaw_secrets` store isolates tenant/user by the caller's
/// `MountView`, not the `ResourceScope` argument, so provisioning a secret for
/// a *target* user — the admin use case — needs a store mounted at that user's
/// subtree. Building that mount is deployment shape, so the port is declared
/// here (beside its one caller) and implemented in the composition root.
#[async_trait]
pub trait AdminSecretProvisioner: Send + Sync {
    async fn list(
        &self,
        tenant: &TenantId,
        user: &UserId,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError>;

    async fn put(
        &self,
        tenant: &TenantId,
        user: &UserId,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError>;

    async fn delete(
        &self,
        tenant: &TenantId,
        user: &UserId,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError>;
}

/// Fail-closed placeholder for composition paths that need an
/// [`AdminUserService`] handle purely for tenant-scoped role reads
/// (channel-command admission's `get_user` calls, which never mint tokens)
/// rather than the WebUI admin `create_user` route.
/// [`RebornAdminUserDirectory::create_user`] is the sole caller of the minter;
/// this always denies it rather than silently succeeding without a configured
/// minter.
pub struct RejectingAdminApiTokenMinter;

#[async_trait]
impl AdminApiTokenMinter for RejectingAdminApiTokenMinter {
    async fn mint(&self, _tenant: &TenantId, _user_id: &UserId) -> Result<SecretString, String> {
        Err("admin API token minting is not configured for this composition path".to_string())
    }
}

/// Adapter wiring the identity directory, admin secret provisioner, and token
/// minter into the [`AdminUserService`] contract.
pub struct RebornAdminUserDirectory {
    directory: Arc<dyn RebornUserDirectory>,
    secrets: Arc<dyn AdminSecretProvisioner>,
    token_minter: Arc<dyn AdminApiTokenMinter>,
}

impl RebornAdminUserDirectory {
    pub fn new(
        directory: Arc<dyn RebornUserDirectory>,
        secrets: Arc<dyn AdminSecretProvisioner>,
        token_minter: Arc<dyn AdminApiTokenMinter>,
    ) -> Self {
        Self {
            directory,
            secrets,
            token_minter,
        }
    }

    /// Fetch a user and confirm it belongs to `tenant`. A record with no
    /// persisted tenant (pre-admin single-tenant deployments) is allowed;
    /// otherwise a mismatch reads as "not found" so a cross-tenant id guess
    /// cannot enumerate or mutate another tenant's user.
    async fn tenant_scoped_user(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
    ) -> Result<Option<RebornUser>, AdminUserError> {
        let user = self
            .directory
            .get_user(user_id)
            .await
            .map_err(map_identity_error)?;
        Ok(user.filter(|user| match &user.tenant_id {
            Some(owner) => owner == tenant,
            None => true,
        }))
    }
}

#[async_trait]
impl AdminUserService for RebornAdminUserDirectory {
    async fn list_users(
        &self,
        tenant: &TenantId,
        status: Option<AdminUserStatus>,
        after: Option<&UserId>,
        limit: usize,
    ) -> Result<Vec<AdminUserRecord>, AdminUserError> {
        let users = self
            .directory
            .list_users(tenant, status.map(status_to_identity), after, limit)
            .await
            .map_err(map_identity_error)?;
        Ok(users.into_iter().map(to_admin_record).collect())
    }

    async fn get_user(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
    ) -> Result<Option<AdminUserRecord>, AdminUserError> {
        Ok(self
            .tenant_scoped_user(tenant, user_id)
            .await?
            .map(to_admin_record))
    }

    async fn create_user(
        &self,
        tenant: &TenantId,
        actor: &UserId,
        fields: AdminCreateUserFields,
    ) -> Result<AdminCreatedUser, AdminUserError> {
        let created = self
            .directory
            .create_user(
                tenant,
                fields.email,
                fields.display_name,
                role_to_identity(fields.role),
                actor,
            )
            .await
            .map_err(map_identity_error)?;
        // Mint the one-time bearer after the user exists. A mint failure leaves
        // the user created but tokenless; surfaced as Internal (logged) so the
        // admin can re-issue rather than silently succeeding.
        let api_token = self
            .token_minter
            .mint(tenant, &created.user_id)
            .await
            .map_err(|reason| {
                tracing::error!(error = %reason, "admin api token mint failed");
                AdminUserError::Internal
            })?;
        Ok(AdminCreatedUser {
            record: to_admin_record(created),
            api_token,
        })
    }

    async fn update_profile(
        &self,
        _tenant: &TenantId,
        user_id: &UserId,
        display_name: Option<String>,
        metadata: Option<BTreeMap<String, String>>,
    ) -> Result<AdminUserRecord, AdminUserError> {
        // The service has already tenant-scoped the target via get_user before
        // calling this, so the directory's user-id-keyed mutation is safe.
        let user = self
            .directory
            .update_profile(
                user_id,
                RebornUserProfileUpdate {
                    display_name,
                    metadata,
                },
            )
            .await
            .map_err(map_identity_error)?;
        Ok(to_admin_record(user))
    }

    async fn set_status(
        &self,
        _tenant: &TenantId,
        user_id: &UserId,
        status: AdminUserStatus,
    ) -> Result<AdminUserRecord, AdminUserError> {
        let user = self
            .directory
            .update_status(user_id, status_to_identity(status))
            .await
            .map_err(map_identity_error)?;
        Ok(to_admin_record(user))
    }

    async fn set_role(
        &self,
        _tenant: &TenantId,
        user_id: &UserId,
        role: AdminUserRole,
    ) -> Result<AdminUserRecord, AdminUserError> {
        let user = self
            .directory
            .update_role(user_id, role_to_identity(role))
            .await
            .map_err(map_identity_error)?;
        Ok(to_admin_record(user))
    }

    async fn delete_user(&self, tenant: &TenantId, user_id: &UserId) -> Result<(), AdminUserError> {
        self.directory
            .delete_user(tenant, user_id)
            .await
            .map_err(map_identity_error)
    }

    async fn count_active_admins(&self, tenant: &TenantId) -> Result<usize, AdminUserError> {
        self.directory
            .count_active_admins(tenant)
            .await
            .map_err(map_identity_error)
    }

    async fn list_secrets(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
    ) -> Result<Vec<AdminUserSecretMeta>, AdminUserError> {
        let secrets = self
            .secrets
            .list(tenant, user_id)
            .await
            .map_err(map_secret_error)?;
        Ok(secrets.into_iter().map(to_secret_meta).collect())
    }

    async fn put_secret(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
        handle: SecretHandle,
        material: SecretString,
    ) -> Result<AdminUserSecretMeta, AdminUserError> {
        // `handle` is already the validated domain type — the WebUI handler
        // parses `SecretHandle` at the HTTP edge (a malformed handle is a 400
        // there), so the adapter never sees a raw string to re-validate.
        let meta = self
            .secrets
            .put(tenant, user_id, handle, material)
            .await
            .map_err(map_secret_error)?;
        Ok(to_secret_meta(meta))
    }

    async fn delete_secret(
        &self,
        tenant: &TenantId,
        user_id: &UserId,
        handle: SecretHandle,
    ) -> Result<bool, AdminUserError> {
        self.secrets
            .delete(tenant, user_id, &handle)
            .await
            .map_err(map_secret_error)
    }
}

fn to_admin_record(user: RebornUser) -> AdminUserRecord {
    AdminUserRecord {
        user_id: user.user_id,
        email: user.email,
        display_name: user.display_name,
        status: status_from_identity(user.status),
        role: role_from_identity(user.role),
        created_at: user.created_at,
        updated_at: user.updated_at,
        created_by: user.created_by,
        last_login_at: user.last_login_at,
        metadata: user.metadata,
    }
}

fn to_secret_meta(meta: SecretMetadata) -> AdminUserSecretMeta {
    AdminUserSecretMeta {
        handle: meta.handle.as_str().to_string(),
        created_at: None,
        updated_at: None,
    }
}

fn role_to_identity(role: AdminUserRole) -> RebornUserRole {
    match role {
        AdminUserRole::Owner => RebornUserRole::Owner,
        AdminUserRole::Admin => RebornUserRole::Admin,
        AdminUserRole::Member => RebornUserRole::Member,
    }
}

fn role_from_identity(role: RebornUserRole) -> AdminUserRole {
    match role {
        RebornUserRole::Owner => AdminUserRole::Owner,
        RebornUserRole::Admin => AdminUserRole::Admin,
        RebornUserRole::Member => AdminUserRole::Member,
    }
}

fn status_to_identity(status: AdminUserStatus) -> RebornUserStatus {
    match status {
        AdminUserStatus::Active => RebornUserStatus::Active,
        AdminUserStatus::Suspended => RebornUserStatus::Suspended,
    }
}

fn status_from_identity(status: RebornUserStatus) -> AdminUserStatus {
    match status {
        RebornUserStatus::Active => AdminUserStatus::Active,
        RebornUserStatus::Suspended => AdminUserStatus::Suspended,
    }
}

/// Map identity errors into the coarse port error, scrubbing storage paths
/// (`RebornIdentityError::Backend` carries a `ScopedPath`).
fn map_identity_error(error: RebornIdentityError) -> AdminUserError {
    match error {
        RebornIdentityError::UserNotFound(_) => AdminUserError::NotFound,
        RebornIdentityError::Backend(_) => AdminUserError::Unavailable,
        // A persisted-id inconsistency or a channel-actor misuse is not
        // retryable and not the client's fault.
        RebornIdentityError::InvalidUserId(_) | RebornIdentityError::ChannelActorNotMintable => {
            AdminUserError::Internal
        }
        // Only `resolve_or_create` (the SSO login path) raises this; the admin
        // directory operations never resolve external identities, so reaching
        // it here is a backend inconsistency rather than the client's fault.
        RebornIdentityError::UserSuspended(_) => AdminUserError::Internal,
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
