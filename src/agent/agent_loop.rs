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
use uuid::Uuid;

use crate::agent::context_monitor::ContextMonitor;
use crate::agent::heartbeat::{spawn_heartbeat, spawn_multi_user_heartbeat};
use crate::agent::routine_engine::{RoutineEngine, spawn_cron_ticker};
use crate::agent::self_repair::{DefaultSelfRepair, RepairResult, SelfRepair};
use crate::agent::session::ThreadState;
use crate::agent::session_manager::SessionManager;
use crate::agent::submission::{Submission, SubmissionParser, SubmissionResult};
use crate::agent::{HeartbeatConfig as AgentHeartbeatConfig, Router, Scheduler, SchedulerDeps};
use crate::channels::{ChannelManager, IncomingMessage, MessageSource, OutgoingResponse};
use crate::config::{AgentConfig, HeartbeatConfig, RoutineConfig, SkillsConfig};
use crate::context::ContextManager;
use crate::db::Database;
use crate::error::{ChannelError, Error};
use crate::extensions::ExtensionManager;
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::safety::SafetyLayer;
use crate::skills::SkillRegistry;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

/// Static greeting persisted to DB and broadcast on first launch.
///
/// Sent before the LLM is involved so the user sees something immediately.
/// The conversational onboarding (profile building, channel setup) happens
/// organically in the subsequent turns driven by BOOTSTRAP.md.
const BOOTSTRAP_GREETING: &str = include_str!("../workspace/seeds/GREETING.md");

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

#[cfg(test)]
fn resolve_routine_notification_user(metadata: &serde_json::Value) -> Option<String> {
    resolve_owner_scope_notification_user(
        metadata.get("notify_user").and_then(|value| value.as_str()),
        metadata.get("owner_id").and_then(|value| value.as_str()),
    )
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn resolve_owner_scope_notification_user(
    explicit_user: Option<&str>,
    owner_fallback: Option<&str>,
) -> Option<String> {
    trimmed_option(explicit_user).or_else(|| trimmed_option(owner_fallback))
}

fn is_single_message_repl(message: &IncomingMessage) -> bool {
    message.channel == "repl"
        && message
            .metadata
            .get("single_message_mode")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
}

async fn resolve_channel_notification_user(
    extension_manager: Option<&Arc<ExtensionManager>>,
    channel: Option<&str>,
    explicit_user: Option<&str>,
    owner_fallback: Option<&str>,
) -> Option<String> {
    if let Some(user) = trimmed_option(explicit_user) {
        return Some(user);
    }

    if let Some(channel_name) = trimmed_option(channel)
        && let Some(extension_manager) = extension_manager
        && let Some(target) = extension_manager
            .notification_target_for_channel(&channel_name)
            .await
    {
        return Some(target);
    }

    resolve_owner_scope_notification_user(explicit_user, owner_fallback)
}

async fn resolve_routine_notification_target(
    extension_manager: Option<&Arc<ExtensionManager>>,
    metadata: &serde_json::Value,
) -> Option<String> {
    resolve_channel_notification_user(
        extension_manager,
        metadata
            .get("notify_channel")
            .and_then(|value| value.as_str()),
        metadata.get("notify_user").and_then(|value| value.as_str()),
        metadata.get("owner_id").and_then(|value| value.as_str()),
    )
    .await
}

pub(crate) fn chat_tool_execution_metadata(message: &IncomingMessage) -> serde_json::Value {
    serde_json::json!({
        "notify_channel": message.channel,
        "notify_user": message
            .routing_target()
            .unwrap_or_else(|| message.user_id.clone()),
        "notify_thread_id": message.thread_id,
        "notify_metadata": message.metadata,
    })
}

fn should_fallback_routine_notification(error: &ChannelError) -> bool {
    !matches!(error, ChannelError::MissingRoutingTarget { .. })
}

/// Core dependencies for the agent.
///
/// Bundles the shared components to reduce argument count.
pub struct AgentDeps {
    /// Resolved durable owner scope for the instance.
    pub owner_id: String,
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
    /// SSE manager for live job event streaming to the web gateway.
    pub sse_tx: Option<Arc<crate::channels::web::sse::SseManager>>,
    /// HTTP interceptor for trace recording/replay.
    pub http_interceptor: Option<Arc<dyn crate::llm::recording::HttpInterceptor>>,
    /// Audio transcription middleware for voice messages.
    pub transcription: Option<Arc<crate::llm::transcription::TranscriptionMiddleware>>,
    /// Document text extraction middleware for PDF, DOCX, PPTX, etc.
    pub document_extraction: Option<Arc<crate::document_extraction::DocumentExtractionMiddleware>>,
    /// Sandbox readiness state for full-job routine dispatch.
    pub sandbox_readiness: crate::agent::routine_engine::SandboxReadiness,
    /// Software builder for self-repair tool rebuilding.
    pub builder: Option<Arc<dyn crate::tools::SoftwareBuilder>>,
    /// Resolved LLM backend identifier (e.g., "nearai", "openai", "groq").
    /// Used by `/model` persistence to determine which env var to update.
    pub llm_backend: String,
    /// Per-tenant rate limiting registry (lazily creates rate state per user).
    pub tenant_rates: Arc<crate::tenant::TenantRateRegistry>,
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
    /// Shared routine-engine slot used for internal event matching and for exposing
    /// the engine to gateway/manual trigger entry points.
    pub(super) routine_engine_slot:
        Arc<tokio::sync::RwLock<Option<Arc<crate::agent::routine_engine::RoutineEngine>>>>,
}

impl Agent {
    pub(super) fn owner_id(&self) -> &str {
        if let Some(workspace) = self.deps.workspace.as_ref() {
            debug_assert_eq!(
                workspace.user_id(),
                self.deps.owner_id,
                "workspace.user_id() must stay aligned with deps.owner_id"
            );
        }

        &self.deps.owner_id
    }

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

        let mut scheduler = Scheduler::new(
            config.clone(),
            context_manager.clone(),
            deps.llm.clone(),
            deps.safety.clone(),
            SchedulerDeps {
                tools: deps.tools.clone(),
                extension_manager: deps.extension_manager.clone(),
                store: deps
                    .store
                    .as_ref()
                    .map(|db| crate::tenant::AdminScope::new(Arc::clone(db))),
                hooks: deps.hooks.clone(),
            },
        );
        if let Some(ref sse) = deps.sse_tx {
            scheduler.set_sse_sender(Arc::clone(sse));
        }
        if let Some(ref interceptor) = deps.http_interceptor {
            scheduler.set_http_interceptor(Arc::clone(interceptor));
        }
        let scheduler = Arc::new(scheduler);

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
            routine_engine_slot: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Replace the routine-engine slot with a shared one so the gateway and
    /// agent reference the same engine.
    pub fn set_routine_engine_slot(
        &mut self,
        slot: Arc<tokio::sync::RwLock<Option<Arc<crate::agent::routine_engine::RoutineEngine>>>>,
    ) {
        self.routine_engine_slot = slot;
    }

    async fn routine_engine(&self) -> Option<Arc<crate::agent::routine_engine::RoutineEngine>> {
        self.routine_engine_slot.read().await.clone()
    }

    // Convenience accessors

    /// Get the scheduler (for external wiring, e.g. CreateJobTool).
    pub fn scheduler(&self) -> Arc<Scheduler> {
        Arc::clone(&self.scheduler)
    }

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

    /// Build a tenant-scoped execution context for the given user.
    ///
    /// This is the standard entry point for per-user operations. The returned
    /// [`TenantCtx`] provides a [`TenantScope`] that auto-binds `user_id` on
    /// every database operation and a per-user rate limiter.
    pub(super) async fn tenant_ctx(&self, user_id: &str) -> crate::tenant::TenantCtx {
        let rate = self.deps.tenant_rates.get_or_create(user_id).await;

        let store = self
            .deps
            .store
            .as_ref()
            .map(|db| crate::tenant::TenantScope::new(user_id, Arc::clone(db)));

        // Reuse the owner workspace if user matches, otherwise create per-user.
        // Per-user workspaces are seeded on first creation so they get identity
        // files and BOOTSTRAP.md (which triggers the onboarding greeting).
        let workspace = match &self.deps.workspace {
            Some(ws) if ws.user_id() == user_id => Some(Arc::clone(ws)),
            _ => {
                if let Some(db) = self.deps.store.as_ref() {
                    let ws = Arc::new(Workspace::new_with_db(user_id, Arc::clone(db)));
                    if let Err(e) = ws.seed_if_empty().await {
                        tracing::warn!(
                            user_id = user_id,
                            "Failed to seed per-user workspace: {}",
                            e
                        );
                    }
                    Some(ws)
                } else {
                    None
                }
            }
        };

        crate::tenant::TenantCtx::new(
            user_id,
            store,
            workspace,
            Arc::clone(&self.deps.cost_guard),
            rate,
        )
    }

    /// Get an admin-scoped database accessor for cross-tenant operations.
    ///
    /// Only for system-level components (heartbeat, routine engine, self-repair,
    /// scheduler). Handler code should use [`tenant_ctx()`](Self::tenant_ctx) instead.
    pub(super) fn admin_store(&self) -> Option<crate::tenant::AdminScope> {
        self.deps
            .store
            .as_ref()
            .map(|db| crate::tenant::AdminScope::new(Arc::clone(db)))
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
        // Proactive bootstrap: persist the static greeting to DB *before*
        // starting channels so the first web client sees it via history.
        let bootstrap_thread_id = if self
            .workspace()
            .is_some_and(|ws| ws.take_bootstrap_pending())
        {
            tracing::debug!(
                "Fresh workspace detected — persisting static bootstrap greeting to DB"
            );
            if let Some(store) = self.store() {
                let thread_id = store
                    .get_or_create_assistant_conversation("default", "gateway")
                    .await
                    .ok();
                if let Some(id) = thread_id {
                    self.persist_assistant_response(id, "gateway", "default", BOOTSTRAP_GREETING)
                        .await;
                }
                thread_id
            } else {
                None
            }
        } else {
            None
        };

        // Start channels
        let mut message_stream = self.channels.start_all().await?;

        // Start self-repair task with notification forwarding
        let mut self_repair = DefaultSelfRepair::new(
            self.context_manager.clone(),
            self.config.stuck_threshold,
            self.config.max_repair_attempts,
        );
        if let Some(admin) = self.admin_store() {
            self_repair = self_repair.with_store(admin);
        }
        if let Some(ref builder) = self.deps.builder {
            self_repair = self_repair.with_builder(Arc::clone(builder), Arc::clone(self.tools()));
        }
        let repair = Arc::new(self_repair);
        let repair_interval = self.config.repair_check_interval;
        let repair_channels = self.channels.clone();
        let repair_owner_id = self.owner_id().to_string();
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
                        let _ = repair_channels
                            .broadcast_all(&repair_owner_id, response)
                            .await;
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
                            let _ = repair_channels
                                .broadcast_all(&repair_owner_id, response)
                                .await;
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
                    let mut config = AgentHeartbeatConfig::default()
                        .with_interval(std::time::Duration::from_secs(hb_config.interval_secs));
                    config.quiet_hours_start = hb_config.quiet_hours_start;
                    config.quiet_hours_end = hb_config.quiet_hours_end;
                    config.multi_tenant = hb_config.multi_tenant;
                    config.timezone = hb_config
                        .timezone
                        .clone()
                        .or_else(|| Some(self.config.default_timezone.clone()));
                    let heartbeat_notify_user = resolve_owner_scope_notification_user(
                        hb_config.notify_user.as_deref(),
                        Some(self.owner_id()),
                    );
                    if let Some(channel) = &hb_config.notify_channel
                        && let Some(user) = heartbeat_notify_user.as_deref()
                    {
                        config = config.with_notify(user, channel);
                    }

                    // Set up notification channel
                    let (notify_tx, mut notify_rx) =
                        tokio::sync::mpsc::channel::<OutgoingResponse>(16);

                    // Spawn notification forwarder that routes through channel manager
                    let notify_channel = hb_config.notify_channel.clone();
                    let notify_target = resolve_channel_notification_user(
                        self.deps.extension_manager.as_ref(),
                        hb_config.notify_channel.as_deref(),
                        hb_config.notify_user.as_deref(),
                        Some(self.owner_id()),
                    )
                    .await;
                    let notify_user = heartbeat_notify_user;
                    let channels = self.channels.clone();
                    let is_multi_tenant = hb_config.multi_tenant;
                    tokio::spawn(async move {
                        while let Some(response) = notify_rx.recv().await {
                            // In multi-tenant mode, extract the owning user_id from
                            // the response metadata so notifications reach the
                            // correct user rather than the agent's owner.
                            // This intentionally overrides the configured notify_target
                            // because each user's heartbeat should notify that user.
                            let effective_user = if is_multi_tenant {
                                response
                                    .metadata
                                    .get("owner_id")
                                    .and_then(|v| v.as_str())
                                    .map(String::from)
                            } else {
                                None
                            };

                            // Try the configured channel first, fall back to
                            // broadcasting on all channels.
                            let targeted_ok = if let Some(ref channel) = notify_channel {
                                let target = effective_user.as_deref().or(notify_target.as_deref());
                                if let Some(user) = target {
                                    channels
                                        .broadcast(channel, user, response.clone())
                                        .await
                                        .is_ok()
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            if !targeted_ok {
                                let fallback = effective_user.as_deref().or(notify_user.as_deref());
                                if let Some(user) = fallback {
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
                        }
                    });

                    let hygiene = self
                        .hygiene_config
                        .as_ref()
                        .map(|h| h.to_workspace_config())
                        .unwrap_or_default();

                    if config.multi_tenant {
                        if let Some(admin) = self.admin_store() {
                            Some(spawn_multi_user_heartbeat(
                                config,
                                hygiene,
                                self.cheap_llm().clone(),
                                Some(notify_tx),
                                admin,
                            ))
                        } else {
                            tracing::warn!("Multi-tenant heartbeat requires a database store");
                            None
                        }
                    } else {
                        Some(spawn_heartbeat(
                            config,
                            hygiene,
                            workspace.clone(),
                            self.cheap_llm().clone(),
                            Some(notify_tx),
                            self.admin_store(),
                        ))
                    }
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
                        crate::tenant::AdminScope::new(Arc::clone(store)),
                        self.llm().clone(),
                        Arc::clone(workspace),
                        notify_tx,
                        Some(self.scheduler.clone()),
                        self.deps.extension_manager.clone(),
                        self.tools().clone(),
                        self.safety().clone(),
                        self.deps.sandbox_readiness,
                    ));

                    // Register routine tools
                    self.deps
                        .tools
                        .register_routine_tools(Arc::clone(store), Arc::clone(&engine));

                    // Load initial event cache
                    engine.refresh_event_cache().await;

                    // Spawn notification forwarder (mirrors heartbeat pattern)
                    let channels = self.channels.clone();
                    let extension_manager = self.deps.extension_manager.clone();
                    tokio::spawn(async move {
                        while let Some(response) = notify_rx.recv().await {
                            let notify_channel = response
                                .metadata
                                .get("notify_channel")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let fallback_user = resolve_owner_scope_notification_user(
                                response
                                    .metadata
                                    .get("notify_user")
                                    .and_then(|v| v.as_str()),
                                response.metadata.get("owner_id").and_then(|v| v.as_str()),
                            );
                            let Some(user) = resolve_routine_notification_target(
                                extension_manager.as_ref(),
                                &response.metadata,
                            )
                            .await
                            else {
                                tracing::warn!(
                                    notify_channel = ?notify_channel,
                                    "Skipping routine notification with no explicit target or owner scope"
                                );
                                continue;
                            };

                            // Try the configured channel first, fall back to
                            // broadcasting on all channels.
                            let targeted_ok = if let Some(ref channel) = notify_channel {
                                match channels.broadcast(channel, &user, response.clone()).await {
                                    Ok(()) => true,
                                    Err(e) => {
                                        let should_fallback =
                                            should_fallback_routine_notification(&e);
                                        tracing::warn!(
                                            channel = %channel,
                                            user = %user,
                                            error = %e,
                                            should_fallback,
                                            "Failed to send routine notification to configured channel"
                                        );
                                        if !should_fallback {
                                            continue;
                                        }
                                        false
                                    }
                                }
                            } else {
                                false
                            };

                            if !targeted_ok && let Some(user) = fallback_user {
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
                        }
                    });

                    // Spawn cron ticker
                    let cron_interval =
                        std::time::Duration::from_secs(rt_config.cron_check_interval_secs);
                    let cron_handle = spawn_cron_ticker(Arc::clone(&engine), cron_interval);

                    // Store engine reference for event trigger checking
                    // Safety: we're in run() which takes self, no other reference exists
                    let engine_ref = Arc::clone(&engine);
                    // SAFETY: self is consumed by run(), we can smuggle the engine in
                    // via a local to use in the message loop below.

                    // Expose engine to gateway for manual triggering
                    *self.routine_engine_slot.write().await = Some(Arc::clone(&engine));

                    tracing::debug!(
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

        // Bootstrap phase 2: register the thread in session manager and
        // broadcast the greeting via SSE for any clients already connected.
        // The greeting was already persisted to DB before start_all(), so
        // clients that connect after this point will see it via history.
        if let Some(id) = bootstrap_thread_id {
            // Use get_or_create_session (not resolve_thread) to avoid creating
            // an orphan thread. Then insert the DB-sourced thread directly.
            let session = self.session_manager.get_or_create_session("default").await;
            {
                use crate::agent::session::Thread;
                let mut sess = session.lock().await;
                let thread = Thread::with_id(id, sess.id);
                sess.active_thread = Some(id);
                sess.threads.entry(id).or_insert(thread);
            }
            self.session_manager
                .register_thread("default", "gateway", id, session)
                .await;

            let mut out = OutgoingResponse::text(BOOTSTRAP_GREETING.to_string());
            out.thread_id = Some(id.to_string());
            let _ = self.channels.broadcast("gateway", "default", out).await;
        }

        // Main message loop
        tracing::debug!("Agent {} ready and listening", self.config.name);

        loop {
            let message = tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => {
                    tracing::debug!("Ctrl+C received, shutting down...");
                    break;
                }
                msg = message_stream.next() => {
                    match msg {
                        Some(m) => m,
                        None => {
                            tracing::debug!("All channel streams ended, shutting down...");
                            break;
                        }
                    }
                }
            };

            // Apply transcription middleware to audio attachments
            let mut message = message;
            if let Some(ref transcription) = self.deps.transcription {
                transcription.process(&mut message).await;
            }

            // Apply document extraction middleware to document attachments
            if let Some(ref doc_extraction) = self.deps.document_extraction {
                doc_extraction.process(&mut message).await;
            }

            // Store successfully extracted document text in workspace for indexing
            self.store_extracted_documents(&message).await;

            match self.handle_message(&message).await {
                Ok(Some(response)) if !response.is_empty() => {
                    if message.source == MessageSource::RoutineReview {
                        // Route via broadcast (not channel.respond) because
                        // "routine-review" is not a real channel.
                        let notify_channel = message
                            .metadata
                            .get("notify_channel")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty());

                        let outgoing = OutgoingResponse {
                            content: response,
                            thread_id: None,
                            attachments: Vec::new(),
                            metadata: message.metadata.clone(),
                        };

                        if let Some(channel) = notify_channel {
                            if let Err(e) = self
                                .channels
                                .broadcast(channel, &message.user_id, outgoing.clone())
                                .await
                            {
                                tracing::warn!(
                                    channel = %channel,
                                    error = %e,
                                    "Failed to send routine review response, falling back"
                                );
                                let _ = self
                                    .channels
                                    .broadcast_all(&message.user_id, outgoing)
                                    .await;
                            }
                        } else {
                            let _ = self
                                .channels
                                .broadcast_all(&message.user_id, outgoing)
                                .await;
                        }
                    } else {
                        // Hook: BeforeOutbound — allow hooks to modify or suppress outbound
                        let event = crate::hooks::HookEvent::Outbound {
                            user_id: message.user_id.clone(),
                            channel: message.channel.clone(),
                            content: response.clone(),
                            thread_id: message.thread_id.clone(),
                        };
                        match self.hooks().run(&event).await {
                            Err(err) => {
                                tracing::warn!("BeforeOutbound hook blocked response: {}", err);
                            }
                            Ok(crate::hooks::HookOutcome::Continue {
                                modified: Some(new_content),
                            }) => {
                                if let Err(e) = self
                                    .channels
                                    .respond(&message, OutgoingResponse::text(new_content))
                                    .await
                                {
                                    tracing::error!(
                                        channel = %message.channel,
                                        error = %e,
                                        "Failed to send response to channel"
                                    );
                                }
                            }
                            _ => {
                                if let Err(e) = self
                                    .channels
                                    .respond(&message, OutgoingResponse::text(response))
                                    .await
                                {
                                    tracing::error!(
                                        channel = %message.channel,
                                        error = %e,
                                        "Failed to send response to channel"
                                    );
                                }
                            }
                        }
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
                    tracing::debug!("Shutdown command received, exiting...");
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
        }

        // Cleanup
        tracing::debug!("Agent shutting down...");
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

        Ok(())
    }

    /// Store extracted document text in workspace memory for future search/recall.
    async fn store_extracted_documents(&self, message: &IncomingMessage) {
        let workspace = match self.workspace() {
            Some(ws) => ws,
            None => return,
        };

        for attachment in &message.attachments {
            if attachment.kind != crate::channels::AttachmentKind::Document {
                continue;
            }
            let text = match &attachment.extracted_text {
                Some(t) if !t.starts_with('[') => t, // skip error messages like "[Failed to..."
                _ => continue,
            };

            // Sanitize filename: strip path separators to prevent directory traversal
            let raw_name = attachment.filename.as_deref().unwrap_or("unnamed_document");
            let filename: String = raw_name
                .chars()
                .map(|c| {
                    if c == '/' || c == '\\' || c == '\0' {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            let filename = filename.trim_start_matches('.');
            let filename = if filename.is_empty() {
                "unnamed_document"
            } else {
                filename
            };
            let date = chrono::Utc::now().format("%Y-%m-%d");
            let path = format!("documents/{date}/{filename}");

            let header = format!(
                "# {filename}\n\n\
                 > Uploaded by **{}** via **{}** on {date}\n\
                 > MIME: {} | Size: {} bytes\n\n---\n\n",
                message.user_id,
                message.channel,
                attachment.mime_type,
                attachment.size_bytes.unwrap_or(0),
            );
            let content = format!("{header}{text}");

            match workspace.write(&path, &content).await {
                Ok(_) => {
                    tracing::info!(
                        path = %path,
                        text_len = text.len(),
                        "Stored extracted document in workspace memory"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path,
                        error = %e,
                        "Failed to store extracted document in workspace"
                    );
                }
            }
        }
    }

    async fn handle_message(&self, message: &IncomingMessage) -> Result<Option<String>, Error> {
        // Log sensitive details at debug level for troubleshooting
        tracing::debug!(
            message_id = %message.id,
            user_id = %message.user_id,
            channel = %message.channel,
            thread_id = ?message.thread_id,
            "Message details"
        );

        // Dispatch based on message origin. The `source` field and builder
        // methods are pub(crate), so external channels cannot spoof these.
        match message.source {
            MessageSource::Internal => {
                // Internal messages (e.g. job-monitor notifications) are already
                // rendered text — forward directly without the user-input pipeline.
                tracing::debug!(
                    message_id = %message.id,
                    channel = %message.channel,
                    "Forwarding internal message"
                );
                return Ok(Some(message.content.clone()));
            }
            MessageSource::RoutineReview => {
                // Skip parser, hooks, event triggers, auth. Resolve thread and
                // enter agentic loop directly.
                tracing::info!(
                    message_id = %message.id,
                    routine_name = message.metadata.get("routine_name")
                        .and_then(|v| v.as_str()).unwrap_or("unknown"),
                    "Processing routine review"
                );

                let (session, thread_id) = self
                    .session_manager
                    .resolve_thread(
                        &message.user_id,
                        &message.channel, // "routine-review"
                        message.conversation_scope(),
                    )
                    .await;

                let tenant = self.tenant_ctx(&message.user_id).await;

                let result = self
                    .process_user_input(message, tenant, session, thread_id, &message.content)
                    .await;

                return match result {
                    Ok(SubmissionResult::Response { content }) => Ok(Some(content)),
                    Ok(SubmissionResult::Ok { message: msg }) => Ok(msg),
                    Ok(SubmissionResult::Error { message: err }) => Ok(Some(err)),
                    Ok(_) => {
                        tracing::debug!("Routine review returned non-standard result");
                        Ok(None)
                    }
                    Err(e) => Err(e),
                };
            }
            MessageSource::User => {
                // Fall through to existing pipeline
            }
        }

        // Set message tool context for this turn (current channel and target)
        // For Signal, use signal_target from metadata (group:ID or phone number),
        // otherwise fall back to user_id
        let target = message
            .routing_target()
            .unwrap_or_else(|| message.user_id.clone());
        self.tools()
            .set_message_tool_context(Some(message.channel.clone()), Some(target))
            .await;

        // Parse submission type first
        let mut submission = SubmissionParser::parse(&message.content);
        tracing::trace!(
            "[agent_loop] Parsed submission: {:?}",
            std::any::type_name_of_val(&submission)
        );

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
        if let Some(external_thread_id) = message.conversation_scope() {
            tracing::trace!(
                message_id = %message.id,
                thread_id = %external_thread_id,
                "Hydrating thread from DB"
            );
            if let Some(rejection) = self.maybe_hydrate_thread(message, external_thread_id).await {
                return Ok(Some(format!("Error: {}", rejection)));
            }
        }

        // Resolve session and thread. Approval submissions are allowed to
        // target an already-loaded owned thread by UUID across channels so the
        // web approval UI can approve work that originated from HTTP/other
        // owner-scoped channels.
        let approval_thread_uuid = if matches!(
            submission,
            Submission::ExecApproval { .. } | Submission::ApprovalResponse { .. }
        ) {
            message
                .conversation_scope()
                .and_then(|thread_id| Uuid::parse_str(thread_id).ok())
        } else {
            None
        };

        let (session, thread_id) = if let Some(target_thread_id) = approval_thread_uuid {
            let session = self
                .session_manager
                .get_or_create_session(&message.user_id)
                .await;
            let mut sess = session.lock().await;
            if sess.threads.contains_key(&target_thread_id) {
                sess.active_thread = Some(target_thread_id);
                sess.last_active_at = chrono::Utc::now();
                drop(sess);
                self.session_manager
                    .register_thread(
                        &message.user_id,
                        &message.channel,
                        target_thread_id,
                        Arc::clone(&session),
                    )
                    .await;
                (session, target_thread_id)
            } else {
                drop(sess);
                self.session_manager
                    .resolve_thread_with_parsed_uuid(
                        &message.user_id,
                        &message.channel,
                        message.conversation_scope(),
                        approval_thread_uuid,
                    )
                    .await
            }
        } else {
            self.session_manager
                .resolve_thread(
                    &message.user_id,
                    &message.channel,
                    message.conversation_scope(),
                )
                .await
        };
        tracing::debug!(
            message_id = %message.id,
            thread_id = %thread_id,
            "Resolved session and thread"
        );

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
            if pending.is_expired() {
                // TTL exceeded — clear stale auth mode
                tracing::warn!(
                    extension = %pending.extension_name,
                    "Auth mode expired after TTL, clearing"
                );
                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.pending_auth = None;
                    }
                }
                // If this was a user message (possibly a pasted token), return an
                // explicit error instead of forwarding it to the LLM/history.
                if matches!(submission, Submission::UserInput { .. }) {
                    return Ok(Some(format!(
                        "Authentication for **{}** expired. Please try again.",
                        pending.extension_name
                    )));
                }
                // Control submissions (interrupt, undo, etc.) fall through to normal handling
            } else {
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
        }

        tracing::trace!(
            "Received message from {} on {} ({} chars)",
            message.user_id,
            message.channel,
            message.content.len()
        );

        if message.source == MessageSource::User
            && let Submission::UserInput { ref content } = submission
            && let Some(engine) = self.routine_engine().await
        {
            let single_message_repl = is_single_message_repl(message);
            // Use post-hook content so that BeforeInbound hooks that rewrite
            // input are respected by event trigger matching.
            let fired = if single_message_repl {
                engine.check_event_triggers_and_wait(message, content).await
            } else {
                engine.check_event_triggers(message, content).await
            };
            if fired > 0 {
                tracing::debug!(
                    channel = %message.channel,
                    user = %message.user_id,
                    fired,
                    "Consumed inbound user message with matching event-triggered routine(s)"
                );
                return if single_message_repl {
                    Ok(None)
                } else {
                    Ok(Some(String::new()))
                };
            }
        }

        // Build per-tenant execution context once; threaded through all handlers.
        let tenant = self.tenant_ctx(&message.user_id).await;

        // Per-user bootstrap: if this user's workspace was just seeded (fresh),
        // persist the static greeting to their assistant conversation and
        // broadcast it so the web client shows it immediately.
        if tenant
            .workspace()
            .is_some_and(|ws| ws.take_bootstrap_pending())
        {
            tracing::info!(
                user_id = message.user_id,
                "Fresh user workspace — persisting bootstrap greeting"
            );
            if let Some(store) = tenant.store()
                && let Ok(conv_id) = store
                    .get_or_create_assistant_conversation(&message.channel)
                    .await
            {
                let _ = store
                    .add_conversation_message(conv_id, "assistant", BOOTSTRAP_GREETING)
                    .await;
                let mut out = OutgoingResponse::text(BOOTSTRAP_GREETING.to_string());
                out.thread_id = Some(conv_id.to_string());
                let _ = self
                    .channels
                    .broadcast(&message.channel, &message.user_id, out)
                    .await;
            }
        }

        let session_for_empty_exit = Arc::clone(&session);

        // Process based on submission type
        let result = match submission {
            Submission::UserInput { content } => {
                let mut result = self
                    .process_user_input(
                        message,
                        tenant.clone(),
                        session.clone(),
                        thread_id,
                        &content,
                    )
                    .await;

                // Drain any messages queued during processing.
                // Messages are merged (newline-separated) so the LLM receives
                // full context from rapid consecutive inputs instead of
                // processing each as a separate turn with partial context (#259).
                //
                // Only `Response` continues the drain — the user got a normal
                // reply and there may be more queued messages to process.
                //
                // Everything else stops the loop:
                // - `NeedApproval`: thread is blocked on user approval
                // - `Interrupted`: turn was cancelled
                // - `Ok`: control-command acknowledgment (including the "queued"
                //    ack returned when a message arrives during Processing)
                // - `Error`: soft error — draining more messages after an error
                //    would produce confusing interleaved output
                // - `Err(_)`: hard error
                while let Ok(SubmissionResult::Response { content: outgoing }) = &result {
                    let merged = {
                        let mut sess = session.lock().await;
                        sess.threads
                            .get_mut(&thread_id)
                            .and_then(|t| t.drain_pending_messages())
                    };
                    let Some(next_content) = merged else {
                        break;
                    };

                    tracing::debug!(
                        thread_id = %thread_id,
                        merged_len = next_content.len(),
                        "Drain loop: processing merged queued messages"
                    );

                    // Send the completed turn's response before starting the next.
                    //
                    // Known limitations:
                    // - One-shot channels (HttpChannel) consume the response
                    //   sender on the first respond() call keyed by msg.id.
                    //   Subsequent calls (including the outer handler's final
                    //   respond) are silently dropped. For one-shot channels
                    //   only this intermediate response is delivered.
                    // - All drain-loop responses are routed via the original
                    //   `message`, so channels that key routing on message
                    //   identity will attribute every response to the first
                    //   message. This is acceptable for the current
                    //   single-user-per-thread model.
                    if let Err(e) = self
                        .channels
                        .respond(message, OutgoingResponse::text(outgoing.clone()))
                        .await
                    {
                        tracing::warn!(
                            thread_id = %thread_id,
                            "Failed to send intermediate drain-loop response: {e}"
                        );
                    }

                    // Process merged queued messages as a single turn.
                    // Use a message clone with cleared attachments so
                    // augment_with_attachments doesn't re-apply the original
                    // message's attachments to unrelated queued text.
                    let mut queued_msg = message.clone();
                    queued_msg.attachments.clear();
                    result = self
                        .process_user_input(
                            &queued_msg,
                            tenant.clone(),
                            session.clone(),
                            thread_id,
                            &next_content,
                        )
                        .await;

                    // If processing failed, re-queue the drained content so it
                    // isn't lost. It will be picked up on the next successful turn.
                    if !matches!(&result, Ok(SubmissionResult::Response { .. })) {
                        let mut sess = session.lock().await;
                        if let Some(thread) = sess.threads.get_mut(&thread_id) {
                            thread.requeue_drained(next_content);
                            tracing::debug!(
                                thread_id = %thread_id,
                                "Re-queued drained content after non-Response result"
                            );
                        }
                    }
                }

                result
            }
            Submission::SystemCommand { command, args } => {
                tracing::debug!(
                    "[agent_loop] SystemCommand: command={}, channel={}",
                    command,
                    message.channel
                );
                // /reasoning is special-cased here (not in handle_system_command)
                // because it needs the session + thread_id to read turn reasoning
                // data, which handle_system_command's signature doesn't provide.
                if command == "reasoning" {
                    let result = self
                        .handle_reasoning_command(&args, &session, thread_id)
                        .await;
                    return match result {
                        SubmissionResult::Response { content } => Ok(Some(content)),
                        SubmissionResult::Ok { message } => Ok(message),
                        SubmissionResult::Error { message } => {
                            Ok(Some(format!("Error: {}", message)))
                        }
                        _ => {
                            if is_single_message_repl(message) {
                                Ok(None)
                            } else {
                                Ok(Some(String::new()))
                            }
                        }
                    };
                }
                // Authorization checks (including restart channel check) are enforced in handle_system_command
                self.handle_system_command(&command, &args, &message.channel, &tenant)
                    .await
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
            Submission::JobStatus { job_id } => {
                self.process_job_status(&tenant, job_id.as_deref()).await
            }
            Submission::JobCancel { job_id } => self.process_job_cancel(&tenant, &job_id).await,
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
            SubmissionResult::Ok {
                message: output_message,
            } => {
                let should_exit =
                    if output_message.as_deref() == Some("") && is_single_message_repl(message) {
                        let sess = session_for_empty_exit.lock().await;
                        sess.threads
                            .get(&thread_id)
                            .map(|thread| thread.state != ThreadState::AwaitingApproval)
                            .unwrap_or(true)
                    } else {
                        false
                    };

                if should_exit {
                    Ok(None)
                } else {
                    Ok(output_message)
                }
            }
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            SubmissionResult::Interrupted => Ok(Some("Interrupted.".into())),
            SubmissionResult::NeedApproval { .. } => {
                // ApprovalNeeded status was already sent by thread_ops.rs before
                // returning this result. Empty string signals the caller to skip
                // respond() (no duplicate text).
                Ok(Some(String::new()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chat_tool_execution_metadata, is_single_message_repl, resolve_routine_notification_user,
        should_fallback_routine_notification, truncate_for_preview,
    };
    use crate::channels::IncomingMessage;
    use crate::error::ChannelError;

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

    #[test]
    fn resolve_routine_notification_user_prefers_explicit_target() {
        let metadata = serde_json::json!({
            "notify_user": "12345",
            "owner_id": "owner-scope",
        });

        let resolved = resolve_routine_notification_user(&metadata);
        assert_eq!(resolved.as_deref(), Some("12345")); // safety: test-only assertion
    }

    #[test]
    fn resolve_routine_notification_user_falls_back_to_owner_scope() {
        let metadata = serde_json::json!({
            "notify_user": null,
            "owner_id": "owner-scope",
        });

        let resolved = resolve_routine_notification_user(&metadata);
        assert_eq!(resolved.as_deref(), Some("owner-scope")); // safety: test-only assertion
    }

    #[test]
    fn resolve_routine_notification_user_rejects_missing_values() {
        let metadata = serde_json::json!({
            "notify_user": "   ",
        });

        assert_eq!(resolve_routine_notification_user(&metadata), None); // safety: test-only assertion
    }

    #[test]
    fn chat_tool_execution_metadata_prefers_message_routing_target() {
        let message = IncomingMessage::new("telegram", "owner-scope", "hello")
            .with_sender_id("telegram-user")
            .with_thread("thread-7")
            .with_metadata(serde_json::json!({
                "chat_id": 424242,
                "chat_type": "private",
            }));

        let metadata = chat_tool_execution_metadata(&message);
        assert_eq!(
            metadata.get("notify_channel").and_then(|v| v.as_str()),
            Some("telegram")
        ); // safety: test-only assertion
        assert_eq!(
            metadata.get("notify_user").and_then(|v| v.as_str()),
            Some("424242")
        ); // safety: test-only assertion
        assert_eq!(
            metadata.get("notify_thread_id").and_then(|v| v.as_str()),
            Some("thread-7")
        ); // safety: test-only assertion
    }

    #[test]
    fn chat_tool_execution_metadata_falls_back_to_user_scope_without_route() {
        let message = IncomingMessage::new("gateway", "owner-scope", "hello").with_sender_id("");

        let metadata = chat_tool_execution_metadata(&message);
        assert_eq!(
            metadata.get("notify_channel").and_then(|v| v.as_str()),
            Some("gateway")
        ); // safety: test-only assertion
        assert_eq!(
            metadata.get("notify_user").and_then(|v| v.as_str()),
            Some("owner-scope")
        ); // safety: test-only assertion
        assert_eq!(
            metadata.get("notify_thread_id"),
            Some(&serde_json::Value::Null)
        ); // safety: test-only assertion
    }

    #[test]
    fn targeted_routine_notifications_do_not_fallback_without_owner_route() {
        let error = ChannelError::MissingRoutingTarget {
            name: "telegram".to_string(),
            reason: "No stored owner routing target for channel 'telegram'.".to_string(),
        };

        assert!(!should_fallback_routine_notification(&error)); // safety: test-only assertion
    }

    #[test]
    fn targeted_routine_notifications_may_fallback_for_other_errors() {
        let error = ChannelError::SendFailed {
            name: "telegram".to_string(),
            reason: "timeout talking to channel".to_string(),
        };

        assert!(should_fallback_routine_notification(&error)); // safety: test-only assertion
    }

    #[test]
    fn single_message_repl_detection_requires_repl_channel_and_metadata_flag() {
        let repl = IncomingMessage::new("repl", "owner-scope", "hello")
            .with_metadata(serde_json::json!({ "single_message_mode": true }));
        let gateway = IncomingMessage::new("gateway", "owner-scope", "hello")
            .with_metadata(serde_json::json!({ "single_message_mode": true }));
        let plain_repl = IncomingMessage::new("repl", "owner-scope", "hello");

        assert!(is_single_message_repl(&repl)); // safety: test-only assertion
        assert!(!is_single_message_repl(&gateway)); // safety: test-only assertion
        assert!(!is_single_message_repl(&plain_repl)); // safety: test-only assertion
    }
}
