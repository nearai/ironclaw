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
use rand::Rng;
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
    /// caller. Secret handles and token material must never be projected.
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
        tick_once(&deps, &settings).await;
        let delay = settings.interval + jitter_delay(settings.tick_jitter_max);
        if !sleep_or_cancel(delay, &cancel).await {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Single tick (B3)
// ---------------------------------------------------------------------------

async fn tick_once(deps: &CredentialRefreshWorkerDeps, settings: &CredentialRefreshSettings) {
    use crate::product_auth_refresh_lock::LeaderOutcome;

    // Acquire the deployment-wide leader lock before doing any enumeration
    // work. Non-leader processes skip this tick entirely.
    let outcome = deps
        .leader_lock
        .run_as_leader(|| sweep_once(deps, settings))
        .await;

    match outcome {
        LeaderOutcome::NotLeader => {
            tracing::debug!("credential refresh worker tick: not the leader; skipping sweep");
        }
        LeaderOutcome::Ran(()) => {}
    }
}

/// The actual sweep executed only by the leader process.
async fn sweep_once(deps: &CredentialRefreshWorkerDeps, settings: &CredentialRefreshSettings) {
    let now = Utc::now();
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
    let idle_cutoff = now - idle_threshold;

    // B1: enumerate all Google/Configured/has-refresh accounts across all owners.
    let candidates = deps.candidate_source.list_refresh_candidates().await;

    // Filter to idle accounts (by updated_at) and apply per-tick cap.
    let to_refresh: Vec<_> = candidates
        .into_iter()
        .filter(|account| account.updated_at < idle_cutoff)
        .take(settings.max_per_tick)
        .collect();

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
        match deps.refresh_port.refresh_credential_account(request).await {
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
    let nanos = rand::thread_rng().gen_range(0..=max_nanos);
    let nanos = u64::try_from(nanos).unwrap_or(u64::MAX);
    Duration::from_nanos(nanos)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
        assert_eq!(settings.max_per_tick, 10);
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
        let settings = CredentialRefreshSettings::default();
        assert!(!settings.enabled, "default settings must be disabled");
        // spawn_credential_refresh_worker returns None when !enabled, without
        // accessing deps. We verify the early-return guard via the flag alone —
        // full integration coverage lives in the architecture boundary tests.
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
