use std::path::PathBuf;

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Skills system configuration.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Whether the skills system is enabled.
    pub enabled: bool,
    /// Directory containing local skills (default: ~/.ironclaw/skills/).
    pub local_dir: PathBuf,
    /// Maximum number of skills that can be active simultaneously.
    pub max_active_skills: usize,
    /// Maximum total context tokens allocated to skill prompts.
    pub max_context_tokens: usize,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            local_dir: default_skills_dir(),
            max_active_skills: 3,
            max_context_tokens: 4000,
        }
    }
}

/// Get the default skills directory (~/.ironclaw/skills/).
fn default_skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("skills")
}

impl SkillsConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("SKILLS_ENABLED", true)?,
            local_dir: optional_env("SKILLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_skills_dir),
            max_active_skills: parse_optional_env("SKILLS_MAX_ACTIVE", 3)?,
            max_context_tokens: parse_optional_env("SKILLS_MAX_CONTEXT_TOKENS", 4000)?,
        })
    }
}
