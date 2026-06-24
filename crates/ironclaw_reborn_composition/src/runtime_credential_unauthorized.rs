use std::sync::Arc;

use crate::runtime_credential_reauth::RuntimeCredentialReauthBridge;
use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountId, CredentialAccountService,
    CredentialAccountStatus, CredentialRefreshRequest,
};
use ironclaw_host_api::{
    CapabilityId, ResourceScope, RuntimeCredentialAccountSurface, RuntimeCredentialUnauthorized,
    RuntimeCredentialUnauthorizedPolicy, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
};

pub(crate) struct RuntimeCredentialUnauthorizedRecoveryEgress {
    inner: Arc<dyn RuntimeHttpEgress>,
    credential_accounts: Arc<dyn CredentialAccountService>,
    reauth_bridge: Arc<RuntimeCredentialReauthBridge>,
}

impl RuntimeCredentialUnauthorizedRecoveryEgress {
    pub(crate) fn new(
        inner: Arc<dyn RuntimeHttpEgress>,
        credential_accounts: Arc<dyn CredentialAccountService>,
        reauth_bridge: Arc<RuntimeCredentialReauthBridge>,
    ) -> Self {
        Self {
            inner,
            credential_accounts,
            reauth_bridge,
        }
    }

    async fn recover_unauthorized_credential(
        &self,
        request_scope: &ResourceScope,
        capability_id: &CapabilityId,
        response: &RuntimeHttpEgressResponse,
    ) -> Result<(), ironclaw_auth::AuthProductError> {
        let Some(unauthorized) = &response.credential_unauthorized else {
            return Ok(());
        };
        if unauthorized.scope != *request_scope {
            tracing::debug!(
                request_scope = ?request_scope,
                marker_scope = ?unauthorized.scope,
                "runtime HTTP credential unauthorized marker ignored because request scope did not match"
            );
            return Ok(());
        }
        let account_id = CredentialAccountId::from_uuid(unauthorized.account_id.as_uuid());
        let scope = AuthProductScope::credential_owner(
            &unauthorized.scope,
            auth_surface(unauthorized.account_surface),
        );
        let account_updated_at = unauthorized.account_updated_at;
        match unauthorized.unauthorized_policy {
            RuntimeCredentialUnauthorizedPolicy::RevokeAccount => {
                if self
                    .revoke_if_unchanged(
                        &scope,
                        account_id,
                        account_updated_at,
                        unauthorized.requester_extension.clone(),
                    )
                    .await?
                {
                    self.record_recovered_auth_required(request_scope, capability_id, unauthorized);
                }
            }
            RuntimeCredentialUnauthorizedPolicy::RefreshAccount => {
                if self
                    .refresh_if_unchanged(&scope, account_id, account_updated_at, unauthorized)
                    .await?
                {
                    self.record_recovered_auth_required(request_scope, capability_id, unauthorized);
                }
            }
        }
        Ok(())
    }

    fn record_recovered_auth_required(
        &self,
        request_scope: &ResourceScope,
        capability_id: &CapabilityId,
        unauthorized: &RuntimeCredentialUnauthorized,
    ) {
        self.reauth_bridge.record_recovered_auth_required(
            request_scope,
            capability_id,
            vec![unauthorized.auth_requirement.clone()],
        );
    }

    async fn revoke_if_unchanged(
        &self,
        scope: &AuthProductScope,
        account_id: CredentialAccountId,
        account_updated_at: ironclaw_host_api::Timestamp,
        requester_extension: Option<ironclaw_host_api::ExtensionId>,
    ) -> Result<bool, ironclaw_auth::AuthProductError> {
        let account_id_for_log = account_id.to_string();
        match self
            .credential_accounts
            .revoke_if_unchanged(scope, account_id, account_updated_at, requester_extension)
            .await?
        {
            Some(_) => Ok(true),
            None => {
                tracing::debug!(
                    account_id = %account_id_for_log,
                    "runtime HTTP credential unauthorized recovery skipped because account changed or disappeared after staging"
                );
                Ok(false)
            }
        }
    }

    async fn refresh_if_unchanged(
        &self,
        scope: &AuthProductScope,
        account_id: CredentialAccountId,
        account_updated_at: ironclaw_host_api::Timestamp,
        unauthorized: &RuntimeCredentialUnauthorized,
    ) -> Result<bool, ironclaw_auth::AuthProductError> {
        let request = match refresh_request(scope, account_id, unauthorized) {
            Ok(request) => request,
            Err(_) => return Ok(false),
        };
        match self
            .credential_accounts
            .refresh_if_unchanged(request, account_updated_at)
            .await?
        {
            Some(report) => {
                if report.account.status != CredentialAccountStatus::Configured {
                    return Ok(true);
                }
                if !report.refreshed {
                    tracing::debug!(
                        account_id = %unauthorized.account_id,
                        "runtime HTTP credential unauthorized recovery refresh left account unchanged; requiring re-auth"
                    );
                    return Ok(true);
                }
                Ok(false)
            }
            None => {
                tracing::debug!(
                    account_id = %unauthorized.account_id,
                    "runtime HTTP credential unauthorized recovery skipped refresh because account changed or disappeared after staging"
                );
                Ok(false)
            }
        }
    }
}

fn auth_surface(surface: RuntimeCredentialAccountSurface) -> AuthSurface {
    match surface {
        RuntimeCredentialAccountSurface::Chat => AuthSurface::Chat,
        RuntimeCredentialAccountSurface::Web => AuthSurface::Web,
        RuntimeCredentialAccountSurface::Cli => AuthSurface::Cli,
        RuntimeCredentialAccountSurface::Tui => AuthSurface::Tui,
        RuntimeCredentialAccountSurface::Api => AuthSurface::Api,
        RuntimeCredentialAccountSurface::SetupAdmin => AuthSurface::SetupAdmin,
        RuntimeCredentialAccountSurface::Callback => AuthSurface::Callback,
    }
}

fn refresh_request(
    scope: &AuthProductScope,
    account_id: CredentialAccountId,
    unauthorized: &RuntimeCredentialUnauthorized,
) -> Result<CredentialRefreshRequest, ironclaw_auth::AuthProductError> {
    let provider =
        AuthProviderId::new(unauthorized.account_provider.as_str()).map_err(|error| {
            tracing::debug!(
                provider = %unauthorized.account_provider.as_str(),
                err = %error,
                "runtime HTTP credential unauthorized marker carried an invalid provider id"
            );
            ironclaw_auth::AuthProductError::MalformedConfig
        })?;
    let mut request = CredentialRefreshRequest::new(scope.clone(), provider, account_id);
    if let Some(requester_extension) = unauthorized.requester_extension.clone() {
        request = request.for_extension(requester_extension);
    }
    Ok(request)
}

#[async_trait]
impl RuntimeHttpEgress for RuntimeCredentialUnauthorizedRecoveryEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let request_scope = request.scope.clone();
        let capability_id = request.capability_id.clone();
        let response = self.inner.execute(request).await?;
        if response.status == 401 {
            self.recover_unauthorized_credential(&request_scope, &capability_id, &response)
                .await
                .map_err(|error| RuntimeHttpEgressError::Credential {
                    reason: error.to_string(),
                })?;
        } else if response.credential_unauthorized.is_some() {
            tracing::debug!(
                status = response.status,
                "runtime HTTP credential unauthorized marker ignored because response was not 401"
            );
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests;
