//! Baseline base-URL validation for provider model-discovery requests.
//!
//! This is a **defense-in-depth** SSRF check, not the full operator policy.
//! The binary applies a richer, operator-tunable policy
//! (`validate_operator_base_url` in `src/config/helpers.rs`) at config-resolve
//! time; this module covers the model-listing egress point in
//! [`crate::rig_adapter`], which both the Reborn provider probe and the v1
//! `/v1/models` proxy reach through `LlmProvider::list_models`.
//!
//! What this enforces:
//! - URL parses
//! - Scheme is `http` or `https`
//! - Host (when it is a literal IP) is not in the `AlwaysBlocked` class:
//!   cloud-metadata (`169.254.169.254`), link-local, multicast, the
//!   unspecified `0.0.0.0`/`::`. These are *never* legitimate provider
//!   endpoints, regardless of policy.
//!
//! What this does NOT do:
//! - DNS-resolve hostnames (the binary's policy does that; doing it here would
//!   couple the crate to a runtime and a DNS-availability heuristic).
//! - Reject private/loopback IPs — those are legitimate for self-hosted Ollama
//!   and similar setups; the operator-tunable policy in the binary makes that
//!   call.

use std::net::{IpAddr, Ipv4Addr};

use crate::error::LlmError;

/// Validate a base/endpoint URL before issuing an outbound model-discovery
/// request. Returns `LlmError::RequestFailed` on parse failure, non-http(s)
/// scheme, missing host, or an `AlwaysBlocked` literal IP host.
pub(crate) fn check_models_url(provider_id: &str, url: &str) -> Result<(), LlmError> {
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
    if let Ok(ip) = normalized_host.parse::<IpAddr>()
        && is_always_blocked(&ip)
    {
        return Err(reject(format!(
            "host '{host}' is not a permitted model-discovery endpoint"
        )));
    }

    Ok(())
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

fn is_always_blocked(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_link_local()
                || *v4 == Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(v6) => {
            // `to_ipv4()` (not `to_ipv4_mapped()`) so both embedded-IPv4 forms
            // are unwrapped and checked against the V4 rules: IPv4-mapped
            // (`::ffff:a.b.c.d`) and IPv4-compatible (`::a.b.c.d`). The latter
            // would otherwise sail past as a plain v6 address — e.g.
            // `::169.254.169.254` reaching the metadata endpoint.
            if let Some(v4) = v6.to_ipv4() {
                return is_always_blocked(&IpAddr::V4(v4));
            }
            v6.is_unspecified() || v6.octets()[0] == 0xff || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_and_local_endpoints() {
        check_models_url("p", "https://api.openai.com/v1/models").unwrap();
        check_models_url("p", "http://localhost:11434/api/tags").unwrap();
        check_models_url("p", "http://127.0.0.1:11434/api/tags").unwrap();
        check_models_url("p", "http://192.168.1.50:8000/models").unwrap();
    }

    #[test]
    fn rejects_metadata_link_local_multicast_unspecified() {
        check_models_url("p", "https://169.254.169.254/models").expect_err("metadata IP");
        check_models_url("p", "https://[fe80::1]/models").expect_err("link-local v6");
        check_models_url("p", "http://224.0.0.1/models").expect_err("multicast");
        check_models_url("p", "http://0.0.0.0/models").expect_err("unspecified");
    }

    #[test]
    fn rejects_embedded_ipv4_metadata_in_both_v6_forms() {
        // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d) both
        // embed the metadata address; neither may bypass the V4 block rules.
        check_models_url("p", "https://[::ffff:169.254.169.254]/models")
            .expect_err("ipv4-mapped metadata");
        check_models_url("p", "https://[::169.254.169.254]/models")
            .expect_err("ipv4-compatible metadata");
    }

    #[test]
    fn allows_ipv6_loopback() {
        // ::1 maps to 0.0.0.1 under to_ipv4(), which is not in the blocked
        // class — self-hosted providers on IPv6 loopback stay reachable.
        check_models_url("p", "http://[::1]:11434/api/tags").unwrap();
    }

    #[test]
    fn rejects_non_http_and_unparseable() {
        check_models_url("p", "file:///etc/passwd").expect_err("file scheme");
        check_models_url("p", "not a url").expect_err("garbage");
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
