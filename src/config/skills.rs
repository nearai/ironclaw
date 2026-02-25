use std::path::PathBuf;

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Skills system configuration.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Whether the skills system is enabled.
    pub enabled: bool,
    /// Directory containing user-placed skills (default: ~/.clawyer/skills/).
    /// Skills here are loaded with `Trusted` trust level.
    pub local_dir: PathBuf,
    /// Directory containing registry-installed skills (default: ~/.clawyer/installed_skills/).
    /// Skills here are loaded with `Installed` trust level and get read-only tool access.
    pub installed_dir: PathBuf,
    /// Directory containing bundled first-party skills shipped with the app.
    pub bundled_dir: PathBuf,
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
            installed_dir: default_installed_skills_dir(),
            bundled_dir: default_bundled_skills_dir(),
            max_active_skills: 3,
            max_context_tokens: 4000,
        }
    }
}

/// Get the default user skills directory (~/.clawyer/skills/).
fn default_skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".clawyer")
        .join("skills")
}

/// Get the default installed skills directory (~/.clawyer/installed_skills/).
fn default_installed_skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".clawyer")
        .join("installed_skills")
}

/// Get the default bundled skills directory (<repo>/skills/bundled in dev builds).
fn default_bundled_skills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("skills")
        .join("bundled")
}

impl SkillsConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("SKILLS_ENABLED", true)?,
            local_dir: optional_env("SKILLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_skills_dir),
            installed_dir: optional_env("SKILLS_INSTALLED_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_installed_skills_dir),
            bundled_dir: optional_env("SKILLS_BUNDLED_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_bundled_skills_dir),
            max_active_skills: parse_optional_env("SKILLS_MAX_ACTIVE", 3)?,
            max_context_tokens: parse_optional_env("SKILLS_MAX_CONTEXT_TOKENS", 4000)?,
        })
    }
}
