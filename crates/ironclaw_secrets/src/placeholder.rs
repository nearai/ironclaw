//! Stable credential placeholder registry and JIT session wiring.
//!
//! The container running a sandboxed invocation never sees real secret
//! material. Instead it is handed a **placeholder token** — an inert string
//! that identifies "the credential for this tenant/user/provider" without
//! granting anything on its own. The egress proxy (W6-EGRESS-PROXY, not built
//! yet) swaps the placeholder for a live [`CredentialSession`] at request
//! time, host side only.
//!
//! This module owns two host-side responsibilities:
//!
//! 1. [`CredentialPlaceholderRegistry`] — mints and remembers one stable
//!    placeholder per `(tenant, user, provider)`, and resolves a placeholder
//!    back to its owner. The registry never grants access; it is pure
//!    identity bookkeeping, kept deliberately separate from
//!    [`InMemoryCredentialBroker`]'s session store so that holding a
//!    placeholder can never be conflated with holding a session.
//! 2. [`CredentialSessionLease`] — the JIT (just-in-time) minting handle
//!    returned by [`InMemoryCredentialBroker::mint_on_first_use`]. A session
//!    is minted only when a binding is actually used, and the lease
//!    guarantees the session is revoked when the lease is dropped — success,
//!    error, timeout (future dropped by `tokio::time::timeout`), or panic
//!    (drop still runs during unwind) — so a missed revoke path can never
//!    leave a standing grant.
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ironclaw_host_api::{CapabilityId, ExtensionId, InvocationId, TenantId, UserId};
use uuid::Uuid;

use crate::{
    CredentialAccountId, CredentialBrokerError, CredentialSessionId, CredentialSessionRequest,
    InMemoryCredentialBroker,
};

/// Fixed prefix for every placeholder token.
///
/// The `sandbox_credential_placeholder` leak-detector pattern in
/// `ironclaw_safety::leak_detector` (owned there, not here — see that
/// crate's `default_patterns()`) independently recognizes this same prefix,
/// so a placeholder that somehow escapes the container is flagged the same
/// way a real secret would be, even though, unlike a real secret, holding one
/// grants nothing. `ironclaw_safety` deliberately does not depend on this
/// crate (it stays a dependency-light substrate), so the two patterns are
/// pinned together by a regression test instead of a shared dependency: see
/// `sandbox_credential_placeholder_prefix_matches_registry` in
/// `ironclaw_safety/src/leak_detector.rs`. If this prefix ever changes,
/// update the regex there too.
pub const CREDENTIAL_PLACEHOLDER_PREFIX: &str = "icsbx_";

/// Opaque, stable placeholder token for a `(tenant, user, provider)` triple.
///
/// Deliberately **not** bearer-like: unlike [`CredentialSessionId`], a
/// placeholder is inert on its own (`Display` shows it in full, and that is
/// intentional — it is designed to sit in a container's environment/config
/// where a human or log line may see it, because it can't be used to reach a
/// real credential without a live session behind it).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CredentialPlaceholderToken(String);

impl CredentialPlaceholderToken {
    fn generate() -> Self {
        Self(format!(
            "{CREDENTIAL_PLACEHOLDER_PREFIX}{}",
            Uuid::new_v4().simple()
        ))
    }

    fn validate(value: &str) -> Result<(), CredentialBrokerError> {
        if !value.starts_with(CREDENTIAL_PLACEHOLDER_PREFIX) {
            return Err(CredentialBrokerError::InvalidPlaceholderToken {
                value: value.to_string(),
                reason: format!("must start with '{CREDENTIAL_PLACEHOLDER_PREFIX}'"),
            });
        }
        Ok(())
    }

    /// Parses a placeholder token received from the sandbox side (e.g. off an
    /// outbound request), rejecting anything that does not carry the fixed
    /// prefix.
    pub fn parse(value: impl Into<String>) -> Result<Self, CredentialBrokerError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for CredentialPlaceholderToken {
    type Error = CredentialBrokerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl AsRef<str> for CredentialPlaceholderToken {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CredentialPlaceholderToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CredentialPlaceholderToken")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for CredentialPlaceholderToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The `(tenant, user, provider)` triple a placeholder token identifies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialPlaceholderOwner {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub provider_or_extension_id: ExtensionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CredentialPlaceholderOwnerKey {
    tenant_id: TenantId,
    user_id: UserId,
    provider_or_extension_id: ExtensionId,
}

/// Host-side registry mapping `(tenant, user, provider)` to a stable
/// placeholder token, and back.
///
/// Stability is structural, not incidental: a container recycle/removal
/// touches nothing here (the registry lives at process/host lifetime, not
/// container lifetime), so the same triple always yields the same token.
/// Holding a registry-issued token grants nothing — see
/// [`InMemoryCredentialBroker::mint_on_first_use`] for the piece that does.
#[derive(Debug, Default)]
pub struct CredentialPlaceholderRegistry {
    by_owner: Mutex<HashMap<CredentialPlaceholderOwnerKey, CredentialPlaceholderToken>>,
    by_token: Mutex<HashMap<CredentialPlaceholderToken, CredentialPlaceholderOwner>>,
}

impl CredentialPlaceholderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the stable placeholder for `(tenant, user, provider)`, seeding
    /// one on first request. Idempotent: repeated calls for the same triple
    /// — including calls made after a simulated container recycle, since
    /// nothing here is tied to a container's lifetime — return the same
    /// token.
    pub fn get_or_create(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        provider_id: &ExtensionId,
    ) -> Result<CredentialPlaceholderToken, CredentialBrokerError> {
        let key = CredentialPlaceholderOwnerKey {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
            provider_or_extension_id: provider_id.clone(),
        };
        let mut by_owner =
            self.by_owner
                .lock()
                .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                    reason: error.to_string(),
                })?;
        if let Some(existing) = by_owner.get(&key) {
            return Ok(existing.clone());
        }
        let token = CredentialPlaceholderToken::generate();
        by_owner.insert(key, token.clone());
        drop(by_owner);
        self.by_token
            .lock()
            .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                reason: error.to_string(),
            })?
            .insert(
                token.clone(),
                CredentialPlaceholderOwner {
                    tenant_id: tenant_id.clone(),
                    user_id: user_id.clone(),
                    provider_or_extension_id: provider_id.clone(),
                },
            );
        Ok(token)
    }

    /// Reverse lookup used by the egress proxy: given a placeholder token,
    /// find the `(tenant, user, provider)` it identifies.
    ///
    /// Backed by an exact-match `HashMap` (no prefix/substring scan), so this
    /// is O(1) and structurally cannot cross-match another user's token.
    pub fn resolve(
        &self,
        token: &CredentialPlaceholderToken,
    ) -> Result<Option<CredentialPlaceholderOwner>, CredentialBrokerError> {
        Ok(self
            .by_token
            .lock()
            .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                reason: error.to_string(),
            })?
            .get(token)
            .cloned())
    }
}

/// Identity of a `(invocation, binding)` pair used to key JIT session
/// minting, so re-use within the same dispatch does not mint a second
/// session for the same binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct JitMintKey {
    invocation_id: InvocationId,
    capability_id: CapabilityId,
    account_id: CredentialAccountId,
}

/// RAII handle for a JIT-minted [`CredentialSession`](crate::CredentialSession).
///
/// The session behind this lease is revoked exactly once, no matter how the
/// lease is dropped:
/// - **success** / **error**: call [`CredentialSessionLease::revoke`] explicitly.
/// - **timeout**: if the future holding the lease is dropped by
///   `tokio::time::timeout` (or any other cancellation), `Drop` revokes it.
/// - **panic**: `Drop` still runs during unwind, so a panic mid-dispatch
///   revokes it too.
///
/// A missed revoke path would leave a standing grant, so this type has no
/// safe way to leak the session past its own lifetime: there is no `mem::forget`-safe
/// accessor, and the only way to keep the session alive is to hold the lease.
pub struct CredentialSessionLease {
    broker: Arc<InMemoryCredentialBroker>,
    session_id: CredentialSessionId,
    revoked: bool,
}

impl CredentialSessionLease {
    /// The id of the session this lease guards. Read-only: it cannot be used
    /// to extend the session's life past the lease being dropped.
    pub fn session_id(&self) -> CredentialSessionId {
        self.session_id
    }

    /// Explicitly revokes the session now. Intended for the success and error
    /// exit paths of a dispatch, where the caller can revoke synchronously
    /// rather than waiting for `Drop`.
    pub fn revoke(mut self) {
        self.revoke_inner();
    }

    fn revoke_inner(&mut self) {
        if !self.revoked {
            self.revoked = true;
            self.broker.release_lease(self.session_id);
        }
    }
}

impl Drop for CredentialSessionLease {
    fn drop(&mut self) {
        self.revoke_inner();
    }
}

impl InMemoryCredentialBroker {
    /// Mints a [`CredentialSession`](crate::CredentialSession) the first time
    /// a `(invocation, binding)` pair is actually used, associates it with
    /// `placeholder` so the proxy can find it by placeholder, and returns a
    /// lease that revokes the session on drop.
    ///
    /// If the same `(invocation, capability, account)` binding is minted
    /// again while its previous session is still live, the existing session
    /// is reused rather than minting a second one — staging every possible
    /// binding up front would itself be a standing grant, so callers must
    /// only call this at actual first use. Because that reuse can hand out
    /// more than one [`CredentialSessionLease`] for the identical session,
    /// each lease only *releases* its own reference on drop/`revoke()`; the
    /// session itself is revoked once the last outstanding lease releases it
    /// (see [`InMemoryCredentialBroker::release_lease`]), so an earlier
    /// caller finishing first can never pull the session out from under a
    /// later caller still using it.
    pub fn mint_on_first_use(
        self: &Arc<Self>,
        placeholder: &CredentialPlaceholderToken,
        request: CredentialSessionRequest,
    ) -> Result<CredentialSessionLease, CredentialBrokerError> {
        let key = JitMintKey {
            invocation_id: request.invocation_id,
            capability_id: request.capability_id.clone(),
            account_id: request.account_id.clone(),
        };

        if let Some(session_id) = self.jit_minted_session_id(&key)?
            && self
                .validate_session(session_id, chrono::Utc::now())
                .is_ok()
        {
            self.bind_placeholder_to_session(placeholder.clone(), session_id)?;
            self.acquire_lease_refcount(session_id)?;
            return Ok(CredentialSessionLease {
                broker: self.clone(),
                session_id,
                revoked: false,
            });
        }

        let session = self.create_session(request)?;
        let session_id = session.correlation_id();
        self.record_jit_mint(key, session_id)?;
        self.bind_placeholder_to_session(placeholder.clone(), session_id)?;
        self.acquire_lease_refcount(session_id)?;
        Ok(CredentialSessionLease {
            broker: self.clone(),
            session_id,
            revoked: false,
        })
    }

    /// Finds a live session currently bound to `placeholder`, if any.
    ///
    /// Returns `Ok(None)` — never an error — when the placeholder has no
    /// live session: an unminted, expired, use-exhausted, or already-revoked
    /// binding all "grant nothing" the same way. Every candidate session
    /// bound to the placeholder is additionally checked against `scope` so a
    /// placeholder can never resolve to a session outside the caller's own
    /// tenant/user/invocation scope.
    ///
    /// More than one live session can be bound to the same placeholder at
    /// once (e.g. two accounts under one provider, or two overlapping
    /// invocations); this returns the first one whose scope matches. Picking
    /// the *correct* one by request target when several match is the egress
    /// proxy's job (not built yet), so a caller that needs to disambiguate
    /// further must do so itself once it has the candidate.
    pub fn find_session_by_placeholder(
        &self,
        placeholder: &CredentialPlaceholderToken,
        scope: &ironclaw_host_api::ResourceScope,
    ) -> Result<Option<crate::CredentialSession>, CredentialBrokerError> {
        for session_id in self.placeholder_session_ids(placeholder)? {
            match self.validate_session(session_id, chrono::Utc::now()) {
                Ok(session) if session.scope() == scope => return Ok(Some(session)),
                Ok(_) => continue,
                Err(CredentialBrokerError::UnknownSession { .. }) => continue,
                Err(CredentialBrokerError::SessionExpired { .. }) => continue,
                Err(CredentialBrokerError::SessionUseLimitExceeded { .. }) => continue,
                Err(other) => return Err(other),
            }
        }
        Ok(None)
    }

    /// Explicitly revokes a session, e.g. from
    /// [`CredentialSessionLease`]'s success/error/drop paths (by way of
    /// [`InMemoryCredentialBroker::release_lease`]). Idempotent: revoking an
    /// already-unknown session is a no-op, since "no session" and "revoked
    /// session" both mean the placeholder grants nothing.
    ///
    /// Also prunes this session out of the `jit_minted` and
    /// `sessions_by_placeholder` secondary indices so a long-lived process
    /// does not accumulate stale entries for every invocation/binding that
    /// has ever existed.
    pub fn revoke_session(&self, session_id: CredentialSessionId) {
        lock_or_recover(&self.sessions).remove(&session_id);
        lock_or_recover(&self.jit_minted)
            .retain(|_, minted_session_id| *minted_session_id != session_id);
        lock_or_recover(&self.sessions_by_placeholder).retain(|_, session_ids| {
            session_ids.remove(&session_id);
            !session_ids.is_empty()
        });
        lock_or_recover(&self.lease_refcounts).remove(&session_id);
    }

    /// Registers one outstanding lease reference for `session_id`. See
    /// [`InMemoryCredentialBroker::release_lease`] for the matching release
    /// half of this pair.
    fn acquire_lease_refcount(
        &self,
        session_id: CredentialSessionId,
    ) -> Result<(), CredentialBrokerError> {
        *lock_or_recover(&self.lease_refcounts)
            .entry(session_id)
            .or_insert(0) += 1;
        Ok(())
    }

    /// Releases one outstanding lease reference for `session_id`, revoking
    /// the underlying session only once the last outstanding reference is
    /// released. See [`CredentialSessionLease::revoke_inner`] — every path
    /// that drops or explicitly revokes a lease calls this instead of
    /// `revoke_session` directly, so concurrent callers reusing the same
    /// JIT-minted session (`mint_on_first_use`'s cache-hit path) cannot have
    /// their still-live session revoked out from under them by an earlier
    /// caller finishing first.
    pub(crate) fn release_lease(&self, session_id: CredentialSessionId) {
        let should_revoke = {
            let mut counts = lock_or_recover(&self.lease_refcounts);
            match counts.get_mut(&session_id) {
                Some(count) if *count > 1 => {
                    *count -= 1;
                    false
                }
                _ => {
                    counts.remove(&session_id);
                    true
                }
            }
        };
        if should_revoke {
            self.revoke_session(session_id);
        }
    }

    fn jit_minted_session_id(
        &self,
        key: &JitMintKey,
    ) -> Result<Option<CredentialSessionId>, CredentialBrokerError> {
        Ok(self
            .jit_minted
            .lock()
            .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                reason: error.to_string(),
            })?
            .get(key)
            .copied())
    }

    fn record_jit_mint(
        &self,
        key: JitMintKey,
        session_id: CredentialSessionId,
    ) -> Result<(), CredentialBrokerError> {
        self.jit_minted
            .lock()
            .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                reason: error.to_string(),
            })?
            .insert(key, session_id);
        Ok(())
    }

    fn bind_placeholder_to_session(
        &self,
        placeholder: CredentialPlaceholderToken,
        session_id: CredentialSessionId,
    ) -> Result<(), CredentialBrokerError> {
        self.sessions_by_placeholder
            .lock()
            .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                reason: error.to_string(),
            })?
            .entry(placeholder)
            .or_default()
            .insert(session_id);
        Ok(())
    }

    fn placeholder_session_ids(
        &self,
        placeholder: &CredentialPlaceholderToken,
    ) -> Result<Vec<CredentialSessionId>, CredentialBrokerError> {
        Ok(self
            .sessions_by_placeholder
            .lock()
            .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                reason: error.to_string(),
            })?
            .get(placeholder)
            .map(|session_ids| session_ids.iter().copied().collect())
            .unwrap_or_default())
    }
}

/// Locks `mutex`, recovering the inner guard on poison rather than silently
/// no-op'ing. A poisoned lock here must never be treated as "nothing to
/// clean up" — that would let a revoke path fail open and leave a standing
/// grant, contradicting this module's core invariant.
fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use ironclaw_host_api::{
        CapabilityId, ExtensionId, InvocationId, NetworkMethod, ProjectId, ResourceScope,
        SecretHandle, TenantId, UserId,
    };

    use crate::{
        CredentialAccount, CredentialAccountId, CredentialAccountStatus, CredentialBrokerError,
        CredentialPathPolicy, CredentialSessionRequest, CredentialTargetPolicy,
        InMemoryCredentialBroker, RedactedJson,
    };

    use super::{CredentialPlaceholderRegistry, CredentialPlaceholderToken};

    fn scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("project-a").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn account(
        scope: ResourceScope,
        id: CredentialAccountId,
        handle: SecretHandle,
    ) -> CredentialAccount {
        CredentialAccount {
            scope,
            id,
            provider_or_extension_id: ExtensionId::new("google").unwrap(),
            label: "Prod".to_string(),
            status: CredentialAccountStatus::Active,
            secret_handles: vec![handle],
            allowed_targets: vec![CredentialTargetPolicy {
                scheme: "https".to_string(),
                host: "api.example.com".to_string(),
                port: Some(443),
                path: CredentialPathPolicy::Prefix("/v1/".to_string()),
                methods: vec![NetworkMethod::Get],
            }],
            redacted_metadata: RedactedJson::new(serde_json::json!({})),
            updated_at: Utc::now(),
        }
    }

    fn request(
        scope: ResourceScope,
        account_id: CredentialAccountId,
        url: &str,
    ) -> CredentialSessionRequest {
        CredentialSessionRequest {
            invocation_id: scope.invocation_id,
            scope,
            capability_id: CapabilityId::new("google.drive").unwrap(),
            extension_id: ExtensionId::new("google").unwrap(),
            account_id,
            method: NetworkMethod::Get,
            url: url.to_string(),
            expires_at: None,
            max_uses: None,
        }
    }

    #[test]
    fn placeholder_is_stable_across_repeated_lookups_and_recycle() {
        let registry = CredentialPlaceholderRegistry::new();
        let tenant = TenantId::new("tenant-a").unwrap();
        let user = UserId::new("user-a").unwrap();
        let provider = ExtensionId::new("google").unwrap();

        let first = registry.get_or_create(&tenant, &user, &provider).unwrap();
        let second = registry.get_or_create(&tenant, &user, &provider).unwrap();
        assert_eq!(first, second);

        // Simulate a container recycle: nothing about the registry is tied to
        // a container, so a "new container" asking for the same triple again
        // gets the identical token.
        drop(first);
        let after_recycle = registry.get_or_create(&tenant, &user, &provider).unwrap();
        assert_eq!(second, after_recycle);
    }

    #[test]
    fn placeholders_are_distinct_and_isolated_per_user() {
        let registry = CredentialPlaceholderRegistry::new();
        let tenant = TenantId::new("tenant-a").unwrap();
        let provider = ExtensionId::new("google").unwrap();
        let user_a = UserId::new("user-a").unwrap();
        let user_b = UserId::new("user-b").unwrap();

        let token_a = registry.get_or_create(&tenant, &user_a, &provider).unwrap();
        let token_b = registry.get_or_create(&tenant, &user_b, &provider).unwrap();
        assert_ne!(token_a, token_b);

        let owner_a = registry.resolve(&token_a).unwrap().unwrap();
        let owner_b = registry.resolve(&token_b).unwrap().unwrap();
        assert_eq!(owner_a.user_id, user_a);
        assert_eq!(owner_b.user_id, user_b);
        assert_ne!(owner_a.user_id, owner_b.user_id);

        // User A's placeholder must never resolve to user B's owner triple.
        assert_ne!(
            registry.resolve(&token_a).unwrap(),
            registry.resolve(&token_b).unwrap()
        );
    }

    #[test]
    fn placeholder_token_requires_fixed_prefix() {
        assert!(CredentialPlaceholderToken::parse("icsbx_abc").is_ok());
        let err = CredentialPlaceholderToken::parse("not_a_placeholder").unwrap_err();
        assert!(matches!(
            err,
            CredentialBrokerError::InvalidPlaceholderToken { .. }
        ));
    }

    #[test]
    fn placeholder_with_no_live_session_grants_nothing() {
        let broker = Arc::new(InMemoryCredentialBroker::new());
        let registry = CredentialPlaceholderRegistry::new();
        let tenant = TenantId::new("tenant-a").unwrap();
        let user = UserId::new("user-a").unwrap();
        let provider = ExtensionId::new("google").unwrap();
        let token = registry.get_or_create(&tenant, &user, &provider).unwrap();

        let found = broker
            .find_session_by_placeholder(&token, &scope("tenant-a", "user-a"))
            .unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn jit_mint_binds_session_to_placeholder_and_enforces_target_policy() {
        let broker = Arc::new(InMemoryCredentialBroker::new());
        let registry = CredentialPlaceholderRegistry::new();
        let tenant = TenantId::new("tenant-a").unwrap();
        let user = UserId::new("user-a").unwrap();
        let provider = ExtensionId::new("google").unwrap();
        let token = registry.get_or_create(&tenant, &user, &provider).unwrap();

        let caller_scope = scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("google_prod").unwrap();
        broker
            .put_account(account(
                caller_scope.clone(),
                account_id.clone(),
                SecretHandle::new("google_key").unwrap(),
            ))
            .unwrap();

        // Out-of-policy request (wrong path) must be rejected, not minted.
        let rejected = broker.mint_on_first_use(
            &token,
            request(
                caller_scope.clone(),
                account_id.clone(),
                "https://api.example.com/v2/x",
            ),
        );
        assert!(matches!(
            rejected,
            Err(CredentialBrokerError::CredentialPolicyMismatch { .. })
        ));
        assert!(
            broker
                .find_session_by_placeholder(&token, &caller_scope)
                .unwrap()
                .is_none()
        );

        // In-policy request mints and binds.
        let lease = broker
            .mint_on_first_use(
                &token,
                request(
                    caller_scope.clone(),
                    account_id,
                    "https://api.example.com/v1/x",
                ),
            )
            .unwrap();
        let bound = broker
            .find_session_by_placeholder(&token, &caller_scope)
            .unwrap()
            .expect("session bound to placeholder after JIT mint");
        assert_eq!(bound.correlation_id(), lease.session_id());
        lease.revoke();
    }

    #[test]
    fn placeholder_session_lookup_does_not_cross_user_scope() {
        let broker = Arc::new(InMemoryCredentialBroker::new());
        let registry = CredentialPlaceholderRegistry::new();
        let tenant = TenantId::new("tenant-a").unwrap();
        let provider = ExtensionId::new("google").unwrap();
        let user_a = UserId::new("user-a").unwrap();
        let token_a = registry.get_or_create(&tenant, &user_a, &provider).unwrap();

        let scope_a = scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("google_prod").unwrap();
        broker
            .put_account(account(
                scope_a.clone(),
                account_id.clone(),
                SecretHandle::new("google_key").unwrap(),
            ))
            .unwrap();
        let lease = broker
            .mint_on_first_use(
                &token_a,
                request(scope_a, account_id, "https://api.example.com/v1/x"),
            )
            .unwrap();

        // User B's scope must never see user A's session through the placeholder.
        let other_scope = scope("tenant-a", "user-b");
        let found = broker
            .find_session_by_placeholder(&token_a, &other_scope)
            .unwrap();
        assert!(found.is_none());
        lease.revoke();
    }

    #[test]
    fn lease_revokes_on_explicit_success_call() {
        let broker = Arc::new(InMemoryCredentialBroker::new());
        let registry = CredentialPlaceholderRegistry::new();
        let (token, scope_a, account_id) = seeded(&broker, &registry, "user-success");

        let lease = broker
            .mint_on_first_use(
                &token,
                request(scope_a, account_id, "https://api.example.com/v1/x"),
            )
            .unwrap();
        let session_id = lease.session_id();
        lease.revoke(); // success path calls this explicitly
        assert!(matches!(
            broker.validate_session(session_id, Utc::now()),
            Err(CredentialBrokerError::UnknownSession { .. })
        ));
    }

    #[test]
    fn lease_revokes_on_explicit_error_call() {
        let broker = Arc::new(InMemoryCredentialBroker::new());
        let registry = CredentialPlaceholderRegistry::new();
        let (token, scope_a, account_id) = seeded(&broker, &registry, "user-error");

        let lease = broker
            .mint_on_first_use(
                &token,
                request(scope_a, account_id, "https://api.example.com/v1/x"),
            )
            .unwrap();
        let session_id = lease.session_id();

        let dispatch_result: Result<(), &'static str> = Err("upstream 500");
        if dispatch_result.is_err() {
            lease.revoke(); // error path calls this explicitly too
        }
        assert!(matches!(
            broker.validate_session(session_id, Utc::now()),
            Err(CredentialBrokerError::UnknownSession { .. })
        ));
    }

    #[tokio::test]
    async fn lease_revokes_on_timeout_via_drop() {
        let broker = Arc::new(InMemoryCredentialBroker::new());
        let registry = CredentialPlaceholderRegistry::new();
        let (token, scope_a, account_id) = seeded(&broker, &registry, "user-timeout");

        let lease = broker
            .mint_on_first_use(
                &token,
                request(scope_a, account_id, "https://api.example.com/v1/x"),
            )
            .unwrap();
        let session_id = lease.session_id();

        let outcome = tokio::time::timeout(Duration::from_millis(5), async move {
            let _lease = lease; // held across the (never-completing) sleep
            tokio::time::sleep(Duration::from_secs(60)).await;
        })
        .await;

        assert!(outcome.is_err(), "expected the dispatch to time out");
        assert!(matches!(
            broker.validate_session(session_id, Utc::now()),
            Err(CredentialBrokerError::UnknownSession { .. })
        ));
    }

    #[test]
    fn lease_revokes_on_panic_via_drop() {
        let broker = Arc::new(InMemoryCredentialBroker::new());
        let registry = CredentialPlaceholderRegistry::new();
        let (token, scope_a, account_id) = seeded(&broker, &registry, "user-panic");

        let lease = broker
            .mint_on_first_use(
                &token,
                request(scope_a, account_id, "https://api.example.com/v1/x"),
            )
            .unwrap();
        let session_id = lease.session_id();

        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _lease = lease; // dropped during unwind
            panic!("simulated panic mid-dispatch");
        }));

        assert!(result.is_err());
        assert!(matches!(
            broker.validate_session(session_id, Utc::now()),
            Err(CredentialBrokerError::UnknownSession { .. })
        ));
    }

    #[test]
    fn reused_session_survives_the_first_of_two_leases_revoking() {
        // Two mint_on_first_use calls for the identical (invocation,
        // capability, account) binding — the documented cache-hit reuse path
        // — must hand out two independent leases over the *same* session.
        // Revoking the first-acquired lease must not revoke the session out
        // from under the second lease still holding it; only releasing both
        // should actually revoke it.
        let broker = Arc::new(InMemoryCredentialBroker::new());
        let registry = CredentialPlaceholderRegistry::new();
        let (token, scope_a, account_id) = seeded(&broker, &registry, "user-refcount");

        let first_lease = broker
            .mint_on_first_use(
                &token,
                request(
                    scope_a.clone(),
                    account_id.clone(),
                    "https://api.example.com/v1/x",
                ),
            )
            .unwrap();
        let second_lease = broker
            .mint_on_first_use(
                &token,
                request(scope_a, account_id, "https://api.example.com/v1/x"),
            )
            .unwrap();
        assert_eq!(
            first_lease.session_id(),
            second_lease.session_id(),
            "cache-hit reuse must return the same underlying session"
        );
        let session_id = first_lease.session_id();

        first_lease.revoke();
        assert!(
            broker.validate_session(session_id, Utc::now()).is_ok(),
            "the second lease is still outstanding; revoking the first must not kill the shared session"
        );

        second_lease.revoke();
        assert!(matches!(
            broker.validate_session(session_id, Utc::now()),
            Err(CredentialBrokerError::UnknownSession { .. })
        ));
    }

    #[test]
    fn revoke_prunes_secondary_indices_so_placeholder_no_longer_resolves() {
        // Revoking a session must not just make validate_session fail — it
        // must also stop the placeholder from resolving to it at all, and
        // must not leave stale bookkeeping (jit_minted / sessions_by_placeholder)
        // behind for the life of the process.
        let broker = Arc::new(InMemoryCredentialBroker::new());
        let registry = CredentialPlaceholderRegistry::new();
        let (token, scope_a, account_id) = seeded(&broker, &registry, "user-prune");

        let lease = broker
            .mint_on_first_use(
                &token,
                request(scope_a.clone(), account_id, "https://api.example.com/v1/x"),
            )
            .unwrap();
        assert!(
            broker
                .find_session_by_placeholder(&token, &scope_a)
                .unwrap()
                .is_some()
        );

        lease.revoke();

        assert!(
            broker
                .find_session_by_placeholder(&token, &scope_a)
                .unwrap()
                .is_none(),
            "placeholder must not resolve to a revoked session"
        );
    }

    fn seeded(
        broker: &Arc<InMemoryCredentialBroker>,
        registry: &CredentialPlaceholderRegistry,
        user: &str,
    ) -> (
        CredentialPlaceholderToken,
        ResourceScope,
        CredentialAccountId,
    ) {
        let tenant = TenantId::new("tenant-a").unwrap();
        let user_id = UserId::new(user).unwrap();
        let provider = ExtensionId::new("google").unwrap();
        let token = registry
            .get_or_create(&tenant, &user_id, &provider)
            .unwrap();
        let caller_scope = scope("tenant-a", user);
        let account_id = CredentialAccountId::new("google_prod").unwrap();
        broker
            .put_account(account(
                caller_scope.clone(),
                account_id.clone(),
                SecretHandle::new("google_key").unwrap(),
            ))
            .unwrap();
        (token, caller_scope, account_id)
    }
}
