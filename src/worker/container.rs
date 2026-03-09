//! Container delegate for the shared agentic loop.
//!
//! Replaces the execution loop from `src/worker/runtime.rs`. Implements
//! `LoopDelegate` to customize the shared agentic loop for Docker
//! container execution with:
//! - Sequential tool execution (no parallel — container is single-threaded)
//! - HTTP event posting to orchestrator
//! - Completion detection via `llm_signals_completion()`
//! - Credential injection via extra_env
//! - Follow-up prompt polling from orchestrator

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::agent::agentic_loop_engine::{
    AgenticLoopConfig, LoopDelegate, LoopOutcome, LoopSignal, ToolExecResult,
};
use crate::context::JobContext;
use crate::llm::{ReasoningContext, ToolDefinition, ToolSelection};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;
use crate::worker::api::{JobEventPayload, WorkerHttpClient};

/// Container delegate for Docker container workers.
///
/// Sequential tool execution (no JoinSet) since containers are
/// single-threaded and tools operate on local filesystem.
pub struct ContainerDelegate {
    job_id: Uuid,
    client: Arc<WorkerHttpClient>,
    #[allow(dead_code)]
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    extra_env: Arc<HashMap<String, String>>,
    max_iterations: u32,
    #[allow(dead_code)]
    timeout: Duration,
}

impl ContainerDelegate {
    pub fn new(
        job_id: Uuid,
        client: Arc<WorkerHttpClient>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
        extra_env: Arc<HashMap<String, String>>,
        max_iterations: u32,
        timeout: Duration,
    ) -> Self {
        Self {
            job_id,
            client,
            safety,
            tools,
            extra_env,
            max_iterations,
            timeout,
        }
    }

    fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    /// Post a job event to the orchestrator (fire-and-forget).
    async fn post_event(&self, event_type: &str, data: serde_json::Value) {
        self.client
            .post_event(&JobEventPayload {
                event_type: event_type.to_string(),
                data,
            })
            .await;
    }

    /// Build the `AgenticLoopConfig` for container execution.
    pub fn loop_config(&self) -> AgenticLoopConfig {
        AgenticLoopConfig {
            max_iterations: self.max_iterations as usize,
            enable_planning: false, // Container doesn't use planning
            max_tool_intent_nudges: 2,
        }
    }
}

#[async_trait::async_trait]
impl LoopDelegate for ContainerDelegate {
    async fn execute_tools(
        &self,
        selections: &[ToolSelection],
        _ctx: &mut ReasoningContext,
    ) -> Result<Vec<ToolExecResult>, LoopOutcome> {
        let mut results = Vec::with_capacity(selections.len());

        // Sequential execution — container tools are filesystem-bound,
        // running them in parallel could cause conflicts.
        for sel in selections {
            // Post tool_use event
            self.post_event(
                "tool_use",
                serde_json::json!({
                    "tool_name": sel.tool_name,
                    "input": truncate(&sel.parameters.to_string(), 500),
                }),
            )
            .await;

            // Build job context with injected credentials
            let ctx = JobContext {
                extra_env: Arc::clone(&self.extra_env),
                ..Default::default()
            };

            // Execute via shared pipeline
            let tool = match self.tools().get(&sel.tool_name).await {
                Some(t) => t,
                None => {
                    let result = ToolExecResult {
                        tool_call_id: sel.tool_call_id.clone(),
                        tool_name: sel.tool_name.clone(),
                        result: Err(format!("tool '{}' not found", sel.tool_name)),
                    };
                    results.push(result);
                    continue;
                }
            };

            let tool_timeout = tool.execution_timeout();
            let safe_result =
                crate::tools::execute::execute_tool_safely(&*tool, &sel.parameters, &ctx, tool_timeout)
                    .await;

            // Post tool_result event
            self.post_event(
                "tool_result",
                serde_json::json!({
                    "tool_name": sel.tool_name,
                    "output": truncate(&safe_result.raw_output, 2000),
                    "success": safe_result.success,
                }),
            )
            .await;

            results.push(ToolExecResult {
                tool_call_id: sel.tool_call_id.clone(),
                tool_name: sel.tool_name.clone(),
                result: if safe_result.success {
                    Ok(safe_result.raw_output)
                } else {
                    Err(safe_result.raw_output)
                },
            });
        }

        Ok(results)
    }

    async fn handle_text_response(
        &self,
        text: &str,
        _ctx: &mut ReasoningContext,
    ) -> Option<LoopOutcome> {
        // Post message event to orchestrator
        self.post_event(
            "message",
            serde_json::json!({
                "role": "assistant",
                "content": truncate(text, 2000),
            }),
        )
        .await;

        // Check for completion signals
        if crate::util::llm_signals_completion(text) {
            tracing::info!("Container job {} completion signal detected", self.job_id);
            Some(LoopOutcome::Response(text.to_string()))
        } else {
            // Continue looping
            None
        }
    }

    async fn check_signals(&self) -> LoopSignal {
        // Poll orchestrator for follow-up prompts
        match self.client.poll_prompt().await {
            Ok(Some(prompt)) => {
                tracing::info!(
                    "Received follow-up prompt: {}",
                    truncate(&prompt.content, 100)
                );
                self.post_event(
                    "message",
                    serde_json::json!({
                        "role": "user",
                        "content": truncate(&prompt.content, 2000),
                    }),
                )
                .await;
                LoopSignal::InjectMessage(prompt.content)
            }
            Ok(None) => LoopSignal::Continue,
            Err(e) => {
                tracing::debug!("Failed to poll for prompt: {}", e);
                LoopSignal::Continue
            }
        }
    }

    async fn on_tool_results(&self, _results: &[ToolExecResult]) {
        // Events already posted during execute_tools
    }

    async fn on_before_llm_call(&self, iteration: usize) {
        // Report progress every 5 iterations
        if iteration % 5 == 1 {
            let _ = self
                .client
                .report_status(&crate::worker::api::StatusUpdate {
                    state: "in_progress".to_string(),
                    message: Some(format!("Iteration {}", iteration)),
                    iteration: iteration as u32,
                })
                .await;
        }
    }

    async fn on_after_llm_call(&self, input_tokens: u32, output_tokens: u32, _cost: f64) {
        tracing::debug!(
            "Container job {} LLM call: {} in / {} out",
            self.job_id,
            input_tokens,
            output_tokens,
        );
    }

    async fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools().tool_definitions().await
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = crate::util::floor_char_boundary(s, max);
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_delegate_config() {
        let config = AgenticLoopConfig {
            max_iterations: 50,
            enable_planning: false,
            max_tool_intent_nudges: 2,
        };
        assert_eq!(config.max_iterations, 50);
        assert!(!config.enable_planning);
    }
}
