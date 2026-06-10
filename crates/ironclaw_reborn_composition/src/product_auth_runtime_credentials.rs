use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount,
    CredentialAccountRecordSource, CredentialAccountSelectionRequest, CredentialAccountStatus,
    CredentialOwnership, ProviderScope,
};
use ironclaw_host_api::CredentialStageError;
use ironclaw_host_api::RuntimeCredentialAccountSetup;
use ironclaw_host_runtime::{
    RuntimeCredentialAccessSecret, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver,
};

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
    setup: RuntimeCredentialAccountSetup,
    provider_scopes: Vec<ProviderScope>,
}

impl RuntimeCredentialAccountSelectionRequest {
    pub(crate) fn new(
        lookup: CredentialAccountSelectionRequest,
        runtime_scope: AuthProductScope,
        setup: RuntimeCredentialAccountSetup,
        provider_scopes: Vec<ProviderScope>,
    ) -> Self {
        Self {
            lookup,
            runtime_scope,
            setup,
            provider_scopes,
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
                    && account_has_provider_scopes(
                        account,
                        &request.setup,
                        &request.provider_scopes,
                    )
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
            _ => select_latest_duplicate_user_reusable_account(&selectable)
                .ok_or(AuthProductError::AccountSelectionRequired),
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
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
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
        let provider_scopes = request
            .provider_scopes
            .iter()
            .map(|scope| {
                ProviderScope::new(scope.clone()).map_err(|e| {
                    tracing::debug!(
                        scope = %scope,
                        err = %e,
                        "runtime credential provider scope is invalid"
                    );
                    CredentialStageError::Backend
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let account = self
            .accounts
            .select_unique_configured_runtime_account(
                RuntimeCredentialAccountSelectionRequest::new(
                    CredentialAccountSelectionRequest::new(auth_scope, provider)
                        .for_extension(request.requester_extension.clone()),
                    AuthProductScope::new(request.scope.clone(), AuthSurface::Api),
                    request.setup.clone(),
                    provider_scopes,
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
        let handle = account.access_secret.ok_or(CredentialStageError::Backend)?;
        Ok(RuntimeCredentialAccessSecret {
            scope: account.scope.resource,
            handle,
        })
    }
}

fn account_has_provider_scopes(
    account: &CredentialAccount,
    setup: &RuntimeCredentialAccountSetup,
    required_scopes: &[ProviderScope],
) -> bool {
    if !credential_setup_requires_stored_scopes(setup) {
        return true;
    }
    required_scopes
        .iter()
        .all(|required| account.scopes.iter().any(|scope| scope == required))
}

fn credential_setup_requires_stored_scopes(setup: &RuntimeCredentialAccountSetup) -> bool {
    match setup {
        RuntimeCredentialAccountSetup::OAuth { .. } => true,
        RuntimeCredentialAccountSetup::ManualToken => false,
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

/// Runtime "default account" rule for the credential gate.
///
/// The runtime auth gate has no interactive account picker: when a capability
/// needs a provider credential the engine must resolve to exactly one account
/// or it raises an auth gate. When more than one reusable account matches
/// (which happens because the OAuth `account_label` historically encoded the
/// triggering capability — "gmail google", "google-calendar google", "google"
/// — so the same login produced several rows), we deterministically select the
/// **most-recently-used** account rather than failing with
/// `AccountSelectionRequired` (→ `AuthRequired`), which re-prompted the user on
/// every capability call (#auth-gate-reuse).
///
/// Recency *is* the default: the setup-time account picker controls which
/// account wins at runtime by touching it (overwrite bumps `updated_at`;
/// create-new starts a fresh, now-latest account). Account *selection* between
/// genuinely distinct logins is a setup-time concern; at runtime we always have
/// a usable default.
///
/// We still restrict to the *reusable, unbound* class — `UserReusable`, no
/// `owner_extension`, no `granted_extensions`, `access_secret` present — so this
/// never auto-selects across extension-owned or requester-bound accounts; those
/// keep their explicit binding semantics.
fn select_latest_duplicate_user_reusable_account(
    accounts: &[CredentialAccount],
) -> Option<CredentialAccount> {
    let first = accounts.first()?;
    if !accounts.iter().all(|account| {
        account.provider == first.provider
            && account.ownership == CredentialOwnership::UserReusable
            && account.owner_extension.is_none()
            && account.granted_extensions.is_empty()
            && account.access_secret.is_some()
    }) {
        return None;
    }
    accounts
        .iter()
        .max_by_key(|account| (account.updated_at, account.created_at, account.id))
        .cloned()
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
        SecretHandle, ThreadId, UserId,
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
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(resolved.handle, access_secret);
        assert_eq!(resolved.scope, scope);
    }

    #[tokio::test]
    async fn resolver_accepts_unscoped_github_manual_token_for_scoped_runtime_request() {
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
        let required_scopes = vec!["repo".to_string()];

        let resolved = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &required_scopes,
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .expect("GitHub PAT scopes are encoded in the token and cannot be introspected");

        assert_eq!(resolved.handle, access_secret);
        assert_eq!(resolved.scope, scope);
    }

    #[tokio::test]
    async fn resolver_does_not_use_reusable_account_from_different_user() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let alice_scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let admin_scope =
            ResourceScope::local_default(UserId::new("admin").unwrap(), InvocationId::new())
                .unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::new(alice_scope, AuthSurface::Api),
                provider: AuthProviderId::new("google").unwrap(),
                label: CredentialAccountLabel::new("alice google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("alice-google-access").unwrap()),
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
                scope: &admin_scope,
                provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
                requester_extension: &ExtensionId::new("gmail").unwrap(),
            })
            .await
            .expect_err("admin must not resolve alice's reusable account");

        assert_eq!(error, CredentialStageError::AuthRequired);
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
                scope: AuthProductScope::new(setup_scope.clone(), AuthSurface::Callback),
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
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(resolved.handle, access_secret);
        assert_eq!(resolved.scope, setup_scope);
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
                scope: AuthProductScope::new(setup_scope.clone(), AuthSurface::Callback),
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
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(resolved.handle, access_secret);
        assert_eq!(resolved.scope, setup_scope);
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
                scope: AuthProductScope::new(setup_scope.clone(), AuthSurface::Callback),
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
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(resolved.handle, access_secret);
        assert_eq!(resolved.scope, setup_scope);
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
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
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
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, CredentialStageError::AuthRequired);
    }

    #[tokio::test]
    async fn resolver_requires_requested_provider_scopes() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("google").unwrap(),
                label: CredentialAccountLabel::new("work google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("google_manual_access").unwrap()),
                refresh_secret: None,
                scopes: vec![
                    ProviderScope::new("https://www.googleapis.com/auth/gmail.send").unwrap(),
                ],
            })
            .await
            .unwrap();
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));
        let required_scopes = vec!["https://www.googleapis.com/auth/drive".to_string()];

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
                setup: &RuntimeCredentialAccountSetup::OAuth {
                    scopes: required_scopes.clone(),
                },
                provider_scopes: &required_scopes,
                requester_extension: &ExtensionId::new("google-drive").unwrap(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, CredentialStageError::AuthRequired);
    }

    #[tokio::test]
    async fn resolver_does_not_treat_unscoped_google_account_as_scoped() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("google").unwrap(),
                label: CredentialAccountLabel::new("work google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("google_manual_access").unwrap()),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));
        let required_scopes = vec!["https://www.googleapis.com/auth/drive".to_string()];

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
                setup: &RuntimeCredentialAccountSetup::OAuth {
                    scopes: required_scopes.clone(),
                },
                provider_scopes: &required_scopes,
                requester_extension: &ExtensionId::new("google-drive").unwrap(),
            })
            .await
            .expect_err("unscoped OAuth accounts must not satisfy scoped Google requirements");

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
        let required_scopes = vec!["repo".to_string()];

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("github").unwrap(),
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &required_scopes,
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
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
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
    async fn resolver_uses_most_recent_account_across_multiple_reusable_logins() {
        // Runtime default rule (#auth-gate-reuse): when several reusable,
        // unbound accounts match the same provider — even under different
        // labels — the gate has no interactive picker, so the resolver selects
        // the most-recently-used account rather than failing with
        // `AccountSelectionRequired` (which re-prompted on every call). The
        // setup-time picker controls which one wins by bumping its recency.
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let latest_secret = SecretHandle::new("work-token").unwrap();
        // Two reusable accounts for the same provider under distinct labels.
        // The second one is created later, so it is the most-recently-used.
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: AuthProviderId::new("github").unwrap(),
                label: CredentialAccountLabel::new("personal github").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("personal-token").unwrap()),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("github").unwrap(),
                label: CredentialAccountLabel::new("work github").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(latest_secret.clone()),
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
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .expect("runtime must resolve to the most-recent reusable account, not re-prompt");

        assert_eq!(resolved.handle, latest_secret);
    }

    #[tokio::test]
    async fn resolver_uses_latest_duplicate_user_reusable_account() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let first_secret = SecretHandle::new("old-token").unwrap();
        let latest_secret = SecretHandle::new("new-token").unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: AuthProviderId::new("github").unwrap(),
                label: CredentialAccountLabel::new("GitHub").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(first_secret),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("github").unwrap(),
                label: CredentialAccountLabel::new("GitHub").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(latest_secret.clone()),
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
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
                requester_extension: &ExtensionId::new("github").unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(resolved.handle, latest_secret);
    }

    /// Direct reproduction of the reported bug (#auth-gate-reuse): a single
    /// Google login authenticated through different gate/setup surfaces ends up
    /// stored as multiple reusable accounts under capability-derived labels
    /// ("gmail google", "google-calendar google"). The runtime resolver must
    /// pick the most-recent usable credential instead of returning
    /// `AuthRequired`, which re-prompted the user on every gmail/calendar call.
    #[tokio::test]
    async fn resolver_resolves_google_capability_labeled_duplicates() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let gmail_scope =
            ProviderScope::new("https://www.googleapis.com/auth/gmail.modify").unwrap();
        let latest_secret = SecretHandle::new("calendar-surface-token").unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: AuthProviderId::new("google").unwrap(),
                label: CredentialAccountLabel::new("gmail google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("gmail-surface-token").unwrap()),
                refresh_secret: None,
                scopes: vec![gmail_scope.clone()],
            })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("google").unwrap(),
                label: CredentialAccountLabel::new("google-calendar google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(latest_secret.clone()),
                refresh_secret: None,
                scopes: vec![gmail_scope.clone()],
            })
            .await
            .unwrap();
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let resolved = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
                setup: &RuntimeCredentialAccountSetup::OAuth {
                    scopes: vec![gmail_scope.as_str().to_string()],
                },
                provider_scopes: &[gmail_scope.as_str().to_string()],
                requester_extension: &ExtensionId::new("gmail").unwrap(),
            })
            .await
            .expect("capability-labeled google duplicates must resolve, not re-prompt");

        assert_eq!(resolved.handle, latest_secret);
    }

    #[tokio::test]
    async fn resolver_does_not_auto_select_mixed_reusable_and_extension_owned_accounts() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let requester = ExtensionId::new("gmail").unwrap();
        let google_scope =
            ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: AuthProviderId::new("google").unwrap(),
                label: CredentialAccountLabel::new("reusable google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("reusable-token").unwrap()),
                refresh_secret: None,
                scopes: vec![google_scope.clone()],
            })
            .await
            .unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("google").unwrap(),
                label: CredentialAccountLabel::new("extension google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::ExtensionOwned,
                owner_extension: Some(requester.clone()),
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("extension-token").unwrap()),
                refresh_secret: None,
                scopes: vec![google_scope.clone()],
            })
            .await
            .unwrap();
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
                setup: &RuntimeCredentialAccountSetup::OAuth {
                    scopes: vec![google_scope.as_str().to_string()],
                },
                provider_scopes: &[google_scope.as_str().to_string()],
                requester_extension: &requester,
            })
            .await
            .expect_err("mixed ownership must require explicit account selection");

        assert_eq!(error, CredentialStageError::AuthRequired);
    }

    #[tokio::test]
    async fn resolver_does_not_auto_select_mixed_reusable_and_shared_admin_accounts() {
        let accounts = Arc::new(InMemoryAuthProductServices::new());
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let requester = ExtensionId::new("gmail").unwrap();
        let google_scope =
            ProviderScope::new("https://www.googleapis.com/auth/gmail.readonly").unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: AuthProviderId::new("google").unwrap(),
                label: CredentialAccountLabel::new("reusable google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("reusable-token").unwrap()),
                refresh_secret: None,
                scopes: vec![google_scope.clone()],
            })
            .await
            .unwrap();
        accounts
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: AuthProviderId::new("google").unwrap(),
                label: CredentialAccountLabel::new("shared google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::SharedAdminManaged,
                owner_extension: None,
                granted_extensions: vec![requester.clone()],
                access_secret: Some(SecretHandle::new("shared-token").unwrap()),
                refresh_secret: None,
                scopes: vec![google_scope.clone()],
            })
            .await
            .unwrap();
        let resolver = ProductAuthRuntimeCredentialResolver::new(Arc::new(
            ProductAuthRuntimeCredentialAccountSelector::new(accounts),
        ));

        let error = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &scope,
                provider: &RuntimeCredentialAccountProviderId::new("google").unwrap(),
                setup: &RuntimeCredentialAccountSetup::OAuth {
                    scopes: vec![google_scope.as_str().to_string()],
                },
                provider_scopes: &[google_scope.as_str().to_string()],
                requester_extension: &requester,
            })
            .await
            .expect_err("mixed sharing semantics must require explicit account selection");

        assert_eq!(error, CredentialStageError::AuthRequired);
    }
}
