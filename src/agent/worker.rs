//! Per-job worker execution.

use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::scheduler::WorkerMessage;
use crate::agent::task::TaskOutput;
use crate::context::{ContextManager, JobState};
use crate::db::Database;
use crate::error::Error;
use crate::llm::{
    ActionPlan, ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult, ToolSelection,
};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;

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
    pub timeout: Duration,
    pub use_planning: bool,
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
            Some(WorkerMessage::Ping) => {}
        }

        // Get job context
        let job_ctx = self.context_manager().get_context(self.job_id).await?;

        // Create reasoning engine
        let reasoning = Reasoning::new(self.llm().clone(), self.safety().clone());

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
        let max_iterations = 50;
        let mut iteration = 0;

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

        // If we have a plan, execute it
        if let Some(ref plan) = plan {
            return self.execute_plan(rx, reasoning, reason_ctx, plan).await;
        }

        // Otherwise, use direct tool selection loop
        loop {
            // Check for stop signal
            if let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMessage::Stop => {
                        tracing::debug!("Worker for job {} received stop signal", self.job_id);
                        return Ok(());
                    }
                    WorkerMessage::Ping => {
                        tracing::trace!("Worker for job {} received ping", self.job_id);
                    }
                    WorkerMessage::Start => {}
                }
            }

            // Check for cancellation
            if let Ok(ctx) = self.context_manager().get_context(self.job_id).await
                && ctx.state == JobState::Cancelled
            {
                tracing::info!("Worker for job {} detected cancellation", self.job_id);
                return Ok(());
            }

            iteration += 1;
            if iteration > max_iterations {
                self.mark_stuck("Maximum iterations exceeded").await?;
                return Ok(());
            }

            // Refresh tool definitions so newly built tools become visible
            reason_ctx.available_tools = self.tools().tool_definitions().await;

            // Select next tool(s) to use
            let selections = reasoning.select_tools(reason_ctx).await?;

            if selections.is_empty() {
                // No tools from select_tools, ask LLM directly (may still return tool calls)
                let respond_output = reasoning.respond_with_tools(reason_ctx).await?;

                match respond_output.result {
                    RespondResult::Text(response) => {
                        // Check for explicit completion phrases. Use word-boundary
                        // aware checks to avoid false positives like "incomplete",
                        // "not done", or "unfinished". Only the LLM's own response
                        // (not tool output) can trigger this.
                        if crate::util::llm_signals_completion(&response) {
                            self.mark_completed().await?;
                            return Ok(());
                        }

                        // Add assistant response to context
                        reason_ctx.messages.push(ChatMessage::assistant(&response));

                        // Give it one more chance to select a tool
                        if iteration > 3 && iteration % 5 == 0 {
                            reason_ctx.messages.push(ChatMessage::user(
                                "Are you stuck? Do you need help completing this job?",
                            ));
                        }
                    }
                    RespondResult::ToolCalls {
                        tool_calls,
                        content,
                    } => {
                        // Model returned tool calls - execute them
                        tracing::debug!(
                            "Job {} respond_with_tools returned {} tool calls",
                            self.job_id,
                            tool_calls.len()
                        );

                        // Add assistant message with tool_calls (OpenAI protocol)
                        reason_ctx
                            .messages
                            .push(ChatMessage::assistant_with_tool_calls(
                                content,
                                tool_calls.clone(),
                            ));

                        for tc in tool_calls {
                            let result = self.execute_tool(&tc.name, &tc.arguments).await;

                            // Create synthetic selection for process_tool_result
                            let selection = ToolSelection {
                                tool_name: tc.name.clone(),
                                parameters: tc.arguments.clone(),
                                reasoning: String::new(),
                                alternatives: vec![],
                                tool_call_id: tc.id.clone(),
                            };

                            self.process_tool_result(reason_ctx, &selection, result)
                                .await?;
                        }
                    }
                }
            } else if selections.len() == 1 {
                // Single tool: execute directly
                let selection = &selections[0];
                tracing::debug!(
                    "Job {} selecting tool: {} - {}",
                    self.job_id,
                    selection.tool_name,
                    selection.reasoning
                );

                let result = self
                    .execute_tool(&selection.tool_name, &selection.parameters)
                    .await;

                self.process_tool_result(reason_ctx, selection, result)
                    .await?;
            } else {
                // Multiple tools: execute in parallel
                tracing::debug!(
                    "Job {} executing {} tools in parallel",
                    self.job_id,
                    selections.len()
                );

                let results = self.execute_tools_parallel(&selections).await;

                // Process all results
                for (selection, result) in selections.iter().zip(results) {
                    self.process_tool_result(reason_ctx, selection, result.result)
                        .await?;
                }
            }

            // Small delay between iterations
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Execute multiple tools in parallel.
    async fn execute_tools_parallel(&self, selections: &[ToolSelection]) -> Vec<ToolExecResult> {
        let futures: Vec<_> = selections
            .iter()
            .map(|selection| {
                let tool_name = selection.tool_name.clone();
                let params = selection.parameters.clone();
                let tools = self.tools().clone();
                let context_manager = self.context_manager().clone();
                let safety = self.safety().clone();
                let job_id = self.job_id;
                let store = self.deps.store.clone();

                async move {
                    let result = Self::execute_tool_inner(
                        tools,
                        context_manager,
                        safety,
                        store,
                        job_id,
                        &tool_name,
                        &params,
                    )
                    .await;
                    ToolExecResult { result }
                }
            })
            .collect();

        join_all(futures).await
    }

    /// Inner tool execution logic that can be called from both single and parallel paths.
    async fn execute_tool_inner(
        tools: Arc<ToolRegistry>,
        context_manager: Arc<ContextManager>,
        safety: Arc<SafetyLayer>,
        store: Option<Arc<dyn Database>>,
        job_id: Uuid,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<String, Error> {
        let tool = tools
            .get(tool_name)
            .await
            .ok_or_else(|| crate::error::ToolError::NotFound {
                name: tool_name.to_string(),
            })?;

        // Tools requiring approval are blocked in autonomous jobs
        if tool.requires_approval() {
            return Err(crate::error::ToolError::AuthRequired {
                name: tool_name.to_string(),
            }
            .into());
        }

        // Get job context for the tool
        let job_ctx = context_manager.get_context(job_id).await?;
        if job_ctx.state == JobState::Cancelled {
            return Err(crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: "Job is cancelled".to_string(),
            }
            .into());
        }

        // Validate tool parameters
        let validation = safety.validator().validate_tool_params(params);
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

        tracing::debug!(
            tool = %tool_name,
            params = %params,
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
                    .map(|s| safety.sanitize_tool_output(tool_name, &s).content);
                context_manager
                    .update_memory(job_id, |mem| {
                        let rec = mem.create_action(tool_name, params.clone()).succeed(
                            output_str.clone(),
                            output.result.clone(),
                            elapsed,
                        );
                        mem.record_action(rec.clone());
                        rec
                    })
                    .await
                    .ok()
            }
            Ok(Err(e)) => context_manager
                .update_memory(job_id, |mem| {
                    let rec = mem
                        .create_action(tool_name, params.clone())
                        .fail(e.to_string(), elapsed);
                    mem.record_action(rec.clone());
                    rec
                })
                .await
                .ok(),
            Err(_) => context_manager
                .update_memory(job_id, |mem| {
                    let rec = mem
                        .create_action(tool_name, params.clone())
                        .fail("Execution timeout", elapsed);
                    mem.record_action(rec.clone());
                    rec
                })
                .await
                .ok(),
        };

        // Persist action to database (fire-and-forget)
        if let (Some(action), Some(store)) = (action, store) {
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
    async fn process_tool_result(
        &self,
        reason_ctx: &mut ReasoningContext,
        selection: &ToolSelection,
        result: Result<String, Error>,
    ) -> Result<bool, Error> {
        match result {
            Ok(output) => {
                // Sanitize output
                let sanitized = self
                    .safety()
                    .sanitize_tool_output(&selection.tool_name, &output);

                // Add to context
                let wrapped = self.safety().wrap_for_llm(
                    &selection.tool_name,
                    &sanitized.content,
                    sanitized.was_modified,
                );

                reason_ctx.messages.push(ChatMessage::tool_result(
                    &selection.tool_call_id,
                    &selection.tool_name,
                    wrapped,
                ));

                // Tool output never drives job completion. A malicious tool could
                // emit "TASK_COMPLETE" to force premature completion. Only the LLM's
                // own structured response (in execution_loop) can mark a job done.
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

                reason_ctx.messages.push(ChatMessage::tool_result(
                    &selection.tool_call_id,
                    &selection.tool_name,
                    format!("Error: {}", e),
                ));

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
            // Check for stop signal
            if let Ok(msg) = rx.try_recv() {
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

            // Execute the planned tool
            let result = self
                .execute_tool(&action.tool_name, &action.parameters)
                .await;

            // Create a synthetic ToolSelection for process_tool_result.
            // Plan actions don't originate from an LLM tool_call response so
            // there is no real tool_call_id; generate a unique one.
            let selection = ToolSelection {
                tool_name: action.tool_name.clone(),
                parameters: action.parameters.clone(),
                reasoning: action.reasoning.clone(),
                alternatives: vec![],
                tool_call_id: format!("plan_{}_{}", self.job_id, i),
            };

            // Process the result
            let completed = self
                .process_tool_result(reason_ctx, &selection, result)
                .await?;

            if completed {
                return Ok(());
            }

            // Small delay between actions
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
            // Job not complete, could re-plan or fall back to direct selection
            tracing::info!(
                "Job {} plan completed but work remains, falling back to direct selection",
                self.job_id
            );
            // Continue with standard execution loop by returning (will be picked up by main loop)
            self.mark_stuck("Plan completed but job incomplete - needs re-planning")
                .await?;
        }

        Ok(())
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<String, Error> {
        Self::execute_tool_inner(
            self.tools().clone(),
            self.context_manager().clone(),
            self.safety().clone(),
            self.deps.store.clone(),
            self.job_id,
            tool_name,
            params,
        )
        .await
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

        self.persist_status(JobState::Stuck, Some(reason.to_string()));
        Ok(())
    }
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
        // These contain completion keywords but also negation, should NOT trigger.
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
        // Bare words embedded in other text should NOT trigger completion.
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
        // A malicious tool output echoed by the LLM should not trigger
        // completion unless it forms a genuine completion phrase.
        assert!(!llm_signals_completion("TASK_COMPLETE"));
        assert!(!llm_signals_completion("JOB_DONE"));
        assert!(!llm_signals_completion(
            "The tool returned: TASK_COMPLETE signal"
        ));
    }
}
