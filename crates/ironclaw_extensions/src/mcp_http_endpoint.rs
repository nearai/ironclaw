//! Canonical parser and matcher for manifest-declared MCP HTTP endpoints.
//!
//! HTTPS endpoints may use any syntactically valid host. Plaintext HTTP is
//! accepted only for literal IPv4 loopback hosts; DNS names, IPv6, and
//! non-loopback addresses are rejected. Parsing establishes endpoint identity;
//! manifest-source authorization and egress policy remain host concerns.

/// Scheme accepted for a validated MCP HTTP endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpHttpScheme {
    /// Plaintext HTTP on a literal IPv4 loopback host.
    Http,
    /// HTTPS on a syntactically valid URL host.
    Https,
}

/// Canonical manifest-declared MCP HTTP endpoint.
///
/// Parsing rejects userinfo, queries, and fragments. Matching compares the
/// scheme, canonical host, explicit port, and path after trailing-slash
/// normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHttpEndpoint {
    scheme: McpHttpScheme,
    host: String,
    port: Option<u16>,
    path: String,
}

impl McpHttpEndpoint {
    /// Parses an HTTPS endpoint or a literal IPv4 loopback HTTP endpoint.
    pub fn parse(url: &str) -> Option<Self> {
        let parsed = url::Url::parse(url).ok()?;
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return None;
        }
        let scheme = match parsed.scheme() {
            "https" => McpHttpScheme::Https,
            "http" if literal_ipv4_loopback_host(&parsed) => McpHttpScheme::Http,
            _ => return None,
        };
        Some(Self {
            scheme,
            host: parsed.host_str()?.to_ascii_lowercase(),
            port: parsed.port(),
            path: normalize_path(parsed.path()),
        })
    }

    /// Returns the validated endpoint scheme.
    pub fn scheme(&self) -> McpHttpScheme {
        self.scheme
    }

    /// Returns the canonical lowercase host.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the explicitly declared port, if any.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Returns whether this is a plaintext literal IPv4 loopback endpoint.
    pub fn is_literal_ipv4_loopback_http(&self) -> bool {
        self.scheme == McpHttpScheme::Http
    }

    /// Returns whether `url` resolves to this same canonical endpoint.
    pub fn matches_url(&self, url: &str) -> bool {
        Self::parse(url).is_some_and(|candidate| candidate == *self)
    }
}

fn literal_ipv4_loopback_host(url: &url::Url) -> bool {
    matches!(url.host(), Some(url::Host::Ipv4(address)) if address.is_loopback())
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_and_literal_ipv4_loopback_http() {
        assert_eq!(
            McpHttpEndpoint::parse("https://MCP.example.com:8443/mcp/")
                .expect("HTTPS endpoint")
                .host(),
            "mcp.example.com"
        );
        assert!(
            McpHttpEndpoint::parse("http://127.0.0.2:4321/mcp")
                .expect("loopback endpoint")
                .is_literal_ipv4_loopback_http()
        );
    }

    #[test]
    fn rejects_non_literal_or_non_loopback_plaintext_and_url_metadata() {
        for url in [
            "http://localhost:4321/mcp",
            "http://[::1]:4321/mcp",
            "http://192.168.1.10:4321/mcp",
            "https://user@example.com/mcp",
            "https://example.com/mcp?query=1",
            "https://example.com/mcp#fragment",
        ] {
            assert!(McpHttpEndpoint::parse(url).is_none(), "accepted {url}");
        }
    }
}
