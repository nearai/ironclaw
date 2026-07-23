//! Test-support host rewrite seam for vendor HTTP egress (env-gated,
//! fail-closed).
//!
//! E2E harnesses need the composed runtime's vendor egress (webhook replies,
//! OAuth token exchanges) to land on local fake vendor APIs instead of the
//! real internet. This module is the ONE seam that makes that possible:
//! [`RewriteNetworkTransport`] wraps the production transport and — only when
//! `IRONCLAW_TEST_HTTP_REWRITE_MAP` is set — redirects the resolved
//! connection target for the mapped hosts to a loopback address.
//! `IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP` remains a compatibility fallback.
//!
//! Guard rails (mirrors the spirit of the v1 escape hatch in
//! `src/tools/wasm/http_security.rs`):
//!
//! - **Inert by default.** Without the env var the wrapper is a zero-cost
//!   passthrough — no parsing, no behavior change.
//! - **Fail-closed activation.** A set-but-malformed map is a hard
//!   construction error (the process refuses to boot) rather than a silent
//!   fallback to real vendor egress mid-test.
//! - **Loopback-only targets.** Rewrite targets must be loopback IP literals
//!   (`127.0.0.0/8`, `::1`); anything else is rejected at parse time.
//! - **Debug builds only.** A release build refuses to activate the map at
//!   all (hard error when the env var is set), so production binaries cannot
//!   be redirected.
//! - **Policy stays intact.** The rewrite happens at the *transport* layer,
//!   AFTER `PolicyNetworkHttpEgress` authorized the vendor URL against the
//!   network policy and resolved/screened its public IPs. Only the resolved
//!   connection target changes; no allowlist, SSRF, or header check is
//!   weakened.

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, OnceLock},
};

use async_trait::async_trait;

use crate::{
    egress::{NetworkHttpTransport, PolicyNetworkHttpEgress},
    error::NetworkHttpError,
    transport::ReqwestNetworkTransport,
    types::{NetworkHttpResponse, NetworkTransportRequest},
};

/// Environment variable carrying the test-only rewrite map:
/// `host=127.0.0.1:PORT[,host2=127.0.0.1:PORT2...]`.
pub const TEST_HTTP_REWRITE_MAP_ENV: &str = "IRONCLAW_TEST_HTTP_REWRITE_MAP";
const LEGACY_TEST_HTTP_REWRITE_MAP_ENV: &str = "IRONCLAW_REBORN_TEST_HTTP_REWRITE_MAP";

/// Construction-time failures for the rewrite seam. All fail-closed: the
/// caller must treat any of these as fatal to process startup.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostRewriteMapError {
    #[error("{TEST_HTTP_REWRITE_MAP_ENV} entry is malformed: {reason}")]
    Malformed { reason: String },
    #[error(
        "{TEST_HTTP_REWRITE_MAP_ENV} target for `{host}` is not a loopback IP literal; \
         only 127.0.0.0/8 and ::1 targets are allowed"
    )]
    NotLoopback { host: String },
    #[error(
        "{TEST_HTTP_REWRITE_MAP_ENV} is set but this is a release build; \
         the test-only HTTP rewrite seam is only available in debug builds"
    )]
    UnavailableInRelease,
}

/// Parsed `host -> loopback target` map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRewriteMap {
    entries: BTreeMap<String, SocketAddr>,
}

impl HostRewriteMap {
    /// Parse the env-var value. Every entry must be `host=ip:port` with a
    /// loopback IP literal target; duplicates and empty pieces are errors.
    pub fn parse(raw: &str) -> Result<Self, HostRewriteMapError> {
        let mut entries = BTreeMap::new();
        for piece in raw.split(',') {
            let piece = piece.trim();
            if piece.is_empty() {
                return Err(HostRewriteMapError::Malformed {
                    reason: "empty entry (double comma or trailing comma)".to_string(),
                });
            }
            let Some((host, target)) = piece.split_once('=') else {
                return Err(HostRewriteMapError::Malformed {
                    reason: format!("entry `{piece}` is not of the form host=ip:port"),
                });
            };
            let host = host.trim().to_ascii_lowercase();
            if host.is_empty() {
                return Err(HostRewriteMapError::Malformed {
                    reason: "entry has an empty host".to_string(),
                });
            }
            let target: SocketAddr =
                target
                    .trim()
                    .parse()
                    .map_err(|_| HostRewriteMapError::Malformed {
                        reason: format!(
                            "target for `{host}` must be an ip:port socket address literal"
                        ),
                    })?;
            if !target.ip().is_loopback() {
                return Err(HostRewriteMapError::NotLoopback { host });
            }
            if entries.insert(host.clone(), target).is_some() {
                return Err(HostRewriteMapError::Malformed {
                    reason: format!("duplicate entry for host `{host}`"),
                });
            }
        }
        if entries.is_empty() {
            return Err(HostRewriteMapError::Malformed {
                reason: "rewrite map has no entries".to_string(),
            });
        }
        Ok(Self { entries })
    }

    fn target_for(&self, host: &str) -> Option<SocketAddr> {
        self.entries.get(&host.to_ascii_lowercase()).copied()
    }
}

/// Transport wrapper applying the env-gated host rewrite map. Passthrough
/// (zero behavior change) when no map is active.
#[derive(Debug, Clone)]
pub struct RewriteNetworkTransport<T> {
    inner: T,
    map: Option<Arc<HostRewriteMap>>,
}

impl<T> RewriteNetworkTransport<T> {
    /// Wrap `inner`, activating the rewrite map from
    /// [`TEST_HTTP_REWRITE_MAP_ENV`] when set. Fail-closed: a set-but-invalid
    /// value (or a release build with the value set) is an error.
    pub fn from_env(inner: T) -> Result<Self, HostRewriteMapError> {
        let value = std::env::var(TEST_HTTP_REWRITE_MAP_ENV)
            .ok()
            .or_else(|| std::env::var(LEGACY_TEST_HTTP_REWRITE_MAP_ENV).ok());
        Self::from_env_value(inner, value.as_deref())
    }

    /// Core of [`Self::from_env`], testable without process-global env
    /// mutation. `None` / blank => inert passthrough.
    pub fn from_env_value(inner: T, value: Option<&str>) -> Result<Self, HostRewriteMapError> {
        let Some(raw) = value.map(str::trim).filter(|raw| !raw.is_empty()) else {
            return Ok(Self { inner, map: None });
        };
        if !cfg!(debug_assertions) {
            return Err(HostRewriteMapError::UnavailableInRelease);
        }
        let map = HostRewriteMap::parse(raw)?;
        // Once per process: loud, so a test-redirected run is never mistaken
        // for a production one.
        static ACTIVE_LOGGED: OnceLock<()> = OnceLock::new();
        ACTIVE_LOGGED.get_or_init(|| {
            tracing::warn!(
                env = TEST_HTTP_REWRITE_MAP_ENV,
                hosts = ?map.entries.keys().collect::<Vec<_>>(),
                "test-only HTTP host rewrite seam is ACTIVE; mapped vendor egress is \
                 redirected to loopback targets"
            );
        });
        Ok(Self {
            inner,
            map: Some(Arc::new(map)),
        })
    }

    /// Whether a rewrite map is active (for wiring assertions).
    pub fn is_active(&self) -> bool {
        self.map.is_some()
    }
}

/// Rewrite the request in place when its URL host is mapped: scheme becomes
/// `http`, host/port become the loopback target, and any pre-resolved IP
/// pins are dropped (they belong to the vendor host the policy layer
/// resolved). Unmapped hosts and unparseable URLs pass through untouched —
/// the inner transport owns URL validation and its sanitized diagnostics.
fn apply_rewrite(map: &HostRewriteMap, request: &mut NetworkTransportRequest) {
    let Ok(mut url) = url::Url::parse(&request.url) else {
        return;
    };
    let Some(host) = url.host_str() else {
        return;
    };
    let Some(target) = map.target_for(host) else {
        return;
    };
    let original_host = host.to_string();
    if url.set_scheme("http").is_err()
        || url.set_host(Some(&target.ip().to_string())).is_err()
        || url.set_port(Some(target.port())).is_err()
    {
        // A mapped host we cannot rewrite must not silently egress to the
        // real vendor mid-test; surface it loudly and leave the request
        // untouched (the vendor target was already policy-approved).
        tracing::warn!(
            host = %original_host,
            "test HTTP rewrite could not rewrite a mapped URL; leaving request untouched"
        );
        return;
    }
    tracing::debug!(
        host = %original_host,
        target = %target,
        "test HTTP rewrite redirected vendor egress to loopback"
    );
    request.url = url.to_string();
    request.resolved_ips.clear();
}

#[async_trait]
impl<T> NetworkHttpTransport for RewriteNetworkTransport<T>
where
    T: NetworkHttpTransport + Send + Sync,
{
    async fn execute(
        &self,
        mut request: NetworkTransportRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        if let Some(map) = &self.map {
            apply_rewrite(map, &mut request);
        }
        self.inner.execute(request).await
    }
}

/// The standard production HTTP egress: policy enforcement over the reqwest
/// transport, honoring the env-gated test-only host rewrite seam. This is
/// the single construction seam compositions should use so ALL vendor egress
/// behaves identically under test redirection.
pub fn default_policy_http_egress() -> Result<
    PolicyNetworkHttpEgress<RewriteNetworkTransport<ReqwestNetworkTransport>>,
    HostRewriteMapError,
> {
    Ok(PolicyNetworkHttpEgress::new(
        RewriteNetworkTransport::from_env(ReqwestNetworkTransport::default())?,
    ))
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::sync::Mutex;

    use ironclaw_host_api::NetworkMethod;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::types::NetworkUsage;

    fn request(url: &str, resolved_ips: Vec<IpAddr>) -> NetworkTransportRequest {
        NetworkTransportRequest {
            method: NetworkMethod::Post,
            url: url.to_string(),
            headers: vec![("authorization".to_string(), "Bearer sk-test".to_string())],
            body: b"{\"ping\":true}".to_vec(),
            resolved_ips,
            response_body_limit: None,
            timeout_ms: None,
        }
    }

    #[derive(Default)]
    struct RecordingTransport {
        seen: Mutex<Vec<NetworkTransportRequest>>,
    }

    #[async_trait]
    impl NetworkHttpTransport for &RecordingTransport {
        async fn execute(
            &self,
            request: NetworkTransportRequest,
        ) -> Result<NetworkHttpResponse, NetworkHttpError> {
            self.seen.lock().expect("seen lock").push(request);
            Ok(NetworkHttpResponse {
                status: 200,
                headers: Vec::new(),
                body: Vec::new(),
                usage: NetworkUsage {
                    request_bytes: 0,
                    response_bytes: 0,
                    resolved_ip: None,
                },
            })
        }
    }

    #[test]
    fn parse_accepts_multi_entry_loopback_maps() {
        let map = HostRewriteMap::parse(
            "api.example.test=127.0.0.1:8443, files.example.test=127.0.0.1:9000",
        )
        .expect("valid map");
        assert_eq!(
            map.target_for("API.EXAMPLE.TEST"),
            Some("127.0.0.1:8443".parse().expect("addr"))
        );
        assert_eq!(
            map.target_for("files.example.test"),
            Some("127.0.0.1:9000".parse().expect("addr"))
        );
        assert_eq!(map.target_for("other.example.test"), None);
    }

    #[test]
    fn parse_rejects_malformed_entries() {
        for raw in [
            "",
            "api.example.test",
            "api.example.test=",
            "=127.0.0.1:1",
            "api.example.test=127.0.0.1",
            "api.example.test=localhost:80",
            "api.example.test=127.0.0.1:1,,b.example.test=127.0.0.1:2",
            "a.example.test=127.0.0.1:1,a.example.test=127.0.0.1:2",
        ] {
            assert!(
                matches!(
                    HostRewriteMap::parse(raw),
                    Err(HostRewriteMapError::Malformed { .. })
                ),
                "`{raw}` must be rejected as malformed"
            );
        }
    }

    #[test]
    fn parse_rejects_non_loopback_targets() {
        for raw in [
            "api.example.test=192.0.2.10:8443",
            "api.example.test=0.0.0.0:8443",
            "api.example.test=10.0.0.1:8443",
        ] {
            assert!(
                matches!(
                    HostRewriteMap::parse(raw),
                    Err(HostRewriteMapError::NotLoopback { .. })
                ),
                "`{raw}` must be rejected as non-loopback"
            );
        }
    }

    #[tokio::test]
    async fn unset_env_value_is_a_pure_passthrough() {
        let inner = RecordingTransport::default();
        for value in [None, Some(""), Some("   ")] {
            let transport =
                RewriteNetworkTransport::from_env_value(&inner, value).expect("inert wrapper");
            assert!(!transport.is_active());
            transport
                .execute(request(
                    "https://api.example.test/v1/messages",
                    vec!["192.0.2.7".parse().expect("ip")],
                ))
                .await
                .expect("passthrough execute");
        }
        let seen = inner.seen.lock().expect("seen lock");
        assert_eq!(seen.len(), 3);
        for forwarded in seen.iter() {
            assert_eq!(forwarded.url, "https://api.example.test/v1/messages");
            assert_eq!(
                forwarded.resolved_ips,
                vec!["192.0.2.7".parse::<IpAddr>().expect("ip")]
            );
        }
    }

    #[tokio::test]
    async fn active_map_rewrites_mapped_hosts_and_passes_others_untouched() {
        let inner = RecordingTransport::default();
        let transport = RewriteNetworkTransport::from_env_value(
            &inner,
            Some("api.example.test=127.0.0.1:9099"),
        )
        .expect("active wrapper");
        assert!(transport.is_active());

        transport
            .execute(request(
                "https://api.example.test/v1/send?limit=2",
                vec!["192.0.2.7".parse().expect("ip")],
            ))
            .await
            .expect("mapped execute");
        transport
            .execute(request(
                "https://unmapped.example.test/v1/send",
                vec!["192.0.2.8".parse().expect("ip")],
            ))
            .await
            .expect("unmapped execute");

        let seen = inner.seen.lock().expect("seen lock");
        assert_eq!(seen[0].url, "http://127.0.0.1:9099/v1/send?limit=2");
        assert!(
            seen[0].resolved_ips.is_empty(),
            "vendor IP pins must be dropped on rewrite"
        );
        assert_eq!(
            seen[0].headers[0],
            ("authorization".to_string(), "Bearer sk-test".to_string()),
            "headers must be forwarded untouched"
        );
        assert_eq!(seen[1].url, "https://unmapped.example.test/v1/send");
        assert_eq!(
            seen[1].resolved_ips,
            vec!["192.0.2.8".parse::<IpAddr>().expect("ip")]
        );
    }

    /// Wiring proof over the REAL reqwest transport: a mapped vendor URL
    /// (with a deliberately unroutable vendor IP pin) lands as plain HTTP on
    /// a local server. Without the rewrite this request could only try TLS
    /// against the TEST-NET pin and fail.
    #[tokio::test]
    async fn rewrite_over_real_transport_lands_on_the_loopback_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local server");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buffer = vec![0_u8; 4096];
            let read = stream.read(&mut buffer).await.expect("read request head");
            let head = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = "{\"received\":true}";
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\
                 content-type: application/json\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            stream.shutdown().await.ok();
            head
        });

        let transport = RewriteNetworkTransport::from_env_value(
            ReqwestNetworkTransport::default(),
            Some(&format!("api.example.test={addr}")),
        )
        .expect("active wrapper");
        let response = transport
            .execute(request(
                "https://api.example.test/v1/send",
                vec!["192.0.2.1".parse().expect("ip")],
            ))
            .await
            .expect("rewritten request reaches the local server");
        assert_eq!(response.status, 200);

        let head = server.await.expect("server task");
        assert!(
            head.starts_with("POST /v1/send HTTP/1.1"),
            "request line must carry the vendor path: {head}"
        );
        assert!(
            head.to_ascii_lowercase()
                .contains("authorization: bearer sk-test"),
            "headers must be forwarded to the rewritten target: {head}"
        );
    }

    #[test]
    fn default_policy_http_egress_builds_without_the_env_var() {
        // The seam constructor must be inert (and infallible) when the env
        // var is unset — the production path.
        if std::env::var(TEST_HTTP_REWRITE_MAP_ENV).is_err() {
            default_policy_http_egress().expect("inert egress builds");
        }
    }
}
