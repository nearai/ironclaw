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
    fn rejects_non_http_and_unparseable() {
        check_models_url("p", "file:///etc/passwd").expect_err("file scheme");
        check_models_url("p", "not a url").expect_err("garbage");
    }
}
