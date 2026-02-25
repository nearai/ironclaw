//! Tool-level retry with exponential backoff for transient errors.
//!
//! Wraps only `tool.execute()` — hooks, validation, rate limits, and approval
//! checks are NOT retried. Permanent errors (`InvalidParameters`, `ExecutionFailed`,
//! `NotAuthorized`) fail immediately; transient errors (`RateLimited`,
//! `ExternalService`, `Timeout`, `Sandbox`) are retried with backoff.

use std::time::{Duration, Instant};

use rand::Rng;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolDomain, ToolError, ToolErrorKind, ToolOutput, ToolRetryConfig};

/// Outcome of a retry-wrapped tool execution.
#[derive(Debug)]
pub struct ToolRetryOutcome {
    /// The final result (success or last error).
    pub result: Result<ToolOutput, ToolError>,
    /// Number of retry attempts (0 = succeeded on first try).
    pub retry_attempts: u32,
    /// Total wall-clock time across all attempts including sleeps.
    pub total_duration: Duration,
}

/// Calculate exponential backoff delay with 25% jitter, capped at `max_delay`.
///
/// Formula: `base_delay * 2^attempt`, then add uniform jitter in [-25%, +25%].
fn backoff_delay(config: &ToolRetryConfig, attempt: u32) -> Duration {
    let base_ms = config.base_delay.as_millis() as u64;
    let exp_ms = base_ms.saturating_mul(2u64.saturating_pow(attempt));
    let capped_ms = exp_ms.min(config.max_delay.as_millis() as u64);

    let jitter_range = capped_ms / 4; // 25%
    let jitter = if jitter_range > 0 {
        let offset = rand::thread_rng().gen_range(0..=jitter_range * 2);
        offset as i64 - jitter_range as i64
    } else {
        0
    };
    let delay_ms = (capped_ms as i64 + jitter).max(100) as u64;
    Duration::from_millis(delay_ms)
}

/// Execute a tool with automatic retry on transient errors.
///
/// - Loops up to `config.max_retries + 1` attempts
/// - On permanent error, returns immediately
/// - On transient error, sleeps with exponential backoff + jitter
/// - Honors `RateLimited(Some(duration))` by using the server-suggested delay
/// - Tracks `remaining_budget` to stop before exceeding `budget`
pub async fn retry_tool_execute(
    tool: &dyn Tool,
    params: &serde_json::Value,
    ctx: &JobContext,
    config: &ToolRetryConfig,
    budget: Duration,
) -> ToolRetryOutcome {
    let start = Instant::now();
    let mut retry_attempts: u32 = 0;

    for attempt in 0..=config.max_retries {
        match tool.execute(params.clone(), ctx).await {
            Ok(output) => {
                return ToolRetryOutcome {
                    result: Ok(output),
                    retry_attempts,
                    total_duration: start.elapsed(),
                };
            }
            Err(err) => {
                // Permanent errors: fail immediately
                if err.kind() == ToolErrorKind::Permanent {
                    return ToolRetryOutcome {
                        result: Err(err),
                        retry_attempts,
                        total_duration: start.elapsed(),
                    };
                }

                // Last attempt: return the error without sleeping
                if attempt == config.max_retries {
                    return ToolRetryOutcome {
                        result: Err(err),
                        retry_attempts,
                        total_duration: start.elapsed(),
                    };
                }

                // Calculate delay: prefer server-suggested for RateLimited
                let delay = match err.retry_after() {
                    Some(suggested) => suggested.min(config.max_delay),
                    None => backoff_delay(config, attempt),
                };

                // Budget check: stop if sleeping would exceed remaining time
                let elapsed = start.elapsed();
                let remaining = budget.saturating_sub(elapsed);
                if delay >= remaining {
                    tracing::warn!(
                        tool = %tool.name(),
                        attempt = attempt + 1,
                        delay_ms = delay.as_millis() as u64,
                        remaining_ms = remaining.as_millis() as u64,
                        error = %err,
                        "Retry budget exhausted, returning last error"
                    );
                    return ToolRetryOutcome {
                        result: Err(err),
                        retry_attempts,
                        total_duration: start.elapsed(),
                    };
                }

                retry_attempts += 1;
                tracing::warn!(
                    tool = %tool.name(),
                    attempt = attempt + 1,
                    max_retries = config.max_retries,
                    delay_ms = delay.as_millis() as u64,
                    error = %err,
                    "Retrying tool after transient error"
                );

                tokio::time::sleep(delay).await;
            }
        }
    }

    // Should not be reached, but handle gracefully
    ToolRetryOutcome {
        result: Err(ToolError::ExecutionFailed(
            "retry loop exited unexpectedly".to_string(),
        )),
        retry_attempts,
        total_duration: start.elapsed(),
    }
}

/// Determine the effective retry config for a tool.
///
/// Priority:
/// 1. Tool's own `retry_config()` override
/// 2. Container-domain tools get `ToolRetryConfig::sandbox()` (2 retries)
/// 3. Default (5 retries)
pub fn effective_retry_config(tool: &dyn Tool) -> ToolRetryConfig {
    if let Some(config) = tool.retry_config() {
        return config;
    }
    match tool.domain() {
        ToolDomain::Container => ToolRetryConfig::sandbox(),
        ToolDomain::Orchestrator => ToolRetryConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use async_trait::async_trait;

    use crate::context::JobContext;
    use crate::tools::tool::{ToolDomain, ToolOutput, ToolRetryConfig};

    /// Mock tool that fails a configurable number of times, then succeeds.
    struct FailNThenSucceedTool {
        fail_count: AtomicU32,
        remaining_failures: AtomicU32,
        error_kind: ToolError,
        domain: ToolDomain,
        custom_retry_config: Option<ToolRetryConfig>,
    }

    impl FailNThenSucceedTool {
        fn new(failures: u32, error: ToolError) -> Self {
            Self {
                fail_count: AtomicU32::new(0),
                remaining_failures: AtomicU32::new(failures),
                error_kind: error,
                domain: ToolDomain::Orchestrator,
                custom_retry_config: None,
            }
        }

        fn with_domain(mut self, domain: ToolDomain) -> Self {
            self.domain = domain;
            self
        }

        fn with_retry_config(mut self, config: ToolRetryConfig) -> Self {
            self.custom_retry_config = Some(config);
            self
        }

        fn call_count(&self) -> u32 {
            self.fail_count.load(Ordering::SeqCst)
        }
    }

    impl std::fmt::Debug for FailNThenSucceedTool {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("FailNThenSucceedTool").finish()
        }
    }

    #[async_trait]
    impl Tool for FailNThenSucceedTool {
        fn name(&self) -> &str {
            "fail_n_tool"
        }
        fn description(&self) -> &str {
            "Test tool"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            self.fail_count.fetch_add(1, Ordering::SeqCst);
            let remaining = self.remaining_failures.load(Ordering::SeqCst);
            if remaining > 0 {
                self.remaining_failures.fetch_sub(1, Ordering::SeqCst);
                // Re-create the error based on the stored kind variant
                let err = match &self.error_kind {
                    ToolError::RateLimited(d) => ToolError::RateLimited(*d),
                    ToolError::ExternalService(s) => ToolError::ExternalService(s.clone()),
                    ToolError::Timeout(d) => ToolError::Timeout(*d),
                    ToolError::Sandbox(s) => ToolError::Sandbox(s.clone()),
                    ToolError::InvalidParameters(s) => ToolError::InvalidParameters(s.clone()),
                    ToolError::ExecutionFailed(s) => ToolError::ExecutionFailed(s.clone()),
                    ToolError::NotAuthorized(s) => ToolError::NotAuthorized(s.clone()),
                };
                Err(err)
            } else {
                Ok(ToolOutput::text("ok", Duration::from_millis(1)))
            }
        }

        fn domain(&self) -> ToolDomain {
            self.domain
        }

        fn retry_config(&self) -> Option<ToolRetryConfig> {
            self.custom_retry_config.clone()
        }
    }

    fn fast_config(max_retries: u32) -> ToolRetryConfig {
        ToolRetryConfig {
            max_retries,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(100),
        }
    }

    #[tokio::test]
    async fn test_succeeds_first_try() {
        let tool = FailNThenSucceedTool::new(0, ToolError::ExternalService("err".into()));
        let ctx = JobContext::default();
        let config = fast_config(3);

        let outcome = retry_tool_execute(
            &tool,
            &serde_json::json!({}),
            &ctx,
            &config,
            Duration::from_secs(60),
        )
        .await;

        assert!(outcome.result.is_ok());
        assert_eq!(outcome.retry_attempts, 0);
        assert_eq!(tool.call_count(), 1);
    }

    #[tokio::test]
    async fn test_transient_retries_then_succeeds() {
        let tool = FailNThenSucceedTool::new(2, ToolError::ExternalService("503".into()));
        let ctx = JobContext::default();
        let config = fast_config(5);

        let outcome = retry_tool_execute(
            &tool,
            &serde_json::json!({}),
            &ctx,
            &config,
            Duration::from_secs(60),
        )
        .await;

        assert!(outcome.result.is_ok());
        assert_eq!(outcome.retry_attempts, 2);
        assert_eq!(tool.call_count(), 3); // 2 failures + 1 success
    }

    #[tokio::test]
    async fn test_permanent_fails_immediately() {
        let tool = FailNThenSucceedTool::new(10, ToolError::InvalidParameters("bad".into()));
        let ctx = JobContext::default();
        let config = fast_config(5);

        let outcome = retry_tool_execute(
            &tool,
            &serde_json::json!({}),
            &ctx,
            &config,
            Duration::from_secs(60),
        )
        .await;

        assert!(outcome.result.is_err());
        assert_eq!(outcome.retry_attempts, 0);
        assert_eq!(tool.call_count(), 1); // Only 1 call — no retries for permanent
    }

    #[tokio::test]
    async fn test_max_retries_exhausted() {
        let tool = FailNThenSucceedTool::new(100, ToolError::ExternalService("always fail".into()));
        let ctx = JobContext::default();
        let config = fast_config(2);

        let outcome = retry_tool_execute(
            &tool,
            &serde_json::json!({}),
            &ctx,
            &config,
            Duration::from_secs(60),
        )
        .await;

        assert!(outcome.result.is_err());
        assert_eq!(tool.call_count(), 3); // 1 initial + 2 retries
    }

    #[tokio::test]
    async fn test_rate_limited_uses_suggested_delay() {
        let suggested = Duration::from_millis(50);
        let tool = FailNThenSucceedTool::new(1, ToolError::RateLimited(Some(suggested)));
        let ctx = JobContext::default();
        let config = ToolRetryConfig {
            max_retries: 3,
            base_delay: Duration::from_secs(10), // much larger than suggested
            max_delay: Duration::from_secs(30),
        };

        let start = Instant::now();
        let outcome = retry_tool_execute(
            &tool,
            &serde_json::json!({}),
            &ctx,
            &config,
            Duration::from_secs(60),
        )
        .await;
        let elapsed = start.elapsed();

        assert!(outcome.result.is_ok());
        assert_eq!(outcome.retry_attempts, 1);
        // Should have used the suggested 50ms delay, not the 10s base delay
        assert!(
            elapsed < Duration::from_secs(5),
            "Expected fast retry with suggested delay, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_budget_stops_early() {
        let tool = FailNThenSucceedTool::new(100, ToolError::ExternalService("fail".into()));
        let ctx = JobContext::default();
        // Large base delay relative to the budget
        let config = ToolRetryConfig {
            max_retries: 10,
            base_delay: Duration::from_secs(5),
            max_delay: Duration::from_secs(30),
        };

        // Budget is tiny — should stop after 1st attempt since delay > remaining
        let outcome = retry_tool_execute(
            &tool,
            &serde_json::json!({}),
            &ctx,
            &config,
            Duration::from_millis(100),
        )
        .await;

        assert!(outcome.result.is_err());
        // Should not have retried many times — budget too small
        assert!(tool.call_count() <= 2);
    }

    #[test]
    fn test_effective_config_container_tool() {
        let tool = FailNThenSucceedTool::new(0, ToolError::ExternalService("x".into()))
            .with_domain(ToolDomain::Container);
        let config = effective_retry_config(&tool);
        assert_eq!(config.max_retries, 2); // sandbox preset
    }

    #[test]
    fn test_effective_config_orchestrator_tool() {
        let tool = FailNThenSucceedTool::new(0, ToolError::ExternalService("x".into()))
            .with_domain(ToolDomain::Orchestrator);
        let config = effective_retry_config(&tool);
        assert_eq!(config.max_retries, 5); // default
    }

    #[test]
    fn test_effective_config_tool_override() {
        let custom = ToolRetryConfig {
            max_retries: 10,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(60),
        };
        let tool = FailNThenSucceedTool::new(0, ToolError::ExternalService("x".into()))
            .with_domain(ToolDomain::Container)
            .with_retry_config(custom);
        let config = effective_retry_config(&tool);
        assert_eq!(config.max_retries, 10); // custom wins over sandbox preset
    }

    // --- Arc-wrapped tools work too (common in production) ---

    #[tokio::test]
    async fn test_works_with_arc() {
        let tool: Arc<dyn Tool> = Arc::new(FailNThenSucceedTool::new(
            1,
            ToolError::ExternalService("err".into()),
        ));
        let ctx = JobContext::default();
        let config = fast_config(3);

        let outcome = retry_tool_execute(
            tool.as_ref(),
            &serde_json::json!({}),
            &ctx,
            &config,
            Duration::from_secs(60),
        )
        .await;

        assert!(outcome.result.is_ok());
        assert_eq!(outcome.retry_attempts, 1);
    }
}
