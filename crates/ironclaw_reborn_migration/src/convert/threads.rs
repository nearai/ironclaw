//! Conversation-history → Reborn session-thread converter.
//!
//! Each v1 `conversations` row becomes a Reborn `SessionThreadRecord` (original
//! id preserved via `EnsureThreadRequest.thread_id`); its `conversation_messages`
//! become transcript messages in order through `SessionThreadService`. Because
//! the append APIs assign their own timestamps and carry no per-message
//! metadata, the original per-message `(role, created_at, id)` provenance is
//! preserved in the thread's `metadata_json` under a `legacy_v1` key. Roles
//! without a first-class append port retain their content there as an archive;
//! user and assistant content lives in first-class transcript rows.
//!
//! The reusable [`write_thread`] helper is also used by the automations
//! converter to migrate engine-v2 mission threads.

use chrono::{DateTime, Utc};
use ironclaw_host_api::{MissionId, ProjectId, ThreadId, UserId};
use ironclaw_threads::{
    AcceptInboundMessageRequest, AppendFinalizedAssistantMessageRequest, EnsureThreadRequest,
    LoadContextMessagesRequest, MessageContent, MessageKind, MessageStatus, ThreadScope,
};
use serde_json::json;
use uuid::Uuid;

use crate::error::MigrationError;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;
use crate::target::ids;

/// Normalized role of a source transcript message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportRole {
    User,
    Assistant,
    /// System / tool / anything without a first-class Reborn append path.
    Other,
}

impl ImportRole {
    pub(crate) fn from_v1(role: &str) -> Self {
        match role.to_ascii_lowercase().as_str() {
            "user" => ImportRole::User,
            "assistant" => ImportRole::Assistant,
            _ => ImportRole::Other,
        }
    }
}

/// One source transcript message to migrate.
pub(crate) struct ImportMessage {
    pub(crate) role: ImportRole,
    pub(crate) raw_role: String,
    pub(crate) content: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) orig_id: Option<String>,
}

/// A source conversation/thread to migrate into one Reborn thread.
pub(crate) struct ThreadImport {
    /// Original id, preserved as the Reborn `ThreadId`.
    pub(crate) thread_id: Uuid,
    pub(crate) owner_user: String,
    pub(crate) title: Option<String>,
    pub(crate) project_id: Option<Uuid>,
    pub(crate) mission_id: Option<Uuid>,
    /// Provenance stored on the thread (channel, timestamps, source kind…).
    pub(crate) provenance: serde_json::Value,
    pub(crate) messages: Vec<ImportMessage>,
}

pub(crate) async fn run(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let users = src.distinct_users().await?;
    for user_id in &users {
        let conversations = src
            .db
            .list_conversations_all_channels(user_id, i64::MAX)
            .await
            .map_err(|e| MigrationError::ReadSource {
                domain: "conversations".into(),
                reason: e.to_string(),
            })?;

        for conv in conversations {
            let messages = src
                .db
                .list_conversation_messages(conv.id)
                .await
                .map_err(|e| MigrationError::ReadSource {
                    domain: "conversation_messages".into(),
                    reason: e.to_string(),
                })?;

            let import = ThreadImport {
                thread_id: conv.id,
                owner_user: user_id.clone(),
                title: conv.title.clone(),
                project_id: None,
                mission_id: None,
                provenance: json!({
                    "channel": conv.channel,
                    "thread_type": conv.thread_type,
                    "started_at": conv.started_at.to_rfc3339(),
                    "last_activity": conv.last_activity.to_rfc3339(),
                    "source": "v1_conversation",
                }),
                messages: messages
                    .into_iter()
                    .map(|m| ImportMessage {
                        role: ImportRole::from_v1(&m.role),
                        raw_role: m.role,
                        content: m.content,
                        created_at: m.created_at,
                        orig_id: Some(m.id.to_string()),
                    })
                    .collect(),
            };

            if options.dry_run {
                report.stats.threads += 1;
                report.stats.messages += import
                    .messages
                    .iter()
                    .filter(|m| m.role != ImportRole::Other)
                    .count();
                record_other_role_losses(report, &import);
            } else {
                write_thread(tgt, options, report, import).await?;
            }
        }
    }
    Ok(())
}

/// Write one source thread into Reborn, preserving id, ordering, role, content,
/// and original per-message timestamps (the latter in `metadata_json`). Shared
/// by conversation and mission-thread migration.
pub(crate) async fn write_thread(
    tgt: &RebornTarget,
    _options: &MigrationOptions,
    report: &mut MigrationReport,
    import: ThreadImport,
) -> Result<(), MigrationError> {
    // Malformed identity on one source thread is a per-item loss, not a run
    // abort: record it and skip this thread so the rest of the migration
    // continues (`run_migration` must not be short-circuited by one bad row).
    let owner_user = match UserId::new(import.owner_user.clone()) {
        Ok(owner_user) => owner_user,
        Err(e) => {
            record_thread_id_loss(report, &import.thread_id, "owner_user_id", e.to_string());
            return Ok(());
        }
    };
    let thread_id = match ThreadId::new(import.thread_id.to_string()) {
        Ok(thread_id) => thread_id,
        Err(e) => {
            record_thread_id_loss(report, &import.thread_id, "thread_id", e.to_string());
            return Ok(());
        }
    };
    let mission_id = match import.mission_id {
        Some(id) => match MissionId::new(id.to_string()) {
            Ok(mission_id) => Some(mission_id),
            Err(e) => {
                record_thread_id_loss(report, &import.thread_id, "mission_id", e.to_string());
                return Ok(());
            }
        },
        None => None,
    };
    let project_id = match import.project_id {
        Some(id) => match ProjectId::new(id.to_string()) {
            Ok(project_id) => Some(project_id),
            Err(e) => {
                record_thread_id_loss(report, &import.thread_id, "project_id", e.to_string());
                return Ok(());
            }
        },
        None => None,
    };

    let scope = ThreadScope {
        tenant_id: tgt.tenant_id.clone(),
        agent_id: tgt.agent_id.clone(),
        project_id,
        owner_user_id: Some(owner_user),
        mission_id,
    };
    let actor_id = scope
        .owner_user_id
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();

    let metadata_json = build_metadata_json(&import)?;
    let migration_identity = ids::MigrationIdentity::from_report(report)?;

    let persisted_thread = tgt
        .thread_service
        .ensure_thread(EnsureThreadRequest {
            scope: scope.clone(),
            thread_id: Some(thread_id.clone()),
            created_by_actor_id: actor_id.clone(),
            title: import.title.clone(),
            metadata_json: Some(metadata_json.clone()),
        })
        .await
        .map_err(|e| write_err("thread", &import.thread_id, e.to_string()))?;
    if persisted_thread.created_by_actor_id != actor_id
        || persisted_thread.title != import.title
        || persisted_thread.metadata_json.as_deref() != Some(metadata_json.as_str())
    {
        return Err(write_err(
            "thread",
            &import.thread_id,
            "deterministic thread id already contains divergent source data; refusing to overwrite"
                .to_string(),
        ));
    }

    let source_binding_id = migration_identity.thread_source_binding(import.thread_id);
    for (message_index, message) in import.messages.into_iter().enumerate() {
        let stable_message_key = migration_identity.message_key(
            import.thread_id,
            message_index,
            message.orig_id.as_deref(),
        );
        match message.role {
            ImportRole::User => {
                let source_content = message.content;
                let accepted = tgt
                    .thread_service
                    .accept_inbound_message(AcceptInboundMessageRequest {
                        scope: scope.clone(),
                        thread_id: thread_id.clone(),
                        actor_id: actor_id.clone(),
                        source_binding_id: Some(source_binding_id.clone()),
                        reply_target_binding_id: None,
                        external_event_id: Some(stable_message_key),
                        content: MessageContent::text(source_content.clone()),
                    })
                    .await
                    .map_err(|e| write_err("message", &import.thread_id, e.to_string()))?;
                if accepted.idempotent_replay {
                    let replay = tgt
                        .thread_service
                        .load_context_messages(LoadContextMessagesRequest {
                            scope: scope.clone(),
                            thread_id: thread_id.clone(),
                            message_ids: vec![accepted.message_id],
                        })
                        .await
                        .map_err(|e| write_err("message", &import.thread_id, e.to_string()))?;
                    let exact_replay = replay.messages.iter().any(|persisted| {
                        persisted.message_id == Some(accepted.message_id)
                            && persisted.kind == MessageKind::User
                            && persisted.content == source_content
                    });
                    if !exact_replay {
                        return Err(write_err(
                            "message",
                            &import.thread_id,
                            "idempotency key already contains divergent user-message data; refusing to overwrite"
                                .to_string(),
                        ));
                    }
                }
                report.stats.messages += 1;
            }
            ImportRole::Assistant => {
                let source_content = message.content;
                let persisted = tgt
                    .thread_service
                    .append_finalized_assistant_message(AppendFinalizedAssistantMessageRequest {
                        scope: scope.clone(),
                        thread_id: thread_id.clone(),
                        turn_run_id: stable_message_key,
                        content: MessageContent::text(source_content.clone()),
                    })
                    .await
                    .map_err(|e| write_err("message", &import.thread_id, e.to_string()))?;
                if persisted.kind != MessageKind::Assistant
                    || persisted.status != MessageStatus::Finalized
                    || persisted.content.as_deref() != Some(source_content.as_str())
                {
                    return Err(write_err(
                        "message",
                        &import.thread_id,
                        "deterministic assistant-message key already contains divergent data; refusing to overwrite"
                            .to_string(),
                    ));
                }
                report.stats.messages += 1;
            }
            ImportRole::Other => {
                // No first-class Reborn append path for system/tool transcript
                // messages. Content is retained in `legacy_v1.messages` (built
                // above), but it does not become a standalone transcript row.
                record_other_role_loss(report, import.thread_id, &message.raw_role);
            }
        }
    }

    report.stats.threads += 1;
    Ok(())
}

fn build_metadata_json(import: &ThreadImport) -> Result<String, MigrationError> {
    let legacy_messages: Vec<serde_json::Value> = import
        .messages
        .iter()
        .map(|m| {
            json!({
                "role": m.raw_role,
                "created_at": m.created_at.to_rfc3339(),
                "orig_id": m.orig_id,
                // User/assistant content lives in first-class transcript rows.
                // Roles without an append port are archived here so their
                // payload is never silently reduced to metadata-only claims.
                "archived_content": (m.role == ImportRole::Other).then_some(&m.content),
            })
        })
        .collect();
    serde_json::to_string(&json!({
        "legacy_v1": {
            "orig_thread_id": import.thread_id.to_string(),
            "provenance": import.provenance,
            "messages": legacy_messages,
        }
    }))
    .map_err(MigrationError::Serde)
}

pub(crate) fn record_other_role_losses(report: &mut MigrationReport, import: &ThreadImport) {
    for message in &import.messages {
        if message.role == ImportRole::Other {
            record_other_role_loss(report, import.thread_id, &message.raw_role);
        }
    }
}

/// Record a thread skipped because one of its identity fields
/// (`owner_user_id` / `thread_id` / `mission_id`) is not a valid Reborn id.
fn record_thread_id_loss(
    report: &mut MigrationReport,
    thread_id: &Uuid,
    field: &str,
    reason: String,
) {
    report.record_loss(
        Domain::Thread,
        thread_id.to_string(),
        field,
        LossReason::Unparseable,
        format!("v1 thread {field} is not a valid Reborn id (thread skipped): {reason}"),
    );
}

fn record_other_role_loss(report: &mut MigrationReport, thread_id: Uuid, raw_role: &str) {
    report.record_loss(
        Domain::Message,
        thread_id.to_string(),
        format!("role={raw_role}"),
        LossReason::Degraded,
        "no Reborn append path for non-user/assistant transcript roles; content \
         retained in thread metadata legacy_v1.messages"
            .to_string(),
    );
}

fn write_err(what: &str, id: &Uuid, reason: String) -> MigrationError {
    MigrationError::WriteTarget {
        domain: format!("{what} for thread {id}"),
        reason,
    }
}
