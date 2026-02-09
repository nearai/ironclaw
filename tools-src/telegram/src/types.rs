//! Types for the Telegram user-mode tool (MTProto direct).

use serde::{Deserialize, Serialize};

/// Input parameters for the Telegram tool.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TelegramAction {
    /// Start login: generate auth key + send verification code.
    Login {
        /// Phone number in international format (e.g., "+1234567890").
        phone_number: String,
    },

    /// Submit the verification code received after login.
    SubmitAuthCode {
        /// The verification code received via SMS or Telegram.
        code: String,
    },

    /// Submit 2FA password if the account has two-factor auth enabled.
    Submit2faPassword {
        /// The two-factor authentication password.
        password: String,
    },

    /// Get the authenticated user's profile info.
    GetMe,

    /// Get the user's contact list.
    GetContacts,

    /// List the user's recent chats/conversations.
    GetChats {
        /// Maximum number of chats to return (default: 20).
        #[serde(default = "default_chat_limit")]
        limit: i32,
    },

    /// Read message history from a chat. Does NOT mark messages as read.
    GetMessages {
        /// Chat ID (numeric, negative for groups/channels).
        chat_id: i64,
        /// Maximum number of messages to return (default: 20).
        #[serde(default = "default_message_limit")]
        limit: i32,
        /// Return messages starting from this message ID (for pagination).
        #[serde(default)]
        from_message_id: Option<i32>,
    },

    /// Send a text message to a chat.
    SendMessage {
        /// Chat ID to send the message to.
        chat_id: i64,
        /// Message text.
        text: String,
    },

    /// Forward messages from one chat to another.
    ForwardMessage {
        /// Source chat ID.
        from_chat_id: i64,
        /// Destination chat ID.
        to_chat_id: i64,
        /// Message IDs to forward.
        message_ids: Vec<i32>,
    },

    /// Delete messages.
    DeleteMessage {
        /// Message IDs to delete.
        message_ids: Vec<i32>,
        /// Also delete for other participants (default: false).
        #[serde(default)]
        revoke: bool,
    },

    /// Search for messages across chats or within a specific chat.
    SearchMessages {
        /// Query string to search for.
        query: String,
        /// Chat ID to search within (omit for global search).
        #[serde(default)]
        chat_id: Option<i64>,
        /// Maximum number of results (default: 20).
        #[serde(default = "default_message_limit")]
        limit: i32,
    },

    /// Poll for new incoming updates.
    GetUpdates,
}

fn default_chat_limit() -> i32 {
    20
}

fn default_message_limit() -> i32 {
    20
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// Result from the login action (code_sent phase).
#[derive(Debug, Serialize)]
pub struct LoginResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_code_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Result from auth code / 2FA / signIn.
#[derive(Debug, Serialize)]
pub struct AuthResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// User profile info.
#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub first_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
}

/// Chat information.
#[derive(Debug, Serialize)]
pub struct ChatInfo {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// A message in a chat.
#[derive(Debug, Serialize)]
pub struct MessageInfo {
    pub message_id: i32,
    pub date: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_user_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<i64>,
}

/// Result from sending a message.
#[derive(Debug, Serialize)]
pub struct SendResult {
    pub message_id: i32,
    pub date: i32,
}

/// Result from forwarding messages.
#[derive(Debug, Serialize)]
pub struct ForwardResult {
    pub ok: bool,
}

/// Result from deleting messages.
#[derive(Debug, Serialize)]
pub struct DeleteResult {
    pub ok: bool,
}

/// An update from getDifference.
#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub update_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<MessageInfo>,
}
