use ironclaw_common::AttachmentRef;
use ironclaw_host_api::{AgentId, MissionId, ProjectId, TenantId, ThreadId, UserId};
use serde::{Deserialize, Serialize};

use crate::capability_display_preview::CapabilityDisplayPreviewEnvelope;
use crate::identifiers::{SummaryArtifactId, ThreadMessageId};
use crate::tool_result_reference::{ProviderToolCallReferenceEnvelope, ToolResultSafeSummary};

pub const GOAL_STATEMENT_MAX_CHARS: usize = 4000;

/// Canonical scope carried by a Reborn session thread.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadScope {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<MissionId>,
}

impl ThreadScope {
    /// Convert into a [`ironclaw_host_api::ResourceScope`] suitable for the
    /// per-tenant filesystem resolver. `user_id` falls back to a per-thread
    /// system-tenant slot when `owner_user_id` is absent (system-scoped
    /// thread infrastructure that has no owning user).
    pub fn to_resource_scope(&self) -> ironclaw_host_api::ResourceScope {
        ironclaw_host_api::ResourceScope {
            tenant_id: self.tenant_id.clone(),
            user_id: self.owner_user_id.clone().unwrap_or_else(|| {
                ironclaw_host_api::UserId::from_trusted(
                    ironclaw_host_api::SYSTEM_RESERVED_ID.to_string(),
                )
            }),
            agent_id: Some(self.agent_id.clone()),
            project_id: self.project_id.clone(),
            mission_id: self.mission_id.clone(),
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        }
    }
}

/// Safe transcript content accepted by this boundary.
///
/// Model visibility is determined by message kind/status at context-read time;
/// durable UI-only records such as capability previews also store their
/// sanitized payloads here. Attachments are carried as references only (see
/// [`AttachmentRef`]) — never raw bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageContent {
    text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<AttachmentRef>,
}

impl MessageContent {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            text: value.into(),
            attachments: Vec::new(),
        }
    }

    /// Build content carrying both text and attachment references.
    pub fn with_attachments(value: impl Into<String>, attachments: Vec<AttachmentRef>) -> Self {
        Self {
            text: value.into(),
            attachments,
        }
    }

    pub fn as_text(&self) -> &str {
        &self.text
    }

    pub fn attachments(&self) -> &[AttachmentRef] {
        &self.attachments
    }

    /// Consume the content into its text body, **discarding any attachment
    /// references**. Only correct for content known to be attachment-free
    /// (assistant drafts, tool results); use [`Self::into_parts`] whenever
    /// attachments must survive. The debug assertion turns a silent drop into a
    /// loud failure in debug/test builds.
    pub fn into_text(self) -> String {
        debug_assert!(
            self.attachments.is_empty(),
            "into_text() dropped {} attachment ref(s); use into_parts() to keep them",
            self.attachments.len()
        );
        self.text
    }

    /// Consume the content into its text body and attachment references. Use
    /// this when persisting both parts so neither is dropped (`into_text`
    /// alone discards attachments).
    pub fn into_parts(self) -> (String, Vec<AttachmentRef>) {
        (self.text, self.attachments)
    }
}

/// Upper bound on a single attachment's `extracted_text` accepted at the
/// transcript boundary.
///
/// Extraction caps text upstream (~100K chars); this is a generous defensive
/// ceiling so a misbehaving producer cannot persist an unbounded blob that is
/// then pulled inline on every message load. The contract rejects loudly at
/// this layer rather than silently truncating.
pub(crate) const MAX_EXTRACTED_TEXT_CHARS: usize = 200_000;

/// Validate the attachment references on an inbound message before they are
/// persisted. Enforces the invariants the doc comments promise but the plain
/// `String`/`Vec` shapes cannot: ids are unique within the message (so
/// per-attachment lookup/update/delete is unambiguous) and `extracted_text` is
/// bounded ([`MAX_EXTRACTED_TEXT_CHARS`]).
pub(crate) fn validate_attachment_refs(
    attachments: &[AttachmentRef],
) -> Result<(), crate::error::SessionThreadError> {
    let mut seen = std::collections::HashSet::with_capacity(attachments.len());
    for attachment in attachments {
        if !seen.insert(attachment.id.as_str()) {
            return Err(crate::error::SessionThreadError::InvalidAttachment(
                format!("duplicate attachment id {:?} in one message", attachment.id),
            ));
        }
        if let Some(text) = &attachment.extracted_text
            && text.chars().count() > MAX_EXTRACTED_TEXT_CHARS
        {
            return Err(crate::error::SessionThreadError::InvalidAttachment(
                format!(
                    "attachment {:?} extracted_text exceeds {MAX_EXTRACTED_TEXT_CHARS} chars",
                    attachment.id
                ),
            ));
        }
    }
    Ok(())
}

/// Canonical kind of a transcript message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    User,
    Assistant,
    System,
    Summary,
    CheckpointReference,
    ToolResultReference,
    CapabilityDisplayPreview,
}

/// Explicit transcript status. Callers must not infer this from nullable refs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Accepted,
    Submitted,
    DeferredBusy,
    Draft,
    Finalized,
    Interrupted,
    Superseded,
    Redacted,
    Deleted,
}

/// Canonical thread metadata returned by the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionThreadRecord {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub created_by_actor_id: String,
    pub title: Option<String>,
    pub metadata_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<ThreadGoal>,
}

/// Transcript message snapshot for UI/projection reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMessageRecord {
    pub message_id: ThreadMessageId,
    pub thread_id: ThreadId,
    pub sequence: u64,
    pub kind: MessageKind,
    pub status: MessageStatus,
    pub actor_id: Option<String>,
    pub source_binding_id: Option<String>,
    pub reply_target_binding_id: Option<String>,
    pub turn_id: Option<String>,
    pub turn_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result_ref: Option<String>,
    /// Internal provider replay metadata for reconstructing tool-call turns.
    /// Product surfaces must render `content`, not this raw provider side channel.
    #[serde(default, skip_serializing)]
    pub tool_result_provider_call: Option<ProviderToolCallReferenceEnvelope>,
    pub content: Option<String>,
    /// Attachment references for this message. Empty for messages that carry
    /// no attachments. Cleared on redaction in parity with `content`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentRef>,
    pub redaction_ref: Option<String>,
}

/// Summary artifact over a stable transcript sequence range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryArtifact {
    pub summary_id: SummaryArtifactId,
    pub thread_id: ThreadId,
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub summary_kind: SummaryKind,
    /// Plain-text summary body. Summary artifacts intentionally persist text
    /// content, even though thread messages may carry richer side-channel data.
    pub content: String,
    pub model_context_policy: Option<SummaryModelContextPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GoalStatement(String);

impl GoalStatement {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err("goal statement must not be empty".to_string());
        }
        if trimmed.chars().count() > GOAL_STATEMENT_MAX_CHARS {
            return Err(format!(
                "goal statement must be at most {GOAL_STATEMENT_MAX_CHARS} chars"
            ));
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GoalStatement {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<GoalStatement> for String {
    fn from(value: GoalStatement) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadGoal {
    pub statement: GoalStatement,
    pub refined_at_sequence: u64,
    pub refinement_count: u32,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryKind {
    #[serde(alias = "model_context")]
    Compaction,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryModelContextPolicy {
    ReplaceRangeWhenSelected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureThreadRequest {
    pub scope: ThreadScope,
    pub thread_id: Option<ThreadId>,
    pub created_by_actor_id: String,
    pub title: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptInboundMessageRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub actor_id: String,
    pub source_binding_id: Option<String>,
    pub reply_target_binding_id: Option<String>,
    pub external_event_id: Option<String>,
    pub content: MessageContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedInboundMessage {
    pub thread_id: ThreadId,
    pub message_id: ThreadMessageId,
    pub sequence: u64,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedInboundMessageReplay {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub message_id: ThreadMessageId,
    pub sequence: u64,
    pub status: MessageStatus,
    pub actor_id: Option<String>,
    pub source_binding_id: Option<String>,
    pub reply_target_binding_id: Option<String>,
    pub turn_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayAcceptedInboundMessageRequest {
    pub scope: ThreadScope,
    pub actor_id: String,
    pub source_binding_id: String,
    pub external_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendAssistantDraftRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub turn_run_id: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendToolResultReferenceRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub turn_run_id: String,
    pub result_ref: String,
    pub safe_summary: ToolResultSafeSummary,
    pub provider_call: Option<ProviderToolCallReferenceEnvelope>,
    pub model_observation: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendCapabilityDisplayPreviewRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub turn_run_id: String,
    pub preview: CapabilityDisplayPreviewEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateToolResultReferenceRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub turn_run_id: String,
    pub result_ref: String,
    pub safe_summary: ToolResultSafeSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAssistantDraftRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub message_id: ThreadMessageId,
    pub content: MessageContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactMessageRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub message_id: ThreadMessageId,
    pub redaction_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHistoryRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMessageRangeRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub after_sequence: u64,
    pub through_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestThreadMessageRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub kind: MessageKind,
    pub status: MessageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedAssistantMessageByRunRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub turn_run_id: String,
}

/// Browser-driven list-threads query scoped to a single caller.
///
/// Pagination is opaque: `cursor` is whatever value the backend
/// returned as `next_cursor` in a prior response. Stores that have
/// no enumeration support today return an empty list + `None`
/// cursor, which is the default trait impl on `SessionThreadService`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListThreadsForScopeRequest {
    pub scope: ThreadScope,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListThreadsForScopeResponse {
    pub threads: Vec<SessionThreadRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHistory {
    pub thread: SessionThreadRecord,
    pub messages: Vec<ThreadMessageRecord>,
    pub summary_artifacts: Vec<SummaryArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMessageRange {
    pub thread: SessionThreadRecord,
    pub messages: Vec<ThreadMessageRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadContextWindowRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub max_messages: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadContextMessagesRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub message_ids: Vec<ThreadMessageId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMessage {
    pub message_id: Option<ThreadMessageId>,
    pub summary_id: Option<SummaryArtifactId>,
    pub sequence: u64,
    pub kind: MessageKind,
    pub tool_result_provider_call: Option<ProviderToolCallReferenceEnvelope>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindow {
    pub thread_id: ThreadId,
    pub messages: Vec<ContextMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMessages {
    pub thread_id: ThreadId,
    pub messages: Vec<ContextMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSummaryArtifactRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub summary_kind: SummaryKind,
    /// Plain-text summary body to persist for this range. Compaction summaries
    /// are model-visible text artifacts, not rich message payload snapshots.
    pub content: MessageContent,
    pub model_context_policy: Option<SummaryModelContextPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateThreadGoalRequest {
    pub thread_id: ThreadId,
    pub goal: ThreadGoal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_common::AttachmentKind;

    fn sample_ref() -> AttachmentRef {
        AttachmentRef {
            id: "att-1".to_string(),
            kind: AttachmentKind::Document,
            mime_type: "application/pdf".to_string(),
            filename: Some("report.pdf".to_string()),
            size_bytes: Some(2048),
            storage_key: Some("attachments/2026-06-09/m1-report.pdf".to_string()),
            extracted_text: Some("quarterly numbers".to_string()),
        }
    }

    #[test]
    fn text_constructor_carries_no_attachments() {
        let content = MessageContent::text("hello");
        assert_eq!(content.as_text(), "hello");
        assert!(content.attachments().is_empty());
    }

    #[test]
    fn into_parts_preserves_text_and_attachments() {
        let content = MessageContent::with_attachments("see attached", vec![sample_ref()]);
        assert_eq!(content.attachments().len(), 1);
        let (text, attachments) = content.into_parts();
        assert_eq!(text, "see attached");
        assert_eq!(attachments, vec![sample_ref()]);
    }

    #[test]
    fn validate_attachment_refs_accepts_distinct_ids() {
        let mut second = sample_ref();
        second.id = "att-2".to_string();
        assert!(validate_attachment_refs(&[sample_ref(), second]).is_ok());
    }

    #[test]
    fn validate_attachment_refs_rejects_duplicate_ids() {
        let err = validate_attachment_refs(&[sample_ref(), sample_ref()])
            .expect_err("duplicate attachment ids must be rejected");
        assert!(matches!(
            err,
            crate::error::SessionThreadError::InvalidAttachment(_)
        ));
    }

    #[test]
    fn validate_attachment_refs_rejects_oversized_extracted_text() {
        let mut oversized = sample_ref();
        oversized.extracted_text = Some("x".repeat(MAX_EXTRACTED_TEXT_CHARS + 1));
        let err = validate_attachment_refs(&[oversized])
            .expect_err("extracted_text past the cap must be rejected");
        assert!(matches!(
            err,
            crate::error::SessionThreadError::InvalidAttachment(_)
        ));
    }

    #[test]
    fn validate_attachment_refs_accepts_extracted_text_at_cap() {
        let mut at_cap = sample_ref();
        at_cap.extracted_text = Some("x".repeat(MAX_EXTRACTED_TEXT_CHARS));
        assert!(validate_attachment_refs(&[at_cap]).is_ok());
    }

    #[test]
    fn into_text_keeps_text_when_attachment_free() {
        // The non-lossy use: `into_text` is the text-only accessor for content
        // known to carry no attachments (assistant drafts, tool results).
        let content = MessageContent::text("body");
        assert_eq!(content.into_text(), "body");
    }

    #[test]
    #[should_panic(expected = "dropped 1 attachment ref")]
    fn into_text_debug_asserts_when_it_would_drop_attachments() {
        // Callers that must keep attachments use `into_parts`; `into_text` on
        // attachment-bearing content is a bug and fails loudly in debug/tests.
        let content = MessageContent::with_attachments("body", vec![sample_ref()]);
        let _ = content.into_text();
    }

    #[test]
    fn message_content_skips_empty_attachments_on_the_wire() {
        let json = serde_json::to_string(&MessageContent::text("hi")).unwrap();
        assert_eq!(json, r#"{"text":"hi"}"#);
    }

    #[test]
    fn attachment_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&AttachmentKind::Image).unwrap(),
            r#""image""#
        );
        assert_eq!(
            serde_json::from_str::<AttachmentKind>(r#""document""#).unwrap(),
            AttachmentKind::Document
        );
    }

    #[test]
    fn attachment_ref_round_trips_and_omits_empty_optionals() {
        let minimal = AttachmentRef {
            id: "att-2".to_string(),
            kind: AttachmentKind::Image,
            mime_type: "image/png".to_string(),
            filename: None,
            size_bytes: None,
            storage_key: None,
            extracted_text: None,
        };
        let json = serde_json::to_string(&minimal).unwrap();
        assert_eq!(
            json,
            r#"{"id":"att-2","kind":"image","mime_type":"image/png"}"#
        );
        assert_eq!(
            serde_json::from_str::<AttachmentRef>(&json).unwrap(),
            minimal
        );

        let full = sample_ref();
        let round =
            serde_json::from_str::<AttachmentRef>(&serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(round, full);
    }

    #[test]
    fn thread_message_record_attachments_default_when_absent() {
        // Old persisted rows have no `attachments` field; it must default to
        // empty rather than failing deserialization.
        let json = r#"{
            "message_id": "00000000-0000-0000-0000-000000000001",
            "thread_id": "thread-x",
            "sequence": 1,
            "kind": "user",
            "status": "accepted",
            "actor_id": null,
            "source_binding_id": null,
            "reply_target_binding_id": null,
            "turn_id": null,
            "turn_run_id": null,
            "content": "legacy row",
            "redaction_ref": null
        }"#;
        let record: ThreadMessageRecord = serde_json::from_str(json).unwrap();
        assert!(record.attachments.is_empty());
        assert_eq!(record.content.as_deref(), Some("legacy row"));
    }
}
