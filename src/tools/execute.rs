//! Shared tool execution pipeline.
//!
//! Provides a single implementation of the validate → timeout → execute → serialize
//! pipeline used by all agentic loop consumers (chat, job, container) and the
//! scheduler's subtask execution.

use crate::context::JobContext;
use crate::error::Error;
use crate::llm::ChatMessage;
use crate::safety::SafetyLayer;
use crate::tools::{ToolRegistry, prepare_tool_params, redact_params};

/// Execute a tool with safety checks: lookup → validate → timeout → execute → serialize.
///
/// This is the single canonical implementation of tool execution. All consumers
/// (chat dispatcher, job worker, container runtime, scheduler subtasks) use this
/// function instead of maintaining their own copies.
pub async fn execute_tool_with_safety(
    tools: &ToolRegistry,
    safety: &SafetyLayer,
    tool_name: &str,
    params: &serde_json::Value,
    job_ctx: &JobContext,
) -> Result<String, Error> {
    if tool_name.is_empty() {
        return Err(crate::error::ToolError::NotFound {
            name: tool_name.to_string(),
        }
        .into());
    }
    let tool = tools
        .get(tool_name)
        .await
        .ok_or_else(|| crate::error::ToolError::NotFound {
            name: tool_name.to_string(),
        })?;

    let normalized_params = prepare_tool_params(tool.as_ref(), params);

    // Validate tool parameters
    let validation = safety.validator().validate_tool_params(&normalized_params);
    if !validation.is_valid {
        let details = validation
            .errors
            .iter()
            .map(|e| format!("{}: {}", e.field, e.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(crate::error::ToolError::InvalidParameters {
            name: tool_name.to_string(),
            reason: format!("Invalid tool parameters: {}", details),
        }
        .into());
    }

    let safe_params = redact_params(&normalized_params, tool.sensitive_params());
    tracing::debug!(
        tool = %tool_name,
        params = %safe_params,
        "Tool call started"
    );

    // Execute with per-tool timeout
    let timeout = tool.execution_timeout();
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(timeout, async {
        tool.execute(normalized_params.clone(), job_ctx).await
    })
    .await;
    let elapsed = start.elapsed();

    match &result {
        Ok(Ok(output)) => {
            let result_size = serde_json::to_string(&output.result)
                .map(|s| s.len())
                .unwrap_or(0);
            tracing::debug!(
                tool = %tool_name,
                elapsed_ms = elapsed.as_millis() as u64,
                result_size_bytes = result_size,
                "Tool call succeeded"
            );
        }
        Ok(Err(e)) => {
            tracing::debug!(
                tool = %tool_name,
                elapsed_ms = elapsed.as_millis() as u64,
                error = %e,
                "Tool call failed"
            );
        }
        Err(_) => {
            tracing::debug!(
                tool = %tool_name,
                elapsed_ms = elapsed.as_millis() as u64,
                timeout_secs = timeout.as_secs(),
                "Tool call timed out"
            );
        }
    }

    let result = result
        .map_err(|_| crate::error::ToolError::Timeout {
            name: tool_name.to_string(),
            timeout,
        })?
        .map_err(|e| crate::error::ToolError::ExecutionFailed {
            name: tool_name.to_string(),
            reason: e.to_string(),
        })?;

    serde_json::to_string_pretty(&result.result).map_err(|e| {
        crate::error::ToolError::ExecutionFailed {
            name: tool_name.to_string(),
            reason: format!("Failed to serialize result: {}", e),
        }
        .into()
    })
}

/// Process a tool result into a `ChatMessage::tool_result` with safety sanitization.
///
/// On success: sanitize → wrap → ChatMessage::tool_result.
/// On error: format error → ChatMessage::tool_result.
///
/// Returns the content string and the ChatMessage.
pub fn process_tool_result(
    safety: &SafetyLayer,
    tool_name: &str,
    tool_call_id: &str,
    result: &Result<String, impl std::fmt::Display>,
) -> (String, ChatMessage) {
    let content = match result {
        Ok(output) => {
            let sanitized = safety.sanitize_tool_output(tool_name, output);
            safety.wrap_for_llm(tool_name, &sanitized.content, sanitized.was_modified)
        }
        Err(e) => format!("Error: {}", e),
    };
    let message = ChatMessage::tool_result(tool_call_id, tool_name, content.clone());
    (content, message)
}

/// Execute a tool with safety checks, returning a string error (for container runtime).
///
/// This is a thin wrapper around `execute_tool_with_safety` that converts
/// `Error` to `String` for the container runtime's simpler error model.
pub async fn execute_tool_simple(
    tools: &ToolRegistry,
    safety: &SafetyLayer,
    tool_name: &str,
    params: &serde_json::Value,
    job_ctx: &JobContext,
) -> Result<String, String> {
    execute_tool_with_safety(tools, safety, tool_name, params, job_ctx)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tool::{Tool, ToolError, ToolOutput};
    use std::sync::Arc;
    use std::time::Duration;

    struct EchoTool;

    #[async_trait::async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes input"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(params, Duration::default()))
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    struct FailTool;

    #[async_trait::async_trait]
    impl Tool for FailTool {
        fn name(&self) -> &str {
            "fail_tool"
        }
        fn description(&self) -> &str {
            "Always fails"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _: serde_json::Value,
            _: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Err(ToolError::ExecutionFailed(
                "intentional failure".to_string(),
            ))
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    struct SlowTool;

    #[async_trait::async_trait]
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            "slow_tool"
        }
        fn description(&self) -> &str {
            "Sleeps forever"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _: serde_json::Value,
            _: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            unreachable!()
        }
        fn execution_timeout(&self) -> Duration {
            Duration::from_millis(50)
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    struct ArrayEchoTool;

    #[async_trait::async_trait]
    impl Tool for ArrayEchoTool {
        fn name(&self) -> &str {
            "array_echo"
        }
        fn description(&self) -> &str {
            "Echoes normalized params"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "values": {
                        "type": "array",
                        "items": { "type": "integer" }
                    }
                }
            })
        }
        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(params, Duration::default()))
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    fn test_safety() -> SafetyLayer {
        SafetyLayer::new(&crate::config::SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        })
    }

    fn test_job_ctx() -> JobContext {
        JobContext::default()
    }

    async fn registry_with(tools: Vec<Arc<dyn Tool>>) -> ToolRegistry {
        let registry = ToolRegistry::new();
        for tool in tools {
            registry.register(tool).await;
        }
        registry
    }

    #[tokio::test]
    async fn test_execute_empty_tool_name_returns_not_found() {
        // Regression: execute_tool_with_safety must reject empty tool names before
        // even attempting a registry lookup (the debug_assert guards this invariant).
        let registry = registry_with(vec![]).await;
        let safety = test_safety();

        let result = execute_tool_with_safety(
            &registry,
            &safety,
            "",
            &serde_json::json!({}),
            &test_job_ctx(),
        )
        .await;

        assert!(result.is_err(), "Empty tool name should return an error"); // safety: test-only assertion
    }

    #[tokio::test]
    async fn test_execute_success() {
        let registry = registry_with(vec![Arc::new(EchoTool)]).await;
        let safety = test_safety();
        let params = serde_json::json!({"message": "hello"});

        let result =
            execute_tool_with_safety(&registry, &safety, "echo", &params, &test_job_ctx()).await;

        assert!(result.is_ok(), "Echo tool should succeed");
        let output = result.unwrap();
        assert!(
            output.contains("hello"),
            "Output should contain the echoed input"
        );
    }

    #[tokio::test]
    async fn test_execute_missing_tool() {
        let registry = registry_with(vec![]).await;
        let safety = test_safety();

        let result = execute_tool_with_safety(
            &registry,
            &safety,
            "nonexistent",
            &serde_json::json!({}),
            &test_job_ctx(),
        )
        .await;

        assert!(result.is_err(), "Missing tool should return error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nonexistent") || err.contains("not found"),
            "Error should mention the tool: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_execute_tool_failure() {
        let registry = registry_with(vec![Arc::new(FailTool)]).await;
        let safety = test_safety();

        let result = execute_tool_with_safety(
            &registry,
            &safety,
            "fail_tool",
            &serde_json::json!({}),
            &test_job_ctx(),
        )
        .await;

        assert!(result.is_err(), "FailTool should return error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("intentional failure"),
            "Error should contain the failure reason: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_execute_tool_timeout() {
        let registry = registry_with(vec![Arc::new(SlowTool)]).await;
        let safety = test_safety();

        let start = std::time::Instant::now();
        let result = execute_tool_with_safety(
            &registry,
            &safety,
            "slow_tool",
            &serde_json::json!({}),
            &test_job_ctx(),
        )
        .await;
        let elapsed = start.elapsed();

        assert!(result.is_err(), "SlowTool should timeout");
        let err = result.unwrap_err().to_string();
        assert!(
            err.to_lowercase().contains("timeout") || err.to_lowercase().contains("timed out"),
            "Error should mention timeout: {}",
            err
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "Should timeout quickly, not wait 60s"
        );
    }

    #[tokio::test]
    async fn test_execute_normalizes_stringified_array_params() {
        let registry = registry_with(vec![Arc::new(ArrayEchoTool)]).await;
        let safety = test_safety();

        let result = execute_tool_with_safety(
            &registry,
            &safety,
            "array_echo",
            &serde_json::json!({"values": "[\"1\", \"2\", 3]"}),
            &test_job_ctx(),
        )
        .await
        .expect("array_echo should succeed"); // safety: test-only assertion

        let output: serde_json::Value =
            serde_json::from_str(&result).expect("tool result should be valid JSON"); // safety: test-only assertion
        assert_eq!(output["values"], serde_json::json!([1, 2, 3])); // safety: test-only assertion
    }

    #[test]
    fn test_process_tool_result_success() {
        let safety = test_safety();
        let result: Result<String, String> = Ok("tool output data".to_string());

        let (content, message) = process_tool_result(&safety, "echo", "call_1", &result);

        assert!(
            content.contains("tool_output"),
            "Content should be XML-wrapped: {}",
            content
        );
        assert!(
            content.contains("tool output data"),
            "Content should contain the output: {}",
            content
        );
        assert_eq!(message.role, crate::llm::Role::Tool);
        assert_eq!(message.name.as_deref(), Some("echo"));
    }

    #[test]
    fn test_process_tool_result_error() {
        let safety = test_safety();
        let result: Result<String, String> = Err("something went wrong".to_string());

        let (content, message) = process_tool_result(&safety, "echo", "call_1", &result);

        assert!(
            content.contains("Error:"),
            "Error content should start with 'Error:': {}",
            content
        );
        assert!(
            content.contains("something went wrong"),
            "Error content should contain the message: {}",
            content
        );
        assert_eq!(message.role, crate::llm::Role::Tool);
    }
}
