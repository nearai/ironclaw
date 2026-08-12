//! Baseline base-URL validation for the real mem0 transport.
//!
//! This mirrors the embedding-provider factory's `check_base_url`
//! (`ironclaw_embeddings::url_check`) so the mem0 provider applies the same
//! defense-in-depth SSRF gate the other config-driven providers do. It is a
//! baseline check, not the full operator SSRF policy.
//!
//! What this enforces:
//! - URL parses
//! - Scheme is `http` or `https`
//! - No embedded userinfo (credentials belong in the redacted API key)
//! - Host is not the hosted mem0 cloud (`mem0.ai` / `*.mem0.ai`): this adapter is
//!   self-hosted-OSS only, so a cloud host fails closed.
//! - Host (when it is a literal IP) is not in the `AlwaysBlocked` class:
//!   cloud-metadata (`169.254.169.254`), link-local, multicast, the
//!   unspecified `0.0.0.0`/`::`. These are *never* legitimate operator
//!   endpoints, regardless of policy.
//!
//! What this does NOT do:
//! - DNS-resolve hostnames.
//! - Reject private/loopback IPs — those are legitimate for self-hosted mem0.

use std::net::{IpAddr, Ipv4Addr};

use crate::error::Mem0Error;

/// Validate the configured mem0 base URL.
///
/// Returns `Err(Mem0Error::InvalidUrl { .. })` on parse failure, non-http(s)
/// scheme, embedded credentials, missing host, the hosted mem0 cloud host, or an
/// `AlwaysBlocked` literal IP host.
pub(crate) fn check_base_url(url: &str) -> Result<(), Mem0Error> {
    // None of these rejections carry the raw URL: only a redacted `reason` survives
    // into the error (see `Mem0Error::InvalidUrl`), so a misconfigured host or a
    // query-string token cannot leak into host logs.
    let parsed = reqwest::Url::parse(url).map_err(|error| Mem0Error::InvalidUrl {
        reason: error.to_string(),
    })?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(Mem0Error::InvalidUrl {
            reason: format!("only http/https are allowed (got '{scheme}')"),
        });
    }

    // Reject a base URL that carries embedded credentials (`https://user:pass@host`).
    // Credentials belong in `MEMORY_MEM0_API_KEY` (a redacted secret), never in the
    // operator base URL where they would leak into logs and error messages. The
    // error carries no URL at all, so the password cannot be echoed back.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Mem0Error::InvalidUrl {
            reason: "must not embed credentials in the base URL (userinfo is not allowed)"
                .to_string(),
        });
    }

    let host = parsed.host_str().ok_or_else(|| Mem0Error::InvalidUrl {
        reason: "missing host".to_string(),
    })?;

    // Fail closed on the hosted mem0 cloud. This adapter targets self-hosted mem0
    // OSS only (see crate docs); config separately documents that the base URL is
    // never the cloud, but enforce it here too so a misconfigured cloud URL is
    // rejected at construction rather than silently talking to the cloud.
    if is_mem0_cloud_host(host) {
        return Err(Mem0Error::InvalidUrl {
            reason: format!(
                "host '{host}' is the hosted mem0 cloud; this adapter supports self-hosted mem0 OSS only"
            ),
        });
    }

    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = normalized_host.parse::<IpAddr>()
        && is_always_blocked(&ip)
    {
        return Err(Mem0Error::InvalidUrl {
            reason: format!("host '{host}' is not a permitted endpoint"),
        });
    }

    Ok(())
}

/// Hosts belonging to the hosted mem0 cloud. The adapter is self-hosted-OSS only,
/// so the cloud apex (`mem0.ai`) and any subdomain (`*.mem0.ai`) are rejected. DNS
/// is case-insensitive and may carry a trailing FQDN dot, so normalize both before
/// comparing.
fn is_mem0_cloud_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "mem0.ai" || host.ends_with(".mem0.ai")
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
        check_base_url("https://mem0.example.com").unwrap();
        check_base_url("http://localhost:8080").unwrap();
        check_base_url("http://192.168.1.50:8000").unwrap(); // private — allowed at this layer
        check_base_url("http://127.0.0.1:8888").unwrap();
    }

    #[test]
    fn rejects_embedded_credentials() {
        // A base URL carrying userinfo must be rejected: credentials belong in
        // the (redacted) API key, not the URL.
        let err = check_base_url("https://operator:s3cr3t-token@mem0.example.com")
            .expect_err("a URL with embedded credentials is rejected");
        assert!(matches!(err, Mem0Error::InvalidUrl { .. }));
        // The rejection must not echo the embedded password back into the error.
        assert!(
            !err.to_string().contains("s3cr3t-token"),
            "embedded password must be redacted from the rejection error"
        );
        // A username-only URL (`https://user@host`) is rejected too.
        check_base_url("https://operator@mem0.example.com")
            .expect_err("a URL with embedded userinfo is rejected");
    }

    #[test]
    fn rejects_aws_metadata_ip() {
        let err = check_base_url("https://169.254.169.254").expect_err("metadata IP rejected");
        assert!(matches!(err, Mem0Error::InvalidUrl { .. }));
        assert!(err.to_string().contains("169.254.169.254"));
    }

    #[test]
    fn rejects_link_local_ipv6() {
        check_base_url("https://[fe80::1]").expect_err("link-local IPv6 rejected");
    }

    #[test]
    fn rejects_multicast() {
        check_base_url("http://224.0.0.1").expect_err("multicast rejected");
    }

    #[test]
    fn rejects_unspecified() {
        check_base_url("http://0.0.0.0").expect_err("unspecified rejected");
    }

    #[test]
    fn rejects_non_http_scheme() {
        check_base_url("file:///etc/passwd").expect_err("file scheme rejected");
    }

    #[test]
    fn rejects_hosted_mem0_cloud() {
        // The adapter is self-hosted-OSS only, so a hosted mem0 cloud host fails
        // closed — both the apex and a subdomain, case-insensitively.
        let err =
            check_base_url("https://api.mem0.ai").expect_err("hosted mem0 cloud apex rejected");
        assert!(matches!(err, Mem0Error::InvalidUrl { .. }));
        check_base_url("https://app.mem0.ai/v1").expect_err("hosted mem0 cloud subdomain rejected");
        check_base_url("https://API.MEM0.AI")
            .expect_err("hosted mem0 cloud host is matched case-insensitively");
        // A self-hosted host that merely *contains* "mem0" is NOT the cloud and
        // stays allowed (no over-blocking of legitimate self-hosted endpoints).
        check_base_url("https://mem0.internal.example.com").unwrap();
    }

    #[test]
    fn rejection_error_does_not_echo_the_configured_url() {
        // A misconfigured base URL can carry a sensitive host or a query-string
        // token. The rejection error must not echo the raw URL (host/path/query)
        // back into a log line — only the cause/kind survives. A non-http scheme is
        // rejected before host parsing, so this exercises that general path.
        let err = check_base_url("ftp://secret-host.internal/path?token=swordfish")
            .expect_err("non-http scheme rejected");
        let rendered = err.to_string();
        assert!(
            !rendered.contains("secret-host.internal"),
            "the configured host must not leak into the error: {rendered}"
        );
        assert!(
            !rendered.contains("swordfish"),
            "a query-string token must not leak into the error: {rendered}"
        );
    }
}
