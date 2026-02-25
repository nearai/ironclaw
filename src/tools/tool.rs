//! Tool trait and types.

use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::context::JobContext;

/// Risk level of a tool invocation.
///
/// Used by the shell tool to classify commands and by the worker to drive
/// approval decisions and observability logging. Implements `Ord` so callers
/// can compare levels (e.g. `risk >= RiskLevel::High`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Read-only, safe, reversible (e.g. `ls`, `cat`, `grep`).
    Low,
    /// Creates or modifies state, but generally reversible
    /// (e.g. `mkdir`, `git commit`, `cargo build`).
    Medium,
    /// Destructive, irreversible, or security-sensitive
    /// (e.g. `rm -rf`, `git push --force`, `kill -9`).
    High,
}

/// How much approval a specific tool invocation requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRequirement {
    /// No approval needed.
    Never,
    /// Needs approval, but session auto-approve can bypass.
    UnlessAutoApproved,
    /// Always needs explicit approval (even if auto-approved).
    Always,
}

impl ApprovalRequirement {
    /// Whether this invocation requires approval in contexts where
    /// auto-approve is irrelevant (e.g. autonomous worker/scheduler).
    pub fn is_required(&self) -> bool {
        !matches!(self, Self::Never)
    }
}

/// Per-tool rate limit configuration for built-in tool invocations.
///
/// Controls how many times a tool can be invoked per user, per time window.
/// Read-only tools (echo, time, json, file_read, etc.) should NOT be rate limited.
/// Write/external tools (shell, http, file_write, memory_write, create_job) should be.
#[derive(Debug, Clone)]
pub struct ToolRateLimitConfig {
    /// Maximum invocations per minute.
    pub requests_per_minute: u32,
    /// Maximum invocations per hour.
    pub requests_per_hour: u32,
}

impl ToolRateLimitConfig {
    /// Create a config with explicit limits.
    pub fn new(requests_per_minute: u32, requests_per_hour: u32) -> Self {
        Self {
            requests_per_minute,
            requests_per_hour,
        }
    }
}

impl Default for ToolRateLimitConfig {
    /// Default: 60 requests/minute, 1000 requests/hour (generous for WASM HTTP).
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
            requests_per_hour: 1000,
        }
    }
}

/// Where a tool should execute: orchestrator process or inside a container.
///
/// Orchestrator tools run in the main agent process (memory access, job mgmt, etc).
/// Container tools run inside Docker containers (shell, file ops, code mods).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolDomain {
    /// Safe to run in the orchestrator (pure functions, memory, job management).
    Orchestrator,
    /// Must run inside a sandboxed container (filesystem, shell, code).
    Container,
}

/// Error type for tool execution.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Timeout after {0:?}")]
    Timeout(Duration),

    #[error("Not authorized: {0}")]
    NotAuthorized(String),

    #[error("Rate limited, retry after {0:?}")]
    RateLimited(Option<Duration>),

    #[error("External service error: {0}")]
    ExternalService(String),

    #[error("Sandbox error: {0}")]
    Sandbox(String),
}

/// Whether a tool error is transient (worth retrying) or permanent (fail immediately).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorKind {
    /// Could succeed on retry (rate limit, external service hiccup, sandbox crash).
    Transient,
    /// Retrying won't help (bad params, not authorized, logic failure).
    Permanent,
}

impl ToolError {
    /// Classify this error as transient or permanent for retry decisions.
    pub fn kind(&self) -> ToolErrorKind {
        match self {
            // Transient: could succeed on retry
            ToolError::RateLimited(..) | ToolError::ExternalService(..) => ToolErrorKind::Transient,
            // Transient but capped at 2 retries (via max_retries_for)
            ToolError::Sandbox(..) | ToolError::Timeout(..) => ToolErrorKind::Transient,
            // Permanent: retrying won't help
            ToolError::InvalidParameters(..)
            | ToolError::ExecutionFailed(..)
            | ToolError::NotAuthorized(..) => ToolErrorKind::Permanent,
        }
    }
}

/// Retry configuration for tool execution.
#[derive(Debug, Clone)]
pub struct ToolRetryConfig {
    /// Maximum number of retry attempts (not counting the initial attempt).
    pub max_retries: u32,
    /// Base delay before first retry.
    pub base_delay: Duration,
    /// Maximum delay cap (delays are capped at this value).
    pub max_delay: Duration,
}

impl Default for ToolRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl ToolRetryConfig {
    /// Effective max retries for a given error.
    ///
    /// Sandbox/Timeout errors are capped at 2 — repeated failures suggest a
    /// configuration issue rather than a transient blip.
    pub fn max_retries_for(&self, error: &ToolError) -> u32 {
        match error {
            ToolError::Sandbox(..) | ToolError::Timeout(..) => self.max_retries.min(2),
            _ => self.max_retries,
        }
    }
}

/// Exponential backoff delay with jitter for tool retries.
///
/// Respects `RateLimited(Some(hint))` from the tool, capped at `config.max_delay`.
/// Mirrors `llm::retry::retry_backoff_delay` but uses the tool's own base/max delays.
pub fn tool_retry_delay(
    attempt: u32,
    config: &ToolRetryConfig,
    error: Option<&ToolError>,
) -> Duration {
    // Honor the provider-supplied retry-after hint for rate limiting
    if let Some(ToolError::RateLimited(Some(hint))) = error {
        return (*hint).min(config.max_delay);
    }
    let base_ms = config.base_delay.as_millis() as u64;
    let exp_ms = base_ms.saturating_mul(2u64.saturating_pow(attempt));
    let capped_ms = exp_ms.min(config.max_delay.as_millis() as u64);
    let jitter_range = capped_ms / 4; // 25% jitter
    let jitter = if jitter_range > 0 {
        let offset = rand::thread_rng().gen_range(0..=jitter_range * 2);
        offset as i64 - jitter_range as i64
    } else {
        0
    };
    Duration::from_millis((capped_ms as i64 + jitter).max(100) as u64)
}

/// Output from a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// The result data.
    pub result: serde_json::Value,
    /// Cost incurred (if any).
    pub cost: Option<Decimal>,
    /// Time taken.
    pub duration: Duration,
    /// Raw output before sanitization (for debugging).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl ToolOutput {
    /// Create a successful output with a JSON result.
    pub fn success(result: serde_json::Value, duration: Duration) -> Self {
        Self {
            result,
            cost: None,
            duration,
            raw: None,
        }
    }

    /// Create a text output.
    pub fn text(text: impl Into<String>, duration: Duration) -> Self {
        Self {
            result: serde_json::Value::String(text.into()),
            cost: None,
            duration,
            raw: None,
        }
    }

    /// Set the cost.
    pub fn with_cost(mut self, cost: Decimal) -> Self {
        self.cost = Some(cost);
        self
    }

    /// Set the raw output.
    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }
}

/// Definition of a tool's parameters using JSON Schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSchema {
    /// Create a new tool schema.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    /// Set the parameters schema.
    pub fn with_parameters(mut self, parameters: serde_json::Value) -> Self {
        self.parameters = parameters;
        self
    }
}

/// Trait for tools that the agent can use.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool name.
    fn name(&self) -> &str;

    /// Get a description of what the tool does.
    fn description(&self) -> &str;

    /// Get the JSON Schema for the tool's parameters.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given parameters.
    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError>;

    /// Estimate the cost of running this tool with the given parameters.
    fn estimated_cost(&self, _params: &serde_json::Value) -> Option<Decimal> {
        None
    }

    /// Estimate how long this tool will take with the given parameters.
    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        None
    }

    /// Whether this tool's output needs sanitization.
    ///
    /// Returns true for tools that interact with external services,
    /// where the output might contain malicious content.
    fn requires_sanitization(&self) -> bool {
        true
    }

    /// Risk level for a specific invocation of this tool.
    ///
    /// Defaults to `Low` (read-only, safe). Override for tools whose risk
    /// depends on the parameters — the shell tool classifies commands into
    /// Low / Medium / High based on the command string.
    ///
    /// The worker logs this value with every tool call so operators can audit
    /// what risk level each execution was classified at.
    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Low
    }

    /// Whether this tool invocation requires user approval.
    ///
    /// Returns `Never` by default (most tools run in a sandboxed environment).
    /// Override to return `UnlessAutoApproved` for tools that need approval
    /// but can be session-auto-approved, or `Always` for invocations that
    /// must always prompt (e.g. destructive shell commands, HTTP with auth).
    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    /// Maximum time this tool is allowed to run before the caller kills it.
    /// Override for long-running tools like sandbox execution.
    /// Default: 60 seconds.
    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    /// Where this tool should execute.
    ///
    /// `Orchestrator` tools run in the main agent process (safe, no FS access).
    /// `Container` tools run inside Docker containers (shell, file ops).
    ///
    /// Default: `Orchestrator` (safe for the main process).
    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    /// Per-invocation rate limit for this tool.
    ///
    /// Return `Some(config)` to throttle how often this tool can be called per user.
    /// Read-only tools (echo, time, json, file_read, memory_search, etc.) should
    /// return `None`. Write/external tools (shell, http, file_write, memory_write,
    /// create_job) should return sensible limits to prevent runaway agents.
    ///
    /// Rate limits are per-user, per-tool, and in-memory (reset on restart).
    /// This is orthogonal to `requires_approval()` — a tool can be both
    /// approval-gated and rate limited. Rate limit is checked first (cheaper).
    ///
    /// Default: `None` (no rate limiting).
    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        None
    }

    /// Retry configuration for transient failures.
    ///
    /// Controls how many times a transient error is retried and the backoff
    /// schedule. Override for tools that should never retry (e.g. destructive
    /// tools) or that need a longer backoff (e.g. heavily rate-limited APIs).
    ///
    /// Default: 3 retries, 2s base delay, 30s max delay.
    fn retry_config(&self) -> ToolRetryConfig {
        ToolRetryConfig::default()
    }

    /// Get the tool schema for LLM function calling.
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

/// Extract a required string parameter from a JSON object.
///
/// Returns `ToolError::InvalidParameters` if the key is missing or not a string.
pub fn require_str<'a>(params: &'a serde_json::Value, name: &str) -> Result<&'a str, ToolError> {
    params
        .get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing '{}' parameter", name)))
}

/// Extract a required parameter of any type from a JSON object.
///
/// Returns `ToolError::InvalidParameters` if the key is missing.
pub fn require_param<'a>(
    params: &'a serde_json::Value,
    name: &str,
) -> Result<&'a serde_json::Value, ToolError> {
    params
        .get(name)
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing '{}' parameter", name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple no-op tool for testing.
    #[derive(Debug)]
    pub struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echoes back the input message. Useful for testing."
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The message to echo back"
                    }
                },
                "required": ["message"]
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            let message = require_str(&params, "message")?;

            Ok(ToolOutput::text(message, Duration::from_millis(1)))
        }

        fn requires_sanitization(&self) -> bool {
            false // Echo is a trusted internal tool
        }
    }

    #[tokio::test]
    async fn test_echo_tool() {
        let tool = EchoTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"message": "hello"}), &ctx)
            .await
            .unwrap();

        assert_eq!(result.result, serde_json::json!("hello"));
    }

    #[test]
    fn test_tool_schema() {
        let tool = EchoTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "echo");
        assert!(!schema.description.is_empty());
    }

    #[test]
    fn test_execution_timeout_default() {
        let tool = EchoTool;
        assert_eq!(tool.execution_timeout(), Duration::from_secs(60));
    }

    #[test]
    fn test_require_str_present() {
        let params = serde_json::json!({"name": "alice"});
        assert_eq!(require_str(&params, "name").unwrap(), "alice");
    }

    #[test]
    fn test_require_str_missing() {
        let params = serde_json::json!({});
        let err = require_str(&params, "name").unwrap_err();
        assert!(err.to_string().contains("missing 'name'"));
    }

    #[test]
    fn test_require_str_wrong_type() {
        let params = serde_json::json!({"name": 42});
        let err = require_str(&params, "name").unwrap_err();
        assert!(err.to_string().contains("missing 'name'"));
    }

    #[test]
    fn test_require_param_present() {
        let params = serde_json::json!({"data": [1, 2, 3]});
        assert_eq!(
            require_param(&params, "data").unwrap(),
            &serde_json::json!([1, 2, 3])
        );
    }

    #[test]
    fn test_require_param_missing() {
        let params = serde_json::json!({});
        let err = require_param(&params, "data").unwrap_err();
        assert!(err.to_string().contains("missing 'data'"));
    }

    #[test]
    fn test_requires_approval_default() {
        let tool = EchoTool;
        // Default requires_approval() returns Never.
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"message": "hi"})),
            ApprovalRequirement::Never
        );
        assert!(!ApprovalRequirement::Never.is_required());
        assert!(ApprovalRequirement::UnlessAutoApproved.is_required());
        assert!(ApprovalRequirement::Always.is_required());
    }

    // -- ToolErrorKind classification tests --

    #[test]
    fn tool_error_kind_transient_variants() {
        assert_eq!(
            ToolError::RateLimited(None).kind(),
            ToolErrorKind::Transient
        );
        assert_eq!(
            ToolError::RateLimited(Some(Duration::from_secs(5))).kind(),
            ToolErrorKind::Transient
        );
        assert_eq!(
            ToolError::ExternalService("upstream down".into()).kind(),
            ToolErrorKind::Transient
        );
        assert_eq!(
            ToolError::Sandbox("container crash".into()).kind(),
            ToolErrorKind::Transient
        );
        assert_eq!(
            ToolError::Timeout(Duration::from_secs(60)).kind(),
            ToolErrorKind::Transient
        );
    }

    #[test]
    fn tool_error_kind_permanent_variants() {
        assert_eq!(
            ToolError::InvalidParameters("bad input".into()).kind(),
            ToolErrorKind::Permanent
        );
        assert_eq!(
            ToolError::ExecutionFailed("logic error".into()).kind(),
            ToolErrorKind::Permanent
        );
        assert_eq!(
            ToolError::NotAuthorized("missing scope".into()).kind(),
            ToolErrorKind::Permanent
        );
    }

    #[test]
    fn tool_retry_config_sandbox_capped_at_2() {
        let cfg = ToolRetryConfig {
            max_retries: 5,
            ..Default::default()
        };
        assert!(cfg.max_retries_for(&ToolError::Sandbox("crash".into())) <= 2);
        assert!(cfg.max_retries_for(&ToolError::Timeout(Duration::from_secs(1))) <= 2);
        assert_eq!(cfg.max_retries_for(&ToolError::RateLimited(None)), 5);
    }

    #[test]
    fn tool_retry_delay_uses_rate_limit_hint() {
        let cfg = ToolRetryConfig::default();
        let hint = Duration::from_secs(10);
        let delay = tool_retry_delay(0, &cfg, Some(&ToolError::RateLimited(Some(hint))));
        assert_eq!(delay, hint);

        // Hint exceeding max_delay should be capped
        let big_hint = Duration::from_secs(1000);
        let delay = tool_retry_delay(0, &cfg, Some(&ToolError::RateLimited(Some(big_hint))));
        assert_eq!(delay, cfg.max_delay);
    }

    #[test]
    fn tool_retry_delay_exponential_growth() {
        let cfg = ToolRetryConfig {
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(30),
            max_retries: 3,
        };
        // Run multiple samples to account for jitter
        for _ in 0..20 {
            // attempt 0: base 2000ms, jitter +/-500ms -> [1500, 2500]
            let d0 = tool_retry_delay(0, &cfg, None);
            assert!(d0.as_millis() >= 1500, "attempt 0 too low: {:?}", d0);
            assert!(d0.as_millis() <= 2500, "attempt 0 too high: {:?}", d0);

            // attempt 1: base 4000ms, jitter +/-1000ms -> [3000, 5000]
            let d1 = tool_retry_delay(1, &cfg, None);
            assert!(d1.as_millis() >= 3000, "attempt 1 too low: {:?}", d1);
            assert!(d1.as_millis() <= 5000, "attempt 1 too high: {:?}", d1);

            // attempt 5: exp would be 64000ms, capped at 30000ms, jitter +/-7500ms -> [22500, 37500]
            // (jitter can push the result above max_delay — we only cap the pre-jitter base)
            let d5 = tool_retry_delay(5, &cfg, None);
            assert!(d5.as_millis() >= 22500, "attempt 5 too low: {:?}", d5);
            assert!(d5.as_millis() <= 37500, "attempt 5 too high: {:?}", d5);
        }
    }
}
