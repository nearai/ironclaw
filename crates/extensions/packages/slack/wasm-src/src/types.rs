//! Types for Slack user-token API requests and responses.
//!
//! Field names and output shapes mirror the standardized messaging
//! framework's canonical contracts
//! (`crates/ironclaw_host_api/schemas/messaging/*.json`) exactly: the host
//! validates every standard op's input pre-dispatch and its output
//! post-dispatch against those schemas, so a shape drift here becomes a
//! model-visible tool failure, not a silent mismatch.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Invocation context the host passes alongside params. The host selects the
/// operation via the capability id (e.g. `slack.search_messages`); the
/// action is NOT carried in the params object.
#[derive(Debug, Deserialize)]
pub(crate) struct ToolContext {
    pub(crate) capability_id: String,
}

/// A conversation kind, per the standard messaging vocabulary. Slack has no
/// native "other" conversation type; it exists so the enum is total and a
/// future vendor with an unmapped kind has somewhere to put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Dm,
    GroupDm,
    Channel,
    Other,
}

/// Input parameters for the Slack personal (user-token) tool.
///
/// `JsonSchema` is derived so the advertised tool schema mirrors the
/// serde-enforced contract: each variant becomes a `oneOf` entry with
/// its own `required` array. Field names are the standard messaging
/// framework's canonical names (never Slack-native spellings like `channel`
/// or `thread_ts`) — the host's pre-dispatch validation enforces this
/// against the canonical input schema regardless of what this derive
/// advertises, so the two must never drift apart.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SlackUserAction {
    /// Search across all messages you can see (DMs, group DMs, and
    /// channels you are a member of). Requires the `search:read` user scope.
    SearchMessages {
        /// Search query. Supports Slack search operators: `from:me` for your
        /// own messages (NOT `from:@me` — there is no user named "me"), plus
        /// `from:@username`, `in:#channel`, `after:2024-01-01`, `has:link`.
        query: String,
        /// Maximum number of matches to return (default: 20, max: 100).
        #[serde(default)]
        limit: Option<u32>,
        /// Opaque pagination cursor from a previous call's `next_cursor`.
        /// Slack paginates search results by page number; this cursor
        /// encodes that page number as an opaque string.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// List conversations visible to you: channels, private channels,
    /// DMs, and group DMs (`is_member` marks which channels you belong
    /// to). Use this to discover DM conversation IDs.
    ListConversations {
        /// Restrict results to these conversation kinds. Omit to return
        /// every kind.
        #[serde(default)]
        kinds: Option<Vec<ConversationKind>>,
        /// Maximum number of conversations to return (default: 200).
        #[serde(default)]
        limit: Option<u32>,
        /// Pagination cursor from a previous call's `next_cursor`.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// Retrieve one exact conversation by its known conversation ID. For a
    /// DM, the returned `counterpart` is the authoritative counterpart.
    GetConversationInfo {
        /// Conversation ref for this extension (e.g. `C123...` for a
        /// channel, `D123...` for a DM).
        conversation: String,
    },

    /// Read message history from any conversation you can see — a channel,
    /// a DM, or a group DM — identified by its conversation ID.
    GetConversationHistory {
        /// Conversation ref (e.g. `C123...` for a channel, `D123...` for a
        /// DM).
        conversation: String,
        /// Maximum number of messages to return (default: 50, max: 999 —
        /// Slack rejects 1000; out-of-range values are clamped).
        #[serde(default)]
        limit: Option<u32>,
        /// Opaque pagination cursor from a previous call's `next_cursor`.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// Read the replies of one thread (`conversations.replies`). Thread
    /// replies are NOT part of conversation history — the parent's
    /// `reply_count`/thread anchor point here.
    GetThreadReplies {
        /// Conversation ref the thread lives in (e.g. `C123...`).
        conversation: String,
        /// The thread parent message's `ts` (opaque thread anchor).
        thread: String,
        /// Maximum number of messages to return (default: 50, max: 999).
        #[serde(default)]
        limit: Option<u32>,
        /// Opaque pagination cursor from a previous call's `next_cursor`.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// Get information about a user (name, real name).
    GetUserInfo {
        /// Opaque user id from this extension's people operations (e.g.
        /// "U1234567890").
        user_ref: String,
    },

    /// Resolve who the connected Slack account is (`auth.test`), with a
    /// best-effort display-name lookup. Takes no parameters.
    Whoami,

    /// Send a message as you to a channel or DM. Requires the `chat:write`
    /// user scope. The message will appear to come from your account.
    SendMessage {
        /// Conversation ref (e.g., a channel id or a DM conversation id).
        conversation: String,
        /// Message text (supports Slack mrkdwn formatting). Never use this
        /// operation for a run's own final reply when outbound delivery is
        /// configured. To notify someone else, mention them as `<@U…>` with
        /// their real user id — a plain `@name` does not notify. Never derive
        /// a user id from a conversation id. For a known DM conversation ID,
        /// call `slack.get_conversation_info` and use its `counterpart`;
        /// use `slack.list_conversations` only when the ID is unknown.
        text: String,
        /// Opaque thread anchor (a message's ts) to post into that
        /// thread/topic container. Distinct from `reply_to`: on Slack both
        /// map onto `thread_ts` (see `api::send_message`).
        #[serde(default)]
        thread: Option<String>,
        /// The specific message being quoted or replied to (pre-merge
        /// amendment W4) — distinct from `thread`. On Slack both map onto
        /// `thread_ts`; when both are supplied and disagree, `thread` wins.
        #[serde(default)]
        reply_to: Option<ReplyToRef>,
    },
}

/// The canonical `reply_to` input shape (a `message_ref`): the specific
/// message being quoted or replied to. Distinct from `thread`, the
/// thread/topic container to post into — see `SlackUserAction::SendMessage`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReplyToRef {
    pub conversation: String,
    pub message_id: String,
}

/// Provider-issued evidence for a sent/edited message; identifies the
/// message for follow-up edit/delete/reaction operations.
#[derive(Debug, Serialize)]
pub struct MessageRef {
    pub conversation: String,
    pub message_id: String,
}

/// The author of a message, per the standard messaging framework's shared
/// `author` shape.
#[derive(Debug, Serialize)]
pub struct Author {
    pub user_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Opaque thread anchor plus reply count, per the standard messaging
/// framework's shared `message.thread` shape.
#[derive(Debug, Serialize)]
pub struct ThreadRef {
    pub thread: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_count: Option<u64>,
}

/// One message, per the standard messaging framework's shared `message`
/// shape (`crates/ironclaw_host_api/schemas/messaging/get_conversation_history.output.v1.json`
/// and siblings). Used for history, thread replies, and search matches — the
/// canonical schema is identical across all three.
#[derive(Debug, Serialize)]
pub struct Message {
    pub message_ref: MessageRef,
    pub author: Author,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// `true` when the CONNECTED account authored this message. Always a
    /// concrete bool (never absent) per the canonical schema's `required`
    /// list — defaults to `false` when the connected identity or the
    /// author is unknown, never fabricated as `true`.
    pub is_self: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<ThreadRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited: Option<bool>,
}

/// Result from search_messages.
#[derive(Debug, Serialize)]
pub struct SearchMessagesResult {
    pub matches: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// A conversation, per the standard messaging framework's shared
/// `conversation_info` shape. Used both for `list_conversations` items and
/// (top-level) for `get_conversation_info`'s output.
#[derive(Debug, Serialize)]
pub struct ConversationInfo {
    pub conversation: String,
    pub kind: ConversationKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Whether the connected account is a member of this channel. Slack
    /// lists channels you can SEE, not only ones you're in — this marks the
    /// difference. Absent for DMs (no membership axis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_member: Option<bool>,
    /// For a DM, the other participant — the authoritative mention target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterpart: Option<Author>,
}

/// Result from list_conversations.
#[derive(Debug, Serialize)]
pub struct ListConversationsResult {
    pub conversations: Vec<ConversationInfo>,
    /// Cursor for the next page (pass as `cursor`). Absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result from get_conversation_history (also the shape of get_thread_replies).
#[derive(Debug, Serialize)]
pub struct ConversationHistoryResult {
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result from get_user_info, per the standard messaging framework's
/// top-level `get_user_info` output shape (no `user` wrapper).
#[derive(Debug, Serialize)]
pub struct GetUserInfoResult {
    pub user_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub real_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_emoji: Option<String>,
    /// IANA timezone (e.g. "America/New_York"). Absent when Slack omits it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// Job title from the profile. Absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub is_bot: bool,
    /// Vendor extras with no canonical field (schema's optional `vendor:
    /// object` passthrough). Absent entirely when there is nothing to carry
    /// — never an empty object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<GetUserInfoVendor>,
}

/// `get_user_info`'s `vendor` passthrough payload: today, only Slack's
/// status auto-clear timestamp, which has no canonical field
/// (`status_text` does not encode it, and it is the only machine-readable
/// way to know when a status expires).
#[derive(Debug, Serialize)]
pub struct GetUserInfoVendor {
    pub status_expiration: i64,
}

/// Result from send_message.
#[derive(Debug, Serialize)]
pub struct SendMessageResult {
    pub message_ref: MessageRef,
    /// Echoes the input `thread` when supplied, so a silent drop is
    /// checkable (pre-merge amendment W3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// Echoes the input `reply_to` when supplied, so a silent drop is
    /// checkable (pre-merge amendment W3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageRef>,
}

/// Result from whoami: the CONNECTED account's identity.
#[derive(Debug, Serialize)]
pub struct WhoamiResult {
    pub user_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}
