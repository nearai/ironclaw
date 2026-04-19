use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;

/// Memory hygiene configuration.
///
/// Controls automatic cleanup of stale workspace documents.
/// Maps to `crate::workspace::hygiene::HygieneConfig`.
#[derive(Debug, Clone)]
pub struct HygieneConfig {
    /// Whether hygiene is enabled. Env: `MEMORY_HYGIENE_ENABLED` (default: true).
    pub enabled: bool,
    /// Days before `daily/` documents are deleted. Env: `MEMORY_HYGIENE_DAILY_RETENTION_DAYS` (default: 30).
    pub daily_retention_days: u32,
    /// Days before `conversations/` documents are deleted. Env: `MEMORY_HYGIENE_CONVERSATION_RETENTION_DAYS` (default: 7).
    pub conversation_retention_days: u32,
    /// Minimum hours between hygiene passes. Env: `MEMORY_HYGIENE_CADENCE_HOURS` (default: 12).
    pub cadence_hours: u32,
}

impl Default for HygieneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            daily_retention_days: 30,
            conversation_retention_days: 7,
            cadence_hours: 12,
        }
    }
}

impl HygieneConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("MEMORY_HYGIENE_ENABLED", true)?,
            daily_retention_days: parse_optional_env("MEMORY_HYGIENE_DAILY_RETENTION_DAYS", 30)?,
            conversation_retention_days: parse_optional_env(
                "MEMORY_HYGIENE_CONVERSATION_RETENTION_DAYS",
                7,
            )?,
            cadence_hours: parse_optional_env("MEMORY_HYGIENE_CADENCE_HOURS", 12)?,
        })
    }

    /// Convert to the workspace hygiene config, resolving the state directory
    /// to the standard `~/.ironclaw` location.
    pub fn to_workspace_config(&self) -> crate::workspace::hygiene::HygieneConfig {
        crate::workspace::hygiene::HygieneConfig {
            enabled: self.enabled,
            daily_retention_days: self.daily_retention_days,
            conversation_retention_days: self.conversation_retention_days,
            cadence_hours: self.cadence_hours,
            state_dir: ironclaw_base_dir(),
        }
    }
}
