//! Channel trait and message types.

use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use uuid::Uuid;

use crate::error::ChannelError;

/// Kind of attachment carried on an incoming message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentKind {
    /// Audio content (voice notes, audio files).
    Audio,
    /// Image content (photos, screenshots).
    Image,
    /// Video content (video files, clips).
    Video,
    /// Document content (PDFs, files).
    Document,
}

impl AttachmentKind {
    /// Infer attachment kind from MIME type.
    pub fn from_mime_type(mime: &str) -> Self {
        let base = mime.split(';').next().unwrap_or(mime).trim();
        if base.starts_with("audio/") {
            Self::Audio
        } else if base.starts_with("image/") {
            Self::Image
        } else if base.starts_with("video/") {
            Self::Video
        } else {
            Self::Document
        }
    }
}

/// A file or media attachment on an incoming message.
#[derive(Debug, Clone)]
pub struct IncomingAttachment {
    /// Unique identifier within the channel (e.g., Telegram file_id).
    pub id: String,
    /// What kind of content this is.
    pub kind: AttachmentKind,
    /// MIME type (e.g., "image/jpeg", "audio/ogg", "application/pdf").
    pub mime_type: String,
    /// Original filename, if known.
    pub filename: Option<String>,
    /// File size in bytes, if known.
    pub size_bytes: Option<u64>,
    /// URL to download the file from the channel's API.
    pub source_url: Option<String>,
    /// Opaque key for host-side storage (e.g., after download/caching).
    pub storage_key: Option<String>,
    /// Extracted text content (e.g., OCR result, PDF text, audio transcript).
    pub extracted_text: Option<String>,
    /// Raw file bytes (for small files downloaded by the channel).
    pub data: Vec<u8>,
    /// Duration in seconds (for audio/video).
    pub duration_secs: Option<u32>,
}

/// A message received from an external channel.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// Unique message ID.
    pub id: Uuid,
    /// Channel this message came from.
    pub channel: String,
    /// Resolved owner identity for this message.
    ///
    /// For owner-capable channels this is the stable instance owner ID when the
    /// configured owner is speaking; for pairing-aware channels (e.g. WASM) this
    /// is the result of `pairing_resolve_identity`; for non-pairing channels
    /// (HTTP, web, REPL) it comes directly from the authenticated token.
    pub user_id: String,
    /// Channel-specific sender/actor identifier.
    pub sender_id: String,
    /// Optional display name.
    pub user_name: Option<String>,
    /// Message content.
    pub content: String,
    /// Thread/conversation ID for threaded conversations.
    pub thread_id: Option<String>,
    /// Stable channel/chat/thread scope for this conversation.
    pub conversation_scope_id: Option<String>,
    /// When the message was received.
    pub received_at: DateTime<Utc>,
    /// Channel-specific metadata.
    pub metadata: serde_json::Value,
    /// IANA timezone string from the client (e.g. "America/New_York").
    pub timezone: Option<String>,
    /// File or media attachments on this message.
    pub attachments: Vec<IncomingAttachment>,
    /// Internal-only flag: message was generated inside the process (e.g. job
    /// monitor) and must bypass the normal user-input pipeline. This field is
    /// not settable via metadata, so external channels cannot spoof it.
    pub(crate) is_internal: bool,
    /// `true` when this message represents an *agent broadcast* echoing
    /// back through the channel — e.g. an outbound bot message that the
    /// channel adapter (Slack, Discord, etc.) re-delivers as an inbound
    /// event. Mission `OnEvent` firing skips messages with this flag set
    /// to avoid self-recursion: a mission whose pattern matches its own
    /// output would otherwise re-trigger forever.
    ///
    /// Channel adapters that have echo behavior MUST set this when
    /// re-emitting the agent's own outbound text. Adapters without echo
    /// behavior (CLI, REPL, web gateway) leave it `false`.
    pub is_agent_broadcast: bool,
    /// When set, this message was produced as a side effect of a mission
    /// firing — typically the mission's notification text re-entering
    /// through a channel adapter. Mission `OnEvent` firing skips messages
    /// tagged with a `triggering_mission_id` to bound chain-recursion
    /// across distinct missions (mission A → notification → mission B →
    /// notification → mission C → ...). The string is the originating
    /// `MissionId` for diagnostics.
    pub triggering_mission_id: Option<String>,
}

impl IncomingMessage {
    /// Create a new incoming message.
    pub fn new(
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let user_id = user_id.into();
        Self {
            id: Uuid::new_v4(),
            channel: channel.into(),
            sender_id: user_id.clone(),
            user_id,
            user_name: None,
            content: content.into(),
            thread_id: None,
            conversation_scope_id: None,
            received_at: Utc::now(),
            metadata: serde_json::Value::Null,
            timezone: None,
            attachments: Vec::new(),
            is_internal: false,
            is_agent_broadcast: false,
            triggering_mission_id: None,
        }
    }

    /// Mark this message as an agent broadcast echo. Channel adapters that
    /// re-emit the agent's own outbound text as an inbound event MUST call
    /// this so mission `OnEvent` firing skips it.
    pub fn with_agent_broadcast(mut self) -> Self {
        self.is_agent_broadcast = true;
        self
    }

    /// Mark this message as having been produced by a mission firing.
    /// Used for chain-recursion guards across distinct missions.
    pub fn with_triggering_mission(mut self, mission_id: impl Into<String>) -> Self {
        self.triggering_mission_id = Some(mission_id.into());
        self
    }

    /// Set the thread ID.
    pub fn with_thread(mut self, thread_id: impl Into<String>) -> Self {
        let thread_id = thread_id.into();
        self.conversation_scope_id = Some(thread_id.clone());
        self.thread_id = Some(thread_id);
        self
    }

    /// Set the channel-specific sender/actor identifier.
    pub fn with_sender_id(mut self, sender_id: impl Into<String>) -> Self {
        self.sender_id = sender_id.into();
        self
    }

    /// Set the conversation scope for this message.
    pub fn with_conversation_scope(mut self, scope_id: impl Into<String>) -> Self {
        self.conversation_scope_id = Some(scope_id.into());
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set user name.
    pub fn with_user_name(mut self, name: impl Into<String>) -> Self {
        self.user_name = Some(name.into());
        self
    }

    /// Set the client timezone.
    pub fn with_timezone(mut self, tz: impl Into<String>) -> Self {
        self.timezone = Some(tz.into());
        self
    }

    /// Set attachments.
    pub fn with_attachments(mut self, attachments: Vec<IncomingAttachment>) -> Self {
        self.attachments = attachments;
        self
    }

    /// Mark this message as internal (bypasses user-input pipeline).
    pub(crate) fn into_internal(mut self) -> Self {
        self.is_internal = true;
        self
    }

    /// Effective conversation scope, falling back to thread_id for legacy callers.
    pub fn conversation_scope(&self) -> Option<&str> {
        self.conversation_scope_id
            .as_deref()
            .or(self.thread_id.as_deref())
    }

    /// Best-effort routing target for proactive replies on the current channel.
    pub fn routing_target(&self) -> Option<String> {
        routing_target_from_metadata(&self.metadata).or_else(|| {
            if self.sender_id.is_empty() {
                None
            } else {
                Some(self.sender_id.clone())
            }
        })
    }
}

/// Extract a channel-specific proactive routing target from message metadata.
///
/// Checked keys (first match wins):
/// - `signal_target` — Signal phone number or group ID
/// - `chat_id` — Telegram chat ID
/// - `channel_id` — Slack channel/DM ID (used by channel-relay)
/// - `target` — generic fallback
pub fn routing_target_from_metadata(metadata: &serde_json::Value) -> Option<String> {
    // Helper to extract a string or numeric value from a JSON key.
    let extract = |key: &str| -> Option<String> {
        metadata.get(key).and_then(|value| match value {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    };

    extract("signal_target")
        .or_else(|| extract("chat_id"))
        .or_else(|| extract("channel_id"))
        .or_else(|| extract("target"))
}

/// Stream of incoming messages.
pub type MessageStream = Pin<Box<dyn Stream<Item = IncomingMessage> + Send>>;

/// Response to send back to a channel.
#[derive(Debug, Clone)]
pub struct OutgoingResponse {
    /// The content to send.
    pub content: String,
    /// Optional thread ID to reply in.
    pub thread_id: Option<String>,
    /// Optional file paths to attach.
    pub attachments: Vec<String>,
    /// Channel-specific metadata for the response.
    pub metadata: serde_json::Value,
}

impl OutgoingResponse {
    /// Create a simple text response.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            thread_id: None,
            attachments: Vec::new(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Set the thread ID for the response.
    pub fn in_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    /// Add attachments to the response.
    pub fn with_attachments(mut self, paths: Vec<String>) -> Self {
        self.attachments = paths;
        self
    }
}

/// A single tool decision within a reasoning update.
#[derive(Debug, Clone)]
pub struct ToolDecision {
    /// Tool name.
    pub tool_name: String,
    /// Agent's reasoning for choosing this tool.
    pub rationale: String,
}

/// Coarse-grained agent phase, emitted as a transition signal so channels
/// can render a phase-aware status line (icon + text) without inferring
/// from the sequence of Thinking/ToolStarted/StreamChunk events.
///
/// Variants are stable: channels that don't care should ignore the whole
/// [`StatusUpdate::PhaseChanged`] variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Model is reasoning / preparing the next step.
    Thinking,
    /// A tool is executing.
    UsingTool,
    /// Model is streaming answer tokens.
    Generating,
}

/// Status update types for showing agent activity.
#[derive(Debug, Clone)]
pub enum StatusUpdate {
    /// Agent is thinking/processing.
    Thinking(String),
    /// Agent phase transitioned. Emitted only on transitions, not every tick.
    PhaseChanged(Phase),
    /// Tool execution started.
    ToolStarted {
        name: String,
        /// Short contextual summary extracted from tool arguments.
        detail: Option<String>,
        /// Stable tool-call ID when available, used to disambiguate repeated
        /// calls to the same tool name in a single turn.
        call_id: Option<String>,
    },
    /// Tool execution completed.
    ///
    /// Use [`StatusUpdate::tool_completed`] to construct this variant — it
    /// handles redaction of sensitive parameters and keeps the 9-line pattern
    /// in one place.
    ToolCompleted {
        name: String,
        success: bool,
        /// Error message when success is false.
        error: Option<String>,
        /// Tool input parameters (JSON string) for display on failure.
        /// Only populated when `success` is `false`. Values listed in the
        /// tool's `sensitive_params()` are replaced with `"[REDACTED]"`.
        parameters: Option<String>,
        /// Stable tool-call ID when available.
        call_id: Option<String>,
    },
    /// Brief preview of tool execution output.
    ToolResult {
        name: String,
        preview: String,
        /// Stable tool-call ID when available.
        call_id: Option<String>,
    },
    /// Streaming text chunk.
    StreamChunk(String),
    /// General status message.
    Status(String),
    /// A sandbox job has started (shown as a clickable card in the UI).
    JobStarted {
        job_id: String,
        title: String,
        browse_url: String,
    },
    /// Tool requires user approval before execution.
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: serde_json::Value,
        /// When `true`, the UI should offer an "always" option that auto-approves
        /// future calls to this tool for the rest of the session.  When `false`
        /// (i.e. `ApprovalRequirement::Always`), the tool must be approved every
        /// time and the "always" button should be hidden.
        allow_always: bool,
    },
    /// Extension needs user authentication (token or OAuth).
    AuthRequired {
        extension_name: String,
        instructions: Option<String>,
        auth_url: Option<String>,
        setup_url: Option<String>,
    },
    /// Extension authentication completed.
    AuthCompleted {
        extension_name: String,
        success: bool,
        message: String,
    },
    /// An image was generated by a tool.
    ImageGenerated {
        /// Base64 data URL of the generated image.
        data_url: String,
        /// Optional workspace path where the image was saved.
        path: Option<String>,
    },
    /// A sandbox job's status changed.
    JobStatus { job_id: String, status: String },
    /// A sandbox job completed with final result.
    JobResult { job_id: String, status: String },
    /// A routine was created, updated, or deleted.
    RoutineUpdate {
        id: String,
        name: String,
        trigger_type: String,
        enabled: bool,
        last_run: Option<String>,
        next_fire: Option<String>,
    },
    /// Context pressure update (token usage approaching limit).
    ContextPressure {
        used_tokens: u64,
        max_tokens: u64,
        percentage: u8,
        warning: Option<String>,
    },
    /// Sandbox / Docker status update.
    SandboxStatus {
        docker_available: bool,
        running_containers: u32,
        status: String,
    },
    /// Secrets vault status update.
    SecretsStatus { count: u32, vault_unlocked: bool },
    /// Cost guard / budget status update.
    CostGuard {
        session_budget_usd: Option<String>,
        spent_usd: String,
        remaining_usd: Option<String>,
        limit_reached: bool,
    },
    /// Suggested follow-up messages for the user.
    Suggestions { suggestions: Vec<String> },
    /// Agent reasoning update (why it chose specific tools).
    ReasoningUpdate {
        /// Human-readable summary of the agent's decision.
        narrative: String,
        /// Per-tool decisions.
        decisions: Vec<ToolDecision>,
    },
    /// Per-turn token usage and cost summary (shown as subtle metadata).
    TurnCost {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: String,
    },
    /// Skills activated for this conversation turn.
    SkillActivated { skill_names: Vec<String> },
    /// Thread list for interactive resume picker.
    ThreadList { threads: Vec<ThreadSummary> },
    /// Engine v2 thread list for TUI activity sidebar.
    EngineThreadList { threads: Vec<EngineThreadSummary> },
    /// Full conversation history for displaying a resumed thread in the TUI.
    ConversationHistory {
        thread_id: String,
        messages: Vec<HistoryMessage>,
        pending_approval: Option<ChatApprovalPrompt>,
    },
}

/// A single message from conversation history, for hydrating the TUI on thread resume.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Engine v2 thread summary for TUI sidebar display.
#[derive(Debug, Clone)]
pub struct EngineThreadSummary {
    pub id: String,
    pub goal: String,
    pub thread_type: String,
    pub state: String,
    pub step_count: usize,
    pub total_tokens: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// Lightweight thread summary for the resume picker.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    pub title: Option<String>,
    pub message_count: i64,
    pub last_activity: String,
    pub channel: String,
}

/// Shared chat-style approval prompt formatting used by non-web channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatApprovalPrompt {
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub allow_always: bool,
}

const APPROVAL_PARAMETER_PREVIEW_BYTES: usize = 1200;
const APPROVAL_PARAMETER_TRUNCATION_SUFFIX: &str = "\n... [parameters truncated]";
const APPROVAL_SUMMARY_DESCRIPTION_BYTES: usize = 120;
const DETAIL_MAX_LEN: usize = 80;

/// Extract a short, non-sensitive one-liner from tool arguments.
///
/// Returns `None` for unknown tools or when no relevant field is present.
pub fn tool_call_detail(name: &str, args: &serde_json::Value) -> Option<String> {
    let raw = match name {
        "http" | "http_request" | "web_fetch" => {
            let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
            let url = args.get("url").and_then(|v| v.as_str())?;
            format!("{method} {url}")
        }
        "shell" | "execute_command" => args.get("command").and_then(|v| v.as_str())?.to_string(),
        "read_file" | "write_file" | "list_dir" => {
            args.get("path").and_then(|v| v.as_str())?.to_string()
        }
        "memory_search" => {
            let q = args.get("query").and_then(|v| v.as_str())?;
            format!("query: {q}")
        }
        "memory_read" => args.get("path").and_then(|v| v.as_str())?.to_string(),
        "memory_write" => {
            let target = args.get("target").and_then(|v| v.as_str())?;
            format!("target: {target}")
        }
        "create_job" => args.get("title").and_then(|v| v.as_str())?.to_string(),
        "message" | "send_message" => {
            let channel = args.get("channel").and_then(|v| v.as_str())?;
            format!("to: {channel}")
        }
        "skill_search" | "tool_search" => args.get("query").and_then(|v| v.as_str())?.to_string(),
        _ => return None,
    };

    Some(truncate_detail(&raw))
}

fn truncate_detail(s: &str) -> String {
    if s.chars().count() <= DETAIL_MAX_LEN {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(DETAIL_MAX_LEN.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

impl StatusUpdate {
    /// Build a `ToolStarted` status with a derived contextual detail.
    pub fn tool_started(name: String, arguments: &serde_json::Value) -> Self {
        Self::tool_started_with_id(name, arguments, None)
    }

    /// Build a `ToolStarted` status with an explicit tool-call ID.
    pub fn tool_started_with_id(
        name: String,
        arguments: &serde_json::Value,
        call_id: Option<String>,
    ) -> Self {
        Self::ToolStarted {
            detail: tool_call_detail(&name, arguments),
            name,
            call_id,
        }
    }

    /// Build a `ToolCompleted` status with redacted parameters.
    ///
    /// On failure, serializes the tool's input parameters as pretty JSON after
    /// replacing any keys listed in the tool's `sensitive_params()` with
    /// `"[REDACTED]"`. On success, no parameters or error are included.
    ///
    /// Pass the resolved `Tool` reference (if available) so this method can
    /// query `sensitive_params()` directly — callers don't need to manage the
    /// borrow lifetime of the sensitive slice.
    pub fn tool_completed(
        name: String,
        call_id: Option<String>,
        result: &Result<String, crate::error::Error>,
        params: &serde_json::Value,
        tool: Option<&dyn crate::tools::Tool>,
    ) -> Self {
        let success = result.is_ok();
        let sensitive = tool.map(|t| t.sensitive_params()).unwrap_or(&[]);
        Self::ToolCompleted {
            name,
            success,
            error: result.as_ref().err().map(|e| e.to_string()),
            parameters: if !success {
                let safe = crate::tools::redact_params(params, sensitive);
                Some(serde_json::to_string_pretty(&safe).unwrap_or_else(|_| safe.to_string()))
            } else {
                None
            },
            call_id,
        }
    }
}

impl ChatApprovalPrompt {
    /// Build a shared chat approval prompt from a status update.
    pub fn from_status(status: &StatusUpdate) -> Option<Self> {
        let StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
            allow_always,
        } = status
        else {
            return None;
        };

        Some(Self {
            request_id: request_id.clone(),
            tool_name: tool_name.clone(),
            description: description.clone(),
            parameters: parameters.clone(),
            allow_always: *allow_always,
        })
    }

    fn truncated_text(input: &str, max_bytes: usize, suffix: &str) -> String {
        if input.len() <= max_bytes {
            return input.to_string();
        }

        let budget = max_bytes.saturating_sub(suffix.len());
        let end = crate::util::floor_char_boundary(input, budget);
        format!("{}{}", &input[..end], suffix)
    }

    /// Pretty-printed tool parameters for display, bounded for chat channels.
    pub fn parameters_preview(&self) -> String {
        let rendered = serde_json::to_string_pretty(&self.parameters)
            .unwrap_or_else(|_| self.parameters.to_string());
        Self::truncated_text(
            &rendered,
            APPROVAL_PARAMETER_PREVIEW_BYTES,
            APPROVAL_PARAMETER_TRUNCATION_SUFFIX,
        )
    }

    /// Shared reply vocabulary summary for compact status surfaces.
    pub fn reply_summary(&self) -> &'static str {
        if self.allow_always {
            "yes (or /approve), no (or /deny), or always (or /always)"
        } else {
            "yes (or /approve) or no (or /deny)"
        }
    }

    /// Compact approval summary for fallback/accessibility surfaces.
    pub fn summary_text(&self) -> String {
        let description = Self::truncated_text(
            &self.description.replace('\n', " "),
            APPROVAL_SUMMARY_DESCRIPTION_BYTES,
            "...",
        );
        format!(
            "Approval needed for {}: {} (Request ID: {}). Reply with {}.",
            self.tool_name,
            description,
            self.request_id,
            self.reply_summary()
        )
    }

    fn markdown_parameters_preview(&self) -> String {
        self.parameters_preview().replace('`', "\\`")
    }

    /// Approval prompt formatted for plain-text chat channels.
    pub fn plain_text_message(&self) -> String {
        let mut lines = vec![
            format!("Approval needed: {}", self.tool_name),
            self.description.clone(),
            String::new(),
            format!("Request ID: {}", self.request_id),
            "Parameters:".to_string(),
            self.parameters_preview(),
            String::new(),
            "Reply with:".to_string(),
            "- yes, y, approve, or /approve to approve this request".to_string(),
        ];

        if self.allow_always {
            lines.push(format!(
                "- always, a, or /always to approve this request and auto-approve future {} requests",
                self.tool_name
            ));
        }

        lines.push("- no, n, deny, or /deny to deny this request".to_string());
        lines.join("\n")
    }

    /// Approval prompt formatted for Markdown-capable chat channels.
    pub fn markdown_message(&self) -> String {
        let mut lines = vec![
            "⚠️ *Approval Required*".to_string(),
            String::new(),
            format!("*Request ID:* `{}`", self.request_id),
            format!("*Tool:* {}", self.tool_name),
            format!("*Description:* {}", self.description),
            "*Parameters:*".to_string(),
            format!("```json\n{}\n```", self.markdown_parameters_preview()),
            String::new(),
            "Reply with:".to_string(),
            "• `yes`, `y`, `approve`, or `/approve` - Approve this request".to_string(),
        ];

        if self.allow_always {
            lines.push(format!(
                "• `always`, `a`, or `/always` - Approve this request and auto-approve future {} requests",
                self.tool_name
            ));
        }

        lines.push("• `no`, `n`, `deny`, or `/deny` - Deny this request".to_string());
        lines.join("\n")
    }
}

/// Trait for message channels.
///
/// Channels receive messages from external sources and convert them to
/// a unified format. They also handle sending responses back.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Get the channel name (e.g., "cli", "slack", "telegram", "http").
    fn name(&self) -> &str;

    /// Start listening for messages.
    ///
    /// Returns a stream of incoming messages. The channel should handle
    /// reconnection and error recovery internally.
    async fn start(&self) -> Result<MessageStream, ChannelError>;

    /// Send a response back to the user.
    ///
    /// The response is sent in the context of the original message
    /// (same channel, same thread if applicable).
    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError>;

    /// Send a status update (thinking, tool execution, etc.).
    ///
    /// The metadata contains channel-specific routing info (e.g., Telegram chat_id)
    /// needed to deliver the status to the correct destination.
    ///
    /// Default implementation does nothing (for channels that don't support status).
    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Send a proactive message without a prior incoming message.
    ///
    /// Used for alerts, heartbeat notifications, and other agent-initiated communication.
    /// The user_id helps target a specific user within the channel.
    ///
    /// Default implementation does nothing (for channels that don't support broadcast).
    async fn broadcast(
        &self,
        _user_id: &str,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Check if the channel is healthy.
    async fn health_check(&self) -> Result<(), ChannelError>;

    /// Get conversation context from message metadata for system prompt.
    ///
    /// Returns key-value pairs like "sender", "sender_uuid", "group" that
    /// help the LLM understand who it's talking to.
    ///
    /// Default implementation returns empty map.
    fn conversation_context(&self, _metadata: &serde_json::Value) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Gracefully shut down the channel.
    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

/// Trait for channels that support hot-secret-swapping during SIGHUP reload.
///
/// This allows channels to update authentication credentials without restarting,
/// enabling zero-downtime configuration reloads. Channels that don't support
/// secret updates can simply not implement this trait.
#[async_trait]
pub trait ChannelSecretUpdater: Send + Sync {
    /// Update the secret for this channel.
    ///
    /// Called during SIGHUP configuration reload. Implementation should:
    /// - Apply the new secret atomically
    /// - Not fail the entire reload if secret update fails
    /// - Log appropriate errors/info messages
    ///
    /// The secret is optional (may be None if secret is no longer configured).
    async fn update_secret(&self, new_secret: Option<secrecy::SecretString>);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::credentials::TEST_REDACT_SECRET_123;

    /// Stub tool that marks `"value"` as sensitive.
    struct SecretTool;

    #[async_trait]
    impl crate::tools::Tool for SecretTool {
        fn name(&self) -> &str {
            "secret_save"
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &crate::context::JobContext,
        ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
            unreachable!()
        }
        fn sensitive_params(&self) -> &[&str] {
            &["value"]
        }
    }

    #[test]
    fn tool_completed_redacts_sensitive_params_on_failure() {
        let params = serde_json::json!({"name": "api_key", "value": TEST_REDACT_SECRET_123});
        let err: Result<String, crate::error::Error> =
            Err(crate::error::ToolError::ExecutionFailed {
                name: "secret_save".into(),
                reason: "db error".into(),
            }
            .into());
        let tool = SecretTool;

        let status = StatusUpdate::tool_completed(
            "secret_save".into(),
            None,
            &err,
            &params,
            Some(&tool as &dyn crate::tools::Tool),
        );

        if let StatusUpdate::ToolCompleted {
            success,
            error,
            parameters,
            ..
        } = &status
        {
            assert!(!success);
            let err_msg = error.as_deref().expect("should have error");
            assert!(err_msg.contains("db error"), "error: {}", err_msg);
            let param_str = parameters
                .as_ref()
                .expect("should have parameters on failure");
            assert!(
                param_str.contains("[REDACTED]"),
                "sensitive value should be redacted: {}",
                param_str
            );
            assert!(
                !param_str.contains(TEST_REDACT_SECRET_123),
                "raw secret should not appear: {}",
                param_str
            );
            assert!(
                param_str.contains("api_key"),
                "non-sensitive params should be preserved: {}",
                param_str
            );
        } else {
            panic!("expected ToolCompleted variant");
        }
    }

    #[test]
    fn tool_completed_no_params_on_success() {
        let params = serde_json::json!({"name": "key", "value": "secret"});
        let ok: Result<String, crate::error::Error> = Ok("done".into());

        let status = StatusUpdate::tool_completed("secret_save".into(), None, &ok, &params, None);

        if let StatusUpdate::ToolCompleted {
            success,
            error,
            parameters,
            ..
        } = &status
        {
            assert!(success);
            assert!(error.is_none());
            assert!(parameters.is_none(), "no params should be sent on success");
        } else {
            panic!("expected ToolCompleted variant");
        }
    }

    #[test]
    fn tool_completed_no_tool_passes_params_unredacted() {
        let params = serde_json::json!({"cmd": "ls -la"});
        let err: Result<String, crate::error::Error> =
            Err(crate::error::ToolError::ExecutionFailed {
                name: "shell".into(),
                reason: "timeout".into(),
            }
            .into());

        let status = StatusUpdate::tool_completed("shell".into(), None, &err, &params, None);

        if let StatusUpdate::ToolCompleted { parameters, .. } = &status {
            let param_str = parameters.as_ref().expect("should have parameters");
            assert!(
                param_str.contains("ls -la"),
                "non-sensitive params should pass through: {}",
                param_str
            );
        } else {
            panic!("expected ToolCompleted variant");
        }
    }

    #[test]
    fn test_incoming_message_with_timezone() {
        let msg = IncomingMessage::new("test", "user1", "hello").with_timezone("America/New_York");
        assert_eq!(msg.timezone.as_deref(), Some("America/New_York"));
    }

    #[test]
    fn tool_call_detail_http() {
        let args = serde_json::json!({"method": "POST", "url": "https://api.example.com/data"});
        let detail = super::tool_call_detail("http", &args);
        assert_eq!(detail.as_deref(), Some("POST https://api.example.com/data"));
    }

    #[test]
    fn tool_call_detail_shell() {
        let args = serde_json::json!({"command": "cargo test --all"});
        let detail = super::tool_call_detail("shell", &args);
        assert_eq!(detail.as_deref(), Some("cargo test --all"));
    }

    #[test]
    fn tool_call_detail_memory_search() {
        let args = serde_json::json!({"query": "database migration"});
        let detail = super::tool_call_detail("memory_search", &args);
        assert_eq!(detail.as_deref(), Some("query: database migration"));
    }

    #[test]
    fn tool_call_detail_unknown_tool() {
        let args = serde_json::json!({"foo": "bar"});
        let detail = super::tool_call_detail("unknown_tool_xyz", &args);
        assert!(detail.is_none());
    }

    #[test]
    fn tool_call_detail_truncation() {
        let long_url = format!("https://example.com/{}", "a".repeat(100));
        let args = serde_json::json!({"url": long_url});
        let detail = super::tool_call_detail("http", &args).unwrap();
        assert!(detail.chars().count() <= super::DETAIL_MAX_LEN);
        assert!(detail.ends_with("..."));
    }

    #[test]
    fn routing_target_extracts_slack_channel_id() {
        // Slack relay messages carry channel_id in metadata — this must be
        // picked up for proactive broadcasts to land in the correct channel
        // instead of falling back to sender_id (which routes to DMs).
        let metadata = serde_json::json!({
            "team_id": "T05CUBCSQPL",
            "channel_id": "C088K6C3SQZ",
            "sender_id": "UCBGL1WNS",
        });
        assert_eq!(
            routing_target_from_metadata(&metadata).as_deref(),
            Some("C088K6C3SQZ"),
        );
    }

    #[test]
    fn routing_target_prefers_signal_over_channel_id() {
        let metadata = serde_json::json!({
            "signal_target": "+15551234567",
            "channel_id": "C088K6C3SQZ",
        });
        assert_eq!(
            routing_target_from_metadata(&metadata).as_deref(),
            Some("+15551234567"),
        );
    }

    #[test]
    fn routing_target_prefers_chat_id_over_channel_id() {
        let metadata = serde_json::json!({
            "chat_id": "123456789",
            "channel_id": "C088K6C3SQZ",
        });
        assert_eq!(
            routing_target_from_metadata(&metadata).as_deref(),
            Some("123456789"),
        );
    }

    #[test]
    fn routing_target_returns_none_for_empty_metadata() {
        let metadata = serde_json::json!({});
        assert!(routing_target_from_metadata(&metadata).is_none());
    }

    #[test]
    fn chat_approval_prompt_plain_text_includes_all_reply_forms() {
        let prompt = ChatApprovalPrompt::from_status(&StatusUpdate::ApprovalNeeded {
            request_id: "req-123".into(),
            tool_name: "http".into(),
            description: "Fetch weather data".into(),
            parameters: serde_json::json!({"url": "https://api.weather.test"}),
            allow_always: true,
        })
        .expect("approval prompt");

        let text = prompt.plain_text_message();
        assert!(text.contains("Request ID: req-123"));
        assert!(text.contains("approve, or /approve"));
        assert!(text.contains("always, a, or /always"));
        assert!(text.contains("deny, or /deny"));
    }

    #[test]
    fn chat_approval_prompt_hides_always_when_not_allowed() {
        let prompt = ChatApprovalPrompt::from_status(&StatusUpdate::ApprovalNeeded {
            request_id: "req-456".into(),
            tool_name: "shell".into(),
            description: "Run command".into(),
            parameters: serde_json::json!({"command": "rm -rf /tmp/demo"}),
            allow_always: false,
        })
        .expect("approval prompt");

        let markdown = prompt.markdown_message();
        assert!(markdown.contains("`/approve`"));
        assert!(markdown.contains("`/deny`"));
        assert!(!markdown.contains("`/always`"));
    }

    #[test]
    fn chat_approval_prompt_truncates_large_parameters() {
        let prompt = ChatApprovalPrompt::from_status(&StatusUpdate::ApprovalNeeded {
            request_id: "req-789".into(),
            tool_name: "http".into(),
            description: "Fetch large payload".into(),
            parameters: serde_json::json!({
                "body": "x".repeat(APPROVAL_PARAMETER_PREVIEW_BYTES + 200),
            }),
            allow_always: true,
        })
        .expect("approval prompt");

        let preview = prompt.parameters_preview();
        assert!(preview.contains("[parameters truncated]"));
        assert!(preview.len() <= APPROVAL_PARAMETER_PREVIEW_BYTES);
    }

    #[test]
    fn chat_approval_prompt_escapes_backticks_in_markdown_parameters() {
        let prompt = ChatApprovalPrompt::from_status(&StatusUpdate::ApprovalNeeded {
            request_id: "req-999".into(),
            tool_name: "shell".into(),
            description: "Run command".into(),
            parameters: serde_json::json!({
                "command": "printf '```danger```'"
            }),
            allow_always: true,
        })
        .expect("approval prompt");

        let markdown = prompt.markdown_message();
        assert!(markdown.contains("\\`\\`\\`danger\\`\\`\\`"));
    }
}
