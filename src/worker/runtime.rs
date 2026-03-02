//! Worker runtime: the main execution loop inside a container.
//!
//! Reuses the existing `Reasoning` and `SafetyLayer` infrastructure but
//! connects to the orchestrator for LLM calls instead of calling APIs directly.
//! Streams real-time events (message, tool_use, tool_result, result) through
//! the orchestrator's job event pipeline for UI visibility.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::config::SafetyConfig;
use crate::context::JobContext;
use crate::error::WorkerError;
use crate::llm::{
    ChatMessage, DEFAULT_TOOL_RATIONALE, LlmProvider, Reasoning, ReasoningContext, RespondResult,
    ToolSelection, normalize_tool_reasoning,
};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;
use crate::tools::redaction::redact_sensitive_json;
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
            truncate(&job.description, 100)
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
                        "status": "completed",
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
                        "status": "failed",
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
                        "status": "failed",
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
        let mut next_parallel_group = 0usize;

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
                        let reasoning_narrative = sanitize_worker_narrative(&self.safety, &content);
                        if content.is_some() && reasoning_narrative.is_none() {
                            tracing::warn!(
                                "Worker reasoning narrative was empty or blocked by safety policy"
                            );
                        }

                        if let Some(text) = reasoning_narrative.as_deref() {
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

                        let batch_parallel_group = if tool_calls.len() > 1 {
                            let group = next_parallel_group;
                            next_parallel_group += 1;
                            Some(group)
                        } else {
                            None
                        };

                        let tool_decisions: Vec<serde_json::Value> = tool_calls
                            .iter()
                            .map(|tc| {
                                serde_json::json!({
                                    "tool_call_id": tc.id,
                                    "tool_name": tc.name,
                                    "rationale": sanitize_worker_rationale(&self.safety, &tc.reasoning),
                                    "outcome": "pending",
                                    "parallel_group": batch_parallel_group,
                                })
                            })
                            .collect();

                        self.post_event(
                            "reasoning",
                            serde_json::json!({
                                "narrative": reasoning_narrative,
                                "tool_decisions": tool_decisions,
                            }),
                        )
                        .await;

                        for tc in tool_calls {
                            self.post_event(
                                "tool_use",
                                serde_json::json!({
                                    "tool_name": tc.name,
                                    "input": redact_sensitive_json(&tc.arguments),
                                }),
                            )
                            .await;

                            let result = self.execute_tool(&tc.name, &tc.arguments).await;

                            self.post_event(
                                "tool_result",
                                serde_json::json!({
                                    "tool_name": tc.name,
                                    "output": match &result {
                                        Ok(output) => {
                                            self.safety
                                                .sanitize_tool_output("job_tool_result", output)
                                                .content
                                        }
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
                                reasoning: sanitize_worker_rationale(&self.safety, &tc.reasoning),
                                alternatives: vec![],
                                tool_call_id: tc.id.clone(),
                            };
                            self.process_result(reason_ctx, &selection, result);
                        }
                    }
                }
            } else {
                let batch_parallel_group = if selections.len() > 1 {
                    let group = next_parallel_group;
                    next_parallel_group += 1;
                    Some(group)
                } else {
                    None
                };

                let tool_decisions: Vec<serde_json::Value> = selections
                    .iter()
                    .map(|selection| {
                        serde_json::json!({
                            "tool_call_id": selection.tool_call_id,
                            "tool_name": selection.tool_name,
                            "rationale": sanitize_worker_rationale(&self.safety, &selection.reasoning),
                            "outcome": "pending",
                            "parallel_group": batch_parallel_group,
                        })
                    })
                    .collect();

                self.post_event(
                    "reasoning",
                    serde_json::json!({
                        "narrative": serde_json::Value::Null,
                        "tool_decisions": tool_decisions,
                    }),
                )
                .await;

                // Execute selected tools
                for selection in &selections {
                    self.post_event(
                        "tool_use",
                        serde_json::json!({
                            "tool_name": selection.tool_name,
                            "input": redact_sensitive_json(&selection.parameters),
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
                                Ok(output) => {
                                    self.safety
                                        .sanitize_tool_output("job_tool_result", output)
                                        .content
                                }
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

        let ctx = JobContext {
            extra_env: self.extra_env.clone(),
            ..Default::default()
        };

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

fn sanitize_worker_narrative(
    safety: &crate::safety::SafetyLayer,
    raw_content: &Option<String>,
) -> Option<String> {
    let text = raw_content.as_deref()?.trim();
    if text.is_empty() {
        return None;
    }

    let sanitized = safety.sanitize_tool_output("reasoning", text);
    let cleaned = sanitized.content.trim();
    if cleaned.is_empty() || safety.is_blocked_output(cleaned) {
        return None;
    }

    Some(cleaned.to_string())
}

fn sanitize_worker_rationale(safety: &crate::safety::SafetyLayer, raw_rationale: &str) -> String {
    let rationale = normalize_tool_reasoning(raw_rationale);
    let sanitized = safety.sanitize_tool_output("reasoning", &rationale);
    let cleaned = sanitized.content.trim();
    if cleaned.is_empty() || safety.is_blocked_output(cleaned) {
        tracing::warn!("Worker tool rationale blocked by safety policy; applying fallback");
        return DEFAULT_TOOL_RATIONALE.to_string();
    }

    cleaned.to_string()
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
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use rust_decimal::Decimal;

    use crate::config::SafetyConfig;
    use crate::error::{LlmError, WorkerError};
    use crate::llm::{
        ChatMessage, CompletionRequest, CompletionResponse, DEFAULT_TOOL_RATIONALE, FinishReason,
        LlmProvider, Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    };
    use crate::safety::SafetyLayer;
    use crate::tools::ToolRegistry;
    use crate::worker::runtime::{sanitize_worker_narrative, sanitize_worker_rationale, truncate};

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

    #[test]
    fn test_sanitize_worker_narrative_omits_blocked_content() {
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        }));
        let blocked = Some(
            "my key is sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ABCDEFGHIJKLMNOPQRST"
                .to_string(),
        );
        assert!(sanitize_worker_narrative(&safety, &blocked).is_none());
    }

    #[test]
    fn test_sanitize_worker_rationale_fallback_on_block() {
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        }));
        let blocked = "my key is sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ABCDEFGHIJKLMNOPQRST";
        let rationale = sanitize_worker_rationale(&safety, blocked);
        assert_eq!(rationale, DEFAULT_TOOL_RATIONALE);
    }

    struct QueueProvider {
        tool_responses: std::sync::Mutex<VecDeque<ToolCompletionResponse>>,
    }

    impl QueueProvider {
        fn new(responses: Vec<ToolCompletionResponse>) -> Self {
            Self {
                tool_responses: std::sync::Mutex::new(responses.into()),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for QueueProvider {
        fn model_name(&self) -> &str {
            "queue-provider"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            let mut guard = self.tool_responses.lock().expect("tool responses lock");
            guard.pop_front().ok_or_else(|| LlmError::RequestFailed {
                provider: "queue-provider".to_string(),
                reason: "no queued tool response".to_string(),
            })
        }
    }

    struct TestTool {
        name: &'static str,
    }

    #[async_trait]
    impl crate::tools::Tool for TestTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "additionalProperties": true
            })
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &crate::context::JobContext,
        ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
            Ok(crate::tools::ToolOutput::text(
                format!("{} ok", self.name),
                Duration::from_millis(1),
            ))
        }

        fn domain(&self) -> crate::tools::ToolDomain {
            crate::tools::ToolDomain::Container
        }
    }

    struct RecordingClient {
        events: tokio::sync::Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl RecordingClient {
        fn new() -> Self {
            Self {
                events: tokio::sync::Mutex::new(Vec::new()),
            }
        }

        async fn record(&self, event_type: &str, data: serde_json::Value) {
            self.events
                .lock()
                .await
                .push((event_type.to_string(), data));
        }

        async fn events_by_type(&self, event_type: &str) -> Vec<serde_json::Value> {
            self.events
                .lock()
                .await
                .iter()
                .filter(|(t, _)| t == event_type)
                .map(|(_, d)| d.clone())
                .collect()
        }
    }

    struct TestWorkerRuntime {
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
        events: Arc<RecordingClient>,
        max_iterations: u32,
    }

    impl TestWorkerRuntime {
        fn new(
            llm: Arc<dyn LlmProvider>,
            safety: Arc<SafetyLayer>,
            tools: Arc<ToolRegistry>,
            events: Arc<RecordingClient>,
        ) -> Self {
            Self {
                llm,
                safety,
                tools,
                events,
                max_iterations: 4,
            }
        }

        async fn run(&self) -> Result<(), WorkerError> {
            let reasoning = crate::llm::Reasoning::new(self.llm.clone(), self.safety.clone());
            let mut reason_ctx = crate::llm::ReasoningContext::new();
            reason_ctx.messages.push(ChatMessage {
                role: Role::System,
                content: "test".to_string(),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            });
            reason_ctx.available_tools = self.tools.tool_definitions().await;

            let mut next_parallel_group = 0usize;
            for _ in 0..self.max_iterations {
                let output = reasoning
                    .respond_with_tools(&reason_ctx)
                    .await
                    .map_err(|e| WorkerError::ExecutionFailed {
                        reason: format!("respond_with_tools failed: {}", e),
                    })?;

                match output.result {
                    crate::llm::RespondResult::Text(_) => break,
                    crate::llm::RespondResult::ToolCalls {
                        tool_calls,
                        content,
                    } => {
                        if tool_calls.is_empty() {
                            continue;
                        }

                        let narrative = sanitize_worker_narrative(&self.safety, &content);
                        let batch_parallel_group = if tool_calls.len() > 1 {
                            let g = next_parallel_group;
                            next_parallel_group += 1;
                            Some(g)
                        } else {
                            None
                        };

                        let tool_decisions: Vec<serde_json::Value> = tool_calls
                            .iter()
                            .map(|tc| {
                                serde_json::json!({
                                    "tool_call_id": tc.id,
                                    "tool_name": tc.name,
                                    "rationale": sanitize_worker_rationale(&self.safety, &tc.reasoning),
                                    "outcome": "pending",
                                    "parallel_group": batch_parallel_group,
                                })
                            })
                            .collect();

                        self.events
                            .record(
                                "reasoning",
                                serde_json::json!({
                                    "narrative": narrative,
                                    "tool_decisions": tool_decisions,
                                }),
                            )
                            .await;

                        for tc in tool_calls {
                            let result = self
                                .tools
                                .get(&tc.name)
                                .await
                                .ok_or_else(|| WorkerError::ExecutionFailed {
                                    reason: format!("missing tool {}", tc.name),
                                })?
                                .execute(
                                    tc.arguments.clone(),
                                    &crate::context::JobContext::default(),
                                )
                                .await
                                .map_err(|e| WorkerError::ExecutionFailed {
                                    reason: e.to_string(),
                                })?;

                            reason_ctx.messages.push(ChatMessage::tool_result(
                                &tc.id,
                                &tc.name,
                                serde_json::to_string(&result.result)
                                    .unwrap_or_else(|_| "{}".to_string()),
                            ));
                        }
                    }
                }
            }

            Ok(())
        }
    }

    #[tokio::test]
    async fn test_worker_reasoning_event_parallel_groups_monotonic() {
        let responses = vec![
            ToolCompletionResponse {
                content: Some("first pass".to_string()),
                tool_calls: vec![
                    ToolCall {
                        id: "call_a1".to_string(),
                        name: "tool_a".to_string(),
                        arguments: serde_json::json!({}),
                        reasoning: "r1".to_string(),
                    },
                    ToolCall {
                        id: "call_b1".to_string(),
                        name: "tool_b".to_string(),
                        arguments: serde_json::json!({}),
                        reasoning: "r2".to_string(),
                    },
                ],
                input_tokens: 10,
                output_tokens: 10,
                finish_reason: FinishReason::ToolUse,
            },
            ToolCompletionResponse {
                content: Some("second pass".to_string()),
                tool_calls: vec![
                    ToolCall {
                        id: "call_a2".to_string(),
                        name: "tool_a".to_string(),
                        arguments: serde_json::json!({}),
                        reasoning: "r3".to_string(),
                    },
                    ToolCall {
                        id: "call_b2".to_string(),
                        name: "tool_b".to_string(),
                        arguments: serde_json::json!({}),
                        reasoning: "r4".to_string(),
                    },
                ],
                input_tokens: 10,
                output_tokens: 10,
                finish_reason: FinishReason::ToolUse,
            },
            ToolCompletionResponse {
                content: Some("done".to_string()),
                tool_calls: vec![],
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
            },
        ];

        let llm = Arc::new(QueueProvider::new(responses));
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        }));

        let tools = Arc::new(ToolRegistry::new());
        tools.register_sync(Arc::new(TestTool { name: "tool_a" }));
        tools.register_sync(Arc::new(TestTool { name: "tool_b" }));

        let recorder = Arc::new(RecordingClient::new());
        let runtime = TestWorkerRuntime::new(llm, safety, tools, Arc::clone(&recorder));
        runtime.run().await.expect("runtime should succeed");

        let reasoning_events = recorder.events_by_type("reasoning").await;
        assert!(reasoning_events.len() >= 2);

        let first_group = reasoning_events[0]["tool_decisions"][0]["parallel_group"].as_u64();
        let second_group = reasoning_events[1]["tool_decisions"][0]["parallel_group"].as_u64();
        assert_eq!(first_group, Some(0));
        assert_eq!(second_group, Some(1));
    }
}
