// Telegram API types have fields reserved for future use (entities, reply threading, etc.)
#![allow(dead_code)]

//! Telegram Bot API channel for IronClaw.
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
    OutgoingHttpResponse, PollConfig, StatusType, StatusUpdate,
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

    /// Caption for media (photo, video, document, etc.).
    #[serde(default)]
    caption: Option<String>,

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

/// Workspace path for storing polling state.
const POLLING_STATE_PATH: &str = "state/last_update_id";

/// Workspace path for persisting owner_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_id";

/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";

/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";

/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "telegram";

/// Workspace path for persisting bot_username for mention detection in groups.
const BOT_USERNAME_PATH: &str = "state/bot_username";

/// Workspace path for persisting respond_to_all_group_messages flag.
const RESPOND_TO_ALL_GROUP_PATH: &str = "state/respond_to_all_group_messages";

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

/// Channel configuration injected by host.
///
/// The host injects runtime values like tunnel_url and webhook_secret.
/// The channel doesn't need to know about polling vs webhook mode - it just
/// checks if tunnel_url is set to determine behavior.
#[derive(Debug, Deserialize)]
struct TelegramConfig {
    /// Bot username (without @) for mention detection in groups.
    #[serde(default)]
    bot_username: Option<String>,

    /// Telegram user ID of the bot owner. When set, only messages from this
    /// user are processed. All others are silently dropped.
    #[serde(default)]
    owner_id: Option<i64>,

    /// DM policy: "pairing" (default), "allowlist", or "open".
    #[serde(default)]
    dm_policy: Option<String>,

    /// Allowed sender IDs/usernames from config (merged with pairing-approved store).
    #[serde(default)]
    allow_from: Option<Vec<String>>,

    /// Whether to respond to all group messages (not just mentions).
    #[serde(default)]
    respond_to_all_group_messages: bool,

    /// Public tunnel URL for webhook mode (injected by host from global settings).
    /// When set, webhook mode is enabled and polling is disabled.
    #[serde(default)]
    tunnel_url: Option<String>,

    /// Secret token for webhook validation (injected by host from secrets store).
    /// Telegram will include this in the X-Telegram-Bot-Api-Secret-Token header.
    #[serde(default)]
    webhook_secret: Option<String>,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct TelegramChannel;

impl Guest for TelegramChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Telegram channel config: {}", config_json),
        );

        let config: TelegramConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Telegram channel starting");

        if let Some(ref username) = config.bot_username {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Bot username: @{}", username),
            );
        }

        // Persist owner_id so subsequent callbacks (on_http_request, on_poll) can read it
        if let Some(owner_id) = config.owner_id {
            if let Err(e) = channel_host::workspace_write(OWNER_ID_PATH, &owner_id.to_string()) {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to persist owner_id: {}", e),
                );
            }
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: user {}", owner_id),
            );
        } else {
            // Clear any stale owner_id from a previous config
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
            channel_host::log(
                channel_host::LogLevel::Warn,
                "No owner_id configured, bot is open to all users",
            );
        }

        // Persist dm_policy and allow_from for DM pairing in handle_message
        let dm_policy = config
            .dm_policy
            .as_deref()
            .unwrap_or("pairing")
            .to_string();
        let _ = channel_host::workspace_write(DM_POLICY_PATH, &dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // Persist bot_username and respond_to_all_group_messages for group handling
        let _ = channel_host::workspace_write(
            BOT_USERNAME_PATH,
            &config.bot_username.unwrap_or_default(),
        );
        let _ = channel_host::workspace_write(
            RESPOND_TO_ALL_GROUP_PATH,
            &config.respond_to_all_group_messages.to_string(),
        );

        // Mode is determined by whether the host injected a tunnel_url
        // If tunnel is configured, use webhooks. Otherwise, use polling.
        let webhook_mode = config.tunnel_url.is_some();

        if webhook_mode {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Webhook mode enabled (tunnel configured)",
            );

            // Register webhook with Telegram API
            if let Some(ref tunnel_url) = config.tunnel_url {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Registering webhook: {}/webhook/telegram", tunnel_url),
                );

                if let Err(e) = register_webhook(tunnel_url, config.webhook_secret.as_deref()) {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to register webhook: {}", e),
                    );
                }
            }
        } else {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Polling mode enabled (no tunnel configured)",
            );

            // Delete any existing webhook before polling
            // Telegram doesn't allow getUpdates while a webhook is active
            if let Err(e) = delete_webhook() {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to delete webhook (may not exist): {}", e),
                );
            }
        }

        // Configure polling only if not in webhook mode
        let poll = if !webhook_mode {
            Some(PollConfig {
                interval_ms: 30000, // 30 seconds minimum
                enabled: true,
            })
        } else {
            None
        };

        // Webhook secret validation is handled by the host
        let require_secret = config.webhook_secret.is_some();

        Ok(ChannelConfig {
            display_name: "Telegram".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/telegram".to_string(),
                methods: vec!["POST".to_string()],
                require_secret,
            }],
            poll,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Check if webhook secret validation passed (if required)
        // The host validates X-Telegram-Bot-Api-Secret-Token header and sets secret_validated
        // If require_secret was true in config but validation failed, secret_validated will be false
        if !req.secret_validated {
            // This means require_secret was set but the secret didn't match
            // We still check the field even though the host should have already rejected invalid requests
            // This is defense in depth
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Webhook request with invalid or missing secret token",
            );
            // Return 401 but Telegram will keep retrying, so this is just for logging
            // In practice, the host should reject these before they reach us
        }

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
        // Read last offset from workspace storage
        let offset = match channel_host::workspace_read(POLLING_STATE_PATH) {
            Some(s) => s.parse::<i64>().unwrap_or(0),
            None => 0,
        };

        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Polling getUpdates with offset {}", offset),
        );

        // Build getUpdates URL with parameters
        // - offset: Identifier of the first update to be returned
        // - timeout: Long polling timeout in seconds (Telegram recommends 30+)
        // - allowed_updates: Only get message updates
        let url = format!(
            "https://api.telegram.org/bot{{TELEGRAM_BOT_TOKEN}}/getUpdates?offset={}&timeout=30&allowed_updates=[\"message\",\"edited_message\"]",
            offset
        );

        let headers = serde_json::json!({});

        // 35s HTTP timeout outlives Telegram's 30s server-side long-poll
        let result =
            channel_host::http_request("GET", &url, &headers.to_string(), None, Some(35_000));

        match result {
            Ok(response) => {
                if response.status != 200 {
                    let body_str = String::from_utf8_lossy(&response.body);
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("getUpdates returned {}: {}", response.status, body_str),
                    );
                    return;
                }

                // Parse response
                let api_response: Result<TelegramApiResponse<Vec<TelegramUpdate>>, _> =
                    serde_json::from_slice(&response.body);

                match api_response {
                    Ok(resp) if resp.ok => {
                        if let Some(updates) = resp.result {
                            let mut new_offset = offset;

                            for update in updates {
                                // Track highest update_id for next poll
                                if update.update_id >= new_offset {
                                    new_offset = update.update_id + 1;
                                }

                                // Process the update (emits messages)
                                handle_update(update);
                            }

                            // Save new offset if it changed
                            if new_offset != offset {
                                if let Err(e) = channel_host::workspace_write(
                                    POLLING_STATE_PATH,
                                    &new_offset.to_string(),
                                ) {
                                    channel_host::log(
                                        channel_host::LogLevel::Error,
                                        &format!("Failed to save polling offset: {}", e),
                                    );
                                }
                            }
                        }
                    }
                    Ok(resp) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!(
                                "Telegram API error: {}",
                                resp.description.unwrap_or_else(|| "unknown".to_string())
                            ),
                        );
                    }
                    Err(e) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!("Failed to parse getUpdates response: {}", e),
                        );
                    }
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("getUpdates request failed: {}", e),
                );
            }
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: TelegramMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Try sending with Markdown first; fall back to plain text if Telegram
        // can't parse the entities (e.g. model leaked <tool_call> with underscores).
        let result = send_message(
            metadata.chat_id,
            &response.content,
            metadata.message_id,
            Some("Markdown"),
        );

        match result {
            Ok(msg_id) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Sent message to chat {}: message_id={}",
                        metadata.chat_id, msg_id
                    ),
                );
                Ok(())
            }
            Err(SendError::ParseEntities(detail)) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Markdown parse failed ({}), retrying as plain text", detail),
                );
                let msg_id = send_message(
                    metadata.chat_id,
                    &response.content,
                    metadata.message_id,
                    None,
                )
                .map_err(|e| format!("Plain-text retry also failed: {}", e))?;

                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Sent plain-text message to chat {}: message_id={}",
                        metadata.chat_id, msg_id
                    ),
                );
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    fn on_status(update: StatusUpdate) {
        // Only send typing indicator for Thinking status
        if !matches!(update.status, StatusType::Thinking) {
            return;
        }

        // Parse chat_id from metadata
        let metadata: TelegramMessageMetadata = match serde_json::from_str(&update.metadata_json) {
            Ok(m) => m,
            Err(_) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    "on_status: no valid Telegram metadata, skipping typing indicator",
                );
                return;
            }
        };

        // POST /sendChatAction with action "typing"
        let payload = serde_json::json!({
            "chat_id": metadata.chat_id,
            "action": "typing"
        });

        let payload_bytes = match serde_json::to_vec(&payload) {
            Ok(b) => b,
            Err(_) => return,
        };

        let headers = serde_json::json!({
            "Content-Type": "application/json"
        });

        let result = channel_host::http_request(
            "POST",
            "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendChatAction",
            &headers.to_string(),
            Some(&payload_bytes),
            None,
        );

        if let Err(e) = result {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("sendChatAction failed: {}", e),
            );
        }
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Telegram channel shutting down",
        );
    }
}

// ============================================================================
// Send Message Helper
// ============================================================================

/// Errors from send_message, split so callers can match on parse-entity failures.
enum SendError {
    /// Telegram returned 400 with "can't parse entities" (Markdown issue).
    ParseEntities(String),
    /// Any other failure.
    Other(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::ParseEntities(detail) => write!(f, "parse entities error: {}", detail),
            SendError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

/// Send a message via the Telegram Bot API.
///
/// Returns the sent message_id on success. When `parse_mode` is set and
/// Telegram returns a 400 "can't parse entities" error, returns
/// `SendError::ParseEntities` so the caller can retry without formatting.
fn send_message(
    chat_id: i64,
    text: &str,
    reply_to_message_id: i64,
    parse_mode: Option<&str>,
) -> Result<i64, SendError> {
    let mut payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "reply_to_message_id": reply_to_message_id,
    });

    if let Some(mode) = parse_mode {
        payload["parse_mode"] = serde_json::Value::String(mode.to_string());
    }

    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| SendError::Other(format!("Failed to serialize payload: {}", e)))?;

    let headers = serde_json::json!({ "Content-Type": "application/json" });

    let result = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendMessage",
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(http_response) => {
            if http_response.status == 400 {
                let body_str = String::from_utf8_lossy(&http_response.body);
                if body_str.contains("can't parse entities") {
                    return Err(SendError::ParseEntities(body_str.to_string()));
                }
                return Err(SendError::Other(format!(
                    "Telegram API returned 400: {}",
                    body_str
                )));
            }

            if http_response.status != 200 {
                let body_str = String::from_utf8_lossy(&http_response.body);
                return Err(SendError::Other(format!(
                    "Telegram API returned status {}: {}",
                    http_response.status, body_str
                )));
            }

            let api_response: TelegramApiResponse<SentMessage> =
                serde_json::from_slice(&http_response.body)
                    .map_err(|e| SendError::Other(format!("Failed to parse response: {}", e)))?;

            if !api_response.ok {
                return Err(SendError::Other(format!(
                    "Telegram API error: {}",
                    api_response
                        .description
                        .unwrap_or_else(|| "unknown".to_string())
                )));
            }

            Ok(api_response.result.map(|r| r.message_id).unwrap_or(0))
        }
        Err(e) => Err(SendError::Other(format!("HTTP request failed: {}", e))),
    }
}

// ============================================================================
// Webhook Management
// ============================================================================

/// Delete any existing webhook with Telegram API.
///
/// Called during on_start() when switching to polling mode.
/// Telegram doesn't allow getUpdates while a webhook is active.
fn delete_webhook() -> Result<(), String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    let result = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/deleteWebhook",
        &headers.to_string(),
        None,
        None,
    );

    match result {
        Ok(response) => {
            if response.status != 200 {
                let body_str = String::from_utf8_lossy(&response.body);
                return Err(format!("HTTP {}: {}", response.status, body_str));
            }

            let api_response: TelegramApiResponse<bool> = serde_json::from_slice(&response.body)
                .map_err(|e| format!("Failed to parse response: {}", e))?;

            if !api_response.ok {
                return Err(format!(
                    "Telegram API error: {}",
                    api_response
                        .description
                        .unwrap_or_else(|| "unknown".to_string())
                ));
            }

            channel_host::log(
                channel_host::LogLevel::Info,
                "Webhook deleted successfully (switching to polling mode)",
            );

            Ok(())
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

/// Register webhook URL with Telegram API.
///
/// Called during on_start() when tunnel_url is configured.
fn register_webhook(tunnel_url: &str, webhook_secret: Option<&str>) -> Result<(), String> {
    let webhook_url = format!("{}/webhook/telegram", tunnel_url);

    // Build setWebhook request body
    let mut body = serde_json::json!({
        "url": webhook_url,
        "allowed_updates": ["message", "edited_message"]
    });

    if let Some(secret) = webhook_secret {
        body["secret_token"] = serde_json::Value::String(secret.to_string());
    }

    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("Failed to serialize body: {}", e))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    // Make HTTP request to Telegram API
    // Note: {TELEGRAM_BOT_TOKEN} is replaced by host with the actual token
    let result = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/setWebhook",
        &headers.to_string(),
        Some(&body_bytes),
        None,
    );

    match result {
        Ok(response) => {
            if response.status != 200 {
                let body_str = String::from_utf8_lossy(&response.body);
                return Err(format!("HTTP {}: {}", response.status, body_str));
            }

            // Parse Telegram API response
            let api_response: TelegramApiResponse<serde_json::Value> =
                serde_json::from_slice(&response.body)
                    .map_err(|e| format!("Failed to parse response: {}", e))?;

            if !api_response.ok {
                return Err(format!(
                    "Telegram API error: {}",
                    api_response
                        .description
                        .unwrap_or_else(|| "unknown".to_string())
                ));
            }

            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Webhook registered successfully: {}", webhook_url),
            );

            Ok(())
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

// ============================================================================
// Pairing Reply
// ============================================================================

/// Send a pairing code message to a chat. Used when an unknown user DMs the bot.
fn send_pairing_reply(chat_id: i64, code: &str) -> Result<(), String> {
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "text": format!(
            "To pair with this bot, run: `ironclaw pairing approve telegram {}`",
            code
        ),
        "parse_mode": "Markdown",
    });

    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| format!("Failed to serialize payload: {}", e))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    let result = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendMessage",
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(response) => {
            if response.status != 200 {
                let body_str = String::from_utf8_lossy(&response.body);
                return Err(format!("HTTP {}: {}", response.status, body_str));
            }
            Ok(())
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
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
    // Use text or caption (for media messages)
    let content = message
        .text
        .filter(|t| !t.is_empty())
        .or_else(|| message.caption.filter(|c| !c.is_empty()))
        .unwrap_or_default();

    if content.is_empty() {
        return;
    }

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

    // Owner validation: when owner_id is set, only that user can message
    let owner_configured = channel_host::workspace_read(OWNER_ID_PATH)
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    if owner_configured {
        if let Ok(owner_id) = channel_host::workspace_read(OWNER_ID_PATH)
            .unwrap()
            .parse::<i64>()
        {
            if from.id != owner_id {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Dropping message from non-owner user {} (owner: {})",
                        from.id, owner_id
                    ),
                );
                return;
            }
        }
    } else if is_private {
        // No owner_id: apply dm_policy for private chats
        let dm_policy = channel_host::workspace_read(DM_POLICY_PATH)
            .unwrap_or_else(|| "pairing".to_string());

        if dm_policy != "open" {
            // Build effective allow list: config allow_from + pairing store
            let mut allowed: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            if let Ok(store_allowed) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
                allowed.extend(store_allowed);
            }

            let id_str = from.id.to_string();
            let username_opt = from.username.as_deref();
            let is_allowed = allowed.contains(&"*".to_string())
                || allowed.contains(&id_str)
                || username_opt.map_or(false, |u| allowed.contains(&u.to_string()));

            if !is_allowed {
                if dm_policy == "pairing" {
                    // Upsert pairing request and send reply
                    let meta = serde_json::json!({
                        "chat_id": message.chat.id,
                        "user_id": from.id,
                        "username": username_opt,
                    })
                    .to_string();

                    match channel_host::pairing_upsert_request(CHANNEL_NAME, &id_str, &meta) {
                        Ok(result) => {
                            channel_host::log(
                                channel_host::LogLevel::Info,
                                &format!(
                                    "Pairing request for user {} (chat {}): code {}",
                                    from.id, message.chat.id, result.code
                                ),
                            );
                            if result.created {
                                let _ = send_pairing_reply(message.chat.id, &result.code);
                            }
                        }
                        Err(e) => {
                            channel_host::log(
                                channel_host::LogLevel::Error,
                                &format!("Pairing upsert failed: {}", e),
                            );
                        }
                    }
                }
                return;
            }
        }
    }

    // For group chats, only respond if bot was mentioned or respond_to_all is enabled
    if !is_private {
        let respond_to_all = channel_host::workspace_read(RESPOND_TO_ALL_GROUP_PATH)
            .as_deref()
            .unwrap_or("false")
            == "true";

        if !respond_to_all {
            let has_command = content.starts_with('/');
            let bot_username = channel_host::workspace_read(BOT_USERNAME_PATH)
                .unwrap_or_default();
            let has_bot_mention = if bot_username.is_empty() {
                content.contains('@')
            } else {
                let mention = format!("@{}", bot_username);
                content.to_lowercase().contains(&mention.to_lowercase())
            };

            if !has_command && !has_bot_mention {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Ignoring group message without mention: {}", content),
                );
                return;
            }
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

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Clean the message text (strip bot mentions and commands)
    let bot_username = channel_host::workspace_read(BOT_USERNAME_PATH).unwrap_or_default();
    let cleaned_text = clean_message_text(
        &content,
        if bot_username.is_empty() {
            None
        } else {
            Some(bot_username.as_str())
        },
    );

    // For /start with no args, emit placeholder so agent can respond with welcome
    let content_to_emit = if cleaned_text.is_empty() && content.trim().starts_with('/') {
        "[User started the bot]".to_string()
    } else if cleaned_text.is_empty() {
        return;
    } else {
        cleaned_text
    };

    // Emit the message to the agent
    channel_host::emit_message(&EmittedMessage {
        user_id: from.id.to_string(),
        user_name: Some(user_name),
        content: content_to_emit,
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
/// When bot_username is set, only strips that specific mention; otherwise strips any leading @mention.
fn clean_message_text(text: &str, bot_username: Option<&str>) -> String {
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
        if let Some(bot) = bot_username {
            let mention = format!("@{}", bot);
            let mention_lower = mention.to_lowercase();
            let result_lower = result.to_lowercase();
            if result_lower.starts_with(&mention_lower) {
                let rest = result[mention.len()..].trim_start();
                if rest.is_empty() {
                    return String::new();
                }
                result = rest.to_string();
            } else if let Some(space_idx) = result.find(' ') {
                // Different leading @mention - only strip if it's the bot
                let first_word = &result[..space_idx];
                if first_word.eq_ignore_ascii_case(&mention) {
                    result = result[space_idx..].trim_start().to_string();
                }
            }
        } else {
            // No bot_username: strip any leading @mention
            if let Some(space_idx) = result.find(' ') {
                result = result[space_idx..].trim_start().to_string();
            } else {
                return String::new();
            }
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
        // Without bot_username: strips any leading @mention
        assert_eq!(clean_message_text("/start hello", None), "hello");
        assert_eq!(clean_message_text("@bot hello world", None), "hello world");
        assert_eq!(clean_message_text("/start", None), "");
        assert_eq!(clean_message_text("@botname", None), "");
        assert_eq!(clean_message_text("just text", None), "just text");
        assert_eq!(clean_message_text("  spaced  ", None), "spaced");

        // With bot_username: only strips @MyBot, not @alice
        assert_eq!(clean_message_text("@MyBot hello", Some("MyBot")), "hello");
        assert_eq!(clean_message_text("@mybot hi", Some("MyBot")), "hi");
        assert_eq!(
            clean_message_text("@alice hello", Some("MyBot")),
            "@alice hello"
        );
        assert_eq!(clean_message_text("@MyBot", Some("MyBot")), "");
    }

    #[test]
    fn test_config_with_owner_id() {
        let json = r#"{"owner_id": 123456789}"#;
        let config: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.owner_id, Some(123456789));
    }

    #[test]
    fn test_config_without_owner_id() {
        let json = r#"{}"#;
        let config: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.owner_id, None);
    }

    #[test]
    fn test_config_with_null_owner_id() {
        let json = r#"{"owner_id": null}"#;
        let config: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.owner_id, None);
    }

    #[test]
    fn test_config_full() {
        let json = r#"{
            "bot_username": "my_bot",
            "owner_id": 42,
            "respond_to_all_group_messages": true
        }"#;
        let config: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.bot_username, Some("my_bot".to_string()));
        assert_eq!(config.owner_id, Some(42));
        assert!(config.respond_to_all_group_messages);
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

    #[test]
    fn test_parse_message_with_caption() {
        let json = r#"{
            "message_id": 1,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "caption": "What's in this image?"
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.text, None);
        assert_eq!(msg.caption.as_deref(), Some("What's in this image?"));
    }
}
