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
/// scheme, missing host, or an `AlwaysBlocked` literal IP host.
pub(crate) fn check_base_url(url: &str) -> Result<(), Mem0Error> {
    let parsed = reqwest::Url::parse(url).map_err(|error| Mem0Error::InvalidUrl {
        url: url.to_string(),
        reason: error.to_string(),
    })?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(Mem0Error::InvalidUrl {
            url: url.to_string(),
            reason: format!("only http/https are allowed (got '{scheme}')"),
        });
    }

    // Reject a base URL that carries embedded credentials (`https://user:pass@host`).
    // Credentials belong in `MEMORY_MEM0_API_KEY` (a redacted secret), never in the
    // operator base URL where they would leak into logs and error messages. Redact
    // the userinfo from the rejection error so the password is not echoed back.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Mem0Error::InvalidUrl {
            url: redact_userinfo(&parsed),
            reason: "must not embed credentials in the base URL (userinfo is not allowed)"
                .to_string(),
        });
    }

    let host = parsed.host_str().ok_or_else(|| Mem0Error::InvalidUrl {
        url: url.to_string(),
        reason: "missing host".to_string(),
    })?;

    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = normalized_host.parse::<IpAddr>()
        && is_always_blocked(&ip)
    {
        return Err(Mem0Error::InvalidUrl {
            url: url.to_string(),
            reason: format!("host '{host}' is not a permitted endpoint"),
        });
    }

    Ok(())
}

/// A copy of `url` with any embedded userinfo stripped, so a rejection error or
/// log never echoes the credentials it is rejecting.
fn redact_userinfo(url: &reqwest::Url) -> String {
    let mut redacted = url.clone();
    // `set_username`/`set_password` only reject cannot-be-a-base URLs, which an
    // http(s) URL never is; on the impossible failure fall back to scheme + host.
    if redacted.set_username("").is_ok() && redacted.set_password(None).is_ok() {
        redacted.to_string()
    } else {
        format!(
            "{}://{}",
            url.scheme(),
            url.host_str().unwrap_or("<redacted>")
        )
    }
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
}
