//! Turn-start per-user hosted-MCP discovery (P2b).
//!
//! Hosted-MCP providers with a per-user secret credential (e.g. the agent
//! marketplace) serve a **per-principal** `tools/list`. This refresher runs
//! before a turn's capability port is built: for every active hosted-MCP
//! extension whose discovery template binds a [`SecretHandle`]-sourced
//! runtime credential, it stages the TURN USER's secret and re-runs
//! `tools/list` discovery under that user's scope, caching the discovered
//! package in the [`ScopedPackageOverlay`] the surface/dispatch/egress paths
//! all read.
//!
//! Failure semantics (user-visible contract):
//! - **Missing credential** (`CredentialStageError::AuthRequired`): the user
//!   has no secret for the extension — discovery is skipped and
//!   negative-cached, the static manifest surface stays, and a dispatch of a
//!   static tool produces the model-visible auth gate naming the missing
//!   handle (`required_secrets`). No silent success, no wasted egress.
//! - **Transient failure** (provider down, timeout): the last-good discovered
//!   surface is kept serving (its TTL is re-armed) so a provider blip does not
//!   flap the user's tool surface; with no last-good entry the static
//!   manifest fallback applies.
//! - **Permanent failure** (malformed discovery result): negative-cached so a
//!   broken provider is not re-probed every turn.
//!
//! Discovery here never mutates installation state — the overlay is a derived
//! cache (see `ironclaw_extensions::scoped_overlay`).

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use ironclaw_extensions::{
    DEFAULT_SCOPED_OVERLAY_TTL, ExtensionPackage, OverlayFreshness, ScopedPackageOverlay,
    SharedExtensionRegistry, is_hosted_http_mcp_package,
};
use ironclaw_host_api::{
    CapabilityId, CredentialStageError, ExtensionId, ResourceScope,
    RuntimeCredentialRequirementSource, SecretHandle, UserId,
};
use ironclaw_host_runtime::ProductAuthProviderRuntimePorts;
use std::sync::Mutex;

use crate::extension_host::mcp_discovery::{
    HostedMcpDiscoveryError, discover_hosted_mcp_package,
};

/// Per-turn discovery is on the turn-start path: bound it well below the MCP
/// lane's 60 s transport timeout so a hung provider costs one bounded wait,
/// not a wedged turn.
const TURN_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(8);

/// How long a missing-credential / permanent-failure verdict suppresses
/// re-probing. Long enough to keep failed discovery off every turn, short
/// enough that a freshly provisioned token is picked up within ~2 minutes.
const NEGATIVE_TTL: Duration = Duration::from_secs(120);

/// Turn-start per-user hosted-MCP discovery driver. One instance per composed
/// runtime, shared by every capability-port factory invocation.
pub(super) struct HostedMcpOverlayRefresher {
    overlay: Arc<ScopedPackageOverlay>,
    registry: Arc<SharedExtensionRegistry>,
    runtime_ports: ProductAuthProviderRuntimePorts,
    /// Single-flight: a (user, extension) pair being discovered by one turn is
    /// skipped by concurrent turns (they serve the previous surface).
    in_flight: Mutex<HashSet<(UserId, ExtensionId)>>,
    negative_until: Mutex<HashMap<(UserId, ExtensionId), Instant>>,
}

impl HostedMcpOverlayRefresher {
    pub(super) fn new(
        overlay: Arc<ScopedPackageOverlay>,
        registry: Arc<SharedExtensionRegistry>,
        runtime_ports: ProductAuthProviderRuntimePorts,
    ) -> Self {
        Self {
            overlay,
            registry,
            runtime_ports,
            in_flight: Mutex::new(HashSet::new()),
            negative_until: Mutex::new(HashMap::new()),
        }
    }

    /// Refresh the scope user's discovered surfaces for every eligible
    /// hosted-MCP extension. Never fails the turn: every failure mode
    /// degrades to the previous surface (last-good or static manifest).
    pub(super) async fn refresh_for_scope(&self, scope: &ResourceScope) {
        let snapshot = self.registry.snapshot();
        let eligible: Vec<(ExtensionPackage, CapabilityId, SecretHandle)> = snapshot
            .extensions()
            .filter_map(|package| {
                per_user_secret_discovery_template(package)
                    .map(|(capability_id, handle)| (package.clone(), capability_id, handle))
            })
            .collect();
        for (package, capability_id, handle) in eligible {
            let key = (scope.user_id.clone(), package.id.clone());
            if matches!(
                self.overlay.get(&scope.user_id, &package.id),
                Some((_, OverlayFreshness::Fresh))
            ) {
                continue;
            }
            if self.negative_cache_active(&key) {
                continue;
            }
            if !self.begin(&key) {
                continue;
            }
            self.refresh_one(scope, &package, &capability_id, &handle)
                .await;
            self.finish(&key);
        }
    }

    async fn refresh_one(
        &self,
        scope: &ResourceScope,
        package: &ExtensionPackage,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
    ) {
        // Stage the turn user's secret for the discovery egress (the MCP lane
        // consumes staged one-shot credentials only). AuthRequired here IS the
        // "user has no token" verdict.
        match self
            .runtime_ports
            .stage_owner_resolved_secret_once(scope, capability_id, handle)
            .await
        {
            Ok(()) => {}
            Err(CredentialStageError::AuthRequired) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    secret_handle = %handle,
                    "hosted MCP per-user discovery skipped: credential not provisioned"
                );
                // The user has no credential: any previously discovered
                // surface no longer authenticates — drop it so dispatch
                // failures surface the missing credential instead of calling
                // out with tools the provider will reject.
                self.overlay.remove(&scope.user_id, &package.id);
                self.negative_insert(scope, package);
                return;
            }
            Err(CredentialStageError::Backend) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    "hosted MCP per-user discovery skipped: credential staging backend failure"
                );
                self.keep_last_good(scope, package);
                return;
            }
        }

        let discovery = tokio::time::timeout(
            TURN_DISCOVERY_TIMEOUT,
            discover_hosted_mcp_package(
                package,
                scope.clone(),
                self.runtime_ports.runtime_http_egress(),
            ),
        )
        .await;
        match discovery {
            Ok(Ok(discovered)) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    capability_count = discovered.capabilities.len(),
                    "hosted MCP per-user discovery refreshed the user's tool surface"
                );
                self.overlay
                    .insert(scope.user_id.clone(), discovered, DEFAULT_SCOPED_OVERLAY_TTL);
            }
            Ok(Err(HostedMcpDiscoveryError::Transient(reason))) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    reason,
                    "hosted MCP per-user discovery failed transiently; keeping last-good surface"
                );
                self.keep_last_good(scope, package);
            }
            Ok(Err(HostedMcpDiscoveryError::Permanent(reason))) => {
                tracing::warn!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    reason,
                    "hosted MCP per-user discovery failed permanently; suppressing re-probe"
                );
                self.negative_insert(scope, package);
            }
            Err(_elapsed) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    "hosted MCP per-user discovery timed out; keeping last-good surface"
                );
                self.keep_last_good(scope, package);
            }
        }
    }

    /// Re-arm a stale last-good entry so a transient provider failure does not
    /// drop the user's discovered surface (and does not re-probe every turn).
    fn keep_last_good(&self, scope: &ResourceScope, package: &ExtensionPackage) {
        if self.overlay.get(&scope.user_id, &package.id).is_some() {
            self.overlay
                .touch(&scope.user_id, &package.id, DEFAULT_SCOPED_OVERLAY_TTL);
        } else {
            self.negative_insert(scope, package);
        }
    }

    fn negative_cache_active(&self, key: &(UserId, ExtensionId)) -> bool {
        let Ok(mut negative) = self.negative_until.lock() else {
            return false; // silent-ok: poisoned negative cache only re-probes
        };
        match negative.get(key) {
            Some(until) if *until > Instant::now() => true,
            Some(_) => {
                negative.remove(key);
                false
            }
            None => false,
        }
    }

    fn negative_insert(&self, scope: &ResourceScope, package: &ExtensionPackage) {
        if let Ok(mut negative) = self.negative_until.lock() {
            negative.insert(
                (scope.user_id.clone(), package.id.clone()),
                Instant::now() + NEGATIVE_TTL,
            );
        }
    }

    fn begin(&self, key: &(UserId, ExtensionId)) -> bool {
        self.in_flight
            .lock()
            .map(|mut in_flight| in_flight.insert(key.clone()))
            .unwrap_or(false) // silent-ok: poisoned single-flight skips refresh this turn
    }

    fn finish(&self, key: &(UserId, ExtensionId)) {
        if let Ok(mut in_flight) = self.in_flight.lock() {
            in_flight.remove(key);
        }
    }
}

/// The discovery template for per-user hosted-MCP refresh: the package must be
/// a hosted HTTP MCP provider whose FIRST capability (the discovery planning
/// template) binds at least one [`SecretHandle`]-sourced runtime credential.
/// Product-auth-account providers (e.g. NEAR AI) keep their activation-time
/// discovery semantics and are not refreshed per user here.
fn per_user_secret_discovery_template(
    package: &ExtensionPackage,
) -> Option<(CapabilityId, SecretHandle)> {
    if !is_hosted_http_mcp_package(package) {
        return None;
    }
    let template = package.manifest.capabilities.first()?;
    let handle = template.runtime_credentials.iter().find_map(|credential| {
        matches!(
            credential.source,
            RuntimeCredentialRequirementSource::SecretHandle
        )
        .then(|| credential.handle.clone())
    })?;
    Some((template.id.clone(), handle))
}
