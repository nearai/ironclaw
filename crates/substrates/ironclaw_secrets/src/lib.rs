// arch-exempt: large_file, §4.3 secrets consolidation — this PR deletes ~660 lines (adapter + InMemorySecretStore); remaining size is pre-existing, plan #6168
//! Tenant-scoped secret service boundary for IronClaw Reborn.
//!
//! This crate stores and leases secret material behind opaque
//! [`SecretHandle`] values. It does not decide authorization, inject secrets into
//! runtimes, emit audit records, or expose raw values through metadata. Runtime
//! injection is not enforced until a higher-level obligation-handler/runtime
//! composition slice consumes these primitives.
#![warn(unreachable_pub)]

mod crypto;
pub mod keychain;
mod legacy_store;
mod placeholder;
mod secret_store;

pub use placeholder::{
    CREDENTIAL_PLACEHOLDER_PREFIX, CREDENTIAL_PLACEHOLDER_SUFFIX_LEN, CredentialPlaceholderOwner,
    CredentialPlaceholderRegistry, CredentialPlaceholderToken, CredentialSessionLease,
};
pub use secret_store::{CredentialBroker, SecretStore};

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
pub use crypto::{
    SecretsCrypto, credential_account_aad, credential_session_aad, filesystem_secret_aad,
    secret_record_aad, validate_master_key_material,
};
use ironclaw_host_api::{
    Timestamp,
    action::NetworkMethod,
    ids::{
        AgentId, CapabilityId, ExtensionId, InvocationId, ProjectId, SecretHandle, TenantId, UserId,
    },
    resource::ResourceScope,
};
pub use legacy_store::SecretError;
pub use secrecy::SecretString as SecretMaterial;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

const CREDENTIAL_ID_MAX_LEN: usize = 128;
const DEFAULT_SECRET_LEASE_TTL_SECONDS: i64 = 300;

/// Default lifetime handed to a [`CredentialSession`] whose
/// [`CredentialSessionRequest::expires_at`] is `None`. A caller that asks for
/// "no expiry" still gets a bounded session — see
/// [`CREDENTIAL_SESSION_MAX_TTL_SECONDS`] for why unbounded is never actually
/// granted.
const CREDENTIAL_SESSION_DEFAULT_TTL_SECONDS: i64 = 30 * 60;

/// Hard cap on how far in the future any [`CredentialSession::expires_at`]
/// may be set, regardless of what the caller requests. Explicit revocation
/// (via [`CredentialSessionLease`]/[`InMemoryCredentialBroker::revoke_session`])
/// is the primary way a session's life ends; this is only the backstop for a
/// lease held or leaked past the caller's own error/timeout/panic handling —
/// see `create_session`'s doc comment for why a background sweep task is not
/// the fix here.
const CREDENTIAL_SESSION_MAX_TTL_SECONDS: i64 = 30 * 60;

/// Opaque identifier for a one-shot secret lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretLeaseId(Uuid);

impl SecretLeaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SecretLeaseId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SecretLeaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Redacted metadata for a stored secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretMetadata {
    pub scope: ResourceScope,
    pub handle: SecretHandle,
    /// When the secret material expires, if known.
    ///
    /// Populated only for access tokens written through [`SecretStorePort::put`] with a
    /// non-`None` `expires_at` argument (e.g. OAuth access tokens). Legacy records
    /// and secrets written without a TTL leave this `None`.
    pub expires_at: Option<Timestamp>,
}

/// Lease lifecycle for one secret access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretLeaseStatus {
    Active,
    Consumed,
    Revoked,
    Expired,
}

/// Metadata for a scoped one-shot secret lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretLease {
    pub id: SecretLeaseId,
    pub scope: ResourceScope,
    pub handle: SecretHandle,
    pub status: SecretLeaseStatus,
}

/// Secret service failures. Variants intentionally avoid secret material.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecretStoreError {
    #[error("unknown secret {handle} for tenant/user scope")]
    UnknownSecret {
        scope: Box<ResourceScope>,
        handle: SecretHandle,
    },
    #[error("unknown secret lease {lease_id} for tenant/user scope")]
    UnknownLease {
        scope: Box<ResourceScope>,
        lease_id: SecretLeaseId,
    },
    #[error("secret lease {lease_id} was already consumed")]
    LeaseConsumed { lease_id: SecretLeaseId },
    #[error("secret lease {lease_id} was revoked")]
    LeaseRevoked { lease_id: SecretLeaseId },
    #[error("secret lease {lease_id} expired")]
    LeaseExpired { lease_id: SecretLeaseId },
    #[error("secret expired")]
    SecretExpired,
    #[error("secret backend is misconfigured: {reason}")]
    BackendMisconfigured { reason: String },
    #[error("secret store state is unavailable: {reason}")]
    StoreUnavailable { reason: String },
}

impl SecretStoreError {
    pub fn stable_reason(&self) -> &'static str {
        match self {
            Self::UnknownSecret { .. } => "MissingCredential",
            Self::UnknownLease { .. } => "MissingCredential",
            Self::LeaseConsumed { .. } => "CredentialExpired",
            Self::LeaseRevoked { .. } => "CredentialRevoked",
            Self::LeaseExpired { .. } => "CredentialExpired",
            Self::SecretExpired => "CredentialExpired",
            Self::BackendMisconfigured { .. } => "BackendMisconfigured",
            Self::StoreUnavailable { .. } => "BackendUnavailable",
        }
    }

    pub fn is_unknown_secret(&self) -> bool {
        matches!(self, Self::UnknownSecret { .. })
    }

    pub fn is_unknown_lease(&self) -> bool {
        matches!(self, Self::UnknownLease { .. })
    }

    pub fn is_consumed(&self) -> bool {
        matches!(self, Self::LeaseConsumed { .. })
    }

    pub fn is_revoked(&self) -> bool {
        matches!(self, Self::LeaseRevoked { .. })
    }

    pub fn is_expired(&self) -> bool {
        matches!(self, Self::SecretExpired | Self::LeaseExpired { .. })
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RedactedJson(Value);

impl RedactedJson {
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    pub fn as_value(&self) -> &Value {
        &self.0
    }

    pub fn into_value(self) -> Value {
        self.0
    }
}

impl fmt::Debug for RedactedJson {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED_JSON]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct CredentialAccountId(String);

impl CredentialAccountId {
    pub fn new(value: impl Into<String>) -> Result<Self, CredentialBrokerError> {
        let value = value.into();
        validate_credential_id("credential_account", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for CredentialAccountId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for CredentialAccountId {
    type Error = CredentialBrokerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<CredentialAccountId> for String {
    fn from(value: CredentialAccountId) -> Self {
        value.0
    }
}

impl<'de> Deserialize<'de> for CredentialAccountId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for CredentialAccountId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Opaque bearer-like identifier for a credential session.
///
/// This id is intentionally not `Serialize`: durable stores persist it through
/// private encrypted DTOs only, and public API/log surfaces must not emit it as
/// a reusable session credential.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialSessionId(Uuid);

impl CredentialSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub(crate) fn to_private_storage_string(self) -> String {
        self.0.to_string()
    }
}

impl Default for CredentialSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CredentialSessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialSessionId([REDACTED])")
    }
}

impl fmt::Display for CredentialSessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Bearer-like identifier: Display must not leak the raw UUID, because
        // `format!("{id}")`, `tracing::info!(%id, ...)`, and any
        // `error.to_string()` interpolation would otherwise echo a value an
        // attacker can reuse. Narrow storage paths must call
        // `to_private_storage_string()` instead.
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialAccountStatus {
    /// Account can issue new sessions and satisfy matching credential requests.
    Active,
    /// Account can no longer issue sessions because its upstream credential or
    /// configured lifetime has expired.
    Expired,
    /// Account was explicitly disabled and must not issue new sessions.
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialPathPolicy {
    Exact(String),
    Prefix(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialTargetPolicy {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: CredentialPathPolicy,
    pub methods: Vec<NetworkMethod>,
}

impl CredentialTargetPolicy {
    pub fn matches(&self, method: &NetworkMethod, url: &str) -> bool {
        let Ok(parsed) = url::Url::parse(url) else {
            return false;
        };
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return false;
        }
        if raw_url_path(url).is_some_and(path_has_encoded_traversal) {
            return false;
        }
        if self.scheme != parsed.scheme() {
            return false;
        }
        if !parsed
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case(&self.host))
        {
            return false;
        }
        if self
            .port
            .is_some_and(|port| Some(port) != parsed.port_or_known_default())
        {
            return false;
        }
        if !self.methods.iter().any(|allowed| allowed == method) {
            return false;
        }
        match &self.path {
            CredentialPathPolicy::Exact(path) => parsed.path() == path,
            CredentialPathPolicy::Prefix(prefix) => path_matches_prefix(parsed.path(), prefix),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialAccount {
    pub scope: ResourceScope,
    pub id: CredentialAccountId,
    pub provider_or_extension_id: ExtensionId,
    pub label: String,
    pub status: CredentialAccountStatus,
    pub secret_handles: Vec<SecretHandle>,
    pub allowed_targets: Vec<CredentialTargetPolicy>,
    pub redacted_metadata: RedactedJson,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialSession {
    scope: ResourceScope,
    invocation_id: InvocationId,
    capability_id: CapabilityId,
    extension_id: ExtensionId,
    account_id: CredentialAccountId,
    secret_handles: Vec<SecretHandle>,
    allowed_targets: Vec<CredentialTargetPolicy>,
    expires_at: Option<Timestamp>,
    max_uses: Option<u64>,
    correlation_id: CredentialSessionId,
}

/// Crate-private constructor for [`CredentialSession`] used by durable
/// storage backends to rehydrate sessions read from disk.
///
/// This is **not** part of the public API; the fields stay private so callers
/// outside the crate cannot mint trust-bearing sessions. The libSQL/Postgres
/// stores already use the equivalent `pub(crate)` pattern inline; the
/// filesystem backend lives in a sibling module and uses this explicit helper
/// to avoid duplicating that constructor for every backend.
// arch-exempt: too_many_args, needs CredentialSession reconstruction context, plan #4088
#[allow(clippy::too_many_arguments)]
pub(crate) fn __internal_session_for_secret_store(
    scope: ResourceScope,
    invocation_id: InvocationId,
    capability_id: CapabilityId,
    extension_id: ExtensionId,
    account_id: CredentialAccountId,
    secret_handles: Vec<SecretHandle>,
    allowed_targets: Vec<CredentialTargetPolicy>,
    expires_at: Option<Timestamp>,
    max_uses: Option<u64>,
    correlation_id: CredentialSessionId,
) -> CredentialSession {
    CredentialSession {
        scope,
        invocation_id,
        capability_id,
        extension_id,
        account_id,
        secret_handles,
        allowed_targets,
        expires_at,
        max_uses,
        correlation_id,
    }
}

impl CredentialSession {
    pub fn scope(&self) -> &ResourceScope {
        &self.scope
    }
    pub fn invocation_id(&self) -> InvocationId {
        self.invocation_id
    }
    pub fn capability_id(&self) -> &CapabilityId {
        &self.capability_id
    }
    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }
    pub fn account_id(&self) -> &CredentialAccountId {
        &self.account_id
    }
    pub fn secret_handles(&self) -> &[SecretHandle] {
        &self.secret_handles
    }
    pub fn allowed_targets(&self) -> &[CredentialTargetPolicy] {
        &self.allowed_targets
    }
    pub fn expires_at(&self) -> Option<Timestamp> {
        self.expires_at
    }
    pub fn max_uses(&self) -> Option<u64> {
        self.max_uses
    }
    pub fn correlation_id(&self) -> CredentialSessionId {
        self.correlation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CredentialBrokerError {
    #[error("invalid credential account id {value}: {reason}")]
    InvalidAccountId { value: String, reason: String },
    #[error("missing credential account {account_id} for tenant/user scope")]
    MissingCredential { account_id: CredentialAccountId },
    #[error("credential account {account_id} does not match caller scope")]
    CredentialScopeMismatch { account_id: CredentialAccountId },
    #[error("credential session request invocation does not match caller scope for {account_id}")]
    CredentialInvocationMismatch { account_id: CredentialAccountId },
    #[error("credential broker state is unavailable: {reason}")]
    BrokerUnavailable { reason: String },
    #[error("credential session {session_id} is unknown")]
    UnknownSession { session_id: CredentialSessionId },
    #[error("credential session {session_id} is expired")]
    SessionExpired { session_id: CredentialSessionId },
    #[error("credential session {session_id} has no uses remaining")]
    SessionUseLimitExceeded { session_id: CredentialSessionId },
    #[error("credential account {account_id} is expired")]
    CredentialExpired { account_id: CredentialAccountId },
    #[error("credential account {account_id} is revoked")]
    CredentialRevoked { account_id: CredentialAccountId },
    #[error("credential account {account_id} is not allowed for requested extension")]
    CredentialExtensionMismatch { account_id: CredentialAccountId },
    #[error("credential account {account_id} is not allowed for requested target")]
    CredentialPolicyMismatch { account_id: CredentialAccountId },
    #[error("credential placeholder token is invalid: {reason}")]
    InvalidPlaceholderToken { reason: String },
}

impl CredentialBrokerError {
    pub fn stable_reason(&self) -> &'static str {
        match self {
            Self::InvalidAccountId { .. } => "MissingCredential",
            Self::MissingCredential { .. } => "MissingCredential",
            Self::CredentialScopeMismatch { .. } => "CredentialScopeMismatch",
            Self::CredentialInvocationMismatch { .. } => "CredentialScopeMismatch",
            Self::BrokerUnavailable { .. } => "BackendUnavailable",
            Self::UnknownSession { .. } => "MissingCredential",
            Self::SessionExpired { .. } => "CredentialExpired",
            Self::SessionUseLimitExceeded { .. } => "CredentialExpired",
            Self::CredentialExpired { .. } => "CredentialExpired",
            Self::CredentialRevoked { .. } => "CredentialRevoked",
            Self::CredentialExtensionMismatch { .. } => "CredentialPolicyMismatch",
            Self::CredentialPolicyMismatch { .. } => "CredentialPolicyMismatch",
            Self::InvalidPlaceholderToken { .. } => "MissingCredential",
        }
    }

    pub fn is_expired(&self) -> bool {
        matches!(
            self,
            Self::SessionExpired { .. } | Self::CredentialExpired { .. }
        )
    }

    pub fn is_use_limit_exceeded(&self) -> bool {
        matches!(self, Self::SessionUseLimitExceeded { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialSessionRequest {
    pub scope: ResourceScope,
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub extension_id: ExtensionId,
    pub account_id: CredentialAccountId,
    pub method: NetworkMethod,
    pub url: String,
    pub expires_at: Option<Timestamp>,
    pub max_uses: Option<u64>,
}

#[async_trait]
pub trait CredentialAccountStore: Send + Sync {
    async fn put_account(
        &self,
        account: CredentialAccount,
    ) -> Result<CredentialAccount, CredentialBrokerError>;

    async fn get_account(
        &self,
        scope: &ResourceScope,
        account_id: &CredentialAccountId,
    ) -> Result<Option<CredentialAccount>, CredentialBrokerError>;

    async fn accounts_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<CredentialAccount>, CredentialBrokerError>;
}

#[async_trait]
pub trait CredentialSessionStore: Send + Sync {
    async fn issue_session(
        &self,
        session: CredentialSession,
    ) -> Result<CredentialSession, CredentialBrokerError>;

    async fn get_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
    ) -> Result<Option<CredentialSession>, CredentialBrokerError>;

    async fn validate_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError>;

    async fn consume_session_use(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError>;
}

/// All session-lifecycle bookkeeping `InMemoryCredentialBroker` keeps beyond
/// account storage, collapsed under one lock.
///
/// `sessions`, `jit_minted`, and `sessions_by_placeholder` used to be three
/// (really four, counting the outstanding-lease counter) separate `Mutex`es
/// on the broker. They are not independent state — `jit_minted` and
/// `sessions_by_placeholder` are both secondary indices over the same
/// `sessions` entries — so five separately-lockable pieces of one logical
/// session record was over-fragmentation: every mint/revoke sequence had to
/// hand-synchronize removals across maps and reason about which lock was
/// allowed to be held while acquiring which other one (see the
/// `CredentialPlaceholderRegistry` / `RegistryState` pattern just above,
/// which this mirrors). A single lock over all three makes every
/// mint/reuse/revoke sequence atomic from an outside observer's perspective,
/// and removes the need for lock-ordering rules or "ignoring" twin methods
/// that used to exist purely to dodge self-deadlock across separate
/// acquisitions of the same logical state.
#[derive(Debug, Default)]
struct SessionState {
    sessions: HashMap<CredentialSessionId, CredentialSessionRecord>,
    /// Secondary index for JIT minting: `(invocation, capability, account)` ->
    /// the session already minted for it, so re-use within the same dispatch
    /// does not mint a second session. See [`placeholder::JitMintKey`] and
    /// [`InMemoryCredentialBroker::mint_on_first_use`]. Pruned (by session id)
    /// on [`InMemoryCredentialBroker::revoke_session`] so a long-lived process
    /// does not accumulate one entry per invocation forever.
    jit_minted: HashMap<placeholder::JitMintKey, CredentialSessionId>,
    /// Secondary index the egress proxy (W6-EGRESS-PROXY, not built yet) will
    /// use: placeholder token -> the live session(s) currently bound to it.
    /// Multi-valued because a placeholder is stable per `(tenant, user,
    /// provider)` while sessions are minted per invocation/account, so more
    /// than one live session can legitimately be bound to the same
    /// placeholder at once (e.g. two overlapping invocations, or two
    /// accounts under one provider) — a single-slot map would silently drop
    /// one binding when the other was written. See
    /// [`InMemoryCredentialBroker::find_session_by_placeholder`]. Choosing
    /// among multiple scope-matching candidates by request target (e.g.
    /// which of two same-provider accounts a specific outbound URL should
    /// use) is the egress proxy's job, not this registry's — it is not built
    /// yet, so `find_session_by_placeholder` returns the first scope-match it
    /// finds. Pruned (by session id) on `revoke_session`.
    sessions_by_placeholder: HashMap<CredentialPlaceholderToken, HashSet<CredentialSessionId>>,
}

#[derive(Debug, Default)]
pub struct InMemoryCredentialBroker {
    accounts: Mutex<HashMap<CredentialAccountKey, CredentialAccount>>,
    /// See [`SessionState`] for why this is one lock rather than several.
    /// `accounts` stays a separate `Mutex`: `create_session` (by way of
    /// `build_session`) always drops the accounts guard before touching
    /// session state, so it is cleanly separable and merging it would only
    /// widen the critical section for no benefit.
    session_state: Mutex<SessionState>,
}

#[derive(Debug, Clone)]
struct CredentialSessionRecord {
    session: CredentialSession,
    uses: u64,
    /// Outstanding-lease count for this session. `mint_on_first_use` can hand
    /// out more than one [`crate::CredentialSessionLease`] for the *same*
    /// session (its cache-hit path reuses a still-live session rather than
    /// minting a second one), so revoking one lease must not revoke the
    /// session out from under another lease still holding it — the session is
    /// only actually revoked when the last outstanding lease releases it. See
    /// [`InMemoryCredentialBroker::release_lease`]. Zero for sessions created
    /// through [`InMemoryCredentialBroker::create_session`] /
    /// [`CredentialSessionStore::issue_session`] directly (no lease tracking
    /// applies outside the JIT-minting path).
    lease_count: usize,
}

/// Locks `mutex`, recovering the inner guard on poison rather than silently
/// no-op'ing. A poisoned lock here must never be treated as "nothing to
/// clean up" — that would let a revoke path fail open and leave a standing
/// grant, contradicting this crate's core invariant for session state.
///
/// Defined here (crate root) rather than in `placeholder.rs` so both that
/// module's `session_state`-locking helpers and this module's own methods
/// (`create_session`, `validate_session`, `consume_session_use`, the
/// `CredentialSessionStore` impl) can use it: a private item defined in a
/// child module is not visible to its parent, only to its own descendants.
fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn ensure_credential_session_record_usable(
    record: &CredentialSessionRecord,
    session_id: CredentialSessionId,
    now: Timestamp,
) -> Result<(), CredentialBrokerError> {
    if record
        .session
        .expires_at
        .is_some_and(|expires_at| expires_at <= now)
    {
        return Err(CredentialBrokerError::SessionExpired { session_id });
    }
    if record
        .session
        .max_uses
        .is_some_and(|max_uses| record.uses >= max_uses)
    {
        return Err(CredentialBrokerError::SessionUseLimitExceeded { session_id });
    }
    Ok(())
}

impl InMemoryCredentialBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_account(&self, account: CredentialAccount) -> Result<(), CredentialBrokerError> {
        self.accounts
            .lock()
            .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                reason: error.to_string(),
            })?
            .insert(
                CredentialAccountKey::new(&account.scope, &account.id),
                account,
            );
        Ok(())
    }

    /// Mints a [`CredentialSession`] for `request`, capping its expiry so a
    /// session can never be genuinely unbounded.
    ///
    /// `request.expires_at` and `request.max_uses` are both `Option`, and a
    /// caller (in practice, `mint_on_first_use`'s upstream capability
    /// dispatch) can legitimately pass `expires_at: None` meaning "I have no
    /// opinion" rather than "never expire". Combined with revocation that
    /// depends on `Drop` running (see [`CredentialSessionLease`]), a lease
    /// held or leaked forever — a caller that never awaits its future to
    /// completion or cancellation, for instance — would otherwise be a
    /// standing grant for the process lifetime. So `expires_at` is always
    /// defaulted-and-capped here: `None` becomes
    /// [`CREDENTIAL_SESSION_DEFAULT_TTL_SECONDS`] out, and anything longer
    /// than [`CREDENTIAL_SESSION_MAX_TTL_SECONDS`] (including an explicit
    /// request for longer) is clamped down to it. Enforcement of the
    /// resulting expiry is the existing lazy check in
    /// `ensure_credential_session_record_usable` / `validate_session` — this
    /// only ever narrows the stored value, it adds no new enforcement path.
    ///
    /// Deliberately **not** paired with a background sweep task: proactive
    /// eviction of expired-but-unrevoked sessions would be standing
    /// infrastructure (a spawn point, shutdown handling, interval tuning) for
    /// a path with zero live callers today, and this crate owns no runtime to
    /// host it on. Explicit revoke stays the primary mechanism; this cap is
    /// only the backstop. Proactive eviction is deferred to whichever later
    /// PR gives this store an owning runtime.
    pub fn create_session(
        &self,
        request: CredentialSessionRequest,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let session = self.build_session(request)?;
        lock_or_recover(&self.session_state).sessions.insert(
            session.correlation_id,
            CredentialSessionRecord {
                session: session.clone(),
                uses: 0,
                lease_count: 0,
            },
        );
        Ok(session)
    }

    /// Validates `request` against its account and constructs the resulting
    /// [`CredentialSession`], without publishing it anywhere.
    ///
    /// Split out of [`InMemoryCredentialBroker::create_session`] so
    /// [`InMemoryCredentialBroker::mint_on_first_use`] (in `placeholder.rs`)
    /// can hold `session_state` locked across the whole lookup-or-mint
    /// sequence while still calling into this account-validation step: this
    /// method only ever locks `accounts`, dropping that guard before
    /// returning, so nesting it inside an already-held `session_state` lock
    /// cannot deadlock and cannot widen `accounts`'s critical section.
    fn build_session(
        &self,
        request: CredentialSessionRequest,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        if request.invocation_id != request.scope.invocation_id {
            return Err(CredentialBrokerError::CredentialInvocationMismatch {
                account_id: request.account_id,
            });
        }
        let accounts =
            self.accounts
                .lock()
                .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                    reason: error.to_string(),
                })?;
        let account = accounts
            .get(&CredentialAccountKey::new(
                &request.scope,
                &request.account_id,
            ))
            .ok_or_else(|| CredentialBrokerError::MissingCredential {
                account_id: request.account_id.clone(),
            })?;
        if CredentialAccountKey::new(&account.scope, &account.id)
            != CredentialAccountKey::new(&request.scope, &request.account_id)
        {
            return Err(CredentialBrokerError::CredentialScopeMismatch {
                account_id: request.account_id,
            });
        }
        if account.provider_or_extension_id != request.extension_id {
            return Err(CredentialBrokerError::CredentialExtensionMismatch {
                account_id: request.account_id,
            });
        }
        match account.status {
            CredentialAccountStatus::Active => {}
            CredentialAccountStatus::Expired => {
                return Err(CredentialBrokerError::CredentialExpired {
                    account_id: request.account_id,
                });
            }
            CredentialAccountStatus::Revoked => {
                return Err(CredentialBrokerError::CredentialRevoked {
                    account_id: request.account_id,
                });
            }
        }
        if !account
            .allowed_targets
            .iter()
            .any(|target| target.matches(&request.method, &request.url))
        {
            return Err(CredentialBrokerError::CredentialPolicyMismatch {
                account_id: request.account_id,
            });
        }
        let now = chrono::Utc::now();
        let default_expires_at =
            now + chrono::Duration::seconds(CREDENTIAL_SESSION_DEFAULT_TTL_SECONDS);
        let max_expires_at = now + chrono::Duration::seconds(CREDENTIAL_SESSION_MAX_TTL_SECONDS);
        let expires_at = request
            .expires_at
            .unwrap_or(default_expires_at)
            .min(max_expires_at);
        Ok(CredentialSession {
            scope: request.scope,
            invocation_id: request.invocation_id,
            capability_id: request.capability_id,
            extension_id: request.extension_id,
            account_id: account.id.clone(),
            secret_handles: account.secret_handles.clone(),
            allowed_targets: account.allowed_targets.clone(),
            expires_at: Some(expires_at),
            max_uses: request.max_uses,
            correlation_id: CredentialSessionId::new(),
        })
    }

    pub fn validate_session(
        &self,
        session_id: CredentialSessionId,
        now: Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let mut state = lock_or_recover(&self.session_state);
        let record = state
            .sessions
            .get_mut(&session_id)
            .ok_or(CredentialBrokerError::UnknownSession { session_id })?;
        ensure_credential_session_record_usable(record, session_id, now)?;
        Ok(record.session.clone())
    }

    pub fn consume_session_use(
        &self,
        session_id: CredentialSessionId,
        now: Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let mut state = lock_or_recover(&self.session_state);
        let record = state
            .sessions
            .get_mut(&session_id)
            .ok_or(CredentialBrokerError::UnknownSession { session_id })?;
        ensure_credential_session_record_usable(record, session_id, now)?;
        record.uses += 1;
        Ok(record.session.clone())
    }
}

#[async_trait]
impl CredentialAccountStore for InMemoryCredentialBroker {
    async fn put_account(
        &self,
        account: CredentialAccount,
    ) -> Result<CredentialAccount, CredentialBrokerError> {
        InMemoryCredentialBroker::put_account(self, account.clone())?;
        Ok(account)
    }

    async fn get_account(
        &self,
        scope: &ResourceScope,
        account_id: &CredentialAccountId,
    ) -> Result<Option<CredentialAccount>, CredentialBrokerError> {
        let accounts =
            self.accounts
                .lock()
                .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                    reason: error.to_string(),
                })?;
        Ok(accounts
            .get(&CredentialAccountKey::new(scope, account_id))
            .cloned())
    }

    async fn accounts_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<CredentialAccount>, CredentialBrokerError> {
        let accounts =
            self.accounts
                .lock()
                .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                    reason: error.to_string(),
                })?;
        Ok(accounts
            .iter()
            .filter(|(key, _)| key.matches_scope_owner(scope))
            .map(|(_, account)| account.clone())
            .collect())
    }
}

#[async_trait]
impl CredentialSessionStore for InMemoryCredentialBroker {
    async fn issue_session(
        &self,
        session: CredentialSession,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        lock_or_recover(&self.session_state).sessions.insert(
            session.correlation_id,
            CredentialSessionRecord {
                session: session.clone(),
                uses: 0,
                lease_count: 0,
            },
        );
        Ok(session)
    }

    async fn get_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
    ) -> Result<Option<CredentialSession>, CredentialBrokerError> {
        let state = lock_or_recover(&self.session_state);
        Ok(state
            .sessions
            .get(&session_id)
            .filter(|record| record.session.scope == *scope)
            .map(|record| record.session.clone()))
    }

    async fn validate_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let state = lock_or_recover(&self.session_state);
        let record = state
            .sessions
            .get(&session_id)
            .filter(|record| record.session.scope == *scope)
            .ok_or(CredentialBrokerError::UnknownSession { session_id })?;
        ensure_credential_session_record_usable(record, session_id, now)?;
        Ok(record.session.clone())
    }

    async fn consume_session_use(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let mut state = lock_or_recover(&self.session_state);
        let record = state
            .sessions
            .get_mut(&session_id)
            .filter(|record| record.session.scope == *scope)
            .ok_or(CredentialBrokerError::UnknownSession { session_id })?;
        ensure_credential_session_record_usable(record, session_id, now)?;
        record.uses += 1;
        Ok(record.session.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CredentialAccountKey {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    account_id: CredentialAccountId,
}

impl CredentialAccountKey {
    fn new(scope: &ResourceScope, account_id: &CredentialAccountId) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            account_id: account_id.clone(),
        }
    }

    fn matches_scope_owner(&self, scope: &ResourceScope) -> bool {
        self.tenant_id == scope.tenant_id
            && self.user_id == scope.user_id
            && self.agent_id == scope.agent_id
            && self.project_id == scope.project_id
    }
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if path_has_encoded_traversal(path) {
        return false;
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
    if path == prefix {
        return true;
    }
    if path.len() > prefix.len() && path.starts_with(prefix) {
        let next_char = path.as_bytes()[prefix.len()];
        return next_char == b'/';
    }
    false
}

fn raw_url_path(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://")?.1;
    let path_start = after_scheme.find('/')?;
    let path_and_suffix = &after_scheme[path_start..];
    Some(
        path_and_suffix
            .split(['?', '#'])
            .next()
            .unwrap_or(path_and_suffix),
    )
}

fn path_has_encoded_traversal(path: &str) -> bool {
    path.split('/').any(|segment| {
        let decoded = percent_decode_bytes(segment.as_bytes());
        matches!(decoded.as_slice(), b"." | b"..")
            || decoded.contains(&b'/')
            || decoded.contains(&b'%')
    })
}

fn percent_decode_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(hi), Some(lo)) =
                (hex_nibble(bytes[index + 1]), hex_nibble(bytes[index + 2]))
        {
            out.push((hi << 4) | lo);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn validate_credential_id(kind: &'static str, value: &str) -> Result<(), CredentialBrokerError> {
    if value.is_empty() {
        return Err(CredentialBrokerError::InvalidAccountId {
            value: value.to_string(),
            reason: format!("{kind} must not be empty"),
        });
    }
    if value.len() > CREDENTIAL_ID_MAX_LEN {
        return Err(CredentialBrokerError::InvalidAccountId {
            value: value.to_string(),
            reason: format!("{kind} must be at most {CREDENTIAL_ID_MAX_LEN} bytes"),
        });
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(CredentialBrokerError::InvalidAccountId {
            value: value.to_string(),
            reason: format!("{kind} must contain only ASCII letters, digits, '-' or '_'"),
        });
    }
    Ok(())
}

/// Scoped secret store contract.
#[async_trait]
pub trait SecretStorePort: Send + Sync {
    /// Stores or replaces a secret under the caller's tenant/user/project scope and returns redacted metadata.
    ///
    /// Intended for trusted setup, composition, migration, or storage-code paths that are already
    /// allowed to manage secret material. This low-level primitive intentionally does not authorize
    /// arbitrary runtime/plugin callers.
    ///
    /// `expires_at` should be set for time-bounded secrets such as OAuth access tokens. Pass
    /// `None` for secrets without a known expiry (e.g. refresh tokens, API keys).
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError>;

    /// Atomically store a secret only when the scoped handle is absent.
    /// Returns `true` for the sole writer that created it and `false` when a
    /// value already exists. Intended for replica-safe generated credentials.
    async fn put_if_absent(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<Timestamp>,
    ) -> Result<bool, SecretStoreError>;

    /// Returns redacted metadata for a secret without exposing material.
    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError>;

    /// Lists redacted metadata for secrets under exactly the caller's owner scope.
    ///
    /// This intentionally exposes handles only, never material or leases. Backends
    /// should avoid per-handle point reads when they can enumerate the owner prefix
    /// directly.
    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError>;

    /// Deletes a scoped secret if it exists. Returns whether a stored secret was removed.
    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError>;

    /// Creates a one-shot lease for later secret consumption.
    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError>;

    /// Consumes an active one-shot lease and returns secret material exactly once.
    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError>;

    /// Revokes an active one-shot lease without returning material.
    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError>;

    /// Lists leases visible to the caller's tenant/user/project scope.
    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError>;
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironclaw_host_api::{
        action::NetworkMethod,
        ids::{
            CapabilityId, ExtensionId, InvocationId, MissionId, ProjectId, SecretHandle, TenantId,
            ThreadId, UserId,
        },
        resource::ResourceScope,
    };
    use serde_json::json;

    use crate::{
        CREDENTIAL_ID_MAX_LEN, CREDENTIAL_SESSION_DEFAULT_TTL_SECONDS,
        CREDENTIAL_SESSION_MAX_TTL_SECONDS, CredentialAccount, CredentialAccountId,
        CredentialAccountStatus, CredentialBrokerError, CredentialPathPolicy, CredentialSessionId,
        CredentialSessionRequest, CredentialTargetPolicy, InMemoryCredentialBroker, RedactedJson,
        SecretStoreError,
    };

    #[test]
    fn credential_account_id_validates_and_round_trips() {
        let id = CredentialAccountId::new("openai_prod-1").unwrap();
        assert_eq!(id.as_ref(), "openai_prod-1");
        assert_eq!(String::from(id.clone()), "openai_prod-1");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"openai_prod-1\"");
        assert_eq!(
            serde_json::from_str::<CredentialAccountId>("\"openai_prod-1\"").unwrap(),
            id
        );

        for invalid in ["", "a/b", "a b", "a.b"] {
            assert!(CredentialAccountId::new(invalid).is_err());
        }
        assert!(CredentialAccountId::new("a".repeat(CREDENTIAL_ID_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn credential_target_policy_matches_scheme_host_port_path_and_method() {
        let policy = CredentialTargetPolicy {
            scheme: "https".to_string(),
            host: "api.example.com".to_string(),
            port: Some(443),
            path: CredentialPathPolicy::Prefix("/v1/".to_string()),
            methods: vec![NetworkMethod::Get],
        };

        assert!(policy.matches(&NetworkMethod::Get, "https://api.example.com/v1/models"));
        assert!(policy.matches(&NetworkMethod::Get, "https://api.example.com:443/v1/models"));
        assert!(!policy.matches(
            &NetworkMethod::Get,
            "https://api.example.com:8443/v1/models"
        ));
        assert!(!policy.matches(&NetworkMethod::Post, "https://api.example.com/v1/models"));
        assert!(!policy.matches(&NetworkMethod::Get, "https://api.example.com/v2/models"));
        assert!(!policy.matches(
            &NetworkMethod::Get,
            "https://api.example.com/v1-evil/models"
        ));
        assert!(!policy.matches(&NetworkMethod::Get, "http://api.example.com/v1/models"));
        assert!(!policy.matches(&NetworkMethod::Get, "https://evil.example.com/v1/models"));
        assert!(!policy.matches(
            &NetworkMethod::Get,
            "https://user:pass@api.example.com/v1/models"
        ));
        assert!(!policy.matches(
            &NetworkMethod::Get,
            "https://api.example.com/v1/%2e%2e%2fadmin"
        ));
        assert!(!policy.matches(
            &NetworkMethod::Get,
            "https://api.example.com/v1/%252e%252e%252fadmin"
        ));
        assert!(!policy.matches(&NetworkMethod::Get, "https://api.example.com/v1/%2e/admin"));

        let policy_without_port_constraint = CredentialTargetPolicy {
            port: None,
            ..policy
        };
        assert!(
            policy_without_port_constraint
                .matches(&NetworkMethod::Get, "https://api.example.com/v1/models")
        );
        assert!(policy_without_port_constraint.matches(
            &NetworkMethod::Get,
            "https://api.example.com:8443/v1/models"
        ));
    }

    #[test]
    fn credential_account_debug_redacts_metadata() {
        let account = sample_account(
            sample_scope("tenant-a", "user-a"),
            CredentialAccountId::new("openai_prod").unwrap(),
            SecretHandle::new("openai_key").unwrap(),
        );
        let debug = format!("{account:?}");
        assert!(!debug.contains("refresh_token"));
        assert!(!debug.contains("sk-live-sentinel"));
        assert!(debug.contains("[REDACTED_JSON]"));
    }

    #[test]
    fn credential_session_id_display_redacts_bearer_like_value() {
        let raw_session_id = "3f2f4a08-f8ef-4d83-a8f6-624d77cf9181";
        let session_id = CredentialSessionId::parse(raw_session_id).unwrap();
        let display = session_id.to_string();

        assert!(!display.contains(raw_session_id));
        assert!(display.contains("[REDACTED]"));
    }

    #[test]
    fn credential_broker_session_error_displays_redact_session_id() {
        let raw_session_id = "3f2f4a08-f8ef-4d83-a8f6-624d77cf9181";
        let session_id = CredentialSessionId::parse(raw_session_id).unwrap();

        for (error, stable_reason) in [
            (
                CredentialBrokerError::UnknownSession { session_id },
                "MissingCredential",
            ),
            (
                CredentialBrokerError::SessionExpired { session_id },
                "CredentialExpired",
            ),
            (
                CredentialBrokerError::SessionUseLimitExceeded { session_id },
                "CredentialExpired",
            ),
        ] {
            let display = error.to_string();

            assert!(!display.contains(raw_session_id), "{display}");
            assert!(display.contains("[REDACTED]"), "{display}");
            assert_eq!(error.stable_reason(), stable_reason);
        }
    }

    #[test]
    fn stable_reason_tokens_are_locked() {
        let account_id = CredentialAccountId::new("openai_prod").unwrap();
        assert_eq!(
            CredentialBrokerError::BrokerUnavailable {
                reason: "poisoned".to_string()
            }
            .stable_reason(),
            "BackendUnavailable"
        );
        assert_eq!(
            CredentialBrokerError::CredentialExpired { account_id }.stable_reason(),
            "CredentialExpired"
        );
        assert_eq!(
            SecretStoreError::SecretExpired.stable_reason(),
            "CredentialExpired"
        );
        assert_eq!(
            SecretStoreError::BackendMisconfigured {
                reason: "missing key".to_string()
            }
            .stable_reason(),
            "BackendMisconfigured"
        );
        assert_eq!(
            CredentialBrokerError::InvalidPlaceholderToken {
                reason: "must start with 'icsbx_'".to_string(),
            }
            .stable_reason(),
            "MissingCredential"
        );
    }

    #[test]
    fn credential_session_creation_requires_explicit_scoped_account_and_redacts_material() {
        let broker = InMemoryCredentialBroker::new();
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("openai_prod").unwrap();
        let secret_handle = SecretHandle::new("openai_key").unwrap();
        broker
            .put_account(sample_account(
                scope.clone(),
                account_id.clone(),
                secret_handle.clone(),
            ))
            .unwrap();

        let session = broker
            .create_session(CredentialSessionRequest {
                scope: scope.clone(),
                invocation_id: scope.invocation_id,
                capability_id: CapabilityId::new("openai.chat").unwrap(),
                extension_id: ExtensionId::new("openai").unwrap(),
                account_id,
                method: NetworkMethod::Get,
                url: "https://api.example.com/v1/models".to_string(),
                expires_at: None,
                max_uses: Some(1),
            })
            .unwrap();

        assert_eq!(session.scope(), &scope);
        assert_eq!(session.secret_handles(), &[secret_handle]);
        let debug = format!("{session:?}");
        assert!(!debug.contains("sk-live-sentinel"));
        assert!(!debug.contains("token"));
        // CredentialSessionId is bearer-like: the raw UUID (obtainable only via
        // to_private_storage_string) must never appear in Debug output. Display
        // is now redacted to "[REDACTED]" so a contains-on-Display check would
        // be tautologically true here; this assertion still catches a
        // regression that would leak the underlying UUID.
        assert!(
            !debug.contains(&session.correlation_id().to_private_storage_string()),
            "CredentialSession Debug must not include the raw correlation UUID"
        );
        assert!(debug.contains("CredentialSessionId([REDACTED])"));
    }

    #[test]
    fn create_session_defaults_and_caps_expires_at() {
        // A request with no opinion on expiry (`None`) must not come back
        // genuinely unbounded — it gets the default TTL. A request asking
        // for longer than the hard cap must be clamped down to the cap
        // rather than honored as requested. Both defend the same invariant:
        // a leaked/held-forever lease can never be a standing grant for the
        // process lifetime (see `create_session`'s doc comment).
        let broker = InMemoryCredentialBroker::new();
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("openai_prod").unwrap();
        broker
            .put_account(sample_account(
                scope.clone(),
                account_id.clone(),
                SecretHandle::new("openai_key").unwrap(),
            ))
            .unwrap();

        let before = Utc::now();
        let defaulted = broker
            .create_session(CredentialSessionRequest {
                expires_at: None,
                ..session_request(
                    scope.clone(),
                    account_id.clone(),
                    "https://api.example.com/v1/models",
                )
            })
            .unwrap();
        let after = Utc::now();
        let defaulted_expiry = defaulted
            .expires_at()
            .expect("None must be defaulted, not left unbounded");
        assert!(
            defaulted_expiry
                >= before + chrono::Duration::seconds(CREDENTIAL_SESSION_DEFAULT_TTL_SECONDS)
                && defaulted_expiry
                    <= after + chrono::Duration::seconds(CREDENTIAL_SESSION_DEFAULT_TTL_SECONDS),
            "expires_at: None must default to ~{CREDENTIAL_SESSION_DEFAULT_TTL_SECONDS}s out, got {defaulted_expiry}"
        );

        let requested_far_future = Utc::now() + chrono::Duration::days(365);
        let capped = broker
            .create_session(CredentialSessionRequest {
                expires_at: Some(requested_far_future),
                ..session_request(scope, account_id, "https://api.example.com/v1/models")
            })
            .unwrap();
        let capped_expiry = capped
            .expires_at()
            .expect("capped session must still carry an expiry");
        assert!(
            capped_expiry < requested_far_future,
            "a request for a 1-year expiry must be clamped down to the cap, got {capped_expiry}"
        );
        assert!(
            capped_expiry
                <= Utc::now() + chrono::Duration::seconds(CREDENTIAL_SESSION_MAX_TTL_SECONDS),
            "clamped expiry must not exceed the {CREDENTIAL_SESSION_MAX_TTL_SECONDS}s cap, got {capped_expiry}"
        );
    }

    #[test]
    fn credential_session_validation_enforces_expiry_and_use_limits() {
        let broker = InMemoryCredentialBroker::new();
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("openai_prod").unwrap();
        broker
            .put_account(sample_account(
                scope.clone(),
                account_id.clone(),
                SecretHandle::new("openai_key").unwrap(),
            ))
            .unwrap();
        let session = broker
            .create_session(CredentialSessionRequest {
                expires_at: Some(Utc::now() + chrono::Duration::seconds(60)),
                max_uses: Some(1),
                ..session_request(
                    scope.clone(),
                    account_id,
                    "https://api.example.com/v1/models",
                )
            })
            .unwrap();

        broker
            .validate_session(session.correlation_id(), Utc::now())
            .unwrap();
        broker
            .consume_session_use(session.correlation_id(), Utc::now())
            .unwrap();
        assert!(matches!(
            broker.consume_session_use(session.correlation_id(), Utc::now()),
            Err(CredentialBrokerError::SessionUseLimitExceeded { .. })
        ));

        let expired_id = CredentialAccountId::new("openai_expiring").unwrap();
        broker
            .put_account(sample_account(
                scope.clone(),
                expired_id.clone(),
                SecretHandle::new("openai_expiring_key").unwrap(),
            ))
            .unwrap();
        let expired = broker
            .create_session(CredentialSessionRequest {
                expires_at: Some(Utc::now() - chrono::Duration::seconds(1)),
                ..session_request(scope, expired_id, "https://api.example.com/v1/models")
            })
            .unwrap();
        assert!(matches!(
            broker.validate_session(expired.correlation_id(), Utc::now()),
            Err(CredentialBrokerError::SessionExpired { .. })
        ));
    }

    #[test]
    fn credential_session_creation_rejects_invocation_mismatch() {
        let broker = InMemoryCredentialBroker::new();
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("openai_prod").unwrap();
        broker
            .put_account(sample_account(
                scope.clone(),
                account_id.clone(),
                SecretHandle::new("openai_key").unwrap(),
            ))
            .unwrap();

        let result = broker.create_session(CredentialSessionRequest {
            invocation_id: InvocationId::new(),
            ..session_request(scope, account_id, "https://api.example.com/v1/models")
        });
        assert!(matches!(
            result,
            Err(CredentialBrokerError::CredentialInvocationMismatch { .. })
        ));
    }

    #[test]
    fn credential_session_creation_accepts_project_scoped_account_across_invocations() {
        let broker = InMemoryCredentialBroker::new();
        let account_scope = sample_scope("tenant-a", "user-a");
        let request_scope = ResourceScope {
            mission_id: Some(MissionId::new("mission-b").unwrap()),
            thread_id: Some(ThreadId::new("thread-b").unwrap()),
            invocation_id: InvocationId::new(),
            ..account_scope.clone()
        };
        let account_id = CredentialAccountId::new("openai_prod").unwrap();
        let secret_handle = SecretHandle::new("openai_key").unwrap();
        broker
            .put_account(sample_account(
                account_scope,
                account_id.clone(),
                secret_handle.clone(),
            ))
            .unwrap();

        let session = broker
            .create_session(session_request(
                request_scope.clone(),
                account_id,
                "https://api.example.com/v1/models",
            ))
            .unwrap();

        assert_eq!(session.scope(), &request_scope);
        assert_eq!(session.secret_handles(), &[secret_handle]);
    }

    #[test]
    fn credential_session_creation_denies_missing_cross_scope_revoked_and_policy_mismatch() {
        let broker = InMemoryCredentialBroker::new();
        let scope = sample_scope("tenant-a", "user-a");
        let other_scope = sample_scope("tenant-b", "user-b");
        let account_id = CredentialAccountId::new("github_prod").unwrap();
        let secret_handle = SecretHandle::new("github_key").unwrap();
        broker
            .put_account(sample_account(
                scope.clone(),
                account_id.clone(),
                secret_handle,
            ))
            .unwrap();

        let missing = broker.create_session(session_request(
            scope.clone(),
            CredentialAccountId::new("missing").unwrap(),
            "https://api.example.com/v1/models",
        ));
        assert!(matches!(
            missing,
            Err(CredentialBrokerError::MissingCredential { .. })
        ));

        let cross_scope = broker.create_session(session_request(
            other_scope,
            account_id.clone(),
            "https://api.example.com/v1/models",
        ));
        assert!(matches!(
            cross_scope,
            Err(CredentialBrokerError::MissingCredential { .. })
        ));

        let policy_mismatch = broker.create_session(session_request(
            scope.clone(),
            account_id.clone(),
            "https://api.example.com/v2/models",
        ));
        assert!(matches!(
            policy_mismatch,
            Err(CredentialBrokerError::CredentialPolicyMismatch { .. })
        ));

        let extension_mismatch = broker.create_session(CredentialSessionRequest {
            extension_id: ExtensionId::new("other_extension").unwrap(),
            ..session_request(
                scope.clone(),
                account_id.clone(),
                "https://api.example.com/v1/models",
            )
        });
        assert!(matches!(
            extension_mismatch,
            Err(CredentialBrokerError::CredentialExtensionMismatch { .. })
        ));

        let expired_id = CredentialAccountId::new("github_expired").unwrap();
        let mut expired = sample_account(
            scope.clone(),
            expired_id.clone(),
            SecretHandle::new("github_expired_key").unwrap(),
        );
        expired.status = CredentialAccountStatus::Expired;
        broker.put_account(expired).unwrap();
        let expired_result = broker.create_session(session_request(
            scope.clone(),
            expired_id,
            "https://api.example.com/v1/models",
        ));
        assert!(matches!(
            expired_result,
            Err(CredentialBrokerError::CredentialExpired { .. })
        ));

        let revoked_id = CredentialAccountId::new("github_revoked").unwrap();
        let mut revoked = sample_account(
            scope.clone(),
            revoked_id.clone(),
            SecretHandle::new("github_revoked_key").unwrap(),
        );
        revoked.status = CredentialAccountStatus::Revoked;
        broker.put_account(revoked).unwrap();
        let revoked_result = broker.create_session(session_request(
            scope,
            revoked_id,
            "https://api.example.com/v1/models",
        ));
        assert!(matches!(
            revoked_result,
            Err(CredentialBrokerError::CredentialRevoked { .. })
        ));
    }

    fn sample_account(
        scope: ResourceScope,
        id: CredentialAccountId,
        secret_handle: SecretHandle,
    ) -> CredentialAccount {
        CredentialAccount {
            scope,
            id,
            provider_or_extension_id: ExtensionId::new("openai").unwrap(),
            label: "Production".to_string(),
            status: CredentialAccountStatus::Active,
            secret_handles: vec![secret_handle],
            allowed_targets: vec![CredentialTargetPolicy {
                scheme: "https".to_string(),
                host: "api.example.com".to_string(),
                port: Some(443),
                path: CredentialPathPolicy::Prefix("/v1/".to_string()),
                methods: vec![NetworkMethod::Get],
            }],
            redacted_metadata: RedactedJson::new(json!({
                "last_four": "1234",
                "refresh_token": "sk-live-sentinel"
            })),
            updated_at: Utc::now(),
        }
    }

    fn session_request(
        scope: ResourceScope,
        account_id: CredentialAccountId,
        url: &str,
    ) -> CredentialSessionRequest {
        CredentialSessionRequest {
            invocation_id: scope.invocation_id,
            scope,
            capability_id: CapabilityId::new("openai.chat").unwrap(),
            extension_id: ExtensionId::new("openai").unwrap(),
            account_id,
            method: NetworkMethod::Get,
            url: url.to_string(),
            expires_at: None,
            max_uses: Some(1),
        }
    }

    fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("project-a").unwrap()),
            mission_id: Some(MissionId::new("mission-a").unwrap()),
            thread_id: Some(ThreadId::new("thread-a").unwrap()),
            invocation_id: InvocationId::new(),
        }
    }
}
