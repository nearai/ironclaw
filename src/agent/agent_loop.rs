//! Main agent loop.

use std::sync::Arc;

/// Escape special characters for Telegram's legacy Markdown.
///
/// In Telegram's Markdown mode, these characters have special meaning:
/// - `_` starts/ends italic
/// - `*` starts/ends bold
/// - `` ` `` starts/ends code
/// - `[` starts a link
///
/// We escape them with backslash so dynamic content doesn't break formatting.
fn escape_telegram_markdown(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '_' | '*' | '`' | '[' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result
}

use futures::StreamExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::compaction::ContextCompactor;
use crate::agent::context_monitor::ContextMonitor;
use crate::agent::heartbeat::spawn_heartbeat;
use crate::agent::self_repair::DefaultSelfRepair;
use crate::agent::session::{PendingApproval, Session, ThreadState};
use crate::agent::session_manager::SessionManager;
use crate::agent::submission::{Submission, SubmissionParser, SubmissionResult};
use crate::agent::{
    HeartbeatConfig as AgentHeartbeatConfig, MessageIntent, RepairTask, Router, Scheduler,
};
use crate::channels::{ChannelManager, IncomingMessage, OutgoingResponse, StatusUpdate};
use crate::config::{AgentConfig, HeartbeatConfig};
use crate::context::ContextManager;
use crate::context::JobContext;
use crate::error::Error;
use crate::history::Store;
use crate::llm::{ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

/// Result of the agentic loop execution.
enum AgenticLoopResult {
    /// Completed with a response.
    Response(String),
    /// A tool requires approval before continuing.
    NeedApproval {
        /// The pending approval request to store.
        pending: PendingApproval,
    },
}

/// Core dependencies for the agent.
///
/// Bundles the shared components to reduce argument count.
pub struct AgentDeps {
    pub store: Option<Arc<Store>>,
    pub llm: Arc<dyn LlmProvider>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub workspace: Option<Arc<Workspace>>,
}

/// The main agent that coordinates all components.
pub struct Agent {
    config: AgentConfig,
    deps: AgentDeps,
    channels: Arc<ChannelManager>,
    context_manager: Arc<ContextManager>,
    scheduler: Arc<Scheduler>,
    router: Router,
    session_manager: Arc<SessionManager>,
    context_monitor: ContextMonitor,
    heartbeat_config: Option<HeartbeatConfig>,
}

impl Agent {
    /// Create a new agent.
    ///
    /// Optionally accepts a pre-created `ContextManager` for sharing with job tools.
    /// If not provided, creates a new one.
    pub fn new(
        config: AgentConfig,
        deps: AgentDeps,
        channels: ChannelManager,
        heartbeat_config: Option<HeartbeatConfig>,
        context_manager: Option<Arc<ContextManager>>,
    ) -> Self {
        let context_manager = context_manager
            .unwrap_or_else(|| Arc::new(ContextManager::new(config.max_parallel_jobs)));

        let scheduler = Arc::new(Scheduler::new(
            config.clone(),
            context_manager.clone(),
            deps.llm.clone(),
            deps.safety.clone(),
            deps.tools.clone(),
            deps.store.clone(),
        ));

        Self {
            config,
            deps,
            channels: Arc::new(channels),
            context_manager,
            scheduler,
            router: Router::new(),
            session_manager: Arc::new(SessionManager::new()),
            context_monitor: ContextMonitor::new(),
            heartbeat_config,
        }
    }

    // Convenience accessors
    fn store(&self) -> Option<&Arc<Store>> {
        self.deps.store.as_ref()
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

    fn workspace(&self) -> Option<&Arc<Workspace>> {
        self.deps.workspace.as_ref()
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
                if let Some(workspace) = self.workspace() {
                    let config = AgentHeartbeatConfig::default()
                        .with_interval(std::time::Duration::from_secs(hb_config.interval_secs));

                    // Set up notification channel
                    let (notify_tx, mut notify_rx) =
                        tokio::sync::mpsc::channel::<OutgoingResponse>(16);

                    // Spawn notification forwarder that routes through channel manager
                    let notify_channel = hb_config.notify_channel.clone();
                    let notify_user = hb_config.notify_user.clone();
                    let channels = self.channels.clone();
                    tokio::spawn(async move {
                        while let Some(response) = notify_rx.recv().await {
                            // Route notification to configured channel/user, or broadcast to all
                            match (&notify_channel, &notify_user) {
                                (Some(channel), Some(user)) => {
                                    // Send to specific channel and user
                                    if let Err(e) =
                                        channels.broadcast(channel, user, response.clone()).await
                                    {
                                        tracing::warn!(
                                            "Failed to send heartbeat to {}/{}: {}",
                                            channel,
                                            user,
                                            e
                                        );
                                    } else {
                                        tracing::debug!(
                                            "Heartbeat notification sent to {}/{}",
                                            channel,
                                            user
                                        );
                                    }
                                }
                                (None, Some(user)) => {
                                    // Broadcast to all channels for this user
                                    let results = channels.broadcast_all(user, response).await;
                                    for (ch, result) in results {
                                        if let Err(e) = result {
                                            tracing::warn!(
                                                "Failed to broadcast heartbeat to {}: {}",
                                                ch,
                                                e
                                            );
                                        }
                                    }
                                }
                                _ => {
                                    // No target configured, just log
                                    tracing::info!(
                                        "Heartbeat notification (no target configured): {}",
                                        &response.content
                                    );
                                }
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
                        self.llm().clone(),
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
            Submission::ExecApproval {
                request_id,
                approved,
                always,
            } => {
                self.process_approval(
                    message,
                    session,
                    thread_id,
                    Some(request_id),
                    approved,
                    always,
                )
                .await
            }
            Submission::ApprovalResponse { approved, always } => {
                self.process_approval(message, session, thread_id, None, approved, always)
                    .await
            }
        };

        // Convert SubmissionResult to response string
        match result? {
            SubmissionResult::Response { content } => Ok(Some(content)),
            SubmissionResult::Ok { message } => Ok(message),
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            SubmissionResult::Interrupted => Ok(Some("Interrupted.".into())),
            SubmissionResult::NeedApproval {
                request_id,
                tool_name,
                description,
                parameters,
            } => {
                // Format approval request for user
                let params_preview = serde_json::to_string_pretty(&parameters)
                    .unwrap_or_else(|_| parameters.to_string());
                let params_truncated = if params_preview.chars().count() > 200 {
                    format!(
                        "{}...",
                        params_preview.chars().take(200).collect::<String>()
                    )
                } else {
                    params_preview
                };
                // Escape Markdown special chars in dynamic values to avoid breaking
                // Telegram's Markdown parser (underscores, asterisks, backticks, brackets)
                let tool_name_escaped = escape_telegram_markdown(&tool_name);
                let description_escaped = escape_telegram_markdown(&description);
                // Params go inside a code block, so no escaping needed there
                Ok(Some(format!(
                    "🔒 Tool requires approval:\n\n\
                     *Tool:* {}\n\
                     *Description:* {}\n\
                     *Parameters:*\n```\n{}\n```\n\n\
                     Reply with:\n\
                     • yes or approve to allow this tool\n\
                     • always to always allow this tool in this session\n\
                     • no or deny to reject\n\n\
                     Request ID: {}",
                    tool_name_escaped, description_escaped, params_truncated, request_id
                )))
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

        // Handle explicit commands (starting with /) directly
        // Everything else goes through the normal agentic loop with tools
        let temp_message = IncomingMessage {
            content: content.to_string(),
            ..message.clone()
        };

        if let Some(intent) = self.router.route_command(&temp_message) {
            // Explicit command like /status, /job, /list - handle directly
            return self.handle_job_or_command(intent, message).await;
        }

        // Natural language goes through the agentic loop
        // Job tools (create_job, list_jobs, etc.) are in the tool registry

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
                let compactor = ContextCompactor::new(self.llm().clone());
                if let Err(e) = compactor
                    .compact(thread, strategy, self.workspace().map(|w| w.as_ref()))
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

        // Run the agentic tool execution loop
        let result = self
            .run_agentic_loop(message, session.clone(), thread_id, turn_messages)
            .await;

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

        // Complete, fail, or request approval
        match result {
            Ok(AgenticLoopResult::Response(response)) => {
                thread.complete_turn(&response);
                let _ = self
                    .channels
                    .send_status(&message.channel, StatusUpdate::Status("Done".into()))
                    .await;
                Ok(SubmissionResult::response(response))
            }
            Ok(AgenticLoopResult::NeedApproval { pending }) => {
                // Store pending approval in thread and update state
                let request_id = pending.request_id;
                let tool_name = pending.tool_name.clone();
                let description = pending.description.clone();
                let parameters = pending.parameters.clone();
                thread.await_approval(pending);
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Awaiting approval".into()),
                    )
                    .await;
                Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                })
            }
            Err(e) => {
                thread.fail_turn(e.to_string());
                Ok(SubmissionResult::error(e.to_string()))
            }
        }
    }

    /// Run the agentic loop: call LLM, execute tools, repeat until text response.
    ///
    /// Returns `AgenticLoopResult::Response` on completion, or
    /// `AgenticLoopResult::NeedApproval` if a tool requires user approval.
    async fn run_agentic_loop(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        initial_messages: Vec<ChatMessage>,
    ) -> Result<AgenticLoopResult, Error> {
        // Load workspace system prompt (identity files: AGENTS.md, SOUL.md, etc.)
        let system_prompt = if let Some(ws) = self.workspace() {
            match ws.system_prompt().await {
                Ok(prompt) if !prompt.is_empty() => Some(prompt),
                Ok(_) => None,
                Err(e) => {
                    tracing::debug!("Could not load workspace system prompt: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let mut reasoning = Reasoning::new(self.llm().clone(), self.safety().clone());
        if let Some(prompt) = system_prompt {
            reasoning = reasoning.with_system_prompt(prompt);
        }

        // Build context with messages that we'll mutate during the loop
        let mut context_messages = initial_messages;

        // Create a JobContext for tool execution (chat doesn't have a real job)
        let job_ctx = JobContext::with_user(&message.user_id, "chat", "Interactive chat session");

        const MAX_TOOL_ITERATIONS: usize = 10;
        let mut iteration = 0;
        let mut tools_executed = false;

        loop {
            iteration += 1;
            if iteration > MAX_TOOL_ITERATIONS {
                return Err(crate::error::LlmError::InvalidResponse {
                    provider: "nearai".to_string(),
                    reason: format!("Exceeded maximum tool iterations ({})", MAX_TOOL_ITERATIONS),
                }
                .into());
            }

            // Check if interrupted
            {
                let sess = session.lock().await;
                if let Some(thread) = sess.threads.get(&thread_id) {
                    if thread.state == ThreadState::Interrupted {
                        return Err(crate::error::JobError::ContextError {
                            id: thread_id,
                            reason: "Interrupted".to_string(),
                        }
                        .into());
                    }
                }
            }

            // Refresh tool definitions each iteration so newly built tools become visible
            let tool_defs = self.tools().tool_definitions().await;

            // Call LLM with current context
            let context = ReasoningContext::new()
                .with_messages(context_messages.clone())
                .with_tools(tool_defs);

            let result = reasoning.respond_with_tools(&context).await?;

            match result {
                RespondResult::Text(text) => {
                    // If no tools have been executed yet, prompt the LLM to use tools
                    // This handles the case where the model explains what it will do
                    // instead of actually calling tools
                    if !tools_executed && iteration < 3 {
                        tracing::debug!(
                            "No tools executed yet (iteration {}), prompting for tool use",
                            iteration
                        );
                        context_messages.push(ChatMessage::assistant(&text));
                        context_messages.push(ChatMessage::user(
                            "Please proceed and use the available tools to complete this task.",
                        ));
                        continue;
                    }

                    // Tools have been executed or we've tried multiple times, return response
                    return Ok(AgenticLoopResult::Response(text));
                }
                RespondResult::ToolCalls(tool_calls) => {
                    tools_executed = true;
                    // Execute tools and add results to context
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Thinking(format!(
                                "Executing {} tool(s)...",
                                tool_calls.len()
                            )),
                        )
                        .await;

                    // Record tool calls in the thread
                    {
                        let mut sess = session.lock().await;
                        if let Some(thread) = sess.threads.get_mut(&thread_id) {
                            if let Some(turn) = thread.last_turn_mut() {
                                for tc in &tool_calls {
                                    turn.record_tool_call(&tc.name, tc.arguments.clone());
                                }
                            }
                        }
                    }

                    // Execute each tool (with approval checking)
                    for tc in tool_calls {
                        // Check if tool requires approval
                        if let Some(tool) = self.tools().get(&tc.name).await {
                            if tool.requires_approval() {
                                // Check if auto-approved for this session
                                let is_auto_approved = {
                                    let sess = session.lock().await;
                                    sess.is_tool_auto_approved(&tc.name)
                                };

                                if !is_auto_approved {
                                    // Need approval - store pending request and return
                                    let pending = PendingApproval {
                                        request_id: Uuid::new_v4(),
                                        tool_name: tc.name.clone(),
                                        parameters: tc.arguments.clone(),
                                        description: tool.description().to_string(),
                                        tool_call_id: tc.id.clone(),
                                        context_messages: context_messages.clone(),
                                    };

                                    return Ok(AgenticLoopResult::NeedApproval { pending });
                                }
                            }
                        }

                        let tool_result = self
                            .execute_chat_tool(&tc.name, &tc.arguments, &job_ctx)
                            .await;

                        // Record result in thread
                        {
                            let mut sess = session.lock().await;
                            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                                if let Some(turn) = thread.last_turn_mut() {
                                    match &tool_result {
                                        Ok(output) => {
                                            turn.record_tool_result(serde_json::json!(output));
                                        }
                                        Err(e) => {
                                            turn.record_tool_error(e.to_string());
                                        }
                                    }
                                }
                            }
                        }

                        // Add tool result to context for next LLM call
                        let result_content = match tool_result {
                            Ok(output) => {
                                // Sanitize output before showing to LLM
                                let sanitized =
                                    self.safety().sanitize_tool_output(&tc.name, &output);
                                self.safety().wrap_for_llm(
                                    &tc.name,
                                    &sanitized.content,
                                    sanitized.was_modified,
                                )
                            }
                            Err(e) => format!("Error: {}", e),
                        };

                        context_messages.push(ChatMessage::tool_result(
                            &tc.id,
                            &tc.name,
                            result_content,
                        ));
                    }
                }
            }
        }
    }

    /// Execute a tool for chat (without full job context).
    async fn execute_chat_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        job_ctx: &JobContext,
    ) -> Result<String, Error> {
        let tool =
            self.tools()
                .get(tool_name)
                .await
                .ok_or_else(|| crate::error::ToolError::NotFound {
                    name: tool_name.to_string(),
                })?;

        // Execute with timeout
        let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            tool.execute(params.clone(), job_ctx).await
        })
        .await
        .map_err(|_| crate::error::ToolError::Timeout {
            name: tool_name.to_string(),
            timeout: std::time::Duration::from_secs(60),
        })?
        .map_err(|e| crate::error::ToolError::ExecutionFailed {
            name: tool_name.to_string(),
            reason: e.to_string(),
        })?;

        // Convert result to string
        serde_json::to_string_pretty(&result.result).map_err(|e| {
            crate::error::ToolError::ExecutionFailed {
                name: tool_name.to_string(),
                reason: format!("Failed to serialize result: {}", e),
            }
            .into()
        })
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

        let compactor = ContextCompactor::new(self.llm().clone());
        match compactor
            .compact(thread, strategy, self.workspace().map(|w| w.as_ref()))
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

    /// Process an approval or rejection of a pending tool execution.
    async fn process_approval(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        request_id: Option<Uuid>,
        approved: bool,
        always: bool,
    ) -> Result<SubmissionResult, Error> {
        // Get thread state and pending approval
        let (_thread_state, pending) = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            if thread.state != ThreadState::AwaitingApproval {
                return Ok(SubmissionResult::error("No pending approval request."));
            }

            let pending = thread.take_pending_approval();
            (thread.state, pending)
        };

        let pending = match pending {
            Some(p) => p,
            None => return Ok(SubmissionResult::error("No pending approval request.")),
        };

        // Verify request ID if provided
        if let Some(req_id) = request_id {
            if req_id != pending.request_id {
                // Put it back and return error
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.await_approval(pending);
                }
                return Ok(SubmissionResult::error(
                    "Request ID mismatch. Use the correct request ID.",
                ));
            }
        }

        if approved {
            // If always, add to auto-approved set
            if always {
                let mut sess = session.lock().await;
                sess.auto_approve_tool(&pending.tool_name);
                tracing::info!(
                    "Auto-approved tool '{}' for session {}",
                    pending.tool_name,
                    sess.id
                );
            }

            // Reset thread state to processing
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.state = ThreadState::Processing;
                }
            }

            // Execute the approved tool and continue the loop
            let job_ctx =
                JobContext::with_user(&message.user_id, "chat", "Interactive chat session");

            let tool_result = self
                .execute_chat_tool(&pending.tool_name, &pending.parameters, &job_ctx)
                .await;

            // Build context including the tool result
            let mut context_messages = pending.context_messages;

            // Record result in thread
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    if let Some(turn) = thread.last_turn_mut() {
                        match &tool_result {
                            Ok(output) => {
                                turn.record_tool_result(serde_json::json!(output));
                            }
                            Err(e) => {
                                turn.record_tool_error(e.to_string());
                            }
                        }
                    }
                }
            }

            // Add tool result to context
            let result_content = match tool_result {
                Ok(output) => {
                    let sanitized = self
                        .safety()
                        .sanitize_tool_output(&pending.tool_name, &output);
                    self.safety().wrap_for_llm(
                        &pending.tool_name,
                        &sanitized.content,
                        sanitized.was_modified,
                    )
                }
                Err(e) => format!("Error: {}", e),
            };

            context_messages.push(ChatMessage::tool_result(
                &pending.tool_call_id,
                &pending.tool_name,
                result_content,
            ));

            // Continue the agentic loop
            let result = self
                .run_agentic_loop(message, session.clone(), thread_id, context_messages)
                .await;

            // Handle the result
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            match result {
                Ok(AgenticLoopResult::Response(response)) => {
                    thread.complete_turn(&response);
                    let _ = self
                        .channels
                        .send_status(&message.channel, StatusUpdate::Status("Done".into()))
                        .await;
                    Ok(SubmissionResult::response(response))
                }
                Ok(AgenticLoopResult::NeedApproval {
                    pending: new_pending,
                }) => {
                    let request_id = new_pending.request_id;
                    let tool_name = new_pending.tool_name.clone();
                    let description = new_pending.description.clone();
                    let parameters = new_pending.parameters.clone();
                    thread.await_approval(new_pending);
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Status("Awaiting approval".into()),
                        )
                        .await;
                    Ok(SubmissionResult::NeedApproval {
                        request_id,
                        tool_name,
                        description,
                        parameters,
                    })
                }
                Err(e) => {
                    thread.fail_turn(e.to_string());
                    Ok(SubmissionResult::error(e.to_string()))
                }
            }
        } else {
            // Rejected - clear approval and return to idle
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.clear_pending_approval();
                }
            }

            let _ = self
                .channels
                .send_status(&message.channel, StatusUpdate::Status("Rejected".into()))
                .await;

            Ok(SubmissionResult::response(format!(
                "Tool '{}' was rejected. The agent will not execute this tool.\n\n\
                 You can continue the conversation or try a different approach.",
                pending.tool_name
            )))
        }
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
        if let Some(store) = self.store() {
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
                let tools = self.tools().list().await;
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
