//! Types for Slack user-token API requests and responses.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Invocation context the host passes alongside params. The host selects the
/// operation via the capability id (e.g. `slack.search_messages`); the
/// action is NOT carried in the params object.
#[derive(Debug, Deserialize)]
pub(crate) struct ToolContext {
    pub(crate) capability_id: String,
}

/// Input parameters for the Slack personal (user-token) tool.
///
/// `JsonSchema` is derived so the advertised tool schema mirrors the
/// serde-enforced contract: each variant becomes a `oneOf` entry with
/// its own `required` array.
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
        #[serde(default = "default_search_count")]
        count: u32,
        /// Sort by relevance (default) or recency.
        #[serde(default)]
        sort: Option<SearchSort>,
        /// Result page to fetch (1-based), passed through to Slack paging.
        #[serde(default)]
        page: Option<u32>,
    },

    /// List conversations visible to you: channels, private channels,
    /// DMs, and group DMs (`is_member` marks which channels you belong
    /// to). Use this to discover DM conversation IDs.
    ListConversations {
        /// Comma-separated conversation types to include. Defaults to
        /// `public_channel,private_channel,im,mpim`.
        #[serde(default = "default_conversation_types")]
        types: String,
        /// Maximum number of conversations to return (default: 200).
        #[serde(default = "default_list_limit")]
        limit: u32,
        /// Pagination cursor from a previous call's `next_cursor`.
        #[serde(default)]
        cursor: Option<String>,
    },

    /// Retrieve one exact conversation by its known conversation ID. For a
    /// DM, the returned `user` is the authoritative counterpart ID.
    GetConversationInfo {
        /// Exact conversation ID (e.g. `C123...` for a channel, `D123...`
        /// for a DM).
        channel: String,
    },

    /// Read message history from any conversation you can see — a channel,
    /// a DM, or a group DM — identified by its conversation ID.
    GetConversationHistory {
        /// Conversation ID (e.g. `C123...` for a channel, `D123...` for a DM).
        channel: String,
        /// Maximum number of messages to return (default: 50, max: 999 —
        /// Slack rejects 1000; out-of-range values are clamped).
        #[serde(default = "default_history_limit")]
        limit: u32,
        /// Only return messages before this timestamp (pagination cursor).
        #[serde(default)]
        latest: Option<String>,
        /// Only return messages after this timestamp.
        #[serde(default)]
        oldest: Option<String>,
    },

    /// Read the replies of one thread (`conversations.replies`). Thread
    /// replies are NOT part of conversation history — the parent's
    /// `reply_count`/`thread_ts` point here.
    GetThreadReplies {
        /// Conversation ID the thread lives in (e.g. `C123...`).
        channel: String,
        /// The thread parent's `ts` (also exposed as `thread_ts` on replies).
        thread_ts: String,
        /// Maximum number of messages to return (default: 50, max: 999).
        #[serde(default = "default_history_limit")]
        limit: u32,
    },

    /// Get information about a user (name, real name).
    GetUserInfo {
        /// User ID (e.g., "U1234567890").
        user_id: String,
    },

    /// Resolve who the connected Slack account is (`auth.test`), with a
    /// best-effort display-name lookup. Takes no parameters.
    Whoami,

    /// Send a message as you to a channel or DM. Requires the `chat:write`
    /// user scope. The message will appear to come from your account.
    SendMessage {
        /// Channel ID or name (e.g., "#general" or "C1234567890"), or a
        /// DM conversation ID.
        channel: String,
        /// Message text (supports Slack mrkdwn formatting). Never use this
        /// operation for a run's own final reply when outbound delivery is
        /// configured. To notify someone else, mention them as `<@U…>` with
        /// their real user id — a plain `@name` does not notify. Never derive
        /// a user id from a conversation id. For a known DM conversation ID,
        /// call `slack.get_conversation_info` and use its `conversation.user`;
        /// use `slack.list_conversations` only when the ID is unknown.
        text: String,
        /// Optional thread timestamp to reply in a thread.
        #[serde(default)]
        thread_ts: Option<String>,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    Score,
    Timestamp,
}

impl SearchSort {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Score => "score",
            Self::Timestamp => "timestamp",
        }
    }
}

fn default_search_count() -> u32 {
    20
}

fn default_conversation_types() -> String {
    "public_channel,private_channel,im,mpim".to_string()
}

fn default_list_limit() -> u32 {
    200
}

fn default_history_limit() -> u32 {
    50
}

/// A single message match from search.messages.
#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub ts: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Human-readable name for `user`, resolved via `users.info`
    /// (best-effort: absent when the lookup fails). Use this in user-facing
    /// output instead of the raw user ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    /// Present on threaded matches: the thread parent's ts — follow up with
    /// `get_thread_replies` to read the thread.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permalink: Option<String>,
}

/// Result from search_messages.
#[derive(Debug, Serialize)]
pub struct SearchMessagesResult {
    pub ok: bool,
    pub total: u64,
    pub matches: Vec<SearchMatch>,
}

/// A conversation (channel, private channel, DM, or group DM).
#[derive(Debug, Serialize)]
pub struct Conversation {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub is_channel: bool,
    pub is_private: bool,
    pub is_im: bool,
    pub is_mpim: bool,
    /// Whether the connected account is a member of this channel. Slack lists
    /// channels you can SEE, not only ones you're in — this marks the
    /// difference. Absent for DMs (no membership axis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_member: Option<bool>,
    /// For DMs (`im`), the authoritative user ID on the other side. Use this
    /// for follow-up calls or mention encoding; never derive it from `id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Human-readable name for `user`, resolved via `users.info` (best-effort:
    /// absent when the lookup fails). DMs have no `name`, so without this the
    /// only handle on the conversation is the raw user ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_display_name: Option<String>,
}

/// Result from list_conversations.
#[derive(Debug, Serialize)]
pub struct ListConversationsResult {
    pub ok: bool,
    pub conversations: Vec<Conversation>,
    /// Cursor for the next page (pass as `cursor`). Absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result from get_conversation_info.
#[derive(Debug, Serialize)]
pub struct GetConversationInfoResult {
    pub ok: bool,
    pub conversation: Conversation,
}

/// A message from conversation history.
#[derive(Debug, Serialize)]
pub struct HistoryMessage {
    pub ts: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Human-readable name for `user`, resolved via `users.info` (one lookup
    /// per distinct author, best-effort: absent when the lookup fails). Use
    /// this in user-facing output instead of the raw user ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_display_name: Option<String>,
    /// `Some(true)` when this message was authored by the CONNECTED account
    /// (the requesting user), `Some(false)` for other authors. Absent when the
    /// connected identity or the author is unknown — never fabricated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_current_user: Option<bool>,
    /// Number of thread replies under this message (thread parents only).
    /// History does NOT include the replies themselves — fetch them with
    /// `get_thread_replies`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_count: Option<u64>,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
}

/// Result from get_conversation_history (also the shape of get_thread_replies).
#[derive(Debug, Serialize)]
pub struct ConversationHistoryResult {
    pub ok: bool,
    pub messages: Vec<HistoryMessage>,
    pub has_more: bool,
    /// User ID of the CONNECTED account (from `auth.test`), so callers can
    /// attribute `is_current_user` messages to the requester. Best-effort:
    /// absent when `auth.test` fails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_user_id: Option<String>,
}

/// User information.
#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub real_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub is_bot: bool,
    /// IANA timezone (e.g. "America/New_York"). Absent when Slack omits it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tz_label: Option<String>,
    /// Job title from the profile. Absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Slack status text (e.g. "On vacation until July 20"). Absent when the
    /// user has no status set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_emoji: Option<String>,
    /// Unix timestamp when the status expires. Absent when there is no status
    /// or the status does not expire (Slack reports 0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_expiration: Option<i64>,
}

/// Result from get_user_info.
#[derive(Debug, Serialize)]
pub struct GetUserInfoResult {
    pub ok: bool,
    pub user: UserInfo,
}

/// Result from send_message.
#[derive(Debug, Serialize)]
pub struct SendMessageResult {
    pub ok: bool,
    pub channel: String,
    pub ts: String,
}

/// Result from whoami: the CONNECTED account's identity.
#[derive(Debug, Serialize)]
pub struct WhoamiResult {
    pub ok: bool,
    /// Raw Slack user ID of the connected account (`auth.test` user_id).
    pub user_id: String,
    /// Human-readable name for the connected account, resolved via
    /// `users.info` (best-effort: absent when the lookup fails).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
}
