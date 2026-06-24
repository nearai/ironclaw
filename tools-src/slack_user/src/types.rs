//! Types for Slack user-token API requests and responses.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Invocation context the host passes alongside params. The host selects the
/// operation via the capability id (e.g. `slack_user.search_messages`); the
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
        /// Search query. Supports Slack search operators such as
        /// `from:@me`, `in:#channel`, `after:2024-01-01`, `has:link`.
        query: String,
        /// Maximum number of matches to return (default: 20, max: 100).
        #[serde(default = "default_search_count")]
        count: u32,
        /// Sort by `score` (relevance, default) or `timestamp` (recency).
        #[serde(default)]
        sort: Option<String>,
    },

    /// List conversations you belong to: channels, private channels,
    /// DMs, and group DMs. Use this to discover DM conversation IDs.
    ListConversations {
        /// Comma-separated conversation types to include. Defaults to
        /// `public_channel,private_channel,im,mpim` (everything you're in).
        #[serde(default = "default_conversation_types")]
        types: String,
        /// Maximum number of conversations to return (default: 200).
        #[serde(default = "default_list_limit")]
        limit: u32,
    },

    /// Read message history from any conversation you can see — a channel,
    /// a DM, or a group DM — identified by its conversation ID.
    GetConversationHistory {
        /// Conversation ID (e.g. `C123...` for a channel, `D123...` for a DM).
        channel: String,
        /// Maximum number of messages to return (default: 50).
        #[serde(default = "default_history_limit")]
        limit: u32,
        /// Only return messages before this timestamp (pagination cursor).
        #[serde(default)]
        latest: Option<String>,
        /// Only return messages after this timestamp.
        #[serde(default)]
        oldest: Option<String>,
    },

    /// Get information about a user (name, real name, email).
    GetUserInfo {
        /// User ID (e.g., "U1234567890").
        user_id: String,
    },

    /// Send a message as you to a channel or DM. Requires the `chat:write`
    /// user scope. The message will appear to come from your account.
    SendMessage {
        /// Channel ID or name (e.g., "#general" or "C1234567890"), or a
        /// DM conversation ID.
        channel: String,
        /// Message text (supports Slack mrkdwn formatting).
        text: String,
        /// Optional thread timestamp to reply in a thread.
        #[serde(default)]
        thread_ts: Option<String>,
    },
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
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
    /// For DMs (`im`), the user ID on the other side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// Result from list_conversations.
#[derive(Debug, Serialize)]
pub struct ListConversationsResult {
    pub ok: bool,
    pub conversations: Vec<Conversation>,
}

/// A message from conversation history.
#[derive(Debug, Serialize)]
pub struct HistoryMessage {
    pub ts: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
}

/// Result from get_conversation_history.
#[derive(Debug, Serialize)]
pub struct ConversationHistoryResult {
    pub ok: bool,
    pub messages: Vec<HistoryMessage>,
    pub has_more: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub is_bot: bool,
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
