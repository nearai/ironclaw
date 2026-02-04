// Telegram API types have fields reserved for future use (entities, reply threading, etc.)
#![allow(dead_code)]

//! Telegram Bot API channel for NEAR Agent.
//!
//! This WASM component implements the channel interface for handling Telegram
//! webhooks and sending messages back via the Bot API.
//!
//! # Features
//!
//! - Webhook-based message receiving
//! - Private chat (DM) support
//! - Group chat support with @mention triggering
//! - Reply threading support
//! - User name extraction
//!
//! # Security
//!
//! - Bot token is injected by host during HTTP requests
//! - WASM never sees raw credentials
//! - Optional webhook secret validation by host

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig,
};
use near::agent::channel_host::{self, EmittedMessage};

// ============================================================================
// Telegram API Types
// ============================================================================

/// Telegram Update object (webhook payload).
/// https://core.telegram.org/bots/api#update
#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    /// Unique update identifier.
    update_id: i64,

    /// New incoming message.
    message: Option<TelegramMessage>,

    /// Edited message.
    edited_message: Option<TelegramMessage>,

    /// Channel post (we ignore these for now).
    channel_post: Option<TelegramMessage>,
}

/// Telegram Message object.
/// https://core.telegram.org/bots/api#message
#[derive(Debug, Deserialize)]
struct TelegramMessage {
    /// Unique message identifier.
    message_id: i64,

    /// Sender (empty for channel posts).
    from: Option<TelegramUser>,

    /// Chat the message belongs to.
    chat: TelegramChat,

    /// Message text.
    text: Option<String>,

    /// Original message if this is a reply.
    reply_to_message: Option<Box<TelegramMessage>>,

    /// Bot command entities (for /commands).
    entities: Option<Vec<MessageEntity>>,
}

/// Telegram User object.
/// https://core.telegram.org/bots/api#user
#[derive(Debug, Deserialize)]
struct TelegramUser {
    /// Unique user identifier.
    id: i64,

    /// True if this is a bot.
    is_bot: bool,

    /// User's first name.
    first_name: String,

    /// User's last name.
    last_name: Option<String>,

    /// Username (without @).
    username: Option<String>,
}

/// Telegram Chat object.
/// https://core.telegram.org/bots/api#chat
#[derive(Debug, Deserialize)]
struct TelegramChat {
    /// Unique chat identifier.
    id: i64,

    /// Type of chat: private, group, supergroup, or channel.
    #[serde(rename = "type")]
    chat_type: String,

    /// Title for groups/channels.
    title: Option<String>,

    /// Username for private chats.
    username: Option<String>,
}

/// Message entity (for parsing @mentions, commands, etc.).
/// https://core.telegram.org/bots/api#messageentity
#[derive(Debug, Deserialize)]
struct MessageEntity {
    /// Type: mention, bot_command, etc.
    #[serde(rename = "type")]
    entity_type: String,

    /// Offset in UTF-16 code units.
    offset: i64,

    /// Length in UTF-16 code units.
    length: i64,

    /// For "mention" type, the mentioned user.
    user: Option<TelegramUser>,
}

/// Telegram API response wrapper.
#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    /// True if the request was successful.
    ok: bool,

    /// Error description if not ok.
    description: Option<String>,

    /// Result on success.
    result: Option<T>,
}

/// Response from sendMessage.
#[derive(Debug, Deserialize)]
struct SentMessage {
    message_id: i64,
}

// ============================================================================
// Channel Metadata
// ============================================================================

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
struct TelegramMessageMetadata {
    /// Chat ID where the message was received.
    chat_id: i64,

    /// Original message ID (for reply_to_message_id).
    message_id: i64,

    /// User ID who sent the message.
    user_id: i64,

    /// Whether this is a private (DM) chat.
    is_private: bool,
}

/// Channel configuration from capabilities file.
#[derive(Debug, Deserialize)]
struct TelegramConfig {
    /// Bot username (without @) for mention detection in groups.
    #[serde(default)]
    bot_username: Option<String>,

    /// Whether to respond to all group messages (not just mentions).
    #[serde(default)]
    respond_to_all_group_messages: bool,

    /// Whether to use polling instead of webhooks.
    #[serde(default)]
    polling_enabled: bool,

    /// Polling interval in milliseconds (if polling enabled).
    #[serde(default = "default_poll_interval")]
    poll_interval_ms: u32,
}

fn default_poll_interval() -> u32 {
    30000 // 30 seconds (minimum allowed)
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct TelegramChannel;

impl Guest for TelegramChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: TelegramConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Telegram channel starting");

        if let Some(ref username) = config.bot_username {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Bot username: @{}", username),
            );
        }

        // Configure polling if enabled
        let poll = if config.polling_enabled {
            Some(PollConfig {
                interval_ms: config.poll_interval_ms.max(30000), // Enforce minimum
                enabled: true,
            })
        } else {
            None
        };

        Ok(ChannelConfig {
            display_name: "Telegram".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/telegram".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false, // Telegram doesn't use signing secrets by default
            }],
            poll,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Parse the request body as UTF-8
        let body_str = match std::str::from_utf8(&req.body) {
            Ok(s) => s,
            Err(_) => {
                return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
            }
        };

        // Parse as Telegram Update
        let update: TelegramUpdate = match serde_json::from_str(body_str) {
            Ok(u) => u,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse Telegram update: {}", e),
                );
                // Still return 200 to prevent Telegram from retrying
                return json_response(200, serde_json::json!({"ok": true}));
            }
        };

        // Handle the update
        handle_update(update);

        // Always respond 200 quickly (Telegram expects fast responses)
        json_response(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        // Polling mode: call getUpdates API
        // For now, we focus on webhook mode. Polling can be added later.
        channel_host::log(
            channel_host::LogLevel::Debug,
            "Polling tick (not implemented yet)",
        );
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        // Parse metadata to get chat info
        let metadata: TelegramMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Build sendMessage payload
        let mut payload = serde_json::json!({
            "chat_id": metadata.chat_id,
            "text": response.content,
            "parse_mode": "Markdown",
        });

        // Reply to the original message for context
        payload["reply_to_message_id"] = serde_json::Value::Number(metadata.message_id.into());

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| format!("Failed to serialize payload: {}", e))?;

        // Make HTTP request to Telegram API
        // The bot token is injected into the URL by the host
        let headers = serde_json::json!({
            "Content-Type": "application/json"
        });

        let result = channel_host::http_request(
            "POST",
            "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendMessage",
            &headers.to_string(),
            Some(&payload_bytes),
        );

        match result {
            Ok(http_response) => {
                if http_response.status != 200 {
                    let body_str = String::from_utf8_lossy(&http_response.body);
                    return Err(format!(
                        "Telegram API returned status {}: {}",
                        http_response.status, body_str
                    ));
                }

                // Parse Telegram response
                let api_response: TelegramApiResponse<SentMessage> =
                    serde_json::from_slice(&http_response.body).map_err(|e| {
                        format!("Failed to parse Telegram response: {}", e)
                    })?;

                if !api_response.ok {
                    return Err(format!(
                        "Telegram API error: {}",
                        api_response.description.unwrap_or_else(|| "unknown".to_string())
                    ));
                }

                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Sent message to chat {}: message_id={}",
                        metadata.chat_id,
                        api_response.result.map(|r| r.message_id).unwrap_or(0)
                    ),
                );

                Ok(())
            }
            Err(e) => Err(format!("HTTP request failed: {}", e)),
        }
    }

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Telegram channel shutting down");
    }
}

// ============================================================================
// Update Handling
// ============================================================================

/// Process a Telegram update and emit messages if applicable.
fn handle_update(update: TelegramUpdate) {
    // Handle regular messages
    if let Some(message) = update.message {
        handle_message(message);
    }

    // Optionally handle edited messages the same way
    if let Some(message) = update.edited_message {
        handle_message(message);
    }
}

/// Process a single message.
fn handle_message(message: TelegramMessage) {
    // Skip messages without text
    let text = match message.text {
        Some(t) if !t.is_empty() => t,
        _ => return,
    };

    // Skip messages without a sender (channel posts)
    let from = match message.from {
        Some(f) => f,
        None => return,
    };

    // Skip bot messages to avoid loops
    if from.is_bot {
        return;
    }

    let is_private = message.chat.chat_type == "private";

    // For group chats, check if the bot was mentioned
    // TODO: Read bot_username from config and check mentions
    // For now, process all messages in private chats and groups
    if !is_private {
        // In groups, only respond if there's a bot mention or command
        // This is a simplified check - proper implementation would use entities
        let has_command = text.starts_with('/');
        let has_mention = text.contains('@');

        if !has_command && !has_mention {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Ignoring group message without mention: {}", text),
            );
            return;
        }
    }

    // Build user display name
    let user_name = if let Some(ref last) = from.last_name {
        format!("{} {}", from.first_name, last)
    } else {
        from.first_name.clone()
    };

    // Build metadata for response routing
    let metadata = TelegramMessageMetadata {
        chat_id: message.chat.id,
        message_id: message.message_id,
        user_id: from.id,
        is_private,
    };

    let metadata_json =
        serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Clean the message text (strip bot mentions and commands)
    let cleaned_text = clean_message_text(&text);

    if cleaned_text.is_empty() {
        return;
    }

    // Emit the message to the agent
    channel_host::emit_message(&EmittedMessage {
        user_id: from.id.to_string(),
        user_name: Some(user_name),
        content: cleaned_text,
        thread_id: None, // Telegram doesn't have threads in the same way
        metadata_json,
    });

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "Emitted message from user {} in chat {}",
            from.id, message.chat.id
        ),
    );
}

/// Clean message text by removing bot commands and @mentions at the start.
fn clean_message_text(text: &str) -> String {
    let mut result = text.trim().to_string();

    // Remove leading /command
    if result.starts_with('/') {
        if let Some(space_idx) = result.find(' ') {
            result = result[space_idx..].trim_start().to_string();
        } else {
            // Just a command with no text
            return String::new();
        }
    }

    // Remove leading @mention
    if result.starts_with('@') {
        if let Some(space_idx) = result.find(' ') {
            result = result[space_idx..].trim_start().to_string();
        } else {
            // Just a mention with no text
            return String::new();
        }
    }

    result
}

// ============================================================================
// Utilities
// ============================================================================

/// Create a JSON HTTP response.
fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let headers = serde_json::json!({"Content-Type": "application/json"});

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

// Export the component
export!(TelegramChannel);

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_message_text() {
        assert_eq!(clean_message_text("/start hello"), "hello");
        assert_eq!(clean_message_text("@bot hello world"), "hello world");
        assert_eq!(clean_message_text("/start"), "");
        assert_eq!(clean_message_text("@botname"), "");
        assert_eq!(clean_message_text("just text"), "just text");
        assert_eq!(clean_message_text("  spaced  "), "spaced");
    }

    #[test]
    fn test_parse_update() {
        let json = r#"{
            "update_id": 123,
            "message": {
                "message_id": 456,
                "from": {
                    "id": 789,
                    "is_bot": false,
                    "first_name": "John",
                    "last_name": "Doe"
                },
                "chat": {
                    "id": 789,
                    "type": "private"
                },
                "text": "Hello bot"
            }
        }"#;

        let update: TelegramUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(update.update_id, 123);

        let message = update.message.unwrap();
        assert_eq!(message.message_id, 456);
        assert_eq!(message.text.unwrap(), "Hello bot");

        let from = message.from.unwrap();
        assert_eq!(from.id, 789);
        assert_eq!(from.first_name, "John");
    }
}
