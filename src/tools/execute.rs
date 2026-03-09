//! Shared tool execution pipeline.
//!
//! Replaces the 4 copies of validate → timeout → execute → serialize that
//! existed in `dispatcher.rs`, `worker.rs`, `runtime.rs`, and `scheduler.rs`.

use std::time::Duration;

use crate::context::JobContext;
use crate::tools::Tool;

/// Result of executing a tool through the shared pipeline.
pub struct SafeToolResult {
    /// Raw tool output (before sanitization).
    pub raw_output: String,
    /// Whether the tool execution succeeded.
    pub success: bool,
    /// Execution duration.
    pub duration: Duration,
}

/// Execute a tool with timeout and serialize the result.
///
/// This is the single source of truth for tool execution. All consumers
/// (dispatcher, worker, container, scheduler) should use this function
/// instead of rolling their own timeout → execute → serialize.
///
/// Note: parameter validation is handled separately via
/// `safety.validator().validate_tool_params()` because it requires
/// the SafetyLayer, which is consumer-specific.
pub async fn execute_tool_safely(
    tool: &dyn Tool,
    params: &serde_json::Value,
    job_ctx: &JobContext,
    timeout: Duration,
) -> SafeToolResult {
    let start = std::time::Instant::now();

    // Execute with timeout
    let result = tokio::time::timeout(timeout, tool.execute(params.clone(), job_ctx)).await;

    let duration = start.elapsed();

    match result {
        Ok(Ok(output)) => {
            // Serialize output
            let serialized = serde_json::to_string_pretty(&output.result)
                .unwrap_or_else(|_| output.result.to_string());
            SafeToolResult {
                raw_output: serialized,
                success: true,
                duration,
            }
        }
        Ok(Err(e)) => SafeToolResult {
            raw_output: format!("Tool error: {}", e),
            success: false,
            duration,
        },
        Err(_) => SafeToolResult {
            raw_output: format!(
                "Tool execution timed out after {} seconds",
                timeout.as_secs()
            ),
            success: false,
            duration,
        },
    }
}

/// Sanitize tool output and wrap it for inclusion in the LLM context.
///
/// Replaces the 3 copies of sanitize → wrap → ChatMessage that existed in
/// `dispatcher.rs`, `worker.rs`, and `runtime.rs`.
pub fn process_tool_output(
    safety: &crate::safety::SafetyLayer,
    tool_name: &str,
    tool_call_id: &str,
    raw_output: &str,
) -> crate::llm::ChatMessage {
    let sanitized = safety.sanitize_tool_output(tool_name, raw_output);
    let wrapped = safety.wrap_for_llm(tool_name, &sanitized.content, sanitized.was_modified);
    crate::llm::ChatMessage::tool_result(tool_call_id, tool_name, wrapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_tool_result_fields() {
        let result = SafeToolResult {
            raw_output: "test output".to_string(),
            success: true,
            duration: Duration::from_millis(42),
        };
        assert!(result.success);
        assert_eq!(result.raw_output, "test output");
        assert_eq!(result.duration.as_millis(), 42);
    }
}
