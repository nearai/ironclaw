//! Main agent loop.

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::compaction::ContextCompactor;
use crate::agent::context_monitor::ContextMonitor;
use crate::agent::heartbeat::spawn_heartbeat;
use crate::agent::self_repair::DefaultSelfRepair;
use crate::agent::session::{Session, ThreadState};
use crate::agent::session_manager::SessionManager;
use crate::agent::submission::{Submission, SubmissionParser, SubmissionResult};
use crate::agent::{
    HeartbeatConfig as AgentHeartbeatConfig, MessageIntent, RepairTask, Router, Scheduler,
};
use crate::channels::{ChannelManager, IncomingMessage, OutgoingResponse, StatusUpdate};
use crate::config::{AgentConfig, HeartbeatConfig};
use crate::context::ContextManager;
use crate::error::Error;
use crate::history::Store;
use crate::llm::{LlmProvider, Reasoning, ReasoningContext};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

/// The main agent that coordinates all components.
pub struct Agent {
    config: AgentConfig,
    store: Option<Arc<Store>>,
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    channels: ChannelManager,
    context_manager: Arc<ContextManager>,
    scheduler: Arc<Scheduler>,
    router: Router,
    session_manager: Arc<SessionManager>,
    context_monitor: ContextMonitor,
    workspace: Option<Arc<Workspace>>,
    heartbeat_config: Option<HeartbeatConfig>,
}

impl Agent {
    /// Create a new agent.
    pub fn new(
        config: AgentConfig,
        store: Option<Arc<Store>>,
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
        channels: ChannelManager,
        workspace: Option<Arc<Workspace>>,
        heartbeat_config: Option<HeartbeatConfig>,
    ) -> Self {
        let context_manager = Arc::new(ContextManager::new(config.max_parallel_jobs));

        let scheduler = Arc::new(Scheduler::new(
            config.clone(),
            context_manager.clone(),
            llm.clone(),
            safety.clone(),
            tools.clone(),
            store.clone(),
        ));

        Self {
            config,
            store,
            llm,
            safety,
            tools,
            channels,
            context_manager,
            scheduler,
            router: Router::new(),
            session_manager: Arc::new(SessionManager::new()),
            context_monitor: ContextMonitor::new(),
            workspace,
            heartbeat_config,
        }
    }

    /// Run the agent main loop.
    pub async fn run(self) -> Result<(), Error> {
        // Start channels
        let mut message_stream = self.channels.start_all().await?;

        // Start self-repair task
        let repair = Arc::new(DefaultSelfRepair::new(
            self.context_manager.clone(),
            self.config.stuck_threshold,
            self.config.max_repair_attempts,
        ));
        let repair_task = RepairTask::new(repair, self.config.repair_check_interval);

        let repair_handle = tokio::spawn(async move {
            repair_task.run().await;
        });

        // Spawn heartbeat if enabled
        let heartbeat_handle = if let Some(ref hb_config) = self.heartbeat_config {
            if hb_config.enabled {
                if let Some(ref workspace) = self.workspace {
                    let config = AgentHeartbeatConfig::default()
                        .with_interval(std::time::Duration::from_secs(hb_config.interval_secs));

                    // Set up notification channel if configured
                    let (notify_tx, mut notify_rx) =
                        tokio::sync::mpsc::channel::<OutgoingResponse>(16);

                    // Spawn notification forwarder
                    // We can't clone ChannelManager directly, so we just log the notifications
                    // The heartbeat system will handle notifications via the response_tx
                    let notify_channel = hb_config.notify_channel.clone();
                    let notify_user = hb_config.notify_user.clone();
                    tokio::spawn(async move {
                        while let Some(response) = notify_rx.recv().await {
                            if let (Some(ch), Some(user)) = (&notify_channel, &notify_user) {
                                // Log the heartbeat notification
                                // In a full implementation, we'd route this through a shared channel reference
                                tracing::info!(
                                    "Heartbeat notification for {}/{}: {}",
                                    ch,
                                    user,
                                    &response.content
                                );
                            }
                        }
                    });

                    tracing::info!(
                        "Heartbeat enabled with {}s interval",
                        hb_config.interval_secs
                    );
                    Some(spawn_heartbeat(
                        config,
                        workspace.clone(),
                        self.llm.clone(),
                        Some(notify_tx),
                    ))
                } else {
                    tracing::warn!("Heartbeat enabled but no workspace available");
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Main message loop
        tracing::info!("Agent {} ready and listening", self.config.name);

        while let Some(message) = message_stream.next().await {
            match self.handle_message(&message).await {
                Ok(Some(response)) => {
                    let _ = self
                        .channels
                        .respond(&message, OutgoingResponse::text(response))
                        .await;
                }
                Ok(None) => {
                    // Shutdown signal received
                    tracing::info!("Shutdown signal received, exiting...");
                    break;
                }
                Err(e) => {
                    tracing::error!("Error handling message: {}", e);
                    let _ = self
                        .channels
                        .respond(&message, OutgoingResponse::text(format!("Error: {}", e)))
                        .await;
                }
            }
        }

        // Cleanup
        tracing::info!("Agent shutting down...");
        repair_handle.abort();
        if let Some(handle) = heartbeat_handle {
            handle.abort();
        }
        self.scheduler.stop_all().await;
        self.channels.shutdown_all().await?;

        Ok(())
    }

    async fn handle_message(&self, message: &IncomingMessage) -> Result<Option<String>, Error> {
        tracing::debug!(
            "Received message from {} on {}: {}",
            message.user_id,
            message.channel,
            truncate(&message.content, 100)
        );

        // Parse submission type first
        let submission = SubmissionParser::parse(&message.content);

        // Resolve session and thread
        let (session, thread_id) = self
            .session_manager
            .resolve_thread(
                &message.user_id,
                &message.channel,
                message.thread_id.as_deref(),
            )
            .await;

        // Process based on submission type
        let result = match submission {
            Submission::UserInput { content } => {
                self.process_user_input(message, session, thread_id, &content)
                    .await
            }
            Submission::Undo => self.process_undo(session, thread_id).await,
            Submission::Redo => self.process_redo(session, thread_id).await,
            Submission::Interrupt => self.process_interrupt(session, thread_id).await,
            Submission::Compact => self.process_compact(session, thread_id).await,
            Submission::Clear => self.process_clear(session, thread_id).await,
            Submission::NewThread => self.process_new_thread(message).await,
            Submission::SwitchThread { thread_id: target } => {
                self.process_switch_thread(message, target).await
            }
            Submission::Resume { checkpoint_id } => {
                self.process_resume(session, thread_id, checkpoint_id).await
            }
            Submission::ExecApproval { .. } => {
                // Not supported in simple chat flow
                Ok(SubmissionResult::error(
                    "Approval flow not supported in this context",
                ))
            }
        };

        // Convert SubmissionResult to response string
        match result? {
            SubmissionResult::Response { content } => Ok(Some(content)),
            SubmissionResult::Ok { message } => Ok(message),
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            SubmissionResult::Interrupted => Ok(Some("Interrupted.".into())),
            SubmissionResult::NeedApproval { .. } => {
                Ok(Some("Approval required but not supported.".into()))
            }
        }
    }

    async fn process_user_input(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        content: &str,
    ) -> Result<SubmissionResult, Error> {
        // First check thread state without holding lock during I/O
        let thread_state = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.state
        };

        // Check thread state
        match thread_state {
            ThreadState::Processing => {
                return Ok(SubmissionResult::error(
                    "Turn in progress. Use /interrupt to cancel.",
                ));
            }
            ThreadState::AwaitingApproval => {
                return Ok(SubmissionResult::error(
                    "Waiting for approval. Use /interrupt to cancel.",
                ));
            }
            ThreadState::Completed => {
                return Ok(SubmissionResult::error(
                    "Thread completed. Use /thread new.",
                ));
            }
            ThreadState::Idle | ThreadState::Interrupted => {
                // Can proceed
            }
        }

        // Route for job commands (bypass turn system)
        // Build a temporary message with the content to route
        let temp_message = IncomingMessage {
            content: content.to_string(),
            ..message.clone()
        };
        let intent = self.router.route(&temp_message);
        match &intent {
            MessageIntent::CreateJob { .. }
            | MessageIntent::CheckJobStatus { .. }
            | MessageIntent::CancelJob { .. }
            | MessageIntent::ListJobs { .. }
            | MessageIntent::HelpJob { .. }
            | MessageIntent::Command { .. } => {
                return self.handle_job_or_command(intent, message).await;
            }
            _ => {}
        }

        // Auto-compact if needed BEFORE adding new turn
        {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            let messages = thread.messages();
            if let Some(strategy) = self.context_monitor.suggest_compaction(&messages) {
                tracing::info!(
                    "Context at {:.1}% capacity, auto-compacting",
                    self.context_monitor.usage_percent(&messages)
                );
                let compactor = ContextCompactor::new(self.llm.clone());
                if let Err(e) = compactor
                    .compact(thread, strategy, self.workspace.as_deref())
                    .await
                {
                    tracing::warn!("Auto-compaction failed: {}", e);
                }
            }
        }

        // Create checkpoint before turn
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            let mut mgr = undo_mgr.lock().await;
            mgr.checkpoint(
                thread.turn_number(),
                thread.messages(),
                format!("Before turn {}", thread.turn_number()),
            );
        }

        // Start the turn and get messages
        let turn_messages = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.start_turn(content);
            thread.messages()
        };

        // Send thinking status
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Thinking("Processing...".into()),
            )
            .await;

        // Call LLM with thread context and available tools
        let reasoning = Reasoning::new(self.llm.clone(), self.safety.clone());
        let tool_defs = self.tools.tool_definitions().await;
        let context = ReasoningContext::new()
            .with_messages(turn_messages)
            .with_tools(tool_defs);
        let llm_result = reasoning.respond(&context).await;

        // Re-acquire lock and check if interrupted
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        if thread.state == ThreadState::Interrupted {
            let _ = self
                .channels
                .send_status(&message.channel, StatusUpdate::Status("Interrupted".into()))
                .await;
            return Ok(SubmissionResult::Interrupted);
        }

        // Complete or fail the turn
        match llm_result {
            Ok(response) => {
                thread.complete_turn(&response);
                let _ = self
                    .channels
                    .send_status(&message.channel, StatusUpdate::Status("Done".into()))
                    .await;
                Ok(SubmissionResult::response(response))
            }
            Err(e) => {
                thread.fail_turn(e.to_string());
                Ok(SubmissionResult::error(e.to_string()))
            }
        }
    }

    /// Handle job-related intents without turn tracking.
    async fn handle_job_or_command(
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
                )
                .await;
        }

        let response = match intent {
            MessageIntent::CreateJob {
                title,
                description,
                category,
            } => self.handle_create_job(title, description, category).await?,
            MessageIntent::CheckJobStatus { job_id } => self.handle_check_status(job_id).await?,
            MessageIntent::CancelJob { job_id } => self.handle_cancel_job(&job_id).await?,
            MessageIntent::ListJobs { filter } => self.handle_list_jobs(filter).await?,
            MessageIntent::HelpJob { job_id } => self.handle_help_job(&job_id).await?,
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

    async fn process_undo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        if !mgr.can_undo() {
            return Ok(SubmissionResult::ok_with_message("Nothing to undo."));
        }

        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        // Save current state to redo, get previous checkpoint
        let current_messages = thread.messages();
        let current_turn = thread.turn_number();

        if let Some(checkpoint) = mgr.undo(current_turn, current_messages) {
            // Extract values before consuming the reference
            let turn_number = checkpoint.turn_number;
            let messages = checkpoint.messages.clone();
            let undo_count = mgr.undo_count();
            // Restore thread from checkpoint
            thread.restore_from_messages(messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Undone to turn {}. {} undo(s) remaining.",
                turn_number, undo_count
            )))
        } else {
            Ok(SubmissionResult::error("Undo failed."))
        }
    }

    async fn process_redo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        if !mgr.can_redo() {
            return Ok(SubmissionResult::ok_with_message("Nothing to redo."));
        }

        if let Some(checkpoint) = mgr.redo() {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.restore_from_messages(checkpoint.messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Redone to turn {}.",
                checkpoint.turn_number
            )))
        } else {
            Ok(SubmissionResult::error("Redo failed."))
        }
    }

    async fn process_interrupt(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        match thread.state {
            ThreadState::Processing | ThreadState::AwaitingApproval => {
                thread.interrupt();
                Ok(SubmissionResult::ok_with_message("Interrupted."))
            }
            _ => Ok(SubmissionResult::ok_with_message("Nothing to interrupt.")),
        }
    }

    async fn process_compact(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let messages = thread.messages();
        let usage = self.context_monitor.usage_percent(&messages);
        let strategy = self
            .context_monitor
            .suggest_compaction(&messages)
            .unwrap_or(
                crate::agent::context_monitor::CompactionStrategy::Summarize { keep_recent: 5 },
            );

        let compactor = ContextCompactor::new(self.llm.clone());
        match compactor
            .compact(thread, strategy, self.workspace.as_deref())
            .await
        {
            Ok(result) => {
                let mut msg = format!(
                    "Compacted: {} turns removed, {} → {} tokens (was {:.1}% full)",
                    result.turns_removed, result.tokens_before, result.tokens_after, usage
                );
                if result.summary_written {
                    msg.push_str(", summary saved to workspace");
                }
                Ok(SubmissionResult::ok_with_message(msg))
            }
            Err(e) => Ok(SubmissionResult::error(format!("Compaction failed: {}", e))),
        }
    }

    async fn process_clear(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
        thread.turns.clear();
        thread.state = ThreadState::Idle;

        // Clear undo history too
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        undo_mgr.lock().await.clear();

        Ok(SubmissionResult::ok_with_message("Thread cleared."))
    }

    async fn process_new_thread(
        &self,
        message: &IncomingMessage,
    ) -> Result<SubmissionResult, Error> {
        let session = self
            .session_manager
            .get_or_create_session(&message.user_id)
            .await;
        let mut sess = session.lock().await;
        let thread = sess.create_thread();
        let thread_id = thread.id;
        Ok(SubmissionResult::ok_with_message(format!(
            "New thread: {}",
            thread_id
        )))
    }

    async fn process_switch_thread(
        &self,
        message: &IncomingMessage,
        target_thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let session = self
            .session_manager
            .get_or_create_session(&message.user_id)
            .await;
        let mut sess = session.lock().await;

        if sess.switch_thread(target_thread_id) {
            Ok(SubmissionResult::ok_with_message(format!(
                "Switched to thread {}",
                target_thread_id
            )))
        } else {
            Ok(SubmissionResult::error("Thread not found."))
        }
    }

    async fn process_resume(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        checkpoint_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        if let Some(checkpoint) = mgr.restore(checkpoint_id) {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.restore_from_messages(checkpoint.messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Resumed from checkpoint: {}",
                checkpoint.description
            )))
        } else {
            Ok(SubmissionResult::error("Checkpoint not found."))
        }
    }

    async fn handle_create_job(
        &self,
        title: String,
        description: String,
        category: Option<String>,
    ) -> Result<String, Error> {
        // Create job context
        let job_id = self
            .context_manager
            .create_job(&title, &description)
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
        if let Some(ref store) = self.store {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await {
                let store = store.clone();
                tokio::spawn(async move {
                    if let Err(e) = store.save_job(&ctx).await {
                        tracing::warn!("Failed to persist new job {}: {}", job_id, e);
                    }
                });
            }
        }

        // Schedule for execution
        self.scheduler.schedule(job_id).await?;

        Ok(format!(
            "Created job: {}\nID: {}\n\nThe job has been scheduled and is now running.",
            title, job_id
        ))
    }

    async fn handle_check_status(&self, job_id: Option<String>) -> Result<String, Error> {
        match job_id {
            Some(id) => {
                let uuid = Uuid::parse_str(&id)
                    .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

                let ctx = self.context_manager.get_context(uuid).await?;

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
                let summary = self.context_manager.summary().await;
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

    async fn handle_cancel_job(&self, job_id: &str) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        self.scheduler.stop(uuid).await?;

        Ok(format!("Job {} has been cancelled.", job_id))
    }

    async fn handle_list_jobs(&self, _filter: Option<String>) -> Result<String, Error> {
        let jobs = self.context_manager.all_jobs().await;

        if jobs.is_empty() {
            return Ok("No jobs found.".to_string());
        }

        let mut output = String::from("Jobs:\n");
        for job_id in jobs {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await {
                output.push_str(&format!("  {} - {} ({:?})\n", job_id, ctx.title, ctx.state));
            }
        }

        Ok(output)
    }

    async fn handle_help_job(&self, job_id: &str) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        let ctx = self.context_manager.get_context(uuid).await?;

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

    async fn handle_command(
        &self,
        command: &str,
        _args: &[String],
    ) -> Result<Option<String>, Error> {
        match command {
            "help" => Ok(Some(
                r#"Commands:
  /job <desc>     - Create a job
  /status [id]    - Check job status
  /cancel <id>    - Cancel a job
  /list           - List all jobs
  /help <job_id>  - Help a stuck job

  /undo           - Undo last turn
  /redo           - Redo undone turn
  /compact        - Compress context
  /clear          - Clear thread
  /interrupt      - Stop current turn
  /thread new     - New thread
  /thread <id>    - Switch thread
  /resume <id>    - Resume checkpoint

  /quit           - Exit"#
                    .to_string(),
            )),

            "ping" => Ok(Some("pong!".to_string())),

            "version" => Ok(Some(format!(
                "{} v{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ))),

            "tools" => {
                let tools = self.tools.list().await;
                Ok(Some(format!("Available tools: {}", tools.join(", "))))
            }

            "quit" | "exit" | "shutdown" => {
                // Signal shutdown - return None to indicate no response needed
                Ok(None)
            }

            _ => Ok(Some(format!("Unknown command: {}. Try /help", command))),
        }
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}
