//! Sandbox credential firewall — obligation-staging chokepoint (design
//! decision W8, `docs/plans/2026-07-26-sandbox-credential-firewall-design.md`
//! §2.2 "End-to-end flow", §2.3 "Placeholder vs grant lifetime", §3.3
//! "what JIT does NOT block", §3.4 "D2 mechanics").
//!
//! Models the same shape as
//! `ironclaw_extension_host::product_lifecycle::ExtensionLifecycleManager::activation_credential_requirements`:
//! a caller stages what an invocation is entitled to, ahead of the moment
//! that entitlement is actually consumed, and the consumer only ever sees a
//! yes/no answer — never a bypass to mint its own grant. There is exactly
//! one implementation; no trait exists here on purpose (an earlier design
//! pass considered one and rejected it as the over-engineering this design
//! explicitly guards against).
//!
//! **Why staged ahead of time, keyed by `(tenant_id, user_id)`.** The
//! consumer of this chokepoint is the shared egress proxy (its
//! connection-attribution resolver is W6 work, not built yet): it is
//! invoked per-TCP-connection and can resolve a peer IP to a
//! `{tenant, user}` (via `ConnectionAttributionResolver`), but
//! it has no way to recover which *invocation* opened that connection —
//! invocation identity simply is not present in anything the proxy can
//! observe from a TCP peer address. So the chokepoint cannot be a
//! request-time "ask which invocation this is" callback; it must be staged
//! ahead of the connection, under the one key the proxy actually has.
//!
//! **Fail-closed, with two distinct outcomes** (§3.4): a request over an
//! *established, attributed* connection with no live grant is a
//! [`SandboxCredentialDecision::NoGrant`] — GRANT-DENIAL, D5's "strip the
//! placeholder, forward the request bare, annotate output" — because the
//! origin's own 401/403 is a better error than blocking a public clone.
//! Everything else (attribution failed, or the lookup/callback did not
//! complete before its deadline) is [`SandboxCredentialFirewallError`] —
//! CONNECTION-DENIAL: deny the connection outright, never forward it. These
//! are deliberately different error shapes so a caller cannot collapse them
//! into a single fail-open branch by accident.
//!
//! **W8 is the chokepoint; W6 (proxy TLS termination) is the consumer, not
//! built yet.** Nothing in this crate calls into
//! [`SandboxCredentialFirewall`] today — it ships unwired, same as the
//! proxy's connection-attribution resolver, until the proxy is wired to
//! call it (profile-gated rollout per the design doc's PR strategy).

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use ironclaw_host_api::{SecretHandle, TenantId, UserId};
use ironclaw_secrets::CredentialTargetPolicy;

/// The key the proxy can actually derive at connection time (see module
/// doc): `(tenant_id, user_id)`, never an invocation id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StagingKey {
    tenant_id: TenantId,
    user_id: UserId,
}

impl StagingKey {
    fn new(tenant_id: &TenantId, user_id: &UserId) -> Self {
        Self {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
        }
    }
}

/// A staged credential grant for one bound provider, entitling requests from
/// its `(tenant_id, user_id)` to have their placeholder swapped for the real
/// secret when the destination matches `allowed_targets` — mirrors
/// `CredentialAccount::{secret_handles, allowed_targets}` narrowed to what a
/// connection-attributed lookup can use (no per-request method/URL is known
/// yet at staging time; `CredentialTargetPolicy::matches` still applies it
/// once the proxy has parsed the actual request).
///
/// Lifetime is explicit and short (D4): a grant staged for an invocation
/// expires on its own even if nothing ever calls [`SandboxCredentialFirewall::revoke`]
/// — and now, structurally, even if the [`StagedObligationLease`] that owns
/// it is never dropped (`std::mem::forget`); see [`MAX_GRANT_TTL`].
#[derive(Clone, PartialEq, Eq)]
#[allow(dead_code)] // consumed by W6 (proxy TLS termination + credential injection); not wired yet
pub(crate) struct StagedCredentialObligation {
    pub(crate) secret_handle: SecretHandle,
    pub(crate) allowed_targets: Vec<CredentialTargetPolicy>,
    expires_at: Instant,
}

impl std::fmt::Debug for StagedCredentialObligation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omit `secret_handle`: repository policy is to never
        // let a secret-handle identifier reach logs/panic output
        // unredacted, and this type's derived `Debug` would otherwise flow
        // through both `SandboxCredentialDecision::Grant` and the
        // firewall's own (also redacted) `Debug`. `allowed_targets` is
        // target metadata, not secret material, so it is safe to print.
        formatter
            .debug_struct("StagedCredentialObligation")
            .field("allowed_targets", &self.allowed_targets)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Outer TTL backstop (D4). Per the design doc, the intended caller-supplied
/// TTL is "invocation timeout + 30s grace" — a few seconds to low minutes,
/// well under this. This constant is not the normal value; it exists so that
/// a misconfigured caller, or a [`StagedObligationLease`] whose `Drop` never
/// runs (see that type's `std::mem::forget` caveat), cannot hold a grant live
/// past a fixed ceiling. Mirrors `create_session`'s 30-minute default session
/// expiry cap in `ironclaw_secrets::placeholder`, which serves the same
/// backstop role for `CredentialSessionLease`.
pub(crate) const MAX_GRANT_TTL: Duration = Duration::from_secs(30 * 60);

impl StagedCredentialObligation {
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn new(
        secret_handle: SecretHandle,
        allowed_targets: Vec<CredentialTargetPolicy>,
        ttl: Duration,
    ) -> Self {
        let now = Instant::now();
        // Clamp to the outer backstop before the fail-closed `checked_add`
        // below: a caller-requested TTL longer than `MAX_GRANT_TTL` is never
        // honored verbatim, whether or not the lease's `Drop` ever runs.
        let clamped_ttl = ttl.min(MAX_GRANT_TTL);
        Self {
            secret_handle,
            allowed_targets,
            // `checked_add` rather than `+`: an overflowing TTL must fail
            // closed (never staged / immediately expired), not panic — see
            // the same idiom in `obligations.rs`'s
            // `RuntimeSecretInjectionStore::insert`.
            expires_at: now.checked_add(clamped_ttl).unwrap_or(now),
        }
    }

    fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

/// Outcome of a successful (non-CONNECTION-DENIAL) firewall lookup.
///
/// GRANT-DENIAL lives here, not in [`SandboxCredentialFirewallError`],
/// because it is not a failure of the firewall itself — the connection was
/// validly attributed and the lookup completed on time; there is simply
/// nothing staged (or it expired). The caller (future W6 proxy code) reacts
/// to `NoGrant` by stripping the placeholder and forwarding the request
/// bare, never by tearing down the connection (D5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) enum SandboxCredentialDecision {
    /// One or more staged obligations are currently live for this
    /// connection's `(tenant_id, user_id)` — never empty (an empty set
    /// decays to `NoGrant`, see `authorize`). The per-user sandbox
    /// concurrency ceiling (`ironclaw_host_api::resource::SandboxQuota`) is >1, so this is the normal
    /// case, not a race: each concurrent invocation stages its own
    /// obligation for possibly a different provider/binding, and all of
    /// them stay live simultaneously.
    ///
    /// The firewall deliberately does not pick one for the caller: per
    /// design doc §2.2/§3.3, obligations carry `allowed_targets`
    /// (`CredentialTargetPolicy`) that only the proxy can evaluate, because
    /// only the proxy has parsed the actual per-request method/URL — the
    /// firewall's `authorize` is called with just a `(tenant_id, user_id)`
    /// and no request target. So matching which obligation authorizes a
    /// given destination is the caller's (W6's) job, via
    /// `CredentialTargetPolicy::matches` against each entry in turn; the
    /// firewall's contract stops at "here is everything currently entitled
    /// for this principal." Also: §3.3 already accepts that any process
    /// during an active invocation can mint a grant for any of that user's
    /// bindings, so handing back the full live set does not widen the
    /// existing security envelope — it was already "all of this user's
    /// bindings," just previously expressed one obligation at a time.
    Grant(Vec<StagedCredentialObligation>),
    /// GRANT-DENIAL: no live obligation for this `(tenant_id, user_id)`.
    /// Strip the placeholder, forward the request bare, annotate output.
    NoGrant,
}

/// CONNECTION-DENIAL. Both variants collapse to the same fail-closed action
/// at the caller: deny the connection outright, never forward it — even
/// bare. Distinct from [`SandboxCredentialDecision::NoGrant`], which is safe
/// to forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) enum SandboxCredentialFirewallError {
    /// The connection's peer could not be attributed to a `(tenant_id,
    /// user_id)` (attribution failure — duplicate IP, malformed labels, a
    /// Docker query error; see the future W6 proxy's connection-attribution
    /// resolver for its own fail-closed cases). There is no principal to
    /// authorize a lookup for.
    #[error(
        "sandbox credential firewall: connection denied — peer could not be attributed to a tenant/user"
    )]
    AttributionFailed,
    /// The lookup did not complete before its deadline. Per §3.4, a timed
    /// out callback into policy is treated exactly like a denial — never as
    /// an implicit pass-through.
    #[error(
        "sandbox credential firewall: connection denied — obligation lookup exceeded its deadline"
    )]
    LookupTimedOut,
}

/// The obligation-staging chokepoint itself. Concrete struct, no trait: this
/// crate has exactly one implementation and one process-local store; see the
/// module doc for why a port here would be the over-engineering this design
/// already rejected.
///
/// **Refcounted-set staging (not a single slot).** The per-user sandbox
/// concurrency ceiling (`ironclaw_host_api::resource::SandboxQuota`) is >1: several invocations for
/// the same `(tenant_id, user_id)` legitimately run at once, each staging
/// its own obligation. `stage()` therefore *adds* an entry under a unique id
/// rather than replacing whatever the key currently holds, and
/// [`SandboxCredentialFirewall::revoke`] removes only the one entry its
/// caller's [`StagedObligationLease`] owns. A key's inner set is dropped
/// entirely once its last entry is gone — there is never a stray empty
/// collection left keyed by `(tenant_id, user_id)`.
///
/// This is a correction of an earlier single-slot design where `stage()`
/// overwrote any existing entry for the key and `revoke()` removed the
/// entire key unconditionally: with concurrency >1 that let an invocation
/// which finished first silently delete a still-live sibling invocation's
/// grant, just by dropping its own lease.
#[derive(Default)]
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) struct SandboxCredentialFirewall {
    /// Eviction policy (bounded-resources rule,
    /// `.claude/rules/safety-and-sandbox.md`): an entry leaves its key's set
    /// via explicit [`SandboxCredentialFirewall::revoke`] (D4's primary
    /// bound), or lazily on the next [`SandboxCredentialFirewall::authorize`]
    /// read that finds it past its TTL (`StagedCredentialObligation::is_expired`),
    /// which removes it before deciding. A key's set is removed from the
    /// outer map the moment it becomes empty by either path — the outer map
    /// never accumulates an empty entry. There is no size cap and no
    /// periodic sweep: the key space is `(TenantId, UserId)` pairs, which an
    /// attacker cannot cheaply mint, and the inner set is bounded by the
    /// per-user sandbox concurrency ceiling, so the only failure mode
    /// without lazy removal is a slow leak from callers who stage once and
    /// never return — not an unbounded-growth attack surface.
    staged: Mutex<HashMap<StagingKey, HashMap<StagedEntryId, StagedCredentialObligation>>>,
    /// Monotonic source of each entry's id. A plain `u64` counter (not a
    /// random nonce): uniqueness only needs to hold within this one
    /// process's lifetime — the same scope `staged` itself lives in — and a
    /// counter is cheaper and trivially testable (no collision-probability
    /// argument needed). `Relaxed` ordering is enough because the counter's
    /// only job is producing distinct values; the `staged` mutex is what
    /// actually orders the insert each id guards.
    next_entry_id: AtomicU64,
}

/// Identifies one entry within a staging key's set. Only
/// [`SandboxCredentialFirewall::stage`] constructs one (via
/// `next_entry_id`), and only [`SandboxCredentialFirewall::revoke`] — private
/// to this module, reachable solely through [`StagedObligationLease`]'s
/// `Drop`/`revoke` — consumes one. Wrapping the counter closes the gap a bare
/// `u64` would leave: nothing in this crate could otherwise construct an
/// arbitrary id and revoke an entry it does not own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct StagedEntryId(u64);

impl std::fmt::Debug for SandboxCredentialFirewall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omit `staged`'s contents: it keys tenant/user
        // identity (compliance-sensitive, even though `StagedCredentialObligation`'s
        // own `Debug` already redacts the secret handle within it). Expose
        // only an aggregate count, mirroring `SandboxCertificateAuthority`'s
        // manual `Debug` in `ca.rs`.
        formatter
            .debug_struct("SandboxCredentialFirewall")
            .field("staged_keys", &self.lock().len())
            .finish_non_exhaustive()
    }
}

impl SandboxCredentialFirewall {
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Stages an obligation at capability-dispatch prepare time, before the
    /// invocation's shell command ever runs — mirrors
    /// `BuiltinObligationHandler::finish_prepare` staging network policy and
    /// secret injections ahead of dispatch in `obligations.rs`. Adds a new
    /// entry to the (possibly already non-empty) set staged for this
    /// `(tenant_id, user_id)`; does not replace or disturb any existing
    /// entry for the same key — see the struct doc for why a single slot is
    /// wrong once concurrency >1 is the normal case.
    ///
    /// Returns a [`StagedObligationLease`]: holding it keeps this specific
    /// obligation staged, dropping it revokes only this one. Takes
    /// `self: &Arc<Self>` (mirroring `InMemoryCredentialBroker::mint_on_first_use`)
    /// because the returned lease needs to outlive this call and revoke
    /// through its own `Arc` clone of the firewall.
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn stage(
        self: &Arc<Self>,
        tenant_id: &TenantId,
        user_id: &UserId,
        obligation: StagedCredentialObligation,
    ) -> StagedObligationLease {
        let entry_id = StagedEntryId(self.next_entry_id.fetch_add(1, Ordering::Relaxed));
        self.lock()
            .entry(StagingKey::new(tenant_id, user_id))
            .or_default()
            .insert(entry_id, obligation);
        StagedObligationLease {
            firewall: Arc::clone(self),
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
            entry_id,
            revoked: false,
        }
    }

    /// Explicit revoke — D4's primary bound ("explicit revoke on invocation
    /// completion"), independent of the obligation's own TTL backstop.
    ///
    /// Removes only the entry identified by `entry_id` from this key's set —
    /// never the whole key, never another invocation's entry — and drops
    /// the key's set from the outer map once it is empty. A no-op if
    /// nothing is staged for the key, or if `entry_id` is not (or no longer)
    /// present in it (already revoked, or reclaimed by `authorize` after
    /// expiry).
    ///
    /// Private, not `pub(crate)`: [`StagedEntryId`] is only constructible by
    /// [`Self::stage`], and only [`StagedObligationLease`] (defined in this
    /// same module, so it can still reach a private method) is meant to call
    /// this — keeping it out of the crate-visible surface means no other
    /// code in this crate can revoke an entry it never staged.
    fn revoke(&self, tenant_id: &TenantId, user_id: &UserId, entry_id: StagedEntryId) {
        let key = StagingKey::new(tenant_id, user_id);
        let mut staged = self.lock();
        if let Some(entries) = staged.get_mut(&key) {
            entries.remove(&entry_id);
            if entries.is_empty() {
                staged.remove(&key);
            }
        }
    }

    /// The chokepoint the proxy calls per intercepted connection (§3.4).
    ///
    /// `identity` is the proxy's already-resolved attribution outcome for
    /// this connection — `None` means attribution failed (duplicate IP,
    /// malformed labels, a Docker query error, ...); this method does not
    /// perform attribution itself — the future W6 proxy's
    /// connection-attribution resolver owns that. `deadline` bounds the
    /// whole call: if it has already passed by the time this runs, the
    /// lookup is treated as timed out — CONNECTION-DENIAL, never forwarded
    /// — exactly like a hung callback into policy per §3.4.
    ///
    /// Returns every currently-live obligation for this `(tenant_id,
    /// user_id)` as one [`SandboxCredentialDecision::Grant`] — see that
    /// variant's doc for why the firewall does not pick one itself.
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn authorize(
        &self,
        identity: Option<(&TenantId, &UserId)>,
        deadline: Instant,
    ) -> Result<SandboxCredentialDecision, SandboxCredentialFirewallError> {
        let Some((tenant_id, user_id)) = identity else {
            return Err(SandboxCredentialFirewallError::AttributionFailed);
        };
        if Instant::now() >= deadline {
            return Err(SandboxCredentialFirewallError::LookupTimedOut);
        }

        let key = StagingKey::new(tenant_id, user_id);
        let mut staged = self.lock();
        // Re-check after acquiring the lock: the doc above promises the
        // deadline bounds the *whole call*, but lock acquisition itself can
        // block under contention — without this check, a caller stalled on
        // `self.lock()` past `deadline` could still fall through to a
        // `Grant`/`NoGrant` decision instead of the required
        // CONNECTION-DENIAL.
        if Instant::now() >= deadline {
            return Err(SandboxCredentialFirewallError::LookupTimedOut);
        }
        let now = Instant::now();
        let Some(entries) = staged.get_mut(&key) else {
            return Ok(SandboxCredentialDecision::NoGrant);
        };
        // Lazy reclamation on read (see the struct doc on `staged`): an
        // expired entry is dead weight the caller will never return for, so
        // remove it here rather than leaving it to accumulate until an
        // explicit `revoke` that may never come — reclaimed one entry at a
        // time so still-live siblings staged under the same key survive.
        entries.retain(|_entry_id, obligation| !obligation.is_expired(now));
        if entries.is_empty() {
            staged.remove(&key);
            return Ok(SandboxCredentialDecision::NoGrant);
        }
        let live: Vec<StagedCredentialObligation> = entries.values().cloned().collect();
        Ok(SandboxCredentialDecision::Grant(live))
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<
        '_,
        HashMap<StagingKey, HashMap<StagedEntryId, StagedCredentialObligation>>,
    > {
        self.staged
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Test-only observability into the total number of staged obligation
    /// entries across all keys, so a test can pin *reclamation* (the entry
    /// is gone) rather than only the decision returned to the caller (which
    /// looks identical whether an entry was removed or merely skipped).
    #[cfg(test)]
    fn staged_len(&self) -> usize {
        self.lock().values().map(|entries| entries.len()).sum()
    }
}

/// RAII handle for a staged [`StagedCredentialObligation`] — mirrors
/// `ironclaw_secrets::placeholder::CredentialSessionLease` exactly, and for
/// the same reason: relying on a caller to call `revoke()` explicitly on
/// every exit path (success, error, timeout, panic) is exactly the
/// discipline gap that shipped a Critical standing-grant leak in that
/// sibling mechanism (PR #6689, `finish_lease`). Doing this now, while
/// `stage()` has zero callers, is free; retrofitting it after W6 exists would
/// mean auditing every one of its exit paths instead.
///
/// The guard is the primary mechanism; [`SandboxCredentialFirewall::authorize`]'s
/// lazy TTL reclamation and [`MAX_GRANT_TTL`] are the backstop for whatever
/// exit path the guard's `Drop` fails to run on:
/// - **success** / **error**: call [`StagedObligationLease::revoke`] explicitly.
/// - **timeout**: if the future holding the lease is dropped (cancellation,
///   `tokio::time::timeout`, or any other future drop), `Drop` revokes it.
/// - **panic**: `Drop` still runs during unwind, so a panic mid-dispatch
///   revokes it too.
///
/// As with `CredentialSessionLease`, this is intent enforced by the type's
/// API shape, not an unconditional guarantee: `std::mem::forget(lease)` is
/// safe Rust, skips `Drop`, and would strand the staged obligation past
/// invocation end exactly the way a forgotten `MutexGuard` would strand a
/// lock. `MAX_GRANT_TTL`'s clamp in `StagedCredentialObligation::new` is the
/// backstop for that case: even a stranded reference cannot hold the grant
/// live past that ceiling.
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) struct StagedObligationLease {
    firewall: Arc<SandboxCredentialFirewall>,
    tenant_id: TenantId,
    user_id: UserId,
    /// The id this lease's `stage()` call assigned its entry in the
    /// key's set. Carried so this lease's revoke can only ever delete *its
    /// own* entry — never the whole key, and never a sibling entry that a
    /// concurrent or later `stage()` call for the same `(tenant_id,
    /// user_id)` added alongside it.
    entry_id: StagedEntryId,
    revoked: bool,
}

impl StagedObligationLease {
    /// Explicitly revokes the staged obligation now. Intended for the
    /// success and error exit paths, where the caller can revoke
    /// synchronously rather than waiting for `Drop`.
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn revoke(mut self) {
        self.revoke_inner();
    }

    fn revoke_inner(&mut self) {
        if !self.revoked {
            self.revoked = true;
            self.firewall
                .revoke(&self.tenant_id, &self.user_id, self.entry_id);
        }
    }
}

impl Drop for StagedObligationLease {
    fn drop(&mut self) {
        self.revoke_inner();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::NetworkMethod;
    use ironclaw_secrets::CredentialPathPolicy;

    const FAR_FUTURE: Duration = Duration::from_secs(3600);

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).unwrap()
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).unwrap()
    }

    fn handle(value: &str) -> SecretHandle {
        SecretHandle::new(value).unwrap()
    }

    fn allow_all_targets() -> Vec<CredentialTargetPolicy> {
        Vec::new()
    }

    fn far_future_deadline() -> Instant {
        Instant::now() + FAR_FUTURE
    }

    #[test]
    fn staged_obligation_is_retrievable_by_its_tenant_and_user() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let _lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                FAR_FUTURE,
            ),
        );

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("attributed lookup within deadline must not error");

        match decision {
            SandboxCredentialDecision::Grant(obligations) => {
                assert_eq!(obligations.len(), 1);
                assert_eq!(obligations[0].secret_handle, handle("github-token"));
            }
            SandboxCredentialDecision::NoGrant => {
                panic!("expected a grant for the exact (tenant, user) that staged it")
            }
        }
    }

    /// GRANT-DENIAL: no obligation was ever staged for this (tenant, user).
    /// Per D5, this must be a *decision* (safe to forward bare), never an
    /// `Err` (which would tear the connection down) — a wrong-reason
    /// failure here would be returning `Err` instead of `Ok(NoGrant)`, which
    /// would incorrectly deny public/uncredentialed traffic outright.
    #[test]
    fn no_staged_obligation_is_grant_denial_not_connection_denial() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("an unstaged lookup is a decision, not a firewall error");

        assert_eq!(decision, SandboxCredentialDecision::NoGrant);
    }

    /// CONNECTION-DENIAL: attribution itself failed (proxy passes `None`).
    /// Must be a distinct `Err` variant from `LookupTimedOut` so a caller
    /// cannot collapse both into one branch and lose the audit distinction.
    #[test]
    fn unattributed_connection_is_denied_not_forwarded() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());

        let error = firewall
            .authorize(None, far_future_deadline())
            .expect_err("an unattributed connection must never resolve to a decision");

        assert_eq!(error, SandboxCredentialFirewallError::AttributionFailed);
    }

    /// CONNECTION-DENIAL: the lookup deadline already passed. Must deny,
    /// never silently fall through to a `NoGrant` decision that a caller
    /// might treat as "safe to forward" — a wrong-reason pass here would be
    /// the timeout path returning `Ok(NoGrant)` instead of `Err`.
    #[test]
    fn expired_deadline_denies_the_connection_even_when_a_grant_is_staged() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let _lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                FAR_FUTURE,
            ),
        );
        let already_passed = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("process has been running for at least 1 second by the time tests run");

        let error = firewall
            .authorize(Some((&tenant_a, &user_a)), already_passed)
            .expect_err("a deadline that already passed must deny even with a live grant staged");

        assert_eq!(error, SandboxCredentialFirewallError::LookupTimedOut);
    }

    /// A staged obligation past its own TTL is GRANT-DENIAL (safe to
    /// forward bare), distinct from a timed-out *lookup* — expiry is a
    /// property of the obligation, not of how long authorization took.
    #[test]
    fn expired_obligation_is_grant_denial() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let _lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                Duration::ZERO,
            ),
        );
        // The obligation's `expires_at` is `Instant::now() + 0` at staging
        // time. Sleeping 2ms guarantees `authorize`'s own `now` (captured
        // when it runs, below) is strictly past `expires_at` regardless of
        // clock resolution, so the obligation is deterministically expired
        // by the time `authorize` checks it — independent of the `deadline`
        // argument, which is a far-future value precisely so this test
        // isolates obligation expiry from lookup-deadline expiry.
        std::thread::sleep(Duration::from_millis(2));

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("expiry is a decision, not a firewall error");

        assert_eq!(decision, SandboxCredentialDecision::NoGrant);
    }

    /// Explicit revoke (D4's primary bound) removes a grant immediately,
    /// without waiting for its TTL.
    #[test]
    fn revoke_removes_a_staged_obligation_before_its_ttl_expires() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                FAR_FUTURE,
            ),
        );

        lease.revoke();

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("revoked lookup is a decision, not a firewall error");
        assert_eq!(decision, SandboxCredentialDecision::NoGrant);
    }

    /// Cross-user isolation: user B must never retrieve user A's staged
    /// obligation, even under the same tenant. A wrong-reason pass here
    /// would be keying staging by `tenant_id` alone and ignoring `user_id`.
    #[test]
    fn user_b_cannot_retrieve_user_a_staged_obligation() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let user_b = user("user-b");
        let _lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                FAR_FUTURE,
            ),
        );

        let decision_for_b = firewall
            .authorize(Some((&tenant_a, &user_b)), far_future_deadline())
            .expect("an unstaged (tenant, user) is a decision, not a firewall error");
        assert_eq!(decision_for_b, SandboxCredentialDecision::NoGrant);

        // Sanity: user A's own lookup still resolves — proves the isolation
        // above is real key-scoping, not a bug that denies everyone.
        let decision_for_a = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("user a's own lookup must still succeed");
        assert!(matches!(
            decision_for_a,
            SandboxCredentialDecision::Grant(_)
        ));
    }

    /// An expired obligation must be reclaimed from the staging map on read,
    /// not merely filtered on the way out. Without this, `staged` grows
    /// monotonically with every distinct `(tenant_id, user_id)` the process
    /// has ever seen — a caller that stages once and never returns leaves a
    /// dead entry forever (unbounded-resource rule,
    /// `.claude/rules/safety-and-sandbox.md`). The return value alone
    /// (`NoGrant`) cannot distinguish "filtered on read" from "actually
    /// removed" — `expired_obligation_is_grant_denial` above already pins the
    /// return value; this test pins the map side effect.
    #[test]
    fn expired_obligation_is_removed_from_staging_map_on_read() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let _lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                Duration::ZERO,
            ),
        );
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(
            firewall.staged_len(),
            1,
            "sanity: obligation must be staged before authorize reclaims it"
        );

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("expiry is a decision, not a firewall error");

        assert_eq!(decision, SandboxCredentialDecision::NoGrant);
        assert_eq!(
            firewall.staged_len(),
            0,
            "expired obligation must be removed from the staging map on read, \
             not just filtered from the returned decision"
        );
    }

    /// Same user id under a different tenant must also be isolated — the
    /// staging key is the full `(tenant_id, user_id)` pair, not user_id alone.
    #[test]
    fn same_user_id_under_a_different_tenant_is_isolated() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let tenant_b = tenant("tenant-b");
        let shared_user = user("user-a");
        let _lease = firewall.stage(
            &tenant_a,
            &shared_user,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                FAR_FUTURE,
            ),
        );

        let decision = firewall
            .authorize(Some((&tenant_b, &shared_user)), far_future_deadline())
            .expect("an unstaged (tenant, user) is a decision, not a firewall error");

        assert_eq!(decision, SandboxCredentialDecision::NoGrant);
    }

    /// Staging twice for the same `(tenant_id, user_id)` key adds a second
    /// live entry — it does NOT replace the first. The per-user sandbox
    /// concurrency ceiling (`ironclaw_host_api::resource::SandboxQuota`) is >1, so two invocations
    /// staging for the same principal at once is the normal path, and both
    /// obligations must stay live until each is independently revoked.
    ///
    /// This supersedes an earlier version of this test (single-slot design,
    /// concurrency ceiling of 1) that asserted restaging *replaced* the
    /// prior obligation — that assertion is no longer correct now that the
    /// map holds a set per key rather than one slot.
    #[test]
    fn staging_the_same_key_twice_keeps_both_obligations_live() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let _lease_old = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("old-token"), allow_all_targets(), FAR_FUTURE),
        );
        let _lease_new = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("new-token"), allow_all_targets(), FAR_FUTURE),
        );

        assert_grant_contains(
            &firewall,
            &tenant_a,
            &user_a,
            &[handle("old-token"), handle("new-token")],
        );
    }

    /// HIGH-severity regression: a stale lease dropped *after* a newer
    /// invocation has restaged the same `(tenant_id, user_id)` must not
    /// revoke the newer invocation's still-live grant.
    ///
    /// `staging_the_same_key_twice_keeps_both_obligations_live` above cannot
    /// catch this: each lease already owns its own entry in the refcounted
    /// set, so `_lease_old` and `_lease_new` falling out of scope in reverse
    /// declaration order just removes each one's own entry — a harmless,
    /// order-independent outcome that would pass whether or not per-entry
    /// isolation actually holds. This test forces the actual hazard order:
    /// drop the older lease *first*, while the newer lease (and its grant)
    /// is still alive, to prove one lease's revoke never reaches into a
    /// sibling's still-live entry.
    #[test]
    fn dropping_a_stale_lease_does_not_revoke_a_newer_grant_for_the_same_key() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");

        let lease_old = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("old-token"), allow_all_targets(), FAR_FUTURE),
        );
        let lease_new = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("new-token"), allow_all_targets(), FAR_FUTURE),
        );

        // The hazard order: the older invocation finishes and drops its
        // lease first, while the newer invocation's lease — and grant — is
        // still alive.
        drop(lease_old);

        assert_grant_contains(&firewall, &tenant_a, &user_a, &[handle("new-token")]);

        // Overshoot guard: the fix must not go so far that dropping the
        // *newest* lease stops revoking. `lease_new` is still live, so
        // dropping it must still remove its own entry — and since it was
        // the last entry left for this key, the key itself must be gone.
        drop(lease_new);
        let decision_after_newest_drop = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("attributed lookup within deadline must not error");
        assert_eq!(
            decision_after_newest_drop,
            SandboxCredentialDecision::NoGrant,
            "dropping the current (newest) lease must still revoke its grant"
        );
    }

    /// The primary RAII guarantee: dropping the lease revokes the staged
    /// obligation, mirroring `CredentialSessionLease`'s Drop-as-backstop
    /// contract. This is the structural fix for the #6689-shaped leak class —
    /// no caller discipline required.
    #[test]
    fn dropping_the_lease_revokes_the_staged_obligation() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                FAR_FUTURE,
            ),
        );

        drop(lease);

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("post-drop lookup is a decision, not a firewall error");
        assert_eq!(decision, SandboxCredentialDecision::NoGrant);
    }

    /// Explicit `revoke(self)` is the fast path for success/error exits. It
    /// must revoke immediately, and the `Drop` that still runs on the
    /// moved-from value at scope end must not double-revoke or panic.
    #[test]
    fn explicit_revoke_does_not_double_revoke_or_panic_on_drop() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                FAR_FUTURE,
            ),
        );

        lease.revoke();

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("post-revoke lookup is a decision, not a firewall error");
        assert_eq!(decision, SandboxCredentialDecision::NoGrant);
    }

    /// The backstop path: a panic mid-dispatch still unwinds through the
    /// lease's `Drop`, so a standing grant cannot survive a panic the way an
    /// explicit-only `revoke()` discipline would miss (the #6689 shape).
    #[test]
    fn lease_dropped_during_unwind_still_revokes() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                FAR_FUTURE,
            ),
        );

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _lease = lease;
            panic!("simulated mid-dispatch panic");
        }));
        assert!(
            result.is_err(),
            "the panic must actually unwind for this test to be meaningful"
        );

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("post-unwind lookup is a decision, not a firewall error");
        assert_eq!(decision, SandboxCredentialDecision::NoGrant);
    }

    /// TTL clamp: `MAX_GRANT_TTL` is the outer backstop for a caller that
    /// supplies an unreasonable TTL (or for a lease whose `Drop` never runs,
    /// e.g. `std::mem::forget`) — a 24h request must not be honored verbatim.
    #[test]
    fn ttl_is_clamped_to_max_grant_ttl_backstop() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("github-token"),
                allow_all_targets(),
                Duration::from_secs(24 * 60 * 60),
            ),
        );
        // Captured *after* `stage()` returns, so the internal `Instant::now()`
        // that `StagedCredentialObligation::new` clamped against is guaranteed
        // to be at or before this point — an upper bound, not a racy lower one.
        let after_staging = Instant::now();

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("attributed lookup within deadline must not error");
        let obligations = match decision {
            SandboxCredentialDecision::Grant(obligations) => obligations,
            SandboxCredentialDecision::NoGrant => panic!("expected a grant"),
        };
        assert_eq!(obligations.len(), 1);

        assert!(
            obligations[0].expires_at <= after_staging + MAX_GRANT_TTL,
            "a 24h TTL must be clamped to MAX_GRANT_TTL, not honored verbatim"
        );

        lease.revoke();
    }

    /// Helper: assert that `authorize` currently returns a live grant whose
    /// obligation secret handles are exactly `expected` (order-independent —
    /// the staging map is a `HashMap`, so iteration order over a key's set
    /// is not guaranteed).
    fn assert_grant_contains(
        firewall: &Arc<SandboxCredentialFirewall>,
        tenant_id: &TenantId,
        user_id: &UserId,
        expected: &[SecretHandle],
    ) {
        let decision = firewall
            .authorize(Some((tenant_id, user_id)), far_future_deadline())
            .expect("attributed lookup within deadline must not error");
        match decision {
            SandboxCredentialDecision::Grant(obligations) => {
                let actual: std::collections::HashSet<_> = obligations
                    .iter()
                    .map(|o| o.secret_handle.clone())
                    .collect();
                let expected_set: std::collections::HashSet<_> = expected.iter().cloned().collect();
                assert_eq!(
                    actual, expected_set,
                    "live grant set did not match the expected staged obligations"
                );
            }
            SandboxCredentialDecision::NoGrant => {
                panic!("expected a live grant set containing {expected:?}, got NoGrant")
            }
        }
    }

    /// N-safety: with the per-user sandbox concurrency ceiling now >1
    /// (`ironclaw_host_api::resource::SandboxQuota`), 4 concurrent invocations staging for the same
    /// `(tenant_id, user_id)` is the normal case. Dropping their leases in a
    /// SHUFFLED order (neither LIFO nor FIFO — the order most likely to
    /// defeat an implementation that only happens to work for stack- or
    /// queue-shaped drop patterns) must revoke only each dropped lease's own
    /// entry; every remaining sibling grant must survive until its own
    /// lease drops, and the key must disappear only once the last one does.
    #[test]
    fn dropping_four_concurrent_leases_in_shuffled_order_only_revokes_each_ones_own_entry() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");

        let lease_a = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("token-a"), allow_all_targets(), FAR_FUTURE),
        );
        let lease_b = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("token-b"), allow_all_targets(), FAR_FUTURE),
        );
        let lease_c = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("token-c"), allow_all_targets(), FAR_FUTURE),
        );
        let lease_d = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("token-d"), allow_all_targets(), FAR_FUTURE),
        );

        assert_eq!(
            firewall.staged_len(),
            4,
            "sanity: all four concurrent obligations must be staged before any drop"
        );

        // Shuffled order: c, a, d, b.
        drop(lease_c);
        assert_eq!(firewall.staged_len(), 3);
        assert_grant_contains(
            &firewall,
            &tenant_a,
            &user_a,
            &[handle("token-a"), handle("token-b"), handle("token-d")],
        );

        drop(lease_a);
        assert_eq!(firewall.staged_len(), 2);
        assert_grant_contains(
            &firewall,
            &tenant_a,
            &user_a,
            &[handle("token-b"), handle("token-d")],
        );

        drop(lease_d);
        assert_eq!(firewall.staged_len(), 1);
        assert_grant_contains(&firewall, &tenant_a, &user_a, &[handle("token-b")]);

        drop(lease_b);
        assert_eq!(
            firewall.staged_len(),
            0,
            "the key's set must be gone once its last entry drops"
        );
        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("attributed lookup within deadline must not error");
        assert_eq!(
            decision,
            SandboxCredentialDecision::NoGrant,
            "no entries remain staged for this key after the last lease drops"
        );
    }

    /// Expiry reclamation on read must remove only the expired entry from a
    /// key's set, not the whole key — live siblings staged under the same
    /// `(tenant_id, user_id)` must survive the reclaim.
    #[test]
    fn expiry_reclaims_one_entry_without_disturbing_live_siblings() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");

        let _short_lived = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(
                handle("expiring-token"),
                allow_all_targets(),
                Duration::ZERO,
            ),
        );
        let _long_lived = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("live-token"), allow_all_targets(), FAR_FUTURE),
        );
        std::thread::sleep(Duration::from_millis(2));

        assert_eq!(
            firewall.staged_len(),
            2,
            "sanity: both obligations staged before the expired one is reclaimed"
        );

        assert_grant_contains(&firewall, &tenant_a, &user_a, &[handle("live-token")]);
        assert_eq!(
            firewall.staged_len(),
            1,
            "the expired sibling must be reclaimed, the live one must remain"
        );
    }

    /// Cross-user isolation must still hold when a key has multiple staged
    /// entries: user B must never see any of user A's entries, even under
    /// the same tenant.
    #[test]
    fn user_b_cannot_retrieve_any_of_user_a_multiple_staged_obligations() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let user_b = user("user-b");
        let _lease_1 = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("token-1"), allow_all_targets(), FAR_FUTURE),
        );
        let _lease_2 = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("token-2"), allow_all_targets(), FAR_FUTURE),
        );

        let decision_for_b = firewall
            .authorize(Some((&tenant_a, &user_b)), far_future_deadline())
            .expect("an unstaged (tenant, user) is a decision, not a firewall error");
        assert_eq!(decision_for_b, SandboxCredentialDecision::NoGrant);

        // Sanity: user A's own lookup still resolves both entries.
        assert_grant_contains(
            &firewall,
            &tenant_a,
            &user_a,
            &[handle("token-1"), handle("token-2")],
        );
    }

    /// The firewall stops at "here is everything currently entitled" and
    /// leaves target matching to the future proxy caller (see
    /// `SandboxCredentialDecision::Grant`'s doc) — but that only works if
    /// non-empty `allowed_targets` actually survive `stage`/`authorize`
    /// unchanged. Every other test in this module stages an empty policy
    /// vector; this one pins that real policies round-trip intact.
    #[test]
    fn non_empty_target_policies_survive_stage_and_authorize_unchanged() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");
        let policies = vec![
            CredentialTargetPolicy {
                scheme: "https".to_string(),
                host: "api.example.com".to_string(),
                port: Some(443),
                path: CredentialPathPolicy::Prefix("/v1".to_string()),
                methods: vec![NetworkMethod::Get],
            },
            CredentialTargetPolicy {
                scheme: "https".to_string(),
                host: "upload.example.com".to_string(),
                port: None,
                path: CredentialPathPolicy::Exact("/upload".to_string()),
                methods: vec![NetworkMethod::Post],
            },
        ];
        let _lease = firewall.stage(
            &tenant_a,
            &user_a,
            StagedCredentialObligation::new(handle("github-token"), policies.clone(), FAR_FUTURE),
        );

        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("attributed lookup within deadline must not error");

        match decision {
            SandboxCredentialDecision::Grant(obligations) => {
                assert_eq!(obligations.len(), 1);
                assert_eq!(
                    obligations[0].allowed_targets, policies,
                    "allowed_targets must round-trip through stage/authorize unchanged"
                );
            }
            SandboxCredentialDecision::NoGrant => panic!("expected a grant"),
        }
    }

    /// Real contention exercise (not this module's usual single-thread
    /// pattern): several threads staging, authorizing, and revoking
    /// concurrently for the same `(tenant_id, user_id)` must still leave
    /// each entry's lifecycle isolated from its siblings' — the property
    /// the shuffled-drop-order test above pins sequentially, exercised here
    /// under real thread interleaving.
    #[test]
    fn concurrent_stage_authorize_and_revoke_preserve_per_entry_isolation() {
        let firewall = Arc::new(SandboxCredentialFirewall::new());
        let tenant_a = tenant("tenant-a");
        let user_a = user("user-a");

        let mut handles = Vec::new();
        for i in 0..8 {
            let firewall = Arc::clone(&firewall);
            let tenant_a = tenant_a.clone();
            let user_a = user_a.clone();
            handles.push(std::thread::spawn(move || {
                let lease = firewall.stage(
                    &tenant_a,
                    &user_a,
                    StagedCredentialObligation::new(
                        handle(&format!("token-{i}")),
                        allow_all_targets(),
                        FAR_FUTURE,
                    ),
                );
                // Concurrent reads while siblings are still staging/revoking.
                let _ = firewall.authorize(Some((&tenant_a, &user_a)), far_future_deadline());
                lease.revoke();
            }));
        }

        for handle in handles {
            handle.join().expect("worker thread must not panic");
        }

        // Every entry revoked its own lease; none should be left staged.
        let decision = firewall
            .authorize(Some((&tenant_a, &user_a)), far_future_deadline())
            .expect("attributed lookup within deadline must not error");
        assert_eq!(
            decision,
            SandboxCredentialDecision::NoGrant,
            "all 8 concurrently staged-and-revoked entries must be gone"
        );
    }
}
