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
use crate::tenant::SystemScope;
use crate::tools::builtin::{
    FinishJobSignal, FinishJobStatus, parse_finish_job_signal_from_output,
};
use crate::tools::execute::process_tool_result;
use crate::tools::rate_limiter::RateLimitResult;
use crate::tools::{ApprovalContext, ToolRegistry, prepare_tool_params, redact_params};
use crate::worker::autonomous_recovery::{
    AutonomousRecoveryAction, AutonomousRecoveryState, EMPTY_TOOL_COMPLETION_FAILURE,
    EMPTY_TOOL_COMPLETION_NUDGE, EMPTY_TOOL_COMPLETION_STRICT,
};
use ironclaw_common::{AppEvent, JobResultStatus};
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
    pub store: Option<SystemScope>,
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
    /// Whether the deployment is multi-tenant (used for admin tool policy filtering).
    pub multi_tenant: bool,
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

fn finish_job_signal_from_result(
    result: &Result<String, Error>,
) -> Result<Option<FinishJobSignal>, Error> {
    match result {
        Ok(output) => parse_finish_job_signal_from_output(output)
            .map(Some)
            .map_err(|e| {
                crate::error::Error::from(crate::error::ToolError::ExecutionFailed {
                    name: "finish_job".to_string(),
                    reason: format!("finish_job executed but result could not be interpreted: {e}"),
                })
            }),
        Err(_) => Ok(None),
    }
}

impl Worker {
    const DEFAULT_COMPLETION_MESSAGE: &str = "Job completed successfully";

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

    fn store(&self) -> Option<&SystemScope> {
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
                "result" => {
                    // JSON payloads from sandbox containers are a trust
                    // boundary — parse the wire string into the typed
                    // enum and fall back to `Failed` (not `Completed`)
                    // on unknown values so a mis-labeled container run
                    // cannot silently register as success.
                    let raw_status = data.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    let status = raw_status.parse::<JobResultStatus>().unwrap_or_else(|_| {
                        tracing::warn!(
                            job_id = %job_id,
                            raw_status = %raw_status,
                            "unknown job result status from container; defaulting to Failed"
                        );
                        JobResultStatus::Failed
                    });
                    Some(AppEvent::JobResult {
                        job_id: job_id_str,
                        status,
                        session_id: data
                            .get("session_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        fallback_deliverable: data.get("fallback_deliverable").cloned(),
                    })
                }
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
The only way to end this job is to call the `finish_job` tool.
Call `finish_job` only after all other work is done.
Use status \"completed\" when all required work is finished,
or status \"failed\" when you hit an unresolvable blocker."#,
            job_ctx.title, job_ctx.description
        )));

        // Main execution loop with timeout
        let result = tokio::time::timeout(self.timeout(), async {
            self.execution_loop(&mut rx, &reasoning, &mut reason_ctx)
                .await
        })
        .await;

        match result {
            Ok(Ok(LoopOutcome::Response(summary))) => {
                tracing::info!(
                    job_id = %self.job_id,
                    summary = %summary,
                    "Worker completed successfully"
                );
            }
            Ok(Ok(LoopOutcome::Failure(reason))) => {
                tracing::error!("Worker for job {} failed: {}", self.job_id, reason);
                self.mark_failed(&reason).await?;
            }
            Ok(Ok(LoopOutcome::MaxIterations)) => {
                self.mark_failed("Maximum iterations exceeded: job hit the iteration cap")
                    .await?;
            }
            Ok(Ok(LoopOutcome::Stopped)) => {
                tracing::info!("Worker for job {} stopped", self.job_id);
            }
            Ok(Ok(LoopOutcome::NeedApproval(_))) => {
                self.mark_failed(
                    "Autonomous job execution paused for approval, which is unsupported in worker jobs",
                )
                .await?;
            }
            Ok(Ok(LoopOutcome::AuthPending(_))) => {
                self.mark_failed(
                    "Autonomous job execution paused for authentication, which is unsupported in worker jobs",
                )
                .await?;
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
    ) -> Result<LoopOutcome, Error> {
        let max_iterations = self
            .context_manager()
            .get_context(self.job_id)
            .await
            .ok()
            .and_then(|ctx| ctx.metadata.get("max_iterations").and_then(|v| v.as_u64()))
            .unwrap_or(50) as usize;
        let max_iterations = max_iterations.min(ironclaw_common::MAX_WORKER_ITERATIONS as usize);

        // Planning should only see work tools. Job-control tools like
        // `finish_job` are reserved for the agentic loop / execution layer.
        reason_ctx.available_tools = self
            .tools()
            .tool_definitions_for_job()
            .await
            .into_iter()
            .filter(|def| def.name != "finish_job")
            .collect();

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
            if let Some(outcome) = self.execute_plan(rx, reason_ctx, plan).await? {
                return Ok(outcome);
            }

            if let Ok(ctx) = self.context_manager().get_context(self.job_id).await
                && (ctx.state.is_terminal()
                    || ctx.state == JobState::Stuck
                    || ctx.state == JobState::Completed)
            {
                return Ok(Self::loop_outcome_from_context(&ctx));
            }
        }

        // Build the delegate and run the shared agentic loop
        let delegate = JobDelegate {
            worker: self,
            rx: tokio::sync::Mutex::new(rx),
            consecutive_rate_limits: std::sync::atomic::AtomicUsize::new(0),
            recovery_state: tokio::sync::Mutex::new(AutonomousRecoveryState::default()),
            cached_user_info: tokio::sync::OnceCell::new(),
            cached_admin_tool_policy: tokio::sync::OnceCell::new(),
        };

        let config = AgenticLoopConfig {
            max_iterations,
            enable_tool_intent_nudge: true,
            max_tool_intent_nudges: 2,
        };

        run_agentic_loop(&delegate, reasoning, reason_ctx, &config).await
    }

    fn loop_outcome_from_context(ctx: &crate::context::JobContext) -> LoopOutcome {
        let summary = ctx
            .transitions
            .last()
            .and_then(|transition| transition.reason.clone())
            .unwrap_or_else(|| match ctx.state {
                JobState::Completed => Self::DEFAULT_COMPLETION_MESSAGE.to_string(),
                JobState::Failed => format!("Job {} failed", ctx.job_id),
                JobState::Stuck => format!("Job {} became stuck", ctx.job_id),
                JobState::Submitted | JobState::Accepted => {
                    format!("Job {} finished ({})", ctx.job_id, ctx.state)
                }
                JobState::Cancelled => "Job was cancelled".to_string(),
                JobState::Pending | JobState::InProgress => {
                    format!("Job {} is still active", ctx.job_id)
                }
            });

        match ctx.state {
            JobState::Completed | JobState::Submitted | JobState::Accepted => {
                LoopOutcome::Response(summary)
            }
            JobState::Failed | JobState::Stuck => LoopOutcome::Failure(summary),
            JobState::Cancelled => LoopOutcome::Stopped,
            JobState::Pending | JobState::InProgress => LoopOutcome::Failure(summary),
        }
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
        let tool = deps.tools.get_for_job(tool_name).await.ok_or_else(|| {
            crate::error::ToolError::NotFound {
                name: tool_name.to_string(),
            }
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
        reason_ctx: &mut ReasoningContext,
        plan: &ActionPlan,
    ) -> Result<Option<LoopOutcome>, Error> {
        for (i, action) in plan.actions.iter().enumerate() {
            // Check for stop signal and injected user messages
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMessage::Stop => {
                        tracing::debug!(
                            "Worker for job {} received stop signal during plan execution",
                            self.job_id
                        );
                        return Ok(Some(LoopOutcome::Stopped));
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
                        return Ok(None);
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
            let finish_signal = if selection.tool_name == "finish_job" {
                finish_job_signal_from_result(&result)?
            } else {
                None
            };
            let finish_outcome = finish_signal.clone().map(|signal| match signal.status {
                FinishJobStatus::Completed => LoopOutcome::Response(signal.summary),
                FinishJobStatus::Failed => LoopOutcome::Failure(signal.summary),
            });

            self.process_tool_result_job(reason_ctx, &selection, result)
                .await?;

            if let Some(outcome) = finish_outcome {
                if let Some(signal) = finish_signal.as_ref()
                    && signal.status == FinishJobStatus::Completed
                {
                    self.mark_completed(&signal.summary).await.map_err(|e| {
                        crate::error::Error::from(crate::error::JobError::ContextError {
                            id: self.job_id,
                            reason: e.to_string(),
                        })
                    })?;
                }
                return Ok(Some(outcome));
            }

            if selection.tool_name == "finish_job" {
                return Ok(None);
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Plan completed — prompt the LLM to continue. The normal agentic loop
        // will pick up after this method returns; the LLM can call finish_job
        // when the job is actually done, or call more tools if work remains.
        reason_ctx.messages.push(ChatMessage::user(
            "The planned actions have been executed. Continue working toward the goal. \
             The only way to end the job is to call finish_job with valid arguments.",
        ));
        tracing::info!(
            "Job {} plan completed, continuing to agentic loop",
            self.job_id
        );
        self.log_event(
            "status",
            serde_json::json!({
                "message": "Plan completed, continuing execution...",
            }),
        );

        Ok(None)
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<String, Error> {
        Self::execute_tool_inner(&self.deps, self.job_id, tool_name, params).await
    }

    async fn mark_completed(&self, summary: &str) -> Result<(), Error> {
        let message = if summary.trim().is_empty() {
            Self::DEFAULT_COMPLETION_MESSAGE
        } else {
            summary
        };

        self.context_manager()
            .update_context(self.job_id, |ctx| {
                ctx.transition_to(JobState::Completed, Some(message.to_string()))
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
                "message": truncate_for_preview(message, 2000),
            }),
        );
        self.persist_status(JobState::Completed, None);
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

        // Emit via the typed enum so the wire string stays in sync with
        // `JobResultStatus::Stuck::as_str()` — no string-literal drift.
        self.log_event(
            "result",
            serde_json::json!({
                "status": JobResultStatus::Stuck,
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
///
/// Handles: signal channel (stop/ping/user messages), cancellation checks,
/// rate-limit retry, parallel tool execution, DB persistence, SSE broadcasting.
struct JobDelegate<'a> {
    worker: &'a Worker,
    rx: tokio::sync::Mutex<&'a mut mpsc::Receiver<WorkerMessage>>,
    /// Tracks consecutive rate-limit errors to fail fast instead of burning iterations.
    consecutive_rate_limits: std::sync::atomic::AtomicUsize,
    recovery_state: tokio::sync::Mutex<AutonomousRecoveryState>,
    /// Cached (user_id, is_admin) for admin tool policy filtering. Populated once
    /// on first access to avoid repeated DB lookups.
    cached_user_info: tokio::sync::OnceCell<(String, bool)>,
    /// Cached admin tool policy result for this worker loop.
    cached_admin_tool_policy: crate::tools::permissions::AdminToolPolicyCache,
}

impl<'a> JobDelegate<'a> {
    const MAX_CONSECUTIVE_RATE_LIMITS: usize = 10;

    /// Resolve and cache (user_id, is_admin) for admin tool policy filtering.
    ///
    /// Reads the job's `user_id` from the context manager and looks up the
    /// user's role from the database. Falls back to `false` when the DB
    /// lookup fails or no store is configured (safe default: non-admin users
    /// still see the filtered tool list).
    async fn resolve_user_info(&self) -> &(String, bool) {
        self.cached_user_info
            .get_or_init(|| async {
                let user_id = self
                    .worker
                    .context_manager()
                    .get_context(self.worker.job_id)
                    .await
                    .map(|ctx| ctx.user_id.clone())
                    .unwrap_or_default();

                let is_admin = if let Some(store) = self.worker.store() {
                    match store.get_user_role(&user_id).await {
                        Ok(Some(role)) => role.is_admin(),
                        Ok(None) => false,
                        Err(e) => {
                            tracing::debug!(
                                job_id = %self.worker.job_id,
                                "Failed to look up user role, defaulting to non-admin: {e}"
                            );
                            false
                        }
                    }
                } else {
                    false
                };

                (user_id, is_admin)
            })
            .await
    }

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
        let recovery_mode_active = {
            let mut recovery = self.recovery_state.lock().await;
            recovery.begin_iteration()
        };

        if recovery_mode_active {
            tracing::warn!(
                job_id = %self.worker.job_id,
                "Retrying after malformed tool completions with tools still enabled"
            );
        }

        // Refresh tool definitions so newly built tools become visible.
        let tool_defs = self.worker.tools().tool_definitions_for_job().await;

        // Apply admin tool policy filtering (multi-tenant only).
        let (user_id, is_admin) = self.resolve_user_info().await;
        let admin_policy = self
            .cached_admin_tool_policy
            .get_or_init(|| async {
                let Some(store) = self.worker.store() else {
                    return crate::tools::permissions::AdminToolPolicyState::Missing;
                };

                match store.get_admin_tool_policy().await {
                    Ok(Some(policy)) => {
                        crate::tools::permissions::AdminToolPolicyState::Loaded(policy)
                    }
                    Ok(None) => crate::tools::permissions::AdminToolPolicyState::Missing,
                    Err(error) => {
                        tracing::warn!(
                            job_id = %self.worker.job_id,
                            %error,
                            "Failed to load admin tool policy for worker, failing closed"
                        );
                        crate::tools::permissions::AdminToolPolicyState::FailClosed
                    }
                }
            })
            .await;
        let tool_defs = crate::tools::permissions::filter_admin_disabled_tools(
            tool_defs,
            self.worker.deps.multi_tenant,
            *is_admin,
            user_id,
            admin_policy,
        );

        reason_ctx.available_tools = tool_defs;

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
            Err(e) => return Err(e.into()),
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
            Err(e) => Err(e.into()),
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
            AutonomousRecoveryAction::StrictToolRecovery => {
                tracing::warn!(
                    job_id = %self.worker.job_id,
                    "Autonomous recovery escalated; requiring a valid tool call on the next reply"
                );
                self.worker.log_event(
                    "status",
                    serde_json::json!({
                        "message": "Model failed to recover after a tool-use nudge; requiring a valid tool call or finish_job next.",
                    }),
                );
                reason_ctx
                    .messages
                    .push(ChatMessage::user(EMPTY_TOOL_COMPLETION_STRICT));
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

        // Empty text: do not auto-complete. Completion is only via finish_job.
        // Rate-limit backoff frames (no prior output) and genuine empties are both
        // handled by returning Continue; the loop's max_iterations cap and nudge
        // mechanism prevent infinite spinning.
        if text.is_empty() {
            return TextAction::Continue;
        }

        // Jobs run autonomously — strip <suggestions> tags that are only
        // meaningful for interactive chat sessions.
        let text = crate::agent::strip_suggestions(text);

        // Add assistant response to context
        reason_ctx.messages.push(ChatMessage::assistant(&text));

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

        // Convert non-finish_job calls to ToolSelections and execute.
        let selections: Vec<ToolSelection> = other_calls
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
        let mut tool_failure_count: usize = 0;
        let total_tools = selections.len() + finish_job_calls.len();

        if selections.len() == 1 {
            let selection = &selections[0];
            let result = self
                .worker
                .execute_tool(&selection.tool_name, &selection.parameters)
                .await;
            if result.is_err() {
                tool_failure_count += 1;
            }
            self.worker
                .process_tool_result_job(reason_ctx, selection, result)
                .await?;
        } else if !selections.is_empty() {
            let results = self.worker.execute_tools_parallel(&selections).await;
            for (selection, result) in selections.iter().zip(results) {
                if result.result.is_err() {
                    tool_failure_count += 1;
                }
                self.worker
                    .process_tool_result_job(reason_ctx, selection, result.result)
                    .await?;
            }
        }

        reason_ctx.last_tool_batch_all_failed =
            total_tools > 0 && tool_failure_count == total_tools;

        // Now handle finish_job — always runs last so other tools complete first.
        for (idx, tc) in finish_job_calls.iter().enumerate() {
            let is_last_finish_job = idx + 1 == finish_job_calls.len();
            let selection = ToolSelection {
                tool_name: tc.name.clone(),
                parameters: tc.arguments.clone(),
                reasoning: tc.reasoning.clone().unwrap_or_default(),
                alternatives: vec![],
                tool_call_id: tc.id.clone(),
            };
            let result = self
                .worker
                .execute_tool(&selection.tool_name, &selection.parameters)
                .await;
            let finish_signal = if is_last_finish_job {
                finish_job_signal_from_result(&result)?
            } else {
                None
            };
            if result.is_err() {
                tool_failure_count += 1;
                self.worker
                    .process_tool_result_job(reason_ctx, &selection, result)
                    .await?;
                reason_ctx.last_tool_batch_all_failed =
                    total_tools > 0 && tool_failure_count == total_tools;
                if is_last_finish_job {
                    return Ok(None);
                }
                continue;
            }
            self.worker
                .process_tool_result_job(reason_ctx, &selection, result)
                .await?;

            if is_last_finish_job {
                let signal = finish_signal.ok_or_else(|| {
                    crate::error::Error::from(crate::error::ToolError::ExecutionFailed {
                        name: "finish_job".to_string(),
                        reason: "finish_job executed successfully but returned no signal"
                            .to_string(),
                    })
                })?;

                if signal.status == FinishJobStatus::Completed {
                    self.worker
                        .mark_completed(&signal.summary)
                        .await
                        .map_err(|e| {
                            crate::error::Error::from(crate::error::JobError::ContextError {
                                id: self.worker.job_id,
                                reason: e.to_string(),
                            })
                        })?;
                    return Ok(Some(LoopOutcome::Response(signal.summary)));
                }
                return Ok(Some(LoopOutcome::Failure(signal.summary)));
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
    use std::sync::{Arc, Mutex as StdMutex};

    use crate::channels::ChannelManager;
    #[cfg(feature = "libsql")]
    use crate::db::Database;
    use crate::llm::ToolSelection;

    use super::*;
    use crate::config::SafetyConfig;
    use crate::context::JobContext;
    use crate::llm::{
        CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
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

    struct PlanningFinishJobLlm {
        planning_prompt: StdMutex<Option<String>>,
    }

    impl PlanningFinishJobLlm {
        fn new() -> Self {
            Self {
                planning_prompt: StdMutex::new(None),
            }
        }

        fn planning_prompt(&self) -> Option<String> {
            self.planning_prompt.lock().unwrap().clone() // safety: test-only mutex access
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for PlanningFinishJobLlm {
        fn model_name(&self) -> &str {
            "planning-finish-job"
        }

        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
        }

        async fn complete(
            &self,
            req: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            let system_prompt = req
                .messages
                .iter()
                .find(|message| message.role == crate::llm::Role::System)
                .map(|message| message.content.clone())
                .unwrap_or_default();
            *self.planning_prompt.lock().unwrap() = Some(system_prompt); // safety: test-only mutex access

            Ok(CompletionResponse {
                content: serde_json::json!({
                    "goal": "Finish the job cleanly",
                    "actions": [
                        {
                            "tool_name": "finish_job",
                            "parameters": {
                                "status": "completed",
                                "summary": "Planned completion"
                            },
                            "reasoning": "The job is already done",
                            "expected_outcome": "The worker terminates"
                        }
                    ],
                    "estimated_cost": 0.0,
                    "estimated_time_secs": 0,
                    "confidence": 1.0
                })
                .to_string(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            panic!("agentic loop should not continue after planned finish_job");
        }
    }

    /// Build a Worker wired to a ToolRegistry containing the given tools.
    async fn make_worker(tools: Vec<Arc<dyn Tool>>) -> Worker {
        make_worker_with_llm(tools, Arc::new(StubLlm), false).await
    }

    async fn make_worker_with_llm(
        tools: Vec<Arc<dyn Tool>>,
        llm: Arc<dyn LlmProvider>,
        use_planning: bool,
    ) -> Worker {
        let registry = ToolRegistry::new();
        registry.register_builtin_tools();
        for t in tools {
            registry.register(t).await;
        }

        let cm = Arc::new(crate::context::ContextManager::new(5));
        let job_id = cm.create_job("test", "test job").await.unwrap(); // safety: test

        let deps = WorkerDeps {
            context_manager: cm,
            llm,
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            })),
            tools: Arc::new(registry),
            store: None,
            hooks: Arc::new(crate::hooks::HookRegistry::new()),
            timeout: Duration::from_secs(30),
            use_planning,
            sse_tx: None,
            approval_context: None,
            http_interceptor: None,
            multi_tenant: false,
        };

        Worker::new(job_id, deps)
    }

    #[cfg(feature = "libsql")]
    async fn make_worker_with_store(tools: Vec<Arc<dyn Tool>>, store: Arc<dyn Database>) -> Worker {
        let registry = ToolRegistry::new();
        registry.register_builtin_tools();
        for t in tools {
            registry.register(t).await;
        }

        let cm = Arc::new(crate::context::ContextManager::new(5));
        let job_id = cm
            .create_job_for_user("worker-test-user", "test", "test job")
            .await
            .unwrap(); // safety: test
        let ctx = cm.get_context(job_id).await.unwrap(); // safety: test

        let system_store = SystemScope::new(store);
        system_store.save_job(&ctx).await.unwrap(); // safety: test

        let deps = WorkerDeps {
            context_manager: cm,
            llm: Arc::new(StubLlm),
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            })),
            tools: Arc::new(registry),
            store: Some(system_store),
            hooks: Arc::new(crate::hooks::HookRegistry::new()),
            timeout: Duration::from_secs(30),
            use_planning: false,
            sse_tx: None,
            approval_context: None,
            http_interceptor: None,
            multi_tenant: false,
        };

        Worker::new(job_id, deps)
    }

    async fn transition_worker_to_in_progress(worker: &Worker) {
        worker
            .context_manager()
            .update_context(worker.job_id, |ctx| {
                ctx.transition_to(JobState::InProgress, None)
            })
            .await
            .unwrap() // safety: test
            .unwrap(); // safety: test
    }

    fn make_job_delegate<'a>(
        worker: &'a Worker,
        rx: &'a mut tokio::sync::mpsc::Receiver<WorkerMessage>,
    ) -> JobDelegate<'a> {
        JobDelegate {
            worker,
            rx: tokio::sync::Mutex::new(rx),
            consecutive_rate_limits: std::sync::atomic::AtomicUsize::new(0),
            recovery_state: tokio::sync::Mutex::new(AutonomousRecoveryState::default()),
            cached_user_info: tokio::sync::OnceCell::new(),
            cached_admin_tool_policy: tokio::sync::OnceCell::new(),
        }
    }

    fn finish_job_call(id: &str, status: &str, summary: Option<&str>) -> crate::llm::ToolCall {
        let arguments = match summary {
            Some(summary) => serde_json::json!({
                "status": status,
                "summary": summary,
            }),
            None => serde_json::json!({
                "status": status,
            }),
        };

        crate::llm::ToolCall {
            id: id.to_string(),
            name: "finish_job".to_string(),
            arguments,
            reasoning: None,
        }
    }

    async fn execute_tool_batch(
        worker: &Worker,
        tool_calls: Vec<crate::llm::ToolCall>,
    ) -> Result<(Option<LoopOutcome>, ReasoningContext), crate::error::Error> {
        let (_, mut rx) = tokio::sync::mpsc::channel(1);
        let delegate = make_job_delegate(worker, &mut rx);
        let mut reason_ctx = ReasoningContext::new();
        let outcome = delegate
            .execute_tool_calls(tool_calls, None, &mut reason_ctx)
            .await?;
        Ok((outcome, reason_ctx))
    }

    fn assert_response_outcome(outcome: Option<LoopOutcome>, expected: &str) {
        assert!(
            matches!(outcome, Some(LoopOutcome::Response(ref s)) if s == expected),
            "expected Response({expected:?})"
        ); // safety: test
    }

    fn assert_failure_outcome(outcome: Option<LoopOutcome>, expected: &str) {
        assert!(
            matches!(outcome, Some(LoopOutcome::Failure(ref s)) if s == expected),
            "expected Failure({expected:?})"
        ); // safety: test
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

        transition_worker_to_in_progress(&worker).await;

        worker
            .mark_completed("Completed in first pass")
            .await
            .unwrap(); // safety: test

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::Completed); // safety: test

        // Second mark_completed should succeed (idempotent) rather than
        // erroring, matching the fix for the execution_loop / worker wrapper
        // race condition.
        let result = worker.mark_completed("Completed in first pass").await;
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

    // --- Completion persistence ---------------------------------------------

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_finish_job_persists_completion_summary_as_result_event() {
        let (db, _tmp) = crate::testing::test_db().await;
        let worker = make_worker_with_store(vec![], Arc::clone(&db)).await;
        transition_worker_to_in_progress(&worker).await;

        let summary = "Indexed the repo and wrote the final report";
        let outcome = execute_tool_batch(
            &worker,
            vec![finish_job_call("call_finish", "completed", Some(summary))],
        )
        .await
        .map(|(outcome, _)| outcome)
        .unwrap();

        assert_response_outcome(outcome, summary);

        let system_store = SystemScope::new(db);
        let persisted_message = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(message) = system_store
                    .get_agent_job_result_message(worker.job_id)
                    .await
                    .unwrap()
                {
                    break message;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("result event should be persisted"); // safety: test

        assert_eq!(persisted_message, summary); // safety: test
        assert!(
            system_store
                .get_agent_job_failure_reason(worker.job_id)
                .await
                .unwrap()
                .is_none(),
            "successful jobs must not populate failure_reason"
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
            multi_tenant: false,
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

        transition_worker_to_in_progress(&worker).await;

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

        transition_worker_to_in_progress(&worker).await;

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

    // --- Completion contract -------------------------------------------------

    /// Text responses no longer auto-complete the job. Only finish_job terminates.
    #[tokio::test]
    async fn test_text_response_does_not_terminate_loop() {
        let worker = make_worker(vec![]).await;
        transition_worker_to_in_progress(&worker).await;

        let (_, mut rx) = tokio::sync::mpsc::channel(1);
        let delegate = make_job_delegate(&worker, &mut rx);
        let mut reason_ctx = ReasoningContext::new();

        let action = delegate
            .handle_text_response(
                "Weekly review created in Notion and notification sent.",
                ResponseMetadata::default(),
                &mut reason_ctx,
            )
            .await;

        assert!(
            matches!(action, TextAction::Continue),
            "Text response should NOT terminate the loop, got Return"
        ); // safety: test

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::InProgress); // safety: test
    }

    /// finish_job tool call terminates the loop and marks the job complete.
    #[tokio::test]
    async fn test_finish_job_terminates_loop() {
        let worker = make_worker(vec![]).await;
        transition_worker_to_in_progress(&worker).await;

        let outcome = execute_tool_batch(
            &worker,
            vec![finish_job_call(
                "call_1",
                "completed",
                Some("All tasks done"),
            )],
        )
        .await
        .map(|(outcome, _)| outcome)
        .unwrap();

        assert_response_outcome(outcome, "All tasks done");

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::Completed); // safety: test
        assert_eq!(
            ctx.transitions
                .last()
                .and_then(|transition| transition.reason.as_deref()),
            Some("All tasks done")
        ); // safety: test
    }

    /// Blank completion summaries are normalized to a stable default message.
    #[tokio::test]
    async fn test_finish_job_blank_completed_summary_uses_default_message() {
        let worker = make_worker(vec![]).await;
        transition_worker_to_in_progress(&worker).await;

        let outcome = execute_tool_batch(
            &worker,
            vec![finish_job_call(
                "call_blank_complete",
                "completed",
                Some(" \n\t "),
            )],
        )
        .await
        .map(|(outcome, _)| outcome)
        .unwrap();

        assert_response_outcome(outcome, "Job completed successfully with no summary.");

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::Completed); // safety: test
        assert_eq!(
            ctx.transitions
                .last()
                .and_then(|transition| transition.reason.as_deref()),
            Some("Job completed successfully with no summary.")
        ); // safety: test
    }

    /// Empty text responses no longer auto-complete the job; they return Continue.
    #[tokio::test]
    async fn test_empty_text_continues_not_terminates() {
        let worker = make_worker(vec![]).await;
        transition_worker_to_in_progress(&worker).await;

        let (_, mut rx) = tokio::sync::mpsc::channel(1);
        let delegate = make_job_delegate(&worker, &mut rx);
        let mut reason_ctx = ReasoningContext::new();

        let action = delegate
            .handle_text_response("", ResponseMetadata::default(), &mut reason_ctx)
            .await;

        assert!(
            matches!(action, TextAction::Continue),
            "Empty text must return Continue, not terminate the loop"
        ); // safety: test

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::InProgress); // safety: test
    }

    #[tokio::test]
    async fn test_planning_excludes_finish_job_but_executes_it_if_returned_anyway() {
        let llm = Arc::new(PlanningFinishJobLlm::new());
        let worker = make_worker_with_llm(vec![], llm.clone(), true).await;
        transition_worker_to_in_progress(&worker).await;

        let reasoning =
            Reasoning::new(worker.llm().clone()).with_model_name(worker.llm().active_model_name());
        let mut reason_ctx = ReasoningContext::new().with_job("test job");
        let (_, mut rx) = tokio::sync::mpsc::channel(1);

        worker
            .execution_loop(&mut rx, &reasoning, &mut reason_ctx)
            .await
            .unwrap(); // safety: test

        let planning_prompt = llm.planning_prompt().unwrap_or_default();
        assert!(
            !planning_prompt.contains("finish_job"),
            "planning prompt must not advertise finish_job: {planning_prompt}"
        ); // safety: test

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::Completed); // safety: test
    }

    // --- LLM bookkeeping regressions ----------------------------------------

    // --- finish_job batch semantics -----------------------------------------

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

    /// finish_job in a mixed batch executes other tools first, then terminates the loop.
    #[tokio::test]
    async fn test_finish_job_in_mixed_batch_executes_others_first() {
        use crate::tools::{Tool, ToolError as ToolExecError, ToolOutput};
        use std::sync::atomic::{AtomicBool, Ordering};

        /// A tool that records whether it was called.
        struct RecordTool {
            called: Arc<AtomicBool>,
        }

        #[async_trait::async_trait]
        impl Tool for RecordTool {
            fn name(&self) -> &str {
                "record_tool"
            }
            fn description(&self) -> &str {
                "Records execution"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object", "properties": {}})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &JobContext,
            ) -> Result<ToolOutput, ToolExecError> {
                self.called.store(true, Ordering::Relaxed);
                Ok(ToolOutput::text(
                    "recorded",
                    std::time::Duration::from_millis(1),
                ))
            }
            fn requires_sanitization(&self) -> bool {
                false
            }
        }

        let called = Arc::new(AtomicBool::new(false));
        let worker = make_worker(vec![Arc::new(RecordTool {
            called: called.clone(),
        }) as Arc<dyn Tool>])
        .await;
        transition_worker_to_in_progress(&worker).await;

        let outcome = execute_tool_batch(
            &worker,
            vec![
                crate::llm::ToolCall {
                    id: "call_other".to_string(),
                    name: "record_tool".to_string(),
                    arguments: serde_json::json!({}),
                    reasoning: None,
                },
                finish_job_call("call_fj", "completed", Some("Mixed batch done")),
            ],
        )
        .await
        .map(|(outcome, _)| outcome)
        .unwrap();

        // record_tool must have executed
        assert!(called.load(Ordering::Relaxed), "record_tool was not called"); // safety: test

        assert_response_outcome(outcome, "Mixed batch done");

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::Completed); // safety: test
    }

    /// If the model emits multiple finish_job calls, only the last call decides
    /// the final state and summary, but every call still gets a tool result.
    #[tokio::test]
    async fn test_last_finish_job_call_wins() {
        let worker = make_worker(vec![]).await;
        transition_worker_to_in_progress(&worker).await;

        let (outcome, reason_ctx) = execute_tool_batch(
            &worker,
            vec![
                finish_job_call("call_finish_1", "failed", Some("stale failure")),
                finish_job_call("call_finish_2", "completed", Some("latest completion")),
            ],
        )
        .await
        .unwrap();

        assert!(
            matches!(outcome, Some(LoopOutcome::Response(ref s)) if s == "latest completion"),
            "the last finish_job call must decide the final outcome"
        ); // safety: test

        let tool_result_count = reason_ctx
            .messages
            .iter()
            .filter(|msg| msg.role == crate::llm::Role::Tool)
            .count();
        assert_eq!(
            tool_result_count, 2,
            "every finish_job call should still emit a tool result"
        ); // safety: test

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::Completed); // safety: test
    }

    /// finish_job with status="failed" returns LoopOutcome::Failure (DB update by outer match).
    #[tokio::test]
    async fn test_finish_job_failed_status_returns_failure() {
        let worker = make_worker(vec![]).await;
        transition_worker_to_in_progress(&worker).await;

        let outcome = execute_tool_batch(
            &worker,
            vec![finish_job_call(
                "call_fail",
                "failed",
                Some("Could not reach the API"),
            )],
        )
        .await
        .map(|(outcome, _)| outcome)
        .unwrap();

        assert_failure_outcome(outcome, "Could not reach the API");
    }

    /// Blank failure summaries are normalized to a stable default message.
    #[tokio::test]
    async fn test_finish_job_blank_failed_summary_uses_default_message() {
        let worker = make_worker(vec![]).await;
        transition_worker_to_in_progress(&worker).await;

        let outcome = execute_tool_batch(
            &worker,
            vec![finish_job_call("call_blank_failed", "failed", Some("   "))],
        )
        .await
        .map(|(outcome, _)| outcome)
        .unwrap();

        assert_failure_outcome(outcome, "Job failed with no summary.");
    }

    /// Malformed finish_job parameters should behave like a normal tool error and let the loop continue.
    #[tokio::test]
    async fn test_finish_job_malformed_params_return_tool_error() {
        let worker = make_worker(vec![]).await;
        transition_worker_to_in_progress(&worker).await;

        let (outcome, reason_ctx) = execute_tool_batch(
            &worker,
            vec![finish_job_call("call_bad_finish", "completed", None)],
        )
        .await
        .unwrap();

        assert!(
            outcome.is_none(),
            "malformed finish_job should return a recoverable tool error"
        ); // safety: test
        assert!(
            reason_ctx.last_tool_batch_all_failed,
            "malformed finish_job should count as a failed tool batch"
        ); // safety: test

        let tool_result_message = reason_ctx
            .messages
            .iter()
            .find(|msg| msg.role == crate::llm::Role::Tool)
            .map(|msg| msg.content.as_str())
            .unwrap_or("");
        assert!(
            tool_result_message.contains("summary"),
            "tool result should explain the malformed finish_job payload: {tool_result_message}"
        ); // safety: test

        let ctx = worker
            .context_manager()
            .get_context(worker.job_id)
            .await
            .unwrap(); // safety: test
        assert_eq!(ctx.state, JobState::InProgress); // safety: test
    }
}
