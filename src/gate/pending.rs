//! Pending gate state — unified type replacing `PendingApproval` and `PendingAuth`.

use chrono::{DateTime, Utc};
use ironclaw_common::ExternalThreadId;
use ironclaw_engine::{CapabilityLease, ResumeKind, ThreadId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Composite key for the pending gate store.
///
/// Keyed by `(user_id, thread_id)` — exactly one pending gate per thread.
/// This eliminates the `Ambiguous` resolution path that existed when
/// approvals were keyed only by `user_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PendingGateKey {
    pub user_id: String,
    pub thread_id: ThreadId,
}

/// Unified pending state for any gate that pauses execution.
///
/// Replaces both `PendingApproval` and `PendingAuth` from the router.
/// Stored in [`PendingGateStore`] and persisted via [`GatePersistence`].
///
/// [`PendingGateStore`]: super::store::PendingGateStore
/// [`GatePersistence`]: super::store::GatePersistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingGate {
    /// Unique ID for this pending gate request.
    pub request_id: Uuid,
    /// Which gate created this pending state (e.g. "approval", "authentication").
    pub gate_name: String,
    /// User who triggered the gate.
    pub user_id: String,
    /// Engine thread that is paused.
    pub thread_id: ThreadId,
    /// External/client-visible thread id for channels that maintain their own
    /// conversation identifiers above engine threads.
    ///
    /// This is the channel-supplied identifier (web UUID, Telegram chat id,
    /// Slack `thread_ts`) — not the internal engine [`ThreadId`]. See the
    /// `ExternalThreadId` rationale in `crates/ironclaw_common/src/identity.rs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_thread_id: Option<ExternalThreadId>,
    /// Conversation the thread belongs to.
    pub conversation_id: ironclaw_engine::ConversationId,
    /// Channel that originated the request.
    /// Resolution MUST come from the same channel (or a trusted channel).
    pub source_channel: String,
    /// Tool that triggered the gate.
    pub action_name: String,
    /// Tool call ID from the LLM.
    pub call_id: String,
    /// Tool parameters.
    pub parameters: serde_json::Value,
    /// Redacted parameters safe for UI display and history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_parameters: Option<serde_json::Value>,
    /// Human-readable description of what the tool will do.
    pub description: String,
    /// What kind of resolution is expected.
    pub resume_kind: ResumeKind,
    /// When this pending state was created.
    pub created_at: DateTime<Utc>,
    /// When this pending state expires (fail-closed after expiry).
    pub expires_at: DateTime<Utc>,
    /// Original user message to retry when the gate came from a fallback path
    /// that completed instead of pausing the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_message: Option<String>,
    /// Completed action output to inject on resume after auth finishes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_output: Option<serde_json::Value>,
    /// Lease snapshot to reuse when resuming a paused action whose original
    /// lease use was already consumed before the gate fired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_lease: Option<CapabilityLease>,
    /// Whether approval has already been granted for this paused action.
    #[serde(default)]
    pub approval_already_granted: bool,
}

impl PendingGate {
    /// Check whether this pending gate has expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Effective wire thread identifier for channel events — the external
    /// scope when set (preserving whatever the channel uses), otherwise the
    /// internal engine UUID rendered as a string. Chosen because downstream
    /// `AppEvent` fields carry plain strings today (`#[serde(transparent)]`
    /// would re-wrap legacy rows) and channels rely on a single source of
    /// truth for which identifier to route back.
    pub fn effective_wire_thread_id(&self) -> String {
        self.scope_thread_id
            .as_ref()
            .map(|t| t.as_str().to_string())
            .unwrap_or_else(|| self.thread_id.to_string())
    }

    /// Build the composite key for this gate.
    pub fn key(&self) -> PendingGateKey {
        PendingGateKey {
            user_id: self.user_id.clone(),
            thread_id: self.thread_id,
        }
    }
}

/// Read-only view of a pending gate for API responses.
#[derive(Debug, Clone, Serialize)]
pub struct PendingGateView {
    pub request_id: String,
    pub thread_id: String,
    pub gate_name: String,
    pub tool_name: String,
    pub description: String,
    pub parameters: String,
    pub resume_kind: ResumeKind,
}

impl From<&PendingGate> for PendingGateView {
    fn from(gate: &PendingGate) -> Self {
        Self {
            request_id: gate.request_id.to_string(),
            thread_id: gate.effective_wire_thread_id(),
            gate_name: gate.gate_name.clone(),
            tool_name: gate.action_name.clone(),
            description: gate.description.clone(),
            parameters: serde_json::to_string_pretty(
                gate.display_parameters.as_ref().unwrap_or(&gate.parameters),
            )
            .unwrap_or_default(),
            resume_kind: gate.resume_kind.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn sample_gate(expires_in_secs: i64) -> PendingGate {
        PendingGate {
            request_id: Uuid::new_v4(),
            gate_name: "approval".into(),
            user_id: "user1".into(),
            thread_id: ThreadId::new(),
            scope_thread_id: None,
            conversation_id: ironclaw_engine::ConversationId::new(),
            source_channel: "telegram".into(),
            action_name: "shell".into(),
            call_id: "call_1".into(),
            parameters: serde_json::json!({"command": "ls"}),
            display_parameters: None,
            description: "Run shell command".into(),
            resume_kind: ResumeKind::Approval { allow_always: true },
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::seconds(expires_in_secs),
            original_message: None,
            resume_output: None,
            paused_lease: None,
            approval_already_granted: false,
        }
    }

    #[test]
    fn test_not_expired() {
        let gate = sample_gate(300);
        assert!(!gate.is_expired());
    }

    #[test]
    fn test_expired() {
        let gate = sample_gate(-1);
        assert!(gate.is_expired());
    }

    #[test]
    fn test_key_round_trip() {
        let gate = sample_gate(300);
        let key = gate.key();
        assert_eq!(key.user_id, "user1");
        assert_eq!(key.thread_id, gate.thread_id);
    }

    #[test]
    fn test_view_from_gate() {
        let gate = sample_gate(300);
        let view = PendingGateView::from(&gate);
        assert_eq!(view.tool_name, "shell");
        assert_eq!(view.gate_name, "approval");
    }
}
