//! Job scheduler for parallel execution.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::agent::task::{Task, TaskContext, TaskOutput};
use crate::channels::web::types::SseEvent;
use crate::config::AgentConfig;
use crate::context::{ContextManager, JobContext, JobState};
use crate::db::Database;
use crate::error::{Error, JobError};
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::safety::SafetyLayer;
use crate::tools::{ApprovalContext, ToolRegistry};
use crate::worker::job::{Worker, WorkerDeps};

/// Message to send to a worker.
#[derive(Debug)]
pub enum WorkerMessage {
    /// Start working on the job.
    Start,
    /// Stop the job.
    Stop,
    /// Check health.
    Ping,
    /// Inject a follow-up user message into the worker's reasoning context.
    UserMessage(String),
}

/// Status of a scheduled job.
#[derive(Debug)]
pub struct ScheduledJob {
    pub handle: JoinHandle<()>,
    pub tx: mpsc::Sender<WorkerMessage>,
}

/// Status of a scheduled sub-task.
struct ScheduledSubtask {
    handle: JoinHandle<Result<TaskOutput, Error>>,
}

/// Schedules and manages parallel job execution.
pub struct Scheduler {
    config: AgentConfig,
    context_manager: Arc<ContextManager>,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    store: Option<Arc<dyn Database>>,
    hooks: Arc<HookRegistry>,
    /// SSE broadcast sender for live job event streaming.
    sse_tx: Option<tokio::sync::broadcast::Sender<SseEvent>>,
    /// HTTP interceptor for trace recording/replay (propagated to workers).
    http_interceptor: Option<Arc<dyn crate::llm::recording::HttpInterceptor>>,
    /// Running jobs (main LLM-driven jobs).
    jobs: Arc<RwLock<HashMap<Uuid, ScheduledJob>>>,
    /// Running sub-tasks (tool executions, background tasks).
    subtasks: Arc<RwLock<HashMap<Uuid, ScheduledSubtask>>>,
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new(
        config: AgentConfig,
        context_manager: Arc<ContextManager>,
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
        store: Option<Arc<dyn Database>>,
        hooks: Arc<HookRegistry>,
    ) -> Self {
        Self {
            config,
            context_manager,
            llm,
            safety,
            tools,
            store,
            hooks,
            sse_tx: None,
            http_interceptor: None,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            subtasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set the SSE broadcast sender for live job event streaming.
    pub fn set_sse_sender(&mut self, tx: tokio::sync::broadcast::Sender<SseEvent>) {
        self.sse_tx = Some(tx);
    }

    /// Set the HTTP interceptor for trace recording/replay.
    pub fn set_http_interceptor(
        &mut self,
        interceptor: Arc<dyn crate::llm::recording::HttpInterceptor>,
    ) {
        self.http_interceptor = Some(interceptor);
    }

    /// Create, persist, and schedule a job in one shot.
    ///
    /// This is the preferred entry point for dispatching new jobs. It:
    /// 1. Creates the job context via `ContextManager`
    /// 2. Optionally applies metadata (e.g. `max_iterations`)
    /// 3. Persists the job to the database (so FK references from
    ///    `job_actions` / `llm_calls` work immediately)
    /// 4. Schedules the job for worker execution
    ///
    /// Returns the new job ID.
    pub async fn dispatch_job(
        &self,
        user_id: &str,
        title: &str,
        description: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<Uuid, JobError> {
        self.dispatch_job_inner(user_id, title, description, metadata, None)
            .await
    }

    /// Dispatch a job with an explicit approval context for autonomous execution.
    ///
    /// Same as `dispatch_job`, but the worker will use the given `ApprovalContext`
    /// to determine which tools are pre-approved (instead of blocking all non-`Never` tools).
    pub async fn dispatch_job_with_context(
        &self,
        user_id: &str,
        title: &str,
        description: &str,
        metadata: Option<serde_json::Value>,
        approval_context: ApprovalContext,
    ) -> Result<Uuid, JobError> {
        self.dispatch_job_inner(
            user_id,
            title,
            description,
            metadata,
            Some(approval_context),
        )
        .await
    }

    /// Shared implementation for `dispatch_job` and `dispatch_job_with_context`.
    async fn dispatch_job_inner(
        &self,
        user_id: &str,
        title: &str,
        description: &str,
        metadata: Option<serde_json::Value>,
        approval_context: Option<ApprovalContext>,
    ) -> Result<Uuid, JobError> {
        let job_id = self
            .context_manager
            .create_job_for_user(user_id, title, description)
            .await?;

        // Apply metadata and token budget in a single atomic update.
        // This prevents concurrent workers from observing partial state.
        // Cap user-supplied max_tokens at the configured limit (Issue #815).
        let user_max_tokens = metadata
            .as_ref()
            .and_then(|m| m.get("max_tokens"))
            .and_then(|v| v.as_u64());

        let max_tokens = user_max_tokens
            .map(|user_val| {
                if self.config.max_tokens_per_job == 0 {
                    // Config is "unlimited": use the user-supplied value directly.
                    user_val
                } else {
                    std::cmp::min(user_val, self.config.max_tokens_per_job)
                }
            })
            .unwrap_or(self.config.max_tokens_per_job);

        // Apply both metadata and token budget in one closure (Issue #813: atomic update)
        if let Some(meta) = metadata {
            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.metadata = meta;
                    if max_tokens > 0 {
                        ctx.max_tokens = max_tokens;
                    }
                    // Store approval context in JobContext so tools can access it
                    if let Some(ref approval) = approval_context {
                        ctx.approval_context = Some(approval.clone());
                    }
                })
                .await?;
        } else {
            self.context_manager
                .update_context(job_id, |ctx| {
                    if max_tokens > 0 {
                        ctx.max_tokens = max_tokens;
                    }
                    // Store approval context in JobContext so tools can access it
                    if let Some(ref approval) = approval_context {
                        ctx.approval_context = Some(approval.clone());
                    }
                })
                .await?;
        }

        // Persist to DB before scheduling so the worker's FK references are valid
        if let Some(ref store) = self.store {
            let ctx = self.context_manager.get_context(job_id).await?;
            store.save_job(&ctx).await.map_err(|e| JobError::Failed {
                id: job_id,
                reason: format!("failed to persist job: {e}"),
            })?;
        }

        self.schedule_with_context(job_id, approval_context).await?;
        Ok(job_id)
    }

    /// Schedule a job for execution.
    pub async fn schedule(&self, job_id: Uuid) -> Result<(), JobError> {
        self.schedule_with_context(job_id, None).await
    }

    /// Schedule a job with an optional approval context.
    async fn schedule_with_context(
        &self,
        job_id: Uuid,
        approval_context: Option<ApprovalContext>,
    ) -> Result<(), JobError> {
        // Hold write lock for the entire check-insert sequence to prevent
        // TOCTOU races where two concurrent calls both pass the checks.
        {
            let mut jobs = self.jobs.write().await;

            if jobs.contains_key(&job_id) {
                return Ok(());
            }

            if jobs.len() >= self.config.max_parallel_jobs {
                return Err(JobError::MaxJobsExceeded {
                    max: self.config.max_parallel_jobs,
                });
            }

            // Transition job to in_progress
            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.transition_to(
                        JobState::InProgress,
                        Some("Scheduled for execution".to_string()),
                    )
                })
                .await?
                .map_err(|s| JobError::ContextError {
                    id: job_id,
                    reason: s,
                })?;

            // Create worker channel
            let (tx, rx) = mpsc::channel(16);

            // Create worker with shared dependencies
            let deps = WorkerDeps {
                context_manager: self.context_manager.clone(),
                llm: self.llm.clone(),
                safety: self.safety.clone(),
                tools: self.tools.clone(),
                store: self.store.clone(),
                hooks: self.hooks.clone(),
                timeout: self.config.job_timeout,
                use_planning: self.config.use_planning,
                sse_tx: self.sse_tx.clone(),
                approval_context,
                http_interceptor: self.http_interceptor.clone(),
            };
            let worker = Worker::new(job_id, deps);

            // Spawn worker task
            let handle = tokio::spawn(async move {
                if let Err(e) = worker.run(rx).await {
                    tracing::error!("Worker for job {} failed: {}", job_id, e);
                }
            });

            // Start the worker
            if tx.send(WorkerMessage::Start).await.is_err() {
                tracing::error!(job_id = %job_id, "Worker died before receiving Start message");
            }

            // Insert while still holding the write lock
            jobs.insert(job_id, ScheduledJob { handle, tx });
        }

        // Cleanup task for this job to avoid capacity leaks
        let jobs = Arc::clone(&self.jobs);
        tokio::spawn(async move {
            loop {
                let finished = {
                    let jobs_read = jobs.read().await;
                    match jobs_read.get(&job_id) {
                        Some(scheduled) => scheduled.handle.is_finished(),
                        None => true,
                    }
                };

                if finished {
                    jobs.write().await.remove(&job_id);
                    break;
                }

                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });

        tracing::info!("Scheduled job {} for execution", job_id);
        Ok(())
    }

    /// Schedule a sub-task from within a worker.
    ///
    /// Sub-tasks are lightweight tasks that don't go through the full job lifecycle.
    /// They're used for parallel tool execution and background computations.
    ///
    /// Returns a oneshot receiver to get the result.
    pub async fn spawn_subtask(
        &self,
        parent_id: Uuid,
        task: Task,
    ) -> Result<oneshot::Receiver<Result<TaskOutput, Error>>, JobError> {
        let task_id = Uuid::new_v4();
        let (result_tx, result_rx) = oneshot::channel();

        let handle = match task {
            Task::Job { .. } => {
                // Jobs should go through schedule(), not spawn_subtask
                return Err(JobError::ContextError {
                    id: parent_id,
                    reason: "Use schedule() for Job tasks, not spawn_subtask()".to_string(),
                });
            }

            Task::ToolExec {
                parent_id: tool_parent_id,
                tool_name,
                params,
            } => {
                let tools = self.tools.clone();
                let context_manager = self.context_manager.clone();
                let safety = self.safety.clone();

                // TODO: propagate parent job's ApprovalContext here when subtasks
                // are used in autonomous/routine paths (currently only used in tests).
                tokio::spawn(async move {
                    let result = Self::execute_tool_task(
                        tools,
                        context_manager,
                        safety,
                        None,
                        tool_parent_id,
                        &tool_name,
                        params,
                    )
                    .await;

                    // Send result (ignore if receiver dropped)
                    let _ = result_tx.send(result);
                })
            }

            Task::Background { id: _, handler } => {
                let ctx = TaskContext::new(task_id).with_parent(parent_id);

                tokio::spawn(async move {
                    let result = handler.run(ctx).await;
                    let _ = result_tx.send(result);
                })
            }
        };

        // Track the subtask
        self.subtasks.write().await.insert(
            task_id,
            ScheduledSubtask {
                handle: tokio::spawn(async move {
                    // Wrap the handle to get its result
                    match handle.await {
                        Ok(()) => Err(Error::Job(JobError::ContextError {
                            id: task_id,
                            reason: "Subtask completed but result not captured".to_string(),
                        })),
                        Err(e) => Err(Error::Job(JobError::ContextError {
                            id: task_id,
                            reason: format!("Subtask panicked: {}", e),
                        })),
                    }
                }),
            },
        );

        // Cleanup task for subtask tracking
        let subtasks = Arc::clone(&self.subtasks);
        tokio::spawn(async move {
            loop {
                let finished = {
                    let subtasks_read = subtasks.read().await;
                    match subtasks_read.get(&task_id) {
                        Some(scheduled) => scheduled.handle.is_finished(),
                        None => true,
                    }
                };

                if finished {
                    subtasks.write().await.remove(&task_id);
                    break;
                }

                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });

        tracing::debug!(
            parent_id = %parent_id,
            task_id = %task_id,
            "Spawned subtask"
        );

        Ok(result_rx)
    }

    /// Schedule multiple tasks in parallel and wait for all to complete.
    ///
    /// Returns results in the same order as the input tasks.
    pub async fn spawn_batch(
        &self,
        parent_id: Uuid,
        tasks: Vec<Task>,
    ) -> Vec<Result<TaskOutput, Error>> {
        if tasks.is_empty() {
            return Vec::new();
        }

        let mut receivers = Vec::with_capacity(tasks.len());

        // Spawn all tasks
        for task in tasks {
            match self.spawn_subtask(parent_id, task).await {
                Ok(rx) => receivers.push(Some(rx)),
                Err(e) => {
                    // Store the error directly
                    receivers.push(None);
                    tracing::warn!(
                        parent_id = %parent_id,
                        error = %e,
                        "Failed to spawn subtask in batch"
                    );
                }
            }
        }

        // Collect results
        let mut results = Vec::with_capacity(receivers.len());
        for rx in receivers {
            let result = match rx {
                Some(receiver) => match receiver.await {
                    Ok(task_result) => task_result,
                    Err(_) => Err(Error::Job(JobError::ContextError {
                        id: parent_id,
                        reason: "Subtask channel closed unexpectedly".to_string(),
                    })),
                },
                None => Err(Error::Job(JobError::ContextError {
                    id: parent_id,
                    reason: "Subtask failed to spawn".to_string(),
                })),
            };
            results.push(result);
        }

        results
    }

    /// Execute a single tool as a subtask.
    ///
    /// Performs scheduler-specific checks (approval, cancellation) then
    /// delegates to the shared `execute_tool_with_safety` pipeline.
    async fn execute_tool_task(
        tools: Arc<ToolRegistry>,
        context_manager: Arc<ContextManager>,
        safety: Arc<SafetyLayer>,
        approval_context: Option<ApprovalContext>,
        job_id: Uuid,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<TaskOutput, Error> {
        let start = std::time::Instant::now();

        // Get the tool for approval check
        let tool = tools.get(tool_name).await.ok_or_else(|| {
            Error::Tool(crate::error::ToolError::NotFound {
                name: tool_name.to_string(),
            })
        })?;

        // Get job context
        let job_ctx: JobContext = context_manager.get_context(job_id).await?;
        if job_ctx.state == JobState::Cancelled {
            return Err(crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: "Job is cancelled".to_string(),
            }
            .into());
        }

        // Scheduler-specific approval check
        let requirement = tool.requires_approval(&params);
        let blocked =
            ApprovalContext::is_blocked_or_default(&approval_context, tool_name, requirement);
        if blocked {
            return Err(crate::error::ToolError::AuthRequired {
                name: tool_name.to_string(),
            }
            .into());
        }

        // Delegate to shared tool execution pipeline
        let output_str = crate::tools::execute::execute_tool_with_safety(
            &tools, &safety, tool_name, &params, &job_ctx,
        )
        .await?;

        // Parse back to Value for TaskOutput; this should be infallible given
        // `execute_tool_with_safety` uses `serde_json::to_string_pretty`, but if it
        // ever fails we surface a clear error instead of silently changing types.
        let result_value: serde_json::Value = serde_json::from_str(&output_str).map_err(|e| {
            Error::Tool(crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: format!("Failed to parse tool output as JSON: {}", e),
            })
        })?;

        Ok(TaskOutput::new(result_value, start.elapsed()))
    }

    /// Stop a running job.
    pub async fn stop(&self, job_id: Uuid) -> Result<(), JobError> {
        let mut jobs = self.jobs.write().await;

        if let Some(scheduled) = jobs.remove(&job_id) {
            // Send stop signal
            let _ = scheduled.tx.send(WorkerMessage::Stop).await;

            // Give it a moment to clean up
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Abort if still running
            if !scheduled.handle.is_finished() {
                scheduled.handle.abort();
            }

            // Update job state
            self.context_manager
                .update_context(job_id, |ctx| {
                    if let Err(e) = ctx.transition_to(
                        JobState::Cancelled,
                        Some("Stopped by scheduler".to_string()),
                    ) {
                        tracing::warn!(
                            job_id = %job_id,
                            error = %e,
                            "Failed to transition job to Cancelled state"
                        );
                    }
                })
                .await?;

            // Persist cancellation (fire-and-forget)
            if let Some(ref store) = self.store {
                let store = store.clone();
                tokio::spawn(async move {
                    if let Err(e) = store
                        .update_job_status(
                            job_id,
                            JobState::Cancelled,
                            Some("Stopped by scheduler"),
                        )
                        .await
                    {
                        tracing::warn!("Failed to persist cancellation for job {}: {}", job_id, e);
                    }
                });
            }

            tracing::info!("Stopped job {}", job_id);
        }

        Ok(())
    }

    /// Send a follow-up user message to a running job.
    ///
    /// Returns `Ok(())` if the message was queued, `Err` if the job is not running.
    pub async fn send_message(&self, job_id: Uuid, content: String) -> Result<(), JobError> {
        // Clone the sender while holding the lock, then release before the
        // async send to avoid blocking scheduler writes during backpressure.
        let tx = {
            let jobs = self.jobs.read().await;
            let scheduled = jobs.get(&job_id).ok_or(JobError::NotFound { id: job_id })?;
            scheduled.tx.clone()
        };
        tx.send(WorkerMessage::UserMessage(content))
            .await
            .map_err(|_| JobError::Failed {
                id: job_id,
                reason: "Worker channel closed".to_string(),
            })?;
        Ok(())
    }

    /// Check if a job is running.
    pub async fn is_running(&self, job_id: Uuid) -> bool {
        self.jobs.read().await.contains_key(&job_id)
    }

    /// Get count of running jobs.
    pub async fn running_count(&self) -> usize {
        self.jobs.read().await.len()
    }

    /// Get count of running subtasks.
    pub async fn subtask_count(&self) -> usize {
        self.subtasks.read().await.len()
    }

    /// Get all running job IDs.
    pub async fn running_jobs(&self) -> Vec<Uuid> {
        self.jobs.read().await.keys().cloned().collect()
    }

    /// Clean up finished jobs and subtasks.
    pub async fn cleanup_finished(&self) {
        // Clean up jobs
        {
            let mut jobs = self.jobs.write().await;
            let mut finished = Vec::new();

            for (id, scheduled) in jobs.iter() {
                if scheduled.handle.is_finished() {
                    finished.push(*id);
                }
            }

            for id in finished {
                jobs.remove(&id);
                tracing::debug!("Cleaned up finished job {}", id);
            }
        }

        // Clean up subtasks
        {
            let mut subtasks = self.subtasks.write().await;
            let mut finished = Vec::new();

            for (id, scheduled) in subtasks.iter() {
                if scheduled.handle.is_finished() {
                    finished.push(*id);
                }
            }

            for id in finished {
                subtasks.remove(&id);
                tracing::trace!("Cleaned up finished subtask {}", id);
            }
        }
    }

    /// Stop all jobs.
    pub async fn stop_all(&self) {
        let job_ids: Vec<Uuid> = self.jobs.read().await.keys().cloned().collect();

        for job_id in job_ids {
            let _ = self.stop(job_id).await;
        }

        // Abort all subtasks
        let mut subtasks = self.subtasks.write().await;
        for (_, scheduled) in subtasks.drain() {
            scheduled.handle.abort();
        }
    }

    /// Get access to the tools registry.
    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.tools
    }

    /// Get access to the context manager.
    pub fn context_manager(&self) -> &Arc<ContextManager> {
        &self.context_manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SafetyConfig;
    use crate::llm::{
        CompletionRequest, CompletionResponse, LlmError, LlmProvider, ToolCompletionRequest,
        ToolCompletionResponse,
    };
    use crate::safety::SafetyLayer;
    use crate::tools::{ApprovalRequirement, Tool, ToolError, ToolOutput};
    use rust_decimal_macros::dec;

    /// Minimal LLM provider stub for scheduler tests that don't exercise LLM calls.
    struct StubLlm;

    #[async_trait::async_trait]
    impl LlmProvider for StubLlm {
        fn model_name(&self) -> &str {
            "stub"
        }
        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            (dec!(0), dec!(0))
        }
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: "stub".into(),
                reason: "not implemented".into(),
            })
        }
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: "stub".into(),
                reason: "not implemented".into(),
            })
        }
    }

    /// Create a Scheduler for token-budget tests. The LLM stub will fail if a
    /// worker actually tries to call it, but `dispatch_job` sets the token
    /// budget *before* spawning the worker so we can inspect the context
    /// immediately after dispatch.
    fn make_test_scheduler(max_tokens_per_job: u64) -> Scheduler {
        let config = AgentConfig {
            name: "test".to_string(),
            max_parallel_jobs: 5,
            job_timeout: std::time::Duration::from_secs(30),
            stuck_threshold: std::time::Duration::from_secs(300),
            repair_check_interval: std::time::Duration::from_secs(3600),
            max_repair_attempts: 0,
            use_planning: false,
            session_idle_timeout: std::time::Duration::from_secs(3600),
            allow_local_tools: true,
            max_cost_per_day_cents: None,
            max_actions_per_hour: None,
            max_tool_iterations: 10,
            auto_approve_tools: true,
            default_timezone: "UTC".to_string(),
            max_tokens_per_job,
        };
        let cm = Arc::new(ContextManager::new(5));
        let llm: Arc<dyn LlmProvider> = Arc::new(StubLlm);
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        }));
        let tools = Arc::new(ToolRegistry::new());
        let hooks = Arc::new(HookRegistry::default());

        Scheduler::new(config, cm, llm, safety, tools, None, hooks)
    }

    #[tokio::test]
    async fn test_dispatch_job_caps_user_max_tokens() {
        let sched = make_test_scheduler(1000);
        let meta = serde_json::json!({ "max_tokens": 5000 });
        let job_id = sched
            .dispatch_job("user1", "test", "desc", Some(meta))
            .await
            .unwrap();

        let ctx = sched.context_manager.get_context(job_id).await.unwrap();
        assert_eq!(ctx.max_tokens, 1000, "should cap at configured limit");
    }

    #[tokio::test]
    async fn test_dispatch_job_unlimited_config_preserves_user_tokens() {
        let sched = make_test_scheduler(0); // 0 = unlimited
        let meta = serde_json::json!({ "max_tokens": 5000 });
        let job_id = sched
            .dispatch_job("user1", "test", "desc", Some(meta))
            .await
            .unwrap();

        let ctx = sched.context_manager.get_context(job_id).await.unwrap();
        assert_eq!(
            ctx.max_tokens, 5000,
            "unlimited config should preserve user value"
        );
    }

    #[tokio::test]
    async fn test_dispatch_job_no_user_tokens_uses_config() {
        let sched = make_test_scheduler(2000);
        let job_id = sched
            .dispatch_job("user1", "test", "desc", None)
            .await
            .unwrap();

        let ctx = sched.context_manager.get_context(job_id).await.unwrap();
        assert_eq!(
            ctx.max_tokens, 2000,
            "should use config default when no user value"
        );
    }

    #[tokio::test]
    async fn test_dispatch_job_atomic_metadata_and_tokens() {
        let sched = make_test_scheduler(10_000);
        let meta = serde_json::json!({
            "max_tokens": 3000,
            "custom_key": "custom_value"
        });
        let job_id = sched
            .dispatch_job("user1", "test", "desc", Some(meta))
            .await
            .unwrap();

        let ctx = sched.context_manager.get_context(job_id).await.unwrap();
        assert_eq!(ctx.max_tokens, 3000, "should use user value within limit");
        assert_eq!(
            ctx.metadata.get("custom_key").and_then(|v| v.as_str()),
            Some("custom_value"),
            "metadata should be set atomically with token budget"
        );
    }

    #[test]
    fn test_scheduler_creation() {
        // Would need to mock dependencies for proper testing
    }

    #[tokio::test]
    async fn test_spawn_batch_empty() {
        // This test would need mock dependencies.
        // For now just verify the empty case doesn't panic.
    }

    /// A tool that returns `UnlessAutoApproved`.
    struct SoftApprovalTool;

    #[async_trait::async_trait]
    impl Tool for SoftApprovalTool {
        fn name(&self) -> &str {
            "soft_gate"
        }
        fn description(&self) -> &str {
            "needs soft approval"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text(
                "soft_ok",
                std::time::Instant::now().elapsed(),
            ))
        }
        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            ApprovalRequirement::UnlessAutoApproved
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    /// A tool that returns `Always`.
    struct HardApprovalTool;

    #[async_trait::async_trait]
    impl Tool for HardApprovalTool {
        fn name(&self) -> &str {
            "hard_gate"
        }
        fn description(&self) -> &str {
            "needs hard approval"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text(
                "hard_ok",
                std::time::Instant::now().elapsed(),
            ))
        }
        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            ApprovalRequirement::Always
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    async fn setup_tools_and_job() -> (
        Arc<ToolRegistry>,
        Arc<ContextManager>,
        Arc<SafetyLayer>,
        Uuid,
    ) {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(SoftApprovalTool)).await;
        registry.register(Arc::new(HardApprovalTool)).await;

        let cm = Arc::new(ContextManager::new(5));
        let job_id = cm.create_job("test", "approval test").await.unwrap();
        cm.update_context(job_id, |ctx| ctx.transition_to(JobState::InProgress, None))
            .await
            .unwrap()
            .unwrap();

        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        }));

        (Arc::new(registry), cm, safety, job_id)
    }

    #[tokio::test]
    async fn test_execute_tool_task_blocks_without_context() {
        let (tools, cm, safety, job_id) = setup_tools_and_job().await;

        // Without approval context, UnlessAutoApproved is blocked
        let result = Scheduler::execute_tool_task(
            tools.clone(),
            cm.clone(),
            safety.clone(),
            None,
            job_id,
            "soft_gate",
            serde_json::json!({}),
        )
        .await;
        assert!(
            result.is_err(),
            "soft_gate should be blocked without context"
        );

        // Always is also blocked
        let result = Scheduler::execute_tool_task(
            tools,
            cm,
            safety,
            None,
            job_id,
            "hard_gate",
            serde_json::json!({}),
        )
        .await;
        assert!(
            result.is_err(),
            "hard_gate should be blocked without context"
        );
    }

    #[tokio::test]
    async fn test_execute_tool_task_autonomous_unblocks_soft() {
        let (tools, cm, safety, job_id) = setup_tools_and_job().await;

        // Autonomous context auto-approves UnlessAutoApproved
        let result = Scheduler::execute_tool_task(
            tools.clone(),
            cm.clone(),
            safety.clone(),
            Some(ApprovalContext::autonomous()),
            job_id,
            "soft_gate",
            serde_json::json!({}),
        )
        .await;
        assert!(
            result.is_ok(),
            "soft_gate should pass with autonomous context"
        );

        // But still blocks Always
        let result = Scheduler::execute_tool_task(
            tools,
            cm,
            safety,
            Some(ApprovalContext::autonomous()),
            job_id,
            "hard_gate",
            serde_json::json!({}),
        )
        .await;
        assert!(
            result.is_err(),
            "hard_gate should still be blocked without explicit permission"
        );
    }

    #[tokio::test]
    async fn test_execute_tool_task_autonomous_with_permissions() {
        let (tools, cm, safety, job_id) = setup_tools_and_job().await;

        // Autonomous context with explicit permission for hard_gate
        let ctx = ApprovalContext::autonomous_with_tools(["hard_gate".to_string()]);

        let result = Scheduler::execute_tool_task(
            tools.clone(),
            cm.clone(),
            safety.clone(),
            Some(ctx.clone()),
            job_id,
            "soft_gate",
            serde_json::json!({}),
        )
        .await;
        assert!(result.is_ok(), "soft_gate should pass");

        let result = Scheduler::execute_tool_task(
            tools,
            cm,
            safety,
            Some(ctx),
            job_id,
            "hard_gate",
            serde_json::json!({}),
        )
        .await;
        assert!(
            result.is_ok(),
            "hard_gate should pass with explicit permission"
        );
    }
}
