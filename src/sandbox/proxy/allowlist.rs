//! Domain allowlist for the network proxy.
//!
//! Validates that HTTP requests only go to allowed domains.
//! Supports exact matches and wildcard patterns.

use std::fmt;

/// Pattern for matching allowed domains.
#[derive(Debug, Clone)]
pub struct DomainPattern {
    /// The domain pattern (e.g., "api.example.com" or "*.example.com").
    pattern: String,
    /// Whether this is a wildcard pattern.
    is_wildcard: bool,
    /// The base domain for wildcard matching.
    base_domain: String,
}

impl DomainPattern {
    /// Create a new domain pattern.
    pub fn new(pattern: &str) -> Self {
        let is_wildcard = pattern.starts_with("*.");
        let base_domain = if is_wildcard {
            pattern[2..].to_lowercase()
        } else {
            pattern.to_lowercase()
        };

        Self {
            pattern: pattern.to_string(),
            is_wildcard,
            base_domain,
        }
    }

    /// Check if a host matches this pattern.
    pub fn matches(&self, host: &str) -> bool {
        let host_lower = host.to_lowercase();

        if self.is_wildcard {
            // *.example.com matches foo.example.com, bar.baz.example.com, example.com
            host_lower == self.base_domain
                || host_lower.ends_with(&format!(".{}", self.base_domain))
        } else {
            host_lower == self.base_domain
        }
    }

    /// Get the pattern string.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }
}

impl fmt::Display for DomainPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.pattern)
    }
}

/// Result of domain validation.
#[derive(Debug, Clone)]
pub enum DomainValidationResult {
    /// Domain is allowed.
    Allowed,
    /// Domain is denied with a reason.
    Denied(String),
}

impl DomainValidationResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, DomainValidationResult::Allowed)
    }
}

/// Validates domains against an allowlist.
#[derive(Debug, Clone)]
pub struct DomainAllowlist {
    patterns: Vec<DomainPattern>,
}

impl DomainAllowlist {
    /// Create a new allowlist from domain strings.
    pub fn new(domains: &[String]) -> Self {
        Self {
            patterns: domains.iter().map(|d| DomainPattern::new(d)).collect(),
        }
    }

    /// Create an empty allowlist (denies everything).
    pub fn empty() -> Self {
        Self { patterns: vec![] }
    }

    /// Add a domain pattern to the allowlist.
    pub fn add(&mut self, pattern: &str) {
        self.patterns.push(DomainPattern::new(pattern));
    }

    /// Check if a domain is allowed.
    pub fn is_allowed(&self, host: &str) -> DomainValidationResult {
        if self.patterns.is_empty() {
            return DomainValidationResult::Denied("empty allowlist".to_string());
        }

        for pattern in &self.patterns {
            if pattern.matches(host) {
                return DomainValidationResult::Allowed;
            }
        }

        DomainValidationResult::Denied(format!(
            "host '{}' not in allowlist: [{}]",
            host,
            self.patterns
                .iter()
                .map(|p| p.pattern())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// Get all patterns in the allowlist.
    pub fn patterns(&self) -> &[DomainPattern] {
        &self.patterns
    }

    /// Check if the allowlist is empty.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Get the number of patterns.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }
}

impl Default for DomainAllowlist {
    fn default() -> Self {
        Self::new(&crate::sandbox::config::default_allowlist())
    }
}

/// Parse host from a URL string.
pub fn extract_host(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.host_str().map(|h| {
        h.strip_prefix('[')
            .and_then(|v| v.strip_suffix(']'))
            .unwrap_or(h)
            .to_lowercase()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let pattern = DomainPattern::new("api.example.com");
        assert!(pattern.matches("api.example.com"));
        assert!(pattern.matches("API.EXAMPLE.COM"));
        assert!(!pattern.matches("foo.api.example.com"));
        assert!(!pattern.matches("example.com"));
    }

    #[test]
    fn test_wildcard_match() {
        let pattern = DomainPattern::new("*.example.com");
        assert!(pattern.matches("api.example.com"));
        assert!(pattern.matches("foo.bar.example.com"));
        assert!(pattern.matches("example.com")); // Base domain also matches
        assert!(!pattern.matches("exampleXcom"));
        assert!(!pattern.matches("other.com"));
    }

    #[test]
    fn test_allowlist_allows() {
        let allowlist =
            DomainAllowlist::new(&["crates.io".to_string(), "*.github.com".to_string()]);

        assert!(allowlist.is_allowed("crates.io").is_allowed());
        assert!(allowlist.is_allowed("api.github.com").is_allowed());
        assert!(
            !allowlist
                .is_allowed("raw.githubusercontent.com")
                .is_allowed()
        );
    }

    #[test]
    fn test_allowlist_denies() {
        let allowlist = DomainAllowlist::new(&["crates.io".to_string()]);

        let result = allowlist.is_allowed("evil.com");
        assert!(!result.is_allowed());
    }

    #[test]
    fn test_empty_allowlist() {
        let allowlist = DomainAllowlist::empty();
        assert!(!allowlist.is_allowed("anything.com").is_allowed());
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://api.example.com/v1/endpoint"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            extract_host("http://localhost:8080/api"),
            Some("localhost".to_string())
        );
        assert_eq!(
            extract_host("https://EXAMPLE.COM"),
            Some("example.com".to_string())
        );
        assert_eq!(
            extract_host("https://user:pass@api.example.com:443/path"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            extract_host("http://[::1]:8080/path"),
            Some("::1".to_string())
        );
        assert_eq!(extract_host("not-a-url"), None);
        assert_eq!(extract_host("ftp://example.com/file"), None);
    }

    // === QA Plan P1 - 4.5: Adversarial allowlist tests ===

    #[test]
    fn test_subdomain_bypass_attempt() {
        let allowlist = DomainAllowlist::new(&["api.example.com".to_string()]);

        // Exact match should work
        assert!(allowlist.is_allowed("api.example.com").is_allowed());

        // Subdomain of exact match should NOT be allowed
        assert!(!allowlist.is_allowed("evil.api.example.com").is_allowed());

        // Similar-looking domains should NOT be allowed
        assert!(
            !allowlist
                .is_allowed("api.example.com.evil.com")
                .is_allowed()
        );
        assert!(!allowlist.is_allowed("api-example.com").is_allowed());
        assert!(!allowlist.is_allowed("notapi.example.com").is_allowed());
    }

    #[test]
    fn test_wildcard_depth() {
        let allowlist = DomainAllowlist::new(&["*.github.com".to_string()]);

        // Direct subdomain
        assert!(allowlist.is_allowed("api.github.com").is_allowed());
        // Multi-level subdomain
        assert!(allowlist.is_allowed("a.b.c.github.com").is_allowed());
        // Base domain itself
        assert!(allowlist.is_allowed("github.com").is_allowed());

        // But NOT a completely different domain
        assert!(!allowlist.is_allowed("github.com.evil.com").is_allowed());
        assert!(!allowlist.is_allowed("notgithub.com").is_allowed());
    }

    #[test]
    fn test_case_insensitive_domains() {
        let allowlist = DomainAllowlist::new(&["crates.io".to_string()]);

        assert!(allowlist.is_allowed("CRATES.IO").is_allowed());
        assert!(allowlist.is_allowed("Crates.Io").is_allowed());
        assert!(allowlist.is_allowed("cRaTeS.iO").is_allowed());
    }

    #[test]
    fn test_extract_host_with_credentials_in_url() {
        // Credentials in URL should not affect host extraction
        assert_eq!(
            extract_host("https://secret_key:password@evil.com/exfil"),
            Some("evil.com".to_string())
        );
    }

    #[test]
    fn test_extract_host_port_ignored() {
        // Port should not affect host extraction
        assert_eq!(
            extract_host("https://api.example.com:9999/path"),
            Some("api.example.com".to_string())
        );
    }

    #[test]
    fn test_empty_and_single_pattern() {
        // Empty allowlist denies everything
        let empty = DomainAllowlist::empty();
        assert!(!empty.is_allowed("localhost").is_allowed());
        assert!(!empty.is_allowed("127.0.0.1").is_allowed());

        // Single wildcard should allow subdomains but not unrelated domains
        let single = DomainAllowlist::new(&["*.example.com".to_string()]);
        assert!(single.is_allowed("any.example.com").is_allowed());
        assert!(!single.is_allowed("other.org").is_allowed());
    }

    #[test]
    fn test_ip_address_not_matched_by_domain() {
        let allowlist = DomainAllowlist::new(&["example.com".to_string()]);

        // IP addresses should NOT match domain names
        assert!(!allowlist.is_allowed("93.184.216.34").is_allowed());
        assert!(!allowlist.is_allowed("127.0.0.1").is_allowed());
    }

    #[test]
    fn test_extract_host_ipv6() {
        // IPv6 addresses with brackets stripped
        assert_eq!(
            extract_host("https://[::1]:8080/api"),
            Some("::1".to_string())
        );
        assert_eq!(
            extract_host("https://[2001:db8::1]/path"),
            Some("2001:db8::1".to_string())
        );
    }
}
