use std::time::Duration;

use crate::config::helpers::{
    db_first_bool, db_first_or_default, optional_env, parse_bool_env, parse_option_env,
};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Parse `DISABLE_TOOLS_LIST` into a list of tool names.
///
/// Accepts either bare CSV (`secret_delete,memory_write`) or a bracketed
/// list (`[secret_delete,memory_write]` / `["secret_delete","memory_write"]`).
/// Strips surrounding `[]`, `"` and `'`, trims whitespace, and drops empty
/// entries. Returns an empty vector when the var is unset or contains
/// nothing but separators.
fn parse_disabled_tools(raw: Option<String>) -> Vec<String> {
    let Some(s) = raw else { return Vec::new() };
    // `strip_prefix` / `strip_suffix` remove at most ONE bracket each, so
    // `[[shell]]` becomes `[shell]` (the inner bracket survives, the per-item
    // trim treats it as part of the name). `trim_*_matches` was greedy and
    // chewed every leading/trailing `[` / `]`, masking malformed input.
    let outer = s.trim();
    let outer = outer.strip_prefix('[').unwrap_or(outer);
    let outer = outer.strip_suffix(']').unwrap_or(outer);
    outer
        .split(',')
        .map(|item| {
            item.trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string()
        })
        .filter(|item| !item.is_empty())
        .collect()
}

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
    /// Additional tool names to remove from the registry after all
    /// registration completes. Extends `allow_local_tools=false` by
    /// disabling any tool name listed here (e.g. `secret_delete`).
    /// Parsed from the `DISABLE_TOOLS_LIST` env var as CSV or
    /// bracketed list. Empty when unset.
    pub disabled_tools: Vec<String>,
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
    /// Enable engine v2 routing (Strategy C parallel deployment).
    /// Set via `ENGINE_V2=true` env var or programmatically in tests.
    pub engine_v2: bool,
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
            disabled_tools: Vec::new(),
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
            engine_v2: false,
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
            disabled_tools: parse_disabled_tools(optional_env("DISABLE_TOOLS_LIST")?),
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
            engine_v2: parse_bool_env("ENGINE_V2", false)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

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
    fn parse_disabled_tools_unset_returns_empty() {
        assert!(parse_disabled_tools(None).is_empty());
    }

    #[test]
    fn parse_disabled_tools_csv() {
        assert_eq!(
            parse_disabled_tools(Some("secret_delete,memory_write".to_string())),
            vec!["secret_delete", "memory_write"]
        );
    }

    #[test]
    fn parse_disabled_tools_bracketed_quoted() {
        assert_eq!(
            parse_disabled_tools(Some("[\"secret_delete\", \"memory_write\"]".to_string())),
            vec!["secret_delete", "memory_write"]
        );
    }

    #[test]
    fn parse_disabled_tools_skips_empty_and_trims() {
        assert_eq!(
            parse_disabled_tools(Some(" secret_delete , , shell ".to_string())),
            vec!["secret_delete", "shell"]
        );
    }

    #[test]
    fn parse_disabled_tools_strips_at_most_one_bracket_each_side() {
        // Gemini + zmanian flagged that `trim_start_matches('[')` chewed
        // every leading `[`, so `[[shell]]` silently parsed as `shell`
        // instead of `[shell]`. Switched to `strip_prefix` / `strip_suffix`
        // (single-char) — assert the new behavior is non-greedy.
        assert_eq!(
            parse_disabled_tools(Some("[[shell]]".to_string())),
            vec!["[shell]"],
            "only ONE outer bracket should be stripped on each side"
        );
        // A well-formed bracketed list still unwraps cleanly.
        assert_eq!(
            parse_disabled_tools(Some("[shell]".to_string())),
            vec!["shell"]
        );
    }

    #[test]
    fn disabled_tools_resolves_from_env() {
        let _guard = lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::set_var("DISABLE_TOOLS_LIST", "secret_delete,shell") };

        let config = AgentConfig::resolve(&Settings::default()).expect("resolve");
        assert_eq!(config.disabled_tools, vec!["secret_delete", "shell"]);

        unsafe { std::env::remove_var("DISABLE_TOOLS_LIST") };
    }

    #[test]
    fn disabled_tools_defaults_to_empty_when_unset() {
        let _guard = lock_env();
        // SAFETY: under ENV_MUTEX
        unsafe { std::env::remove_var("DISABLE_TOOLS_LIST") };

        let config = AgentConfig::resolve(&Settings::default()).expect("resolve");
        assert!(config.disabled_tools.is_empty());
    }
}
