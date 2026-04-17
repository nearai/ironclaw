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
    /// When true (and `auto_approve_tools` is also true), additionally skip
    /// approval prompts for `ApprovalRequirement::Always`-gated destructive
    /// tools. Strict refinement of `auto_approve_tools` — has no effect on
    /// its own. Activated via `AGENT_AUTO_APPROVE_DESTRUCTIVE=true` env var
    /// or settings. Intended for trusted demo / single-user agents.
    pub auto_approve_destructive: bool,
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
    /// Enable engine v2 routing (Strategy C parallel deployment).
    /// Set via `ENGINE_V2=true` env var or programmatically in tests.
    pub engine_v2: bool,
    /// Platform brand name for technical UI surfaces (OAuth pages, User-Agent,
    /// MCP client registration). Set via `PLATFORM_NAME` env var. Default: "C3S".
    pub platform_name: String,
    /// Whether this instance is managed by a platform (e.g. LobsterPool).
    /// When true, the system prompt uses workspace identity instead of the
    /// hardcoded "IronClaw Agent" preamble, and platform-injected skills
    /// are not downgraded. Set via `PLATFORM_MANAGED=true` env var (typically
    /// injected by the platform at container creation time).
    pub platform_managed: bool,

    /// Route incoming messages through the per-thread fanout layer instead
    /// of the legacy sequential loop. Set via `AGENT_THREAD_FANOUT_ENABLED`
    /// env var. Default: `true`. When `false`, `Agent::run` falls back to
    /// strict serial handling — the pre-refactor behavior. Kept for
    /// emergency rollback; expected to be removed one release after
    /// sustained prod observation.
    pub thread_fanout_enabled: bool,
    /// Global cap on concurrently executing agent turns when the fanout
    /// layer is enabled. Each per-thread bucket worker must acquire one
    /// semaphore permit before invoking `Agent::process_one`. Default: 32.
    /// Set via `AGENT_MAX_CONCURRENT_TURNS`.
    pub max_concurrent_turns: usize,
    /// Idle timeout after which a per-thread bucket is reaped (bucket
    /// registry cleanup). Default: 300 s (5 min). Set via
    /// `AGENT_BUCKET_IDLE_TIMEOUT_SECS`.
    pub bucket_idle_timeout_secs: u64,
    /// Per-bucket mpsc queue capacity. When a bucket's queue is full, new
    /// dispatches surface `DispatchError::BucketFull` and the caller emits
    /// a `Status::Busy` update. Default: 16. Set via
    /// `AGENT_BUCKET_QUEUE_CAPACITY`.
    pub bucket_queue_capacity: usize,
    /// Graceful-drain window during fanout shutdown. Workers that exceed
    /// this are aborted with a warn-level log. Default: 30 s. Set via
    /// `AGENT_SHUTDOWN_GRACE_SECS`.
    pub shutdown_grace_secs: u64,
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
            auto_approve_destructive: false,
            default_timezone: "UTC".to_string(),
            max_jobs_per_user: None,
            max_tokens_per_job: 0,
            multi_tenant: false,
            max_llm_concurrent_per_user: None,
            max_jobs_concurrent_per_user: None,
            engine_v2: false,
            platform_name: "C3S".to_string(),
            platform_managed: false,
            thread_fanout_enabled: true,
            max_concurrent_turns: 32,
            bucket_idle_timeout_secs: 300,
            bucket_queue_capacity: 16,
            shutdown_grace_secs: 30,
        }
    }

    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = crate::settings::AgentSettings::default();

        let resolved_auto_approve = db_first_bool(
            settings.agent.auto_approve_tools,
            defaults.auto_approve_tools,
            "AGENT_AUTO_APPROVE_TOOLS",
        )?;
        let raw_auto_approve_destructive = db_first_bool(
            settings.agent.auto_approve_destructive,
            defaults.auto_approve_destructive,
            "AGENT_AUTO_APPROVE_DESTRUCTIVE",
        )?;
        // Clamp invariant: destructive bypass requires the standard auto-approve
        // flag. If only `auto_approve_destructive` is set, the operator either
        // misconfigured or the platform pushed a partial state — emit a warn
        // once at startup and fall back to standard interactive behavior so
        // approval cards are still shown for `Always` tools.
        let resolved_auto_approve_destructive = if raw_auto_approve_destructive
            && !resolved_auto_approve
        {
            tracing::warn!(
                "AGENT_AUTO_APPROVE_DESTRUCTIVE=true requires AGENT_AUTO_APPROVE_TOOLS=true; \
                     destructive bypass is disabled until the standard auto-approve flag is also enabled."
            );
            false
        } else {
            raw_auto_approve_destructive
        };

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
            auto_approve_tools: resolved_auto_approve,
            auto_approve_destructive: resolved_auto_approve_destructive,
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
            engine_v2: parse_bool_env("ENGINE_V2", false)?,
            platform_name: {
                let default_pn = crate::settings::default_platform_name();
                std::env::var("PLATFORM_NAME").unwrap_or(default_pn)
            },
            platform_managed: parse_bool_env("PLATFORM_MANAGED", false)?,
            thread_fanout_enabled: parse_bool_env("AGENT_THREAD_FANOUT_ENABLED", true)?,
            max_concurrent_turns: parse_option_env("AGENT_MAX_CONCURRENT_TURNS")?.unwrap_or(32),
            bucket_idle_timeout_secs: parse_option_env("AGENT_BUCKET_IDLE_TIMEOUT_SECS")?
                .unwrap_or(300),
            bucket_queue_capacity: parse_option_env("AGENT_BUCKET_QUEUE_CAPACITY")?.unwrap_or(16),
            shutdown_grace_secs: parse_option_env("AGENT_SHUTDOWN_GRACE_SECS")?.unwrap_or(30),
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

    /// `AGENT_AUTO_APPROVE_DESTRUCTIVE=true` only takes effect when
    /// `AGENT_AUTO_APPROVE_TOOLS=true` is also set. With the standard flag
    /// off, the destructive flag must be clamped to `false` so approval
    /// cards still appear for `Always` tools. This protects operators who
    /// accidentally set only the destructive flag.
    #[test]
    fn test_auto_approve_destructive_is_clamped_without_standard_flag() {
        let mut settings = Settings::default();
        settings.agent.auto_approve_tools = false;
        settings.agent.auto_approve_destructive = true;

        let config = AgentConfig::resolve(&settings).expect("resolve");
        assert!(
            !config.auto_approve_destructive,
            "destructive bypass must be clamped when standard auto-approve is disabled"
        );
        assert!(!config.auto_approve_tools);
    }

    /// When both flags are on the destructive bypass survives resolution.
    #[test]
    fn test_auto_approve_destructive_survives_when_standard_flag_set() {
        let mut settings = Settings::default();
        settings.agent.auto_approve_tools = true;
        settings.agent.auto_approve_destructive = true;

        let config = AgentConfig::resolve(&settings).expect("resolve");
        assert!(config.auto_approve_tools);
        assert!(config.auto_approve_destructive);
    }
}
