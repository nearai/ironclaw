//! Tool executor for programmatic tool calling (PTC).
//!
//! Provides a standalone execution engine that can be used by both the
//! Docker HTTP RPC path (orchestrator endpoint) and the WASM host function
//! path (tool_invoke). Extracts the tool dispatch flow into a reusable struct.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::context::JobContext;
use crate::safety::SafetyLayer;
use crate::tools::registry::ToolRegistry;

/// Maximum allowed nesting depth for tool-invokes-tool chains.
pub const MAX_NESTING_DEPTH: u32 = 5;

/// Maximum per-call timeout (5 minutes).
const MAX_TIMEOUT_SECS: u64 = 300;

/// Result of a programmatic tool call.
#[derive(Debug, Clone)]
pub struct PtcToolResult {
    /// Tool output (potentially sanitized).
    pub output: String,
    /// Whether the output was modified by the safety layer.
    pub was_sanitized: bool,
    /// Wall-clock duration of the tool execution.
    pub duration: Duration,
}

/// Errors that can occur during programmatic tool execution.
#[derive(Debug, thiserror::Error)]
pub enum PtcError {
    #[error("Tool not found: {name}")]
    NotFound { name: String },

    #[error("Tool execution failed: {name}: {reason}")]
    ExecutionFailed { name: String, reason: String },

    #[error("Tool execution timed out: {name} (timeout: {timeout:?})")]
    Timeout { name: String, timeout: Duration },

    #[error("Invalid parameters for tool {name}: {reason}")]
    InvalidParameters { name: String, reason: String },

    #[error("Tool {name} is rate limited")]
    RateLimited { name: String },

    #[error("Tool output blocked by safety layer: {reason}")]
    SafetyBlocked { reason: String },

    #[error("Nesting depth exceeded (max {max})")]
    NestingDepthExceeded { max: u32 },
}

/// Standalone tool execution engine for programmatic tool calling.
///
/// Used by:
/// - The orchestrator's `POST /worker/{job_id}/tools/call` endpoint
/// - The WASM `tool_invoke` host function
pub struct ToolExecutor {
    tools: Arc<ToolRegistry>,
    safety: Arc<SafetyLayer>,
    default_timeout: Duration,
}

impl ToolExecutor {
    /// Create a new tool executor.
    pub fn new(
        tools: Arc<ToolRegistry>,
        safety: Arc<SafetyLayer>,
        default_timeout: Duration,
    ) -> Self {
        Self {
            tools,
            safety,
            default_timeout,
        }
    }

    /// Execute a tool by name with the given parameters.
    ///
    /// Flow: lookup -> execute with timeout -> sanitize output -> return.
    pub async fn execute(
        &self,
        tool_name: &str,
        params: serde_json::Value,
        ctx: &JobContext,
        timeout_override: Option<Duration>,
    ) -> Result<PtcToolResult, PtcError> {
        // Enforce global nesting depth limit
        if ctx.tool_nesting_depth >= MAX_NESTING_DEPTH {
            return Err(PtcError::NestingDepthExceeded {
                max: MAX_NESTING_DEPTH,
            });
        }

        let start = Instant::now();

        // Look up the tool
        let tool = self
            .tools
            .get(tool_name)
            .await
            .ok_or_else(|| PtcError::NotFound {
                name: tool_name.to_string(),
            })?;

        // Determine timeout: caller override -> tool's own timeout -> default,
        // capped at MAX_TIMEOUT_SECS.
        let timeout = timeout_override
            .unwrap_or_else(|| {
                let tool_timeout = tool.execution_timeout();
                if tool_timeout > Duration::from_secs(MAX_TIMEOUT_SECS) {
                    self.default_timeout
                } else {
                    tool_timeout
                }
            })
            .min(Duration::from_secs(MAX_TIMEOUT_SECS));

        // Execute with timeout
        let tool_result = tokio::time::timeout(timeout, tool.execute(params, ctx))
            .await
            .map_err(|_| PtcError::Timeout {
                name: tool_name.to_string(),
                timeout,
            })?
            .map_err(|e| match e {
                crate::tools::ToolError::InvalidParameters(reason) => {
                    PtcError::InvalidParameters {
                        name: tool_name.to_string(),
                        reason,
                    }
                }
                crate::tools::ToolError::RateLimited(_) => PtcError::RateLimited {
                    name: tool_name.to_string(),
                },
                other => PtcError::ExecutionFailed {
                    name: tool_name.to_string(),
                    reason: other.to_string(),
                },
            })?;

        // Get output string
        let raw_output = tool_result
            .raw
            .as_deref()
            .or_else(|| tool_result.result.as_str())
            .unwrap_or("")
            .to_string();

        let raw_output = if raw_output.is_empty() {
            serde_json::to_string(&tool_result.result).unwrap_or_default()
        } else {
            raw_output
        };

        // Sanitize output if the tool requires it
        let (output, was_sanitized) = if tool.requires_sanitization() {
            let sanitized = self.safety.sanitize_tool_output(tool_name, &raw_output);
            if sanitized.was_modified
                && sanitized.content.starts_with("[Output blocked")
            {
                return Err(PtcError::SafetyBlocked {
                    reason: sanitized.content,
                });
            }
            (sanitized.content, sanitized.was_modified)
        } else {
            (raw_output, false)
        };

        Ok(PtcToolResult {
            output,
            was_sanitized,
            duration: start.elapsed(),
        })
    }
}

impl std::fmt::Debug for ToolExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolExecutor")
            .field("default_timeout", &self.default_timeout)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::SafetyLayer;
    use crate::tools::tool::{Tool, ToolError, ToolOutput};

    fn test_safety_config() -> crate::config::SafetyConfig {
        crate::config::SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        }
    }

    struct SlowTool;

    #[async_trait::async_trait]
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            "slow_tool"
        }
        fn description(&self) -> &str {
            "A tool that sleeps"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(ToolOutput::text("done", Duration::from_secs(10)))
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn test_execute_not_found() {
        let tools = Arc::new(ToolRegistry::new());
        let safety = Arc::new(SafetyLayer::new(&test_safety_config()));
        let executor = ToolExecutor::new(tools, safety, Duration::from_secs(60));

        let ctx = JobContext::new("test", "test");
        let result = executor
            .execute("nonexistent", serde_json::json!({}), &ctx, None)
            .await;

        assert!(matches!(result, Err(PtcError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_execute_echo() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register_builtin_tools();
        let safety = Arc::new(SafetyLayer::new(&test_safety_config()));
        let executor = ToolExecutor::new(tools, safety, Duration::from_secs(60));

        let ctx = JobContext::new("test", "test");
        let result = executor
            .execute(
                "echo",
                serde_json::json!({"message": "hello"}),
                &ctx,
                None,
            )
            .await;

        assert!(result.is_ok());
        let ptc_result = result.as_ref().ok();
        assert!(ptc_result.is_some());
        assert!(ptc_result.map(|r| r.output.contains("hello")).unwrap_or(false));
    }

    #[tokio::test]
    async fn test_execute_timeout() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(SlowTool)).await;
        let safety = Arc::new(SafetyLayer::new(&test_safety_config()));
        let executor = ToolExecutor::new(tools, safety, Duration::from_secs(60));

        let ctx = JobContext::new("test", "test");
        let result = executor
            .execute(
                "slow_tool",
                serde_json::json!({}),
                &ctx,
                Some(Duration::from_millis(50)),
            )
            .await;

        assert!(matches!(result, Err(PtcError::Timeout { .. })));
    }

    #[tokio::test]
    async fn test_nesting_depth_exceeded() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register_builtin_tools();
        let safety = Arc::new(SafetyLayer::new(&test_safety_config()));
        let executor = ToolExecutor::new(tools, safety, Duration::from_secs(60));

        let mut ctx = JobContext::new("test", "test");
        ctx.tool_nesting_depth = MAX_NESTING_DEPTH; // already at max

        let result = executor
            .execute("echo", serde_json::json!({"message": "hello"}), &ctx, None)
            .await;

        assert!(matches!(
            result,
            Err(PtcError::NestingDepthExceeded { .. })
        ));
    }

    #[tokio::test]
    async fn test_nesting_depth_within_limit() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register_builtin_tools();
        let safety = Arc::new(SafetyLayer::new(&test_safety_config()));
        let executor = ToolExecutor::new(tools, safety, Duration::from_secs(60));

        let mut ctx = JobContext::new("test", "test");
        ctx.tool_nesting_depth = MAX_NESTING_DEPTH - 1; // one below max

        let result = executor
            .execute("echo", serde_json::json!({"message": "hello"}), &ctx, None)
            .await;

        assert!(result.is_ok());
    }

    struct LeakyTool;

    #[async_trait::async_trait]
    impl Tool for LeakyTool {
        fn name(&self) -> &str {
            "leaky_tool"
        }
        fn description(&self) -> &str {
            "Returns output with fake bearer token"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            // Bearer token pattern triggers LeakAction::Redact (not Block),
            // so the safety layer redacts it and returns sanitized output.
            let output =
                "Here is some data: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9_longtokenvalue end";
            Ok(ToolOutput::text(output, Duration::from_millis(1)))
        }
        fn requires_sanitization(&self) -> bool {
            true
        }
    }

    struct InvalidParamsTool;

    #[async_trait::async_trait]
    impl Tool for InvalidParamsTool {
        fn name(&self) -> &str {
            "invalid_params_tool"
        }
        fn description(&self) -> &str {
            "Always fails with InvalidParameters"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Err(ToolError::InvalidParameters("bad params".to_string()))
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn test_execute_safety_sanitization() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(LeakyTool)).await;
        let safety = Arc::new(SafetyLayer::new(&test_safety_config()));
        let executor = ToolExecutor::new(tools, safety, Duration::from_secs(60));

        let ctx = JobContext::new("test", "test");
        let result = executor
            .execute("leaky_tool", serde_json::json!({}), &ctx, None)
            .await;

        // The safety layer should detect the API key pattern and modify the output
        assert!(result.is_ok());
        let ptc_result = result.unwrap();
        assert!(
            ptc_result.was_sanitized,
            "Output with API key should be sanitized"
        );
    }

    #[tokio::test]
    async fn test_execute_invalid_params() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(InvalidParamsTool)).await;
        let safety = Arc::new(SafetyLayer::new(&test_safety_config()));
        let executor = ToolExecutor::new(tools, safety, Duration::from_secs(60));

        let ctx = JobContext::new("test", "test");
        let result = executor
            .execute("invalid_params_tool", serde_json::json!({}), &ctx, None)
            .await;

        match result {
            Err(PtcError::InvalidParameters { name, reason }) => {
                assert_eq!(name, "invalid_params_tool");
                assert!(reason.contains("bad params"));
            }
            other => panic!("Expected InvalidParameters, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_sequential_calls() {
        let tools = Arc::new(ToolRegistry::new());
        tools.register_builtin_tools();
        let safety = Arc::new(SafetyLayer::new(&test_safety_config()));
        let executor = ToolExecutor::new(tools, safety, Duration::from_secs(60));

        let ctx = JobContext::new("test", "test");

        let messages = ["alpha", "beta", "gamma"];
        for msg in &messages {
            let result = executor
                .execute("echo", serde_json::json!({"message": msg}), &ctx, None)
                .await
                .expect("echo should succeed");
            assert!(
                result.output.contains(msg),
                "Output should contain '{}'",
                msg
            );
        }
    }
}
