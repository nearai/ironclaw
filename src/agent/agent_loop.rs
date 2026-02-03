//! Main agent loop.

use std::sync::Arc;

use futures::StreamExt;
use uuid::Uuid;

use crate::agent::self_repair::DefaultSelfRepair;
use crate::agent::{MessageIntent, RepairTask, Router, Scheduler};
use crate::channels::{ChannelManager, IncomingMessage, OutgoingResponse};
use crate::config::AgentConfig;
use crate::context::ContextManager;
use crate::error::Error;
use crate::history::Store;
use crate::llm::{ChatMessage, LlmProvider, Reasoning, ReasoningContext};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;

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

        // Main message loop
        tracing::info!("Agent {} ready and listening", self.config.name);

        while let Some(message) = message_stream.next().await {
            if let Err(e) = self.handle_message(&message).await {
                tracing::error!("Error handling message: {}", e);

                // Try to send error response
                let _ = self
                    .channels
                    .respond(&message, OutgoingResponse::text(format!("Error: {}", e)))
                    .await;
            }
        }

        // Cleanup
        tracing::info!("Agent shutting down...");
        repair_handle.abort();
        self.scheduler.stop_all().await;
        self.channels.shutdown_all().await?;

        Ok(())
    }

    async fn handle_message(&self, message: &IncomingMessage) -> Result<(), Error> {
        tracing::debug!(
            "Received message from {} on {}: {}",
            message.user_id,
            message.channel,
            truncate(&message.content, 100)
        );

        // Route the message
        let intent = self.router.route(message);
        tracing::debug!("Routed to intent: {:?}", intent);

        // Handle based on intent
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

            MessageIntent::Chat { content } => self.handle_chat(message, &content).await?,

            MessageIntent::Command { command, args } => {
                self.handle_command(&command, &args).await?
            }

            MessageIntent::Unknown => {
                "I'm not sure what you're asking. Try '/help' for available commands.".to_string()
            }
        };

        // Send response
        self.channels
            .respond(message, OutgoingResponse::text(response))
            .await?;

        Ok(())
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

    async fn handle_chat(
        &self,
        _message: &IncomingMessage,
        content: &str,
    ) -> Result<String, Error> {
        // Use LLM for general chat
        let reasoning = Reasoning::new(self.llm.clone(), self.safety.clone());

        let context = ReasoningContext::new().with_message(ChatMessage::user(content));

        let response = reasoning.respond(&context).await?;

        Ok(response)
    }

    async fn handle_command(&self, command: &str, _args: &[String]) -> Result<String, Error> {
        match command {
            "help" => Ok(r#"Available commands:
  /job <description>  - Create a new job
  /status [job_id]    - Check job status
  /cancel <job_id>    - Cancel a job
  /list               - List all jobs
  /help <job_id>      - Help a stuck job

Or just chat naturally and I'll try to understand what you need!"#
                .to_string()),

            "ping" => Ok("pong!".to_string()),

            "version" => Ok(format!(
                "{} v{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            )),

            "tools" => {
                let tools = self.tools.list().await;
                Ok(format!("Available tools: {}", tools.join(", ")))
            }

            _ => Ok(format!("Unknown command: {}. Try /help", command)),
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
