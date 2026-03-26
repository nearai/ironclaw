use std::time::Duration;

use crate::agent::drift_monitor::DriftConfig;
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
    /// Maximum daily LLM spend per user in cents. None = unlimited.
    pub max_cost_per_user_per_day_cents: Option<u64>,
    /// Maximum tool-call iterations per agentic loop invocation. Default 50.
    pub max_tool_iterations: usize,
    /// When true, skip tool approval checks entirely. For benchmarks/CI.
    pub auto_approve_tools: bool,
    /// Default timezone for new sessions (IANA name, e.g. "America/New_York").
    pub default_timezone: String,
    /// Maximum concurrent jobs per user. None = use global max_parallel_jobs.
    pub max_jobs_per_user: Option<usize>,
    /// Maximum tokens per job (0 = unlimited).
    pub max_tokens_per_job: u64,
    /// Whether the deployment is multi-tenant (multiple users sharing one
    /// instance). Defaults to false; can be set via AGENT_MULTI_TENANT env var.
    pub multi_tenant: bool,
    /// Maximum concurrent LLM calls per user. None = use default (4).
    pub max_llm_concurrent_per_user: Option<usize>,
    /// Maximum concurrent jobs per user. None = use default (3).
    pub max_jobs_concurrent_per_user: Option<usize>,
    /// Drift monitor configuration.
    pub drift: DriftConfig,
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
            max_cost_per_user_per_day_cents: None,
            max_tool_iterations: 10,
            auto_approve_tools: true,
            default_timezone: "UTC".to_string(),
            max_jobs_per_user: None,
            max_tokens_per_job: 0,
            multi_tenant: false,
            max_llm_concurrent_per_user: None,
            max_jobs_concurrent_per_user: None,
            drift: DriftConfig::default(),
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
            max_cost_per_user_per_day_cents: parse_option_env("MAX_COST_PER_USER_PER_DAY_CENTS")?,
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
            max_jobs_per_user: parse_option_env("MAX_JOBS_PER_USER")?,
            max_tokens_per_job: parse_optional_env(
                "AGENT_MAX_TOKENS_PER_JOB",
                settings.agent.max_tokens_per_job,
            )?,
            multi_tenant: parse_bool_env("AGENT_MULTI_TENANT", false)?,
            max_llm_concurrent_per_user: parse_option_env("TENANT_MAX_LLM_CONCURRENT")?,
            max_jobs_concurrent_per_user: parse_option_env("TENANT_MAX_JOBS_CONCURRENT")?,
            drift: DriftConfig {
                enabled: parse_bool_env("IRONCLAW_DRIFT_ENABLED", settings.agent.drift.enabled)?,
                repetition_threshold: parse_optional_env(
                    "IRONCLAW_DRIFT_REPETITION_THRESHOLD",
                    settings.agent.drift.repetition_threshold,
                )?,
                repetition_window: parse_optional_env(
                    "IRONCLAW_DRIFT_REPETITION_WINDOW",
                    settings.agent.drift.repetition_window,
                )?,
                failure_spiral_threshold: parse_optional_env(
                    "IRONCLAW_DRIFT_FAILURE_THRESHOLD",
                    settings.agent.drift.failure_spiral_threshold,
                )?,
                cycling_window: parse_optional_env(
                    "IRONCLAW_DRIFT_CYCLING_WINDOW",
                    settings.agent.drift.cycling_window,
                )?,
                silence_threshold: parse_optional_env(
                    "IRONCLAW_DRIFT_SILENCE_THRESHOLD",
                    settings.agent.drift.silence_threshold,
                )?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_drift_config_defaults_from_settings() {
        let _guard = crate::config::helpers::lock_env();
        // Ensure no drift env vars are set
        for key in [
            "IRONCLAW_DRIFT_ENABLED",
            "IRONCLAW_DRIFT_REPETITION_THRESHOLD",
            "IRONCLAW_DRIFT_REPETITION_WINDOW",
            "IRONCLAW_DRIFT_FAILURE_THRESHOLD",
            "IRONCLAW_DRIFT_CYCLING_WINDOW",
            "IRONCLAW_DRIFT_SILENCE_THRESHOLD",
        ] {
            // safety: test-only, guarded by ENV_MUTEX
            unsafe { std::env::remove_var(key) };
        }

        let settings = Settings::default();
        let config = AgentConfig::resolve(&settings).expect("resolve");
        assert!(config.drift.enabled);
        assert_eq!(config.drift.repetition_threshold, 3);
        assert_eq!(config.drift.repetition_window, 10);
        assert_eq!(config.drift.failure_spiral_threshold, 4);
        assert_eq!(config.drift.cycling_window, 6);
        assert_eq!(config.drift.silence_threshold, 15);
    }

    #[test]
    fn test_drift_config_env_override() {
        let _guard = crate::config::helpers::lock_env();
        // safety: test-only, guarded by ENV_MUTEX
        unsafe {
            std::env::set_var("IRONCLAW_DRIFT_ENABLED", "false");
            std::env::set_var("IRONCLAW_DRIFT_REPETITION_THRESHOLD", "5");
            std::env::set_var("IRONCLAW_DRIFT_SILENCE_THRESHOLD", "20");
        }

        let settings = Settings::default();
        let config = AgentConfig::resolve(&settings).expect("resolve");
        assert!(!config.drift.enabled);
        assert_eq!(config.drift.repetition_threshold, 5);
        assert_eq!(config.drift.silence_threshold, 20);
        // Non-overridden fields keep defaults
        assert_eq!(config.drift.failure_spiral_threshold, 4);

        // Cleanup
        unsafe {
            std::env::remove_var("IRONCLAW_DRIFT_ENABLED");
            std::env::remove_var("IRONCLAW_DRIFT_REPETITION_THRESHOLD");
            std::env::remove_var("IRONCLAW_DRIFT_SILENCE_THRESHOLD");
        }
    }
}
