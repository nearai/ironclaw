//! Main agent loop.
//!
//! Contains the `Agent` struct, `AgentDeps`, and the core event loop (`run`).
//! The heavy lifting is delegated to sibling modules:
//!
//! - `dispatcher` - Tool dispatch (agentic loop, tool execution)
//! - `commands` - System commands and job handlers
//! - `thread_ops` - Thread/session operations (user input, undo, approval, persistence)

use std::sync::Arc;

use futures::StreamExt;

use crate::agent::context_monitor::ContextMonitor;
use crate::agent::heartbeat::spawn_heartbeat;
use crate::agent::routine_engine::{RoutineEngine, spawn_cron_ticker};
use crate::agent::self_repair::{DefaultSelfRepair, RepairResult, SelfRepair};
use crate::agent::session_manager::SessionManager;
use crate::agent::submission::{Submission, SubmissionParser, SubmissionResult};
use crate::agent::{HeartbeatConfig as AgentHeartbeatConfig, Router, Scheduler};
use crate::channels::{ChannelManager, IncomingMessage, OutgoingResponse, StatusUpdate};
use crate::config::{AgentConfig, HeartbeatConfig, RoutineConfig, SkillsConfig};
use crate::context::ContextManager;
use crate::db::Database;
use crate::error::Error;
use crate::extensions::ExtensionManager;
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::safety::SafetyLayer;
use crate::skills::SkillRegistry;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

/// Collapse a tool output string into a single-line preview for display.
pub(crate) fn truncate_for_preview(output: &str, max_chars: usize) -> String {
    let collapsed: String = output
        .chars()
        .take(max_chars + 50)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // char_indices gives us byte offsets at char boundaries, so the slice is always valid UTF-8.
    if collapsed.chars().count() > max_chars {
        let byte_offset = collapsed
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(collapsed.len());
        format!("{}...", &collapsed[..byte_offset])
    } else {
        collapsed
    }
}

/// Core dependencies for the agent.
///
/// Bundles the shared components to reduce argument count.
pub struct AgentDeps {
    pub store: Option<Arc<dyn Database>>,
    pub llm: Arc<dyn LlmProvider>,
    /// Cheap/fast LLM for lightweight tasks (heartbeat, routing, evaluation).
    /// Falls back to the main `llm` if None.
    pub cheap_llm: Option<Arc<dyn LlmProvider>>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub workspace: Option<Arc<Workspace>>,
    pub extension_manager: Option<Arc<ExtensionManager>>,
    pub skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
    pub skill_catalog: Option<Arc<crate::skills::catalog::SkillCatalog>>,
    pub skills_config: SkillsConfig,
    pub hooks: Arc<HookRegistry>,
    /// Cost enforcement guardrails (daily budget, hourly rate limits).
    pub cost_guard: Arc<crate::agent::cost_guard::CostGuard>,
    /// Observability backend for recording events and metrics.
    pub observer: Arc<dyn crate::observability::Observer>,
}

/// The main agent that coordinates all components.
pub struct Agent {
    pub(super) config: AgentConfig,
    pub(super) deps: AgentDeps,
    pub(super) channels: Arc<ChannelManager>,
    pub(super) context_manager: Arc<ContextManager>,
    pub(super) scheduler: Arc<Scheduler>,
    pub(super) router: Router,
    pub(super) session_manager: Arc<SessionManager>,
    pub(super) context_monitor: ContextMonitor,
    pub(super) heartbeat_config: Option<HeartbeatConfig>,
    pub(super) hygiene_config: Option<crate::config::HygieneConfig>,
    pub(super) routine_config: Option<RoutineConfig>,
}

impl Agent {
    /// Create a new agent.
    ///
    /// Optionally accepts pre-created `ContextManager` and `SessionManager` for sharing
    /// with external components (job tools, web gateway). Creates new ones if not provided.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: AgentConfig,
        deps: AgentDeps,
        channels: Arc<ChannelManager>,
        heartbeat_config: Option<HeartbeatConfig>,
        hygiene_config: Option<crate::config::HygieneConfig>,
        routine_config: Option<RoutineConfig>,
        context_manager: Option<Arc<ContextManager>>,
        session_manager: Option<Arc<SessionManager>>,
    ) -> Self {
        let context_manager = context_manager
            .unwrap_or_else(|| Arc::new(ContextManager::new(config.max_parallel_jobs)));

        let session_manager = session_manager.unwrap_or_else(|| Arc::new(SessionManager::new()));

        let scheduler = Arc::new(Scheduler::new(
            config.clone(),
            context_manager.clone(),
            deps.llm.clone(),
            deps.safety.clone(),
            deps.tools.clone(),
            deps.store.clone(),
            deps.hooks.clone(),
        ));

        Self {
            config,
            deps,
            channels,
            context_manager,
            scheduler,
            router: Router::new(),
            session_manager,
            context_monitor: ContextMonitor::new(),
            heartbeat_config,
            hygiene_config,
            routine_config,
        }
    }

    // Convenience accessors

    pub(super) fn store(&self) -> Option<&Arc<dyn Database>> {
        self.deps.store.as_ref()
    }

    pub(super) fn llm(&self) -> &Arc<dyn LlmProvider> {
        &self.deps.llm
    }

    /// Get the cheap/fast LLM provider, falling back to the main one.
    pub(super) fn cheap_llm(&self) -> &Arc<dyn LlmProvider> {
        self.deps.cheap_llm.as_ref().unwrap_or(&self.deps.llm)
    }

    pub(super) fn safety(&self) -> &Arc<SafetyLayer> {
        &self.deps.safety
    }

    pub(super) fn tools(&self) -> &Arc<ToolRegistry> {
        &self.deps.tools
    }

    pub(super) fn workspace(&self) -> Option<&Arc<Workspace>> {
        self.deps.workspace.as_ref()
    }

    pub(super) fn hooks(&self) -> &Arc<HookRegistry> {
        &self.deps.hooks
    }

    pub(super) fn cost_guard(&self) -> &Arc<crate::agent::cost_guard::CostGuard> {
        &self.deps.cost_guard
    }

    pub(super) fn observer(&self) -> &Arc<dyn crate::observability::Observer> {
        &self.deps.observer
    }

    pub(super) fn skill_registry(&self) -> Option<&Arc<std::sync::RwLock<SkillRegistry>>> {
        self.deps.skill_registry.as_ref()
    }

    pub(super) fn skill_catalog(&self) -> Option<&Arc<crate::skills::catalog::SkillCatalog>> {
        self.deps.skill_catalog.as_ref()
    }

    /// Select active skills for a message using deterministic prefiltering.
    pub(super) fn select_active_skills(
        &self,
        message_content: &str,
    ) -> Vec<crate::skills::LoadedSkill> {
        if let Some(registry) = self.skill_registry() {
            let guard = match registry.read() {
                Ok(g) => g,
                Err(e) => {
                    tracing::error!("Skill registry lock poisoned: {}", e);
                    return vec![];
                }
            };
            let available = guard.skills();
            let skills_cfg = &self.deps.skills_config;
            let selected = crate::skills::prefilter_skills(
                message_content,
                available,
                skills_cfg.max_active_skills,
                skills_cfg.max_context_tokens,
            );

            if !selected.is_empty() {
                tracing::debug!(
                    "Selected {} skill(s) for message: {}",
                    selected.len(),
                    selected
                        .iter()
                        .map(|s| s.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            selected.into_iter().cloned().collect()
        } else {
            vec![]
        }
    }

    /// Run the agent main loop.
    pub async fn run(self) -> Result<(), Error> {
        // Start channels
        let mut message_stream = self.channels.start_all().await?;

        // Start self-repair task with notification forwarding
        let repair = Arc::new(DefaultSelfRepair::new(
            self.context_manager.clone(),
            self.config.stuck_threshold,
            self.config.max_repair_attempts,
        ));
        let repair_interval = self.config.repair_check_interval;
        let repair_channels = self.channels.clone();
        let repair_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(repair_interval).await;

                // Check stuck jobs
                let stuck_jobs = repair.detect_stuck_jobs().await;
                for job in stuck_jobs {
                    tracing::info!("Attempting to repair stuck job {}", job.job_id);
                    let result = repair.repair_stuck_job(&job).await;
                    let notification = match &result {
                        Ok(RepairResult::Success { message }) => {
                            tracing::info!("Repair succeeded: {}", message);
                            Some(format!(
                                "Job {} was stuck for {}s, recovery succeeded: {}",
                                job.job_id,
                                job.stuck_duration.as_secs(),
                                message
                            ))
                        }
                        Ok(RepairResult::Failed { message }) => {
                            tracing::error!("Repair failed: {}", message);
                            Some(format!(
                                "Job {} was stuck for {}s, recovery failed permanently: {}",
                                job.job_id,
                                job.stuck_duration.as_secs(),
                                message
                            ))
                        }
                        Ok(RepairResult::ManualRequired { message }) => {
                            tracing::warn!("Manual intervention needed: {}", message);
                            Some(format!(
                                "Job {} needs manual intervention: {}",
                                job.job_id, message
                            ))
                        }
                        Ok(RepairResult::Retry { message }) => {
                            tracing::warn!("Repair needs retry: {}", message);
                            None // Don't spam the user on retries
                        }
                        Err(e) => {
                            tracing::error!("Repair error: {}", e);
                            None
                        }
                    };

                    if let Some(msg) = notification {
                        let response = OutgoingResponse::text(format!("Self-Repair: {}", msg));
                        let _ = repair_channels.broadcast_all("default", response).await;
                    }
                }

                // Check broken tools
                let broken_tools = repair.detect_broken_tools().await;
                for tool in broken_tools {
                    tracing::info!("Attempting to repair broken tool: {}", tool.name);
                    match repair.repair_broken_tool(&tool).await {
                        Ok(RepairResult::Success { message }) => {
                            let response = OutgoingResponse::text(format!(
                                "Self-Repair: Tool '{}' repaired: {}",
                                tool.name, message
                            ));
                            let _ = repair_channels.broadcast_all("default", response).await;
                        }
                        Ok(result) => {
                            tracing::info!("Tool repair result: {:?}", result);
                        }
                        Err(e) => {
                            tracing::error!("Tool repair error: {}", e);
                        }
                    }
                }
            }
        });

        // Spawn session pruning task
        let session_mgr = self.session_manager.clone();
        let session_idle_timeout = self.config.session_idle_timeout;
        let pruning_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(600)); // Every 10 min
            interval.tick().await; // Skip immediate first tick
            loop {
                interval.tick().await;
                session_mgr.prune_stale_sessions(session_idle_timeout).await;
            }
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
                            let user = notify_user.as_deref().unwrap_or("default");

                            // Try the configured channel first, fall back to
                            // broadcasting on all channels.
                            let targeted_ok = if let Some(ref channel) = notify_channel {
                                channels
                                    .broadcast(channel, user, response.clone())
                                    .await
                                    .is_ok()
                            } else {
                                false
                            };

                            if !targeted_ok {
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
                        }
                    });

                    let hygiene = self
                        .hygiene_config
                        .as_ref()
                        .map(|h| h.to_workspace_config())
                        .unwrap_or_default();

                    Some(spawn_heartbeat(
                        config,
                        hygiene,
                        workspace.clone(),
                        self.cheap_llm().clone(),
                        self.safety().clone(),
                        self.observer().clone(),
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

        // Spawn routine engine if enabled
        let routine_handle = if let Some(ref rt_config) = self.routine_config {
            if rt_config.enabled {
                if let (Some(store), Some(workspace)) = (self.store(), self.workspace()) {
                    // Set up notification channel (same pattern as heartbeat)
                    let (notify_tx, mut notify_rx) =
                        tokio::sync::mpsc::channel::<OutgoingResponse>(32);

                    let engine = Arc::new(RoutineEngine::new(
                        rt_config.clone(),
                        Arc::clone(store),
                        self.llm().clone(),
                        Arc::clone(workspace),
                        notify_tx,
                        Some(self.scheduler.clone()),
                    ));

                    // Register routine tools
                    self.deps
                        .tools
                        .register_routine_tools(Arc::clone(store), Arc::clone(&engine));

                    // Load initial event cache
                    engine.refresh_event_cache().await;

                    // Spawn notification forwarder
                    let channels = self.channels.clone();
                    tokio::spawn(async move {
                        while let Some(response) = notify_rx.recv().await {
                            let user = response
                                .metadata
                                .get("notify_user")
                                .and_then(|v| v.as_str())
                                .unwrap_or("default")
                                .to_string();
                            let results = channels.broadcast_all(&user, response).await;
                            for (ch, result) in results {
                                if let Err(e) = result {
                                    tracing::warn!(
                                        "Failed to broadcast routine notification to {}: {}",
                                        ch,
                                        e
                                    );
                                }
                            }
                        }
                    });

                    // Spawn cron ticker
                    let cron_interval =
                        std::time::Duration::from_secs(rt_config.cron_check_interval_secs);
                    let cron_handle = spawn_cron_ticker(Arc::clone(&engine), cron_interval);

                    // Store engine reference for event trigger checking in the
                    // message loop below. This is just an Arc::clone; safe and cheap.
                    let engine_ref = Arc::clone(&engine);

                    tracing::info!(
                        "Routines enabled: cron ticker every {}s, max {} concurrent",
                        rt_config.cron_check_interval_secs,
                        rt_config.max_concurrent_routines
                    );

                    Some((cron_handle, engine_ref))
                } else {
                    tracing::warn!("Routines enabled but store/workspace not available");
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Extract engine ref for use in message loop
        let routine_engine_for_loop = routine_handle.as_ref().map(|(_, e)| Arc::clone(e));

        // Main message loop
        tracing::info!("Agent {} ready and listening", self.config.name);

        loop {
            let message = tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Ctrl+C received, shutting down...");
                    break;
                }
                msg = message_stream.next() => {
                    match msg {
                        Some(m) => m,
                        None => {
                            tracing::info!("All channel streams ended, shutting down...");
                            break;
                        }
                    }
                }
            };

            // H8: Emit inbound channel message event.
            self.observer()
                .record_event(&crate::observability::ObserverEvent::ChannelMessage {
                    channel: message.channel.clone(),
                    direction: "inbound".to_string(),
                });

            match self.handle_message(&message).await {
                Ok(Some(response)) if !response.is_empty() => {
                    // Hook: BeforeOutbound — allow hooks to modify or suppress outbound
                    let event = crate::hooks::HookEvent::Outbound {
                        user_id: message.user_id.clone(),
                        channel: message.channel.clone(),
                        content: response.clone(),
                        thread_id: message.thread_id.clone(),
                    };
                    let send_result = match self.hooks().run(&event).await {
                        Err(err) => {
                            tracing::warn!("BeforeOutbound hook blocked response: {}", err);
                            None // Hook blocked — nothing sent
                        }
                        Ok(crate::hooks::HookOutcome::Continue {
                            modified: Some(new_content),
                        }) => Some(
                            self.channels
                                .respond(&message, OutgoingResponse::text(new_content))
                                .await,
                        ),
                        _ => Some(
                            self.channels
                                .respond(&message, OutgoingResponse::text(response))
                                .await,
                        ),
                    };
                    // H8: Emit outbound event only after response was actually sent.
                    match send_result {
                        Some(Ok(())) => {
                            self.observer().record_event(
                                &crate::observability::ObserverEvent::ChannelMessage {
                                    channel: message.channel.clone(),
                                    direction: "outbound".to_string(),
                                },
                            );
                        }
                        Some(Err(e)) => {
                            tracing::error!(
                                channel = %message.channel,
                                error = %e,
                                "Failed to send response to channel"
                            );
                        }
                        None => {} // Hook blocked, no event
                    }
                }
                Ok(Some(empty)) => {
                    // Empty response, nothing to send (e.g. approval handled via send_status)
                    tracing::debug!(
                        channel = %message.channel,
                        user = %message.user_id,
                        empty_len = empty.len(),
                        "Suppressed empty response (not sent to channel)"
                    );
                }
                Ok(None) => {
                    // Shutdown signal received (/quit, /exit, /shutdown)
                    tracing::info!("Shutdown command received, exiting...");
                    break;
                }
                Err(e) => {
                    tracing::error!("Error handling message: {}", e);
                    if let Err(send_err) = self
                        .channels
                        .respond(&message, OutgoingResponse::text(format!("Error: {}", e)))
                        .await
                    {
                        tracing::error!(
                            channel = %message.channel,
                            error = %send_err,
                            "Failed to send error response to channel"
                        );
                    }
                }
            }

            // Check event triggers (cheap in-memory regex, fires async if matched)
            if let Some(ref engine) = routine_engine_for_loop {
                let fired = engine.check_event_triggers(&message).await;
                if fired > 0 {
                    tracing::debug!("Fired {} event-triggered routines", fired);
                }
            }
        }

        // Cleanup
        tracing::info!("Agent shutting down...");
        repair_handle.abort();
        pruning_handle.abort();
        if let Some(handle) = heartbeat_handle {
            handle.abort();
        }
        if let Some((cron_handle, _)) = routine_handle {
            cron_handle.abort();
        }
        self.scheduler.stop_all().await;
        self.channels.shutdown_all().await?;
        // C5: Flush and release observer resources (OTEL batch exporter, etc.)
        self.observer().shutdown();

        Ok(())
    }

    async fn handle_message(&self, message: &IncomingMessage) -> Result<Option<String>, Error> {
        // Parse submission type first
        let mut submission = SubmissionParser::parse(&message.content);

        // Hook: BeforeInbound — allow hooks to modify or reject user input
        if let Submission::UserInput { ref content } = submission {
            let event = crate::hooks::HookEvent::Inbound {
                user_id: message.user_id.clone(),
                channel: message.channel.clone(),
                content: content.clone(),
                thread_id: message.thread_id.clone(),
            };
            match self.hooks().run(&event).await {
                Err(crate::hooks::HookError::Rejected { reason }) => {
                    return Ok(Some(format!("[Message rejected: {}]", reason)));
                }
                Err(err) => {
                    return Ok(Some(format!("[Message blocked by hook policy: {}]", err)));
                }
                Ok(crate::hooks::HookOutcome::Continue {
                    modified: Some(new_content),
                }) => {
                    submission = Submission::UserInput {
                        content: new_content,
                    };
                }
                _ => {} // Continue, fail-open errors already logged in registry
            }
        }

        // Hydrate thread from DB if it's a historical thread not in memory
        if let Some(ref external_thread_id) = message.thread_id {
            self.maybe_hydrate_thread(message, external_thread_id).await;
        }

        // Resolve session and thread
        let (session, thread_id) = self
            .session_manager
            .resolve_thread(
                &message.user_id,
                &message.channel,
                message.thread_id.as_deref(),
            )
            .await;

        // Auth mode interception: if the thread is awaiting a token, route
        // the message directly to the credential store. Nothing touches
        // logs, turns, history, or compaction.
        let pending_auth = {
            let sess = session.lock().await;
            sess.threads
                .get(&thread_id)
                .and_then(|t| t.pending_auth.clone())
        };

        if let Some(pending) = pending_auth {
            match &submission {
                Submission::UserInput { content } => {
                    return self
                        .process_auth_token(message, &pending, content, session, thread_id)
                        .await;
                }
                _ => {
                    // Any control submission (interrupt, undo, etc.) cancels auth mode
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.pending_auth = None;
                    }
                    // Fall through to normal handling
                }
            }
        }

        tracing::debug!(
            "Received message from {} on {} ({} chars)",
            message.user_id,
            message.channel,
            message.content.len()
        );

        // Process based on submission type
        let result = match submission {
            Submission::UserInput { content } => {
                self.process_user_input(message, session, thread_id, &content)
                    .await
            }
            Submission::SystemCommand { command, args } => {
                self.handle_system_command(&command, &args).await
            }
            Submission::Undo => self.process_undo(session, thread_id).await,
            Submission::Redo => self.process_redo(session, thread_id).await,
            Submission::Interrupt => self.process_interrupt(session, thread_id).await,
            Submission::Compact => self.process_compact(session, thread_id).await,
            Submission::Clear => self.process_clear(session, thread_id).await,
            Submission::NewThread => self.process_new_thread(message).await,
            Submission::Heartbeat => self.process_heartbeat().await,
            Submission::Summarize => self.process_summarize(session, thread_id).await,
            Submission::Suggest => self.process_suggest(session, thread_id).await,
            Submission::Quit => return Ok(None),
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
            SubmissionResult::Response { content } => {
                // Suppress silent replies (e.g. from group chat "nothing to say" responses)
                if crate::llm::is_silent_reply(&content) {
                    tracing::debug!("Suppressing silent reply token");
                    Ok(None)
                } else {
                    Ok(Some(content))
                }
            }
            SubmissionResult::Ok { message } => Ok(message),
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            SubmissionResult::Interrupted => Ok(Some("Interrupted.".into())),
            SubmissionResult::NeedApproval {
                request_id,
                tool_name,
                description,
                parameters,
            } => {
                // Each channel renders the approval prompt via send_status.
                // Web gateway shows an inline card, REPL prints a formatted prompt, etc.
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ApprovalNeeded {
                            request_id: request_id.to_string(),
                            tool_name,
                            description,
                            parameters,
                        },
                        &message.metadata,
                    )
                    .await;

                // Empty string signals the caller to skip respond() (no duplicate text)
                Ok(Some(String::new()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_for_preview;

    /// Regression test for C5: observer must be shut down at agent shutdown.
    ///
    /// Before the fix, Agent::run() never called observer.flush() or shutdown(),
    /// causing OTEL spans buffered in the batch exporter to be silently dropped.
    /// Now calls observer.shutdown() which flushes and releases resources (C4).
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn observer_flush_called_at_shutdown() {
        use std::sync::Arc;

        use crate::agent::Agent;
        use crate::channels::{ChannelManager, IncomingMessage};
        use crate::config::AgentConfig;
        use crate::observability::recording::RecordingObserver;
        use crate::testing::TestHarnessBuilder;

        // Build test deps with a recording observer that tracks flush calls.
        let (observer, _events, _metrics, flush_count) = RecordingObserver::with_flush_counter();

        let harness = TestHarnessBuilder::new()
            .with_observer(Arc::new(observer))
            .build()
            .await;

        // Create a channel that sends a /quit message then stays open.
        let channels = Arc::new(ChannelManager::new());
        struct QuitChannel;
        #[async_trait::async_trait]
        impl crate::channels::Channel for QuitChannel {
            fn name(&self) -> &str {
                "test-quit"
            }
            async fn start(
                &self,
            ) -> Result<crate::channels::MessageStream, crate::error::ChannelError> {
                let quit_msg = IncomingMessage {
                    id: uuid::Uuid::new_v4(),
                    content: "/quit".to_string(),
                    user_id: "test".to_string(),
                    user_name: None,
                    channel: "test-quit".to_string(),
                    thread_id: None,
                    received_at: chrono::Utc::now(),
                    metadata: serde_json::Value::Null,
                };
                Ok(Box::pin(futures::stream::once(async { quit_msg })))
            }
            async fn respond(
                &self,
                _msg: &crate::channels::IncomingMessage,
                _response: crate::channels::OutgoingResponse,
            ) -> Result<(), crate::error::ChannelError> {
                Ok(())
            }
            async fn health_check(&self) -> Result<(), crate::error::ChannelError> {
                Ok(())
            }
        }
        channels.add(Box::new(QuitChannel)).await;

        let config = AgentConfig {
            name: "test-agent".to_string(),
            max_parallel_jobs: 1,
            job_timeout: std::time::Duration::from_secs(10),
            stuck_threshold: std::time::Duration::from_secs(30),
            repair_check_interval: std::time::Duration::from_secs(60),
            max_repair_attempts: 1,
            use_planning: false,
            session_idle_timeout: std::time::Duration::from_secs(300),
            allow_local_tools: false,
            max_cost_per_day_cents: None,
            max_actions_per_hour: None,
            max_tool_iterations: 10,
            auto_approve_tools: false,
        };

        let agent = Agent::new(config, harness.deps, channels, None, None, None, None, None);

        // Run the agent — /quit triggers Ok(None) which breaks the loop.
        agent.run().await.expect("agent should shut down cleanly");

        // REGRESSION: Before fix, flush/shutdown was never called (count == 0).
        // shutdown() calls flush() via the trait default, so flush_count == 1.
        assert_eq!(
            flush_count.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "observer.shutdown() must be called exactly once during agent shutdown"
        );
    }

    #[test]
    fn test_truncate_short_input() {
        assert_eq!(truncate_for_preview("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_empty_input() {
        assert_eq!(truncate_for_preview("", 10), "");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate_for_preview("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_over_limit() {
        let result = truncate_for_preview("hello world, this is long", 10);
        assert!(result.ends_with("..."));
        // "hello worl" = 10 chars + "..."
        assert_eq!(result, "hello worl...");
    }

    #[test]
    fn test_truncate_collapses_newlines() {
        let result = truncate_for_preview("line1\nline2\nline3", 100);
        assert!(!result.contains('\n'));
        assert_eq!(result, "line1 line2 line3");
    }

    #[test]
    fn test_truncate_collapses_whitespace() {
        let result = truncate_for_preview("hello   world", 100);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_truncate_multibyte_utf8() {
        // Each emoji is 4 bytes. Truncating at char boundary must not panic.
        let input = "😀😁😂🤣😃😄😅😆😉😊";
        let result = truncate_for_preview(input, 5);
        assert!(result.ends_with("..."));
        // First 5 chars = 5 emoji
        assert_eq!(result, "😀😁😂🤣😃...");
    }

    #[test]
    fn test_truncate_cjk_characters() {
        // CJK chars are 3 bytes each in UTF-8.
        let input = "你好世界测试数据很长的字符串";
        let result = truncate_for_preview(input, 4);
        assert_eq!(result, "你好世界...");
    }

    #[test]
    fn test_truncate_mixed_multibyte_and_ascii() {
        let input = "hello 世界 foo";
        let result = truncate_for_preview(input, 8);
        // 'h','e','l','l','o',' ','世','界' = 8 chars
        assert_eq!(result, "hello 世界...");
    }

    /// Tests for outbound event emission (H8 fix: emit only after successful send).
    ///
    /// These tests exercise the `send_result` / `match send_result` block in the agent
    /// loop, verifying that `ObserverEvent::ChannelMessage { direction: "outbound" }`
    /// is emitted only when the channel send actually succeeds.
    mod outbound_event_tests {
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use async_trait::async_trait;
        use futures::stream;

        use crate::agent::agent_loop::{Agent, AgentDeps};
        use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
        use crate::channels::{
            Channel, ChannelManager, IncomingMessage, MessageStream, OutgoingResponse,
        };
        use crate::config::{AgentConfig, SafetyConfig, SkillsConfig};
        use crate::error::ChannelError;
        use crate::hooks::hook::HookFailureMode;
        use crate::hooks::{
            Hook, HookContext, HookError, HookEvent, HookOutcome, HookPoint, HookRegistry,
        };
        use crate::observability::recording::RecordingObserver;
        use crate::observability::traits::ObserverEvent;
        use crate::safety::SafetyLayer;
        use crate::testing::StubLlm;
        use crate::tools::ToolRegistry;

        // ── Stub channel ────────────────────────────────────────────

        /// A channel that records `respond` calls and can be configured to fail.
        struct StubChannel {
            name: String,
            /// What `respond` should return.
            respond_result: Mutex<Result<(), ChannelError>>,
            /// Content of each `respond` call, for assertion.
            sent: Mutex<Vec<String>>,
        }

        impl StubChannel {
            fn ok(name: &str) -> Self {
                Self {
                    name: name.to_string(),
                    respond_result: Mutex::new(Ok(())),
                    sent: Mutex::new(Vec::new()),
                }
            }

            fn failing(name: &str) -> Self {
                Self {
                    name: name.to_string(),
                    respond_result: Mutex::new(Err(ChannelError::SendFailed {
                        name: name.to_string(),
                        reason: "simulated failure".to_string(),
                    })),
                    sent: Mutex::new(Vec::new()),
                }
            }

            fn sent_contents(&self) -> Vec<String> {
                self.sent.lock().unwrap().clone()
            }
        }

        #[async_trait]
        impl Channel for StubChannel {
            fn name(&self) -> &str {
                &self.name
            }

            async fn start(&self) -> Result<MessageStream, ChannelError> {
                // Return an empty stream; we use the inject sender instead.
                Ok(Box::pin(stream::empty()))
            }

            async fn respond(
                &self,
                _msg: &IncomingMessage,
                response: OutgoingResponse,
            ) -> Result<(), ChannelError> {
                self.sent.lock().unwrap().push(response.content.clone());
                // Clone the stored result
                match &*self.respond_result.lock().unwrap() {
                    Ok(()) => Ok(()),
                    Err(e) => Err(ChannelError::SendFailed {
                        name: self.name.clone(),
                        reason: format!("{}", e),
                    }),
                }
            }

            async fn health_check(&self) -> Result<(), ChannelError> {
                Ok(())
            }
        }

        // ── Stub hooks ─────────────────────────────────────────────

        /// Hook that rejects all BeforeOutbound events.
        struct RejectOutbound;

        #[async_trait]
        impl Hook for RejectOutbound {
            fn name(&self) -> &str {
                "reject-outbound"
            }
            fn hook_points(&self) -> &[HookPoint] {
                &[HookPoint::BeforeOutbound]
            }
            fn failure_mode(&self) -> HookFailureMode {
                HookFailureMode::FailClosed
            }
            async fn execute(
                &self,
                _event: &HookEvent,
                _ctx: &HookContext,
            ) -> Result<HookOutcome, HookError> {
                Ok(HookOutcome::reject("blocked by test hook"))
            }
        }

        /// Hook that modifies outbound content.
        struct ModifyOutbound {
            replacement: String,
        }

        impl ModifyOutbound {
            fn new(replacement: &str) -> Self {
                Self {
                    replacement: replacement.to_string(),
                }
            }
        }

        #[async_trait]
        impl Hook for ModifyOutbound {
            fn name(&self) -> &str {
                "modify-outbound"
            }
            fn hook_points(&self) -> &[HookPoint] {
                &[HookPoint::BeforeOutbound]
            }
            async fn execute(
                &self,
                _event: &HookEvent,
                _ctx: &HookContext,
            ) -> Result<HookOutcome, HookError> {
                Ok(HookOutcome::modify(self.replacement.clone()))
            }
        }

        /// Hook that errors with fail-closed mode.
        struct FailClosedHook;

        #[async_trait]
        impl Hook for FailClosedHook {
            fn name(&self) -> &str {
                "fail-closed"
            }
            fn hook_points(&self) -> &[HookPoint] {
                &[HookPoint::BeforeOutbound]
            }
            fn failure_mode(&self) -> HookFailureMode {
                HookFailureMode::FailClosed
            }
            async fn execute(
                &self,
                _event: &HookEvent,
                _ctx: &HookContext,
            ) -> Result<HookOutcome, HookError> {
                Err(HookError::ExecutionFailed {
                    reason: "simulated hook failure".to_string(),
                })
            }
        }

        /// Hook that errors with fail-open mode (default).
        struct FailOpenHook;

        #[async_trait]
        impl Hook for FailOpenHook {
            fn name(&self) -> &str {
                "fail-open"
            }
            fn hook_points(&self) -> &[HookPoint] {
                &[HookPoint::BeforeOutbound]
            }
            fn failure_mode(&self) -> HookFailureMode {
                HookFailureMode::FailOpen
            }
            async fn execute(
                &self,
                _event: &HookEvent,
                _ctx: &HookContext,
            ) -> Result<HookOutcome, HookError> {
                Err(HookError::ExecutionFailed {
                    reason: "simulated hook failure".to_string(),
                })
            }
        }

        // ── Helpers ─────────────────────────────────────────────────

        fn make_agent_config() -> AgentConfig {
            AgentConfig {
                name: "test-outbound".to_string(),
                max_parallel_jobs: 1,
                job_timeout: Duration::from_secs(60),
                stuck_threshold: Duration::from_secs(60),
                repair_check_interval: Duration::from_secs(30),
                max_repair_attempts: 1,
                use_planning: false,
                session_idle_timeout: Duration::from_secs(300),
                allow_local_tools: false,
                max_cost_per_day_cents: None,
                max_actions_per_hour: None,
                max_tool_iterations: 50,
                auto_approve_tools: true,
            }
        }

        /// Build an agent that uses the given channel and hooks, with a recording observer.
        /// Returns (Agent, observer events handle, channel reference).
        async fn build_agent(
            channel: Arc<StubChannel>,
            hooks: Arc<HookRegistry>,
            response: &str,
        ) -> (Agent, Arc<Mutex<Vec<ObserverEvent>>>) {
            let (observer, events, _, _) = RecordingObserver::with_flush_counter();

            let cm = Arc::new(ChannelManager::new());
            // Wrap in a newtype so we can pass Arc<StubChannel> as Box<dyn Channel>
            cm.add(Box::new(ArcChannel(Arc::clone(&channel)))).await;

            let deps = AgentDeps {
                store: None,
                llm: Arc::new(StubLlm::new(response)),
                cheap_llm: None,
                safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                })),
                tools: Arc::new(ToolRegistry::new()),
                workspace: None,
                extension_manager: None,
                skill_registry: None,
                skill_catalog: None,
                skills_config: SkillsConfig::default(),
                hooks,
                cost_guard: Arc::new(CostGuard::new(CostGuardConfig::default())),
                observer: Arc::new(observer),
            };

            let agent = Agent::new(
                make_agent_config(),
                deps,
                cm.clone(),
                None,
                None,
                None,
                None,
                None,
            );

            (agent, events)
        }

        /// Wrapper to use `Arc<StubChannel>` as `Box<dyn Channel>`.
        struct ArcChannel(Arc<StubChannel>);

        #[async_trait]
        impl Channel for ArcChannel {
            fn name(&self) -> &str {
                self.0.name()
            }
            async fn start(&self) -> Result<MessageStream, ChannelError> {
                self.0.start().await
            }
            async fn respond(
                &self,
                msg: &IncomingMessage,
                response: OutgoingResponse,
            ) -> Result<(), ChannelError> {
                self.0.respond(msg, response).await
            }
            async fn health_check(&self) -> Result<(), ChannelError> {
                self.0.health_check().await
            }
        }

        /// Helper to count outbound events.
        fn count_outbound_events(events: &[ObserverEvent]) -> usize {
            events
                .iter()
                .filter(|e| {
                    matches!(e,
                        ObserverEvent::ChannelMessage { direction, .. } if direction == "outbound"
                    )
                })
                .count()
        }

        /// Helper to count inbound events.
        fn count_inbound_events(events: &[ObserverEvent]) -> usize {
            events
                .iter()
                .filter(|e| {
                    matches!(e,
                        ObserverEvent::ChannelMessage { direction, .. } if direction == "inbound"
                    )
                })
                .count()
        }

        /// Send a single message followed by /quit and run the agent loop.
        async fn send_and_run(
            agent: Agent,
            events: Arc<Mutex<Vec<ObserverEvent>>>,
            channel_name: &str,
            content: &str,
        ) -> Vec<ObserverEvent> {
            let inject_tx = agent.channels.inject_sender();

            // Send the test message, then /quit to terminate the loop.
            let msg = IncomingMessage::new(channel_name, "test-user", content);
            inject_tx.send(msg).await.expect("inject message");
            let quit = IncomingMessage::new(channel_name, "test-user", "/quit");
            inject_tx.send(quit).await.expect("inject quit");

            // Run the agent loop. It will process messages and exit on /quit.
            agent.run().await.ok();

            // Collect events
            events.lock().unwrap().clone()
        }

        // ── Tests ───────────────────────────────────────────────────

        #[tokio::test]
        async fn outbound_event_emitted_on_successful_send() {
            let channel = Arc::new(StubChannel::ok("test-ch"));
            let hooks = Arc::new(HookRegistry::new());
            let (agent, events) = build_agent(channel.clone(), hooks, "Hello!").await;

            let recorded = send_and_run(agent, events, "test-ch", "/ping").await;

            assert_eq!(
                count_inbound_events(&recorded),
                2,
                "should have 2 inbound events (ping + quit)"
            );
            assert_eq!(
                count_outbound_events(&recorded),
                1,
                "should have 1 outbound event"
            );
        }

        #[tokio::test]
        async fn no_outbound_event_when_hook_rejects() {
            let channel = Arc::new(StubChannel::ok("test-ch"));
            let hooks = Arc::new(HookRegistry::new());
            hooks.register(Arc::new(RejectOutbound)).await;
            let (agent, events) = build_agent(channel.clone(), hooks, "Hello!").await;

            let recorded = send_and_run(agent, events, "test-ch", "/ping").await;

            assert_eq!(
                count_inbound_events(&recorded),
                2,
                "should have 2 inbound events (ping + quit)"
            );
            assert_eq!(
                count_outbound_events(&recorded),
                0,
                "hook rejected: should have 0 outbound events"
            );
            assert!(
                channel.sent_contents().is_empty(),
                "nothing should be sent to channel"
            );
        }

        #[tokio::test]
        async fn outbound_event_emitted_when_hook_modifies() {
            let channel = Arc::new(StubChannel::ok("test-ch"));
            let hooks = Arc::new(HookRegistry::new());
            hooks
                .register(Arc::new(ModifyOutbound::new("Modified response")))
                .await;
            let (agent, events) = build_agent(channel.clone(), hooks, "Original").await;

            let recorded = send_and_run(agent, events, "test-ch", "/ping").await;

            assert_eq!(
                count_outbound_events(&recorded),
                1,
                "should have 1 outbound event for modified content"
            );
            let sent = channel.sent_contents();
            assert_eq!(sent.len(), 1);
            assert_eq!(
                sent[0], "Modified response",
                "channel should receive the modified content"
            );
        }

        #[tokio::test]
        async fn no_outbound_event_when_send_fails() {
            let channel = Arc::new(StubChannel::failing("test-ch"));
            let hooks = Arc::new(HookRegistry::new());
            let (agent, events) = build_agent(channel, hooks, "Hello!").await;

            let recorded = send_and_run(agent, events, "test-ch", "/ping").await;

            assert_eq!(
                count_inbound_events(&recorded),
                2,
                "should have 2 inbound events (ping + quit)"
            );
            assert_eq!(
                count_outbound_events(&recorded),
                0,
                "send failed: should have 0 outbound events"
            );
        }

        #[tokio::test]
        async fn no_outbound_event_when_hook_fail_closed() {
            let channel = Arc::new(StubChannel::ok("test-ch"));
            let hooks = Arc::new(HookRegistry::new());
            hooks.register(Arc::new(FailClosedHook)).await;
            let (agent, events) = build_agent(channel.clone(), hooks, "Hello!").await;

            let recorded = send_and_run(agent, events, "test-ch", "/ping").await;

            assert_eq!(
                count_inbound_events(&recorded),
                2,
                "should have 2 inbound events (ping + quit)"
            );
            assert_eq!(
                count_outbound_events(&recorded),
                0,
                "fail-closed hook: should have 0 outbound events"
            );
            assert!(
                channel.sent_contents().is_empty(),
                "nothing should be sent to channel"
            );
        }

        #[tokio::test]
        async fn outbound_event_emitted_when_hook_fail_open() {
            let channel = Arc::new(StubChannel::ok("test-ch"));
            let hooks = Arc::new(HookRegistry::new());
            hooks.register(Arc::new(FailOpenHook)).await;
            let (agent, events) = build_agent(channel.clone(), hooks, "Hello!").await;

            let recorded = send_and_run(agent, events, "test-ch", "/ping").await;

            assert_eq!(
                count_inbound_events(&recorded),
                2,
                "should have 2 inbound events (ping + quit)"
            );
            assert_eq!(
                count_outbound_events(&recorded),
                1,
                "fail-open hook: should still emit outbound event"
            );
            assert!(
                !channel.sent_contents().is_empty(),
                "message should be sent to channel"
            );
        }

        #[tokio::test]
        async fn outbound_events_match_successful_sends() {
            // Send 3 messages; only the ones that succeed should emit outbound events.
            // We use a channel that always succeeds, no hooks, and count.
            let channel = Arc::new(StubChannel::ok("test-ch"));
            let hooks = Arc::new(HookRegistry::new());
            let (agent, events) = build_agent(channel.clone(), hooks, "Reply").await;

            let inject_tx = agent.channels.inject_sender();

            // Send 3 messages then /quit
            for i in 0..3 {
                let msg = IncomingMessage::new("test-ch", "user", format!("msg-{i}"));
                inject_tx.send(msg).await.expect("inject");
            }
            let quit = IncomingMessage::new("test-ch", "user", "/quit");
            inject_tx.send(quit).await.expect("inject quit");

            agent.run().await.ok();

            let recorded = events.lock().unwrap().clone();
            let inbound = count_inbound_events(&recorded);
            let outbound = count_outbound_events(&recorded);

            // 4 inbound events: 3 messages + 1 /quit
            assert_eq!(inbound, 4, "should have 4 inbound events (3 msgs + quit)");
            assert_eq!(
                outbound, 3,
                "should have 3 outbound events (all sends succeeded)"
            );
            assert_eq!(
                channel.sent_contents().len(),
                3,
                "3 messages sent to channel"
            );
        }
    }
}
