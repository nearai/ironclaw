use std::time::Duration;

use crate::config::helpers::{optional_env, parse_bool_env, parse_option_env, parse_optional_env};
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
    /// Maximum tool-call iterations per agentic loop invocation. Default 50.
    pub max_tool_iterations: usize,
    /// When true, skip tool approval checks entirely. For benchmarks/CI.
    pub auto_approve_tools: bool,
    /// Default timezone for new sessions (IANA name, e.g. "America/New_York").
    pub default_timezone: String,
    /// Maximum tokens per job (0 = unlimited).
    pub max_tokens_per_job: u64,
    /// User IDs allowed to interact with the agent across all channels.
    /// Empty list = allow all users (backward compatible).
    pub allowed_users: Vec<String>,
}

impl AgentConfig {
    /// Create a test-friendly config without reading env vars.
    #[cfg(feature = "libsql")]
    pub fn for_testing() -> Self {
        Self {
            name: "test-rig".to_string(),
            max_parallel_jobs: 1,
            job_timeout: Duration::from_secs(30),
            stuck_threshold: Duration::from_secs(300),
            repair_check_interval: Duration::from_secs(3600),
            max_repair_attempts: 0,
            use_planning: false,
            session_idle_timeout: Duration::from_secs(3600),
            allow_local_tools: true,
            max_cost_per_day_cents: None,
            max_actions_per_hour: None,
            max_tool_iterations: 10,
            auto_approve_tools: true,
            default_timezone: "UTC".to_string(),
            max_tokens_per_job: 0,
            allowed_users: vec![],
        }
    }

    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            name: parse_optional_env("AGENT_NAME", settings.agent.name.clone())?,
            max_parallel_jobs: parse_optional_env(
                "AGENT_MAX_PARALLEL_JOBS",
                settings.agent.max_parallel_jobs as usize,
            )?,
            job_timeout: Duration::from_secs(parse_optional_env(
                "AGENT_JOB_TIMEOUT_SECS",
                settings.agent.job_timeout_secs,
            )?),
            stuck_threshold: Duration::from_secs(parse_optional_env(
                "AGENT_STUCK_THRESHOLD_SECS",
                settings.agent.stuck_threshold_secs,
            )?),
            repair_check_interval: Duration::from_secs(parse_optional_env(
                "SELF_REPAIR_CHECK_INTERVAL_SECS",
                settings.agent.repair_check_interval_secs,
            )?),
            max_repair_attempts: parse_optional_env(
                "SELF_REPAIR_MAX_ATTEMPTS",
                settings.agent.max_repair_attempts,
            )?,
            use_planning: parse_bool_env("AGENT_USE_PLANNING", settings.agent.use_planning)?,
            session_idle_timeout: Duration::from_secs(parse_optional_env(
                "SESSION_IDLE_TIMEOUT_SECS",
                settings.agent.session_idle_timeout_secs,
            )?),
            allow_local_tools: parse_bool_env("ALLOW_LOCAL_TOOLS", false)?,
            max_cost_per_day_cents: parse_option_env("MAX_COST_PER_DAY_CENTS")?,
            max_actions_per_hour: parse_option_env("MAX_ACTIONS_PER_HOUR")?,
            max_tool_iterations: parse_optional_env(
                "AGENT_MAX_TOOL_ITERATIONS",
                settings.agent.max_tool_iterations,
            )?,
            auto_approve_tools: parse_bool_env(
                "AGENT_AUTO_APPROVE_TOOLS",
                settings.agent.auto_approve_tools,
            )?,
            default_timezone: {
                let tz: String = parse_optional_env(
                    "DEFAULT_TIMEZONE",
                    settings.agent.default_timezone.clone(),
                )?;
                if crate::timezone::parse_timezone(&tz).is_none() {
                    return Err(ConfigError::InvalidValue {
                        key: "DEFAULT_TIMEZONE".into(),
                        message: format!("invalid IANA timezone: '{tz}'"),
                    });
                }
                tz
            },
            max_tokens_per_job: parse_optional_env(
                "AGENT_MAX_TOKENS_PER_JOB",
                settings.agent.max_tokens_per_job,
            )?,
            allowed_users: {
                let from_env = optional_env("ALLOWED_USERS")?
                    .map(|s| {
                        s.split(',')
                            .map(|e| e.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if from_env.is_empty() {
                    settings.agent.allowed_users.clone()
                } else {
                    from_env
                }
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::ENV_MUTEX;

    #[test]
    fn test_default_timezone_rejects_invalid() {
        let mut settings = Settings::default();
        settings.agent.default_timezone = "Fake/Zone".to_string();

        let result = AgentConfig::resolve(&settings);
        assert!(result.is_err(), "invalid IANA timezone should be rejected");
    }

    #[test]
    fn test_default_timezone_accepts_valid() {
        let settings = Settings::default(); // default is "UTC"
        let config = AgentConfig::resolve(&settings).expect("resolve");
        assert_eq!(config.default_timezone, "UTC");
    }

    #[test]
    fn test_allowed_users_defaults_to_empty() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let settings = Settings::default();
        let config = AgentConfig::resolve(&settings).expect("resolve");
        assert!(config.allowed_users.is_empty());
    }

    #[test]
    fn test_allowed_users_from_settings() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let mut settings = Settings::default();
        settings.agent.allowed_users = vec!["user1".to_string(), "user2".to_string()];
        let config = AgentConfig::resolve(&settings).expect("resolve");
        assert_eq!(config.allowed_users, vec!["user1", "user2"]);
    }
}
