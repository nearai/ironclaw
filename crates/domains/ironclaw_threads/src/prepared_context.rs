//! The prepared-context accept door's contract types and shared policy helpers.
//!
//! `accept_prepared_context` is the ONE shared accept door for every
//! non-channel caller (subagent spawn, OpenAI-compat, suggestions, future
//! unbound features — docs/internal/design/2026-08-12-unbound-turns.md
//! §4.2): it mints an unbound thread, seeds the caller's complete
//! point-in-time context as ordinary transcript rows, journals the per-run
//! declarations beside them, and is idempotent by `idempotency_key` — a
//! crash-retry returns the SAME prepared context instead of minting an
//! orphan. The helpers here are shared by both backends (the in-memory and
//! filesystem services) so the mint/seed/replay policy cannot drift.
//!
//! Replay discipline: the thread id and every seeded message id are pure
//! deterministic functions of `(scope, idempotency_key)`, so a partially
//! crashed accept re-runs to the same rows (existing rows are skipped) and
//! the journaled [`PreparedContextRecord`] — written last — is the commit
//! marker a replay returns from.

use chrono::{DateTime, Utc};
use ironclaw_host_api::ids::ThreadId;
use ironclaw_host_api::prepared_context::PreparedTurnDeclarations;
use ironclaw_host_api::turn::AcceptedMessageRef;
use ironclaw_llm::agent_message::{
    AGENT_MESSAGE_TEXT_PART_MAX_BYTES, AgentMessage, AgentMessageRole, ContentPart,
    validate_agent_messages,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    AttachmentRef, MessageKind, MessageStatus, SessionThreadError, ThreadMessageId,
    ThreadMessageRecord, ThreadScope,
};

/// Current schema version for [`PreparedContextRecord`] rows.
pub const PREPARED_CONTEXT_RECORD_SCHEMA_VERSION: u32 = 1;

/// The prepared-context accept request.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedContextRequest {
    /// Scope of the minted thread. `owner_user_id: None` is the unbound
    /// default — the thread is structurally invisible to every owner-scoped
    /// conversation listing. `Some(owner)` exists solely to preserve the
    /// subagent child thread's owner-mirroring behavior (its evidence checks
    /// read the child edge back under the parent's real-owner scope);
    /// product callers must pass `None`.
    pub scope: ThreadScope,
    /// Acting identity recorded on the seeded user rows
    /// (run-acts-as-invoker).
    pub actor_id: String,
    /// The caller's task prompt; seeded as a `System` row when non-empty.
    /// The resolved profile may prepend host protocol assets at
    /// materialization — callers own the task prompt, not the host frame.
    pub system_prompt: String,
    /// Complete point-in-time input, seeded as the thread's rows. Must be
    /// non-empty: the last message is the accepted pin the submit carries.
    pub messages: Vec<AgentMessage>,
    /// Journaled beside the content; read at admission to derive the
    /// unbound profile and at host build to enforce the output contract.
    pub declarations: PreparedTurnDeclarations,
    /// Replay key: retrying returns the same prepared context.
    pub idempotency_key: String,
    /// Explicit thread id override. `None` (the norm) derives a deterministic
    /// id from `(scope, idempotency_key)`; the subagent spawn path passes its
    /// own `subagent-{child_run_id}` id because crash reconstruction and the
    /// await-edge machinery reference that scheme.
    pub thread_id: Option<ThreadId>,
    /// Optional human-facing title and metadata for the minted thread (the
    /// subagent path stores its crash-reconstruction metadata here).
    pub title: Option<String>,
    pub metadata_json: Option<String>,
}

/// The prepared-context pin the workflow then submits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedPreparedContext {
    pub thread_id: ThreadId,
    /// Pin for `submit_turn`: `msg:{last seeded message id}`.
    pub accepted_message_ref: AcceptedMessageRef,
    pub idempotent_replay: bool,
}

/// Durable journal record for a prepared context, stored beside the
/// thread. Written LAST during the accept, so its presence is the commit
/// marker replays return from; admission and the loop host read the
/// declarations back through it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparedContextRecord {
    pub schema_version: u32,
    pub idempotency_key: String,
    pub actor_id: String,
    /// Wire form of the accepted pin (`msg:{message_id}`).
    pub accepted_message_ref: String,
    #[serde(default)]
    pub declarations: PreparedTurnDeclarations,
    pub seeded_message_count: u64,
    pub created_at: DateTime<Utc>,
}

/// Deterministic thread id for a prepared-context accept: a pure function of
/// `(scope, idempotency_key)`, so a crash-retry converges on the same thread
/// (no orphans) and the same key under a different scope mints a different
/// thread.
pub(crate) fn prepared_thread_id(
    request: &PreparedContextRequest,
) -> Result<ThreadId, SessionThreadError> {
    if let Some(thread_id) = &request.thread_id {
        return Ok(thread_id.clone());
    }
    unbound_thread_id(&request.scope, &request.idempotency_key)
}

pub(crate) fn unbound_thread_id(
    scope: &ThreadScope,
    idempotency_key: &str,
) -> Result<ThreadId, SessionThreadError> {
    let scope_bytes = serde_json::to_vec(scope)
        .map_err(|error| SessionThreadError::Serialization(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(b"prepared-context:v1\0");
    hasher.update(&scope_bytes);
    hasher.update(b"\0");
    hasher.update(idempotency_key.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    ThreadId::new(format!("unbound-{hex}")).map_err(|error| {
        SessionThreadError::GeneratedThreadId(format!("unbound thread id invalid: {error}"))
    })
}

/// Deterministic message id for seeded row `index` of a unbound thread, so
/// a crashed retry re-writes the same rows instead of duplicating them
/// (precedent: the capability display preview's derived message ids).
pub(crate) fn prepared_seed_message_id(thread_id: &ThreadId, index: usize) -> ThreadMessageId {
    let mut hasher = Sha256::new();
    hasher.update(b"prepared-context-seed:v1\0");
    hasher.update(thread_id.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(index.to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    ThreadMessageId::from_uuid(Uuid::from_bytes(bytes))
}

pub(crate) fn accepted_prepared_message_ref(
    message_id: ThreadMessageId,
) -> Result<AcceptedMessageRef, SessionThreadError> {
    AcceptedMessageRef::new(format!("msg:{message_id}")).map_err(|error| {
        SessionThreadError::Backend(format!("accepted unbound message ref invalid: {error}"))
    })
}

fn invalid(reason: impl Into<String>) -> SessionThreadError {
    SessionThreadError::InvalidPreparedContext {
        reason: reason.into(),
    }
}

/// Validate the request fail-closed before any state is minted.
pub(crate) fn validate_prepared_context_request(
    request: &PreparedContextRequest,
) -> Result<(), SessionThreadError> {
    if request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 256
        || request.idempotency_key.chars().any(char::is_control)
    {
        return Err(invalid(
            "idempotency_key must be 1..=256 bytes with no control characters",
        ));
    }
    if request.actor_id.is_empty() {
        return Err(invalid("actor_id must not be empty"));
    }
    if request.messages.is_empty() {
        return Err(invalid(
            "messages must not be empty; the last message is the accepted pin",
        ));
    }
    if request.system_prompt.len() > AGENT_MESSAGE_TEXT_PART_MAX_BYTES {
        return Err(invalid(format!(
            "system_prompt exceeds {AGENT_MESSAGE_TEXT_PART_MAX_BYTES} bytes"
        )));
    }
    validate_agent_messages(&request.messages)
        .map_err(|error| invalid(format!("invalid message list: {error}")))?;
    // Seeding constraint (first increment): tool-interaction and reasoning
    // parts are valid vocabulary but have no faithful seeded-transcript
    // representation yet — they land with the OpenAI-compat adoption.
    // Rejected loudly here so nothing is silently dropped.
    for message in &request.messages {
        for part in &message.content {
            match part {
                ContentPart::ToolCall(_) | ContentPart::ToolResult(_) => {
                    return Err(invalid(
                        "tool-interaction parts are not seedable yet; \
                         land tool history with the OpenAI-compat adoption",
                    ));
                }
                ContentPart::Reasoning { .. } => {
                    return Err(invalid(
                        "reasoning parts are not seedable yet; \
                         land provider reasoning with the OpenAI-compat adoption",
                    ));
                }
                ContentPart::Text { .. } | ContentPart::Image { .. } | ContentPart::File { .. } => {
                }
            }
        }
    }
    Ok(())
}

fn seeded_text_and_attachments(parts: &[ContentPart]) -> (String, Vec<AttachmentRef>) {
    let mut sections: Vec<&str> = Vec::new();
    let mut attachments = Vec::new();
    for part in parts {
        match part {
            ContentPart::Text { text } => sections.push(text),
            ContentPart::Image { attachment } | ContentPart::File { attachment } => {
                attachments.push(attachment.clone());
            }
            // Rejected by validation above; unreachable by construction.
            ContentPart::ToolCall(_)
            | ContentPart::ToolResult(_)
            | ContentPart::Reasoning { .. } => {}
        }
    }
    (sections.join("\n\n"), attachments)
}

/// Build the seeded transcript rows (validation must already have passed).
/// Sequences are left at 0 for the backend to assign in order; ids and
/// timestamps are final.
pub(crate) fn prepared_seed_rows(
    request: &PreparedContextRequest,
    thread_id: &ThreadId,
    now: DateTime<Utc>,
) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
    let mut rows = Vec::with_capacity(request.messages.len() + 1);
    let mut index = 0usize;
    let mut push_row = |kind: MessageKind,
                        status: MessageStatus,
                        actor_id: Option<String>,
                        content: String,
                        attachments: Vec<AttachmentRef>| {
        rows.push(ThreadMessageRecord {
            message_id: prepared_seed_message_id(thread_id, index),
            thread_id: thread_id.clone(),
            sequence: 0,
            kind,
            status,
            created_at: Some(now),
            updated_at: Some(now),
            actor_id,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(content),
            attachments,
            redaction_ref: None,
        });
        index += 1;
    };

    if !request.system_prompt.is_empty() {
        push_row(
            MessageKind::System,
            MessageStatus::Finalized,
            None,
            request.system_prompt.clone(),
            Vec::new(),
        );
    }
    for message in &request.messages {
        let (content, attachments) = seeded_text_and_attachments(&message.content);
        crate::contract::validate_attachment_refs(&attachments)?;
        match message.role {
            AgentMessageRole::User => push_row(
                MessageKind::User,
                MessageStatus::Accepted,
                Some(request.actor_id.clone()),
                content,
                attachments,
            ),
            AgentMessageRole::Assistant => push_row(
                MessageKind::Assistant,
                MessageStatus::Finalized,
                None,
                content,
                attachments,
            ),
            // Rejected by validation above; unreachable by construction.
            AgentMessageRole::Tool => {
                return Err(invalid("tool messages are not seedable yet"));
            }
        }
    }
    Ok(rows)
}

/// Validate a stored record against a replaying request. Returns the replay
/// response or the typed mismatch.
pub(crate) fn replay_prepared_context(
    record: &PreparedContextRecord,
    request: &PreparedContextRequest,
    thread_id: &ThreadId,
) -> Result<AcceptedPreparedContext, SessionThreadError> {
    if record.idempotency_key != request.idempotency_key {
        return Err(SessionThreadError::PreparedContextKeyMismatch {
            thread_id: thread_id.clone(),
        });
    }
    if record.actor_id != request.actor_id {
        return Err(SessionThreadError::IdempotentReplayActorMismatch {
            stored_actor_id: record.actor_id.clone(),
            requested_actor_id: request.actor_id.clone(),
        });
    }
    let accepted_message_ref = AcceptedMessageRef::new(record.accepted_message_ref.clone())
        .map_err(|error| {
            SessionThreadError::Backend(format!(
                "stored unbound context carries an invalid accepted ref: {error}"
            ))
        })?;
    Ok(AcceptedPreparedContext {
        thread_id: thread_id.clone(),
        accepted_message_ref,
        idempotent_replay: true,
    })
}

/// Map a turn scope onto the thread scope its prepared-context record lives
/// under. `None` when the scope cannot name a prepared thread at all: threads
/// physically require an agent axis, and an actor-fallback owner cannot be
/// resolved without an actor.
pub(crate) fn prepared_thread_scope_for_turn_scope(
    scope: &ironclaw_host_api::turn::TurnScope,
) -> Option<ThreadScope> {
    use ironclaw_host_api::turn::TurnThreadOwner;
    let agent_id = scope.agent_id.clone()?;
    let owner_user_id = match &scope.thread_owner {
        TurnThreadOwner::Ownerless => None,
        TurnThreadOwner::ExplicitUser { owner_user_id } => Some(owner_user_id.clone()),
        TurnThreadOwner::ActorFallback => return None,
    };
    Some(ThreadScope {
        tenant_id: scope.tenant_id.clone(),
        agent_id,
        project_id: scope.project_id.clone(),
        owner_user_id,
        mission_id: None,
    })
}

/// Host-build-side read of an ADMITTED run's journaled declarations, keyed by
/// the run's own thread. Unlike the admission probe below, no ref match is
/// required: the coordinator already pinned the submission to the prepared
/// context, so the thread IS the identity here.
pub async fn read_declarations_for_run_scope(
    thread_service: &dyn crate::SessionThreadService,
    scope: &ironclaw_host_api::turn::TurnScope,
) -> Result<Option<PreparedTurnDeclarations>, SessionThreadError> {
    let Some(thread_scope) = prepared_thread_scope_for_turn_scope(scope) else {
        return Ok(None);
    };
    match thread_service
        .read_prepared_context(&thread_scope, &scope.thread_id)
        .await
    {
        Ok(record) => Ok(record.map(|record| record.declarations)),
        Err(SessionThreadError::UnknownThread { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Admission-side probe over the threads-tier prepared-context record: the
/// one production implementation of
/// [`ironclaw_host_api::prepared_context::PreparedContextSource`], wired
/// into the turn coordinator by composition. `Ok(None)` means "not a
/// prepared context" (admission rejects ref-less submissions on
/// that answer); storage faults surface as `Unavailable`, never as `None`.
pub struct ThreadServicePreparedContextSource {
    thread_service: std::sync::Arc<dyn crate::SessionThreadService>,
}

impl ThreadServicePreparedContextSource {
    pub fn new(thread_service: std::sync::Arc<dyn crate::SessionThreadService>) -> Self {
        Self { thread_service }
    }
}

#[async_trait::async_trait]
impl ironclaw_host_api::prepared_context::PreparedContextSource
    for ThreadServicePreparedContextSource
{
    async fn read_declarations(
        &self,
        scope: &ironclaw_host_api::turn::TurnScope,
        actor: &ironclaw_host_api::turn::TurnActor,
        accepted_message_ref: &AcceptedMessageRef,
    ) -> Result<
        Option<PreparedTurnDeclarations>,
        ironclaw_host_api::prepared_context::PreparedContextReadError,
    > {
        use ironclaw_host_api::prepared_context::PreparedContextReadError;
        use ironclaw_host_api::turn::TurnThreadOwner;

        // Threads physically require an agent axis; a turn scope without one
        // cannot name an unbound-prepared thread.
        let Some(agent_id) = scope.agent_id.clone() else {
            return Ok(None);
        };
        let owner_user_id = match &scope.thread_owner {
            TurnThreadOwner::Ownerless => None,
            TurnThreadOwner::ExplicitUser { owner_user_id } => Some(owner_user_id.clone()),
            TurnThreadOwner::ActorFallback => Some(actor.user_id.clone()),
        };
        let thread_scope = ThreadScope {
            tenant_id: scope.tenant_id.clone(),
            agent_id,
            project_id: scope.project_id.clone(),
            owner_user_id,
            mission_id: None,
        };
        match self
            .thread_service
            .read_prepared_context(&thread_scope, &scope.thread_id)
            .await
        {
            Ok(Some(record)) => {
                // The submitted ref must pin the prepared context exactly; a
                // mismatched ref is not an unbound submission.
                if record.accepted_message_ref != accepted_message_ref.as_str() {
                    return Ok(None);
                }
                Ok(Some(record.declarations))
            }
            Ok(None) => Ok(None),
            Err(SessionThreadError::UnknownThread { .. }) => Ok(None),
            Err(error) => Err(PreparedContextReadError::Unavailable {
                reason: error.kind_name().to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::ids::{AgentId, TenantId};

    fn scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-unbound-helpers").expect("tenant"),
            agent_id: AgentId::new("agent-unbound-helpers").expect("agent"),
            project_id: None,
            owner_user_id: None,
            mission_id: None,
        }
    }

    fn request() -> PreparedContextRequest {
        PreparedContextRequest {
            scope: scope(),
            actor_id: "user-unbound".to_string(),
            system_prompt: "You are a background task.".to_string(),
            messages: vec![AgentMessage {
                role: AgentMessageRole::User,
                content: vec![ContentPart::text("do the thing")],
            }],
            declarations: PreparedTurnDeclarations::default(),
            idempotency_key: "unbound-key-1".to_string(),
            thread_id: None,
            title: None,
            metadata_json: None,
        }
    }

    #[test]
    fn thread_and_message_ids_are_deterministic_per_scope_and_key() {
        let first = unbound_thread_id(&scope(), "key-a").expect("thread id");
        let second = unbound_thread_id(&scope(), "key-a").expect("thread id");
        assert_eq!(first, second, "same scope+key converges on one thread");

        let other_key = unbound_thread_id(&scope(), "key-b").expect("thread id");
        assert_ne!(first, other_key, "a different key mints a different thread");

        let mut other_scope = scope();
        other_scope.owner_user_id =
            Some(ironclaw_host_api::ids::UserId::new("user-x").expect("user"));
        let cross_scope = unbound_thread_id(&other_scope, "key-a").expect("thread id");
        assert_ne!(first, cross_scope, "scope is part of the identity");

        assert_eq!(
            prepared_seed_message_id(&first, 0),
            prepared_seed_message_id(&first, 0)
        );
        assert_ne!(
            prepared_seed_message_id(&first, 0),
            prepared_seed_message_id(&first, 1)
        );
    }

    #[test]
    fn seed_rows_map_roles_onto_transcript_kinds_in_order() {
        let mut request = request();
        request.messages.push(AgentMessage {
            role: AgentMessageRole::Assistant,
            content: vec![ContentPart::text("earlier answer")],
        });
        let thread_id = unbound_thread_id(&request.scope, &request.idempotency_key).unwrap();
        let rows = prepared_seed_rows(&request, &thread_id, Utc::now()).expect("rows");

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, MessageKind::System);
        assert_eq!(rows[0].status, MessageStatus::Finalized);
        assert_eq!(
            rows[0].content.as_deref(),
            Some("You are a background task.")
        );
        assert_eq!(rows[1].kind, MessageKind::User);
        assert_eq!(rows[1].status, MessageStatus::Accepted);
        assert_eq!(rows[1].actor_id.as_deref(), Some("user-unbound"));
        assert_eq!(rows[2].kind, MessageKind::Assistant);
        assert_eq!(rows[2].status, MessageStatus::Finalized);
        assert!(rows.iter().all(|row| row.source_binding_id.is_none()));
        assert!(rows.iter().all(|row| row.reply_target_binding_id.is_none()));
    }

    #[test]
    fn validation_rejects_empty_and_tool_bearing_requests() {
        let mut empty = request();
        empty.messages.clear();
        assert!(matches!(
            validate_prepared_context_request(&empty),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));

        let mut blank_key = request();
        blank_key.idempotency_key.clear();
        assert!(matches!(
            validate_prepared_context_request(&blank_key),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));

        let mut tool_bearing = request();
        tool_bearing.messages = vec![
            AgentMessage {
                role: AgentMessageRole::Assistant,
                content: vec![ContentPart::ToolCall(
                    ironclaw_llm::agent_message::ToolCallContent {
                        call_id: "call-1".into(),
                        capability: ironclaw_host_api::ids::CapabilityId::new("web.search")
                            .expect("capability"),
                        arguments: serde_json::json!({}),
                    },
                )],
            },
            AgentMessage {
                role: AgentMessageRole::Tool,
                content: vec![ContentPart::ToolResult(
                    ironclaw_llm::agent_message::ToolResultContent {
                        call_id: "call-1".into(),
                        outcome: ironclaw_llm::agent_message::ToolResultOutcome::Text {
                            text: "ok".into(),
                        },
                        is_error: false,
                    },
                )],
            },
        ];
        assert!(matches!(
            validate_prepared_context_request(&tool_bearing),
            Err(SessionThreadError::InvalidPreparedContext { .. })
        ));
    }

    #[test]
    fn replay_checks_key_and_actor_fail_closed() {
        let request = request();
        let thread_id = unbound_thread_id(&request.scope, &request.idempotency_key).unwrap();
        let record = PreparedContextRecord {
            schema_version: PREPARED_CONTEXT_RECORD_SCHEMA_VERSION,
            idempotency_key: request.idempotency_key.clone(),
            actor_id: request.actor_id.clone(),
            accepted_message_ref: format!("msg:{}", prepared_seed_message_id(&thread_id, 1)),
            declarations: request.declarations.clone(),
            seeded_message_count: 2,
            created_at: Utc::now(),
        };

        let replay = replay_prepared_context(&record, &request, &thread_id).expect("replay");
        assert!(replay.idempotent_replay);
        assert_eq!(replay.thread_id, thread_id);

        let mut wrong_key = request.clone();
        wrong_key.idempotency_key = "different-key".to_string();
        assert!(matches!(
            replay_prepared_context(&record, &wrong_key, &thread_id),
            Err(SessionThreadError::PreparedContextKeyMismatch { .. })
        ));

        let mut wrong_actor = request;
        wrong_actor.actor_id = "user-other".to_string();
        assert!(matches!(
            replay_prepared_context(&record, &wrong_actor, &thread_id),
            Err(SessionThreadError::IdempotentReplayActorMismatch { .. })
        ));
    }
}
