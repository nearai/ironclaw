use std::time::Duration;

use crate::config::helpers::optional_env;
use crate::error::ConfigError;
use crate::settings::Settings;

/// Agent behavior configuration.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub max_parallel_jobs: usize,
    pub job_timeout: Duration,
    pub stuck_threshold: Duration,
    pub repair_check_interval: Duration,
    pub max_repair_attempts: u32,
    /// Whether to use planning before tool execution.
    pub use_planning: bool,
    /// Session idle timeout. Sessions inactive longer than this are pruned.
    pub session_idle_timeout: Duration,
    /// Allow chat to use filesystem/shell tools directly (bypass sandbox).
    pub allow_local_tools: bool,
    /// Maximum daily LLM spend in cents (e.g. 10000 = $100). None = unlimited.
    pub max_cost_per_day_cents: Option<u64>,
    /// Maximum LLM/tool actions per hour. None = unlimited.
    pub max_actions_per_hour: Option<u64>,
}

impl AgentConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            name: optional_env("AGENT_NAME")?.unwrap_or_else(|| settings.agent.name.clone()),
            max_parallel_jobs: optional_env("AGENT_MAX_PARALLEL_JOBS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "AGENT_MAX_PARALLEL_JOBS".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?
                .unwrap_or(settings.agent.max_parallel_jobs as usize),
            job_timeout: Duration::from_secs(
                optional_env("AGENT_JOB_TIMEOUT_SECS")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "AGENT_JOB_TIMEOUT_SECS".to_string(),
                        message: format!("must be a positive integer: {e}"),
                    })?
                    .unwrap_or(settings.agent.job_timeout_secs),
            ),
            stuck_threshold: Duration::from_secs(
                optional_env("AGENT_STUCK_THRESHOLD_SECS")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "AGENT_STUCK_THRESHOLD_SECS".to_string(),
                        message: format!("must be a positive integer: {e}"),
                    })?
                    .unwrap_or(settings.agent.stuck_threshold_secs),
            ),
            repair_check_interval: Duration::from_secs(
                optional_env("SELF_REPAIR_CHECK_INTERVAL_SECS")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "SELF_REPAIR_CHECK_INTERVAL_SECS".to_string(),
                        message: format!("must be a positive integer: {e}"),
                    })?
                    .unwrap_or(settings.agent.repair_check_interval_secs),
            ),
            max_repair_attempts: optional_env("SELF_REPAIR_MAX_ATTEMPTS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "SELF_REPAIR_MAX_ATTEMPTS".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?
                .unwrap_or(settings.agent.max_repair_attempts),
            use_planning: optional_env("AGENT_USE_PLANNING")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "AGENT_USE_PLANNING".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(settings.agent.use_planning),
            session_idle_timeout: Duration::from_secs(
                optional_env("SESSION_IDLE_TIMEOUT_SECS")?
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e| ConfigError::InvalidValue {
                        key: "SESSION_IDLE_TIMEOUT_SECS".to_string(),
                        message: format!("must be a positive integer: {e}"),
                    })?
                    .unwrap_or(settings.agent.session_idle_timeout_secs),
            ),
            allow_local_tools: optional_env("ALLOW_LOCAL_TOOLS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "ALLOW_LOCAL_TOOLS".to_string(),
                    message: format!("must be 'true' or 'false': {e}"),
                })?
                .unwrap_or(false),
            max_cost_per_day_cents: optional_env("MAX_COST_PER_DAY_CENTS")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "MAX_COST_PER_DAY_CENTS".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?,
            max_actions_per_hour: optional_env("MAX_ACTIONS_PER_HOUR")?
                .map(|s| s.parse())
                .transpose()
                .map_err(|e| ConfigError::InvalidValue {
                    key: "MAX_ACTIONS_PER_HOUR".to_string(),
                    message: format!("must be a positive integer: {e}"),
                })?,
        })
    }
}
