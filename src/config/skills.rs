use std::path::PathBuf;

use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Skills system configuration.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Whether the skills system is enabled.
    pub enabled: bool,
    /// Whether the public ClawHub registry is accessible.
    pub clawhub_enabled: bool,
    /// Directory containing user-placed skills (default: ~/.ironclaw/skills/).
    /// Skills here are loaded with `Trusted` trust level.
    pub local_dir: PathBuf,
    /// Directory containing registry-installed skills (default: ~/.ironclaw/installed_skills/).
    /// Skills here are loaded with `Installed` trust level and get read-only tool access.
    pub installed_dir: PathBuf,
    /// Maximum number of skills that can be active simultaneously.
    pub max_active_skills: usize,
    /// Maximum total context tokens allocated to skill prompts.
    pub max_context_tokens: usize,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            clawhub_enabled: true,
            local_dir: default_skills_dir(),
            installed_dir: default_installed_skills_dir(),
            max_active_skills: 3,
            max_context_tokens: 4000,
        }
    }
}

/// Get the default user skills directory (~/.ironclaw/skills/).
fn default_skills_dir() -> PathBuf {
    ironclaw_base_dir().join("skills")
}

/// Get the default installed skills directory (~/.ironclaw/installed_skills/).
fn default_installed_skills_dir() -> PathBuf {
    ironclaw_base_dir().join("installed_skills")
}

impl SkillsConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("SKILLS_ENABLED", true)?,
            clawhub_enabled: parse_bool_env("CLAWHUB_ENABLED", true)?,
            local_dir: optional_env("SKILLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_skills_dir),
            installed_dir: optional_env("SKILLS_INSTALLED_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_installed_skills_dir),
            max_active_skills: parse_optional_env("SKILLS_MAX_ACTIVE", 3)?,
            max_context_tokens: parse_optional_env("SKILLS_MAX_CONTEXT_TOKENS", 4000)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn clawhub_enabled_defaults_true() {
        let _guard = lock_env();
        unsafe {
            std::env::remove_var("CLAWHUB_ENABLED");
        }
        let config = SkillsConfig::resolve().unwrap();
        assert!(config.clawhub_enabled);
    }

    #[test]
    fn clawhub_enabled_false_from_env() {
        let _guard = lock_env();
        unsafe {
            std::env::set_var("CLAWHUB_ENABLED", "false");
        }
        let config = SkillsConfig::resolve().unwrap();
        assert!(!config.clawhub_enabled);
        unsafe {
            std::env::remove_var("CLAWHUB_ENABLED");
        }
    }

    #[test]
    fn clawhub_enabled_rejects_invalid() {
        let _guard = lock_env();
        unsafe {
            std::env::set_var("CLAWHUB_ENABLED", "maybe");
        }
        let result = SkillsConfig::resolve();
        assert!(result.is_err());
        unsafe {
            std::env::remove_var("CLAWHUB_ENABLED");
        }
    }
}
