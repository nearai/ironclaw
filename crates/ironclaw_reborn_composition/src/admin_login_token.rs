//! Login-token boundary for explicit administrator-issued user credentials.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_workflow::{AdminIssuedLoginToken, AdminUserError, AdminUserLoginTokenIssuer};
use ironclaw_reborn_identity::{RebornLoginPolicy, new_user_id};
use secrecy::SecretString;

/// Host-owned signer. Composition validates identity policy before invoking it.
#[async_trait]
pub trait AdminLoginTokenMinter: Send + Sync {
    async fn mint(&self, tenant: &TenantId, user_id: &UserId) -> Result<SecretString, String>;
}

/// Host-authentication policy for reusable credentials. Implementations must
/// re-read current identity state so suspension and deletion take effect
/// without rotating the deployment signing key.
#[async_trait]
pub trait ReusableLoginTokenValidator: Send + Sync {
    async fn validate(&self, tenant: &TenantId, user_id: &UserId) -> bool;
}

pub(crate) struct RebornAdminLoginTokenIssuer {
    login_policy: Arc<dyn RebornLoginPolicy>,
    minter: Arc<dyn AdminLoginTokenMinter>,
}

impl RebornAdminLoginTokenIssuer {
    pub(crate) fn new(
        login_policy: Arc<dyn RebornLoginPolicy>,
        minter: Arc<dyn AdminLoginTokenMinter>,
    ) -> Self {
        Self {
            login_policy,
            minter,
        }
    }
}

#[async_trait]
impl AdminUserLoginTokenIssuer for RebornAdminLoginTokenIssuer {
    async fn issue_login_token(
        &self,
        tenant: &TenantId,
        actor_user_id: &UserId,
        actor_is_operator: bool,
    ) -> Result<AdminIssuedLoginToken, AdminUserError> {
        let authorized = if actor_is_operator {
            true
        } else {
            self.login_policy
                .authorize_admin_login_token_issuance(tenant, actor_user_id)
                .await
                .map_err(|error| {
                    tracing::warn!(
                        tenant_id = %tenant,
                        actor_user_id = %actor_user_id,
                        error = %error,
                        "login-token issuance policy lookup failed"
                    );
                    AdminUserError::Unavailable
                })?
        };
        if !authorized {
            return Err(AdminUserError::Forbidden);
        }
        let subject_user_id = new_user_id().map_err(|error| {
            tracing::error!(
                tenant_id = %tenant,
                actor_user_id = %actor_user_id,
                error = %error,
                "identity domain could not allocate a login-token subject"
            );
            AdminUserError::Internal
        })?;
        let token = self
            .minter
            .mint(tenant, &subject_user_id)
            .await
            .map_err(|error| {
                tracing::warn!(
                    tenant_id = %tenant,
                    actor_user_id = %actor_user_id,
                    subject_user_id = %subject_user_id,
                    error = %error,
                    "login-token signing failed"
                );
                AdminUserError::Unavailable
            })?;
        tracing::info!(
            tenant_id = %tenant,
            actor_user_id = %actor_user_id,
            subject_user_id = %subject_user_id,
            "administrator issued a private-user login token"
        );
        Ok(AdminIssuedLoginToken {
            subject_user_id,
            token,
        })
    }
}

pub(crate) struct RebornReusableLoginTokenValidator {
    login_policy: Arc<dyn RebornLoginPolicy>,
}

impl RebornReusableLoginTokenValidator {
    pub(crate) fn new(login_policy: Arc<dyn RebornLoginPolicy>) -> Self {
        Self { login_policy }
    }
}

#[async_trait]
impl ReusableLoginTokenValidator for RebornReusableLoginTokenValidator {
    async fn validate(&self, tenant: &TenantId, user_id: &UserId) -> bool {
        match self
            .login_policy
            .authorize_reusable_login_token(tenant, user_id)
            .await
        {
            Ok(authorized) => authorized,
            Err(error) => {
                tracing::warn!(
                    tenant_id = %tenant,
                    subject_user_id = %user_id,
                    error = %error,
                    "reusable login-token policy lookup failed"
                );
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use ironclaw_reborn_identity::RebornIdentityError;

    enum PolicyMode {
        Allow,
        Deny,
        Fail,
    }

    struct StubLoginPolicy {
        mode: PolicyMode,
    }

    #[async_trait]
    impl RebornLoginPolicy for StubLoginPolicy {
        async fn authorize_admin_login_token_issuance(
            &self,
            _tenant_id: &TenantId,
            _actor_user_id: &UserId,
        ) -> Result<bool, RebornIdentityError> {
            match self.mode {
                PolicyMode::Allow => Ok(true),
                PolicyMode::Deny => Ok(false),
                PolicyMode::Fail => Err(RebornIdentityError::Backend("offline".to_string())),
            }
        }

        async fn authorize_reusable_login_token(
            &self,
            _tenant_id: &TenantId,
            _subject_user_id: &UserId,
        ) -> Result<bool, RebornIdentityError> {
            match self.mode {
                PolicyMode::Allow => Ok(true),
                PolicyMode::Deny => Ok(false),
                PolicyMode::Fail => Err(RebornIdentityError::Backend("offline".to_string())),
            }
        }
    }

    struct StubMinter {
        fail: bool,
        minted_for: Mutex<Vec<UserId>>,
    }

    #[async_trait]
    impl AdminLoginTokenMinter for StubMinter {
        async fn mint(&self, _tenant: &TenantId, user_id: &UserId) -> Result<SecretString, String> {
            self.minted_for
                .lock()
                .expect("mint recorder")
                .push(user_id.clone());
            if self.fail {
                Err("signer offline".to_string())
            } else {
                Ok(SecretString::from("signed-token"))
            }
        }
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant-a").expect("tenant")
    }

    fn actor() -> UserId {
        UserId::new("admin-a").expect("actor")
    }

    #[tokio::test]
    async fn issuer_denial_and_policy_failure_do_not_call_the_signer() {
        for (mode, expected) in [
            (PolicyMode::Deny, AdminUserError::Forbidden),
            (PolicyMode::Fail, AdminUserError::Unavailable),
        ] {
            let minter = Arc::new(StubMinter {
                fail: false,
                minted_for: Mutex::new(Vec::new()),
            });
            let issuer = RebornAdminLoginTokenIssuer::new(
                Arc::new(StubLoginPolicy { mode }),
                minter.clone(),
            );
            let error = match issuer.issue_login_token(&tenant(), &actor(), false).await {
                Ok(_) => panic!("issuance must fail"),
                Err(error) => error,
            };
            assert_eq!(error, expected);
            assert!(
                minter.minted_for.lock().expect("mint recorder").is_empty(),
                "policy denial must happen before signing"
            );
        }
    }

    #[tokio::test]
    async fn signer_failure_is_sanitized_and_success_binds_token_to_allocated_subject() {
        let failing = RebornAdminLoginTokenIssuer::new(
            Arc::new(StubLoginPolicy {
                mode: PolicyMode::Allow,
            }),
            Arc::new(StubMinter {
                fail: true,
                minted_for: Mutex::new(Vec::new()),
            }),
        );
        let error = match failing.issue_login_token(&tenant(), &actor(), false).await {
            Ok(_) => panic!("signer failure must reject issuance"),
            Err(error) => error,
        };
        assert_eq!(error, AdminUserError::Unavailable);

        let minter = Arc::new(StubMinter {
            fail: false,
            minted_for: Mutex::new(Vec::new()),
        });
        let issued = RebornAdminLoginTokenIssuer::new(
            Arc::new(StubLoginPolicy {
                mode: PolicyMode::Allow,
            }),
            minter.clone(),
        )
        .issue_login_token(&tenant(), &actor(), false)
        .await
        .expect("issuance");
        assert_eq!(
            minter.minted_for.lock().expect("mint recorder").as_slice(),
            &[issued.subject_user_id]
        );
    }

    #[tokio::test]
    async fn authenticated_operator_may_issue_without_a_directory_record() {
        let minter = Arc::new(StubMinter {
            fail: false,
            minted_for: Mutex::new(Vec::new()),
        });
        let issued = RebornAdminLoginTokenIssuer::new(
            Arc::new(StubLoginPolicy {
                mode: PolicyMode::Deny,
            }),
            minter.clone(),
        )
        .issue_login_token(&tenant(), &actor(), true)
        .await
        .expect("the host operator is an implicit owner");
        assert_eq!(
            minter.minted_for.lock().expect("mint recorder").as_slice(),
            &[issued.subject_user_id]
        );
    }

    #[tokio::test]
    async fn reusable_validator_fails_closed_on_denial_and_backend_error() {
        for mode in [PolicyMode::Deny, PolicyMode::Fail] {
            let validator =
                RebornReusableLoginTokenValidator::new(Arc::new(StubLoginPolicy { mode }));
            assert!(
                !validator
                    .validate(&tenant(), &UserId::new("subject").expect("user"))
                    .await
            );
        }
    }
}
