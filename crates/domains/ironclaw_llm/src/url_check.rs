//! Base-URL SSRF validation for provider model-discovery requests.
//!
//! This is the operator base-URL policy applied at the model-listing **egress
//! point** in [`crate::rig_adapter`] — which both the Reborn provider
//! probe/list/test path and the v1 `/v1/models` proxy reach through
//! `LlmProvider::list_models`. It mirrors the binary's `validate_operator_base_url`
//! (`src/config/helpers.rs`, `AllowPrivateNetwork` posture) so the same policy
//! runs even though this crate cannot depend on the binary.
//!
//! What this enforces:
//! - URL parses; scheme is `http` or `https`.
//! - The host — a literal IP **or a hostname resolved via DNS** — is not in the
//!   `AlwaysBlocked` class: cloud-metadata (`169.254.169.254`), link-local,
//!   multicast, the unspecified `0.0.0.0`/`::`. This is what stops a hostname
//!   that *resolves* to a blocked address from reaching it.
//! - Non-TLS `http` is allowed only for localhost or private/loopback endpoints
//!   (so self-hosted Ollama keeps working) and rejected for public hosts.
//!
//! Private/loopback IPs are intentionally allowed (self-hosted Ollama, vLLM).
//! When DNS is globally unavailable (egress-proxy / offline environments that
//! resolve on the caller's behalf), IP validation is skipped — the syntactic
//! checks still apply and the proxy resolves at request time.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::error::LlmError;

/// Outcome of [`check_models_url`]: the URL passed the SSRF policy, plus the
/// pin the caller must apply to the outbound request.
#[derive(Debug)]
pub(crate) struct ValidatedModelsUrl {
    /// When `Some`, the host was a DNS name we resolved and validated; the
    /// caller MUST pin the HTTP client to exactly these `(host, addrs)` so the
    /// connect-time resolver cannot rebind the name to a blocked IP after the
    /// guard cleared a safe one (DNS time-of-check/time-of-use). `None` when
    /// the host is a literal IP (no name to rebind) or DNS was globally
    /// unavailable (a trusted egress proxy resolves on our behalf).
    pub pin: Option<(String, Vec<SocketAddr>)>,
}

/// Validate a base/endpoint URL before issuing an outbound model-discovery
/// request. This is the operator base-URL SSRF policy applied at the egress
/// point (mirrors the binary's `validate_operator_base_url` with the
/// `AllowPrivateNetwork` posture), so the Reborn provider probe/list/test path
/// and the v1 `/v1/models` proxy — which both reach `LlmProvider::list_models`
/// — are covered in one place.
///
/// Enforced: parses; scheme is http/https; the host, whether a literal IP **or
/// a hostname resolved via DNS**, is not in the `AlwaysBlocked` class
/// (cloud-metadata, link-local, multicast, unspecified); and non-TLS `http` is
/// allowed only for localhost or private/loopback endpoints (so self-hosted
/// Ollama keeps working) but rejected for public hosts.
///
/// When DNS resolution is entirely unavailable (sandboxed CI / egress-proxy
/// environments that resolve on the caller's behalf), IP validation is skipped
/// — the syntactic checks still apply and the proxy resolves at request time —
/// so model discovery does not break where names cannot be resolved locally.
///
/// On success returns a [`ValidatedModelsUrl`] whose `pin` the caller must
/// apply to the HTTP client: validation resolves the hostname once, but
/// `reqwest` would resolve it again at connect time, so a DNS-rebinding
/// endpoint could return a safe IP here and a blocked one (e.g.
/// `169.254.169.254`) when the request actually fires. Pinning the client to
/// the exact validated addresses closes that time-of-check/time-of-use gap.
pub(crate) async fn check_models_url(
    provider_id: &str,
    url: &str,
) -> Result<ValidatedModelsUrl, LlmError> {
    let reject = |reason: String| LlmError::RequestFailed {
        provider: provider_id.to_string(),
        reason,
    };

    let parsed =
        reqwest::Url::parse(url).map_err(|e| reject(format!("invalid models URL: {e}")))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(reject(format!(
            "only http/https are allowed for model discovery (got '{scheme}')"
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| reject("models URL is missing a host".to_string()))?;
    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    let port = parsed
        .port()
        .unwrap_or(if scheme == "http" { 80 } else { 443 });

    if let Ok(ip) = normalized_host.parse::<IpAddr>() {
        enforce_resolved_policy(provider_id, scheme, host, normalized_host, &[ip])?;
        // Literal IP: `reqwest` connects to it directly with no DNS lookup, so
        // there is nothing to rebind and nothing to pin.
        return Ok(ValidatedModelsUrl { pin: None });
    }

    match resolve_host_ips(normalized_host, port).await {
        HostResolution::Resolved(addrs) => {
            let ips: Vec<IpAddr> = addrs.iter().map(SocketAddr::ip).collect();
            enforce_resolved_policy(provider_id, scheme, host, normalized_host, &ips)?;
            // Pin the client to the addresses we just validated so the
            // connect-time resolver can't rebind to a blocked IP.
            Ok(ValidatedModelsUrl {
                pin: Some((normalized_host.to_string(), addrs)),
            })
        }
        HostResolution::Unresolvable => {
            Err(reject(format!("could not resolve models host '{host}'")))
        }
        HostResolution::DnsUnavailable => {
            tracing::debug!(
                host = %host,
                provider = %provider_id,
                "DNS resolution unavailable; skipping SSRF IP validation for model-discovery URL"
            );
            // No local resolution available: the trusted egress proxy resolves
            // at request time, so there is no address to pin.
            Ok(ValidatedModelsUrl { pin: None })
        }
    }
}

/// Apply the IP-class / scheme policy to a host's literal or DNS-resolved IPs.
/// Split out from [`check_models_url`] (no DNS, no IO) so the security decision
/// is unit-testable against synthetic resolved-IP sets.
fn enforce_resolved_policy(
    provider_id: &str,
    scheme: &str,
    host_display: &str,
    normalized_host: &str,
    ips: &[IpAddr],
) -> Result<(), LlmError> {
    let reject = |reason: String| LlmError::RequestFailed {
        provider: provider_id.to_string(),
        reason,
    };

    if ips.iter().any(is_always_blocked) {
        return Err(reject(format!(
            "host '{host_display}' resolves to a blocked address and is not a permitted model-discovery endpoint"
        )));
    }

    // Non-TLS http only for localhost or private/internal endpoints; public
    // hosts must use https. Mirrors the binary's AllowPrivateNetwork posture so
    // self-hosted Ollama over http keeps working.
    if scheme == "http" && !host_is_localhost_name(normalized_host) {
        let all_private = !ips.is_empty()
            && ips
                .iter()
                .all(|ip| matches!(classify_ip(ip), IpClass::PrivateOrLoopback));
        if !all_private {
            return Err(reject(format!(
                "HTTP (non-TLS) model discovery is only allowed for localhost or private endpoints, got '{host_display}'; use HTTPS for public endpoints"
            )));
        }
    }

    Ok(())
}

fn host_is_localhost_name(normalized_host: &str) -> bool {
    let host = normalized_host.to_ascii_lowercase();
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".localhost")
}

/// Whether a model-discovery URL targets a loopback / `localhost` host.
///
/// Used to bypass any HTTP proxy for local providers (e.g. self-hosted Ollama).
/// A system- or env-configured proxy cannot reach the caller's own loopback
/// service and answers the forwarded request with `502 Bad Gateway`, so a
/// loopback request must go direct. Remote hosts keep default proxy behavior so
/// corporate proxies still cover hosted providers. Unparseable input returns
/// `false` (no special-casing) — `check_models_url` rejects it separately.
pub(crate) fn is_loopback_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = normalized_host.parse::<IpAddr>() {
        return ip.is_loopback();
    }
    normalized_host.eq_ignore_ascii_case("localhost")
        || normalized_host.to_ascii_lowercase().ends_with(".localhost")
}

/// Apply the shared loopback-proxy-bypass policy to a `reqwest::ClientBuilder`
/// and build the client.
///
/// A system/env HTTP proxy cannot reach the caller's own loopback service and
/// answers the forwarded request with `502 Bad Gateway`, so a self-hosted local
/// provider (Ollama, vLLM, …) must go direct — see [`is_loopback_url`]. Remote
/// hosts keep default proxy behavior so corporate proxies still cover hosted
/// providers. Callers pass a pre-configured builder so each can set its own
/// timeout / redirect policy (the model-discovery client disables redirects as
/// an SSRF guard; the chat client must keep them) without duplicating the
/// proxy-bypass-and-build boilerplate.
pub(crate) fn build_http_client(
    provider_id: &str,
    url: &str,
    builder: reqwest::ClientBuilder,
) -> Result<reqwest::Client, LlmError> {
    let builder = if is_loopback_url(url) {
        builder.no_proxy()
    } else {
        builder
    };
    builder.build().map_err(|e| LlmError::RequestFailed {
        provider: provider_id.to_string(),
        reason: format!("failed to build HTTP client: {e}"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IpClass {
    /// Never a legitimate provider endpoint: cloud-metadata, link-local,
    /// multicast, unspecified.
    AlwaysBlocked,
    /// Private or loopback — legitimate for self-hosted providers over http.
    PrivateOrLoopback,
    /// Public, routable address.
    Public,
}

fn classify_ip(ip: &IpAddr) -> IpClass {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_link_local()
                || *v4 == Ipv4Addr::new(169, 254, 169, 254)
            {
                IpClass::AlwaysBlocked
            } else if v4.is_private()
                || v4.is_loopback()
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
            {
                IpClass::PrivateOrLoopback
            } else {
                IpClass::Public
            }
        }
        IpAddr::V6(v6) => {
            // Check v6-native classes (incl. loopback `::1` and ULA) before
            // unwrapping embedded IPv4, so loopback stays loopback rather than
            // mapping to `0.0.0.1`.
            if v6.is_unspecified()
                || v6.octets()[0] == 0xff
                || (v6.segments()[0] & 0xffc0) == 0xfe80
            {
                IpClass::AlwaysBlocked
            } else if v6.is_loopback() || (v6.octets()[0] & 0xfe) == 0xfc {
                IpClass::PrivateOrLoopback
            } else if let Some(v4) = v6.to_ipv4() {
                // `to_ipv4()` (not `to_ipv4_mapped()`) covers both embedded
                // forms — IPv4-mapped (`::ffff:a.b.c.d`) and IPv4-compatible
                // (`::a.b.c.d`). The latter would otherwise classify as a plain
                // (Public) v6 address, letting `::169.254.169.254` reach the
                // metadata endpoint.
                classify_ip(&IpAddr::V4(v4))
            } else {
                IpClass::Public
            }
        }
    }
}

fn is_always_blocked(ip: &IpAddr) -> bool {
    matches!(classify_ip(ip), IpClass::AlwaysBlocked)
}

/// Result of resolving a hostname to socket addresses for SSRF validation.
/// Socket addresses (not bare IPs) are carried so the caller can pin the HTTP
/// client to exactly these endpoints and avoid a second, rebindable lookup.
enum HostResolution {
    Resolved(Vec<SocketAddr>),
    /// The name did not resolve, but DNS itself is working — a genuine
    /// "unknown host".
    Unresolvable,
    /// DNS resolution is globally unavailable (proxy/offline env); the caller
    /// should skip IP validation rather than reject.
    DnsUnavailable,
}

async fn resolve_host_ips(host: &str, port: u16) -> HostResolution {
    match tokio::net::lookup_host((host, port)).await {
        Ok(addrs) => {
            let addrs: Vec<SocketAddr> = addrs.collect();
            if addrs.is_empty() {
                HostResolution::Unresolvable
            } else {
                HostResolution::Resolved(addrs)
            }
        }
        // The target itself didn't resolve — distinguish "DNS is down" (skip
        // validation) from "this hostname is invalid" (reject) via a generic
        // probe of a well-known name.
        Err(_) if dns_probe_available().await => HostResolution::Unresolvable,
        Err(_) => HostResolution::DnsUnavailable,
    }
}

/// Time-to-live for the cached DNS-availability probe. Re-probing every 5
/// minutes ensures a transient DNS outage doesn't permanently disable SSRF
/// validation for the process.
const DNS_PROBE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Whether external DNS resolution is functional, cached for [`DNS_PROBE_TTL`].
async fn dns_probe_available() -> bool {
    use std::sync::Mutex;
    static PROBE: Mutex<Option<(bool, std::time::Instant)>> = Mutex::new(None);

    {
        let guard = PROBE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((available, expires_at)) = *guard
            && std::time::Instant::now() < expires_at
        {
            return available;
        }
    }

    let available = tokio::net::lookup_host(("one.one.one.one", 443))
        .await
        .is_ok();

    let mut guard = PROBE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some((available, std::time::Instant::now() + DNS_PROBE_TTL));
    available
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn accepts_normal_and_local_endpoints() {
        // Literal public over https, plus local/private endpoints (localhost
        // resolves through the system resolver, no network).
        check_models_url("p", "https://93.184.216.34/v1/models")
            .await
            .unwrap();
        check_models_url("p", "http://localhost:11434/api/tags")
            .await
            .unwrap();
        check_models_url("p", "http://127.0.0.1:11434/api/tags")
            .await
            .unwrap();
        check_models_url("p", "http://192.168.1.50:8000/models")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn rejects_metadata_link_local_multicast_unspecified() {
        check_models_url("p", "https://169.254.169.254/models")
            .await
            .expect_err("metadata IP");
        check_models_url("p", "https://[fe80::1]/models")
            .await
            .expect_err("link-local v6");
        check_models_url("p", "http://224.0.0.1/models")
            .await
            .expect_err("multicast");
        check_models_url("p", "http://0.0.0.0/models")
            .await
            .expect_err("unspecified");
    }

    #[tokio::test]
    async fn rejects_public_http_endpoint() {
        // Public host over non-TLS http is rejected even though the IP is not
        // in the always-blocked class; use https for public endpoints.
        check_models_url("p", "http://8.8.8.8/models")
            .await
            .expect_err("public http");
    }

    #[tokio::test]
    async fn rejects_embedded_ipv4_metadata_in_both_v6_forms() {
        // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d) both
        // embed the metadata address; neither may bypass the V4 block rules.
        check_models_url("p", "https://[::ffff:169.254.169.254]/models")
            .await
            .expect_err("ipv4-mapped metadata");
        check_models_url("p", "https://[::169.254.169.254]/models")
            .await
            .expect_err("ipv4-compatible metadata");
    }

    #[tokio::test]
    async fn literal_ip_url_carries_no_pin() {
        // A literal IP is connected to directly with no DNS lookup, so there's
        // nothing to rebind and the result must not pin.
        let validated = check_models_url("p", "https://93.184.216.34/v1/models")
            .await
            .unwrap();
        assert!(validated.pin.is_none());
    }

    #[tokio::test]
    async fn resolved_hostname_pins_validated_addresses() {
        // `localhost` resolves through the system resolver to loopback
        // addresses; the guard must hand back a pin so the outbound request
        // can't rebind the name to a different IP at connect time (DNS TOCTOU).
        let validated = check_models_url("p", "http://localhost:11434/api/tags")
            .await
            .unwrap();
        let (host, addrs) = validated.pin.expect("a resolved hostname carries a pin");
        assert_eq!(host, "localhost");
        assert!(!addrs.is_empty());
        assert!(addrs.iter().all(|addr| addr.ip().is_loopback()));
    }

    #[tokio::test]
    async fn allows_ipv6_loopback() {
        // ::1 classifies as loopback (private), so self-hosted providers on the
        // IPv6 loopback stay reachable over http.
        check_models_url("p", "http://[::1]:11434/api/tags")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn rejects_non_http_and_unparseable() {
        check_models_url("p", "file:///etc/passwd")
            .await
            .expect_err("file scheme");
        check_models_url("p", "not a url")
            .await
            .expect_err("garbage");
    }

    // The crux of the SSRF fix: a *hostname* (not a literal IP) whose resolved
    // address is blocked must be rejected. enforce_resolved_policy is the pure
    // decision the resolving check feeds, so this is hermetic — no real DNS.
    #[test]
    fn enforce_policy_blocks_hostname_resolving_to_metadata() {
        let metadata: IpAddr = "169.254.169.254".parse().unwrap();
        enforce_resolved_policy("p", "https", "evil.example", "evil.example", &[metadata])
            .expect_err("hostname resolving to the metadata IP is blocked even over https");
    }

    #[test]
    fn enforce_policy_blocks_public_http_but_allows_private_and_https() {
        let public: IpAddr = "8.8.8.8".parse().unwrap();
        let private: IpAddr = "192.168.1.50".parse().unwrap();
        // Public host over http -> rejected.
        enforce_resolved_policy("p", "http", "cdn.example", "cdn.example", &[public])
            .expect_err("public http rejected");
        // Public host over https -> allowed.
        enforce_resolved_policy("p", "https", "api.example", "api.example", &[public]).unwrap();
        // Private host over http (self-hosted Ollama) -> allowed.
        enforce_resolved_policy("p", "http", "ollama.lan", "ollama.lan", &[private]).unwrap();
    }

    #[test]
    fn classify_ip_handles_v6_loopback_and_embedded_metadata() {
        assert_eq!(
            classify_ip(&"::1".parse().unwrap()),
            IpClass::PrivateOrLoopback
        );
        assert_eq!(
            classify_ip(&"::169.254.169.254".parse().unwrap()),
            IpClass::AlwaysBlocked
        );
        assert_eq!(
            classify_ip(&"::ffff:127.0.0.1".parse().unwrap()),
            IpClass::PrivateOrLoopback
        );
        assert_eq!(classify_ip(&"8.8.8.8".parse().unwrap()), IpClass::Public);
    }

    #[test]
    fn loopback_detection_matches_localhost_and_loopback_ips() {
        assert!(is_loopback_url("http://localhost:11434/api/tags"));
        assert!(is_loopback_url("http://LOCALHOST:11434"));
        assert!(is_loopback_url("http://api.localhost:8080/v1"));
        assert!(is_loopback_url("http://127.0.0.1:11434/api/tags"));
        assert!(is_loopback_url("http://127.5.6.7:8000"));
        assert!(is_loopback_url("http://[::1]:11434/api/tags"));
    }

    #[test]
    fn loopback_detection_excludes_remote_and_private_lan_hosts() {
        assert!(!is_loopback_url("https://api.openai.com/v1/models"));
        assert!(!is_loopback_url("https://cloud-api.near.ai"));
        // Private LAN is intentionally NOT treated as loopback: a corporate
        // proxy may legitimately route to it, so keep default proxy behavior.
        assert!(!is_loopback_url("http://192.168.1.50:8000/models"));
        assert!(!is_loopback_url("not a url"));
    }
}
