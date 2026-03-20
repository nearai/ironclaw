use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env, parse_string_env};
use crate::error::ConfigError;

/// Docker sandbox configuration.
#[derive(Debug, Clone)]
pub struct SandboxModeConfig {
    /// Whether the Docker sandbox is enabled.
    pub enabled: bool,
    /// Sandbox policy: "readonly", "workspace_write", or "full_access".
    pub policy: String,
    /// Explicit opt-in for `FullAccess` policy.
    ///
    /// When `policy` is `full_access` but this is `false`, the policy is
    /// downgraded to `workspace_write` with a loud error log. This prevents
    /// accidental host-level command execution from a single misconfigured
    /// env var.
    pub allow_full_access: bool,
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
    /// How often the reaper scans for orphaned containers (seconds). Default: 300 (5 min).
    pub reaper_interval_secs: u64,
    /// Containers older than this with no active job are reaped (seconds). Default: 600 (10 min).
    pub orphan_threshold_secs: u64,
}

impl Default for SandboxModeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: "readonly".to_string(),
            allow_full_access: false,
            timeout_secs: 120,
            memory_limit_mb: 2048,
            cpu_shares: 1024,
            image: "ironclaw-worker:latest".to_string(),
            auto_pull_image: true,
            extra_allowed_domains: Vec::new(),
            reaper_interval_secs: 300,
            orphan_threshold_secs: 600,
        }
    }
}

impl SandboxModeConfig {
    pub(crate) fn resolve(settings: &crate::settings::Settings) -> Result<Self, ConfigError> {
        let ss = &settings.sandbox;

        let extra_domains = optional_env("SANDBOX_EXTRA_DOMAINS")?
            .map(|s| s.split(',').map(|d| d.trim().to_string()).collect())
            .unwrap_or_else(|| {
                if ss.extra_allowed_domains.is_empty() {
                    Vec::new()
                } else {
                    ss.extra_allowed_domains.clone()
                }
            });

        // reaper/orphan fields have no Settings counterpart — env > default only.
        let reaper_interval_secs: u64 = parse_optional_env("SANDBOX_REAPER_INTERVAL_SECS", 300)?;
        let orphan_threshold_secs: u64 = parse_optional_env("SANDBOX_ORPHAN_THRESHOLD_SECS", 600)?;

        // Validate that reaper timings are non-zero to prevent tokio::time::interval panics
        if reaper_interval_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "SANDBOX_REAPER_INTERVAL_SECS".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }

        if orphan_threshold_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "SANDBOX_ORPHAN_THRESHOLD_SECS".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }

        Ok(Self {
            enabled: parse_bool_env("SANDBOX_ENABLED", ss.enabled)?,
            policy: parse_string_env("SANDBOX_POLICY", ss.policy.clone())?,
            // allow_full_access has no Settings counterpart — env > default only.
            allow_full_access: parse_bool_env("SANDBOX_ALLOW_FULL_ACCESS", false)?,
            timeout_secs: parse_optional_env("SANDBOX_TIMEOUT_SECS", ss.timeout_secs)?,
            memory_limit_mb: parse_optional_env("SANDBOX_MEMORY_LIMIT_MB", ss.memory_limit_mb)?,
            cpu_shares: parse_optional_env("SANDBOX_CPU_SHARES", ss.cpu_shares)?,
            image: parse_string_env("SANDBOX_IMAGE", ss.image.clone())?,
            auto_pull_image: parse_bool_env("SANDBOX_AUTO_PULL", ss.auto_pull_image)?,
            extra_allowed_domains: extra_domains,
            reaper_interval_secs,
            orphan_threshold_secs,
        })
    }

    /// Convert to SandboxConfig for the sandbox module.
    ///
    /// If `policy` is `FullAccess` but `allow_full_access` is `false`,
    /// the policy is downgraded to `WorkspaceWrite` and an error is logged.
    pub fn to_sandbox_config(&self) -> crate::sandbox::SandboxConfig {
        use crate::sandbox::SandboxPolicy;
        use std::time::Duration;

        let mut policy = self.policy.parse().unwrap_or(SandboxPolicy::ReadOnly);

        // Double opt-in guard: FullAccess requires SANDBOX_ALLOW_FULL_ACCESS=true
        if policy == SandboxPolicy::FullAccess && !self.allow_full_access {
            tracing::error!(
                "SANDBOX_POLICY=full_access is set but SANDBOX_ALLOW_FULL_ACCESS is not \
                 set to 'true'. FullAccess bypasses Docker and runs commands directly on \
                 the host. Downgrading to WorkspaceWrite for safety. Set \
                 SANDBOX_ALLOW_FULL_ACCESS=true to explicitly enable FullAccess."
            );
            policy = SandboxPolicy::WorkspaceWrite;
        }

        let mut allowlist = crate::sandbox::default_allowlist();
        allowlist.extend(self.extra_allowed_domains.clone());

        crate::sandbox::SandboxConfig {
            enabled: self.enabled,
            policy,
            allow_full_access: self.allow_full_access,
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
        match Self::resolve_env_only() {
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

    pub(crate) fn resolve(settings: &crate::settings::Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        Ok(Self {
            // Use settings.sandbox.claude_code_enabled as fallback (written by setup wizard).
            enabled: parse_bool_env("CLAUDE_CODE_ENABLED", settings.sandbox.claude_code_enabled)?,
            config_dir: optional_env("CLAUDE_CONFIG_DIR")?
                .map(std::path::PathBuf::from)
                .unwrap_or(defaults.config_dir),
            model: parse_string_env("CLAUDE_CODE_MODEL", defaults.model)?,
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

    /// Resolve from env vars only, no Settings. Used inside containers.
    fn resolve_env_only() -> Result<Self, ConfigError> {
        let defaults = Self::default();
        Ok(Self {
            enabled: parse_bool_env("CLAUDE_CODE_ENABLED", defaults.enabled)?,
            config_dir: optional_env("CLAUDE_CONFIG_DIR")?
                .map(std::path::PathBuf::from)
                .unwrap_or(defaults.config_dir),
            model: parse_string_env("CLAUDE_CODE_MODEL", defaults.model)?,
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
    let token = creds["claudeAiOauth"]["accessToken"].as_str()?;
    // Validate that the token looks like a real OAuth token before using it.
    // Claude CLI tokens start with "sk-ant-oat".
    if !token.starts_with("sk-ant-oat") {
        tracing::debug!("Ignoring credential store token with unexpected prefix");
        return None;
    }
    Some(token.to_string())
}

#[cfg(test)]
mod tests {
    use crate::config::sandbox::*;
    use crate::testing::credentials::*;

    // ── SandboxModeConfig defaults ──────────────────────────────────

    #[test]
    fn sandbox_mode_config_default_values() {
        let cfg = SandboxModeConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.policy, "readonly");
        assert_eq!(cfg.timeout_secs, 120);
        assert_eq!(cfg.memory_limit_mb, 2048);
        assert_eq!(cfg.cpu_shares, 1024);
        assert_eq!(cfg.image, "ironclaw-worker:latest");
        assert!(cfg.auto_pull_image);
        assert!(cfg.extra_allowed_domains.is_empty());
    }

    #[test]
    fn sandbox_mode_config_custom_values() {
        let cfg = SandboxModeConfig {
            enabled: false,
            policy: "full_access".to_string(),
            timeout_secs: 600,
            memory_limit_mb: 4096,
            cpu_shares: 512,
            image: "custom-worker:v2".to_string(),
            auto_pull_image: false,
            extra_allowed_domains: vec!["example.com".to_string()],
            reaper_interval_secs: 300,
            orphan_threshold_secs: 600,
            allow_full_access: false,
        };
        assert!(!cfg.enabled);
        assert_eq!(cfg.policy, "full_access");
        assert_eq!(cfg.timeout_secs, 600);
        assert_eq!(cfg.memory_limit_mb, 4096);
        assert_eq!(cfg.cpu_shares, 512);
        assert_eq!(cfg.image, "custom-worker:v2");
        assert!(!cfg.auto_pull_image);
        assert_eq!(cfg.extra_allowed_domains, vec!["example.com"]);
    }

    #[test]
    fn sandbox_mode_to_sandbox_config_propagates_fields() {
        let mode = SandboxModeConfig {
            enabled: true,
            policy: "workspace_write".to_string(),
            timeout_secs: 300,
            memory_limit_mb: 1024,
            cpu_shares: 2048,
            image: "test:latest".to_string(),
            auto_pull_image: false,
            extra_allowed_domains: vec!["custom.example.com".to_string()],
            reaper_interval_secs: 300,
            orphan_threshold_secs: 600,
            allow_full_access: false,
        };
        let sc = mode.to_sandbox_config();
        assert!(sc.enabled);
        assert_eq!(sc.policy, crate::sandbox::SandboxPolicy::WorkspaceWrite);
        assert_eq!(sc.timeout, std::time::Duration::from_secs(300));
        assert_eq!(sc.memory_limit_mb, 1024);
        assert_eq!(sc.cpu_shares, 2048);
        assert_eq!(sc.image, "test:latest");
        assert!(!sc.auto_pull_image);
        // extra domain should be in the allowlist
        assert!(
            sc.network_allowlist
                .contains(&"custom.example.com".to_string()),
            "expected custom domain in allowlist"
        );
    }

    #[test]
    fn sandbox_mode_to_sandbox_config_invalid_policy_falls_back_to_readonly() {
        let mode = SandboxModeConfig {
            policy: "garbage_value".to_string(),
            ..SandboxModeConfig::default()
        };
        let sc = mode.to_sandbox_config();
        assert_eq!(sc.policy, crate::sandbox::SandboxPolicy::ReadOnly);
    }

    #[test]
    fn sandbox_mode_to_sandbox_config_includes_default_allowlist() {
        let mode = SandboxModeConfig::default();
        let sc = mode.to_sandbox_config();
        // The default allowlist from sandbox module should be non-empty
        assert!(
            !sc.network_allowlist.is_empty(),
            "default allowlist should not be empty"
        );
    }

    // ── ClaudeCodeConfig defaults ───────────────────────────────────

    #[test]
    fn claude_code_config_default_values() {
        let cfg = ClaudeCodeConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.model, "sonnet");
        assert_eq!(cfg.max_turns, 50);
        assert_eq!(cfg.memory_limit_mb, 4096);
        assert!(cfg.config_dir.ends_with(".claude"));
        // Should have all the standard tools
        assert!(!cfg.allowed_tools.is_empty());
        assert!(cfg.allowed_tools.contains(&"Bash(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"Read(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"Edit(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"Write(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"Grep(*)".to_string()));
        assert!(cfg.allowed_tools.contains(&"WebFetch(*)".to_string()));
    }

    #[test]
    fn claude_code_config_custom_values() {
        let cfg = ClaudeCodeConfig {
            enabled: true,
            config_dir: std::path::PathBuf::from("/opt/claude"),
            model: "opus".to_string(),
            max_turns: 100,
            memory_limit_mb: 8192,
            allowed_tools: vec!["Read(*)".to_string(), "Bash(*)".to_string()],
        };
        assert!(cfg.enabled);
        assert_eq!(cfg.config_dir, std::path::PathBuf::from("/opt/claude"));
        assert_eq!(cfg.model, "opus");
        assert_eq!(cfg.max_turns, 100);
        assert_eq!(cfg.memory_limit_mb, 8192);
        assert_eq!(cfg.allowed_tools.len(), 2);
    }

    // ── parse_oauth_access_token ────────────────────────────────────

    #[test]
    fn parse_oauth_token_valid() {
        let json = format!(
            r#"{{"claudeAiOauth": {{"accessToken": "{}"}}}}"#,
            TEST_ANTHROPIC_OAUTH_BASIC
        );
        let token = parse_oauth_access_token(&json);
        assert_eq!(token, Some(TEST_ANTHROPIC_OAUTH_BASIC.to_string()));
    }

    #[test]
    fn parse_oauth_token_missing_access_token() {
        let json = r#"{"claudeAiOauth": {}}"#;
        assert_eq!(parse_oauth_access_token(json), None);
    }

    #[test]
    fn parse_oauth_token_missing_oauth_key() {
        let json = r#"{"someOtherKey": {"accessToken": "tok"}}"#;
        assert_eq!(parse_oauth_access_token(json), None);
    }

    #[test]
    fn parse_oauth_token_invalid_json() {
        assert_eq!(parse_oauth_access_token("not json at all"), None);
    }

    #[test]
    fn parse_oauth_token_empty_string() {
        assert_eq!(parse_oauth_access_token(""), None);
    }

    #[test]
    fn parse_oauth_token_nested_extra_fields() {
        let json = format!(
            r#"{{
            "claudeAiOauth": {{
                "accessToken": "{}",
                "refreshToken": "rt-abc",
                "expiresAt": 1700000000
            }}
        }}"#,
            TEST_ANTHROPIC_OAUTH_NESTED
        );
        assert_eq!(
            parse_oauth_access_token(&json),
            Some(TEST_ANTHROPIC_OAUTH_NESTED.to_string())
        );
    }

    #[test]
    fn parse_oauth_token_access_token_is_not_string() {
        let json = r#"{"claudeAiOauth": {"accessToken": 12345}}"#;
        assert_eq!(parse_oauth_access_token(json), None);
    }

    #[test]
    fn parse_oauth_token_rejects_invalid_prefix() {
        let json = r#"{"claudeAiOauth": {"accessToken": "not-an-oauth-token"}}"#;
        assert_eq!(parse_oauth_access_token(json), None);
    }

    // ── default_claude_code_allowed_tools ───────────────────────────

    #[test]
    fn default_allowed_tools_has_expected_count() {
        let tools = default_claude_code_allowed_tools();
        // 10 tools: Read, Write, Edit, Glob, Grep, NotebookEdit, Bash, Task, WebFetch, WebSearch
        assert_eq!(tools.len(), 10);
    }

    #[test]
    fn default_allowed_tools_all_have_glob_pattern() {
        let tools = default_claude_code_allowed_tools();
        for tool in &tools {
            assert!(
                tool.ends_with("(*)"),
                "tool '{tool}' should end with '(*)' glob pattern"
            );
        }
    }

    #[test]
    fn test_full_access_downgraded_without_allow() {
        let config = SandboxModeConfig {
            policy: "full_access".to_string(),
            allow_full_access: false,
            ..Default::default()
        };
        let sandbox = config.to_sandbox_config();
        // Should have been downgraded to WorkspaceWrite
        assert_eq!(
            sandbox.policy,
            crate::sandbox::SandboxPolicy::WorkspaceWrite
        );
        assert!(!sandbox.allow_full_access);
    }

    #[test]
    fn test_full_access_allowed_with_explicit_opt_in() {
        let config = SandboxModeConfig {
            policy: "full_access".to_string(),
            allow_full_access: true,
            ..Default::default()
        };
        let sandbox = config.to_sandbox_config();
        assert_eq!(sandbox.policy, crate::sandbox::SandboxPolicy::FullAccess);
        assert!(sandbox.allow_full_access);
    }

    #[test]
    fn test_non_full_access_policy_unaffected() {
        let config = SandboxModeConfig {
            policy: "workspace_write".to_string(),
            allow_full_access: false,
            ..Default::default()
        };
        let sandbox = config.to_sandbox_config();
        assert_eq!(
            sandbox.policy,
            crate::sandbox::SandboxPolicy::WorkspaceWrite
        );
    }

    // ── Settings fallback tests ──────────────────────────────────────

    #[test]
    fn sandbox_resolve_falls_back_to_settings() {
        let _guard = crate::config::helpers::ENV_MUTEX
            .lock()
            .expect("env mutex poisoned");
        let mut settings = crate::settings::Settings::default();
        settings.sandbox.cpu_shares = 99;
        settings.sandbox.auto_pull_image = false;
        settings.sandbox.enabled = false;

        let cfg = SandboxModeConfig::resolve(&settings).expect("resolve");
        assert!(!cfg.enabled);
        assert_eq!(cfg.cpu_shares, 99);
        assert!(!cfg.auto_pull_image);
    }

    #[test]
    fn sandbox_env_overrides_settings() {
        let _guard = crate::config::helpers::ENV_MUTEX
            .lock()
            .expect("env mutex poisoned");
        let mut settings = crate::settings::Settings::default();
        settings.sandbox.timeout_secs = 999;

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("SANDBOX_TIMEOUT_SECS", "5") };
        let cfg = SandboxModeConfig::resolve(&settings).expect("resolve");
        unsafe { std::env::remove_var("SANDBOX_TIMEOUT_SECS") };

        assert_eq!(cfg.timeout_secs, 5);
    }

    // ── ClaudeCodeConfig settings fallback tests ────────────────────

    #[test]
    fn claude_code_resolve_uses_settings_enabled() {
        let _guard = crate::config::helpers::ENV_MUTEX
            .lock()
            .expect("env mutex poisoned");
        let mut settings = crate::settings::Settings::default();
        settings.sandbox.claude_code_enabled = true;

        let cfg = ClaudeCodeConfig::resolve(&settings).expect("resolve");
        assert!(cfg.enabled);
    }

    #[test]
    fn claude_code_resolve_defaults_disabled() {
        let _guard = crate::config::helpers::ENV_MUTEX
            .lock()
            .expect("env mutex poisoned");
        let settings = crate::settings::Settings::default();
        let cfg = ClaudeCodeConfig::resolve(&settings).expect("resolve");
        assert!(!cfg.enabled);
    }

    #[test]
    fn claude_code_env_overrides_settings() {
        let _guard = crate::config::helpers::ENV_MUTEX
            .lock()
            .expect("env mutex poisoned");
        let mut settings = crate::settings::Settings::default();
        settings.sandbox.claude_code_enabled = true;

        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe { std::env::set_var("CLAUDE_CODE_ENABLED", "false") };
        let cfg = ClaudeCodeConfig::resolve(&settings).expect("resolve");
        unsafe { std::env::remove_var("CLAUDE_CODE_ENABLED") };

        assert!(!cfg.enabled);
    }

    #[test]
    fn test_readonly_policy_unaffected() {
        let config = SandboxModeConfig {
            policy: "readonly".to_string(),
            allow_full_access: false,
            ..Default::default()
        };
        let sandbox = config.to_sandbox_config();
        assert_eq!(sandbox.policy, crate::sandbox::SandboxPolicy::ReadOnly);
    }
}
