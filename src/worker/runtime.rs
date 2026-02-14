//! Worker runtime: the main execution loop inside a container.
//!
//! Reuses the existing `Reasoning` and `SafetyLayer` infrastructure but
//! connects to the orchestrator for LLM calls instead of calling APIs directly.
//! Streams real-time events (message, tool_use, tool_result, result) through
//! the orchestrator's job event pipeline for UI visibility.

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::config::SafetyConfig;
use crate::context::JobContext;
use crate::error::WorkerError;
use crate::llm::{
    ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult, ToolSelection,
};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;
use crate::worker::api::{CompletionReport, JobEventPayload, StatusUpdate, WorkerHttpClient};
use crate::worker::proxy_llm::ProxyLlmProvider;

/// Configuration for the worker runtime.
pub struct WorkerConfig {
    pub job_id: Uuid,
    pub orchestrator_url: String,
    pub max_iterations: u32,
    pub timeout: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            job_id: Uuid::nil(),
            orchestrator_url: String::new(),
            max_iterations: 50,
            timeout: Duration::from_secs(600),
        }
    }
}

/// The worker runtime runs inside a Docker container.
///
/// It connects to the orchestrator over HTTP, fetches its job description,
/// then runs a tool execution loop until the job is complete. Events are
/// streamed to the orchestrator so the UI can show real-time progress.
pub struct WorkerRuntime {
    config: WorkerConfig,
    client: Arc<WorkerHttpClient>,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
}

impl WorkerRuntime {
    /// Create a new worker runtime.
    ///
    /// Reads `IRONCLAW_WORKER_TOKEN` from the environment for auth.
    pub fn new(config: WorkerConfig) -> Result<Self, WorkerError> {
        let client = Arc::new(WorkerHttpClient::from_env(
            config.orchestrator_url.clone(),
            config.job_id,
        )?);

        let llm: Arc<dyn LlmProvider> = Arc::new(ProxyLlmProvider::new(
            Arc::clone(&client),
            "proxied".to_string(),
        ));

        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        }));

        let tools = Arc::new(ToolRegistry::new());
        // Register only container-safe tools
        tools.register_container_tools();

        Ok(Self {
            config,
            client,
            llm,
            safety,
            tools,
        })
    }

    /// Run the worker until the job is complete or an error occurs.
    pub async fn run(self) -> Result<(), WorkerError> {
        tracing::info!("Worker starting for job {}", self.config.job_id);

        // Fetch job description from orchestrator
        let job = self.client.get_job().await?;

        tracing::info!(
            "Received job: {} - {}",
            job.title,
            truncate(&job.description, 100)
        );

        // Report that we're starting
        self.client
            .report_status(&StatusUpdate {
                state: "in_progress".to_string(),
                message: Some("Worker started, beginning execution".to_string()),
                iteration: 0,
            })
            .await?;

        // Create reasoning engine
        let reasoning = Reasoning::new(self.llm.clone(), self.safety.clone());

        // Build initial context
        let mut reason_ctx = ReasoningContext::new().with_job(&job.description);

        reason_ctx.messages.push(ChatMessage::system(format!(
            r#"You are an autonomous agent running inside a Docker container.

Job: {}
Description: {}

You have tools for shell commands, file operations, and code editing.
Work independently to complete this job. Report when done."#,
            job.title, job.description
        )));

        // Run with timeout
        let result = tokio::time::timeout(self.config.timeout, async {
            self.execution_loop(&reasoning, &mut reason_ctx).await
        })
        .await;

        match result {
            Ok(Ok(output)) => {
                tracing::info!("Worker completed job {} successfully", self.config.job_id);
                self.post_event(
                    "result",
                    serde_json::json!({
                        "success": true,
                        "message": truncate(&output, 2000),
                    }),
                )
                .await;
                self.client
                    .report_complete(&CompletionReport {
                        success: true,
                        message: Some(output),
                        iterations: 0,
                    })
                    .await?;
            }
            Ok(Err(e)) => {
                tracing::error!("Worker failed for job {}: {}", self.config.job_id, e);
                self.post_event(
                    "result",
                    serde_json::json!({
                        "success": false,
                        "message": format!("Execution failed: {}", e),
                    }),
                )
                .await;
                self.client
                    .report_complete(&CompletionReport {
                        success: false,
                        message: Some(format!("Execution failed: {}", e)),
                        iterations: 0,
                    })
                    .await?;
            }
            Err(_) => {
                tracing::warn!("Worker timed out for job {}", self.config.job_id);
                self.post_event(
                    "result",
                    serde_json::json!({
                        "success": false,
                        "message": "Execution timed out",
                    }),
                )
                .await;
                self.client
                    .report_complete(&CompletionReport {
                        success: false,
                        message: Some("Execution timed out".to_string()),
                        iterations: 0,
                    })
                    .await?;
            }
        }

        Ok(())
    }

    async fn execution_loop(
        &self,
        reasoning: &Reasoning,
        reason_ctx: &mut ReasoningContext,
    ) -> Result<String, WorkerError> {
        let max_iterations = self.config.max_iterations;
        let mut last_output = String::new();

        // Load tool definitions
        reason_ctx.available_tools = self.tools.tool_definitions().await;

        for iteration in 1..=max_iterations {
            // Report progress
            if iteration % 5 == 1 {
                let _ = self
                    .client
                    .report_status(&StatusUpdate {
                        state: "in_progress".to_string(),
                        message: Some(format!("Iteration {}", iteration)),
                        iteration,
                    })
                    .await;
            }

            // Poll for follow-up prompts from the user
            self.poll_and_inject_prompt(reason_ctx).await;

            // Refresh tools (in case WASM tools were built)
            reason_ctx.available_tools = self.tools.tool_definitions().await;

            // Ask the LLM what to do next
            let selections = reasoning.select_tools(reason_ctx).await.map_err(|e| {
                WorkerError::ExecutionFailed {
                    reason: format!("tool selection failed: {}", e),
                }
            })?;

            if selections.is_empty() {
                // No tools selected, try direct response
                let respond_result =
                    reasoning
                        .respond_with_tools(reason_ctx)
                        .await
                        .map_err(|e| WorkerError::ExecutionFailed {
                            reason: format!("respond_with_tools failed: {}", e),
                        })?;

                match respond_result.result {
                    RespondResult::Text(response) => {
                        self.post_event(
                            "message",
                            serde_json::json!({
                                "role": "assistant",
                                "content": truncate(&response, 2000),
                            }),
                        )
                        .await;

                        if crate::util::llm_signals_completion(&response) {
                            if last_output.is_empty() {
                                last_output = response.clone();
                            }
                            return Ok(last_output);
                        }
                        reason_ctx.messages.push(ChatMessage::assistant(&response));
                    }
                    RespondResult::ToolCalls {
                        tool_calls,
                        content,
                    } => {
                        if let Some(ref text) = content {
                            self.post_event(
                                "message",
                                serde_json::json!({
                                    "role": "assistant",
                                    "content": truncate(text, 2000),
                                }),
                            )
                            .await;
                        }

                        // Add assistant message with tool_calls (OpenAI protocol)
                        reason_ctx
                            .messages
                            .push(ChatMessage::assistant_with_tool_calls(
                                content,
                                tool_calls.clone(),
                            ));

                        for tc in tool_calls {
                            self.post_event(
                                "tool_use",
                                serde_json::json!({
                                    "tool_name": tc.name,
                                    "input": truncate(&tc.arguments.to_string(), 500),
                                }),
                            )
                            .await;

                            let result = self.execute_tool(&tc.name, &tc.arguments).await;

                            self.post_event(
                                "tool_result",
                                serde_json::json!({
                                    "tool_name": tc.name,
                                    "output": match &result {
                                        Ok(output) => truncate(output, 2000),
                                        Err(e) => format!("Error: {}", truncate(e, 500)),
                                    },
                                    "success": result.is_ok(),
                                }),
                            )
                            .await;

                            if let Ok(ref output) = result {
                                last_output = output.clone();
                            }
                            let selection = ToolSelection {
                                tool_name: tc.name.clone(),
                                parameters: tc.arguments.clone(),
                                reasoning: String::new(),
                                alternatives: vec![],
                                tool_call_id: tc.id.clone(),
                            };
                            self.process_result(reason_ctx, &selection, result);
                        }
                    }
                }
            } else {
                // Execute selected tools
                for selection in &selections {
                    self.post_event(
                        "tool_use",
                        serde_json::json!({
                            "tool_name": selection.tool_name,
                            "input": truncate(&selection.parameters.to_string(), 500),
                        }),
                    )
                    .await;

                    let result = self
                        .execute_tool(&selection.tool_name, &selection.parameters)
                        .await;

                    self.post_event(
                        "tool_result",
                        serde_json::json!({
                            "tool_name": selection.tool_name,
                            "output": match &result {
                                Ok(output) => truncate(output, 2000),
                                Err(e) => format!("Error: {}", truncate(e, 500)),
                            },
                            "success": result.is_ok(),
                        }),
                    )
                    .await;

                    if let Ok(ref output) = result {
                        last_output = output.clone();
                    }

                    let completed = self.process_result(reason_ctx, selection, result);
                    if completed {
                        return Ok(last_output);
                    }
                }
            }

            // Brief pause between iterations
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Err(WorkerError::ExecutionFailed {
            reason: format!("max iterations ({}) exceeded", max_iterations),
        })
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<String, String> {
        let tool = match self.tools.get(tool_name).await {
            Some(t) => t,
            None => return Err(format!("tool '{}' not found", tool_name)),
        };

        let ctx = JobContext::default();

        // Validate params
        let validation = self.safety.validator().validate_tool_params(params);
        if !validation.is_valid {
            let details = validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!("invalid parameters: {}", details));
        }

        // Execute with per-tool timeout
        let tool_timeout = tool.execution_timeout();
        let result = tokio::time::timeout(tool_timeout, tool.execute(params.clone(), &ctx)).await;

        match result {
            Ok(Ok(output)) => serde_json::to_string_pretty(&output.result)
                .map_err(|e| format!("serialization error: {}", e)),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("tool execution timed out".to_string()),
        }
    }

    /// Process a tool result into the reasoning context. Returns true if the job is complete.
    fn process_result(
        &self,
        reason_ctx: &mut ReasoningContext,
        selection: &ToolSelection,
        result: Result<String, String>,
    ) -> bool {
        match result {
            Ok(output) => {
                let sanitized = self
                    .safety
                    .sanitize_tool_output(&selection.tool_name, &output);
                let wrapped = self.safety.wrap_for_llm(
                    &selection.tool_name,
                    &sanitized.content,
                    sanitized.was_modified,
                );

                reason_ctx.messages.push(ChatMessage::tool_result(
                    &selection.tool_call_id,
                    &selection.tool_name,
                    wrapped,
                ));

                // Tool output should never signal job completion. Only the LLM's
                // natural language response should decide when a job is done. A
                // tool could return text containing "TASK_COMPLETE" in its output
                // (e.g. from file contents) and trigger a false positive.
                false
            }
            Err(e) => {
                tracing::warn!("Tool {} failed: {}", selection.tool_name, e);
                reason_ctx.messages.push(ChatMessage::tool_result(
                    &selection.tool_call_id,
                    &selection.tool_name,
                    format!("Error: {}", e),
                ));
                false
            }
        }
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

    /// Poll the orchestrator for a follow-up prompt. If one is available,
    /// inject it as a user message into the reasoning context.
    async fn poll_and_inject_prompt(&self, reason_ctx: &mut ReasoningContext) {
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
                reason_ctx.messages.push(ChatMessage::user(&prompt.content));
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!("Failed to poll for prompt: {}", e);
            }
        }
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
    use crate::worker::runtime::truncate;

    #[test]
    fn test_truncate_within_limit() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_at_limit() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_beyond_limit() {
        let result = truncate("hello world", 5);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_truncate_multibyte_safe() {
        // "é" is 2 bytes in UTF-8; slicing at byte 1 would panic without safety
        let result = truncate("é is fancy", 1);
        // Should truncate to 0 chars (can't fit "é" in 1 byte)
        assert_eq!(result, "...");
    }
}
