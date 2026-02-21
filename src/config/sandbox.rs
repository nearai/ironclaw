use crate::config::helpers::{optional_env, parse_optional_env};
use crate::error::ConfigError;

/// Docker sandbox configuration.
#[derive(Debug, Clone)]
pub struct SandboxModeConfig {
    /// Whether the Docker sandbox is enabled.
    pub enabled: bool,
    /// Sandbox policy: "readonly", "workspace_write", or "full_access".
    pub policy: String,
    /// Command timeout in seconds.
    pub timeout_secs: u64,
    /// Memory limit in megabytes.
    pub memory_limit_mb: u64,
    /// CPU shares (relative weight).
    pub cpu_shares: u32,
    /// Docker image for the sandbox.
    pub image: String,
    /// Whether to auto-pull the image if not found.
    pub auto_pull_image: bool,
    /// Additional domains to allow through the network proxy.
    pub extra_allowed_domains: Vec<String>,
}

impl Default for SandboxModeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: "readonly".to_string(),
            timeout_secs: 120,
            memory_limit_mb: 2048,
            cpu_shares: 1024,
            image: "ironclaw-worker:latest".to_string(),
            auto_pull_image: true,
            extra_allowed_domains: Vec::new(),
        }
    }
}

impl SandboxModeConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let extra_domains = optional_env("SANDBOX_EXTRA_DOMAINS")?
            .map(|s| s.split(',').map(|d| d.trim().to_string()).collect())
            .unwrap_or_default();

        Ok(Self {
            enabled: optional_env("SANDBOX_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "SANDBOX_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
            policy: optional_env("SANDBOX_POLICY")?.unwrap_or_else(|| "readonly".to_string()),
            timeout_secs: parse_optional_env("SANDBOX_TIMEOUT_SECS", 120)?,
            memory_limit_mb: parse_optional_env("SANDBOX_MEMORY_LIMIT_MB", 2048)?,
            cpu_shares: parse_optional_env("SANDBOX_CPU_SHARES", 1024)?,
            image: optional_env("SANDBOX_IMAGE")?
                .unwrap_or_else(|| "ironclaw-worker:latest".to_string()),
            auto_pull_image: optional_env("SANDBOX_AUTO_PULL")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "SANDBOX_AUTO_PULL".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
            extra_allowed_domains: extra_domains,
        })
    }

    /// Convert to SandboxConfig for the sandbox module.
    pub fn to_sandbox_config(&self) -> crate::sandbox::SandboxConfig {
        use crate::sandbox::SandboxPolicy;
        use std::time::Duration;

        let policy = self.policy.parse().unwrap_or(SandboxPolicy::ReadOnly);

        let mut allowlist = crate::sandbox::default_allowlist();
        allowlist.extend(self.extra_allowed_domains.clone());

        crate::sandbox::SandboxConfig {
            enabled: self.enabled,
            policy,
            timeout: Duration::from_secs(self.timeout_secs),
            memory_limit_mb: self.memory_limit_mb,
            cpu_shares: self.cpu_shares,
            network_allowlist: allowlist,
            image: self.image.clone(),
            auto_pull_image: self.auto_pull_image,
            proxy_port: 0, // Auto-assign
        }
    }
}

/// Claude Code sandbox configuration.
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    /// Whether Claude Code sandbox mode is available.
    pub enabled: bool,
    /// Host directory containing Claude auth config (not mounted into containers;
    /// auth is handled via ANTHROPIC_API_KEY env var instead).
    pub config_dir: std::path::PathBuf,
    /// Claude model to use (e.g. "sonnet", "opus").
    pub model: String,
    /// Maximum agentic turns before stopping.
    pub max_turns: u32,
    /// Memory limit in MB for Claude Code containers (heavier than workers).
    pub memory_limit_mb: u64,
    /// Allowed tool patterns for Claude Code permission settings.
    ///
    /// Written to `/workspace/.claude/settings.json` before spawning the CLI.
    /// Provides defense-in-depth: only explicitly listed tools are auto-approved.
    /// Any new/unknown tools would require interactive approval (which times out
    /// in the non-interactive container, failing safely).
    ///
    /// Patterns follow Claude Code syntax: `"Bash(*)"`, `"Read"`, `"Edit(*)"`, etc.
    pub allowed_tools: Vec<String>,
}

/// Default allowed tools for Claude Code inside containers.
///
/// These cover all standard Claude Code tools needed for autonomous operation.
/// The Docker container provides the primary security boundary; this allowlist
/// provides defense-in-depth by preventing any future unknown tools from being
/// silently auto-approved.
fn default_claude_code_allowed_tools() -> Vec<String> {
    [
        // File system -- glob patterns match Claude Code's settings.json format
        "Read(*)",
        "Write(*)",
        "Edit(*)",
        "Glob(*)",
        "Grep(*)",
        "NotebookEdit(*)",
        // Execution
        "Bash(*)",
        "Task(*)",
        // Network
        "WebFetch(*)",
        "WebSearch(*)",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            config_dir: dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".claude"),
            model: "sonnet".to_string(),
            max_turns: 50,
            memory_limit_mb: 4096,
            allowed_tools: default_claude_code_allowed_tools(),
        }
    }
}

impl ClaudeCodeConfig {
    /// Load from environment variables only (used inside containers where
    /// there is no database or full config).
    pub fn from_env() -> Self {
        match Self::resolve() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to resolve ClaudeCodeConfig: {e}, using defaults");
                Self::default()
            }
        }
    }

    /// Extract the OAuth access token from the host's credential store.
    ///
    /// On macOS: reads from Keychain (`Claude Code-credentials` service).
    /// On Linux: reads from `~/.claude/.credentials.json`.
    ///
    /// Returns the access token if found. The token typically expires in
    /// 8-12 hours, which is sufficient for any single container job.
    pub fn extract_oauth_token() -> Option<String> {
        // macOS: extract from Keychain
        if cfg!(target_os = "macos") {
            match std::process::Command::new("security")
                .args([
                    "find-generic-password",
                    "-s",
                    "Claude Code-credentials",
                    "-w",
                ])
                .output()
            {
                Ok(output) if output.status.success() => {
                    if let Ok(json) = String::from_utf8(output.stdout) {
                        return parse_oauth_access_token(json.trim());
                    }
                }
                Ok(_) => {
                    tracing::debug!("No Claude Code credentials in macOS Keychain");
                }
                Err(e) => {
                    tracing::debug!("Failed to query macOS Keychain: {e}");
                }
            }
        }

        // Linux / fallback: read from ~/.claude/.credentials.json
        if let Some(home) = dirs::home_dir() {
            let creds_path = home.join(".claude").join(".credentials.json");
            if let Ok(json) = std::fs::read_to_string(&creds_path) {
                return parse_oauth_access_token(&json);
            }
        }

        None
    }

    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let defaults = Self::default();
        Ok(Self {
            enabled: optional_env("CLAUDE_CODE_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "CLAUDE_CODE_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(defaults.enabled),
            config_dir: optional_env("CLAUDE_CONFIG_DIR")?
                .map(std::path::PathBuf::from)
                .unwrap_or(defaults.config_dir),
            model: optional_env("CLAUDE_CODE_MODEL")?.unwrap_or(defaults.model),
            max_turns: parse_optional_env("CLAUDE_CODE_MAX_TURNS", defaults.max_turns)?,
            memory_limit_mb: parse_optional_env(
                "CLAUDE_CODE_MEMORY_LIMIT_MB",
                defaults.memory_limit_mb,
            )?,
            allowed_tools: optional_env("CLAUDE_CODE_ALLOWED_TOOLS")?
                .map(|s| {
                    s.split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or(defaults.allowed_tools),
        })
    }
}

/// Parse the OAuth access token from a Claude Code credentials JSON blob.
///
/// Expected shape: `{"claudeAiOauth": {"accessToken": "sk-ant-oat01-..."}}`
fn parse_oauth_access_token(json: &str) -> Option<String> {
    let creds: serde_json::Value = serde_json::from_str(json).ok()?;
    creds["claudeAiOauth"]["accessToken"]
        .as_str()
        .map(String::from)
}
