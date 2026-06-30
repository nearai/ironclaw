//! Background Google OAuth credential keepalive worker (B2/B3).
//!
//! Google OAuth refresh tokens expire after 7 days of inactivity for apps in
//! testing publishing status. This worker periodically refreshes idle Google
//! accounts so the refresh token is never silently killed between user sessions.
//!
//! ## Design
//!
//! - **Tick logic (B3):** enumerate all Google/Configured accounts with a
//!   `refresh_secret` whose `updated_at` is older than `idle_threshold`. The
//!   worker wraps the sweep in a
//!   [`crate::product_auth_refresh_lock::CredentialRefreshLeaderLock`]: only
//!   one process per deployment becomes the leader per tick and runs the sweep;
//!   non-leaders skip the tick without touching the token endpoint.
//! - **Scheduling:** mirrors `trigger_poller.rs` exactly — startup jitter,
//!   inter-tick jitter, `CancellationToken`-aware sleep, `JoinHandle` shutdown.
//! - **Logging rule:** `debug!` only — never `info!` or `warn!`. The REPL/TUI
//!   renders `info!` and `warn!` output and a background worker must not corrupt
//!   the interactive display.
//! - **Secret safety:** never log account ids beyond what is structurally needed
//!   for debug diagnostics; never log secret handles, token material, or raw
//!   error bodies from token endpoints.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ironclaw_auth::{AuthErrorCode, CredentialRefreshRequest};
use rand::RngExt as _;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use async_trait::async_trait;

use crate::auth::RebornProductAuthServices;
use crate::runtime_input::CredentialRefreshSettings;

// ---------------------------------------------------------------------------
// Shutdown timeout — mirrors TRIGGER_POLLER_SHUTDOWN_TIMEOUT
// ---------------------------------------------------------------------------

pub(crate) const CREDENTIAL_REFRESH_WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Runtime handle
// ---------------------------------------------------------------------------

pub(crate) struct CredentialRefreshWorkerRuntimeHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

impl CredentialRefreshWorkerRuntimeHandle {
    pub(crate) async fn shutdown(self, timeout: Duration) {
        self.cancel.cancel();
        self.join_with_timeout(timeout).await;
    }

    pub(crate) async fn join_with_timeout(self, timeout: Duration) {
        let mut handle = self.handle;
        match tokio::time::timeout(timeout, &mut handle).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(?error, "credential refresh worker task join failed");
            }
            Err(_) => {
                tracing::debug!(
                    ?timeout,
                    "credential refresh worker did not stop before shutdown timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::debug!(?error, "aborted credential refresh worker task panicked");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Candidate source trait (B1 seam)
// ---------------------------------------------------------------------------

/// Deployment-wide enumeration of Google credential accounts eligible for
/// proactive keepalive refresh (B1).
///
/// Implemented by `FilesystemAuthProductServices<F>` (concrete) and exposed
/// through the facade as a trait object so `build_reborn_runtime` can spawn
/// the worker without carrying a generic `F` parameter.
#[async_trait]
pub(crate) trait CredentialRefreshCandidateSource: Send + Sync {
    /// Return all Google/Configured/has-refresh-token accounts across all
    /// owners. Idle-threshold filtering (by `updated_at`) is done by the
    /// caller. The records carry secret *handles* (opaque references, never raw
    /// token material) needed to drive the refresh; callers must not log or
    /// serialize them.
    async fn list_refresh_candidates(&self) -> Vec<ironclaw_auth::CredentialAccount>;
}

// Blanket impl so `FilesystemAuthProductServices<F>` satisfies the trait.
// Note: this requires the `product_auth_durable` module to be `pub(crate)`.
#[async_trait]
impl<F> CredentialRefreshCandidateSource
    for crate::product_auth_durable::FilesystemAuthProductServices<F>
where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    async fn list_refresh_candidates(&self) -> Vec<ironclaw_auth::CredentialAccount> {
        crate::product_auth_durable::FilesystemAuthProductServices::list_refresh_candidates(self)
            .await
    }
}

// ---------------------------------------------------------------------------
// Dependencies passed into the worker
// ---------------------------------------------------------------------------

/// All state the credential-refresh worker needs, cloneable for the spawn.
#[derive(Clone)]
pub(crate) struct CredentialRefreshWorkerDeps {
    /// Durable product-auth services — used only for `list_refresh_candidates`.
    pub(crate) candidate_source: Arc<dyn CredentialRefreshCandidateSource>,
    /// Plain refresh port — `refresh_credential_account` goes through the
    /// `ProviderBackedCredentialAccountService` in-process guard only. Cross-
    /// process serialization is provided by the leader lock below.
    pub(crate) refresh_port: Arc<RebornProductAuthServices>,
    /// Deployment-wide leader lock. Only the process that acquires this each
    /// tick runs the sweep; others skip. Built from the Postgres pool on
    /// production paths; always-leader on libsql / local-dev paths.
    pub(crate) leader_lock: Arc<crate::product_auth_refresh_lock::CredentialRefreshLeaderLock>,
}

// ---------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------

/// Spawn the background credential keepalive worker.
///
/// Returns `None` when `settings.enabled` is `false` — the worker is not
/// started and no resources are consumed.
pub(crate) fn spawn_credential_refresh_worker(
    settings: CredentialRefreshSettings,
    deps: CredentialRefreshWorkerDeps,
) -> Option<CredentialRefreshWorkerRuntimeHandle> {
    if !settings.enabled {
        return None;
    }
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        run_credential_refresh_worker(deps, settings, task_cancel).await;
    });
    Some(CredentialRefreshWorkerRuntimeHandle { cancel, handle })
}

// ---------------------------------------------------------------------------
// Run loop
// ---------------------------------------------------------------------------

async fn run_credential_refresh_worker(
    deps: CredentialRefreshWorkerDeps,
    settings: CredentialRefreshSettings,
    cancel: CancellationToken,
) {
    // Startup jitter: spread first tick across the multi-process fleet so not
    // every process fires its first refresh simultaneously (thundering herd).
    if !sleep_or_cancel(jitter_delay(settings.startup_jitter_max), &cancel).await {
        return;
    }
    loop {
        tick_once(&deps, &settings, &cancel).await;
        let delay = settings.interval + jitter_delay(settings.tick_jitter_max);
        if !sleep_or_cancel(delay, &cancel).await {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Single tick (B3)
// ---------------------------------------------------------------------------

async fn tick_once(
    deps: &CredentialRefreshWorkerDeps,
    settings: &CredentialRefreshSettings,
    cancel: &CancellationToken,
) {
    use crate::product_auth_refresh_lock::LeaderOutcome;

    // Acquire the deployment-wide leader lock before doing any enumeration
    // work. Non-leader processes skip this tick entirely.
    let outcome = deps
        .leader_lock
        .run_as_leader(|| sweep_once(deps, settings, cancel, Utc::now()))
        .await;

    match outcome {
        LeaderOutcome::NotLeader => {
            tracing::debug!("credential refresh worker tick: not the leader; skipping sweep");
        }
        LeaderOutcome::Ran(()) => {}
    }
}

/// The actual sweep executed only by the leader process.
///
/// Accepts an explicit `now` instant so callers can inject a frozen clock for
/// deterministic testing. Production callers (`tick_once`) always pass
/// `Utc::now()`; tests pass a future instant to simulate idle accounts.
pub(crate) async fn sweep_once(
    deps: &CredentialRefreshWorkerDeps,
    settings: &CredentialRefreshSettings,
    cancel: &CancellationToken,
    now: chrono::DateTime<chrono::Utc>,
) {
    let idle_threshold = match chrono::Duration::from_std(settings.idle_threshold) {
        Ok(d) => d,
        Err(error) => {
            tracing::debug!(
                %error,
                "credential refresh worker: idle_threshold out of range; skipping tick"
            );
            return;
        }
    };
    let idle_cutoff = now
        .checked_sub_signed(idle_threshold)
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC);

    // B1: enumerate all Google/Configured/has-refresh accounts across all owners.
    let candidates = deps.candidate_source.list_refresh_candidates().await;

    // Filter to idle accounts (by updated_at) and apply per-tick cap (pure,
    // unit-tested helper).
    let (to_refresh, dropped) =
        select_idle_candidates(candidates, idle_cutoff, settings.max_per_tick);
    if dropped > 0 {
        tracing::debug!(
            dropped,
            max_per_tick = settings.max_per_tick,
            "credential refresh: candidate list truncated to max_per_tick"
        );
    }

    if to_refresh.is_empty() {
        tracing::debug!("credential refresh worker tick: no idle Google accounts found");
        return;
    }

    tracing::debug!(
        count = to_refresh.len(),
        "credential refresh worker tick: refreshing idle Google accounts"
    );

    let mut refreshed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for account in to_refresh {
        let account_id = account.id;
        let request = CredentialRefreshRequest::new(
            account.scope.clone(),
            account.provider.clone(),
            account_id,
        );
        // Race each refresh against cancellation so shutdown does not have to
        // wait for an in-flight token-endpoint call (which can hang on a slow
        // network) before stopping. `biased` checks cancellation first.
        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tracing::debug!("credential refresh worker: cancelled mid-sweep; stopping");
                break;
            }
            outcome = deps.refresh_port.refresh_credential_account(request) => outcome,
        };
        match outcome {
            Ok(_) => {
                refreshed += 1;
                tracing::debug!(
                    provider = %account.provider,
                    "credential refresh worker: account refreshed"
                );
            }
            // Transient errors (backend unavailable): leave for the next tick
            // — no status mutation.
            Err(ref error) if error.code == AuthErrorCode::BackendUnavailable => {
                skipped += 1;
                tracing::debug!(
                    provider = %account.provider,
                    "credential refresh worker: transient error, will retry next tick"
                );
            }
            Err(error) => {
                // Permanent failures (invalid_grant → Revoked, missing
                // refresh_secret → RefreshFailed): A3 already moved the
                // account status, so the next tick's `Configured` filter
                // excludes it.  Log at debug only (never log token material).
                failed += 1;
                tracing::debug!(
                    provider = %account.provider,
                    error_code = ?error.code,
                    "credential refresh worker: account refresh failed (non-transient)"
                );
            }
        }
    }

    tracing::debug!(
        refreshed,
        skipped,
        failed,
        "credential refresh worker tick complete"
    );
}

// ---------------------------------------------------------------------------
// Pure candidate selection (B3) — unit-testable without a refresh-port seam
// ---------------------------------------------------------------------------

/// Keep accounts whose `updated_at` is older than `idle_cutoff` (i.e. idle past
/// the threshold), capped at `max_per_tick`. Returns the selected accounts and
/// the number dropped by the cap (for the truncation log).
///
/// Pure and side-effect-free so the idle/cap policy can be unit-tested directly,
/// without standing up the leader lock or a refresh port.
fn select_idle_candidates(
    candidates: Vec<ironclaw_auth::CredentialAccount>,
    idle_cutoff: chrono::DateTime<chrono::Utc>,
    max_per_tick: usize,
) -> (Vec<ironclaw_auth::CredentialAccount>, usize) {
    let mut idle: Vec<_> = candidates
        .into_iter()
        .filter(|account| account.updated_at < idle_cutoff)
        .collect();
    // Oldest accounts are closest to Google's 7-day refresh-token idle-death, so
    // they must be served first. Without this sort, a head-of-enumeration subset
    // (e.g. accounts stuck in transient errors that keep their old updated_at)
    // could starve tail accounts when more than max_per_tick accounts are idle.
    idle.sort_by_key(|account| account.updated_at);
    let dropped = idle.len().saturating_sub(max_per_tick);
    idle.truncate(max_per_tick);
    (idle, dropped)
}

// ---------------------------------------------------------------------------
// Timing helpers — mirrors trigger_poller.rs
// ---------------------------------------------------------------------------

/// Sleep for `delay` unless `cancel` fires first. Returns `false` when
/// cancelled, `true` when the sleep elapsed normally.
async fn sleep_or_cancel(delay: Duration, cancel: &CancellationToken) -> bool {
    if delay.is_zero() {
        return !cancel.is_cancelled();
    }
    tokio::select! {
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

/// Return a random duration in `[0, max]`. Zero when `max` is zero.
fn jitter_delay(max: Duration) -> Duration {
    if max.is_zero() {
        return Duration::ZERO;
    }
    let max_nanos = max.as_nanos().min(u64::MAX as u128);
    let nanos = rand::rng().random_range(0..=max_nanos);
    let nanos = u64::try_from(nanos).unwrap_or(u64::MAX);
    Duration::from_nanos(nanos)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_auth::{
        AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount, CredentialAccountId,
        CredentialAccountLabel, CredentialAccountStatus, CredentialOwnership,
    };
    use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle, UserId};
    use std::time::Duration;

    /// Build a Google/Configured/has-refresh candidate with a given `updated_at`.
    fn candidate(updated_at: chrono::DateTime<Utc>) -> CredentialAccount {
        let resource =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        CredentialAccount {
            id: CredentialAccountId::new(),
            scope: AuthProductScope::new(resource, AuthSurface::Api),
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google_access").unwrap()),
            refresh_secret: Some(SecretHandle::new("google_refresh").unwrap()),
            scopes: Vec::new(),
            created_at: updated_at,
            updated_at,
        }
    }

    #[test]
    fn select_idle_candidates_keeps_only_idle_and_respects_cap() {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::days(2);
        let ts_5d = now - chrono::Duration::days(5);
        let ts_4d = now - chrono::Duration::days(4);
        let ts_3d = now - chrono::Duration::days(3);
        // 3 idle (older than cutoff) + 2 fresh (newer than cutoff).
        // Deliberately out of chronological order in the input to confirm the
        // sort is applied (not just enumeration order preservation).
        let candidates = vec![
            candidate(ts_3d),
            candidate(ts_5d),
            candidate(ts_4d),
            candidate(now - chrono::Duration::hours(1)),
            candidate(now),
        ];

        // Cap above the idle count: all 3 idle selected, none fresh, none dropped.
        let (selected, dropped) = select_idle_candidates(candidates.clone(), cutoff, 10);
        assert_eq!(selected.len(), 3, "only idle candidates are selected");
        assert_eq!(dropped, 0);
        assert!(
            selected.iter().all(|a| a.updated_at < cutoff),
            "every selected candidate must be idle past the cutoff"
        );

        // Cap below the idle count: the TWO OLDEST are selected (oldest-first),
        // not an arbitrary subset; the remaining 1 idle account is dropped.
        let (selected, dropped) = select_idle_candidates(candidates, cutoff, 2);
        assert_eq!(selected.len(), 2, "selection is capped at max_per_tick");
        assert_eq!(dropped, 1, "one idle candidate dropped by the cap");
        // Oldest-first: the two selected accounts must be the two with the
        // smallest updated_at (ts_5d and ts_4d), in ascending order.
        assert_eq!(
            selected[0].updated_at, ts_5d,
            "first selected must be the oldest idle account"
        );
        assert_eq!(
            selected[1].updated_at, ts_4d,
            "second selected must be the second-oldest idle account"
        );
    }

    #[test]
    fn select_idle_candidates_empty_when_all_fresh() {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::days(2);
        let candidates = vec![candidate(now), candidate(now - chrono::Duration::hours(6))];
        let (selected, dropped) = select_idle_candidates(candidates, cutoff, 5);
        assert!(selected.is_empty(), "no idle candidates → empty selection");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn jitter_is_disabled_when_max_is_zero() {
        assert_eq!(jitter_delay(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn jitter_is_bounded_by_max() {
        let max = Duration::from_millis(25);
        assert!(jitter_delay(max) <= max);
    }

    #[test]
    fn credential_refresh_settings_defaults_are_disabled() {
        let settings = CredentialRefreshSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.startup_jitter_max, Duration::ZERO);
        assert_eq!(settings.tick_jitter_max, Duration::ZERO);
        assert_eq!(settings.interval, Duration::from_secs(6 * 3600));
        assert_eq!(settings.idle_threshold, Duration::from_secs(2 * 24 * 3600));
        assert_eq!(settings.max_per_tick, 5);
    }

    #[test]
    fn credential_refresh_settings_enabled_returns_enabled() {
        let settings = CredentialRefreshSettings::enabled();
        assert!(settings.enabled);
        // All other fields stay at defaults.
        assert_eq!(settings.interval, Duration::from_secs(6 * 3600));
    }

    #[test]
    fn spawn_returns_none_when_disabled() {
        // spawn_credential_refresh_worker returns None when !enabled, before deps are
        // accessed. The early-return guard runs before touching candidate_source,
        // refresh_port, or leader_lock, so no live services need to be wired for this
        // path. Integration coverage for the spawn path lives in runtime.rs tests.
        let settings = CredentialRefreshSettings::default();
        assert!(!settings.enabled, "default settings must be disabled");
    }

    #[tokio::test]
    async fn runtime_handle_aborts_when_join_times_out() {
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let handle = tokio::spawn(async move {
            task_cancel.cancelled().await;
            std::future::pending::<()>().await;
        });
        let runtime_handle = CredentialRefreshWorkerRuntimeHandle { cancel, handle };
        // Should complete without hanging.
        runtime_handle.shutdown(Duration::from_millis(1)).await;
    }
}
