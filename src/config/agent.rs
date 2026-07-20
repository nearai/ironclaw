use std::time::Duration;

use crate::config::helpers::{
    db_first_bool, db_first_or_default, parse_bool_env, parse_option_env,
};
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
    /// Override `ContextMonitor`'s context-window limit (tokens). None = use
    /// its own default (`DEFAULT_CONTEXT_LIMIT`, currently 100_000, compaction
    /// fires at 80% of it). Exists so callers driving the legacy Agent loop
    /// directly (benchmarks, evals) can test the same session with compaction
    /// on vs. off — a raw-context control against the shipped default,
    /// without a code change per comparison. Settable via
    /// `AGENT_CONTEXT_TOKEN_LIMIT`.
    pub context_token_limit: Option<usize>,
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
            context_token_limit: None,
        }
    }

    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = crate::settings::AgentSettings::default();

        Ok(Self {
            name: db_first_or_default(&settings.agent.name, &defaults.name, "AGENT_NAME")?,
            // Settings stores u32, config uses usize — cast for comparison.
            max_parallel_jobs: db_first_or_default(
                &(settings.agent.max_parallel_jobs as usize),
                &(defaults.max_parallel_jobs as usize),
                "AGENT_MAX_PARALLEL_JOBS",
            )?,
            job_timeout: Duration::from_secs(db_first_or_default(
                &settings.agent.job_timeout_secs,
                &defaults.job_timeout_secs,
                "AGENT_JOB_TIMEOUT_SECS",
            )?),
            stuck_threshold: Duration::from_secs(db_first_or_default(
                &settings.agent.stuck_threshold_secs,
                &defaults.stuck_threshold_secs,
                "AGENT_STUCK_THRESHOLD_SECS",
            )?),
            repair_check_interval: Duration::from_secs(db_first_or_default(
                &settings.agent.repair_check_interval_secs,
                &defaults.repair_check_interval_secs,
                "SELF_REPAIR_CHECK_INTERVAL_SECS",
            )?),
            max_repair_attempts: db_first_or_default(
                &settings.agent.max_repair_attempts,
                &defaults.max_repair_attempts,
                "SELF_REPAIR_MAX_ATTEMPTS",
            )?,
            use_planning: db_first_bool(
                settings.agent.use_planning,
                defaults.use_planning,
                "AGENT_USE_PLANNING",
            )?,
            session_idle_timeout: Duration::from_secs(db_first_or_default(
                &settings.agent.session_idle_timeout_secs,
                &defaults.session_idle_timeout_secs,
                "SESSION_IDLE_TIMEOUT_SECS",
            )?),
            allow_local_tools: parse_bool_env("ALLOW_LOCAL_TOOLS", false)?,
            max_cost_per_day_cents: parse_option_env("MAX_COST_PER_DAY_CENTS")?,
            max_actions_per_hour: parse_option_env("MAX_ACTIONS_PER_HOUR")?,
            max_cost_per_user_per_day_cents: parse_option_env("MAX_COST_PER_USER_PER_DAY_CENTS")?,
            max_tool_iterations: db_first_or_default(
                &settings.agent.max_tool_iterations,
                &defaults.max_tool_iterations,
                "AGENT_MAX_TOOL_ITERATIONS",
            )?,
            auto_approve_tools: db_first_bool(
                settings.agent.auto_approve_tools,
                defaults.auto_approve_tools,
                "AGENT_AUTO_APPROVE_TOOLS",
            )?,
            default_timezone: {
                let tz: String = db_first_or_default(
                    &settings.agent.default_timezone,
                    &defaults.default_timezone,
                    "DEFAULT_TIMEZONE",
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
            max_tokens_per_job: db_first_or_default(
                &settings.agent.max_tokens_per_job,
                &defaults.max_tokens_per_job,
                "AGENT_MAX_TOKENS_PER_JOB",
            )?,
            multi_tenant: parse_bool_env("AGENT_MULTI_TENANT", false)?,
            max_llm_concurrent_per_user: parse_option_env("TENANT_MAX_LLM_CONCURRENT")?,
            max_jobs_concurrent_per_user: parse_option_env("TENANT_MAX_JOBS_CONCURRENT")?,
            context_token_limit: parse_option_env("AGENT_CONTEXT_TOKEN_LIMIT")?,
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

    // Serializes tests that mutate AGENT_CONTEXT_TOKEN_LIMIT (process-global
    // state) so they don't race under the default parallel test runner.
    static CONTEXT_TOKEN_LIMIT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_context_token_limit_defaults_to_none() {
        let _guard = CONTEXT_TOKEN_LIMIT_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // SAFETY: serialized by the lock above.
        unsafe {
            std::env::remove_var("AGENT_CONTEXT_TOKEN_LIMIT");
        }
        let settings = Settings::default();
        let config = AgentConfig::resolve(&settings).expect("resolve");
        assert_eq!(config.context_token_limit, None);
    }

    #[test]
    fn test_context_token_limit_reads_env_override() {
        let _guard = CONTEXT_TOKEN_LIMIT_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // SAFETY: serialized by the lock above; restored before return.
        unsafe {
            std::env::set_var("AGENT_CONTEXT_TOKEN_LIMIT", "500000");
        }
        let settings = Settings::default();
        let config = AgentConfig::resolve(&settings).expect("resolve");
        unsafe {
            std::env::remove_var("AGENT_CONTEXT_TOKEN_LIMIT");
        }
        assert_eq!(config.context_token_limit, Some(500_000));
    }
}
