//! Pure hosted-MCP admission values shared by registration and reconciliation.
//!
//! This is intentionally private host policy: the public request carries an
//! opaque endpoint and auth selection, while the durable manifest remains the
//! sole stored contract.

use ironclaw_host_api::{HostedMcpEndpoint, VendorId};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalHostedMcpEndpoint(String);

impl CanonicalHostedMcpEndpoint {
    pub fn parse(input: &HostedMcpEndpoint) -> Result<Self, HostedMcpAdmissionError> {
        let url = url::Url::parse(input.as_str())
            .map_err(|_| HostedMcpAdmissionError::InvalidEndpoint)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(HostedMcpAdmissionError::InvalidEndpoint);
        }
        let host = url
            .host_str()
            .ok_or(HostedMcpAdmissionError::InvalidEndpoint)?;
        if host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok() {
            return Err(HostedMcpAdmissionError::InvalidEndpoint);
        }
        const CREDENTIAL_QUERY_KEYS: &[&str] = &[
            "access_token",
            "token",
            "api_key",
            "apikey",
            "key",
            "secret",
            "authorization",
            "auth",
            "bearer",
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
            "https://127.0.0.1/mcp",
            "http://mcp.example.test",
            "https://mcp.example.test/rpc?Access_Token=must-not-persist",
        ] {
            let input = HostedMcpEndpoint::new(endpoint).expect("wire endpoint");
            assert_eq!(
                CanonicalHostedMcpEndpoint::parse(&input),
                Err(HostedMcpAdmissionError::InvalidEndpoint)
            );
        }
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
