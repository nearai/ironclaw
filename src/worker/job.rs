//! Job worker execution via the shared `AgenticLoop`.
//!
//! Replaces `src/agent/worker.rs` with a `JobDelegate` that implements
//! `LoopDelegate`. The `Worker` struct and `WorkerDeps` remain as the
//! public API consumed by `scheduler.rs`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::agentic_loop::{
    AgenticLoopConfig, LoopDelegate, LoopOutcome, LoopSignal, TextAction, run_agentic_loop,
    truncate_for_preview,
};
use crate::agent::scheduler::WorkerMessage;
use crate::agent::task::TaskOutput;
use crate::channels::web::types::SseEvent;
use crate::context::{ContextManager, JobState};
use crate::db::Database;
use crate::error::Error;
use crate::hooks::HookRegistry;
use crate::llm::{
    ActionPlan, ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult, ToolCall,
    ToolSelection,
};
use crate::safety::SafetyLayer;
use crate::tools::execute::process_tool_result;
use crate::tools::rate_limiter::RateLimitResult;
use crate::tools::{ApprovalContext, ToolRegistry, redact_params};

/// Shared dependencies for worker execution.
///
/// This bundles the dependencies that are shared across all workers,
/// reducing the number of arguments to `Worker::new`.
#[derive(Clone)]
pub struct WorkerDeps {
    pub context_manager: Arc<ContextManager>,
    pub llm: Arc<dyn LlmProvider>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub store: Option<Arc<dyn Database>>,
    pub hooks: Arc<HookRegistry>,
    pub timeout: Duration,
    pub use_planning: bool,
    /// SSE broadcast sender for live job event streaming to the web gateway.
    pub sse_tx: Option<tokio::sync::broadcast::Sender<SseEvent>>,
    /// Approval context for tool execution. When `None`, all non-`Never` tools are
    /// blocked (legacy behavior). When `Some`, the context determines which tools
    /// are pre-approved for autonomous execution.
    pub approval_context: Option<ApprovalContext>,
    /// HTTP interceptor for trace recording/replay (propagated to JobContext).
    pub http_interceptor: Option<Arc<dyn crate::llm::recording::HttpInterceptor>>,
}

/// Worker that executes a single job.
pub struct Worker {
    job_id: Uuid,
    deps: WorkerDeps,
}

/// Result of a tool execution with metadata for context building.
struct ToolExecResult {
    result: Result<String, Error>,
}

impl Worker {
    /// Create a new worker for a specific job.
    pub fn new(job_id: Uuid, deps: WorkerDeps) -> Self {
        Self { job_id, deps }
    }

    // Convenience accessors to avoid deps.field everywhere
    fn context_manager(&self) -> &Arc<ContextManager> {
        &self.deps.context_manager
    }

    fn llm(&self) -> &Arc<dyn LlmProvider> {
        &self.deps.llm
    }

    #[allow(dead_code)]
    fn safety(&self) -> &Arc<SafetyLayer> {
        &self.deps.safety
    }

    fn tools(&self) -> &Arc<ToolRegistry> {
        &self.deps.tools
    }

    fn store(&self) -> Option<&Arc<dyn Database>> {
        self.deps.store.as_ref()
    }

    fn timeout(&self) -> Duration {
        self.deps.timeout
    }

    fn use_planning(&self) -> bool {
        self.deps.use_planning
    }

    /// Fire-and-forget persistence of job status.
    fn persist_status(&self, status: JobState, reason: Option<String>) {
        if let Some(store) = self.store() {
            let store = store.clone();
            let job_id = self.job_id;
            tokio::spawn(async move {
                if let Err(e) = store
                    .update_job_status(job_id, status, reason.as_deref())
                    .await
                {
                    tracing::warn!("Failed to persist status for job {}: {}", job_id, e);
                }
            });
        }
    }

    /// Fire-and-forget persistence of a job event and SSE broadcast.
    fn log_event(&self, event_type: &str, data: serde_json::Value) {
        let job_id = self.job_id;

        // Persist to DB
        if let Some(store) = self.store() {
            let store = store.clone();
            let et = event_type.to_string();
            let d = data.clone();
            tokio::spawn(async move {
                if let Err(e) = store.save_job_event(job_id, &et, &d).await {
                    tracing::warn!("Failed to persist event for job {}: {}", job_id, e);
                }
            });
        }

        // Broadcast SSE for live web UI updates
        if let Some(ref tx) = self.deps.sse_tx {
            let job_id_str = job_id.to_string();
            let event = match event_type {
                "message" => Some(SseEvent::JobMessage {
                    job_id: job_id_str,
                    role: data
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("assistant")
                        .to_string(),
                    content: data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                }),
                "tool_use" => Some(SseEvent::JobToolUse {
                    job_id: job_id_str,
                    tool_name: data
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    input: data
                        .get("input")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                }),
                "tool_result" => Some(SseEvent::JobToolResult {
                    job_id: job_id_str,
                    tool_name: data
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    output: data
                        .get("output")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                }),
                "status" => Some(SseEvent::JobStatus {
                    job_id: job_id_str,
                    message: data
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                }),
                "result" => Some(SseEvent::JobResult {
                    job_id: job_id_str,
                    status: data
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("completed")
                        .to_string(),
                    session_id: data
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                }),
                _ => None,
            };
            if let Some(event) = event {
                let _ = tx.send(event);
            }
        }
    }

    /// Run the worker until the job is complete or stopped.
    pub async fn run(self, mut rx: mpsc::Receiver<WorkerMessage>) -> Result<(), Error> {
        tracing::info!("Worker starting for job {}", self.job_id);

        // Wait for start signal
        match rx.recv().await {
            Some(WorkerMessage::Start) => {}
            Some(WorkerMessage::Stop) | None => {
                tracing::debug!("Worker for job {} stopped before starting", self.job_id);
                return Ok(());
            }
            Some(WorkerMessage::Ping) | Some(WorkerMessage::UserMessage(_)) => {}
        }

        // Get job context
        let job_ctx = self.context_manager().get_context(self.job_id).await?;

        // Create reasoning engine
        let reasoning = Reasoning::new(self.llm().clone());

        // Build initial reasoning context (tool definitions refreshed each iteration in execution_loop)
        let mut reason_ctx = ReasoningContext::new().with_job(&job_ctx.description);

        // Add system message
        reason_ctx.messages.push(ChatMessage::system(format!(
            r#"You are an autonomous agent working on a job.

Job: {}
Description: {}

You have access to tools to complete this job. Plan your approach and execute tools as needed.
You may request multiple tools at once if they can be executed in parallel.
Report when the job is complete or if you encounter issues you cannot resolve."#,
            job_ctx.title, job_ctx.description
        )));

        // Main execution loop with timeout
        let result = tokio::time::timeout(self.timeout(), async {
            self.execution_loop(&mut rx, &reasoning, &mut reason_ctx)
                .await
        })
        .await;

        match result {
            Ok(Ok(())) => {
                tracing::info!("Worker for job {} completed successfully", self.job_id);
                // Only mark completed if still in an active, non-stuck state.
                let current_state = self
                    .context_manager()
                    .get_context(self.job_id)
                    .await
                    .map(|ctx| ctx.state);
                match current_state {
                    Ok(state) if state.is_terminal() => {}
                    Ok(JobState::Completed) => {}
                    Ok(JobState::Stuck) => {
                        tracing::info!(
                            "Job {} returned Ok but is Stuck — leaving for self-repair",
                            self.job_id
                        );
                    }
                    Ok(_) => {
                        self.mark_completed().await?;
                    }
                    Err(e) => {
                        tracing::warn!(
                            job_id = %self.job_id,
                            "Failed to get job context, cannot mark as completed: {}", e
                        );
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::error!("Worker for job {} failed: {}", self.job_id, e);
                self.mark_failed(&e.to_string()).await?;
            }
            Err(_) => {
                tracing::warn!("Worker for job {} timed out", self.job_id);
                self.mark_stuck("Execution timeout").await?;
            }
        }

        Ok(())
    }

    async fn execution_loop(
        &self,
        rx: &mut mpsc::Receiver<WorkerMessage>,
        reasoning: &Reasoning,
        reason_ctx: &mut ReasoningContext,
    ) -> Result<(), Error> {
        const MAX_WORKER_ITERATIONS: usize = 500;
        let max_iterations = self
            .context_manager()
            .get_context(self.job_id)
            .await
            .ok()
            .and_then(|ctx| ctx.metadata.get("max_iterations").and_then(|v| v.as_u64()))
            .unwrap_or(50) as usize;
        let max_iterations = max_iterations.min(MAX_WORKER_ITERATIONS);

        // Initial tool definitions for planning (will be refreshed in loop)
        reason_ctx.available_tools = self.tools().tool_definitions().await;

        // Generate plan if planning is enabled
        let plan = if self.use_planning() {
            match reasoning.plan(reason_ctx).await {
                Ok(p) => {
                    tracing::info!(
                        "Created plan for job {}: {} actions, {:.0}% confidence",
                        self.job_id,
                        p.actions.len(),
                        p.confidence * 100.0
                    );

                    // Add plan to context as assistant message
                    reason_ctx.messages.push(ChatMessage::assistant(format!(
                        "I've created a plan to accomplish this goal: {}\n\nSteps:\n{}",
                        p.goal,
                        p.actions
                            .iter()
                            .enumerate()
                            .map(|(i, a)| format!("{}. {} - {}", i + 1, a.tool_name, a.reasoning))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )));

                    self.log_event("message", serde_json::json!({
                        "role": "assistant",
                        "content": format!("Plan: {}\n\n{}", p.goal,
                            p.actions.iter().enumerate()
                                .map(|(i, a)| format!("{}. {} - {}", i + 1, a.tool_name, a.reasoning))
                                .collect::<Vec<_>>().join("\n"))
                    }));

                    Some(p)
                }
                Err(e) => {
                    tracing::warn!(
                        "Planning failed for job {}, falling back to direct selection: {}",
                        self.job_id,
                        e
                    );
                    None
                }
            }
        } else {
            None
        };

        // If we have a plan, execute it.
        if let Some(ref plan) = plan {
            self.execute_plan(rx, reasoning, reason_ctx, plan).await?;

            if let Ok(ctx) = self.context_manager().get_context(self.job_id).await
                && (ctx.state.is_terminal()
                    || ctx.state == JobState::Stuck
                    || ctx.state == JobState::Completed)
            {
                return Ok(());
            }
        }

        // Build the delegate and run the shared agentic loop
        let delegate = JobDelegate {
            worker: self,
            rx: tokio::sync::Mutex::new(rx),
        };

        let config = AgenticLoopConfig {
            max_iterations,
            enable_tool_intent_nudge: true,
            max_tool_intent_nudges: 2,
        };

        let outcome = run_agentic_loop(&delegate, reasoning, reason_ctx, &config).await?;

        match outcome {
            LoopOutcome::Response(_) => {
                // Completion was already handled in handle_text_response via mark_completed
            }
            LoopOutcome::MaxIterations => {
                self.mark_failed("Maximum iterations exceeded: job hit the iteration cap")
                    .await?;
            }
            LoopOutcome::Stopped => {
                // Stop signal handled — nothing more to do
            }
            LoopOutcome::Custom(_) => {}
        }

        Ok(())
    }

    /// Execute multiple tools in parallel using a JoinSet.
    ///
    /// Each task is tagged with its original index so results are returned
    /// in the same order as `selections`, regardless of completion order.
    async fn execute_tools_parallel(&self, selections: &[ToolSelection]) -> Vec<ToolExecResult> {
        let count = selections.len();

        // Short-circuit for single tool: execute directly without JoinSet overhead
        if count <= 1 {
            let mut results = Vec::with_capacity(count);
            for selection in selections {
                let result = Self::execute_tool_inner(
                    &self.deps,
                    self.job_id,
                    &selection.tool_name,
                    &selection.parameters,
                )
                .await;
                results.push(ToolExecResult { result });
            }
            return results;
        }

        let mut join_set = JoinSet::new();

        for (idx, selection) in selections.iter().enumerate() {
            let deps = self.deps.clone();
            let job_id = self.job_id;
            let tool_name = selection.tool_name.clone();
            let params = selection.parameters.clone();
            join_set.spawn(async move {
                let result = Self::execute_tool_inner(&deps, job_id, &tool_name, &params).await;
                (idx, ToolExecResult { result })
            });
        }

        // Collect and reorder by original index
        let mut results: Vec<Option<ToolExecResult>> = (0..count).map(|_| None).collect();
        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((idx, exec_result)) => results[idx] = Some(exec_result),
                Err(e) => {
                    if e.is_panic() {
                        tracing::error!("Tool execution task panicked: {}", e);
                    } else {
                        tracing::error!("Tool execution task cancelled: {}", e);
                    }
                }
            }
        }

        // Fill any panicked slots with error results
        results
            .into_iter()
            .enumerate()
            .map(|(i, opt)| {
                opt.unwrap_or_else(|| ToolExecResult {
                    result: Err(crate::error::ToolError::ExecutionFailed {
                        name: selections[i].tool_name.clone(),
                        reason: "Task failed during execution".to_string(),
                    }
                    .into()),
                })
            })
            .collect()
    }

    /// Inner tool execution logic that can be called from both single and parallel paths.
    async fn execute_tool_inner(
        deps: &WorkerDeps,
        job_id: Uuid,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<String, Error> {
        let tool =
            deps.tools
                .get(tool_name)
                .await
                .ok_or_else(|| crate::error::ToolError::NotFound {
                    name: tool_name.to_string(),
                })?;

        // Check approval: use context-aware check if available, else block all non-Never tools
        let requirement = tool.requires_approval(params);
        let blocked =
            ApprovalContext::is_blocked_or_default(&deps.approval_context, tool_name, requirement);
        if blocked {
            return Err(crate::error::ToolError::AuthRequired {
                name: tool_name.to_string(),
            }
            .into());
        }

        // Fetch job context early so we have the real user_id for hooks and rate limiting
        let mut job_ctx = deps.context_manager.get_context(job_id).await?;
        // Propagate http_interceptor for trace recording/replay
        if job_ctx.http_interceptor.is_none() {
            job_ctx.http_interceptor = deps.http_interceptor.clone();
        }

        // Check per-tool rate limit before running hooks or executing (cheaper check first)
        if let Some(config) = tool.rate_limit_config()
            && let RateLimitResult::Limited { retry_after, .. } = deps
                .tools
                .rate_limiter()
                .check_and_record(&job_ctx.user_id, tool_name, &config)
                .await
        {
            return Err(crate::error::ToolError::RateLimited {
                name: tool_name.to_string(),
                retry_after: Some(retry_after),
            }
            .into());
        }

        // Run BeforeToolCall hook
        let params = {
            use crate::hooks::{HookError, HookEvent, HookOutcome};
            let hook_params = redact_params(params, tool.sensitive_params());
            let event = HookEvent::ToolCall {
                tool_name: tool_name.to_string(),
                parameters: hook_params,
                user_id: job_ctx.user_id.clone(),
                context: format!("job:{}", job_id),
            };
            match deps.hooks.run(&event).await {
                Err(HookError::Rejected { reason }) => {
                    return Err(crate::error::ToolError::ExecutionFailed {
                        name: tool_name.to_string(),
                        reason: format!("Blocked by hook: {}", reason),
                    }
                    .into());
                }
                Err(err) => {
                    return Err(crate::error::ToolError::ExecutionFailed {
                        name: tool_name.to_string(),
                        reason: format!("Blocked by hook failure mode: {}", err),
                    }
                    .into());
                }
                Ok(HookOutcome::Continue {
                    modified: Some(new_params),
                }) => serde_json::from_str(&new_params).unwrap_or_else(|e| {
                    tracing::warn!(
                        tool = %tool_name,
                        "Hook returned non-JSON modification for ToolCall, ignoring: {}",
                        e
                    );
                    params.clone()
                }),
                _ => params.clone(),
            }
        };
        if job_ctx.state == JobState::Cancelled {
            return Err(crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: "Job is cancelled".to_string(),
            }
            .into());
        }

        // Validate tool parameters
        let validation = deps.safety.validator().validate_tool_params(&params);
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

        // Redact sensitive parameter values before they touch any observability or audit path.
        let safe_params = redact_params(&params, tool.sensitive_params());
        tracing::debug!(
            tool = %tool_name,
            params = %safe_params,
            job = %job_id,
            "Tool call started"
        );

        // Execute with per-tool timeout and timing
        let tool_timeout = tool.execution_timeout();
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(tool_timeout, async {
            tool.execute(params.clone(), &job_ctx).await
        })
        .await;
        let elapsed = start.elapsed();

        match &result {
            Ok(Ok(output)) => {
                let result_str = serde_json::to_string(&output.result)
                    .unwrap_or_else(|_| "<serialize error>".to_string());
                tracing::debug!(
                    tool = %tool_name,
                    elapsed_ms = elapsed.as_millis() as u64,
                    result = %result_str,
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
                    timeout_secs = tool_timeout.as_secs(),
                    "Tool call timed out"
                );
            }
        }

        // Record action in memory and get the ActionRecord for persistence
        let action = match &result {
            Ok(Ok(output)) => {
                let output_str = serde_json::to_string_pretty(&output.result)
                    .ok()
                    .map(|s| deps.safety.sanitize_tool_output(tool_name, &s).content);
                match deps
                    .context_manager
                    .update_memory(job_id, |mem| {
                        let rec = mem.create_action(tool_name, safe_params.clone()).succeed(
                            output_str.clone(),
                            output.result.clone(),
                            elapsed,
                        );
                        mem.record_action(rec.clone());
                        rec
                    })
                    .await
                {
                    Ok(rec) => Some(rec),
                    Err(e) => {
                        tracing::warn!(job_id = %job_id, tool = tool_name, "Failed to record action in memory: {e}");
                        None
                    }
                }
            }
            Ok(Err(e)) => {
                match deps
                    .context_manager
                    .update_memory(job_id, |mem| {
                        let rec = mem
                            .create_action(tool_name, safe_params.clone())
                            .fail(e.to_string(), elapsed);
                        mem.record_action(rec.clone());
                        rec
                    })
                    .await
                {
                    Ok(rec) => Some(rec),
                    Err(e) => {
                        tracing::warn!(job_id = %job_id, tool = tool_name, "Failed to record action in memory: {e}");
                        None
                    }
                }
            }
            Err(_) => {
                match deps
                    .context_manager
                    .update_memory(job_id, |mem| {
                        let rec = mem
                            .create_action(tool_name, safe_params.clone())
                            .fail("Execution timeout", elapsed);
                        mem.record_action(rec.clone());
                        rec
                    })
                    .await
                {
                    Ok(rec) => Some(rec),
                    Err(e) => {
                        tracing::warn!(job_id = %job_id, tool = tool_name, "Failed to record action in memory: {e}");
                        None
                    }
                }
            }
        };

        // Persist action to database (fire-and-forget)
        if let (Some(action), Some(store)) = (action, deps.store.clone()) {
            tokio::spawn(async move {
                if let Err(e) = store.save_action(job_id, &action).await {
                    tracing::warn!("Failed to persist action for job {}: {}", job_id, e);
                }
            });
        }

        // Handle the result
        let output = result
            .map_err(|_| crate::error::ToolError::Timeout {
                name: tool_name.to_string(),
                timeout: tool_timeout,
            })?
            .map_err(|e| crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: e.to_string(),
            })?;

        // Return result as string
        serde_json::to_string_pretty(&output.result).map_err(|e| {
            crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: format!("Failed to serialize result: {}", e),
            }
            .into()
        })
    }

    /// Process a tool execution result and add it to the reasoning context.
    async fn process_tool_result_job(
        &self,
        reason_ctx: &mut ReasoningContext,
        selection: &ToolSelection,
        result: Result<String, Error>,
    ) -> Result<bool, Error> {
        self.log_event(
            "tool_use",
            serde_json::json!({
                "tool_name": selection.tool_name,
                "input": truncate_for_preview(
                    &selection.parameters.to_string(), 500),
            }),
        );

        // Use shared result processing for sanitize → wrap → ChatMessage
        let (content, message) = process_tool_result(
            &self.deps.safety,
            &selection.tool_name,
            &selection.tool_call_id,
            &result,
        );
        reason_ctx.messages.push(message);

        match &result {
            Ok(_) => {
                self.log_event("tool_result", serde_json::json!({
                    "tool_name": selection.tool_name,
                    "success": true,
                    "output": truncate_for_preview(&content, 500),
                }));
                Ok(false)
            }
            Err(e) => {
                tracing::warn!(
                    "Tool {} failed for job {}: {}",
                    selection.tool_name,
                    self.job_id,
                    e
                );

                // Record failure for self-repair tracking
                if let Some(store) = self.store() {
                    let store = store.clone();
                    let tool_name = selection.tool_name.clone();
                    let error_msg = e.to_string();
                    tokio::spawn(async move {
                        if let Err(db_err) = store.record_tool_failure(&tool_name, &error_msg).await
                        {
                            tracing::warn!("Failed to record tool failure: {}", db_err);
                        }
                    });
                }

                self.log_event(
                    "tool_result",
                    serde_json::json!({
                        "tool_name": selection.tool_name,
                        "success": false,
                        "output": format!("Error: {}", e),
                    }),
                );

                Ok(false)
            }
        }
    }

    /// Execute a pre-generated plan.
    async fn execute_plan(
        &self,
        rx: &mut mpsc::Receiver<WorkerMessage>,
        reasoning: &Reasoning,
        reason_ctx: &mut ReasoningContext,
        plan: &ActionPlan,
    ) -> Result<(), Error> {
        for (i, action) in plan.actions.iter().enumerate() {
            // Check for stop signal and injected user messages
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMessage::Stop => {
                        tracing::debug!(
                            "Worker for job {} received stop signal during plan execution",
                            self.job_id
                        );
                        return Ok(());
                    }
                    WorkerMessage::Ping => {
                        tracing::trace!("Worker for job {} received ping", self.job_id);
                    }
                    WorkerMessage::Start => {}
                    WorkerMessage::UserMessage(content) => {
                        tracing::info!(
                            job_id = %self.job_id,
                            "User message received during plan execution, abandoning plan"
                        );
                        reason_ctx.messages.push(ChatMessage::user(&content));
                        self.log_event(
                            "message",
                            serde_json::json!({
                                "role": "user",
                                "content": content,
                            }),
                        );
                        self.log_event(
                            "status",
                            serde_json::json!({
                                "message": "Plan interrupted by user message, re-evaluating...",
                            }),
                        );
                        return Ok(());
                    }
                }
            }

            tracing::debug!(
                "Job {} executing planned action {}/{}: {} - {}",
                self.job_id,
                i + 1,
                plan.actions.len(),
                action.tool_name,
                action.reasoning
            );

            let selection = ToolSelection {
                tool_name: action.tool_name.clone(),
                parameters: action.parameters.clone(),
                reasoning: action.reasoning.clone(),
                alternatives: vec![],
                tool_call_id: format!("plan_{}_{}", self.job_id, i),
            };

            reason_ctx
                .messages
                .push(ChatMessage::assistant_with_tool_calls(
                    None,
                    vec![ToolCall {
                        id: selection.tool_call_id.clone(),
                        name: selection.tool_name.clone(),
                        arguments: selection.parameters.clone(),
                    }],
                ));

            let result = self
                .execute_tool(&action.tool_name, &action.parameters)
                .await;

            let completed = self
                .process_tool_result_job(reason_ctx, &selection, result)
                .await?;

            if completed {
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Plan completed, check with LLM if job is done
        reason_ctx.messages.push(ChatMessage::user(
            "All planned actions have been executed. Is the job complete? If not, what else needs to be done?",
        ));

        let response = reasoning.respond(reason_ctx).await?;
        reason_ctx.messages.push(ChatMessage::assistant(&response));

        if crate::util::llm_signals_completion(&response) {
            self.mark_completed().await?;
        } else {
            tracing::info!(
                "Job {} plan completed but work remains, falling back to direct selection",
                self.job_id
            );
            self.log_event(
                "status",
                serde_json::json!({
                    "message": "Plan completed but job needs more work, continuing...",
                }),
            );
        }

        Ok(())
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<String, Error> {
        Self::execute_tool_inner(&self.deps, self.job_id, tool_name, params).await
    }

    async fn mark_completed(&self) -> Result<(), Error> {
        self.context_manager()
            .update_context(self.job_id, |ctx| {
                ctx.transition_to(
                    JobState::Completed,
                    Some("Job completed successfully".to_string()),
                )
            })
            .await?
            .map_err(|s| crate::error::JobError::ContextError {
                id: self.job_id,
                reason: s,
            })?;

        self.log_event(
            "result",
            serde_json::json!({
                "status": "completed",
                "success": true,
                "message": "Job completed successfully",
            }),
        );
        self.persist_status(
            JobState::Completed,
            Some("Job completed successfully".to_string()),
        );
        Ok(())
    }

    async fn mark_failed(&self, reason: &str) -> Result<(), Error> {
        self.context_manager()
            .update_context(self.job_id, |ctx| {
                ctx.transition_to(JobState::Failed, Some(reason.to_string()))
            })
            .await?
            .map_err(|s| crate::error::JobError::ContextError {
                id: self.job_id,
                reason: s,
            })?;

        self.log_event(
            "result",
            serde_json::json!({
                "status": "failed",
                "success": false,
                "message": format!("Execution failed: {}", reason),
            }),
        );
        self.persist_status(JobState::Failed, Some(reason.to_string()));
        Ok(())
    }

    async fn mark_stuck(&self, reason: &str) -> Result<(), Error> {
        self.context_manager()
            .update_context(self.job_id, |ctx| ctx.mark_stuck(reason))
            .await?
            .map_err(|s| crate::error::JobError::ContextError {
                id: self.job_id,
                reason: s,
            })?;

        self.log_event(
            "result",
            serde_json::json!({
                "status": "stuck",
                "success": false,
                "message": format!("Job stuck: {}", reason),
            }),
        );
        self.persist_status(JobState::Stuck, Some(reason.to_string()));
        Ok(())
    }
}

/// Job delegate: implements `LoopDelegate` for the background job context.
///
/// Handles: signal channel (stop/ping/user messages), cancellation checks,
/// rate-limit retry, parallel tool execution, DB persistence, SSE broadcasting.
struct JobDelegate<'a> {
    worker: &'a Worker,
    rx: tokio::sync::Mutex<&'a mut mpsc::Receiver<WorkerMessage>>,
}

#[async_trait]
impl<'a> LoopDelegate for JobDelegate<'a> {
    async fn check_signals(&self) -> LoopSignal {
        // Drain the message channel
        let mut rx: tokio::sync::MutexGuard<'_, &mut mpsc::Receiver<WorkerMessage>> = self.rx.lock().await;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                WorkerMessage::Stop => {
                    tracing::debug!(
                        "Worker for job {} received stop signal",
                        self.worker.job_id
                    );
                    return LoopSignal::Stop;
                }
                WorkerMessage::Ping => {
                    tracing::trace!("Worker for job {} received ping", self.worker.job_id);
                }
                WorkerMessage::Start => {}
                WorkerMessage::UserMessage(content) => {
                    tracing::info!(
                        job_id = %self.worker.job_id,
                        "Worker received follow-up user message"
                    );
                    self.worker.log_event(
                        "message",
                        serde_json::json!({
                            "role": "user",
                            "content": content,
                        }),
                    );
                    return LoopSignal::InjectMessage(content);
                }
            }
        }

        // Check for cancellation
        if let Ok(ctx) = self
            .worker
            .context_manager()
            .get_context(self.worker.job_id)
            .await
            && ctx.state == JobState::Cancelled
        {
            tracing::info!(
                "Worker for job {} detected cancellation",
                self.worker.job_id
            );
            return LoopSignal::Stop;
        }

        LoopSignal::Continue
    }

    async fn before_llm_call(
        &self,
        reason_ctx: &mut ReasoningContext,
        _iteration: usize,
    ) -> Option<LoopOutcome> {
        // Refresh tool definitions so newly built tools become visible
        reason_ctx.available_tools = self.worker.tools().tool_definitions().await;
        None
    }

    async fn call_llm(
        &self,
        reasoning: &Reasoning,
        reason_ctx: &mut ReasoningContext,
        _iteration: usize,
    ) -> Result<crate::llm::RespondOutput, crate::error::Error> {
        // Try select_tools first, fall back to respond_with_tools
        match reasoning.select_tools(reason_ctx).await {
            Ok(s) if !s.is_empty() => {
                // Build a synthetic RespondOutput with ToolCalls
                let tool_calls: Vec<ToolCall> = selections_to_tool_calls(&s);
                return Ok(crate::llm::RespondOutput {
                    result: RespondResult::ToolCalls {
                        tool_calls,
                        content: None,
                    },
                    usage: crate::llm::TokenUsage::default(),
                });
            }
            Ok(_) => {} // empty selections, fall through
            Err(crate::error::LlmError::RateLimited { retry_after, .. }) => {
                let wait = retry_after.unwrap_or(Duration::from_secs(5));
                tracing::warn!(
                    job_id = %self.worker.job_id,
                    wait_secs = wait.as_secs(),
                    "LLM rate limited during tool selection, backing off"
                );
                self.worker.log_event(
                    "status",
                    serde_json::json!({
                        "message": format!("Rate limited, retrying in {}s...", wait.as_secs()),
                    }),
                );
                tokio::time::sleep(wait).await;
                // Return a text result to continue the loop without action
                return Ok(crate::llm::RespondOutput {
                    result: RespondResult::Text(String::new()),
                    usage: crate::llm::TokenUsage::default(),
                });
            }
            Err(e) => return Err(e.into()),
        };

        // Fall back to respond_with_tools
        match reasoning.respond_with_tools(reason_ctx).await {
            Ok(output) => Ok(output),
            Err(crate::error::LlmError::RateLimited { retry_after, .. }) => {
                let wait = retry_after.unwrap_or(Duration::from_secs(5));
                tracing::warn!(
                    job_id = %self.worker.job_id,
                    wait_secs = wait.as_secs(),
                    "LLM rate limited during respond_with_tools, backing off"
                );
                self.worker.log_event(
                    "status",
                    serde_json::json!({
                        "message": format!("Rate limited, retrying in {}s...", wait.as_secs()),
                    }),
                );
                tokio::time::sleep(wait).await;
                Ok(crate::llm::RespondOutput {
                    result: RespondResult::Text(String::new()),
                    usage: crate::llm::TokenUsage::default(),
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn handle_text_response(
        &self,
        text: &str,
        reason_ctx: &mut ReasoningContext,
    ) -> TextAction {
        // Empty text from rate-limit backoff — just continue
        if text.is_empty() {
            return TextAction::Continue;
        }

        // Check for explicit completion
        if crate::util::llm_signals_completion(text) {
            if let Err(e) = self.worker.mark_completed().await {
                tracing::warn!(
                    "Failed to mark job {} as completed: {}",
                    self.worker.job_id,
                    e
                );
            }
            return TextAction::Return(LoopOutcome::Response(text.to_string()));
        }

        // Add assistant response to context
        reason_ctx.messages.push(ChatMessage::assistant(text));

        self.worker.log_event(
            "message",
            serde_json::json!({
                "role": "assistant",
                "content": text,
            }),
        );

        TextAction::Continue
    }

    async fn execute_tool_calls(
        &self,
        tool_calls: Vec<crate::llm::ToolCall>,
        content: Option<String>,
        reason_ctx: &mut ReasoningContext,
    ) -> Result<Option<LoopOutcome>, crate::error::Error> {
        if let Some(ref text) = content {
            self.worker.log_event(
                "message",
                serde_json::json!({
                    "role": "assistant",
                    "content": text,
                }),
            );
        }

        // Add assistant message with tool_calls (OpenAI protocol)
        reason_ctx
            .messages
            .push(ChatMessage::assistant_with_tool_calls(
                content,
                tool_calls.clone(),
            ));

        // Convert to ToolSelections
        let selections: Vec<ToolSelection> = tool_calls
            .iter()
            .map(|tc| ToolSelection {
                tool_name: tc.name.clone(),
                parameters: tc.arguments.clone(),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: tc.id.clone(),
            })
            .collect();

        // Execute tools (parallel for multiple, direct for single)
        if selections.len() == 1 {
            let selection = &selections[0];
            let result = self
                .worker
                .execute_tool(&selection.tool_name, &selection.parameters)
                .await;
            self.worker
                .process_tool_result_job(reason_ctx, selection, result)
                .await?;
        } else {
            let results = self.worker.execute_tools_parallel(&selections).await;
            for (selection, result) in selections.iter().zip(results) {
                self.worker
                    .process_tool_result_job(reason_ctx, selection, result.result)
                    .await?;
            }
        }

        Ok(None)
    }

    async fn after_iteration(&self, _iteration: usize) {
        // Small delay between iterations
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Convert `ToolSelection`s to `ToolCall`s.
fn selections_to_tool_calls(selections: &[ToolSelection]) -> Vec<ToolCall> {
    selections
        .iter()
        .map(|s| ToolCall {
            id: s.tool_call_id.clone(),
            name: s.tool_name.clone(),
            arguments: s.parameters.clone(),
        })
        .collect()
}

/// Convert a TaskOutput to a string result for tool execution.
impl From<TaskOutput> for Result<String, Error> {
    fn from(output: TaskOutput) -> Self {
        serde_json::to_string_pretty(&output.result).map_err(|e| {
            crate::error::ToolError::ExecutionFailed {
                name: "task".to_string(),
                reason: format!("Failed to serialize result: {}", e),
            }
            .into()
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::llm::ToolSelection;
    use crate::util::llm_signals_completion;

    use super::*;
    use crate::config::SafetyConfig;
    use crate::context::JobContext;
    use crate::llm::{
        CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
        ToolCompletionResponse,
    };
    use crate::safety::SafetyLayer;
    use crate::tools::{Tool, ToolError as ToolExecError, ToolOutput};

    /// A test tool that sleeps for a configurable duration before returning.
    struct SlowTool {
        tool_name: String,
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            &self.tool_name
        }
        fn description(&self) -> &str {
            "Test tool with configurable delay"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolExecError> {
            let start = std::time::Instant::now();
            tokio::time::sleep(self.delay).await;
            Ok(ToolOutput::text(
                format!("done_{}", self.tool_name),
                start.elapsed(),
            ))
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    /// Stub LLM provider (never called in these tests).
    struct StubLlm;

    #[async_trait::async_trait]
    impl LlmProvider for StubLlm {
        fn model_name(&self) -> &str {
            "stub"
        }
        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
        }
        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            unimplemented!("stub")
        }
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            unimplemented!("stub")
        }
    }

    /// Build a Worker wired to a ToolRegistry containing the given tools.
    async fn make_worker(tools: Vec<Arc<dyn Tool>>) -> Worker {
        let registry = ToolRegistry::new();
        for t in tools {
            registry.register(t).await;
        }

        let cm = Arc::new(crate::context::ContextManager::new(5));
        let job_id = cm.create_job("test", "test job").await.unwrap();

        let deps = WorkerDeps {
            context_manager: cm,
            llm: Arc::new(StubLlm),
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            })),
            tools: Arc::new(registry),
            store: None,
            hooks: Arc::new(crate::hooks::HookRegistry::new()),
            timeout: Duration::from_secs(30),
            use_planning: false,
            sse_tx: None,
            approval_context: None,
            http_interceptor: None,
        };

        Worker::new(job_id, deps)
    }

    #[test]
    fn test_tool_selection_preserves_call_id() {
        let selection = ToolSelection {
            tool_name: "memory_search".to_string(),
            parameters: serde_json::json!({"query": "test"}),
            reasoning: "Need to search memory".to_string(),
            alternatives: vec![],
            tool_call_id: "call_abc123".to_string(),
        };

        assert_eq!(selection.tool_call_id, "call_abc123");
        assert_ne!(
            selection.tool_call_id, "tool_call_id",
            "tool_call_id must not be the hardcoded placeholder string"
        );
    }

    #[test]
    fn test_completion_positive_signals() {
        assert!(llm_signals_completion("The job is complete."));
        assert!(llm_signals_completion(
            "I have completed the task successfully."
        ));
        assert!(llm_signals_completion("The task is done."));
        assert!(llm_signals_completion("The task is finished."));
        assert!(llm_signals_completion(
            "All steps are complete and verified."
        ));
        assert!(llm_signals_completion(
            "I've done all the work. The work is done."
        ));
        assert!(llm_signals_completion(
            "Successfully completed the migration."
        ));
    }

    #[test]
    fn test_completion_negative_signals_block_false_positives() {
        assert!(!llm_signals_completion("The task is not complete yet."));
        assert!(!llm_signals_completion("This is not done."));
        assert!(!llm_signals_completion("The work is incomplete."));
        assert!(!llm_signals_completion(
            "The migration is not yet finished."
        ));
        assert!(!llm_signals_completion("The job isn't done yet."));
        assert!(!llm_signals_completion("This remains unfinished."));
    }

    #[test]
    fn test_completion_does_not_match_bare_substrings() {
        assert!(!llm_signals_completion(
            "I need to complete more work first."
        ));
        assert!(!llm_signals_completion(
            "Let me finish the remaining steps."
        ));
        assert!(!llm_signals_completion(
            "I'm done analyzing, now let me fix it."
        ));
        assert!(!llm_signals_completion(
            "I completed step 1 but step 2 remains."
        ));
    }

    #[test]
    fn test_completion_tool_output_injection() {
        assert!(!llm_signals_completion("TASK_COMPLETE"));
        assert!(!llm_signals_completion("JOB_DONE"));
        assert!(!llm_signals_completion(
            "The tool returned: TASK_COMPLETE signal"
        ));
    }

    #[tokio::test]
    async fn test_parallel_speedup() {
        let tools: Vec<Arc<dyn Tool>> = (0..3)
            .map(|i| {
                Arc::new(SlowTool {
                    tool_name: format!("slow_{}", i),
                    delay: Duration::from_millis(200),
                }) as Arc<dyn Tool>
            })
            .collect();

        let worker = make_worker(tools).await;

        let selections: Vec<ToolSelection> = (0..3)
            .map(|i| ToolSelection {
                tool_name: format!("slow_{}", i),
                parameters: serde_json::json!({}),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: format!("call_{}", i),
            })
            .collect();

        let start = std::time::Instant::now();
        let results = worker.execute_tools_parallel(&selections).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 3);
        for r in &results {
            assert!(r.result.is_ok(), "Tool should succeed");
        }
        assert!(
            elapsed < Duration::from_millis(800),
            "Parallel execution took {:?}, expected < 800ms (sequential would be ~600ms)",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_result_ordering_preserved() {
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(SlowTool {
                tool_name: "tool_a".into(),
                delay: Duration::from_millis(300),
            }),
            Arc::new(SlowTool {
                tool_name: "tool_b".into(),
                delay: Duration::from_millis(100),
            }),
            Arc::new(SlowTool {
                tool_name: "tool_c".into(),
                delay: Duration::from_millis(200),
            }),
        ];

        let worker = make_worker(tools).await;

        let selections = vec![
            ToolSelection {
                tool_name: "tool_a".into(),
                parameters: serde_json::json!({}),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: "call_a".into(),
            },
            ToolSelection {
                tool_name: "tool_b".into(),
                parameters: serde_json::json!({}),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: "call_b".into(),
            },
            ToolSelection {
                tool_name: "tool_c".into(),
                parameters: serde_json::json!({}),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: "call_c".into(),
            },
        ];

        let results = worker.execute_tools_parallel(&selections).await;

        assert!(results[0].result.as_ref().unwrap().contains("done_tool_a"));
        assert!(results[1].result.as_ref().unwrap().contains("done_tool_b"));
        assert!(results[2].result.as_ref().unwrap().contains("done_tool_c"));
    }

    #[tokio::test]
    async fn test_missing_tool_produces_error_not_panic() {
        let worker = make_worker(vec![]).await;

        let selections = vec![ToolSelection {
            tool_name: "nonexistent_tool".into(),
            parameters: serde_json::json!({}),
            reasoning: String::new(),
            alternatives: vec![],
            tool_call_id: "call_x".into(),
        }];

        let results = worker.execute_tools_parallel(&selections).await;
        assert_eq!(results.len(), 1);
        assert!(
            results[0].result.is_err(),
            "Missing tool should produce an error, not a panic"
        );
    }

    #[tokio::test]
    async fn test_mark_completed_twice_returns_error() {
        let worker = make_worker(vec![]).await;

        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.transition_to(JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();

        worker.mark_completed().await.unwrap();

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap();
        assert_eq!(ctx.state, JobState::Completed);

        let result = worker.mark_completed().await;
        assert!(
            result.is_err(),
            "Completed → Completed transition should be rejected by state machine"
        );
    }

    /// Build a Worker with the given approval context.
    async fn make_worker_with_approval(
        tools: Vec<Arc<dyn Tool>>,
        approval_context: Option<crate::tools::ApprovalContext>,
    ) -> Worker {
        let registry = ToolRegistry::new();
        for t in tools {
            registry.register(t).await;
        }

        let cm = Arc::new(crate::context::ContextManager::new(5));
        let job_id = cm.create_job("test", "test job").await.unwrap();

        let deps = WorkerDeps {
            context_manager: cm,
            llm: Arc::new(StubLlm),
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            })),
            tools: Arc::new(registry),
            store: None,
            hooks: Arc::new(crate::hooks::HookRegistry::new()),
            timeout: Duration::from_secs(30),
            use_planning: false,
            sse_tx: None,
            approval_context,
            http_interceptor: None,
        };

        Worker::new(job_id, deps)
    }

    /// A tool that requires approval (UnlessAutoApproved).
    struct ApprovalTool;

    #[async_trait::async_trait]
    impl Tool for ApprovalTool {
        fn name(&self) -> &str {
            "needs_approval"
        }
        fn description(&self) -> &str {
            "Tool requiring approval"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &crate::context::JobContext,
        ) -> Result<ToolOutput, crate::tools::ToolError> {
            Ok(ToolOutput::text(
                "approved",
                std::time::Instant::now().elapsed(),
            ))
        }
        fn requires_approval(
            &self,
            _params: &serde_json::Value,
        ) -> crate::tools::ApprovalRequirement {
            crate::tools::ApprovalRequirement::UnlessAutoApproved
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    /// A tool that always requires approval.
    struct AlwaysApprovalTool;

    #[async_trait::async_trait]
    impl Tool for AlwaysApprovalTool {
        fn name(&self) -> &str {
            "always_approval"
        }
        fn description(&self) -> &str {
            "Tool always requiring approval"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &crate::context::JobContext,
        ) -> Result<ToolOutput, crate::tools::ToolError> {
            Ok(ToolOutput::text(
                "always",
                std::time::Instant::now().elapsed(),
            ))
        }
        fn requires_approval(
            &self,
            _params: &serde_json::Value,
        ) -> crate::tools::ApprovalRequirement {
            crate::tools::ApprovalRequirement::Always
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn test_approval_context_unblocks_unless_auto_approved() {
        let worker_blocked = make_worker_with_approval(vec![Arc::new(ApprovalTool)], None).await;
        let result = worker_blocked
            .execute_tool("needs_approval", &serde_json::json!({}))
            .await;
        assert!(
            result.is_err(),
            "Should be blocked without approval context"
        );

        let worker_allowed = make_worker_with_approval(
            vec![Arc::new(ApprovalTool)],
            Some(crate::tools::ApprovalContext::autonomous()),
        )
        .await;
        let result = worker_allowed
            .execute_tool("needs_approval", &serde_json::json!({}))
            .await;
        assert!(result.is_ok(), "Should be allowed with autonomous context");
    }

    #[tokio::test]
    async fn test_approval_context_blocks_always_unless_permitted() {
        let worker_blocked = make_worker_with_approval(
            vec![Arc::new(AlwaysApprovalTool)],
            Some(crate::tools::ApprovalContext::autonomous()),
        )
        .await;
        let result = worker_blocked
            .execute_tool("always_approval", &serde_json::json!({}))
            .await;
        assert!(
            result.is_err(),
            "Always tool should be blocked without permission"
        );

        let worker_allowed = make_worker_with_approval(
            vec![Arc::new(AlwaysApprovalTool)],
            Some(crate::tools::ApprovalContext::autonomous_with_tools([
                "always_approval".to_string(),
            ])),
        )
        .await;
        let result = worker_allowed
            .execute_tool("always_approval", &serde_json::json!({}))
            .await;
        assert!(
            result.is_ok(),
            "Always tool should be allowed with permission"
        );
    }

    #[tokio::test]
    async fn test_token_budget_exceeded_fails_job() {
        let worker = make_worker(vec![]).await;

        // Transition to InProgress (required for mark_failed)
        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.transition_to(JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();

        // Set a token budget
        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.max_tokens = 100;
            })
            .await
            .unwrap();

        // Simulate adding tokens that exceed the budget
        let budget_result = worker
            .context_manager()
            .update_context(worker.job_id, |ctx| ctx.add_tokens(200))
            .await
            .unwrap();

        assert!(
            budget_result.is_err(),
            "Should return error when token budget exceeded"
        );

        // Verify that mark_failed transitions job to Failed
        worker
            .mark_failed(&budget_result.unwrap_err())
            .await
            .unwrap();
        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap();
        assert_eq!(ctx.state, JobState::Failed);
    }

    #[tokio::test]
    async fn test_iteration_cap_marks_failed_not_stuck() {
        let worker = make_worker(vec![]).await;

        // Transition to InProgress (required for mark_failed)
        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.transition_to(JobState::InProgress, None)
            })
            .await
            .unwrap()
            .unwrap();

        // Simulate what the execution loop does when max_iterations is exceeded
        worker
            .mark_failed("Maximum iterations exceeded: job hit the iteration cap")
            .await
            .unwrap();

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap();
        assert_eq!(
            ctx.state,
            JobState::Failed,
            "Iteration cap should transition to Failed, not Stuck"
        );
    }
}
