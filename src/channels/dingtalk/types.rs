//! DingTalk API types and data structures.

use serde::{Deserialize, Serialize};

// ─── DingTalk Stream Protocol Types ─────────────────────────────────────────

/// A message frame received over the Stream WebSocket.
///
/// Per the DingTalk Stream protocol, `topic` and `messageId` live inside
/// the `headers` map — NOT as top-level fields. Helper methods extract them.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StreamFrame {
    /// "SYSTEM" for control, "CALLBACK" for messages, "EVENT" for events
    #[serde(rename = "type")]
    pub frame_type: Option<String>,
    /// JSON-encoded payload
    pub data: Option<String>,
    /// Protocol headers containing topic, messageId, contentType, time, etc.
    #[serde(default)]
    pub headers: Option<serde_json::Value>,
}

impl StreamFrame {
    pub fn topic(&self) -> Option<&str> {
        self.headers
            .as_ref()
            .and_then(|h| h.get("topic"))
            .and_then(|v| v.as_str())
    }

    pub fn message_id(&self) -> Option<&str> {
        self.headers
            .as_ref()
            .and_then(|h| h.get("messageId"))
            .and_then(|v| v.as_str())
    }
}

/// DingTalk bot callback message payload (inside StreamFrame.data).
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BotCallbackPayload {
    pub conversation_id: Option<String>,
    pub conversation_type: Option<String>,
    pub text: Option<TextContent>,
    #[serde(default)]
    pub rich_text: Option<serde_json::Value>,
    pub sender_id: Option<String>,
    pub sender_nick: Option<String>,
    pub sender_staff_id: Option<String>,
    pub msg_id: Option<String>,
    pub msgtype: Option<String>,
    pub robot_code: Option<String>,
    #[serde(default)]
    pub is_in_at_list: Option<bool>,
    /// Session webhook URL (for direct replies within ~1 hour)
    pub session_webhook: Option<String>,
    /// Session webhook expiry timestamp (ms)
    pub session_webhook_expired_time: Option<u64>,
    /// General content blob for non-text message types (audio, video, file, picture, etc.)
    /// DingTalk encodes this as a nested JSON string or object depending on msgtype.
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    /// Whether this message is a reply/quote to another message.
    #[serde(default, rename = "isReplyMsg")]
    pub is_reply_msg: Option<bool>,
    /// The quoted/replied-to message payload (present when `is_reply_msg` is true).
    #[serde(default, rename = "repliedMsg")]
    pub replied_msg: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
pub struct TextContent {
    pub content: Option<String>,
}

// ─── Metadata for Response Routing ──────────────────────────────────────────

/// Stored per incoming message for routing replies back.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DingTalkMetadata {
    pub conversation_id: String,
    pub conversation_type: String,
    pub sender_staff_id: String,
    pub sender_nick: String,
    pub msg_id: String,
    pub robot_code: Option<String>,
    /// Session webhook URL for direct reply (faster, no auth needed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_webhook: Option<String>,
    /// Session webhook expiry timestamp in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_webhook_expired_time: Option<u64>,
}

// ─── AI Card State ─────────────────────────────────────────────────────────

/// Phase of an AI streaming card lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardPhase {
    /// Card created, no content sent yet.
    Processing,
    /// Streaming content in progress.
    Inputing,
}

/// Coarse agent phase surfaced in the status line (icon + short text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPhase {
    Thinking,
    UsingTool,
    Generating,
}

impl AgentPhase {
    /// zh-CN label (icon + text) used on the first line of the rendered card.
    pub fn label(self) -> &'static str {
        match self {
            AgentPhase::Thinking => "🧠 思考中",
            AgentPhase::UsingTool => "🔧 调用工具",
            AgentPhase::Generating => "✍️ 生成回答",
        }
    }
}

/// Channel-level privacy gate: group chats default to opaque rendering, DMs
/// fall back to the existing `tool_call_detail`-style summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelLevel {
    /// Direct message — single recipient, verbose rendering allowed.
    Dm,
    /// Group chat — other members may observe; use display_name fallback for
    /// tools that have not opted into `safe_for_group_display`.
    Group,
}

/// Slow-operation escalation tier. One-way transitions: `None → Warn → Critical`.
/// Reset only when the card transitions to a terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlowTier {
    #[default]
    None,
    /// `>=15s` elapsed for the current phase.
    Warn,
    /// `>=60s` elapsed — also surfaces a `Reply /stop to cancel` hint.
    Critical,
}

/// Active-tool snapshot used by the status-line renderer. Populated when a
/// tool starts, cleared when it completes. `summary` is pre-scrubbed per the
/// current `ChannelLevel` so the renderer stays a pure function of state + time.
#[derive(Debug, Clone)]
pub struct ToolActivity {
    pub name: String,
    /// Scrubbed, channel-level-aware summary (e.g. `shell: ls -la`).
    pub summary: String,
    pub started_at: std::time::Instant,
}

/// State of an active AI streaming card for a single message.
///
/// Intentionally NOT `Clone` — owns a `tokio::task::JoinHandle` which is not
/// Clone. Reading code should take a borrow or extract just the fields it
/// needs; `cleanup_message_state` is the only path that consumes the tick
/// handle.
#[derive(Debug)]
pub struct CardState {
    /// DingTalk card instance ID returned by createAndDeliver.
    pub instance_id: String,
    /// Accumulated content buffer for the card.
    pub content_buffer: String,
    /// Accumulated thinking/reasoning buffer (for `all` mode).
    pub thinking_buffer: String,
    /// Last time real user-visible content was streamed to the card.
    ///
    /// The initial empty activation stream is tracked separately and must not
    /// consume the first real-content flush budget.
    pub last_content_update: Option<std::time::Instant>,
    /// Current phase of the card.
    pub phase: CardPhase,
    /// When true, stop attempting card delivery and fall back to markdown.
    pub fallback_required: bool,

    // ─── Anti-silence extensions (plan: 2026-04-18-001) ──────────────────

    /// Card creation wall-clock; the single source of truth for cumulative
    /// seconds shown as `(Ns)` in the status line. Never reset across phase
    /// transitions.
    pub created_at: std::time::Instant,
    /// Whether the card was created in a group chat or DM; drives the
    /// renderer's privacy gate and is fixed at card-creation time.
    pub channel_level: ChannelLevel,
    /// Current agent phase (🧠 / 🔧 / ✍️). Updated on `PhaseChanged` events.
    pub agent_phase: AgentPhase,
    /// Active tool snapshot (populated during `UsingTool`, cleared at completion).
    pub current_tool: Option<ToolActivity>,
    /// Scrubbed reasoning excerpt ("最近思路：…"), capped by the renderer.
    pub reasoning_excerpt: Option<String>,
    /// Snapshot of `SettingsStore` opt-in at card creation; the tick task and
    /// renderer read this in-state so the setting can't flip mid-turn.
    pub reasoning_summary_enabled: bool,
    /// Slow-operation escalation state (see [`SlowTier`]).
    pub slow_tier: SlowTier,
    /// Per-card cancel signal. The tick task awaits `cancel.notified()` and
    /// exits; `cleanup_message_state` fires `notify_one` + awaits the handle.
    pub tick_cancel: std::sync::Arc<tokio::sync::Notify>,
    /// Handle to the per-card tick task. Moved out and awaited on cleanup.
    pub tick_handle: Option<tokio::task::JoinHandle<()>>,
    /// When true, this card is beyond the `max_active_cards` cap: the tick
    /// task skips 2s-interval PUTs and only emits on R9 threshold crossings
    /// or phase/terminal events.
    pub tick_degraded: bool,
    /// Strings that were previously seen as sensitive tool params/returns
    /// on this card. The reasoning scrubber substring-matches against this
    /// set to prevent a sensitive value from leaking via the reasoning path.
    /// Cleared at cleanup; never persisted, never crosses request IDs.
    pub seen_sensitive: std::collections::HashSet<String>,
    /// Originating user id, carried for the supersede secondary index.
    pub originating_user_id: String,
    /// Number of tools used so far (for the `✅ Used N tools · Ys` summary).
    pub tools_used: u32,
    /// Retry attempt counter (R12). `0 = no retry yet`, `1 = retrying now`,
    /// `2 = terminal ❌`.
    pub retry_attempt: u8,
}

// ─── DingTalk API Types ─────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct MarkdownMsgParam {
    pub title: String,
    pub text: String,
}
