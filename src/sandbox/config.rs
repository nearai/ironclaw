//! Configuration for the Docker execution sandbox.

use std::time::Duration;

/// Configuration for the sandbox system.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Whether the sandbox is enabled.
    pub enabled: bool,
    /// Security policy for sandbox execution.
    pub policy: SandboxPolicy,
    /// Default timeout for command execution.
    pub timeout: Duration,
    /// Memory limit in megabytes.
    pub memory_limit_mb: u64,
    /// CPU shares (relative weight, default 1024).
    pub cpu_shares: u32,
    /// Network allowlist for proxied requests.
    pub network_allowlist: Vec<String>,
    /// Docker image to use for the sandbox.
    pub image: String,
    /// Whether to auto-pull the image if not found.
    pub auto_pull_image: bool,
    /// Port for the HTTP proxy (0 = auto-assign).
    pub proxy_port: u16,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default until Docker is confirmed available
            policy: SandboxPolicy::ReadOnly,
            timeout: Duration::from_secs(120),
            memory_limit_mb: 2048,
            cpu_shares: 1024,
            network_allowlist: default_allowlist(),
            image: "ironclaw-worker:latest".to_string(),
            auto_pull_image: true,
            proxy_port: 0,
        }
    }
}

/// Security policy for sandbox execution.
///
/// ```text
/// ┌─────────────────────────────────────────────────────────────────────┐
/// │                        Sandbox Policies                              │
/// ├─────────────────┬──────────────────┬────────────────────────────────┤
/// │ Policy          │ Filesystem       │ Network                        │
/// ├─────────────────┼──────────────────┼────────────────────────────────┤
/// │ ReadOnly        │ /workspace (ro)  │ Proxied (allowlist only)       │
/// │ WorkspaceWrite  │ /workspace (rw)  │ Proxied (allowlist only)       │
/// │ FullAccess      │ Full host        │ Full network (DANGER)          │
/// └─────────────────┴──────────────────┴────────────────────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxPolicy {
    /// Read-only access to workspace, proxied network.
    /// Use for: exploring code, fetching docs, read-only operations.
    #[default]
    ReadOnly,

    /// Read/write access to workspace, proxied network.
    /// Use for: building software, running tests, generating files.
    WorkspaceWrite,

    /// Full access (no sandbox). Use with extreme caution.
    /// This bypasses all isolation and runs directly on host.
    FullAccess,
}

impl SandboxPolicy {
    /// Returns true if filesystem writes are allowed.
    pub fn allows_writes(&self) -> bool {
        matches!(
            self,
            SandboxPolicy::WorkspaceWrite | SandboxPolicy::FullAccess
        )
    }

    /// Returns true if network requests bypass the proxy.
    pub fn has_full_network(&self) -> bool {
        matches!(self, SandboxPolicy::FullAccess)
    }

    /// Returns true if running in a container.
    pub fn is_sandboxed(&self) -> bool {
        !matches!(self, SandboxPolicy::FullAccess)
    }
}

impl std::str::FromStr for SandboxPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "readonly" | "read_only" | "ro" => Ok(SandboxPolicy::ReadOnly),
            "workspacewrite" | "workspace_write" | "rw" => Ok(SandboxPolicy::WorkspaceWrite),
            "fullaccess" | "full_access" | "full" | "none" => Ok(SandboxPolicy::FullAccess),
            _ => Err(format!(
                "invalid sandbox policy '{}', expected 'readonly', 'workspace_write', or 'full_access'",
                s
            )),
        }
    }
}

/// Resource limits for container execution.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum memory in bytes.
    pub memory_bytes: u64,
    /// CPU shares (relative weight).
    pub cpu_shares: u32,
    /// Maximum execution time.
    pub timeout: Duration,
    /// Maximum output size in bytes.
    pub max_output_bytes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 2 * 1024 * 1024 * 1024, // 2 GB
            cpu_shares: 1024,
            timeout: Duration::from_secs(120),
            max_output_bytes: 64 * 1024, // 64 KB
        }
    }
}

/// Default network allowlist for common development operations.
pub fn default_allowlist() -> Vec<String> {
    vec![
        // Package registries
        "crates.io".to_string(),
        "static.crates.io".to_string(),
        "index.crates.io".to_string(),
        "registry.npmjs.org".to_string(),
        "proxy.golang.org".to_string(),
        "pypi.org".to_string(),
        "files.pythonhosted.org".to_string(),
        // Documentation
        "docs.rs".to_string(),
        "doc.rust-lang.org".to_string(),
        "nodejs.org".to_string(),
        "go.dev".to_string(),
        "docs.python.org".to_string(),
        // Version control (read-only)
        "github.com".to_string(),
        "raw.githubusercontent.com".to_string(),
        "api.github.com".to_string(),
        "codeload.github.com".to_string(),
        // Common APIs (credentials will be injected by proxy)
        "api.openai.com".to_string(),
        "api.anthropic.com".to_string(),
        "api.near.ai".to_string(),
    ]
}

/// Default credential mappings for common APIs.
pub fn default_credential_mappings() -> Vec<crate::secrets::CredentialMapping> {
    use crate::secrets::CredentialMapping;

    vec![
        CredentialMapping::bearer("OPENAI_API_KEY", "api.openai.com"),
        CredentialMapping::header("ANTHROPIC_API_KEY", "x-api-key", "api.anthropic.com"),
        CredentialMapping::bearer("NEARAI_API_KEY", "api.near.ai"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_parsing() {
        assert_eq!(
            "readonly".parse::<SandboxPolicy>().unwrap(),
            SandboxPolicy::ReadOnly
        );
        assert_eq!(
            "workspace_write".parse::<SandboxPolicy>().unwrap(),
            SandboxPolicy::WorkspaceWrite
        );
        assert_eq!(
            "full_access".parse::<SandboxPolicy>().unwrap(),
            SandboxPolicy::FullAccess
        );
        assert!("invalid".parse::<SandboxPolicy>().is_err());
    }

    #[test]
    fn test_policy_properties() {
        assert!(!SandboxPolicy::ReadOnly.allows_writes());
        assert!(SandboxPolicy::WorkspaceWrite.allows_writes());
        assert!(SandboxPolicy::FullAccess.allows_writes());

        assert!(!SandboxPolicy::ReadOnly.has_full_network());
        assert!(!SandboxPolicy::WorkspaceWrite.has_full_network());
        assert!(SandboxPolicy::FullAccess.has_full_network());

        assert!(SandboxPolicy::ReadOnly.is_sandboxed());
        assert!(SandboxPolicy::WorkspaceWrite.is_sandboxed());
        assert!(!SandboxPolicy::FullAccess.is_sandboxed());
    }

    #[test]
    fn test_default_allowlist_has_common_registries() {
        let allowlist = default_allowlist();
        assert!(allowlist.contains(&"crates.io".to_string()));
        assert!(allowlist.contains(&"registry.npmjs.org".to_string()));
        assert!(allowlist.contains(&"github.com".to_string()));
    }
}
