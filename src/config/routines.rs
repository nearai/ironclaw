use crate::config::helpers::{db_first_bool, db_first_or_default};
use crate::error::ConfigError;
use crate::settings::Settings;

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
    /// Enable agent review for routine completions (default: true).
    pub agent_review_enabled: bool,
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
            agent_review_enabled: true,
        }
    }
}

impl RoutineConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = crate::settings::RoutineSettings::default();
        let rs = &settings.routines;

        let max_iterations: u32 = db_first_or_default(
            &rs.lightweight_max_iterations,
            &defaults.lightweight_max_iterations,
            "ROUTINES_LIGHTWEIGHT_MAX_ITERATIONS",
        )?;
        Ok(Self {
            enabled: db_first_bool(rs.enabled, defaults.enabled, "ROUTINES_ENABLED")?,
            cron_check_interval_secs: db_first_or_default(
                &rs.cron_check_interval_secs,
                &defaults.cron_check_interval_secs,
                "ROUTINES_CRON_INTERVAL",
            )?,
            max_concurrent_routines: db_first_or_default(
                &rs.max_concurrent_routines,
                &defaults.max_concurrent_routines,
                "ROUTINES_MAX_CONCURRENT",
            )?,
            default_cooldown_secs: db_first_or_default(
                &rs.default_cooldown_secs,
                &defaults.default_cooldown_secs,
                "ROUTINES_DEFAULT_COOLDOWN",
            )?,
            max_lightweight_tokens: db_first_or_default(
                &rs.max_lightweight_tokens,
                &defaults.max_lightweight_tokens,
                "ROUTINES_MAX_TOKENS",
            )?,
            lightweight_tools_enabled: db_first_bool(
                rs.lightweight_tools_enabled,
                defaults.lightweight_tools_enabled,
                "ROUTINES_LIGHTWEIGHT_TOOLS",
            )?,
            lightweight_max_iterations: max_iterations.min(5), // cap at 5
            agent_review_enabled: parse_bool_env("ROUTINE_AGENT_REVIEW_ENABLED", true)?,
        })
    }
}
