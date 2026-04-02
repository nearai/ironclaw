//! Request and response DTOs for the web gateway API.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --- Chat ---

/// Base64-encoded image data sent from the web frontend.
#[derive(Debug, Clone, Deserialize)]
pub struct ImageData {
    /// MIME type (e.g., "image/png", "image/jpeg").
    pub media_type: String,
    /// Base64-encoded image data (without data: URL prefix).
    pub data: String,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
    pub thread_id: Option<String>,
    pub timezone: Option<String>,
    /// Optional images attached to the message.
    #[serde(default)]
    pub images: Vec<ImageData>,
}

#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub message_id: Uuid,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ThreadInfo {
    pub id: Uuid,
    pub state: String,
    pub turn_count: usize,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThreadListResponse {
    /// The pinned assistant thread (always present after first load).
    pub assistant_thread: Option<ThreadInfo>,
    /// Regular conversation threads.
    pub threads: Vec<ThreadInfo>,
    pub active_thread: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct TurnInfo {
    pub turn_number: usize,
    pub user_input: String,
    pub response: Option<String>,
    pub state: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub tool_calls: Vec<ToolCallInfo>,
    /// Agent's reasoning narrative for this turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narrative: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolCallInfo {
    pub name: String,
    pub has_result: bool,
    pub has_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Agent's reasoning for choosing this tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub thread_id: Uuid,
    pub turns: Vec<TurnInfo>,
    /// Whether there are older messages available.
    #[serde(default)]
    pub has_more: bool,
    /// Cursor for the next page (ISO8601 timestamp of the oldest message returned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_timestamp: Option<String>,
    /// Pending tool approval that needs user action (re-rendered on thread switch).
    ///
    /// Only populated from in-memory state; not persisted to DB.
    /// Server restart clears pending approvals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_approval: Option<PendingApprovalInfo>,
}

/// Lightweight DTO for a pending tool approval (excludes context_messages).
#[derive(Debug, Serialize)]
pub struct PendingApprovalInfo {
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    pub parameters: String,
}

// --- Approval ---

#[derive(Debug, Deserialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    /// "approve", "always", or "deny"
    pub action: String,
    /// Thread that owns the pending approval (so the agent loop finds the right session).
    pub thread_id: Option<String>,
}

// --- App Event (re-exported from ironclaw_common) ---

pub use ironclaw_common::{AppEvent, ToolDecisionDto};

// --- Memory ---

#[derive(Debug, Serialize)]
pub struct MemoryTreeResponse {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Serialize)]
pub struct TreeEntry {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub path: String,
    pub entries: Vec<ListEntry>,
}

#[derive(Debug, Serialize)]
pub struct ListEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MemoryReadResponse {
    pub path: String,
    pub content: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryWriteRequest {
    pub path: String,
    pub content: String,
    /// Optional layer to write to. When present, uses `write_to_layer()`
    /// which enables privacy classification and redirect.
    pub layer: Option<String>,
    /// When true and a layer is specified, appends to existing content
    /// instead of replacing it.
    #[serde(default)]
    pub append: bool,
    /// Skip privacy classification and write directly to the specified layer.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct MemoryWriteResponse {
    pub path: String,
    pub status: &'static str,
    /// Whether the write was redirected to a different layer (e.g., sensitive
    /// content redirected from shared to private).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirected: Option<bool>,
    /// The layer the content was actually written to (may differ from requested
    /// layer if privacy redirect occurred).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_layer: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MemorySearchRequest {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct MemorySearchResponse {
    pub results: Vec<SearchHit>,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub content: String,
    pub score: f64,
}

// --- Jobs ---

#[derive(Debug, Serialize)]
pub struct JobInfo {
    pub id: Uuid,
    pub title: String,
    pub state: String,
    pub user_id: String,
    pub created_at: String,
    pub started_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JobListResponse {
    pub jobs: Vec<JobInfo>,
}

#[derive(Debug, Serialize)]
pub struct JobSummaryResponse {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub stuck: usize,
}

#[derive(Debug, Serialize)]
pub struct JobDetailResponse {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub state: String,
    pub user_id: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub elapsed_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browse_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_mode: Option<String>,
    pub transitions: Vec<TransitionInfo>,
    /// Whether this job can be restarted from the UI.
    #[serde(default)]
    pub can_restart: bool,
    /// Whether follow-up prompts can be sent to this job.
    #[serde(default)]
    pub can_prompt: bool,
    /// The kind of job: "sandbox" or "agent".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_kind: Option<String>,
}

// --- Project Files ---

#[derive(Debug, Serialize)]
pub struct ProjectFileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct ProjectFilesResponse {
    pub entries: Vec<ProjectFileEntry>,
}

#[derive(Debug, Serialize)]
pub struct ProjectFileReadResponse {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct TransitionInfo {
    pub from: String,
    pub to: String,
    pub timestamp: String,
    pub reason: Option<String>,
}

// --- Extensions ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionActivationStatus {
    Installed,
    Configured,
    Pairing,
    Active,
    Failed,
}

pub fn classify_wasm_channel_activation(
    ext: &crate::extensions::InstalledExtension,
    has_paired: bool,
    has_owner_binding: bool,
) -> Option<ExtensionActivationStatus> {
    if ext.kind != crate::extensions::ExtensionKind::WasmChannel {
        return None;
    }

    Some(if ext.activation_error.is_some() {
        ExtensionActivationStatus::Failed
    } else if !ext.authenticated {
        ExtensionActivationStatus::Installed
    } else if ext.active {
        if has_paired || has_owner_binding {
            ExtensionActivationStatus::Active
        } else {
            ExtensionActivationStatus::Pairing
        }
    } else {
        ExtensionActivationStatus::Configured
    })
}

#[derive(Debug, Serialize)]
pub struct ExtensionInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub kind: String,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub authenticated: bool,
    pub active: bool,
    pub tools: Vec<String>,
    /// Whether this extension has configurable secrets (setup schema).
    #[serde(default)]
    pub needs_setup: bool,
    /// Whether this extension has an auth configuration (OAuth or manual token).
    #[serde(default)]
    pub has_auth: bool,
    /// WASM channel activation status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_status: Option<ExtensionActivationStatus>,
    /// Human-readable error when activation_status is "failed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_error: Option<String>,
    /// Extension version (semver).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExtensionListResponse {
    pub extensions: Vec<ExtensionInfo>,
}

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct ToolListResponse {
    pub tools: Vec<ToolInfo>,
}

#[derive(Debug, Deserialize)]
pub struct InstallExtensionRequest {
    pub name: String,
    pub url: Option<String>,
    pub kind: Option<String>,
}

// --- Extension Setup ---

#[derive(Debug, Serialize)]
pub struct ExtensionSetupResponse {
    pub name: String,
    pub kind: String,
    pub secrets: Vec<SecretFieldInfo>,
    pub fields: Vec<SetupFieldInfo>,
}

#[derive(Debug, Serialize)]
pub struct SecretFieldInfo {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    /// Whether this secret is already stored.
    pub provided: bool,
    /// Whether the secret will be auto-generated if left empty.
    pub auto_generate: bool,
}

#[derive(Debug, Serialize)]
pub struct SetupFieldInfo {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    /// Whether this field already has a stored value.
    pub provided: bool,
    /// Input type for web UI rendering.
    pub input_type: crate::tools::wasm::ToolSetupFieldInputType,
}

#[derive(Debug, Deserialize)]
pub struct ExtensionSetupRequest {
    #[serde(default)]
    pub secrets: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub fields: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub success: bool,
    pub message: String,
    /// Auth URL to open (when activation requires OAuth).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    /// Whether the extension is waiting for a manual token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awaiting_token: Option<bool>,
    /// Instructions for manual token entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Whether the channel was successfully activated after setup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated: Option<bool>,
    /// Whether a restart is required for the new configuration to take effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_restart: Option<bool>,
    /// Pending manual verification challenge (for Telegram owner binding, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<crate::extensions::VerificationChallenge>,
}

impl ActionResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            auth_url: None,
            awaiting_token: None,
            instructions: None,
            activated: None,
            needs_restart: None,
            verification: None,
        }
    }

    pub fn fail(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            auth_url: None,
            awaiting_token: None,
            instructions: None,
            activated: None,
            needs_restart: None,
            verification: None,
        }
    }
}

// --- Registry ---

#[derive(Debug, Serialize)]
pub struct RegistryEntryInfo {
    pub name: String,
    pub display_name: String,
    pub kind: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegistrySearchResponse {
    pub entries: Vec<RegistryEntryInfo>,
}

#[derive(Debug, Deserialize)]
pub struct RegistrySearchQuery {
    pub query: Option<String>,
}

// --- Pairing ---

#[derive(Debug, Serialize)]
pub struct PairingListResponse {
    pub channel: String,
    pub requests: Vec<PairingRequestInfo>,
}

#[derive(Debug, Serialize)]
pub struct PairingRequestInfo {
    pub code: String,
    pub sender_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct PairingApproveRequest {
    pub code: String,
}

// --- Skills ---

#[derive(Debug, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub trust: String,
    pub source: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillListResponse {
    pub skills: Vec<SkillInfo>,
    pub count: usize,
}

#[derive(Debug, Deserialize)]
pub struct SkillSearchRequest {
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct SkillSearchResponse {
    pub catalog: Vec<serde_json::Value>,
    pub installed: Vec<SkillInfo>,
    pub registry_url: String,
    /// If the catalog registry was unreachable or errored, a human-readable message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillInstallRequest {
    pub name: String,
    /// Registry slug (e.g. "owner/skill-name"). Preferred over `name` for
    /// constructing the download URL when fetching from ClawHub.
    pub slug: Option<String>,
    pub url: Option<String>,
    pub content: Option<String>,
}

// --- Auth Token ---

/// Request to submit an auth token for an extension (dedicated endpoint).
#[derive(Debug, Deserialize)]
pub struct AuthTokenRequest {
    pub extension_name: String,
    pub token: String,
}

/// Request to cancel an in-progress auth flow.
#[derive(Debug, Deserialize)]
pub struct AuthCancelRequest {
    pub extension_name: String,
}

// --- WebSocket ---

/// Message sent by a WebSocket client to the server.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum WsClientMessage {
    /// Send a chat message to the agent.
    #[serde(rename = "message")]
    Message {
        content: String,
        thread_id: Option<String>,
        timezone: Option<String>,
        /// Optional images attached to the message.
        #[serde(default)]
        images: Vec<ImageData>,
    },
    /// Approve or deny a pending tool execution.
    #[serde(rename = "approval")]
    Approval {
        request_id: String,
        /// "approve", "always", or "deny"
        action: String,
        /// Thread that owns the pending approval.
        thread_id: Option<String>,
    },
    /// Submit an auth token for an extension (bypasses message pipeline).
    #[serde(rename = "auth_token")]
    AuthToken {
        extension_name: String,
        token: String,
    },
    /// Cancel an in-progress auth flow.
    #[serde(rename = "auth_cancel")]
    AuthCancel { extension_name: String },
    /// Client heartbeat ping.
    #[serde(rename = "ping")]
    Ping,
}

/// Message sent by the server to a WebSocket client.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WsServerMessage {
    /// An SSE-style event forwarded over WebSocket.
    #[serde(rename = "event")]
    Event {
        /// The event sub-type (response, thinking, tool_started, etc.)
        event_type: String,
        /// The event payload as a JSON value.
        data: serde_json::Value,
    },
    /// Server heartbeat pong.
    #[serde(rename = "pong")]
    Pong,
    /// Error message.
    #[serde(rename = "error")]
    Error { message: String },
}

impl WsServerMessage {
    /// Create a WsServerMessage from an AppEvent.
    pub fn from_app_event(event: &AppEvent) -> Self {
        let event_type = event.event_type();
        let data = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
        WsServerMessage::Event {
            event_type: event_type.to_string(),
            data,
        }
    }
}

// --- Routines ---

#[derive(Debug, Serialize)]
pub struct RoutineInfo {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger_type: String,
    pub trigger_raw: String,
    pub trigger_summary: String,
    pub action_type: String,
    pub last_run_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub status: String,
    pub verification_status: String,
}

impl RoutineInfo {
    /// Convert a `Routine` to the trimmed `RoutineInfo` for list display.
    pub fn from_routine(
        r: &crate::agent::routine::Routine,
        last_run_status: Option<crate::agent::routine::RunStatus>,
    ) -> Self {
        let (trigger_type, trigger_raw, trigger_summary) = match &r.trigger {
            crate::agent::routine::Trigger::Cron { schedule, timezone } => (
                "cron".to_string(),
                schedule.clone(),
                crate::agent::routine::describe_cron(schedule, timezone.as_deref()),
            ),
            crate::agent::routine::Trigger::Event {
                pattern, channel, ..
            } => {
                let ch = channel.as_deref().unwrap_or("any");
                (
                    "event".to_string(),
                    String::new(),
                    format!("on {} /{}/", ch, pattern),
                )
            }
            crate::agent::routine::Trigger::SystemEvent {
                source, event_type, ..
            } => (
                "system_event".to_string(),
                String::new(),
                format!("event: {}.{}", source, event_type),
            ),
            crate::agent::routine::Trigger::Webhook { path, .. } => {
                let p = path.as_deref().unwrap_or("default");
                (
                    "webhook".to_string(),
                    String::new(),
                    format!("webhook: /api/webhooks/{}", p),
                )
            }
            crate::agent::routine::Trigger::Manual => (
                "manual".to_string(),
                String::new(),
                "manual only".to_string(),
            ),
        };

        let action_type = match &r.action {
            crate::agent::routine::RoutineAction::Lightweight { .. } => "lightweight",
            crate::agent::routine::RoutineAction::FullJob { .. } => "full_job",
        };

        let verification_status = crate::agent::routine::routine_verification_status(r);
        let status = crate::agent::routine::routine_display_status_for_verification(
            r,
            verification_status,
            last_run_status,
        )
        .as_str();

        RoutineInfo {
            id: r.id,
            name: r.name.clone(),
            description: r.description.clone(),
            enabled: r.enabled,
            trigger_type,
            trigger_raw,
            trigger_summary,
            action_type: action_type.to_string(),
            last_run_at: r.last_run_at.map(|dt| dt.to_rfc3339()),
            next_fire_at: r.next_fire_at.map(|dt| dt.to_rfc3339()),
            run_count: r.run_count,
            consecutive_failures: r.consecutive_failures,
            status: status.to_string(),
            verification_status: verification_status.as_str().to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RoutineListResponse {
    pub routines: Vec<RoutineInfo>,
}

#[derive(Debug, Serialize)]
pub struct RoutineSummaryResponse {
    pub total: u64,
    pub enabled: u64,
    pub disabled: u64,
    pub unverified: u64,
    pub failing: u64,
    pub runs_today: u64,
}

#[derive(Debug, Serialize)]
pub struct RoutineDetailResponse {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger_type: String,
    pub trigger_raw: String,
    pub trigger_summary: String,
    pub trigger: serde_json::Value,
    pub action: serde_json::Value,
    pub guardrails: serde_json::Value,
    pub notify: serde_json::Value,
    pub last_run_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub status: String,
    pub verification_status: String,
    pub created_at: String,
    pub conversation_id: Option<Uuid>,
    pub recent_runs: Vec<RoutineRunInfo>,
}

#[derive(Debug, Serialize)]
pub struct RoutineRunInfo {
    pub id: Uuid,
    pub trigger_type: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub result_summary: Option<String>,
    pub tokens_used: Option<i32>,
    pub job_id: Option<Uuid>,
}

// --- Settings ---

#[derive(Debug, Serialize)]
pub struct SettingResponse {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct SettingsListResponse {
    pub settings: Vec<SettingResponse>,
}

#[derive(Debug, Deserialize)]
pub struct SettingWriteRequest {
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct SettingsImportRequest {
    pub settings: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct SettingsExportResponse {
    pub settings: std::collections::HashMap<String, serde_json::Value>,
}

// --- Tool Permissions ---

/// Single tool entry returned by `GET /api/settings/tools`.
#[derive(Debug, Serialize)]
pub struct ToolPermissionEntry {
    pub name: String,
    pub description: String,
    pub current_state: String,
    pub default_state: String,
    pub locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolPermissionsResponse {
    pub tools: Vec<ToolPermissionEntry>,
}

/// Body for `PUT /api/settings/tools/:name`.
#[derive(Debug, Deserialize)]
pub struct UpdateToolPermissionRequest {
    pub state: String,
}

// --- Health ---

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub channel: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // ---- WsClientMessage deserialization tests ----

    #[test]
    fn test_ws_client_message_parse() {
        let json = r#"{"type":"message","content":"hello","thread_id":"t1"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Message {
                content, thread_id, ..
            } => {
                assert_eq!(content, "hello");
                assert_eq!(thread_id.as_deref(), Some("t1"));
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_ws_client_message_no_thread() {
        let json = r#"{"type":"message","content":"hi"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Message {
                content, thread_id, ..
            } => {
                assert_eq!(content, "hi");
                assert!(thread_id.is_none());
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_ws_client_approval_parse() {
        let json =
            r#"{"type":"approval","request_id":"abc-123","action":"approve","thread_id":"t1"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Approval {
                request_id,
                action,
                thread_id,
            } => {
                assert_eq!(request_id, "abc-123");
                assert_eq!(action, "approve");
                assert_eq!(thread_id.as_deref(), Some("t1"));
            }
            _ => panic!("Expected Approval variant"),
        }
    }

    #[test]
    fn test_ws_client_approval_parse_no_thread() {
        let json = r#"{"type":"approval","request_id":"abc-123","action":"deny"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Approval {
                request_id,
                action,
                thread_id,
            } => {
                assert_eq!(request_id, "abc-123");
                assert_eq!(action, "deny");
                assert!(thread_id.is_none());
            }
            _ => panic!("Expected Approval variant"),
        }
    }

    #[test]
    fn test_ws_client_ping_parse() {
        let json = r#"{"type":"ping"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, WsClientMessage::Ping));
    }

    #[test]
    fn test_ws_client_unknown_type_fails() {
        let json = r#"{"type":"unknown"}"#;
        let result: Result<WsClientMessage, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // ---- WsServerMessage serialization tests ----

    #[test]
    fn test_ws_server_pong_serialize() {
        let msg = WsServerMessage::Pong;
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"pong"}"#);
    }

    #[test]
    fn test_ws_server_error_serialize() {
        let msg = WsServerMessage::Error {
            message: "bad request".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["message"], "bad request");
    }

    #[test]
    fn test_ws_server_from_app_event_response() {
        let event = AppEvent::Response {
            content: "hello".to_string(),
            thread_id: "t1".to_string(),
        };
        let ws = WsServerMessage::from_app_event(&event);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "response");
                assert_eq!(data["content"], "hello");
                assert_eq!(data["thread_id"], "t1");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_app_event_thinking() {
        let event = AppEvent::Thinking {
            message: "reasoning...".to_string(),
            thread_id: None,
        };
        let ws = WsServerMessage::from_app_event(&event);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "thinking");
                assert_eq!(data["message"], "reasoning...");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_app_event_approval_needed() {
        let event = AppEvent::ApprovalNeeded {
            request_id: "r1".to_string(),
            tool_name: "shell".to_string(),
            description: "Run ls".to_string(),
            parameters: "{}".to_string(),
            thread_id: Some("t1".to_string()),
            allow_always: true,
        };
        let ws = WsServerMessage::from_app_event(&event);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "approval_needed");
                assert_eq!(data["tool_name"], "shell");
                assert_eq!(data["thread_id"], "t1");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_app_event_heartbeat() {
        let event = AppEvent::Heartbeat;
        let ws = WsServerMessage::from_app_event(&event);
        match ws {
            WsServerMessage::Event { event_type, .. } => {
                assert_eq!(event_type, "heartbeat");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    // ---- Auth type tests ----

    #[test]
    fn test_ws_client_auth_token_parse() {
        let json = r#"{"type":"auth_token","extension_name":"notion","token":"sk-123"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::AuthToken {
                extension_name,
                token,
            } => {
                assert_eq!(extension_name, "notion");
                assert_eq!(token, "sk-123");
            }
            _ => panic!("Expected AuthToken variant"),
        }
    }

    #[test]
    fn test_ws_client_auth_cancel_parse() {
        let json = r#"{"type":"auth_cancel","extension_name":"notion"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::AuthCancel { extension_name } => {
                assert_eq!(extension_name, "notion");
            }
            _ => panic!("Expected AuthCancel variant"),
        }
    }

    #[test]
    fn test_app_event_auth_required_serialize() {
        let event = AppEvent::AuthRequired {
            extension_name: "notion".to_string(),
            instructions: Some("Get your token from...".to_string()),
            auth_url: None,
            setup_url: Some("https://notion.so/integrations".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "auth_required");
        assert_eq!(parsed["extension_name"], "notion");
        assert_eq!(parsed["instructions"], "Get your token from...");
        assert!(parsed.get("auth_url").is_none());
        assert_eq!(parsed["setup_url"], "https://notion.so/integrations");
    }

    #[test]
    fn test_app_event_auth_completed_serialize() {
        let event = AppEvent::AuthCompleted {
            extension_name: "notion".to_string(),
            success: true,
            message: "notion authenticated (3 tools loaded)".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "auth_completed");
        assert_eq!(parsed["extension_name"], "notion");
        assert_eq!(parsed["success"], true);
    }

    #[test]
    fn test_ws_server_from_app_event_auth_required() {
        let event = AppEvent::AuthRequired {
            extension_name: "openai".to_string(),
            instructions: Some("Enter API key".to_string()),
            auth_url: None,
            setup_url: None,
        };
        let ws = WsServerMessage::from_app_event(&event);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "auth_required");
                assert_eq!(data["extension_name"], "openai");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_app_event_auth_completed() {
        let event = AppEvent::AuthCompleted {
            extension_name: "slack".to_string(),
            success: false,
            message: "Invalid token".to_string(),
        };
        let ws = WsServerMessage::from_app_event(&event);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "auth_completed");
                assert_eq!(data["success"], false);
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_auth_token_request_deserialize() {
        let json = r#"{"extension_name":"telegram","token":"bot12345"}"#;
        let req: AuthTokenRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extension_name, "telegram");
        assert_eq!(req.token, "bot12345");
    }

    #[test]
    fn test_auth_cancel_request_deserialize() {
        let json = r#"{"extension_name":"telegram"}"#;
        let req: AuthCancelRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extension_name, "telegram");
    }

    #[test]
    fn test_extension_setup_request_defaults() {
        let json = r#"{}"#;
        let req: ExtensionSetupRequest = serde_json::from_str(json).unwrap();
        assert!(req.secrets.is_empty());
        assert!(req.fields.is_empty());
    }

    #[test]
    fn test_extension_setup_request_deserialize_with_fields() {
        let json = r#"{
            "secrets": { "api_key": "sk-123" },
            "fields": { "llm_backend": "openai", "selected_model": "gpt-4o" }
        }"#;
        let req: ExtensionSetupRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.secrets.get("api_key").unwrap(), "sk-123");
        assert_eq!(req.fields.get("llm_backend").unwrap(), "openai");
        assert_eq!(req.fields.get("selected_model").unwrap(), "gpt-4o");
    }

    #[test]
    fn test_setup_field_info_serializes_input_type_as_enum_string() {
        let field = SetupFieldInfo {
            name: "selected_model".to_string(),
            prompt: "Model".to_string(),
            optional: false,
            provided: true,
            input_type: crate::tools::wasm::ToolSetupFieldInputType::Password,
        };

        let json = serde_json::to_value(field).unwrap();
        assert_eq!(json["input_type"], "password");
    }

    // ---- ThreadInfo channel field tests ----

    #[test]
    fn test_thread_info_channel_serialized() {
        let info = ThreadInfo {
            id: Uuid::nil(),
            state: "Idle".to_string(),
            turn_count: 0,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            title: None,
            thread_type: None,
            channel: Some("telegram".to_string()),
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["channel"], "telegram");
    }

    #[test]
    fn test_thread_info_channel_omitted_when_none() {
        let info = ThreadInfo {
            id: Uuid::nil(),
            state: "Idle".to_string(),
            turn_count: 0,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            title: None,
            thread_type: None,
            channel: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("channel").is_none());
    }

    fn make_routine_for_status_tests() -> crate::agent::routine::Routine {
        crate::agent::routine::Routine {
            id: Uuid::new_v4(),
            name: "status-check".to_string(),
            description: "routine status test".to_string(),
            user_id: "test-user".to_string(),
            enabled: true,
            trigger: crate::agent::routine::Trigger::Manual,
            action: crate::agent::routine::RoutineAction::Lightweight {
                prompt: "Check status".to_string(),
                context_paths: Vec::new(),
                max_tokens: 256,
                use_tools: false,
                max_tool_rounds: 1,
            },
            guardrails: crate::agent::routine::RoutineGuardrails::default(),
            notify: crate::agent::routine::NotifyConfig::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_routine_info_marks_new_routine_unverified() {
        let mut routine = make_routine_for_status_tests();
        routine.state = crate::agent::routine::reset_routine_verification_state(
            &routine.state,
            crate::agent::routine::routine_verification_fingerprint(&routine),
        );

        let info = RoutineInfo::from_routine(&routine, None);

        assert_eq!(info.status, "unverified");
        assert_eq!(info.verification_status, "unverified");
    }

    #[test]
    fn test_routine_info_preserves_verified_state_for_description_only_changes() {
        let mut routine = make_routine_for_status_tests();
        let fingerprint = crate::agent::routine::routine_verification_fingerprint(&routine);
        routine.state = crate::agent::routine::reset_routine_verification_state(
            &routine.state,
            fingerprint.clone(),
        );
        routine.state = crate::agent::routine::apply_routine_verification_result(
            &routine.state,
            fingerprint,
            crate::agent::routine::RunStatus::Ok,
            Utc::now(),
        );
        routine.description = "Updated description".to_string();

        let info = RoutineInfo::from_routine(&routine, Some(crate::agent::routine::RunStatus::Ok));

        assert_eq!(info.status, "active");
        assert_eq!(info.verification_status, "verified");
    }

    #[test]
    fn test_routine_info_surfaces_running_before_unverified() {
        let mut routine = make_routine_for_status_tests();
        routine.state = crate::agent::routine::reset_routine_verification_state(
            &routine.state,
            crate::agent::routine::routine_verification_fingerprint(&routine),
        );

        let info =
            RoutineInfo::from_routine(&routine, Some(crate::agent::routine::RunStatus::Running));

        assert_eq!(info.status, "running");
        assert_eq!(info.verification_status, "unverified");
    }

    #[test]
    fn test_routine_info_keeps_verified_state_when_disabled() {
        let mut routine = make_routine_for_status_tests();
        let fingerprint = crate::agent::routine::routine_verification_fingerprint(&routine);
        routine.state = crate::agent::routine::reset_routine_verification_state(
            &routine.state,
            fingerprint.clone(),
        );
        routine.state = crate::agent::routine::apply_routine_verification_result(
            &routine.state,
            fingerprint,
            crate::agent::routine::RunStatus::Ok,
            Utc::now(),
        );
        routine.enabled = false;

        let info = RoutineInfo::from_routine(&routine, Some(crate::agent::routine::RunStatus::Ok));

        assert_eq!(info.status, "disabled");
        assert_eq!(info.verification_status, "verified");
    }

    #[test]
    fn test_routine_info_treats_legacy_run_history_as_verified() {
        let mut routine = make_routine_for_status_tests();
        routine.run_count = 2;

        let info = RoutineInfo::from_routine(&routine, Some(crate::agent::routine::RunStatus::Ok));

        assert_eq!(info.status, "active");
        assert_eq!(info.verification_status, "verified");
    }

    #[test]
    fn test_routine_info_keeps_unverified_state_when_disabled() {
        let mut routine = make_routine_for_status_tests();
        routine.state = crate::agent::routine::reset_routine_verification_state(
            &routine.state,
            crate::agent::routine::routine_verification_fingerprint(&routine),
        );
        routine.enabled = false;

        let info = RoutineInfo::from_routine(&routine, None);

        assert_eq!(info.status, "disabled");
        assert_eq!(info.verification_status, "unverified");
    }
}
