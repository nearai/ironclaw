use std::{collections::HashMap, fmt, sync::Arc, sync::Mutex};

use async_trait::async_trait;
use ironclaw_extension_contracts::linked_session::{LinkedSessionVersion, SessionBytes};
use ironclaw_host_api::ids::{
    AgentId, ExtensionId, MissionId, ProjectId, SecretHandle, TenantId, ThreadId, UserId,
};
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
    /// How many times a linked-device session has been established against
    /// this account. `0` for every account that is not a linked device.
    ///
    /// **Part of the identity of a live session, not decoration.** A user id
    /// never changes across unlink/relink, so anything caching a session
    /// (a pooled vendor client, an opened custody handle) has to key on this
    /// too — bumping it is what invalidates the old credential's readers.
    ///
    /// `#[serde(default)]` because this is a persisted record: rows written
    /// before linked devices existed carry no such key and must rehydrate.
    #[serde(default)]
    pub link_revision: u64,
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
            .field("link_revision", &self.link_revision)
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

    /// Whether this account carries a linked-device session.
    ///
    /// `link_revision` is `0` for every account that has never been linked, and
    /// [`CredentialAccountService::bump_link_revision`] is the only writer, so a
    /// non-zero revision is the durable record of "a live vendor device
    /// authorization hangs off this credential".
    pub fn is_linked_device(&self) -> bool {
        self.link_revision > 0
    }

    /// Whether this account carries the ownership a linked device requires:
    /// [`CredentialOwnership::ExtensionOwned`], a named owner extension, and no
    /// grants.
    ///
    /// **Not a style preference — the reusable default is a reachability bug
    /// here.** [`Self::is_authorized_for_requester`] answers `true` for *any*
    /// requester on [`CredentialOwnership::UserReusable`], so a linked device
    /// left at that default would hand every installed extension a working
    /// handle onto the user's personal vendor session; and ownership-aware
    /// cleanup deliberately does not delete reusable credentials, so the same
    /// account (and the live device behind it) would survive uninstall. Both
    /// consequences follow from the ownership fields alone, which is why the
    /// pin is checked at the record rather than trusted at each mint site
    /// (PROPOSAL §4.5).
    pub fn linked_device_ownership_is_pinned(&self) -> bool {
        self.ownership == CredentialOwnership::ExtensionOwned
            && self.owner_extension.is_some()
            && self.granted_extensions.is_empty()
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

impl NewCredentialAccount {
    /// The one sanctioned shape for the credential account a device link mints.
    ///
    /// **Every ownership field is pinned here rather than chosen at the call
    /// site**, because the fields a mint site would otherwise fill in by hand
    /// are exactly the ones whose defaults are dangerous: reusable ownership
    /// makes the account reachable by every installed extension and survives
    /// uninstall along with the live vendor device
    /// ([`CredentialAccount::linked_device_ownership_is_pinned`] states the
    /// mechanism). So `ownership` is [`CredentialOwnership::ExtensionOwned`],
    /// `owner_extension` is the extension that ran the link, and
    /// `granted_extensions` is empty — sharing a linked device is not a thing
    /// this constructor can express.
    ///
    /// `scopes` is empty and `refresh_secret` is `None` by construction too: a
    /// linked device holds the account's own authority, so there is no scope to
    /// intersect and no refresh token to exchange. The session blob rotates
    /// through custody compare-and-swap, not through a refresh.
    pub fn for_linked_device(
        scope: AuthProductScope,
        provider: AuthProviderId,
        label: CredentialAccountLabel,
        owner_extension: ExtensionId,
        access_secret: SecretHandle,
    ) -> Self {
        Self {
            scope,
            provider,
            label,
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::ExtensionOwned,
            owner_extension: Some(owner_extension),
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret),
            refresh_secret: None,
            scopes: Vec::new(),
        }
    }
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

/// Addresses the opaque session material of exactly one linked account, at
/// exactly one link revision.
///
/// The revision is part of the address, not a hint: a request presenting a
/// stale one is refused ([`AuthProductError::LinkRevisionStale`]) rather than
/// served, so a caller holding a handle from before an unlink cannot read or
/// clobber the credential that replaced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaqueMaterialRequest {
    pub scope: AuthProductScope,
    pub account_id: CredentialAccountId,
    pub requester_extension: Option<ExtensionId>,
    pub link_revision: u64,
}

/// A loaded blob and the token to present on the next write.
///
/// Not `PartialEq`, not `Clone`-compared, and never `Debug`-printed in full:
/// [`SessionBytes`] redacts to a length. The bytes are a full account
/// credential in a vendor-private format that **this crate deliberately
/// cannot read** — see [`CredentialAccountService::store_opaque_material`].
#[derive(Debug)]
pub struct OpaqueMaterialSnapshot {
    pub material: SessionBytes,
    pub version: LinkedSessionVersion,
}

/// A compare-and-swap write of opaque session material.
#[derive(Debug)]
pub struct OpaqueMaterialWrite {
    pub target: OpaqueMaterialRequest,
    /// The version the caller loaded. [`LinkedSessionVersion::absent`] is the
    /// only value a genuine first write may present.
    pub expected: LinkedSessionVersion,
    pub material: SessionBytes,
}

/// What a compare-and-swap write did.
#[derive(Debug)]
pub enum OpaqueMaterialWriteOutcome {
    /// The write applied; `version` is the token for the next one.
    Stored { version: LinkedSessionVersion },
    /// The write lost: someone else stored newer material.
    ///
    /// **This is the whole point of the method.** `SecretStorePort::put` is
    /// last-writer-wins, and a rotating vendor auth key clobbered by a
    /// concurrent write leaves a session the vendor rejects — a silently dead
    /// link. Returning the current version lets the owner of the format
    /// reload and merge; retrying the write unconditionally is exactly the
    /// bug this exists to prevent.
    Conflict { current: LinkedSessionVersion },
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

    /// Load the opaque linked-device session material bound to an account.
    ///
    /// `Ok(None)` means "nothing stored yet"; a stale `link_revision` is an
    /// error, not an absence, because the two demand different recoveries.
    ///
    /// Defaulted and fail-closed: a credential service with no linked-device
    /// custody keeps compiling and refuses the call by name.
    async fn load_opaque_material(
        &self,
        _request: OpaqueMaterialRequest,
    ) -> Result<Option<OpaqueMaterialSnapshot>, AuthProductError> {
        Err(AuthProductError::UnsupportedOperation {
            operation: "load_opaque_material",
        })
    }

    /// Compare-and-swap the opaque linked-device session material.
    ///
    /// **This crate owns conflict *detection* and nothing else.** The blob is
    /// opaque bytes here by design: merging two versions of a vendor session
    /// (union the peer cache, take the newest cursor, never replace a live
    /// auth key) requires reading a vendor-private structure, and the crate
    /// that may read it is the extension package — not this one, which may not
    /// name a vendor at all. So a lost CAS returns
    /// [`OpaqueMaterialWriteOutcome::Conflict`] with the current version and
    /// stops; the semantic merge, and its tests, live package-side.
    ///
    /// Implementations must not last-writer-wins, and must not inspect,
    /// parse, log, or embed `material` in an error.
    async fn store_opaque_material(
        &self,
        _write: OpaqueMaterialWrite,
    ) -> Result<OpaqueMaterialWriteOutcome, AuthProductError> {
        Err(AuthProductError::UnsupportedOperation {
            operation: "store_opaque_material",
        })
    }

    /// Advance an account's `link_revision`, returning the updated record.
    ///
    /// Called once per (re)link, after custody is durable. The bump is what
    /// invalidates every handle and cached client bound to the previous
    /// revision, so it must be a single durable write, never a read-then-set
    /// from a caller-supplied value.
    ///
    /// **This is where the ownership pin is enforced.** The bump is the
    /// operation that makes an account a linked device, so an implementation
    /// must refuse it on an account that is not
    /// [`CredentialAccount::linked_device_ownership_is_pinned`] — otherwise a
    /// reusable account picks up a live vendor device authorization that every
    /// installed extension may use and that uninstall will not delete
    /// (PROPOSAL §4.5). Checking here rather than only at the mint site is
    /// deliberate: the mint site is one caller, and this is the invariant's
    /// only durable writer.
    async fn bump_link_revision(
        &self,
        _scope: &AuthProductScope,
        _account_id: CredentialAccountId,
    ) -> Result<CredentialAccount, AuthProductError> {
        Err(AuthProductError::UnsupportedOperation {
            operation: "bump_link_revision",
        })
    }

    /// Complete one device link: mint (or reuse) the linked-device credential
    /// account, advance its `link_revision`, and store the session material —
    /// in that order, so a completion is never reported before custody is
    /// durable (PROPOSAL §4.3: store → mint → report; the caller reports).
    ///
    /// The one place the completion policy lives, deliberately a provided
    /// method built from the trait's own operations so every implementation
    /// behaves identically:
    ///
    /// - **Reuse before create.** A non-revoked linked-device account for the
    ///   same `(scope, provider, owner_extension)` is re-linked rather than
    ///   duplicated — its revision bumps, which is what evicts every live
    ///   handle bound to the old one. A **revoked** account is never reused:
    ///   its status machine is terminal, so a fresh account is minted.
    /// - **Ownership is pinned at the constructor** —
    ///   [`NewCredentialAccount::for_linked_device`] — and enforced again by
    ///   [`CredentialAccountService::bump_link_revision`].
    /// - **The blob write is load-then-CAS, never absent-only.** A crashed
    ///   prior link may have left an orphan blob; relinking overwrites it.
    ///   When the orphan cannot even be loaded, the write still learns the
    ///   current version from the conflict outcome and retries exactly once —
    ///   so a corrupt orphan cannot brick relinking, and a genuine concurrent
    ///   writer still wins.
    async fn complete_linked_device_link(
        &self,
        request: LinkedDeviceLinkCompletion,
    ) -> Result<CredentialAccount, AuthProductError> {
        let reusable = self
            .list_accounts(CredentialAccountListRequest {
                scope: request.scope.clone(),
                provider: request.provider.clone(),
                requester_extension: Some(request.owner_extension.clone()),
                cursor: None,
                limit: CredentialAccountListRequest::MAX_LIMIT,
            })
            .await?
            .accounts
            .into_iter()
            .find(|account| {
                account.ownership == CredentialOwnership::ExtensionOwned
                    && account.owner_extension.as_ref() == Some(&request.owner_extension)
                    && account.granted_extensions.is_empty()
                    && account.status != CredentialAccountStatus::Revoked
            });
        let account = match reusable {
            Some(projection) => self
                .get_account(CredentialAccountLookupRequest {
                    scope: request.scope.clone(),
                    account_id: projection.id,
                    requester_extension: Some(request.owner_extension.clone()),
                })
                .await?
                .ok_or(AuthProductError::CredentialMissing)?,
            None => {
                self.create_account(NewCredentialAccount::for_linked_device(
                    request.scope.clone(),
                    request.provider.clone(),
                    request.label.clone(),
                    request.owner_extension.clone(),
                    linked_session_secret_handle()?,
                ))
                .await?
            }
        };
        let account = self.bump_link_revision(&request.scope, account.id).await?;
        let account = if account.status == CredentialAccountStatus::Configured {
            account
        } else {
            self.update_status(
                &request.scope,
                account.id,
                CredentialAccountStatus::Configured,
            )
            .await?
        };

        let target = OpaqueMaterialRequest {
            scope: request.scope.clone(),
            account_id: account.id,
            requester_extension: Some(request.owner_extension.clone()),
            link_revision: account.link_revision,
        };
        // Load-then-CAS: learn the stored version (an orphan blob from a
        // crashed link is legal here), then overwrite it.
        let expected = match self.load_opaque_material(target.clone()).await {
            Ok(Some(snapshot)) => snapshot.version,
            Ok(None) => LinkedSessionVersion::absent(),
            // silent-ok: best-effort version probe before the relink write.
            // A failed load gets the same answer as a missing blob because
            // the compare-and-swap below is the authority either way: if a
            // blob exists after all, this `Absent` expectation loses with the
            // current version and the conflict arm retries against it — so a
            // transient read error can cost one extra round-trip, never a
            // clobbered or corrupted custody record.
            Err(_) => LinkedSessionVersion::absent(),
        };
        let material = SessionBytes::new(request.material.expose().to_vec())
            .map_err(|_| AuthProductError::invalid_request("session material violates bounds"))?;
        let outcome = self
            .store_opaque_material(OpaqueMaterialWrite {
                target: target.clone(),
                expected,
                material,
            })
            .await?;
        match outcome {
            OpaqueMaterialWriteOutcome::Stored { .. } => Ok(account),
            OpaqueMaterialWriteOutcome::Conflict { current } => {
                // One retry against the version the conflict reported — the
                // corrupt-orphan case. A second conflict is a genuine
                // concurrent writer and the caller must not blind-write over
                // it.
                let material =
                    SessionBytes::new(request.material.expose().to_vec()).map_err(|_| {
                        AuthProductError::invalid_request("session material violates bounds")
                    })?;
                match self
                    .store_opaque_material(OpaqueMaterialWrite {
                        target,
                        expected: current,
                        material,
                    })
                    .await?
                {
                    OpaqueMaterialWriteOutcome::Stored { .. } => Ok(account),
                    OpaqueMaterialWriteOutcome::Conflict { .. } => {
                        Err(AuthProductError::BackendConflict)
                    }
                }
            }
        }
    }
}

/// Everything a completed device link resolves to before the account exists.
///
/// `material` is the session credential the vendor handshake produced —
/// opaque bytes this crate stores and never reads.
#[derive(Debug)]
pub struct LinkedDeviceLinkCompletion {
    pub scope: AuthProductScope,
    pub provider: AuthProviderId,
    pub owner_extension: ExtensionId,
    pub label: CredentialAccountLabel,
    pub material: SessionBytes,
}

/// Mint the secret handle one linked-device account's session blob lives
/// under. Fresh per account — the association travels on the account record
/// (`access_secret`), which is also what cleanup purges by, so the name only
/// has to be unique and shaped like a handle.
fn linked_session_secret_handle() -> Result<SecretHandle, AuthProductError> {
    SecretHandle::new(format!(
        "product-auth-linked-session-{}",
        uuid::Uuid::new_v4()
    ))
    .map_err(|error| {
        AuthProductError::invalid_request(format!("linked-session handle invalid: {error}"))
    })
}

/// Stable credential-account owner fields used by read models that need to
/// find accounts across transient invocation ids, product surfaces, or runtime
/// sub-scopes. Missing mission/thread/session ids match both global and
/// scoped accounts for the owner; present ids match only that exact scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountOwnerScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub session_id: Option<crate::AuthSessionId>,
}

impl CredentialAccountOwnerScope {
    pub fn from_scope(scope: &AuthProductScope) -> Self {
        Self {
            tenant_id: scope.resource.tenant_id.clone(),
            user_id: scope.resource.user_id.clone(),
            agent_id: scope.resource.agent_id.clone(),
            project_id: scope.resource.project_id.clone(),
            mission_id: scope.resource.mission_id.clone(),
            thread_id: scope.resource.thread_id.clone(),
            session_id: scope.session_id.clone(),
        }
    }

    pub fn matches(&self, account: &CredentialAccount) -> bool {
        let resource = &account.scope.resource;
        resource.tenant_id == self.tenant_id
            && resource.user_id == self.user_id
            && resource.agent_id == self.agent_id
            && resource.project_id == self.project_id
            && self
                .mission_id
                .as_ref()
                .is_none_or(|mission_id| resource.mission_id.as_ref() == Some(mission_id))
            && self
                .thread_id
                .as_ref()
                .is_none_or(|thread_id| resource.thread_id.as_ref() == Some(thread_id))
            && self
                .session_id
                .as_ref()
                .is_none_or(|session_id| account.scope.session_id.as_ref() == Some(session_id))
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
/// reconnect and forked a duplicate account (#4935 defect A). This keeps
/// tenant/user/agent/project hard-required (via [`CredentialAccountOwnerScope`])
/// and `session_id` matched (it is path-segmenting), while clearing
/// `thread_id`/`mission_id` and ignoring `invocation_id` (which
/// [`CredentialAccountOwnerScope`] does not compare). Requester authorization is
/// enforced separately by the callers; this is only the owner-boundary check.
///
/// `session_id` and `surface` are compared for **exact** equality (including
/// `None == None` for session), NOT wildcarded the way
/// `CredentialAccountOwnerScope::matches` wildcards a `None` owner session for
/// runtime reads. The bind/update *write* path is segmented on disk by both
/// `surface` and `session_id` (`product_auth_durable` keys account records by
/// surface path segment + session), and the update reads the account at the
/// flow scope's surface/session path — so binding a flow to an account stored
/// on a different surface (or session) would select a record the callback can
/// never read or update, and would surface as a spurious `CredentialMissing`
/// that aborts the reconnect instead of an unbound fresh flow. Require exact
/// surface and session equality so a cross-surface / cross-session account is
/// never bound.
pub fn binding_scope_owns_account(scope: &AuthProductScope, account: &CredentialAccount) -> bool {
    let owner_scope = scope.to_credential_owner();
    CredentialAccountOwnerScope::from_scope(&owner_scope).matches(account)
        && account.scope.session_id.as_ref() == owner_scope.session_id.as_ref()
        && account.scope.surface == owner_scope.surface
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

    // Linked-device custody is not a provider conversation: there is no token
    // to exchange and no refresh to serialize, so this decorator adds nothing
    // and must not silently swallow the capability of the service it wraps.
    async fn load_opaque_material(
        &self,
        request: OpaqueMaterialRequest,
    ) -> Result<Option<OpaqueMaterialSnapshot>, AuthProductError> {
        self.accounts.load_opaque_material(request).await
    }

    async fn store_opaque_material(
        &self,
        write: OpaqueMaterialWrite,
    ) -> Result<OpaqueMaterialWriteOutcome, AuthProductError> {
        self.accounts.store_opaque_material(write).await
    }

    async fn bump_link_revision(
        &self,
        scope: &AuthProductScope,
        account_id: CredentialAccountId,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.accounts.bump_link_revision(scope, account_id).await
    }

    async fn complete_linked_device_link(
        &self,
        request: LinkedDeviceLinkCompletion,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.accounts.complete_linked_device_link(request).await
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
            link_revision: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// `link_revision` is additive on a **persisted** record: a row written
    /// before linked devices existed carries no such key and must rehydrate to
    /// `0` rather than failing the store open.
    #[test]
    fn an_account_row_written_without_link_revision_rehydrates_at_zero() {
        let account = make_account(AuthProductScope::new(
            owner_resource(InvocationId::new()),
            AuthSurface::Web,
        ));
        let mut wire = serde_json::to_value(&account).expect("serialize");
        let object = wire.as_object_mut().expect("account is a JSON object");
        assert!(
            object.remove("link_revision").is_some(),
            "the field must be written; only its ABSENCE from legacy rows is defaulted"
        );

        let decoded: CredentialAccount =
            serde_json::from_value(wire).expect("legacy row rehydrates");
        assert_eq!(decoded.link_revision, 0);
    }

    /// Build a base ResourceScope for a known owner with a given invocation_id.
    fn owner_resource(invocation_id: InvocationId) -> ResourceScope {
        ResourceScope::local_default(UserId::new("alice").unwrap(), invocation_id).unwrap()
    }

    fn device_link_mint() -> NewCredentialAccount {
        NewCredentialAccount::for_linked_device(
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Web),
            AuthProviderId::new("vendor-a").unwrap(),
            CredentialAccountLabel::new("vendor-a-linked").unwrap(),
            ExtensionId::new("ext-a").unwrap(),
            ironclaw_host_api::ids::SecretHandle::new("vendor_a_session").unwrap(),
        )
    }

    /// PROPOSAL §4.5: the mint pins ownership rather than inheriting the
    /// reusable default.
    #[test]
    fn a_device_link_mint_is_extension_owned_with_no_grants() {
        let mint = device_link_mint();

        assert_eq!(mint.ownership, CredentialOwnership::ExtensionOwned);
        assert_eq!(
            mint.owner_extension,
            Some(ExtensionId::new("ext-a").unwrap()),
            "the linking extension owns the account"
        );
        assert!(
            mint.granted_extensions.is_empty(),
            "a linked device is never shared with another extension"
        );
        // A linked device holds the account's own authority: nothing to refresh
        // and no scope to intersect.
        assert!(mint.refresh_secret.is_none());
        assert!(mint.scopes.is_empty());
        assert_eq!(mint.status, CredentialAccountStatus::Configured);
    }

    /// The hazard the pin closes: `UserReusable` answers `true` for ANY
    /// requester, so an unpinned linked device would be reachable by every
    /// installed extension.
    #[test]
    fn a_different_extension_is_not_authorized_for_a_device_link_account() {
        let mint = device_link_mint();
        let account = CredentialAccount {
            id: CredentialAccountId::new(),
            scope: mint.scope.clone(),
            provider: mint.provider.clone(),
            label: mint.label.clone(),
            status: mint.status,
            ownership: mint.ownership,
            owner_extension: mint.owner_extension.clone(),
            granted_extensions: mint.granted_extensions.clone(),
            access_secret: mint.access_secret.clone(),
            refresh_secret: None,
            scopes: Vec::new(),
            provider_identity: None,
            link_revision: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(account.is_authorized_for_requester(Some(&ExtensionId::new("ext-a").unwrap())));
        assert!(
            !account.is_authorized_for_requester(Some(&ExtensionId::new("ext-b").unwrap())),
            "another installed extension must not reach the linked account"
        );
        assert!(
            !account.is_authorized_for_requester(None),
            "an unattributed requester must not reach the linked account either"
        );
        assert!(account.is_linked_device());
        assert!(account.linked_device_ownership_is_pinned());
    }

    /// The three shapes the pin refuses, each for its own reason.
    #[test]
    fn the_linked_device_ownership_pin_refuses_reusable_ownerless_and_granted_accounts() {
        let mut reusable = make_account(AuthProductScope::new(
            owner_resource(InvocationId::new()),
            AuthSurface::Web,
        ));
        reusable.ownership = CredentialOwnership::UserReusable;
        assert!(!reusable.linked_device_ownership_is_pinned());

        let mut ownerless = reusable.clone();
        ownerless.ownership = CredentialOwnership::ExtensionOwned;
        ownerless.owner_extension = None;
        assert!(!ownerless.linked_device_ownership_is_pinned());

        let mut granted = ownerless.clone();
        granted.owner_extension = Some(ExtensionId::new("ext-a").unwrap());
        granted.granted_extensions = vec![ExtensionId::new("ext-b").unwrap()];
        assert!(!granted.linked_device_ownership_is_pinned());

        granted.granted_extensions.clear();
        assert!(granted.linked_device_ownership_is_pinned());
    }

    /// `link_revision == 0` is "never linked"; only a bump makes an account a
    /// linked device.
    #[test]
    fn only_a_bumped_link_revision_marks_an_account_as_a_linked_device() {
        let mut account = make_account(AuthProductScope::new(
            owner_resource(InvocationId::new()),
            AuthSurface::Web,
        ));

        assert!(!account.is_linked_device());
        account.link_revision = 1;
        assert!(account.is_linked_device());
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

    // Case 2: owner matches and surface matches, but session_id differs.
    // Exact-match invariant on session_id must reject the binding.
    #[test]
    fn binding_scope_owns_account_returns_false_when_session_differs() {
        let account_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Web)
                .with_session_id(AuthSessionId::new("session-s1").unwrap());
        let account = make_account(account_scope);

        let flow_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Web)
                .with_session_id(AuthSessionId::new("session-s2").unwrap());

        assert!(
            !binding_scope_owns_account(&flow_scope, &account),
            "mismatched session_id must return false"
        );
    }

    // Case 3: owner matches and session matches, but surface differs.
    // Exact-match invariant on surface must reject the binding.
    #[test]
    fn binding_scope_owns_account_returns_false_when_surface_differs() {
        let session = AuthSessionId::new("ses-xyz").unwrap();

        let account_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Web)
                .with_session_id(session.clone());
        let account = make_account(account_scope);

        // Same owner and session, but the flow comes from the Chat surface.
        let flow_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Chat)
                .with_session_id(session);

        assert!(
            !binding_scope_owns_account(&flow_scope, &account),
            "mismatched surface must return false"
        );
    }

    // Case 4a: account has Some session, flow scope has None.
    // session_id is compared with as_ref() equality, so Some(..) != None => false.
    #[test]
    fn binding_scope_owns_account_returns_false_when_account_has_session_but_scope_does_not() {
        let account_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Api)
                .with_session_id(AuthSessionId::new("ses-present").unwrap());
        let account = make_account(account_scope);

        // Flow scope carries no session_id.
        let flow_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Api);

        assert!(
            !binding_scope_owns_account(&flow_scope, &account),
            "account Some(session) vs scope None must return false"
        );
    }

    // Case 4b: flow scope has Some session, account has None.
    // same as_ref() equality: Some(..) != None => false.
    #[test]
    fn binding_scope_owns_account_returns_false_when_scope_has_session_but_account_does_not() {
        let account_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Api);
        let account = make_account(account_scope);

        let flow_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Api)
                .with_session_id(AuthSessionId::new("ses-present").unwrap());

        assert!(
            !binding_scope_owns_account(&flow_scope, &account),
            "scope Some(session) vs account None must return false"
        );
    }

    // Case 4c: both scope and account have None session — None == None => true.
    #[test]
    fn binding_scope_owns_account_returns_true_when_both_sessions_are_none() {
        let account_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Api);
        let account = make_account(account_scope);

        let flow_scope =
            AuthProductScope::new(owner_resource(InvocationId::new()), AuthSurface::Api);

        assert!(
            binding_scope_owns_account(&flow_scope, &account),
            "None session on both sides must return true (None == None)"
        );
    }
}
