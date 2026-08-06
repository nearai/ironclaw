//! Unit + gated real-Docker tests for [`super::ConnectionAttributionResolver`].
//! Split out of `attribution.rs` (see that file's `mod tests` declaration)
//! purely to keep the production resolver file under this repo's file-size
//! target — this module is `#[cfg(test)]`-only and changes no behavior.

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
/// lets tests assert cache hit/miss behavior precisely. An optional
/// `barrier` forces every concurrent caller to land inside this async
/// fn's body at the same time, so a test can prove tasks actually
/// overlap in the miss/query/insert window instead of merely hoping
/// the scheduler interleaves them.
#[derive(Default)]
struct FakeLookup {
    calls: AtomicUsize,
    containers: Vec<ContainerSummary>,
    barrier: Option<std::sync::Arc<tokio::sync::Barrier>>,
}

impl FakeLookup {
    fn new(containers: Vec<ContainerSummary>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            containers,
            barrier: None,
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Every call to `containers_on_network` waits on `barrier` before
    /// returning, so `barrier`'s party count callers all reach that
    /// point before any of them proceeds to insert into the cache.
    fn with_barrier(mut self, barrier: std::sync::Arc<tokio::sync::Barrier>) -> Self {
        self.barrier = Some(barrier);
        self
    }
}

#[async_trait]
impl NetworkContainerLookup for FakeLookup {
    async fn containers_on_network(
        &self,
        _network: &str,
    ) -> Result<Vec<ContainerSummary>, RuntimeProcessError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(barrier) = &self.barrier {
            barrier.wait().await;
        }
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
async fn empty_container_listing_is_unattributed() {
    // An empty running-container response (e.g. the egress network has
    // no live containers yet) must fail closed exactly like "no match
    // found among several", not panic on an empty iterator or otherwise
    // special-case zero containers.
    let lookup = FakeLookup::new(Vec::new());
    let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

    let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

    assert_eq!(outcome, ConnectionAttribution::Unattributed);
}

#[tokio::test]
async fn ipv6_container_address_resolves_to_its_labeled_tenant_and_user() {
    // Regression for `container_addresses_on_network` only reading
    // bollard's IPv4 `ip_address` field: a container with only a
    // `global_ipv6_address` on the network must still be matchable.
    let networks = HashMap::from([(
        NETWORK.to_string(),
        EndpointSettings {
            global_ipv6_address: Some("fd00::5".to_string()),
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

    let outcome = resolver.resolve("fd00::5".parse().unwrap()).await;

    assert_eq!(
        outcome,
        ConnectionAttribution::Attributed {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("user-a").unwrap(),
        }
    );
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
async fn missing_tenant_label_is_rejected() {
    // Mirror of `partial_label_set_is_rejected_not_partially_parsed`, but
    // with the tenant label missing and the user label present/valid —
    // `parse_attribution_labels` must fail closed on this half-labeled
    // shape too, not only on the missing-user shape.
    let only_user = HashMap::from([(label_user(PREFIX), "user-a".to_string())]);
    let lookup = FakeLookup::new(vec![container_with(
        "c1",
        Some("10.200.0.5"),
        Some(only_user),
    )]);
    let resolver = ConnectionAttributionResolver::with_lookup(lookup, NETWORK, PREFIX);

    let outcome = resolver.resolve("10.200.0.5".parse().unwrap()).await;

    assert_eq!(outcome, ConnectionAttribution::Unattributed);
}

#[tokio::test]
async fn malformed_tenant_label_value_is_rejected() {
    // "/" fails `TenantId`'s scope-id validation (path separators
    // forbidden), mirroring `malformed_label_value_is_rejected` for the
    // tenant field instead of the user field.
    let lookup = FakeLookup::new(vec![container_with(
        "c1",
        Some("10.200.0.5"),
        Some(labels("tenant/../escape", "user-a")),
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
    //
    // `FakeLookup::containers_on_network` returns immediately with no
    // yield point of its own, so without forcing one, tasks could run
    // to completion sequentially and never actually overlap inside the
    // miss/query/insert window this test claims to exercise. The
    // barrier (party count == task count) makes every one of the 20
    // calls land inside `containers_on_network` before any of them is
    // allowed to proceed to the insert — this is sound because the
    // cache starts empty, so all 20 `resolve` calls miss and reach the
    // lookup before any has inserted.
    use std::sync::Arc;

    let barrier = Arc::new(tokio::sync::Barrier::new(20));
    let lookup = FakeLookup::new(vec![
        container_with("c1", Some("10.200.0.5"), Some(labels("tenant-a", "user-a"))),
        container_with("c2", Some("10.200.0.6"), Some(labels("tenant-b", "user-b"))),
    ])
    .with_barrier(Arc::clone(&barrier));
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
        // Bounded so a regression that prevents even one of the 20 tasks
        // from reaching the barrier fails this test loudly instead of
        // hanging CI indefinitely.
        let (ip, outcome) = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("concurrent resolve tasks must not deadlock")
            .expect("resolve task must not panic");
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
/// (see `attribution.rs`'s `docker_gate` declaration).
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

    let resolver = ConnectionAttributionResolver::new(docker.clone(), network_name.clone(), PREFIX);
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
