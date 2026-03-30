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
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("MEMORY_HYGIENE_ENABLED", true)?,
            version_keep_count: parse_optional_env("MEMORY_HYGIENE_VERSION_KEEP_COUNT", 50)?,
            cadence_hours: parse_optional_env("MEMORY_HYGIENE_CADENCE_HOURS", 12)?,
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
