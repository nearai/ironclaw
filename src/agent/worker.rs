//! Per-job worker execution.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::scheduler::WorkerMessage;
use crate::context::{ContextManager, JobState};
use crate::error::Error;
use crate::history::Store;
use crate::llm::{ChatMessage, LlmProvider, Reasoning, ReasoningContext};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;

/// Worker that executes a single job.
pub struct Worker {
    job_id: Uuid,
    context_manager: Arc<ContextManager>,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    store: Option<Arc<Store>>,
    timeout: Duration,
}

impl Worker {
    /// Create a new worker.
    pub fn new(
        job_id: Uuid,
        context_manager: Arc<ContextManager>,
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
        store: Option<Arc<Store>>,
        timeout: Duration,
    ) -> Self {
        Self {
            job_id,
            context_manager,
            llm,
            safety,
            tools,
            store,
            timeout,
        }
    }

    /// Fire-and-forget persistence of job status.
    fn persist_status(&self, status: JobState, reason: Option<String>) {
        if let Some(ref store) = self.store {
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
        let job_ctx = self.context_manager.get_context(self.job_id).await?;

        // Create reasoning engine
        let reasoning = Reasoning::new(self.llm.clone(), self.safety.clone());

        // Build initial reasoning context
        let tool_defs = self.tools.tool_definitions().await;
        let mut reason_ctx = ReasoningContext::new()
            .with_job(&job_ctx.description)
            .with_tools(tool_defs);

        // Add system message
        reason_ctx.messages.push(ChatMessage::system(format!(
            r#"You are an autonomous agent working on a job.

Job: {}
Description: {}

You have access to tools to complete this job. Plan your approach and execute tools as needed.
Report when the job is complete or if you encounter issues you cannot resolve."#,
            job_ctx.title, job_ctx.description
        )));

        // Main execution loop with timeout
        let result = tokio::time::timeout(self.timeout, async {
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

            iteration += 1;
            if iteration > max_iterations {
                self.mark_stuck("Maximum iterations exceeded").await?;
                return Ok(());
            }

            // Select next tool to use
            let selection = reasoning.select_tool(reason_ctx).await?;

            match selection {
                Some(tool_selection) => {
                    tracing::debug!(
                        "Job {} selecting tool: {} - {}",
                        self.job_id,
                        tool_selection.tool_name,
                        tool_selection.reasoning
                    );

                    // Execute the tool
                    let result = self
                        .execute_tool(&tool_selection.tool_name, &tool_selection.parameters)
                        .await;

                    // Record the result
                    match result {
                        Ok(output) => {
                            // Sanitize output
                            let sanitized = self
                                .safety
                                .sanitize_tool_output(&tool_selection.tool_name, &output);

                            // Add to context
                            let wrapped = self.safety.wrap_for_llm(
                                &tool_selection.tool_name,
                                &sanitized.content,
                                sanitized.was_modified,
                            );

                            reason_ctx.messages.push(ChatMessage::tool_result(
                                "tool_call_id",
                                &tool_selection.tool_name,
                                wrapped,
                            ));

                            // Check if job is complete
                            if output.contains("TASK_COMPLETE") || output.contains("JOB_DONE") {
                                self.mark_completed().await?;
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Tool {} failed for job {}: {}",
                                tool_selection.tool_name,
                                self.job_id,
                                e
                            );

                            reason_ctx.messages.push(ChatMessage::tool_result(
                                "tool_call_id",
                                &tool_selection.tool_name,
                                format!("Error: {}", e),
                            ));
                        }
                    }
                }
                None => {
                    // No tool selected, ask LLM for next steps
                    let response = reasoning.respond(reason_ctx).await?;

                    if response.to_lowercase().contains("complete")
                        || response.to_lowercase().contains("finished")
                        || response.to_lowercase().contains("done")
                    {
                        self.mark_completed().await?;
                        return Ok(());
                    }

                    // Add assistant response to context
                    reason_ctx.messages.push(ChatMessage::assistant(&response));

                    // Give it one more chance to select a tool
                    if iteration > 3 && iteration % 5 == 0 {
                        // Ask if stuck
                        reason_ctx.messages.push(ChatMessage::user(
                            "Are you stuck? Do you need help completing this job?",
                        ));
                    }
                }
            }

            // Small delay between iterations
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<String, Error> {
        let tool =
            self.tools
                .get(tool_name)
                .await
                .ok_or_else(|| crate::error::ToolError::NotFound {
                    name: tool_name.to_string(),
                })?;

        // Get job context for the tool
        let job_ctx = self.context_manager.get_context(self.job_id).await?;

        // Execute with timeout and timing
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            tool.execute(params.clone(), &job_ctx).await
        })
        .await;
        let elapsed = start.elapsed();

        // Record action in memory and get the ActionRecord for persistence
        let action = match &result {
            Ok(Ok(output)) => {
                let output_str = serde_json::to_string_pretty(&output.result).ok();
                self.context_manager
                    .update_memory(self.job_id, |mem| {
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
            Ok(Err(e)) => self
                .context_manager
                .update_memory(self.job_id, |mem| {
                    let rec = mem
                        .create_action(tool_name, params.clone())
                        .fail(e.to_string(), elapsed);
                    mem.record_action(rec.clone());
                    rec
                })
                .await
                .ok(),
            Err(_) => self
                .context_manager
                .update_memory(self.job_id, |mem| {
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
        if let (Some(action), Some(store)) = (action, &self.store) {
            let store = store.clone();
            let job_id = self.job_id;
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
                timeout: Duration::from_secs(60),
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

    async fn mark_completed(&self) -> Result<(), Error> {
        self.context_manager
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
        self.context_manager
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
        self.context_manager
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
