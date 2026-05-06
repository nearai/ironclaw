//! Outbound envelope, projection-derived payloads, and projection cursor.

use chrono::{DateTime, Utc};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::{AdapterInstallationId, ProductAdapterId};

/// Opaque, durable, per-thread projection cursor.
///
/// Reborn projection cursors are scoped to `(tenant, thread)` and validated
/// against participant access on every consume — they are NOT transport-local
/// SSE/WebSocket event ids.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectionCursor(String);

impl ProjectionCursor {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Final-reply view derived from the canonical thread projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalReplyView {
    pub turn_run_id: TurnRunId,
    /// Markdown-safe plaintext body. Adapters protocol-translate (HTML for
    /// Telegram, mrkdwn for Slack, etc.); the canonical projection always
    /// emits plaintext or fenced code blocks.
    pub text: String,
    pub generated_at: DateTime<Utc>,
}

/// Progress update view (typing indicators, "thinking", tool-call status).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressUpdateView {
    pub turn_run_id: TurnRunId,
    pub kind: ProgressKind,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressKind {
    Typing,
    ToolRunning,
    Reflecting,
}

/// Approval-gate prompt view. Deferred to #3094; this contract renders a
/// placeholder body so adapters can light up rendering tests without wiring
/// real interaction services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatePromptView {
    pub turn_run_id: TurnRunId,
    pub gate_ref: String,
    pub headline: String,
    pub body: String,
}

/// Auth-prompt view (deferred to #3094 along with `GatePromptView`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPromptView {
    pub turn_run_id: TurnRunId,
    pub auth_request_ref: String,
    pub headline: String,
    pub body: String,
}

/// Snapshot of the canonical thread projection (full state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSnapshot {
    pub cursor: ProjectionCursor,
    pub thread_id: String,
    pub generated_at: DateTime<Utc>,
}

/// Incremental projection update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionUpdate {
    pub cursor: ProjectionCursor,
    pub thread_id: String,
    pub generated_at: DateTime<Utc>,
}

/// All supported outbound payload kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductOutboundPayload {
    FinalReply(FinalReplyView),
    Progress(ProgressUpdateView),
    GatePrompt(GatePromptView),
    AuthPrompt(AuthPromptView),
    ProjectionSnapshot(ProjectionSnapshot),
    ProjectionUpdate(ProjectionUpdate),
}

/// Outbound envelope handed to the adapter for protocol translation +
/// delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductOutboundEnvelope {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub target: ReplyTargetBindingRef,
    pub projection_cursor: Option<ProjectionCursor>,
    pub payload: ProductOutboundPayload,
    /// Stable id for the egress attempt — used by [`crate::OutboundDeliverySink`]
    /// to dedupe and track delivery status.
    pub delivery_attempt_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips() {
        let cursor = ProjectionCursor::new("thread:42#cursor:7");
        let json = serde_json::to_string(&cursor).expect("serialize");
        let parsed: ProjectionCursor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(cursor, parsed);
    }

    #[test]
    fn final_reply_serializes_with_plaintext() {
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hello world".into(),
            generated_at: Utc::now(),
        };
        let json = serde_json::to_value(&view).expect("serialize");
        assert_eq!(json["text"], "hello world");
    }
}
