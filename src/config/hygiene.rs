use crate::config::helpers::optional_env;
use crate::error::ConfigError;

/// Memory hygiene configuration.
///
/// Controls automatic cleanup of stale workspace documents.
/// Maps to `crate::workspace::hygiene::HygieneConfig`.
#[derive(Debug, Clone)]
pub struct HygieneConfig {
    /// Whether hygiene is enabled. Env: `MEMORY_HYGIENE_ENABLED` (default: true).
    pub enabled: bool,
    /// Days before `daily/` documents are deleted. Env: `MEMORY_HYGIENE_RETENTION_DAYS` (default: 30).
    pub retention_days: u32,
    /// Minimum hours between hygiene passes. Env: `MEMORY_HYGIENE_CADENCE_HOURS` (default: 12).
    pub cadence_hours: u32,
}

impl Default for HygieneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 30,
            cadence_hours: 12,
        }
    }
}

impl HygieneConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: optional_env("MEMORY_HYGIENE_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "MEMORY_HYGIENE_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
            retention_days: optional_env("MEMORY_HYGIENE_RETENTION_DAYS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "MEMORY_HYGIENE_RETENTION_DAYS".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?
                .unwrap_or(30),
            cadence_hours: optional_env("MEMORY_HYGIENE_CADENCE_HOURS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "MEMORY_HYGIENE_CADENCE_HOURS".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?
                .unwrap_or(12),
        })
    }

    /// Convert to the workspace hygiene config, resolving the state directory
    /// to the standard `~/.ironclaw` location.
    pub fn to_workspace_config(&self) -> crate::workspace::hygiene::HygieneConfig {
        crate::workspace::hygiene::HygieneConfig {
            enabled: self.enabled,
            retention_days: self.retention_days,
            cadence_hours: self.cadence_hours,
            state_dir: dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".ironclaw"),
        }
    }
}
