//! Pure hosted-MCP admission values shared by registration and reconciliation.
//!
//! This is intentionally private host policy: the public request carries an
//! opaque endpoint and auth selection, while the durable manifest remains the
//! sole stored contract.

use ironclaw_host_api::ids::VendorId;

use ironclaw_extension_contracts::hosted_mcp::HostedMcpEndpoint;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalHostedMcpEndpoint(String);

impl CanonicalHostedMcpEndpoint {
    pub fn parse(input: &HostedMcpEndpoint) -> Result<Self, HostedMcpAdmissionError> {
        let url = url::Url::parse(input.as_str())
            .map_err(|_| HostedMcpAdmissionError::InvalidEndpoint)?;
        let host = url.host().ok_or(HostedMcpAdmissionError::InvalidEndpoint)?;
        // A literal loopback IP (127.0.0.0/8 or ::1) is a safe, non-rebindable
        // on-device target: it is the single case where `http` is admitted and
        // where an IP literal is allowed. Every other endpoint must be a public
        // `https` URL, exactly as before.
        let loopback_literal = is_loopback_ip_literal(&host);
        let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && loopback_literal);
        if !scheme_ok
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(HostedMcpAdmissionError::InvalidEndpoint);
        }
        let host_str = url
            .host_str()
            .ok_or(HostedMcpAdmissionError::InvalidEndpoint)?;
        // `localhost` (a DNS name a resolver could rebind) stays rejected; IP
        // literals stay rejected unless they are a literal loopback address.
        let is_ip_literal = matches!(host, url::Host::Ipv4(_) | url::Host::Ipv6(_));
        if host_str.eq_ignore_ascii_case("localhost") || (is_ip_literal && !loopback_literal) {
            return Err(HostedMcpAdmissionError::InvalidEndpoint);
        }
        // Denylist, not allowlist: query parameters are load-bearing identity
        // for legitimate endpoints (see the ordering-preservation comment
        // below and `canonical_endpoint_keeps_query_identity_and_normalizes_path`),
        // so a blanket query rejection is not viable. This list is
        // necessarily incomplete-by-construction; it blocks the well-known
        // credential-bearing names plus common vendor-specific signature
        // forms observed in hosted MCP / cloud-API query auth.
        const CREDENTIAL_QUERY_KEYS: &[&str] = &[
            "access_token",
            "token",
            "api_key",
            "apikey",
            "key",
            "secret",
            "client_secret",
            "authorization",
            "auth",
            "bearer",
            "password",
            "signature",
            "sig",
            "x-amz-signature",
            "x-amz-security-token",
            "x-amz-credential",
            "session_token",
            "id_token",
            "refresh_token",
            "sas",
        ];
        if url.query_pairs().any(|(key, _)| {
            CREDENTIAL_QUERY_KEYS
                .iter()
                .any(|blocked| key.eq_ignore_ascii_case(blocked))
        }) {
            return Err(HostedMcpAdmissionError::InvalidEndpoint);
        }
        // `Url` serializes its parsed form, including default port, IDNA and
        // dot-segment normalization. Query ordering is intentionally retained.
        Ok(Self(url.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A literal IPv4/IPv6 loopback address (`127.0.0.0/8` or `::1`). Hostnames
/// such as `localhost` are intentionally excluded: only a literal loopback IP
/// is exempted, so no DNS name can later rebind to a non-loopback address.
/// Shared by the admission gate above and the hosted-MCP egress planner in
/// [`crate::mcp`] so the two agree on exactly what "loopback" means.
pub(crate) fn is_loopback_ip_literal(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
        url::Host::Domain(_) => false,
    }
}

/// [`is_loopback_ip_literal`] for a stored `NetworkTargetPattern` host, which
/// is a bare string rather than a parsed URL host. IPv6 patterns may or may not
/// carry the URL bracket form, so both are accepted. A wildcard or DNS pattern
/// never parses as an IP and is therefore never loopback.
pub(crate) fn is_loopback_host_pattern(host_pattern: &str) -> bool {
    host_pattern
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedMcpAdmissionError {
    InvalidEndpoint,
    InvalidVendorId,
}

/// Deterministic provider identity; the endpoint itself never becomes a
/// provider id, log field, or durable package name.
pub fn hosted_mcp_vendor_id(
    endpoint: &CanonicalHostedMcpEndpoint,
) -> Result<VendorId, HostedMcpAdmissionError> {
    let digest = Sha256::digest(endpoint.as_str().as_bytes());
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    VendorId::new(format!("mcp-{suffix}")).map_err(|_| HostedMcpAdmissionError::InvalidVendorId)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_endpoint_rejects_credential_and_private_literal_forms() {
        for endpoint in [
            "https://user@example.test",
            "http://mcp.example.test",
            "https://mcp.example.test/rpc?Access_Token=must-not-persist",
            // `localhost` is a DNS name, not a literal loopback IP: rejected.
            "http://localhost/mcp",
            "https://localhost/mcp",
            // A non-loopback IP literal stays rejected.
            "https://[2001:db8::1]/mcp",
            "https://192.168.1.10/mcp",
            "https://mcp.example.test/rpc?client_secret=must-not-persist",
            "https://mcp.example.test/rpc?Password=must-not-persist",
            "https://mcp.example.test/rpc?signature=must-not-persist",
            "https://mcp.example.test/rpc?X-Amz-Signature=must-not-persist",
        ] {
            let input = HostedMcpEndpoint::new(endpoint).expect("wire endpoint");
            assert_eq!(
                CanonicalHostedMcpEndpoint::parse(&input),
                Err(HostedMcpAdmissionError::InvalidEndpoint)
            );
        }
    }

    #[test]
    fn canonical_endpoint_admits_literal_loopback_over_http_or_https() {
        // A literal loopback IP is a safe on-device target: `http` is admitted
        // and the IP literal is allowed, with the scheme/port preserved.
        for endpoint in [
            "http://127.0.0.1:5001/mcp",
            "https://127.0.0.1/mcp",
            "http://[::1]:5001/mcp",
            "https://[::1]/mcp",
        ] {
            let input = HostedMcpEndpoint::new(endpoint).expect("wire endpoint");
            CanonicalHostedMcpEndpoint::parse(&input)
                .unwrap_or_else(|_| panic!("loopback endpoint should be admitted: {endpoint}"));
        }

        let input = HostedMcpEndpoint::new("http://127.0.0.1:5001/mcp").expect("wire endpoint");
        let endpoint = CanonicalHostedMcpEndpoint::parse(&input).expect("canonical endpoint");
        assert_eq!(endpoint.as_str(), "http://127.0.0.1:5001/mcp");
    }

    #[test]
    fn canonical_endpoint_keeps_query_identity_and_normalizes_path() {
        let input = HostedMcpEndpoint::new("https://MCP.example.test/a/../rpc?b=2&a=1")
            .expect("wire endpoint");
        let endpoint = CanonicalHostedMcpEndpoint::parse(&input).expect("canonical endpoint");
        assert_eq!(endpoint.as_str(), "https://mcp.example.test/rpc?b=2&a=1");
        assert_eq!(
            hosted_mcp_vendor_id(&endpoint).expect("valid deterministic vendor"),
            hosted_mcp_vendor_id(&endpoint).expect("valid deterministic vendor")
        );
    }
}
