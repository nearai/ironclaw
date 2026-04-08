//! Engine v2 router — handles user messages via the engine when enabled.

use std::sync::{Arc, OnceLock};

use tokio::sync::RwLock;
use tracing::debug;

use ironclaw_engine::{
    Capability, CapabilityRegistry, ConversationManager, LeaseManager, MissionManager,
    PolicyEngine, Project, Store, ThreadConfig, ThreadManager, ThreadOutcome,
};

use ironclaw_common::AppEvent;
use ironclaw_engine::types::{is_shared_owner, shared_owner_id};

use crate::agent::Agent;
use crate::bridge::auth_manager::AuthManager;
use crate::bridge::effect_adapter::EffectBridgeAdapter;
use crate::bridge::llm_adapter::LlmBridgeAdapter;
use crate::bridge::store_adapter::HybridStore;
use crate::channels::web::sse::SseManager;
use crate::channels::{IncomingMessage, OutgoingResponse, StatusUpdate};
use crate::db::Database;
use crate::error::Error;
use crate::extensions::naming::legacy_extension_alias;
use crate::gate::pending::{PendingGate, PendingGateKey};

#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;

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

fn gate_display_parameters(pending: &PendingGate) -> serde_json::Value {
    pending
        .display_parameters
        .clone()
        .unwrap_or_else(|| pending.parameters.clone())
}

fn resumed_action_result_message(
    call_id: &str,
    action_name: &str,
    output: &serde_json::Value,
) -> ironclaw_engine::ThreadMessage {
    let rendered = serde_json::to_string_pretty(output).unwrap_or_else(|_| output.to_string());
    ironclaw_engine::ThreadMessage::action_result(call_id, action_name, rendered)
}

fn resolved_call_id_for_pending_action(
    thread: &ironclaw_engine::Thread,
    pending: &PendingGate,
) -> String {
    // New pending gates persist the exact call_id at insertion time.
    // Only infer from history for legacy rows created before call_id was stored.
    if !pending.call_id.is_empty() {
        return pending.call_id.clone();
    }

    let resolved_ids: HashSet<&str> = thread
        .messages
        .iter()
        .filter_map(|message| {
            (message.role == ironclaw_engine::types::message::MessageRole::ActionResult)
                .then_some(message.action_call_id.as_deref())
                .flatten()
        })
        .collect();

    thread
        .messages
        .iter()
        .rev()
        .find_map(|message| {
            if message.role != ironclaw_engine::types::message::MessageRole::Assistant {
                return None;
            }
            message.action_calls.as_ref().and_then(|calls| {
                calls.iter().find_map(|call| {
                    (call.action_name == pending.action_name
                        && !resolved_ids.contains(call.id.as_str()))
                    .then(|| call.id.clone())
                })
            })
        })
        .unwrap_or_default()
}

async fn notify_pending_gate(
    agent: &Agent,
    state: &EngineState,
    message: &IncomingMessage,
    pending: &PendingGate,
) -> Result<Option<String>, Error> {
    let display_parameters = gate_display_parameters(pending);

    if let Some(ref sse) = state.sse {
        sse.broadcast_for_user(
            &message.user_id,
            AppEvent::GateRequired {
                request_id: pending.request_id.to_string(),
                gate_name: pending.gate_name.clone(),
                tool_name: pending.action_name.clone(),
                description: pending.description.clone(),
                parameters: serde_json::to_string_pretty(&display_parameters)
                    .unwrap_or_else(|_| display_parameters.to_string()),
                resume_kind: serde_json::to_value(&pending.resume_kind).unwrap_or_default(),
                thread_id: pending
                    .scope_thread_id
                    .clone()
                    .or_else(|| Some(pending.thread_id.to_string())),
            },
        );
    }

    match &pending.resume_kind {
        ironclaw_engine::ResumeKind::Approval { allow_always } => {
            let _ = agent
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::ApprovalNeeded {
                        request_id: pending.request_id.to_string(),
                        tool_name: pending.action_name.clone(),
                        description: pending.description.clone(),
                        parameters: display_parameters,
                        allow_always: *allow_always,
                    },
                    &message.metadata,
                )
                .await;

            Ok(Some(format!(
                "Tool '{}' requires approval. Reply 'yes' to approve, 'no' to deny.",
                pending.action_name
            )))
        }
        ironclaw_engine::ResumeKind::Authentication {
            credential_name,
            instructions,
            auth_url,
        } => {
            let _ = agent
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::AuthRequired {
                        extension_name: credential_name.clone(),
                        instructions: Some(instructions.clone()),
                        auth_url: auth_url.clone(),
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
        ironclaw_engine::ResumeKind::External { callback_id } => {
            tracing::debug!(
                gate = %pending.gate_name,
                callback = %callback_id,
                "GatePaused(External)"
            );
            Ok(Some(format!(
                "Waiting for external confirmation (gate: {})...",
                pending.gate_name
            )))
        }
    }
}

async fn insert_and_notify_pending_gate(
    agent: &Agent,
    state: &EngineState,
    message: &IncomingMessage,
    pending: PendingGate,
) -> Result<Option<String>, Error> {
    state
        .pending_gates
        .insert(pending.clone())
        .await
        .map_err(|e| engine_err("pending gate insert", e))?;

    notify_pending_gate(agent, state, message, &pending).await
}

async fn execute_pending_gate_action(
    agent: &Agent,
    state: &EngineState,
    message: &IncomingMessage,
    pending: &PendingGate,
    approval_already_granted: bool,
    approval_event: Option<(String, bool)>,
) -> Result<Option<String>, Error> {
    let thread = state
        .store
        .load_thread(pending.thread_id)
        .await
        .map_err(|e| engine_err("load thread", e))?
        .ok_or_else(|| engine_err("load thread", "thread not found"))?;
    let resolved_call_id = resolved_call_id_for_pending_action(&thread, pending);

    let lease = state
        .thread_manager
        .leases
        .find_lease_for_action(pending.thread_id, &pending.action_name)
        .await
        .ok_or_else(|| {
            engine_err(
                "resume lease",
                format!("no active lease covers action '{}'", pending.action_name),
            )
        })?;

    let exec_ctx = ironclaw_engine::ThreadExecutionContext {
        thread_id: pending.thread_id,
        thread_type: thread.thread_type,
        project_id: thread.project_id,
        user_id: thread.user_id.clone(),
        step_id: ironclaw_engine::StepId::new(),
        current_call_id: Some(resolved_call_id.clone()),
        source_channel: Some(pending.source_channel.clone()),
    };

    state.effect_adapter.reset_call_count();
    match state
        .effect_adapter
        .execute_resolved_pending_action(
            &pending.action_name,
            pending.parameters.clone(),
            &lease,
            &exec_ctx,
            approval_already_granted,
        )
        .await
    {
        Ok(result) => {
            state
                .thread_manager
                .resume_thread(
                    pending.thread_id,
                    message.user_id.clone(),
                    Some(resumed_action_result_message(
                        &resolved_call_id,
                        &pending.action_name,
                        &result.output,
                    )),
                    approval_event,
                    Some(resolved_call_id),
                )
                .await
                .map_err(|e| engine_err("resume error", e))?;
            await_thread_outcome(
                agent,
                state,
                message,
                pending.conversation_id,
                pending.thread_id,
            )
            .await
        }
        Err(ironclaw_engine::EngineError::GatePaused {
            gate_name,
            action_name,
            call_id,
            parameters,
            resume_kind,
            resume_output,
        }) => {
            let display_parameters = state
                .effect_adapter
                .tools()
                .get(&action_name)
                .await
                .map(|tool| crate::tools::redact_params(&parameters, tool.sensitive_params()));
            let pending_gate = PendingGate {
                request_id: uuid::Uuid::new_v4(),
                gate_name,
                user_id: message.user_id.clone(),
                thread_id: pending.thread_id,
                scope_thread_id: pending.scope_thread_id.clone(),
                conversation_id: pending.conversation_id,
                source_channel: message.channel.clone(),
                action_name: action_name.clone(),
                call_id,
                parameters: *parameters,
                display_parameters,
                description: format!(
                    "Tool '{}' requires {}.",
                    action_name,
                    resume_kind.kind_name()
                ),
                resume_kind: *resume_kind,
                created_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
                // Preserve the initiating user prompt when a resumed gate
                // immediately chains into another gate (for example approval
                // followed by authentication). OAuth callback replay depends
                // on this being the original request, not the approval payload.
                original_message: pending
                    .original_message
                    .clone()
                    .or_else(|| Some(message.content.clone())),
                resume_output: resume_output.map(|value| *value),
                approval_already_granted: approval_already_granted
                    || matches!(
                        pending.resume_kind,
                        ironclaw_engine::ResumeKind::Approval { .. }
                    ),
            };
            insert_and_notify_pending_gate(agent, state, message, pending_gate).await
        }
        Err(e) => Err(engine_err("execute pending gate action", e)),
    }
}

/// Resolve the default project for a user, creating one if needed.
///
/// In multi-user deployments each user gets their own project so threads,
/// missions, and memory docs are isolated. The owner's project (passed as
/// `fallback`) is used when the user IS the owner, avoiding an extra store
/// lookup in the common single-user case.
async fn resolve_user_project(
    store: &Arc<dyn Store>,
    user_id: &str,
    fallback: ironclaw_engine::ProjectId,
) -> Result<ironclaw_engine::ProjectId, Error> {
    // Fast path: check if fallback project belongs to this user
    if let Ok(Some(project)) = store.load_project(fallback).await
        && project.is_owned_by(user_id)
    {
        return Ok(fallback);
    }

    // Look for an existing default project owned by this user
    let projects = store
        .list_projects(user_id)
        .await
        .map_err(|e| engine_err("project lookup", e))?;

    if let Some(project) = projects.iter().find(|p| p.name == "default") {
        return Ok(project.id);
    }

    // Create a new default project for this user
    let project = ironclaw_engine::Project::new(user_id, "default", "Default project");
    let pid = project.id;
    store
        .save_project(&project)
        .await
        .map_err(|e| engine_err("create project", e))?;
    debug!(user_id, project_id = %pid, "created default project for user");
    Ok(pid)
}

/// Persistent engine state that lives across messages.
struct EngineState {
    thread_manager: Arc<ThreadManager>,
    conversation_manager: Arc<ConversationManager>,
    effect_adapter: Arc<EffectBridgeAdapter>,
    store: Arc<dyn Store>,
    default_project_id: ironclaw_engine::ProjectId,
    /// Unified pending gate store — keyed by (user_id, thread_id).
    pending_gates: Arc<crate::gate::store::PendingGateStore>,
    /// SSE manager for broadcasting AppEvents to the web gateway.
    sse: Option<Arc<SseManager>>,
    /// V1 database for writing conversation messages (gateway reads from here).
    db: Option<Arc<dyn Database>>,
    /// Secrets store for storing credentials after auth flow.
    secrets_store: Option<Arc<dyn crate::secrets::SecretsStore + Send + Sync>>,
    /// Centralized auth manager for setup instruction lookup and credential checks.
    auth_manager: Option<Arc<AuthManager>>,
}

/// Global engine state, initialized on first use.
static ENGINE_STATE: OnceLock<RwLock<Option<EngineState>>> = OnceLock::new();

enum PendingGateResolution {
    None,
    Resolved(Box<PendingGate>),
    Ambiguous,
}

fn parse_engine_thread_id(scope: Option<&str>) -> Option<ironclaw_engine::ThreadId> {
    scope
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(ironclaw_engine::ThreadId)
}

fn parse_scope_uuid(scope: Option<&str>) -> Option<uuid::Uuid> {
    scope.and_then(|s| uuid::Uuid::parse_str(s).ok())
}

async fn reconcile_pending_gate_state(
    store: &Arc<dyn Store>,
    pending_gates: &crate::gate::store::PendingGateStore,
) -> Result<(), Error> {
    let restored_gates = pending_gates.list_all().await;
    let gate_keys: HashSet<_> = restored_gates.iter().map(PendingGate::key).collect();

    for gate in &restored_gates {
        let thread = store
            .load_thread(gate.thread_id)
            .await
            .map_err(|e| engine_err("load thread", e))?;
        let Some(thread) = thread else {
            let _ = pending_gates.discard(&gate.key()).await;
            continue;
        };

        if thread.state != ironclaw_engine::ThreadState::Waiting
            || !thread.is_owned_by(&gate.user_id)
        {
            let _ = pending_gates.discard(&gate.key()).await;
        }
    }

    let projects = store
        .list_all_projects()
        .await
        .map_err(|e| engine_err("list all projects", e))?;
    for project in projects {
        let threads = store
            .list_all_threads(project.id)
            .await
            .map_err(|e| engine_err("list all threads", e))?;
        for mut thread in threads {
            if thread.state != ironclaw_engine::ThreadState::Waiting {
                continue;
            }
            let key = PendingGateKey {
                user_id: thread.user_id.clone(),
                thread_id: thread.id,
            };
            if gate_keys.contains(&key) {
                continue;
            }

            if let Err(e) = thread.transition_to(
                ironclaw_engine::ThreadState::Failed,
                Some("pending gate missing during recovery".into()),
            ) {
                debug!(thread_id = %thread.id, error = %e, "failed to reconcile waiting thread");
                continue;
            }
            store
                .save_thread(&thread)
                .await
                .map_err(|e| engine_err("save reconciled thread", e))?;
        }
    }

    Ok(())
}

async fn fail_orphaned_waiting_thread_if_needed(
    state: &EngineState,
    user_id: &str,
    thread_id: ironclaw_engine::ThreadId,
) -> Result<bool, Error> {
    if state
        .pending_gates
        .peek(&PendingGateKey {
            user_id: user_id.to_string(),
            thread_id,
        })
        .await
        .is_some()
    {
        return Ok(false);
    }

    let Some(mut thread) = state
        .store
        .load_thread(thread_id)
        .await
        .map_err(|e| engine_err("load thread", e))?
    else {
        return Ok(false);
    };

    if !thread.is_owned_by(user_id) || thread.state != ironclaw_engine::ThreadState::Waiting {
        return Ok(false);
    }

    thread
        .transition_to(
            ironclaw_engine::ThreadState::Failed,
            Some("pending gate missing before resume".into()),
        )
        .map_err(|e| engine_err("reconcile waiting thread", e))?;
    state
        .store
        .save_thread(&thread)
        .await
        .map_err(|e| engine_err("save reconciled thread", e))?;
    Ok(true)
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

    let effect_adapter = Arc::new(
        EffectBridgeAdapter::new(
            agent.tools().clone(),
            agent.safety().clone(),
            agent.hooks().clone(),
        )
        .with_global_auto_approve(agent.config().auto_approve_tools),
    );

    // Build centralized auth manager for pre-flight credential checks.
    let has_secrets = agent.tools().secrets_store().is_some();
    let has_cred_reg = agent.tools().credential_registry().is_some();
    debug!(
        has_secrets_store = has_secrets,
        has_credential_registry = has_cred_reg,
        "engine v2: auth manager init check"
    );
    let auth_manager = if let Some(mgr) = agent.deps.auth_manager.clone() {
        effect_adapter.set_auth_manager(Arc::clone(&mgr)).await;
        debug!("engine v2: auth manager set on effect adapter");
        Some(mgr)
    } else if let Some(ss) = agent.tools().secrets_store().cloned() {
        let mgr = Arc::new(AuthManager::new(
            ss,
            agent.deps.skill_registry.clone(),
            agent.deps.extension_manager.clone(),
            Some(Arc::clone(agent.tools())),
        ));
        effect_adapter.set_auth_manager(Arc::clone(&mgr)).await;
        debug!("engine v2: auth manager set on effect adapter");
        Some(mgr)
    } else {
        debug!("engine v2: no secrets store — auth manager NOT created");
        None
    };

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

    let mut capabilities = CapabilityRegistry::new();

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
                description: "Create a new mission (routine). Use when the user wants to set up a recurring task, scheduled check, or periodic routine. Results are delivered to the current channel by default.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Short name for the mission/routine"},
                        "goal": {"type": "string", "description": "What this mission should accomplish each run"},
                        "cadence": {"type": "string", "description": "How often to run: 'hourly', '30m', '6h', 'daily', 'manual'"},
                        "notify_channels": {"type": "array", "items": {"type": "string"}, "description": "Channels to deliver results to (e.g. ['gateway', 'repl']). Defaults to current channel."}
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
                name: "mission_update".into(),
                description: "Update a mission/routine. Change name, goal, cadence, notification channels, daily budget, or success criteria.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Mission/routine ID to update"},
                        "name": {"type": "string", "description": "New name"},
                        "goal": {"type": "string", "description": "New goal"},
                        "cadence": {"type": "string", "description": "New cadence: 'hourly', '30m', '6h', 'daily', 'manual'"},
                        "notify_channels": {"type": "array", "items": {"type": "string"}, "description": "Channels to deliver results to (e.g. ['gateway', 'repl'])"},
                        "max_threads_per_day": {"type": "integer", "description": "Max threads per day (0 = unlimited)"},
                        "success_criteria": {"type": "string", "description": "Criteria for declaring mission complete"}
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

    // Migrate legacy records: pre-existing engine records deserialize without a
    // user_id field and get the serde default "legacy". Stamp the owner's identity
    // onto them so user-scoped queries find them after upgrade.
    let owner_id = &agent.deps.owner_id;
    migrate_legacy_user_ids(&store_dyn, owner_id).await;

    // Reuse the persisted default project when available.
    let project_id = match store
        .list_projects(owner_id)
        .await
        .map_err(|e| engine_err("store error", e))?
        .into_iter()
        .find(|project| project.name == "default")
    {
        Some(project) => project.id,
        None => {
            let project = Project::new(owner_id, "default", "Default project for engine v2");
            let project_id = project.id;
            store
                .save_project(&project)
                .await
                .map_err(|e| engine_err("store error", e))?;
            project_id
        }
    };

    let conversation_manager = Arc::new(ConversationManager::new(
        Arc::clone(&thread_manager),
        store.clone(),
    ));
    if let Err(e) = conversation_manager
        .bootstrap_user(&agent.deps.owner_id)
        .await
    {
        debug!("engine v2: bootstrap_user failed: {e}");
    }

    // Create mission manager and start cron ticker. Attach:
    // - WorkspaceReader so missions with `context_paths` can preload
    //   workspace documents into their meta-prompt at fire time.
    // - BudgetGate over the host's CostGuard so a mission fire is refused
    //   when the user has exhausted their daily LLM budget.
    let mut mission_manager_inner =
        MissionManager::new(store_dyn.clone(), Arc::clone(&thread_manager));
    if let Some(workspace) = agent.workspace().cloned() {
        let reader: Arc<dyn ironclaw_engine::WorkspaceReader> =
            Arc::new(crate::bridge::WorkspaceReaderAdapter::new(workspace));
        mission_manager_inner = mission_manager_inner.with_workspace_reader(reader);
    }
    let cost_guard = Arc::clone(&agent.deps.cost_guard);
    let budget_gate: Arc<dyn ironclaw_engine::BudgetGate> =
        Arc::new(crate::bridge::CostGuardBudgetGate::new(cost_guard));
    mission_manager_inner = mission_manager_inner.with_budget_gate(budget_gate);
    let mission_manager = Arc::new(mission_manager_inner);
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

    // Subscribe to mission outcome notifications and route results to channels.
    {
        let mut notification_rx = mission_manager.subscribe_notifications();
        let channels = Arc::clone(&agent.channels);
        let sse_ref = agent.deps.sse_tx.clone();
        let db_ref = agent.deps.store.clone();
        let conv_mgr_ref = Arc::clone(&conversation_manager);
        tokio::spawn(async move {
            loop {
                match notification_rx.recv().await {
                    Ok(notif) => {
                        handle_mission_notification(
                            &notif,
                            &channels,
                            sse_ref.as_ref(),
                            db_ref.as_ref(),
                            Some(conv_mgr_ref.as_ref()),
                        )
                        .await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        debug!("mission notification receiver lagged by {n}");
                    }
                }
            }
        });
    }

    // Ensure per-user learning missions exist for the owner
    if let Err(e) = mission_manager
        .ensure_learning_missions(project_id, owner_id)
        .await
    {
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

    let pending_gates = Arc::new(crate::gate::store::PendingGateStore::new(Some(Arc::new(
        crate::gate::persistence::FileGatePersistence::with_default_path(),
    ))));
    if let Err(e) = pending_gates.restore_from_persistence().await {
        debug!("engine v2: failed to restore pending gates: {e}");
    }
    if let Err(e) = reconcile_pending_gate_state(&store_dyn, &pending_gates).await {
        debug!("engine v2: pending gate reconciliation failed: {e}");
    }

    *guard = Some(EngineState {
        thread_manager,
        conversation_manager,
        effect_adapter,
        store: store.clone(),
        default_project_id: project_id,
        pending_gates,
        sse: agent.deps.sse_tx.clone(),
        db: agent.deps.store.clone(),
        secrets_store: agent.tools().secrets_store().cloned(),
        auth_manager,
    });

    Ok(())
}

async fn resolve_pending_gate_for_user(
    pending_gates: &crate::gate::store::PendingGateStore,
    user_id: &str,
    thread_id_hint: Option<&str>,
) -> PendingGateResolution {
    let hinted_uuid = parse_scope_uuid(thread_id_hint);
    let hinted_scope = thread_id_hint;
    let candidates: Vec<_> = pending_gates
        .list_for_user(user_id)
        .await
        .into_iter()
        .filter(|gate| {
            hinted_scope.is_none_or(|hint| {
                gate.scope_thread_id.as_deref() == Some(hint)
                    || hinted_uuid.is_none_or(|uuid| {
                        gate.thread_id.0 == uuid || gate.conversation_id.0 == uuid
                    })
            })
        })
        .collect();

    match candidates.len() {
        0 => PendingGateResolution::None,
        1 => PendingGateResolution::Resolved(Box::new(
            candidates.into_iter().next().unwrap(), // safety: len==1 guarantees Some
        )),
        _ if hinted_uuid.is_some() => PendingGateResolution::Resolved(Box::new(
            candidates
                .into_iter()
                .max_by_key(|gate| gate.created_at)
                .unwrap(), // safety: len>=2 guarantees Some
        )),
        _ => PendingGateResolution::Ambiguous,
    }
}

pub async fn get_engine_pending_gate(
    user_id: &str,
    thread_id: Option<&str>,
) -> Result<Option<crate::gate::pending::PendingGateView>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(None);
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(None);
    };

    match resolve_pending_gate_for_user(&state.pending_gates, user_id, thread_id).await {
        PendingGateResolution::Resolved(gate) => Ok(Some(
            crate::gate::pending::PendingGateView::from(gate.as_ref()),
        )),
        PendingGateResolution::None | PendingGateResolution::Ambiguous => Ok(None),
    }
}

pub enum AuthCallbackContinuation {
    None,
    ResolveGateExternal {
        channel: String,
        thread_scope: Option<String>,
        request_id: uuid::Uuid,
    },
    ReplayMessage {
        channel: String,
        thread_scope: Option<String>,
        content: String,
    },
}

pub async fn resolve_engine_auth_callback(
    user_id: &str,
    credential_name: &str,
) -> Result<AuthCallbackContinuation, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(AuthCallbackContinuation::None);
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(AuthCallbackContinuation::None);
    };

    let mut matching: Vec<_> = state
        .pending_gates
        .list_for_user(user_id)
        .await
        .into_iter()
        .filter(|gate| {
            matches!(
                &gate.resume_kind,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name: gate_credential,
                    ..
                } if gate_credential == credential_name
            )
        })
        .collect();

    matching.sort_by_key(|gate| gate.created_at);
    let Some(pending) = matching.pop() else {
        return Ok(AuthCallbackContinuation::None);
    };

    if pending.action_name == "authentication_fallback" {
        if let Some(content) = pending.original_message.clone() {
            return Ok(AuthCallbackContinuation::ReplayMessage {
                channel: pending.source_channel,
                thread_scope: pending.scope_thread_id,
                content,
            });
        }
        tracing::warn!(
            user_id = %user_id,
            credential_name = %credential_name,
            thread_id = %pending.thread_id,
            "OAuth callback matched authentication fallback without a replayable request"
        );
        return Ok(AuthCallbackContinuation::None);
    }

    Ok(AuthCallbackContinuation::ResolveGateExternal {
        channel: pending.source_channel,
        thread_scope: pending.scope_thread_id,
        request_id: pending.request_id,
    })
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
    let pending = match resolve_pending_gate_for_user(&state.pending_gates, &message.user_id, None)
        .await
    {
        PendingGateResolution::Resolved(p) => p,
        PendingGateResolution::None => {
            debug!(user_id = %message.user_id, "engine v2: no pending approval for user, ignoring");
            return Ok(Some("No pending approval for this thread.".into()));
        }
        PendingGateResolution::Ambiguous => {
            return Ok(Some(
                "Multiple pending gates are waiting. Resolve from the original thread or retry with that thread selected.".into(),
            ));
        }
    };

    if !matches!(
        pending.resume_kind,
        ironclaw_engine::ResumeKind::Approval { .. }
    ) {
        return Ok(Some(
            "The selected pending gate is not an approval request.".into(),
        ));
    }

    let request_id = pending.request_id;
    let thread_id = pending.thread_id;
    drop(guard);
    resolve_gate(
        agent,
        message,
        thread_id,
        request_id,
        if approved {
            ironclaw_engine::GateResolution::Approved { always }
        } else {
            ironclaw_engine::GateResolution::Denied { reason: None }
        },
    )
    .await
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

    if let Some(thread_id) = parse_engine_thread_id(message.conversation_scope())
        && let Some(gate) = state
            .pending_gates
            .peek(&crate::gate::pending::PendingGateKey {
                user_id: message.user_id.clone(),
                thread_id,
            })
            .await
        && gate.request_id == request_id.to_string()
        && matches!(
            gate.resume_kind,
            ironclaw_engine::ResumeKind::Approval { .. }
        )
    {
        drop(guard);
        return resolve_gate(
            agent,
            message,
            thread_id,
            request_id,
            if approved {
                ironclaw_engine::GateResolution::Approved { always }
            } else {
                ironclaw_engine::GateResolution::Denied { reason: None }
            },
        )
        .await;
    }

    let pending = state
        .pending_gates
        .list_for_user(&message.user_id)
        .await
        .into_iter()
        .find(|gate| {
            matches!(
                gate.resume_kind,
                ironclaw_engine::ResumeKind::Approval { .. }
            ) && gate.request_id == request_id
        });
    drop(guard);

    if let Some(pending) = pending {
        return resolve_gate(
            agent,
            message,
            pending.thread_id,
            request_id,
            if approved {
                ironclaw_engine::GateResolution::Approved { always }
            } else {
                ironclaw_engine::GateResolution::Denied { reason: None }
            },
        )
        .await;
    }

    debug!(
        user_id = %message.user_id,
        request_id = %request_id,
        "engine v2: no matching pending approval for request_id"
    );
    Ok(Some("No matching pending approval found.".into()))
}

pub async fn handle_external_callback(
    agent: &Agent,
    message: &IncomingMessage,
    request_id: uuid::Uuid,
) -> Result<Option<String>, Error> {
    init_engine(agent).await?;

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    if let Some(thread_id) = parse_engine_thread_id(message.conversation_scope())
        && let Some(gate) = state
            .pending_gates
            .peek(&crate::gate::pending::PendingGateKey {
                user_id: message.user_id.clone(),
                thread_id,
            })
            .await
        && gate.request_id == request_id.to_string()
        && matches!(
            gate.resume_kind,
            ironclaw_engine::ResumeKind::Authentication { .. }
        )
    {
        drop(guard);
        return resolve_gate(
            agent,
            message,
            thread_id,
            request_id,
            ironclaw_engine::GateResolution::ExternalCallback {
                payload: serde_json::Value::Null,
            },
        )
        .await;
    }

    let pending = state
        .pending_gates
        .list_for_user(&message.user_id)
        .await
        .into_iter()
        .find(|gate| {
            matches!(
                gate.resume_kind,
                ironclaw_engine::ResumeKind::Authentication { .. }
            ) && gate.request_id == request_id
        });
    drop(guard);

    if let Some(pending) = pending {
        return resolve_gate(
            agent,
            message,
            pending.thread_id,
            request_id,
            ironclaw_engine::GateResolution::ExternalCallback {
                payload: serde_json::Value::Null,
            },
        )
        .await;
    }

    debug!(
        user_id = %message.user_id,
        request_id = %request_id,
        "engine v2: no matching pending auth gate for external callback"
    );
    Ok(Some(
        "No matching pending authentication gate found.".into(),
    ))
}

/// Resolve a unified pending gate.
///
/// This is the single entry point for resolving gates stored in the
/// [`PendingGateStore`]. It atomically verifies request_id, channel
/// authorization, and expiry before resuming or stopping the thread.
///
/// Replaces the separate approval and auth resolution paths for new
/// code paths using the unified gate abstraction.
pub async fn resolve_gate(
    agent: &Agent,
    message: &IncomingMessage,
    thread_id: ironclaw_engine::ThreadId,
    request_id: uuid::Uuid,
    resolution: ironclaw_engine::GateResolution,
) -> Result<Option<String>, Error> {
    init_engine(agent).await?;

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    let key = crate::gate::pending::PendingGateKey {
        user_id: message.user_id.clone(),
        thread_id,
    };

    let pending = state
        .pending_gates
        .take_verified(&key, request_id, &message.channel)
        .await
        .map_err(|e| {
            use crate::gate::store::GateStoreError;
            match e {
                GateStoreError::ChannelMismatch { expected, actual } => engine_err(
                    "authorization",
                    format!("Channel '{actual}' cannot resolve gates from channel '{expected}'"),
                ),
                GateStoreError::RequestIdMismatch => {
                    engine_err("stale", "Approval request is stale or already resolved")
                }
                GateStoreError::Expired => engine_err("expired", "Approval request has expired"),
                other => engine_err("gate", other),
            }
        })?;

    match resolution {
        ironclaw_engine::GateResolution::Approved { always } => {
            if let Some(ref sse) = state.sse {
                sse.broadcast_for_user(
                    &message.user_id,
                    AppEvent::GateResolved {
                        request_id: pending.request_id.to_string(),
                        gate_name: pending.gate_name.clone(),
                        tool_name: pending.action_name.clone(),
                        resolution: if always {
                            "approved_always"
                        } else {
                            "approved"
                        }
                        .into(),
                        message: "Gate approved. Resuming execution.".into(),
                        thread_id: pending
                            .scope_thread_id
                            .clone()
                            .or_else(|| Some(pending.thread_id.to_string())),
                    },
                );
            }
            let legacy_registry_name = legacy_extension_alias(&pending.action_name);
            if always {
                state
                    .effect_adapter
                    .auto_approve_tool(&pending.action_name)
                    .await;
                if let Some(ref registry_name) = legacy_registry_name {
                    state.effect_adapter.auto_approve_tool(registry_name).await;
                }
            }
            let result = execute_pending_gate_action(
                agent,
                state,
                message,
                &pending,
                true,
                Some((pending.call_id.clone(), true)),
            )
            .await;

            if always && result.is_err() {
                state
                    .effect_adapter
                    .revoke_auto_approve(&pending.action_name)
                    .await;
                if let Some(registry_name) = legacy_registry_name {
                    state
                        .effect_adapter
                        .revoke_auto_approve(&registry_name)
                        .await;
                }
            }
            return result;
        }

        ironclaw_engine::GateResolution::Denied { reason } => {
            if let Some(ref sse) = state.sse {
                sse.broadcast_for_user(
                    &message.user_id,
                    AppEvent::GateResolved {
                        request_id: pending.request_id.to_string(),
                        gate_name: pending.gate_name.clone(),
                        tool_name: pending.action_name.clone(),
                        resolution: "denied".into(),
                        message: "Gate denied.".into(),
                        thread_id: pending
                            .scope_thread_id
                            .clone()
                            .or_else(|| Some(pending.thread_id.to_string())),
                    },
                );
            }
            let _ = agent
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status("Tool call denied.".into()),
                    &message.metadata,
                )
                .await;

            let deny_msg = ironclaw_engine::ThreadMessage::user(format!(
                "User denied action '{}'. Do not execute it; choose an alternative approach.{}",
                pending.action_name,
                reason
                    .as_deref()
                    .map(|r| format!(" Reason: {r}"))
                    .unwrap_or_default()
            ));

            state.effect_adapter.reset_call_count();
            state
                .thread_manager
                .resume_thread(
                    pending.thread_id,
                    message.user_id.clone(),
                    Some(deny_msg),
                    Some((pending.call_id.clone(), false)),
                    None,
                )
                .await
                .map_err(|e| engine_err("resume error", e))?;
        }

        ironclaw_engine::GateResolution::Cancelled => {
            if let Some(ref sse) = state.sse {
                sse.broadcast_for_user(
                    &message.user_id,
                    AppEvent::GateResolved {
                        request_id: pending.request_id.to_string(),
                        gate_name: pending.gate_name.clone(),
                        tool_name: pending.action_name.clone(),
                        resolution: "cancelled".into(),
                        message: "Gate cancelled.".into(),
                        thread_id: pending
                            .scope_thread_id
                            .clone()
                            .or_else(|| Some(pending.thread_id.to_string())),
                    },
                );
            }
            // Stop the thread entirely (fix: 49b4c398 — cancel during auth
            // was misrouted to approval handler)
            if let Err(e) = state
                .thread_manager
                .stop_thread(pending.thread_id, &message.user_id)
                .await
            {
                tracing::debug!(error = %e, "Failed to stop thread on cancel");
            }
            return Ok(Some("Cancelled.".into()));
        }

        ironclaw_engine::GateResolution::CredentialProvided { token } => {
            // Store credential then RESUME (not retry) — preserves thread work
            if let ironclaw_engine::ResumeKind::Authentication {
                ref credential_name,
                ..
            } = pending.resume_kind
            {
                if let Some(ref sse) = state.sse {
                    sse.broadcast_for_user(
                        &message.user_id,
                        AppEvent::GateResolved {
                            request_id: pending.request_id.to_string(),
                            gate_name: pending.gate_name.clone(),
                            tool_name: pending.action_name.clone(),
                            resolution: "credential_provided".into(),
                            message: "Credential received. Resuming execution.".into(),
                            thread_id: pending
                                .scope_thread_id
                                .clone()
                                .or_else(|| Some(pending.thread_id.to_string())),
                        },
                    );
                }
                if let Some(ref auth_manager) = state.auth_manager {
                    match auth_manager
                        .submit_auth_token(credential_name, &token, &message.user_id)
                        .await
                    {
                        Ok(result) if result.activated => {
                            let _ = agent
                                .channels
                                .send_status(
                                    &message.channel,
                                    StatusUpdate::AuthCompleted {
                                        extension_name: credential_name.clone(),
                                        success: true,
                                        message: format!("{}. Resuming...", result.message),
                                    },
                                    &message.metadata,
                                )
                                .await;
                        }
                        Ok(result) => {
                            let _ = agent
                                .channels
                                .send_status(
                                    &message.channel,
                                    StatusUpdate::AuthRequired {
                                        extension_name: credential_name.clone(),
                                        instructions: Some(result.message.clone()),
                                        auth_url: result.auth_url.clone(),
                                        setup_url: None,
                                    },
                                    &message.metadata,
                                )
                                .await;
                            return Ok(Some(result.message));
                        }
                        Err(crate::extensions::ExtensionError::ValidationFailed(msg)) => {
                            let _ = agent
                                .channels
                                .send_status(
                                    &message.channel,
                                    StatusUpdate::AuthRequired {
                                        extension_name: credential_name.clone(),
                                        instructions: Some(msg.clone()),
                                        auth_url: None,
                                        setup_url: None,
                                    },
                                    &message.metadata,
                                )
                                .await;
                            return Ok(Some(msg));
                        }
                        Err(error) => {
                            let msg = error.to_string();
                            let _ = agent
                                .channels
                                .send_status(
                                    &message.channel,
                                    StatusUpdate::AuthCompleted {
                                        extension_name: credential_name.clone(),
                                        success: false,
                                        message: msg.clone(),
                                    },
                                    &message.metadata,
                                )
                                .await;
                            return Ok(Some(msg));
                        }
                    }
                } else if let Some(ref ss) = state.secrets_store {
                    let params = crate::secrets::CreateSecretParams::new(credential_name, &token);
                    ss.create(&message.user_id, params)
                        .await
                        .map_err(|e| engine_err("secrets", e))?;
                }

                if pending.action_name == "authentication_fallback"
                    && let Some(retry_content) = pending.original_message.clone()
                {
                    let retry_msg = IncomingMessage {
                        content: retry_content.clone(),
                        channel: pending.source_channel.clone(),
                        user_id: pending.user_id.clone(),
                        metadata: message.metadata.clone(),
                        ..message.clone()
                    };
                    drop(guard);
                    return Box::pin(handle_with_engine_inner(
                        agent,
                        &retry_msg,
                        &retry_content,
                        1,
                    ))
                    .await;
                }

                if let Some(resume_output) = pending.resume_output.clone() {
                    state
                        .thread_manager
                        .resume_thread(
                            pending.thread_id,
                            message.user_id.clone(),
                            Some(resumed_action_result_message(
                                &pending.call_id,
                                &pending.action_name,
                                &resume_output,
                            )),
                            None,
                            Some(pending.call_id.clone()),
                        )
                        .await
                        .map_err(|e| engine_err("resume error", e))?;
                } else {
                    return execute_pending_gate_action(
                        agent,
                        state,
                        message,
                        &pending,
                        pending.approval_already_granted,
                        None,
                    )
                    .await;
                }
            } else {
                return Err(engine_err(
                    "resolution mismatch",
                    "CredentialProvided sent for non-authentication gate",
                ));
            }
        }

        ironclaw_engine::GateResolution::ExternalCallback { .. } => {
            if let Some(ref sse) = state.sse {
                sse.broadcast_for_user(
                    &message.user_id,
                    AppEvent::GateResolved {
                        request_id: pending.request_id.to_string(),
                        gate_name: pending.gate_name.clone(),
                        tool_name: pending.action_name.clone(),
                        resolution: "external_callback".into(),
                        message: "External callback received. Resuming execution.".into(),
                        thread_id: pending
                            .scope_thread_id
                            .clone()
                            .or_else(|| Some(pending.thread_id.to_string())),
                    },
                );
            }
            if let Some(resume_output) = pending.resume_output.clone() {
                state
                    .thread_manager
                    .resume_thread(
                        pending.thread_id,
                        message.user_id.clone(),
                        Some(resumed_action_result_message(
                            &pending.call_id,
                            &pending.action_name,
                            &resume_output,
                        )),
                        None,
                        Some(pending.call_id.clone()),
                    )
                    .await
                    .map_err(|e| engine_err("resume error", e))?;
            } else {
                return execute_pending_gate_action(
                    agent,
                    state,
                    message,
                    &pending,
                    pending.approval_already_granted,
                    None,
                )
                .await;
            }
        }
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
            if let Err(e) = state
                .thread_manager
                .stop_thread(*tid, &message.user_id)
                .await
            {
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

/// Handle `/expected <description>` — collect context from the engine thread
/// and fire the expected-behavior learning mission.
///
/// In v2, conversation history lives in engine threads (not v1 sessions).
/// This handler finds the most recent thread for the user's conversation,
/// extracts the last N messages, and fires the system event.
pub async fn handle_expected(
    agent: &Agent,
    message: &IncomingMessage,
    description: &str,
) -> Result<Option<String>, Error> {
    init_engine(agent).await?;

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    // Find the conversation for this channel+user
    let scope = message.conversation_scope();
    let channel_key = match scope {
        Some(tid) => format!("{}:{}", message.channel, tid),
        None => message.channel.clone(),
    };

    let conv_id = state
        .conversation_manager
        .get_or_create_conversation(&channel_key, &message.user_id)
        .await
        .map_err(|e| engine_err("conversation error", e))?;

    let conv = state.conversation_manager.get_conversation(conv_id).await;

    // Find the most recent thread in this conversation (active or completed)
    let recent_thread = find_most_recent_thread(state, &conv, &message.user_id).await;

    let Some(thread) = recent_thread else {
        return Ok(Some(
            "No conversation history to attach feedback to.".into(),
        ));
    };

    // Extract recent messages (last 10) as context for the learning mission
    let start = thread.messages.len().saturating_sub(10);
    let recent_messages: Vec<serde_json::Value> = thread.messages[start..]
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role,
                "content_preview": m.content.chars().take(500).collect::<String>(),
                "action_name": m.action_name,
            })
        })
        .collect();

    // Extract tool call events for richer context
    let tool_events: Vec<serde_json::Value> = thread
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            ironclaw_engine::EventKind::ActionExecuted {
                action_name,
                params_summary,
                ..
            } => Some(serde_json::json!({
                "tool": action_name,
                "params": params_summary,
                "success": true,
            })),
            ironclaw_engine::EventKind::ActionFailed {
                action_name, error, ..
            } => Some(serde_json::json!({
                "tool": action_name,
                "error": error,
                "success": false,
            })),
            _ => None,
        })
        .collect();

    let payload = serde_json::json!({
        "expected_behavior": description,
        "thread_id": thread.id.to_string(),
        "goal": thread.goal,
        "recent_messages": recent_messages,
        "tool_events": tool_events,
        "step_count": thread.step_count,
        "thread_state": thread.state,
    });

    // Fire the expected-behavior learning mission
    let mgr = state.effect_adapter.mission_manager().await;
    let fired = if let Some(mgr) = mgr {
        match mgr
            .fire_on_system_event(
                "user_feedback",
                "expected_behavior",
                &message.user_id,
                Some(payload),
            )
            .await
        {
            Ok(ids) => ids.len(),
            Err(e) => {
                debug!("failed to fire expected-behavior mission: {e}");
                0
            }
        }
    } else {
        0
    };

    if fired > 0 {
        Ok(Some(format!(
            "Feedback captured. Fired {fired} self-improvement thread(s) to investigate."
        )))
    } else {
        Ok(Some(
            "Feedback noted but no self-improvement missions are configured to handle it. \
             The engine will use this context in future learning cycles."
                .into(),
        ))
    }
}

/// Find the most recent thread in a conversation (checks active threads first,
/// then falls back to the last completed thread visible in conversation entries).
async fn find_most_recent_thread(
    state: &EngineState,
    conv: &Option<ironclaw_engine::ConversationSurface>,
    user_id: &str,
) -> Option<ironclaw_engine::Thread> {
    let conv = conv.as_ref()?;

    // Try active threads first (most recent interaction)
    for tid in conv.active_threads.iter().rev() {
        if let Ok(Some(thread)) = state.store.load_thread(*tid).await
            && thread.is_owned_by(user_id)
        {
            return Some(thread);
        }
    }

    // Fall back to the most recent thread referenced in entries
    for entry in conv.entries.iter().rev() {
        let Some(tid) = entry.origin_thread_id else {
            continue;
        };
        if let Ok(Some(thread)) = state.store.load_thread(tid).await
            && thread.is_owned_by(user_id)
        {
            return Some(thread);
        }
    }

    None
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
                let _ = state
                    .thread_manager
                    .stop_thread(*tid, &message.user_id)
                    .await;
            }
            let _ = state
                .pending_gates
                .discard(&PendingGateKey {
                    user_id: message.user_id.clone(),
                    thread_id: *tid,
                })
                .await;
        }
    }

    // Clear the conversation entries and active thread list
    state
        .conversation_manager
        .clear_conversation(conv_id, &message.user_id)
        .await
        .map_err(|e| engine_err("clear conversation error", e))?;

    debug!(
        user_id = %message.user_id,
        conversation_id = %conv_id,
        "engine v2: conversation cleared"
    );

    Ok(())
}

pub async fn has_pending_auth(user_id: &str) -> bool {
    let Some(lock) = ENGINE_STATE.get() else {
        return false;
    };
    let Ok(guard) = lock.try_read() else {
        return false;
    };
    let Some(state) = guard.as_ref() else {
        return false;
    };
    state
        .pending_gates
        .list_for_user(user_id)
        .await
        .into_iter()
        .any(|gate| {
            matches!(
                gate.resume_kind,
                ironclaw_engine::ResumeKind::Authentication { .. }
            )
        })
}

/// Get pending auth info for a user (credential name + instructions).
///
/// Used by the history endpoint to include auth state in the response,
/// so SSE reconnects can re-show the auth card.
pub async fn get_engine_pending_auth(
    user_id: &str,
    thread_id: Option<&str>,
) -> Option<(Option<String>, String, Option<String>)> {
    let lock = ENGINE_STATE.get()?;
    let guard = lock.read().await;
    let state = guard.as_ref()?;
    match resolve_pending_gate_for_user(&state.pending_gates, user_id, thread_id).await {
        PendingGateResolution::Resolved(gate) => {
            if let ironclaw_engine::ResumeKind::Authentication {
                credential_name,
                instructions,
                ..
            } = gate.resume_kind
            {
                let instructions = if instructions.is_empty() {
                    state
                        .auth_manager
                        .as_ref()
                        .and_then(|mgr| mgr.get_setup_instructions(&credential_name))
                } else {
                    Some(instructions)
                };
                Some((
                    Some(gate.request_id.to_string()),
                    credential_name,
                    instructions,
                ))
            } else {
                None
            }
        }
        PendingGateResolution::None | PendingGateResolution::Ambiguous => None,
    }
}

/// Clear pending auth state for a user in the v2 engine.
///
/// Called from the gateway's `/api/chat/auth-token` and `/api/chat/auth-cancel`
/// endpoints to ensure pending authentication gates are cleared when the
/// frontend handles auth directly (not through the chat message path).
pub async fn clear_engine_pending_auth(user_id: &str, thread_id: Option<&str>) {
    let Some(lock) = ENGINE_STATE.get() else {
        return;
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return;
    };

    if let Some(thread_id) = thread_id {
        match resolve_pending_gate_for_user(&state.pending_gates, user_id, Some(thread_id)).await {
            PendingGateResolution::Resolved(gate)
                if matches!(
                    gate.resume_kind,
                    ironclaw_engine::ResumeKind::Authentication { .. }
                ) =>
            {
                let _ = state.pending_gates.discard(&gate.key()).await;
            }
            PendingGateResolution::Resolved(_)
            | PendingGateResolution::None
            | PendingGateResolution::Ambiguous => {}
        }
        return;
    }

    for gate in state.pending_gates.list_for_user(user_id).await {
        if matches!(
            gate.resume_kind,
            ironclaw_engine::ResumeKind::Authentication { .. }
        ) {
            let _ = state.pending_gates.discard(&gate.key()).await;
        }
    }
}

/// Handle a user message through the engine v2 pipeline.
pub async fn handle_with_engine(
    agent: &Agent,
    message: &IncomingMessage,
    content: &str,
) -> Result<Option<String>, Error> {
    handle_with_engine_inner(agent, message, content, 0).await
}

/// Maximum depth for auth-retry recursion (credential stored → retry original message).
const MAX_AUTH_RETRY_DEPTH: u8 = 2;

async fn handle_with_engine_inner(
    agent: &Agent,
    message: &IncomingMessage,
    content: &str,
    depth: u8,
) -> Result<Option<String>, Error> {
    if depth > MAX_AUTH_RETRY_DEPTH {
        return Ok(Some(
            "Credential stored, but too many auth retries. Please resend your message.".into(),
        ));
    }

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

    let thread_scope = message.conversation_scope();
    let scoped_thread_id = parse_engine_thread_id(thread_scope);

    match resolve_pending_gate_for_user(&state.pending_gates, &message.user_id, thread_scope).await
    {
        PendingGateResolution::Resolved(gate)
            if matches!(
                gate.resume_kind,
                ironclaw_engine::ResumeKind::Authentication { .. }
            ) =>
        {
            let request_id = gate.request_id;
            let resolution =
                if content.trim().is_empty() || content.trim().eq_ignore_ascii_case("cancel") {
                    ironclaw_engine::GateResolution::Cancelled
                } else {
                    ironclaw_engine::GateResolution::CredentialProvided {
                        token: content.trim().to_string(),
                    }
                };
            drop(guard);
            return resolve_gate(agent, message, gate.thread_id, request_id, resolution).await;
        }
        PendingGateResolution::Resolved(gate)
            if matches!(
                gate.resume_kind,
                ironclaw_engine::ResumeKind::Approval { .. }
            ) =>
        {
            let pending = gate.clone();
            return notify_pending_gate(agent, state, message, &pending).await;
        }
        PendingGateResolution::Ambiguous => {
            return Ok(Some(
                "Multiple pending approval or authentication prompts are waiting. Reply from the original thread.".into(),
            ));
        }
        PendingGateResolution::Resolved(_) | PendingGateResolution::None => {}
    }

    if let Some(thread_id) = scoped_thread_id
        && fail_orphaned_waiting_thread_if_needed(state, &message.user_id, thread_id).await?
    {
        return Ok(Some(
            "This thread was waiting on approval or authentication, but that pending state was lost. The thread has been marked failed; resend your request.".into(),
        ));
    }

    // Fire any active OnEvent missions whose pattern (and optional channel
    // filter) match this inbound message. Mission firings here are side
    // effects of the message — independent of, and parallel to, the normal
    // conversation thread spawned below. Errors are logged but never block
    // user-facing message handling.
    //
    // v1-created routines are NOT touched by this path: they live in the
    // v1 routine store and are fired by the v1 RoutineEngine in the
    // background. Missions created via the routine_create alias live in
    // the engine store and are fired here.
    fire_event_missions_for_message(state, message, content).await;

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

    // Resolve per-user project (creates if needed).
    let project_id =
        resolve_user_project(&state.store, &message.user_id, state.default_project_id).await?;

    // Handle the message — spawns a new thread or injects into active one
    let thread_id = state
        .conversation_manager
        .handle_user_message(
            conv_id,
            content,
            project_id,
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
                .ensure_conversation(
                    uuid,
                    &message.channel,
                    &message.user_id,
                    Some(tid),
                    Some(&message.channel),
                )
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

/// Fire active OnEvent missions whose pattern matches the inbound message.
///
/// Builds a payload containing the message metadata that mission threads
/// can read via `state["trigger_payload"]`. Skips empty content and
/// system-channel messages. Errors are logged at debug level — a failure
/// here must never block the user-facing message flow.
async fn fire_event_missions_for_message(
    state: &EngineState,
    message: &IncomingMessage,
    content: &str,
) {
    // Skip empty messages — there's nothing to pattern-match against
    // and we don't want missions firing on every status update or empty
    // user input.
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return;
    }

    // Recursion guards. Channel adapters that echo the agent's own
    // outbound text back as inbound events MUST set is_agent_broadcast
    // (Slack/Discord-style); messages produced as a side effect of a
    // mission firing MUST set triggering_mission_id (chain-recursion
    // across distinct missions). Either flag means: do not re-fire.
    if message.is_agent_broadcast {
        debug!(
            channel = %message.channel,
            "engine v2: skipping mission firing — message is an agent broadcast echo"
        );
        return;
    }
    if let Some(ref upstream) = message.triggering_mission_id {
        debug!(
            channel = %message.channel,
            upstream_mission_id = %upstream,
            "engine v2: skipping mission firing — message originated from a mission"
        );
        return;
    }

    let Some(mission_manager) = state.effect_adapter.mission_manager().await else {
        return;
    };

    let payload = serde_json::json!({
        "channel": message.channel,
        "user_id": message.user_id,
        "content": content,
        "metadata": message.metadata,
    });

    match mission_manager
        .fire_on_message_event(&message.channel, content, &message.user_id, Some(payload))
        .await
    {
        Ok(spawned) if !spawned.is_empty() => {
            debug!(
                count = spawned.len(),
                channel = %message.channel,
                user_id = %message.user_id,
                "engine v2: fired {} OnEvent mission(s) from inbound message",
                spawned.len()
            );
        }
        Ok(_) => {}
        Err(error) => {
            debug!(
                channel = %message.channel,
                error = %error,
                "engine v2: fire_on_message_event failed; continuing with normal handling"
            );
        }
    }
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
    // shows it correctly for all outcomes that produce a response.
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

            // Text-based auth fallback: detect authentication_required in the
            // response and enter auth mode. This is a defense-in-depth safety net
            // — the pre-flight auth gate should catch most cases before execution.
            if let Some(ref text) = response
                && text.contains("authentication_required")
            {
                debug!(
                    thread_id = %thread_id,
                    "text-based auth fallback triggered — pre-flight gate did not catch this"
                );

                // Extract credential name from the response text and validate
                // it against the expected pattern (alphanumeric + underscores).
                let parsed_cred_name = text
                    .split("credential_name")
                    .nth(1)
                    .and_then(|s| {
                        // Handle both JSON ("credential_name":"foo") and prose
                        s.split(&['"', '\'', '`'][..]) // safety: slice of char array, not string byte slicing
                            .find(|seg| !seg.is_empty() && !seg.contains(':') && !seg.contains(' '))
                    })
                    .filter(|name| {
                        // Reject names that don't look like valid credential identifiers
                        !name.is_empty()
                            && name.len() <= 64
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    })
                    .map(|s| s.to_string());

                // Defense against credential-name injection: only honor a
                // fallback auth gate when the parsed name is actually a
                // registered credential. A tool that fabricates an
                // `authentication_required` message with a chosen credential
                // name must not be able to coerce the user into providing an
                // unrelated secret. Test/embed harnesses without a credential
                // registry preserve existing behavior.
                let Some(cred_name) = parsed_cred_name.filter(|name| {
                    agent
                        .tools()
                        .credential_registry()
                        .is_none_or(|reg| reg.has_secret(name))
                }) else {
                    tracing::warn!(
                        thread_id = %thread_id,
                        "text-based auth fallback rejected unknown or missing credential name from tool output"
                    );
                    // Hand the original response back without inserting a gate.
                    return Ok(Some(text.clone()));
                };

                // Look up setup instructions via AuthManager (or fall back to inline lookup)
                let setup_hint = state
                    .auth_manager
                    .as_ref()
                    .and_then(|mgr| mgr.get_setup_instructions(&cred_name))
                    .unwrap_or_else(|| format!("Provide your {} token", cred_name));

                let pending = PendingGate {
                    request_id: uuid::Uuid::new_v4(),
                    gate_name: "authentication".into(),
                    user_id: message.user_id.clone(),
                    thread_id,
                    scope_thread_id: message.conversation_scope().map(str::to_string),
                    conversation_id: conv_id,
                    source_channel: message.channel.clone(),
                    action_name: "authentication_fallback".into(),
                    call_id: format!("fallback-auth-{thread_id}"),
                    parameters: serde_json::json!({ "credential_name": cred_name }),
                    display_parameters: None,
                    description: format!("Authentication required for '{}'.", cred_name),
                    resume_kind: ironclaw_engine::ResumeKind::Authentication {
                        credential_name: cred_name.clone(),
                        instructions: setup_hint.clone(),
                        auth_url: None,
                    },
                    created_at: chrono::Utc::now(),
                    expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
                    original_message: Some(message.content.clone()),
                    resume_output: None,
                    approval_already_granted: false,
                };
                if let Err(e) = state.pending_gates.insert(pending).await {
                    tracing::debug!(error = %e, "failed to store fallback auth gate");
                }

                // Show auth prompt via channel
                let _ = agent
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthRequired {
                            extension_name: cred_name.clone(),
                            instructions: Some(setup_hint.clone()),
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
        ThreadOutcome::GatePaused {
            gate_name,
            action_name,
            call_id,
            parameters,
            resume_kind,
            resume_output,
        } => {
            use crate::gate::pending::PendingGate;

            // Redact sensitive params before storing/broadcasting
            let redacted_params =
                if let Some(tool) = state.effect_adapter.tools().get(&action_name).await {
                    crate::tools::redact_params(&parameters, tool.sensitive_params())
                } else {
                    parameters.clone()
                };

            // Store in unified PendingGateStore (keyed by user_id + thread_id)
            let pending = PendingGate {
                request_id: uuid::Uuid::new_v4(),
                gate_name: gate_name.clone(),
                user_id: message.user_id.clone(),
                thread_id,
                scope_thread_id: message.conversation_scope().map(str::to_string),
                conversation_id: conv_id,
                source_channel: message.channel.clone(),
                action_name: action_name.clone(),
                call_id,
                parameters,
                display_parameters: Some(redacted_params.clone()),
                description: format!(
                    "Tool '{}' requires {} (gate: {gate_name})",
                    action_name,
                    resume_kind.kind_name()
                ),
                resume_kind: resume_kind.clone(),
                created_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
                original_message: Some(message.content.clone()),
                resume_output,
                approval_already_granted: false,
            };

            if let Err(e) = state.pending_gates.insert(pending.clone()).await {
                tracing::debug!(
                    gate = %gate_name,
                    error = %e,
                    "failed to store pending gate (may be duplicate)"
                );
            }

            // Send appropriate StatusUpdate via channel
            match &resume_kind {
                ironclaw_engine::ResumeKind::Approval { allow_always } => {
                    let _ = agent
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ApprovalNeeded {
                                request_id: pending.request_id.to_string(),
                                tool_name: action_name.clone(),
                                description: pending.description.clone(),
                                parameters: redacted_params,
                                allow_always: *allow_always,
                            },
                            &message.metadata,
                        )
                        .await;

                    Ok(Some(format!(
                        "Tool '{}' requires approval. Reply 'yes' to approve, 'no' to deny.",
                        action_name
                    )))
                }
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name,
                    instructions,
                    auth_url,
                } => {
                    let _ = agent
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::AuthRequired {
                                extension_name: credential_name.clone(),
                                instructions: Some(instructions.clone()),
                                auth_url: auth_url.clone(),
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
                ironclaw_engine::ResumeKind::External { callback_id } => {
                    tracing::debug!(
                        gate = %gate_name,
                        callback = %callback_id,
                        "GatePaused(External)"
                    );
                    Ok(Some(format!(
                        "Waiting for external confirmation (gate: {gate_name})..."
                    )))
                }
            }
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

// ── Shared event display helpers ────────────────────────────

/// Format an action name with optional parameter summary for display.
/// e.g., `"http(https://api.github.com/...)"` or just `"web_search"`.
fn format_action_display_name(action_name: &str, params_summary: &Option<String>) -> String {
    match params_summary {
        Some(summary) => format!("{}({})", action_name, summary),
        None => action_name.to_string(),
    }
}

/// Interpret a MessageAdded event into a human-readable status message.
/// Returns `None` for events that don't need UI surfacing.
fn interpret_message_event(role: &str, content_preview: &str) -> Option<&'static str> {
    if role == "User" && content_preview.starts_with("[stdout]") {
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
    }
}

/// Deliver a mission thread outcome to the mission's notify_channels.
///
/// Three sinks need the mission output so the assistant conversation stays
/// coherent across follow-ups:
///
/// 1. **Channel broadcast** — pushes the message to the live channel (REPL,
///    web SSE, etc.) so the user sees it in real time.
/// 2. **SSE app events** — pushes a `Response` event for the web gateway.
/// 3. **v1 conversation_messages table** — the gateway history API reads from
///    here, so a missing write would leave the message out of `/api/chat/history`.
/// 4. **v2 ConversationManager entries** — the engine v2 follow-up code path
///    builds new-thread context from `ConversationSurface.entries` (see
///    `build_history_from_entries`). Without an entry here, when the user
///    replies after a mission notification the new thread spawns with no
///    knowledge of the mission's output and the agent will (correctly, given
///    its empty context) say "I haven't sent you a digest". Recording an
///    `Agent` entry tagged with the mission's thread id keeps the v2 history
///    consistent with what the user actually saw.
async fn handle_mission_notification(
    notif: &ironclaw_engine::MissionNotification,
    channels: &std::sync::Arc<crate::channels::ChannelManager>,
    sse: Option<&Arc<SseManager>>,
    db: Option<&Arc<dyn Database>>,
    conv_mgr: Option<&ironclaw_engine::ConversationManager>,
) {
    let Some(ref text) = notif.response else {
        return;
    };

    let full_text = format!("**[{}]** {text}", notif.mission_name);

    // `notify_user` takes precedence over the mission owner's user_id when
    // set — it lets a routine/mission deliver to a specific recipient
    // (channel target) different from the mission's owning user.
    let broadcast_user = notif.notify_user.as_deref().unwrap_or(&notif.user_id);

    for channel_name in &notif.notify_channels {
        // Send via channel broadcast (proactive, no incoming message required)
        if let Err(e) = channels
            .broadcast(
                channel_name,
                broadcast_user,
                OutgoingResponse::text(&full_text),
            )
            .await
        {
            debug!(
                channel = %channel_name,
                mission = %notif.mission_name,
                "failed to broadcast mission result: {e}"
            );
        }
    }

    // Also write to SSE for the web gateway
    if let Some(sse) = sse {
        sse.broadcast_for_user(
            &notif.user_id,
            AppEvent::Response {
                content: full_text.clone(),
                thread_id: notif.thread_id.to_string(),
            },
        );
    }

    // Write to v1 DB so the history API shows the mission result.
    // Use the "assistant" conversation for the user on the first notify channel.
    if let Some(db) = db
        && let Some(channel_name) = notif.notify_channels.first()
        && let Ok(conv_id) = db
            .get_or_create_assistant_conversation(&notif.user_id, channel_name)
            .await
    {
        let _ = db
            .add_conversation_message(conv_id, "assistant", &full_text)
            .await;
    }

    // Inject the mission output into each notify channel's v2 conversation
    // so follow-up user messages spawn threads whose history includes the
    // mission output. Without this step, the engine v2 conversation history
    // (`build_history_from_entries`) is unaware of the mission and the user
    // can't ask follow-ups about its content.
    if let Some(conv_mgr) = conv_mgr {
        for channel_name in &notif.notify_channels {
            match conv_mgr
                .get_or_create_conversation(channel_name, &notif.user_id)
                .await
            {
                Ok(conv_id) => {
                    if let Err(e) = conv_mgr
                        .record_external_agent_message(
                            conv_id,
                            notif.thread_id,
                            &notif.user_id,
                            full_text.clone(),
                        )
                        .await
                    {
                        debug!(
                            channel = %channel_name,
                            mission = %notif.mission_name,
                            "failed to record mission output in v2 conversation: {e}"
                        );
                    }
                }
                Err(e) => {
                    debug!(
                        channel = %channel_name,
                        mission = %notif.mission_name,
                        "failed to resolve v2 conversation for mission notification: {e}"
                    );
                }
            }
        }
    }
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
            let display_name = format_action_display_name(action_name, params_summary);
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
            let display_name = format_action_display_name(action_name, params_summary);
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
            if let Some(text) = interpret_message_event(role, content_preview) {
                let _ = channels
                    .send_status(channel_name, StatusUpdate::Thinking(text.into()), metadata)
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
            let display_name = format_action_display_name(action_name, params_summary);
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
            let display_name = format_action_display_name(action_name, params_summary);
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
        } => interpret_message_event(role, content_preview)
            .map(|text| AppEvent::Thinking {
                message: text.into(),
                thread_id: Some(thread_id.into()),
            })
            .into_iter()
            .collect(),
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
    /// Human-readable description of the cadence (e.g. "every Monday at 09:00",
    /// "webhook: /github", "manual"). Renders nicer than `cadence_type` alone.
    pub cadence_description: String,
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
    pub notify_channels: Vec<String>,
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

/// Human-readable description of a mission cadence for the UI.
///
/// For cron expressions, recognizes common patterns ("every hour",
/// "every Monday at 09:00", etc.) and falls back to `"cron: <expression>"`
/// for unrecognized patterns. Other cadence types include their pattern/path
/// so the user can see what triggers the mission.
fn cadence_description(cadence: &ironclaw_engine::types::mission::MissionCadence) -> String {
    use ironclaw_engine::types::mission::MissionCadence;
    match cadence {
        MissionCadence::Cron {
            expression,
            timezone,
        } => {
            let base = describe_cron(expression).unwrap_or_else(|| format!("cron: {expression}"));
            match timezone {
                Some(tz) => format!("{base} ({tz})"),
                None => base,
            }
        }
        MissionCadence::OnEvent { event_pattern, .. } => format!("on event: {event_pattern}"),
        MissionCadence::OnSystemEvent {
            source, event_type, ..
        } => {
            format!("on system event: {source}/{event_type}")
        }
        MissionCadence::Webhook { path, .. } => format!("webhook: {path}"),
        MissionCadence::Manual => "manual".to_string(),
    }
}

/// Translate a 5-field cron expression into an English description for common
/// patterns. Returns `None` if the expression doesn't match a known shape; the
/// caller should fall back to showing the raw expression.
fn describe_cron(expression: &str) -> Option<String> {
    let parts: Vec<&str> = expression.split_whitespace().collect();
    // Accept standard 5-field cron; ignore 6/7-field variants for now.
    if parts.len() != 5 {
        return None;
    }
    let (minute, hour, dom, month, dow) = (parts[0], parts[1], parts[2], parts[3], parts[4]);

    // Helpers
    let is_any = |s: &str| s == "*";
    let parse_num = |s: &str| s.parse::<u32>().ok();
    let day_name = |n: u32| match n % 7 {
        0 | 7 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => "",
    };

    // Every minute
    if is_any(minute) && is_any(hour) && is_any(dom) && is_any(month) && is_any(dow) {
        return Some("every minute".to_string());
    }
    // Every hour at minute M
    if is_any(hour)
        && is_any(dom)
        && is_any(month)
        && is_any(dow)
        && let Some(m) = parse_num(minute)
    {
        if m == 0 {
            return Some("every hour".to_string());
        }
        return Some(format!("every hour at :{m:02}"));
    }
    // Daily at H:M (no day-of-week, no day-of-month restriction)
    if is_any(dom)
        && is_any(month)
        && is_any(dow)
        && let (Some(m), Some(h)) = (parse_num(minute), parse_num(hour))
    {
        return Some(format!("every day at {h:02}:{m:02}"));
    }
    // Weekly on a single day at H:M
    if is_any(dom)
        && is_any(month)
        && let (Some(m), Some(h), Some(d)) = (parse_num(minute), parse_num(hour), parse_num(dow))
    {
        let name = day_name(d);
        if !name.is_empty() {
            return Some(format!("every {name} at {h:02}:{m:02}"));
        }
    }
    // Monthly on day-of-month at H:M
    if is_any(month)
        && is_any(dow)
        && let (Some(m), Some(h), Some(d)) = (parse_num(minute), parse_num(hour), parse_num(dom))
    {
        return Some(format!("monthly on day {d} at {h:02}:{m:02}"));
    }
    None
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
pub async fn list_engine_threads(
    project_id: Option<&str>,
    user_id: &str,
) -> Result<Vec<EngineThreadInfo>, Error> {
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
        .list_threads(pid, user_id)
        .await
        .map_err(|e| engine_err("list threads", e))?;

    Ok(threads.iter().map(thread_to_info).collect())
}

/// Get a single engine thread by ID.
pub async fn get_engine_thread(
    thread_id: &str,
    user_id: &str,
) -> Result<Option<EngineThreadDetail>, Error> {
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

    // Ownership check: only return thread if it belongs to the requesting user
    if !thread.is_owned_by(user_id) {
        return Ok(None);
    }

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
pub async fn list_engine_thread_steps(
    thread_id: &str,
    user_id: &str,
) -> Result<Vec<EngineStepInfo>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(Vec::new());
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(Vec::new());
    };

    let tid = uuid::Uuid::parse_str(thread_id).map_err(|e| engine_err("parse thread_id", e))?;

    // Validate thread ownership before returning steps.
    if let Some(thread) = state
        .store
        .load_thread(ironclaw_engine::ThreadId(tid))
        .await
        .map_err(|e| engine_err("load thread", e))?
    {
        if !thread.is_owned_by(user_id) {
            return Ok(Vec::new());
        }
    } else {
        return Ok(Vec::new());
    }

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
pub async fn list_engine_thread_events(
    thread_id: &str,
    user_id: &str,
) -> Result<Vec<serde_json::Value>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(Vec::new());
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(Vec::new());
    };

    let tid = uuid::Uuid::parse_str(thread_id).map_err(|e| engine_err("parse thread_id", e))?;

    // Validate thread ownership before returning events.
    if let Some(thread) = state
        .store
        .load_thread(ironclaw_engine::ThreadId(tid))
        .await
        .map_err(|e| engine_err("load thread", e))?
    {
        if !thread.is_owned_by(user_id) {
            return Ok(Vec::new());
        }
    } else {
        return Ok(Vec::new());
    }

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
pub async fn list_engine_projects(user_id: &str) -> Result<Vec<EngineProjectInfo>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(Vec::new());
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(Vec::new());
    };

    let projects = state
        .store
        .list_projects(user_id)
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
pub async fn get_engine_project(
    project_id: &str,
    user_id: &str,
) -> Result<Option<EngineProjectInfo>, Error> {
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

    Ok(project
        .filter(|p| p.is_owned_by(user_id))
        .map(|p| EngineProjectInfo {
            id: p.id.to_string(),
            name: p.name,
            description: p.description,
            created_at: p.created_at.to_rfc3339(),
        }))
}

/// List missions, optionally filtered by project.
pub async fn list_engine_missions(
    project_id: Option<&str>,
    user_id: &str,
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
        .list_missions_with_shared(pid, user_id)
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
            cadence_description: cadence_description(&m.cadence),
            thread_count: m.thread_history.len(),
            current_focus: m.current_focus.clone(),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        })
        .collect())
}

/// Get a single mission by ID.
pub async fn get_engine_mission(
    mission_id: &str,
    user_id: &str,
) -> Result<Option<EngineMissionDetail>, Error> {
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

    // Ownership check: allow access to user's own missions and shared missions.
    if m.user_id != user_id && !is_shared_owner(&m.user_id) {
        return Ok(None);
    }

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
            cadence_description: cadence_description(&m.cadence),
            thread_count: m.thread_history.len(),
            current_focus: m.current_focus.clone(),
            created_at: m.created_at.to_rfc3339(),
            updated_at: m.updated_at.to_rfc3339(),
        },
        cadence: cadence_json,
        approach_history: m.approach_history.clone(),
        notify_channels: m.notify_channels.clone(),
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
///
/// For shared missions, the caller must be an admin (pass `is_admin=true`).
/// For user missions, ownership is enforced by the engine.
pub async fn pause_engine_mission(
    mission_id: &str,
    user_id: &str,
    is_admin: bool,
) -> Result<(), Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };

    let mid = uuid::Uuid::parse_str(mission_id).map_err(|e| engine_err("parse mission_id", e))?;
    let mgr = state
        .effect_adapter
        .mission_manager()
        .await
        .ok_or_else(|| engine_err("mission", "mission manager not available"))?;

    // Shared missions require admin role; pass the shared owner id to satisfy engine check.
    let effective_user_id = resolve_mission_user_id(&state.store, mid, user_id, is_admin).await?;
    mgr.pause_mission(ironclaw_engine::MissionId(mid), &effective_user_id)
        .await
        .map_err(|e| engine_err("pause mission", e))
}

/// Resume a paused mission.
///
/// For shared missions, the caller must be an admin (pass `is_admin=true`).
/// For user missions, ownership is enforced by the engine.
pub async fn resume_engine_mission(
    mission_id: &str,
    user_id: &str,
    is_admin: bool,
) -> Result<(), Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Err(engine_err("not initialized", "engine v2 is not running"));
    };

    let mid = uuid::Uuid::parse_str(mission_id).map_err(|e| engine_err("parse mission_id", e))?;
    let mgr = state
        .effect_adapter
        .mission_manager()
        .await
        .ok_or_else(|| engine_err("mission", "mission manager not available"))?;

    let effective_user_id = resolve_mission_user_id(&state.store, mid, user_id, is_admin).await?;
    mgr.resume_mission(ironclaw_engine::MissionId(mid), &effective_user_id)
        .await
        .map_err(|e| engine_err("resume mission", e))
}

/// Reset the global engine state so a fresh engine can be initialized.
///
/// Used by the test rig to isolate engine v2 tests — each test gets a clean
/// engine state instead of inheriting the prior test's `OnceLock` singleton.
#[cfg(feature = "libsql")]
pub async fn reset_engine_state() {
    if let Some(lock) = ENGINE_STATE.get() {
        *lock.write().await = None;
    }
}

/// Resolve the effective user_id for mission management operations.
///
/// If the mission is shared-owned, requires admin role and returns the shared owner id
/// so the engine ownership check passes. Otherwise returns the caller's user_id.
async fn resolve_mission_user_id(
    store: &Arc<dyn ironclaw_engine::Store>,
    mid: uuid::Uuid,
    user_id: &str,
    is_admin: bool,
) -> Result<String, Error> {
    if let Ok(Some(mission)) = store.load_mission(ironclaw_engine::MissionId(mid)).await
        && is_shared_owner(&mission.user_id)
    {
        if !is_admin {
            return Err(engine_err(
                "forbidden",
                "shared missions can only be managed by admins",
            ));
        }
        return Ok(shared_owner_id().to_string());
    }
    Ok(user_id.to_string())
}

// ── Legacy migration ────────────────────────────────────────────

/// One-time migration: stamp the owner's user_id onto any engine records that
/// deserialized with the serde default `"legacy"` (pre-multi-tenancy data).
///
/// Runs at engine init before user-scoped queries. After migration, records
/// are findable by the owner's identity and the "legacy" sentinel disappears.
async fn migrate_legacy_user_ids(store: &Arc<dyn ironclaw_engine::Store>, owner_id: &str) {
    // Projects
    if let Ok(legacy) = store.list_projects("legacy").await {
        for mut project in legacy {
            project.user_id = owner_id.to_string();
            project.updated_at = chrono::Utc::now();
            let _ = store.save_project(&project).await;
        }
    }

    // We need a project_id to query threads/missions/docs. Use list_projects
    // with the now-migrated owner_id, or fall back to "legacy" in case save failed.
    let all_projects: Vec<ironclaw_engine::Project> =
        store.list_projects(owner_id).await.unwrap_or_default();

    for project in &all_projects {
        let pid = project.id;

        // Threads
        if let Ok(legacy) = store.list_all_threads(pid).await {
            for mut thread in legacy.into_iter().filter(|t| t.user_id == "legacy") {
                thread.user_id = owner_id.to_string();
                thread.updated_at = chrono::Utc::now();
                let _ = store.save_thread(&thread).await;
            }
        }

        // Missions
        if let Ok(legacy) = store.list_all_missions(pid).await {
            for mut mission in legacy.into_iter().filter(|m| m.user_id == "legacy") {
                // System learning missions keep "system"; only stamp truly orphaned ones.
                mission.user_id = owner_id.to_string();
                mission.updated_at = chrono::Utc::now();
                let _ = store.save_mission(&mission).await;
            }
        }

        // Memory docs (use list_memory_docs directly since "legacy" is the user_id)
        if let Ok(legacy) = store.list_memory_docs(pid, "legacy").await {
            for mut doc in legacy {
                doc.user_id = owner_id.to_string();
                doc.updated_at = chrono::Utc::now();
                let _ = store.save_memory_doc(&doc).await;
            }
        }
    }

    debug!("engine v2: legacy user_id migration complete for owner {owner_id}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::{
        Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    };
    use crate::error::ChannelError;
    use futures::stream;
    use rust_decimal::Decimal;
    use std::sync::LazyLock;
    use std::time::Duration;
    use tokio::sync::Mutex as TokioMutex;
    use tokio::sync::RwLock as TokioRwLock;

    static ENGINE_STATE_TEST_LOCK: LazyLock<TokioMutex<()>> = LazyLock::new(|| TokioMutex::new(()));

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

    #[derive(Clone)]
    struct RecordingStatusChannel {
        name: String,
        statuses: Arc<TokioMutex<Vec<StatusUpdate>>>,
    }

    #[async_trait::async_trait]
    impl Channel for RecordingStatusChannel {
        fn name(&self) -> &str {
            &self.name
        }

        async fn start(&self) -> Result<MessageStream, ChannelError> {
            Ok(Box::pin(stream::empty()))
        }

        async fn respond(
            &self,
            _msg: &IncomingMessage,
            _response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            Ok(())
        }

        async fn send_status(
            &self,
            status: StatusUpdate,
            _metadata: &serde_json::Value,
        ) -> Result<(), ChannelError> {
            self.statuses.lock().await.push(status);
            Ok(())
        }

        async fn health_check(&self) -> Result<(), ChannelError> {
            Ok(())
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
            _user_id: &str,
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
            _user_id: &str,
        ) -> Result<Vec<ironclaw_engine::Project>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
        async fn list_all_projects(
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
            _user_id: &str,
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
            _user_id: &str,
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

    fn sample_pending_gate(
        user_id: &str,
        thread_id: ironclaw_engine::ThreadId,
        resume_kind: ironclaw_engine::ResumeKind,
    ) -> PendingGate {
        PendingGate {
            request_id: uuid::Uuid::new_v4(),
            gate_name: resume_kind.kind_name().to_string(),
            user_id: user_id.into(),
            thread_id,
            scope_thread_id: None,
            conversation_id: ironclaw_engine::ConversationId::new(),
            source_channel: "web".into(),
            action_name: "shell".into(),
            call_id: format!("call-{thread_id}"),
            parameters: serde_json::json!({"cmd": "ls"}),
            display_parameters: None,
            description: "pending gate".into(),
            resume_kind,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
            original_message: None,
            resume_output: None,
            approval_already_granted: false,
        }
    }

    async fn make_router_test_agent(
        sse: Option<Arc<SseManager>>,
    ) -> (Agent, Arc<TokioMutex<Vec<StatusUpdate>>>) {
        struct StaticLlmProvider;

        #[async_trait::async_trait]
        impl crate::llm::LlmProvider for StaticLlmProvider {
            fn model_name(&self) -> &str {
                "static-mock"
            }

            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }

            async fn complete(
                &self,
                _request: crate::llm::CompletionRequest,
            ) -> Result<crate::llm::CompletionResponse, crate::error::LlmError> {
                Ok(crate::llm::CompletionResponse {
                    content: "ok".to_string(),
                    input_tokens: 0,
                    output_tokens: 0,
                    finish_reason: crate::llm::FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }

            async fn complete_with_tools(
                &self,
                _request: crate::llm::ToolCompletionRequest,
            ) -> Result<crate::llm::ToolCompletionResponse, crate::error::LlmError> {
                Ok(crate::llm::ToolCompletionResponse {
                    content: Some("ok".to_string()),
                    tool_calls: Vec::new(),
                    input_tokens: 0,
                    output_tokens: 0,
                    finish_reason: crate::llm::FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
        }

        let deps = crate::agent::AgentDeps {
            owner_id: "default".to_string(),
            store: None,
            llm: Arc::new(StaticLlmProvider),
            cheap_llm: None,
            safety: Arc::new(ironclaw_safety::SafetyLayer::new(
                &ironclaw_safety::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: true,
                },
            )),
            tools: Arc::new(crate::tools::ToolRegistry::new()),
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: crate::config::SkillsConfig::default(),
            hooks: Arc::new(crate::hooks::HookRegistry::new()),
            auth_manager: None,
            cost_guard: Arc::new(crate::agent::cost_guard::CostGuard::new(
                crate::agent::cost_guard::CostGuardConfig::default(),
            )),
            sse_tx: sse,
            http_interceptor: None,
            transcription: None,
            document_extraction: None,
            sandbox_readiness: crate::agent::routine_engine::SandboxReadiness::DisabledByConfig,
            builder: None,
            llm_backend: "nearai".to_string(),
            tenant_rates: Arc::new(crate::tenant::TenantRateRegistry::new(4, 3)),
        };

        let channels = Arc::new(crate::channels::ChannelManager::new());
        let statuses = Arc::new(TokioMutex::new(Vec::new()));
        channels
            .add(Box::new(RecordingStatusChannel {
                name: "web".to_string(),
                statuses: Arc::clone(&statuses),
            }))
            .await;

        let agent = Agent::new(
            crate::config::AgentConfig {
                name: "router-test-agent".to_string(),
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
                engine_v2: true,
            },
            deps,
            channels,
            None,
            None,
            None,
            Some(Arc::new(crate::context::ContextManager::new(1))),
            None,
        );

        (agent, statuses)
    }

    #[tokio::test]
    async fn insert_and_notify_pending_gate_emits_gate_required_not_legacy_auth_sse() {
        let store = Arc::new(TestStore::new());
        let sse = Arc::new(SseManager::new());
        let mut receiver = sse.sender().subscribe();
        let (agent, _statuses) = make_router_test_agent(Some(Arc::clone(&sse))).await;
        let mut state = make_expected_test_state(store);
        state.sse = Some(Arc::clone(&sse));

        let thread_id = ironclaw_engine::ThreadId::new();
        let pending = sample_pending_gate(
            "alice",
            thread_id,
            ironclaw_engine::ResumeKind::Authentication {
                credential_name: "google_oauth_token".to_string(),
                instructions: "Sign in with Google".to_string(),
                auth_url: Some("https://example.test/oauth".to_string()),
            },
        );
        let mut message = crate::channels::IncomingMessage::new("web", "alice", "use google");
        message.thread_id = Some(thread_id.to_string());

        let prompt = insert_and_notify_pending_gate(&agent, &state, &message, pending)
            .await
            .expect("pending gate inserted");
        assert!(
            prompt
                .as_deref()
                .unwrap_or_default()
                .contains("Authentication required for 'google_oauth_token'")
        );

        let scoped = receiver.recv().await.expect("sse event");
        assert_eq!(scoped.user_id.as_deref(), Some("alice"));
        match scoped.event {
            AppEvent::GateRequired {
                gate_name,
                tool_name,
                thread_id: Some(event_thread_id),
                ..
            } => {
                assert_eq!(gate_name, "authentication");
                assert_eq!(tool_name, "shell");
                assert_eq!(event_thread_id, thread_id.to_string());
            }
            other => panic!("expected GateRequired event, got {other:?}"),
        }

        assert!(
            receiver.try_recv().is_err(),
            "unexpected legacy auth SSE event"
        );
    }

    #[tokio::test]
    async fn handle_with_engine_re_emits_pending_approval_on_follow_up() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let store = Arc::new(TestStore::new());
        let sse = Arc::new(SseManager::new());
        let mut receiver = sse.sender().subscribe();
        let (agent, statuses) = make_router_test_agent(Some(Arc::clone(&sse))).await;
        let mut state = make_expected_test_state(store);
        state.sse = Some(Arc::clone(&sse));

        let thread_id = ironclaw_engine::ThreadId::new();
        let pending = sample_pending_gate(
            "alice",
            thread_id,
            ironclaw_engine::ResumeKind::Approval { allow_always: true },
        );
        let request_id = pending.request_id.to_string();
        state.pending_gates.insert(pending).await.unwrap();

        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;
        *lock.write().await = Some(state);

        let mut message =
            crate::channels::IncomingMessage::new("web", "alice", "what's happening?");
        message.thread_id = Some(thread_id.to_string());

        let response = handle_with_engine(&agent, &message, &message.content)
            .await
            .expect("follow-up handled");

        assert_eq!(
            response.as_deref(),
            Some("Tool 'shell' requires approval. Reply 'yes' to approve, 'no' to deny.")
        );

        let scoped = receiver.recv().await.expect("sse event");
        match scoped.event {
            AppEvent::GateRequired {
                request_id: event_request_id,
                gate_name,
                tool_name,
                thread_id: Some(event_thread_id),
                ..
            } => {
                assert_eq!(event_request_id, request_id);
                assert_eq!(gate_name, "approval");
                assert_eq!(tool_name, "shell");
                assert_eq!(event_thread_id, thread_id.to_string());
            }
            other => panic!("expected GateRequired event, got {other:?}"),
        }

        let statuses = statuses.lock().await.clone();
        assert!(statuses.iter().any(|status| matches!(
            status,
            StatusUpdate::ApprovalNeeded {
                request_id: status_request_id,
                tool_name,
                ..
            } if status_request_id == &request_id && tool_name == "shell"
        )));

        *lock.write().await = None;
    }

    #[tokio::test]
    async fn resolve_pending_gate_is_thread_scoped() {
        let store = crate::gate::store::PendingGateStore::in_memory();
        let thread_a = ironclaw_engine::ThreadId::new();
        let thread_b = ironclaw_engine::ThreadId::new();
        store
            .insert(sample_pending_gate(
                "alice",
                thread_a,
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            ))
            .await
            .unwrap();
        store
            .insert(sample_pending_gate(
                "alice",
                thread_b,
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            ))
            .await
            .unwrap();

        let resolved =
            resolve_pending_gate_for_user(&store, "alice", Some(&thread_b.to_string())).await;

        let PendingGateResolution::Resolved(gate) = resolved else {
            panic!("expected a thread-scoped gate");
        };
        assert_eq!(gate.thread_id, thread_b);
    }

    #[tokio::test]
    async fn resolve_pending_gate_detects_ambiguity_without_thread_hint() {
        let store = crate::gate::store::PendingGateStore::in_memory();
        store
            .insert(sample_pending_gate(
                "alice",
                ironclaw_engine::ThreadId::new(),
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            ))
            .await
            .unwrap();
        store
            .insert(sample_pending_gate(
                "alice",
                ironclaw_engine::ThreadId::new(),
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            ))
            .await
            .unwrap();

        let resolved = resolve_pending_gate_for_user(&store, "alice", None).await;
        assert!(matches!(resolved, PendingGateResolution::Ambiguous));
    }

    #[tokio::test]
    async fn resolve_pending_gate_filters_by_kind() {
        let store = crate::gate::store::PendingGateStore::in_memory();
        let thread_id = ironclaw_engine::ThreadId::new();
        store
            .insert(sample_pending_gate(
                "alice",
                thread_id,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name: "github".into(),
                    instructions: "paste token".into(),
                    auth_url: None,
                },
            ))
            .await
            .unwrap();

        let resolved =
            resolve_pending_gate_for_user(&store, "alice", Some(&thread_id.to_string())).await;

        let PendingGateResolution::Resolved(gate) = resolved else {
            panic!("expected an auth gate");
        };
        assert!(matches!(
            gate.resume_kind,
            ironclaw_engine::ResumeKind::Authentication { .. }
        ));
    }

    #[test]
    fn resolved_call_id_prefers_stored_id_for_parallel_same_name_calls() {
        let mut thread = ironclaw_engine::Thread::new(
            "goal",
            ironclaw_engine::ThreadType::Foreground,
            ironclaw_engine::ProjectId::new(),
            "alice",
            ironclaw_engine::ThreadConfig::default(),
        );
        thread.add_message(ironclaw_engine::ThreadMessage::assistant_with_actions(
            Some("parallel shell calls".to_string()),
            vec![
                ironclaw_engine::ActionCall {
                    id: "call-1".to_string(),
                    action_name: "shell".to_string(),
                    parameters: serde_json::json!({"cmd": "pwd"}),
                },
                ironclaw_engine::ActionCall {
                    id: "call-2".to_string(),
                    action_name: "shell".to_string(),
                    parameters: serde_json::json!({"cmd": "ls"}),
                },
            ],
        ));

        let pending = PendingGate {
            call_id: "call-1".to_string(),
            parameters: serde_json::json!({"cmd": "pwd"}),
            ..sample_pending_gate(
                "alice",
                thread.id,
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            )
        };

        assert_eq!(
            resolved_call_id_for_pending_action(&thread, &pending),
            "call-1"
        );
    }

    #[test]
    fn resolved_call_id_legacy_fallback_uses_last_unresolved_parallel_call() {
        let mut thread = ironclaw_engine::Thread::new(
            "goal",
            ironclaw_engine::ThreadType::Foreground,
            ironclaw_engine::ProjectId::new(),
            "alice",
            ironclaw_engine::ThreadConfig::default(),
        );
        thread.add_message(ironclaw_engine::ThreadMessage::assistant_with_actions(
            Some("parallel shell calls".to_string()),
            vec![
                ironclaw_engine::ActionCall {
                    id: "call-1".to_string(),
                    action_name: "shell".to_string(),
                    parameters: serde_json::json!({"cmd": "pwd"}),
                },
                ironclaw_engine::ActionCall {
                    id: "call-2".to_string(),
                    action_name: "shell".to_string(),
                    parameters: serde_json::json!({"cmd": "ls"}),
                },
            ],
        ));
        thread.add_message(ironclaw_engine::ThreadMessage::action_result(
            "call-1",
            "shell",
            "{\"ok\":true}",
        ));

        let pending = PendingGate {
            call_id: String::new(),
            ..sample_pending_gate(
                "alice",
                thread.id,
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            )
        };

        assert_eq!(
            resolved_call_id_for_pending_action(&thread, &pending),
            "call-2"
        );
    }

    #[tokio::test]
    async fn clear_engine_pending_auth_scopes_to_thread_when_hint_provided() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let store = Arc::new(TestStore::new());
        let state = make_expected_test_state(store);
        let thread_a = ironclaw_engine::ThreadId::new();
        let thread_b = ironclaw_engine::ThreadId::new();

        state
            .pending_gates
            .insert(sample_pending_gate(
                "alice",
                thread_a,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name: "github_token".into(),
                    instructions: "paste token".into(),
                    auth_url: None,
                },
            ))
            .await
            .unwrap();
        state
            .pending_gates
            .insert(sample_pending_gate(
                "alice",
                thread_b,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name: "linear_token".into(),
                    instructions: "paste token".into(),
                    auth_url: None,
                },
            ))
            .await
            .unwrap();

        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;
        *lock.write().await = Some(state);

        clear_engine_pending_auth("alice", Some(&thread_a.to_string())).await;

        let guard = lock.read().await;
        let state = guard.as_ref().unwrap();
        let remaining = state.pending_gates.list_for_user("alice").await;
        assert_eq!(remaining.len(), 1);
        assert!(remaining.iter().any(|gate| gate.thread_id == thread_b));
        drop(guard);
        *lock.write().await = None;
    }

    #[tokio::test]
    async fn clear_engine_pending_auth_without_hint_clears_all_auth_gates() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let store = Arc::new(TestStore::new());
        let state = make_expected_test_state(store);
        let thread_a = ironclaw_engine::ThreadId::new();
        let thread_b = ironclaw_engine::ThreadId::new();

        state
            .pending_gates
            .insert(sample_pending_gate(
                "alice",
                thread_a,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name: "github_token".into(),
                    instructions: "paste token".into(),
                    auth_url: None,
                },
            ))
            .await
            .unwrap();
        state
            .pending_gates
            .insert(sample_pending_gate(
                "alice",
                thread_b,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name: "linear_token".into(),
                    instructions: "paste token".into(),
                    auth_url: None,
                },
            ))
            .await
            .unwrap();

        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;
        *lock.write().await = Some(state);

        clear_engine_pending_auth("alice", None).await;

        let guard = lock.read().await;
        let state = guard.as_ref().unwrap();
        assert!(state.pending_gates.list_for_user("alice").await.is_empty());
        drop(guard);
        *lock.write().await = None;
    }

    // ── /expected command tests ─────────────────────────────────

    /// Build a minimal EngineState backed by a TestStore for /expected tests.
    fn make_expected_test_state(store: Arc<TestStore>) -> EngineState {
        use ironclaw_engine::{
            CapabilityRegistry, ConversationManager, LeaseManager, PolicyEngine, ThreadManager,
        };

        // Minimal mocks — /expected doesn't execute threads, just reads state
        struct NoopLlm;
        #[async_trait::async_trait]
        impl ironclaw_engine::LlmBackend for NoopLlm {
            async fn complete(
                &self,
                _: &[ironclaw_engine::ThreadMessage],
                _: &[ironclaw_engine::ActionDef],
                _: &ironclaw_engine::LlmCallConfig,
            ) -> Result<ironclaw_engine::LlmOutput, ironclaw_engine::EngineError> {
                Ok(ironclaw_engine::LlmOutput {
                    response: ironclaw_engine::LlmResponse::Text("done".into()),
                    usage: ironclaw_engine::TokenUsage::default(),
                })
            }
            fn model_name(&self) -> &str {
                "noop"
            }
        }

        struct NoopEffects;
        #[async_trait::async_trait]
        impl ironclaw_engine::EffectExecutor for NoopEffects {
            async fn execute_action(
                &self,
                _: &str,
                _: serde_json::Value,
                _: &ironclaw_engine::CapabilityLease,
                _: &ironclaw_engine::ThreadExecutionContext,
            ) -> Result<ironclaw_engine::ActionResult, ironclaw_engine::EngineError> {
                unreachable!()
            }
            async fn available_actions(
                &self,
                _: &[ironclaw_engine::CapabilityLease],
            ) -> Result<Vec<ironclaw_engine::ActionDef>, ironclaw_engine::EngineError> {
                Ok(vec![])
            }
        }

        let store_dyn: Arc<dyn Store> = store;
        let effect_adapter = Arc::new(EffectBridgeAdapter::new(
            Arc::new(crate::tools::ToolRegistry::new()),
            Arc::new(ironclaw_safety::SafetyLayer::new(
                &ironclaw_safety::SafetyConfig {
                    max_output_length: 10_000,
                    injection_check_enabled: false,
                },
            )),
            Arc::new(crate::hooks::HookRegistry::default()),
        ));

        let tm = Arc::new(ThreadManager::new(
            Arc::new(NoopLlm),
            Arc::new(NoopEffects),
            store_dyn.clone(),
            Arc::new(CapabilityRegistry::new()),
            Arc::new(LeaseManager::new()),
            Arc::new(PolicyEngine::new()),
        ));

        let cm = Arc::new(ConversationManager::new(Arc::clone(&tm), store_dyn.clone()));

        EngineState {
            thread_manager: tm,
            conversation_manager: cm,
            effect_adapter,
            store: store_dyn,
            default_project_id: ironclaw_engine::ProjectId::new(),
            pending_gates: Arc::new(crate::gate::store::PendingGateStore::in_memory()),
            sse: None,
            db: None,
            secrets_store: None,
            auth_manager: None,
        }
    }

    /// find_most_recent_thread returns the active thread when one exists.
    #[tokio::test]
    async fn find_recent_thread_returns_active() {
        let store = Arc::new(TestStore::new());
        let state = make_expected_test_state(store.clone());

        let project_id = state.default_project_id;
        let mut thread = ironclaw_engine::Thread::new(
            "test goal",
            ironclaw_engine::ThreadType::Foreground,
            project_id,
            "alice",
            ironclaw_engine::ThreadConfig::default(),
        );
        thread.add_message(ironclaw_engine::ThreadMessage::user("hello"));
        thread.add_message(ironclaw_engine::ThreadMessage::assistant("hi there"));
        let tid = thread.id;
        store.save_thread(&thread).await.unwrap();

        let mut conv = ironclaw_engine::ConversationSurface::new("web", "alice");
        conv.track_thread(tid);
        let conv_opt = Some(conv);

        let result = find_most_recent_thread(&state, &conv_opt, "alice").await;
        assert!(result.is_some(), "should find thread");
        assert_eq!(result.unwrap().id, tid);
    }

    /// find_most_recent_thread returns None for empty conversation.
    #[tokio::test]
    async fn find_recent_thread_empty_conv_returns_none() {
        let store = Arc::new(TestStore::new());
        let state = make_expected_test_state(store);

        let conv = Some(ironclaw_engine::ConversationSurface::new("web", "alice"));
        let result = find_most_recent_thread(&state, &conv, "alice").await;
        assert!(result.is_none());
    }

    /// find_most_recent_thread filters by user_id (tenant isolation).
    #[tokio::test]
    async fn find_recent_thread_filters_by_user() {
        let store = Arc::new(TestStore::new());
        let state = make_expected_test_state(store.clone());

        let project_id = state.default_project_id;
        let thread = ironclaw_engine::Thread::new(
            "bob's thread",
            ironclaw_engine::ThreadType::Foreground,
            project_id,
            "bob", // owned by bob
            ironclaw_engine::ThreadConfig::default(),
        );
        let tid = thread.id;
        store.save_thread(&thread).await.unwrap();

        let mut conv = ironclaw_engine::ConversationSurface::new("web", "alice");
        conv.track_thread(tid);

        // Alice should NOT see Bob's thread
        let result = find_most_recent_thread(&state, &Some(conv), "alice").await;
        assert!(result.is_none(), "alice should not see bob's thread"); // safety: test-only
    }

    /// find_most_recent_thread falls back to entry-referenced threads
    /// when no active threads exist.
    #[tokio::test]
    async fn find_recent_thread_falls_back_to_entries() {
        let store = Arc::new(TestStore::new());
        let state = make_expected_test_state(store.clone());

        let project_id = state.default_project_id;
        let mut thread = ironclaw_engine::Thread::new(
            "completed goal",
            ironclaw_engine::ThreadType::Foreground,
            project_id,
            "alice",
            ironclaw_engine::ThreadConfig::default(),
        );
        thread.add_message(ironclaw_engine::ThreadMessage::user("do something"));
        thread.add_message(ironclaw_engine::ThreadMessage::assistant("done"));
        let tid = thread.id;
        store.save_thread(&thread).await.unwrap();

        // Conversation with no active threads, but an entry referencing the thread
        let mut conv = ironclaw_engine::ConversationSurface::new("web", "alice");
        conv.add_entry(ironclaw_engine::ConversationEntry::agent(tid, "done"));
        // Thread is NOT in active_threads (it completed and was untracked)

        let result = find_most_recent_thread(&state, &Some(conv), "alice").await;
        assert!(result.is_some(), "should find thread via entry fallback");
        assert_eq!(result.unwrap().id, tid);
    }
}
