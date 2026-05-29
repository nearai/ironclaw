use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface,
    CredentialAccountLookupRequest, CredentialAccountSelectionRequest, CredentialAccountService,
    CredentialAccountStatus,
};
use ironclaw_host_api::SecretHandle;
use ironclaw_host_runtime::{
    RuntimeCredentialAccountError, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver,
};
use tracing;

#[derive(Clone)]
pub(crate) struct ProductAuthRuntimeCredentialResolver {
    accounts: Arc<dyn CredentialAccountService>,
}

impl ProductAuthRuntimeCredentialResolver {
    pub(crate) fn new(accounts: Arc<dyn CredentialAccountService>) -> Self {
        Self { accounts }
    }
}

impl std::fmt::Debug for ProductAuthRuntimeCredentialResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductAuthRuntimeCredentialResolver")
            .field("accounts", &"<credential_account_service>")
            .finish()
    }
}

#[async_trait]
impl RuntimeCredentialAccountResolver for ProductAuthRuntimeCredentialResolver {
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<SecretHandle, RuntimeCredentialAccountError> {
        let auth_scope = AuthProductScope::new(request.scope.clone(), AuthSurface::Api);
        let provider = AuthProviderId::new(request.provider.as_str()).map_err(|e| {
            tracing::debug!(
                provider = %request.provider.as_str(),
                err = %e,
                "product-auth provider id is invalid"
            );
            RuntimeCredentialAccountError::Failed
        })?;
        let selected = self
            .accounts
            .select_unique_configured_account(
                CredentialAccountSelectionRequest::new(auth_scope.clone(), provider)
                    .for_extension(request.requester_extension.clone()),
            )
            .await
            .map_err(map_account_error)?;
        let account = self
            .accounts
            .get_account(
                CredentialAccountLookupRequest::new(auth_scope, selected.id)
                    .for_extension(request.requester_extension.clone()),
            )
            .await
            .map_err(map_account_error)?
            .ok_or(RuntimeCredentialAccountError::AuthRequired)?;
        if account.status != CredentialAccountStatus::Configured {
            return Err(RuntimeCredentialAccountError::AuthRequired);
        }
        // A Configured account missing access_secret indicates data corruption, not
        // a re-auth prompt. Return Failed so the caller does not loop through auth.
        account
            .access_secret
            .ok_or(RuntimeCredentialAccountError::Failed)
    }
}

fn map_account_error(error: AuthProductError) -> RuntimeCredentialAccountError {
    match error {
        AuthProductError::CredentialMissing
        | AuthProductError::CrossScopeDenied
        | AuthProductError::AccountSelectionRequired => RuntimeCredentialAccountError::AuthRequired,
        _ => RuntimeCredentialAccountError::Failed,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_auth::{
        CredentialAccountLabel, CredentialOwnership, InMemoryAuthProductServices,
        NewCredentialAccount,
    };
    use ironclaw_host_api::{
        ExtensionId, InvocationId, ResourceScope, RuntimeCredentialAccountProviderId, UserId,
    };

    use super::*;

    #[tokio::test]
    async fn resolver_returns_configured_product_auth_access_secret() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let access_secret = SecretHandle::new("github_manual_access").unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("github").unwrap(),
                label: CredentialAccountLabel::new("work github").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(access_secret.clone()),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();
        let resolver = ProductAuthRuntimeCredentialResolver::new(accounts);

        let resolved = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(resolved, access_secret);
    }

    #[tokio::test]
    async fn resolver_maps_missing_account_to_auth_required() {
        let resolver =
            ProductAuthRuntimeCredentialResolver::new(Arc::new(InMemoryAuthProductServices::new()));
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, RuntimeCredentialAccountError::AuthRequired);
    }

    #[tokio::test]
    async fn resolver_maps_unconfigured_account_status_to_auth_required() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("github").unwrap(),
                label: CredentialAccountLabel::new("work github").unwrap(),
                status: CredentialAccountStatus::PendingSetup,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: None,
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();
        let resolver = ProductAuthRuntimeCredentialResolver::new(accounts);

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, RuntimeCredentialAccountError::AuthRequired);
    }

    #[tokio::test]
    async fn resolver_maps_configured_account_without_access_secret_to_failed() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("github").unwrap(),
                label: CredentialAccountLabel::new("work github").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: None, // Configured but missing secret — data corruption
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();
        let resolver = ProductAuthRuntimeCredentialResolver::new(accounts);

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap_err();

        // Data corruption: should be Failed, not AuthRequired (re-auth would not fix it)
        assert_eq!(error, RuntimeCredentialAccountError::Failed);
    }

    #[tokio::test]
    async fn resolver_maps_multiple_accounts_to_auth_required() {
        // AccountSelectionRequired fires when two accounts match the same provider/scope.
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        // Create two accounts for the same provider.
        for label in ["personal github", "work github"] {
            accounts
                .create_account(NewCredentialAccount {
                    scope: auth_scope.clone(),
                    provider: AuthProviderId::new("github").unwrap(),
                    label: CredentialAccountLabel::new(label).unwrap(),
                    status: CredentialAccountStatus::Configured,
                    ownership: CredentialOwnership::UserReusable,
                    owner_extension: None,
                    granted_extensions: Vec::new(),
                    access_secret: Some(SecretHandle::new("token").unwrap()),
                    refresh_secret: None,
                    scopes: Vec::new(),
                })
                .await
                .unwrap();
        }
        let resolver = ProductAuthRuntimeCredentialResolver::new(accounts);

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, RuntimeCredentialAccountError::AuthRequired);
    }
}
