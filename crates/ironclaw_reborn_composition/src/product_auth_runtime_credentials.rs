use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount,
    CredentialAccountRecordSource, CredentialAccountSelectionRequest, CredentialAccountStatus,
    CredentialOwnership,
};
use ironclaw_host_api::{CredentialStageError, SecretHandle};
use ironclaw_host_runtime::{RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver};

#[derive(Clone)]
pub(crate) struct ProductAuthRuntimeCredentialResolver {
    accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
}

impl ProductAuthRuntimeCredentialResolver {
    pub(crate) fn new(accounts: Arc<dyn RuntimeCredentialAccountSelectionService>) -> Self {
        Self { accounts }
    }
}

#[async_trait]
pub(crate) trait RuntimeCredentialAccountSelectionService: Send + Sync {
    async fn select_unique_configured_runtime_account(
        &self,
        request: RuntimeCredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError>;
}

pub(crate) struct RuntimeCredentialAccountSelectionRequest {
    lookup: CredentialAccountSelectionRequest,
    runtime_scope: AuthProductScope,
}

impl RuntimeCredentialAccountSelectionRequest {
    pub(crate) fn new(
        lookup: CredentialAccountSelectionRequest,
        runtime_scope: AuthProductScope,
    ) -> Self {
        Self {
            lookup,
            runtime_scope,
        }
    }
}

pub(crate) struct ProductAuthRuntimeCredentialAccountSelector {
    accounts: Arc<dyn CredentialAccountRecordSource>,
}

impl ProductAuthRuntimeCredentialAccountSelector {
    pub(crate) fn new(accounts: Arc<dyn CredentialAccountRecordSource>) -> Self {
        Self { accounts }
    }
}

impl std::fmt::Debug for ProductAuthRuntimeCredentialAccountSelector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductAuthRuntimeCredentialAccountSelector")
            .field("accounts", &"<credential_account_record_source>")
            .finish()
    }
}

#[async_trait]
impl RuntimeCredentialAccountSelectionService for ProductAuthRuntimeCredentialAccountSelector {
    async fn select_unique_configured_runtime_account(
        &self,
        request: RuntimeCredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError> {
        let configured = self
            .accounts
            .accounts_for_owner(&request.lookup.scope)
            .await?
            .into_iter()
            .filter(|account| {
                account.provider == request.lookup.provider
                    && account.status == CredentialAccountStatus::Configured
                    && account_visible_from_runtime_scope(account, &request.runtime_scope)
            })
            .collect::<Vec<_>>();
        if configured.is_empty() {
            return Err(AuthProductError::CredentialMissing);
        }
        let selectable = configured
            .into_iter()
            .filter(|account| {
                account.is_authorized_for_requester(request.lookup.requester_extension.as_ref())
            })
            .collect::<Vec<_>>();
        match selectable.as_slice() {
            [] => Err(AuthProductError::CrossScopeDenied),
            [account] => Ok(account.clone()),
            _ => Err(AuthProductError::AccountSelectionRequired),
        }
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
    ) -> Result<SecretHandle, CredentialStageError> {
        let auth_scope =
            AuthProductScope::new(runtime_account_owner_scope(request.scope), AuthSurface::Api);
        let provider = AuthProviderId::new(request.provider.as_str()).map_err(|e| {
            tracing::debug!(
                provider = %request.provider.as_str(),
                err = %e,
                "product-auth provider id is invalid"
            );
            CredentialStageError::Backend
        })?;
        let account = self
            .accounts
            .select_unique_configured_runtime_account(
                RuntimeCredentialAccountSelectionRequest::new(
                    CredentialAccountSelectionRequest::new(auth_scope, provider)
                        .for_extension(request.requester_extension.clone()),
                    AuthProductScope::new(request.scope.clone(), AuthSurface::Api),
                ),
            )
            .await
            .map_err(map_account_error)?;
        if account.status != CredentialAccountStatus::Configured {
            return Err(CredentialStageError::AuthRequired);
        }
        // A Configured account missing access_secret indicates data corruption,
        // not a re-auth prompt. The durable product-auth store (#4234) preserves
        // the Configured ↔ access_secret=Some invariant (manual-token submit sets
        // both together; cleanup/uninstall clears status to Revoked together with
        // the handle), so this branch can only fire on corrupt state. Return
        // Backend so the caller does not loop through re-auth.
        account.access_secret.ok_or(CredentialStageError::Backend)
    }
}

fn account_visible_from_runtime_scope(
    account: &CredentialAccount,
    runtime_scope: &AuthProductScope,
) -> bool {
    if account.ownership == CredentialOwnership::UserReusable {
        return true;
    }
    let account_resource = &account.scope.resource;
    let runtime_resource = &runtime_scope.resource;
    account_resource.tenant_id == runtime_resource.tenant_id
        && account_resource.user_id == runtime_resource.user_id
        && account_resource.agent_id == runtime_resource.agent_id
        && account_resource.project_id == runtime_resource.project_id
        && account_resource.mission_id == runtime_resource.mission_id
        && account_resource.thread_id == runtime_resource.thread_id
        && account.scope.session_id == runtime_scope.session_id
}

fn runtime_account_owner_scope(
    scope: &ironclaw_host_api::ResourceScope,
) -> ironclaw_host_api::ResourceScope {
    let mut owner = scope.clone();
    owner.mission_id = None;
    owner.thread_id = None;
    owner
}

fn map_account_error(error: AuthProductError) -> CredentialStageError {
    match error {
        AuthProductError::CredentialMissing
        | AuthProductError::CrossScopeDenied
        | AuthProductError::AccountSelectionRequired => CredentialStageError::AuthRequired,
        _ => CredentialStageError::Backend,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_auth::{
        CredentialAccountLabel, CredentialAccountService, CredentialOwnership,
        InMemoryAuthProductServices, NewCredentialAccount,
    };
    use ironclaw_host_api::{
        ExtensionId, InvocationId, MissionId, ResourceScope, RuntimeCredentialAccountProviderId,
        ThreadId, UserId,
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
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

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
    async fn resolver_matches_callback_setup_account_from_runtime_invocation() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let mut setup_scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        setup_scope.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
        let mut runtime_scope = setup_scope.clone();
        runtime_scope.invocation_id = InvocationId::new();
        let access_secret = SecretHandle::new("github_manual_access").unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::new(setup_scope, AuthSurface::Callback),
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
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let resolved = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &runtime_scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(resolved, access_secret);
    }

    #[tokio::test]
    async fn resolver_matches_reusable_setup_account_from_new_thread() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let mut setup_scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        setup_scope.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
        let mut runtime_scope = setup_scope.clone();
        runtime_scope.thread_id = Some(ThreadId::new("thread-auth-2").unwrap());
        runtime_scope.invocation_id = InvocationId::new();
        let access_secret = SecretHandle::new("github_manual_access").unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::new(setup_scope, AuthSurface::Callback),
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
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let resolved = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &runtime_scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(resolved, access_secret);
    }

    #[tokio::test]
    async fn resolver_matches_reusable_setup_account_from_new_mission() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let mut setup_scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        setup_scope.mission_id = Some(MissionId::new("mission-auth-1").unwrap());
        let mut runtime_scope = setup_scope.clone();
        runtime_scope.mission_id = Some(MissionId::new("mission-auth-2").unwrap());
        runtime_scope.invocation_id = InvocationId::new();
        let access_secret = SecretHandle::new("github_manual_access").unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::new(setup_scope, AuthSurface::Callback),
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
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let resolved = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &runtime_scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(resolved, access_secret);
    }

    #[tokio::test]
    async fn resolver_rejects_extension_owned_account_from_new_thread() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let mut setup_scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        setup_scope.thread_id = Some(ThreadId::new("thread-auth-1").unwrap());
        let mut runtime_scope = setup_scope.clone();
        runtime_scope.thread_id = Some(ThreadId::new("thread-auth-2").unwrap());
        runtime_scope.invocation_id = InvocationId::new();
        accounts
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::new(setup_scope, AuthSurface::Callback),
                provider: AuthProviderId::new("github").unwrap(),
                label: CredentialAccountLabel::new("work github").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::ExtensionOwned,
                owner_extension: Some(ExtensionId::new("github").unwrap()),
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("github_manual_access").unwrap()),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &runtime_scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, CredentialStageError::AuthRequired);
    }

    #[tokio::test]
    async fn resolver_maps_missing_account_to_auth_required() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));
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

        assert_eq!(error, CredentialStageError::AuthRequired);
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
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, CredentialStageError::AuthRequired);
    }

    #[tokio::test]
    async fn resolver_maps_configured_account_without_access_secret_to_backend() {
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
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap_err();

        // Data corruption: should be Backend, not AuthRequired (re-auth would not fix it).
        // The durable product-auth store preserves Configured ↔ access_secret=Some,
        // so this state cannot arise from legitimate cleanup or rotation paths.
        assert_eq!(error, CredentialStageError::Backend);
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
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, CredentialStageError::AuthRequired);
    }
}
