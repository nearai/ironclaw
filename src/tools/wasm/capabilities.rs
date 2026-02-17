//! Extended capabilities for WASM sandbox.
//!
//! Defines the capability system that controls what a WASM tool can do.
//! All capabilities are opt-in; tools have NO access by default.
//!
//! # Capability Types
//!
//! - **Workspace**: Read files from the agent's workspace
//! - **HTTP**: Make HTTP requests to allowlisted endpoints
//! - **ToolInvoke**: Call other tools via aliases
//! - **Secrets**: Check if secrets exist (never read values)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::hooks::hook::{HookFailureMode, HookPoint};
use crate::secrets::CredentialMapping;

/// All capabilities that can be granted to a WASM tool.
///
/// By default, all capabilities are `None` (disabled).
/// Each must be explicitly granted.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    /// Read files from workspace.
    pub workspace_read: Option<WorkspaceCapability>,
    /// Make HTTP requests.
    pub http: Option<HttpCapability>,
    /// Invoke other tools.
    pub tool_invoke: Option<ToolInvokeCapability>,
    /// Check if secrets exist.
    pub secrets: Option<SecretsCapability>,
    /// Lifecycle hook declarations.
    pub hooks: Option<HookCapabilities>,
}

/// Lifecycle hook capabilities for a WASM tool.
#[derive(Debug, Clone)]
pub struct HookCapabilities {
    /// Hook points this tool intercepts.
    pub points: Vec<HookPoint>,
    /// How to handle hook execution failures.
    pub failure_mode: HookFailureMode,
    /// Maximum execution time for the hook.
    pub timeout: Duration,
    /// Priority (lower = runs first).
    pub priority: u32,
}

impl Capabilities {
    /// Create capabilities with no permissions.
    pub fn none() -> Self {
        Self::default()
    }

    /// Enable workspace read with the given allowed prefixes.
    pub fn with_workspace_read(mut self, prefixes: Vec<String>) -> Self {
        self.workspace_read = Some(WorkspaceCapability {
            allowed_prefixes: prefixes,
            reader: None,
        });
        self
    }

    /// Enable HTTP requests with the given configuration.
    pub fn with_http(mut self, http: HttpCapability) -> Self {
        self.http = Some(http);
        self
    }

    /// Enable tool invocation with the given aliases.
    pub fn with_tool_invoke(mut self, aliases: HashMap<String, String>) -> Self {
        self.tool_invoke = Some(ToolInvokeCapability {
            aliases,
            rate_limit: RateLimitConfig::default(),
        });
        self
    }

    /// Enable secret existence checks.
    pub fn with_secrets(mut self, allowed: Vec<String>) -> Self {
        self.secrets = Some(SecretsCapability {
            allowed_names: allowed,
        });
        self
    }
}

/// Workspace read capability configuration.
#[derive(Clone, Default)]
pub struct WorkspaceCapability {
    /// Allowed path prefixes (e.g., ["context/", "daily/"]).
    /// Empty means all paths allowed (within safety constraints).
    pub allowed_prefixes: Vec<String>,
    /// Function to actually read from workspace.
    /// This is injected by the runtime to avoid coupling to workspace impl.
    pub reader: Option<Arc<dyn WorkspaceReader>>,
}

impl std::fmt::Debug for WorkspaceCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceCapability")
            .field("allowed_prefixes", &self.allowed_prefixes)
            .field("reader", &self.reader.is_some())
            .finish()
    }
}

/// Trait for reading from workspace (allows mocking in tests).
pub trait WorkspaceReader: Send + Sync {
    fn read(&self, path: &str) -> Option<String>;
}

/// HTTP request capability configuration.
#[derive(Debug, Clone)]
pub struct HttpCapability {
    /// Allowed endpoint patterns.
    pub allowlist: Vec<EndpointPattern>,
    /// Credential mappings (secret name -> injection location).
    pub credentials: HashMap<String, CredentialMapping>,
    /// Rate limiting configuration.
    pub rate_limit: RateLimitConfig,
    /// Maximum request body size in bytes.
    pub max_request_bytes: usize,
    /// Maximum response body size in bytes.
    pub max_response_bytes: usize,
    /// Request timeout.
    pub timeout: Duration,
}

impl Default for HttpCapability {
    fn default() -> Self {
        Self {
            allowlist: Vec::new(),
            credentials: HashMap::new(),
            rate_limit: RateLimitConfig::default(),
            max_request_bytes: 1024 * 1024,       // 1 MB
            max_response_bytes: 10 * 1024 * 1024, // 10 MB
            timeout: Duration::from_secs(30),
        }
    }
}

impl HttpCapability {
    /// Create a new HTTP capability with an allowlist.
    pub fn new(allowlist: Vec<EndpointPattern>) -> Self {
        Self {
            allowlist,
            ..Default::default()
        }
    }

    /// Add a credential mapping.
    pub fn with_credential(mut self, name: impl Into<String>, mapping: CredentialMapping) -> Self {
        self.credentials.insert(name.into(), mapping);
        self
    }

    /// Set rate limiting.
    pub fn with_rate_limit(mut self, rate_limit: RateLimitConfig) -> Self {
        self.rate_limit = rate_limit;
        self
    }

    /// Set request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set max request body size.
    pub fn with_max_request_bytes(mut self, bytes: usize) -> Self {
        self.max_request_bytes = bytes;
        self
    }

    /// Set max response body size.
    pub fn with_max_response_bytes(mut self, bytes: usize) -> Self {
        self.max_response_bytes = bytes;
        self
    }
}

/// Pattern for matching allowed HTTP endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointPattern {
    /// Hostname pattern (e.g., "api.example.com", "*.example.com").
    pub host: String,
    /// Path prefix (e.g., "/v1/", "/api/").
    pub path_prefix: Option<String>,
    /// Allowed HTTP methods (empty = all methods allowed).
    pub methods: Vec<String>,
}

impl EndpointPattern {
    /// Create a pattern for a specific host.
    pub fn host(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            path_prefix: None,
            methods: Vec::new(),
        }
    }

    /// Add a path prefix constraint.
    pub fn with_path_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.path_prefix = Some(prefix.into());
        self
    }

    /// Restrict to specific HTTP methods.
    pub fn with_methods(mut self, methods: Vec<String>) -> Self {
        self.methods = methods;
        self
    }

    /// Check if this pattern matches a URL and method.
    pub fn matches(&self, url_host: &str, url_path: &str, method: &str) -> bool {
        // Check host
        if !self.host_matches(url_host) {
            return false;
        }

        // Check path prefix
        if let Some(ref prefix) = self.path_prefix
            && !url_path.starts_with(prefix)
        {
            return false;
        }

        // Check method
        if !self.methods.is_empty() {
            let method_upper = method.to_uppercase();
            if !self
                .methods
                .iter()
                .any(|m| m.to_uppercase() == method_upper)
            {
                return false;
            }
        }

        true
    }

    /// Check if host pattern matches (public for allowlist validation).
    pub fn host_matches(&self, url_host: &str) -> bool {
        if self.host == url_host {
            return true;
        }

        // Support wildcard: *.example.com matches sub.example.com
        if let Some(suffix) = self.host.strip_prefix("*.")
            && url_host.ends_with(suffix)
            && url_host.len() > suffix.len()
        {
            // Ensure there's a dot before the suffix (or it's the whole thing)
            let prefix = &url_host[..url_host.len() - suffix.len()];
            if prefix.ends_with('.') || prefix.is_empty() {
                return true;
            }
        }

        false
    }
}

/// Tool invocation capability.
#[derive(Debug, Clone, Default)]
pub struct ToolInvokeCapability {
    /// Mapping from alias to real tool name.
    /// WASM calls tools by alias, never by real name.
    pub aliases: HashMap<String, String>,
    /// Rate limiting for tool calls.
    pub rate_limit: RateLimitConfig,
}

impl ToolInvokeCapability {
    /// Create with a set of aliases.
    pub fn new(aliases: HashMap<String, String>) -> Self {
        Self {
            aliases,
            rate_limit: RateLimitConfig::default(),
        }
    }

    /// Resolve an alias to a real tool name.
    pub fn resolve_alias(&self, alias: &str) -> Option<&str> {
        self.aliases.get(alias).map(|s| s.as_str())
    }
}

/// Secrets capability (existence check only).
#[derive(Debug, Clone, Default)]
pub struct SecretsCapability {
    /// Secret names this tool can check existence of.
    /// Supports glob: "openai_*" matches "openai_key", "openai_org".
    pub allowed_names: Vec<String>,
}

impl SecretsCapability {
    /// Check if a secret name is allowed.
    pub fn is_allowed(&self, name: &str) -> bool {
        for pattern in &self.allowed_names {
            if pattern == name {
                return true;
            }
            if let Some(prefix) = pattern.strip_suffix('*')
                && name.starts_with(prefix)
            {
                return true;
            }
        }
        false
    }
}

/// Rate limiting configuration.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests per minute.
    pub requests_per_minute: u32,
    /// Maximum requests per hour.
    pub requests_per_hour: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
            requests_per_hour: 1000,
        }
    }
}

impl RateLimitConfig {
    /// Create a restrictive rate limit.
    pub fn restrictive() -> Self {
        Self {
            requests_per_minute: 10,
            requests_per_hour: 100,
        }
    }

    /// Create a permissive rate limit.
    pub fn permissive() -> Self {
        Self {
            requests_per_minute: 120,
            requests_per_hour: 5000,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::wasm::capabilities::{Capabilities, EndpointPattern, SecretsCapability};

    #[test]
    fn test_capabilities_default_is_none() {
        let caps = Capabilities::default();
        assert!(caps.workspace_read.is_none());
        assert!(caps.http.is_none());
        assert!(caps.tool_invoke.is_none());
        assert!(caps.secrets.is_none());
    }

    #[test]
    fn test_endpoint_pattern_exact_host() {
        let pattern = EndpointPattern::host("api.example.com");

        assert!(pattern.matches("api.example.com", "/", "GET"));
        assert!(!pattern.matches("other.example.com", "/", "GET"));
    }

    #[test]
    fn test_endpoint_pattern_wildcard_host() {
        let pattern = EndpointPattern::host("*.example.com");

        assert!(pattern.matches("api.example.com", "/", "GET"));
        assert!(pattern.matches("sub.api.example.com", "/", "GET"));
        assert!(!pattern.matches("example.com", "/", "GET"));
        assert!(!pattern.matches("notexample.com", "/", "GET"));
    }

    #[test]
    fn test_endpoint_pattern_path_prefix() {
        let pattern = EndpointPattern::host("api.example.com").with_path_prefix("/v1/");

        assert!(pattern.matches("api.example.com", "/v1/users", "GET"));
        assert!(pattern.matches("api.example.com", "/v1/", "GET"));
        assert!(!pattern.matches("api.example.com", "/v2/users", "GET"));
        assert!(!pattern.matches("api.example.com", "/", "GET"));
    }

    #[test]
    fn test_endpoint_pattern_methods() {
        let pattern = EndpointPattern::host("api.example.com")
            .with_methods(vec!["GET".to_string(), "POST".to_string()]);

        assert!(pattern.matches("api.example.com", "/", "GET"));
        assert!(pattern.matches("api.example.com", "/", "get")); // case insensitive
        assert!(pattern.matches("api.example.com", "/", "POST"));
        assert!(!pattern.matches("api.example.com", "/", "DELETE"));
    }

    #[test]
    fn test_secrets_capability_exact_match() {
        let cap = SecretsCapability {
            allowed_names: vec!["openai_key".to_string()],
        };

        assert!(cap.is_allowed("openai_key"));
        assert!(!cap.is_allowed("anthropic_key"));
    }

    #[test]
    fn test_secrets_capability_glob() {
        let cap = SecretsCapability {
            allowed_names: vec!["openai_*".to_string()],
        };

        assert!(cap.is_allowed("openai_key"));
        assert!(cap.is_allowed("openai_org"));
        assert!(!cap.is_allowed("anthropic_key"));
    }

    #[test]
    fn test_capabilities_builder() {
        let caps = Capabilities::none()
            .with_workspace_read(vec!["context/".to_string()])
            .with_secrets(vec!["test_*".to_string()]);

        assert!(caps.workspace_read.is_some());
        assert!(caps.secrets.is_some());
        assert!(caps.http.is_none());
    }
}
