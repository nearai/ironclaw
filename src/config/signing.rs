//! Configuration for cryptographic tool call signing.

use crate::config::helpers::{optional_env, parse_bool_env};
use crate::error::ConfigError;

/// Configuration for cryptographic tool call signing via signet-core.
///
/// Env-only (not settable via DB/TOML) because signing is a security
/// boundary — the operator must explicitly opt out, not a per-user setting.
#[derive(Debug, Clone)]
pub struct SigningConfig {
    /// Master switch. Env: `SIGNING_ENABLED` (default: `true`).
    pub enabled: bool,

    /// Tools to skip signing. Env: `SIGNING_SKIP_TOOLS` (comma-separated).
    /// Example: `echo,time,json_parse`
    pub skip_tools: Vec<String>,
}

impl Default for SigningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            skip_tools: Vec::new(),
        }
    }
}

impl SigningConfig {
    /// Resolve from environment variables.
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let enabled = parse_bool_env("SIGNING_ENABLED", true)?;

        let skip_tools = optional_env("SIGNING_SKIP_TOOLS")?
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            enabled,
            skip_tools,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn test_default_config_signs_everything() {
        let config = SigningConfig::default();
        assert!(config.enabled);
        assert!(config.skip_tools.is_empty());
    }

    #[test]
    fn test_resolve_defaults_to_enabled() {
        let _guard = lock_env();
        // Ensure env vars are unset
        // SAFETY: under ENV_MUTEX
        unsafe {
            std::env::remove_var("SIGNING_ENABLED");
            std::env::remove_var("SIGNING_SKIP_TOOLS");
        }

        let config = SigningConfig::resolve().expect("resolve");
        assert!(config.enabled);
        assert!(config.skip_tools.is_empty());

        // No cleanup needed — vars were already removed
    }

    #[test]
    fn test_resolve_disabled_via_env() {
        let _guard = lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("SIGNING_ENABLED", "false") };

        let config = SigningConfig::resolve().expect("resolve");
        assert!(!config.enabled);

        unsafe { std::env::remove_var("SIGNING_ENABLED") };
    }

    #[test]
    fn test_resolve_skip_tools_from_env() {
        let _guard = lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("SIGNING_SKIP_TOOLS", "echo, time, json_parse") };

        let config = SigningConfig::resolve().expect("resolve");
        assert_eq!(config.skip_tools, vec!["echo", "time", "json_parse"]);

        unsafe { std::env::remove_var("SIGNING_SKIP_TOOLS") };
    }

    #[test]
    fn test_resolve_skip_tools_filters_empty_entries() {
        let _guard = lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("SIGNING_SKIP_TOOLS", "echo,,time,") };

        let config = SigningConfig::resolve().expect("resolve");
        assert_eq!(config.skip_tools, vec!["echo", "time"]);

        unsafe { std::env::remove_var("SIGNING_SKIP_TOOLS") };
    }
}
