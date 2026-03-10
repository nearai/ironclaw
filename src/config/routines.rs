use crate::config::helpers::{parse_bool_env, parse_optional_env};
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
    /// Enable tool execution in lightweight routines (default: true).
    pub lightweight_tools_enabled: bool,
    /// Max tool iterations for lightweight routines (default: 3, max: 5).
    pub lightweight_max_iterations: u32,
}

impl Default for RoutineConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cron_check_interval_secs: 15,
            max_concurrent_routines: 10,
            default_cooldown_secs: 300,
            max_lightweight_tokens: 4096,
            lightweight_tools_enabled: true,
            lightweight_max_iterations: 3,
        }
    }
}

impl RoutineConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let max_iterations: u32 = parse_optional_env("ROUTINES_LIGHTWEIGHT_MAX_ITERATIONS", 3)?;
        Ok(Self {
            enabled: parse_bool_env("ROUTINES_ENABLED", true)?,
            cron_check_interval_secs: parse_optional_env("ROUTINES_CRON_INTERVAL", 15)?,
            max_concurrent_routines: parse_optional_env("ROUTINES_MAX_CONCURRENT", 10)?,
            default_cooldown_secs: parse_optional_env("ROUTINES_DEFAULT_COOLDOWN", 300)?,
            max_lightweight_tokens: parse_optional_env("ROUTINES_MAX_TOKENS", 4096)?,
            lightweight_tools_enabled: parse_bool_env("ROUTINES_LIGHTWEIGHT_TOOLS", true)?,
            lightweight_max_iterations: max_iterations.min(5), // cap at 5
        })
    }
}
