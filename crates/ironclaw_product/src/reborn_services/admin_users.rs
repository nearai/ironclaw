//! The admin user-management wire contract + the fail-closed default.
//!
//! The [`AdminUserService`] port and its record vocabulary moved to
//! `ironclaw_product_contracts::admin_users` (PROPOSAL §6.1.3): its only
//! production implementation is a `ironclaw_reborn_composition` adapter over
//! the identity user-directory, and `ironclaw_extension_host` reads the same
//! directory to resolve a channel actor's admin role. What stays here is
//! product's own surface: the `Reborn*` request/response types the WebChat v2
//! admin routes serialize, and the fail-closed default `RebornServices` wires
//! before composition installs the real adapter.

use std::collections::BTreeMap;

use async_trait::async_trait;
use ironclaw_host_api::ids::{SecretHandle, TenantId, UserId};
use ironclaw_product_contracts::admin_users::{
    AdminCreateUserFields, AdminCreatedUser, AdminUserError, AdminUserRecord, AdminUserRole,
    AdminUserSecretMeta, AdminUserService, AdminUserStatus,
};
use secrecy::SecretString;

pub use ironclaw_product_contracts::admin_users::{
    RebornAdminCreateUserRequest, RebornAdminDeleteSecretProductRequest,
    RebornAdminPutSecretProductRequest, RebornAdminPutSecretRequest,
    RebornAdminSecretDeletedResponse, RebornAdminSecretResponse, RebornAdminSetRoleProductRequest,
    RebornAdminSetRoleRequest, RebornAdminSetStatusProductRequest, RebornAdminSetStatusRequest,
    RebornAdminUpdateUserProductRequest, RebornAdminUpdateUserRequest,
    RebornAdminUserCreatedResponse, RebornAdminUserDeletedResponse, RebornAdminUserListQuery,
    RebornAdminUserListResponse, RebornAdminUserRequest, RebornAdminUserResponse,
    RebornAdminUserSecretsListResponse,
};

/// Fail-closed default wired into `RebornServices` before composition installs
/// the real adapter. Every operation reports the service unavailable, so a
/// deployment that never wires the admin surface serves 503s rather than
/// panicking or silently succeeding. Mirrors the `Rejecting*` default pattern
/// used for the other optional-but-live services on `RebornServices`.
pub(crate) struct RejectingAdminUserService;

#[async_trait]
impl AdminUserService for RejectingAdminUserService {
    async fn list_users(
        &self,
        _tenant: &TenantId,
        _status: Option<AdminUserStatus>,
        _after: Option<&UserId>,
        _limit: usize,
    ) -> Result<Vec<AdminUserRecord>, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn get_user(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
    ) -> Result<Option<AdminUserRecord>, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn create_user(
        &self,
        _tenant: &TenantId,
        _actor: &UserId,
        _fields: AdminCreateUserFields,
    ) -> Result<AdminCreatedUser, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn update_profile(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        _display_name: Option<String>,
        _metadata: Option<BTreeMap<String, String>>,
    ) -> Result<AdminUserRecord, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn set_status(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        _status: AdminUserStatus,
    ) -> Result<AdminUserRecord, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn set_role(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        _role: AdminUserRole,
    ) -> Result<AdminUserRecord, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn delete_user(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
    ) -> Result<(), AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn count_active_admins(&self, _tenant: &TenantId) -> Result<usize, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn list_secrets(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
    ) -> Result<Vec<AdminUserSecretMeta>, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn put_secret(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        _handle: SecretHandle,
        _material: SecretString,
    ) -> Result<AdminUserSecretMeta, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }

    async fn delete_secret(
        &self,
        _tenant: &TenantId,
        _user_id: &UserId,
        _handle: SecretHandle,
    ) -> Result<bool, AdminUserError> {
        Err(AdminUserError::Unavailable)
    }
}
