//! Engine-owned credential keepalive sweep.
//!
//! Some vendors expire refresh tokens after a period of inactivity (an idle
//! lifetime). On-demand refresh at injection time cannot prevent idle death —
//! a credential that is never injected is never refreshed — so a vendor may
//! declare its idle lifetime in its auth recipe
//! (`refresh.keepalive_idle_seconds`, a vendor lifetime constraint), and the
//! engine executes one generic, vendor-blind background sweep for every
//! declaring vendor.
//!
//! ## Design
//!
//! - **Recipe-driven:** only accounts whose vendor's active recipe declares
//!   `keepalive_idle_seconds` are ever swept. Vendors without the field opt
//!   out entirely.
//! - **Proactive half-life trigger:** an account becomes due once its idle age
//!   (`now - updated_at`) reaches **half** the declared lifetime. Sweeping
//!   only past the full lifetime would refresh tokens the vendor already
//!   killed; half-life leaves headroom for downtime and deployment gaps while
//!   staying entirely derived from the declared vendor constraint.
//! - **Soonest-death-first:** due accounts are refreshed in ascending
//!   projected-death order (`updated_at + lifetime`) so the per-tick cap can
//!   never starve the accounts closest to expiry.
//! - **Leader lock:** one process per deployment runs the sweep per tick
//!   ([`KeepaliveLeaderLock`], implemented by the composition layer);
//!   non-leaders skip without touching any token endpoint.
//! - **Scheduling:** startup jitter, inter-tick jitter,
//!   `CancellationToken`-aware sleep, `JoinHandle` shutdown with abort.
//! - **Logging rule:** `debug!` only — never `info!` or `warn!`. The REPL/TUI
//!   renders `info!`/`warn!` and a background job must not corrupt the
//!   interactive display.
//! - **Secret safety:** never log secret handles, token material, or raw
//!   vendor response bodies.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::credential::{
    CredentialAccount, CredentialAccountService, CredentialAccountStatus, CredentialOwnership,
    CredentialRefreshReport, CredentialRefreshRequest, ProviderBackedCredentialAccountService,
};
use crate::engine::AuthRecipeResolver;
use crate::error::AuthProductError;
use ironclaw_host_api::ids::ExtensionId;

/// How long shutdown waits for an in-flight sweep before aborting the task.
pub const KEEPALIVE_SWEEP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Deployment-level scheduling knobs for the keepalive sweep. The *threshold*
/// is deliberately absent: idle lifetimes are per-vendor recipe data
/// (`refresh.keepalive_idle_seconds`), never a deployment setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepaliveSweepSettings {
    /// Whether the sweep is enabled. Defaults to `false`; use
    /// [`KeepaliveSweepSettings::enabled`] to turn on.
    pub enabled: bool,
    /// How often the sweep wakes and looks for due accounts. Default: 6 hours.
    pub interval: Duration,
    /// Maximum random jitter applied once at startup before the first tick,
    /// spreading first ticks across a multi-process fleet. Default: zero.
    pub startup_jitter_max: Duration,
    /// Maximum random jitter appended to each inter-tick sleep. Default: zero.
    pub tick_jitter_max: Duration,
    /// Maximum number of due accounts refreshed per tick. Bounds a single
    /// sweep so a large backfill cannot overload a token endpoint. Default: 5.
    pub max_per_tick: usize,
}

impl Default for KeepaliveSweepSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: Duration::from_secs(6 * 3600),
            startup_jitter_max: Duration::ZERO,
            tick_jitter_max: Duration::ZERO,
            max_per_tick: 5,
        }
    }
}

impl KeepaliveSweepSettings {
    /// Settings with the sweep enabled and a 5-minute startup spread
    /// (prevents fleet-wide sweep storms on simultaneous startup).
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            startup_jitter_max: Duration::from_secs(300),
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

/// Deployment-wide enumeration of accounts eligible for keepalive refresh:
/// every `Configured` account with a refresh secret handle, across all owners
/// and vendors. Recipe-threshold filtering is the sweep's job, not the
/// source's. Records carry secret *handles* (opaque references, never raw
/// token material); implementations and callers must not log or serialize
/// them.
#[async_trait]
pub trait KeepaliveCandidateSource: Send + Sync {
    async fn list_keepalive_candidates(&self) -> Vec<CredentialAccount>;
}

/// Narrow refresh port: exactly the one call the sweep needs. The production
/// implementation must route through the engine-owned refresh path (per-
/// account single-flight, durable status transitions on failure).
#[async_trait]
pub trait KeepaliveRefreshPort: Send + Sync {
    async fn refresh_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError>;
}

#[async_trait]
impl KeepaliveRefreshPort for ProviderBackedCredentialAccountService {
    async fn refresh_account(
        &self,
        request: CredentialRefreshRequest,
    ) -> Result<CredentialRefreshReport, AuthProductError> {
        CredentialAccountService::refresh_account(self, request).await
    }
}

/// Outcome of one leader-gated tick.
#[derive(Debug, PartialEq, Eq)]
pub enum LeaderOutcome<T> {
    /// Another process holds deployment leadership this tick; skip.
    NotLeader,
    /// This process was the leader and ran the sweep.
    Ran(T),
}

/// The sweep future handed to the leader lock (owned, `'static`, so a lock
/// implementation may run it inside a transaction or task of its own).
pub type KeepaliveSweepFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Deployment-wide leader election for one tick. Exactly one process per
/// deployment should run the sweep per tick; implementations must hold
/// leadership for the sweep's duration and fail closed (`NotLeader`) on any
/// election error.
#[async_trait]
pub trait KeepaliveLeaderLock: Send + Sync {
    async fn run_as_leader(&self, sweep: KeepaliveSweepFuture) -> LeaderOutcome<()>;
}

/// Always-leader lock for single-writer topologies and tests.
#[derive(Debug, Default)]
pub struct AlwaysLeaderKeepaliveLock;

#[async_trait]
impl KeepaliveLeaderLock for AlwaysLeaderKeepaliveLock {
    async fn run_as_leader(&self, sweep: KeepaliveSweepFuture) -> LeaderOutcome<()> {
        sweep.await;
        LeaderOutcome::Ran(())
    }
}

/// Everything the sweep needs, cloneable for the spawn.
#[derive(Clone)]
pub struct KeepaliveSweepDeps {
    /// Vendor-blind account enumeration.
    pub candidates: Arc<dyn KeepaliveCandidateSource>,
    /// Active recipe data — declares which vendors have an idle lifetime.
    pub recipes: Arc<dyn AuthRecipeResolver>,
    /// Engine-owned refresh path (single-flight per account inside).
    pub refresh: Arc<dyn KeepaliveRefreshPort>,
    /// Deployment-wide per-tick leader election.
    pub leader_lock: Arc<dyn KeepaliveLeaderLock>,
}

// ---------------------------------------------------------------------------
// Runtime handle + spawn
// ---------------------------------------------------------------------------

/// Handle to the spawned sweep task.
pub struct KeepaliveSweepHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

impl KeepaliveSweepHandle {
    /// Cancel the sweep and wait up to `timeout` for it to stop, aborting the
    /// task if it does not.
    pub async fn shutdown(self, timeout: Duration) {
        self.cancel.cancel();
        self.join_with_timeout(timeout).await;
    }

    async fn join_with_timeout(self, timeout: Duration) {
        let mut handle = self.handle;
        match tokio::time::timeout(timeout, &mut handle).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::debug!(?error, "keepalive sweep task join failed");
            }
            Err(_) => {
                tracing::debug!(
                    ?timeout,
                    "keepalive sweep did not stop before shutdown timeout; aborting"
                );
                handle.abort();
                if let Err(error) = handle.await
                    && error.is_panic()
                {
                    tracing::debug!(?error, "aborted keepalive sweep task panicked");
                }
            }
        }
    }
}

/// Spawn the background keepalive sweep. Returns `None` when
/// `settings.enabled` is `false` — nothing is started and no resources are
/// consumed.
pub fn spawn_keepalive_sweep(
    settings: KeepaliveSweepSettings,
    deps: KeepaliveSweepDeps,
) -> Option<KeepaliveSweepHandle> {
    if !settings.enabled {
        return None;
    }
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        run_keepalive_sweep(deps, settings, task_cancel).await;
    });
    Some(KeepaliveSweepHandle { cancel, handle })
}

// ---------------------------------------------------------------------------
// Run loop
// ---------------------------------------------------------------------------

async fn run_keepalive_sweep(
    deps: KeepaliveSweepDeps,
    settings: KeepaliveSweepSettings,
    cancel: CancellationToken,
) {
    // Startup jitter: spread first ticks across the fleet (thundering herd).
    if !sleep_or_cancel(jitter_delay(settings.startup_jitter_max), &cancel).await {
        return;
    }
    loop {
        tick_once(&deps, &settings, &cancel, Utc::now()).await;
        let delay = settings.interval + jitter_delay(settings.tick_jitter_max);
        if !sleep_or_cancel(delay, &cancel).await {
            return;
        }
    }
}

/// One leader-gated tick. `now` is injectable for deterministic tests;
/// the production loop always passes `Utc::now()`.
pub async fn tick_once(
    deps: &KeepaliveSweepDeps,
    settings: &KeepaliveSweepSettings,
    cancel: &CancellationToken,
    now: DateTime<Utc>,
) {
    let sweep_deps = deps.clone();
    let sweep_settings = settings.clone();
    let sweep_cancel = cancel.clone();
    let sweep: KeepaliveSweepFuture = Box::pin(async move {
        sweep_once(&sweep_deps, &sweep_settings, &sweep_cancel, now).await;
    });
    match deps.leader_lock.run_as_leader(sweep).await {
        LeaderOutcome::NotLeader => {
            tracing::debug!("keepalive sweep tick: not the leader; skipping");
        }
        LeaderOutcome::Ran(()) => {}
    }
}

/// The sweep executed only by the leader process. Enumerates candidates,
/// keeps those whose vendor declares an idle lifetime and whose idle age
/// passed half of it, and refreshes them soonest-death-first through the
/// engine-owned refresh path.
pub async fn sweep_once(
    deps: &KeepaliveSweepDeps,
    settings: &KeepaliveSweepSettings,
    cancel: &CancellationToken,
    now: DateTime<Utc>,
) {
    let candidates = deps.candidates.list_keepalive_candidates().await;

    // Keep accounts whose vendor's active recipe declares an idle lifetime;
    // everything else opts out of the sweep entirely.
    let mut declaring: Vec<(CredentialAccount, chrono::Duration, Option<ExtensionId>)> = Vec::new();
    for account in candidates {
        if !is_refreshable(&account) {
            continue;
        }
        let requester_extension = match account.ownership {
            CredentialOwnership::ExtensionOwned => {
                // An extension-owned account cannot outlive the installed
                // manifest that declared its recipe.
                let Some(extension) = account.owner_extension.clone() else {
                    continue;
                };
                Some(extension)
            }
            _ => None,
        };
        let Some(resolved) = deps
            .recipes
            .resolve(
                requester_extension.as_ref(),
                Some(&account.scope.resource.user_id),
                account.provider.as_str(),
            )
            .await
        else {
            continue;
        };
        let Some(lifetime) = resolved.recipe.keepalive_idle_threshold() else {
            continue;
        };
        let Ok(lifetime) = chrono::Duration::from_std(lifetime) else {
            // Unreachable for recipes that passed validation (bounded field);
            // skip rather than panic if a resolver hands back raw data.
            continue;
        };
        declaring.push((account, lifetime, requester_extension));
    }

    let (due, dropped) = select_due_candidates(declaring, now, settings.max_per_tick);
    if dropped > 0 {
        tracing::debug!(
            dropped,
            max_per_tick = settings.max_per_tick,
            "keepalive sweep: due list truncated to max_per_tick"
        );
    }
    if due.is_empty() {
        tracing::debug!("keepalive sweep tick: no due accounts");
        return;
    }
    tracing::debug!(
        count = due.len(),
        "keepalive sweep tick: refreshing due accounts"
    );

    let mut refreshed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for (account, requester_extension) in due {
        let mut request = CredentialRefreshRequest::new(
            account.scope.clone(),
            account.provider.clone(),
            account.id,
        );
        if let Some(extension) = requester_extension {
            request = request.for_extension(extension);
        }
        // Race each refresh against cancellation so shutdown never waits on a
        // slow token endpoint. `biased` checks cancellation first.
        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tracing::debug!("keepalive sweep: cancelled mid-sweep; stopping");
                break;
            }
            outcome = deps.refresh.refresh_account(request) => outcome,
        };
        match outcome {
            Ok(_) => {
                refreshed += 1;
                tracing::debug!(
                    vendor = %account.provider,
                    "keepalive sweep: account refreshed"
                );
            }
            // Transient store trouble: leave the account for the next tick —
            // no status mutation.
            Err(AuthProductError::BackendUnavailable) => {
                skipped += 1;
                tracing::debug!(
                    vendor = %account.provider,
                    "keepalive sweep: transient error, will retry next tick"
                );
            }
            Err(error) => {
                // Permanent failures already moved the durable account status
                // (invalid_grant → Revoked, refresh failure → RefreshFailed)
                // inside the engine-owned refresh path, so the next tick's
                // refreshable filter excludes the account. Never log token
                // material or vendor bodies.
                failed += 1;
                tracing::debug!(
                    vendor = %account.provider,
                    error = %error,
                    "keepalive sweep: account refresh failed (non-transient)"
                );
            }
        }
    }

    tracing::debug!(refreshed, skipped, failed, "keepalive sweep tick complete");
}

// ---------------------------------------------------------------------------
// Pure candidate selection
// ---------------------------------------------------------------------------

/// Keep accounts whose idle age (`now - updated_at`) reached half their
/// vendor-declared lifetime, ordered by projected token death
/// (`updated_at + lifetime`, ascending), capped at `max_per_tick`. Returns
/// the selected accounts and the number dropped by the cap.
fn select_due_candidates(
    candidates: Vec<(CredentialAccount, chrono::Duration, Option<ExtensionId>)>,
    now: DateTime<Utc>,
    max_per_tick: usize,
) -> (Vec<(CredentialAccount, Option<ExtensionId>)>, usize) {
    // Due at half the declared lifetime: sweeping only past the full lifetime
    // would refresh tokens the vendor already killed; half-life leaves
    // headroom for downtime while staying derived from the vendor constraint.
    let mut due: Vec<_> = candidates
        .into_iter()
        .filter(|(account, lifetime, _)| {
            now.signed_duration_since(account.updated_at) >= *lifetime / 2
        })
        .collect();
    // Soonest projected death first, so the cap cannot starve the accounts
    // closest to expiry.
    due.sort_by_key(|(account, lifetime, _)| account.updated_at + *lifetime);
    let dropped = due.len().saturating_sub(max_per_tick);
    due.truncate(max_per_tick);
    (
        due.into_iter()
            .map(|(account, _, requester_extension)| (account, requester_extension))
            .collect(),
        dropped,
    )
}

/// Defensive re-check of the candidate-source contract: only `Configured`
/// accounts holding a refresh secret can be refreshed.
fn is_refreshable(account: &CredentialAccount) -> bool {
    account.status == CredentialAccountStatus::Configured && account.refresh_secret.is_some()
}

// ---------------------------------------------------------------------------
// Timing helpers
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

/// A random duration in `[0, max]`. Zero when `max` is zero.
fn jitter_delay(max: Duration) -> Duration {
    if max.is_zero() {
        return Duration::ZERO;
    }
    use rand::RngExt as _;
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
    use crate::{
        AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountId, CredentialAccountLabel,
        CredentialOwnership,
    };
    use ironclaw_host_api::{
        ids::{InvocationId, SecretHandle, UserId},
        resource::ResourceScope,
    };

    /// A `Configured`/has-refresh candidate for `vendor` with a given
    /// `updated_at`.
    fn candidate(vendor: &str, updated_at: DateTime<Utc>) -> CredentialAccount {
        let resource =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        CredentialAccount {
            id: CredentialAccountId::new(),
            scope: AuthProductScope::new(resource, AuthSurface::Api),
            provider: AuthProviderId::new(vendor).unwrap(),
            label: CredentialAccountLabel::new(vendor).unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("access-handle").unwrap()),
            refresh_secret: Some(SecretHandle::new("refresh-handle").unwrap()),
            scopes: Vec::new(),
            provider_identity: None,
            link_revision: 0,
            created_at: updated_at,
            updated_at,
        }
    }

    #[test]
    fn due_selection_triggers_at_half_life_and_orders_by_projected_death() {
        let now = Utc::now();
        let week = chrono::Duration::days(7);
        let month = chrono::Duration::days(30);

        // alpha (7d lifetime): 4d idle → past half-life (3.5d), due; its
        // projected death is now+3d.
        let alpha = candidate("alpha", now - chrono::Duration::days(4));
        // beta (30d lifetime): 16d idle → past half-life (15d), due; its
        // projected death is now+14d — later than alpha's.
        let beta = candidate("beta", now - chrono::Duration::days(16));
        // gamma (7d lifetime): 3d idle → under half-life, not due.
        let gamma = candidate("gamma", now - chrono::Duration::days(3));

        let candidates = vec![
            (beta.clone(), month, None),
            (gamma.clone(), week, None),
            (alpha.clone(), week, None),
        ];

        let (selected, dropped) = select_due_candidates(candidates.clone(), now, 10);
        assert_eq!(
            selected.iter().map(|(a, _)| a.id).collect::<Vec<_>>(),
            vec![alpha.id, beta.id],
            "due accounts only, soonest projected death first"
        );
        assert_eq!(dropped, 0);

        // Cap below the due count: the account dying soonest wins the slot.
        let (selected, dropped) = select_due_candidates(candidates, now, 1);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0.id, alpha.id, "cap keeps the soonest death");
        assert_eq!(dropped, 1);
    }

    #[test]
    fn due_selection_is_empty_when_all_fresh() {
        let now = Utc::now();
        let week = chrono::Duration::days(7);
        let candidates = vec![
            (candidate("alpha", now), week, None),
            (
                candidate("alpha", now - chrono::Duration::days(1)),
                week,
                None,
            ),
        ];
        let (selected, dropped) = select_due_candidates(candidates, now, 5);
        assert!(selected.is_empty());
        assert_eq!(dropped, 0);
    }

    #[test]
    fn refreshable_requires_configured_status_and_refresh_secret() {
        let now = Utc::now();
        let ok = candidate("alpha", now);
        assert!(is_refreshable(&ok));

        let mut revoked = candidate("alpha", now);
        revoked.status = CredentialAccountStatus::Revoked;
        assert!(!is_refreshable(&revoked));

        let mut no_refresh = candidate("alpha", now);
        no_refresh.refresh_secret = None;
        assert!(!is_refreshable(&no_refresh));
    }

    #[tokio::test]
    async fn sweep_refreshes_extension_owned_account_only_when_extension_context_is_threaded() {
        // Regression pin (extension-owned keepalive refresh silently never
        // firing): before this fix, sweep_once resolved recipes by vendor
        // alone and never carried `owner_extension` into the resolver call.
        // A real installed-manifest resolver fails closed when the
        // requester extension is absent (`AuthRecipeResolver::resolve`'s
        // documented contract), so an extension-owned hosted-MCP account
        // would never resolve a recipe, never appear in the due list, and
        // never refresh again — with no error, log, or test failure
        // anywhere. This test drives the real caller seam (`sweep_once`,
        // the entry point used by `tick_once`/`run_keepalive_sweep`) with a
        // resolver shaped like that real contract.
        let now = Utc::now();
        let extension = ExtensionId::new("acme-mcp").unwrap();

        let mut account = candidate("acme-mcp-vendor", now - chrono::Duration::days(10));
        account.ownership = CredentialOwnership::ExtensionOwned;
        account.owner_extension = Some(extension.clone());

        let recipe: ironclaw_extension_contracts::recipe::VendorAuthRecipe =
            serde_json::from_value(serde_json::json!({
                "method": "oauth2_code",
                "display_name": "Acme MCP",
                "authorization_endpoint": "https://auth.acme.example/authorize",
                "token_endpoint": "https://auth.acme.example/token",
                "scopes": [],
                "client_credentials": { "client_id_handle": "acme_client_id" },
                "token_response": { "access_token": "/access_token" },
                "refresh": { "keepalive_idle_seconds": 604_800 },
            }))
            .expect("recipe parses");
        let resolved = crate::engine::ResolvedVendorAuthRecipe {
            vendor: "acme-mcp-vendor".to_string(),
            recipe,
            token_exchange_resource: None,
            protected_resource_metadata_url: None,
        };

        /// Shaped like a real installed-manifest resolver: only resolves
        /// this vendor's recipe for its owning extension, and fails closed
        /// (returns `None`) for any other requester — including `None`.
        #[derive(Debug)]
        struct ExtensionScopedResolver {
            extension: ExtensionId,
            recipe: crate::engine::ResolvedVendorAuthRecipe,
        }
        #[async_trait]
        impl AuthRecipeResolver for ExtensionScopedResolver {
            async fn resolve(
                &self,
                requester_extension: Option<&ExtensionId>,
                _caller: Option<&ironclaw_host_api::ids::UserId>,
                vendor: &str,
            ) -> Option<crate::engine::ResolvedVendorAuthRecipe> {
                if requester_extension == Some(&self.extension) && vendor == self.recipe.vendor {
                    Some(self.recipe.clone())
                } else {
                    None
                }
            }
        }

        struct FixedSource(CredentialAccount);
        #[async_trait]
        impl KeepaliveCandidateSource for FixedSource {
            async fn list_keepalive_candidates(&self) -> Vec<CredentialAccount> {
                vec![self.0.clone()]
            }
        }

        #[derive(Default)]
        struct RecordingRefresh {
            requests: tokio::sync::Mutex<Vec<CredentialRefreshRequest>>,
        }
        #[async_trait]
        impl KeepaliveRefreshPort for RecordingRefresh {
            async fn refresh_account(
                &self,
                request: CredentialRefreshRequest,
            ) -> Result<CredentialRefreshReport, AuthProductError> {
                self.requests.lock().await.push(request);
                Err(AuthProductError::MalformedConfig)
            }
        }

        let refresh = Arc::new(RecordingRefresh::default());
        let deps = KeepaliveSweepDeps {
            candidates: Arc::new(FixedSource(account)),
            recipes: Arc::new(ExtensionScopedResolver {
                extension: extension.clone(),
                recipe: resolved,
            }),
            refresh: refresh.clone(),
            leader_lock: Arc::new(AlwaysLeaderKeepaliveLock),
        };
        let settings = KeepaliveSweepSettings::enabled();
        let cancel = CancellationToken::new();

        sweep_once(&deps, &settings, &cancel, now).await;

        let requests = refresh.requests.lock().await;
        assert_eq!(
            requests.len(),
            1,
            "extension-owned account must be swept, not silently dropped"
        );
        assert_eq!(
            requests[0].requester_extension.as_ref(),
            Some(&extension),
            "refresh request must carry the owning extension so the resolver can find its recipe"
        );
    }

    #[test]
    fn jitter_is_disabled_when_max_is_zero() {
        assert_eq!(jitter_delay(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn jitter_is_bounded_and_not_constant() {
        // Regression pin for the PR #6116 review finding: jitter must come
        // from a clock-independent random source. A wall-clock-derived
        // implementation collapses to one repeated value when the clock's
        // resolution is coarser than the draw cadence, so 64 back-to-back
        // draws over a wide range would all collide.
        let max = Duration::from_secs(3600);
        let draws: std::collections::HashSet<Duration> =
            (0..64).map(|_| jitter_delay(max)).collect();
        assert!(
            draws.iter().all(|delay| *delay <= max),
            "every draw stays within max"
        );
        assert!(
            draws.len() > 1,
            "jitter_delay must not collapse to a constant value across draws"
        );
    }

    #[test]
    fn settings_default_disabled_and_enabled_constructor() {
        let settings = KeepaliveSweepSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.interval, Duration::from_secs(6 * 3600));
        assert_eq!(settings.startup_jitter_max, Duration::ZERO);
        assert_eq!(settings.tick_jitter_max, Duration::ZERO);
        assert_eq!(settings.max_per_tick, 5);

        let enabled = KeepaliveSweepSettings::enabled();
        assert!(enabled.enabled);
        assert_eq!(enabled.startup_jitter_max, Duration::from_secs(300));
        assert_eq!(enabled.interval, Duration::from_secs(6 * 3600));
    }

    #[tokio::test]
    async fn spawn_returns_none_when_disabled() {
        // The early-return guard runs before deps are touched, so no live
        // services need wiring for this path — deps that panic on use prove
        // it.
        struct PanicSource;
        #[async_trait]
        impl KeepaliveCandidateSource for PanicSource {
            async fn list_keepalive_candidates(&self) -> Vec<CredentialAccount> {
                panic!("disabled sweep must not enumerate")
            }
        }
        struct PanicRefresh;
        #[async_trait]
        impl KeepaliveRefreshPort for PanicRefresh {
            async fn refresh_account(
                &self,
                _request: CredentialRefreshRequest,
            ) -> Result<CredentialRefreshReport, AuthProductError> {
                panic!("disabled sweep must not refresh")
            }
        }
        let deps = KeepaliveSweepDeps {
            candidates: Arc::new(PanicSource),
            recipes: Arc::new(crate::engine::StaticAuthRecipeResolver::default()),
            refresh: Arc::new(PanicRefresh),
            leader_lock: Arc::new(AlwaysLeaderKeepaliveLock),
        };
        assert!(spawn_keepalive_sweep(KeepaliveSweepSettings::default(), deps).is_none());
    }

    #[tokio::test]
    async fn runtime_handle_aborts_when_join_times_out() {
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let handle = tokio::spawn(async move {
            task_cancel.cancelled().await;
            std::future::pending::<()>().await;
        });
        let runtime_handle = KeepaliveSweepHandle { cancel, handle };
        // Must complete without hanging.
        runtime_handle.shutdown(Duration::from_millis(1)).await;
    }
}
