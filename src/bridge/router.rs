//! Engine v2 router — handles user messages via the engine when enabled.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use tokio::sync::RwLock;
use tracing::debug;

use ironclaw_engine::{
    Capability, CapabilityRegistry, ConversationManager, LeaseManager, MissionManager,
    PolicyEngine, Project, Store, ThreadConfig, ThreadManager, ThreadOutcome,
};

use ironclaw_common::AppEvent;

use crate::agent::Agent;
use crate::bridge::effect_adapter::EffectBridgeAdapter;
use crate::bridge::llm_adapter::LlmBridgeAdapter;
use crate::bridge::store_adapter::HybridStore;
use crate::channels::web::sse::SseManager;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::db::Database;
use crate::error::Error;

/// Check if the engine v2 is enabled via `ENGINE_V2=true` environment variable.
pub fn is_engine_v2_enabled() -> bool {
    std::env::var("ENGINE_V2")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false)
}

/// Shorthand for building an `Error` from an engine-related failure.
fn engine_err(context: &str, e: impl std::fmt::Display) -> Error {
    Error::from(crate::error::JobError::ContextError {
        id: uuid::Uuid::nil(),
        reason: format!("engine v2 {context}: {e}"),
    })
}

/// Pending approval info stored between the NeedApproval outcome and the user's response.
#[derive(Clone)]
struct PendingApproval {
    request_id: String,
    action_name: String,
    thread_id: ironclaw_engine::ThreadId,
    conversation_id: ironclaw_engine::ConversationId,
    call_id: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct PendingApprovalView {
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    pub parameters: String,
}

/// Pending credential auth: the next user message is treated as a token value.
#[derive(Clone)]
struct PendingAuth {
    credential_name: String,
    /// The original user message to retry after token is stored.
    original_message: String,
    user_id: String,
    channel: String,
    metadata: serde_json::Value,
}

/// Persistent engine state that lives across messages.
struct EngineState {
    thread_manager: Arc<ThreadManager>,
    conversation_manager: ConversationManager,
    effect_adapter: Arc<EffectBridgeAdapter>,
    store: Arc<dyn Store>,
    default_project_id: ironclaw_engine::ProjectId,
    /// Per-user pending approvals (keyed by user_id).
    pending_approvals: RwLock<HashMap<String, PendingApproval>>,
    /// Per-user pending credential auth (keyed by user_id).
    pending_auth: RwLock<HashMap<String, PendingAuth>>,
    /// SSE manager for broadcasting AppEvents to the web gateway.
    sse: Option<Arc<SseManager>>,
    /// V1 database for writing conversation messages (gateway reads from here).
    db: Option<Arc<dyn Database>>,
    /// Secrets store for storing credentials after auth flow.
    secrets_store: Option<Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
}

/// Global engine state, initialized on first use.
static ENGINE_STATE: OnceLock<RwLock<Option<EngineState>>> = OnceLock::new();

const PENDING_APPROVAL_METADATA_KEY: &str = "pending_approval";

enum PendingApprovalResolution {
    None,
    Resolved(PendingApproval),
    Ambiguous,
}

/// Get or initialize the engine state using the agent's dependencies.
///
/// Called eagerly at startup (from `Agent::run()`) when `ENGINE_V2=true`,
/// and defensively from each handler as a lazy fallback.
pub async fn init_engine(agent: &Agent) -> Result<(), Error> {
    let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
    let guard = lock.read().await;
    if guard.is_some() {
        return Ok(());
    }
    drop(guard);

    // Initialize
    let mut guard = lock.write().await;
    if guard.is_some() {
        return Ok(()); // double-check after acquiring write lock
    }

    debug!("engine v2: initializing engine state");

    let llm_adapter = Arc::new(LlmBridgeAdapter::new(
        agent.llm().clone(),
        Some(agent.cheap_llm().clone()),
    ));

    let effect_adapter = Arc::new(EffectBridgeAdapter::new(
        agent.tools().clone(),
        agent.safety().clone(),
        agent.hooks().clone(),
    ));

    // Wire auth_required callback: emits SSE event when a credential is missing.
    // Best-effort — if no frontend is connected, the event is silently dropped.
    if let Some(sse) = agent.deps.sse_tx.clone() {
        let sse_for_auth = sse;
        effect_adapter
            .set_auth_required_callback(Arc::new(Box::new(move |credential_name, action_name| {
                let event = ironclaw_common::AppEvent::AuthRequired {
                    extension_name: credential_name.to_string(),
                    instructions: Some(format!(
                        "Tool '{}' needs the '{}' credential. Please authenticate to continue.",
                        action_name, credential_name
                    )),
                    auth_url: None,
                    setup_url: None,
                };
                sse_for_auth.broadcast(event);
            })))
            .await;
    }

    let store = Arc::new(HybridStore::new(agent.workspace().cloned()));
    store.load_state_from_workspace().await;

    // Clean up completed threads and dead leases from prior runs
    let cleaned = store
        .cleanup_terminal_state(chrono::Duration::minutes(5))
        .await;
    if cleaned > 0 {
        debug!("engine v2: cleaned {cleaned} terminal state entries on startup");
    }

    // Generate the engine workspace README
    store.generate_engine_readme().await;

    // Build capability registry from available tools
    let mut capabilities = CapabilityRegistry::new();
    let tool_defs = agent.tools().tool_definitions().await;
    if !tool_defs.is_empty() {
        capabilities.register(Capability {
            name: "tools".into(),
            description: "Available tools".into(),
            actions: tool_defs
                .into_iter()
                .map(|td| ironclaw_engine::ActionDef {
                    name: td.name.replace('-', "_"),
                    description: td.description,
                    parameters_schema: td.parameters,
                    effects: vec![],
                    requires_approval: false,
                })
                .collect(),
            knowledge: vec![],
            policies: vec![],
        });
    }

    // Register mission functions as a capability so threads receive leases.
    // Handled by EffectBridgeAdapter::handle_mission_call() before the
    // regular tool executor. Use "mission_*" names only — descriptions
    // mention "routine" so the LLM maps user intent correctly.
    capabilities.register(Capability {
        name: "missions".into(),
        description: "Mission and routine lifecycle management".into(),
        actions: vec![
            ironclaw_engine::ActionDef {
                name: "mission_create".into(),
                description: "Create a new mission (routine). Use when the user wants to set up a recurring task, scheduled check, or periodic routine.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Short name for the mission/routine"},
                        "goal": {"type": "string", "description": "What this mission should accomplish each run"},
                        "cadence": {"type": "string", "description": "How often to run: 'hourly', '30m', '6h', 'daily', 'manual'"}
                    },
                    "required": ["name", "goal"]
                }),
                effects: vec![],
                requires_approval: false,
            },
            ironclaw_engine::ActionDef {
                name: "mission_list".into(),
                description: "List all missions and routines in the current project.".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![],
                requires_approval: false,
            },
            ironclaw_engine::ActionDef {
                name: "mission_fire".into(),
                description: "Manually trigger a mission or routine to run immediately.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Mission/routine ID to trigger"}
                    },
                    "required": ["id"]
                }),
                effects: vec![],
                requires_approval: false,
            },
            ironclaw_engine::ActionDef {
                name: "mission_pause".into(),
                description: "Pause a running mission or routine.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Mission/routine ID to pause"}
                    },
                    "required": ["id"]
                }),
                effects: vec![],
                requires_approval: false,
            },
            ironclaw_engine::ActionDef {
                name: "mission_resume".into(),
                description: "Resume a paused mission or routine.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Mission/routine ID to resume"}
                    },
                    "required": ["id"]
                }),
                effects: vec![],
                requires_approval: false,
            },
            ironclaw_engine::ActionDef {
                name: "mission_delete".into(),
                description: "Delete a mission or routine permanently.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Mission/routine ID to delete"}
                    },
                    "required": ["id"]
                }),
                effects: vec![],
                requires_approval: false,
            },
        ],
        knowledge: vec![],
        policies: vec![],
    });

    let leases = Arc::new(LeaseManager::new());
    let policy = Arc::new(PolicyEngine::new());

    let store_dyn: Arc<dyn Store> = store.clone();

    let thread_manager = Arc::new(ThreadManager::new(
        llm_adapter,
        effect_adapter.clone(),
        store_dyn.clone(),
        Arc::new(capabilities),
        leases,
        policy,
    ));

    // Reuse the persisted default project when available.
    let project_id = match store
        .list_projects()
        .await
        .map_err(|e| engine_err("store error", e))?
        .into_iter()
        .find(|project| project.name == "default")
    {
        Some(project) => project.id,
        None => {
            let project = Project::new("default", "Default project for engine v2");
            let project_id = project.id;
            store
                .save_project(&project)
                .await
                .map_err(|e| engine_err("store error", e))?;
            project_id
        }
    };

    let conversation_manager = ConversationManager::new(Arc::clone(&thread_manager), store.clone());
    if let Err(e) = conversation_manager
        .bootstrap_user(&agent.deps.owner_id)
        .await
    {
        debug!("engine v2: bootstrap_user failed: {e}");
    }

    // Create mission manager and start cron ticker
    let mission_manager = Arc::new(MissionManager::new(
        store_dyn.clone(),
        Arc::clone(&thread_manager),
    ));
    if let Err(e) = thread_manager.recover_project_threads(project_id).await {
        debug!("engine v2: recover_project_threads failed: {e}");
    }
    if let Err(e) = mission_manager.bootstrap_project(project_id).await {
        debug!("engine v2: bootstrap_project failed: {e}");
    }
    if let Err(e) = mission_manager
        .resume_recoverable_threads(&agent.deps.owner_id)
        .await
    {
        debug!("engine v2: resume_recoverable_threads failed: {e}");
    }
    if let Err(e) = thread_manager.resume_background_threads(project_id).await {
        debug!("engine v2: resume_background_threads failed: {e}");
    }
    mission_manager.start_cron_ticker(agent.deps.owner_id.clone());
    mission_manager.start_event_listener(agent.deps.owner_id.clone());

    // Ensure all learning missions exist for this project
    if let Err(e) = mission_manager.ensure_learning_missions(project_id).await {
        debug!("engine v2: failed to create learning missions: {e}");
    }

    // Migrate v1 skills to v2 MemoryDocs (skill selection happens in the
    // Python orchestrator at runtime via __list_skills__).
    if let Some(registry) = agent.deps.skill_registry.as_ref() {
        let skills_snapshot = {
            let guard = registry
                .read()
                .map_err(|e| engine_err("skill registry", format!("lock poisoned: {e}")))?;
            guard.skills().to_vec()
        };
        if !skills_snapshot.is_empty() {
            match crate::bridge::skill_migration::migrate_v1_skill_list(
                &skills_snapshot,
                &store_dyn,
                project_id,
            )
            .await
            {
                Ok(count) if count > 0 => {
                    debug!("engine v2: migrated {count} v1 skill(s)");
                }
                Err(e) => {
                    debug!("engine v2: skill migration failed: {e}");
                }
                _ => {}
            }
        }
    }

    // Wire mission manager into effect adapter for mission_* function calls
    effect_adapter
        .set_mission_manager(Arc::clone(&mission_manager))
        .await;

    // Wire mission manager into agent for /expected command
    agent
        .set_mission_manager(Arc::clone(&mission_manager))
        .await;

    *guard = Some(EngineState {
        thread_manager,
        conversation_manager,
        effect_adapter,
        store: store.clone(),
        default_project_id: project_id,
        pending_approvals: RwLock::new(HashMap::new()),
        pending_auth: RwLock::new(HashMap::new()),
        sse: agent.deps.sse_tx.clone(),
        db: agent.deps.store.clone(),
        secrets_store: agent.tools().secrets_store().cloned(),
    });

    Ok(())
}

async fn persist_pending_approval(
    store: &Arc<dyn Store>,
    pending: &PendingApproval,
) -> Result<(), Error> {
    let mut thread = store
        .load_thread(pending.thread_id)
        .await
        .map_err(|e| engine_err("store error", e))?
        .ok_or_else(|| engine_err("thread not found", pending.thread_id))?;

    let metadata = thread
        .metadata
        .as_object_mut()
        .ok_or_else(|| engine_err("thread metadata", "must be an object"))?;
    metadata.insert(
        PENDING_APPROVAL_METADATA_KEY.into(),
        serde_json::json!({
            "request_id": pending.request_id,
            "action_name": pending.action_name,
            "thread_id": pending.thread_id.to_string(),
            "conversation_id": pending.conversation_id.to_string(),
            "call_id": pending.call_id,
            "description": pending.description,
            "parameters": pending.parameters,
        }),
    );
    thread.updated_at = chrono::Utc::now();
    store
        .save_thread(&thread)
        .await
        .map_err(|e| engine_err("store error", e))
}

async fn load_pending_approval_from_thread(
    store: &Arc<dyn Store>,
    conversation_id: ironclaw_engine::ConversationId,
    thread_id: ironclaw_engine::ThreadId,
) -> Result<Option<PendingApproval>, Error> {
    let Some(thread) = store
        .load_thread(thread_id)
        .await
        .map_err(|e| engine_err("store error", e))?
    else {
        return Ok(None);
    };

    if thread.state != ironclaw_engine::ThreadState::Waiting {
        return Ok(None);
    }

    let Some(pending) = thread
        .metadata
        .get(PENDING_APPROVAL_METADATA_KEY)
        .and_then(|value| value.as_object())
    else {
        return Ok(None);
    };

    let Some(request_id) = pending.get("request_id").and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    let Some(action_name) = pending.get("action_name").and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    let Some(call_id) = pending.get("call_id").and_then(|value| value.as_str()) else {
        return Ok(None);
    };

    let description = pending
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Tool '{}' requires approval to execute.", action_name));
    let parameters = pending
        .get("parameters")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    Ok(Some(PendingApproval {
        request_id: request_id.to_string(),
        action_name: action_name.to_string(),
        thread_id,
        conversation_id,
        call_id: call_id.to_string(),
        description,
        parameters,
    }))
}

async fn clear_pending_approval_metadata(
    store: &Arc<dyn Store>,
    thread_id: ironclaw_engine::ThreadId,
) -> Result<(), Error> {
    let Some(mut thread) = store
        .load_thread(thread_id)
        .await
        .map_err(|e| engine_err("store error", e))?
    else {
        return Ok(());
    };

    if let Some(metadata) = thread.metadata.as_object_mut() {
        metadata.remove(PENDING_APPROVAL_METADATA_KEY);
        thread.updated_at = chrono::Utc::now();
        store
            .save_thread(&thread)
            .await
            .map_err(|e| engine_err("store error", e))?;
    }

    Ok(())
}

async fn resolve_pending_approval_for_thread(
    store: &Arc<dyn Store>,
    pending_approvals: &RwLock<HashMap<String, PendingApproval>>,
    user_id: &str,
    thread_id_hint: Option<&str>,
) -> Result<PendingApprovalResolution, Error> {
    let hinted_thread_id = thread_id_hint.and_then(|id| uuid::Uuid::parse_str(id).ok());

    if let Some(cached) = pending_approvals.read().await.get(user_id).cloned() {
        let hint_matches = hinted_thread_id
            .map(|id| cached.thread_id.0 == id)
            .unwrap_or(true);
        if hint_matches {
            if let Some(pending) =
                load_pending_approval_from_thread(store, cached.conversation_id, cached.thread_id)
                    .await?
            {
                return Ok(PendingApprovalResolution::Resolved(pending));
            }

            let mut approvals = pending_approvals.write().await;
            if approvals
                .get(user_id)
                .is_some_and(|pending| pending.thread_id == cached.thread_id)
            {
                approvals.remove(user_id);
            }
        }
    }

    let conversations = store
        .list_conversations(user_id)
        .await
        .map_err(|e| engine_err("store error", e))?;

    let mut candidates = Vec::new();
    for conversation in conversations {
        for thread_id in conversation.active_threads {
            if hinted_thread_id.is_some_and(|hint| thread_id.0 != hint) {
                continue;
            }

            let Some(thread) = store
                .load_thread(thread_id)
                .await
                .map_err(|e| engine_err("store error", e))?
            else {
                continue;
            };

            let Some(pending) =
                load_pending_approval_from_thread(store, conversation.id, thread_id).await?
            else {
                continue;
            };

            candidates.push((thread.updated_at, pending));
        }
    }

    if hinted_thread_id.is_none() && candidates.len() > 1 {
        return Ok(PendingApprovalResolution::Ambiguous);
    }

    candidates.sort_by_key(|(updated_at, _)| *updated_at);
    let resolved = candidates.pop().map(|(_, pending)| pending);
    if let Some(ref pending) = resolved {
        pending_approvals
            .write()
            .await
            .insert(user_id.to_string(), pending.clone());
    }
    Ok(match resolved {
        Some(pending) => PendingApprovalResolution::Resolved(pending),
        None => PendingApprovalResolution::None,
    })
}

pub async fn pending_approval_for_user_thread(
    user_id: &str,
    thread_id: Option<&str>,
) -> Result<Option<PendingApprovalView>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(None);
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(None);
    };

    match resolve_pending_approval_for_thread(
        &state.store,
        &state.pending_approvals,
        user_id,
        thread_id,
    )
    .await?
    {
        PendingApprovalResolution::Resolved(pending) => Ok(Some(PendingApprovalView {
            request_id: pending.request_id,
            tool_name: pending.action_name,
            description: pending.description,
            parameters: serde_json::to_string_pretty(&pending.parameters)
                .unwrap_or_else(|_| pending.parameters.to_string()),
        })),
        PendingApprovalResolution::None | PendingApprovalResolution::Ambiguous => Ok(None),
    }
}

/// Handle an approval response (yes/no/always) for engine v2.
///
/// Called from `handle_message` when the user responds to an approval request.
pub async fn handle_approval(
    agent: &Agent,
    message: &IncomingMessage,
    approved: bool,
    always: bool,
) -> Result<Option<String>, Error> {
    init_engine(agent).await?;

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    // Don't pass the v1 thread_id as a hint — the v1 session uses different
    // UUIDs from the engine.  The user_id alone is sufficient for single-user
    // deployments; ambiguity resolution kicks in for multi-user.
    let pending = match resolve_pending_approval_for_thread(
        &state.store,
        &state.pending_approvals,
        &message.user_id,
        None,
    )
    .await?
    {
        PendingApprovalResolution::Resolved(p) => p,
        PendingApprovalResolution::None => {
            debug!(user_id = %message.user_id, "engine v2: no pending approval for user, ignoring");
            return Ok(Some("No pending approval for this thread.".into()));
        }
        PendingApprovalResolution::Ambiguous => {
            return Ok(Some(
                "Multiple pending approvals are waiting. Approve from the original thread or retry with that thread selected.".into(),
            ));
        }
    };

    process_resolved_approval(agent, state, message, pending, approved, always).await
}

/// Handle an `ExecApproval` submission (web gateway JSON approval with explicit request_id).
pub async fn handle_exec_approval(
    agent: &Agent,
    message: &IncomingMessage,
    request_id: uuid::Uuid,
    approved: bool,
    always: bool,
) -> Result<Option<String>, Error> {
    init_engine(agent).await?;

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    let request_id_str = request_id.to_string();

    // First try the in-memory cache (keyed by user_id, but we match on request_id).
    let cached = state
        .pending_approvals
        .read()
        .await
        .get(&message.user_id)
        .filter(|p| p.request_id == request_id_str)
        .cloned();

    if let Some(pending) = cached {
        return process_resolved_approval(agent, state, message, pending, approved, always).await;
    }

    // Fall back to scanning thread metadata for this user's conversations.
    // Don't use v1 thread_id as hint (different UUID space from engine).
    let resolution = resolve_pending_approval_for_thread(
        &state.store,
        &state.pending_approvals,
        &message.user_id,
        None,
    )
    .await?;

    match resolution {
        PendingApprovalResolution::Resolved(pending) if pending.request_id == request_id_str => {
            process_resolved_approval(agent, state, message, pending, approved, always).await
        }
        _ => {
            debug!(
                user_id = %message.user_id,
                request_id = %request_id,
                "engine v2: no matching pending approval for request_id"
            );
            Ok(Some("No matching pending approval found.".into()))
        }
    }
}

/// Shared logic for processing a resolved pending approval.
async fn process_resolved_approval(
    agent: &Agent,
    state: &EngineState,
    message: &IncomingMessage,
    pending: PendingApproval,
    approved: bool,
    always: bool,
) -> Result<Option<String>, Error> {
    if !approved {
        let _ = agent
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Status("Tool call denied.".into()),
                &message.metadata,
            )
            .await;
    }

    debug!(
        tool = %pending.action_name,
        always,
        approved,
        "engine v2: tool approval received"
    );

    if approved && always {
        let registry_name = pending.action_name.replace('_', "-");
        state
            .effect_adapter
            .auto_approve_tool(&pending.action_name)
            .await;
        state.effect_adapter.auto_approve_tool(&registry_name).await;
        debug!(
            tool = %pending.action_name,
            "engine v2: tool auto-approved for session"
        );
    }

    let _ = agent
        .channels
        .send_status(
            &message.channel,
            StatusUpdate::Thinking("Resuming pending thread...".into()),
            &message.metadata,
        )
        .await;

    let resume_message = if approved {
        ironclaw_engine::ThreadMessage::user(format!(
            "User approved action '{}'. Continue from the pending step and reuse the approved action if still needed.",
            pending.action_name
        ))
    } else {
        ironclaw_engine::ThreadMessage::user(format!(
            "User denied action '{}'. Do not execute it; choose an alternative approach.",
            pending.action_name
        ))
    };

    state.effect_adapter.reset_call_count();
    state
        .thread_manager
        .resume_thread(
            pending.thread_id,
            message.user_id.clone(),
            Some(resume_message),
            Some((pending.call_id.clone(), approved)),
        )
        .await
        .map_err(|e| engine_err("resume error", e))?;
    clear_pending_approval_metadata(&state.store, pending.thread_id).await?;
    let mut approvals = state.pending_approvals.write().await;
    if approvals
        .get(&message.user_id)
        .is_some_and(|cached| cached.thread_id == pending.thread_id)
    {
        approvals.remove(&message.user_id);
    }

    await_thread_outcome(
        agent,
        state,
        message,
        pending.conversation_id,
        pending.thread_id,
    )
    .await
}

/// Handle an interrupt submission — stop active engine threads.
pub async fn handle_interrupt(
    agent: &Agent,
    message: &IncomingMessage,
) -> Result<Option<String>, Error> {
    init_engine(agent).await?;

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    let conv_id = state
        .conversation_manager
        .get_or_create_conversation(&message.channel, &message.user_id)
        .await
        .map_err(|e| engine_err("conversation error", e))?;

    let conv = state.conversation_manager.get_conversation(conv_id).await;
    let active_threads = conv
        .as_ref()
        .map(|c| c.active_threads.clone())
        .unwrap_or_default();

    let mut stopped = 0u32;
    for tid in &active_threads {
        if state.thread_manager.is_running(*tid).await {
            if let Err(e) = state.thread_manager.stop_thread(*tid).await {
                debug!(thread_id = %tid, error = %e, "engine v2: failed to stop thread");
            } else {
                stopped += 1;
            }
        }
    }

    if stopped > 0 {
        debug!(stopped, "engine v2: interrupted running threads");
        Ok(Some("Interrupted.".into()))
    } else {
        Ok(Some("Nothing to interrupt.".into()))
    }
}

/// Handle a new-thread submission — clear conversation for a fresh start.
pub async fn handle_new_thread(
    agent: &Agent,
    message: &IncomingMessage,
) -> Result<Option<String>, Error> {
    clear_engine_conversation(agent, message).await?;
    Ok(Some("Started new conversation.".into()))
}

/// Handle a clear submission — stop threads and reset conversation.
pub async fn handle_clear(
    agent: &Agent,
    message: &IncomingMessage,
) -> Result<Option<String>, Error> {
    clear_engine_conversation(agent, message).await?;
    Ok(Some("Conversation cleared.".into()))
}

/// Stop all active threads and clear conversation entries.
async fn clear_engine_conversation(agent: &Agent, message: &IncomingMessage) -> Result<(), Error> {
    init_engine(agent).await?;

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    let conv_id = state
        .conversation_manager
        .get_or_create_conversation(&message.channel, &message.user_id)
        .await
        .map_err(|e| engine_err("conversation error", e))?;

    // Stop all active threads first
    if let Some(conv) = state.conversation_manager.get_conversation(conv_id).await {
        for tid in &conv.active_threads {
            if state.thread_manager.is_running(*tid).await {
                let _ = state.thread_manager.stop_thread(*tid).await;
            }
        }
    }

    // Clear the conversation entries and active thread list
    state
        .conversation_manager
        .clear_conversation(conv_id)
        .await
        .map_err(|e| engine_err("clear conversation error", e))?;

    // Also clear any pending approvals for this user
    state
        .pending_approvals
        .write()
        .await
        .remove(&message.user_id);

    debug!(
        user_id = %message.user_id,
        conversation_id = %conv_id,
        "engine v2: conversation cleared"
    );

    Ok(())
}

/// Handle a user message through the engine v2 pipeline.
pub async fn handle_with_engine(
    agent: &Agent,
    message: &IncomingMessage,
    content: &str,
) -> Result<Option<String>, Error> {
    // Ensure engine is initialized
    init_engine(agent).await?;

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    debug!(
        user_id = %message.user_id,
        channel = %message.channel,
        "engine v2: handling message"
    );

    // Check for pending auth — if the user is responding to an auth prompt,
    // treat the message as a token value and store it as a secret.
    {
        let pending = state.pending_auth.write().await.remove(&message.user_id);
        if let Some(pending) = pending {
            let token = content.trim().to_string();
            if token.is_empty() || token.eq_ignore_ascii_case("cancel") {
                let response = "Authentication cancelled.".to_string();
                // Write to v1 DB so the history API shows the response.
                if let Some(ref db) = state.db {
                    let scope = message.conversation_scope();
                    let v1_conv_id = if let Some(tid) = scope
                        && let Ok(uuid) = uuid::Uuid::parse_str(tid)
                    {
                        Some(uuid)
                    } else {
                        db.get_or_create_assistant_conversation(&message.user_id, &message.channel)
                            .await
                            .ok()
                    };
                    if let Some(cid) = v1_conv_id {
                        let _ = db
                            .add_conversation_message(cid, "assistant", &response)
                            .await;
                    }
                }
                return Ok(Some(response));
            }

            if let Some(ref ss) = state.secrets_store {
                let params =
                    crate::secrets::CreateSecretParams::new(&pending.credential_name, &token);
                match ss.create(&message.user_id, params).await {
                    Ok(_) => {
                        let _ = agent
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: pending.credential_name.clone(),
                                    success: true,
                                    message: format!(
                                        "Credential '{}' stored. Retrying your request...",
                                        pending.credential_name
                                    ),
                                },
                                &message.metadata,
                            )
                            .await;

                        // Retry the original request — drop the read guard first,
                        // then re-enter handle_with_engine with the original message.
                        let retry_msg = IncomingMessage {
                            content: pending.original_message.clone(),
                            channel: pending.channel,
                            user_id: pending.user_id,
                            metadata: pending.metadata,
                            ..message.clone()
                        };
                        let retry_content = pending.original_message;
                        drop(guard);
                        return Box::pin(handle_with_engine(agent, &retry_msg, &retry_content))
                            .await;
                    }
                    Err(e) => {
                        return Ok(Some(format!(
                            "Failed to store credential '{}': {}",
                            pending.credential_name, e
                        )));
                    }
                }
            } else {
                return Ok(Some(
                    "No secrets store available. Cannot store credentials.".into(),
                ));
            }
        }
    }

    // Send "Thinking..." status to the channel
    let _ = agent
        .channels
        .send_status(
            &message.channel,
            StatusUpdate::Thinking("Processing...".into()),
            &message.metadata,
        )
        .await;

    // Reset the per-step call counter so each thread starts fresh
    state.effect_adapter.reset_call_count();

    // Scope the engine conversation by (channel, user, thread).
    // When the frontend sends a thread_id (user created a new conversation),
    // use it as part of the channel key so each v1 thread maps to a distinct
    // engine conversation. Without this, all threads share one conversation
    // and messages appear in the wrong place.
    let scope = message.conversation_scope();
    let channel_key = match scope {
        Some(tid) => format!("{}:{}", message.channel, tid),
        None => message.channel.clone(),
    };

    // Get or create conversation for this scoped channel+user
    let conv_id = state
        .conversation_manager
        .get_or_create_conversation(&channel_key, &message.user_id)
        .await
        .map_err(|e| engine_err("conversation error", e))?;

    // Handle the message — spawns a new thread or injects into active one
    let thread_id = state
        .conversation_manager
        .handle_user_message(
            conv_id,
            content,
            state.default_project_id,
            &message.user_id,
            ThreadConfig::default(),
        )
        .await
        .map_err(|e| engine_err("thread error", e))?;

    // Dual-write to v1 database so the gateway history API shows messages.
    // Use the thread-scoped conversation (from thread_id) when available,
    // falling back to the default assistant conversation.
    if let Some(ref db) = state.db {
        let v1_conv_id = if let Some(tid) = scope
            && let Ok(uuid) = uuid::Uuid::parse_str(tid)
        {
            // Ensure the v1 conversation exists for this thread
            let _ = db
                .ensure_conversation(uuid, &message.channel, &message.user_id, Some(tid))
                .await;
            Some(uuid)
        } else {
            db.get_or_create_assistant_conversation(&message.user_id, &message.channel)
                .await
                .ok()
        };
        if let Some(cid) = v1_conv_id {
            let _ = db.add_conversation_message(cid, "user", content).await;
        }
    }

    debug!(thread_id = %thread_id, "engine v2: thread spawned");
    await_thread_outcome(agent, state, message, conv_id, thread_id).await
}

async fn await_thread_outcome(
    agent: &Agent,
    state: &EngineState,
    message: &IncomingMessage,
    conv_id: ironclaw_engine::ConversationId,
    thread_id: ironclaw_engine::ThreadId,
) -> Result<Option<String>, Error> {
    let mut event_rx = state.thread_manager.subscribe_events();
    let channels = &agent.channels;
    let channel_name = &message.channel;
    let metadata = &message.metadata;
    let sse = state.sse.as_ref();
    let tid_str = thread_id.to_string();

    // Safety timeout: if the thread doesn't finish within 5 minutes,
    // break out to avoid hanging the user session forever (e.g. after
    // a denied approval where the thread fails to resume).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Ok(ref evt) if evt.thread_id == thread_id => {
                        forward_event_to_channel(evt, channels, channel_name, metadata).await;
                        if let Some(sse) = sse {
                            for app_event in thread_event_to_app_events(evt, &tid_str) {
                                sse.broadcast_for_user(&message.user_id, app_event);
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {}
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                if !state.thread_manager.is_running(thread_id).await {
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    tracing::warn!(
                        thread_id = %thread_id,
                        "await_thread_outcome timed out after 5 minutes — breaking to avoid hang"
                    );
                    break;
                }
            }
        }
    }

    let outcome = state
        .thread_manager
        .join_thread(thread_id)
        .await
        .map_err(|e| engine_err("join error", e))?;

    state
        .conversation_manager
        .record_thread_outcome(conv_id, thread_id, &outcome)
        .await
        .map_err(|e| engine_err("conversation error", e))?;

    // Helper: write the outcome response to the v1 DB so the history API
    // shows it correctly (not just for Completed, but for ALL outcomes that
    // produce a response, including NeedApproval and NeedAuthentication).
    let write_v1_response = |db: &Arc<dyn crate::db::Database>, text: &str| {
        let db = Arc::clone(db);
        let scope = message.conversation_scope().map(String::from);
        let user_id = message.user_id.clone();
        let channel = message.channel.clone();
        let text = text.to_string();
        async move {
            let v1_conv_id = if let Some(tid) = scope
                && let Ok(uuid) = uuid::Uuid::parse_str(&tid)
            {
                Some(uuid)
            } else {
                db.get_or_create_assistant_conversation(&user_id, &channel)
                    .await
                    .ok()
            };
            if let Some(cid) = v1_conv_id {
                let _ = db.add_conversation_message(cid, "assistant", &text).await;
            }
        }
    };

    if let Some(ref sse) = state.sse
        && let ThreadOutcome::Completed {
            response: Some(ref text),
        } = outcome
    {
        sse.broadcast_for_user(
            &message.user_id,
            AppEvent::Response {
                content: text.clone(),
                thread_id: thread_id.to_string(),
            },
        );
    }

    let result = match outcome {
        ThreadOutcome::Completed { response } => {
            debug!(thread_id = %thread_id, "engine v2: completed");

            // Detect authentication_required in the response and enter auth mode.
            // The user sees a prompt to paste their token; the next message stores
            // it and retries the original request.
            if let Some(ref text) = response
                && text.contains("authentication_required")
            {
                // Extract credential name from the response text
                let cred_name = text
                    .split("credential_name")
                    .nth(1)
                    .and_then(|s| {
                        // Handle both JSON ("credential_name":"foo") and prose
                        s.split(&['"', '\'', '`'][..])
                            .find(|seg| !seg.is_empty() && !seg.contains(':') && !seg.contains(' '))
                    })
                    .unwrap_or("unknown")
                    .to_string();

                // Find setup instructions from skill credential spec
                let setup_hint = agent
                    .deps
                    .skill_registry
                    .as_ref()
                    .and_then(|sr| {
                        let reg = sr.read().ok()?;
                        reg.skills().iter().find_map(|s| {
                            s.manifest.credentials.iter().find_map(|c| {
                                if c.name == cred_name {
                                    c.setup_instructions.clone()
                                } else {
                                    None
                                }
                            })
                        })
                    })
                    .unwrap_or_else(|| format!("Provide your {} token", cred_name));

                // Store pending auth for this user
                state.pending_auth.write().await.insert(
                    message.user_id.clone(),
                    PendingAuth {
                        credential_name: cred_name.clone(),
                        original_message: message.content.clone(),
                        user_id: message.user_id.clone(),
                        channel: message.channel.clone(),
                        metadata: message.metadata.clone(),
                    },
                );

                // Show auth prompt via channel
                let _ = agent
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthRequired {
                            extension_name: cred_name.clone(),
                            instructions: Some(setup_hint),
                            auth_url: None,
                            setup_url: None,
                        },
                        &message.metadata,
                    )
                    .await;

                return Ok(Some(format!(
                    "Authentication required for '{}'. Paste your token below (or type 'cancel'):",
                    cred_name
                )));
            }

            Ok(response)
        }
        ThreadOutcome::Stopped => Ok(Some("Thread was stopped.".into())),
        ThreadOutcome::MaxIterations => Ok(Some(
            "Reached maximum iterations without completing.".into(),
        )),
        ThreadOutcome::Failed { error } => Ok(Some(format!("Error: {error}"))),
        ThreadOutcome::NeedApproval {
            action_name,
            call_id,
            parameters,
        } => {
            let request_id = uuid::Uuid::new_v4().to_string();
            let description = format!("Tool '{}' requires approval to execute.", action_name);
            let pending = PendingApproval {
                request_id: request_id.clone(),
                action_name: action_name.clone(),
                thread_id,
                conversation_id: conv_id,
                call_id,
                description: description.clone(),
                parameters: parameters.clone(),
            };
            state
                .pending_approvals
                .write()
                .await
                .insert(message.user_id.clone(), pending.clone());
            persist_pending_approval(&state.store, &pending).await?;

            // Send approval request to channel (matches v1 ApprovalNeeded format)
            let _ = agent
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::ApprovalNeeded {
                        request_id,
                        tool_name: action_name.clone(),
                        description,
                        parameters,
                        allow_always: true,
                    },
                    &message.metadata,
                )
                .await;

            Ok(Some(format!(
                "Tool '{}' requires approval. Reply 'yes' to approve, 'always' to auto-approve future uses of this tool, or 'no' to deny.",
                action_name
            )))
        }
        ThreadOutcome::NeedAuthentication {
            credential_name, ..
        } => {
            // Look up setup instructions from the skill's credential spec.
            let setup_hint = agent
                .deps
                .skill_registry
                .as_ref()
                .and_then(|sr| {
                    let reg = sr.read().ok()?;
                    reg.skills().iter().find_map(|s| {
                        s.manifest.credentials.iter().find_map(|c| {
                            if c.name == credential_name {
                                c.setup_instructions.clone()
                            } else {
                                None
                            }
                        })
                    })
                })
                .unwrap_or_else(|| format!("Provide your {} token", credential_name));

            // Enter the guided auth flow — next user message is treated as a token.
            state.pending_auth.write().await.insert(
                message.user_id.clone(),
                PendingAuth {
                    credential_name: credential_name.clone(),
                    original_message: message.content.clone(),
                    user_id: message.user_id.clone(),
                    channel: message.channel.clone(),
                    metadata: message.metadata.clone(),
                },
            );

            let _ = agent
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::AuthRequired {
                        extension_name: credential_name.clone(),
                        instructions: Some(setup_hint),
                        auth_url: None,
                        setup_url: None,
                    },
                    &message.metadata,
                )
                .await;

            Ok(Some(format!(
                "Authentication required for '{}'. Paste your token below (or type 'cancel'):",
                credential_name
            )))
        }
    };

    // Write the response to the v1 DB for all outcomes so the history
    // endpoint shows the correct state (not just for Completed).
    if let Ok(Some(ref text)) = result
        && let Some(ref db) = state.db
    {
        write_v1_response(db, text).await;
    }

    result
}

/// Forward an engine ThreadEvent to the channel as a StatusUpdate.
async fn forward_event_to_channel(
    event: &ironclaw_engine::ThreadEvent,
    channels: &std::sync::Arc<crate::channels::ChannelManager>,
    channel_name: &str,
    metadata: &serde_json::Value,
) {
    use ironclaw_engine::EventKind;

    match &event.kind {
        EventKind::StepStarted { .. } => {
            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::Thinking("Calling LLM...".into()),
                    metadata,
                )
                .await;
        }
        EventKind::ActionExecuted {
            action_name,
            duration_ms,
            params_summary,
            ..
        } => {
            // Format tool name with params summary: "http(https://api.github.com/...)"
            let display_name = match params_summary {
                Some(summary) => format!("{}({})", action_name, summary),
                None => action_name.clone(),
            };

            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::ToolStarted {
                        name: display_name.clone(),
                    },
                    metadata,
                )
                .await;
            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::ToolCompleted {
                        name: display_name,
                        success: true,
                        error: None,
                        parameters: Some(format!("{duration_ms}ms")),
                    },
                    metadata,
                )
                .await;
        }
        EventKind::ActionFailed {
            action_name,
            error,
            params_summary,
            ..
        } => {
            let display_name = match params_summary {
                Some(summary) => format!("{}({})", action_name, summary),
                None => action_name.clone(),
            };

            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::ToolStarted {
                        name: display_name.clone(),
                    },
                    metadata,
                )
                .await;
            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::ToolCompleted {
                        name: display_name,
                        success: false,
                        error: Some(error.clone()),
                        parameters: None,
                    },
                    metadata,
                )
                .await;

            // When the HTTP tool fails with authentication_required, show the
            // auth prompt in the CLI/REPL so the user can authenticate.
            if error.contains("authentication_required") {
                let cred_name = error
                    .split("credential '")
                    .nth(1)
                    .and_then(|s| s.split('\'').next())
                    .unwrap_or("unknown")
                    .to_string();
                let _ = channels
                    .send_status(
                        channel_name,
                        StatusUpdate::AuthRequired {
                            extension_name: cred_name,
                            instructions: Some(
                                "Store the credential with: ironclaw secret set <name> <value>"
                                    .into(),
                            ),
                            auth_url: None,
                            setup_url: None,
                        },
                        metadata,
                    )
                    .await;
            }
        }
        EventKind::StepCompleted { tokens, .. } => {
            let tok_msg = format!(
                "Step complete — {} in / {} out tokens",
                tokens.input_tokens, tokens.output_tokens
            );
            let _ = channels
                .send_status(channel_name, StatusUpdate::Thinking(tok_msg), metadata)
                .await;
        }
        EventKind::MessageAdded {
            role,
            content_preview,
        } => {
            // Surface code execution and tool results as thinking status
            let msg = if role == "User" && content_preview.starts_with("[stdout]") {
                Some("Code executed".to_string())
            } else if role == "User" && content_preview.starts_with("[code ") {
                Some("Code executed (no output)".to_string())
            } else if role == "User"
                && (content_preview.contains("Error") || content_preview.starts_with("Traceback"))
            {
                Some("Code error — retrying...".to_string())
            } else if role == "Assistant" {
                Some("Executing code...".to_string())
            } else {
                None
            };
            if let Some(text) = msg {
                let _ = channels
                    .send_status(channel_name, StatusUpdate::Thinking(text), metadata)
                    .await;
            }
        }
        EventKind::SkillActivated { skill_names } => {
            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::SkillActivated {
                        skill_names: skill_names.clone(),
                    },
                    metadata,
                )
                .await;
        }
        _ => {}
    }
}

/// Convert a ThreadEvent to AppEvents for the web gateway SSE stream.
///
/// Returns multiple events when needed (e.g., ToolStarted + ToolCompleted
/// so the frontend creates the card then resolves it).
fn thread_event_to_app_events(
    event: &ironclaw_engine::ThreadEvent,
    thread_id: &str,
) -> Vec<AppEvent> {
    use ironclaw_engine::EventKind;

    match &event.kind {
        EventKind::StepStarted { .. } => vec![AppEvent::Thinking {
            message: "Calling LLM...".into(),
            thread_id: Some(thread_id.into()),
        }],
        EventKind::ActionExecuted {
            action_name,
            duration_ms,
            params_summary,
            ..
        } => {
            let display_name = match params_summary {
                Some(s) => format!("{}({})", action_name, s),
                None => action_name.clone(),
            };
            vec![
                AppEvent::ToolStarted {
                    name: display_name.clone(),
                    thread_id: Some(thread_id.into()),
                },
                AppEvent::ToolCompleted {
                    name: display_name,
                    success: true,
                    error: None,
                    parameters: Some(format!("{duration_ms}ms")),
                    thread_id: Some(thread_id.into()),
                },
            ]
        }
        EventKind::ActionFailed {
            action_name,
            error,
            params_summary,
            ..
        } => {
            let display_name = match params_summary {
                Some(s) => format!("{}({})", action_name, s),
                None => action_name.clone(),
            };
            vec![
                AppEvent::ToolStarted {
                    name: display_name.clone(),
                    thread_id: Some(thread_id.into()),
                },
                AppEvent::ToolCompleted {
                    name: display_name,
                    success: false,
                    error: Some(error.clone()),
                    parameters: None,
                    thread_id: Some(thread_id.into()),
                },
            ]
        }
        EventKind::StepCompleted { tokens, .. } => vec![AppEvent::Status {
            message: format!(
                "Step complete — {} in / {} out tokens",
                tokens.input_tokens, tokens.output_tokens
            ),
            thread_id: Some(thread_id.into()),
        }],
        EventKind::MessageAdded {
            role,
            content_preview,
        } => {
            let msg = if role == "User" && content_preview.starts_with("[stdout]") {
                Some("Code executed")
            } else if role == "User" && content_preview.starts_with("[code ") {
                Some("Code executed (no output)")
            } else if role == "User"
                && (content_preview.contains("Error") || content_preview.starts_with("Traceback"))
            {
                Some("Code error — retrying...")
            } else if role == "Assistant" {
                Some("Executing code...")
            } else {
                None
            };
            msg.map(|text| AppEvent::Thinking {
                message: text.into(),
                thread_id: Some(thread_id.into()),
            })
            .into_iter()
            .collect()
        }
        EventKind::StateChanged { from, to, reason } => {
            vec![AppEvent::ThreadStateChanged {
                thread_id: thread_id.into(),
                from_state: format!("{from:?}"),
                to_state: format!("{to:?}"),
                reason: reason.clone(),
            }]
        }
        EventKind::ChildSpawned { child_id, goal } => vec![AppEvent::ChildThreadSpawned {
            parent_thread_id: thread_id.into(),
            child_thread_id: child_id.to_string(),
            goal: goal.clone(),
        }],
        EventKind::SkillActivated { skill_names } => vec![AppEvent::SkillActivated {
            skill_names: skill_names.clone(),
            thread_id: Some(thread_id.into()),
        }],
        _ => vec![],
    }
}

// ── Engine query DTOs ────────────────────────────────────────

/// Lightweight thread summary for list views.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineThreadInfo {
    pub id: String,
    pub goal: String,
    pub thread_type: String,
    pub state: String,
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub step_count: usize,
    pub total_tokens: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// Thread detail with messages and config.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineThreadDetail {
    #[serde(flatten)]
    pub info: EngineThreadInfo,
    pub messages: Vec<serde_json::Value>,
    pub max_iterations: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    pub total_cost_usd: f64,
}

/// Step summary for thread detail views.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineStepInfo {
    pub id: String,
    pub sequence: usize,
    pub status: String,
    pub tier: String,
    pub action_results_count: usize,
    pub tokens_input: u64,
    pub tokens_output: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// Project summary.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineProjectInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
}

/// Mission summary for list views.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineMissionInfo {
    pub id: String,
    pub name: String,
    pub goal: String,
    pub status: String,
    pub cadence_type: String,
    pub thread_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_focus: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Mission detail with full strategy and budget info.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineMissionDetail {
    #[serde(flatten)]
    pub info: EngineMissionInfo,
    pub cadence: serde_json::Value,
    pub approach_history: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_criteria: Option<String>,
    pub threads_today: u32,
    pub max_threads_per_day: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_fire_at: Option<String>,
    pub threads: Vec<EngineThreadInfo>,
}

// ── Engine query functions ───────────────────────────────────

fn cadence_type_label(cadence: &ironclaw_engine::types::mission::MissionCadence) -> &'static str {
    use ironclaw_engine::types::mission::MissionCadence;
    match cadence {
        MissionCadence::Cron { .. } => "cron",
        MissionCadence::OnEvent { .. } => "event",
        MissionCadence::OnSystemEvent { .. } => "system_event",
        MissionCadence::Webhook { .. } => "webhook",
        MissionCadence::Manual => "manual",
    }
}

fn thread_to_info(t: &ironclaw_engine::Thread) -> EngineThreadInfo {
    EngineThreadInfo {
        id: t.id.to_string(),
        goal: t.goal.clone(),
        thread_type: format!("{:?}", t.thread_type),
        state: format!("{:?}", t.state),
        project_id: t.project_id.to_string(),
        parent_id: t.parent_id.map(|id| id.to_string()),
        step_count: t.step_count,
        total_tokens: t.total_tokens_used,
        created_at: t.created_at.to_rfc3339(),
        updated_at: t.updated_at.to_rfc3339(),
    }
}

/// List engine threads, optionally filtered by project.
pub async fn list_engine_threads(project_id: Option<&str>) -> Result<Vec<EngineThreadInfo>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(Vec::new());
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(Vec::new());
    };

    let pid = match project_id {
        Some(id) => {
            let uuid = uuid::Uuid::parse_str(id).map_err(|e| engine_err("parse project_id", e))?;
            ironclaw_engine::ProjectId(uuid)
        }
        None => state.default_project_id,
    };

    let threads = state
        .store
        .list_threads(pid)
        .await
        .map_err(|e| engine_err("list threads", e))?;

    Ok(threads.iter().map(thread_to_info).collect())
}

/// Get a single engine thread by ID.
pub async fn get_engine_thread(thread_id: &str) -> Result<Option<EngineThreadDetail>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(None);
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(None);
    };

    let tid = uuid::Uuid::parse_str(thread_id).map_err(|e| engine_err("parse thread_id", e))?;
    let tid = ironclaw_engine::ThreadId(tid);

    let Some(thread) = state
        .store
        .load_thread(tid)
        .await
        .map_err(|e| engine_err("load thread", e))?
    else {
        return Ok(None);
    };

    let messages: Vec<serde_json::Value> = thread
        .messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": format!("{:?}", m.role),
                "content": m.content,
                "timestamp": m.timestamp.to_rfc3339(),
            })
        })
        .collect();

    Ok(Some(EngineThreadDetail {
        info: thread_to_info(&thread),
        messages,
        max_iterations: thread.config.max_iterations,
        completed_at: thread.completed_at.map(|dt| dt.to_rfc3339()),
        total_cost_usd: thread.total_cost_usd,
    }))
}

/// List steps for a thread.
pub async fn list_engine_thread_steps(thread_id: &str) -> Result<Vec<EngineStepInfo>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(Vec::new());
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(Vec::new());
    };

    let tid = uuid::Uuid::parse_str(thread_id).map_err(|e| engine_err("parse thread_id", e))?;
    let steps = state
        .store
        .load_steps(ironclaw_engine::ThreadId(tid))
        .await
        .map_err(|e| engine_err("load steps", e))?;

    Ok(steps
        .iter()
        .map(|s| EngineStepInfo {
            id: s.id.0.to_string(),
            sequence: s.sequence,
            status: format!("{:?}", s.status),
            tier: format!("{:?}", s.tier),
            action_results_count: s.action_results.len(),
            tokens_input: s.tokens_used.input_tokens,
            tokens_output: s.tokens_used.output_tokens,
            started_at: Some(s.started_at.to_rfc3339()),
            completed_at: s.completed_at.map(|dt| dt.to_rfc3339()),
        })
        .collect())
}

/// List events for a thread as raw JSON values.
pub async fn list_engine_thread_events(thread_id: &str) -> Result<Vec<serde_json::Value>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(Vec::new());
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(Vec::new());
    };

    let tid = uuid::Uuid::parse_str(thread_id).map_err(|e| engine_err("parse thread_id", e))?;
    let events = state
        .store
        .load_events(ironclaw_engine::ThreadId(tid))
        .await
        .map_err(|e| engine_err("load events", e))?;

    Ok(events
        .iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .collect())
}

/// List all projects.
pub async fn list_engine_projects() -> Result<Vec<EngineProjectInfo>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(Vec::new());
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(Vec::new());
    };

    let projects = state
        .store
        .list_projects()
        .await
        .map_err(|e| engine_err("list projects", e))?;

    Ok(projects
        .iter()
        .map(|p| EngineProjectInfo {
            id: p.id.to_string(),
            name: p.name.clone(),
            description: p.description.clone(),
            created_at: p.created_at.to_rfc3339(),
        })
        .collect())
}

/// Get a single project by ID.
pub async fn get_engine_project(project_id: &str) -> Result<Option<EngineProjectInfo>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(None);
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(None);
    };

    let pid = uuid::Uuid::parse_str(project_id).map_err(|e| engine_err("parse project_id", e))?;
    let project = state
        .store
        .load_project(ironclaw_engine::ProjectId(pid))
        .await
        .map_err(|e| engine_err("load project", e))?;

    Ok(project.map(|p| EngineProjectInfo {
        id: p.id.to_string(),
        name: p.name,
        description: p.description,
        created_at: p.created_at.to_rfc3339(),
    }))
}

/// List missions, optionally filtered by project.
pub async fn list_engine_missions(
    project_id: Option<&str>,
) -> Result<Vec<EngineMissionInfo>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(Vec::new());
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(Vec::new());
    };

    let pid = match project_id {
        Some(id) => {
            let uuid = uuid::Uuid::parse_str(id).map_err(|e| engine_err("parse project_id", e))?;
            ironclaw_engine::ProjectId(uuid)
        }
        None => state.default_project_id,
    };

    let missions = state
        .store
        .list_missions(pid)
        .await
        .map_err(|e| engine_err("list missions", e))?;

    Ok(missions
        .iter()
        .map(|m| EngineMissionInfo {
            id: m.id.to_string(),
            name: m.name.clone(),
            goal: m.goal.clone(),
            status: format!("{:?}", m.status),
            cadence_type: cadence_type_label(&m.cadence).to_string(),
            thread_count: m.thread_history.len(),
            current_focus: m.current_focus.clone(),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        })
        .collect())
}

/// Get a single mission by ID.
pub async fn get_engine_mission(mission_id: &str) -> Result<Option<EngineMissionDetail>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(None);
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(None);
    };

    let mid = uuid::Uuid::parse_str(mission_id).map_err(|e| engine_err("parse mission_id", e))?;
    let mission = state
        .store
        .load_mission(ironclaw_engine::MissionId(mid))
        .await
        .map_err(|e| engine_err("load mission", e))?;

    let Some(m) = mission else {
        return Ok(None);
    };

    let cadence_json = serde_json::to_value(&m.cadence).unwrap_or(serde_json::Value::Null);

    // Load thread summaries for the spawned threads table
    let mut threads = Vec::new();
    for tid in &m.thread_history {
        if let Ok(Some(thread)) = state.store.load_thread(*tid).await {
            threads.push(thread_to_info(&thread));
        }
    }

    Ok(Some(EngineMissionDetail {
        info: EngineMissionInfo {
            id: m.id.to_string(),
            name: m.name.clone(),
            goal: m.goal.clone(),
            status: format!("{:?}", m.status),
            cadence_type: cadence_type_label(&m.cadence).to_string(),
            thread_count: m.thread_history.len(),
            current_focus: m.current_focus.clone(),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        },
        cadence: cadence_json,
        approach_history: m.approach_history.clone(),
        success_criteria: m.success_criteria.clone(),
        threads_today: m.threads_today,
        max_threads_per_day: m.max_threads_per_day,
        next_fire_at: m.next_fire_at.map(|dt| dt.to_rfc3339()),
        threads,
    }))
}

/// Manually fire a mission (spawn a new thread).
pub async fn fire_engine_mission(mission_id: &str, user_id: &str) -> Result<Option<String>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };

    let mid = uuid::Uuid::parse_str(mission_id).map_err(|e| engine_err("parse mission_id", e))?;
    let mid = ironclaw_engine::MissionId(mid);

    let result = state
        .effect_adapter
        .mission_manager()
        .await
        .ok_or_else(|| engine_err("mission", "mission manager not available"))?
        .fire_mission(mid, user_id, None)
        .await
        .map_err(|e| engine_err("fire mission", e))?;

    Ok(result.map(|tid| tid.to_string()))
}

/// Pause a mission.
pub async fn pause_engine_mission(mission_id: &str) -> Result<(), Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };

    let mid = uuid::Uuid::parse_str(mission_id).map_err(|e| engine_err("parse mission_id", e))?;
    state
        .effect_adapter
        .mission_manager()
        .await
        .ok_or_else(|| engine_err("mission", "mission manager not available"))?
        .pause_mission(ironclaw_engine::MissionId(mid))
        .await
        .map_err(|e| engine_err("pause mission", e))
}

/// Resume a paused mission.
pub async fn resume_engine_mission(mission_id: &str) -> Result<(), Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };

    let mid = uuid::Uuid::parse_str(mission_id).map_err(|e| engine_err("parse mission_id", e))?;
    state
        .effect_adapter
        .mission_manager()
        .await
        .ok_or_else(|| engine_err("mission", "mission manager not available"))?
        .resume_mission(ironclaw_engine::MissionId(mid))
        .await
        .map_err(|e| engine_err("resume mission", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::RwLock as TokioRwLock;

    struct TestStore {
        conversations: TokioRwLock<Vec<ironclaw_engine::ConversationSurface>>,
        threads: TokioRwLock<HashMap<ironclaw_engine::ThreadId, ironclaw_engine::Thread>>,
    }

    impl TestStore {
        fn new() -> Self {
            Self {
                conversations: TokioRwLock::new(Vec::new()),
                threads: TokioRwLock::new(HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl Store for TestStore {
        async fn save_thread(
            &self,
            thread: &ironclaw_engine::Thread,
        ) -> Result<(), ironclaw_engine::EngineError> {
            self.threads.write().await.insert(thread.id, thread.clone());
            Ok(())
        }
        async fn load_thread(
            &self,
            id: ironclaw_engine::ThreadId,
        ) -> Result<Option<ironclaw_engine::Thread>, ironclaw_engine::EngineError> {
            Ok(self.threads.read().await.get(&id).cloned())
        }
        async fn list_threads(
            &self,
            _project_id: ironclaw_engine::ProjectId,
        ) -> Result<Vec<ironclaw_engine::Thread>, ironclaw_engine::EngineError> {
            Ok(self.threads.read().await.values().cloned().collect())
        }
        async fn update_thread_state(
            &self,
            _id: ironclaw_engine::ThreadId,
            _state: ironclaw_engine::ThreadState,
        ) -> Result<(), ironclaw_engine::EngineError> {
            Ok(())
        }
        async fn save_step(
            &self,
            _: &ironclaw_engine::Step,
        ) -> Result<(), ironclaw_engine::EngineError> {
            Ok(())
        }
        async fn load_steps(
            &self,
            _: ironclaw_engine::ThreadId,
        ) -> Result<Vec<ironclaw_engine::Step>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
        async fn append_events(
            &self,
            _: &[ironclaw_engine::ThreadEvent],
        ) -> Result<(), ironclaw_engine::EngineError> {
            Ok(())
        }
        async fn load_events(
            &self,
            _: ironclaw_engine::ThreadId,
        ) -> Result<Vec<ironclaw_engine::ThreadEvent>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
        async fn save_project(
            &self,
            _: &ironclaw_engine::Project,
        ) -> Result<(), ironclaw_engine::EngineError> {
            Ok(())
        }
        async fn load_project(
            &self,
            _: ironclaw_engine::ProjectId,
        ) -> Result<Option<ironclaw_engine::Project>, ironclaw_engine::EngineError> {
            Ok(None)
        }
        async fn list_projects(
            &self,
        ) -> Result<Vec<ironclaw_engine::Project>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
        async fn save_conversation(
            &self,
            conversation: &ironclaw_engine::ConversationSurface,
        ) -> Result<(), ironclaw_engine::EngineError> {
            let mut conversations = self.conversations.write().await;
            conversations.retain(|existing| existing.id != conversation.id);
            conversations.push(conversation.clone());
            Ok(())
        }
        async fn load_conversation(
            &self,
            id: ironclaw_engine::ConversationId,
        ) -> Result<Option<ironclaw_engine::ConversationSurface>, ironclaw_engine::EngineError>
        {
            Ok(self
                .conversations
                .read()
                .await
                .iter()
                .find(|conversation| conversation.id == id)
                .cloned())
        }
        async fn list_conversations(
            &self,
            user_id: &str,
        ) -> Result<Vec<ironclaw_engine::ConversationSurface>, ironclaw_engine::EngineError>
        {
            Ok(self
                .conversations
                .read()
                .await
                .iter()
                .filter(|conversation| conversation.user_id == user_id)
                .cloned()
                .collect())
        }
        async fn save_memory_doc(
            &self,
            _: &ironclaw_engine::MemoryDoc,
        ) -> Result<(), ironclaw_engine::EngineError> {
            Ok(())
        }
        async fn load_memory_doc(
            &self,
            _: ironclaw_engine::DocId,
        ) -> Result<Option<ironclaw_engine::MemoryDoc>, ironclaw_engine::EngineError> {
            Ok(None)
        }
        async fn list_memory_docs(
            &self,
            _: ironclaw_engine::ProjectId,
        ) -> Result<Vec<ironclaw_engine::MemoryDoc>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
        async fn save_lease(
            &self,
            _: &ironclaw_engine::CapabilityLease,
        ) -> Result<(), ironclaw_engine::EngineError> {
            Ok(())
        }
        async fn load_active_leases(
            &self,
            _: ironclaw_engine::ThreadId,
        ) -> Result<Vec<ironclaw_engine::CapabilityLease>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
        async fn revoke_lease(
            &self,
            _: ironclaw_engine::LeaseId,
            _: &str,
        ) -> Result<(), ironclaw_engine::EngineError> {
            Ok(())
        }
        async fn save_mission(
            &self,
            _: &ironclaw_engine::Mission,
        ) -> Result<(), ironclaw_engine::EngineError> {
            Ok(())
        }
        async fn load_mission(
            &self,
            _: ironclaw_engine::MissionId,
        ) -> Result<Option<ironclaw_engine::Mission>, ironclaw_engine::EngineError> {
            Ok(None)
        }
        async fn list_missions(
            &self,
            _: ironclaw_engine::ProjectId,
        ) -> Result<Vec<ironclaw_engine::Mission>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
        async fn update_mission_status(
            &self,
            _: ironclaw_engine::MissionId,
            _: ironclaw_engine::MissionStatus,
        ) -> Result<(), ironclaw_engine::EngineError> {
            Ok(())
        }
    }

    /// Per-user approval storage: two users' approvals don't collide.
    #[tokio::test]
    async fn pending_approvals_are_per_user() {
        let approvals: RwLock<HashMap<String, PendingApproval>> = RwLock::new(HashMap::new());

        // User A stores an approval
        approvals.write().await.insert(
            "alice".into(),
            PendingApproval {
                request_id: "req-a".into(),
                action_name: "shell".into(),
                thread_id: ironclaw_engine::ThreadId::new(),
                conversation_id: ironclaw_engine::ConversationId::new(),
                call_id: "call-a".into(),
                description: "desc".into(),
                parameters: serde_json::json!({}),
            },
        );

        // User B stores a different approval
        approvals.write().await.insert(
            "bob".into(),
            PendingApproval {
                request_id: "req-b".into(),
                action_name: "web_fetch".into(),
                thread_id: ironclaw_engine::ThreadId::new(),
                conversation_id: ironclaw_engine::ConversationId::new(),
                call_id: "call-b".into(),
                description: "desc".into(),
                parameters: serde_json::json!({}),
            },
        );

        // Taking Alice's approval doesn't affect Bob's
        let alice_approval = approvals.write().await.remove("alice");
        assert_eq!(alice_approval.unwrap().action_name, "shell");

        let bob_approval = approvals.write().await.remove("bob");
        assert_eq!(bob_approval.unwrap().action_name, "web_fetch");
    }

    /// A second approval from the same user overwrites their previous one,
    /// but doesn't affect other users.
    #[tokio::test]
    async fn same_user_approval_overwrites() {
        let approvals: RwLock<HashMap<String, PendingApproval>> = RwLock::new(HashMap::new());

        approvals.write().await.insert(
            "alice".into(),
            PendingApproval {
                request_id: "req-1".into(),
                action_name: "shell".into(),
                thread_id: ironclaw_engine::ThreadId::new(),
                conversation_id: ironclaw_engine::ConversationId::new(),
                call_id: "call-1".into(),
                description: "desc".into(),
                parameters: serde_json::json!({}),
            },
        );
        approvals.write().await.insert(
            "alice".into(),
            PendingApproval {
                request_id: "req-2".into(),
                action_name: "http".into(),
                thread_id: ironclaw_engine::ThreadId::new(),
                conversation_id: ironclaw_engine::ConversationId::new(),
                call_id: "call-2".into(),
                description: "desc".into(),
                parameters: serde_json::json!({}),
            },
        );

        let pending = approvals.write().await.remove("alice");
        assert_eq!(pending.unwrap().action_name, "http");
    }

    /// No pending approval for an unknown user returns None.
    #[tokio::test]
    async fn no_approval_for_unknown_user() {
        let approvals: RwLock<HashMap<String, PendingApproval>> = RwLock::new(HashMap::new());

        let result = approvals.write().await.remove("nobody");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn persist_and_resolve_pending_approval_from_thread_metadata() {
        let store: Arc<dyn Store> = Arc::new(TestStore::new());
        let thread_id = ironclaw_engine::ThreadId::new();
        let conversation_id = ironclaw_engine::ConversationId::new();
        let pending_approvals = RwLock::new(HashMap::new());

        let mut thread = ironclaw_engine::Thread::new(
            "goal",
            ironclaw_engine::ThreadType::Foreground,
            ironclaw_engine::ProjectId::new(),
            ironclaw_engine::ThreadConfig::default(),
        );
        thread.id = thread_id;
        thread
            .transition_to(ironclaw_engine::ThreadState::Running, None)
            .unwrap();
        thread
            .transition_to(
                ironclaw_engine::ThreadState::Waiting,
                Some("approval".into()),
            )
            .unwrap();
        store.save_thread(&thread).await.unwrap();

        let mut conversation = ironclaw_engine::ConversationSurface::new("web", "user1");
        conversation.id = conversation_id;
        conversation.track_thread(thread_id);
        store.save_conversation(&conversation).await.unwrap();

        let pending = PendingApproval {
            request_id: "req-123".into(),
            action_name: "shell".into(),
            thread_id,
            conversation_id,
            call_id: "call-123".into(),
            description: "Tool 'shell' requires approval to execute.".into(),
            parameters: serde_json::json!({"cmd": "ls"}),
        };
        persist_pending_approval(&store, &pending).await.unwrap();

        let resolved =
            resolve_pending_approval_for_thread(&store, &pending_approvals, "user1", None)
                .await
                .unwrap();
        let PendingApprovalResolution::Resolved(resolved) = resolved else {
            panic!("expected resolved pending approval");
        };
        assert_eq!(resolved.action_name, "shell");
        assert_eq!(resolved.thread_id, thread_id);
        assert_eq!(resolved.request_id, "req-123");
        assert_eq!(resolved.parameters["cmd"], "ls");

        clear_pending_approval_metadata(&store, thread_id)
            .await
            .unwrap();
        let thread = store.load_thread(thread_id).await.unwrap().unwrap();
        assert!(thread.metadata.get(PENDING_APPROVAL_METADATA_KEY).is_none());
    }

    #[tokio::test]
    async fn resolve_pending_approval_detects_ambiguity_without_thread_hint() {
        let store: Arc<dyn Store> = Arc::new(TestStore::new());
        let pending_approvals = RwLock::new(HashMap::new());

        for call_id in ["call-1", "call-2"] {
            let thread_id = ironclaw_engine::ThreadId::new();
            let mut thread = ironclaw_engine::Thread::new(
                "goal",
                ironclaw_engine::ThreadType::Foreground,
                ironclaw_engine::ProjectId::new(),
                ironclaw_engine::ThreadConfig::default(),
            );
            thread.id = thread_id;
            thread
                .transition_to(ironclaw_engine::ThreadState::Running, None)
                .unwrap();
            thread
                .transition_to(
                    ironclaw_engine::ThreadState::Waiting,
                    Some("approval".into()),
                )
                .unwrap();
            store.save_thread(&thread).await.unwrap();

            let mut conversation = ironclaw_engine::ConversationSurface::new("web", "user1");
            conversation.track_thread(thread_id);
            let conversation_id = conversation.id;
            store.save_conversation(&conversation).await.unwrap();

            let pending = PendingApproval {
                request_id: format!("req-{call_id}"),
                action_name: "shell".into(),
                thread_id,
                conversation_id,
                call_id: call_id.into(),
                description: "Tool 'shell' requires approval to execute.".into(),
                parameters: serde_json::json!({}),
            };
            persist_pending_approval(&store, &pending).await.unwrap();
        }

        let resolved =
            resolve_pending_approval_for_thread(&store, &pending_approvals, "user1", None)
                .await
                .unwrap();
        assert!(matches!(resolved, PendingApprovalResolution::Ambiguous));
    }
}
