//! Login-token boundary for explicit administrator-issued user credentials.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_workflow::{AdminIssuedLoginToken, AdminUserError, AdminUserLoginTokenIssuer};
use ironclaw_reborn_identity::{RebornUserDirectory, RebornUserStatus, UserContentAccessPolicy};
use secrecy::SecretString;

/// Host-owned signer. Composition validates identity policy before invoking it.
#[async_trait]
pub trait AdminLoginTokenMinter: Send + Sync {
    async fn mint(&self, tenant: &TenantId, user_id: &UserId) -> Result<SecretString, String>;
}

pub(crate) struct RebornAdminLoginTokenIssuer {
    directory: Arc<dyn RebornUserDirectory>,
    minter: Arc<dyn AdminLoginTokenMinter>,
}

impl RebornAdminLoginTokenIssuer {
    pub(crate) fn new(
        directory: Arc<dyn RebornUserDirectory>,
        minter: Arc<dyn AdminLoginTokenMinter>,
    ) -> Self {
        Self { directory, minter }
    }
}

#[async_trait]
impl AdminUserLoginTokenIssuer for RebornAdminLoginTokenIssuer {
    async fn issue_login_token(
        &self,
        tenant: &TenantId,
        actor_user_id: &UserId,
        subject_user_id: &UserId,
    ) -> Result<AdminIssuedLoginToken, AdminUserError> {
        let subject = self
            .directory
            .get_user(subject_user_id)
            .await
            .map_err(|_| AdminUserError::Unavailable)?
            .ok_or(AdminUserError::Forbidden)?;
        let same_tenant = subject
            .tenant_id
            .as_ref()
            .is_none_or(|owner| owner == tenant);
        if !same_tenant
            || subject.status != RebornUserStatus::Active
            || subject.content_access_policy != UserContentAccessPolicy::Private
        {
            return Err(AdminUserError::Forbidden);
        }
        let token = self
            .minter
            .mint(tenant, subject_user_id)
            .await
            .map_err(|_| AdminUserError::Unavailable)?;
        tracing::info!(
            tenant_id = %tenant,
            actor_user_id = %actor_user_id,
            subject_user_id = %subject_user_id,
            "administrator issued a private-user login token"
        );
        Ok(AdminIssuedLoginToken { token })
    }
}
