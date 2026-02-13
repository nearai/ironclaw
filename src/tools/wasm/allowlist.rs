//! HTTP endpoint allowlist validation.
//!
//! Validates that HTTP requests from WASM tools only go to allowed endpoints.
//! This is the first line of defense against unauthorized API access.
//!
//! # Validation Flow
//!
//! ```text
//! WASM HTTP request ──► Parse URL ──► Check allowlist ──► Allow/Deny
//!                          │               │
//!                          │               ├─► Host match?
//!                          │               ├─► Path prefix match?
//!                          │               └─► Method allowed?
//!                          │
//!                          └─► Validate URL format
//! ```

use std::fmt;

use crate::tools::wasm::capabilities::EndpointPattern;

/// Result of allowlist validation.
#[derive(Debug, Clone)]
pub enum AllowlistResult {
    /// Request is allowed.
    Allowed,
    /// Request is denied with reason.
    Denied(DenyReason),
}

impl AllowlistResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, AllowlistResult::Allowed)
    }
}

/// Reason why a request was denied.
#[derive(Debug, Clone)]
pub enum DenyReason {
    /// URL could not be parsed.
    InvalidUrl(String),
    /// Host is not in the allowlist.
    HostNotAllowed(String),
    /// Path does not match any allowed prefix.
    PathNotAllowed { host: String, path: String },
    /// HTTP method is not allowed for this endpoint.
    MethodNotAllowed { method: String, host: String },
    /// Allowlist is empty (no endpoints configured).
    EmptyAllowlist,
    /// URL scheme is not HTTPS.
    InsecureScheme(String),
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DenyReason::InvalidUrl(url) => write!(f, "Invalid URL: {}", url),
            DenyReason::HostNotAllowed(host) => write!(f, "Host not in allowlist: {}", host),
            DenyReason::PathNotAllowed { host, path } => {
                write!(f, "Path not allowed for host {}: {}", host, path)
            }
            DenyReason::MethodNotAllowed { method, host } => {
                write!(f, "Method {} not allowed for host {}", method, host)
            }
            DenyReason::EmptyAllowlist => write!(f, "No endpoints in allowlist"),
            DenyReason::InsecureScheme(scheme) => {
                write!(f, "Insecure scheme: {} (only HTTPS allowed)", scheme)
            }
        }
    }
}

/// Validates HTTP requests against an allowlist.
pub struct AllowlistValidator {
    patterns: Vec<EndpointPattern>,
    /// Whether to require HTTPS (default: true).
    require_https: bool,
}

impl AllowlistValidator {
    /// Create a new validator with the given patterns.
    pub fn new(patterns: Vec<EndpointPattern>) -> Self {
        Self {
            patterns,
            require_https: true,
        }
    }

    /// Allow HTTP (insecure) requests. Use with caution.
    pub fn allow_http(mut self) -> Self {
        self.require_https = false;
        self
    }

    /// Check if a request is allowed.
    pub fn validate(&self, url: &str, method: &str) -> AllowlistResult {
        // Check for empty allowlist
        if self.patterns.is_empty() {
            return AllowlistResult::Denied(DenyReason::EmptyAllowlist);
        }

        // Parse the URL
        let parsed = match parse_url(url) {
            Ok(p) => p,
            Err(e) => return AllowlistResult::Denied(DenyReason::InvalidUrl(e)),
        };

        // Check HTTPS requirement
        if self.require_https && parsed.scheme != "https" {
            return AllowlistResult::Denied(DenyReason::InsecureScheme(parsed.scheme.clone()));
        }

        // Find a matching pattern
        for pattern in &self.patterns {
            if pattern.matches(&parsed.host, &parsed.path, method) {
                return AllowlistResult::Allowed;
            }
        }

        // No pattern matched, figure out why for better error messages
        let host_matches: Vec<_> = self
            .patterns
            .iter()
            .filter(|p| p.host_matches(&parsed.host))
            .collect();

        if host_matches.is_empty() {
            AllowlistResult::Denied(DenyReason::HostNotAllowed(parsed.host))
        } else {
            // Host matches but path/method doesn't
            let path_matches: Vec<_> = host_matches
                .iter()
                .filter(|p| {
                    p.path_prefix.is_none()
                        || parsed
                            .path
                            .starts_with(p.path_prefix.as_deref().unwrap_or(""))
                })
                .collect();

            if path_matches.is_empty() {
                AllowlistResult::Denied(DenyReason::PathNotAllowed {
                    host: parsed.host,
                    path: parsed.path,
                })
            } else {
                AllowlistResult::Denied(DenyReason::MethodNotAllowed {
                    method: method.to_string(),
                    host: parsed.host,
                })
            }
        }
    }

    /// Check if any pattern would allow this host.
    pub fn host_allowed(&self, host: &str) -> bool {
        self.patterns.iter().any(|p| p.host_matches(host))
    }

    /// Get all allowed hosts (for debugging/logging).
    pub fn allowed_hosts(&self) -> Vec<&str> {
        self.patterns.iter().map(|p| p.host.as_str()).collect()
    }
}

/// Parsed URL components.
struct ParsedUrl {
    scheme: String,
    host: String,
    path: String,
}

/// Simple URL parser (avoids pulling in a full URL crate).
fn parse_url(url: &str) -> Result<ParsedUrl, String> {
    // Find scheme
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| "Missing scheme (expected http:// or https://)".to_string())?;

    let scheme = scheme.to_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("Unsupported scheme: {}", scheme));
    }

    // Reject URLs with userinfo (user:pass@host) to prevent allowlist bypass.
    // A URL like https://api.openai.com@evil.com/ would match the allowlist
    // for api.openai.com but actually send traffic to evil.com.
    let authority = match rest.find('/') {
        Some(idx) => &rest[..idx],
        None => rest,
    };
    if authority.contains('@') {
        return Err("URL contains userinfo (@) which is not allowed".to_string());
    }

    // Split host from path
    let (host_and_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

    // Remove port from host
    let host = match host_and_port.rfind(':') {
        Some(idx) => {
            // Make sure this isn't an IPv6 address
            if host_and_port.starts_with('[') {
                // IPv6: [::1]:8080 or [::1]
                if let Some(bracket_idx) = host_and_port.find(']') {
                    // Extract the IPv6 address without brackets
                    &host_and_port[1..bracket_idx]
                } else {
                    return Err("Invalid IPv6 address".to_string());
                }
            } else {
                &host_and_port[..idx]
            }
        }
        None => host_and_port,
    };

    // Reject URLs with userinfo (user:pass@host).
    // A URL like https://api.openai.com@evil.com/ confuses the parser into
    // seeing "api.openai.com" as the host, but reqwest actually sends to
    // "evil.com". Block any '@' in the authority section to prevent this.
    if host.contains('@') || host_and_port.contains('@') {
        return Err("URL contains userinfo (@) which is not allowed".to_string());
    }

    // Validate host
    if host.is_empty() {
        return Err("Empty host".to_string());
    }

    Ok(ParsedUrl {
        scheme,
        host: host.to_lowercase(),
        path: path.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use crate::tools::wasm::allowlist::{AllowlistValidator, DenyReason};
    use crate::tools::wasm::capabilities::EndpointPattern;

    fn validator_with_patterns() -> AllowlistValidator {
        AllowlistValidator::new(vec![
            EndpointPattern::host("api.openai.com").with_path_prefix("/v1/"),
            EndpointPattern::host("api.anthropic.com")
                .with_path_prefix("/v1/messages")
                .with_methods(vec!["POST".to_string()]),
            EndpointPattern::host("*.example.com"),
        ])
    }

    #[test]
    fn test_allowed_request() {
        let validator = validator_with_patterns();

        let result = validator.validate("https://api.openai.com/v1/chat/completions", "POST");
        assert!(result.is_allowed());
    }

    #[test]
    fn test_denied_wrong_host() {
        let validator = validator_with_patterns();

        let result = validator.validate("https://evil.com/steal/data", "GET");
        assert!(!result.is_allowed());

        if let super::AllowlistResult::Denied(reason) = result {
            assert!(matches!(reason, DenyReason::HostNotAllowed(_)));
        } else {
            panic!("Expected denied");
        }
    }

    #[test]
    fn test_denied_wrong_path() {
        let validator = validator_with_patterns();

        let result = validator.validate("https://api.openai.com/v2/different", "GET");
        assert!(!result.is_allowed());

        if let super::AllowlistResult::Denied(reason) = result {
            assert!(matches!(reason, DenyReason::PathNotAllowed { .. }));
        } else {
            panic!("Expected denied");
        }
    }

    #[test]
    fn test_denied_wrong_method() {
        let validator = validator_with_patterns();

        // Anthropic endpoint only allows POST
        let result = validator.validate("https://api.anthropic.com/v1/messages", "GET");
        assert!(!result.is_allowed());

        if let super::AllowlistResult::Denied(reason) = result {
            assert!(matches!(reason, DenyReason::MethodNotAllowed { .. }));
        } else {
            panic!("Expected denied");
        }
    }

    #[test]
    fn test_wildcard_host() {
        let validator = validator_with_patterns();

        let result = validator.validate("https://api.example.com/anything", "GET");
        assert!(result.is_allowed());

        let result = validator.validate("https://sub.api.example.com/anything", "GET");
        assert!(result.is_allowed());
    }

    #[test]
    fn test_require_https() {
        let validator = validator_with_patterns();

        let result = validator.validate("http://api.openai.com/v1/chat", "GET");
        assert!(!result.is_allowed());

        if let super::AllowlistResult::Denied(reason) = result {
            assert!(matches!(reason, DenyReason::InsecureScheme(_)));
        } else {
            panic!("Expected denied");
        }
    }

    #[test]
    fn test_allow_http() {
        let validator = validator_with_patterns().allow_http();

        let result = validator.validate("http://api.example.com/test", "GET");
        assert!(result.is_allowed());
    }

    #[test]
    fn test_empty_allowlist() {
        let validator = AllowlistValidator::new(vec![]);

        let result = validator.validate("https://anything.com/", "GET");
        assert!(!result.is_allowed());

        if let super::AllowlistResult::Denied(reason) = result {
            assert!(matches!(reason, DenyReason::EmptyAllowlist));
        } else {
            panic!("Expected denied");
        }
    }

    #[test]
    fn test_userinfo_rejected() {
        let validator = validator_with_patterns();

        // Userinfo in URL should be rejected to prevent allowlist bypass
        let result = validator.validate("https://api.openai.com@evil.com/v1/chat", "GET");
        assert!(!result.is_allowed());

        if let super::AllowlistResult::Denied(reason) = result {
            assert!(matches!(reason, DenyReason::InvalidUrl(_)));
        } else {
            panic!("Expected denied for userinfo URL");
        }
    }

    #[test]
    fn test_invalid_url() {
        let validator = validator_with_patterns();

        let result = validator.validate("not-a-url", "GET");
        assert!(!result.is_allowed());

        if let super::AllowlistResult::Denied(reason) = result {
            assert!(matches!(reason, DenyReason::InvalidUrl(_)));
        } else {
            panic!("Expected denied");
        }
    }

    #[test]
    fn test_url_with_port() {
        let validator =
            AllowlistValidator::new(vec![EndpointPattern::host("localhost")]).allow_http();

        let result = validator.validate("http://localhost:8080/api", "GET");
        assert!(result.is_allowed());
    }

    #[test]
    fn test_reject_url_with_userinfo() {
        let validator = validator_with_patterns();

        // Attacker uses userinfo to trick the parser: the allowlist sees
        // "api.openai.com" but reqwest would actually connect to "evil.com".
        let result = validator.validate("https://api.openai.com@evil.com/v1/steal", "GET");
        assert!(!result.is_allowed());

        if let super::AllowlistResult::Denied(reason) = result {
            assert!(matches!(reason, DenyReason::InvalidUrl(_)));
        } else {
            panic!("Expected denied due to userinfo");
        }
    }

    #[test]
    fn test_reject_url_with_user_pass() {
        let validator = validator_with_patterns();

        let result = validator.validate("https://user:password@api.openai.com/v1/chat", "GET");
        assert!(!result.is_allowed());
    }
}
