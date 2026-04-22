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
use crate::llm::{ChatMessage, LlmProvider, Reasoning, ReasoningContext, ResponseMetadata};
use crate::tools::ToolRegistry;
use crate::tools::builtin::{FinishJobStatus, parse_finish_job_signal_from_output};
use crate::tools::execute::{execute_job_tool_simple, process_tool_result};
use crate::worker::api::{CompletionReport, JobEventPayload, StatusUpdate, WorkerHttpClient};
use crate::worker::autonomous_recovery::{
    AutonomousRecoveryAction, AutonomousRecoveryState, EMPTY_TOOL_COMPLETION_FAILURE,
    EMPTY_TOOL_COMPLETION_NUDGE, EMPTY_TOOL_COMPLETION_STRICT,
};
use crate::worker::proxy_llm::ProxyLlmProvider;
use ironclaw_safety::SafetyLayer;

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
        // Register container-safe tools plus job-only completion controls.
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
Work independently to complete this job.
The only way to end this job is to call the `finish_job` tool.
Call `finish_job` only after all other work is done.
Use status \"completed\" when all required work is finished,
or status \"failed\" when you hit an unresolvable blocker."#,
            job.title, job.description
        )));

        // Load tool definitions (use job context so finish_job is visible from the first iteration)
        reason_ctx.available_tools = self.tools.tool_definitions_for_job().await;

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
                recovery_state: Mutex::new(AutonomousRecoveryState::default()),
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
            Ok(Ok(LoopOutcome::Failure(reason))) => {
                tracing::warn!("Worker failed for job {}: {}", self.config.job_id, reason);
                self.post_event(
                    "result",
                    serde_json::json!({
                        "success": false,
                        "message": reason,
                    }),
                )
                .await;
                self.client
                    .report_complete(&CompletionReport {
                        success: false,
                        message: Some(reason),
                        iterations,
                    })
                    .await?;
            }
            Ok(Ok(
                LoopOutcome::Stopped | LoopOutcome::NeedApproval(_) | LoopOutcome::AuthPending(_),
            )) => {
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
/// Completion is detected when the LLM calls the `finish_job` tool.
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
    recovery_state: Mutex<AutonomousRecoveryState>,
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

        let recovery_mode_active = {
            let mut recovery = self.recovery_state.lock().await;
            recovery.begin_iteration()
        };
        if recovery_mode_active {
            tracing::warn!("Retrying after malformed tool completions with tools still enabled");
        }
        // Refresh tools (in case WASM tools were built)
        reason_ctx.available_tools = self.tools.tool_definitions_for_job().await;

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
        metadata: ResponseMetadata,
        reason_ctx: &mut ReasoningContext,
    ) -> TextAction {
        let action = {
            let mut recovery = self.recovery_state.lock().await;
            recovery.on_text_response(metadata, text)
        };
        match action {
            AutonomousRecoveryAction::ToolModeNudge => {
                tracing::warn!("Malformed empty tool completion detected; retrying in tool mode");
                self.post_event(
                    "status",
                    serde_json::json!({
                        "message": "Model returned an empty tool-completion response; retrying with a stronger tool-use nudge.",
                    }),
                )
                .await;
                reason_ctx
                    .messages
                    .push(ChatMessage::user(EMPTY_TOOL_COMPLETION_NUDGE));
                return TextAction::Continue;
            }
            AutonomousRecoveryAction::StrictToolRecovery => {
                tracing::warn!(
                    "Autonomous recovery escalated; requiring a valid tool call on the next reply"
                );
                self.post_event(
                    "status",
                    serde_json::json!({
                        "message": "Model failed to recover after a tool-use nudge; requiring a valid tool call or finish_job next.",
                    }),
                )
                .await;
                reason_ctx
                    .messages
                    .push(ChatMessage::user(EMPTY_TOOL_COMPLETION_STRICT));
                return TextAction::Continue;
            }
            AutonomousRecoveryAction::Fail => {
                tracing::warn!("Failing fast after repeated malformed autonomous responses");
                return TextAction::Return(LoopOutcome::Failure(
                    EMPTY_TOOL_COMPLETION_FAILURE.to_string(),
                ));
            }
            AutonomousRecoveryAction::Continue => {}
        }

        self.post_event(
            "message",
            serde_json::json!({
                "role": "assistant",
                "content": truncate_for_preview(text, 2000),
            }),
        )
        .await;

        // Empty text: do not auto-complete. Completion is only via finish_job.
        // The agentic loop's max_iterations cap and nudge mechanism handle
        // persistent empty responses.

        if !text.is_empty() {
            reason_ctx.messages.push(ChatMessage::assistant(text));
        }
        TextAction::Continue
    }

    async fn execute_tool_calls(
        &self,
        tool_calls: Vec<crate::llm::ToolCall>,
        content: Option<String>,
        reason_ctx: &mut ReasoningContext,
    ) -> Result<Option<LoopOutcome>, crate::error::Error> {
        {
            let mut recovery = self.recovery_state.lock().await;
            recovery.on_valid_tool_call();
        }

        // Partition: keep all non-finish_job calls first, then handle every
        // finish_job call at the end. If the model emits multiple finish_job
        // calls, only the last one decides the final status/summary.
        let mut finish_job_calls: Vec<crate::llm::ToolCall> = Vec::new();
        let mut other_calls: Vec<crate::llm::ToolCall> = Vec::with_capacity(tool_calls.len());
        for tc in tool_calls {
            if tc.name == "finish_job" {
                finish_job_calls.push(tc);
            } else {
                other_calls.push(tc);
            }
        }

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

        // Build the full tool_calls list for the assistant message (protocol requires it).
        let all_calls_for_msg: Vec<crate::llm::ToolCall> = other_calls
            .iter()
            .cloned()
            .chain(finish_job_calls.iter().cloned())
            .collect();

        // Add assistant message with tool_calls (OpenAI protocol)
        reason_ctx
            .messages
            .push(ChatMessage::assistant_with_tool_calls(
                content,
                all_calls_for_msg,
            ));

        // Execute non-finish_job tools sequentially (container context — no parallel execution)
        let mut tool_failure_count: usize = 0;
        let total_tools = other_calls.len() + finish_job_calls.len();
        for tc in other_calls {
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

            let result = execute_job_tool_simple(
                &self.tools,
                &self.safety,
                &tc.name,
                tc.arguments.clone(),
                &job_ctx,
            )
            .await;

            self.post_event(
                "tool_result",
                serde_json::json!({
                    "tool_name": tc.name,
                    "output": match &result {
                        Ok(output) => truncate_for_preview(output, 2000),
                        Err(e) => format!("Error: {}", truncate_for_preview(e, 500)).into(),
                    },
                    "success": result.is_ok(),
                }),
            )
            .await;

            if result.is_err() {
                tool_failure_count += 1;
            }

            if let Ok(ref output) = result {
                *self.last_output.lock().await = output.clone();
            }

            // Use shared result processing
            let (_, message) = process_tool_result(&self.safety, &tc.name, &tc.id, &result);
            reason_ctx.messages.push(message);
        }

        reason_ctx.last_tool_batch_all_failed =
            total_tools > 0 && tool_failure_count == total_tools;

        // Now handle finish_job — always runs last so other tools complete first.
        for (idx, tc) in finish_job_calls.iter().enumerate() {
            let is_last_finish_job = idx + 1 == finish_job_calls.len();
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
            let result = execute_job_tool_simple(
                &self.tools,
                &self.safety,
                &tc.name,
                tc.arguments.clone(),
                &job_ctx,
            )
            .await;

            self.post_event(
                "tool_result",
                serde_json::json!({
                    "tool_name": tc.name,
                    "output": match &result {
                        Ok(output) => truncate_for_preview(output, 2000),
                        Err(e) => format!("Error: {}", truncate_for_preview(e, 500)).into(),
                    },
                    "success": result.is_ok(),
                }),
            )
            .await;

            if result.is_err() {
                tool_failure_count += 1;
                let (_, message) = process_tool_result(&self.safety, &tc.name, &tc.id, &result);
                reason_ctx.messages.push(message);
                reason_ctx.last_tool_batch_all_failed =
                    total_tools > 0 && tool_failure_count == total_tools;
                if is_last_finish_job {
                    return Ok(None);
                }
                continue;
            }

            let (_, message) = process_tool_result(&self.safety, &tc.name, &tc.id, &result);
            reason_ctx.messages.push(message);

            if is_last_finish_job {
                let signal = match &result {
                    Ok(output) => parse_finish_job_signal_from_output(output),
                    Err(_) => unreachable!("finish_job error path already returned"),
                }
                .map_err(|e| {
                    crate::error::Error::from(crate::error::ToolError::ExecutionFailed {
                        name: "finish_job".to_string(),
                        reason: format!(
                            "finish_job executed but result could not be interpreted: {e}"
                        ),
                    })
                })?;

                if signal.status == FinishJobStatus::Completed {
                    return Ok(Some(LoopOutcome::Response(signal.summary)));
                }
                return Ok(Some(LoopOutcome::Failure(signal.summary)));
            }
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
