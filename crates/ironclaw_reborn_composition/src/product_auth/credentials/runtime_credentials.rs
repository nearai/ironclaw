use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount,
    CredentialAccountRecordSource, CredentialAccountSelectionRequest, CredentialAccountStatus,
    CredentialRefreshReport, CredentialRefreshRequest, ProviderScope,
    select_latest_duplicate_user_reusable_account,
};
use ironclaw_host_api::{
    CredentialStageError, ExtensionId, ResourceScope, RuntimeCredentialAccountSetup,
    RuntimeCredentialAuthRequirement, VendorId,
};
use ironclaw_host_runtime::{
    RuntimeCredentialAccessSecret, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver,
};

/// Minimum time remaining before an access token is considered fresh enough
/// to skip an inline refresh round-trip. Fixed at 5 minutes — not operator
/// configurable.
pub(crate) const DEFAULT_ACCESS_REFRESH_MARGIN: std::time::Duration =
    std::time::Duration::from_secs(5 * 60);

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

    /// Select the owner's existing configured account for an OAuth *bind*
    /// decision — i.e. "does this owner already have an account for this
    /// provider that this requester may update?". Unlike runtime resolution,
    /// this deliberately does NOT apply the provider-scope gate: a reconnect
    /// that adds a new scope must still find (and bind to) the existing
    /// account that lacks that scope, instead of forking a duplicate. Callers
    /// are responsible for passing an owner-granularity scope (thread/mission
    /// stripped).
    ///
    /// Required (no default): an unwired binding path must fail at the type
    /// level, not silently no-op. Test doubles that do not exercise binding
    /// return `CredentialMissing` explicitly.
    #[allow(dead_code)]
    async fn select_configured_account_for_binding(
        &self,
        lookup: CredentialAccountSelectionRequest,
        runtime_scope: AuthProductScope,
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
    let (_, missing) =
        runtime_credential_accounts_and_missing_requirements(accounts, scope, requirements).await?;
    Ok(missing)
}

/// Resolve both missing requirements and the existing redacted account
/// snapshots selected to authorize an operation. Callers that perform
/// external work after dropping their own lock compare the snapshots again
/// before publishing; account replacement, refresh, or revocation therefore
/// fences stale work without inventing a composition-owned authority type.
pub(crate) async fn runtime_credential_accounts_and_missing_requirements(
    accounts: &dyn RuntimeCredentialAccountSelectionService,
    scope: &ResourceScope,
    requirements: Vec<RuntimeCredentialAuthRequirement>,
) -> Result<
    (
        Vec<CredentialAccount>,
        Vec<RuntimeCredentialAuthRequirement>,
    ),
    CredentialStageError,
> {
    let mut missing = Vec::new();
    let mut selected_accounts = Vec::new();
    for requirement in requirements {
        match configured_runtime_credential_account(accounts, scope, &requirement).await? {
            Some(account) => selected_accounts.push(account),
            None => missing.push(requirement),
        }
    }
    Ok((selected_accounts, missing))
}

async fn configured_runtime_credential_account(
    accounts: &dyn RuntimeCredentialAccountSelectionService,
    scope: &ResourceScope,
    requirement: &RuntimeCredentialAuthRequirement,
) -> Result<Option<CredentialAccount>, CredentialStageError> {
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
        Ok(account) if account.access_secret.is_some() => Ok(Some(account)),
        Ok(_) => Err(CredentialStageError::Backend),
        Err(error) => match map_account_error(error) {
            CredentialStageError::AuthRequired => Ok(None),
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

/// Decides whether a resolved credential account is visible to a given
/// requester extension. Injected on [`crate::RebornProductAuthServices`] so the
/// assembling binary can supply an extension-family-aware policy (e.g. the
/// GSuite Google-account policy) without composition naming a concrete
/// extension crate; absent an injection, the strictly-more-restrictive
/// [`DefaultRuntimeCredentialAccountVisibilityPolicy`] applies.
pub trait RuntimeCredentialAccountVisibilityPolicy: Send + Sync {
    fn account_visible_to_requester(
        &self,
        account: &CredentialAccount,
        lookup: &CredentialAccountSelectionRequest,
    ) -> bool;
}

/// The safe default: an account is visible only when it is authorized for the
/// requester extension. Strictly more restrictive than any extension-family
/// policy an injected [`RuntimeCredentialAccountVisibilityPolicy`] may widen to,
/// so a build that injects no policy fails closed.
pub(crate) struct DefaultRuntimeCredentialAccountVisibilityPolicy;

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
    secret_store: Arc<dyn ironclaw_secrets::SecretStore>,
}

impl ProductAuthRuntimeCredentialAccountRefresher {
    pub(crate) fn new(
        refresh_accounts: Arc<dyn RuntimeCredentialAccountRefreshPort>,
        secret_store: Arc<dyn ironclaw_secrets::SecretStore>,
    ) -> Self {
        Self {
            refresh_accounts,
            secret_store,
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

/// Why the owner pre-filter is running, which decides whether the provider
/// scope gate applies.
///
/// The two modes are materially different and must not collapse into a nullable
/// flag: runtime resolution requires the account to already carry the requested
/// provider scopes; binding deliberately skips that gate so an OAuth reconnect
/// that adds a new scope still finds (and updates) the existing account instead
/// of forking a duplicate.
enum AccountSelectionPurpose<'a> {
    /// Runtime resolution — the account must already hold `provider_scopes`.
    Runtime {
        setup: &'a RuntimeCredentialAccountSetup,
        provider_scopes: &'a [ProviderScope],
    },
    /// OAuth bind — match the owner's existing account regardless of scopes,
    /// but only within the flow's own `session_id` (see the filter below).
    #[allow(dead_code)]
    Binding,
}

impl ProductAuthRuntimeCredentialAccountSelector {
    /// Owner-scoped pre-filter shared by runtime resolution and OAuth binding.
    ///
    /// `purpose` selects whether the provider-scope gate applies (see
    /// [`AccountSelectionPurpose`]). Requester authorization is NOT applied
    /// here — that is the caller's `finalize_selection` stage.
    async fn configured_accounts_for_requester(
        &self,
        lookup: &CredentialAccountSelectionRequest,
        runtime_scope: &AuthProductScope,
        purpose: AccountSelectionPurpose<'_>,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        Ok(self
            .accounts
            .accounts_for_owner(&lookup.scope)
            .await?
            .into_iter()
            .filter(|account| {
                account.provider == lookup.provider
                    && account.status == CredentialAccountStatus::Configured
                    && match &purpose {
                        AccountSelectionPurpose::Runtime {
                            setup,
                            provider_scopes,
                        } => account_has_provider_scopes(account, setup, provider_scopes),
                        // Bind/update is segmented on disk by surface AND
                        // session, and the callback updates the account at the
                        // flow scope's surface/session path. `accounts_for_owner`
                        // enumerates every surface (and wildcards session when
                        // the owner session is `None`), so require exact surface
                        // and session equality here or a reconnect could select —
                        // and then fail to read/update — an account stored on a
                        // different surface or session.
                        AccountSelectionPurpose::Binding => {
                            account.scope.session_id.as_ref() == lookup.scope.session_id.as_ref()
                                && account.scope.surface == lookup.scope.surface
                        }
                    }
                    && account_visible_from_runtime_scope(account, runtime_scope)
            })
            .collect())
    }

    /// Apply the requester-authorization stage and resolve to a single account.
    /// `configured` is the owner pre-filtered set from
    /// `configured_accounts_for_requester`.
    fn finalize_selection(
        &self,
        configured: Vec<CredentialAccount>,
        lookup: &CredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError> {
        if configured.is_empty() {
            return Err(AuthProductError::CredentialMissing);
        }
        let selectable = configured
            .into_iter()
            .filter(|account| {
                self.visibility_policy
                    .account_visible_to_requester(account, lookup)
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
impl RuntimeCredentialAccountSelectionService for ProductAuthRuntimeCredentialAccountSelector {
    async fn select_unique_configured_runtime_account(
        &self,
        request: RuntimeCredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError> {
        let configured = self
            .configured_accounts_for_requester(
                &request.lookup,
                &request.runtime_scope,
                AccountSelectionPurpose::Runtime {
                    setup: &request.setup,
                    provider_scopes: &request.provider_scopes,
                },
            )
            .await?;
        self.finalize_selection(configured, &request.lookup)
    }

    async fn select_configured_account_for_binding(
        &self,
        lookup: CredentialAccountSelectionRequest,
        runtime_scope: AuthProductScope,
    ) -> Result<CredentialAccount, AuthProductError> {
        let configured = self
            .configured_accounts_for_requester(
                &lookup,
                &runtime_scope,
                AccountSelectionPurpose::Binding,
            )
            .await?;
        self.finalize_selection(configured, &lookup)
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

        // A2: If the access secret has a known expiry that is still outside
        // the refresh margin, skip the token-endpoint round-trip and reuse the
        // staged token. We always re-read from the store (never cache).
        // Skip only when `expires_at` is present — absent means legacy record
        // or cleanup deleted it, both are fail-safe: proceed with refresh.
        let mut access_secret_requires_refresh = false;
        if let Some(access_handle) = &account.access_secret {
            let metadata = match self
                .secret_store
                .metadata(&account.scope.resource, access_handle)
                .await
            {
                Ok(Some(metadata)) => Some(metadata),
                Ok(None) => {
                    access_secret_requires_refresh = true;
                    None
                }
                Err(_) => {
                    access_secret_requires_refresh = true;
                    None
                }
            };
            if let Some(meta) = metadata
                && let Some(expires_at) = meta.expires_at
            {
                let margin = chrono::Duration::from_std(DEFAULT_ACCESS_REFRESH_MARGIN)
                    .unwrap_or(chrono::Duration::seconds(300));
                access_secret_requires_refresh = expires_at
                    .checked_sub_signed(margin)
                    .is_none_or(|cutoff| cutoff <= Utc::now());
                if !access_secret_requires_refresh {
                    tracing::debug!(
                        provider = %account.provider,
                        "oauth access token still fresh, skipping inline refresh"
                    );
                    return Ok(account);
                }
            }
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
                accounts
                    .select_unique_configured_runtime_account(request)
                    .await
            }
            Err(
                error @ (AuthProductError::BackendUnavailable
                | AuthProductError::BackendConflict
                | AuthProductError::MalformedConfig),
            ) => {
                tracing::debug!(
                    provider = %account.provider,
                    account_status = ?account.status,
                    has_access_secret = account.access_secret.is_some(),
                    has_refresh_secret = account.refresh_secret.is_some(),
                    auth_error = ?error,
                    "runtime product-auth refresh fell back to existing access secret"
                );
                if access_secret_requires_refresh {
                    tracing::warn!(
                        provider = %account.provider,
                        account_status = ?account.status,
                        auth_error = ?error,
                        "runtime product-auth refresh failed with a known stale access secret"
                    );
                    return Err(error);
                }
                Ok(account)
            }
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
            .map_err(|error| {
                tracing::debug!(
                    provider = %request.provider,
                    requester_extension = %request.requester_extension,
                    auth_error = ?error,
                    "runtime product-auth account selection failed"
                );
                map_account_error(error)
            })?;
        tracing::debug!(
            provider = %request.provider,
            requester_extension = %request.requester_extension,
            has_access_secret = account.access_secret.is_some(),
            has_refresh_secret = account.refresh_secret.is_some(),
            status = ?account.status,
            "runtime product-auth account selected"
        );
        let account = self
            .refresher
            .refresh_configured_runtime_account(selection_request, account, self.accounts.as_ref())
            .await
            .map_err(|error| {
                tracing::debug!(
                    provider = %request.provider,
                    requester_extension = %request.requester_extension,
                    auth_error = ?error,
                    "runtime product-auth account refresh failed"
                );
                map_account_error(error)
            })?;
        tracing::debug!(
            provider = %request.provider,
            requester_extension = %request.requester_extension,
            has_access_secret = account.access_secret.is_some(),
            has_refresh_secret = account.refresh_secret.is_some(),
            status = ?account.status,
            "runtime product-auth account refresh resolved"
        );
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
    provider: &VendorId,
    setup: RuntimeCredentialAccountSetup,
    provider_scopes: &[String],
    requester_extension: &ExtensionId,
) -> Result<RuntimeCredentialAccountSelectionRequest, CredentialStageError> {
    let owner_scope = AuthProductScope::credential_owner(scope, AuthSurface::Api);
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
        RuntimeCredentialAccountSetup::ManualToken
        | RuntimeCredentialAccountSetup::Pairing
        | RuntimeCredentialAccountSetup::Retired => false,
    }
}

fn account_visible_from_runtime_scope(
    account: &CredentialAccount,
    runtime_scope: &AuthProductScope,
) -> bool {
    // Runtime credential accounts are owned at tenant/user/agent/project
    // granularity. `mission_id`/`thread_id`/`session_id` are transient runtime
    // sub-scopes and MUST NOT narrow visibility: a credential authorized in one
    // thread is resolvable from every thread of the same owner. Which requester
    // may USE a non-reusable account is governed separately by ownership/grant
    // policy (`VisibilityPolicy::account_visible_to_requester` +
    // `CredentialAccount::is_authorized_for_requester`), not by the thread it
    // was authorized in. Re-binding to the thread here is what made Google (and
    // every other non-`UserReusable`) credential vanish on a new chat thread.
    let account_resource = &account.scope.resource;
    let runtime_resource = &runtime_scope.resource;
    account_resource.tenant_id == runtime_resource.tenant_id
        && account_resource.user_id == runtime_resource.user_id
        && account_resource.agent_id == runtime_resource.agent_id
        && account_resource.project_id == runtime_resource.project_id
}

fn map_account_error(error: AuthProductError) -> CredentialStageError {
    match error {
        AuthProductError::CredentialMissing
        | AuthProductError::CrossScopeDenied
        | AuthProductError::AccountSelectionRequired => CredentialStageError::AuthRequired,
        _ => CredentialStageError::Backend,
    }
}

pub(crate) mod host_managed_fallback;

#[cfg(test)]
mod tests;
