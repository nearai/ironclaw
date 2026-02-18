//! System commands and job handlers for the agent.
//!
//! Extracted from `agent_loop.rs` to isolate the /help, /model, /status,
//! and other command processing from the core agent loop.

use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::session::Session;
use crate::agent::submission::SubmissionResult;
use crate::agent::{Agent, MessageIntent};
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::error::Error;
use crate::llm::ChatMessage;

impl Agent {
    /// Handle job-related intents without turn tracking.
    pub(super) async fn handle_job_or_command(
        &self,
        intent: MessageIntent,
        message: &IncomingMessage,
    ) -> Result<SubmissionResult, Error> {
        // Send thinking status for non-trivial operations
        if let MessageIntent::CreateJob { .. } = &intent {
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Thinking("Processing...".into()),
                    &message.metadata,
                )
                .await;
        }

        let response = match intent {
            MessageIntent::CreateJob {
                title,
                description,
                category,
            } => {
                self.handle_create_job(&message.user_id, title, description, category)
                    .await?
            }
            MessageIntent::CheckJobStatus { job_id } => {
                self.handle_check_status(&message.user_id, job_id).await?
            }
            MessageIntent::CancelJob { job_id } => {
                self.handle_cancel_job(&message.user_id, &job_id).await?
            }
            MessageIntent::ListJobs { filter } => {
                self.handle_list_jobs(&message.user_id, filter).await?
            }
            MessageIntent::HelpJob { job_id } => {
                self.handle_help_job(&message.user_id, &job_id).await?
            }
            MessageIntent::Command { command, args } => {
                match self.handle_command(&command, &args).await? {
                    Some(s) => s,
                    None => return Ok(SubmissionResult::Ok { message: None }), // Shutdown signal
                }
            }
            _ => "Unknown intent".to_string(),
        };
        Ok(SubmissionResult::response(response))
    }

    async fn handle_create_job(
        &self,
        user_id: &str,
        title: String,
        description: String,
        category: Option<String>,
    ) -> Result<String, Error> {
        // Create job context
        let job_id = self
            .context_manager
            .create_job_for_user(user_id, &title, &description)
            .await?;

        // Update category if provided
        if let Some(cat) = category {
            self.context_manager
                .update_context(job_id, |ctx| {
                    ctx.category = Some(cat);
                })
                .await?;
        }

        // Persist new job to database (fire-and-forget)
        if let Some(store) = self.store()
            && let Ok(ctx) = self.context_manager.get_context(job_id).await
        {
            let store = store.clone();
            tokio::spawn(async move {
                if let Err(e) = store.save_job(&ctx).await {
                    tracing::warn!("Failed to persist new job {}: {}", job_id, e);
                }
            });
        }

        // Schedule for execution
        self.scheduler.schedule(job_id).await?;

        Ok(format!(
            "Created job: {}\nID: {}\n\nThe job has been scheduled and is now running.",
            title, job_id
        ))
    }

    async fn handle_check_status(
        &self,
        user_id: &str,
        job_id: Option<String>,
    ) -> Result<String, Error> {
        match job_id {
            Some(id) => {
                let uuid = Uuid::parse_str(&id)
                    .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

                let ctx = self.context_manager.get_context(uuid).await?;
                if ctx.user_id != user_id {
                    return Err(crate::error::JobError::NotFound { id: uuid }.into());
                }

                Ok(format!(
                    "Job: {}\nStatus: {:?}\nCreated: {}\nStarted: {}\nActual cost: {}",
                    ctx.title,
                    ctx.state,
                    ctx.created_at.format("%Y-%m-%d %H:%M:%S"),
                    ctx.started_at
                        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "Not started".to_string()),
                    ctx.actual_cost
                ))
            }
            None => {
                // Show summary of all jobs
                let summary = self.context_manager.summary_for(user_id).await;
                Ok(format!(
                    "Jobs summary:\n  Total: {}\n  In Progress: {}\n  Completed: {}\n  Failed: {}\n  Stuck: {}",
                    summary.total,
                    summary.in_progress,
                    summary.completed,
                    summary.failed,
                    summary.stuck
                ))
            }
        }
    }

    async fn handle_cancel_job(&self, user_id: &str, job_id: &str) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        let ctx = self.context_manager.get_context(uuid).await?;
        if ctx.user_id != user_id {
            return Err(crate::error::JobError::NotFound { id: uuid }.into());
        }

        self.scheduler.stop(uuid).await?;

        Ok(format!("Job {} has been cancelled.", job_id))
    }

    async fn handle_list_jobs(
        &self,
        user_id: &str,
        _filter: Option<String>,
    ) -> Result<String, Error> {
        let jobs = self.context_manager.all_jobs_for(user_id).await;

        if jobs.is_empty() {
            return Ok("No jobs found.".to_string());
        }

        let mut output = String::from("Jobs:\n");
        for job_id in jobs {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await
                && ctx.user_id == user_id
            {
                output.push_str(&format!("  {} - {} ({:?})\n", job_id, ctx.title, ctx.state));
            }
        }

        Ok(output)
    }

    async fn handle_help_job(&self, user_id: &str, job_id: &str) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        let ctx = self.context_manager.get_context(uuid).await?;
        if ctx.user_id != user_id {
            return Err(crate::error::JobError::NotFound { id: uuid }.into());
        }

        if ctx.state == crate::context::JobState::Stuck {
            // Attempt recovery
            self.context_manager
                .update_context(uuid, |ctx| ctx.attempt_recovery())
                .await?
                .map_err(|s| crate::error::JobError::ContextError {
                    id: uuid,
                    reason: s,
                })?;

            // Reschedule
            self.scheduler.schedule(uuid).await?;

            Ok(format!(
                "Job {} was stuck. Attempting recovery (attempt #{}).",
                job_id,
                ctx.repair_attempts + 1
            ))
        } else {
            Ok(format!(
                "Job {} is not stuck (current state: {:?}). No help needed.",
                job_id, ctx.state
            ))
        }
    }

    /// Trigger a manual heartbeat check.
    pub(super) async fn process_heartbeat(&self) -> Result<SubmissionResult, Error> {
        let Some(workspace) = self.workspace() else {
            return Ok(SubmissionResult::error(
                "Heartbeat requires a workspace (database must be connected).",
            ));
        };

        let runner = crate::agent::HeartbeatRunner::new(
            crate::agent::HeartbeatConfig::default(),
            workspace.clone(),
            self.llm().clone(),
        );

        match runner.check_heartbeat().await {
            crate::agent::HeartbeatResult::Ok => Ok(SubmissionResult::ok_with_message(
                "Heartbeat: all clear, nothing needs attention.",
            )),
            crate::agent::HeartbeatResult::NeedsAttention(msg) => Ok(SubmissionResult::response(
                format!("Heartbeat findings:\n\n{}", msg),
            )),
            crate::agent::HeartbeatResult::Skipped => Ok(SubmissionResult::ok_with_message(
                "Heartbeat skipped: no HEARTBEAT.md checklist found in workspace.",
            )),
            crate::agent::HeartbeatResult::Failed(err) => Ok(SubmissionResult::error(format!(
                "Heartbeat failed: {}",
                err
            ))),
        }
    }

    /// Summarize the current thread's conversation.
    pub(super) async fn process_summarize(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let messages = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.messages()
        };

        if messages.is_empty() {
            return Ok(SubmissionResult::ok_with_message(
                "Nothing to summarize (empty thread).",
            ));
        }

        // Build a summary prompt with the conversation
        let mut context = Vec::new();
        context.push(ChatMessage::system(
            "Summarize the conversation so far in 3-5 concise bullet points. \
             Focus on decisions made, actions taken, and key outcomes. \
             Be brief and factual.",
        ));
        // Include the conversation messages (truncate to last 20 to avoid context overflow)
        let start = if messages.len() > 20 {
            messages.len() - 20
        } else {
            0
        };
        context.extend_from_slice(&messages[start..]);
        context.push(ChatMessage::user("Summarize this conversation."));

        let request = crate::llm::CompletionRequest::new(context)
            .with_max_tokens(512)
            .with_temperature(0.3);

        match self.llm().complete(request).await {
            Ok(response) => Ok(SubmissionResult::response(format!(
                "Thread Summary:\n\n{}",
                response.content.trim()
            ))),
            Err(e) => Ok(SubmissionResult::error(format!("Summarize failed: {}", e))),
        }
    }

    /// Suggest next steps based on the current thread.
    pub(super) async fn process_suggest(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let messages = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.messages()
        };

        if messages.is_empty() {
            return Ok(SubmissionResult::ok_with_message(
                "Nothing to suggest from (empty thread).",
            ));
        }

        let mut context = Vec::new();
        context.push(ChatMessage::system(
            "Based on the conversation so far, suggest 2-4 concrete next steps the user could take. \
             Be actionable and specific. Format as a numbered list.",
        ));
        let start = if messages.len() > 20 {
            messages.len() - 20
        } else {
            0
        };
        context.extend_from_slice(&messages[start..]);
        context.push(ChatMessage::user("What should I do next?"));

        let request = crate::llm::CompletionRequest::new(context)
            .with_max_tokens(512)
            .with_temperature(0.5);

        match self.llm().complete(request).await {
            Ok(response) => Ok(SubmissionResult::response(format!(
                "Suggested Next Steps:\n\n{}",
                response.content.trim()
            ))),
            Err(e) => Ok(SubmissionResult::error(format!("Suggest failed: {}", e))),
        }
    }

    /// Handle system commands that bypass thread-state checks entirely.
    pub(super) async fn handle_system_command(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<SubmissionResult, Error> {
        match command {
            "help" => Ok(SubmissionResult::response(concat!(
                "System:\n",
                "  /help             Show this help\n",
                "  /model [name]     Show or switch the active model\n",
                "  /version          Show version info\n",
                "  /tools            List available tools\n",
                "  /debug            Toggle debug mode\n",
                "  /ping             Connectivity check\n",
                "\n",
                "Jobs:\n",
                "  /job <desc>       Create a new job\n",
                "  /status [id]      Check job status\n",
                "  /cancel <id>      Cancel a job\n",
                "  /list             List all jobs\n",
                "\n",
                "Session:\n",
                "  /undo             Undo last turn\n",
                "  /redo             Redo undone turn\n",
                "  /compact          Compress context window\n",
                "  /clear            Clear current thread\n",
                "  /interrupt        Stop current operation\n",
                "  /new              New conversation thread\n",
                "  /thread <id>      Switch to thread\n",
                "  /resume <id>      Resume from checkpoint\n",
                "\n",
                "Agent:\n",
                "  /heartbeat        Run heartbeat check\n",
                "  /summarize        Summarize current thread\n",
                "  /suggest          Suggest next steps\n",
                "\n",
                "  /quit             Exit",
            ))),

            "ping" => Ok(SubmissionResult::response("pong!")),

            "version" => Ok(SubmissionResult::response(format!(
                "{} v{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ))),

            "tools" => {
                let tools = self.tools().list().await;
                Ok(SubmissionResult::response(format!(
                    "Available tools: {}",
                    tools.join(", ")
                )))
            }

            "debug" => {
                // Debug toggle is handled client-side in the REPL.
                // For non-REPL channels, just acknowledge.
                Ok(SubmissionResult::ok_with_message(
                    "Debug toggle is handled by your client.",
                ))
            }

            "model" => {
                if args.is_empty() {
                    // Show current model
                    let name = self.llm().active_model_name();
                    Ok(SubmissionResult::response(format!(
                        "Active model: {}",
                        name
                    )))
                } else {
                    let requested = &args[0];

                    // Validate the model exists
                    match self.llm().list_models().await {
                        Ok(models) if !models.is_empty() => {
                            if !models.iter().any(|m| m == requested) {
                                return Ok(SubmissionResult::error(format!(
                                    "Unknown model: {}. Available models:\n  {}",
                                    requested,
                                    models.join("\n  ")
                                )));
                            }
                        }
                        Ok(_) => {
                            // Empty model list, can't validate but try anyway
                        }
                        Err(e) => {
                            tracing::warn!("Could not fetch model list for validation: {}", e);
                            // Proceed anyway, the provider will error on the next call if invalid
                        }
                    }

                    match self.llm().set_model(requested) {
                        Ok(()) => Ok(SubmissionResult::response(format!(
                            "Switched model to: {}",
                            requested
                        ))),
                        Err(e) => Ok(SubmissionResult::error(format!(
                            "Failed to switch model: {}",
                            e
                        ))),
                    }
                }
            }

            _ => Ok(SubmissionResult::error(format!(
                "Unknown command: {}. Try /help",
                command
            ))),
        }
    }

    /// Handle legacy command routing from the Router (job commands that go through
    /// process_user_input -> router -> handle_job_or_command -> here).
    pub(super) async fn handle_command(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<Option<String>, Error> {
        // System commands are now handled directly via Submission::SystemCommand,
        // but the router may still send us unknown /commands.
        match self.handle_system_command(command, args).await? {
            SubmissionResult::Response { content } => Ok(Some(content)),
            SubmissionResult::Ok { message } => Ok(message),
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            _ => Ok(None),
        }
    }
}
