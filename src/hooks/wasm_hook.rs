//! WASM plugin hook wrapper.
//!
//! Allows WASM tools to participate in the hook lifecycle by declaring
//! hook points in their `capabilities.json`. The tool receives serialized
//! hook events and returns JSON-encoded outcomes.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::hooks::hook::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint,
};
use crate::tools::Tool;

/// Wraps a WASM tool as a lifecycle hook.
///
/// When a hook event fires, the wrapper serializes the event to JSON and calls
/// the WASM tool's `execute()` with a special `__hook_event` parameter. The
/// tool's output is parsed as a JSON hook outcome:
///
/// ```json
/// {"action": "continue"}
/// {"action": "modify", "content": "..."}
/// {"action": "reject", "reason": "..."}
/// ```
///
/// If the tool doesn't return valid hook JSON, the wrapper treats it as `ok()`.
pub struct WasmHookWrapper {
    name: String,
    points: Vec<HookPoint>,
    failure_mode: HookFailureMode,
    timeout: Duration,
    tool: Arc<dyn Tool>,
}

impl WasmHookWrapper {
    /// Create a new WASM hook wrapper.
    pub fn new(
        name: String,
        points: Vec<HookPoint>,
        failure_mode: HookFailureMode,
        timeout: Duration,
        tool: Arc<dyn Tool>,
    ) -> Self {
        Self {
            name,
            points,
            failure_mode,
            timeout,
            tool,
        }
    }

    /// Parse a tool output string as a hook outcome.
    ///
    /// If the output is not valid hook JSON (missing/unknown `action` field),
    /// returns `Err` so the caller can apply the hook's `failure_mode` policy.
    fn parse_outcome(output: &str) -> Result<HookOutcome, HookError> {
        let value = serde_json::from_str::<serde_json::Value>(output).map_err(|e| {
            HookError::ExecutionFailed {
                reason: format!("WASM hook returned non-JSON output: {}", e),
            }
        })?;

        match value.get("action").and_then(|a| a.as_str()) {
            Some("continue") => {
                let content = value
                    .get("content")
                    .and_then(|c| c.as_str())
                    .map(String::from);
                Ok(HookOutcome::Continue { modified: content })
            }
            Some("modify") => {
                let content = value
                    .get("content")
                    .and_then(|c| c.as_str())
                    .map(String::from);
                Ok(HookOutcome::Continue { modified: content })
            }
            Some("reject") => {
                let reason = value
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("Rejected by WASM hook")
                    .to_string();
                Ok(HookOutcome::Reject { reason })
            }
            other => Err(HookError::ExecutionFailed {
                reason: format!(
                    "WASM hook returned unknown action: {:?}",
                    other.unwrap_or("(missing)")
                ),
            }),
        }
    }
}

#[async_trait]
impl Hook for WasmHookWrapper {
    fn name(&self) -> &str {
        &self.name
    }

    fn hook_points(&self) -> &[HookPoint] {
        &self.points
    }

    fn failure_mode(&self) -> HookFailureMode {
        self.failure_mode
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        // Serialize the event to JSON for the WASM tool
        let event_json = serde_json::to_value(event).map_err(|e| HookError::ExecutionFailed {
            reason: format!("Failed to serialize hook event: {}", e),
        })?;

        // Call the WASM tool with a special __hook_event parameter
        let params = serde_json::json!({
            "__hook_event": event_json,
        });

        // Create a minimal job context for the hook call
        let job_ctx = JobContext::default();

        let result =
            self.tool
                .execute(params, &job_ctx)
                .await
                .map_err(|e| HookError::ExecutionFailed {
                    reason: format!("WASM hook tool execution failed: {}", e),
                })?;

        // Parse the tool output as a hook outcome
        let output_str =
            serde_json::to_string(&result.result).map_err(|e| HookError::ExecutionFailed {
                reason: format!("Failed to serialize WASM hook output: {}", e),
            })?;
        Self::parse_outcome(&output_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_hook_wrapper_parses_continue() {
        let outcome = WasmHookWrapper::parse_outcome(r#"{"action": "continue"}"#).unwrap();
        assert!(matches!(outcome, HookOutcome::Continue { modified: None }));
    }

    #[test]
    fn test_wasm_hook_wrapper_parses_modify() {
        let outcome = WasmHookWrapper::parse_outcome(
            r#"{"action": "modify", "content": "sanitized content"}"#,
        )
        .unwrap();
        match outcome {
            HookOutcome::Continue {
                modified: Some(content),
            } => assert_eq!(content, "sanitized content"),
            other => panic!("Expected modify, got: {:?}", other),
        }
    }

    #[test]
    fn test_wasm_hook_wrapper_parses_reject() {
        let outcome =
            WasmHookWrapper::parse_outcome(r#"{"action": "reject", "reason": "too spicy"}"#)
                .unwrap();
        match outcome {
            HookOutcome::Reject { reason } => assert_eq!(reason, "too spicy"),
            other => panic!("Expected reject, got: {:?}", other),
        }
    }

    #[test]
    fn test_wasm_hook_wrapper_errors_on_non_json_output() {
        // Non-JSON output is now an error (respects failure_mode in registry)
        let result = WasmHookWrapper::parse_outcome("Hello, world!");
        assert!(result.is_err());
    }

    #[test]
    fn test_wasm_hook_wrapper_errors_on_unknown_action() {
        // Unknown action is now an error (respects failure_mode in registry)
        let result = WasmHookWrapper::parse_outcome(r#"{"action": "explode"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_wasm_hook_wrapper_serializes_event() {
        // Verify that HookEvent can be serialized to JSON (needed for the wrapper)
        let event = HookEvent::Inbound {
            user_id: "user-1".to_string(),
            channel: "test".to_string(),
            content: "hello".to_string(),
            thread_id: None,
        };

        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("Inbound").is_some());
        let inbound = &json["Inbound"];
        assert_eq!(inbound["user_id"], "user-1");
        assert_eq!(inbound["content"], "hello");
    }
}
