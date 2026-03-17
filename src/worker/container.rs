//! Worker runtime: the main execution loop inside a container.
//!
//! Reuses the existing `Reasoning` and `SafetyLayer` infrastructure but
//! connects to the orchestrator for LLM calls instead of calling APIs directly.
//! Streams real-time events (message, tool_use, tool_result, result) through
//! the orchestrator's job event pipeline for UI visibility.
//!
//! Uses the shared `AgenticLoop` engine via `ContainerDelegate`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::agentic_loop::{
    AgenticLoopConfig, LoopDelegate, LoopOutcome, LoopSignal, TextAction, truncate_for_preview,
};
use crate::config::SafetyConfig;
use crate::context::JobContext;
use crate::error::WorkerError;
use crate::llm::{ChatMessage, LlmProvider, Reasoning, ReasoningContext};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;
use crate::tools::execute::{execute_tool_simple, process_tool_result};
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
    /// Credentials fetched from the orchestrator, injected into child processes
    /// via `Command::envs()` rather than mutating the global process environment.
    ///
    /// Wrapped in `Arc` to avoid deep-cloning the map on every tool invocation.
    extra_env: Arc<HashMap<String, String>>,
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
            extra_env: Arc::new(HashMap::new()),
        })
    }

    /// Run the worker until the job is complete or an error occurs.
    pub async fn run(mut self) -> Result<(), WorkerError> {
        tracing::info!("Worker starting for job {}", self.config.job_id);

        // Fetch job description from orchestrator
        let job = self.client.get_job().await?;

        tracing::info!(
            "Received job: {} - {}",
            job.title,
            truncate_for_preview(&job.description, 100)
        );

        // Fetch credentials and store them for injection into child processes
        // via Command::envs() (avoids unsafe std::env::set_var in multi-threaded runtime).
        let credentials = self.client.fetch_credentials().await?;
        {
            let mut env_map = HashMap::new();
            for cred in &credentials {
                env_map.insert(cred.env_var.clone(), cred.value.clone());
            }
            self.extra_env = Arc::new(env_map);
        }
        if !credentials.is_empty() {
            tracing::info!(
                "Fetched {} credential(s) for child process injection",
                credentials.len()
            );
        }

        // Report that we're starting
        self.client
            .report_status(&StatusUpdate {
                state: "in_progress".to_string(),
                message: Some("Worker started, beginning execution".to_string()),
                iteration: 0,
            })
            .await?;

        // Create reasoning engine
        let reasoning = Reasoning::new(self.llm.clone());

        // Build initial context
        let mut reason_ctx = ReasoningContext::new().with_job(&job.description);

        reason_ctx.messages.push(ChatMessage::system(format!(
            r#"You are an autonomous agent running inside a Docker container.

Job: {}
Description: {}

You have tools for shell commands, file operations, and code editing.
Work independently to complete this job. When finished, your final message MUST include the phrase "The job is complete" to signal termination."#,
            job.title, job.description
        )));

        // Load tool definitions
        reason_ctx.available_tools = self.tools.tool_definitions().await;

        // Shared iteration tracker — read after the loop to report accurate counts.
        let iteration_tracker = Arc::new(Mutex::new(0u32));

        // Run with timeout using the shared agentic loop
        let result = tokio::time::timeout(self.config.timeout, async {
            let delegate = ContainerDelegate {
                client: self.client.clone(),
                safety: self.safety.clone(),
                tools: self.tools.clone(),
                extra_env: self.extra_env.clone(),
                last_output: Mutex::new(String::new()),
                iteration_tracker: iteration_tracker.clone(),
            };

            let config = AgenticLoopConfig {
                max_iterations: self.config.max_iterations as usize,
                enable_tool_intent_nudge: true,
                max_tool_intent_nudges: 2,
            };

            crate::agent::agentic_loop::run_agentic_loop(
                &delegate,
                &reasoning,
                &mut reason_ctx,
                &config,
            )
            .await
        })
        .await;

        let iterations = *iteration_tracker.lock().await;

        match result {
            Ok(Ok(LoopOutcome::Response(output))) => {
                tracing::info!("Worker completed job {} successfully", self.config.job_id);
                self.post_event(
                    "result",
                    serde_json::json!({
                        "success": true,
                        "message": truncate_for_preview(&output, 2000),
                    }),
                )
                .await;
                self.client
                    .report_complete(&CompletionReport {
                        success: true,
                        message: Some(output),
                        iterations,
                    })
                    .await?;
            }
            Ok(Ok(LoopOutcome::MaxIterations)) => {
                let msg = format!("max iterations ({}) exceeded", self.config.max_iterations);
                tracing::warn!("Worker failed for job {}: {}", self.config.job_id, msg);
                self.post_event(
                    "result",
                    serde_json::json!({
                        "success": false,
                        "message": format!("Execution failed: {}", msg),
                    }),
                )
                .await;
                self.client
                    .report_complete(&CompletionReport {
                        success: false,
                        message: Some(format!("Execution failed: {}", msg)),
                        iterations,
                    })
                    .await?;
            }
            Ok(Ok(LoopOutcome::Stopped | LoopOutcome::NeedApproval(_))) => {
                tracing::info!("Worker for job {} stopped", self.config.job_id);
                self.client
                    .report_complete(&CompletionReport {
                        success: false,
                        message: Some("Execution stopped".to_string()),
                        iterations,
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
                        iterations,
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
                        iterations,
                    })
                    .await?;
            }
        }

        Ok(())
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
}

/// Container delegate: implements `LoopDelegate` for the Docker container context.
///
/// Tools execute sequentially. Events are posted to the orchestrator via HTTP.
/// Completion is detected via `llm_signals_completion()`.
struct ContainerDelegate {
    client: Arc<WorkerHttpClient>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    extra_env: Arc<HashMap<String, String>>,
    /// Tracks the last successful tool output for the final response.
    last_output: Mutex<String>,
    /// Tracks the current iteration — shared with the outer `run` method so
    /// `CompletionReport` can include accurate iteration counts.
    iteration_tracker: Arc<Mutex<u32>>,
}

impl ContainerDelegate {
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
                    truncate_for_preview(&prompt.content, 100)
                );
                self.post_event(
                    "message",
                    serde_json::json!({
                        "role": "user",
                        "content": truncate_for_preview(&prompt.content, 2000),
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

#[async_trait]
impl LoopDelegate for ContainerDelegate {
    async fn check_signals(&self) -> LoopSignal {
        // Container runtime has no stop signals — the orchestrator manages lifecycle.
        LoopSignal::Continue
    }

    async fn before_llm_call(
        &self,
        reason_ctx: &mut ReasoningContext,
        iteration: usize,
    ) -> Option<LoopOutcome> {
        let iteration = iteration as u32;
        *self.iteration_tracker.lock().await = iteration;

        // Report progress every 5 iterations
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

        // Claude 4.6 rejects assistant prefill; NEAR AI rejects any non-user-ending
        // conversation. Ensure the last message is user-role before calling the LLM.
        crate::util::ensure_ends_with_user_message(&mut reason_ctx.messages);

        // Refresh tools (in case WASM tools were built)
        reason_ctx.available_tools = self.tools.tool_definitions().await;

        None
    }

    async fn call_llm(
        &self,
        reasoning: &Reasoning,
        reason_ctx: &mut ReasoningContext,
        _iteration: usize,
    ) -> Result<crate::llm::RespondOutput, crate::error::Error> {
        // Container uses respond_with_tools (which may return either text or tool calls)
        reasoning
            .respond_with_tools(reason_ctx)
            .await
            .map_err(Into::into)
    }

    async fn handle_text_response(
        &self,
        text: &str,
        reason_ctx: &mut ReasoningContext,
    ) -> TextAction {
        self.post_event(
            "message",
            serde_json::json!({
                "role": "assistant",
                "content": truncate_for_preview(text, 2000),
            }),
        )
        .await;

        // Check for completion
        if crate::util::llm_signals_completion(text) {
            let last = self.last_output.lock().await;
            let output = if last.is_empty() {
                text.to_string()
            } else {
                last.clone()
            };
            return TextAction::Return(LoopOutcome::Response(output));
        }

        reason_ctx.messages.push(ChatMessage::assistant(text));
        TextAction::Continue
    }

    async fn execute_tool_calls(
        &self,
        tool_calls: Vec<crate::llm::ToolCall>,
        content: Option<String>,
        reason_ctx: &mut ReasoningContext,
    ) -> Result<Option<LoopOutcome>, crate::error::Error> {
        if let Some(ref text) = content {
            self.post_event(
                "message",
                serde_json::json!({
                    "role": "assistant",
                    "content": truncate_for_preview(text, 2000),
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

        // Execute tools sequentially (container context — no parallel execution)
        for tc in tool_calls {
            self.post_event(
                "tool_use",
                serde_json::json!({
                    "tool_name": tc.name,
                    "input": truncate_for_preview(&tc.arguments.to_string(), 500),
                }),
            )
            .await;

            let job_ctx = JobContext {
                extra_env: self.extra_env.clone(),
                ..Default::default()
            };

            let result =
                execute_tool_simple(&self.tools, &self.safety, &tc.name, &tc.arguments, &job_ctx)
                    .await;

            self.post_event(
                "tool_result",
                serde_json::json!({
                    "tool_name": tc.name,
                    "output": match &result {
                        Ok(output) => truncate_for_preview(output, 2000),
                        Err(e) => format!("Error: {}", truncate_for_preview(e, 500)),
                    },
                    "success": result.is_ok(),
                }),
            )
            .await;

            if let Ok(ref output) = result {
                *self.last_output.lock().await = output.clone();
            }

            // Use shared result processing
            let (_, message) = process_tool_result(&self.safety, &tc.name, &tc.id, &result);
            reason_ctx.messages.push(message);
        }

        Ok(None)
    }

    async fn on_tool_intent_nudge(&self, text: &str, _reason_ctx: &mut ReasoningContext) {
        self.post_event(
            "message",
            serde_json::json!({
                "role": "assistant",
                "content": truncate_for_preview(text, 2000),
                "nudge": true,
            }),
        )
        .await;
    }

    async fn after_iteration(&self, _iteration: usize) {
        // Brief pause between iterations
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::agentic_loop::truncate_for_preview;

    #[test]
    fn test_truncate_within_limit() {
        assert_eq!(truncate_for_preview("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_at_limit() {
        assert_eq!(truncate_for_preview("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_beyond_limit() {
        let result = truncate_for_preview("hello world", 5);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_truncate_multibyte_safe() {
        // "é" is 2 bytes in UTF-8; slicing at byte 1 would panic without safety
        let result = truncate_for_preview("é is fancy", 1);
        // Should truncate to 0 chars (can't fit "é" in 1 byte)
        assert_eq!(result, "...");
    }
}
