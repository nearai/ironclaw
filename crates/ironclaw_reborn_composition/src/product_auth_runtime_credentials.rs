use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount,
    CredentialAccountId, CredentialAccountRecordSource, CredentialAccountSelectionRequest,
    CredentialAccountStatus, CredentialOwnership, CredentialRefreshReport,
    CredentialRefreshRequest, ProviderScope, select_latest_duplicate_user_reusable_account,
};
use ironclaw_host_api::{
    CredentialStageError, ExtensionId, ResourceScope, RuntimeCredentialAccountProviderId,
    RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement,
};
use ironclaw_host_runtime::{
    RuntimeCredentialAccessSecret, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver,
};

#[derive(Clone)]
pub(crate) struct ProductAuthRuntimeCredentialResolver {
    accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    refresher: Arc<dyn RuntimeCredentialAccountRefreshService>,
}

impl ProductAuthRuntimeCredentialResolver {
    #[cfg(test)]
    pub(crate) fn new(accounts: Arc<dyn RuntimeCredentialAccountSelectionService>) -> Self {
        Self {
            accounts,
            refresher: Arc::new(NoopRuntimeCredentialAccountRefresher),
        }
    }

    pub(crate) fn new_with_refresh(
        accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
        refresher: Arc<dyn RuntimeCredentialAccountRefreshService>,
    ) -> Self {
        Self {
            accounts,
            refresher,
        }
    }
}

#[async_trait]
pub(crate) trait RuntimeCredentialAccountSelectionService: Send + Sync {
    async fn select_unique_configured_runtime_account(
        &self,
        request: RuntimeCredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError>;
}

#[async_trait]
pub(crate) trait RuntimeCredentialAccountRefreshService: Send + Sync {
    async fn refresh_configured_runtime_account(
        &self,
        request: RuntimeCredentialAccountSelectionRequest,
        account: CredentialAccount,
        accounts: &dyn RuntimeCredentialAccountSelectionService,
    ) -> Result<CredentialAccount, AuthProductError>;
}

#[cfg(test)]
struct NoopRuntimeCredentialAccountRefresher;

#[cfg(test)]
#[async_trait]
impl RuntimeCredentialAccountRefreshService for NoopRuntimeCredentialAccountRefresher {
    async fn refresh_configured_runtime_account(
        &self,
        _request: RuntimeCredentialAccountSelectionRequest,
        account: CredentialAccount,
        _accounts: &dyn RuntimeCredentialAccountSelectionService,
    ) -> Result<CredentialAccount, AuthProductError> {
        Ok(account)
    }
}

#[derive(Clone)]
pub(crate) struct RuntimeCredentialAccountSelectionRequest {
    lookup: CredentialAccountSelectionRequest,
    runtime_scope: AuthProductScope,
    setup: RuntimeCredentialAccountSetup,
    provider_scopes: Vec<ProviderScope>,
}

#[async_trait]
pub(crate) trait RuntimeCredentialAccountRefreshPort: Send + Sync {
    async fn refresh_credential_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError>;
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

pub(crate) async fn missing_runtime_credential_auth_requirements(
    accounts: &dyn RuntimeCredentialAccountSelectionService,
    scope: &ResourceScope,
    requirements: Vec<RuntimeCredentialAuthRequirement>,
) -> Result<Vec<RuntimeCredentialAuthRequirement>, CredentialStageError> {
    let mut missing = Vec::new();
    for requirement in requirements {
        if runtime_credential_auth_requirement_configured(accounts, scope, &requirement).await? {
            continue;
        }
        missing.push(requirement);
    }
    Ok(missing)
}

async fn runtime_credential_auth_requirement_configured(
    accounts: &dyn RuntimeCredentialAccountSelectionService,
    scope: &ResourceScope,
    requirement: &RuntimeCredentialAuthRequirement,
) -> Result<bool, CredentialStageError> {
    let request = runtime_credential_account_selection_request(
        scope,
        &requirement.provider,
        requirement.setup.clone(),
        &requirement.provider_scopes,
        &requirement.requester_extension,
    )?;
    match accounts
        .select_unique_configured_runtime_account(request)
        .await
    {
        Ok(account) if account.access_secret.is_some() => Ok(true),
        Ok(_) => Err(CredentialStageError::Backend),
        Err(error) => match map_account_error(error) {
            CredentialStageError::AuthRequired => Ok(false),
            CredentialStageError::Backend => Err(CredentialStageError::Backend),
        },
    }
}

pub(crate) struct ProductAuthRuntimeCredentialAccountSelector {
    accounts: Arc<dyn CredentialAccountRecordSource>,
    visibility_policy: Arc<dyn RuntimeCredentialAccountVisibilityPolicy>,
}

impl ProductAuthRuntimeCredentialAccountSelector {
    #[cfg(test)]
    pub(crate) fn new(accounts: Arc<dyn CredentialAccountRecordSource>) -> Self {
        Self {
            accounts,
            visibility_policy: Arc::new(DefaultRuntimeCredentialAccountVisibilityPolicy),
        }
    }

    pub(crate) fn new_with_visibility(
        accounts: Arc<dyn CredentialAccountRecordSource>,
        visibility_policy: Arc<dyn RuntimeCredentialAccountVisibilityPolicy>,
    ) -> Self {
        Self {
            accounts,
            visibility_policy,
        }
    }
}

pub(crate) trait RuntimeCredentialAccountVisibilityPolicy: Send + Sync {
    fn account_visible_to_requester(
        &self,
        account: &CredentialAccount,
        lookup: &CredentialAccountSelectionRequest,
    ) -> bool;
}

#[cfg(test)]
struct DefaultRuntimeCredentialAccountVisibilityPolicy;

#[cfg(test)]
impl RuntimeCredentialAccountVisibilityPolicy for DefaultRuntimeCredentialAccountVisibilityPolicy {
    fn account_visible_to_requester(
        &self,
        account: &CredentialAccount,
        lookup: &CredentialAccountSelectionRequest,
    ) -> bool {
        account.is_authorized_for_requester(lookup.requester_extension.as_ref())
    }
}

pub(crate) struct ProductAuthRuntimeCredentialAccountRefresher {
    refresh_accounts: Arc<dyn RuntimeCredentialAccountRefreshPort>,
    refreshed_account_ids: tokio::sync::Mutex<HashSet<CredentialAccountId>>,
}

impl ProductAuthRuntimeCredentialAccountRefresher {
    pub(crate) fn new(refresh_accounts: Arc<dyn RuntimeCredentialAccountRefreshPort>) -> Self {
        Self {
            refresh_accounts,
            refreshed_account_ids: tokio::sync::Mutex::new(HashSet::new()),
        }
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
                self.visibility_policy
                    .account_visible_to_requester(account, &request.lookup)
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

#[async_trait]
impl RuntimeCredentialAccountRefreshService for ProductAuthRuntimeCredentialAccountRefresher {
    async fn refresh_configured_runtime_account(
        &self,
        request: RuntimeCredentialAccountSelectionRequest,
        account: CredentialAccount,
        accounts: &dyn RuntimeCredentialAccountSelectionService,
    ) -> Result<CredentialAccount, AuthProductError> {
        if !matches!(request.setup, RuntimeCredentialAccountSetup::OAuth { .. }) {
            return Ok(account);
        }
        if account.refresh_secret.is_none() {
            return Ok(account);
        }
        let account_id = account.id;
        let mut refreshed_account_ids = self.refreshed_account_ids.lock().await;
        if refreshed_account_ids.contains(&account_id) {
            return accounts
                .select_unique_configured_runtime_account(request)
                .await;
        }

        let mut refresh_request = CredentialRefreshRequest::new(
            account.scope.clone(),
            account.provider.clone(),
            account_id,
        );
        if let Some(requester_extension) =
            refresh_requester_for_account(&account, request.lookup.requester_extension.as_ref())
        {
            refresh_request = refresh_request.for_extension(requester_extension);
        }
        match self
            .refresh_accounts
            .refresh_credential_account(refresh_request)
            .await
        {
            Ok(_) => {
                refreshed_account_ids.insert(account_id);
                accounts
                    .select_unique_configured_runtime_account(request)
                    .await
            }
            Err(
                AuthProductError::BackendUnavailable
                | AuthProductError::BackendConflict
                | AuthProductError::MalformedConfig,
            ) => Ok(account),
            Err(error) => Err(error),
        }
    }
}

fn refresh_requester_for_account(
    account: &CredentialAccount,
    requester_extension: Option<&ExtensionId>,
) -> Option<ExtensionId> {
    if let Some(requester_extension) = requester_extension
        && account.is_authorized_for_requester(Some(requester_extension))
    {
        return Some(requester_extension.clone());
    }
    account
        .owner_extension
        .clone()
        .filter(|owner_extension| account.is_authorized_for_requester(Some(owner_extension)))
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
        let selection_request = runtime_credential_account_selection_request(
            request.scope,
            request.provider,
            request.setup.clone(),
            request.provider_scopes,
            request.requester_extension,
        )?;
        let account = self
            .accounts
            .select_unique_configured_runtime_account(selection_request.clone())
            .await
            .map_err(map_account_error)?;
        let account = self
            .refresher
            .refresh_configured_runtime_account(selection_request, account, self.accounts.as_ref())
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

fn runtime_credential_account_selection_request(
    scope: &ResourceScope,
    provider: &RuntimeCredentialAccountProviderId,
    setup: RuntimeCredentialAccountSetup,
    provider_scopes: &[String],
    requester_extension: &ExtensionId,
) -> Result<RuntimeCredentialAccountSelectionRequest, CredentialStageError> {
    let owner_scope = AuthProductScope::new(runtime_account_owner_scope(scope), AuthSurface::Api);
    let provider = AuthProviderId::new(provider.as_str()).map_err(|e| {
        tracing::debug!(
            provider = %provider.as_str(),
            err = %e,
            "product-auth provider id is invalid"
        );
        CredentialStageError::Backend
    })?;
    let provider_scopes = provider_scopes
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
    Ok(RuntimeCredentialAccountSelectionRequest::new(
        CredentialAccountSelectionRequest::new(owner_scope, provider)
            .for_extension(requester_extension.clone()),
        AuthProductScope::new(scope.clone(), AuthSurface::Api),
        setup,
        provider_scopes,
    ))
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

pub(crate) fn runtime_account_owner_scope(
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
mod tests;
