use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{db_first_bool, db_first_or_default};
use crate::error::ConfigError;
use crate::settings::Settings;

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
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = crate::settings::HygieneSettings::default();
        let hs = &settings.hygiene;

        Ok(Self {
            enabled: db_first_bool(hs.enabled, defaults.enabled, "MEMORY_HYGIENE_ENABLED")?,
            daily_retention_days: db_first_or_default(
                &hs.daily_retention_days,
                &defaults.daily_retention_days,
                "MEMORY_HYGIENE_DAILY_RETENTION_DAYS",
            )?,
            conversation_retention_days: db_first_or_default(
                &hs.conversation_retention_days,
                &defaults.conversation_retention_days,
                "MEMORY_HYGIENE_CONVERSATION_RETENTION_DAYS",
            )?,
            cadence_hours: db_first_or_default(
                &hs.cadence_hours,
                &defaults.cadence_hours,
                "MEMORY_HYGIENE_CADENCE_HOURS",
            )?,
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
