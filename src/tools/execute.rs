//! Shared tool execution pipeline.
//!
//! Provides a single implementation of the validate → timeout → execute → serialize
//! pipeline used by all agentic loop consumers (chat, job, container) and the
//! scheduler's subtask execution.

use crate::context::JobContext;
use crate::error::Error;
use crate::llm::ChatMessage;
use crate::safety::SafetyLayer;
use crate::tools::{ToolRegistry, redact_params};

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
    let tool = tools
        .get(tool_name)
        .await
        .ok_or_else(|| crate::error::ToolError::NotFound {
            name: tool_name.to_string(),
        })?;

    // Validate tool parameters
    let validation = safety.validator().validate_tool_params(params);
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

    let safe_params = redact_params(params, tool.sensitive_params());
    tracing::debug!(
        tool = %tool_name,
        params = %safe_params,
        "Tool call started"
    );

    // Execute with per-tool timeout
    let timeout = tool.execution_timeout();
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(timeout, async {
        tool.execute(params.clone(), job_ctx).await
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
