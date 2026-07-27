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
use std::sync::{Arc, Mutex};

use ironclaw_host_api::{CapabilityId, ExtensionId, InvocationId, TenantId, UserId};
use uuid::Uuid;

use crate::{
    CredentialAccountId, CredentialBrokerError, CredentialSessionId, CredentialSessionRecord,
    CredentialSessionRequest, InMemoryCredentialBroker, SessionState, lock_or_recover,
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

/// Required length of the suffix after [`CREDENTIAL_PLACEHOLDER_PREFIX`].
///
/// Registry-issued tokens are always exactly this long — a UUIDv4
/// `simple()` string (32 lowercase hex characters, see
/// [`CredentialPlaceholderToken::generate`]) — and nothing in this crate
/// legitimately needs a shorter or longer suffix. [`CredentialPlaceholderToken::parse`]
/// enforces this length exactly (not just a minimum) so that a sandbox
/// caller cannot hand back an arbitrarily long `icsbx_...` string and drive
/// unbounded hashing/cloning/map-insertion work on the host from unvalidated
/// input.
///
/// Exposed publicly so `ironclaw_safety::leak_detector`'s
/// `sandbox_credential_placeholder` pattern — which independently matches
/// `icsbx_` plus 16+ alphanumeric characters — can pin its own minimum
/// against a token this crate's public API would actually accept, rather
/// than a hardcoded literal drifting silently out of sync. See
/// `sandbox_credential_placeholder_prefix_matches_registry` in
/// `ironclaw_safety/src/leak_detector.rs`.
pub const CREDENTIAL_PLACEHOLDER_SUFFIX_LEN: usize = 32;

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
        let Some(suffix) = value.strip_prefix(CREDENTIAL_PLACEHOLDER_PREFIX) else {
            return Err(CredentialBrokerError::InvalidPlaceholderToken {
                reason: format!("must start with '{CREDENTIAL_PLACEHOLDER_PREFIX}'"),
            });
        };
        // Mirror `validate_credential_id`'s charset discipline (this crate's
        // established convention for opaque id newtypes) rather than
        // accepting an arbitrary suffix: this token is designed to sit in a
        // container's environment/config and a human or log line may see it
        // (see the type doc comment), so malformed input — including control
        // characters or shell metacharacters an attacker-controlled sandbox
        // side might send back through `parse` — must fail closed here
        // instead of propagating downstream.
        //
        // The length check is an exact match, not just a minimum: registry-
        // issued tokens are always exactly `CREDENTIAL_PLACEHOLDER_SUFFIX_LEN`
        // characters (see that constant's doc comment), and nothing here
        // legitimately needs a variable-length suffix. Accepting an
        // arbitrarily long `icsbx_...` string from the sandbox would let
        // untrusted input drive unbounded hashing/cloning/map-insertion work
        // on the host.
        // Counted in `char`s, not bytes: the error message below promises
        // "characters", and while the charset check just below rejects any
        // non-ASCII input anyway (so `.len()` and `.chars().count()` agree on
        // every value this function ever accepts), the length check runs
        // first and should measure what it claims to measure regardless of
        // charset-check ordering.
        if suffix.chars().count() != CREDENTIAL_PLACEHOLDER_SUFFIX_LEN
            || !suffix
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return Err(CredentialBrokerError::InvalidPlaceholderToken {
                reason: format!(
                    "must be '{CREDENTIAL_PLACEHOLDER_PREFIX}' followed by exactly {} ASCII alphanumeric characters",
                    CREDENTIAL_PLACEHOLDER_SUFFIX_LEN
                ),
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

/// Both maps `CredentialPlaceholderRegistry` keeps, under one lock.
///
/// `by_owner` and `by_token` are two views of the identical bookkeeping (one
/// triple <-> one token), not two independent pieces of state — so they are
/// kept in a single `Mutex` rather than one each. Two separate locks would
/// let a concurrent `get_or_create` publish into `by_owner`, drop that guard,
/// and be observed by another caller's early return *before* the matching
/// `by_token` entry existed, so `resolve()` could answer `None` for a token
/// `get_or_create` had already handed out. A single lock over both maps
/// makes every publish atomic from an outside observer's perspective.
#[derive(Debug, Default)]
struct RegistryState {
    by_owner: HashMap<CredentialPlaceholderOwnerKey, CredentialPlaceholderToken>,
    by_token: HashMap<CredentialPlaceholderToken, CredentialPlaceholderOwner>,
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
    state: Mutex<RegistryState>,
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
        let mut state =
            self.state
                .lock()
                .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                    reason: error.to_string(),
                })?;
        if let Some(existing) = state.by_owner.get(&key) {
            return Ok(existing.clone());
        }
        let token = CredentialPlaceholderToken::generate();
        state.by_owner.insert(key, token.clone());
        state.by_token.insert(
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
    /// Because `by_owner` and `by_token` share one lock with `get_or_create`,
    /// a token returned by `get_or_create` always resolves immediately — there
    /// is no window where a freshly-minted token exists in one map but not
    /// the other.
    pub fn resolve(
        &self,
        token: &CredentialPlaceholderToken,
    ) -> Result<Option<CredentialPlaceholderOwner>, CredentialBrokerError> {
        Ok(self
            .state
            .lock()
            .map_err(|error| CredentialBrokerError::BrokerUnavailable {
                reason: error.to_string(),
            })?
            .by_token
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
/// A missed revoke path would leave a standing grant, so dropping or
/// explicitly revoking the lease is the only way this type releases its
/// reference — there is no accessor that extends the session's life past the
/// lease being dropped. That said, this is intent enforced by this type's
/// API shape, not an unconditional guarantee: `std::mem::forget(lease)` is
/// safe Rust, skips `Drop`, and would strand the reference exactly the way a
/// forgotten `MutexGuard` would strand a lock. The backstop for that case
/// (and for any other way a lease's `Drop` fails to run) is the session
/// expiry cap `create_session` applies at mint time — 30 minutes by
/// default — so even a stranded reference cannot hold the session live past
/// that ceiling.
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
    ///
    /// **Concurrency:** the lookup-or-mint sequence below (checking
    /// `jit_minted` for an existing live session, and minting+publishing a
    /// new one if there isn't one) runs under a single, held-once lock on
    /// `session_state`. Two threads racing on an identical `JitMintKey` used
    /// to be able to both observe "nothing minted yet" when `jit_minted` was
    /// checked and updated through separate lock acquisitions, and both mint
    /// their own session, defeating the one-session-per-binding guarantee
    /// this module's doc comment asserts. Holding one coarse lock across the
    /// whole sequence closes that window; see
    /// `mint_on_first_use_is_race_free_under_concurrent_first_use` below.
    ///
    /// `sessions`, `jit_minted`, and `sessions_by_placeholder` (and the
    /// outstanding-lease count, folded into each session's record) all live
    /// behind that same `session_state` lock now, so the mint-then-publish
    /// sequence below — inserting the session, recording it in `jit_minted`,
    /// and binding it to `placeholder` — is a single atomic critical section
    /// with no fallible step in the middle: there is no window where the
    /// session is referenced by one index but not yet by the others, and no
    /// second lock acquisition for any helper to self-deadlock against.
    /// `accounts` (via [`InMemoryCredentialBroker::build_session`]) stays a
    /// separate lock and is only ever acquired while already holding
    /// `session_state`, never the other way around, so nesting it here
    /// cannot deadlock either.
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

        let mut state = lock_or_recover(&self.session_state);

        let session_id = if let Some(&session_id) = state.jit_minted.get(&key)
            && try_join_live_lease(&mut state, session_id, chrono::Utc::now())?
        {
            session_id
        } else {
            let session = self.build_session(request)?;
            let session_id = session.correlation_id();
            state.sessions.insert(
                session_id,
                CredentialSessionRecord {
                    session,
                    uses: 0,
                    // The first (and, at this point, only) outstanding lease
                    // reference for a freshly-minted session.
                    lease_count: 1,
                },
            );
            state.jit_minted.insert(key, session_id);
            session_id
        };
        bind_placeholder_to_session(&mut state, placeholder.clone(), session_id);
        drop(state);

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
        let now = chrono::Utc::now();
        let state = lock_or_recover(&self.session_state);
        let Some(session_ids) = state.sessions_by_placeholder.get(placeholder) else {
            return Ok(None);
        };
        for session_id in session_ids {
            // A session_id can appear in `sessions_by_placeholder` for a
            // session that has since been fully revoked and removed from
            // `sessions` (the two live behind the same lock, but revoke
            // prunes both together, not atomically-with-this-read — so a
            // missing record here just means "grants nothing", same as an
            // expired or use-exhausted one).
            let Some(record) = state.sessions.get(session_id) else {
                continue;
            };
            match crate::ensure_credential_session_record_usable(record, *session_id, now) {
                Ok(()) if record.session.scope() == scope => {
                    return Ok(Some(record.session.clone()));
                }
                Ok(()) => continue,
                Err(CredentialBrokerError::SessionExpired { .. })
                | Err(CredentialBrokerError::SessionUseLimitExceeded { .. }) => continue,
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
    /// Unconditional: removes the session and prunes the `jit_minted` and
    /// `sessions_by_placeholder` secondary indices regardless of any
    /// outstanding lease count, so a long-lived process does not accumulate
    /// stale entries for every invocation/binding that has ever existed. See
    /// [`InMemoryCredentialBroker::release_lease`] for the refcount-aware
    /// variant every lease actually calls.
    pub fn revoke_session(&self, session_id: CredentialSessionId) {
        remove_session_and_indices(&mut lock_or_recover(&self.session_state), session_id);
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
        release_lease_in_state(&mut lock_or_recover(&self.session_state), session_id);
    }
}

/// Attempts to join an existing lease on `session_id`, returning whether it
/// succeeded. Always called with `state` already locked by the caller
/// ([`InMemoryCredentialBroker::mint_on_first_use`]) — `sessions`,
/// `jit_minted`, `sessions_by_placeholder`, and each record's lease count all
/// live behind that one lock now, so this can inspect and mutate all of them
/// as plain field access with no risk of a second acquisition to
/// self-deadlock against.
///
/// The lease-count increment and the liveness check are deliberately ordered
/// increment-then-validate, not validate-then-increment: incrementing first
/// only succeeds if `state.sessions` still has a live entry for
/// `session_id`, and [`release_lease_in_state`] only removes that entry
/// (making the session eligible for revoke) under the very same lock this
/// function always runs under. So once this call has incremented the count,
/// a concurrent release of the last-outstanding prior lease cannot have
/// already removed the session from under it — the two operations serialize
/// on the single `session_state` lock instead of racing across separate
/// acquisitions. Validating the *other* way (validate, then increment) would
/// leave a window where a concurrent revoke could land in between, handing
/// back a lease for an already-dead session.
///
/// If the count was joined but the session turns out to be expired or
/// use-exhausted, the just-acquired reference is released again (via
/// [`release_lease_in_state`], which removes the session and its secondary
/// indices if that was the last reference) so this function never leaves a
/// dangling lease count behind on its own failure path.
fn try_join_live_lease(
    state: &mut SessionState,
    session_id: CredentialSessionId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, CredentialBrokerError> {
    let Some(record) = state.sessions.get_mut(&session_id) else {
        return Ok(false);
    };
    record.lease_count += 1;
    match crate::ensure_credential_session_record_usable(record, session_id, now) {
        Ok(()) => Ok(true),
        Err(error) => {
            release_lease_in_state(state, session_id);
            match error {
                CredentialBrokerError::UnknownSession { .. }
                | CredentialBrokerError::SessionExpired { .. }
                | CredentialBrokerError::SessionUseLimitExceeded { .. } => Ok(false),
                other => Err(other),
            }
        }
    }
}

/// Decrements the outstanding lease count for `session_id`; if that was the
/// last reference, removes the session and its secondary index entries (see
/// [`remove_session_and_indices`]). A missing record is treated the same as
/// "last reference already gone" — nothing left to decrement.
fn release_lease_in_state(state: &mut SessionState, session_id: CredentialSessionId) {
    let is_last_reference = match state.sessions.get_mut(&session_id) {
        Some(record) if record.lease_count > 1 => {
            record.lease_count -= 1;
            false
        }
        _ => true,
    };
    if is_last_reference {
        remove_session_and_indices(state, session_id);
    }
}

/// Removes `session_id` from `sessions` and prunes it out of both secondary
/// indices (`jit_minted`, `sessions_by_placeholder`) unconditionally,
/// regardless of lease count. The single shared tail for
/// [`InMemoryCredentialBroker::revoke_session`] (unconditional) and
/// [`release_lease_in_state`] (only once the last lease reference is gone).
fn remove_session_and_indices(state: &mut SessionState, session_id: CredentialSessionId) {
    state.sessions.remove(&session_id);
    state
        .jit_minted
        .retain(|_, minted_session_id| *minted_session_id != session_id);
    state.sessions_by_placeholder.retain(|_, session_ids| {
        session_ids.remove(&session_id);
        !session_ids.is_empty()
    });
}

fn bind_placeholder_to_session(
    state: &mut SessionState,
    placeholder: CredentialPlaceholderToken,
    session_id: CredentialSessionId,
) {
    state
        .sessions_by_placeholder
        .entry(placeholder)
        .or_default()
        .insert(session_id);
}

#[cfg(test)]
#[path = "placeholder/tests.rs"]
mod tests;
