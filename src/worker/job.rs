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
use crate::channels::web::types::ToolDecisionDto;
use crate::context::{ContextManager, JobState};
use crate::error::Error;
use crate::hooks::HookRegistry;
use crate::llm::{
    ActionPlan, ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult,
    ResponseMetadata, ToolCall, ToolSelection,
};
use crate::tenant::AdminScope;
use crate::tools::execute::process_tool_result;
use crate::tools::rate_limiter::RateLimitResult;
use crate::tools::{ApprovalContext, ToolRegistry, prepare_tool_params, redact_params};
use crate::worker::autonomous_recovery::{
    AutonomousRecoveryAction, AutonomousRecoveryState, EMPTY_TOOL_COMPLETION_FAILURE,
    EMPTY_TOOL_COMPLETION_NUDGE, FORCE_TEXT_RECOVERY_PROMPT,
};
use ironclaw_common::AppEvent;
use ironclaw_safety::SafetyLayer;

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
    pub store: Option<AdminScope>,
    pub hooks: Arc<HookRegistry>,
    pub timeout: Duration,
    pub use_planning: bool,
    /// Broadcast sender for live job event streaming to the web gateway.
    pub sse_tx: Option<Arc<crate::channels::web::sse::SseManager>>,
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

    fn store(&self) -> Option<&AdminScope> {
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
        if let Some(ref sse) = self.deps.sse_tx {
            let job_id_str = job_id.to_string();
            let event = match event_type {
                "message" => Some(AppEvent::JobMessage {
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
                "tool_use" => Some(AppEvent::JobToolUse {
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
                "tool_result" => Some(AppEvent::JobToolResult {
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
                "status" => Some(AppEvent::JobStatus {
                    job_id: job_id_str,
                    message: data
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                }),
                "result" => Some(AppEvent::JobResult {
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
                    fallback_deliverable: data.get("fallback_deliverable").cloned(),
                }),
                "reasoning" => {
                    let narrative = data
                        .get("narrative")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let decisions = ToolDecisionDto::from_json_array(&data["decisions"]);
                    Some(AppEvent::JobReasoning {
                        job_id: job_id_str,
                        narrative,
                        decisions,
                    })
                }
                _ => None,
            };
            if let Some(event) = event {
                sse.broadcast(event);
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
        let reasoning =
            Reasoning::new(self.llm().clone()).with_model_name(self.llm().active_model_name());

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
        let user_id = job_ctx.user_id.clone();
        let scopes = job_ctx.workspace_read_scopes.clone();
        let result = tokio::time::timeout(self.timeout(), async {
            self.execution_loop(&mut rx, &reasoning, &mut reason_ctx, &user_id, &scopes)
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
        user_id: &str,
        workspace_read_scopes: &[String],
    ) -> Result<(), Error> {
        let max_iterations = self
            .context_manager()
            .get_context(self.job_id)
            .await
            .ok()
            .and_then(|ctx| ctx.metadata.get("max_iterations").and_then(|v| v.as_u64()))
            .unwrap_or(50) as usize;
        let max_iterations = max_iterations.min(ironclaw_common::MAX_WORKER_ITERATIONS as usize);

        // Initial tool definitions for planning (will be refreshed in loop)
        reason_ctx.available_tools = self
            .tools()
            .tool_definitions_for_user(user_id, workspace_read_scopes)
            .await;

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
            consecutive_rate_limits: std::sync::atomic::AtomicUsize::new(0),
            recovery_state: tokio::sync::Mutex::new(AutonomousRecoveryState::default()),
            has_text_response: std::sync::atomic::AtomicBool::new(false),
            user_id: user_id.to_string(),
            workspace_read_scopes: workspace_read_scopes.to_vec(),
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
            LoopOutcome::Failure(reason) => {
                self.mark_failed(&reason).await?;
            }
            LoopOutcome::Stopped => {
                // Stop signal handled — nothing more to do
            }
            LoopOutcome::NeedApproval(_) => {}
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

        let normalized_params = prepare_tool_params(tool.as_ref(), params);

        // Fetch job context early for approval checking and other needs
        let mut job_ctx = deps.context_manager.get_context(job_id).await?;

        // Check approval: additive semantics - BOTH job-level AND worker-level must approve
        let requirement = tool.requires_approval(&normalized_params);

        // Check job-level approval context (if set by tools like the builder)
        let job_level_blocked = job_ctx
            .approval_context
            .as_ref()
            .map(|ctx| ctx.is_blocked(tool_name, requirement))
            .unwrap_or(false);

        // Check worker-level approval context (set by scheduler for autonomous jobs)
        let worker_level_blocked =
            ApprovalContext::is_blocked_or_default(&deps.approval_context, tool_name, requirement);

        // Tool is blocked if EITHER level blocks it (additive/intersection semantics)
        // This maintains defense in depth: job-level cannot bypass worker-level restrictions
        if job_level_blocked || worker_level_blocked {
            let reason = if job_level_blocked && worker_level_blocked {
                format!(
                    "Tool '{}' is blocked by both job-level and worker-level approval context",
                    tool_name
                )
            } else if job_level_blocked {
                format!(
                    "Tool '{}' is not in the job-level allowed tools list",
                    tool_name
                )
            } else {
                format!(
                    "Tool '{}' is not available for autonomous execution",
                    tool_name
                )
            };
            return Err(crate::error::ToolError::AutonomousUnavailable {
                name: tool_name.to_string(),
                reason,
            }
            .into());
        }

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
        let effective_params = {
            use crate::hooks::{HookError, HookEvent, HookOutcome};
            let hook_params = redact_params(&normalized_params, tool.sensitive_params());
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
                }) => match serde_json::from_str(&new_params) {
                    // Hook output is fresh JSON text and may reintroduce stringified scalars or
                    // containers, so we normalize it again. The fallback path reuses the already
                    // normalized input because no hook mutation was applied.
                    Ok(parsed) => prepare_tool_params(tool.as_ref(), &parsed),
                    Err(e) => {
                        tracing::warn!(
                            tool = %tool_name,
                            "Hook returned non-JSON modification for ToolCall, ignoring: {}",
                            e
                        );
                        normalized_params
                    }
                },
                _ => normalized_params,
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
        let validation = deps
            .safety
            .validator()
            .validate_tool_params(&effective_params);
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
        let safe_params = redact_params(&effective_params, tool.sensitive_params());
        let risk = tool.risk_level_for(&effective_params);
        tracing::debug!(
            tool = %tool_name,
            params = %safe_params,
            job = %job_id,
            risk = %risk,
            "Tool call started"
        );

        // Execute with per-tool timeout and timing
        let tool_timeout = tool.execution_timeout();
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(tool_timeout, async {
            tool.execute(effective_params.clone(), &job_ctx).await
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
    ) -> Result<(), Error> {
        self.log_event(
            "tool_use",
            serde_json::json!({
                "tool_name": selection.tool_name,
                "input": truncate_for_preview(
                    &selection.parameters.to_string(), 500),
            }),
        );

        // Use shared result processing for sanitize → wrap → ChatMessage.
        // The wrapped content (XML tags) goes into reason_ctx for the LLM.
        // The raw sanitized content goes into events/SSE for human-readable UI.
        let (_wrapped, message) = process_tool_result(
            &self.deps.safety,
            &selection.tool_name,
            &selection.tool_call_id,
            &result,
        );
        reason_ctx.messages.push(message);

        match result {
            Ok(raw_output) => {
                let sanitized = self
                    .deps
                    .safety
                    .sanitize_tool_output(&selection.tool_name, &raw_output);
                self.log_event(
                    "tool_result",
                    serde_json::json!({
                        "tool_name": selection.tool_name,
                        "success": true,
                        "output": truncate_for_preview(&sanitized.content, 500),
                    }),
                );
                Ok(())
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

                let error_preview = {
                    let msg = format!("Error: {}", e);
                    truncate_for_preview(&msg, 500).into_owned()
                };
                self.log_event(
                    "tool_result",
                    serde_json::json!({
                        "tool_name": selection.tool_name,
                        "success": false,
                        "output": error_preview,
                    }),
                );

                // All tool errors (including AutonomousUnavailable) are
                // recoverable — the error message is already recorded in
                // reason_ctx so the LLM can see it and try a different
                // approach. Returning Err here would kill the entire job.
                Ok(())
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
                        reasoning: if action.reasoning.is_empty() {
                            None
                        } else {
                            Some(action.reasoning.clone())
                        },
                    }],
                ));

            let result = self
                .execute_tool(&action.tool_name, &action.parameters)
                .await;

            self.process_tool_result_job(reason_ctx, &selection, result)
                .await?;

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Plan completed — ask the LLM whether the job is done.
        let msg_count_before = reason_ctx.messages.len();
        reason_ctx.messages.push(ChatMessage::user(
            "All planned actions have been executed. Assess the results: \
             if the job is fully complete, state that the job is complete. \
             Otherwise, briefly list what remains.",
        ));

        let response = reasoning.respond(reason_ctx).await?;
        let response = crate::agent::strip_suggestions(&response);

        if crate::util::llm_signals_completion(&response) {
            reason_ctx.messages.push(ChatMessage::assistant(&response));
            self.mark_completed().await?;
        } else {
            // Replace the completion-check exchange with an action-oriented
            // continuation prompt. Leaving the "Is the job complete?" / "No"
            // dialogue in context causes the agentic loop to repeat the same
            // analysis instead of calling tools (self-dialogue loop).
            reason_ctx.messages.truncate(msg_count_before);
            reason_ctx.messages.push(ChatMessage::user(format!(
                "The planned actions are done but the job is not yet complete. \
                 Remaining work:\n\n{response}\n\n\
                 Continue executing now — use tools to finish the job."
            )));
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
        // Build fallback deliverable from memory before transitioning.
        let fallback = self.build_fallback(reason).await;

        self.context_manager()
            .update_context(self.job_id, |ctx| {
                ctx.transition_to(JobState::Failed, Some(reason.to_string()))?;
                store_fallback_in_metadata(ctx, fallback.as_ref());
                Ok(())
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
        // Build fallback deliverable from memory before transitioning.
        let fallback = self.build_fallback(reason).await;

        self.context_manager()
            .update_context(self.job_id, |ctx| {
                ctx.mark_stuck(reason)?;
                store_fallback_in_metadata(ctx, fallback.as_ref());
                Ok(())
            })
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

    /// Build a [`FallbackDeliverable`] from the current job context and memory.
    async fn build_fallback(&self, reason: &str) -> Option<crate::context::FallbackDeliverable> {
        let memory = match self.context_manager().get_memory(self.job_id).await {
            Ok(memory) => memory,
            Err(e) => {
                tracing::warn!(
                    job_id = %self.job_id,
                    "Failed to load memory while building fallback deliverable: {e}"
                );
                return None;
            }
        };
        let ctx = match self.context_manager().get_context(self.job_id).await {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::warn!(
                    job_id = %self.job_id,
                    "Failed to load context while building fallback deliverable: {e}"
                );
                return None;
            }
        };
        Some(crate::context::FallbackDeliverable::build(
            &ctx, &memory, reason,
        ))
    }
}

/// Store a fallback deliverable in the job context's metadata.
fn store_fallback_in_metadata(
    ctx: &mut crate::context::JobContext,
    fallback: Option<&crate::context::FallbackDeliverable>,
) {
    let Some(fb) = fallback else {
        return;
    };
    match serde_json::to_value(fb) {
        Ok(val) => {
            if !ctx.metadata.is_object() {
                ctx.metadata = serde_json::json!({});
            }
            ctx.metadata["fallback_deliverable"] = val;
        }
        Err(e) => {
            tracing::warn!(
                "Failed to serialize fallback deliverable for job {}: {e}",
                ctx.job_id
            );
        }
    }
}

/// Job delegate: implements `LoopDelegate` for the background job context.
/// Whether an LLM error represents a completion-eligible empty response.
///
/// Only `EmptyResponse` (provider returned no choices/content) qualifies.
/// Infrastructure errors (`AuthFailed`, `Http`, `Io`, etc.) never qualify —
/// they must propagate even if prior text output was produced.
fn is_completion_eligible_error(error: &crate::error::LlmError) -> bool {
    matches!(error, crate::error::LlmError::EmptyResponse { .. })
}

///
/// Handles: signal channel (stop/ping/user messages), cancellation checks,
/// rate-limit retry, parallel tool execution, DB persistence, SSE broadcasting.
struct JobDelegate<'a> {
    worker: &'a Worker,
    rx: tokio::sync::Mutex<&'a mut mpsc::Receiver<WorkerMessage>>,
    /// Tracks consecutive rate-limit errors to fail fast instead of burning iterations.
    consecutive_rate_limits: std::sync::atomic::AtomicUsize,
    recovery_state: tokio::sync::Mutex<AutonomousRecoveryState>,
    /// Whether a substantive (non-empty) text response has been produced.
    /// When true, an empty follow-up response is treated as job completion
    /// rather than a retry signal (prevents spurious failures in routines).
    has_text_response: std::sync::atomic::AtomicBool,
    /// User ID for scoping tool definitions to the owning user.
    user_id: String,
    /// Additional workspace scopes for cross-lens tool visibility.
    workspace_read_scopes: Vec<String>,
}

impl<'a> JobDelegate<'a> {
    const MAX_CONSECUTIVE_RATE_LIMITS: usize = 10;

    /// Handle a rate-limit error: back off, increment counter, and fail fast
    /// if the provider remains rate-limited for too many consecutive attempts.
    async fn handle_rate_limit(
        &self,
        retry_after: Option<Duration>,
        context: &str,
    ) -> Result<crate::llm::RespondOutput, crate::error::Error> {
        use std::sync::atomic::Ordering::Relaxed;

        let count = self.consecutive_rate_limits.fetch_add(1, Relaxed) + 1;
        let wait = retry_after.unwrap_or(Duration::from_secs(5));
        tracing::warn!(
            job_id = %self.worker.job_id,
            wait_secs = wait.as_secs(),
            attempt = count,
            "LLM rate limited during {}, backing off",
            context,
        );

        if count >= Self::MAX_CONSECUTIVE_RATE_LIMITS {
            self.worker
                .mark_failed("Persistent rate limiting: exceeded retry limit")
                .await?;
            return Err(crate::error::LlmError::RateLimited {
                provider: "rate-limit-exhausted".to_string(),
                retry_after: None,
            }
            .into());
        }

        self.worker.log_event(
            "status",
            serde_json::json!({
                "message": format!(
                    "Rate limited, retrying in {}s... ({}/{})",
                    wait.as_secs(), count, Self::MAX_CONSECUTIVE_RATE_LIMITS
                ),
            }),
        );
        tokio::time::sleep(wait).await;

        Ok(crate::llm::RespondOutput {
            result: RespondResult::Text(String::new()),
            usage: crate::llm::TokenUsage::default(),
            finish_reason: crate::llm::FinishReason::Stop,
            metadata: ResponseMetadata::default(),
        })
    }

    /// Mark the job as completed, logging a warning on failure.
    async fn mark_completed_or_warn(&self, context: &str) {
        if let Err(e) = self.worker.mark_completed().await {
            tracing::warn!(
                job_id = %self.worker.job_id,
                error = %e,
                "Failed to mark job completed ({context})"
            );
        }
    }

    /// If a substantive text response was already produced and the error
    /// indicates the LLM simply returned nothing, treat it as successful
    /// completion rather than a fatal failure.
    ///
    /// Only swallows `EmptyResponse` — infrastructure errors (`AuthFailed`,
    /// `ContextLengthExceeded`, `Http`, `Io`, etc.) always propagate.
    ///
    /// Returns `Some(empty RespondOutput)` when the error should be swallowed,
    /// `None` when it should propagate normally.
    async fn try_complete_on_error(
        &self,
        context: &str,
        error: &crate::error::LlmError,
    ) -> Option<crate::llm::RespondOutput> {
        if !is_completion_eligible_error(error) {
            return None;
        }
        if !self
            .has_text_response
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return None;
        }
        tracing::info!(
            job_id = %self.worker.job_id,
            error = %error,
            "{context} empty response after text output — treating as completion"
        );
        self.mark_completed_or_warn(context).await;
        Some(crate::llm::RespondOutput {
            result: RespondResult::Text(String::new()),
            usage: crate::llm::TokenUsage::default(),
            finish_reason: crate::llm::FinishReason::Stop,
            metadata: ResponseMetadata::default(),
        })
    }
}

#[async_trait]
impl<'a> LoopDelegate for JobDelegate<'a> {
    async fn check_signals(&self) -> LoopSignal {
        // Drain the entire message channel, prioritizing Stop over user messages.
        // Scope the lock so it's dropped before any .await below.
        let mut stop_requested = false;
        let mut first_user_message: Option<String> = None;
        {
            let mut rx = self.rx.lock().await;
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMessage::Stop => {
                        tracing::debug!(
                            "Worker for job {} received stop signal",
                            self.worker.job_id
                        );
                        stop_requested = true;
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
                        // Keep only the first user message; subsequent ones will be
                        // picked up on the next iteration's drain.
                        if first_user_message.is_none() {
                            first_user_message = Some(content);
                        }
                    }
                }
            }
        } // MutexGuard dropped here, before the cancellation .await

        // Stop takes priority over user messages
        if stop_requested {
            return LoopSignal::Stop;
        }

        if let Some(content) = first_user_message {
            return LoopSignal::InjectMessage(content);
        }

        // Check for terminal or post-completion state. The loop should stop when the
        // job has been cancelled, failed, or already completed — but NOT when Stuck,
        // because Stuck is recoverable (Stuck -> InProgress via self-repair).
        // Stopping on Stuck would prevent recovery from resuming the worker (issue #892).
        if let Ok(ctx) = self
            .worker
            .context_manager()
            .get_context(self.worker.job_id)
            .await
            && matches!(
                ctx.state,
                JobState::Cancelled
                    | JobState::Failed
                    | JobState::Completed
                    | JobState::Submitted
                    | JobState::Accepted
            )
        {
            tracing::info!(
                "Worker for job {} detected terminal state {:?}",
                self.worker.job_id,
                ctx.state,
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
        let force_text_recovery = {
            let mut recovery = self.recovery_state.lock().await;
            recovery.begin_iteration()
        };

        if force_text_recovery {
            tracing::warn!(
                job_id = %self.worker.job_id,
                "Switching to text-only recovery after malformed tool completions"
            );
            reason_ctx.available_tools.clear();
        } else {
            // Refresh tool definitions so newly built tools become visible
            reason_ctx.available_tools = self
                .worker
                .tools()
                .tool_definitions_for_user(&self.user_id, &self.workspace_read_scopes)
                .await;
        }

        // Claude 4.6 rejects assistant prefill; NEAR AI rejects any non-user-ending
        // conversation. Ensure the last message is user-role before calling the LLM.
        crate::util::ensure_ends_with_user_message(&mut reason_ctx.messages);

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
                // Reset counter after a successful LLM call
                self.consecutive_rate_limits
                    .store(0, std::sync::atomic::Ordering::Relaxed);
                // Preserve the LLM's reasoning text so it appears in the
                // assistant_with_tool_calls message pushed by execute_tool_calls.
                let reasoning_text = s
                    .iter()
                    .find_map(|sel| (!sel.reasoning.is_empty()).then_some(sel.reasoning.clone()));
                let tool_calls: Vec<ToolCall> = selections_to_tool_calls(&s);
                return Ok(crate::llm::RespondOutput {
                    result: RespondResult::ToolCalls {
                        tool_calls,
                        content: reasoning_text,
                    },
                    usage: crate::llm::TokenUsage::default(),
                    finish_reason: crate::llm::FinishReason::ToolUse,
                    metadata: ResponseMetadata::default(),
                });
            }
            Ok(_) => {} // empty selections, fall through
            Err(crate::error::LlmError::RateLimited { retry_after, .. }) => {
                return self.handle_rate_limit(retry_after, "tool selection").await;
            }
            Err(e) => {
                if let Some(output) = self.try_complete_on_error("select_tools", &e).await {
                    return Ok(output);
                }
                return Err(e.into());
            }
        };

        // Fall back to respond_with_tools
        match reasoning.respond_with_tools(reason_ctx).await {
            Ok(output) => {
                // Reset counter after a successful LLM call
                self.consecutive_rate_limits
                    .store(0, std::sync::atomic::Ordering::Relaxed);

                // Track token usage against the job budget.
                // NOTE: select_tools() also makes LLM calls but doesn't expose
                // TokenUsage; only respond_with_tools() usage is tracked here.
                let total_tokens = output.usage.total() as u64;
                if total_tokens > 0
                    && let Err(err) = self
                        .worker
                        .context_manager()
                        .update_context(self.worker.job_id, |ctx| ctx.add_tokens(total_tokens))
                        .await?
                {
                    self.worker.mark_failed(&err.to_string()).await?;
                }

                Ok(output)
            }
            Err(crate::error::LlmError::RateLimited { retry_after, .. }) => {
                self.handle_rate_limit(retry_after, "respond_with_tools")
                    .await
            }
            Err(e) => {
                if let Some(output) = self.try_complete_on_error("respond_with_tools", &e).await {
                    return Ok(output);
                }
                Err(e.into())
            }
        }
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
                tracing::warn!(
                    job_id = %self.worker.job_id,
                    "Malformed empty tool completion detected; retrying in tool mode"
                );
                self.worker.log_event(
                    "status",
                    serde_json::json!({
                        "message": "Model returned an empty tool-completion response; retrying with a stronger tool-use nudge.",
                    }),
                );
                reason_ctx
                    .messages
                    .push(ChatMessage::user(EMPTY_TOOL_COMPLETION_NUDGE));
                return TextAction::Continue;
            }
            AutonomousRecoveryAction::ForceTextRecovery => {
                tracing::warn!(
                    job_id = %self.worker.job_id,
                    "Repeated malformed tool completions detected; switching to text-only recovery"
                );
                self.worker.log_event(
                    "status",
                    serde_json::json!({
                        "message": "Model returned repeated empty tool-completion responses; requesting a final status update without tools.",
                    }),
                );
                reason_ctx
                    .messages
                    .push(ChatMessage::user(FORCE_TEXT_RECOVERY_PROMPT));
                return TextAction::Continue;
            }
            AutonomousRecoveryAction::Fail => {
                tracing::warn!(
                    job_id = %self.worker.job_id,
                    "Failing fast after repeated malformed autonomous responses"
                );
                return TextAction::Return(LoopOutcome::Failure(
                    EMPTY_TOOL_COMPLETION_FAILURE.to_string(),
                ));
            }
            AutonomousRecoveryAction::Continue => {}
        }

        // Empty text after a substantive response means the LLM has finished.
        // Treat as successful completion rather than continuing the loop (which
        // would produce "Response contained no message or tool call (empty)").
        if text.is_empty() {
            if self
                .has_text_response
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                tracing::debug!(
                    job_id = %self.worker.job_id,
                    "Empty response after text output — treating as completion"
                );
                self.mark_completed_or_warn("empty text response").await;
                return TextAction::Return(LoopOutcome::Response(String::new()));
            }
            // No prior text response — this is likely a rate-limit backoff retry.
            return TextAction::Continue;
        }

        // Jobs run autonomously — strip <suggestions> tags that are only
        // meaningful for interactive chat sessions.
        let text = crate::agent::strip_suggestions(text);

        // A non-empty text response with no tool intent (already filtered
        // by the agentic loop's nudge mechanism) is the LLM's final answer.
        // Mark the job complete and stop the loop. Without this, the LLM
        // restates its summary every iteration until the cap is hit.
        if let Err(e) = self.worker.mark_completed().await {
            tracing::warn!(
                "Failed to mark job {} as completed: {}",
                self.worker.job_id,
                e
            );
        }

        // Track that a substantive response has been produced.
        self.has_text_response
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Add assistant response to context
        reason_ctx.messages.push(ChatMessage::assistant(&text));

        self.worker.log_event(
            "message",
            serde_json::json!({
                "role": "assistant",
                "content": text,
            }),
        );

        TextAction::Return(LoopOutcome::Response(text))
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

        // Strip suggestions from accompanying text (not useful in job context).
        let content = content.map(|c| crate::agent::strip_suggestions(&c));

        if let Some(ref text) = content {
            self.worker.log_event(
                "message",
                serde_json::json!({
                    "role": "assistant",
                    "content": text,
                }),
            );
        }

        // Emit reasoning event if any tool calls carry reasoning.
        // Sanitize narrative and per-tool rationale through SafetyLayer
        // (parity with ChatDelegate in dispatcher.rs).
        let sanitized_narrative = content
            .as_deref()
            .filter(|c| !c.trim().is_empty())
            .map(|c| {
                self.worker
                    .deps
                    .safety
                    .sanitize_tool_output("job_narrative", c)
                    .content
            })
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_default();
        let decisions: Vec<serde_json::Value> = tool_calls
            .iter()
            .filter_map(|tc| {
                tc.reasoning.as_ref().map(|r| {
                    let sanitized = self
                        .worker
                        .deps
                        .safety
                        .sanitize_tool_output("tool_rationale", r)
                        .content;
                    serde_json::json!({
                        "tool_name": tc.name,
                        "rationale": sanitized,
                    })
                })
            })
            .collect();
        if !decisions.is_empty() {
            self.worker.log_event(
                "reasoning",
                serde_json::json!({
                    "narrative": sanitized_narrative,
                    "decisions": decisions,
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
                reasoning: tc.reasoning.clone().unwrap_or_default(),
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

    async fn on_tool_intent_nudge(&self, text: &str, _reason_ctx: &mut ReasoningContext) {
        self.worker.log_event(
            "message",
            serde_json::json!({
                "role": "assistant",
                "content": truncate_for_preview(text, 2000),
                "nudge": true,
            }),
        );
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
            reasoning: if s.reasoning.is_empty() {
                None
            } else {
                Some(s.reasoning.clone())
            },
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
    use std::sync::Arc;

    use crate::channels::ChannelManager;
    use crate::llm::ToolSelection;

    use super::*;
    use crate::config::SafetyConfig;
    use crate::context::JobContext;
    use crate::llm::{
        CompletionRequest, CompletionResponse, LlmProvider, ToolCompletionRequest,
        ToolCompletionResponse,
    };
    use crate::testing::{BroadcastCapture, RecordingBroadcastChannel};
    use crate::tools::builtin::MessageTool;
    use crate::tools::{Tool, ToolError as ToolExecError, ToolOutput};
    use ironclaw_safety::SafetyLayer;

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
        let job_id = cm.create_job("test", "test job").await.unwrap(); // safety: test

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

    async fn make_worker_with_message_tool()
    -> (Worker, Arc<MessageTool>, BroadcastCapture, BroadcastCapture) {
        let channel_manager = ChannelManager::new();
        let (gateway, gateway_captures) = RecordingBroadcastChannel::new("gateway");
        let (telegram, telegram_captures) = RecordingBroadcastChannel::new("telegram");
        channel_manager.add(Box::new(gateway)).await;
        channel_manager.add(Box::new(telegram)).await;

        let message_tool = Arc::new(MessageTool::new(Arc::new(channel_manager)));
        let worker = make_worker(vec![message_tool.clone()]).await;

        (worker, message_tool, gateway_captures, telegram_captures)
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

        assert_eq!(selection.tool_call_id, "call_abc123"); // safety: test
        assert_ne!(
            /* safety: test */
            selection.tool_call_id, "tool_call_id",
            "tool_call_id must not be the hardcoded placeholder string"
        );
    }

    // Completion detection tests live in src/util.rs (the canonical location).
    // See: test_completion_signals, test_completion_negative, etc.

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

        assert_eq!(results.len(), 3); // safety: test
        for r in &results {
            assert!(r.result.is_ok(), "Tool should succeed"); // safety: test
        }
        assert!(
            /* safety: test */
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

        assert!(results[0].result.as_ref().unwrap().contains("done_tool_a")); // safety: test
        assert!(results[1].result.as_ref().unwrap().contains("done_tool_b")); // safety: test
        assert!(results[2].result.as_ref().unwrap().contains("done_tool_c")); // safety: test
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
        assert_eq!(results.len(), 1); // safety: test
        assert!(
            /* safety: test */
            results[0].result.is_err(),
            "Missing tool should produce an error, not a panic"
        );
    }

    #[tokio::test]
    async fn test_mark_completed_twice_is_idempotent() {
        let worker = make_worker(vec![]).await;

        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.transition_to(JobState::InProgress, None)
            })
            .await
            .unwrap() // safety: test
            .unwrap(); // safety: test

        worker.mark_completed().await.unwrap(); // safety: test

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::Completed); // safety: test

        // Second mark_completed should succeed (idempotent) rather than
        // erroring, matching the fix for the execution_loop / worker wrapper
        // race condition.
        let result = worker.mark_completed().await;
        assert!(
            /* safety: test */
            result.is_ok(),
            "Completed -> Completed transition should be idempotent"
        );

        // State should still be Completed
        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap();
        assert_eq!(ctx.state, JobState::Completed);
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
        let job_id = cm.create_job("test", "test job").await.unwrap(); // safety: test

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
    async fn test_approval_context_requires_explicit_allowed_tool_names() {
        let worker_blocked = make_worker_with_approval(vec![Arc::new(ApprovalTool)], None).await;
        let result = worker_blocked
            .execute_tool("needs_approval", &serde_json::json!({}))
            .await;
        assert!(
            /* safety: test */
            result.is_err(),
            "Should be blocked without approval context"
        );

        let worker_allowed = make_worker_with_approval(
            vec![Arc::new(ApprovalTool)],
            Some(crate::tools::ApprovalContext::autonomous_with_tools([
                "needs_approval".to_string(),
            ])),
        )
        .await;
        let result = worker_allowed
            .execute_tool("needs_approval", &serde_json::json!({}))
            .await;
        assert!(
            result.is_ok(),
            "Should be allowed when the tool is in the autonomous scope"
        ); // safety: test
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
            /* safety: test */
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
            /* safety: test */
            result.is_ok(),
            "Always tool should be allowed with permission"
        );
    }

    #[tokio::test]
    async fn test_approval_context_returns_structured_autonomous_unavailable_error() {
        let worker = make_worker_with_approval(
            vec![Arc::new(AlwaysApprovalTool)],
            Some(crate::tools::ApprovalContext::autonomous()),
        )
        .await;

        let result = worker
            .execute_tool("always_approval", &serde_json::json!({}))
            .await;

        assert!(matches!(
            result,
            Err(Error::Tool(crate::error::ToolError::AutonomousUnavailable { name, .. }))
                if name == "always_approval"
        ));
    }

    #[tokio::test]
    async fn test_additive_approval_semantics_both_levels_must_approve() {
        // Test additive semantics: BOTH job-level AND worker-level must approve
        // If either blocks, the tool is blocked (defense in depth)

        // Scenario 1: Worker-level allows, job-level blocks → BLOCKED
        let worker = make_worker_with_approval(
            vec![Arc::new(AlwaysApprovalTool)],
            // Worker-level allows the tool
            Some(crate::tools::ApprovalContext::autonomous_with_tools([
                "always_approval".to_string(),
            ])),
        )
        .await;

        // Set job-level context to block the tool (by not listing it)
        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.approval_context = Some(crate::tools::ApprovalContext::autonomous());
            })
            .await
            .unwrap();

        let result = worker
            .execute_tool("always_approval", &serde_json::json!({}))
            .await;
        assert!(
            result.is_err(),
            "Tool should be blocked when job-level doesn't allow it, \
             even if worker-level does (additive semantics)"
        );
    }

    #[tokio::test]
    async fn test_additive_approval_worker_block_overrides_job_allow() {
        // Scenario 2: Job-level allows, worker-level blocks → BLOCKED
        let worker = make_worker_with_approval(
            vec![Arc::new(AlwaysApprovalTool)],
            // Worker-level does NOT allow the tool
            Some(crate::tools::ApprovalContext::autonomous()),
        )
        .await;

        // Set job-level context to allow the tool
        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.approval_context =
                    Some(crate::tools::ApprovalContext::autonomous_with_tools([
                        "always_approval".to_string(),
                    ]));
            })
            .await
            .unwrap();

        let result = worker
            .execute_tool("always_approval", &serde_json::json!({}))
            .await;
        assert!(
            result.is_err(),
            "Tool should be blocked when worker-level doesn't allow it, \
             even if job-level does (additive semantics)"
        );
    }

    #[tokio::test]
    async fn test_additive_approval_both_levels_allow() {
        // Scenario 3: Both levels allow → ALLOWED
        let worker = make_worker_with_approval(
            vec![Arc::new(AlwaysApprovalTool)],
            // Worker-level allows
            Some(crate::tools::ApprovalContext::autonomous_with_tools([
                "always_approval".to_string(),
            ])),
        )
        .await;

        // Job-level also allows
        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.approval_context =
                    Some(crate::tools::ApprovalContext::autonomous_with_tools([
                        "always_approval".to_string(),
                    ]));
            })
            .await
            .unwrap();

        let result = worker
            .execute_tool("always_approval", &serde_json::json!({}))
            .await;
        assert!(
            result.is_ok(),
            "Tool should be allowed when both job-level and worker-level allow it"
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
            .unwrap() // safety: test
            .unwrap(); // safety: test

        // Set a token budget
        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.max_tokens = 100;
            })
            .await
            .unwrap(); // safety: test

        // Simulate adding tokens that exceed the budget
        let budget_result = worker
            .context_manager()
            .update_context(worker.job_id, |ctx| ctx.add_tokens(200))
            .await
            .unwrap(); // safety: test

        assert!(
            /* safety: test */
            budget_result.is_err(),
            "Should return error when token budget exceeded"
        );

        // Verify that mark_failed transitions job to Failed
        worker
            .mark_failed(&budget_result.unwrap_err().to_string())
            .await
            .unwrap(); // safety: test
        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::Failed); // safety: test
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
            .unwrap() // safety: test
            .unwrap(); // safety: test

        // Simulate what the execution loop does when max_iterations is exceeded
        worker
            .mark_failed("Maximum iterations exceeded: job hit the iteration cap")
            .await
            .unwrap(); // safety: test

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(
            /* safety: test */
            ctx.state,
            JobState::Failed,
            "Iteration cap should transition to Failed, not Stuck"
        );
    }

    /// Regression: a text response without rigid completion phrases (e.g.
    /// "Weekly review completed and saved to Notion") must still terminate the
    /// agentic loop and mark the job complete, rather than continuing until
    /// max_iterations.
    #[tokio::test]
    async fn test_text_response_terminates_loop_without_explicit_completion_phrase() {
        let worker = make_worker(vec![]).await;
        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.transition_to(JobState::InProgress, None)
            })
            .await
            .unwrap() // safety: test
            .unwrap(); // safety: test

        let (_, mut rx) = tokio::sync::mpsc::channel(1);
        let delegate = JobDelegate {
            worker: &worker,
            rx: tokio::sync::Mutex::new(&mut rx),
            consecutive_rate_limits: std::sync::atomic::AtomicUsize::new(0),
            recovery_state: tokio::sync::Mutex::new(AutonomousRecoveryState::default()),
            has_text_response: std::sync::atomic::AtomicBool::new(false),
            user_id: "test_user".to_string(),
            workspace_read_scopes: Vec::new(),
        };

        let mut reason_ctx = ReasoningContext::new();

        // Text that a real LLM would produce but doesn't match llm_signals_completion
        let action = delegate
            .handle_text_response(
                "Weekly review created in Notion and notification sent.",
                ResponseMetadata::default(),
                &mut reason_ctx,
            )
            .await;

        assert!(
            matches!(action, TextAction::Return(_)),
            "Text response should terminate the loop, got Continue"
        ); // safety: test

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::Completed); // safety: test
    }

    /// Regression test: selections_to_tool_calls must preserve tool_call_id
    /// so that tool_result messages match the assistant_with_tool_calls message
    /// and are not treated as orphaned by sanitize_tool_messages.
    #[test]
    fn test_selections_to_tool_calls_preserves_ids() {
        let selections = vec![
            ToolSelection {
                tool_name: "search".into(),
                parameters: serde_json::json!({"q": "test"}),
                reasoning: "Need to search".into(),
                alternatives: vec![],
                tool_call_id: "call_abc".into(),
            },
            ToolSelection {
                tool_name: "fetch".into(),
                parameters: serde_json::json!({"url": "https://example.com"}),
                reasoning: "Need to fetch".into(),
                alternatives: vec![],
                tool_call_id: "call_def".into(),
            },
        ];

        let tool_calls = selections_to_tool_calls(&selections);

        assert_eq!(tool_calls.len(), 2);
        assert_eq!(tool_calls[0].id, "call_abc");
        assert_eq!(tool_calls[0].name, "search");
        assert_eq!(tool_calls[1].id, "call_def");
        assert_eq!(tool_calls[1].name, "fetch");
    }

    /// Regression test: when select_tools returns selections with reasoning,
    /// the reasoning text should be preserved as content in the RespondResult
    /// so it appears in the assistant_with_tool_calls message. Without this,
    /// the LLM's reasoning context is lost and subsequent turns lack context.
    #[test]
    fn test_reasoning_text_extraction_from_selections() {
        // Simulate what call_llm does: extract first non-empty reasoning
        let selections = [
            ToolSelection {
                tool_name: "search".into(),
                parameters: serde_json::json!({}),
                reasoning: "I need to search for relevant information".into(),
                alternatives: vec![],
                tool_call_id: "call_1".into(),
            },
            ToolSelection {
                tool_name: "fetch".into(),
                parameters: serde_json::json!({}),
                reasoning: "I need to search for relevant information".into(),
                alternatives: vec![],
                tool_call_id: "call_2".into(),
            },
        ];

        let reasoning_text = selections
            .iter()
            .find_map(|sel| (!sel.reasoning.is_empty()).then_some(sel.reasoning.clone()));

        assert_eq!(
            reasoning_text.as_deref(),
            Some("I need to search for relevant information"),
            "Reasoning text should be extracted from first non-empty selection"
        );

        // Empty reasoning should result in None
        let empty_selections = [ToolSelection {
            tool_name: "echo".into(),
            parameters: serde_json::json!({}),
            reasoning: String::new(),
            alternatives: vec![],
            tool_call_id: "call_3".into(),
        }];

        let empty_reasoning = empty_selections
            .iter()
            .find_map(|sel| (!sel.reasoning.is_empty()).then_some(sel.reasoning.clone()));

        assert!(
            empty_reasoning.is_none(),
            "Empty reasoning should not be included as content"
        );
    }

    /// When the first selection has empty reasoning but a subsequent one has
    /// non-empty reasoning, find_map should skip the empty one and return the
    /// first non-empty reasoning.
    #[test]
    fn test_reasoning_text_skips_empty_first_selection() {
        let selections = [
            ToolSelection {
                tool_name: "echo".into(),
                parameters: serde_json::json!({}),
                reasoning: String::new(),
                alternatives: vec![],
                tool_call_id: "call_1".into(),
            },
            ToolSelection {
                tool_name: "search".into(),
                parameters: serde_json::json!({}),
                reasoning: "Found the answer in the second selection".into(),
                alternatives: vec![],
                tool_call_id: "call_2".into(),
            },
            ToolSelection {
                tool_name: "fetch".into(),
                parameters: serde_json::json!({}),
                reasoning: "Third selection reasoning".into(),
                alternatives: vec![],
                tool_call_id: "call_3".into(),
            },
        ];

        let reasoning_text = selections
            .iter()
            .find_map(|sel| (!sel.reasoning.is_empty()).then_some(sel.reasoning.clone()));

        assert_eq!(
            reasoning_text.as_deref(),
            Some("Found the answer in the second selection"),
            "Should skip empty first reasoning and return the first non-empty one"
        );
    }

    #[test]
    fn test_store_fallback_in_metadata_roundtrip() {
        use crate::context::FallbackDeliverable;

        let mut ctx = JobContext::new("Test", "fallback roundtrip");
        let memory = crate::context::Memory::new(ctx.job_id);
        let fb = FallbackDeliverable::build(&ctx, &memory, "test failure");

        // Store into metadata
        store_fallback_in_metadata(&mut ctx, Some(&fb));

        // Verify it's stored and can be deserialized back
        let stored = ctx.metadata.get("fallback_deliverable");
        assert!(stored.is_some(), "fallback missing from metadata"); // safety: test

        let recovered: FallbackDeliverable =
            serde_json::from_value(stored.unwrap().clone()).expect("deserialize fallback"); // safety: test
        assert_eq!(recovered.failure_reason, "test failure"); // safety: test
        assert!(!recovered.partial); // safety: test
    }

    #[test]
    fn test_store_fallback_handles_non_object_metadata() {
        use crate::context::FallbackDeliverable;

        let mut ctx = JobContext::new("Test", "non-object metadata");
        ctx.metadata = serde_json::json!("not an object");

        let memory = crate::context::Memory::new(ctx.job_id);
        let fb = FallbackDeliverable::build(&ctx, &memory, "failed");

        store_fallback_in_metadata(&mut ctx, Some(&fb));

        // Must normalize to object and store
        assert!(ctx.metadata.is_object()); // safety: test
        assert!(ctx.metadata.get("fallback_deliverable").is_some()); // safety: test
    }

    #[test]
    fn test_store_fallback_none_is_noop() {
        let mut ctx = JobContext::new("Test", "noop");
        let original = ctx.metadata.clone();

        store_fallback_in_metadata(&mut ctx, None);

        assert_eq!(ctx.metadata, original); // safety: test
    }

    #[tokio::test]
    async fn autonomous_message_tool_ignores_stale_gateway_context_when_routine_metadata_targets_telegram()
     {
        let (worker, message_tool, gateway_captures, telegram_captures) =
            make_worker_with_message_tool().await;

        message_tool
            .set_context(
                Some("gateway".to_string()),
                Some("stale-gateway-target".to_string()),
            )
            .await;

        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.user_id = "telegram".to_string();
                ctx.metadata = serde_json::json!({
                    "notify_channel": "telegram",
                    "owner_id": "owner-scope",
                });
                Ok::<(), String>(())
            })
            .await
            .unwrap() // safety: test
            .unwrap(); // safety: test

        let result = worker
            .execute_tool(
                "message",
                &serde_json::json!({"content": "hello from routine"}),
            )
            .await
            .unwrap(); // safety: test
        assert!(
            result.contains("telegram:owner-scope"),
            "expected telegram owner-scope routing, got: {result}"
        );

        assert!(gateway_captures.lock().await.is_empty());
        let telegram = telegram_captures.lock().await.clone();
        assert_eq!(telegram.len(), 1);
        assert_eq!(telegram[0].0, "owner-scope");
        assert_eq!(telegram[0].1.content, "hello from routine");
    }

    /// Regression test: only `EmptyResponse` errors are eligible for
    /// completion-swallowing. Infrastructure errors must always propagate.
    #[test]
    fn is_completion_eligible_only_matches_empty_response() {
        use crate::error::LlmError;

        // EmptyResponse is eligible
        assert!(super::is_completion_eligible_error(
            &LlmError::EmptyResponse {
                provider: "test".to_string(),
            }
        ));

        // All other variants are NOT eligible
        assert!(!super::is_completion_eligible_error(
            &LlmError::InvalidResponse {
                provider: "test".to_string(),
                reason: "parse error".to_string(),
            }
        ));
        assert!(!super::is_completion_eligible_error(
            &LlmError::AuthFailed {
                provider: "test".to_string(),
            }
        ));
        assert!(!super::is_completion_eligible_error(
            &LlmError::ContextLengthExceeded {
                used: 100_000,
                limit: 50_000,
            }
        ));
        assert!(!super::is_completion_eligible_error(
            &LlmError::ModelNotAvailable {
                provider: "test".to_string(),
                model: "gpt-4".to_string(),
            }
        ));
        assert!(!super::is_completion_eligible_error(
            &LlmError::RequestFailed {
                provider: "test".to_string(),
                reason: "timeout".to_string(),
            }
        ));
        assert!(!super::is_completion_eligible_error(
            &LlmError::SessionExpired {
                provider: "test".to_string(),
            }
        ));
        assert!(!super::is_completion_eligible_error(
            &LlmError::SessionRenewalFailed {
                provider: "test".to_string(),
                reason: "timeout".to_string(),
            }
        ));
    }

    /// Regression test: AutonomousUnavailable errors must be recoverable.
    /// Previously the job worker treated them as fatal, killing the entire
    /// job instead of feeding the error back to the LLM.
    #[tokio::test]
    async fn test_autonomous_unavailable_is_recoverable() {
        let worker = make_worker(vec![]).await;
        let mut reason_ctx = ReasoningContext::new();
        let selection = ToolSelection {
            tool_name: "secret_list".to_string(),
            parameters: serde_json::json!({}),
            reasoning: "list secrets".to_string(),
            alternatives: vec![],
            tool_call_id: "call_123".to_string(),
        };
        let err = Error::Tool(crate::error::ToolError::AutonomousUnavailable {
            name: "secret_list".to_string(),
            reason: "not available in autonomous jobs".to_string(),
        });

        let result = worker
            .process_tool_result_job(&mut reason_ctx, &selection, Err(err))
            .await;

        assert!(
            result.is_ok(),
            "AutonomousUnavailable must be recoverable, not fatal: {:?}",
            result
        );
        // The error should be fed back to the LLM as a message.
        assert!(
            !reason_ctx.messages.is_empty(),
            "Error message should be added to reason_ctx for the LLM"
        );
    }
}
