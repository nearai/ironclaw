use ironclaw_host_api::{AgentId, MissionId, ProjectId, TenantId, ThreadId, UserId};
use serde::{Deserialize, Serialize};

use crate::capability_display_preview::CapabilityDisplayPreviewEnvelope;
use crate::identifiers::{SummaryArtifactId, ThreadMessageId};
use crate::tool_result_reference::{ProviderToolCallReferenceEnvelope, ToolResultSafeSummary};

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

/// Safe transcript text accepted by this boundary.
///
/// Model visibility is determined by message kind/status at context-read time;
/// durable UI-only records such as capability previews also store their
/// sanitized payloads here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageContent {
    text: String,
}

impl MessageContent {
    pub fn text(value: impl Into<String>) -> Self {
        Self { text: value.into() }
    }

    pub fn as_text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }
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
    pub redaction_ref: Option<String>,
}

/// Summary artifact over a stable transcript sequence range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryArtifact {
    pub summary_id: SummaryArtifactId,
    pub thread_id: ThreadId,
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub summary_kind: String,
    pub content: String,
    pub model_context_policy: Option<String>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendCapabilityDisplayPreviewRequest {
    pub scope: ThreadScope,
    pub thread_id: ThreadId,
    pub turn_run_id: String,
    pub preview: CapabilityDisplayPreviewEnvelope,
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
    pub summary_kind: String,
    pub content: MessageContent,
    pub model_context_policy: Option<String>,
}
