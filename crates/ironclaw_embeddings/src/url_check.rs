//! Baseline base-URL validation for the embedding-provider factory.
//!
//! This is a **defense-in-depth** check, not the full SSRF policy. The binary
//! continues to apply a richer, operator-tunable policy (`validate_operator_base_url`
//! in `src/config/helpers.rs`) at config-resolve time; this module covers the
//! case where a downstream caller constructs `EmbeddingsConfig` directly and
//! reaches `create_provider` without going through that path.
//!
//! What this enforces:
//! - URL parses
//! - Scheme is `http` or `https`
//! - Host (when it is a literal IP) is not in the `AlwaysBlocked` class:
//!   cloud-metadata (`169.254.169.254`), link-local, multicast, the
//!   unspecified `0.0.0.0`/`::`. These are *never* legitimate operator
//!   endpoints, regardless of policy.
//!
//! What this does NOT do:
//! - DNS-resolve hostnames (the binary's policy does that; doing it here
//!   would couple the crate to a runtime and to a DNS-availability heuristic).
//! - Reject private/loopback IPs — those are legitimate for self-hosted
//!   Ollama and similar setups; the operator-tunable policy in the binary
//!   makes that call.

use std::net::{IpAddr, Ipv4Addr};

use crate::provider::EmbeddingError;

/// Validate a base URL configured for an embedding provider.
///
/// Returns `Err(EmbeddingError::InvalidUrl { .. })` on parse failure,
/// non-http(s) scheme, missing host, or an `AlwaysBlocked` literal IP host.
pub(crate) fn check_base_url(url: &str, field_name: &str) -> Result<(), EmbeddingError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| EmbeddingError::InvalidUrl {
        url: url.to_string(),
        reason: format!("{field_name}: {e}"),
    })?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(EmbeddingError::InvalidUrl {
            url: url.to_string(),
            reason: format!("{field_name}: only http/https are allowed (got '{scheme}')"),
        });
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| EmbeddingError::InvalidUrl {
            url: url.to_string(),
            reason: format!("{field_name}: missing host"),
        })?;

    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = normalized_host.parse::<IpAddr>()
        && is_always_blocked(&ip)
    {
        return Err(EmbeddingError::InvalidUrl {
            url: url.to_string(),
            reason: format!("{field_name}: host '{host}' is not a permitted endpoint"),
        });
    }

    Ok(())
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
            if let Some(v4) = v6.to_ipv4_mapped() {
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
    fn accepts_normal_endpoints() {
        check_base_url("https://api.openai.com", "F").unwrap();
        check_base_url("http://localhost:11434", "F").unwrap();
        check_base_url("https://api.near.ai", "F").unwrap();
        check_base_url("http://192.168.1.50:8000", "F").unwrap(); // private — allowed at this layer
        check_base_url("http://127.0.0.1:11434", "F").unwrap();
    }

    #[test]
    fn rejects_aws_metadata_ip() {
        let err = check_base_url("https://169.254.169.254", "OLLAMA_BASE_URL")
            .expect_err("metadata IP must be rejected");
        assert!(matches!(err, EmbeddingError::InvalidUrl { .. }));
        let msg = err.to_string();
        assert!(
            msg.contains("OLLAMA_BASE_URL"),
            "field name in message: {msg}"
        );
        assert!(msg.contains("169.254.169.254"), "host in message: {msg}");
    }

    #[test]
    fn rejects_link_local_ipv6() {
        check_base_url("https://[fe80::1]", "F").expect_err("link-local IPv6 rejected");
    }

    #[test]
    fn rejects_multicast() {
        check_base_url("http://224.0.0.1", "F").expect_err("multicast rejected");
    }

    #[test]
    fn rejects_unspecified() {
        check_base_url("http://0.0.0.0", "F").expect_err("0.0.0.0 rejected");
    }

    #[test]
    fn rejects_non_http_scheme() {
        let err = check_base_url("file:///etc/passwd", "F").expect_err("file:// rejected");
        assert!(err.to_string().contains("http/https"));
    }

    #[test]
    fn rejects_unparseable() {
        check_base_url("not a url", "F").expect_err("garbage rejected");
    }
}
