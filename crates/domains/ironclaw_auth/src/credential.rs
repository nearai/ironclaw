use std::{collections::HashMap, fmt, sync::Arc, sync::Mutex};

use async_trait::async_trait;
use ironclaw_host_api::ids::{ExtensionId, ProjectId, SecretHandle, TenantId, UserId};
use serde::{Deserialize, Serialize};
use tokio::sync::OwnedMutexGuard;

use crate::{
    AuthProductError, AuthProviderClient, CredentialAccountId, CredentialAccountLabel,
    OAuthProviderIdentity, OAuthProviderRefreshRequest, ProviderScope, Timestamp,
    ids::AuthProviderId, scope::AuthProductScope, scope_matches,
};

/// Credential account status projected to product surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialAccountStatus {
    Configured,
    Inactive,
    Missing,
    Expired,
    RefreshFailed,
    Revoked,
    PendingSetup,
}

/// Ownership class determines uninstall/deactivate cleanup behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialOwnership {
    ExtensionOwned,
    UserReusable,
    SharedAdminManaged,
    System,
}

/// Durable credential account metadata. Secret values live behind handles and
/// never appear in this record.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAccount {
    pub id: CredentialAccountId,
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub status: CredentialAccountStatus,
    pub ownership: CredentialOwnership,
    pub owner_extension: Option<ExtensionId>,
    pub granted_extensions: Vec<ExtensionId>,
    pub access_secret: Option<SecretHandle>,
    pub refresh_secret: Option<SecretHandle>,
    pub scopes: Vec<ProviderScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_identity: Option<OAuthProviderIdentity>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl fmt::Debug for CredentialAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialAccount")
            .field("id", &self.id)
            .field("scope", &self.scope)
            .field("provider", &self.provider)
            .field("label", &self.label)
            .field("status", &self.status)
            .field("ownership", &self.ownership)
            .field("owner_extension", &self.owner_extension)
            .field("granted_extensions", &self.granted_extensions)
            .field(
                "access_secret",
                &self.access_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "refresh_secret",
                &self.refresh_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("scopes", &self.scopes)
            .field("provider_identity", &self.provider_identity)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

impl CredentialAccount {
    pub fn projection(&self) -> CredentialAccountProjection {
        let secret_handle_count =
            self.access_secret.iter().count() + self.refresh_secret.iter().count();
        CredentialAccountProjection {
            id: self.id,
            provider: self.provider.clone(),
            label: self.label.clone(),
            status: self.status,
            ownership: self.ownership,
            owner_extension: self.owner_extension.clone(),
            granted_extensions: self.granted_extensions.clone(),
            secret_handle_count,
        }
    }

    pub fn is_authorized_for_requester(&self, requester_extension: Option<&ExtensionId>) -> bool {
        match self.ownership {
            CredentialOwnership::UserReusable => true,
            CredentialOwnership::ExtensionOwned => self
                .owner_extension
                .as_ref()
                .is_some_and(|owner_extension| requester_extension == Some(owner_extension)),
            CredentialOwnership::SharedAdminManaged => requester_extension
                .is_some_and(|requester| self.granted_extensions.contains(requester)),
            CredentialOwnership::System => false,
        }
    }
}

/// Adapter-safe account projection. It does not include raw secret material or
/// backend secret handle names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAccountProjection {
    pub id: CredentialAccountId,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub status: CredentialAccountStatus,
    pub ownership: CredentialOwnership,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_extension: Option<ExtensionId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub granted_extensions: Vec<ExtensionId>,
    pub secret_handle_count: usize,
}

/// Product-facing credential recovery state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialRecoveryKind {
    Configured,
    SetupRequired,
    ReauthorizeRequired,
    AccountSelectionRequired,
}

/// Stable reason a product surface can render without inspecting backend
/// errors or secret handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialRecoveryReason {
    Configured,
    NoAccount,
    AccountMissing,
    PendingSetup,
    AccountExpired,
    RefreshFailed,
    AccountRevoked,
    AccountInactive,
    AmbiguousAccount,
}

/// Adapter-safe credential recovery projection. Account entries are filtered
/// to choices authorized for the requester and never include backend secret
/// handle names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRecoveryProjection {
    pub provider: AuthProviderId,
    pub reason: CredentialRecoveryReason,
    #[serde(flatten)]
    pub state: CredentialRecoveryState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialRecoveryState {
    Configured {
        selected_account: CredentialAccountProjection,
    },
    SetupRequired {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choices: Vec<CredentialAccountProjection>,
    },
    ReauthorizeRequired {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choices: Vec<CredentialAccountProjection>,
    },
    AccountSelectionRequired {
        choices: Vec<CredentialAccountProjection>,
    },
}

impl CredentialRecoveryProjection {
    pub fn configured(
        provider: AuthProviderId,
        selected_account: CredentialAccountProjection,
    ) -> Self {
        Self {
            provider,
            reason: CredentialRecoveryReason::Configured,
            state: CredentialRecoveryState::Configured { selected_account },
        }
    }

    pub fn setup_required(
        provider: AuthProviderId,
        reason: CredentialRecoveryReason,
        choices: Vec<CredentialAccountProjection>,
    ) -> Self {
        Self {
            provider,
            reason,
            state: CredentialRecoveryState::SetupRequired { choices },
        }
    }

    pub fn reauthorize_required(
        provider: AuthProviderId,
        reason: CredentialRecoveryReason,
        choices: Vec<CredentialAccountProjection>,
    ) -> Self {
        Self {
            provider,
            reason,
            state: CredentialRecoveryState::ReauthorizeRequired { choices },
        }
    }

    pub fn account_selection_required(
        provider: AuthProviderId,
        choices: Vec<CredentialAccountProjection>,
    ) -> Self {
        Self {
            provider,
            reason: CredentialRecoveryReason::AmbiguousAccount,
            state: CredentialRecoveryState::AccountSelectionRequired { choices },
        }
    }

    pub fn kind(&self) -> CredentialRecoveryKind {
        match &self.state {
            CredentialRecoveryState::Configured { .. } => CredentialRecoveryKind::Configured,
            CredentialRecoveryState::SetupRequired { .. } => CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryState::ReauthorizeRequired { .. } => {
                CredentialRecoveryKind::ReauthorizeRequired
            }
            CredentialRecoveryState::AccountSelectionRequired { .. } => {
                CredentialRecoveryKind::AccountSelectionRequired
            }
        }
    }

    pub fn selected_account(&self) -> Option<&CredentialAccountProjection> {
        match &self.state {
            CredentialRecoveryState::Configured { selected_account } => Some(selected_account),
            CredentialRecoveryState::SetupRequired { .. }
            | CredentialRecoveryState::ReauthorizeRequired { .. }
            | CredentialRecoveryState::AccountSelectionRequired { .. } => None,
        }
    }

    pub fn choices(&self) -> &[CredentialAccountProjection] {
        match &self.state {
            CredentialRecoveryState::Configured { .. } => &[],
            CredentialRecoveryState::SetupRequired { choices }
            | CredentialRecoveryState::ReauthorizeRequired { choices }
            | CredentialRecoveryState::AccountSelectionRequired { choices } => choices,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountListRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub requester_extension: Option<ExtensionId>,
    pub cursor: Option<CredentialAccountId>,
    pub limit: usize,
}

impl CredentialAccountListRequest {
    pub const DEFAULT_LIMIT: usize = 50;
    pub const MAX_LIMIT: usize = 100;

    pub fn new(scope: AuthProductScope, provider: AuthProviderId) -> Self {
        Self {
            scope,
            provider,
            requester_extension: None,
            cursor: None,
            limit: Self::DEFAULT_LIMIT,
        }
    }

    pub fn for_extension(mut self, extension_id: ExtensionId) -> Self {
        self.requester_extension = Some(extension_id);
        self
    }

    pub fn with_cursor(mut self, cursor: CredentialAccountId) -> Self {
        self.cursor = Some(cursor);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    pub fn validate(&self) -> Result<(), AuthProductError> {
        if self.limit == 0 {
            return Err(AuthProductError::invalid_request(
                "credential account list limit must be non-zero",
            ));
        }
        if self.limit > Self::MAX_LIMIT {
            return Err(AuthProductError::invalid_request(format!(
                "credential account list limit must be at most {}",
                Self::MAX_LIMIT
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountLookupRequest {
    pub scope: AuthProductScope,
    pub account_id: CredentialAccountId,
    pub requester_extension: Option<ExtensionId>,
}

impl CredentialAccountLookupRequest {
    pub fn new(scope: AuthProductScope, account_id: CredentialAccountId) -> Self {
        Self {
            scope,
            account_id,
            requester_extension: None,
        }
    }

    pub fn for_extension(mut self, extension_id: ExtensionId) -> Self {
        self.requester_extension = Some(extension_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAccountListPage {
    pub accounts: Vec<CredentialAccountProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<CredentialAccountId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRecoveryRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub requester_extension: Option<ExtensionId>,
}

impl CredentialRecoveryRequest {
    pub fn new(scope: AuthProductScope, provider: AuthProviderId) -> Self {
        Self {
            scope,
            provider,
            requester_extension: None,
        }
    }

    pub fn for_extension(mut self, extension_id: ExtensionId) -> Self {
        self.requester_extension = Some(extension_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountSelectionRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub requester_extension: Option<ExtensionId>,
}

impl CredentialAccountSelectionRequest {
    pub fn new(scope: AuthProductScope, provider: AuthProviderId) -> Self {
        Self {
            scope,
            provider,
            requester_extension: None,
        }
    }

    pub fn for_extension(mut self, extension_id: ExtensionId) -> Self {
        self.requester_extension = Some(extension_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountChoiceRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub account_id: CredentialAccountId,
    pub requester_extension: Option<ExtensionId>,
}

impl CredentialAccountChoiceRequest {
    pub fn new(
        scope: AuthProductScope,
        provider: AuthProviderId,
        account_id: CredentialAccountId,
    ) -> Self {
        Self {
            scope,
            provider,
            account_id,
            requester_extension: None,
        }
    }

    pub fn for_extension(mut self, extension_id: ExtensionId) -> Self {
        self.requester_extension = Some(extension_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRefreshRequest {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub account_id: CredentialAccountId,
    pub requester_extension: Option<ExtensionId>,
}

impl CredentialRefreshRequest {
    pub fn new(
        scope: AuthProductScope,
        provider: AuthProviderId,
        account_id: CredentialAccountId,
    ) -> Self {
        Self {
            scope,
            provider,
            account_id,
            requester_extension: None,
        }
    }

    pub fn for_extension(mut self, extension_id: ExtensionId) -> Self {
        self.requester_extension = Some(extension_id);
        self
    }
}

/// Adapter-safe refresh result. It carries the updated redacted account plus
/// the stable recovery state after refresh. It never carries backend error
/// strings, secret handles, or provider response bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRefreshReport {
    pub account: CredentialAccountProjection,
    pub recovery: CredentialRecoveryProjection,
    pub refreshed: bool,
}

/// Input used to create or update an account from an OAuth/manual setup result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCredentialAccount {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub label: CredentialAccountLabel,
    pub status: CredentialAccountStatus,
    pub ownership: CredentialOwnership,
    pub owner_extension: Option<ExtensionId>,
    pub granted_extensions: Vec<ExtensionId>,
    pub access_secret: Option<SecretHandle>,
    pub refresh_secret: Option<SecretHandle>,
    pub scopes: Vec<ProviderScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountUpdate {
    pub account_id: CredentialAccountId,
    pub account: NewCredentialAccount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialAccountMutation {
    Create(NewCredentialAccount),
    Update(CredentialAccountUpdate),
}

#[async_trait]
pub trait CredentialAccountService: Send + Sync {
    async fn create_account(
        &self,
        request: NewCredentialAccount,
    ) -> Result<CredentialAccount, AuthProductError>;

    async fn get_account(
        &self,
        request: CredentialAccountLookupRequest,
    ) -> Result<Option<CredentialAccount>, AuthProductError>;

    async fn list_accounts(
        &self,
        request: CredentialAccountListRequest,
    ) -> Result<CredentialAccountListPage, AuthProductError>;

    async fn update_status(
        &self,
        scope: &AuthProductScope,
        account_id: CredentialAccountId,
        status: CredentialAccountStatus,
    ) -> Result<CredentialAccount, AuthProductError>;

    async fn select_unique_configured_account(
        &self,
        request: CredentialAccountSelectionRequest,
    ) -> Result<CredentialAccountProjection, AuthProductError>;

    async fn project_credential_recovery(
        &self,
        request: CredentialRecoveryRequest,
    ) -> Result<CredentialRecoveryProjection, AuthProductError>;

    async fn select_configured_account(
        &self,
        request: CredentialAccountChoiceRequest,
    ) -> Result<CredentialAccountProjection, AuthProductError>;

    async fn refresh_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError>;
}

/// Who owns a credential: a tenant + user, optionally narrowed to a project
/// beneath them.
///
/// These are exactly the fields the ownership rule compares — nothing here is
/// carried for a future reader. `agent_id`, `mission_id`, `thread_id` and
/// `session_id` were dropped once [`Self::matches`] stopped comparing them and
/// the migration's legacy walk stopped narrowing by session: they were
/// populated on every construction and read nowhere, while their doc still
/// advertised a wildcard rule that no longer existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountOwnerScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
}

impl CredentialAccountOwnerScope {
    pub fn from_scope(scope: &AuthProductScope) -> Self {
        Self {
            tenant_id: scope.resource.tenant_id.clone(),
            user_id: scope.resource.user_id.clone(),
            project_id: scope.resource.project_id.clone(),
        }
    }

    /// A credential belongs to a **tenant + user**, optionally narrowed to a
    /// project underneath that user. Nothing else owns it.
    ///
    /// Tenant and user are hard-required: they are the security boundary, and
    /// the only one here. `agent_id`, `thread_id`, `mission_id` and
    /// `session_id` are deliberately NOT compared — a user authorizes a vendor
    /// once, not once per agent, per browser session, or per screen. Comparing
    /// them is what made a credential minted by the chat auth gate invisible to
    /// the extensions registry and the runtime, which reported it as
    /// "needs setup" forever while the tokens sat valid on disk.
    ///
    /// Project **inherits downward, never upward**: a project-scoped caller
    /// sees its own project's credentials plus the user-level ones it inherits,
    /// so creating a project cannot silently disconnect every integration. A
    /// user-level caller sees only user-level credentials, so a project's
    /// sub-credential never leaks out of that project.
    ///
    /// The project comparison lives *here*, in the shared predicate, and not
    /// only in the durable store's root chain. The filesystem store narrows by
    /// reading just the relevant roots, but the in-memory store has no roots and
    /// filters solely through this predicate — so leaving project out made the
    /// fake strictly more permissive than production, letting a project-scoped
    /// caller see every other project's credentials. A fake that fails open is
    /// the one thing this crate's guardrails say it must never be. Comparing it
    /// here makes both implementations enforce one rule, and leaves the durable
    /// root chain as narrowing rather than as the only check.
    ///
    /// Comparing the record's project is sound because migration aligns
    /// provenance with placement: a record living in project P's root carries
    /// provenance project P, and a user-level record carries none.
    pub fn matches(&self, account: &CredentialAccount) -> bool {
        let resource = &account.scope.resource;
        resource.tenant_id == self.tenant_id
            && resource.user_id == self.user_id
            && (resource.project_id == self.project_id || resource.project_id.is_none())
    }
}

/// True iff `account` is owned by the owner of `scope`, ignoring transient
/// invocation provenance.
///
/// OAuth/manual-token reconnect binding and the subsequent bound-account update
/// must resolve at durable owner granularity: the flow `scope` carries a fresh
/// per-flow `invocation_id` (and possibly a thread/mission) that the account —
/// created in an earlier flow — does not share. Comparing those transient
/// fields (the old `scope_matches` full-equality) rejected every legitimate
/// reconnect and forked a duplicate account (#4935 defect A). Requester
/// authorization is enforced separately by the callers; this is only the
/// owner-boundary check.
///
/// This is exactly [`CredentialAccountOwnerScope::matches`] and deliberately
/// adds nothing to it. It previously also required exact `surface` and
/// `session_id` equality, justified by accounts being path-segmented on those
/// axes: binding to an account on another surface would select a record the
/// callback could not then write back to. Accounts now live at one address per
/// credential owner, so that hazard is gone and the extra equality only
/// rejected legitimate reconnects — a credential minted by the chat auth gate
/// (`Callback`) could never be re-bound from the extensions surface (`Web`).
pub fn binding_scope_owns_account(scope: &AuthProductScope, account: &CredentialAccount) -> bool {
    CredentialAccountOwnerScope::from_scope(&scope.to_credential_owner()).matches(account)
}

/// Read-only credential-account projection source for account owner queries.
///
/// This intentionally does not encode host-runtime credential selection. It
/// only exposes accounts owned by a stable product-auth owner; composition
/// layers decide which account, provider, or requester policy to apply.
#[async_trait]
pub trait CredentialAccountRecordSource: Send + Sync {
    async fn accounts_for_owner(
        &self,
        scope: &AuthProductScope,
    ) -> Result<Vec<CredentialAccount>, AuthProductError>;

    async fn select_unique_configured_account_for_owner(
        &self,
        request: CredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError> {
        let configured = self
            .accounts_for_owner(&request.scope)
            .await?
            .into_iter()
            .filter(|account| {
                account.provider == request.provider
                    && account.status == CredentialAccountStatus::Configured
            })
            .collect::<Vec<_>>();
        if configured.is_empty() {
            return Err(AuthProductError::CredentialMissing);
        }
        let selectable = configured
            .into_iter()
            .filter(|account| {
                account.is_authorized_for_requester(request.requester_extension.as_ref())
            })
            .collect::<Vec<_>>();
        match selectable.as_slice() {
            [] => Err(AuthProductError::CrossScopeDenied),
            [account] => Ok(account.clone()),
            _ => Err(AuthProductError::AccountSelectionRequired),
        }
    }
}

#[async_trait]
pub trait CredentialSetupService: Send + Sync {
    async fn create_or_update_account(
        &self,
        request: CredentialAccountMutation,
    ) -> Result<CredentialAccount, AuthProductError>;
}

/// Credential account service that refreshes through the provider client and
/// persists account mutations through the backing account/setup services.
pub struct ProviderBackedCredentialAccountService {
    accounts: Arc<dyn CredentialAccountService>,
    setup: Arc<dyn CredentialSetupService>,
    provider: Arc<dyn AuthProviderClient>,
    refresh_locks: Mutex<HashMap<CredentialAccountId, Arc<tokio::sync::Mutex<()>>>>,
}

impl ProviderBackedCredentialAccountService {
    pub fn new(
        accounts: Arc<dyn CredentialAccountService>,
        setup: Arc<dyn CredentialSetupService>,
        provider: Arc<dyn AuthProviderClient>,
    ) -> Self {
        Self {
            accounts,
            setup,
            provider,
            refresh_locks: Mutex::new(HashMap::new()),
        }
    }

    fn refresh_lock(&self, account_id: CredentialAccountId) -> Arc<tokio::sync::Mutex<()>> {
        let mut refresh_locks = self
            .refresh_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        refresh_locks
            .entry(account_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn acquire_refresh_lock(&self, account_id: CredentialAccountId) -> OwnedMutexGuard<()> {
        self.refresh_lock(account_id).lock_owned().await
    }

    fn release_refresh_lock(&self, account_id: CredentialAccountId) {
        let mut refresh_locks = self
            .refresh_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if refresh_locks
            .get(&account_id)
            .is_some_and(|lock| Arc::strong_count(lock) == 1)
        {
            refresh_locks.remove(&account_id);
        }
    }

    fn refresh_lookup_request(
        request: &CredentialRefreshRequest,
    ) -> CredentialAccountLookupRequest {
        let mut lookup =
            CredentialAccountLookupRequest::new(request.scope.clone(), request.account_id);
        if let Some(requester_extension) = request.requester_extension.clone() {
            lookup = lookup.for_extension(requester_extension);
        }
        lookup
    }

    fn validate_refresh_target(
        account: &CredentialAccount,
        request: &CredentialRefreshRequest,
    ) -> Result<(), AuthProductError> {
        if !scope_matches(&request.scope, &account.scope) || account.provider != request.provider {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if !account_is_authorized_for_requester(account, request.requester_extension.as_ref()) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        if matches!(
            account.status,
            CredentialAccountStatus::Missing
                | CredentialAccountStatus::PendingSetup
                | CredentialAccountStatus::Inactive
                | CredentialAccountStatus::Revoked
        ) {
            return Err(AuthProductError::CredentialMissing);
        }
        Ok(())
    }

    fn account_update(
        account: &CredentialAccount,
        access_secret: Option<SecretHandle>,
        refresh_secret: Option<SecretHandle>,
        status: CredentialAccountStatus,
        scopes: Vec<ProviderScope>,
    ) -> CredentialAccountMutation {
        CredentialAccountMutation::Update(CredentialAccountUpdate {
            account_id: account.id,
            account: NewCredentialAccount {
                scope: account.scope.clone(),
                provider: account.provider.clone(),
                label: account.label.clone(),
                status,
                ownership: account.ownership,
                owner_extension: account.owner_extension.clone(),
                granted_extensions: account.granted_extensions.clone(),
                access_secret,
                refresh_secret,
                scopes,
            },
        })
    }

    async fn report_for(
        &self,
        account: &CredentialAccount,
        requester_extension: Option<&ExtensionId>,
        refreshed: bool,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        let recovery_request =
            CredentialRecoveryRequest::new(account.scope.clone(), account.provider.clone());
        let recovery = self
            .accounts
            .project_credential_recovery(match requester_extension {
                Some(requester_extension) => {
                    recovery_request.for_extension(requester_extension.clone())
                }
                None => recovery_request,
            })
            // silent-ok: refresh reporting is allowed to degrade to the
            // single-account projection when the broader recovery projection
            // lookup fails; the refresh mutation has already been applied and
            // the caller still gets the refreshed account snapshot.
            .await
            .unwrap_or_else(|_| single_account_recovery(account));
        Ok(CredentialRefreshReport {
            account: account.projection(),
            recovery,
            refreshed,
        })
    }

    /// Apply a terminal refresh status (`Revoked` for `invalid_grant`,
    /// `RefreshFailed` for other non-transient failures) to the account and
    /// return the resulting report. The existing access/refresh handles and
    /// scopes are preserved; only the status changes. Re-reads the account
    /// first and bails to a plain report if another writer changed it under us.
    async fn report_terminal_refresh_status(
        &self,
        lookup_request: &CredentialAccountLookupRequest,
        account: &CredentialAccount,
        requester_extension: Option<&ExtensionId>,
        status: CredentialAccountStatus,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        let current = self
            .accounts
            .get_account(lookup_request.clone())
            .await?
            .ok_or(AuthProductError::CredentialMissing)?;
        if current != *account {
            return self.report_for(&current, requester_extension, false).await;
        }
        let updated = self
            .setup
            .create_or_update_account(Self::account_update(
                &current,
                current.access_secret.clone(),
                current.refresh_secret.clone(),
                status,
                current.scopes.clone(),
            ))
            .await?;
        self.report_for(&updated, requester_extension, false).await
    }
}

#[async_trait]
impl CredentialAccountService for ProviderBackedCredentialAccountService {
    async fn create_account(
        &self,
        request: NewCredentialAccount,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.accounts.create_account(request).await
    }

    async fn get_account(
        &self,
        request: CredentialAccountLookupRequest,
    ) -> Result<Option<CredentialAccount>, AuthProductError> {
        self.accounts.get_account(request).await
    }

    async fn list_accounts(
        &self,
        request: CredentialAccountListRequest,
    ) -> Result<CredentialAccountListPage, AuthProductError> {
        self.accounts.list_accounts(request).await
    }

    async fn update_status(
        &self,
        scope: &AuthProductScope,
        account_id: CredentialAccountId,
        status: CredentialAccountStatus,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.accounts.update_status(scope, account_id, status).await
    }

    async fn select_unique_configured_account(
        &self,
        request: CredentialAccountSelectionRequest,
    ) -> Result<CredentialAccountProjection, AuthProductError> {
        self.accounts
            .select_unique_configured_account(request)
            .await
    }

    async fn project_credential_recovery(
        &self,
        request: CredentialRecoveryRequest,
    ) -> Result<CredentialRecoveryProjection, AuthProductError> {
        self.accounts.project_credential_recovery(request).await
    }

    async fn select_configured_account(
        &self,
        request: CredentialAccountChoiceRequest,
    ) -> Result<CredentialAccountProjection, AuthProductError> {
        self.accounts.select_configured_account(request).await
    }

    async fn refresh_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        let lookup_request = Self::refresh_lookup_request(&request);
        let initial_account = self
            .accounts
            .get_account(lookup_request.clone())
            .await?
            .ok_or(AuthProductError::CredentialMissing)?;
        Self::validate_refresh_target(&initial_account, &request)?;
        let refresh_lock = self.acquire_refresh_lock(initial_account.id).await;
        let result = async {
            let account = self
                .accounts
                .get_account(lookup_request.clone())
                .await?
                .ok_or(AuthProductError::CredentialMissing)?;
            if account != initial_account {
                return self
                    .report_for(&account, request.requester_extension.as_ref(), false)
                    .await;
            }

            let Some(refresh_secret) = account.refresh_secret.clone() else {
                let updated = self
                    .setup
                    .create_or_update_account(Self::account_update(
                        &account,
                        account.access_secret.clone(),
                        account.refresh_secret.clone(),
                        CredentialAccountStatus::RefreshFailed,
                        account.scopes.clone(),
                    ))
                    .await?;
                return self
                    .report_for(&updated, request.requester_extension.as_ref(), false)
                    .await;
            };

            let provider_request = OAuthProviderRefreshRequest {
                provider: account.provider.clone(),
                scope: account.scope.clone(),
                account_id: account.id,
                refresh_secret: refresh_secret.clone(),
                scopes: account.scopes.clone(),
            };

            let requester_extension = match account.ownership {
                CredentialOwnership::ExtensionOwned => account.owner_extension.clone(),
                _ => None,
            };
            match self
                .provider
                .refresh_token_for_requester(requester_extension, provider_request)
                .await
            {
                Ok(refresh) => {
                    let current = self
                        .accounts
                        .get_account(lookup_request.clone())
                        .await?
                        .ok_or(AuthProductError::CredentialMissing)?;
                    if current != account {
                        return self
                            .report_for(&current, request.requester_extension.as_ref(), false)
                            .await;
                    }
                    if refresh.provider != current.provider {
                        return Err(AuthProductError::CrossScopeDenied);
                    }
                    let refresh_secret = refresh
                        .refresh_secret
                        .or_else(|| current.refresh_secret.clone());
                    let updated = self
                        .setup
                        .create_or_update_account(Self::account_update(
                            &current,
                            Some(refresh.access_secret),
                            refresh_secret,
                            CredentialAccountStatus::Configured,
                            refresh.scopes,
                        ))
                        .await?;
                    self.report_for(&updated, request.requester_extension.as_ref(), true)
                        .await
                }
                Err(AuthProductError::InvalidGrant) => {
                    self.report_terminal_refresh_status(
                        &lookup_request,
                        &account,
                        request.requester_extension.as_ref(),
                        CredentialAccountStatus::Revoked,
                    )
                    .await
                }
                Err(AuthProductError::RefreshFailed | AuthProductError::TokenExchangeFailed) => {
                    self.report_terminal_refresh_status(
                        &lookup_request,
                        &account,
                        request.requester_extension.as_ref(),
                        CredentialAccountStatus::RefreshFailed,
                    )
                    .await
                }
                Err(error) => Err(error),
            }
        }
        .await;
        drop(refresh_lock);
        self.release_refresh_lock(initial_account.id);
        result
    }
}

fn account_is_authorized_for_requester(
    account: &CredentialAccount,
    requester_extension: Option<&ExtensionId>,
) -> bool {
    match account.ownership {
        CredentialOwnership::UserReusable => true,
        CredentialOwnership::ExtensionOwned => account
            .owner_extension
            .as_ref()
            .is_some_and(|owner_extension| requester_extension == Some(owner_extension)),
        CredentialOwnership::SharedAdminManaged => requester_extension
            .is_some_and(|requester| account.granted_extensions.contains(requester)),
        CredentialOwnership::System => false,
    }
}

fn single_account_recovery(account: &CredentialAccount) -> CredentialRecoveryProjection {
    let (kind, reason) = recovery_kind_and_reason_for_status(account.status);
    match kind {
        CredentialRecoveryKind::Configured => {
            CredentialRecoveryProjection::configured(account.provider.clone(), account.projection())
        }
        CredentialRecoveryKind::SetupRequired => CredentialRecoveryProjection::setup_required(
            account.provider.clone(),
            reason,
            vec![account.projection()],
        ),
        CredentialRecoveryKind::ReauthorizeRequired => {
            CredentialRecoveryProjection::reauthorize_required(
                account.provider.clone(),
                reason,
                vec![account.projection()],
            )
        }
        CredentialRecoveryKind::AccountSelectionRequired => {
            unreachable!("single account recovery cannot produce account selection required")
        }
    }
}

fn recovery_kind_and_reason_for_status(
    status: CredentialAccountStatus,
) -> (CredentialRecoveryKind, CredentialRecoveryReason) {
    match status {
        CredentialAccountStatus::Configured => (
            CredentialRecoveryKind::Configured,
            CredentialRecoveryReason::Configured,
        ),
        CredentialAccountStatus::PendingSetup => (
            CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryReason::PendingSetup,
        ),
        CredentialAccountStatus::Missing => (
            CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryReason::AccountMissing,
        ),
        CredentialAccountStatus::Inactive => (
            CredentialRecoveryKind::SetupRequired,
            CredentialRecoveryReason::AccountInactive,
        ),
        CredentialAccountStatus::Expired => (
            CredentialRecoveryKind::ReauthorizeRequired,
            CredentialRecoveryReason::AccountExpired,
        ),
        CredentialAccountStatus::RefreshFailed => (
            CredentialRecoveryKind::ReauthorizeRequired,
            CredentialRecoveryReason::RefreshFailed,
        ),
        CredentialAccountStatus::Revoked => (
            CredentialRecoveryKind::ReauthorizeRequired,
            CredentialRecoveryReason::AccountRevoked,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthProviderId, AuthSessionId, AuthSurface, CredentialAccountId, CredentialAccountLabel,
        CredentialAccountStatus, ProviderScope, scope::AuthProductScope,
    };
    use chrono::Utc;
    use ironclaw_host_api::{
        ids::{InvocationId, UserId},
        resource::ResourceScope,
    };

    /// Build a minimal CredentialAccount using the same idiom as domain.rs tests.
    fn make_account(scope: AuthProductScope) -> CredentialAccount {
        CredentialAccount {
            id: CredentialAccountId::new(),
            scope,
            provider: AuthProviderId::new("github").unwrap(),
            label: CredentialAccountLabel::new("github-account").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: None,
            refresh_secret: None,
            scopes: vec![ProviderScope::new("read").unwrap()],
            provider_identity: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Build a base ResourceScope for a known owner with a given invocation_id.
    fn owner_resource(invocation_id: InvocationId) -> ResourceScope {
        ResourceScope::local_default(UserId::new("alice").unwrap(), invocation_id).unwrap()
    }

    // Case 1: all axes match, including surface and session. invocation_id differs
    // between the flow scope and the account scope to prove invocation_id is ignored
    // (the exact-match invariant applies only to session_id and surface).
    #[test]
    fn binding_scope_owns_account_returns_true_when_all_axes_match() {
        let session = AuthSessionId::new("ses-abc").unwrap();

        // Account was created in an earlier flow with invocation_id A.
        let account_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Web)
                .with_session_id(session.clone());
        let account = make_account(account_scope);

        // Current reconnect flow has a fresh invocation_id B — should still own.
        let flow_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Web)
                .with_session_id(session);

        assert!(
            binding_scope_owns_account(&flow_scope, &account),
            "same owner/surface/session with differing invocation_id must return true"
        );
    }

    /// Binding is an **owner** boundary and nothing more.
    ///
    /// This replaces four separate cases that each asserted a cross-surface or
    /// cross-session bind was rejected. Those axes are no longer part of a
    /// credential's identity: an account minted by the chat auth gate
    /// (`Callback`, with a session) and one reconnected from the extensions
    /// page (`Web`, without) are the same user's credential, and refusing to
    /// bind them is what stranded a valid credential as "needs setup". Only
    /// tenant/user/project may reject a bind — pinned by the foreign-owner
    /// cases in `product_auth::durable::tests`.
    #[test]
    fn binding_crosses_surface_and_session_but_never_the_owner() {
        let account = make_account(
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Callback)
                .with_session_id(AuthSessionId::new("session-s1").unwrap()),
        );

        for flow_scope in [
            // Same owner, every combination of the axes that used to reject.
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Api),
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Web)
                .with_session_id(AuthSessionId::new("session-s2").unwrap()),
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Callback)
                .with_session_id(AuthSessionId::new("session-s1").unwrap()),
        ] {
            assert!(
                binding_scope_owns_account(&flow_scope, &account),
                "same owner must bind regardless of surface/session: {:?}",
                flow_scope.surface
            );
        }

        // The boundary that still holds. Both identifiers are asserted: this is
        // the test that pins tenant+user as *the* security boundary after four
        // rejection cases were deleted, so an edit that drops either comparison
        // must fail here.
        let mut foreign_user = owner_resource(InvocationId::new());
        foreign_user.user_id = UserId::new("mallory").unwrap();
        assert!(
            !binding_scope_owns_account(
                &AuthProductScope::new(foreign_user, AuthSurface::Api),
                &account
            ),
            "a different user must never bind another user's credential"
        );

        let mut foreign_tenant = owner_resource(InvocationId::new());
        foreign_tenant.tenant_id = TenantId::new("tenant-other").unwrap();
        assert!(
            !binding_scope_owns_account(
                &AuthProductScope::new(foreign_tenant, AuthSurface::Api),
                &account
            ),
            "a different tenant must never bind another tenant's credential"
        );
    }
}
