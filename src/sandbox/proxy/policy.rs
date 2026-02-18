//! Network policy decision making.
//!
//! Determines whether network requests should be allowed, denied,
//! or allowed with credential injection.

use async_trait::async_trait;

use crate::sandbox::proxy::allowlist::DomainAllowlist;
use crate::secrets::{CredentialLocation, CredentialMapping};

/// A network request to be evaluated.
#[derive(Debug, Clone)]
pub struct NetworkRequest {
    /// HTTP method (GET, POST, etc.).
    pub method: String,
    /// Full URL being requested.
    pub url: String,
    /// Host extracted from URL.
    pub host: String,
    /// Path portion of the URL.
    pub path: String,
}

impl NetworkRequest {
    /// Create from a URL string.
    pub fn from_url(method: &str, url: &str) -> Option<Self> {
        let parsed = url::Url::parse(url).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }

        let host = parsed.host_str()?;
        let host = host
            .strip_prefix('[')
            .and_then(|v| v.strip_suffix(']'))
            .unwrap_or(host)
            .to_lowercase();
        let path = parsed.path().to_string();

        Some(Self {
            method: method.to_uppercase(),
            url: url.to_string(),
            host,
            path,
        })
    }
}

/// Extract path from a URL.
#[cfg(test)]
fn extract_path(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return "/".to_string();
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return "/".to_string();
    }
    parsed.path().to_string()
}

/// Decision for a network request.
#[derive(Debug, Clone)]
pub enum NetworkDecision {
    /// Allow the request as-is.
    Allow,
    /// Allow with credential injection.
    AllowWithCredentials {
        /// Name of the secret to look up.
        secret_name: String,
        /// Where to inject the credential.
        location: CredentialLocation,
    },
    /// Deny the request.
    Deny {
        /// Reason for denial.
        reason: String,
    },
}

impl NetworkDecision {
    pub fn is_allowed(&self) -> bool {
        !matches!(self, NetworkDecision::Deny { .. })
    }
}

/// Trait for making network policy decisions.
#[async_trait]
pub trait NetworkPolicyDecider: Send + Sync {
    /// Decide whether a request should be allowed.
    async fn decide(&self, request: &NetworkRequest) -> NetworkDecision;
}

/// Default policy decider that uses allowlist and credential mappings.
pub struct DefaultPolicyDecider {
    allowlist: DomainAllowlist,
    credential_mappings: Vec<CredentialMapping>,
}

impl DefaultPolicyDecider {
    /// Create a new policy decider.
    pub fn new(allowlist: DomainAllowlist, credential_mappings: Vec<CredentialMapping>) -> Self {
        Self {
            allowlist,
            credential_mappings,
        }
    }

    /// Find credential mapping for a host (supports glob patterns like `*.example.com`).
    fn find_credential(&self, host: &str) -> Option<&CredentialMapping> {
        let host_lower = host.to_lowercase();
        self.credential_mappings.iter().find(|m| {
            m.host_patterns
                .iter()
                .any(|pattern| host_matches_pattern(&host_lower, pattern))
        })
    }
}

#[async_trait]
impl NetworkPolicyDecider for DefaultPolicyDecider {
    async fn decide(&self, request: &NetworkRequest) -> NetworkDecision {
        // First check if the domain is allowed
        let validation = self.allowlist.is_allowed(&request.host);
        if !validation.is_allowed()
            && let crate::sandbox::proxy::allowlist::DomainValidationResult::Denied(reason) =
                validation
        {
            return NetworkDecision::Deny { reason };
        }

        // Check if we need to inject credentials
        if let Some(mapping) = self.find_credential(&request.host) {
            return NetworkDecision::AllowWithCredentials {
                secret_name: mapping.secret_name.clone(),
                location: mapping.location.clone(),
            };
        }

        NetworkDecision::Allow
    }
}

/// Check if a host matches a pattern (supports `*.example.com` wildcards).
fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    let pattern_lower = pattern.to_lowercase();
    if pattern_lower == host {
        return true;
    }

    // Support wildcard: *.example.com matches sub.example.com
    if let Some(suffix) = pattern_lower.strip_prefix("*.")
        && host.ends_with(suffix)
        && host.len() > suffix.len()
    {
        let prefix = &host[..host.len() - suffix.len()];
        if prefix.ends_with('.') || prefix.is_empty() {
            return true;
        }
    }

    false
}

/// A policy decider that allows everything (use with FullAccess policy).
pub struct AllowAllDecider;

#[async_trait]
impl NetworkPolicyDecider for AllowAllDecider {
    async fn decide(&self, _request: &NetworkRequest) -> NetworkDecision {
        NetworkDecision::Allow
    }
}

/// A policy decider that denies everything.
pub struct DenyAllDecider {
    reason: String,
}

impl DenyAllDecider {
    pub fn new(reason: &str) -> Self {
        Self {
            reason: reason.to_string(),
        }
    }
}

#[async_trait]
impl NetworkPolicyDecider for DenyAllDecider {
    async fn decide(&self, _request: &NetworkRequest) -> NetworkDecision {
        NetworkDecision::Deny {
            reason: self.reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_request_from_url() {
        let req = NetworkRequest::from_url("GET", "https://api.example.com/v1/data").unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.host, "api.example.com");
        assert_eq!(req.path, "/v1/data");
    }

    #[test]
    fn test_extract_path() {
        assert_eq!(
            extract_path("https://example.com/api/v1"),
            "/api/v1".to_string()
        );
        assert_eq!(extract_path("https://example.com"), "/".to_string());
        assert_eq!(extract_path("https://example.com/"), "/".to_string());
        assert_eq!(
            extract_path("https://example.com/path?q=1#frag"),
            "/path".to_string()
        );
        assert_eq!(extract_path("ftp://example.com/path"), "/".to_string());
    }

    #[tokio::test]
    async fn test_default_policy_allows_listed_domain() {
        let allowlist = DomainAllowlist::new(&["crates.io".to_string()]);
        let decider = DefaultPolicyDecider::new(allowlist, vec![]);

        let req = NetworkRequest::from_url("GET", "https://crates.io/api/v1/crates").unwrap();
        let decision = decider.decide(&req).await;

        assert!(decision.is_allowed());
    }

    #[tokio::test]
    async fn test_default_policy_denies_unlisted_domain() {
        let allowlist = DomainAllowlist::new(&["crates.io".to_string()]);
        let decider = DefaultPolicyDecider::new(allowlist, vec![]);

        let req = NetworkRequest::from_url("GET", "https://evil.com/steal").unwrap();
        let decision = decider.decide(&req).await;

        assert!(!decision.is_allowed());
    }

    #[tokio::test]
    async fn test_credential_injection() {
        let allowlist = DomainAllowlist::new(&["api.openai.com".to_string()]);
        let credentials = vec![CredentialMapping::bearer(
            "OPENAI_API_KEY",
            "api.openai.com",
        )];
        let decider = DefaultPolicyDecider::new(allowlist, credentials);

        let req =
            NetworkRequest::from_url("POST", "https://api.openai.com/v1/chat/completions").unwrap();
        let decision = decider.decide(&req).await;

        match decision {
            NetworkDecision::AllowWithCredentials { secret_name, .. } => {
                assert_eq!(secret_name, "OPENAI_API_KEY");
            }
            _ => panic!("Expected AllowWithCredentials"),
        }
    }

    #[tokio::test]
    async fn test_credential_injection_with_wildcard_host_pattern() {
        let allowlist =
            DomainAllowlist::new(&["api.example.com".to_string(), "sub.example.com".to_string()]);
        let credentials = vec![CredentialMapping {
            secret_name: "EXAMPLE_KEY".to_string(),
            location: CredentialLocation::AuthorizationBearer,
            host_patterns: vec!["*.example.com".to_string()],
        }];
        let decider = DefaultPolicyDecider::new(allowlist, credentials);

        let req = NetworkRequest::from_url("GET", "https://api.example.com/data").unwrap();
        let decision = decider.decide(&req).await;

        match decision {
            NetworkDecision::AllowWithCredentials { secret_name, .. } => {
                assert_eq!(secret_name, "EXAMPLE_KEY");
            }
            _ => panic!("Expected AllowWithCredentials for wildcard match"),
        }

        let req2 = NetworkRequest::from_url("GET", "https://sub.example.com/data").unwrap();
        let decision2 = decider.decide(&req2).await;
        assert!(
            matches!(decision2, NetworkDecision::AllowWithCredentials { .. }),
            "Wildcard pattern should match sub.example.com too"
        );
    }

    #[test]
    fn test_host_matches_pattern_exact() {
        assert!(host_matches_pattern("api.openai.com", "api.openai.com"));
        assert!(!host_matches_pattern("api.openai.com", "evil.com"));
    }

    #[test]
    fn test_host_matches_pattern_wildcard() {
        assert!(host_matches_pattern("api.example.com", "*.example.com"));
        assert!(!host_matches_pattern("example.com", "*.example.com"));
    }
}
