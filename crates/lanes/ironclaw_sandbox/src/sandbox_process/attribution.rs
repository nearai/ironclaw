//! Source-IP connection attribution for the shared egress proxy (design
//! decision D9 — W1.5 "attribution half").
//!
//! All per-user sandbox containers share one Docker network
//! (`broker::SANDBOX_EGRESS_NETWORK_NAME`, `internal: true`), with a single
//! egress proxy at the network gateway as the only route off-host. The
//! proxy is per-TCP-connection and has no built-in concept of which
//! `{tenant, user}` opened a connection — but the credential firewall this
//! feeds (W6) must inject the right user's secret into an intercepted
//! request, so that connection needs an owner.
//!
//! **Why source IP is sound here** (it usually is not, on the open
//! internet): completing a TCP handshake requires the SYN-ACK to reach the
//! address that sent the SYN — a blindly spoofed source address never sees
//! the SYN-ACK and cannot complete the handshake, so an established
//! connection's peer address cannot be blind-spoofed. The remaining risk on
//! a *shared* network would be a sibling container intercepting or
//! injecting into another container's TCP/ICMP path (an on-path attacker
//! does not need to blind-spoof) — that path is closed here because the
//! egress network is created with `enable_icc=false` (see
//! `broker::sandbox_egress_network_create_options`), which drops
//! container-to-container TCP and ICMP entirely while leaving
//! container-to-gateway reachable (verified empirically; see
//! `exec_transport::icc_disabled_blocks_container_to_container`). So the
//! only two parties that can complete a handshake with the proxy at a given
//! source IP are the gateway itself and the one container holding that IP.
//!
//! **Resolution**: peer IP -> `docker inspect`-equivalent (`docker ps`
//! filtered to the egress network) -> the container whose network-settings
//! IP on that network matches -> that container's `{tenant, user}` labels
//! (written by `registry::build_user_container_labels`, read back here via
//! the same `registry::label_tenant`/`registry::label_user` key functions
//! so the label vocabulary can never drift between writer and reader).
//!
//! **Fail closed.** No match, more than one match, a Docker query error, or
//! a missing/malformed label ⇒ [`ConnectionAttribution::Unattributed`].
//! Never guess, never fall back to "first container", never default to a
//! user — misattributing a connection here would hand one user's injected
//! credential to another user's request, which is the exact failure this
//! design exists to prevent.
//!
//! **W6 is the consumer, not built yet.** [`ConnectionAttributionResolver`]
//! is a standalone, independently testable unit; nothing in this crate
//! calls it today. The proxy's TCP-accept loop already discards the peer
//! address it would need to pass in (see `egress_proxy`'s accept loop) —
//! wiring that hand-off, plus TLS termination and credential injection, is
//! W6's job.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bollard::{Docker, container::ListContainersOptions, models::ContainerSummary};
use ironclaw_host_api::ids::{TenantId, UserId};

use ironclaw_host_api::process::RuntimeProcessError;

use crate::sandbox_process::registry::{label_tenant, label_user};

/// Outcome of resolving a peer IP to an owning `{tenant, user}`. See the
/// module doc's "Fail closed" section for exactly which conditions collapse
/// to `Unattributed` — there is no partial/best-guess variant by design.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // consumed by W6 (proxy TLS termination + credential injection); not wired yet
pub(crate) enum ConnectionAttribution {
    Attributed {
        tenant_id: TenantId,
        user_id: UserId,
    },
    Unattributed,
}

/// Seam over the Docker container listing, so unit tests can drive every
/// branch of [`ConnectionAttributionResolver`] without a daemon. The
/// production impl is `NetworkContainerLookup for Docker` below.
#[async_trait]
pub(crate) trait NetworkContainerLookup: Send + Sync {
    async fn containers_on_network(
        &self,
        network: &str,
    ) -> Result<Vec<ContainerSummary>, RuntimeProcessError>;
}

#[async_trait]
impl NetworkContainerLookup for Docker {
    async fn containers_on_network(
        &self,
        network: &str,
    ) -> Result<Vec<ContainerSummary>, RuntimeProcessError> {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("network".to_string(), vec![network.to_string()]);
        self.list_containers(Some(ListContainersOptions {
            // Only running containers hold a live IP on the network — a
            // stopped (idle-parked, see `reaper`) container cannot be the
            // peer of an open TCP connection, so restricting to running
            // containers here is a correctness narrowing, not just an
            // optimization.
            all: false,
            filters,
            ..Default::default()
        }))
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "attribution container list failed: {error}"
            ))
        })
    }
}

/// Default cache TTL for [`ConnectionAttributionResolver`]. See the type's
/// doc comment for the staleness tradeoff this encodes.
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) const DEFAULT_ATTRIBUTION_CACHE_TTL: Duration = Duration::from_secs(5);

struct CacheEntry {
    attribution: ConnectionAttribution,
    inserted_at: Instant,
}

/// Caches peer-IP -> `{tenant, user}` resolutions so the proxy is not
/// forced to re-query Docker on every request of a long-lived connection.
///
/// **Cache invalidation strategy: bounded TTL (default
/// [`DEFAULT_ATTRIBUTION_CACHE_TTL`]) plus explicit [`Self::invalidate`].**
/// Container IPs are reused after teardown (Docker recycles addresses from
/// the subnet pool), so a cache that never expires could attribute a new
/// container's connection to the *previous* tenant/user that held its IP —
/// the exact failure this whole design exists to prevent. A bounded TTL
/// bounds that exposure to, honestly, up to `cache_ttl` of wall-clock time:
/// if a container holding IP X is torn down and a different user's
/// container is assigned the same IP X within the TTL window, a connection
/// from the new container could be attributed to the old owner until the
/// entry expires and is re-queried. `invalidate` exists so a caller that
/// *does* know about a teardown event (e.g. a future hook from `reaper`'s
/// stop/remove path) can collapse that window *toward* zero for the IPs it
/// knows changed, rather than relying on TTL expiry alone.
///
/// **`invalidate` is not itself race-free against a concurrent [`Self::resolve`].**
/// `resolve`'s Docker query runs outside the cache lock (see that method's
/// doc), so a call sequence of resolve-misses -> invalidate -> the in-flight
/// query returns its (now stale) pre-teardown result -> resolve unconditionally
/// re-inserts it can resurrect an entry `invalidate` just removed. Closing
/// that race needs a generation/version check threaded through `resolve` and
/// `invalidate` — deferred until W6/`reaper` actually calls `invalidate`
/// concurrently with in-flight resolves, so the fix is shaped by the real
/// call pattern rather than guessed here (same reasoning as the thundering-herd
/// tradeoff on `resolve`, below). Until such a hook is wired, the TTL is the
/// only bound — keep it short relative to how often containers are recycled
/// if this is tightened further.
pub(crate) struct ConnectionAttributionResolver<L: NetworkContainerLookup = Docker> {
    lookup: L,
    network: String,
    label_prefix: String,
    cache: Mutex<HashMap<IpAddr, CacheEntry>>,
    cache_ttl: Duration,
}

impl ConnectionAttributionResolver<Docker> {
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn new(
        docker: Docker,
        network: impl Into<String>,
        label_prefix: impl Into<String>,
    ) -> Self {
        Self::with_lookup(docker, network, label_prefix)
    }
}

impl<L: NetworkContainerLookup> ConnectionAttributionResolver<L> {
    pub(crate) fn with_lookup(
        lookup: L,
        network: impl Into<String>,
        label_prefix: impl Into<String>,
    ) -> Self {
        Self {
            lookup,
            network: network.into(),
            label_prefix: label_prefix.into(),
            cache: Mutex::new(HashMap::new()),
            cache_ttl: DEFAULT_ATTRIBUTION_CACHE_TTL,
        }
    }

    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn with_cache_ttl(mut self, cache_ttl: Duration) -> Self {
        self.cache_ttl = cache_ttl;
        self
    }

    /// Resolves `peer_ip` to its owning `{tenant, user}`, consulting the
    /// cache first. See the type doc for the cache's staleness guarantee.
    ///
    /// **Known tradeoff, not fixed here:** the Docker query below runs
    /// outside the cache lock, so concurrent misses for the same (or
    /// different) IPs each independently query Docker rather than coalescing
    /// onto one in-flight request — a thundering herd during a connection
    /// burst. Coalescing needs to know the real call shape (one `resolve`
    /// per TCP accept? fanned out?), which only exists once W6 wires the
    /// proxy's accept loop to this resolver; building a dedup mechanism
    /// blind, before that caller exists, risks the wrong shape. Left as a
    /// W6-time follow-up rather than spec'd speculatively here.
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) async fn resolve(&self, peer_ip: IpAddr) -> ConnectionAttribution {
        if let Some(cached) = self.cached(peer_ip) {
            return cached;
        }
        let attribution = self.query(peer_ip).await;
        let mut cache = self.lock_cache();
        // A miss is the one guaranteed opportunity to sweep everyone else's
        // expired entries too — without this, an entry past its TTL is never
        // removed (only ever skipped by `cached`'s elapsed check), so a
        // long-running proxy seeing a stream of distinct, never-repeating
        // peer IPs would grow this map without bound.
        let cache_ttl = self.cache_ttl;
        cache.retain(|_, entry| entry.inserted_at.elapsed() <= cache_ttl);
        cache.insert(
            peer_ip,
            CacheEntry {
                attribution: attribution.clone(),
                inserted_at: Instant::now(),
            },
        );
        attribution
    }

    /// Explicit invalidation for a caller that knows `peer_ip`'s owning
    /// container was just torn down — collapses the staleness window
    /// toward zero for that IP instead of waiting out the TTL, but is not
    /// itself race-free against a concurrent in-flight [`Self::resolve`].
    /// See the type doc's "not race-free" section.
    #[allow(dead_code)] // consumed by W6 / a future reaper teardown hook; not wired yet
    pub(crate) fn invalidate(&self, peer_ip: IpAddr) {
        self.lock_cache().remove(&peer_ip);
    }

    fn cached(&self, peer_ip: IpAddr) -> Option<ConnectionAttribution> {
        let cache = self.lock_cache();
        let entry = cache.get(&peer_ip)?;
        if entry.inserted_at.elapsed() > self.cache_ttl {
            return None;
        }
        Some(entry.attribution.clone())
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, CacheEntry>> {
        self.cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[cfg(test)]
    fn cache_len(&self) -> usize {
        self.lock_cache().len()
    }

    async fn query(&self, peer_ip: IpAddr) -> ConnectionAttribution {
        let containers = match self.lookup.containers_on_network(&self.network).await {
            Ok(containers) => containers,
            Err(error) => {
                tracing::debug!(?error, %peer_ip, "attribution: container query failed");
                return ConnectionAttribution::Unattributed;
            }
        };

        let mut matches = containers.iter().filter(|container| {
            container_addresses_on_network(container, &self.network).any(|ip| ip == peer_ip)
        });

        let Some(first) = matches.next() else {
            return ConnectionAttribution::Unattributed;
        };
        if matches.next().is_some() {
            // Fail closed rather than guess "first match" — see module doc.
            tracing::debug!(
                %peer_ip,
                "attribution: multiple containers report this peer ip on the egress network, refusing to attribute"
            );
            return ConnectionAttribution::Unattributed;
        }

        match parse_attribution_labels(first, &self.label_prefix) {
            Some((tenant_id, user_id)) => ConnectionAttribution::Attributed { tenant_id, user_id },
            None => {
                tracing::debug!(
                    %peer_ip,
                    "attribution: matched container missing or has malformed tenant/user labels"
                );
                ConnectionAttribution::Unattributed
            }
        }
    }
}

/// Yields every address `container` holds on `network` — bollard reports
/// IPv4 and IPv6 in separate `EndpointSettings` fields (`ip_address` and
/// `global_ipv6_address`), and a dual-stack container can legitimately have
/// both, so this checks both rather than only `ip_address` (which would
/// silently make an IPv6-only peer connection un-matchable). Yields nothing
/// if network-settings are absent, the named network is missing from the
/// container's network map, or neither address field parses.
fn container_addresses_on_network<'a>(
    container: &'a ContainerSummary,
    network: &'a str,
) -> impl Iterator<Item = IpAddr> + 'a {
    container
        .network_settings
        .as_ref()
        .and_then(|settings| settings.networks.as_ref())
        .and_then(|networks| networks.get(network))
        .into_iter()
        .flat_map(|endpoint| {
            [
                endpoint.ip_address.as_deref(),
                endpoint.global_ipv6_address.as_deref(),
            ]
            .into_iter()
            .flatten()
            .filter(|ip| !ip.is_empty())
            // silent-ok: an unparseable Docker-reported address string can
            // only mean "this container has no usable address of this
            // family on this network" for attribution purposes — the
            // caller's `query` loop already treats "no address equals
            // `peer_ip`" as no match, so folding a malformed address into
            // that same case still fails closed (`Unattributed`) rather
            // than mis-attributing; it never needs the parse error itself.
            .filter_map(|ip| ip.parse().ok())
        })
}

/// Parses the `{tenant, user}` labels off `container` using the same
/// `registry::label_tenant`/`registry::label_user` key functions the
/// container-creation path (`registry::build_user_container_labels`) writes
/// with — the label vocabulary lives in exactly one place. `None` when
/// either label is missing or fails newtype validation: a malformed label
/// set is rejected outright rather than partially parsed (e.g. a valid
/// tenant with a garbage user is still `None`, never `Attributed` with a
/// half-trusted identity).
fn parse_attribution_labels(
    container: &ContainerSummary,
    label_prefix: &str,
) -> Option<(TenantId, UserId)> {
    let labels = container.labels.as_ref()?;
    // silent-ok: a label that fails `TenantId`/`UserId` newtype validation is
    // exactly the "malformed label set" case this function's doc says must
    // collapse to `None` (never a half-trusted `Attributed`) — the caller
    // already logs at the `Unattributed` outcome this produces, so the
    // per-field validation error itself is redundant, not swallowed.
    let tenant_id = labels
        .get(&label_tenant(label_prefix))
        .and_then(|value| TenantId::new(value).ok())?;
    let user_id = labels
        .get(&label_user(label_prefix))
        .and_then(|value| UserId::new(value).ok())?;
    Some((tenant_id, user_id))
}

// This PR ships `attribution` without `exec_transport` (out of scope — see
// the PR description: `exec_transport` is ~2000 lines and would blow the PR
// size budget). Upstream, `exec_transport` declares this same
// `tests/support/docker_gate.rs` via `#[path]` at its own file's top level
// and `attribution`'s test reuses that single module instance (clippy's
// `duplicate_mod` lint flags loading one file into two module locations).
// With `exec_transport` absent here, this is the only place in this PR that
// needs the gate, so it declares its own instance directly — at this file's
// top level (a `#[path]` module must sit next to a real sibling directory on
// disk to resolve relative `..` segments; it cannot be nested inside the
// `tests` module below, which has no corresponding directory on disk since
// this file is `attribution.rs`, not `attribution/mod.rs`).
#[cfg(test)]
#[path = "../../tests/support/docker_gate.rs"]
mod docker_gate;

#[cfg(test)]
#[path = "attribution_tests.rs"]
mod tests;
