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

/// Persistent engine state that lives across messages.
struct EngineState {
    thread_manager: Arc<ThreadManager>,
    conversation_manager: ConversationManager,
    effect_adapter: Arc<EffectBridgeAdapter>,
    store: Arc<dyn Store>,
    default_project_id: ironclaw_engine::ProjectId,
    /// Per-user pending approvals (keyed by user_id).
    pending_approvals: RwLock<HashMap<String, PendingApproval>>,
    /// SSE manager for broadcasting AppEvents to the web gateway.
    sse: Option<Arc<SseManager>>,
    /// V1 database for writing conversation messages (gateway reads from here).
    db: Option<Arc<dyn Database>>,
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
async fn get_or_init_engine(agent: &Agent) -> Result<(), Error> {
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

    let store = Arc::new(HybridStore::new(agent.workspace().cloned()));
    store.load_state_from_workspace().await;

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
        .map_err(|e| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 store error: {e}"),
            })
        })?
        .into_iter()
        .find(|project| project.name == "default")
    {
        Some(project) => project.id,
        None => {
            let project = Project::new("default", "Default project for engine v2");
            let project_id = project.id;
            store.save_project(&project).await.map_err(|e| {
                crate::error::Error::from(crate::error::JobError::ContextError {
                    id: uuid::Uuid::nil(),
                    reason: format!("engine v2 store error: {e}"),
                })
            })?;
            project_id
        }
    };

    let conversation_manager = ConversationManager::new(Arc::clone(&thread_manager), store.clone());
    let _ = conversation_manager
        .bootstrap_user(&agent.deps.owner_id)
        .await;

    // Create mission manager and start cron ticker
    let mission_manager = Arc::new(MissionManager::new(store_dyn, Arc::clone(&thread_manager)));
    let _ = thread_manager.recover_project_threads(project_id).await;
    let _ = mission_manager.bootstrap_project(project_id).await;
    let _ = mission_manager
        .resume_recoverable_threads(&agent.deps.owner_id)
        .await;
    let _ = thread_manager.resume_background_threads(project_id).await;
    mission_manager.start_cron_ticker(agent.deps.owner_id.clone());
    mission_manager.start_event_listener(agent.deps.owner_id.clone());

    // Ensure self-improvement mission exists for this project
    if let Err(e) = mission_manager
        .ensure_self_improvement_mission(project_id)
        .await
    {
        debug!("engine v2: failed to create self-improvement mission: {e}");
    }

    // Wire mission manager into effect adapter for mission_* function calls
    effect_adapter
        .set_mission_manager(Arc::clone(&mission_manager))
        .await;

    *guard = Some(EngineState {
        thread_manager,
        conversation_manager,
        effect_adapter,
        store: store.clone(),
        default_project_id: project_id,
        pending_approvals: RwLock::new(HashMap::new()),
        sse: agent.deps.sse_tx.clone(),
        db: agent.deps.store.clone(),
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
        .map_err(|e| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 store error: {e}"),
            })
        })?
        .ok_or_else(|| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 thread {} not found", pending.thread_id),
            })
        })?;

    let metadata = thread.metadata.as_object_mut().ok_or_else(|| {
        crate::error::Error::from(crate::error::JobError::ContextError {
            id: uuid::Uuid::nil(),
            reason: "engine v2 thread metadata must be an object".into(),
        })
    })?;
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
    store.save_thread(&thread).await.map_err(|e| {
        crate::error::Error::from(crate::error::JobError::ContextError {
            id: uuid::Uuid::nil(),
            reason: format!("engine v2 store error: {e}"),
        })
    })
}

async fn load_pending_approval_from_thread(
    store: &Arc<dyn Store>,
    conversation_id: ironclaw_engine::ConversationId,
    thread_id: ironclaw_engine::ThreadId,
) -> Result<Option<PendingApproval>, Error> {
    let Some(thread) = store.load_thread(thread_id).await.map_err(|e| {
        crate::error::Error::from(crate::error::JobError::ContextError {
            id: uuid::Uuid::nil(),
            reason: format!("engine v2 store error: {e}"),
        })
    })?
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
    let Some(mut thread) = store.load_thread(thread_id).await.map_err(|e| {
        crate::error::Error::from(crate::error::JobError::ContextError {
            id: uuid::Uuid::nil(),
            reason: format!("engine v2 store error: {e}"),
        })
    })?
    else {
        return Ok(());
    };

    if let Some(metadata) = thread.metadata.as_object_mut() {
        metadata.remove(PENDING_APPROVAL_METADATA_KEY);
        thread.updated_at = chrono::Utc::now();
        store.save_thread(&thread).await.map_err(|e| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 store error: {e}"),
            })
        })?;
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

    let conversations = store.list_conversations(user_id).await.map_err(|e| {
        crate::error::Error::from(crate::error::JobError::ContextError {
            id: uuid::Uuid::nil(),
            reason: format!("engine v2 store error: {e}"),
        })
    })?;

    let mut candidates = Vec::new();
    for conversation in conversations {
        for thread_id in conversation.active_threads {
            if hinted_thread_id.is_some_and(|hint| thread_id.0 != hint) {
                continue;
            }

            let Some(thread) = store.load_thread(thread_id).await.map_err(|e| {
                crate::error::Error::from(crate::error::JobError::ContextError {
                    id: uuid::Uuid::nil(),
                    reason: format!("engine v2 store error: {e}"),
                })
            })?
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
    get_or_init_engine(agent).await?;

    let lock = ENGINE_STATE.get().expect("engine initialized");
    let guard = lock.read().await;
    let state = guard.as_ref().expect("engine initialized");

    let pending = match resolve_pending_approval_for_thread(
        &state.store,
        &state.pending_approvals,
        &message.user_id,
        message.thread_id.as_deref(),
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

    // Approved — persist auto-approval when user chose "always"
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
        .map_err(|e| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 resume error: {e}"),
            })
        })?;
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

/// Handle a user message through the engine v2 pipeline.
pub async fn handle_with_engine(
    agent: &Agent,
    message: &IncomingMessage,
    content: &str,
) -> Result<Option<String>, Error> {
    // Ensure engine is initialized
    get_or_init_engine(agent).await?;

    let lock = ENGINE_STATE.get().expect("engine initialized");
    let guard = lock.read().await;
    let state = guard.as_ref().expect("engine initialized");

    debug!(
        user_id = %message.user_id,
        channel = %message.channel,
        "engine v2: handling message"
    );

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

    // Get or create conversation for this channel+user
    let conv_id = state
        .conversation_manager
        .get_or_create_conversation(&message.channel, &message.user_id)
        .await
        .map_err(|e| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 conversation error: {e}"),
            })
        })?;

    // Handle the message — spawns a new thread or injects into active one
    let thread_id = state
        .conversation_manager
        .handle_user_message(
            conv_id,
            content,
            state.default_project_id,
            &message.user_id,
            ThreadConfig {
                enable_reflection: true,
                ..ThreadConfig::default()
            },
        )
        .await
        .map_err(|e| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 error: {e}"),
            })
        })?;

    if let Some(ref db) = state.db
        && let Ok(conv_id_v1) = db
            .get_or_create_assistant_conversation(&message.user_id, &message.channel)
            .await
    {
        let _ = db
            .add_conversation_message(conv_id_v1, "user", content)
            .await;
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

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Ok(ref evt) if evt.thread_id == thread_id => {
                        forward_event_to_channel(evt, channels, channel_name, metadata).await;
                        if let Some(sse) = sse
                            && let Some(app_event) = thread_event_to_app_event(evt, &tid_str)
                        {
                            sse.broadcast_for_user(&message.user_id, app_event);
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
            }
        }
    }

    let outcome = state
        .thread_manager
        .join_thread(thread_id)
        .await
        .map_err(|e| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 join error: {e}"),
            })
        })?;

    state
        .conversation_manager
        .record_thread_outcome(conv_id, thread_id, &outcome)
        .await
        .map_err(|e| {
            crate::error::Error::from(crate::error::JobError::ContextError {
                id: uuid::Uuid::nil(),
                reason: format!("engine v2 conversation error: {e}"),
            })
        })?;

    if let Some(ref db) = state.db
        && let Ok(conv_id_v1) = db
            .get_or_create_assistant_conversation(&message.user_id, &message.channel)
            .await
        && let ThreadOutcome::Completed {
            response: Some(ref text),
        } = outcome
    {
        let _ = db
            .add_conversation_message(conv_id_v1, "assistant", text)
            .await;
    }

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

    match outcome {
        ThreadOutcome::Completed { response } => {
            debug!(thread_id = %thread_id, "engine v2: completed");
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
                    StatusUpdate::Thinking("Thinking...".into()),
                    metadata,
                )
                .await;
        }
        EventKind::ActionExecuted { action_name, .. } => {
            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::ToolCompleted {
                        name: action_name.clone(),
                        success: true,
                        error: None,
                        parameters: None,
                    },
                    metadata,
                )
                .await;
        }
        EventKind::ActionFailed {
            action_name, error, ..
        } => {
            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::ToolCompleted {
                        name: action_name.clone(),
                        success: false,
                        error: Some(error.clone()),
                        parameters: None,
                    },
                    metadata,
                )
                .await;
        }
        EventKind::StepCompleted { .. } => {
            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::Thinking("Processing results...".into()),
                    metadata,
                )
                .await;
        }
        _ => {}
    }
}

/// Convert a ThreadEvent to an AppEvent for the web gateway SSE stream.
fn thread_event_to_app_event(
    event: &ironclaw_engine::ThreadEvent,
    thread_id: &str,
) -> Option<AppEvent> {
    use ironclaw_engine::EventKind;

    match &event.kind {
        EventKind::StepStarted { .. } => Some(AppEvent::Thinking {
            message: "Thinking...".into(),
            thread_id: Some(thread_id.into()),
        }),
        EventKind::ActionExecuted { action_name, .. } => Some(AppEvent::ToolCompleted {
            name: action_name.clone(),
            success: true,
            error: None,
            parameters: None,
            thread_id: Some(thread_id.into()),
        }),
        EventKind::ActionFailed {
            action_name, error, ..
        } => Some(AppEvent::ToolCompleted {
            name: action_name.clone(),
            success: false,
            error: Some(error.clone()),
            parameters: None,
            thread_id: Some(thread_id.into()),
        }),
        EventKind::StepCompleted { .. } => Some(AppEvent::Status {
            message: "Processing results...".into(),
            thread_id: Some(thread_id.into()),
        }),
        _ => None,
    }
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
