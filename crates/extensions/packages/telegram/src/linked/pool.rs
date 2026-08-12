//! [`SessionPool`] — live authenticated MTProto clients, one per linked
//! credential account, built lazily and evicted without ever blocking.
//!
//! # The key is the credential account, not the user
//!
//! Entries are keyed on the host-issued [`LinkedAccountGrant`] — a
//! [`LinkedAccountRef`] **and** its `link_revision` — never on
//! `ToolCall.scope.user_id`. Three independent reasons (PROPOSAL §3.3):
//!
//! 1. `link_revision` is the only thing that evicts a live client on re-link. A
//!    user id never changes across unlink/relink, so a user-keyed pool would
//!    keep serving a session whose credential has been replaced.
//! 2. Actor resolution bottoms out at a deployment-wide fallback, so a bare
//!    user-id key would conflate every system run onto the operator's identity.
//!    Keying on the resolved credential account fails closed instead: no linked
//!    account, no entry, `AuthRequired`.
//! 3. The package's boundary rule forbids `ironclaw_auth`, so the concrete
//!    credential-account id is not nameable here — the opaque contracts-level
//!    ref is what there is.
//!
//! # Three load-bearing constraints
//!
//! * **The pool is a cache.** Every miss reconnects from the stored blob, so
//!   destroying it (reactivation, admin-config edit, upgrade) is always safe.
//! * **Lazy init only.** Binding is contractually I/O-free; nothing here
//!   connects until a call needs a connection.
//! * **The pool lock is never held across an `await`.** It is a
//!   [`std::sync::Mutex`] precisely so that holding it across one would not
//!   compile.
//!
//! # Unlink never blocks
//!
//! §4.5 wants synchronous eviction and §7.1 forbids waiting on in-flight calls.
//! The arbitration is a **per-account generation fence**: [`SessionPool::revoke`]
//! bumps it, every pooled entry checks it before each vendor RPC
//! ([`PooledSession::check_fence`]), and the entry is dropped immediately. The
//! honest residue: a write already on the wire cannot be recalled.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use grammers_tl_types as tl;
use ironclaw_extension_contracts::linked_session::{
    LinkedAccountGrant, LinkedAccountRef, LinkedSessionPortFactory,
};
use tokio::task::JoinHandle;
use tracing::debug;

use crate::linked::session_store::{IronclawSession, SessionStoreError};
use crate::linked::transport::{MtprotoConnection, VendorOpKind};
use crate::linked::{MAX_POOLED_SESSIONS, SESSION_IDLE_SWEEP_INTERVAL, SESSION_IDLE_TIMEOUT};

/// How long a best-effort `auth.logOut` may take before unlink stops waiting.
///
/// Unlink proceeds either way; a logout this call could not confirm is reported
/// explicitly unverified rather than silently assumed
/// ([`RevokeOutcome::LogoutUnverified`]).
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(10);

/// Typed pool failures.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionPoolError {
    /// The account was unlinked (or re-linked) while this work was in flight.
    /// The fence moved; the caller's connection is no longer authorized to act.
    #[error("the linked telegram account was revoked while this call was in flight")]
    Revoked,
    /// Custody holds no session for this account: it was never linked, or the
    /// blob was deleted. Detected *before* dialling, because connecting with a
    /// blank session would mint a fresh unauthenticated auth key and then fail
    /// every request with a 401 — burning a datacenter handshake to learn what
    /// the empty blob already said.
    #[error("no telegram session is stored for this account")]
    NotLinked,
    /// Session custody or the stored blob failed.
    #[error("linked telegram session custody failed")]
    Custody(#[from] SessionStoreError),
    /// The pool is full of sessions that are all in active use.
    #[error("the telegram session pool is at capacity")]
    AtCapacity,
    /// A pool lock was poisoned by a panic on another task. Process-local
    /// corruption, permanent until restart — deliberately distinct from
    /// [`SessionPoolError::AtCapacity`], which reads as "retry shortly" and
    /// would misdiagnose every call on the deployment forever.
    #[error("a telegram session pool lock is poisoned")]
    Poisoned,
    /// The deployment carries no MTProto application identity
    /// (`telegram_api_id` / `telegram_api_hash`), so no socket may be opened.
    /// Detected before custody or any dial: connecting with a zero `api_id`
    /// would burn a datacenter handshake to fail at the vendor with
    /// `API_ID_INVALID` instead of failing here with the honest reason.
    #[error(
        "the deployment has not configured its MTProto application identity \
         (telegram_api_id / telegram_api_hash)"
    )]
    NotConfigured,
}

/// What a revoke actually managed to do at the vendor.
///
/// Local deletion proceeds in both cases; the difference is what may be
/// *claimed*. `LogoutUnverified` mirrors `SecretCleanupQuarantineReason::RevokeFailed`
/// — the device may still be listed in the user's Telegram, and saying so is
/// the whole point (PROPOSAL §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RevokeOutcome {
    /// Telegram confirmed the device authorization is gone.
    LoggedOut,
    /// The vendor logout could not be confirmed. Local state is still cleared.
    LogoutUnverified,
}

/// One pooled, authenticated session.
pub(crate) struct PooledSession {
    connection: MtprotoConnection,
    session: Arc<IronclawSession>,
    fence: Arc<AtomicU64>,
    /// The fence value this entry was built at.
    generation: u64,
    last_used: Mutex<Instant>,
}

impl PooledSession {
    /// Refuse to act if the account has been unlinked or re-linked since this
    /// entry was built.
    ///
    /// **Call this before *each* vendor RPC, not only at pool lookup.** Checking
    /// only on lookup is what makes unlink either stall for a whole
    /// [`TOOL_CALL_TIMEOUT`] or let a send land after the user pressed unlink.
    pub(crate) fn check_fence(&self) -> Result<(), SessionPoolError> {
        if self.fence.load(Ordering::Acquire) == self.generation {
            Ok(())
        } else {
            Err(SessionPoolError::Revoked)
        }
    }

    /// The connection to invoke through.
    pub(crate) fn connection(&self) -> &MtprotoConnection {
        &self.connection
    }

    /// The session backing this connection.
    pub(crate) fn session(&self) -> &Arc<IronclawSession> {
        &self.session
    }

    /// Mark this entry as used, deferring idle eviction.
    pub(crate) fn touch(&self) {
        if let Ok(mut last_used) = self.last_used.lock() {
            *last_used = Instant::now();
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .map(|last_used| last_used.elapsed())
            .unwrap_or_default()
    }
}

struct SessionPoolInner {
    factory: Arc<dyn LinkedSessionPortFactory>,
    /// `None` when the deployment declared no MTProto identity; every path
    /// that would dial fails closed with [`SessionPoolError::NotConfigured`]
    /// before touching custody or a socket.
    api_id: Option<i32>,
    entries: Mutex<HashMap<LinkedAccountGrant, Arc<PooledSession>>>,
    /// Per-account generation counters. Keyed on the account **without** the
    /// revision, because a bump has to invalidate every revision's entry.
    fences: Mutex<HashMap<LinkedAccountRef, Arc<AtomicU64>>>,
    sweeper: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for SessionPoolInner {
    /// Stops the idle sweeper. The sweeper holds only a [`Weak`], so reaching
    /// here means nothing else is using the pool.
    fn drop(&mut self) {
        if let Ok(mut sweeper) = self.sweeper.lock()
            && let Some(handle) = sweeper.take()
        {
            handle.abort();
        }
    }
}

/// The package-owned pool of live linked-account sessions.
///
/// Shared by the tool half and — through the narrow [`LinkedAccountRevoker`]
/// handle only — by the device-link adapter.
pub struct SessionPool {
    inner: Arc<SessionPoolInner>,
}

impl SessionPool {
    /// Build an empty pool. Performs no I/O and spawns nothing.
    ///
    /// `api_id: None` builds a pool that refuses every acquire and reports
    /// every cold logout unverified — the fail-closed shape for a deployment
    /// without `telegram_api_id` / `telegram_api_hash`.
    pub fn new(factory: Arc<dyn LinkedSessionPortFactory>, api_id: Option<i32>) -> Self {
        Self {
            inner: Arc::new(SessionPoolInner {
                factory,
                api_id,
                entries: Mutex::new(HashMap::new()),
                fences: Mutex::new(HashMap::new()),
                sweeper: Mutex::new(None),
            }),
        }
    }

    /// A handle that can revoke and evict **one named account** and nothing
    /// else.
    ///
    /// The device-link adapter runs inside an auth flow with no capability
    /// authorization, no approval, and no origin gate, so it must not hold a
    /// handle to every user's live authenticated client (PROPOSAL §3.3). It
    /// gets this instead.
    pub(crate) fn revoker(&self) -> LinkedAccountRevoker {
        LinkedAccountRevoker {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Get the live session for an account, connecting on a miss.
    pub(crate) async fn acquire(
        &self,
        grant: &LinkedAccountGrant,
    ) -> Result<Arc<PooledSession>, SessionPoolError> {
        // Before the lookup, not just before the dial: a `None` identity can
        // never have built a live entry, and failing here keeps the error
        // ahead of any custody read.
        let api_id = self.inner.api_id.ok_or(SessionPoolError::NotConfigured)?;
        self.ensure_sweeper();

        match self.inner.lookup(grant)? {
            Lookup::Live(existing) => return Ok(existing),
            // A dead runner is a dead entry. It has to be *removed*, not merely
            // stepped over: leaving it in the map means the insert below finds
            // it again on the race check and hands the caller back the very
            // connection whose every request fails as `Dropped`.
            Lookup::Stale => self.evict(grant).await,
            Lookup::Missing => {}
        }

        // Everything from here to the re-lock happens *outside* the pool lock:
        // hydration touches custody, and holding a std mutex across that await
        // would not compile — which is the point of it being a std mutex.
        let port = self.inner.factory.open(grant);
        let session = IronclawSession::hydrate(port).await?;
        if !session.holds_authorization_key() {
            return Err(SessionPoolError::NotLinked);
        }
        let fence = self.inner.fence_for(grant.account())?;
        let generation = fence.load(Ordering::Acquire);
        let candidate = Arc::new(PooledSession {
            connection: MtprotoConnection::open(Arc::clone(&session), api_id),
            session,
            fence,
            generation,
            last_used: Mutex::new(Instant::now()),
        });

        let evicted = {
            let mut entries = self.inner.lock_entries()?;
            // Another task may have won the race while we were hydrating. Its
            // entry wins — if it is still usable; ours is dropped, which aborts
            // the runner it spawned.
            if let Some(existing) = entries.get(grant)
                && existing.connection.is_runner_alive()
            {
                let existing = Arc::clone(existing);
                existing.touch();
                return Ok(existing);
            }
            let evicted = if entries.len() >= MAX_POOLED_SESSIONS {
                Some(evict_least_recently_used(&mut entries).ok_or(SessionPoolError::AtCapacity)?)
            } else {
                None
            };
            entries.insert(grant.clone(), Arc::clone(&candidate));
            evicted
        };

        if let Some(evicted) = evicted {
            retire(evicted).await;
        }
        Ok(candidate)
    }

    /// Drop one account's live session, flushing it first.
    pub(crate) async fn evict(&self, grant: &LinkedAccountGrant) {
        let removed = {
            let Ok(mut entries) = self.inner.entries.lock() else {
                return;
            };
            entries.remove(grant)
        };
        if let Some(removed) = removed {
            retire(removed).await;
        }
    }

    /// Start the idle sweeper the first time the pool is actually used.
    ///
    /// Lazily, so a pool nobody touches costs no task; and holding only a
    /// [`Weak`], so the task cannot keep the pool alive past its owner.
    fn ensure_sweeper(&self) {
        let Ok(mut sweeper) = self.inner.sweeper.lock() else {
            return;
        };
        if sweeper.is_some() {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        *sweeper = Some(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SESSION_IDLE_SWEEP_INTERVAL);
            loop {
                ticker.tick().await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let idle = inner.take_idle_entries();
                drop(inner);
                for entry in idle {
                    debug!("evicting an idle telegram session");
                    retire(entry).await;
                }
            }
        }));
    }
}

impl SessionPoolInner {
    fn lock_entries(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, HashMap<LinkedAccountGrant, Arc<PooledSession>>>,
        SessionPoolError,
    > {
        self.entries.lock().map_err(|_| SessionPoolError::Poisoned)
    }

    fn lookup(&self, grant: &LinkedAccountGrant) -> Result<Lookup, SessionPoolError> {
        let entries = self.lock_entries()?;
        let Some(existing) = entries.get(grant) else {
            return Ok(Lookup::Missing);
        };
        if !existing.connection.is_runner_alive() {
            return Ok(Lookup::Stale);
        }
        existing.check_fence()?;
        existing.touch();
        Ok(Lookup::Live(Arc::clone(existing)))
    }

    fn fence_for(&self, account: &LinkedAccountRef) -> Result<Arc<AtomicU64>, SessionPoolError> {
        let mut fences = self.fences.lock().map_err(|_| SessionPoolError::Poisoned)?;
        Ok(Arc::clone(
            fences
                .entry(account.clone())
                .or_insert_with(|| Arc::new(AtomicU64::new(0))),
        ))
    }

    fn take_idle_entries(&self) -> Vec<Arc<PooledSession>> {
        let Ok(mut entries) = self.entries.lock() else {
            return Vec::new();
        };
        let stale = entries
            .iter()
            .filter(|(_, entry)| {
                entry.idle_for() >= SESSION_IDLE_TIMEOUT || !entry.connection.is_runner_alive()
            })
            .map(|(grant, _)| grant.clone())
            .collect::<Vec<_>>();
        stale
            .into_iter()
            .filter_map(|grant| entries.remove(&grant))
            .collect()
    }
}

/// What a pool lookup found.
enum Lookup {
    /// A usable entry.
    Live(Arc<PooledSession>),
    /// An entry whose runner has ended. Must be removed before rebuilding.
    Stale,
    /// Nothing pooled for this grant.
    Missing,
}

/// Revoke and evict exactly one account.
///
/// Deliberately not a trait: the containment claim is "this handle can address
/// one account and nothing else", and a concrete handle over a [`Weak`] says
/// that more plainly than a one-implementor trait would.
pub(crate) struct LinkedAccountRevoker {
    inner: Weak<SessionPoolInner>,
}

impl LinkedAccountRevoker {
    /// Tear down an established link.
    ///
    /// Order matters: bump the fence **first** so in-flight work fails at its
    /// next RPC, then drop the entries, then attempt the vendor logout. Doing
    /// the logout first would leave a window in which a queued call rides a
    /// connection whose authorization is already gone.
    pub(crate) async fn revoke(&self, grant: &LinkedAccountGrant) -> RevokeOutcome {
        let Some(inner) = self.inner.upgrade() else {
            // The pool is gone, so there is no live client to log out. The
            // account's durable state is the host's to delete either way.
            return RevokeOutcome::LogoutUnverified;
        };

        if let Ok(fences) = inner.fences.lock()
            && let Some(fence) = fences.get(grant.account())
        {
            fence.fetch_add(1, Ordering::AcqRel);
        }

        let removed = {
            let Ok(mut entries) = inner.entries.lock() else {
                return RevokeOutcome::LogoutUnverified;
            };
            let doomed = entries
                .keys()
                .filter(|key| key.account() == grant.account())
                .cloned()
                .collect::<Vec<_>>();
            doomed
                .into_iter()
                .filter_map(|key| entries.remove(&key))
                .collect::<Vec<_>>()
        };

        let mut outcome = RevokeOutcome::LogoutUnverified;
        for entry in &removed {
            if log_out(entry.connection()).await {
                outcome = RevokeOutcome::LoggedOut;
            }
        }

        if removed.is_empty() {
            // Nothing was pooled, so build a throwaway connection purely to end
            // the device authorization. Without this, unlinking an account that
            // happened to be cold would leave the device listed in Telegram.
            outcome = revoke_cold(&inner, grant).await;
        }
        outcome
    }
}

/// Log out an account that has no live pooled session.
async fn revoke_cold(inner: &SessionPoolInner, grant: &LinkedAccountGrant) -> RevokeOutcome {
    // Without an MTProto identity no dial is possible, so the vendor logout
    // can never be verified. Say so immediately instead of burning a custody
    // read and a doomed handshake.
    let Some(api_id) = inner.api_id else {
        return RevokeOutcome::LogoutUnverified;
    };
    let port = inner.factory.open(grant);
    let Ok(session) = IronclawSession::hydrate(port).await else {
        return RevokeOutcome::LogoutUnverified;
    };
    let connection = MtprotoConnection::open(session, api_id);
    if log_out(&connection).await {
        RevokeOutcome::LoggedOut
    } else {
        RevokeOutcome::LogoutUnverified
    }
}

/// Best-effort `auth.logOut`, bounded.
///
/// Sent as a [`VendorOpKind::Write`] on purpose: it is not idempotent in the
/// sense that matters — a repeat after an unknown outcome tells us nothing new
/// and costs another flood-budget call.
async fn log_out(connection: &MtprotoConnection) -> bool {
    let call = connection.invoke(&tl::functions::auth::LogOut {}, VendorOpKind::Write);
    match tokio::time::timeout(LOGOUT_TIMEOUT, call).await {
        Ok(Ok(_)) => true,
        Ok(Err(error)) => {
            // An already-dead authorization is a successful outcome, not a
            // failure: the device is gone, which is what was asked for.
            let already_gone = matches!(
                error.rpc_name(),
                Some("AUTH_KEY_UNREGISTERED" | "SESSION_REVOKED" | "USER_DEACTIVATED")
            );
            if !already_gone {
                debug!("telegram logout could not be confirmed");
            }
            already_gone
        }
        Err(_) => {
            debug!("telegram logout timed out");
            false
        }
    }
}

/// Flush a session's pending write-through, then drop it.
///
/// The flush is what `MtprotoConnection::Drop` structurally cannot do, so every
/// deliberate removal path routes through here.
async fn retire(entry: Arc<PooledSession>) {
    if let Err(error) = entry.session().flush().await {
        debug!(?error, "flushing an evicted telegram session failed");
    }
    drop(entry);
}

/// Remove the least-recently-used entry, returning it for retirement.
///
/// Capacity pressure evicts rather than refusing: the pool is a cache and a
/// miss reconnects, so eviction costs latency, not correctness. `None` means
/// the map was empty, which the caller reports as capacity exhaustion.
fn evict_least_recently_used(
    entries: &mut HashMap<LinkedAccountGrant, Arc<PooledSession>>,
) -> Option<Arc<PooledSession>> {
    let victim = entries
        .iter()
        .max_by_key(|(_, entry)| entry.idle_for())
        .map(|(grant, _)| grant.clone())?;
    entries.remove(&victim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linked::TOOL_CALL_TIMEOUT;

    fn grant(account: &str, revision: u64) -> LinkedAccountGrant {
        LinkedAccountGrant::new(
            LinkedAccountRef::new(account).expect("valid linked-account ref"),
            revision,
        )
    }

    #[test]
    fn the_pool_key_separates_link_revisions() {
        // The invariant the whole cache-key argument rests on: a re-link must
        // not resolve to the previous revision's entry.
        assert_ne!(grant("acct-1", 1), grant("acct-1", 2));
        assert_ne!(grant("acct-1", 1), grant("acct-2", 1));
        assert_eq!(grant("acct-1", 1), grant("acct-1", 1));
    }

    #[test]
    fn a_bumped_fence_refuses_an_entry_built_before_it() {
        let fence = Arc::new(AtomicU64::new(0));
        let generation = fence.load(Ordering::Acquire);
        assert_eq!(generation, 0);
        fence.fetch_add(1, Ordering::AcqRel);
        assert_ne!(fence.load(Ordering::Acquire), generation);
    }

    /// A factory that panics if custody is ever consulted — proof a
    /// fail-closed path never got that far.
    struct UntouchableFactory;

    impl LinkedSessionPortFactory for UntouchableFactory {
        fn open(
            &self,
            _grant: &LinkedAccountGrant,
        ) -> std::sync::Arc<dyn ironclaw_extension_contracts::linked_session::LinkedSessionPort>
        {
            panic!("custody must not be touched on a fail-closed path");
        }
    }

    #[test]
    fn a_poisoned_pool_lock_reports_poisoned_not_at_capacity() {
        // Before the fix this surfaced as `AtCapacity` — a "retry shortly"
        // answer to permanent process-local corruption.
        let pool = SessionPool::new(Arc::new(UntouchableFactory), Some(1));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = pool.inner.entries.lock().expect("first lock");
            panic!("poison the entries lock");
        }));
        assert!(matches!(
            pool.inner.lock_entries(),
            Err(SessionPoolError::Poisoned)
        ));
    }

    #[tokio::test]
    async fn acquire_without_an_app_identity_fails_closed_before_custody() {
        // Before the fix this dialed Telegram with `api_id = 0` and failed at
        // the vendor; the panicking factory proves custody is never touched.
        let pool = SessionPool::new(Arc::new(UntouchableFactory), None);
        match pool.acquire(&grant("acct-1", 1)).await {
            Err(SessionPoolError::NotConfigured) => {}
            Err(other) => panic!("expected NotConfigured, got {other:?}"),
            Ok(_) => panic!("a deployment without an identity must fail closed"),
        }
    }

    #[tokio::test]
    async fn cold_revoke_without_an_app_identity_reports_logout_unverified() {
        // The unlink still proceeds locally; what may be *claimed* is capped.
        let inner = SessionPoolInner {
            factory: Arc::new(UntouchableFactory),
            api_id: None,
            entries: Mutex::new(HashMap::new()),
            fences: Mutex::new(HashMap::new()),
            sweeper: Mutex::new(None),
        };
        assert_eq!(
            revoke_cold(&inner, &grant("acct-1", 1)).await,
            RevokeOutcome::LogoutUnverified
        );
    }

    #[test]
    fn the_logout_budget_is_shorter_than_a_tool_call() {
        // Unlink must not be able to out-wait the call budget it is racing.
        assert!(LOGOUT_TIMEOUT < TOOL_CALL_TIMEOUT);
    }
}
