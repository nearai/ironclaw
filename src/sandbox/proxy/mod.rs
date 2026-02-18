//! Network proxy for sandboxed container access.
//!
//! The proxy provides:
//! - Domain allowlist validation
//! - Credential injection for API calls
//! - Request logging and monitoring
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      Network Proxy                               │
//! │                                                                  │
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────────────┐  │
//! │  │ HTTP Proxy  │───▶│   Policy    │───▶│ Credential Resolver │  │
//! │  │   Server    │    │   Decider   │    │                     │  │
//! │  └─────────────┘    └─────────────┘    └─────────────────────┘  │
//! │         │                  │                                     │
//! │         │                  ▼                                     │
//! │         │           ┌─────────────┐                             │
//! │         │           │  Allowlist  │                             │
//! │         │           │  Validator  │                             │
//! │         │           └─────────────┘                             │
//! │         ▼                                                        │
//! │  ┌──────────────────────────────────────────────────────────┐   │
//! │  │                    Internet                               │   │
//! │  └──────────────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

pub mod allowlist;
pub mod http;
pub mod policy;

pub use allowlist::{DomainAllowlist, DomainPattern, DomainValidationResult};
pub use http::{CredentialResolver, EnvCredentialResolver, HttpProxy, NoCredentialResolver};
pub use policy::{
    AllowAllDecider, DefaultPolicyDecider, DenyAllDecider, NetworkDecision, NetworkPolicyDecider,
    NetworkRequest,
};

use std::sync::Arc;

use crate::sandbox::config::{SandboxConfig, SandboxPolicy, default_credential_mappings};
use crate::sandbox::error::Result;
use crate::secrets::CredentialMapping;

/// Creates a configured network proxy from sandbox config.
pub struct NetworkProxyBuilder {
    allowlist: Vec<String>,
    credential_mappings: Vec<CredentialMapping>,
    credential_resolver: Arc<dyn CredentialResolver>,
    policy: SandboxPolicy,
}

impl NetworkProxyBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self {
            allowlist: crate::sandbox::config::default_allowlist(),
            credential_mappings: default_credential_mappings(),
            credential_resolver: Arc::new(EnvCredentialResolver),
            policy: SandboxPolicy::ReadOnly,
        }
    }

    /// Create from a sandbox config.
    pub fn from_config(config: &SandboxConfig) -> Self {
        Self {
            allowlist: config.network_allowlist.clone(),
            credential_mappings: default_credential_mappings(),
            credential_resolver: Arc::new(EnvCredentialResolver),
            policy: config.policy,
        }
    }

    /// Set the domain allowlist.
    pub fn with_allowlist(mut self, domains: Vec<String>) -> Self {
        self.allowlist = domains;
        self
    }

    /// Add a domain to the allowlist.
    pub fn allow_domain(mut self, domain: &str) -> Self {
        self.allowlist.push(domain.to_string());
        self
    }

    /// Set credential mappings.
    pub fn with_credentials(mut self, mappings: Vec<CredentialMapping>) -> Self {
        self.credential_mappings = mappings;
        self
    }

    /// Set the credential resolver.
    pub fn with_credential_resolver(mut self, resolver: Arc<dyn CredentialResolver>) -> Self {
        self.credential_resolver = resolver;
        self
    }

    /// Set the sandbox policy.
    pub fn with_policy(mut self, policy: SandboxPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Build the HTTP proxy.
    pub fn build(self) -> HttpProxy {
        let decider: Arc<dyn NetworkPolicyDecider> = if self.policy.has_full_network() {
            Arc::new(AllowAllDecider)
        } else {
            Arc::new(DefaultPolicyDecider::new(
                DomainAllowlist::new(&self.allowlist),
                self.credential_mappings,
            ))
        };

        HttpProxy::new(decider, self.credential_resolver)
    }

    /// Build and start the proxy on the given port.
    pub async fn build_and_start(self, port: u16) -> Result<HttpProxy> {
        let proxy = self.build();
        proxy.start(port).await?;
        Ok(proxy)
    }
}

impl Default for NetworkProxyBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_default() {
        let builder = NetworkProxyBuilder::new();
        assert!(!builder.allowlist.is_empty());
    }

    #[test]
    fn test_builder_with_custom_allowlist() {
        let builder = NetworkProxyBuilder::new()
            .with_allowlist(vec!["custom.com".to_string()])
            .allow_domain("another.com");

        assert!(builder.allowlist.contains(&"custom.com".to_string()));
        assert!(builder.allowlist.contains(&"another.com".to_string()));
    }

    #[tokio::test]
    async fn test_builder_builds_proxy() {
        let proxy = NetworkProxyBuilder::new()
            .with_policy(SandboxPolicy::ReadOnly)
            .build();

        assert!(!proxy.is_running());
    }
}
