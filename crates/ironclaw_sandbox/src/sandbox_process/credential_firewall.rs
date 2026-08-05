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

use ironclaw_host_api::ids::{SecretHandle, TenantId, UserId};
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
        //
        // `try_lock`, not `lock`: formatting must never block on the same
        // mutex `stage`/`revoke`/`authorize` hold — mirrors
        // `RuntimeSecretInjectionStore`'s `Debug` in `obligations.rs`, which
        // also never locks its own store's mutex just to format. Under
        // contention (or a poisoned lock) the count is reported as
        // unavailable rather than blocking or panicking.
        let staged_keys: Option<usize> = match self.staged.try_lock() {
            Ok(staged) => Some(staged.len()),
            Err(std::sync::TryLockError::Poisoned(poison)) => Some(poison.into_inner().len()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        formatter
            .debug_struct("SandboxCredentialFirewall")
            .field("staged_keys", &staged_keys)
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
        let key = StagingKey::new(tenant_id, user_id);
        self.lock()
            .entry(key.clone())
            .or_default()
            .insert(entry_id, obligation);
        StagedObligationLease {
            firewall: Arc::clone(self),
            key,
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
    fn revoke(&self, key: &StagingKey, entry_id: StagedEntryId) {
        let mut staged = self.lock();
        if let Some(entries) = staged.get_mut(key) {
            entries.remove(&entry_id);
            if entries.is_empty() {
                staged.remove(key);
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
        // Re-check once more before materializing the grant: `retain` and
        // the clone below are cheap in the common case but still do real
        // work while `staged`'s lock is held, and the doc above promises the
        // deadline bounds the *whole call* — a `Grant` must never be handed
        // back once the deadline has already passed, even if only this last
        // stretch of in-lock work crossed it.
        if Instant::now() >= deadline {
            return Err(SandboxCredentialFirewallError::LookupTimedOut);
        }
        let live: Vec<StagedCredentialObligation> = entries.values().cloned().collect();
        // Re-check once more after materializing `live`: the clone above is
        // itself real work done while `staged`'s lock is held (and the
        // thread can be descheduled mid-clone), so a deadline that was still
        // valid at the previous check can have passed by the time this
        // returns — the doc above promises the deadline bounds the *whole
        // call*, and a `Grant` must never be handed back once it has.
        if Instant::now() >= deadline {
            return Err(SandboxCredentialFirewallError::LookupTimedOut);
        }
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
    /// The same key `stage()` inserted this entry under — stored once and
    /// reused by `revoke`, rather than kept as separate `tenant_id`/`user_id`
    /// fields that would have to be re-assembled into a `StagingKey` again
    /// at revoke time.
    key: StagingKey,
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
            self.firewall.revoke(&self.key, self.entry_id);
        }
    }
}

impl Drop for StagedObligationLease {
    fn drop(&mut self) {
        self.revoke_inner();
    }
}

#[cfg(test)]
mod tests;
