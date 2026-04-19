//! Types for Slack API requests and responses.

use serde::{Deserialize, Serialize};

/// Input parameters for the Slack tool.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SlackAction {
    /// Send a message to a channel.
    SendMessage {
        /// Channel ID or name (e.g., "#general" or "C1234567890").
        channel: String,
        /// Message text (supports Slack mrkdwn formatting).
        text: String,
        /// Optional thread timestamp to reply in a thread.
        #[serde(default)]
        thread_ts: Option<String>,
    },

    /// List channels the bot has access to.
    ListChannels {
        /// Maximum number of channels to return (default: 100).
        #[serde(default = "default_limit")]
        limit: u32,
    },

    /// Get message history from a channel.
    GetChannelHistory {
        /// Channel ID (e.g., "C1234567890").
        channel: String,
        /// Maximum number of messages to return (default: 20).
        #[serde(default = "default_history_limit")]
        limit: u32,
    },

    /// Add a reaction (emoji) to a message.
    PostReaction {
        /// Channel ID containing the message.
        channel: String,
        /// Timestamp of the message to react to.
        timestamp: String,
        /// Emoji name without colons (e.g., "thumbsup").
        emoji: String,
    },

    /// Get information about a user.
    GetUserInfo {
        /// User ID (e.g., "U1234567890").
        user_id: String,
    },
}

fn default_limit() -> u32 {
    100
}

fn default_history_limit() -> u32 {
    20
}

/// Result from send_message.
#[derive(Debug, Serialize)]
pub struct SendMessageResult {
    pub ok: bool,
    pub channel: String,
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<MessageInfo>,
}

/// Basic message info.
#[derive(Debug, Serialize)]
pub struct MessageInfo {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub ts: String,
}

/// A Slack channel.
#[derive(Debug, Serialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub is_private: bool,
    pub is_member: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// Result from list_channels.
#[derive(Debug, Serialize)]
pub struct ListChannelsResult {
    pub ok: bool,
    pub channels: Vec<Channel>,
}

/// Result from get_channel_history.
#[derive(Debug, Serialize)]
pub struct ChannelHistoryResult {
    pub ok: bool,
    pub messages: Vec<HistoryMessage>,
}

/// A message from channel history.
#[derive(Debug, Serialize)]
pub struct HistoryMessage {
    pub ts: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// Result from post_reaction.
#[derive(Debug, Serialize)]
pub struct PostReactionResult {
    pub ok: bool,
}

/// User information.
#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: String,
    pub name: String,
    pub real_name: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub is_bot: bool,
}

/// Result from get_user_info.
#[derive(Debug, Serialize)]
pub struct GetUserInfoResult {
    pub ok: bool,
    pub user: UserInfo,
}
