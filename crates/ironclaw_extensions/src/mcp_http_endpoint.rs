#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpHttpScheme {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHttpEndpoint {
    scheme: McpHttpScheme,
    host: String,
    port: Option<u16>,
    path: String,
}

impl McpHttpEndpoint {
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

    pub fn scheme(&self) -> McpHttpScheme {
        self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn is_literal_ipv4_loopback_http(&self) -> bool {
        self.scheme == McpHttpScheme::Http
    }

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
