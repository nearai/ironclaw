//! Engine v2 router — handles user messages via the engine when enabled.

use std::path::{Path, PathBuf};
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
use crate::secrets::SecretBindingApproval;
use crate::secrets::binding_approvals::{
    SECRET_BINDING_APPROVAL_GATE_NAME, approval_prompt_message, grant_binding_approval,
};

/// Typed outcome from a v2 bridge handler.
///
/// Replaces the ambiguous `Option<String>` where `None` could mean either
/// "gate created, turn paused" or "completed with no text response". Each
/// variant now encodes the handler's intent explicitly.
#[derive(Debug)]
#[must_use]
pub enum BridgeOutcome {
    /// Send this text response to the user and end the turn.
    Respond(String),
    /// No text response, but the turn completes normally.
    NoResponse,
    /// Turn is paused — a gate (approval/auth/external) was created and the
    /// user must resolve it before the turn continues. The agent loop must
    /// NOT emit a terminal `Done` status.
    Pending,
}

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

const PROJECT_ATTACHMENT_DIR: &str = ".ironclaw/attachments";

#[derive(Debug, Clone)]
struct AttachmentIndexNote {
    title: String,
    content: String,
    metadata: serde_json::Value,
    tags: Vec<String>,
}

fn sanitize_attachment_segment(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('.');
    if sanitized.is_empty() {
        "attachment".to_string()
    } else {
        sanitized.to_string()
    }
}

fn fallback_attachment_filename(index: usize, mime_type: &str) -> String {
    let ext = crate::channels::attachment_extension_for_mime(mime_type);
    format!("attachment-{}.{}", index + 1, ext)
}

fn attachment_project_relative_path(
    message: &IncomingMessage,
    project_id: ironclaw_engine::ProjectId,
    attachment: &crate::channels::IncomingAttachment,
    index: usize,
) -> String {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let owner = sanitize_attachment_segment(&message.user_id);
    let message_id = sanitize_attachment_segment(&message.id.to_string());
    let filename = attachment
        .filename
        .as_deref()
        .map(sanitize_attachment_segment)
        .unwrap_or_else(|| fallback_attachment_filename(index, &attachment.mime_type));
    format!(
        "{}/{}/{}/{}/{}-{}",
        PROJECT_ATTACHMENT_DIR, owner, project_id, date, message_id, filename
    )
}

/// Collapse anything that could break a markdown title/backtick span in a
/// user-supplied filename before embedding it. User content in attachment
/// filenames goes straight into `# Uploaded attachment: ...` and into the
/// note's `title`, so raw newlines / backticks / odd ASCII control codes
/// would corrupt the agent-visible transcript (and, for a title, the
/// searchable memory-doc row).
fn sanitize_filename_for_display(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            '`' => out.push('\''),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return "attachment".to_string();
    }
    // Clamp so a pathological filename can't flood the agent prompt.
    const MAX_DISPLAY_LEN: usize = 256;
    if trimmed.len() <= MAX_DISPLAY_LEN {
        trimmed.to_string()
    } else {
        let mut t = trimmed.to_string();
        t.truncate(MAX_DISPLAY_LEN);
        t
    }
}

fn attachment_index_note(
    message: &IncomingMessage,
    attachment: &crate::channels::IncomingAttachment,
    relative_path: &str,
) -> AttachmentIndexNote {
    let raw_filename = attachment.filename.as_deref().unwrap_or("attachment");
    let filename = sanitize_filename_for_display(raw_filename);
    let attachment_type = match attachment.kind {
        crate::channels::AttachmentKind::Audio => "audio",
        crate::channels::AttachmentKind::Image => "image",
        crate::channels::AttachmentKind::Document => "document",
    };
    let mut content = format!(
        "# Uploaded attachment: {filename}\n\n\
         - Project file: `{relative_path}`\n\
         - Attachment type: `{attachment_type}`\n\
         - MIME type: `{}`\n\
         - Size: `{}` bytes\n\
         - Uploaded by: `{}` via `{}`\n",
        attachment.mime_type,
        attachment
            .size_bytes
            .unwrap_or(attachment.data.len() as u64),
        message.user_id,
        message.channel,
    );

    match attachment.kind {
        crate::channels::AttachmentKind::Audio => {
            if let Some(text) = attachment.extracted_text.as_deref() {
                content.push_str("\n## Transcript\n\n");
                content.push_str(text);
            } else {
                content.push_str("\nTranscript unavailable. The original audio file is stored at the project file path above.");
            }
        }
        crate::channels::AttachmentKind::Image => {
            content.push_str(
                "\nThe original image file is stored at the project file path above. Use that file path in later shell or skill commands if needed.",
            );
        }
        crate::channels::AttachmentKind::Document => {
            if let Some(text) = attachment.extracted_text.as_deref() {
                content.push_str("\n## Extracted text\n\n");
                content.push_str(text);
            } else {
                content.push_str("\nText extraction unavailable. The original document file is stored at the project file path above.");
            }
        }
    }

    AttachmentIndexNote {
        title: format!("attachment:{filename}"),
        content,
        metadata: serde_json::json!({
            "kind": "project_attachment",
            "attachment_type": attachment_type,
            "filename": filename,
            "mime_type": attachment.mime_type,
            "project_path": relative_path,
            "message_id": message.id.to_string(),
        }),
        tags: vec![
            "attachment".to_string(),
            "upload".to_string(),
            attachment_type.to_string(),
        ],
    }
}

async fn persist_project_attachments(
    project_root: &Path,
    message: &IncomingMessage,
    project_id: ironclaw_engine::ProjectId,
    attachments: &mut [crate::channels::IncomingAttachment],
) -> Vec<AttachmentIndexNote> {
    let mut notes = Vec::new();

    for (index, attachment) in attachments.iter_mut().enumerate() {
        if attachment.data.is_empty() || attachment.local_path.is_some() {
            continue;
        }

        let relative_path =
            attachment_project_relative_path(message, project_id, attachment, index);
        let absolute_path = project_root.join(Path::new(&relative_path));
        let Some(parent) = absolute_path.parent() else {
            tracing::warn!(path = %absolute_path.display(), "engine v2: attachment path had no parent");
            continue;
        };

        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            tracing::warn!(path = %parent.display(), error = %e, "engine v2: failed to create attachment directory");
            continue;
        }

        if let Err(e) = tokio::fs::write(&absolute_path, &attachment.data).await {
            tracing::warn!(path = %absolute_path.display(), error = %e, "engine v2: failed to persist attachment file");
            continue;
        }

        attachment.local_path = Some(relative_path.clone());
        // Build the index note while `data` is still populated so the
        // fallback to `data.len()` in `attachment_index_note` reports the
        // real payload size when `size_bytes` wasn't pre-filled.
        notes.push(attachment_index_note(message, attachment, &relative_path));
        // Intentionally *don't* clear `attachment.data` here. The caller
        // (`handle_with_engine_inner` in this file) immediately feeds the
        // same slice to `augment_with_attachments`, which only emits
        // multimodal `image_parts` for images when `att.data` is non-empty.
        // Clearing the buffer here would silently drop every uploaded image
        // from the engine-v2 LLM request — the file is on disk but the
        // model never sees the bytes. The `persisted_attachments` Vec is
        // local to the request and is dropped once the engine dispatch
        // returns, so "storage hygiene" is a no-op anyway.
    }

    notes
}

fn resolve_project_root() -> PathBuf {
    let base_dir = crate::bootstrap::ironclaw_base_dir();
    base_dir.parent().map(PathBuf::from).unwrap_or(base_dir)
}

async fn save_attachment_index_notes(
    store: &Arc<dyn Store>,
    project_id: ironclaw_engine::ProjectId,
    user_id: &str,
    thread_id: ironclaw_engine::ThreadId,
    notes: Vec<AttachmentIndexNote>,
) {
    for note in notes {
        let mut doc = ironclaw_engine::MemoryDoc::new(
            project_id,
            user_id,
            ironclaw_engine::DocType::Note,
            note.title,
            note.content,
        );
        doc.metadata = note.metadata;
        doc.tags = note.tags;
        doc.source_thread_id = Some(thread_id);
        if let Err(e) = store.save_memory_doc(&doc).await {
            tracing::warn!(error = %e, title = %doc.title, "engine v2: failed to save attachment index note");
        }
    }
}

fn gate_display_parameters(pending: &PendingGate) -> serde_json::Value {
    pending
        .display_parameters
        .clone()
        .unwrap_or_else(|| pending.parameters.clone())
}

fn secret_binding_approval_from_resume_output(
    value: Option<&serde_json::Value>,
) -> Option<SecretBindingApproval> {
    value
        .and_then(|value| value.get("secret_binding_approval"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

/// Resolve the owning extension name for a tool action, falling back to a
/// credential name when the action isn't extension-backed. This is the
/// shared core of the auth-gate display + submit routing logic — the same
/// `provider_extension_for_tool + unwrap_or_else(credential_name)` pattern
/// fired in three different sites in this file before, each one with the
/// same fallback rationale: the engine's `ResumeKind::Authentication` only
/// carries `credential_name` (e.g. `google_oauth_token`), which is opaque
/// to users AND fails when fed back into `submit_auth_token` for
/// WASM-tool-backed credentials, while the owning extension name (e.g.
/// `google-drive-tool`) is what both the user-facing UI and
/// `submit_auth_token` actually want. For built-in tools, HTTP, and skill
/// credentials there's no owning extension and the fallback is the right
/// thing.
async fn resolve_extension_for_action(
    auth_manager: Option<&AuthManager>,
    extension_manager: Option<&crate::extensions::ExtensionManager>,
    tools: &crate::tools::ToolRegistry,
    action_name: &str,
    parameters: &serde_json::Value,
    credential_fallback: &str,
    user_id: &str,
) -> ironclaw_common::ExtensionName {
    if let Some(auth_manager) = auth_manager {
        // Resolver enforces identity validation on user-influenced branches
        // and returns a typed `ExtensionName` directly — no wrap needed.
        return auth_manager
            .resolve_extension_name_for_auth_flow(
                action_name,
                parameters,
                credential_fallback,
                user_id,
            )
            .await;
    }
    // No auth manager (hosted instance without SECRETS_MASTER_KEY, or bare
    // test harness): delegate to the same canonical resolver used by the
    // auth-manager path so the extension-manager branch of the precedence
    // still runs instead of falling through to a stringly credential name.
    crate::bridge::auth_manager::resolve_auth_flow_extension_name(
        action_name,
        parameters,
        credential_fallback,
        user_id,
        Some(tools),
        extension_manager,
    )
    .await
}

/// Resolve the installed extension identifier that owns an authentication
/// gate, for surfacing that gate on a channel.
///
/// Returns `Some(ExtensionName)` only for `Authentication` gates — the
/// resolver delegates to [`resolve_extension_for_action`]. Non-auth
/// gate variants (`Approval`, `External`) don't have an extension
/// identity and return `None`.
async fn resolve_auth_gate_extension_name(
    auth_manager: Option<&AuthManager>,
    extension_manager: Option<&crate::extensions::ExtensionManager>,
    tools: &crate::tools::ToolRegistry,
    pending: &PendingGate,
) -> Option<ironclaw_common::ExtensionName> {
    let ironclaw_engine::ResumeKind::Authentication {
        credential_name, ..
    } = &pending.resume_kind
    else {
        return None;
    };
    Some(
        resolve_extension_for_action(
            auth_manager,
            extension_manager,
            tools,
            &pending.action_name,
            &pending.parameters,
            credential_name.as_str(),
            &pending.user_id,
        )
        .await,
    )
}

async fn send_pending_gate_status(
    agent: &Agent,
    message: &IncomingMessage,
    pending: &PendingGate,
    extension_name: Option<&ironclaw_common::ExtensionName>,
) {
    let display_parameters = gate_display_parameters(pending);

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
        }
        ironclaw_engine::ResumeKind::Authentication {
            instructions,
            auth_url,
            ..
        } => {
            // `resolve_auth_gate_extension_name` always returns `Some` for
            // Authentication gates; a `None` here would be an upstream
            // plumbing bug (wrong variant reached this arm).
            let Some(extension_name) = extension_name else {
                tracing::warn!(
                    gate = %pending.gate_name,
                    request_id = %pending.request_id,
                    "Authentication gate reached send_pending_gate_status without a resolved extension name"
                );
                return;
            };
            let _ = agent
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::AuthRequired {
                        extension_name: extension_name.clone(),
                        instructions: Some(instructions.clone()),
                        auth_url: auth_url.clone(),
                        setup_url: None,
                        request_id: Some(pending.request_id.to_string()),
                    },
                    &message.metadata,
                )
                .await;
        }
        ironclaw_engine::ResumeKind::External { .. } => {}
    }
}

fn resumed_action_result_message(
    call_id: &str,
    action_name: &str,
    output: &serde_json::Value,
) -> ironclaw_engine::ThreadMessage {
    let rendered = serde_json::to_string_pretty(output).unwrap_or_else(|_| output.to_string());
    ironclaw_engine::ThreadMessage::action_result(call_id, action_name, rendered)
}

/// Resolve the assistant action `call_id` that a pending gate corresponds to.
///
/// Returns `None` when neither the persisted `call_id` nor a history scan can
/// produce a match. Callers must treat `None` as a real miss and synthesize a
/// fresh id rather than collapsing it into an empty string — an empty
/// `action_call_id` on a `ThreadMessage::action_result` corrupts the engine's
/// call/result pairing and causes the assistant to drop the resumed reply.
fn resolved_call_id_for_pending_action(
    thread: &ironclaw_engine::Thread,
    pending: &PendingGate,
) -> Option<String> {
    // New pending gates persist the exact call_id at insertion time.
    // Only infer from history for legacy rows created before call_id was stored.
    if !pending.call_id.is_empty() {
        return Some(pending.call_id.clone());
    }

    // Scan both user-visible `messages` AND `internal_messages` (the
    // orchestrator's working transcript).  In production the orchestrator
    // writes ActionResult messages to `internal_messages` via
    // `sync_runtime_state`, so scanning only `messages` would leave the
    // resolved-ids set empty and the fallback would never match.
    let all_messages = thread
        .messages
        .iter()
        .chain(thread.internal_messages.iter());

    let resolved_ids: HashSet<&str> = all_messages
        .clone()
        .filter_map(|message| {
            (message.role == ironclaw_engine::types::message::MessageRole::ActionResult)
                .then_some(message.action_call_id.as_deref())
                .flatten()
        })
        .collect();

    all_messages.rev().find_map(|message| {
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
}

/// Synthesize a fresh action call id when no historical id can be recovered.
///
/// Used as a last-resort so the resumed `ActionResult` message still carries a
/// non-empty correlator and the engine does not silently drop the reply.
pub(super) fn synthetic_action_call_id(action_name: &str) -> String {
    format!("synthetic-{}-{}", action_name, uuid::Uuid::new_v4())
}

async fn resolved_or_synthetic_call_id_for_pending_action(
    state: &EngineState,
    pending: &PendingGate,
) -> Result<String, Error> {
    let thread = state
        .store
        .load_thread(pending.thread_id)
        .await
        .map_err(|e| engine_err("load thread", e))?
        .ok_or_else(|| engine_err("load thread", "thread not found"))?;

    Ok(
        resolved_call_id_for_pending_action(&thread, pending).unwrap_or_else(|| {
            tracing::warn!(
                action = %pending.action_name,
                thread_id = %pending.thread_id,
                "no historical call_id for pending gate; synthesizing one to keep \
                 ActionResult correlator non-empty"
            );
            synthetic_action_call_id(&pending.action_name)
        }),
    )
}

/// Validate a credential identifier shape: non-empty, ≤64 chars, ASCII
/// alphanumeric or underscore only. Used by the auth-fallback parser to
/// reject anything that isn't structurally a credential name before it's
/// checked against the registry.
fn is_valid_credential_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Extract a credential name from a tool error / model response that
/// contains an `authentication_required` signal.
///
/// Tries structured JSON first (the http tool emits a JSON object with a
/// `credential_name` field), then falls back to a prose-shaped splitter for
/// free-form errors. The result must additionally pass
/// [`is_valid_credential_name`] before the caller may use it. The caller
/// must still verify the name against the credential registry — this
/// function only normalizes the parse, it does NOT establish trust.
fn parse_credential_name(text: &str) -> Option<String> {
    // Pass 1 — full-text JSON. Cheap and unambiguous when the producer is
    // a tool that already serialized a structured error.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
        && let Some(name) = value.get("credential_name").and_then(|v| v.as_str())
        && is_valid_credential_name(name)
    {
        return Some(name.to_string());
    }

    // Pass 2 — embedded JSON. Slice from the first `{` to the matching
    // closing `}` and try again. We don't try to handle nested objects
    // robustly; the http tool emits a flat shape.
    if let Some(start) = text.find('{')
        && let Some(end) = text[start..].rfind('}')
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text[start..=start + end])
        && let Some(name) = value.get("credential_name").and_then(|v| v.as_str())
        && is_valid_credential_name(name)
    {
        return Some(name.to_string());
    }

    // Pass 3 — prose splitter. Last-resort path for free-form text that
    // mentions `credential_name` without proper JSON structure. Kept narrow
    // and validated. nth(1) intentionally takes the FIRST occurrence of
    // `credential_name`; if a tool emits multiple, only the first wins —
    // and the registry check downstream still gates whether it's honored.
    text.split("credential_name")
        .nth(1)
        .and_then(|s| {
            // Slice on a char-array (not a string) is safe — no UTF-8
            // boundary issues.
            s.split(&['"', '\'', '`'][..])
                .find(|seg| !seg.is_empty() && !seg.contains(':') && !seg.contains(' '))
        })
        .filter(|name| is_valid_credential_name(name))
        .map(|s| s.to_string())
}

/// Notify all surfaces about a pending gate: SSE broadcast (if `sse` is
/// some) plus the channel-level status event and the user-facing prompt.
///
/// Takes `sse` as an owned `Option<Arc<SseManager>>` rather than borrowing
/// from `&EngineState` so callers can clone the Arc out of the engine
/// state read-guard and `drop(guard)` *before* awaiting on broadcast +
/// channel I/O. Holding the engine state guard across these awaits is
/// fine in steady-state production (the outer lock is read-only after
/// init) but breaks down for tests that tear the state down concurrently
/// and is fragile for any future hot-reload path. The `handle_with_engine`
/// terminal-return branches (auth + approval) both rely on this drop
/// discipline to release the guard before talking to the user.
/// Re-notify the user about a pending gate via the channel-level status
/// event (approval card, auth prompt, etc.). Returns `None` — the card
/// is the only user-facing signal; no text response is emitted.
///
/// The `_sse` and `tools` parameters are retained so callers can clone
/// the Arc out of the engine state guard and `drop(guard)` before this
/// await, keeping the read-lock scope tight.
async fn notify_pending_gate(
    agent: &Agent,
    sse: Option<Arc<SseManager>>,
    tools: &crate::tools::ToolRegistry,
    auth_manager: Option<&AuthManager>,
    extension_manager: Option<&crate::extensions::ExtensionManager>,
    message: &IncomingMessage,
    pending: &PendingGate,
) -> Result<BridgeOutcome, Error> {
    let extension_name =
        resolve_auth_gate_extension_name(auth_manager, extension_manager, tools, pending).await;

    if let ironclaw_engine::ResumeKind::External { callback_id } = &pending.resume_kind {
        tracing::debug!(
            gate = %pending.gate_name,
            callback = %callback_id,
            "GatePaused(External)"
        );
    }

    // Send the approval/auth card via the source channel. Each channel
    // renders this natively (web → SSE card, TUI → widget, relay →
    // buttons). No text response is returned to avoid a duplicate message
    // alongside the card.
    if let Some(ref sse) = sse {
        let display_parameters = gate_display_parameters(pending);
        sse.broadcast_for_user(
            &message.user_id,
            AppEvent::GateRequired {
                request_id: pending.request_id.to_string(),
                gate_name: pending.gate_name.clone(),
                tool_name: pending.action_name.clone(),
                description: pending.description.clone(),
                parameters: serde_json::to_string_pretty(&display_parameters)
                    .unwrap_or_else(|_| display_parameters.to_string()),
                extension_name: extension_name.clone(),
                resume_kind: serde_json::to_value(&pending.resume_kind).unwrap_or_default(),
                thread_id: Some(pending.effective_wire_thread_id()),
            },
        );
    }
    send_pending_gate_status(agent, message, pending, extension_name.as_ref()).await;
    Ok(BridgeOutcome::Pending)
}

async fn insert_and_notify_pending_gate(
    agent: &Agent,
    state: &EngineState,
    message: &IncomingMessage,
    pending: PendingGate,
) -> Result<BridgeOutcome, Error> {
    state
        .pending_gates
        .insert(pending.clone())
        .await
        .map_err(|e| engine_err("pending gate insert", e))?;

    notify_pending_gate(
        agent,
        state.sse.clone(),
        state.effect_adapter.tools(),
        state.auth_manager.as_deref(),
        state.extension_manager.as_deref(),
        message,
        &pending,
    )
    .await
}

async fn requeue_auth_pending_gate(
    agent: &Agent,
    state: &EngineState,
    message: &IncomingMessage,
    pending: &PendingGate,
    instructions: String,
    auth_url: Option<String>,
) -> Result<BridgeOutcome, Error> {
    // This path replaces the just-resolved gate for the same `(user, thread)`.
    // `resolve_gate()` has already removed the old gate atomically, and
    // `PendingGateStore::insert()` still enforces at most one live gate per
    // `(user_id, thread_id)`, so retries remain bounded by active paused
    // threads rather than growing unbounded per invalid token attempt.
    let credential_name = match &pending.resume_kind {
        ironclaw_engine::ResumeKind::Authentication {
            credential_name, ..
        } => credential_name.clone(),
        other => {
            return Err(engine_err(
                "resolution mismatch",
                format!("expected authentication gate, got {}", other.kind_name()),
            ));
        }
    };

    let next_pending = PendingGate {
        request_id: uuid::Uuid::new_v4(),
        gate_name: pending.gate_name.clone(),
        user_id: pending.user_id.clone(),
        thread_id: pending.thread_id,
        scope_thread_id: pending.scope_thread_id.clone(),
        conversation_id: pending.conversation_id,
        source_channel: pending.source_channel.clone(),
        action_name: pending.action_name.clone(),
        call_id: pending.call_id.clone(),
        parameters: pending.parameters.clone(),
        display_parameters: pending.display_parameters.clone(),
        description: pending.description.clone(),
        resume_kind: ironclaw_engine::ResumeKind::Authentication {
            credential_name,
            instructions,
            auth_url,
        },
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
        original_message: pending.original_message.clone(),
        resume_output: pending.resume_output.clone(),
        paused_lease: pending.paused_lease.clone(),
        approval_already_granted: pending.approval_already_granted,
    };

    insert_and_notify_pending_gate(agent, state, message, next_pending).await
}

fn pairing_pending_gate_from_auth(pending: &PendingGate, extension_name: &str) -> PendingGate {
    PendingGate {
        request_id: uuid::Uuid::new_v4(),
        gate_name: "pairing".into(),
        user_id: pending.user_id.clone(),
        thread_id: pending.thread_id,
        scope_thread_id: pending.scope_thread_id.clone(),
        conversation_id: pending.conversation_id,
        source_channel: pending.source_channel.clone(),
        action_name: pending.action_name.clone(),
        call_id: pending.call_id.clone(),
        parameters: pending.parameters.clone(),
        display_parameters: pending.display_parameters.clone(),
        description: format!("Pairing required for '{extension_name}'."),
        resume_kind: ironclaw_engine::ResumeKind::External {
            callback_id: format!("pairing:{extension_name}"),
        },
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
        original_message: pending.original_message.clone(),
        resume_output: pending.resume_output.clone(),
        paused_lease: pending.paused_lease.clone(),
        approval_already_granted: pending.approval_already_granted,
    }
}

async fn requeue_pairing_pending_gate(
    state: &EngineState,
    pending: &PendingGate,
    extension_name: &str,
) -> Result<PendingGate, Error> {
    let next_pending = pairing_pending_gate_from_auth(pending, extension_name);
    state
        .pending_gates
        .insert(next_pending.clone())
        .await
        .map_err(|e| engine_err("pending pairing gate insert", e))?;
    Ok(next_pending)
}

/// Persist `AlwaysAllow` to DB when the user clicks "always approve".
///
/// Defense-in-depth: tools that declare `ApprovalRequirement::Always` for
/// the actual pending parameters are never persisted (the UI hides the
/// button, but a crafted client could send it). Tool names are validated
/// before use as settings keys.
///
/// Returns the pre-existing permission value (if any) so the caller can
/// restore it on failure via [`revert_always_allow`].
async fn persist_always_allow(
    agent: &Agent,
    state: &EngineState,
    pending: &PendingGate,
) -> Option<serde_json::Value> {
    // Validate tool name before using it as a settings key. Reject names
    // that contain dots or other characters that could collide with the
    // dotted-path settings namespace.
    if !crate::tools::permissions::is_valid_admin_tool_name(&pending.action_name) {
        debug!(
            tool = %pending.action_name,
            "Skipping AlwaysAllow persist — invalid tool name"
        );
        return None;
    }

    // Defense-in-depth: skip persistence for ApprovalRequirement::Always
    // tools. Uses the actual pending parameters so param-dependent tools
    // (e.g. shell with high-risk commands) are correctly detected.
    let is_locked = state
        .effect_adapter
        .tools()
        .get(&pending.action_name)
        .await
        .map(|t| {
            matches!(
                t.requires_approval(&pending.parameters),
                crate::tools::ApprovalRequirement::Always
            )
        })
        .unwrap_or(false);

    if is_locked {
        debug!(
            tool = %pending.action_name,
            "Skipping AlwaysAllow persist — tool declares ApprovalRequirement::Always"
        );
        return None;
    }

    // Use the CachedSettingsStore exclusively. The raw Database fallback
    // bypasses cache invalidation, causing GET /api/settings/tools to serve
    // stale data until the 5-minute TTL expires. In production the settings
    // store is always available when the DB is; the fallback was dead code
    // that actively broke cache coherence in tests and edge deployments.
    let store: &(dyn crate::db::SettingsStore + Send + Sync) = match &agent.deps.settings_store {
        Some(ss) => ss.as_ref(),
        None => return None,
    };

    let key = format!("tool_permissions.{}", pending.action_name);

    // Read the pre-existing value so we can restore it on failure instead
    // of blindly deleting a long-standing user preference.
    let prior = match store.get_setting(&pending.user_id, &key).await {
        Ok(v) => v,
        Err(e) => {
            debug!(
                tool = %pending.action_name,
                error = %e,
                "resolve_gate: failed to read prior permission, skipping persist"
            );
            return None;
        }
    };

    let val = serde_json::to_value(crate::tools::permissions::PermissionState::AlwaysAllow)
        .unwrap_or(serde_json::json!("always_allow"));

    // dispatch-exempt: engine-internal persist mirrors v1 thread_ops write-through
    match store.set_setting(&pending.user_id, &key, &val).await {
        Ok(()) => debug!(
            tool = %pending.action_name,
            user_id = %pending.user_id,
            "Persisted AlwaysAllow permission to DB settings (engine v2)"
        ),
        Err(e) => tracing::warn!(
            tool = %pending.action_name,
            user_id = %pending.user_id,
            error = %e,
            "resolve_gate: failed to persist AlwaysAllow"
        ),
    }

    prior
}

/// Revert `AlwaysAllow` from DB when a resumed tool execution fails.
///
/// Restores the `prior` value that existed before [`persist_always_allow`]
/// wrote `AlwaysAllow`. If there was no prior value, deletes the key.
async fn revert_always_allow(
    agent: &Agent,
    pending: &PendingGate,
    prior: Option<serde_json::Value>,
) {
    let store: &(dyn crate::db::SettingsStore + Send + Sync) = match &agent.deps.settings_store {
        Some(ss) => ss.as_ref(),
        None => return,
    };

    let key = format!("tool_permissions.{}", pending.action_name);
    let result = match prior {
        // dispatch-exempt: engine-internal revert of persist_always_allow
        Some(ref val) => store
            .set_setting(&pending.user_id, &key, val)
            .await
            .map(|_| ()),
        // dispatch-exempt: engine-internal revert of persist_always_allow
        None => store
            .delete_setting(&pending.user_id, &key)
            .await
            .map(|_| ()),
    };
    if let Err(e) = result {
        tracing::warn!(
            tool = %pending.action_name,
            user_id = %pending.user_id,
            error = %e,
            "resolve_gate: failed to revert AlwaysAllow after execution failure"
        );
    }
}

/// Validate that a `paused_lease` snapshot recorded when a gate paused
/// still represents a usable lease at resume time.
///
/// A gate can sit in the pending-gate store for hours or across process
/// restarts; during that window the original lease might have been
/// revoked, expired, or the pending record could have drifted off its
/// original thread. Callers that fail this check must NOT use the
/// snapshot — fall through to `LeaseManager::find_lease_for_action`
/// (which enforces its own scoping) or fail closed.
fn snapshot_lease_still_valid(
    lease: &ironclaw_engine::CapabilityLease,
    pending: &PendingGate,
) -> bool {
    lease.thread_id == pending.thread_id
        && lease.granted_actions.covers(&pending.action_name)
        && !lease.revoked
        && lease
            .expires_at
            .map(|exp| exp > chrono::Utc::now())
            .unwrap_or(true)
}

/// Pick the lease to use for resuming a pending gate action. Prefers the
/// `paused_lease` snapshot the gate recorded if it's still valid; falls
/// back to a live lookup in the `LeaseManager`. Returns `None` if neither
/// path yields a lease — the caller maps that to a "no active lease"
/// error.
async fn resume_lease_for_pending_gate(
    pending: &PendingGate,
    leases: &ironclaw_engine::LeaseManager,
) -> Option<ironclaw_engine::CapabilityLease> {
    if let Some(snapshot) = pending.paused_lease.clone()
        && snapshot_lease_still_valid(&snapshot, pending)
    {
        return Some(snapshot);
    }
    leases
        .find_lease_for_action(pending.thread_id, &pending.action_name)
        .await
}

/// Broadcast a `GateResolved { resolution: "expired" }` event and return the
/// dismissal outcome. Used when the target thread has been deleted between
/// `take_verified` and resume, so there's no live thread to execute against.
///
/// Callers that persist side effects (e.g. `Approved { always }` writing
/// `AlwaysAllow` to settings) MUST pre-flight with `state.store.load_thread`
/// and call this helper *before* persisting, so a missing thread doesn't
/// silently commit a long-lived preference for a tool that never ran (#2347).
fn emit_gate_expired_dismissal(
    state: &EngineState,
    message: &IncomingMessage,
    pending: &PendingGate,
) -> BridgeOutcome {
    tracing::debug!(
        thread_id = %pending.thread_id,
        gate = %pending.gate_name,
        action = %pending.action_name,
        "thread not found for pending gate; emitting expired resolution"
    );
    if let Some(ref sse) = state.sse {
        sse.broadcast_for_user(
            &message.user_id,
            AppEvent::GateResolved {
                request_id: pending.request_id.to_string(),
                gate_name: pending.gate_name.clone(),
                tool_name: pending.action_name.clone(),
                resolution: "expired".into(),
                message: "Thread no longer exists.".into(),
                thread_id: Some(pending.effective_wire_thread_id()),
            },
        );
    }
    BridgeOutcome::Respond("Thread no longer exists. Approval dismissed.".into())
}

async fn execute_pending_gate_action(
    agent: &Agent,
    state: &EngineState,
    message: &IncomingMessage,
    pending: &PendingGate,
    approval_already_granted: bool,
    approval_event: Option<(String, bool)>,
) -> Result<BridgeOutcome, Error> {
    let thread = match state.store.load_thread(pending.thread_id).await {
        Ok(Some(t)) => t,
        Ok(None) => return Ok(emit_gate_expired_dismissal(state, message, pending)),
        Err(e) => {
            // Transient DB failure -- propagate so the caller can retry
            // rather than permanently discarding the gate.
            return Err(engine_err("load thread", e));
        }
    };
    let resolved_call_id = resolved_or_synthetic_call_id_for_pending_action(state, pending).await?;

    let lease = resume_lease_for_pending_gate(pending, &state.thread_manager.leases)
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
        active_skill_names: thread
            .active_skills()
            .into_iter()
            .map(|skill| skill.name)
            .collect(),
        source_channel: Some(pending.source_channel.clone()),
        user_timezone: thread
            .metadata
            .get("user_timezone")
            .and_then(|v| v.as_str())
            .and_then(ironclaw_engine::ValidTimezone::parse),
        thread_goal: Some(thread.goal.clone()),
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
            paused_lease,
        }) => {
            let display_parameters = state
                .effect_adapter
                .tools()
                .get(&action_name)
                .await
                .map(|tool| crate::tools::redact_params(&parameters, tool.sensitive_params()));
            let description = if gate_name == SECRET_BINDING_APPROVAL_GATE_NAME {
                secret_binding_approval_from_resume_output(resume_output.as_deref())
                    .map(|approval| approval_prompt_message(&approval))
                    .unwrap_or_else(|| {
                        format!(
                            "Tool '{}' requires {}.",
                            action_name,
                            resume_kind.kind_name()
                        )
                    })
            } else {
                format!(
                    "Tool '{}' requires {}.",
                    action_name,
                    resume_kind.kind_name()
                )
            };
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
                description,
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
                paused_lease: paused_lease.map(|lease| *lease),
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
    /// Extension manager for extension-backed auth/setup when no auth manager exists.
    extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
    /// Filesystem root for project-local attachment persistence.
    project_root: PathBuf,
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

    fail_waiting_thread(
        state,
        user_id,
        thread_id,
        "pending gate missing before resume",
    )
    .await
}

/// Transition a Waiting thread owned by `user_id` to Failed with `reason`.
/// Returns `Ok(false)` when the thread does not exist, is owned by someone
/// else, or is not in `Waiting`.
async fn fail_waiting_thread(
    state: &EngineState,
    user_id: &str,
    thread_id: ironclaw_engine::ThreadId,
    reason: &str,
) -> Result<bool, Error> {
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
        .transition_to(ironclaw_engine::ThreadState::Failed, Some(reason.into()))
        .map_err(|e| engine_err("fail waiting thread", e))?;
    state
        .store
        .save_thread(&thread)
        .await
        .map_err(|e| engine_err("save failed thread", e))?;
    Ok(true)
}

/// Build the user-facing "<message>. Resuming..." status text for the
/// auth-completed Ready arm.
///
/// `result.message` from `ExtensionManager::configure_token` already ends
/// with a period (e.g. `"Configuration saved for 'telegram'."`), so a
/// naive `format!("{}. Resuming...", msg)` produces `"...telegram'.. Resuming..."`
/// — double period. Strip trailing periods and whitespace from the raw
/// message before formatting. Other punctuation is intentionally left
/// alone (no real-world backend message ends in `!`/`?`/etc.).
fn format_auth_completed_resuming(raw: &str) -> String {
    let trimmed = raw.trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    format!("{}. Resuming...", trimmed)
}

/// Outcome of `submit_pending_auth_credential` — distinguishes "a backend
/// stored the credential" from "no backend is configured to store it." The
/// caller maps the latter to either thread-fail (prod) or silent continue
/// (bare-resume test harness), see the match arm in `resolve_gate`.
#[derive(Debug)]
enum PendingAuthCredentialSubmission {
    Stored(Box<crate::extensions::ConfigureResult>),
    SkippedNoBackend,
}

/// Try to persist a user-supplied auth credential, falling back across the
/// three backends in priority order:
///
/// 1. `AuthManager::submit_auth_token(submit_target, ...)` — the canonical
///    path (runs the extension's `configure_token`, validates, and emits a
///    `ConfigureResult`). Requires a secrets-backed auth manager.
/// 2. `ExtensionManager::configure_token(submit_target, ...)` — used on
///    hosted instances that run without `SECRETS_MASTER_KEY`, so no
///    persistent auth manager exists but the extension manager's in-memory
///    secrets store can still accept the credential. `NotInstalled` /
///    `NotFound` fall through so non-extension credentials (plain secrets)
///    are stored in step 3.
/// 3. Plain `SecretsStore::create(credential_name, ...)` — stores the
///    credential verbatim for non-extension actions (HTTP tool, skill
///    credentials) when no extension owns the action.
///
/// # Why two keys (`submit_target` + `credential_name`)
///
/// Steps 1–2 take the **extension** identity (`submit_target`, e.g.
/// `"telegram"`). They resolve the actual secret key by walking the
/// extension's capabilities file — that's the whole point of routing
/// through `configure_token`, which validates the extension is installed
/// and picks the correct required-secret slot.
///
/// Step 3 takes the **credential** identity (`credential_name`, e.g.
/// `"telegram_bot_token"` or `"github_token"`) because the secrets store
/// has no concept of extensions — it stores raw secrets keyed by name.
/// For flows that reach step 3, there is no extension to resolve against
/// (builtin HTTP tool, skill credentials), so the credential name *is*
/// the storage key.
///
/// The asymmetry is intentional: the engine's `ResumeKind::Authentication`
/// only carries `credential_name`, so the caller passes both identities
/// through and each backend picks the one it operates on.
///
/// # Credential-name validation on the step-3 fallback
///
/// Steps 1–2 reject unknown credentials via their capabilities lookup
/// (`auth_manager` through `get_credential_spec`; `extension_manager`
/// through `determine_installed_kind`). Step 3 is the non-extension
/// path, so the only validation is the upstream trust chain: the pending
/// gate's `ResumeKind::Authentication.credential_name` is a typed
/// `CredentialName` (newtype validated at construction), and the pending
/// gate itself was inserted by the engine for a specific tool-call whose
/// auth descriptor produced that credential. The caller here receives
/// the value as `&str` because `CreateSecretParams` is string-typed at
/// the boundary, but it originates from a validated newtype upstream.
///
/// Returns `SkippedNoBackend` when none of the three is available (bare
/// test harness with `resume_output` already staged).
async fn submit_pending_auth_credential(
    state: &EngineState,
    submit_target: &str,
    credential_name: &str,
    token: &str,
    user_id: &str,
) -> Result<PendingAuthCredentialSubmission, crate::extensions::ExtensionError> {
    if let Some(auth_manager) = state.auth_manager.as_ref() {
        return auth_manager
            .submit_auth_token(submit_target, token, user_id)
            .await
            .map(Box::new)
            .map(PendingAuthCredentialSubmission::Stored);
    }

    if let Some(ext_mgr) = state.extension_manager.as_ref() {
        match ext_mgr.configure_token(submit_target, token, user_id).await {
            Ok(result) => return Ok(PendingAuthCredentialSubmission::Stored(Box::new(result))),
            // Not an extension-backed credential — fall through to secrets_store.
            Err(crate::extensions::ExtensionError::NotInstalled(_))
            | Err(crate::extensions::ExtensionError::NotFound(_)) => {}
            Err(other) => return Err(other),
        }
    }

    if let Some(ss) = state.secrets_store.as_ref() {
        // Non-extension path (builtin HTTP, skill credentials): store under
        // the raw credential name. See function docs for why steps 1–2
        // take `submit_target` but step 3 takes `credential_name`.
        let params = crate::secrets::CreateSecretParams::new(credential_name, token);
        ss.create(user_id, params).await.map_err(|e| {
            crate::extensions::ExtensionError::Other(format!("Failed to store credential: {e}"))
        })?;
        return Ok(PendingAuthCredentialSubmission::Stored(Box::new(
            crate::extensions::ConfigureResult {
                message: format!("Credential '{}' stored.", credential_name),
                activated: true,
                pairing_required: false,
                auth_url: None,
                onboarding_state: None,
                onboarding: None,
            },
        )));
    }

    Ok(PendingAuthCredentialSubmission::SkippedNoBackend)
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
    // Propagate the trace HTTP interceptor (live recording or replay) so
    // engine v2 tool dispatch records/replays HTTP exchanges. Without this,
    // recorded traces miss every outbound call made from the engine v2 path
    // and replay can't substitute responses.
    if let Some(ref interceptor) = agent.deps.http_interceptor {
        effect_adapter
            .set_http_interceptor(Arc::clone(interceptor))
            .await;
    }

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
    effect_adapter.set_engine_store(store.clone()).await;
    if let Some(skill_registry) = agent.deps.skill_registry.clone() {
        effect_adapter.set_skill_registry(skill_registry).await;
    }

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
                description: "Create a new mission (routine). Use only when the user explicitly wants to set up a recurring task, scheduled check, automation, monitor, or persistent manual mission. Do not use for immediate one-shot requests like 'do it now', 'right now', or 'immediately'; complete those in the current thread. Results are delivered to the current channel by default.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Short name for the mission/routine"},
                        "goal": {"type": "string", "description": "What this mission should accomplish each run"},
                        "cadence": {"type": "string", "description": "Required. How to trigger: 'manual', a cron expression (e.g. '0 9 * * *'), 'event:<channel>:<regex_pattern>' (e.g. 'event:telegram:.*', use 'event:*:<pattern>' for any channel), or 'webhook:<path>'"},
                        "timezone": {"type": "string", "description": "IANA timezone for cron scheduling (e.g. 'America/New_York'). Defaults to the user's channel timezone."},
                        "notify_channels": {"type": "array", "items": {"type": "string"}, "description": "Channels to deliver results to (e.g. ['gateway', 'repl']). Defaults to current channel."},
                        "project_id": {"type": "string", "description": "Project ID to scope this mission to. If omitted, uses the current thread's project."},
                        "cooldown_secs": {"type": "integer", "minimum": 0, "description": "Minimum seconds between triggers (default: 300 for event/webhook, 0 for cron/manual)"},
                        "max_concurrent": {"type": "integer", "minimum": 0, "description": "Max simultaneous running threads (default: 1 for event/webhook, unlimited for cron/manual)"},
                        "dedup_window_secs": {"type": "integer", "minimum": 0, "description": "Suppress duplicate event triggers within this window in seconds (default: 0)"},
                        "max_threads_per_day": {"type": "integer", "minimum": 0, "description": "Daily thread budget (default: 24 for event/webhook, 10 for cron/manual)"},
                        "success_criteria": {"type": "string", "description": "Criteria for declaring mission complete"}
                    },
                    "required": ["name", "goal", "cadence"]
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
                name: "mission_get".into(),
                description: "Get detailed status and results of a specific mission or routine. Returns the mission state, approach history, and recent thread outputs. Use when the user asks about mission results, outcome, or progress.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Mission/routine ID to retrieve"}
                    },
                    "required": ["id"]
                }),
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
                description: "Update a mission/routine. Change name, goal, cadence, guardrails, notification channels, daily budget, or success criteria. Only provided fields are changed.".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Mission/routine ID to update"},
                        "name": {"type": "string", "description": "New name"},
                        "goal": {"type": "string", "description": "New goal"},
                        "cadence": {"type": "string", "description": "New cadence: 'manual', cron expression (e.g. '0 9 * * *'), 'event:<channel>:<regex_pattern>' (e.g. 'event:telegram:.*', use 'event:*:<pattern>' for any channel), or 'webhook:<path>'"},
                        "timezone": {"type": "string", "description": "IANA timezone for cron scheduling (e.g. 'America/New_York'). Defaults to the user's channel timezone."},
                        "notify_channels": {"type": "array", "items": {"type": "string"}, "description": "Channels to deliver results to (e.g. ['gateway', 'repl'])"},
                        "max_threads_per_day": {"type": "integer", "minimum": 0, "description": "Max threads per day (0 = unlimited)"},
                        "cooldown_secs": {"type": "integer", "minimum": 0, "description": "Minimum seconds between triggers"},
                        "max_concurrent": {"type": "integer", "minimum": 0, "description": "Max simultaneous running threads"},
                        "dedup_window_secs": {"type": "integer", "minimum": 0, "description": "Suppress duplicate event triggers within this window in seconds"},
                        "success_criteria": {"type": "string", "description": "Criteria for declaring mission complete"}
                    },
                    "required": ["id"]
                }),
                effects: vec![],
                requires_approval: false,
            },
            ironclaw_engine::ActionDef {
                name: "mission_complete".into(),
                description: "Mark a mission or routine as completed (sets status to completed).".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Mission/routine ID to mark completed"}
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

    // Share the registry with the effect adapter so its `available_actions`
    // can advertise engine-native capability actions (missions) to the LLM.
    // Without this, mission tools have active leases but never appear in
    // the tools list sent with each LLM call.
    let capabilities = Arc::new(capabilities);
    effect_adapter
        .set_capability_registry(Arc::clone(&capabilities))
        .await;

    let thread_manager = Arc::new(ThreadManager::new(
        llm_adapter,
        effect_adapter.clone(),
        store_dyn.clone(),
        capabilities,
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
        MissionManager::new(store_dyn.clone(), Arc::clone(&thread_manager))
            .with_effect_executor(effect_adapter.clone());
    if let Some(workspace) = agent.workspace().cloned() {
        let reader: Arc<dyn ironclaw_engine::WorkspaceReader> =
            Arc::new(crate::bridge::WorkspaceReaderAdapter::new(workspace));
        mission_manager_inner = mission_manager_inner.with_workspace_reader(reader);
    }
    let cost_guard = Arc::clone(&agent.deps.cost_guard);
    let budget_gate: Arc<dyn ironclaw_engine::BudgetGate> =
        Arc::new(crate::bridge::CostGuardBudgetGate::new(cost_guard));
    mission_manager_inner = mission_manager_inner.with_budget_gate(budget_gate);
    // Use the DB-first config system instead of raw std::env::var reads.
    // Resolve MissionsConfig from DB-backed settings when available, falling
    // back to local settings.json + env vars.
    let missions_settings = if let Some(ref store) = agent.deps.store {
        match store.get_all_settings(&agent.deps.owner_id).await {
            Ok(map) => crate::settings::Settings::from_db_map(&map),
            Err(_) => crate::settings::Settings::load(),
        }
    } else {
        crate::settings::Settings::load()
    };
    let missions_config = match crate::config::MissionsConfig::resolve(&missions_settings) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "MissionsConfig::resolve failed; falling back to defaults"
            );
            crate::config::MissionsConfig::default()
        }
    };
    mission_manager_inner =
        mission_manager_inner.with_insights_interval(missions_config.insights_interval);
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
                owner_id,
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

    // Install the per-project workspace mount table on the effect adapter.
    //
    // Two factories share the same `ProjectPathResolver`: a default
    // [`FilesystemMountFactory`] that points `/project/` at the host
    // workspace directory, and a [`ContainerizedMountFactory`] (gated on
    // `SANDBOX_ENABLED=true`) that routes the same prefix into a
    // per-project sandbox container via `ProjectSandboxManager`. The
    // bridge interceptor does not care which factory is in play — Phase 1
    // already routes any `/project/...` tool call through whichever
    // backend the mount table returns.
    {
        use crate::bridge::sandbox::{
            ContainerizedMountFactory, FilesystemMountFactory, ProjectPathResolver,
            ProjectSandboxManager, ensure_project_workspace_dir,
        };
        use ironclaw_engine::{MountError, ProjectMountFactory, WorkspaceMounts};

        let store_for_resolver = store_dyn.clone();
        let resolver: ProjectPathResolver = Arc::new(move |pid| {
            let store_for_resolver = store_for_resolver.clone();
            Box::pin(async move {
                match store_for_resolver.load_project(pid).await {
                    Ok(Some(project)) => {
                        ensure_project_workspace_dir(&project).map_err(|e| MountError::Backend {
                            reason: format!("ensure_project_workspace_dir({pid}): {e}"),
                        })
                    }
                    Ok(None) => Err(MountError::Backend {
                        reason: format!("project {pid} not found"),
                    }),
                    Err(e) => Err(MountError::Backend {
                        reason: format!("store load_project({pid}): {e}"),
                    }),
                }
            })
        });

        let factory: Arc<dyn ProjectMountFactory> =
            if crate::bridge::sandbox::engine_v2_sandbox_enabled() {
                match crate::sandbox::container::connect_docker().await {
                    Ok(docker) => {
                        debug!(
                            "engine v2: SANDBOX_ENABLED=true — using containerized mount factory"
                        );
                        let manager = Arc::new(ProjectSandboxManager::new(docker));
                        Arc::new(ContainerizedMountFactory::new(manager, resolver))
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "engine v2: SANDBOX_ENABLED=true but Docker is not reachable; \
                             falling back to host filesystem mount factory"
                        );
                        Arc::new(FilesystemMountFactory::new(resolver))
                    }
                }
            } else {
                Arc::new(FilesystemMountFactory::new(resolver))
            };
        let mounts = Arc::new(WorkspaceMounts::new(factory));
        effect_adapter.set_workspace_mounts(Some(mounts)).await;
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
        extension_manager: agent.deps.extension_manager.clone(),
        project_root: resolve_project_root(),
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
                gate.scope_thread_id.as_ref().map(|t| t.as_str()) == Some(hint)
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

/// Check whether the user has *any* pending gate (resolved, ambiguous, or
/// otherwise). Unlike `get_engine_pending_gate` which returns `None` for
/// ambiguous resolutions, this returns `true` whenever at least one gate
/// exists — suitable for deciding whether a bare keyword should be treated
/// as an approval response vs. regular user input.
pub async fn has_any_pending_gate(user_id: &str, thread_id: Option<&str>) -> bool {
    let Some(lock) = ENGINE_STATE.get() else {
        return false;
    };
    let Ok(guard) = lock.try_read() else {
        return false;
    };
    let Some(state) = guard.as_ref() else {
        return false;
    };
    !matches!(
        resolve_pending_gate_for_user(&state.pending_gates, user_id, thread_id).await,
        PendingGateResolution::None
    )
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
            // Consume the gate so a duplicate OAuth callback cannot replay it.
            let key = pending.key();
            let _ = state.pending_gates.discard(&key).await;
            return Ok(AuthCallbackContinuation::ReplayMessage {
                channel: pending.source_channel,
                thread_scope: pending.scope_thread_id.map(String::from),
                content,
            });
        }
        tracing::debug!(
            user_id = %user_id,
            credential_name = %credential_name,
            thread_id = %pending.thread_id,
            "OAuth callback matched authentication fallback without a replayable request"
        );
        return Ok(AuthCallbackContinuation::None);
    }

    Ok(AuthCallbackContinuation::ResolveGateExternal {
        channel: pending.source_channel,
        thread_scope: pending.scope_thread_id.map(String::from),
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
) -> Result<BridgeOutcome, Error> {
    init_engine(agent).await?;

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    // Scope explicit approval replies to the active gateway conversation when
    // available so `/approve` cannot resume an unrelated pending gate owned by
    // another thread, such as a background routine. Other channels still use
    // legacy thread IDs that do not map 1:1 to engine conversation scopes.
    let thread_scope = if message.channel == "gateway" {
        message.conversation_scope()
    } else {
        None
    };
    let pending = match resolve_pending_gate_for_user(
        &state.pending_gates,
        &message.user_id,
        thread_scope,
    )
    .await
    {
        PendingGateResolution::Resolved(p) => p,
        PendingGateResolution::None => {
            debug!(user_id = %message.user_id, "engine v2: no pending approval for user, ignoring");
            return Ok(BridgeOutcome::Respond(
                "No pending approval for this thread.".into(),
            ));
        }
        PendingGateResolution::Ambiguous => {
            return Ok(BridgeOutcome::Respond(
                "Multiple pending gates are waiting. Resolve from the original thread or retry with that thread selected.".into(),
            ));
        }
    };

    if !matches!(
        pending.resume_kind,
        ironclaw_engine::ResumeKind::Approval { .. }
    ) {
        return Ok(BridgeOutcome::Respond(
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
) -> Result<BridgeOutcome, Error> {
    init_engine(agent).await?;

    let resolution = if approved {
        ironclaw_engine::GateResolution::Approved { always }
    } else {
        ironclaw_engine::GateResolution::Denied { reason: None }
    };

    if let Some(thread_id) = hinted_pending_gate_thread_id(
        &message.user_id,
        message.conversation_scope(),
        request_id,
        gate_view_is_approval,
    )
    .await?
    {
        return resolve_gate(agent, message, thread_id, request_id, resolution).await;
    }

    if let Some(thread_id) =
        pending_gate_thread_id_for_request(&message.user_id, request_id, gate_is_approval).await?
    {
        return resolve_gate(agent, message, thread_id, request_id, resolution).await;
    }

    debug!(
        user_id = %message.user_id,
        request_id = %request_id,
        "engine v2: no matching pending approval for request_id"
    );
    Ok(BridgeOutcome::Respond(
        "No matching pending approval found.".into(),
    ))
}

pub async fn handle_external_callback(
    agent: &Agent,
    message: &IncomingMessage,
    request_id: uuid::Uuid,
) -> Result<BridgeOutcome, Error> {
    init_engine(agent).await?;

    let resolution = ironclaw_engine::GateResolution::ExternalCallback {
        payload: serde_json::Value::Null,
    };

    if let Some(thread_id) = hinted_pending_gate_thread_id(
        &message.user_id,
        message.conversation_scope(),
        request_id,
        gate_view_is_authentication,
    )
    .await?
    {
        return resolve_gate(agent, message, thread_id, request_id, resolution).await;
    }

    if let Some(thread_id) =
        pending_gate_thread_id_for_request(&message.user_id, request_id, gate_is_authentication)
            .await?
    {
        return resolve_gate(agent, message, thread_id, request_id, resolution).await;
    }

    debug!(
        user_id = %message.user_id,
        request_id = %request_id,
        "engine v2: no matching pending auth gate for external callback"
    );
    Ok(BridgeOutcome::Respond(
        "No matching pending authentication gate found.".into(),
    ))
}

pub async fn handle_auth_gate_resolution(
    agent: &Agent,
    message: &IncomingMessage,
    request_id: uuid::Uuid,
    resolution: crate::agent::submission::AuthGateResolution,
) -> Result<BridgeOutcome, Error> {
    init_engine(agent).await?;

    let gate_resolution = match resolution {
        crate::agent::submission::AuthGateResolution::CredentialProvided { token } => {
            ironclaw_engine::GateResolution::CredentialProvided { token }
        }
        crate::agent::submission::AuthGateResolution::Cancelled => {
            ironclaw_engine::GateResolution::Cancelled
        }
    };

    if let Some(thread_id) = hinted_pending_gate_thread_id(
        &message.user_id,
        message.conversation_scope(),
        request_id,
        gate_view_is_authentication,
    )
    .await?
    {
        return resolve_gate(agent, message, thread_id, request_id, gate_resolution).await;
    }

    if let Some(thread_id) =
        pending_gate_thread_id_for_request(&message.user_id, request_id, gate_is_authentication)
            .await?
    {
        return resolve_gate(agent, message, thread_id, request_id, gate_resolution).await;
    }

    debug!(
        user_id = %message.user_id,
        request_id = %request_id,
        "engine v2: no matching pending auth gate for request_id"
    );
    Ok(BridgeOutcome::Respond(
        "No matching pending authentication gate found.".into(),
    ))
}

fn gate_is_approval(gate: &PendingGate) -> bool {
    matches!(
        gate.resume_kind,
        ironclaw_engine::ResumeKind::Approval { .. }
    )
}

fn gate_is_authentication(gate: &PendingGate) -> bool {
    matches!(
        gate.resume_kind,
        ironclaw_engine::ResumeKind::Authentication { .. }
    )
}

fn gate_view_is_approval(gate: &crate::gate::pending::PendingGateView) -> bool {
    matches!(
        gate.resume_kind,
        ironclaw_engine::ResumeKind::Approval { .. }
    )
}

fn gate_view_is_authentication(gate: &crate::gate::pending::PendingGateView) -> bool {
    matches!(
        gate.resume_kind,
        ironclaw_engine::ResumeKind::Authentication { .. }
    )
}

async fn hinted_pending_gate_thread_id(
    user_id: &str,
    conversation_scope: Option<&str>,
    request_id: uuid::Uuid,
    predicate: fn(&crate::gate::pending::PendingGateView) -> bool,
) -> Result<Option<ironclaw_engine::ThreadId>, Error> {
    let Some(thread_id) = parse_engine_thread_id(conversation_scope) else {
        return Ok(None);
    };

    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    let gate = state
        .pending_gates
        .peek(&crate::gate::pending::PendingGateKey {
            user_id: user_id.to_string(),
            thread_id,
        })
        .await;
    drop(guard);

    Ok(gate
        .filter(|gate| gate.request_id == request_id.to_string() && predicate(gate))
        .map(|_| thread_id))
}

async fn pending_gate_thread_id_for_request(
    user_id: &str,
    request_id: uuid::Uuid,
    predicate: fn(&PendingGate) -> bool,
) -> Result<Option<ironclaw_engine::ThreadId>, Error> {
    let lock = ENGINE_STATE
        .get()
        .ok_or_else(|| engine_err("init", "engine state not initialized"))?;
    let guard = lock.read().await;
    let state = guard
        .as_ref()
        .ok_or_else(|| engine_err("init", "engine state is empty"))?;

    let pending = state
        .pending_gates
        .list_for_user(user_id)
        .await
        .into_iter()
        .find(|gate| gate.request_id == request_id && predicate(gate))
        .map(|gate| gate.thread_id);
    drop(guard);
    Ok(pending)
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
) -> Result<BridgeOutcome, Error> {
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
            // Clamp the caller-supplied `always` flag to what the pending gate
            // actually permits. A protected `memory_write` (orchestrator code,
            // prompt overlays) advertises `Approval { allow_always: false }`
            // so the UI hides the "always approve" button — but the HTTP
            // approval endpoint accepts arbitrary JSON, so a caller could
            // still submit `always: true` and silently install a session-wide
            // auto-approval that bypasses every subsequent gate. The gate's
            // own `allow_always` is the authoritative server-side policy.
            let always = clamp_always_to_resume_kind(always, &pending.resume_kind);

            // Pre-flight thread check before committing `AlwaysAllow`
            // persistence (#2347): if the thread was deleted between
            // `take_verified` and now, persisting auto-approve would leave
            // a permanent preference behind for a tool that never ran. The
            // rollback at the bottom of this branch only fires on `Err`, so
            // execute_pending_gate_action's graceful `Ok(Respond)` on
            // missing-thread would bypass it. Short-circuit here instead.
            match state.store.load_thread(pending.thread_id).await {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Ok(emit_gate_expired_dismissal(state, message, &pending));
                }
                Err(e) => return Err(engine_err("load thread", e)),
            }

            if pending.gate_name == SECRET_BINDING_APPROVAL_GATE_NAME
                && let Some(approval) =
                    secret_binding_approval_from_resume_output(pending.resume_output.as_ref())
            {
                let settings_store = agent
                    .deps
                    .settings_store
                    .as_deref()
                    .map(|store| store as &(dyn crate::db::SettingsStore + Send + Sync));
                grant_binding_approval(settings_store, &pending.user_id, approval)
                    .await
                    .map_err(|error| engine_err("binding approval", error))?;
            }
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
                        thread_id: Some(pending.effective_wire_thread_id()),
                    },
                );
            }
            let legacy_registry_name = legacy_extension_alias(&pending.action_name);
            let prior_permission = if always {
                state
                    .effect_adapter
                    .auto_approve_tool(&pending.action_name)
                    .await;
                if let Some(ref registry_name) = legacy_registry_name {
                    state.effect_adapter.auto_approve_tool(registry_name).await;
                }

                // Persist AlwaysAllow to DB so the preference survives process
                // restarts. Mirrors the v1 path in thread_ops.rs.
                persist_always_allow(agent, state, &pending).await
            } else {
                None
            };
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
                // Revert the DB persistence on execution failure, restoring
                // any pre-existing preference instead of blindly deleting.
                revert_always_allow(agent, &pending, prior_permission).await;
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
                        thread_id: Some(pending.effective_wire_thread_id()),
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
                        thread_id: Some(pending.effective_wire_thread_id()),
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
            return Ok(BridgeOutcome::Respond("Cancelled.".into()));
        }

        ironclaw_engine::GateResolution::CredentialProvided { token } => {
            // Store credential then RESUME (not retry) — preserves thread work
            if let ironclaw_engine::ResumeKind::Authentication {
                ref credential_name,
                ..
            } = pending.resume_kind
            {
                // `submit_auth_token` expects an *extension name* as
                // its first argument and uses `configure_token` to walk
                // the extension's capabilities file for the actual
                // secret name. The engine's `ResumeKind::Authentication`
                // only carries `credential_name`, which fails closed
                // when fed there for WASM-tool-backed credentials. See
                // `resolve_extension_for_action` for the full rationale.
                let submit_target = resolve_extension_for_action(
                    state.auth_manager.as_deref(),
                    state.extension_manager.as_deref(),
                    state.effect_adapter.tools(),
                    &pending.action_name,
                    &pending.parameters,
                    credential_name.as_str(),
                    &message.user_id,
                )
                .await;
                let display_name = submit_target.clone();

                if let Some(ref sse) = state.sse {
                    sse.broadcast_for_user(
                        &message.user_id,
                        AppEvent::GateResolved {
                            request_id: pending.request_id.to_string(),
                            gate_name: pending.gate_name.clone(),
                            tool_name: pending.action_name.clone(),
                            resolution: "credential_provided".into(),
                            message: "Credential received. Resuming execution.".into(),
                            thread_id: Some(pending.effective_wire_thread_id()),
                        },
                    );
                }
                match submit_pending_auth_credential(
                    state,
                    submit_target.as_str(),
                    credential_name.as_str(),
                    &token,
                    &message.user_id,
                )
                .await
                {
                    Ok(PendingAuthCredentialSubmission::Stored(result))
                        if matches!(
                            crate::channels::web::onboarding::classify_configure_result(&result),
                            crate::channels::web::onboarding::ConfigureFlowOutcome::Ready
                        ) =>
                    {
                        let _ = agent
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: display_name.clone(),
                                    success: true,
                                    message: format_auth_completed_resuming(&result.message),
                                },
                                &message.metadata,
                            )
                            .await;
                    }
                    Ok(PendingAuthCredentialSubmission::Stored(result)) => match crate::channels::web::onboarding::classify_configure_result(
                        &result,
                    ) {
                        crate::channels::web::onboarding::ConfigureFlowOutcome::PairingRequired {
                            instructions,
                            onboarding,
                        } => {
                            let next_pending =
                                requeue_pairing_pending_gate(state, &pending, display_name.as_str())
                                    .await?;
                            if let Some(ref sse) = state.sse {
                                sse.broadcast_for_user(
                                    &message.user_id,
                                    ironclaw_common::OnboardingStateDto::pairing_required(
                                        display_name.clone(),
                                        Some(next_pending.request_id.to_string()),
                                        Some(pending.effective_wire_thread_id()),
                                        Some(result.message.clone()),
                                        instructions,
                                        onboarding,
                                    ),
                                );
                            }
                            return Ok(BridgeOutcome::Pending);
                        }
                        crate::channels::web::onboarding::ConfigureFlowOutcome::AuthRequired
                        | crate::channels::web::onboarding::ConfigureFlowOutcome::RetryAuth => {
                            return requeue_auth_pending_gate(
                                agent,
                                state,
                                message,
                                &pending,
                                result.message,
                                result.auth_url,
                            )
                            .await;
                        }
                        crate::channels::web::onboarding::ConfigureFlowOutcome::Ready => {}
                    },
                    // Bare test-harness path: no backend exists, but the
                    // gate carries a staged `resume_output` (set when the
                    // gate was created with a synthetic output), so we can
                    // proceed with the resume below. The caller's token
                    // is intentionally dropped here — the resume_output
                    // already carries whatever the resumed action needs;
                    // production flows always come through the
                    // auth-manager or extension-manager branch above.
                    // `debug!` (not `info!` — would corrupt the REPL/TUI)
                    // so operators can still see the drop when tracing.
                    Ok(PendingAuthCredentialSubmission::SkippedNoBackend)
                        if pending.resume_output.is_some() =>
                    {
                        tracing::debug!(
                            user_id = %message.user_id,
                            thread_id = %pending.thread_id,
                            request_id = %pending.request_id,
                            "auth gate resume: no backend, token dropped because resume_output is staged",
                        );
                    }
                    Ok(PendingAuthCredentialSubmission::SkippedNoBackend) => {
                        let msg =
                            "No auth manager, extension manager, or secrets store available to store credential.".to_string();
                        fail_waiting_thread(state, &message.user_id, pending.thread_id, &msg)
                            .await?;
                        let _ = agent
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: display_name.clone(),
                                    success: false,
                                    message: msg.clone(),
                                },
                                &message.metadata,
                            )
                            .await;
                        return Ok(BridgeOutcome::Respond(msg));
                    }
                    Err(crate::extensions::ExtensionError::ValidationFailed(msg)) => {
                        return requeue_auth_pending_gate(
                            agent, state, message, &pending, msg, None,
                        )
                        .await;
                    }
                    Err(error) => {
                        let msg = error.to_string();
                        let _ = agent
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: display_name.clone(),
                                    success: false,
                                    message: msg.clone(),
                                },
                                &message.metadata,
                            )
                            .await;
                        return Ok(BridgeOutcome::Respond(msg));
                    }
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
                    let resolved_call_id =
                        resolved_or_synthetic_call_id_for_pending_action(state, &pending).await?;
                    state
                        .thread_manager
                        .resume_thread(
                            pending.thread_id,
                            message.user_id.clone(),
                            Some(resumed_action_result_message(
                                &resolved_call_id,
                                &pending.action_name,
                                &resume_output,
                            )),
                            None,
                            Some(resolved_call_id),
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
                        thread_id: Some(pending.effective_wire_thread_id()),
                    },
                );
            }
            if let Some(resume_output) = pending.resume_output.clone() {
                let resolved_call_id =
                    resolved_or_synthetic_call_id_for_pending_action(state, &pending).await?;
                state
                    .thread_manager
                    .resume_thread(
                        pending.thread_id,
                        message.user_id.clone(),
                        Some(resumed_action_result_message(
                            &resolved_call_id,
                            &pending.action_name,
                            &resume_output,
                        )),
                        None,
                        Some(resolved_call_id),
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
) -> Result<BridgeOutcome, Error> {
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
        Ok(BridgeOutcome::Respond("Interrupted.".into()))
    } else {
        Ok(BridgeOutcome::Respond("Nothing to interrupt.".into()))
    }
}

/// Handle a new-thread submission — clear conversation for a fresh start.
pub async fn handle_new_thread(
    agent: &Agent,
    message: &IncomingMessage,
) -> Result<BridgeOutcome, Error> {
    clear_engine_conversation(agent, message).await?;
    Ok(BridgeOutcome::Respond("Started new conversation.".into()))
}

/// Handle a clear submission — stop threads and reset conversation.
pub async fn handle_clear(
    agent: &Agent,
    message: &IncomingMessage,
) -> Result<BridgeOutcome, Error> {
    clear_engine_conversation(agent, message).await?;
    Ok(BridgeOutcome::Respond("Conversation cleared.".into()))
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
) -> Result<BridgeOutcome, Error> {
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
        return Ok(BridgeOutcome::Respond(
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

    // Ensure learning missions exist for this user before firing.
    // They may be missing if the user wasn't the owner at init_engine time
    // or if their missions failed to initialize.
    let mgr = state.effect_adapter.mission_manager().await;
    if let Some(ref mgr) = mgr {
        let user_project_id =
            resolve_user_project(&state.store, &message.user_id, state.default_project_id).await?;
        if let Err(e) = mgr
            .ensure_learning_missions(user_project_id, &message.user_id)
            .await
        {
            debug!(
                "failed to ensure learning missions for user {}: {e}",
                message.user_id
            );
        }
    }

    // Fire the expected-behavior learning mission
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
        Ok(BridgeOutcome::Respond(format!(
            "Feedback captured. Fired {fired} self-improvement thread(s) to investigate."
        )))
    } else {
        Ok(BridgeOutcome::Respond(
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
            // Discard all pending gates for this thread regardless of user,
            // preventing orphaned gates that can never be resolved (#2323).
            state.pending_gates.discard_for_thread(*tid).await;
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

/// Clear pending auth state for a user in the v2 engine.
///
/// Called from gateway-side auth cleanup paths to ensure pending
/// authentication gates are cleared when the browser abandons a prompt or an
/// OAuth callback completes outside the normal chat message path.
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

pub async fn discard_engine_pending_auth_request(
    user_id: &str,
    request_id: uuid::Uuid,
    thread_id: Option<&str>,
) -> bool {
    let Some(lock) = ENGINE_STATE.get() else {
        return false;
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return false;
    };

    let hinted_uuid = parse_scope_uuid(thread_id);
    let hinted_scope = thread_id;
    let matching_gate = state
        .pending_gates
        .list_for_user(user_id)
        .await
        .into_iter()
        .find(|gate| {
            gate.request_id == request_id
                && hinted_scope.is_none_or(|hint| {
                    gate.scope_thread_id.as_ref().map(|t| t.as_str()) == Some(hint)
                        || hinted_uuid.is_none_or(|uuid| {
                            gate.thread_id.0 == uuid || gate.conversation_id.0 == uuid
                        })
                })
                && matches!(
                    gate.resume_kind,
                    ironclaw_engine::ResumeKind::Authentication { .. }
                )
        });

    let Some(gate) = matching_gate else {
        return false;
    };

    state.pending_gates.discard(&gate.key()).await.is_ok()
}

pub async fn transition_engine_pending_auth_request_to_pairing(
    user_id: &str,
    request_id: uuid::Uuid,
    thread_id: Option<&str>,
    extension_name: &str,
) -> Result<Option<String>, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(None);
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(None);
    };

    let hinted_uuid = parse_scope_uuid(thread_id);
    let hinted_scope = thread_id;
    let matching_gate = state
        .pending_gates
        .list_for_user(user_id)
        .await
        .into_iter()
        .find(|gate| {
            gate.request_id == request_id
                && hinted_scope.is_none_or(|hint| {
                    gate.scope_thread_id.as_ref().map(|t| t.as_str()) == Some(hint)
                        || hinted_uuid.is_none_or(|uuid| {
                            gate.thread_id.0 == uuid || gate.conversation_id.0 == uuid
                        })
                })
                && matches!(
                    gate.resume_kind,
                    ironclaw_engine::ResumeKind::Authentication { .. }
                )
        });

    let Some(gate) = matching_gate else {
        return Ok(None);
    };

    state
        .pending_gates
        .discard(&gate.key())
        .await
        .map_err(|e| engine_err("pending auth gate discard", e))?;

    let next_pending = pairing_pending_gate_from_auth(&gate, extension_name);
    state
        .pending_gates
        .insert(next_pending.clone())
        .await
        .map_err(|e| engine_err("pending pairing gate insert", e))?;

    Ok(Some(next_pending.request_id.to_string()))
}

/// Handle a user message through the engine v2 pipeline.
pub async fn handle_with_engine(
    agent: &Agent,
    message: &IncomingMessage,
    content: &str,
) -> Result<BridgeOutcome, Error> {
    handle_with_engine_inner(agent, message, content, 0).await
}

/// Maximum depth for auth-retry recursion (credential stored → retry original message).
const MAX_AUTH_RETRY_DEPTH: u8 = 2;

async fn handle_with_engine_inner(
    agent: &Agent,
    message: &IncomingMessage,
    content: &str,
    depth: u8,
) -> Result<BridgeOutcome, Error> {
    if depth > MAX_AUTH_RETRY_DEPTH {
        return Ok(BridgeOutcome::Respond(
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
            // Clone the SSE arc and the tools registry out of state,
            // then drop the engine read guard before awaiting on
            // broadcast + channel I/O. The auth branch above does the
            // same, and `notify_pending_gate` is signed to accept an
            // owned Option<Arc<SseManager>> precisely so this
            // terminal-return branch can release the lock. The tools
            // registry handle is needed by `notify_pending_gate` to
            // resolve the auth-gate display name without holding the
            // engine state lock.
            let sse = state.sse.clone();
            let tools = Arc::clone(state.effect_adapter.tools());
            let auth_manager = state.auth_manager.clone();
            let extension_manager = state.extension_manager.clone();
            drop(guard);
            return notify_pending_gate(
                agent,
                sse,
                tools.as_ref(),
                auth_manager.as_deref(),
                extension_manager.as_deref(),
                message,
                &pending,
            )
            .await;
        }
        PendingGateResolution::Ambiguous => {
            return Ok(BridgeOutcome::Respond(
                "Multiple pending approval or authentication prompts are waiting. Reply from the original thread.".into(),
            ));
        }
        PendingGateResolution::Resolved(_) | PendingGateResolution::None => {}
    }

    if let Some(thread_id) = scoped_thread_id
        && fail_orphaned_waiting_thread_if_needed(state, &message.user_id, thread_id).await?
    {
        return Ok(BridgeOutcome::Respond(
            "This thread was waiting on approval or authentication, but that pending state was lost. The thread has been marked failed; resend your request.".into(),
        ));
    }

    // Safety checks — mirror the v1 pipeline in thread_ops::process_user_input
    // so both engine paths enforce the same inbound protections. When the
    // message carries attachments, an empty text body is legitimate (the
    // attachment is the payload); skip the validator's empty-input rejection
    // but still apply length / policy checks against the text.
    let trimmed_content = content.trim();
    let skip_empty_check = trimmed_content.is_empty() && !message.attachments.is_empty();
    if !skip_empty_check {
        let validation = agent.safety().validate_input(content);
        if !validation.is_valid {
            let details = validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Ok(BridgeOutcome::Respond(format!(
                "Input rejected by safety validation: {details}"
            )));
        }
    }

    let violations = agent.safety().check_policy(content);
    if violations
        .iter()
        .any(|rule| rule.action == ironclaw_safety::PolicyAction::Block)
    {
        return Ok(BridgeOutcome::Respond(
            "Input rejected by safety policy.".into(),
        ));
    }

    // Scan inbound messages for secrets (API keys, tokens).
    // Catching them here prevents the LLM from echoing them back, which
    // would trigger the outbound leak detector and create error loops.
    if let Some(warning) = agent.safety().scan_inbound_for_secrets(content) {
        tracing::warn!(
            user_id = %message.user_id,
            channel = %message.channel,
            "engine v2: inbound message blocked — contains leaked secret"
        );
        return Ok(BridgeOutcome::Respond(warning));
    }

    // Resolve per-user project (creates if needed).
    let project_id =
        resolve_user_project(&state.store, &message.user_id, state.default_project_id).await?;

    let mut persisted_attachments = message.attachments.clone();
    let attachment_notes = persist_project_attachments(
        &state.project_root,
        message,
        project_id,
        &mut persisted_attachments,
    )
    .await;

    // Engine v2 threads are text-only today, so attachments must be folded
    // into the effective user content before routing to the engine. This
    // preserves extracted document text, project-local file paths, and
    // attachment metadata in both the engine thread and the dual-written
    // gateway history.
    let augmented = crate::agent::augment_with_attachments(content, &persisted_attachments);
    let effective_content = augmented
        .as_ref()
        .map(|result| result.text.as_str())
        .unwrap_or(content);

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
    fire_event_missions_for_message(state, message, effective_content).await;

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

    // Validate the channel-supplied timezone before passing it to the engine.
    // ValidTimezone::parse rejects empty/invalid strings; we send the canonical
    // IANA name (not the raw input) so downstream consumers see a known-good
    // value. Must be passed *into* spawn — setting metadata after the thread
    // starts is invisible to the in-memory executor on the first turn.
    let validated_tz = message
        .timezone
        .as_deref()
        .and_then(ironclaw_engine::ValidTimezone::parse);

    // Detect execution intent and configure obligation accordingly
    let thread_config = {
        let mut cfg = ThreadConfig::default();
        if crate::llm::user_signals_execution_intent(content) {
            cfg.require_action_attempt = true;
        }
        cfg
    };

    // Handle the message — spawns a new thread or injects into active one
    let thread_id = state
        .conversation_manager
        .handle_user_message(
            conv_id,
            effective_content,
            project_id,
            &message.user_id,
            thread_config,
            validated_tz.as_ref().map(|tz| tz.name()),
        )
        .await
        .map_err(|e| engine_err("thread error", e))?;

    if !attachment_notes.is_empty() {
        save_attachment_index_notes(
            &state.store,
            project_id,
            &message.user_id,
            thread_id,
            attachment_notes,
        )
        .await;
    }

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
            let _ = db
                .add_conversation_message(cid, "user", effective_content)
                .await;
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
) -> Result<BridgeOutcome, Error> {
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

                // Extract credential name. Try structured JSON first (which
                // is what the http tool actually emits), then fall back to a
                // string splitter for prose-shaped errors.
                let parsed_cred_name = parse_credential_name(text);

                // Defense against credential-name injection: only honor a
                // fallback auth gate when the parsed name is actually a
                // registered credential. A tool that fabricates an
                // `authentication_required` message with a chosen credential
                // name must not be able to coerce the user into providing an
                // unrelated secret. Without a credential registry there is
                // no way to validate the name, so the gate must not fire —
                // test/embed harnesses without a registry intentionally lose
                // the fallback path rather than gain a prompt-injection vector.
                let Some(cred_name) = parsed_cred_name.filter(|name| {
                    agent
                        .tools()
                        .credential_registry()
                        .is_some_and(|reg| reg.has_secret(name))
                }) else {
                    tracing::warn!(
                        thread_id = %thread_id,
                        "text-based auth fallback rejected unknown or missing credential name from tool output"
                    );
                    // Hand the original response back without inserting a gate.
                    return Ok(BridgeOutcome::Respond(text.clone()));
                };

                // Look up setup instructions via AuthManager (or fall back to inline lookup)
                let setup_hint = state
                    .auth_manager
                    .as_ref()
                    .and_then(|mgr| mgr.get_setup_instructions(&cred_name))
                    .unwrap_or_else(|| {
                        crate::bridge::auth_manager::default_secret_setup_instructions(&cred_name)
                    });

                let pending = PendingGate {
                    request_id: uuid::Uuid::new_v4(),
                    gate_name: "authentication".into(),
                    user_id: message.user_id.clone(),
                    thread_id,
                    scope_thread_id: message.conversation_scope().and_then(|s| {
                        match ironclaw_common::ExternalThreadId::new(s) {
                            Ok(tid) => Some(tid),
                            Err(e) => {
                                tracing::debug!(
                                    candidate = %s,
                                    error = %e,
                                    "router: invalid conversation_scope_id from IncomingMessage; storing None in pending gate"
                                );
                                None
                            }
                        }
                    }),
                    conversation_id: conv_id,
                    source_channel: message.channel.clone(),
                    action_name: "authentication_fallback".into(),
                    call_id: format!("fallback-auth-{thread_id}"),
                    parameters: serde_json::json!({ "credential_name": cred_name }),
                    display_parameters: None,
                    description: format!("Authentication required for '{}'.", cred_name),
                    resume_kind: ironclaw_engine::ResumeKind::Authentication {
                        credential_name: ironclaw_common::CredentialName::from_trusted(
                            cred_name.clone(),
                        ),
                        instructions: setup_hint.clone(),
                        auth_url: None,
                    },
                    created_at: chrono::Utc::now(),
                    expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
                    original_message: Some(message.content.clone()),
                    resume_output: None,
                    paused_lease: None,
                    approval_already_granted: false,
                };
                let pending_request_id = pending.request_id.to_string();
                if let Err(e) = state.pending_gates.insert(pending).await {
                    tracing::debug!(error = %e, "failed to store fallback auth gate");
                }

                // Show auth prompt via channel (card only, no text).
                let _ = agent
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthRequired {
                            extension_name: ironclaw_common::ExtensionName::from_trusted(
                                cred_name.clone(),
                            ),
                            instructions: Some(setup_hint.clone()),
                            auth_url: None,
                            setup_url: None,
                            request_id: Some(pending_request_id),
                        },
                        &message.metadata,
                    )
                    .await;

                return Ok(BridgeOutcome::Pending);
            }

            // Persist tool_calls only for completed threads — not for
            // GatePaused (partial tools, would orphan rows on resume).
            if let Some(ref db) = state.db {
                persist_v2_tool_calls(&state.store, db, thread_id, message).await;
            }

            match response {
                Some(text) => Ok(BridgeOutcome::Respond(text)),
                None => Ok(BridgeOutcome::NoResponse),
            }
        }
        ThreadOutcome::Stopped => Ok(BridgeOutcome::Respond("Thread was stopped.".into())),
        ThreadOutcome::MaxIterations => Ok(BridgeOutcome::Respond(
            "Reached maximum iterations without completing.".into(),
        )),
        ThreadOutcome::Failed { error } => Ok(BridgeOutcome::Respond(format!("Error: {error}"))),
        ThreadOutcome::GatePaused {
            gate_name,
            action_name,
            call_id,
            parameters,
            resume_kind,
            resume_output,
            paused_lease,
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
                scope_thread_id: message.conversation_scope().and_then(|s| {
                    match ironclaw_common::ExternalThreadId::new(s) {
                        Ok(tid) => Some(tid),
                        Err(e) => {
                            tracing::debug!(
                                candidate = %s,
                                error = %e,
                                "router: invalid conversation_scope_id from IncomingMessage; storing None in pending gate"
                            );
                            None
                        }
                    }
                }),
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
                // Unbox: `ThreadOutcome::GatePaused.paused_lease` is
                // `Option<Box<CapabilityLease>>` to keep the outcome
                // enum compact; `PendingGate` stores it unboxed.
                paused_lease: paused_lease.map(|b| *b),
                approval_already_granted: false,
            };

            if let Err(e) = state.pending_gates.insert(pending.clone()).await {
                tracing::debug!(
                    gate = %gate_name,
                    error = %e,
                    "failed to store pending gate (may be duplicate)"
                );
            }

            // Send the approval/auth card via the source channel. Each
            // channel renders this natively (web → SSE card, TUI → widget,
            // relay → buttons). No text response is returned — the caller
            // (agent_loop) detects the pending gate and maps to
            // HandleOutcome::Pending.
            {
                let extension_name = resolve_auth_gate_extension_name(
                    state.auth_manager.as_deref(),
                    state.extension_manager.as_deref(),
                    state.effect_adapter.tools(),
                    &pending,
                )
                .await;
                send_pending_gate_status(agent, message, &pending, extension_name.as_ref()).await;
            }
            Ok(BridgeOutcome::Pending)
        }
    };

    // Write the response to the v1 DB for all outcomes so the history
    // endpoint shows the correct state (not just for Completed).
    if let Ok(BridgeOutcome::Respond(ref text)) = result
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
// pub(crate) for #[cfg(test)] re-export in mod.rs; the module itself
// is private so this has no production visibility beyond router.rs.
pub(crate) async fn handle_mission_notification(
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
        let mut response = OutgoingResponse::text(&full_text);
        // Only attach the mission owner's thread_id when the recipient IS the
        // owner. When notify_user routes to a different user, omit the thread
        // so the gateway's broadcast() fallback resolves the recipient's own
        // assistant thread — avoids leaking the owner's thread_id cross-user.
        if broadcast_user == notif.user_id {
            response = response.in_thread(notif.thread_id.to_string());
        }
        if let Err(e) = channels
            .broadcast(channel_name, broadcast_user, response)
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

/// Persist v2 engine tool call metadata to the v1 conversation DB.
///
/// Loads the completed thread from the v2 store, extracts ActionResult
/// messages (which carry the actual tool output), and writes a
/// `role="tool_calls"` message so the chat history API can reconstruct
/// tool call info (name, result preview, errors) for the web UI.
async fn persist_v2_tool_calls(
    store: &std::sync::Arc<dyn Store>,
    db: &std::sync::Arc<dyn Database>,
    thread_id: ironclaw_engine::ThreadId,
    message: &IncomingMessage,
) {
    // Load the thread -- it's still in the store after join_thread
    // (join only removes from the runtime running map, not the store).
    //
    // Logging level: failures here mean the chat history will be missing
    // the tool_calls row for this turn (web UI shows empty array). That's
    // a user-visible gap, so emit at `warn!` — this path is an HTTP
    // handler, not a TUI-corrupting background task, so CLAUDE.md's
    // "background tasks must not use info!/warn!" rule does not apply.
    let thread = match store.load_thread(thread_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            tracing::warn!(thread_id = %thread_id, "thread not found in store for tool_calls persist");
            return;
        }
        Err(e) => {
            tracing::warn!(thread_id = %thread_id, "failed to load thread for tool_calls persist: {e}");
            return;
        }
    };

    // Extract ActionResult messages from the thread's internal transcript.
    // `internal_messages` has the full execution chain including action
    // results with actual tool output. `messages` only has user/assistant.
    let mut calls = Vec::new();
    for msg in &thread.internal_messages {
        if msg.role != ironclaw_engine::MessageRole::ActionResult {
            continue;
        }
        let action_name = msg.action_name.as_deref().unwrap_or("unknown");
        let preview = if msg.content.len() > 500 {
            let end = msg
                .content
                .char_indices()
                .take_while(|(i, _)| *i < 500)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            format!("{}...", &msg.content[..end]) // safety: end is char-boundary via char_indices
        } else {
            msg.content.clone()
        };
        let mut obj = serde_json::json!({
            "name": action_name,
            "result_preview": preview,
        });
        if let Some(ref call_id) = msg.action_call_id {
            obj["tool_call_id"] = serde_json::Value::String(call_id.clone());
        }
        calls.push(obj);
    }

    if calls.is_empty() {
        return;
    }

    let wrapper = serde_json::json!({ "calls": calls });
    let content = match serde_json::to_string(&wrapper) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(thread_id = %thread_id, "failed to serialize v2 tool_calls: {e}");
            return;
        }
    };

    // Resolve the v1 conversation ID
    let v1_conv_id = if let Some(tid) = message.conversation_scope()
        && let Ok(uuid) = uuid::Uuid::parse_str(tid)
    {
        Some(uuid)
    } else {
        match db
            .get_or_create_assistant_conversation(&message.user_id, &message.channel)
            .await
        {
            Ok(cid) => Some(cid),
            Err(e) => {
                tracing::warn!(
                    thread_id = %thread_id,
                    "failed to resolve v1 conversation for tool_calls persist: {e}"
                );
                return;
            }
        }
    };
    if let Some(cid) = v1_conv_id
        && let Err(e) = db
            .add_conversation_message(cid, "tool_calls", &content)
            .await
    {
        tracing::warn!(thread_id = %thread_id, "failed to persist v2 tool_calls to v1 DB: {e}");
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
            call_id,
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
                        detail: params_summary.clone(),
                        call_id: Some(call_id.clone()),
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
                        parameters: None,
                        call_id: Some(call_id.clone()),
                        duration_ms: Some(*duration_ms),
                    },
                    metadata,
                )
                .await;
        }
        EventKind::ActionFailed {
            action_name,
            call_id,
            error,
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
                        detail: params_summary.clone(),
                        call_id: Some(call_id.clone()),
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
                        call_id: Some(call_id.clone()),
                        duration_ms: Some(*duration_ms),
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
                            extension_name: ironclaw_common::ExtensionName::from_trusted(cred_name),
                            instructions: Some(
                                "Store the credential with: ironclaw secret set <name> <value>"
                                    .into(),
                            ),
                            auth_url: None,
                            setup_url: None,
                            request_id: None,
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
            // The v2 engine event doesn't carry feedback notes yet — those
            // would need to be produced by the Python orchestrator and
            // threaded through `EventKind::SkillActivated`. The v1 dispatcher
            // emits its own `StatusUpdate::SkillActivated` directly with
            // populated feedback (see `agent::dispatcher`).
            let _ = channels
                .send_status(
                    channel_name,
                    StatusUpdate::SkillActivated {
                        skill_names: skill_names.clone(),
                        feedback: Vec::new(),
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
            call_id,
            duration_ms,
            params_summary,
            ..
        } => {
            let display_name = format_action_display_name(action_name, params_summary);
            vec![
                AppEvent::ToolStarted {
                    name: display_name.clone(),
                    detail: params_summary.clone(),
                    call_id: Some(call_id.clone()),
                    thread_id: Some(thread_id.into()),
                },
                AppEvent::ToolCompleted {
                    name: display_name,
                    success: true,
                    error: None,
                    parameters: None,
                    call_id: Some(call_id.clone()),
                    duration_ms: Some(*duration_ms),
                    thread_id: Some(thread_id.into()),
                },
            ]
        }
        EventKind::ActionFailed {
            action_name,
            call_id,
            error,
            duration_ms,
            params_summary,
            ..
        } => {
            let display_name = format_action_display_name(action_name, params_summary);
            vec![
                AppEvent::ToolStarted {
                    name: display_name.clone(),
                    detail: params_summary.clone(),
                    call_id: Some(call_id.clone()),
                    thread_id: Some(thread_id.into()),
                },
                AppEvent::ToolCompleted {
                    name: display_name,
                    success: false,
                    error: Some(error.clone()),
                    parameters: None,
                    call_id: Some(call_id.clone()),
                    duration_ms: Some(*duration_ms),
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
            feedback: Vec::new(),
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<ironclaw_engine::ProjectMetric>,
    pub created_at: String,
}

/// Attention item surfaced in the projects overview.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AttentionItem {
    /// `"gate"` or `"failure"`
    #[serde(rename = "type")]
    pub kind: String,
    pub project_id: String,
    pub project_name: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// Per-project summary with computed health and stats.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectOverviewEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<String>,
    /// `"green"`, `"yellow"`, or `"red"`.
    pub health: String,
    pub active_missions: u64,
    pub total_missions: u64,
    pub threads_today: u64,
    pub cost_today_usd: f64,
    pub failures_24h: u64,
    pub pending_gates: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
    pub created_at: String,
}

/// Full projects overview response.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectsOverviewResponse {
    pub attention: Vec<AttentionItem>,
    pub projects: Vec<ProjectOverviewEntry>,
}

/// Mission summary for list views.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineMissionInfo {
    /// Typed mission identifier, carried through from the engine rather
    /// than round-tripped to `String` at the adapter boundary. Serializes
    /// transparently as a UUID string (via `MissionId`'s derived
    /// `Serialize`), so the wire shape stays identical to the pre-newtype
    /// DTO.
    pub id: ironclaw_engine::MissionId,
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
            goals: p.goals.clone(),
            metrics: p.metrics.clone(),
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
            goals: p.goals,
            metrics: p.metrics,
            created_at: p.created_at.to_rfc3339(),
        }))
}

/// Projects overview — health, stats, attention items for all projects.
///
/// Iterates all projects, computes per-project stats from missions and threads,
/// and collects pending gates as attention items. Designed for the control room
/// dashboard where the user checks in on a highly autonomous agent.
pub async fn get_engine_projects_overview(
    user_id: &str,
) -> Result<ProjectsOverviewResponse, Error> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Ok(ProjectsOverviewResponse {
            attention: vec![],
            projects: vec![],
        });
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Ok(ProjectsOverviewResponse {
            attention: vec![],
            projects: vec![],
        });
    };

    // Clone Arcs to release the lock before I/O.
    let store = state.store.clone();
    let pending_gates = state.pending_gates.clone();
    drop(guard);

    let projects = store
        .list_projects(user_id)
        .await
        .map_err(|e| engine_err("list projects", e))?;

    let now = chrono::Utc::now();
    let today_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc))
        .unwrap_or(now);
    let h24_ago = now - chrono::Duration::hours(24);

    // Collect all user gates once (keyed by thread_id later).
    let user_gates = pending_gates.list_for_user(user_id).await;

    // Fetch threads and missions for all projects concurrently.
    let project_data: Vec<_> = futures::future::try_join_all(projects.iter().map(|project| {
        let store = store.clone();
        let user_id = user_id.to_string();
        async move {
            let pid = project.id;
            let (threads, missions) = tokio::try_join!(
                async {
                    store
                        .list_threads(pid, &user_id)
                        .await
                        .map_err(|e| engine_err("list project threads", e))
                },
                async {
                    store
                        .list_missions_with_shared(pid, &user_id)
                        .await
                        .map_err(|e| engine_err("list project missions", e))
                },
            )?;
            Ok::<_, Error>((threads, missions))
        }
    }))
    .await?;

    let mut attention = Vec::new();
    let mut entries = Vec::new();

    for (project, (threads, missions)) in projects.iter().zip(project_data) {
        let pid = project.id;

        let active_missions = missions
            .iter()
            .filter(|m| {
                matches!(
                    m.status,
                    ironclaw_engine::types::mission::MissionStatus::Active
                )
            })
            .count() as u64;

        let threads_today = threads
            .iter()
            .filter(|t| t.created_at >= today_start)
            .count() as u64;

        let cost_today_usd: f64 = threads
            .iter()
            .filter(|t| t.created_at >= today_start)
            .map(|t| t.total_cost_usd)
            .sum();

        let failures_24h = threads
            .iter()
            .filter(|t| {
                matches!(t.state, ironclaw_engine::types::thread::ThreadState::Failed)
                    && t.updated_at >= h24_ago
            })
            .count() as u64;

        let last_activity = threads
            .iter()
            .map(|t| t.updated_at)
            .max()
            .map(|dt| dt.to_rfc3339());

        // Count pending gates for threads in this project.
        let project_thread_ids: std::collections::HashSet<_> =
            threads.iter().map(|t| t.id).collect();
        let project_gates: Vec<_> = user_gates
            .iter()
            .filter(|g| project_thread_ids.contains(&g.thread_id))
            .collect();
        let pending_gate_count = project_gates.len() as u64;

        // Build attention items for this project.
        for gate in &project_gates {
            attention.push(AttentionItem {
                kind: "gate".to_string(),
                project_id: pid.to_string(),
                project_name: project.name.clone(),
                message: gate.description.clone(),
                thread_id: Some(gate.thread_id.to_string()),
            });
        }
        for thread in &threads {
            if matches!(
                thread.state,
                ironclaw_engine::types::thread::ThreadState::Failed
            ) && thread.updated_at >= h24_ago
            {
                attention.push(AttentionItem {
                    kind: "failure".to_string(),
                    project_id: pid.to_string(),
                    project_name: project.name.clone(),
                    message: format!("Thread failed: {}", thread.goal),
                    thread_id: Some(thread.id.to_string()),
                });
            }
        }

        // Health: red if failures or gates, yellow if any paused, green otherwise.
        let health = if failures_24h > 0 || pending_gate_count > 0 {
            "red"
        } else if missions.iter().any(|m| {
            matches!(
                m.status,
                ironclaw_engine::types::mission::MissionStatus::Paused
            )
        }) {
            "yellow"
        } else {
            "green"
        };

        entries.push(ProjectOverviewEntry {
            id: pid.to_string(),
            name: project.name.clone(),
            description: project.description.clone(),
            goals: project.goals.clone(),
            health: health.to_string(),
            active_missions,
            total_missions: missions.len() as u64,
            threads_today,
            cost_today_usd,
            failures_24h,
            pending_gates: pending_gate_count,
            last_activity,
            created_at: project.created_at.to_rfc3339(),
        });
    }

    Ok(ProjectsOverviewResponse {
        attention,
        projects: entries,
    })
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
            id: m.id,
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
            id: m.id,
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

/// Build retrospective `ExecutionTrace`s for every currently-known engine
/// thread. Returns an empty vector when engine v2 is not initialized.
///
/// Test-only helper: snapshot-based replay tests fold each trace into
/// per-thread entries under `ReplayOutcome.engine_threads`. Not part of any
/// public API; exposed under `#[doc(hidden)]` because integration tests live
/// in a separate crate and cannot see `#[cfg(test)]`-only items.
///
/// **Caller must serialize access** when more than one engine v2 replay can
/// run concurrently — `ENGINE_STATE` is a process-global singleton and this
/// function iterates every thread across every project. Snapshot tests in
/// `tests/e2e_engine_v2.rs` take `engine_v2_test_lock()` for this reason;
/// new test suites that spawn engine threads must do the same or clear state
/// via `reset_engine_state()` before calling.
#[cfg(feature = "libsql")]
pub async fn engine_retrospectives_for_test()
-> Vec<ironclaw_engine::executor::trace::ExecutionTrace> {
    let Some(lock) = ENGINE_STATE.get() else {
        return Vec::new();
    };
    let guard = lock.read().await;
    let Some(state) = guard.as_ref() else {
        return Vec::new();
    };
    let projects = match state.store.list_all_projects().await {
        Ok(projects) => projects,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for project in projects {
        let threads = match state.store.list_all_threads(project.id).await {
            Ok(threads) => threads,
            Err(_) => continue,
        };
        for mut thread in threads {
            if let Ok(events) = state.store.load_events(thread.id).await {
                thread.events = events;
            }
            out.push(ironclaw_engine::executor::trace::build_trace(&thread));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Cross-module test helpers for installing a minimal engine state.
    //!
    //! `src/channels/web/server.rs` and other callers need to exercise the
    //! engine-thread ownership and history paths without standing up a full
    //! `Agent`. This module exposes a shared lock plus a lightweight
    //! `Thread`-seeding store so caller-level tests can drive
    //! `get_engine_thread` / `list_engine_threads` deterministically.
    //!
    //! The in-file `mod tests` block has its own richer `TestStore` for the
    //! router's own tests; the two are deliberately kept separate so the
    //! helper surface exposed here stays minimal.
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock};

    use tokio::sync::{Mutex as TokioMutex, RwLock as TokioRwLock};

    use ironclaw_engine::{
        CapabilityLease, CapabilityRegistry, ConversationManager, EngineError, LeaseId,
        LeaseManager, MemoryDoc, Mission, MissionId, MissionStatus, PolicyEngine, Project,
        ProjectId, Step, Store, Thread, ThreadEvent, ThreadId, ThreadManager, ThreadState,
    };

    use super::{ENGINE_STATE, EngineState};

    /// Shared lock serializing all cross-module engine-state tests.
    ///
    /// Every test that mutates `ENGINE_STATE` — regardless of which module it
    /// lives in — must acquire this before calling `install_engine_state_*`
    /// so tests don't race on the global `OnceLock`.
    pub(crate) static ENGINE_STATE_TEST_LOCK: LazyLock<TokioMutex<()>> =
        LazyLock::new(|| TokioMutex::new(()));

    /// Minimal in-memory `Store` used by caller-level tests.
    ///
    /// Only the thread-related methods are meaningfully implemented; every
    /// other method returns an empty default. That's intentional — these
    /// tests only ever drive `get_engine_thread` / `list_engine_threads`,
    /// which touch `load_thread` and `list_threads`.
    pub(crate) struct ThreadTestStore {
        threads: TokioRwLock<HashMap<ThreadId, Thread>>,
    }

    impl ThreadTestStore {
        pub(crate) fn new() -> Self {
            Self {
                threads: TokioRwLock::new(HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl Store for ThreadTestStore {
        async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
            self.threads.write().await.insert(thread.id, thread.clone());
            Ok(())
        }
        async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
            Ok(self.threads.read().await.get(&id).cloned())
        }
        async fn list_threads(
            &self,
            project_id: ProjectId,
            user_id: &str,
        ) -> Result<Vec<Thread>, EngineError> {
            Ok(self
                .threads
                .read()
                .await
                .values()
                .filter(|t| t.project_id == project_id && t.is_owned_by(user_id))
                .cloned()
                .collect())
        }
        async fn update_thread_state(
            &self,
            _id: ThreadId,
            _state: ThreadState,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_step(&self, _: &Step) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_steps(&self, _: ThreadId) -> Result<Vec<Step>, EngineError> {
            Ok(vec![])
        }
        async fn append_events(&self, _: &[ThreadEvent]) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_events(&self, _: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
            Ok(vec![])
        }
        async fn save_project(&self, _: &Project) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_project(&self, _: ProjectId) -> Result<Option<Project>, EngineError> {
            Ok(None)
        }
        async fn save_memory_doc(&self, _: &MemoryDoc) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_memory_doc(
            &self,
            _: ironclaw_engine::DocId,
        ) -> Result<Option<MemoryDoc>, EngineError> {
            Ok(None)
        }
        async fn list_memory_docs(
            &self,
            _: ProjectId,
            _user_id: &str,
        ) -> Result<Vec<MemoryDoc>, EngineError> {
            Ok(vec![])
        }
        async fn save_lease(&self, _: &CapabilityLease) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_active_leases(
            &self,
            _: ThreadId,
        ) -> Result<Vec<CapabilityLease>, EngineError> {
            Ok(vec![])
        }
        async fn revoke_lease(&self, _: LeaseId, _: &str) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_mission(&self, _: &Mission) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_mission(&self, _: MissionId) -> Result<Option<Mission>, EngineError> {
            Ok(None)
        }
        async fn list_missions(
            &self,
            _: ProjectId,
            _user_id: &str,
        ) -> Result<Vec<Mission>, EngineError> {
            Ok(vec![])
        }
        async fn update_mission_status(
            &self,
            _: MissionId,
            _: MissionStatus,
        ) -> Result<(), EngineError> {
            Ok(())
        }
    }

    /// Build an `EngineState` backed by a `ThreadTestStore` and install it
    /// into the global `ENGINE_STATE`, overwriting any prior state.
    ///
    /// Callers must already hold `ENGINE_STATE_TEST_LOCK`. The returned
    /// project id matches the default project used by
    /// `list_engine_threads(None, ...)`, so seeded threads are visible
    /// through the default-project lookup path.
    pub(crate) async fn install_engine_state_with_threads(threads: Vec<Thread>) -> ProjectId {
        // The seeded store is sufficient for read-only engine lookups; the
        // thread_manager/conversation_manager are wired only so the
        // EngineState is structurally valid — no test here drives execution.
        struct NoopLlm;
        #[async_trait::async_trait]
        impl ironclaw_engine::LlmBackend for NoopLlm {
            async fn complete(
                &self,
                _: &[ironclaw_engine::ThreadMessage],
                _: &[ironclaw_engine::ActionDef],
                _: &ironclaw_engine::LlmCallConfig,
            ) -> Result<ironclaw_engine::LlmOutput, EngineError> {
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
                _: &CapabilityLease,
                _: &ironclaw_engine::ThreadExecutionContext,
            ) -> Result<ironclaw_engine::ActionResult, EngineError> {
                unreachable!("test engine state is read-only")
            }
            async fn available_actions(
                &self,
                _: &[CapabilityLease],
            ) -> Result<Vec<ironclaw_engine::ActionDef>, EngineError> {
                Ok(vec![])
            }
        }

        let store = Arc::new(ThreadTestStore::new());
        for thread in threads {
            store.save_thread(&thread).await.expect("seed thread"); // safety: cfg(test) fixture
        }
        let store_dyn: Arc<dyn Store> = store;

        let effect_adapter = Arc::new(crate::bridge::EffectBridgeAdapter::new(
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

        let project_id = ProjectId::new();
        let state = EngineState {
            thread_manager: tm,
            conversation_manager: cm,
            effect_adapter,
            store: store_dyn,
            default_project_id: project_id,
            pending_gates: Arc::new(crate::gate::store::PendingGateStore::in_memory()),
            sse: None,
            db: None,
            secrets_store: None,
            auth_manager: None,
            extension_manager: None,
            project_root: super::resolve_project_root(),
        };

        let lock = ENGINE_STATE.get_or_init(|| TokioRwLock::new(None));
        *lock.write().await = Some(state);

        project_id
    }

    /// Clear `ENGINE_STATE` after a test so later tests see an empty engine.
    pub(crate) async fn clear_engine_state() {
        if let Some(lock) = ENGINE_STATE.get() {
            *lock.write().await = None;
        }
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

        // Memory docs (use list_memory_docs directly since "legacy" is the user_id).
        // Pre-PR code tagged ALL migrated skills as __shared__, so legacy Skill
        // docs must be restored to shared_owner_id() — stamping them with
        // owner_id would make them invisible to list_skills_global() and break
        // cross-project visibility for gateway users (issue #2084).
        if let Ok(legacy) = store.list_memory_docs(pid, "legacy").await {
            for mut doc in legacy {
                doc.user_id = if doc.doc_type == ironclaw_engine::DocType::Skill {
                    ironclaw_engine::types::shared_owner_id().to_string()
                } else {
                    owner_id.to_string()
                };
                doc.updated_at = chrono::Utc::now();
                let _ = store.save_memory_doc(&doc).await;
            }
        }
    }

    // Memory docs deserialized from old frontmatter (before project_id was
    // persisted) load with project_id = nil. The per-project loop above never
    // matches them because nil isn't a real project. Assign them to the
    // owner's default project so they become visible to project-scoped queries.
    if let Some(default_project) = all_projects.first() {
        let nil_pid = ironclaw_engine::ProjectId(uuid::Uuid::nil());
        if let Ok(orphaned) = store.list_memory_docs(nil_pid, "legacy").await {
            for mut doc in orphaned {
                doc.project_id = default_project.id;
                doc.user_id = if doc.doc_type == ironclaw_engine::DocType::Skill {
                    ironclaw_engine::types::shared_owner_id().to_string()
                } else {
                    owner_id.to_string()
                };
                doc.updated_at = chrono::Utc::now();
                let _ = store.save_memory_doc(&doc).await;
            }
        }
    }

    debug!("engine v2: legacy user_id migration complete for owner {owner_id}");
}

/// Clamp a caller-supplied `always` approval flag to what the pending
/// gate's `ResumeKind` actually permits.
///
/// Gates for protected actions (orchestrator self-modify writes) advertise
/// `ResumeKind::Approval { allow_always: false }` so the UI hides the
/// "always approve" button. The approval HTTP endpoint still accepts a
/// user-supplied `always: true`, though, so without this clamp a crafted
/// request could install a session-wide auto-approval for `memory_write`
/// and bypass every subsequent per-call gate. The pending gate's own
/// `allow_always` is the authoritative server-side policy.
///
/// Non-approval resume kinds (auth, external callback) carry no
/// "always" semantics and always clamp to `false`.
fn clamp_always_to_resume_kind(always: bool, resume_kind: &ironclaw_engine::ResumeKind) -> bool {
    always
        && matches!(
            resume_kind,
            ironclaw_engine::ResumeKind::Approval { allow_always: true }
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock, Mutex as StdMutex};
    use std::time::Duration;
    use tokio::sync::Mutex as TokioMutex;
    use tokio::sync::RwLock as TokioRwLock;

    use crate::agent::AgentDeps;
    use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
    use crate::channels::{
        Channel, ChannelManager, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    };
    use crate::config::{AgentConfig, SafetyConfig, SkillsConfig};
    use crate::context::ContextManager;
    use crate::error::ChannelError;
    use crate::hooks::HookRegistry;
    use crate::testing::{StubChannel, StubLlm};
    use crate::tools::ToolRegistry;
    use futures::{StreamExt, stream};
    use ironclaw_safety::SafetyLayer;
    use rust_decimal::Decimal;

    static ENGINE_STATE_TEST_LOCK: LazyLock<TokioMutex<()>> = LazyLock::new(|| TokioMutex::new(()));
    static CWD_TEST_LOCK: LazyLock<TokioMutex<()>> = LazyLock::new(|| TokioMutex::new(()));

    struct TestStore {
        conversations: TokioRwLock<Vec<ironclaw_engine::ConversationSurface>>,
        threads: TokioRwLock<HashMap<ironclaw_engine::ThreadId, ironclaw_engine::Thread>>,
        docs: TokioRwLock<Vec<ironclaw_engine::MemoryDoc>>,
        projects: TokioRwLock<Vec<ironclaw_engine::Project>>,
    }

    impl TestStore {
        fn new() -> Self {
            Self {
                conversations: TokioRwLock::new(Vec::new()),
                threads: TokioRwLock::new(HashMap::new()),
                docs: TokioRwLock::new(Vec::new()),
                projects: TokioRwLock::new(Vec::new()),
            }
        }
    }

    struct CurrentDirGuard {
        original: std::path::PathBuf,
    }

    impl CurrentDirGuard {
        fn enter(path: &std::path::Path) -> Self {
            let original = std::env::current_dir().expect("capture current dir");
            std::env::set_current_dir(path).expect("switch current dir");
            Self { original }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
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
            project: &ironclaw_engine::Project,
        ) -> Result<(), ironclaw_engine::EngineError> {
            let mut projects = self.projects.write().await;
            projects.retain(|p| p.id != project.id);
            projects.push(project.clone());
            Ok(())
        }
        async fn load_project(
            &self,
            id: ironclaw_engine::ProjectId,
        ) -> Result<Option<ironclaw_engine::Project>, ironclaw_engine::EngineError> {
            Ok(self
                .projects
                .read()
                .await
                .iter()
                .find(|p| p.id == id)
                .cloned())
        }
        async fn list_projects(
            &self,
            user_id: &str,
        ) -> Result<Vec<ironclaw_engine::Project>, ironclaw_engine::EngineError> {
            Ok(self
                .projects
                .read()
                .await
                .iter()
                .filter(|p| p.user_id == user_id)
                .cloned()
                .collect())
        }
        async fn list_all_projects(
            &self,
        ) -> Result<Vec<ironclaw_engine::Project>, ironclaw_engine::EngineError> {
            Ok(self.projects.read().await.clone())
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
            doc: &ironclaw_engine::MemoryDoc,
        ) -> Result<(), ironclaw_engine::EngineError> {
            let mut docs = self.docs.write().await;
            docs.retain(|d| d.id != doc.id);
            docs.push(doc.clone());
            Ok(())
        }
        async fn load_memory_doc(
            &self,
            id: ironclaw_engine::DocId,
        ) -> Result<Option<ironclaw_engine::MemoryDoc>, ironclaw_engine::EngineError> {
            Ok(self.docs.read().await.iter().find(|d| d.id == id).cloned())
        }
        async fn list_memory_docs(
            &self,
            project_id: ironclaw_engine::ProjectId,
            user_id: &str,
        ) -> Result<Vec<ironclaw_engine::MemoryDoc>, ironclaw_engine::EngineError> {
            Ok(self
                .docs
                .read()
                .await
                .iter()
                .filter(|d| d.project_id == project_id && d.user_id == user_id)
                .cloned()
                .collect())
        }
        async fn list_memory_docs_by_owner(
            &self,
            user_id: &str,
        ) -> Result<Vec<ironclaw_engine::MemoryDoc>, ironclaw_engine::EngineError> {
            Ok(self
                .docs
                .read()
                .await
                .iter()
                .filter(|d| d.user_id == user_id)
                .cloned()
                .collect())
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
        sample_pending_gate_with_request_id(user_id, thread_id, uuid::Uuid::new_v4(), resume_kind)
    }

    fn sample_pending_gate_with_request_id(
        user_id: &str,
        thread_id: ironclaw_engine::ThreadId,
        request_id: uuid::Uuid,
        resume_kind: ironclaw_engine::ResumeKind,
    ) -> PendingGate {
        PendingGate {
            request_id,
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
            paused_lease: None,
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
            settings_store: None,
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

    /// Build a real `ExtensionManager` wired to an in-memory secrets store,
    /// so tests can exercise the no-`AuthManager` extension-backed auth
    /// fallback in `submit_pending_auth_credential` without touching the
    /// real filesystem or catalog. Returns the manager plus the two
    /// `TempDir` handles so callers can drop a fake WASM channel (see
    /// `insert_and_notify_pending_gate_uses_extension_manager_for_auth_display_name`).
    fn test_extension_manager() -> (
        Arc<crate::extensions::ExtensionManager>,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    "router-test-key-at-least-32-chars!!".to_string(),
                ))
                .expect("crypto"),
            )));
        let tool_registry = Arc::new(ToolRegistry::new());
        let mcp_sm = Arc::new(crate::tools::mcp::session::McpSessionManager::new());
        let mcp_pm = Arc::new(crate::tools::mcp::process::McpProcessManager::new());
        let wasm_tools_dir = tempfile::tempdir().expect("temp wasm tools dir");
        let wasm_channels_dir = tempfile::tempdir().expect("temp wasm channels dir");
        let ext_mgr = Arc::new(crate::extensions::ExtensionManager::new(
            mcp_sm,
            mcp_pm,
            secrets,
            tool_registry,
            None,
            None,
            wasm_tools_dir.path().to_path_buf(),
            wasm_channels_dir.path().to_path_buf(),
            None,
            "test".to_string(),
            None,
            vec![],
        ));
        (ext_mgr, wasm_tools_dir, wasm_channels_dir)
    }

    /// Write a minimal fake WASM channel (`<name>.wasm` + capabilities file)
    /// into `wasm_channels_dir` so `ExtensionManager::configure_token` can
    /// walk the channel's `required_secrets` without needing a real
    /// compiled module. Extracted so tests that exercise
    /// `pending_gate_extension_name`, `insert_and_notify_pending_gate`,
    /// and `submit_pending_auth_credential` don't duplicate the fixture.
    fn write_fake_wasm_channel(wasm_channels_dir: &tempfile::TempDir, channel_name: &str) {
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.wasm")),
            b"\0asm fake",
        )
        .expect("write fake wasm");
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.capabilities.json")),
            serde_json::json!({
                "type": "channel",
                "name": channel_name,
                "setup": {
                    "required_secrets": [
                        {"name": format!("{channel_name}_token"), "prompt": "Enter token"}
                    ]
                }
            })
            .to_string(),
        )
        .expect("write capabilities");
    }

    #[tokio::test]
    async fn insert_and_notify_pending_gate_sends_status_no_text() {
        let store = Arc::new(TestStore::new());
        let sse = Arc::new(SseManager::new());
        let mut event_stream = Box::pin(
            sse.subscribe_raw(Some("alice".to_string()), false)
                .expect("subscribe raw"),
        );
        let (agent, statuses) = make_router_test_agent(Some(Arc::clone(&sse))).await;
        let mut state = make_expected_test_state(store);
        state.sse = Some(Arc::clone(&sse));

        let thread_id = ironclaw_engine::ThreadId::new();
        let expected_extension_name = "google_oauth_token".to_string();
        let pending = sample_pending_gate(
            "alice",
            thread_id,
            ironclaw_engine::ResumeKind::Authentication {
                credential_name: ironclaw_common::CredentialName::new(&expected_extension_name)
                    .unwrap(),
                instructions: "Sign in with Google".to_string(),
                auth_url: Some("https://example.test/oauth".to_string()),
            },
        );
        let mut message = crate::channels::IncomingMessage::new("web", "alice", "use google");
        message.thread_id = Some(ironclaw_common::ExternalThreadId::from_trusted(
            thread_id.to_string(),
        ));

        let result = insert_and_notify_pending_gate(&agent, &state, &message, pending)
            .await
            .expect("pending gate inserted");

        // Gate-paused: Pending outcome (card-only via send_status).
        assert!(
            matches!(result, BridgeOutcome::Pending),
            "expected Pending, got: {result:?}"
        );

        // Verify AuthRequired status was sent to the channel.
        let statuses = statuses.lock().await.clone();
        assert!(
            statuses.iter().any(|s| matches!(s, StatusUpdate::AuthRequired { extension_name, .. } if extension_name == "google_oauth_token")),
            "expected AuthRequired status, got: {statuses:?}"
        );

        let event = event_stream.next().await.expect("gate event");
        assert!(
            matches!(
                &event,
                AppEvent::GateRequired {
                    tool_name,
                    thread_id: Some(event_thread_id),
                    extension_name: Some(extension_name),
                    ..
                } if tool_name == "shell"
                    && *event_thread_id == thread_id.to_string()
                    && extension_name.as_str() == expected_extension_name.as_str()
            ),
            "expected GateRequired auth event, got: {event:?}"
        );
    }

    /// A hosted instance without `SECRETS_MASTER_KEY` has no auth manager,
    /// but a WASM channel installed through `ExtensionManager` should
    /// still surface as the correct extension name on the auth-gate card
    /// — not the raw credential name. Covers the extension-manager branch
    /// of `resolve_auth_gate_extension_name` (via `notify_pending_gate`).
    #[tokio::test]
    async fn insert_and_notify_pending_gate_uses_extension_manager_for_auth_display_name() {
        let store = Arc::new(TestStore::new());
        let sse = Arc::new(SseManager::new());
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_extension_manager();
        let channel_name = "test_channel";
        write_fake_wasm_channel(&wasm_channels_dir, channel_name);

        let mut event_stream = Box::pin(
            sse.subscribe_raw(Some("alice".to_string()), false)
                .expect("subscribe raw"),
        );
        let (agent, statuses) = make_router_test_agent(Some(Arc::clone(&sse))).await;
        let mut state = make_expected_test_state(store);
        state.sse = Some(Arc::clone(&sse));
        state.extension_manager = Some(ext_mgr);

        let thread_id = ironclaw_engine::ThreadId::new();
        let credential = ironclaw_common::CredentialName::new("test_channel_token").unwrap();
        let pending = PendingGate {
            action_name: channel_name.to_string(),
            resume_kind: ironclaw_engine::ResumeKind::Authentication {
                credential_name: credential.clone(),
                instructions: "Enter token".to_string(),
                auth_url: None,
            },
            ..sample_pending_gate(
                "alice",
                thread_id,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name: credential,
                    instructions: "Enter token".to_string(),
                    auth_url: None,
                },
            )
        };
        let mut message = crate::channels::IncomingMessage::new("web", "alice", "use test");
        message.thread_id = Some(ironclaw_common::ExternalThreadId::from_trusted(
            thread_id.to_string(),
        ));

        let result = insert_and_notify_pending_gate(&agent, &state, &message, pending)
            .await
            .expect("pending gate inserted");

        assert!(matches!(result, BridgeOutcome::Pending));

        let statuses = statuses.lock().await.clone();
        assert!(
            statuses.iter().any(|s| matches!(
                s,
                StatusUpdate::AuthRequired { extension_name, .. } if extension_name.as_str() == channel_name
            )),
            "expected AuthRequired status with extension-manager-resolved name, got: {statuses:?}"
        );

        let event = event_stream.next().await.expect("gate event");
        assert!(
            matches!(
                &event,
                AppEvent::GateRequired {
                    extension_name: Some(extension_name),
                    ..
                } if extension_name.as_str() == channel_name
            ),
            "expected GateRequired auth event with extension-manager name, got: {event:?}"
        );
    }

    #[tokio::test]
    async fn handle_with_engine_re_emits_pending_approval_on_follow_up() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let store = Arc::new(TestStore::new());
        let sse = Arc::new(SseManager::new());
        let _receiver = sse.sender().subscribe();
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
        message.thread_id = Some(ironclaw_common::ExternalThreadId::from_trusted(
            thread_id.to_string(),
        ));

        let response = handle_with_engine(&agent, &message, &message.content)
            .await
            .expect("follow-up handled");

        // Gate-paused: Pending outcome (card-only via send_status).
        assert!(
            matches!(response, BridgeOutcome::Pending),
            "expected Pending for pending gate re-emit, got: {response:?}"
        );

        // Verify ApprovalNeeded status was sent to the channel.
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
                    credential_name: ironclaw_common::CredentialName::new("github").unwrap(),
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

    #[tokio::test]
    async fn handle_approval_ignores_pending_gate_from_different_thread() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;

        let outcome = async {
            let store = Arc::new(TestStore::new());
            let state = make_expected_test_state(store);
            let pending_thread_id = ironclaw_engine::ThreadId::new();
            let active_thread_id = ironclaw_engine::ThreadId::new();
            let pending = sample_pending_gate(
                "alice",
                pending_thread_id,
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            );
            state
                .pending_gates
                .insert(pending)
                .await
                .expect("insert pending gate");

            *lock.write().await = Some(state);

            let (agent, _statuses) = make_router_test_agent(None).await;
            let message = IncomingMessage::new("gateway", "alice", "/approve")
                .with_thread(active_thread_id.to_string());

            let result = handle_approval(&agent, &message, true, false)
                .await
                .expect("handle approval");

            assert!(
                matches!(result, BridgeOutcome::Respond(ref s) if s == "No pending approval for this thread."),
                "expected Respond with no-pending message, got: {result:?}"
            );
        }
        .await;

        *lock.write().await = None;
        outcome
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
            Some("call-1".to_string())
        );
    }

    #[tokio::test]
    async fn forward_event_to_channel_preserves_call_id_for_action_events() {
        let statuses = Arc::new(TokioMutex::new(Vec::new()));
        let manager = ChannelManager::new();
        manager
            .add(Box::new(RecordingStatusChannel {
                name: "test".to_string(),
                statuses: Arc::clone(&statuses),
            }))
            .await;
        let manager = Arc::new(manager);

        let event = ironclaw_engine::ThreadEvent::new(
            ironclaw_engine::ThreadId::new(),
            ironclaw_engine::EventKind::ActionExecuted {
                step_id: ironclaw_engine::StepId::new(),
                action_name: "memory_read".to_string(),
                call_id: "call-memory-read-1".to_string(),
                duration_ms: 42,
                params_summary: Some("notes/today.md".to_string()),
            },
        );

        forward_event_to_channel(&event, &manager, "test", &serde_json::json!({})).await;

        let statuses = statuses.lock().await;
        assert_eq!(statuses.len(), 2);
        assert!(matches!(
            &statuses[0],
            StatusUpdate::ToolStarted {
                call_id,
                detail,
                ..
            } if call_id.as_deref() == Some("call-memory-read-1")
                && detail.as_deref() == Some("notes/today.md")
        ));
        assert!(matches!(
            &statuses[1],
            StatusUpdate::ToolCompleted {
                call_id,
                duration_ms,
                success,
                ..
            } if call_id.as_deref() == Some("call-memory-read-1")
                && duration_ms == &Some(42)
                && *success
        ));
    }

    #[test]
    fn thread_event_to_app_events_preserves_call_id_for_action_events() {
        let event = ironclaw_engine::ThreadEvent::new(
            ironclaw_engine::ThreadId::new(),
            ironclaw_engine::EventKind::ActionFailed {
                step_id: ironclaw_engine::StepId::new(),
                action_name: "memory_read".to_string(),
                call_id: "call-memory-read-2".to_string(),
                error: "permission denied".to_string(),
                duration_ms: 17,
                params_summary: Some("secret.md".to_string()),
            },
        );

        let app_events = thread_event_to_app_events(&event, "thread-123");

        assert_eq!(app_events.len(), 2);
        assert!(matches!(
            &app_events[0],
            AppEvent::ToolStarted {
                call_id,
                detail,
                thread_id,
                ..
            } if call_id.as_deref() == Some("call-memory-read-2")
                && detail.as_deref() == Some("secret.md")
                && thread_id.as_deref() == Some("thread-123")
        ));
        assert!(matches!(
            &app_events[1],
            AppEvent::ToolCompleted {
                call_id,
                error,
                success,
                duration_ms,
                thread_id,
                ..
            } if call_id.as_deref() == Some("call-memory-read-2")
                && error.as_deref() == Some("permission denied")
                && !success
                && duration_ms == &Some(17)
                && thread_id.as_deref() == Some("thread-123")
        ));
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
            Some("call-2".to_string())
        );
    }

    /// Regression: in production the orchestrator writes ActionResult and
    /// assistant-with-actions messages to `internal_messages` via
    /// `sync_runtime_state`, not `messages`.  The legacy fallback must scan
    /// `internal_messages` to find unresolved call ids.
    #[test]
    fn resolved_call_id_legacy_fallback_scans_internal_messages() {
        let mut thread = ironclaw_engine::Thread::new(
            "goal",
            ironclaw_engine::ThreadType::Foreground,
            ironclaw_engine::ProjectId::new(),
            "alice",
            ironclaw_engine::ThreadConfig::default(),
        );

        // Simulate production: assistant + action results in internal_messages
        thread.add_internal_message(ironclaw_engine::ThreadMessage::assistant_with_actions(
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
        thread.add_internal_message(ironclaw_engine::ThreadMessage::action_result(
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

        // Before the fix this returned None because only `messages` was scanned.
        assert_eq!(
            resolved_call_id_for_pending_action(&thread, &pending),
            Some("call-2".to_string())
        );
    }

    #[test]
    fn resolved_call_id_returns_none_when_no_history_match() {
        let thread = ironclaw_engine::Thread::new(
            "goal",
            ironclaw_engine::ThreadType::Foreground,
            ironclaw_engine::ProjectId::new(),
            "alice",
            ironclaw_engine::ThreadConfig::default(),
        );
        let pending = PendingGate {
            call_id: String::new(),
            ..sample_pending_gate(
                "alice",
                thread.id,
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            )
        };

        assert!(resolved_call_id_for_pending_action(&thread, &pending).is_none());
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
                    credential_name: ironclaw_common::CredentialName::new("github_token").unwrap(),
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
                    credential_name: ironclaw_common::CredentialName::new("linear_token").unwrap(),
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
                    credential_name: ironclaw_common::CredentialName::new("github_token").unwrap(),
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
                    credential_name: ironclaw_common::CredentialName::new("linear_token").unwrap(),
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

    #[tokio::test]
    async fn discard_engine_pending_auth_request_discards_only_matching_auth_gate() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let store = Arc::new(TestStore::new());
        let state = make_expected_test_state(store);
        let thread_a = ironclaw_engine::ThreadId::new();
        let thread_b = ironclaw_engine::ThreadId::new();
        let auth_request_id = uuid::Uuid::new_v4();
        let approval_request_id = uuid::Uuid::new_v4();

        state
            .pending_gates
            .insert(sample_pending_gate_with_request_id(
                "alice",
                thread_a,
                auth_request_id,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name: ironclaw_common::CredentialName::new("telegram_bot_token")
                        .unwrap(),
                    instructions: "paste token".into(),
                    auth_url: None,
                },
            ))
            .await
            .unwrap();
        state
            .pending_gates
            .insert(sample_pending_gate_with_request_id(
                "alice",
                thread_b,
                approval_request_id,
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            ))
            .await
            .unwrap();

        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;
        *lock.write().await = Some(state);

        let discarded = discard_engine_pending_auth_request(
            "alice",
            auth_request_id,
            Some(&thread_a.to_string()),
        )
        .await;

        assert!(discarded);
        let guard = lock.read().await;
        let state = guard.as_ref().unwrap();
        let remaining = state.pending_gates.list_for_user("alice").await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].request_id, approval_request_id);
        drop(guard);
        *lock.write().await = None;
    }

    #[tokio::test]
    async fn discard_engine_pending_auth_request_matches_scope_thread_id() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let store = Arc::new(TestStore::new());
        let state = make_expected_test_state(store);
        let thread_id = ironclaw_engine::ThreadId::new();
        let request_id = uuid::Uuid::new_v4();

        let mut pending = sample_pending_gate_with_request_id(
            "alice",
            thread_id,
            request_id,
            ironclaw_engine::ResumeKind::Authentication {
                credential_name: ironclaw_common::CredentialName::new("telegram_bot_token")
                    .unwrap(),
                instructions: "paste token".into(),
                auth_url: None,
            },
        );
        pending.scope_thread_id = Some(ironclaw_common::ExternalThreadId::from_trusted(
            "gateway-thread-123".to_string(),
        ));
        state.pending_gates.insert(pending).await.unwrap();

        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;
        *lock.write().await = Some(state);

        let discarded =
            discard_engine_pending_auth_request("alice", request_id, Some("gateway-thread-123"))
                .await;

        assert!(discarded);
        let guard = lock.read().await;
        let state = guard.as_ref().unwrap();
        assert!(state.pending_gates.list_for_user("alice").await.is_empty());
        drop(guard);
        *lock.write().await = None;
    }

    #[tokio::test]
    async fn transition_engine_pending_auth_request_to_pairing_replaces_gate() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let store = Arc::new(TestStore::new());
        let state = make_expected_test_state(store);
        let thread_id = ironclaw_engine::ThreadId::new();
        let request_id = uuid::Uuid::new_v4();

        let mut pending = sample_pending_gate_with_request_id(
            "alice",
            thread_id,
            request_id,
            ironclaw_engine::ResumeKind::Authentication {
                credential_name: ironclaw_common::CredentialName::new("telegram_bot_token")
                    .unwrap(),
                instructions: "paste token".into(),
                auth_url: None,
            },
        );
        pending.scope_thread_id = Some(ironclaw_common::ExternalThreadId::from_trusted(
            "gateway-thread-123".to_string(),
        ));
        state.pending_gates.insert(pending).await.unwrap();

        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;
        *lock.write().await = Some(state);

        let next_request_id = transition_engine_pending_auth_request_to_pairing(
            "alice",
            request_id,
            Some("gateway-thread-123"),
            "telegram",
        )
        .await
        .expect("transition auth gate to pairing")
        .expect("replacement gate request id");

        let guard = lock.read().await;
        let state = guard.as_ref().unwrap();
        let remaining = state.pending_gates.list_for_user("alice").await;
        assert_eq!(remaining.len(), 1);
        let replacement = &remaining[0];
        assert_eq!(replacement.request_id.to_string(), next_request_id);
        assert_eq!(replacement.gate_name, "pairing");
        assert_eq!(
            replacement.scope_thread_id.as_ref().map(|t| t.as_str()),
            Some("gateway-thread-123")
        );
        assert_eq!(replacement.thread_id, thread_id);
        assert!(matches!(
            replacement.resume_kind,
            ironclaw_engine::ResumeKind::External { .. }
        ));
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
            extension_manager: None,
            project_root: resolve_project_root(),
        }
    }

    async fn make_test_agent_with_status_channel(
        channel_name: &str,
    ) -> (Agent, Arc<StdMutex<Vec<StatusUpdate>>>) {
        let (stub, _sender) = StubChannel::new(channel_name);
        let statuses = stub.captured_statuses_handle();
        let manager = ChannelManager::new();
        manager.add(Box::new(stub)).await;

        let deps = AgentDeps {
            owner_id: "default".to_string(),
            store: None,
            settings_store: None,
            llm: Arc::new(StubLlm::default()),
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

        let agent = Agent::new(
            AgentConfig {
                name: "test-agent".to_string(),
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
            Arc::new(manager),
            None,
            None,
            None,
            Some(Arc::new(ContextManager::new(1))),
            None,
        );

        (agent, statuses)
    }

    fn make_expected_test_state_with_llm(
        store: Arc<TestStore>,
        llm: Arc<dyn ironclaw_engine::LlmBackend>,
    ) -> EngineState {
        use ironclaw_engine::{
            CapabilityRegistry, ConversationManager, LeaseManager, PolicyEngine, ThreadManager,
        };

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
            llm,
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
            extension_manager: None,
            project_root: resolve_project_root(),
        }
    }

    #[tokio::test]
    async fn handle_with_engine_reemits_approval_status_for_pending_gate() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;

        let outcome = async {
            let store = Arc::new(TestStore::new());
            let state = make_expected_test_state(store);
            let thread_id = ironclaw_engine::ThreadId::new();
            let pending = sample_pending_gate(
                "alice",
                thread_id,
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            );
            state
                .pending_gates
                .insert(pending.clone())
                .await
                .expect("insert pending gate");

            *lock.write().await = Some(state);

            let (agent, statuses) = make_test_agent_with_status_channel("tui").await;
            let message = IncomingMessage::new("tui", "alice", "what now?")
                .with_thread(thread_id.to_string());

            let result = handle_with_engine_inner(&agent, &message, &message.content, 0)
                .await
                .expect("handle with engine");

            // Gate-paused: Pending outcome (card-only via send_status).
            assert!(
                matches!(result, BridgeOutcome::Pending),
                "expected Pending for pending gate re-emit, got: {result:?}"
            );

            let statuses = statuses.lock().expect("poisoned").clone();
            assert!(
                statuses.iter().any(|status| matches!(
                    status,
                    StatusUpdate::ApprovalNeeded {
                        request_id,
                        tool_name,
                        description,
                        parameters,
                        allow_always,
                    } if request_id == &pending.request_id.to_string()
                        && tool_name == "shell"
                        && description == "pending gate"
                        && parameters == &serde_json::json!({"cmd": "ls"})
                        && *allow_always
                )),
                "expected approval status to be re-emitted, got: {statuses:?}"
            );

            Ok::<(), crate::error::Error>(())
        }
        .await;

        *lock.write().await = None;
        outcome.expect("router approval re-emit test");
    }

    #[tokio::test]
    async fn handle_with_engine_persists_attachment_files_and_indexes_them() {
        let _engine_guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let _cwd_guard = CWD_TEST_LOCK.lock().await;
        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;

        let outcome = async {
            let store = Arc::new(TestStore::new());
            let temp_dir = tempfile::tempdir().expect("temp dir");
            let _cwd = CurrentDirGuard::enter(temp_dir.path());
            let mut state = make_expected_test_state(store.clone());
            state.project_root = temp_dir.path().join("projects");
            *lock.write().await = Some(state);

            let (agent, _statuses) = make_router_test_agent(None).await;

            let message =
                IncomingMessage::new("gateway", "alice", "Please keep this upload handy.")
                    .with_attachments(vec![crate::channels::IncomingAttachment {
                        id: "att-1".to_string(),
                        kind: crate::channels::AttachmentKind::Document,
                        mime_type: "text/plain".to_string(),
                        filename: Some("notes.txt".to_string()),
                        size_bytes: Some(20),
                        source_url: None,
                        storage_key: None,
                        local_path: None,
                        extracted_text: Some("Remember this file.".to_string()),
                        data: b"Remember this file.\n".to_vec(),
                        duration_secs: None,
                    }]);

            let _ = handle_with_engine_inner(&agent, &message, &message.content, 0)
                .await
                .expect("router handled message");

            let thread = store
                .threads
                .read()
                .await
                .values()
                .next()
                .cloned()
                .expect("thread saved");
            let user_msg = thread
                .messages
                .iter()
                .find(|msg| msg.role == ironclaw_engine::MessageRole::User)
                .expect("user message recorded");
            assert!(
                user_msg
                    .content
                    .contains("project_path=\".ironclaw/attachments/alice/"),
                "expected saved project path in user content, got: {}",
                user_msg.content
            );
            assert!(
                user_msg
                    .content
                    .contains("Saved to project file: .ironclaw/attachments/alice/"),
                "expected saved path hint in user content, got: {}",
                user_msg.content
            );

            let docs = store.docs.read().await;
            let note = docs.iter().next().cloned().expect("attachment note saved");
            drop(docs);

            assert_eq!(note.project_id, thread.project_id);
            assert_eq!(note.user_id, "alice");
            assert_eq!(note.doc_type, ironclaw_engine::DocType::Note);
            assert_eq!(note.source_thread_id, Some(thread.id));
            assert!(note.content.contains("## Extracted text"));
            assert!(note.content.contains("Remember this file."));

            let relative_path = note
                .metadata
                .get("project_path")
                .and_then(|value| value.as_str())
                .expect("project_path metadata");
            let absolute_path = temp_dir.path().join("projects").join(relative_path);
            assert!(
                absolute_path.exists(),
                "expected saved file at {}",
                absolute_path.display()
            );
            let bytes = tokio::fs::read(&absolute_path)
                .await
                .expect("read saved attachment");
            assert_eq!(bytes, b"Remember this file.\n".to_vec());
            assert!(
                message
                    .attachments
                    .first()
                    .is_some_and(|attachment| !attachment.data.is_empty()),
                "source message should remain unchanged"
            );

            Ok::<(), crate::error::Error>(())
        }
        .await;

        *lock.write().await = None;
        outcome.expect("router attachment persistence test");
    }

    #[tokio::test]
    async fn resolve_gate_repairs_call_id_for_resume_output_auth_resume() {
        struct InspectingLlm {
            expected_call_id: String,
        }

        #[async_trait::async_trait]
        impl ironclaw_engine::LlmBackend for InspectingLlm {
            async fn complete(
                &self,
                messages: &[ironclaw_engine::ThreadMessage],
                _: &[ironclaw_engine::ActionDef],
                _: &ironclaw_engine::LlmCallConfig,
            ) -> Result<ironclaw_engine::LlmOutput, ironclaw_engine::EngineError> {
                let matched = messages.iter().any(|message| {
                    message.role == ironclaw_engine::MessageRole::ActionResult
                        && message.action_name.as_deref() == Some("shell")
                        && message.action_call_id.as_deref() == Some(self.expected_call_id.as_str())
                });

                Ok(ironclaw_engine::LlmOutput {
                    response: ironclaw_engine::LlmResponse::Text(if matched {
                        "paired".into()
                    } else {
                        "missing-pairing".into()
                    }),
                    usage: ironclaw_engine::TokenUsage::default(),
                })
            }

            fn model_name(&self) -> &str {
                "inspect-call-id"
            }
        }

        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;

        let outcome = async {
            let store = Arc::new(TestStore::new());
            let llm: Arc<dyn ironclaw_engine::LlmBackend> = Arc::new(InspectingLlm {
                expected_call_id: "call-2".to_string(),
            });

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
            thread.state = ironclaw_engine::ThreadState::Waiting;
            store
                .save_thread(&thread)
                .await
                .expect("save waiting thread");

            let mut conversation = ironclaw_engine::ConversationSurface::new("web", "alice");
            conversation.track_thread(thread.id);
            let conversation_id = conversation.id;
            store
                .save_conversation(&conversation)
                .await
                .expect("save conversation");

            let state = make_expected_test_state_with_llm(store.clone(), llm);
            state
                .conversation_manager
                .bootstrap_user("alice")
                .await
                .expect("bootstrap conversations");

            let pending = PendingGate {
                call_id: String::new(),
                conversation_id,
                action_name: "shell".into(),
                parameters: serde_json::json!({"cmd": "ls"}),
                resume_kind: ironclaw_engine::ResumeKind::Authentication {
                    credential_name: ironclaw_common::CredentialName::new("github_token").unwrap(),
                    instructions: "paste token".into(),
                    auth_url: None,
                },
                resume_output: Some(serde_json::json!({"ok": true})),
                ..sample_pending_gate(
                    "alice",
                    thread.id,
                    ironclaw_engine::ResumeKind::Authentication {
                        credential_name: ironclaw_common::CredentialName::new("github_token")
                            .unwrap(),
                        instructions: "paste token".into(),
                        auth_url: None,
                    },
                )
            };
            state
                .pending_gates
                .insert(pending.clone())
                .await
                .expect("insert pending gate");

            *lock.write().await = Some(state);

            let (agent, _statuses) = make_test_agent_with_status_channel("web").await;
            let message =
                IncomingMessage::new("web", "alice", "token").with_thread(thread.id.to_string());

            let result = resolve_gate(
                &agent,
                &message,
                thread.id,
                pending.request_id,
                ironclaw_engine::GateResolution::CredentialProvided {
                    token: "secret-token".into(),
                },
            )
            .await
            .expect("resolve gate");

            assert!(matches!(
                result,
                BridgeOutcome::Respond(ref text) if text == "paired"
            ));

            Ok::<(), crate::error::Error>(())
        }
        .await;

        *lock.write().await = None;
        outcome.expect("router auth resume_output call-id repair test");
    }

    /// Hosted instance path: no `AuthManager`, but the `ExtensionManager`
    /// can still configure the token on a WASM channel. After
    /// `resolve_gate` on `CredentialProvided`, the auth gate should be
    /// re-queued for the channel's *next* required secret (multi-secret
    /// channels walk the list on re-configure) and the auth-completed
    /// status must carry the extension name, not the credential name.
    #[tokio::test]
    async fn resolve_gate_uses_extension_manager_without_auth_manager_for_auth_resume() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;

        let outcome = async {
            let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_extension_manager();
            let channel_name = "test_channel";
            write_fake_wasm_channel(&wasm_channels_dir, channel_name);

            let store = Arc::new(TestStore::new());

            let mut thread = ironclaw_engine::Thread::new(
                "goal",
                ironclaw_engine::ThreadType::Foreground,
                ironclaw_engine::ProjectId::new(),
                "alice",
                ironclaw_engine::ThreadConfig::default(),
            );
            thread.add_message(ironclaw_engine::ThreadMessage::assistant_with_actions(
                Some("install channel".to_string()),
                vec![ironclaw_engine::ActionCall {
                    id: "call-install".to_string(),
                    action_name: "tool_install".to_string(),
                    parameters: serde_json::json!({"name": channel_name}),
                }],
            ));
            thread.state = ironclaw_engine::ThreadState::Waiting;
            store
                .save_thread(&thread)
                .await
                .expect("save waiting thread");

            let mut conversation = ironclaw_engine::ConversationSurface::new("web", "alice");
            conversation.track_thread(thread.id);
            let conversation_id = conversation.id;
            store
                .save_conversation(&conversation)
                .await
                .expect("save conversation");

            let mut state = make_expected_test_state(store.clone());
            state.extension_manager = Some(ext_mgr);
            state
                .conversation_manager
                .bootstrap_user("alice")
                .await
                .expect("bootstrap conversations");

            let credential = ironclaw_common::CredentialName::new("test_channel_token").unwrap();
            let pending = PendingGate {
                call_id: "call-install".into(),
                conversation_id,
                action_name: "tool_install".into(),
                parameters: serde_json::json!({"name": channel_name}),
                resume_kind: ironclaw_engine::ResumeKind::Authentication {
                    credential_name: credential.clone(),
                    instructions: "paste token".into(),
                    auth_url: None,
                },
                resume_output: Some(serde_json::json!({"ok": true})),
                ..sample_pending_gate(
                    "alice",
                    thread.id,
                    ironclaw_engine::ResumeKind::Authentication {
                        credential_name: credential,
                        instructions: "paste token".into(),
                        auth_url: None,
                    },
                )
            };
            state
                .pending_gates
                .insert(pending.clone())
                .await
                .expect("insert pending gate");

            *lock.write().await = Some(state);

            let (agent, statuses) = make_router_test_agent(None).await;
            let message =
                IncomingMessage::new("web", "alice", "token").with_thread(thread.id.to_string());

            let result = resolve_gate(
                &agent,
                &message,
                thread.id,
                pending.request_id,
                ironclaw_engine::GateResolution::CredentialProvided {
                    token: "secret-token".into(),
                },
            )
            .await
            .expect("resolve gate");

            assert!(matches!(result, BridgeOutcome::Pending));

            let statuses = statuses.lock().await.clone();
            assert!(
                statuses.iter().any(|status| matches!(
                    status,
                    StatusUpdate::AuthRequired { extension_name, .. }
                        if extension_name.as_str() == channel_name
                )),
                "expected AuthRequired with extension-manager-resolved name, got: {statuses:?}"
            );

            let pending_gates = lock
                .read()
                .await
                .as_ref()
                .expect("engine state")
                .pending_gates
                .list_for_user("alice")
                .await;
            assert_eq!(pending_gates.len(), 1, "expected auth gate to be requeued");
            let requeued = &pending_gates[0];
            assert!(matches!(
                &requeued.resume_kind,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name,
                    instructions,
                    auth_url: None,
                } if credential_name.as_str() == "test_channel_token"
                    && instructions.contains("Configuration saved for 'test_channel'.")
            ));

            Ok::<(), crate::error::Error>(())
        }
        .await;

        *lock.write().await = None;
        outcome.expect("router extension-manager auth resume test");
    }

    /// Degenerate case: no auth manager, no extension manager, no secrets
    /// store, and no `resume_output` staged. The submit helper returns
    /// `SkippedNoBackend`, the waiting thread must transition to Failed
    /// with an explicit reason, and the user must get a `BridgeOutcome::Respond`
    /// carrying the same error — so a misconfigured deploy fails loudly
    /// instead of silently dropping the user's credential.
    #[tokio::test]
    async fn resolve_gate_fails_waiting_thread_when_no_auth_backend_and_no_resume_output() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;

        let outcome = async {
            let store = Arc::new(TestStore::new());

            let mut thread = ironclaw_engine::Thread::new(
                "goal",
                ironclaw_engine::ThreadType::Foreground,
                ironclaw_engine::ProjectId::new(),
                "alice",
                ironclaw_engine::ThreadConfig::default(),
            );
            thread.state = ironclaw_engine::ThreadState::Waiting;
            store
                .save_thread(&thread)
                .await
                .expect("save waiting thread");

            let mut conversation = ironclaw_engine::ConversationSurface::new("web", "alice");
            conversation.track_thread(thread.id);
            let conversation_id = conversation.id;
            store
                .save_conversation(&conversation)
                .await
                .expect("save conversation");

            let state = make_expected_test_state(store.clone());
            state
                .conversation_manager
                .bootstrap_user("alice")
                .await
                .expect("bootstrap conversations");

            let credential = ironclaw_common::CredentialName::new("github_token").unwrap();
            let pending = PendingGate {
                conversation_id,
                action_name: "shell".into(),
                parameters: serde_json::json!({"cmd": "ls"}),
                resume_kind: ironclaw_engine::ResumeKind::Authentication {
                    credential_name: credential.clone(),
                    instructions: "paste token".into(),
                    auth_url: None,
                },
                resume_output: None,
                ..sample_pending_gate(
                    "alice",
                    thread.id,
                    ironclaw_engine::ResumeKind::Authentication {
                        credential_name: credential,
                        instructions: "paste token".into(),
                        auth_url: None,
                    },
                )
            };
            state
                .pending_gates
                .insert(pending.clone())
                .await
                .expect("insert pending gate");

            *lock.write().await = Some(state);

            let (agent, statuses) = make_router_test_agent(None).await;
            let message =
                IncomingMessage::new("web", "alice", "token").with_thread(thread.id.to_string());

            let result = resolve_gate(
                &agent,
                &message,
                thread.id,
                pending.request_id,
                ironclaw_engine::GateResolution::CredentialProvided {
                    token: "secret-token".into(),
                },
            )
            .await
            .expect("resolve gate");

            let expected =
                "No auth manager, extension manager, or secrets store available to store credential.";
            assert!(matches!(
                result,
                BridgeOutcome::Respond(ref text) if text == expected
            ));

            let statuses = statuses.lock().await.clone();
            assert!(statuses.iter().any(|status| matches!(
                status,
                StatusUpdate::AuthCompleted {
                    extension_name,
                    success: false,
                    message,
                } if extension_name.as_str() == "github_token" && message == expected
            )));

            let saved = store
                .load_thread(thread.id)
                .await
                .expect("load thread")
                .expect("thread exists");
            assert_eq!(saved.state, ironclaw_engine::ThreadState::Failed);

            Ok::<(), crate::error::Error>(())
        }
        .await;

        *lock.write().await = None;
        outcome.expect("router no-auth-backend failure test");
    }

    /// Regression for #2323: when the target thread is deleted between
    /// `take_verified` and resume, an `Approved` resolution must emit
    /// `GateResolved { resolution: "expired" }` (not just generic error) and
    /// must *not* persist `AlwaysAllow` — otherwise the caller's rollback
    /// (`result.is_err()` branch) would be skipped, leaving a permanent
    /// auto-approve preference behind for a tool that never ran. Covers
    /// both the `always: false` SSE contract and the pre-flight thread
    /// check that gates `persist_always_allow`.
    #[tokio::test]
    async fn resolve_gate_approved_with_missing_thread_emits_expired_and_skips_persist() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;

        let outcome = async {
            let store = Arc::new(TestStore::new());
            let sse = Arc::new(SseManager::new());
            let mut event_stream = Box::pin(
                sse.subscribe_raw(Some("alice".to_string()), false)
                    .expect("subscribe raw"),
            );

            let mut state = make_expected_test_state(store);
            state.sse = Some(Arc::clone(&sse));

            // Thread deleted / never saved — `state.store.load_thread(tid)`
            // returns `Ok(None)`, mimicking the #2323 race.
            let thread_id = ironclaw_engine::ThreadId::new();
            let pending = sample_pending_gate(
                "alice",
                thread_id,
                ironclaw_engine::ResumeKind::Approval { allow_always: true },
            );
            state
                .pending_gates
                .insert(pending.clone())
                .await
                .expect("insert pending gate");

            *lock.write().await = Some(state);

            let (agent, _statuses) = make_router_test_agent(Some(Arc::clone(&sse))).await;
            let message =
                IncomingMessage::new("web", "alice", "approve").with_thread(thread_id.to_string());

            let result = resolve_gate(
                &agent,
                &message,
                thread_id,
                pending.request_id,
                ironclaw_engine::GateResolution::Approved { always: true },
            )
            .await
            .expect("resolve gate");

            // Graceful dismissal, not an error.
            assert!(matches!(
                result,
                BridgeOutcome::Respond(ref text)
                    if text == "Thread no longer exists. Approval dismissed."
            ));

            // The first (and only) SSE event on this subscription must be
            // `expired`. Critically it must NOT be `approved_always` — a
            // prior implementation emitted that first, then discovered the
            // missing thread and committed AlwaysAllow before anyone could
            // roll it back.
            let event = event_stream.next().await.expect("gate event");
            assert!(
                matches!(
                    &event,
                    AppEvent::GateResolved { resolution, .. } if resolution == "expired"
                ),
                "expected expired gate resolution (pre-flight short-circuit), got: {event:?}"
            );

            Ok::<(), crate::error::Error>(())
        }
        .await;

        *lock.write().await = None;
        outcome.expect("router orphaned-approved-gate expired test");
    }

    /// Unit test for the extension-manager branch of
    /// `submit_pending_auth_credential`. The caller-level regression is
    /// `resolve_gate_uses_extension_manager_without_auth_manager_for_auth_resume`;
    /// this helper test just pins the contract that a WASM channel's
    /// `configure_token` produces a `Stored` outcome (not `SkippedNoBackend`)
    /// when only the extension manager is wired up.
    #[tokio::test]
    async fn submit_pending_auth_credential_uses_extension_manager_without_auth_manager() {
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_extension_manager();
        let channel_name = "test_channel";
        write_fake_wasm_channel(&wasm_channels_dir, channel_name);

        let store = Arc::new(TestStore::new());
        let mut state = make_expected_test_state(store);
        state.extension_manager = Some(ext_mgr);

        let result = submit_pending_auth_credential(
            &state,
            channel_name,
            "test_channel_token",
            "dummy-token",
            "test",
        )
        .await
        .expect("extension manager fallback should configure token");

        let PendingAuthCredentialSubmission::Stored(result) = result else {
            panic!("expected stored configure result");
        };

        assert!(
            result
                .message
                .contains("Configuration saved for 'test_channel'."),
            "unexpected configure result: {}",
            result.message
        );
    }

    /// If the underlying backend returns `ValidationFailed` (e.g. the
    /// `AuthManager` rejects an empty token before it ever reaches the
    /// extension), `submit_pending_auth_credential` must propagate it
    /// unchanged so `resolve_gate` can route it to
    /// `requeue_auth_pending_gate` and re-surface the validation message
    /// on the same auth card. Covers the `Err(ValidationFailed)` match
    /// arm introduced alongside the new helper.
    #[tokio::test]
    async fn submit_pending_auth_credential_propagates_validation_failed() {
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    "router-test-key-at-least-32-chars!!".to_string(),
                ))
                .expect("crypto"),
            )));
        let auth_manager = Arc::new(AuthManager::new(secrets, None, None, None));

        let store = Arc::new(TestStore::new());
        let mut state = make_expected_test_state(store);
        state.auth_manager = Some(auth_manager);

        // Empty token is rejected with `ValidationFailed` by
        // `AuthManager::submit_auth_token` before any downstream backend
        // is touched. The helper must bubble that error variant so the
        // `Err(ValidationFailed)` arm in `resolve_gate` can re-queue the
        // gate rather than hard-failing the thread.
        let err = submit_pending_auth_credential(
            &state,
            "test_channel",
            "test_channel_token",
            "",
            "test",
        )
        .await
        .expect_err("empty token must surface as an error");

        assert!(
            matches!(err, crate::extensions::ExtensionError::ValidationFailed(_)),
            "expected ValidationFailed, got: {err:?}"
        );
    }

    /// `format_auth_completed_resuming` strips trailing periods and
    /// whitespace from the upstream backend message before appending
    /// ". Resuming...". Regression coverage for the double-period bug
    /// flagged on PR #2622 — `ExtensionManager::configure_token` returns
    /// "Configuration saved for 'X'." which used to render as
    /// "...'X'.. Resuming...".
    #[test]
    fn format_auth_completed_resuming_strips_trailing_period() {
        // The motivating case: extension-manager backend message.
        assert_eq!(
            format_auth_completed_resuming("Configuration saved for 'telegram'."),
            "Configuration saved for 'telegram'. Resuming..."
        );
        // Multiple trailing periods + whitespace collapse cleanly.
        assert_eq!(
            format_auth_completed_resuming("done...  \n"),
            "done. Resuming..."
        );
        // A message with no trailing punctuation gets exactly one period.
        assert_eq!(format_auth_completed_resuming("ok"), "ok. Resuming...");
        // Non-period punctuation is intentionally left intact (no backend
        // currently produces these, but the spec is "trim periods only").
        assert_eq!(
            format_auth_completed_resuming("ready!"),
            "ready!. Resuming..."
        );
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

    #[test]
    fn parse_credential_name_full_json() {
        let text = r#"{"error":"authentication_required","credential_name":"github_pat"}"#;
        assert_eq!(parse_credential_name(text), Some("github_pat".to_string()));
    }

    #[test]
    fn parse_credential_name_embedded_json() {
        let text = "Tool failed: {\"error\":\"authentication_required\",\"credential_name\":\"slack_token\"}";
        assert_eq!(parse_credential_name(text), Some("slack_token".to_string()));
    }

    #[test]
    fn parse_credential_name_prose_fallback() {
        let text = "authentication_required: missing credential_name 'gmail_oauth' for request";
        assert_eq!(parse_credential_name(text), Some("gmail_oauth".to_string()));
    }

    #[test]
    fn parse_credential_name_rejects_invalid_chars() {
        // hyphen is not in the allowed alphabet
        let text = r#"{"credential_name":"foo-bar"}"#;
        assert_eq!(parse_credential_name(text), None);
        // spaces too
        let text2 = r#"{"credential_name":"foo bar"}"#;
        assert_eq!(parse_credential_name(text2), None);
    }

    #[test]
    fn parse_credential_name_rejects_too_long() {
        let long = "a".repeat(65);
        let text = format!(r#"{{"credential_name":"{long}"}}"#);
        assert_eq!(parse_credential_name(&text), None);
    }

    #[test]
    fn parse_credential_name_first_match_wins_in_prose() {
        // The first occurrence is chosen — registry check downstream is what
        // gates whether it's actually honored.
        let text = "credential_name 'first_one' then credential_name 'second_one'";
        assert_eq!(parse_credential_name(text), Some("first_one".to_string()));
    }

    #[test]
    fn parse_credential_name_none_for_missing_field() {
        assert_eq!(parse_credential_name("nothing to see here"), None);
        assert_eq!(parse_credential_name(r#"{"foo":"bar"}"#), None);
    }

    /// Regression test for #2491: engine v2 must block messages containing
    /// leaked secrets (API keys, tokens) instead of forwarding them to the LLM.
    #[tokio::test]
    async fn handle_with_engine_blocks_inbound_secrets() {
        let _guard = ENGINE_STATE_TEST_LOCK.lock().await;
        let lock = ENGINE_STATE.get_or_init(|| RwLock::new(None));
        *lock.write().await = None;

        let outcome = async {
            let store = Arc::new(TestStore::new());
            let state = make_expected_test_state(store);
            *lock.write().await = Some(state);

            let (agent, _statuses) = make_test_agent_with_status_channel("web").await;

            // Slack bot token — should be caught by LeakDetector
            let secret_msg = IncomingMessage::new("web", "alice", "xoxb-1234567890-abcdefghij");
            let result = handle_with_engine_inner(&agent, &secret_msg, &secret_msg.content, 0)
                .await
                .expect("should not error");
            let warning = match result {
                BridgeOutcome::Respond(text) => text,
                other => panic!("expected Respond with warning, got: {other:?}"),
            };
            assert!(
                warning.contains("secret") || warning.contains("credential"),
                "expected secret-detection warning, got: {warning}"
            );

            // OpenAI key (regex requires 20+ chars after `sk-`)
            let sk_msg = IncomingMessage::new("web", "alice", "my key is sk-abc123def456ghi789jk");
            let result = handle_with_engine_inner(&agent, &sk_msg, &sk_msg.content, 0)
                .await
                .expect("should not error");
            let warning = match result {
                BridgeOutcome::Respond(text) => text,
                other => panic!("expected Respond with warning for OpenAI key, got: {other:?}"),
            };
            assert!(
                warning.contains("secret") || warning.contains("credential"),
                "expected secret-detection warning for OpenAI key, got: {warning}"
            );

            // Clean message should pass through (will fail at conversation
            // manager level since test state has no real engine, but it must
            // NOT be rejected by the safety checks).
            let clean_msg = IncomingMessage::new("web", "alice", "hello world");
            let result = handle_with_engine_inner(&agent, &clean_msg, &clean_msg.content, 0).await;
            // Any outcome other than a safety-rejection is fine — the test
            // store doesn't have a real conversation manager so an Err is
            // expected, but it must NOT be Ok(Respond(secret_warning)).
            if let Ok(BridgeOutcome::Respond(ref text)) = result {
                assert!(
                    !text.contains("secret") && !text.contains("credential"),
                    "clean message should not trigger secret detection, got: {text}"
                );
            }

            Ok::<(), crate::error::Error>(())
        }
        .await;

        *lock.write().await = None;
        outcome.expect("engine v2 secret scan regression test");
    }

    /// Regression test for issue #2084 upgrade path.
    ///
    /// Simulates the scenario where pre-PR on-disk docs are loaded with
    /// `user_id = "legacy"` (because frontmatter lacked the field). After
    /// `migrate_legacy_user_ids` runs, Skill docs must get `__shared__`
    /// ownership (not `owner_id`) so they remain visible via
    /// `list_skills_global()` to all tenants.
    #[tokio::test]
    async fn migrate_legacy_user_ids_preserves_shared_ownership_for_skills() {
        let store = Arc::new(TestStore::new());

        // Seed a project owned by the admin.
        let project = ironclaw_engine::Project::new("admin", "default", "test");
        store.save_project(&project).await.unwrap();

        // Seed legacy docs: a Skill and a Note, both with user_id = "legacy"
        // (simulating pre-PR deserialization fallback).
        let mut skill_doc = ironclaw_engine::MemoryDoc::new(
            project.id,
            "legacy",
            ironclaw_engine::DocType::Skill,
            "skill:bundled-tool",
            "Bundled skill content",
        );
        skill_doc.user_id = "legacy".to_string();
        store.save_memory_doc(&skill_doc).await.unwrap();

        let mut note_doc = ironclaw_engine::MemoryDoc::new(
            project.id,
            "legacy",
            ironclaw_engine::DocType::Note,
            "note:scratch",
            "Some scratch notes",
        );
        note_doc.user_id = "legacy".to_string();
        store.save_memory_doc(&note_doc).await.unwrap();

        // Run the migration.
        let store_dyn: Arc<dyn ironclaw_engine::Store> = store.clone();
        migrate_legacy_user_ids(&store_dyn, "admin").await;

        // Verify: skill doc must have shared ownership.
        let skill = store.load_memory_doc(skill_doc.id).await.unwrap().unwrap();
        assert_eq!(
            skill.user_id,
            ironclaw_engine::types::shared_owner_id(),
            "legacy Skill docs must be stamped as __shared__, not owner_id"
        );

        // Verify: non-skill doc gets owner_id as before.
        let note = store.load_memory_doc(note_doc.id).await.unwrap().unwrap();
        assert_eq!(
            note.user_id, "admin",
            "legacy non-Skill docs must be stamped with owner_id"
        );

        // Verify: the skill is discoverable via list_skills_global.
        let global_skills = store_dyn.list_skills_global().await.unwrap();
        assert!(
            global_skills.iter().any(|d| d.id == skill_doc.id),
            "shared skill must be visible via list_skills_global after migration"
        );
    }

    /// Regression test: legacy frontmatter docs without project_id.
    ///
    /// Old on-disk knowledge docs serialized before project_id/user_id were
    /// persisted in frontmatter load with project_id = nil and user_id =
    /// "legacy". The migration must find these nil-project docs, assign them
    /// to the owner's default project, and stamp the correct user_id.
    #[tokio::test]
    async fn migrate_legacy_user_ids_handles_nil_project_docs() {
        let store = Arc::new(TestStore::new());

        // Seed a project owned by the admin.
        let project = ironclaw_engine::Project::new("admin", "default", "test");
        store.save_project(&project).await.unwrap();

        // Seed docs with project_id = nil, simulating old frontmatter
        // deserialization that lacked project_id.
        let nil_pid = ironclaw_engine::ProjectId(uuid::Uuid::nil());

        let mut skill_doc = ironclaw_engine::MemoryDoc::new(
            nil_pid,
            "legacy",
            ironclaw_engine::DocType::Skill,
            "skill:old-bundled",
            "Old bundled skill from before multi-tenancy",
        );
        skill_doc.user_id = "legacy".to_string();
        store.save_memory_doc(&skill_doc).await.unwrap();

        let mut note_doc = ironclaw_engine::MemoryDoc::new(
            nil_pid,
            "legacy",
            ironclaw_engine::DocType::Note,
            "note:old-scratch",
            "Old scratch notes from before multi-tenancy",
        );
        note_doc.user_id = "legacy".to_string();
        store.save_memory_doc(&note_doc).await.unwrap();

        // Run the migration.
        let store_dyn: Arc<dyn ironclaw_engine::Store> = store.clone();
        migrate_legacy_user_ids(&store_dyn, "admin").await;

        // Verify: skill doc gets __shared__ and the owner's project.
        let skill = store.load_memory_doc(skill_doc.id).await.unwrap().unwrap();
        assert_eq!(
            skill.project_id, project.id,
            "nil-project skill must be assigned to the owner's default project"
        );
        assert_eq!(
            skill.user_id,
            ironclaw_engine::types::shared_owner_id(),
            "nil-project Skill docs must be stamped as __shared__"
        );

        // Verify: note doc gets owner_id and the owner's project.
        let note = store.load_memory_doc(note_doc.id).await.unwrap().unwrap();
        assert_eq!(
            note.project_id, project.id,
            "nil-project note must be assigned to the owner's default project"
        );
        assert_eq!(
            note.user_id, "admin",
            "nil-project non-Skill docs must be stamped with owner_id"
        );

        // Verify: no docs with nil project_id or "legacy" user_id remain.
        let remaining = store.list_memory_docs(nil_pid, "legacy").await.unwrap();
        assert!(
            remaining.is_empty(),
            "no orphaned nil-project legacy docs should remain after migration"
        );

        // Verify: the skill is discoverable via list_skills_global.
        let global_skills = store_dyn.list_skills_global().await.unwrap();
        assert!(
            global_skills.iter().any(|d| d.id == skill_doc.id),
            "migrated nil-project skill must be visible via list_skills_global"
        );
    }

    // ── persist_always_allow / revert_always_allow ─────────────────────

    /// Minimal in-memory SettingsStore for persistence tests.
    struct InMemorySettings {
        data: TokioRwLock<HashMap<String, HashMap<String, serde_json::Value>>>,
    }

    impl InMemorySettings {
        fn new() -> Self {
            Self {
                data: TokioRwLock::new(HashMap::new()),
            }
        }

        async fn get(&self, user_id: &str, key: &str) -> Option<serde_json::Value> {
            self.data
                .read()
                .await
                .get(user_id)
                .and_then(|m| m.get(key))
                .cloned()
        }
    }

    #[async_trait::async_trait]
    impl crate::db::SettingsStore for InMemorySettings {
        async fn get_setting(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<Option<serde_json::Value>, crate::error::DatabaseError> {
            Ok(self.get(user_id, key).await)
        }
        async fn get_setting_full(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<crate::history::SettingRow>, crate::error::DatabaseError> {
            Ok(None)
        }
        async fn set_setting(
            &self,
            user_id: &str,
            key: &str,
            value: &serde_json::Value,
        ) -> Result<(), crate::error::DatabaseError> {
            self.data
                .write()
                .await
                .entry(user_id.to_owned())
                .or_default()
                .insert(key.to_owned(), value.clone());
            Ok(())
        }
        async fn delete_setting(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<bool, crate::error::DatabaseError> {
            Ok(self
                .data
                .write()
                .await
                .get_mut(user_id)
                .and_then(|m| m.remove(key))
                .is_some())
        }
        async fn list_settings(
            &self,
            _: &str,
        ) -> Result<Vec<crate::history::SettingRow>, crate::error::DatabaseError> {
            Ok(vec![])
        }
        async fn get_all_settings(
            &self,
            user_id: &str,
        ) -> Result<HashMap<String, serde_json::Value>, crate::error::DatabaseError> {
            Ok(self
                .data
                .read()
                .await
                .get(user_id)
                .cloned()
                .unwrap_or_default())
        }
        async fn set_all_settings(
            &self,
            user_id: &str,
            settings: &HashMap<String, serde_json::Value>,
        ) -> Result<(), crate::error::DatabaseError> {
            self.data
                .write()
                .await
                .insert(user_id.to_owned(), settings.clone());
            Ok(())
        }
        async fn has_settings(&self, user_id: &str) -> Result<bool, crate::error::DatabaseError> {
            Ok(self
                .data
                .read()
                .await
                .get(user_id)
                .is_some_and(|m| !m.is_empty()))
        }
    }

    /// Build a minimal `EngineState` for persistence tests.
    fn make_persistence_test_state(
        tools: Arc<ToolRegistry>,
        db: Option<Arc<dyn crate::db::Database>>,
    ) -> EngineState {
        use ironclaw_engine::{
            CapabilityRegistry, ConversationManager, LeaseManager, PolicyEngine, ThreadManager,
        };

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
                    response: ironclaw_engine::LlmResponse::Text("ok".into()),
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

        let store: Arc<dyn ironclaw_engine::Store> = Arc::new(TestStore::new());
        let pending_gates = Arc::new(crate::gate::store::PendingGateStore::in_memory());
        let effect = Arc::new(crate::bridge::effect_adapter::EffectBridgeAdapter::new(
            tools,
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::new()),
        ));
        let thread_manager = Arc::new(ThreadManager::new(
            Arc::new(NoopLlm),
            Arc::new(NoopEffects),
            Arc::clone(&store),
            Arc::new(CapabilityRegistry::new()),
            Arc::new(LeaseManager::new()),
            Arc::new(PolicyEngine::new()),
        ));
        EngineState {
            conversation_manager: Arc::new(ConversationManager::new(
                Arc::clone(&thread_manager),
                Arc::clone(&store),
            )),
            thread_manager,
            effect_adapter: effect,
            store,
            default_project_id: ironclaw_engine::ProjectId::new(),
            pending_gates,
            sse: None,
            db,
            secrets_store: None,
            auth_manager: None,
            extension_manager: None,
            project_root: resolve_project_root(),
        }
    }

    /// "Always approve" persists AlwaysAllow to the settings store.
    #[tokio::test]
    async fn test_persist_always_allow_writes_to_settings() {
        let settings = Arc::new(InMemorySettings::new());
        let (agent, _) = make_router_test_agent(None).await;
        // Override the agent's settings_store by constructing new deps.
        // Since AgentDeps fields are pub(crate), we can modify via a
        // wrapper that injects the settings store.
        let mut agent = agent;
        agent.deps.settings_store =
            Some(Arc::clone(&settings) as Arc<dyn crate::db::SettingsStore + Send + Sync>);

        let tools = Arc::new(ToolRegistry::new());
        let state = make_persistence_test_state(tools, None);

        let tid = ironclaw_engine::ThreadId::new();
        let pending = sample_pending_gate(
            "user1",
            tid,
            ironclaw_engine::ResumeKind::Approval { allow_always: true },
        );

        super::persist_always_allow(&agent, &state, &pending).await;

        let val = settings.get("user1", "tool_permissions.shell").await;
        assert!(
            val.is_some(),
            "AlwaysAllow should be persisted to DB settings"
        );
        assert_eq!(
            val.unwrap(),
            serde_json::json!("always_allow"),
            "Persisted value must be the PermissionState serialization"
        );
    }

    /// "Always approve" is NOT persisted for ApprovalRequirement::Always tools
    /// (defense-in-depth — even if a crafted client sends always:true).
    #[tokio::test]
    async fn test_persist_always_allow_skips_locked_tools() {
        use crate::tools::ApprovalRequirement;

        let settings = Arc::new(InMemorySettings::new());
        let mut agent = make_router_test_agent(None).await.0;
        agent.deps.settings_store =
            Some(Arc::clone(&settings) as Arc<dyn crate::db::SettingsStore + Send + Sync>);

        // Register a tool that returns ApprovalRequirement::Always.
        struct LockedTool;
        #[async_trait::async_trait]
        impl crate::tools::Tool for LockedTool {
            fn name(&self) -> &str {
                "locked_tool"
            }
            fn description(&self) -> &str {
                "Always-locked"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
                unreachable!()
            }
            fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
                ApprovalRequirement::Always
            }
        }

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(LockedTool)).await;
        let state = make_persistence_test_state(tools, None);

        let tid = ironclaw_engine::ThreadId::new();
        let mut pending = sample_pending_gate(
            "user1",
            tid,
            ironclaw_engine::ResumeKind::Approval {
                allow_always: false,
            },
        );
        pending.action_name = "locked_tool".into();

        super::persist_always_allow(&agent, &state, &pending).await;

        let val = settings.get("user1", "tool_permissions.locked_tool").await;
        assert!(
            val.is_none(),
            "AlwaysAllow must NOT be persisted for ApprovalRequirement::Always tools"
        );
    }

    /// revert_always_allow deletes a newly-persisted setting (no prior value).
    #[tokio::test]
    async fn test_revert_always_allow_deletes_setting() {
        let settings = Arc::new(InMemorySettings::new());
        let mut agent = make_router_test_agent(None).await.0;
        agent.deps.settings_store =
            Some(Arc::clone(&settings) as Arc<dyn crate::db::SettingsStore + Send + Sync>);

        let tools = Arc::new(ToolRegistry::new());
        let state = make_persistence_test_state(tools, None);

        let tid = ironclaw_engine::ThreadId::new();
        let pending = sample_pending_gate(
            "user1",
            tid,
            ironclaw_engine::ResumeKind::Approval { allow_always: true },
        );

        let prior = super::persist_always_allow(&agent, &state, &pending).await;
        assert!(prior.is_none(), "No prior value should exist");
        assert!(
            settings
                .get("user1", "tool_permissions.shell")
                .await
                .is_some(),
            "AlwaysAllow should exist after persist"
        );

        super::revert_always_allow(&agent, &pending, prior).await;
        assert!(
            settings
                .get("user1", "tool_permissions.shell")
                .await
                .is_none(),
            "AlwaysAllow should be deleted after revert"
        );
    }

    /// revert_always_allow restores a pre-existing value instead of deleting.
    #[tokio::test]
    async fn test_revert_always_allow_restores_prior_value() {
        use crate::db::SettingsStore;

        let settings = Arc::new(InMemorySettings::new());
        SettingsStore::set_setting(
            settings.as_ref(),
            "user1",
            "tool_permissions.shell",
            &serde_json::json!("ask_each_time"),
        )
        .await
        .unwrap();

        let mut agent = make_router_test_agent(None).await.0;
        agent.deps.settings_store =
            Some(Arc::clone(&settings) as Arc<dyn crate::db::SettingsStore + Send + Sync>);

        let tools = Arc::new(ToolRegistry::new());
        let state = make_persistence_test_state(tools, None);

        let tid = ironclaw_engine::ThreadId::new();
        let pending = sample_pending_gate(
            "user1",
            tid,
            ironclaw_engine::ResumeKind::Approval { allow_always: true },
        );

        let prior = super::persist_always_allow(&agent, &state, &pending).await;
        assert_eq!(prior, Some(serde_json::json!("ask_each_time")));
        assert_eq!(
            settings.get("user1", "tool_permissions.shell").await,
            Some(serde_json::json!("always_allow")),
        );

        super::revert_always_allow(&agent, &pending, prior).await;
        assert_eq!(
            settings.get("user1", "tool_permissions.shell").await,
            Some(serde_json::json!("ask_each_time")),
            "Pre-existing preference should be restored after revert"
        );
    }

    /// persist_always_allow rejects tool names with dots or invalid chars.
    #[tokio::test]
    async fn test_persist_always_allow_rejects_invalid_tool_name() {
        let settings = Arc::new(InMemorySettings::new());
        let mut agent = make_router_test_agent(None).await.0;
        agent.deps.settings_store =
            Some(Arc::clone(&settings) as Arc<dyn crate::db::SettingsStore + Send + Sync>);

        let tools = Arc::new(ToolRegistry::new());
        let state = make_persistence_test_state(tools, None);

        let tid = ironclaw_engine::ThreadId::new();
        let mut pending = sample_pending_gate(
            "user1",
            tid,
            ironclaw_engine::ResumeKind::Approval { allow_always: true },
        );
        pending.action_name = "evil.settings.key".into();

        let prior = super::persist_always_allow(&agent, &state, &pending).await;
        assert!(prior.is_none());
        assert!(
            settings
                .get("user1", "tool_permissions.evil.settings.key")
                .await
                .is_none(),
            "Invalid tool names must not be persisted"
        );
    }

    /// persist_always_allow skips when settings_store is None (no DB
    /// fallback — the raw Database bypass breaks CachedSettingsStore
    /// cache coherence).
    #[tokio::test]
    async fn test_persist_skips_when_no_settings_store() {
        let mut agent = make_router_test_agent(None).await.0;
        agent.deps.settings_store = None;

        let tools = Arc::new(ToolRegistry::new());
        let state = make_persistence_test_state(tools, None);

        let tid = ironclaw_engine::ThreadId::new();
        let pending = sample_pending_gate(
            "user1",
            tid,
            ironclaw_engine::ResumeKind::Approval { allow_always: true },
        );

        let prior = super::persist_always_allow(&agent, &state, &pending).await;
        assert!(prior.is_none(), "Should return None when no settings_store");
    }

    // ── clamp_always_to_resume_kind ────────────────────────────────────

    #[test]
    fn clamp_approval_with_allow_always_passes_through() {
        let rk = ironclaw_engine::ResumeKind::Approval { allow_always: true };
        assert!(super::clamp_always_to_resume_kind(true, &rk));
        assert!(!super::clamp_always_to_resume_kind(false, &rk));
    }

    #[test]
    fn clamp_approval_without_allow_always_clamps_to_false() {
        // Regression: PR #1958 round-4 review — caller-supplied `always: true`
        // on an `Approval { allow_always: false }` gate (orchestrator self-
        // modify write) must not install a session-wide auto-approval.
        let rk = ironclaw_engine::ResumeKind::Approval {
            allow_always: false,
        };
        assert!(!super::clamp_always_to_resume_kind(true, &rk));
        assert!(!super::clamp_always_to_resume_kind(false, &rk));
    }

    #[test]
    fn clamp_auth_resume_kind_clamps_to_false() {
        // Auth resumes have no "always" semantics; clamp regardless.
        let rk = ironclaw_engine::ResumeKind::Authentication {
            credential_name: ironclaw_common::CredentialName::new("github_token").unwrap(),
            instructions: String::new(),
            auth_url: None,
        };
        assert!(!super::clamp_always_to_resume_kind(true, &rk));
    }

    #[test]
    fn clamp_external_callback_clamps_to_false() {
        let rk = ironclaw_engine::ResumeKind::External {
            callback_id: "cb-123".into(),
        };
        assert!(!super::clamp_always_to_resume_kind(true, &rk));
    }

    // ── persist_v2_tool_calls unit tests ────────────────────────

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn persist_v2_tool_calls_writes_action_results_from_internal_messages() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(TestStore::new());
        let db: Arc<dyn crate::db::Database> = Arc::new(
            crate::db::libsql::LibSqlBackend::new_local(&tmp.path().join("test.db"))
                .await
                .expect("local libsql"),
        );
        db.run_migrations().await.expect("migrations");

        // Create a thread with ActionResult messages in internal_messages
        let mut thread = ironclaw_engine::Thread::new(
            "goal",
            ironclaw_engine::ThreadType::Foreground,
            ironclaw_engine::ProjectId::new(),
            "test-user",
            ironclaw_engine::ThreadConfig::default(),
        );
        thread.add_internal_message(ironclaw_engine::ThreadMessage::action_result(
            "call-1",
            "echo",
            r#"{"output":"hello"}"#,
        ));
        thread.add_internal_message(ironclaw_engine::ThreadMessage::action_result(
            "call-2",
            "time",
            r#"{"time":"2026-04-15T12:00:00Z"}"#,
        ));
        let thread_id = thread.id;
        store.save_thread(&thread).await.unwrap();

        // Create a conversation so persist can resolve the v1 conversation ID
        let conv_id = db
            .create_conversation("web", "test-user", None)
            .await
            .expect("create conversation");

        let message =
            IncomingMessage::new("web", "test-user", "do stuff").with_thread(conv_id.to_string());

        let store_arc: Arc<dyn Store> = store;
        persist_v2_tool_calls(&store_arc, &db, thread_id, &message).await;

        // Read back and verify the tool_calls message was written
        let messages = db
            .list_conversation_messages(conv_id)
            .await
            .expect("list messages");
        let tool_calls_msgs: Vec<_> = messages.iter().filter(|m| m.role == "tool_calls").collect();
        assert_eq!(
            tool_calls_msgs.len(),
            1,
            "expected exactly one tool_calls row"
        );

        let parsed: serde_json::Value =
            serde_json::from_str(&tool_calls_msgs[0].content).expect("valid JSON");
        let calls = parsed["calls"].as_array().expect("calls array");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["name"], "echo");
        assert_eq!(calls[0]["tool_call_id"], "call-1");
        assert_eq!(calls[1]["name"], "time");
        assert_eq!(calls[1]["tool_call_id"], "call-2");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn persist_v2_tool_calls_skips_when_no_action_results() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(TestStore::new());
        let db: Arc<dyn crate::db::Database> = Arc::new(
            crate::db::libsql::LibSqlBackend::new_local(&tmp.path().join("test.db"))
                .await
                .expect("local libsql"),
        );
        db.run_migrations().await.expect("migrations");

        // Thread with only a user message, no action results
        let mut thread = ironclaw_engine::Thread::new(
            "goal",
            ironclaw_engine::ThreadType::Foreground,
            ironclaw_engine::ProjectId::new(),
            "test-user",
            ironclaw_engine::ThreadConfig::default(),
        );
        thread.add_internal_message(ironclaw_engine::ThreadMessage::user("hello"));
        let thread_id = thread.id;
        store.save_thread(&thread).await.unwrap();

        let conv_id = db
            .create_conversation("web", "test-user", None)
            .await
            .expect("create conversation");

        let message =
            IncomingMessage::new("web", "test-user", "hello").with_thread(conv_id.to_string());

        let store_arc: Arc<dyn Store> = store;
        persist_v2_tool_calls(&store_arc, &db, thread_id, &message).await;

        // No tool_calls row should be written
        let messages = db
            .list_conversation_messages(conv_id)
            .await
            .expect("list messages");
        let tool_calls_msgs: Vec<_> = messages.iter().filter(|m| m.role == "tool_calls").collect();
        assert_eq!(
            tool_calls_msgs.len(),
            0,
            "no tool_calls row for text-only thread"
        );
    }

    /// Regression: the truncation logic in `persist_v2_tool_calls` uses
    /// `char_indices()` + `len_utf8()` to avoid slicing in the middle of a
    /// multi-byte UTF-8 sequence. Exercise the path with a content string
    /// that is well over 500 bytes and composed entirely of 3-byte chars,
    /// where a naive `&s[..500]` would panic on a char boundary.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn persist_v2_tool_calls_truncates_multibyte_content_on_char_boundary() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(TestStore::new());
        let db: Arc<dyn crate::db::Database> = Arc::new(
            crate::db::libsql::LibSqlBackend::new_local(&tmp.path().join("test.db"))
                .await
                .expect("local libsql"),
        );
        db.run_migrations().await.expect("migrations");

        // Content: 400 repetitions of a 3-byte CJK char = 1200 bytes, well
        // over the 500-byte truncation threshold.
        let big = "日".repeat(400);
        assert!(big.len() > 500, "fixture must exceed threshold");

        let mut thread = ironclaw_engine::Thread::new(
            "goal",
            ironclaw_engine::ThreadType::Foreground,
            ironclaw_engine::ProjectId::new(),
            "test-user",
            ironclaw_engine::ThreadConfig::default(),
        );
        thread.add_internal_message(ironclaw_engine::ThreadMessage::action_result(
            "call-utf8",
            "echo",
            &big,
        ));
        let thread_id = thread.id;
        store.save_thread(&thread).await.unwrap();

        let conv_id = db
            .create_conversation("web", "test-user", None)
            .await
            .expect("create conversation");
        let message =
            IncomingMessage::new("web", "test-user", "hi").with_thread(conv_id.to_string());

        let store_arc: Arc<dyn Store> = store;
        persist_v2_tool_calls(&store_arc, &db, thread_id, &message).await;

        let messages = db
            .list_conversation_messages(conv_id)
            .await
            .expect("list messages");
        let tool_calls_msg = messages
            .iter()
            .find(|m| m.role == "tool_calls")
            .expect("tool_calls row must be written");
        let parsed: serde_json::Value =
            serde_json::from_str(&tool_calls_msg.content).expect("valid JSON");
        let preview = parsed["calls"][0]["result_preview"]
            .as_str()
            .expect("result_preview is a string");

        // The preview must be valid UTF-8 (JSON deserialization above would
        // have failed otherwise), must end with the truncation marker, and
        // must be bounded. The truncation rule is "include every char
        // whose *start index* is below 500", so for multi-byte runs the
        // last included char may extend past byte 500 by up to
        // `max_char_bytes - 1` bytes (≤3 for UTF-8). That's the correct
        // behavior — pinning a generous bound is what the char-boundary
        // safety actually provides.
        assert!(
            preview.ends_with("..."),
            "expected truncation marker, got: {preview:?}"
        );
        let body = preview.strip_suffix("...").unwrap();
        assert!(
            body.len() < 504,
            "body must be bounded near 500 bytes (≤500+char_overhead), got {}: {body:?}",
            body.len()
        );
        assert!(
            body.chars().all(|c| c == '日'),
            "body must contain only complete 3-byte chars, got: {body:?}"
        );
        // And critically: body.len() must be on a char boundary. If the
        // truncator sliced mid-char, the earlier JSON parse would have
        // rejected invalid UTF-8; this assertion is belt-and-braces.
        assert!(
            body.len().is_multiple_of(3),
            "body length must be a multiple of 3-byte char width, got {}",
            body.len()
        );
    }

    /// Regression for the bug fixed in commit 652315e8: `persist_v2_tool_calls`
    /// must only be called from the `ThreadOutcome::Completed` arm. If a
    /// future refactor moves the call out of that arm, partial tool
    /// executions on `GatePaused` would orphan a `role="tool_calls"` DB row
    /// that then duplicates when the gate resumes. Pin the call-site
    /// conditional by inspecting the source of `await_thread_outcome`.
    #[test]
    fn persist_v2_tool_calls_only_called_from_completed_arm() {
        let source = include_str!("router.rs");
        let (before_fn, _after_fn) = source
            .split_once("async fn persist_v2_tool_calls")
            .expect("persist_v2_tool_calls must exist in router.rs");

        // There should be exactly one call site in the pre-definition body
        // (the call inside `await_thread_outcome`). The text below the
        // definition is allowed to reference it (doc comments, unit tests).
        let call_sites = before_fn.matches("persist_v2_tool_calls(").count();
        assert_eq!(
            call_sites, 1,
            "expected exactly one call site for persist_v2_tool_calls, found {call_sites}"
        );

        // The call must live inside `ThreadOutcome::Completed` and must not
        // appear in any of the terminal arms that represent non-completion
        // outcomes. `GatePaused` is the one that triggered the bug.
        let completed_idx = before_fn
            .find("ThreadOutcome::Completed")
            .expect("Completed arm must exist");
        let gate_paused_idx = before_fn
            .find("ThreadOutcome::GatePaused")
            .expect("GatePaused arm must exist");
        let call_idx = before_fn
            .find("persist_v2_tool_calls(")
            .expect("call site must exist");

        assert!(
            completed_idx < call_idx && call_idx < gate_paused_idx,
            "persist_v2_tool_calls call must sit between Completed and GatePaused arms, got \
             completed={completed_idx} call={call_idx} gate_paused={gate_paused_idx}"
        );
    }

    // ── resume_lease_for_pending_gate tests ────────────────────
    //
    // Pins the PR #2631 review ask: when resuming a paused gate, we
    // prefer the `paused_lease` snapshot the gate recorded. The snapshot
    // MUST be validated (thread_id match, action coverage, not revoked,
    // not expired) before use — a stale snapshot falling through silently
    // would bypass revocation semantics that `find_lease_for_action`
    // normally enforces. These tests drive the helper end-to-end to pin
    // every branch of the decision.

    fn sample_lease_for_pending(pending: &PendingGate) -> ironclaw_engine::CapabilityLease {
        ironclaw_engine::CapabilityLease {
            id: ironclaw_engine::types::capability::LeaseId::new(),
            thread_id: pending.thread_id,
            capability_name: "tools".into(),
            granted_actions: ironclaw_engine::GrantedActions::Specific(vec![
                pending.action_name.clone(),
            ]),
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        }
    }

    #[tokio::test]
    async fn resume_lease_prefers_snapshot_even_when_lease_manager_empty() {
        // Reproduces the original bug: the LeaseManager has no active
        // lease for the paused action (lease evicted or never persisted
        // through restart), but the pending gate carries a snapshot. The
        // resume must use the snapshot.
        let thread_id = ironclaw_engine::ThreadId::new();
        let mut pending = sample_pending_gate(
            "alice",
            thread_id,
            ironclaw_engine::ResumeKind::Approval {
                allow_always: false,
            },
        );
        let snapshot = sample_lease_for_pending(&pending);
        let snapshot_id = snapshot.id;
        pending.paused_lease = Some(snapshot);

        let leases = ironclaw_engine::LeaseManager::new();
        // Intentionally empty — no lease for this thread/action.

        let lease = resume_lease_for_pending_gate(&pending, &leases)
            .await
            .expect("snapshot must be used when LeaseManager has nothing");
        assert_eq!(lease.id, snapshot_id, "should return the snapshot lease");
    }

    #[tokio::test]
    async fn resume_lease_rejects_revoked_snapshot_and_falls_back() {
        // A revoked snapshot must NOT resume the action. Fall back to
        // the LeaseManager; if that has a valid lease, use it.
        let thread_id = ironclaw_engine::ThreadId::new();
        let mut pending = sample_pending_gate(
            "alice",
            thread_id,
            ironclaw_engine::ResumeKind::Approval {
                allow_always: false,
            },
        );
        let mut revoked_snapshot = sample_lease_for_pending(&pending);
        revoked_snapshot.revoked = true;
        revoked_snapshot.revoked_reason = Some("user revoked mid-pause".into());
        pending.paused_lease = Some(revoked_snapshot);

        let leases = ironclaw_engine::LeaseManager::new();
        let live_lease = leases
            .grant(
                thread_id,
                "tools",
                ironclaw_engine::GrantedActions::Specific(vec!["shell".into()]),
                None,
                None,
            )
            .await
            .expect("grant live lease");

        let lease = resume_lease_for_pending_gate(&pending, &leases)
            .await
            .expect("fallback lease must be found");
        assert_eq!(
            lease.id, live_lease.id,
            "revoked snapshot must be skipped and LeaseManager fallback used"
        );
    }

    #[tokio::test]
    async fn resume_lease_rejects_expired_snapshot_and_falls_back() {
        // An expired snapshot must not be accepted even if all other
        // fields check out.
        let thread_id = ironclaw_engine::ThreadId::new();
        let mut pending = sample_pending_gate(
            "alice",
            thread_id,
            ironclaw_engine::ResumeKind::Approval {
                allow_always: false,
            },
        );
        let mut expired_snapshot = sample_lease_for_pending(&pending);
        expired_snapshot.expires_at = Some(chrono::Utc::now() - chrono::Duration::minutes(1));
        pending.paused_lease = Some(expired_snapshot);

        let leases = ironclaw_engine::LeaseManager::new();
        let live_lease = leases
            .grant(
                thread_id,
                "tools",
                ironclaw_engine::GrantedActions::Specific(vec!["shell".into()]),
                None,
                None,
            )
            .await
            .expect("grant live lease");

        let lease = resume_lease_for_pending_gate(&pending, &leases)
            .await
            .expect("fallback lease must be found");
        assert_eq!(lease.id, live_lease.id, "expired snapshot must be skipped");
    }

    #[tokio::test]
    async fn resume_lease_rejects_snapshot_with_wrong_thread_id() {
        // Defensive: a snapshot whose thread_id doesn't match the pending
        // gate's thread_id must never be trusted, even if other fields
        // look valid. Guards against pending-gate-store drift or future
        // refactors that reuse a snapshot across threads.
        let thread_id = ironclaw_engine::ThreadId::new();
        let mut pending = sample_pending_gate(
            "alice",
            thread_id,
            ironclaw_engine::ResumeKind::Approval {
                allow_always: false,
            },
        );
        let mut mismatched_snapshot = sample_lease_for_pending(&pending);
        mismatched_snapshot.thread_id = ironclaw_engine::ThreadId::new(); // different thread
        pending.paused_lease = Some(mismatched_snapshot);

        let leases = ironclaw_engine::LeaseManager::new();
        // Empty — no fallback.
        let result = resume_lease_for_pending_gate(&pending, &leases).await;
        assert!(
            result.is_none(),
            "mismatched-thread snapshot must be skipped and fallback must fail cleanly"
        );
    }

    #[tokio::test]
    async fn resume_lease_rejects_snapshot_missing_action_coverage() {
        // A snapshot whose granted_actions does not cover the pending
        // action name must not be used, even when untargeted.
        let thread_id = ironclaw_engine::ThreadId::new();
        let mut pending = sample_pending_gate(
            "alice",
            thread_id,
            ironclaw_engine::ResumeKind::Approval {
                allow_always: false,
            },
        );
        let mut mismatched_snapshot = sample_lease_for_pending(&pending);
        mismatched_snapshot.granted_actions =
            ironclaw_engine::GrantedActions::Specific(vec!["unrelated_tool".into()]);
        pending.paused_lease = Some(mismatched_snapshot);

        let leases = ironclaw_engine::LeaseManager::new();
        let result = resume_lease_for_pending_gate(&pending, &leases).await;
        assert!(
            result.is_none(),
            "snapshot must be skipped when it doesn't grant the pending action"
        );
    }

    #[tokio::test]
    async fn resume_lease_returns_none_when_no_snapshot_and_no_active_lease() {
        // Sanity: if there's nothing in either path, the helper returns
        // None so the caller can map to a clean "no active lease" error.
        let thread_id = ironclaw_engine::ThreadId::new();
        let pending = sample_pending_gate(
            "alice",
            thread_id,
            ironclaw_engine::ResumeKind::Approval {
                allow_always: false,
            },
        );
        let leases = ironclaw_engine::LeaseManager::new();
        assert!(
            resume_lease_for_pending_gate(&pending, &leases)
                .await
                .is_none()
        );
    }
}
