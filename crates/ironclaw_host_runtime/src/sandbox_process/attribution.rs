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
use ironclaw_host_api::{TenantId, UserId};

use crate::RuntimeProcessError;

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
/// stop/remove path) can collapse that window to zero for the IPs it knows
/// changed, rather than relying on TTL expiry alone. Until such a hook is
/// wired, the TTL is the only bound — keep it short relative to how often
/// containers are recycled if this is tightened further.
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
    /// container was just torn down — collapses the staleness window to
    /// zero for that IP instead of waiting out the TTL. See the type doc.
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

        let mut matches = containers
            .iter()
            .filter(|container| container_ip_on_network(container, &self.network) == Some(peer_ip));

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

/// Reads `container`'s IPv4/IPv6 address on `network`, or `None` if the
/// container has no recorded address there (network-settings absent, the
/// named network missing from its network map, or an empty/unparseable
/// address string).
fn container_ip_on_network(container: &ContainerSummary, network: &str) -> Option<IpAddr> {
    container
        .network_settings
        .as_ref()?
        .networks
        .as_ref()?
        .get(network)?
        .ip_address
        .as_deref()
        .filter(|ip| !ip.is_empty())
        .and_then(|ip| ip.parse().ok())
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use bollard::models::{ContainerSummaryNetworkSettings, EndpointSettings};

    const NETWORK: &str = "ironclaw-sandbox-egress";
    const PREFIX: &str = "ironclaw";

    fn container_with(
        id: &str,
        ip: Option<&str>,
        labels: Option<HashMap<String, String>>,
    ) -> ContainerSummary {
        let networks = ip.map(|ip| {
            HashMap::from([(
                NETWORK.to_string(),
                EndpointSettings {
                    ip_address: Some(ip.to_string()),
                    ..Default::default()
                },
            )])
        });
        ContainerSummary {
            id: Some(id.to_string()),
            labels,
            network_settings: Some(ContainerSummaryNetworkSettings { networks }),
            ..Default::default()
        }
    }

    fn labels(tenant: &str, user: &str) -> HashMap<String, String> {
        HashMap::from([
            (label_tenant(PREFIX), tenant.to_string()),
            (label_user(PREFIX), user.to_string()),
        ])
    }

    /// Counts calls and returns a fixed, pre-programmed container list —
    /// lets tests assert cache hit/miss behavior precisely.
    #[derive(Default)]
    struct FakeLookup {
        calls: AtomicUsize,
        containers: Vec<ContainerSummary>,
    }

    impl FakeLookup {
        fn new(containers: Vec<ContainerSummary>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                containers,
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl NetworkContainerLookup for FakeLookup {
        async fn containers_on_network(
            &self,
            _network: &str,
        ) -> Result<Vec<ContainerSummary>, RuntimeProcessError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.containers.clone())
        }
    }

    struct FailingLookup;

    #[async_trait]
    impl NetworkContainerLookup for FailingLookup {
        async fn containers_on_network(
            &self,
            _network: &str,
        ) -> Result<Vec<ContainerSummary>, RuntimeProcessError> {
            Err(RuntimeProcessError::ExecutionFailed("boom".to_string()))
        }
    }

    #[tokio::test]
    async fn known_ip_resolves_to_its_labeled_tenant_and_user() {
        let lookup = FakeLookup::new(vec![container_with(
            "c1",
            Some("10.200.0.5"),
            Some(labels("tenant-a", "user-a")),
        )]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(
            outcome,
            ConnectionAttribution::Attributed {
                tenant_id: TenantId::new("tenant-a").unwrap(),
                user_id: UserId::new("user-a").unwrap(),
            }
        );
    }

    #[tokio::test]
    async fn unknown_ip_is_unattributed() {
        let lookup = FakeLookup::new(vec![container_with(
            "c1",
            Some("10.200.0.5"),
            Some(labels("tenant-a", "user-a")),
        )]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.9".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn two_containers_with_different_ips_never_cross_attribute() {
        let lookup = FakeLookup::new(vec![
            container_with("c1", Some("10.200.0.5"), Some(labels("tenant-a", "user-a"))),
            container_with("c2", Some("10.200.0.6"), Some(labels("tenant-b", "user-b"))),
        ]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let first = resolver.resolve("10.200.0.5".parse().unwrap()).await;
        let second = resolver.resolve("10.200.0.6".parse().unwrap()).await;

        assert_eq!(
            first,
            ConnectionAttribution::Attributed {
                tenant_id: TenantId::new("tenant-a").unwrap(),
                user_id: UserId::new("user-a").unwrap(),
            }
        );
        assert_eq!(
            second,
            ConnectionAttribution::Attributed {
                tenant_id: TenantId::new("tenant-b").unwrap(),
                user_id: UserId::new("user-b").unwrap(),
            }
        );
    }

    #[tokio::test]
    async fn duplicate_ip_on_two_containers_refuses_to_guess() {
        let lookup = FakeLookup::new(vec![
            container_with("c1", Some("10.200.0.5"), Some(labels("tenant-a", "user-a"))),
            container_with("c2", Some("10.200.0.5"), Some(labels("tenant-b", "user-b"))),
        ]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn missing_labels_are_rejected() {
        let lookup = FakeLookup::new(vec![container_with("c1", Some("10.200.0.5"), None)]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn partial_label_set_is_rejected_not_partially_parsed() {
        // Tenant label present and valid, user label missing entirely: a
        // half-trusted identity must never surface as `Attributed`.
        let only_tenant = HashMap::from([(label_tenant(PREFIX), "tenant-a".to_string())]);
        let lookup = FakeLookup::new(vec![container_with(
            "c1",
            Some("10.200.0.5"),
            Some(only_tenant),
        )]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn malformed_label_value_is_rejected() {
        // "/" fails `UserId`'s scope-id validation (path separators
        // forbidden) — a corrupt/tampered label must not parse partially.
        let lookup = FakeLookup::new(vec![container_with(
            "c1",
            Some("10.200.0.5"),
            Some(labels("tenant-a", "user/../escape")),
        )]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn docker_query_failure_is_unattributed_not_a_panic() {
        let resolver = ConnectionAttributionResolver::with_lookup(FailingLookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn cache_hit_does_not_requery_docker() {
        let lookup = FakeLookup::new(vec![container_with(
            "c1",
            Some("10.200.0.5"),
            Some(labels("tenant-a", "user-a")),
        )]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);
        let ip = "10.200.0.5".parse().unwrap();

        resolver.resolve(ip).await;
        resolver.resolve(ip).await;
        resolver.resolve(ip).await;

        assert_eq!(resolver.lookup.call_count(), 1);
    }

    #[tokio::test]
    async fn expired_cache_entry_requeries_docker() {
        let lookup = FakeLookup::new(vec![container_with(
            "c1",
            Some("10.200.0.5"),
            Some(labels("tenant-a", "user-a")),
        )]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX)
            .with_cache_ttl(Duration::from_millis(1));
        let ip = "10.200.0.5".parse().unwrap();

        resolver.resolve(ip).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        resolver.resolve(ip).await;

        assert_eq!(resolver.lookup.call_count(), 2);
    }

    #[tokio::test]
    async fn container_with_no_network_settings_is_unattributed() {
        let container = ContainerSummary {
            id: Some("c1".to_string()),
            labels: Some(labels("tenant-a", "user-a")),
            network_settings: None,
            ..Default::default()
        };
        let lookup = FakeLookup::new(vec![container]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn container_on_a_different_network_is_unattributed() {
        let networks = HashMap::from([(
            "some-other-network".to_string(),
            EndpointSettings {
                ip_address: Some("10.200.0.5".to_string()),
                ..Default::default()
            },
        )]);
        let container = ContainerSummary {
            id: Some("c1".to_string()),
            labels: Some(labels("tenant-a", "user-a")),
            network_settings: Some(ContainerSummaryNetworkSettings {
                networks: Some(networks),
            }),
            ..Default::default()
        };
        let lookup = FakeLookup::new(vec![container]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn container_with_empty_ip_string_is_unattributed() {
        let lookup = FakeLookup::new(vec![container_with(
            "c1",
            Some(""),
            Some(labels("tenant-a", "user-a")),
        )]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn container_with_unparseable_ip_is_unattributed() {
        let lookup = FakeLookup::new(vec![container_with(
            "c1",
            Some("not-an-ip"),
            Some(labels("tenant-a", "user-a")),
        )]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

        let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

        assert_eq!(outcome, ConnectionAttribution::Unattributed);
    }

    #[tokio::test]
    async fn expired_entries_are_evicted_not_just_skipped() {
        // A regression for unbounded growth: the TTL alone only ever causes
        // a *miss* on lookup (see `cached`) — nothing previously removed an
        // expired entry from the map, so a resolver that keeps seeing new,
        // never-repeating peer IPs would retain one `CacheEntry` per IP
        // forever. Resolving a distinct IP after the first entry expires
        // must shrink the cache back down instead of growing it.
        let lookup = FakeLookup::new(vec![
            container_with("c1", Some("10.200.0.5"), Some(labels("tenant-a", "user-a"))),
            container_with("c2", Some("10.200.0.6"), Some(labels("tenant-b", "user-b"))),
        ]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX)
            .with_cache_ttl(Duration::from_millis(1));

        resolver.resolve("10.200.0.5".parse().unwrap()).await;
        assert_eq!(resolver.cache_len(), 1);

        tokio::time::sleep(Duration::from_millis(20)).await;
        resolver.resolve("10.200.0.6".parse().unwrap()).await;

        assert_eq!(
            resolver.cache_len(),
            1,
            "expired entry for 10.200.0.5 should have been swept, leaving only the fresh 10.200.0.6 entry"
        );
    }

    #[tokio::test]
    async fn concurrent_resolve_calls_complete_with_consistent_attribution() {
        // `resolve` checks the cache, awaits the Docker query outside the
        // lock, then re-locks to sweep+insert — so concurrent callers race
        // through that miss/query/insert window. This drives many
        // simultaneous `resolve` calls for two distinct IPs sharing one
        // resolver and asserts every call lands on the correct owner, with
        // no panic/deadlock across the shared cache mutex.
        use std::sync::Arc;

        let lookup = FakeLookup::new(vec![
            container_with("c1", Some("10.200.0.5"), Some(labels("tenant-a", "user-a"))),
            container_with("c2", Some("10.200.0.6"), Some(labels("tenant-b", "user-b"))),
        ]);
        let resolver = Arc::new(ConnectionAttributionResolver::with_lookup(
            lookup, NETWORK, PREFIX,
        ));
        let ip_a: IpAddr = "10.200.0.5".parse().unwrap();
        let ip_b: IpAddr = "10.200.0.6".parse().unwrap();

        let mut tasks = Vec::new();
        for index in 0..20 {
            let resolver = Arc::clone(&resolver);
            let ip = if index % 2 == 0 { ip_a } else { ip_b };
            tasks.push(tokio::spawn(
                async move { (ip, resolver.resolve(ip).await) },
            ));
        }

        for task in tasks {
            let (ip, outcome) = task.await.expect("resolve task must not panic");
            let expected = if ip == ip_a {
                ConnectionAttribution::Attributed {
                    tenant_id: TenantId::new("tenant-a").unwrap(),
                    user_id: UserId::new("user-a").unwrap(),
                }
            } else {
                ConnectionAttribution::Attributed {
                    tenant_id: TenantId::new("tenant-b").unwrap(),
                    user_id: UserId::new("user-b").unwrap(),
                }
            };
            assert_eq!(outcome, expected);
        }
    }

    #[tokio::test]
    async fn explicit_invalidate_forces_a_requery() {
        let lookup = FakeLookup::new(vec![container_with(
            "c1",
            Some("10.200.0.5"),
            Some(labels("tenant-a", "user-a")),
        )]);
        let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);
        let ip = "10.200.0.5".parse().unwrap();

        resolver.resolve(ip).await;
        assert_eq!(resolver.lookup.call_count(), 1);

        resolver.invalidate(ip);
        resolver.resolve(ip).await;

        assert_eq!(resolver.lookup.call_count(), 2);
    }

    /// Real-Docker check: a live container on the real egress network
    /// resolves to its real `{tenant, user}` labels via the production
    /// `NetworkContainerLookup for Docker` impl, not just the fake seam
    /// above. Follows this crate's existing gated-real-Docker convention
    /// (see this test module's `docker_gate` declaration above).
    ///
    /// Connects via `super::super::connect_docker()` — the same
    /// local-defaults-then-`unix_socket_candidates()` (`~/.colima/...`,
    /// `~/.rd/...`, etc.) fallback production uses — rather than
    /// `Docker::connect_with_local_defaults()` directly. `docker_gate::
    /// docker_available()` shells out to the `docker` CLI, which resolves
    /// the daemon through whatever context is active (Colima, Docker
    /// Desktop, a remote host); `connect_with_local_defaults()` alone only
    /// honors `DOCKER_HOST` or the hardcoded `/var/run/docker.sock`, so the
    /// CLI gate could report "available" while that direct connect still
    /// fails on any machine using a non-default socket. Reusing
    /// `connect_docker()` makes "available" mean the same thing to the gate
    /// and the connection, and — since even that broader fallback can't
    /// cover every possible context (e.g. a genuinely remote `DOCKER_HOST`)
    /// — a failure here is still a `SKIP`, never a panic/unwrap.
    #[tokio::test]
    async fn real_docker_resolves_a_live_container_on_the_egress_network() {
        if !docker_gate::docker_available() {
            eprintln!(
                "SKIP: no docker daemon reachable — real_docker_resolves_a_live_container_on_the_egress_network requires a real Docker daemon (CI/hosted Docker lane only)"
            );
            return;
        }

        let docker = match super::super::connect_docker().await {
            Ok(docker) => docker,
            Err(error) => {
                // `docker_available()` reached a daemon through the `docker`
                // CLI's context resolution, but `connect_docker()`'s
                // narrower local-defaults + known-socket fallback could
                // not — e.g. a `DOCKER_HOST` context the CLI understands but
                // this fallback list doesn't cover. Under
                // `IRONCLAW_REQUIRE_DOCKER_TESTS=1` that gap must not
                // silently pass this required security test without ever
                // exercising attribution, so panic here exactly like
                // `docker_gate::docker_available()` does for its own
                // required-but-unreachable case; only the optional local
                // path gets the visible skip-and-return.
                if docker_gate::docker_tests_required() {
                    panic!(
                        "IRONCLAW_REQUIRE_DOCKER_TESTS=1 but connect_docker() could not reach \
                         the daemon docker_available() found via the `docker` CLI ({error}) — \
                         docker-gated tests must not silently skip in CI"
                    );
                }
                eprintln!(
                    "SKIP: docker_available() reported a reachable daemon via the `docker` CLI \
                     (context-aware), but connect_docker()'s local-defaults + known-socket \
                     fallback could not reach it ({error}) — \
                     real_docker_resolves_a_live_container_on_the_egress_network requires a \
                     daemon reachable at one of those paths (CI/hosted Docker lane only)"
                );
                return;
            }
        };
        let network_name = format!("ironclaw-test-attribution-{}", uuid::Uuid::new_v4());
        let tenant = TenantId::new("attribution-tenant").unwrap();
        let user = UserId::new("attribution-user").unwrap();

        // The CI Docker runner has a working daemon but not necessarily this
        // image cached — pull it explicitly rather than relying on whatever
        // happens to already be present (a bare `create_container` below
        // would otherwise pass only on a machine that already has the image,
        // which is exactly the gap that let this test fail in CI while
        // passing on a developer's laptop). Draining the stream to
        // completion waits for the pull to finish before we try to use the
        // image.
        use futures_util::StreamExt as _;
        let mut pull_stream = docker.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: "busybox",
                tag: "1.36",
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(progress) = pull_stream.next().await {
            progress.expect("busybox:1.36 image pull succeeds");
        }

        docker
            .create_network(bollard::network::CreateNetworkOptions {
                name: network_name.clone(),
                internal: true,
                ..Default::default()
            })
            .await
            .expect("test network create succeeds");

        let container_name = format!("ironclaw-test-attribution-c-{}", uuid::Uuid::new_v4());
        // The security-posture stamp (W16) is irrelevant to attribution — this
        // test only cares that the tenant/user labels resolve from an IP — so
        // any non-empty stamp value works here.
        let create_labels = super::super::registry::build_user_container_labels(
            PREFIX,
            &tenant,
            &user,
            "attribution-test-posture-stamp",
        );
        let created = docker
            .create_container(
                Some(bollard::container::CreateContainerOptions {
                    name: container_name.clone(),
                    platform: None,
                }),
                bollard::container::Config {
                    image: Some("busybox:1.36".to_string()),
                    cmd: Some(vec!["sleep".to_string(), "60".to_string()]),
                    labels: Some(create_labels),
                    host_config: Some(bollard::models::HostConfig {
                        network_mode: Some(network_name.clone()),
                        auto_remove: Some(false),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .expect("test container create succeeds");
        docker
            .start_container(
                &created.id,
                None::<bollard::container::StartContainerOptions<String>>,
            )
            .await
            .expect("test container start succeeds");

        let inspected = docker
            .inspect_container(
                &created.id,
                None::<bollard::container::InspectContainerOptions>,
            )
            .await
            .expect("test container inspect succeeds");
        let ip: IpAddr = inspected
            .network_settings
            .and_then(|settings| settings.networks)
            .and_then(|networks| networks.get(&network_name).cloned())
            .and_then(|endpoint| endpoint.ip_address)
            .filter(|ip| !ip.is_empty())
            .expect("test container has an ip on the test network")
            .parse()
            .expect("test container ip parses");

        let resolver =
            ConnectionAttributionResolver::new(docker.clone(), network_name.clone(), PREFIX);
        let outcome = resolver.resolve(ip).await;

        // Best-effort cleanup regardless of assertion outcome, so a failed
        // assertion never leaves the daemon dirty.
        let _ = docker
            .remove_container(
                &created.id,
                Some(bollard::container::RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
        let _ = docker.remove_network(&network_name).await;

        assert_eq!(
            outcome,
            ConnectionAttribution::Attributed {
                tenant_id: tenant,
                user_id: user,
            }
        );
    }
}
