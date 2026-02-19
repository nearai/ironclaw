use crate::config::helpers::{optional_env, parse_optional_env};
use crate::error::ConfigError;

/// Routines configuration.
#[derive(Debug, Clone)]
pub struct RoutineConfig {
    /// Whether the routines system is enabled.
    pub enabled: bool,
    /// How often (seconds) to poll for cron routines that need firing.
    pub cron_check_interval_secs: u64,
    /// Max routines executing concurrently across all users.
    pub max_concurrent_routines: usize,
    /// Default cooldown between fires (seconds).
    pub default_cooldown_secs: u64,
    /// Max output tokens for lightweight routine LLM calls.
    pub max_lightweight_tokens: u32,
}

impl Default for RoutineConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cron_check_interval_secs: 15,
            max_concurrent_routines: 10,
            default_cooldown_secs: 300,
            max_lightweight_tokens: 4096,
        }
    }
}

impl RoutineConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: optional_env("ROUTINES_ENABLED")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "ROUTINES_ENABLED".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(true),
            cron_check_interval_secs: parse_optional_env("ROUTINES_CRON_INTERVAL", 15)?,
            max_concurrent_routines: parse_optional_env("ROUTINES_MAX_CONCURRENT", 10)?,
            default_cooldown_secs: parse_optional_env("ROUTINES_DEFAULT_COOLDOWN", 300)?,
            max_lightweight_tokens: parse_optional_env("ROUTINES_MAX_TOKENS", 4096)?,
        })
    }
}
