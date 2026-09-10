//! Turn-start per-user hosted-MCP discovery (P2b).
//!
//! Hosted-MCP providers with a per-user secret credential (e.g. the agent
//! marketplace) serve a **per-principal** `tools/list`. This refresher runs
//! before a turn's capability port is built: for every active hosted-MCP
//! extension whose discovery template binds a per-user-resolvable
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

use ironclaw_extension_registry::{
    DEFAULT_SCOPED_OVERLAY_TTL, ExtensionPackage, OverlayFreshness, OverlayScope,
    ScopedPackageOverlay, SharedExtensionRegistry, is_hosted_http_mcp_package,
};
use ironclaw_host_api::{
    capability::{RuntimeCredentialRequirement, RuntimeCredentialRequirementSource},
    dispatch::CredentialStageError,
    ids::{CapabilityId, ExtensionId},
    resource::ResourceScope,
};
use ironclaw_host_runtime::ProductAuthProviderRuntimePorts;
use std::sync::Mutex;

use ironclaw_extension_host::{HostedMcpDiscoveryError, discover_hosted_mcp_package};

/// Host ceiling for a per-turn discovery refresh, matching the MCP lane's own
/// `MAX_DISCOVERED_MCP_TOOLS` cap.
const TURN_DISCOVERY_MAX_TOOLS: u32 = 1024;

/// Per-turn discovery is on the turn-start path: bound it well below the MCP
/// lane's 60 s transport timeout so a hung provider costs one bounded wait,
/// not a wedged turn.
const TURN_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(8);

/// Ceiling on ALL of a turn's discovery, not just one call. The per-call bound
/// is multiplied by however many hosted-MCP providers are eligible, so without
/// this a turn with N slow providers stalls for N × [`TURN_DISCOVERY_TIMEOUT`]
/// before its capability port is even built. Packages past the budget keep
/// their last-good surface, exactly as a per-call timeout leaves them.
const TURN_DISCOVERY_BUDGET: Duration = Duration::from_secs(12);

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
    /// Single-flight: an (owner, extension) pair being discovered by one turn
    /// is skipped by concurrent turns, which serve the last-good overlay.
    in_flight: Mutex<HashSet<(OverlayScope, ExtensionId)>>,
    negative_until: Mutex<HashMap<(OverlayScope, ExtensionId), Instant>>,
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
        let eligible: Vec<(ExtensionPackage, CapabilityId, RuntimeCredentialRequirement)> =
            snapshot
                .extensions()
                .filter_map(|package| {
                    per_user_secret_discovery_template(package).map(
                        |(capability_id, requirement)| {
                            (package.clone(), capability_id, requirement)
                        },
                    )
                })
                .collect();
        let owner = OverlayScope::new(
            scope.tenant_id.clone(),
            scope.user_id.clone(),
            scope.thread_id.clone(),
        );
        let deadline = tokio::time::Instant::now() + TURN_DISCOVERY_BUDGET;
        for (package, capability_id, requirement) in eligible {
            if tokio::time::Instant::now() >= deadline {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    "hosted MCP per-user discovery skipped: turn budget spent; \
                     keeping last-good surface"
                );
                continue;
            }
            let key = (owner.clone(), package.id.clone());
            if matches!(
                self.overlay.get(&owner, &package.id),
                Some((_, OverlayFreshness::Fresh))
            ) {
                continue;
            }
            if self.negative_cache_active(&key) {
                continue;
            }
            let Some(_in_flight) = InFlightGuard::acquire(self, key) else {
                continue;
            };
            self.refresh_one(scope, &owner, &package, &capability_id, &requirement)
                .await;
        }
    }

    async fn refresh_one(
        &self,
        scope: &ResourceScope,
        owner: &OverlayScope,
        package: &ExtensionPackage,
        capability_id: &CapabilityId,
        requirement: &RuntimeCredentialRequirement,
    ) {
        // Stage the turn user's credential for the discovery egress (the MCP
        // lane consumes staged one-shot credentials only). The requirement
        // router covers both source kinds — a raw secret-store handle AND a
        // product-auth account (vendor-recipe credentials delivered through
        // the extension setup channel). AuthRequired here IS the "user has no
        // token" verdict.
        match self
            .runtime_ports
            .stage_credential_requirement_once(scope, capability_id, requirement, &package.id)
            .await
        {
            Ok(()) => {}
            Err(CredentialStageError::AuthRequired) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    secret_handle = %requirement.handle,
                    "hosted MCP per-user discovery skipped: credential not provisioned"
                );
                // The user has no credential: any previously discovered
                // surface no longer authenticates — drop it so dispatch
                // failures surface the missing credential instead of calling
                // out with tools the provider will reject.
                self.overlay.remove(owner, &package.id);
                self.negative_insert(owner, package);
                return;
            }
            Err(CredentialStageError::Backend) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    "hosted MCP per-user discovery skipped: credential staging backend failure"
                );
                self.keep_last_good(owner, package);
                return;
            }
        }

        // The egress pipeline requires a STAGED network policy for
        // (scope, capability) — the dispatch path stages it via the
        // ApplyNetworkPolicy obligation; stage the equivalent hosted-MCP
        // policy for this discovery call.
        self.runtime_ports.stage_network_policy_once(
            scope,
            capability_id,
            hosted_mcp_discovery_network_policy(package),
        );
        let discovery = tokio::time::timeout(
            TURN_DISCOVERY_TIMEOUT,
            discover_hosted_mcp_package(
                package,
                // Per-turn refresh has no resolved manifest at hand; the host
                // ceiling is the same bound the lane enforces anyway.
                TURN_DISCOVERY_MAX_TOOLS,
                scope.clone(),
                self.runtime_ports.runtime_http_egress(),
            ),
        )
        .await;
        self.runtime_ports
            .discard_staged_discovery_state(scope, capability_id);
        match discovery {
            Ok(Ok(discovered)) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    capability_count = discovered.capabilities.len(),
                    "hosted MCP per-user discovery refreshed the user's tool surface"
                );
                self.overlay
                    .insert(owner.clone(), discovered, DEFAULT_SCOPED_OVERLAY_TTL);
            }
            Ok(Err(HostedMcpDiscoveryError::Transient(reason))) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    reason,
                    "hosted MCP per-user discovery failed transiently; keeping last-good surface"
                );
                self.keep_last_good(owner, package);
            }
            // A rejected credential is the caller's own auth problem, not a
            // provider outage: suppress the re-probe like any permanent
            // failure so one unauthenticated user cannot hammer the server
            // every turn. The connect flow surfaces the challenge elsewhere.
            Ok(Err(HostedMcpDiscoveryError::CredentialsRejected(_challenge))) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    "hosted MCP per-user discovery was rejected by the provider's auth; \
                     suppressing re-probe until the caller reconnects"
                );
                self.negative_insert(owner, package);
            }
            Ok(Err(HostedMcpDiscoveryError::Permanent(reason))) => {
                // `debug!`, not `warn!`: this runs on a background turn path
                // and `warn!` corrupts the REPL/TUI (CLAUDE.md logging rule).
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    reason,
                    "hosted MCP per-user discovery failed permanently; suppressing re-probe"
                );
                self.negative_insert(owner, package);
            }
            Err(_elapsed) => {
                tracing::debug!(
                    extension_id = %package.id,
                    user_id = %scope.user_id,
                    "hosted MCP per-user discovery timed out; keeping last-good surface"
                );
                self.keep_last_good(owner, package);
            }
        }
    }

    /// Re-arm a stale last-good entry so a transient provider failure does not
    /// drop the user's discovered surface (and does not re-probe every turn).
    fn keep_last_good(&self, owner: &OverlayScope, package: &ExtensionPackage) {
        if self.overlay.get(owner, &package.id).is_some() {
            self.overlay
                .touch(owner, &package.id, DEFAULT_SCOPED_OVERLAY_TTL);
        } else {
            self.negative_insert(owner, package);
        }
    }

    fn negative_cache_active(&self, key: &(OverlayScope, ExtensionId)) -> bool {
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

    fn negative_insert(&self, owner: &OverlayScope, package: &ExtensionPackage) {
        if let Ok(mut negative) = self.negative_until.lock() {
            negative.insert(
                (owner.clone(), package.id.clone()),
                Instant::now() + NEGATIVE_TTL,
            );
        }
    }

    fn begin(&self, key: &(OverlayScope, ExtensionId)) -> bool {
        self.in_flight
            .lock()
            .map(|mut in_flight| in_flight.insert(key.clone()))
            .unwrap_or(false) // silent-ok: poisoned single-flight skips refresh this turn
    }

    fn finish(&self, key: &(OverlayScope, ExtensionId)) {
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
/// The discovery egress network policy: allow exactly the provider's audience
/// host(s) over https, mirroring the grant-constraint policy the dispatch path
/// stages for the same extension.
fn hosted_mcp_discovery_network_policy(
    package: &ExtensionPackage,
) -> ironclaw_host_api::action::NetworkPolicy {
    let mut allowed_targets = Vec::new();
    for capability in &package.manifest.capabilities {
        for credential in &capability.runtime_credentials {
            if !allowed_targets.contains(&credential.audience) {
                allowed_targets.push(credential.audience.clone());
            }
        }
        for target in &capability.network_targets {
            if !allowed_targets.contains(target) {
                allowed_targets.push(target.clone());
            }
        }
    }
    ironclaw_host_api::action::NetworkPolicy {
        allowed_targets,
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(2 * 1024 * 1024),
    }
}

fn per_user_secret_discovery_template(
    package: &ExtensionPackage,
) -> Option<(CapabilityId, RuntimeCredentialRequirement)> {
    if !is_hosted_http_mcp_package(package) {
        return None;
    }
    let template = package.manifest.capabilities.first()?;
    // Both credential kinds qualify: a raw `SecretHandle` credential, and a
    // vendor-recipe (`ProductAuthAccount`) credential delivered through the
    // extension's setup channel (the v3 `[mcp]` + `[auth.<vendor>]` shape,
    // e.g. a vendor bearer). Staging routes by the requirement's
    // SOURCE (`stage_credential_requirement_once`), and a user without the
    // credential fails closed into the AuthRequired skip.
    let requirement = template
        .runtime_credentials
        .iter()
        .find(|credential| {
            matches!(
                credential.source,
                RuntimeCredentialRequirementSource::SecretHandle
                    | RuntimeCredentialRequirementSource::ProductAuthAccount { .. }
            )
        })
        .cloned()?;
    Some((template.id.clone(), requirement))
}

/// Releases the single-flight slot on drop.
///
/// `finish` used to run only after `refresh_one` returned. A turn future
/// dropped mid-await — turn abort, client disconnect, shutdown — skipped it,
/// leaving `(owner, extension)` pinned in `in_flight` for the process
/// lifetime; every later turn for that caller then short-circuited and never
/// re-discovered, so the caller stayed on its last-good surface (or the static
/// manifest) until restart. `tokio::time::timeout` does not help: it bounds
/// the inner discovery, not the dropping of the outer future.
struct InFlightGuard<'a> {
    refresher: &'a HostedMcpOverlayRefresher,
    key: (OverlayScope, ExtensionId),
}

impl<'a> InFlightGuard<'a> {
    fn acquire(
        refresher: &'a HostedMcpOverlayRefresher,
        key: (OverlayScope, ExtensionId),
    ) -> Option<Self> {
        refresher.begin(&key).then(|| Self { refresher, key })
    }
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.refresher.finish(&self.key);
    }
}
