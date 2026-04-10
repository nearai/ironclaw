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
    /// Maximum versions to keep per document. Env: `MEMORY_HYGIENE_VERSION_KEEP_COUNT` (default: 50).
    pub version_keep_count: u32,
    /// Minimum hours between hygiene passes. Env: `MEMORY_HYGIENE_CADENCE_HOURS` (default: 12).
    pub cadence_hours: u32,
}

impl Default for HygieneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            version_keep_count: 50,
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
            version_keep_count: db_first_or_default(
                &hs.version_keep_count,
                &defaults.version_keep_count,
                "MEMORY_HYGIENE_VERSION_KEEP_COUNT",
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
            version_keep_count: self.version_keep_count,
            cadence_hours: self.cadence_hours,
            state_dir: ironclaw_base_dir(),
        }
    }
}
