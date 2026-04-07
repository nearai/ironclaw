//! Channel-agnostic tool dispatch with audit trail.
//!
//! `ToolDispatcher` is the universal entry point for executing tools from
//! any non-agent caller — gateway handlers, CLI commands, routine engines,
//! or other channels. It creates a fresh system job for FK integrity,
//! executes the tool, records an `ActionRecord`, and returns the result.
//!
//! This is a third entry point alongside:
//! - v1: `Worker::execute_tool()` (agent agentic loop — has its own sequence tracking)
//! - v2: `EffectBridgeAdapter::execute_action()` (engine Python orchestrator)
//!
//! All three converge on the same `ToolRegistry`. Agent-initiated tool calls
//! must go through the agent's worker (which manages action sequence numbers
//! atomically); the dispatcher is only for callers that don't have an
//! existing agent job context.

use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, warn};
use uuid::Uuid;

use crate::context::{ActionRecord, JobContext};
use crate::db::Database;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{ToolError, ToolOutput};
use crate::tools::{prepare_tool_params, redact_params};
use ironclaw_safety::SafetyLayer;

/// Identifies where a tool dispatch originated.
///
/// `Channel` is intentionally a `String`, not an enum — channels are
/// extensions that can appear at runtime (gateway, CLI, telegram, slack,
/// WASM channels, future custom channels). Each dispatch creates a fresh
/// system job for audit trail purposes; agent-initiated tool calls must
/// use `Worker::execute_tool()` instead, which manages sequence numbers
/// against the agent's existing job.
#[derive(Debug, Clone)]
pub enum DispatchSource {
    /// A channel-initiated operation (gateway, CLI, telegram, etc.).
    Channel(String),
    /// A routine engine operation.
    Routine { routine_id: Uuid },
    /// An internal system operation.
    System,
}

impl std::fmt::Display for DispatchSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Channel(name) => write!(f, "channel:{name}"),
            Self::Routine { routine_id } => write!(f, "routine:{routine_id}"),
            Self::System => write!(f, "system"),
        }
    }
}

/// Channel-agnostic tool dispatcher with audit trail.
///
/// Wraps `ToolRegistry` + `SafetyLayer` + `Database` to provide a single
/// dispatch function that any caller can use to execute tools with the
/// same safety pipeline as the agent worker (param normalization, schema
/// validation, sensitive-param redaction, per-tool timeout, output
/// sanitization) plus `ActionRecord` persistence.
pub struct ToolDispatcher {
    registry: Arc<ToolRegistry>,
    safety: Arc<SafetyLayer>,
    store: Arc<dyn Database>,
}

impl ToolDispatcher {
    /// Create a new dispatcher.
    pub fn new(
        registry: Arc<ToolRegistry>,
        safety: Arc<SafetyLayer>,
        store: Arc<dyn Database>,
    ) -> Self {
        Self {
            registry,
            safety,
            store,
        }
    }

    /// Execute a tool by name with the given parameters.
    ///
    /// Pipeline (mirrors `Worker::execute_tool`):
    /// 1. Resolve the tool from the registry
    /// 2. Normalize parameters via `prepare_tool_params`
    /// 3. Validate parameters via `SafetyLayer::validator()`
    /// 4. Redact sensitive parameters for logging and audit
    /// 5. Create a fresh system job for FK integrity
    /// 6. Execute with the tool's per-tool timeout
    /// 7. Sanitize the result via `SafetyLayer::sanitize_tool_output`
    /// 8. Persist an `ActionRecord` with redacted params and sanitized output
    /// 9. Return the original `ToolOutput`
    ///
    /// Approval checks are skipped — channel-initiated operations are
    /// user-confirmed by definition. Audit-trail persistence failures are
    /// logged via `tracing::warn!` but do not mask the tool result.
    pub async fn dispatch(
        &self,
        tool_name: &str,
        params: serde_json::Value,
        user_id: &str,
        source: DispatchSource,
    ) -> Result<ToolOutput, ToolError> {
        let (resolved_name, tool) =
            self.registry.get_resolved(tool_name).await.ok_or_else(|| {
                ToolError::ExecutionFailed(format!("tool not found: {tool_name}"))
            })?;

        // 1. Normalize parameters (coerce types, fill defaults).
        let normalized_params = prepare_tool_params(tool.as_ref(), &params);

        // 2. Schema validation.
        let validation = self
            .safety
            .validator()
            .validate_tool_params(&normalized_params);
        if !validation.is_valid {
            let details = validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(ToolError::InvalidParameters(format!(
                "Invalid tool parameters: {details}"
            )));
        }

        // 3. Redact sensitive params for log + audit. Sensitive values are
        //    still passed to the tool itself (via normalized_params), but
        //    never appear in the audit row or the dispatch log.
        let safe_params = redact_params(&normalized_params, tool.sensitive_params());

        // 4. Create a fresh system job for audit trail. Each dispatch
        //    becomes its own group of actions — sequence_num starts at 0
        //    with no risk of UNIQUE(job_id, sequence_num) collision.
        let source_label = source.to_string();
        let job_id = self
            .store
            .create_system_job(user_id, &source_label)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to create system job: {e}")))?;

        let ctx = JobContext::system(user_id, job_id);
        let start = Instant::now();

        debug!(
            tool = %resolved_name,
            source = %source,
            user_id = %user_id,
            params = %safe_params,
            "dispatching tool"
        );

        // 5. Execute with per-tool timeout.
        let timeout = tool.execution_timeout();
        let result = tokio::time::timeout(timeout, tool.execute(normalized_params, &ctx)).await;
        let elapsed = start.elapsed();

        let final_result: Result<ToolOutput, ToolError> = match result {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(ToolError::Timeout(timeout)),
        };

        // 6. Build the ActionRecord with sanitized output (mirrors worker pattern).
        let action = ActionRecord::new(0, &resolved_name, safe_params);
        let action = match &final_result {
            Ok(output) => {
                let sanitized = serde_json::to_string_pretty(&output.result)
                    .ok()
                    .map(|s| self.safety.sanitize_tool_output(&resolved_name, &s).content);
                action.succeed(sanitized, output.result.clone(), elapsed)
            }
            Err(e) => action.fail(e.to_string(), elapsed),
        };

        // 7. Persist the audit record. Awaited (not spawned) so short-lived
        //    callers (CLI commands) cannot terminate before the row is written.
        if let Err(e) = self.store.save_action(job_id, &action).await {
            warn!(
                error = %e,
                tool = %resolved_name,
                job_id = %job_id,
                "failed to persist dispatch ActionRecord"
            );
        }

        final_result
    }

    /// Access the underlying tool registry.
    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_source_display() {
        assert_eq!(
            DispatchSource::Channel("gateway".into()).to_string(),
            "channel:gateway"
        );
        let id = Uuid::nil();
        assert_eq!(
            DispatchSource::Routine { routine_id: id }.to_string(),
            format!("routine:{id}")
        );
        assert_eq!(DispatchSource::System.to_string(), "system");
    }
}
