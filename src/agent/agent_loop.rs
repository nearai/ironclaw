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
use crate::channels::{ChannelManager, IncomingMessage, OutgoingResponse, StatusUpdate};
use crate::config::{AgentConfig, HeartbeatConfig, RoutineConfig, SkillsConfig};
use crate::context::ContextManager;
use crate::db::Database;
use crate::error::{ChannelError, Error};
use crate::extensions::ExtensionManager;
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;
use ironclaw_safety::SafetyLayer;
use ironclaw_skills::SkillRegistry;

/// Outcome of [`Agent::handle_message`] — drives the run-loop's response/Done dispatch.
///
/// Distinguishes "no response, turn is over" from "no response, turn is paused"
/// so the run loop can decide whether to emit the terminal `Done` status. Sending
/// `Done` after a pause (e.g. while awaiting tool approval) is incorrect because
/// the thread is not in a terminal state, and would also trip the web UI's
/// missing-response safety net (see #2079).
#[derive(Debug)]
pub(crate) enum HandleOutcome {
    /// Shutdown signal (e.g. `/quit`). Run loop should break.
    Shutdown,
    /// Send this content via the channel, then emit terminal `Done`.
    Respond(String),
    /// No response to send, but the turn is complete — emit `Done` only.
    NoResponse,
    /// Turn is paused (awaiting approval/auth/etc). Do not emit `Done`.
    Pending,
}

impl HandleOutcome {
    /// Convert a legacy `Option<String>` return into a [`HandleOutcome`].
    ///
    /// `None` → `Shutdown`, empty string → `NoResponse`, otherwise `Respond`.
    /// Used to wrap v1 handlers that return `Option<String>`.
    fn from_legacy(opt: Option<String>) -> Self {
        match opt {
            None => HandleOutcome::Shutdown,
            Some(s) if s.is_empty() => HandleOutcome::NoResponse,
            Some(s) => HandleOutcome::Respond(s),
        }
    }
}

impl From<crate::bridge::BridgeOutcome> for HandleOutcome {
    fn from(outcome: crate::bridge::BridgeOutcome) -> Self {
        match outcome {
            crate::bridge::BridgeOutcome::Respond(s) => HandleOutcome::Respond(s),
            crate::bridge::BridgeOutcome::NoResponse => HandleOutcome::NoResponse,
            crate::bridge::BridgeOutcome::Pending => HandleOutcome::Pending,
        }
    }
}

/// Static greeting persisted to DB and broadcast on first launch.
///
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

/// Determine whether a bare-keyword `ApprovalResponse` should be kept as an
/// approval or downgraded to regular user input.
///
/// Returns `true` when the message should be routed as an approval (there IS
/// a pending approval or it's an explicit slash command). Returns `false`
/// when the message should be treated as regular `UserInput`.
///
/// Used by the legacy routing path; the engine_v2 path performs an equivalent
/// check earlier (before the BeforeInbound hook).
fn should_route_as_approval(thread_state: ThreadState, raw_content: &str) -> bool {
    thread_state == ThreadState::AwaitingApproval || raw_content.trim().starts_with('/')
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
    /// Cached settings store. When set, `TenantScope` routes settings reads
    /// through this cache instead of hitting the raw `Database` directly.
    pub settings_store: Option<Arc<dyn crate::db::SettingsStore + Send + Sync>>,
    pub llm: Arc<dyn LlmProvider>,
    /// Cheap/fast LLM for lightweight tasks (heartbeat, routing, evaluation).
    /// Falls back to the main `llm` if None.
    pub cheap_llm: Option<Arc<dyn LlmProvider>>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub workspace: Option<Arc<Workspace>>,
    pub extension_manager: Option<Arc<ExtensionManager>>,
    pub skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
    pub skill_catalog: Option<Arc<ironclaw_skills::catalog::SkillCatalog>>,
    pub skills_config: SkillsConfig,
    pub hooks: Arc<HookRegistry>,
    pub auth_manager: Option<Arc<crate::bridge::auth_manager::AuthManager>>,
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
    pub(crate) deps: AgentDeps,
    pub(crate) channels: Arc<ChannelManager>,
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
    /// Engine v2 mission manager for firing learning missions (set after engine init).
    pub(crate) mission_manager_slot:
        Arc<tokio::sync::RwLock<Option<Arc<ironclaw_engine::MissionManager>>>>,
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
                    .map(|db| crate::tenant::SystemScope::new(Arc::clone(db))),
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
            mission_manager_slot: Arc::new(tokio::sync::RwLock::new(None)),
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

    pub(super) async fn routine_engine(
        &self,
    ) -> Option<Arc<crate::agent::routine_engine::RoutineEngine>> {
        self.routine_engine_slot.read().await.clone()
    }

    /// Set the engine v2 mission manager (called after engine init).
    pub async fn set_mission_manager(&self, mgr: Arc<ironclaw_engine::MissionManager>) {
        *self.mission_manager_slot.write().await = Some(mgr);
    }

    pub(crate) async fn mission_manager(&self) -> Option<Arc<ironclaw_engine::MissionManager>> {
        self.mission_manager_slot.read().await.clone()
    }

    // Convenience accessors

    /// Get the scheduler (for external wiring, e.g. CreateJobTool).
    pub fn scheduler(&self) -> Arc<Scheduler> {
        Arc::clone(&self.scheduler)
    }

    pub(super) fn store(&self) -> Option<&Arc<dyn Database>> {
        self.deps.store.as_ref()
    }

    /// Send a response to the channel, then emit the terminal "Done" status.
    ///
    /// This ordering guarantees that the SSE client receives the assistant
    /// message before the turn-closing event, preventing the web UI from
    /// closing the turn before the message renders (see #2079).
    async fn respond_then_done(
        &self,
        message: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let respond_result = self.channels.respond(message, response).await;
        // Always emit Done regardless of whether respond succeeded, so the
        // client knows the turn is over even when the response delivery fails.
        if let Err(e) = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Status("Done".into()),
                &message.metadata,
            )
            .await
        {
            tracing::warn!(
                channel = %message.channel,
                error = %e,
                "Failed to send Done status after response"
            );
        }
        respond_result
    }

    /// Emit the terminal "Done" status without sending a response first.
    ///
    /// Used by code paths that suppress the response (hook-blocked, empty
    /// response) but still need to close the turn for the client.
    async fn send_done(&self, message: &IncomingMessage) {
        if let Err(e) = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Status("Done".into()),
                &message.metadata,
            )
            .await
        {
            tracing::warn!(
                channel = %message.channel,
                error = %e,
                "Failed to send Done status"
            );
        }
    }

    pub(crate) fn llm(&self) -> &Arc<dyn LlmProvider> {
        &self.deps.llm
    }

    pub(crate) fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// Get the cheap/fast LLM provider, falling back to the main one.
    pub(crate) fn cheap_llm(&self) -> &Arc<dyn LlmProvider> {
        self.deps.cheap_llm.as_ref().unwrap_or(&self.deps.llm)
    }

    pub(crate) fn safety(&self) -> &Arc<SafetyLayer> {
        &self.deps.safety
    }

    pub(crate) fn tools(&self) -> &Arc<ToolRegistry> {
        &self.deps.tools
    }

    pub(crate) fn workspace(&self) -> Option<&Arc<Workspace>> {
        self.deps.workspace.as_ref()
    }

    pub(crate) fn hooks(&self) -> &Arc<HookRegistry> {
        &self.deps.hooks
    }

    /// Build platform metadata for self-awareness in system prompts.
    pub(crate) async fn platform_info(&self) -> ironclaw_engine::PlatformInfo {
        let active_channels = self.channels.channel_names().await;
        let database_backend = std::env::var("DATABASE_BACKEND")
            .ok()
            .or_else(|| self.deps.store.as_ref().map(|_| "postgres".to_string()));
        ironclaw_engine::PlatformInfo {
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            llm_backend: Some(self.deps.llm_backend.clone()),
            model_name: Some(self.deps.llm.active_model_name()),
            database_backend,
            active_channels,
            owner_id: Some(self.deps.owner_id.clone()),
            repo_url: Some("https://github.com/nearai/ironclaw".to_string()),
        }
    }
    /// Build a tenant-scoped execution context for the given user.
    ///
    /// This is the standard entry point for per-user operations. The returned
    /// [`TenantCtx`] provides a [`TenantScope`] that auto-binds `user_id` on
    /// every database operation and a per-user rate limiter.
    pub(super) async fn tenant_ctx(&self, user_id: &str) -> crate::tenant::TenantCtx {
        use crate::ownership::{Identity, OwnerId, UserRole};
        // Bridge: creates Member identity from raw string.
        // Will be replaced by OwnershipCache lookup in Task 9.
        let identity = Identity::new(OwnerId::from(user_id), UserRole::Member);
        self.tenant_ctx_with_identity(identity).await
    }

    /// Build a tenant-scoped execution context from a resolved `Identity`.
    ///
    /// Preferred over [`tenant_ctx`](Self::tenant_ctx) once the call site has a
    /// full `Identity` available.
    pub(super) async fn tenant_ctx_with_identity(
        &self,
        identity: crate::ownership::Identity,
    ) -> crate::tenant::TenantCtx {
        let user_id = identity.owner_id.as_str();
        let rate = self.deps.tenant_rates.get_or_create(user_id).await;

        let store = self.deps.store.as_ref().map(|db| {
            let scope = crate::tenant::TenantScope::with_identity(identity.clone(), Arc::clone(db));
            match &self.deps.settings_store {
                Some(ss) => scope.with_settings_store(Arc::clone(ss)),
                None => scope,
            }
        });

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
            identity,
            store,
            workspace,
            Arc::clone(&self.deps.cost_guard),
            rate,
        )
    }

    /// Get a system-scoped database accessor for cross-tenant operations.
    ///
    /// Only for system-level components (heartbeat, routine engine, self-repair,
    /// scheduler). Handler code should use [`tenant_ctx()`](Self::tenant_ctx) instead.
    pub(super) fn system_store(&self) -> Option<crate::tenant::SystemScope> {
        self.deps
            .store
            .as_ref()
            .map(|db| crate::tenant::SystemScope::new(Arc::clone(db)))
    }

    pub(super) fn skill_registry(&self) -> Option<&Arc<std::sync::RwLock<SkillRegistry>>> {
        self.deps.skill_registry.as_ref()
    }

    pub(super) fn skill_catalog(&self) -> Option<&Arc<ironclaw_skills::catalog::SkillCatalog>> {
        self.deps.skill_catalog.as_ref()
    }

    /// Select active skills for a message using deterministic prefiltering.
    /// Select skills for a message. Returns (active skills, rewritten message).
    ///
    /// Skills are selected in two ways:
    /// 1. **Explicit**: `/skill-name` in the message force-activates that skill.
    ///    The `/skill-name` is replaced with the skill's description so the
    ///    sentence reads naturally for the LLM.
    /// 2. **Implicit**: keyword/pattern scoring against the message content.
    ///
    /// One-time setup skills (`*-setup` persona bundles) declare a
    /// `setup_marker` workspace path in their activation frontmatter. Before
    /// scoring, we check the workspace for each distinct marker referenced
    /// by loaded skills and pass the satisfied set to the selector — any
    /// skill whose marker is present is excluded from candidates so it
    /// doesn't keep burning the activation budget after onboarding has
    /// already run. To re-trigger setup, delete the marker file.
    pub(super) async fn select_active_skills(
        &self,
        message_content: &str,
        user_id: &str,
    ) -> (Vec<ironclaw_skills::LoadedSkill>, String, Vec<String>) {
        let Some(registry) = self.skill_registry() else {
            return (vec![], message_content.to_string(), vec![]);
        };
        // Snapshot the skill list + distinct setup markers under the read
        // lock, then drop the guard before any await. The marker checks
        // and the prefilter call don't need the registry lock and we
        // shouldn't hold a poisonable RwLock across an await point.
        let (available, distinct_markers) = match registry.read() {
            Ok(guard) => {
                let skills_clone: Vec<ironclaw_skills::LoadedSkill> = guard.skills().to_vec();
                let mut markers: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for s in &skills_clone {
                    if let Some(m) = &s.manifest.activation.setup_marker {
                        markers.insert(m.clone());
                    }
                }
                (skills_clone, markers)
            }
            Err(e) => {
                tracing::error!("Skill registry lock poisoned: {}", e);
                return (vec![], message_content.to_string(), vec![]);
            }
        };

        // Resolve which setup markers are satisfied by the current
        // workspace. A marker is "satisfied" iff its path exists.
        // Without a workspace, we conservatively treat all markers as
        // unsatisfied (setup skills can still activate). Errors checking
        // a marker are logged and treated as unsatisfied.
        let mut satisfied: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Some(ws) = self.deps.workspace.as_ref() {
            // Scope the workspace to the requesting user so multi-user
            // channels check the correct user's marker state.
            let scoped_ws = if ws.user_id() == user_id {
                std::sync::Arc::clone(ws)
            } else {
                std::sync::Arc::new(ws.scoped_to_user(user_id))
            };
            for marker in &distinct_markers {
                match scoped_ws.exists(marker).await {
                    Ok(true) => {
                        satisfied.insert(marker.clone());
                    }
                    Ok(false) => {}
                    Err(e) => {
                        tracing::debug!(
                            marker = %marker,
                            "setup-marker existence check failed (treating as unsatisfied): {e}"
                        );
                    }
                }
            }
        }

        // Phase 1: Extract explicit /skill-name mentions
        let (explicit, rewritten) =
            ironclaw_skills::extract_skill_mentions(message_content, &available);

        // Phase 2: Score-based selection on the rewritten message
        let skills_cfg = &self.deps.skills_config;
        let outcome = ironclaw_skills::prefilter_skills(
            &rewritten,
            &available,
            skills_cfg.max_active_skills,
            skills_cfg.max_context_tokens,
            &satisfied,
        );

        // Feedback notes: start with the selector's own notes (chain-load,
        // budget, marker-skipped companions) and prepend a note for each
        // explicit `/mention` force-activation so the UI can explain why
        // a skill loaded even when it didn't score.
        let mut feedback: Vec<String> = explicit
            .iter()
            .map(|s| format!("{}: force-activated via /mention", s.name()))
            .collect();
        feedback.extend(outcome.notes);

        // Merge: explicit mentions first, then scored (dedup by name)
        let mut selected: Vec<ironclaw_skills::LoadedSkill> =
            explicit.into_iter().cloned().collect();
        for skill in outcome.selected {
            if !selected
                .iter()
                .any(|s| s.manifest.name == skill.manifest.name)
            {
                selected.push(skill.clone());
            }
        }

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

        (selected, rewritten, feedback)
    }

    /// Send initial engine thread list and routines to the TUI channel so
    /// the sidebar is populated before the first user message.
    async fn hydrate_tui_sidebar(&self) {
        let empty_meta = serde_json::Value::Object(serde_json::Map::new());

        // Engine threads
        if self.config.engine_v2
            && let Ok(threads) = crate::bridge::list_engine_threads(None, self.owner_id()).await
        {
            let summaries: Vec<crate::channels::EngineThreadSummary> = threads
                .into_iter()
                .map(|t| crate::channels::EngineThreadSummary {
                    id: t.id,
                    goal: t.goal,
                    thread_type: t.thread_type,
                    state: t.state,
                    step_count: t.step_count,
                    total_tokens: t.total_tokens,
                    created_at: t.created_at,
                    updated_at: t.updated_at,
                })
                .collect();
            let _ = self
                .channels
                .send_status(
                    "tui",
                    StatusUpdate::EngineThreadList { threads: summaries },
                    &empty_meta,
                )
                .await;
        }

        // Routines
        if let Some(system) = self.system_store()
            && let Ok(routines) = system.list_all_routines().await
        {
            for routine in routines {
                let _ = self
                    .channels
                    .send_status(
                        "tui",
                        StatusUpdate::RoutineUpdate {
                            id: routine.id.to_string(),
                            name: routine.name.clone(),
                            trigger_type: format!("{:?}", routine.trigger),
                            enabled: routine.enabled,
                            last_run: routine.last_run_at.map(|t| t.to_rfc3339()),
                            next_fire: routine.next_fire_at.map(|t| t.to_rfc3339()),
                        },
                        &empty_meta,
                    )
                    .await;
            }
        }
    }

    /// Run the agent main loop.
    pub async fn run(self) -> Result<(), Error> {
        // Eagerly initialize engine v2 so gateway API endpoints can serve
        // data (projects, missions, threads) before the first chat message.
        if self.config.engine_v2
            && let Err(e) = crate::bridge::init_engine(&self).await
        {
            tracing::debug!("engine v2: eager init failed: {e}");
        }

        // Start channels
        let mut message_stream = self.channels.start_all().await?;

        // Start self-repair task with notification forwarding
        let mut self_repair = DefaultSelfRepair::new(
            self.context_manager.clone(),
            self.config.stuck_threshold,
            self.config.max_repair_attempts,
        );
        if let Some(system) = self.system_store() {
            self_repair = self_repair.with_store(system);
        }
        if let Some(ref builder) = self.deps.builder {
            self_repair = self_repair.with_builder(Arc::clone(builder), Arc::clone(self.tools()));
        }
        let repair = Arc::new(self_repair);
        let repair_interval = self.config.repair_check_interval;
        let repair_channels = self.channels.clone();
        let repair_owner_id = self.owner_id().to_string();
        let repair_handle = tokio::spawn(async move {
            // Track jobs that have already been escalated to ManualRequired
            // to prevent sending duplicate notifications every repair cycle.
            let mut notified_manual: std::collections::HashSet<uuid::Uuid> =
                std::collections::HashSet::new();

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
                            // Dedup: only notify once per job (same pattern as ManualRequired)
                            if notified_manual.insert(job.job_id) {
                                Some(format!(
                                    "Job {} was stuck for {}s, recovery failed permanently: {}",
                                    job.job_id,
                                    job.stuck_duration.as_secs(),
                                    message
                                ))
                            } else {
                                None
                            }
                        }
                        Ok(RepairResult::ManualRequired { message }) => {
                            tracing::warn!("Manual intervention needed: {}", message);
                            // Only notify once per job to prevent notification spam.
                            // The job should have been transitioned to Failed by
                            // repair_stuck_job, but guard against that failing too.
                            if notified_manual.insert(job.job_id) {
                                Some(format!(
                                    "Job {} needs manual intervention: {}",
                                    job.job_id, message
                                ))
                            } else {
                                None
                            }
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
                        if let Some(system) = self.system_store() {
                            Some(spawn_multi_user_heartbeat(
                                config,
                                hygiene,
                                self.cheap_llm().clone(),
                                Some(notify_tx),
                                system,
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
                            self.system_store(),
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
                        crate::tenant::SystemScope::new(Arc::clone(store)),
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

        // Hydrate TUI sidebar with existing engine threads and routines so the
        // activity panel is populated before the first user message.
        self.hydrate_tui_sidebar().await;

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
                Ok(HandleOutcome::Respond(response)) => {
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
                            // Still send Done so the client knows the turn is complete
                            // even though the response was suppressed by the hook.
                            self.send_done(&message).await;
                        }
                        Ok(crate::hooks::HookOutcome::Continue {
                            modified: Some(new_content),
                        }) => {
                            if let Err(e) = self
                                .respond_then_done(&message, OutgoingResponse::text(new_content))
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
                                .respond_then_done(&message, OutgoingResponse::text(response))
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
                Ok(HandleOutcome::NoResponse) => {
                    // Empty response (e.g. routine consumed the message, silent reply).
                    // Send Done so the client knows the turn is complete.
                    tracing::debug!(
                        channel = %message.channel,
                        user = %message.user_id,
                        "Suppressed empty response (not sent to channel)"
                    );
                    self.send_done(&message).await;
                }
                Ok(HandleOutcome::Pending) => {
                    // Turn paused awaiting user action (approval, auth, etc).
                    // Do NOT emit Done — the thread is not in a terminal state.
                    // The relevant ApprovalNeeded/AuthRequired status was already
                    // sent by the inner handler before returning.
                    tracing::debug!(
                        channel = %message.channel,
                        user = %message.user_id,
                        "Turn paused (Pending); suppressing Done"
                    );
                }
                Ok(HandleOutcome::Shutdown) => {
                    // Shutdown signal received (/quit, /exit, /shutdown)
                    tracing::debug!("Shutdown command received, exiting...");
                    break;
                }
                Err(e) => {
                    tracing::error!("Error handling message: {}", e);
                    if let Err(send_err) = self
                        .respond_then_done(
                            &message,
                            OutgoingResponse::text(format!("Error: {}", e)),
                        )
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

            // Refresh engine v2 thread list in the TUI sidebar after each turn.
            if self.config.engine_v2
                && let Ok(threads) =
                    crate::bridge::list_engine_threads(None, &message.user_id).await
            {
                let summaries: Vec<crate::channels::EngineThreadSummary> = threads
                    .into_iter()
                    .map(|t| crate::channels::EngineThreadSummary {
                        id: t.id,
                        goal: t.goal,
                        thread_type: t.thread_type,
                        state: t.state,
                        step_count: t.step_count,
                        total_tokens: t.total_tokens,
                        created_at: t.created_at,
                        updated_at: t.updated_at,
                    })
                    .collect();
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::EngineThreadList { threads: summaries },
                        &message.metadata,
                    )
                    .await;
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

    async fn handle_message(&self, message: &IncomingMessage) -> Result<HandleOutcome, Error> {
        // Log sensitive details at debug level for troubleshooting
        tracing::debug!(
            message_id = %message.id,
            user_id = %message.user_id,
            channel = %message.channel,
            thread_id = ?message.thread_id,
            "Message details"
        );

        // Internal messages (e.g. job-monitor notifications) are already
        // rendered text and should be forwarded directly to the user without
        // entering the normal user-input pipeline (LLM/tool loop).
        // The `is_internal` field and `into_internal()` setter are pub(crate),
        // so external channels cannot spoof this flag.
        if message.is_internal {
            tracing::debug!(
                message_id = %message.id,
                channel = %message.channel,
                "Forwarding internal message"
            );
            return Ok(HandleOutcome::Respond(message.content.clone()));
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
        let mut submission = message
            .structured_submission
            .clone()
            .unwrap_or_else(|| SubmissionParser::parse(&message.content));
        tracing::trace!(
            "[agent_loop] Parsed submission: {:?}",
            std::any::type_name_of_val(&submission)
        );

        // Engine V2 early downgrade: bare-keyword ApprovalResponse → UserInput
        // when no approval gate or auth flow is pending. Done before the
        // BeforeInbound hook check so the downgraded message flows through
        // the full UserInput pipeline (hooks, drain loop, etc.).
        // Only applies to engine_v2 because the legacy path needs session/
        // thread state (not yet resolved) to determine AwaitingApproval.
        if self.config.engine_v2
            && matches!(&submission, Submission::ApprovalResponse { .. })
            && !message.content.trim().starts_with('/')
        {
            let has_pending = crate::bridge::has_pending_auth(&message.user_id).await
                || crate::bridge::has_any_pending_gate(
                    &message.user_id,
                    message.conversation_scope(),
                )
                .await;
            if !has_pending {
                submission = Submission::UserInput {
                    content: message.content.clone(),
                };
            }
        }

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
                    return Ok(HandleOutcome::Respond(format!(
                        "[Message rejected: {}]",
                        reason
                    )));
                }
                Err(err) => {
                    return Ok(HandleOutcome::Respond(format!(
                        "[Message blocked by hook policy: {}]",
                        err
                    )));
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

        // Engine V2 routing (Strategy C: parallel deployment).
        // Bridge handlers return BridgeOutcome which maps directly to
        // HandleOutcome — gate status is encoded in the return type, not
        // queried post-hoc.
        if self.config.engine_v2 {
            match &submission {
                Submission::UserInput { content } => {
                    return crate::bridge::handle_with_engine(self, message, content)
                        .await
                        .map(HandleOutcome::from);
                }
                Submission::ApprovalResponse { approved, always } => {
                    // Reaching here means the message is a slash command (/approve,
                    // /deny) or has a pending gate/auth — early downgrade above
                    // already handled the bare-keyword-with-no-gate case.
                    if crate::bridge::has_pending_auth(&message.user_id).await {
                        let content = &message.content;
                        return crate::bridge::handle_with_engine(self, message, content)
                            .await
                            .map(HandleOutcome::from);
                    }
                    return crate::bridge::handle_approval(self, message, *approved, *always)
                        .await
                        .map(HandleOutcome::from);
                }
                Submission::ExecApproval {
                    request_id,
                    approved,
                    always,
                } => {
                    return crate::bridge::handle_exec_approval(
                        self,
                        message,
                        *request_id,
                        *approved,
                        *always,
                    )
                    .await
                    .map(HandleOutcome::from);
                }
                Submission::ExternalCallback { request_id } => {
                    return crate::bridge::handle_external_callback(self, message, *request_id)
                        .await
                        .map(HandleOutcome::from);
                }
                Submission::GateAuthResolution {
                    request_id,
                    resolution,
                } => {
                    return crate::bridge::handle_auth_gate_resolution(
                        self,
                        message,
                        *request_id,
                        resolution.clone(),
                    )
                    .await
                    .map(HandleOutcome::from);
                }
                Submission::Interrupt => {
                    return crate::bridge::handle_interrupt(self, message)
                        .await
                        .map(HandleOutcome::from);
                }
                Submission::NewThread => {
                    return crate::bridge::handle_new_thread(self, message)
                        .await
                        .map(HandleOutcome::from);
                }
                Submission::Clear => {
                    return crate::bridge::handle_clear(self, message)
                        .await
                        .map(HandleOutcome::from);
                }
                Submission::Expected { description } => {
                    return crate::bridge::handle_expected(self, message, description)
                        .await
                        .map(HandleOutcome::from);
                }
                // Undo/Redo/Resume/SwitchThread: v1-only (engine has no undo;
                // thread switching is implicit via ConversationManager).
                // Compact/Summarize/Suggest: orthogonal to engine (compaction is internal).
                // Heartbeat/SystemCommand/JobStatus/JobCancel/Quit: v1 infrastructure.
                _ => {}
            }
        }

        // V2-only structured submissions must fail before any session/thread
        // resolution on the legacy path. Otherwise a crafted request can
        // switch the active thread via conversation_scope before returning the
        // expected ENGINE_V2 error.
        if !self.config.engine_v2 {
            match submission {
                Submission::ExternalCallback { .. } => {
                    return Ok(HandleOutcome::Respond(
                        "Error: External callbacks require ENGINE_V2".to_string(),
                    ));
                }
                Submission::GateAuthResolution { .. } => {
                    return Ok(HandleOutcome::Respond(
                        "Error: Auth gate resolution requires ENGINE_V2".to_string(),
                    ));
                }
                _ => {}
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
                return Ok(HandleOutcome::Respond(format!("Error: {}", rejection)));
            }
        }

        // Resolve session and thread. Approval submissions are allowed to
        // target an already-loaded owned thread by UUID across channels so the
        // web approval UI can approve work that originated from HTTP/other
        // owner-scoped channels.
        let approval_thread_uuid = if matches!(
            submission,
            Submission::ExecApproval { .. }
                | Submission::ApprovalResponse { .. }
                | Submission::ExternalCallback { .. }
                | Submission::GateAuthResolution { .. }
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
            if let Some(thread) = sess.threads.get(&target_thread_id) {
                // Block ExecApproval (JSON from the approval UI) when there
                // is no pending approval — prevents hijacking a thread by UUID.
                // ApprovalResponse (bare keywords) is allowed through so the
                // should_route_as_approval guard can downgrade it to UserInput.
                if thread.pending_approval.is_none()
                    && matches!(submission, Submission::ExecApproval { .. })
                {
                    tracing::warn!(
                        %target_thread_id,
                        approval_channel = %message.channel,
                        "Blocked approval for thread with no pending approval"
                    );
                    drop(sess);
                    return Ok(HandleOutcome::Respond(
                        "Error: no pending approval on this thread".into(),
                    ));
                }
                // ApprovalResponse (bare "yes"/"no"/"always") without a
                // pending approval: fall through to normal handling so the
                // should_route_as_approval guard can downgrade to UserInput.
                // Skip the cross-channel auth check — it only matters for
                // actual approvals, not messages about to become UserInput.

                if thread.pending_approval.is_some() {
                    let authorized = crate::agent::session::is_approval_authorized(
                        thread.source_channel.as_deref(),
                        &message.channel,
                    );
                    if !authorized {
                        tracing::warn!(
                            %target_thread_id,
                            source_channel = ?thread.source_channel,
                            approval_channel = %message.channel,
                            "Blocked cross-channel approval attempt"
                        );
                        drop(sess);
                        return Ok(HandleOutcome::Respond(
                            "Error: approval not authorized for this channel".into(),
                        ));
                    }
                }
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
                    return Ok(HandleOutcome::Respond(format!(
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
                            .await
                            .map(HandleOutcome::from_legacy);
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

        if !message.is_internal
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
                    Ok(HandleOutcome::Shutdown)
                } else {
                    Ok(HandleOutcome::NoResponse)
                };
            }
        }

        // Build per-tenant execution context once; threaded through all handlers.
        let tenant = self.tenant_ctx(&message.user_id).await;

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
                        .respond_then_done(message, OutgoingResponse::text(outgoing.clone()))
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
                        SubmissionResult::Response { content } => {
                            Ok(HandleOutcome::Respond(content))
                        }
                        SubmissionResult::Ok { message } => Ok(HandleOutcome::from_legacy(message)),
                        SubmissionResult::Error { message } => {
                            Ok(HandleOutcome::Respond(format!("Error: {}", message)))
                        }
                        _ => {
                            if is_single_message_repl(message) {
                                Ok(HandleOutcome::Shutdown)
                            } else {
                                Ok(HandleOutcome::NoResponse)
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
            Submission::Expected { description } => {
                self.process_expected(session, thread_id, &description, &message.user_id)
                    .await
            }
            Submission::JobStatus { job_id } => {
                self.process_job_status(&tenant, job_id.as_deref()).await
            }
            Submission::JobCancel { job_id } => self.process_job_cancel(&tenant, &job_id).await,
            Submission::Quit => return Ok(HandleOutcome::Shutdown),
            Submission::SwitchThread { thread_id: target } => {
                self.process_switch_thread(message, target).await
            }
            Submission::Resume { checkpoint_id } => {
                self.process_resume(session, thread_id, checkpoint_id).await
            }
            Submission::ListThreads => self.process_list_threads(session, message).await,
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
            Submission::ExternalCallback { .. } => Ok(SubmissionResult::Error {
                message: "External callbacks require ENGINE_V2".to_string(),
            }),
            Submission::GateAuthResolution { .. } => Ok(SubmissionResult::Error {
                message: "Auth gate resolution requires ENGINE_V2".to_string(),
            }),
            Submission::ApprovalResponse { approved, always } => {
                let thread_state = {
                    let sess = session.lock().await;
                    sess.threads
                        .get(&thread_id)
                        .map(|t| t.state)
                        .unwrap_or(ThreadState::Idle)
                };
                // NOTE: TOCTOU possible — state could change between check
                // and process_approval; process_approval handles stale cases.
                if should_route_as_approval(thread_state, &message.content) {
                    self.process_approval(message, session, thread_id, None, approved, always)
                        .await
                } else {
                    // Run BeforeInbound hooks for the downgraded content —
                    // the hook check above only fires for UserInput submissions,
                    // and this was parsed as ApprovalResponse.
                    let content = message.content.clone();
                    let hook_event = crate::hooks::HookEvent::Inbound {
                        user_id: message.user_id.clone(),
                        channel: message.channel.clone(),
                        content: content.clone(),
                        thread_id: message.thread_id.clone(),
                    };
                    let content = match self.hooks().run(&hook_event).await {
                        Err(crate::hooks::HookError::Rejected { reason }) => {
                            // Match the main UserInput path's rejection behavior.
                            return Ok(HandleOutcome::Respond(format!(
                                "[Message rejected: {reason}]"
                            )));
                        }
                        Err(err) => {
                            // Match the main UserInput path's error behavior.
                            return Ok(HandleOutcome::Respond(format!(
                                "[Message blocked by hook policy: {err}]"
                            )));
                        }
                        Ok(crate::hooks::HookOutcome::Continue {
                            modified: Some(new_content),
                        }) => new_content,
                        _ => content, // Continue — no modification
                    };

                    // Process as user input with the drain loop so queued
                    // messages during processing are merged, matching the
                    // Submission::UserInput arm's behavior.
                    let mut result = self
                        .process_user_input(
                            message,
                            tenant.clone(),
                            session.clone(),
                            thread_id,
                            &content,
                        )
                        .await;

                    while let Ok(SubmissionResult::Response { content: outgoing }) = &result {
                        let merged = {
                            let mut sess = session.lock().await;
                            sess.threads
                                .get_mut(&thread_id)
                                .and_then(|thread| thread.drain_pending_messages())
                        };
                        let Some(next_content) = merged else {
                            break;
                        };

                        if let Err(e) = self
                            .respond_then_done(message, OutgoingResponse::text(outgoing.clone()))
                            .await
                        {
                            tracing::warn!(
                                %thread_id,
                                "Failed to send intermediate drain-loop response: {e}"
                            );
                        }

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

                        if !matches!(&result, Ok(SubmissionResult::Response { .. })) {
                            let mut sess = session.lock().await;
                            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                                thread.requeue_drained(next_content);
                            }
                        }
                    }

                    result
                }
            }
            Submission::Plan { sub } => {
                use crate::agent::submission::PlanSubcommand;
                let rewritten = match sub {
                    PlanSubcommand::Create { description } => {
                        format!("[PLAN MODE] Create a plan for: {description}")
                    }
                    PlanSubcommand::Approve { plan_ref } => {
                        let r = plan_ref.as_deref().unwrap_or("the most recent plan");
                        format!(
                            "[PLAN MODE] Approve and execute plan {r}. \
                             Create a mission from the plan content using mission_create, \
                             then fire it with mission_fire."
                        )
                    }
                    PlanSubcommand::Status { plan_ref } => {
                        let r = plan_ref.as_deref().unwrap_or("the most recent plan");
                        format!(
                            "[PLAN MODE] Show status of plan {r}. \
                             Check the associated mission's thread_history, \
                             current_focus, and approach_history."
                        )
                    }
                    PlanSubcommand::Revise { plan_ref, feedback } => {
                        let r = plan_ref.as_deref().unwrap_or("the most recent plan");
                        format!("[PLAN MODE] Revise plan {r} based on: {feedback}")
                    }
                    PlanSubcommand::List => {
                        "[PLAN MODE] List all plans. Search memory for plan documents \
                         and show their status."
                            .to_string()
                    }
                };
                self.process_user_input(message, tenant, session, thread_id, &rewritten)
                    .await
            }
        };

        // Convert SubmissionResult to a HandleOutcome.
        match result? {
            SubmissionResult::Response { content } => {
                // Suppress silent replies (e.g. from group chat "nothing to say" responses).
                // Silent replies exit single-message REPL invocations.
                if crate::llm::is_silent_reply(&content) {
                    tracing::debug!("Suppressing silent reply token");
                    Ok(HandleOutcome::Shutdown)
                } else if content.is_empty() {
                    Ok(HandleOutcome::NoResponse)
                } else {
                    Ok(HandleOutcome::Respond(content))
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
                    Ok(HandleOutcome::Shutdown)
                } else {
                    Ok(HandleOutcome::from_legacy(output_message))
                }
            }
            SubmissionResult::Error { message } => {
                Ok(HandleOutcome::Respond(format!("Error: {}", message)))
            }
            SubmissionResult::Interrupted => Ok(HandleOutcome::Respond("Interrupted.".into())),
            SubmissionResult::AuthPending => {
                // Auth-required status already sent by handle_auth_intercept.
                // Thread is in auth mode — suppress text response and Done.
                Ok(HandleOutcome::Pending)
            }
            SubmissionResult::NeedApproval { .. } => {
                // ApprovalNeeded status was already sent by thread_ops.rs before
                // returning this result. The thread is now in AwaitingApproval —
                // do NOT emit a terminal Done because the turn is paused, not
                // complete. Sending Done here would also trip the web UI's
                // missing-response safety net (see #2079).
                Ok(HandleOutcome::Pending)
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
    use crate::agent::agent_loop::{Agent, AgentDeps, HandleOutcome};
    use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
    use crate::agent::submission::{AuthGateResolution, Submission};
    use crate::channels::IncomingMessage;
    use crate::error::ChannelError;
    use crate::hooks::HookRegistry;
    use crate::tools::ToolRegistry;
    use crate::{
        config::{AgentConfig, SafetyConfig, SkillsConfig},
        llm::{
            CompletionRequest, CompletionResponse, FinishReason, LlmProvider,
            ToolCompletionRequest, ToolCompletionResponse,
        },
    };
    use ironclaw_safety::SafetyLayer;
    use rust_decimal::Decimal;
    use std::sync::Arc;
    use std::time::Duration;
    use uuid::Uuid;

    struct StaticLlmProvider;

    #[async_trait::async_trait]
    impl LlmProvider for StaticLlmProvider {
        fn model_name(&self) -> &str {
            "static-mock"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            Ok(CompletionResponse {
                content: "ok".to_string(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("ok".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    fn make_legacy_handle_message_test_agent() -> Agent {
        let deps = AgentDeps {
            owner_id: "default".to_string(),
            store: None,
            settings_store: None,
            llm: Arc::new(StaticLlmProvider),
            cheap_llm: None,
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: true,
            })),
            tools: Arc::new(ToolRegistry::new()),
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks: Arc::new(HookRegistry::new()),
            auth_manager: None,
            cost_guard: Arc::new(CostGuard::new(CostGuardConfig::default())),
            sse_tx: None,
            http_interceptor: None,
            transcription: None,
            document_extraction: None,
            sandbox_readiness: crate::agent::routine_engine::SandboxReadiness::DisabledByConfig,
            builder: None,
            llm_backend: "nearai".to_string(),
            tenant_rates: Arc::new(crate::tenant::TenantRateRegistry::new(4, 3)),
        };

        Agent::new(
            AgentConfig {
                name: "agent-loop-test-agent".to_string(),
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
                max_cost_per_user_per_day_cents: None,
                max_tool_iterations: 50,
                auto_approve_tools: false,
                default_timezone: "UTC".to_string(),
                max_jobs_per_user: None,
                max_tokens_per_job: 0,
                multi_tenant: false,
                max_llm_concurrent_per_user: None,
                max_jobs_concurrent_per_user: None,
                engine_v2: false,
            },
            deps,
            Arc::new(crate::channels::ChannelManager::new()),
            None,
            None,
            None,
            Some(Arc::new(crate::context::ContextManager::new(1))),
            None,
        )
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

    /// Regression: bare "yes"/"no" when thread is Idle must NOT route as
    /// approval. Exercises the `should_route_as_approval` guard that the
    /// legacy match arm uses to decide between `process_approval` and
    /// `process_user_input`.
    #[test]
    fn should_route_as_approval_rejects_bare_keywords_when_idle() {
        use super::should_route_as_approval;
        use crate::agent::session::ThreadState;

        // Bare keywords with non-approval thread states → downgrade to UserInput
        for state in [
            ThreadState::Idle,
            ThreadState::Processing,
            ThreadState::Completed,
            ThreadState::Interrupted,
        ] {
            assert!(
                !should_route_as_approval(state, "yes"),
                "bare 'yes' should not route as approval in {state:?}"
            );
            assert!(
                !should_route_as_approval(state, "no"),
                "bare 'no' should not route as approval in {state:?}"
            );
            assert!(
                !should_route_as_approval(state, "always"),
                "bare 'always' should not route as approval in {state:?}"
            );
            assert!(
                !should_route_as_approval(state, "ok"),
                "bare 'ok' should not route as approval in {state:?}"
            );
        }
    }

    /// When thread IS AwaitingApproval, bare keywords must route as approval.
    #[test]
    fn should_route_as_approval_accepts_keywords_when_awaiting() {
        use super::should_route_as_approval;
        use crate::agent::session::ThreadState;

        assert!(should_route_as_approval(
            ThreadState::AwaitingApproval,
            "yes"
        ));
        assert!(should_route_as_approval(
            ThreadState::AwaitingApproval,
            "no"
        ));
        assert!(should_route_as_approval(
            ThreadState::AwaitingApproval,
            "always"
        ));
    }

    /// Slash commands (/approve, /deny) always route as approval regardless
    /// of thread state — they are explicit user intent.
    #[test]
    fn should_route_as_approval_always_routes_slash_commands() {
        use super::should_route_as_approval;
        use crate::agent::session::ThreadState;

        assert!(should_route_as_approval(ThreadState::Idle, "/approve"));
        assert!(should_route_as_approval(ThreadState::Idle, "/deny"));
        assert!(should_route_as_approval(ThreadState::Idle, "/yes"));
        assert!(should_route_as_approval(ThreadState::Processing, "/always"));
    }

    /// The thread-resolution guard must only early-reject `ExecApproval`
    /// when no approval is pending. `ApprovalResponse` (bare keywords)
    /// must be allowed through so `should_route_as_approval` can downgrade
    /// them to `UserInput`. Regression test for the E2E timeout where
    /// bare "yes"/"no" via API never produced a response in history.
    #[test]
    fn approval_guard_rejects_exec_but_allows_bare_keywords_when_no_pending() {
        use crate::agent::submission::Submission;
        use uuid::Uuid;

        // Simulate: thread has no pending approval, submission is ApprovalResponse
        let pending_approval: Option<()> = None;
        let submission_approval_response = Submission::ApprovalResponse {
            approved: true,
            always: false,
        };

        // ApprovalResponse should NOT be blocked (falls through to downgrade)
        let blocked = pending_approval.is_none()
            && matches!(
                submission_approval_response,
                Submission::ExecApproval { .. }
            );
        assert!(
            !blocked,
            "ApprovalResponse with no pending approval must not be early-rejected"
        );

        // ExecApproval SHOULD be blocked
        let submission_exec = Submission::ExecApproval {
            request_id: Uuid::new_v4(),
            approved: true,
            always: false,
        };
        let blocked = pending_approval.is_none()
            && matches!(submission_exec, Submission::ExecApproval { .. });
        assert!(
            blocked,
            "ExecApproval with no pending approval must be early-rejected"
        );

        // When pending approval IS present, neither should be early-rejected
        let pending_approval: Option<()> = Some(());
        let submission_exec2 = Submission::ExecApproval {
            request_id: Uuid::new_v4(),
            approved: true,
            always: false,
        };
        let blocked = pending_approval.is_none()
            && matches!(submission_exec2, Submission::ExecApproval { .. });
        assert!(
            !blocked,
            "ExecApproval with pending approval must not be early-rejected"
        );
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

    #[tokio::test]
    async fn v2_only_structured_submissions_do_not_switch_threads_when_engine_v2_disabled() {
        let agent = make_legacy_handle_message_test_agent();
        let session = agent.session_manager.get_or_create_session("alice").await;
        let active_thread_id = Uuid::new_v4();
        let target_thread_id = Uuid::new_v4();

        {
            let mut sess = session.lock().await;
            sess.create_thread_with_id(active_thread_id, Some("gateway"));
            sess.create_thread_with_id(target_thread_id, Some("gateway"));
            sess.active_thread = Some(active_thread_id);
        }

        agent
            .session_manager
            .register_thread("alice", "gateway", active_thread_id, Arc::clone(&session))
            .await;
        agent
            .session_manager
            .register_thread("alice", "gateway", target_thread_id, Arc::clone(&session))
            .await;

        let gate_resolution = serde_json::to_string(&Submission::GateAuthResolution {
            request_id: Uuid::new_v4(),
            resolution: AuthGateResolution::Cancelled,
        })
        .expect("serialize gate resolution");
        let gate_message = IncomingMessage::new("gateway", "alice", &gate_resolution)
            .with_thread(target_thread_id);

        let outcome = agent
            .handle_message(&gate_message)
            .await
            .expect("handle message");
        assert!(matches!(
            outcome,
            HandleOutcome::Respond(ref msg) if msg == "Error: Auth gate resolution requires ENGINE_V2"
        ));
        {
            let sess = session.lock().await;
            assert_eq!(sess.active_thread, Some(active_thread_id));
        }

        let callback = serde_json::to_string(&Submission::ExternalCallback {
            request_id: Uuid::new_v4(),
        })
        .expect("serialize external callback");
        let callback_message =
            IncomingMessage::new("gateway", "alice", &callback).with_thread(target_thread_id);

        let outcome = agent
            .handle_message(&callback_message)
            .await
            .expect("handle callback");
        assert!(matches!(
            outcome,
            HandleOutcome::Respond(ref msg) if msg == "Error: External callbacks require ENGINE_V2"
        ));
        {
            let sess = session.lock().await;
            assert_eq!(sess.active_thread, Some(active_thread_id));
        }
    }
}
