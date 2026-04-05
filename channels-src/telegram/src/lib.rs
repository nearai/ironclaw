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
    AgentResponse, Attachment, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage, InboundAttachment};

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

    /// Photo sizes (Telegram sends multiple sizes; last is largest).
    #[serde(default)]
    photo: Option<Vec<PhotoSize>>,

    /// Document attachment.
    document: Option<TelegramDocument>,

    /// Audio attachment.
    audio: Option<TelegramAudio>,

    /// Video attachment.
    video: Option<TelegramVideo>,

    /// Voice message.
    voice: Option<TelegramVoice>,

    /// Sticker.
    sticker: Option<TelegramSticker>,

    /// Forum topic ID. Present when the message is sent inside a forum topic.
    #[serde(default)]
    message_thread_id: Option<i64>,

    /// True when this message is sent inside a forum topic.
    #[serde(default)]
    is_topic_message: Option<bool>,
}

/// Telegram PhotoSize object.
#[derive(Debug, Deserialize)]
struct PhotoSize {
    file_id: String,
    file_unique_id: String,
    width: i32,
    height: i32,
    file_size: Option<i64>,
}

/// Telegram Document object.
#[derive(Debug, Deserialize)]
struct TelegramDocument {
    file_id: String,
    file_unique_id: String,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Audio object.
#[derive(Debug, Deserialize)]
struct TelegramAudio {
    file_id: String,
    file_unique_id: String,
    duration: Option<u32>,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Video object.
#[derive(Debug, Deserialize)]
struct TelegramVideo {
    file_id: String,
    file_unique_id: String,
    duration: Option<u32>,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Voice message object.
#[derive(Debug, Deserialize)]
struct TelegramVoice {
    file_id: String,
    file_unique_id: String,
    duration: u32,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Sticker object.
#[derive(Debug, Deserialize)]
struct TelegramSticker {
    file_id: String,
    file_unique_id: String,
    #[serde(rename = "type")]
    sticker_type: Option<String>,
    file_size: Option<i64>,
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

/// Telegram File object returned by getFile.
/// https://core.telegram.org/bots/api#file
#[derive(Debug, Deserialize)]
struct TelegramFile {
    /// Identifier for this file.
    #[allow(dead_code)]
    file_id: String,

    /// File path for downloading. Use https://api.telegram.org/file/bot<token>/<file_path>.
    file_path: Option<String>,
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

/// Workspace path for storing the most recently processed webhook update ID.
const WEBHOOK_STATE_PATH: &str = "state/last_webhook_update_id";

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

    /// Forum topic thread ID (for routing replies back to the correct topic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<i64>,
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

    /// When true, use polling mode even if tunnel_url is available.
    #[serde(default)]
    polling_enabled: bool,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct TelegramChannel;

#[derive(Debug, Clone, PartialEq, Eq)]
enum TelegramStatusAction {
    Typing,
    Notify(String),
}

const TELEGRAM_STATUS_MAX_CHARS: usize = 600;
/// Telegram's hard limit for message text length.
const TELEGRAM_MAX_MESSAGE_LEN: usize = 4096;

fn truncate_status_message(input: &str, max_chars: usize) -> String {
    let mut iter = input.chars();
    let truncated: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

/// Split a long message into chunks that fit within Telegram's 4096-char limit.
///
/// Tries to split at the most natural boundary available (in priority order):
/// 1. Double newline (paragraph break)
/// 2. Single newline
/// 3. Sentence end (`. `, `! `, `? `)
/// 4. Word boundary (space)
/// 5. Hard cut at the limit (last resort for pathological input)
fn split_message(text: &str) -> Vec<String> {
    if text.chars().count() <= TELEGRAM_MAX_MESSAGE_LEN {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Count chars to find the byte offset for our window.
        let window_bytes = remaining
            .char_indices()
            .take(TELEGRAM_MAX_MESSAGE_LEN)
            .last()
            .map(|(byte_idx, ch)| byte_idx + ch.len_utf8())
            .unwrap_or(remaining.len());

        if window_bytes >= remaining.len() {
            // Remainder fits entirely.
            chunks.push(remaining.to_string());
            break;
        }

        let window = &remaining[..window_bytes];

        // 1. Double newline — best paragraph boundary
        let split_at = window
            .rfind("\n\n")
            // 2. Single newline
            .or_else(|| window.rfind('\n'))
            // 3. Sentence-ending punctuation followed by space.
            //    Note: this only detects ASCII punctuation (. ! ?), not CJK
            //    sentence-ending marks (。！？). CJK text falls through to
            //    word-boundary or hard-cut splitting.
            .or_else(|| {
                let bytes = window.as_bytes();
                // Search backwards for '. ', '! ', '? '
                (1..bytes.len())
                    .rev()
                    .find(|&i| matches!(bytes[i - 1], b'.' | b'!' | b'?') && bytes[i] == b' ')
            })
            // 4. Word boundary (last space)
            .or_else(|| window.rfind(' '))
            // 5. Hard cut
            .unwrap_or(window_bytes);

        // Avoid empty chunks (e.g. text starting with \n\n).
        let split_at = if split_at == 0 {
            window_bytes
        } else {
            split_at
        };

        // Trim whitespace at chunk boundaries for clean Telegram display.
        // Note: this drops leading/trailing spaces at split points, which is
        // acceptable for chat messages but means the concatenation of chunks
        // may not exactly equal the original text when split at spaces.
        chunks.push(remaining[..split_at].trim_end().to_string());
        remaining = remaining[split_at..].trim_start();
    }

    chunks
}

fn status_message_for_user(update: &StatusUpdate) -> Option<String> {
    let message = update.message.trim();
    if message.is_empty() {
        None
    } else {
        Some(truncate_status_message(message, TELEGRAM_STATUS_MAX_CHARS))
    }
}

fn get_updates_url(offset: i64, timeout_secs: u32) -> String {
    format!(
        "https://api.telegram.org/bot{{TELEGRAM_BOT_TOKEN}}/getUpdates?offset={}&timeout={}&allowed_updates=[\"message\",\"edited_message\"]",
        offset, timeout_secs
    )
}

fn classify_status_update(update: &StatusUpdate) -> Option<TelegramStatusAction> {
    match update.status {
        StatusType::Thinking => Some(TelegramStatusAction::Typing),
        StatusType::Done | StatusType::Interrupted => None,
        // Tool telemetry can be noisy in chat; keep it as typing-only UX.
        StatusType::ToolStarted | StatusType::ToolCompleted | StatusType::ToolResult => None,
        StatusType::Status => {
            let msg = update.message.trim();
            if msg.eq_ignore_ascii_case("Done")
                || msg.eq_ignore_ascii_case("Interrupted")
                || msg.eq_ignore_ascii_case("Awaiting approval")
                || msg.eq_ignore_ascii_case("Rejected")
            {
                None
            } else {
                status_message_for_user(update).map(TelegramStatusAction::Notify)
            }
        }
        StatusType::ApprovalNeeded
        | StatusType::JobStarted
        | StatusType::AuthRequired
        | StatusType::AuthCompleted => {
            status_message_for_user(update).map(TelegramStatusAction::Notify)
        }
    }
}

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
        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing").to_string();
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

        // Mode: use polling if explicitly enabled, otherwise use webhooks when tunnel available.
        let webhook_mode = config.tunnel_url.is_some() && !config.polling_enabled;

        if webhook_mode {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Webhook mode enabled (tunnel configured)",
            );

            // Register webhook with Telegram API — propagate errors so a bad token
            // causes activation to fail rather than silently succeeding.
            if let Some(ref tunnel_url) = config.tunnel_url {
                // Clear any stale webhook first to avoid 409 Conflict
                let _ = delete_webhook();

                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Registering webhook: {}/webhook/telegram", tunnel_url),
                );

                register_webhook(tunnel_url, config.webhook_secret.as_deref())
                    .map_err(|e| format!("Failed to register webhook: {}", e))?;
            }
        } else {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Polling mode enabled (no tunnel configured)",
            );

            // Delete any existing webhook before polling. Telegram returns success
            // when no webhook exists, so any error here (e.g. 401) means a bad token.
            delete_webhook().map_err(|e| format!("Bot token validation failed: {}", e))?;
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

        let last_processed = channel_host::workspace_read(WEBHOOK_STATE_PATH)
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(-1);
        let update_id = update.update_id;
        if update_id <= last_processed {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!(
                    "Skipping duplicate or stale webhook update {} (last processed {})",
                    update_id, last_processed
                ),
            );
            return json_response(200, serde_json::json!({"ok": true}));
        }

        // Handle the update
        handle_update(update);

        if let Err(err) = channel_host::workspace_write(WEBHOOK_STATE_PATH, &update_id.to_string())
        {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to persist webhook update id: {}", err),
            );
        }

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

        let headers_json = serde_json::json!({}).to_string();
        let primary_url = get_updates_url(offset, 25);

        // 35s HTTP timeout outlives Telegram's 30s server-side long-poll.
        // If the TCP connection drops, retry once immediately with a short poll
        // so we don't wait a full extra tick (~30s) before delivering updates.
        let result = match channel_host::http_request(
            "GET",
            &primary_url,
            &headers_json,
            None,
            Some(35_000),
        ) {
            Ok(response) => Ok(response),
            Err(primary_err) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "getUpdates request failed ({}), retrying once immediately",
                        primary_err
                    ),
                );

                let retry_url = get_updates_url(offset, 3);
                channel_host::http_request("GET", &retry_url, &headers_json, None, Some(8_000))
                    .map_err(|retry_err| {
                        format!("primary error: {}; retry error: {}", primary_err, retry_err)
                    })
            }
        };

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

        send_response(
            metadata.chat_id,
            &response,
            Some(metadata.message_id),
            metadata.message_thread_id,
        )
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        let chat_id: i64 = user_id
            .parse()
            .map_err(|e| format!("Invalid chat_id '{}': {}", user_id, e))?;

        send_response(chat_id, &response, None, None)
    }

    fn on_status(update: StatusUpdate) {
        let action = match classify_status_update(&update) {
            Some(action) => action,
            None => return,
        };

        // Parse chat_id from metadata
        let metadata: TelegramMessageMetadata = match serde_json::from_str(&update.metadata_json) {
            Ok(m) => m,
            Err(_) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    "on_status: no valid Telegram metadata, skipping status update",
                );
                return;
            }
        };

        match action {
            TelegramStatusAction::Typing => {
                // POST /sendChatAction with action "typing"
                let mut payload = serde_json::json!({
                    "chat_id": metadata.chat_id,
                    "action": "typing"
                });

                if let Some(thread_id) = metadata.message_thread_id {
                    payload["message_thread_id"] = serde_json::Value::Number(thread_id.into());
                }

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
            TelegramStatusAction::Notify(prompt) => {
                // Send user-visible status updates for actionable events.
                if let Err(first_err) = send_message(
                    metadata.chat_id,
                    &prompt,
                    Some(metadata.message_id),
                    None,
                    metadata.message_thread_id,
                ) {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!(
                            "Failed to send status reply ({}), retrying without reply context",
                            first_err
                        ),
                    );

                    if let Err(retry_err) = send_message(
                        metadata.chat_id,
                        &prompt,
                        None,
                        None,
                        metadata.message_thread_id,
                    ) {
                        channel_host::log(
                            channel_host::LogLevel::Debug,
                            &format!(
                                "Failed to send status message without reply context: {}",
                                retry_err
                            ),
                        );
                    }
                }
            }
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

/// Normalize `message_thread_id` for outbound API calls.
///
/// Telegram rejects `sendMessage` and file-send methods when
/// `message_thread_id = 1` (the "General" topic), so omit it in that case.
fn normalize_thread_id(thread_id: Option<i64>) -> Option<i64> {
    thread_id.filter(|&id| id != 1)
}

/// Send a message via the Telegram Bot API.
///
/// Returns the sent message_id on success. When `parse_mode` is set and
/// Telegram returns a 400 "can't parse entities" error, returns
/// `SendError::ParseEntities` so the caller can retry without formatting.
fn send_message(
    chat_id: i64,
    text: &str,
    reply_to_message_id: Option<i64>,
    parse_mode: Option<&str>,
    message_thread_id: Option<i64>,
) -> Result<i64, SendError> {
    let message_thread_id = normalize_thread_id(message_thread_id);

    let mut payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    });

    if let Some(message_id) = reply_to_message_id {
        payload["reply_to_message_id"] = serde_json::Value::Number(message_id.into());
    }

    if let Some(mode) = parse_mode {
        payload["parse_mode"] = serde_json::Value::String(mode.to_string());
    }

    if let Some(thread_id) = message_thread_id {
        payload["message_thread_id"] = serde_json::Value::Number(thread_id.into());
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
// Voice File Download
// ============================================================================

/// Percent-encode a string for safe use as a URL query parameter value.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

/// Maximum file size to download (20 MB). Files larger than this are discarded
/// to avoid excessive memory use and slow downloads in the WASM runtime.
const MAX_DOWNLOAD_SIZE_BYTES: u64 = 20 * 1024 * 1024;

fn download_telegram_file(file_id: &str) -> Result<Vec<u8>, String> {
    // Reject file_id containing curly braces to prevent credential placeholder injection
    if file_id.contains('{') || file_id.contains('}') {
        return Err("invalid file_id: contains forbidden characters".to_string());
    }

    // Step 1: Call getFile to get file_path
    let get_file_url = format!(
        "https://api.telegram.org/bot{{TELEGRAM_BOT_TOKEN}}/getFile?file_id={}",
        percent_encode(file_id)
    );

    let headers = serde_json::json!({});
    let result = channel_host::http_request("GET", &get_file_url, &headers.to_string(), None, None);

    let response = result.map_err(|e| format!("getFile request failed: {}", e))?;

    if response.status != 200 {
        let body_str = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "getFile returned {}: {}",
            response.status, body_str
        ));
    }

    let api_response: TelegramApiResponse<TelegramFile> = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse getFile response: {}", e))?;

    if !api_response.ok {
        return Err(format!(
            "getFile API error: {}",
            api_response
                .description
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }

    let file = api_response
        .result
        .ok_or_else(|| "getFile returned no result".to_string())?;

    let file_path = file
        .file_path
        .ok_or_else(|| "getFile returned no file_path".to_string())?;

    // Sanitize file_path against credential placeholder injection
    if file_path.contains('{') || file_path.contains('}') {
        return Err("invalid file_path: contains forbidden characters".to_string());
    }

    // Step 2: Download the actual file bytes
    let download_url = format!(
        "https://api.telegram.org/file/bot{{TELEGRAM_BOT_TOKEN}}/{}",
        file_path
    );

    let result = channel_host::http_request("GET", &download_url, &headers.to_string(), None, None);

    let response = result.map_err(|e| format!("File download failed: {}", e))?;

    if response.status != 200 {
        return Err(format!("File download returned status {}", response.status));
    }

    // Post-download size guard: Telegram metadata file_size is optional,
    // so enforce the limit on actual downloaded bytes.
    if response.body.len() as u64 > MAX_DOWNLOAD_SIZE_BYTES {
        return Err(format!(
            "Downloaded file exceeds {} MB limit ({} bytes)",
            MAX_DOWNLOAD_SIZE_BYTES / (1024 * 1024),
            response.body.len()
        ));
    }

    Ok(response.body)
}

// ============================================================================
// Attachment Sending (Photo / Voice / Document)
// ============================================================================

/// Write a multipart/form-data text field.
fn write_multipart_field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", name).as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

/// Write a multipart/form-data file field.
fn write_multipart_file(
    body: &mut Vec<u8>,
    boundary: &str,
    field: &str,
    filename: &str,
    content_type: &str,
    data: &[u8],
) {
    // Sanitize filename: strip quotes, newlines, and non-ASCII to prevent header injection
    let safe_filename: String = filename
        .chars()
        .filter(|c| *c != '"' && *c != '\r' && *c != '\n' && *c != '\\' && c.is_ascii())
        .collect();
    let safe_filename = if safe_filename.is_empty() {
        "file".to_string()
    } else {
        safe_filename
    };
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
            field, safe_filename
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", content_type).as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(b"\r\n");
}

/// Image MIME types that Telegram's sendPhoto API supports.
const PHOTO_MIME_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// Audio MIME types that Telegram's sendVoice API supports (ogg/opus container).
const VOICE_MIME_TYPES: &[&str] = &["audio/ogg", "audio/opus"];

/// Maximum photo size for Telegram sendPhoto (10 MB).
const MAX_PHOTO_SIZE: usize = 10 * 1024 * 1024;

/// Maximum voice note size for Telegram sendVoice (50 MB).
const MAX_VOICE_SIZE: usize = 50 * 1024 * 1024;

/// Send a multipart file upload to a Telegram Bot API endpoint.
///
/// Shared implementation for sendPhoto, sendVoice, and sendDocument.
/// `api_method` is the Telegram method name (e.g. "sendPhoto"),
/// `field_name` is the multipart field (e.g. "photo", "voice", "document").
#[allow(clippy::too_many_arguments)]
fn send_multipart_upload(
    api_method: &str,
    field_name: &str,
    chat_id: i64,
    filename: &str,
    mime_type: &str,
    data: &[u8],
    reply_to_message_id: Option<i64>,
    message_thread_id: Option<i64>,
) -> Result<(), String> {
    let message_thread_id = normalize_thread_id(message_thread_id);

    let boundary = format!("ironclaw-{}", channel_host::now_millis());
    let mut body = Vec::new();

    write_multipart_field(&mut body, &boundary, "chat_id", &chat_id.to_string());
    if let Some(msg_id) = reply_to_message_id {
        write_multipart_field(
            &mut body,
            &boundary,
            "reply_to_message_id",
            &msg_id.to_string(),
        );
    }
    if let Some(thread_id) = message_thread_id {
        write_multipart_field(
            &mut body,
            &boundary,
            "message_thread_id",
            &thread_id.to_string(),
        );
    }
    write_multipart_file(&mut body, &boundary, field_name, filename, mime_type, data);
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

    let headers = serde_json::json!({
        "Content-Type": format!("multipart/form-data; boundary={}", boundary)
    });

    let url = format!(
        "https://api.telegram.org/bot{{TELEGRAM_BOT_TOKEN}}/{}",
        api_method
    );

    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&body),
        Some(60_000), // 60s timeout for file uploads
    );

    match result {
        Ok(resp) if resp.status == 200 => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Sent {} '{}' to chat {}", field_name, filename, chat_id),
            );
            Ok(())
        }
        Ok(resp) => {
            let body_str = String::from_utf8_lossy(&resp.body);
            Err(format!(
                "{} failed (HTTP {}): {}",
                api_method, resp.status, body_str
            ))
        }
        Err(e) => Err(format!("{} HTTP request failed: {}", api_method, e)),
    }
}

/// Send a photo via the Telegram Bot API (multipart upload).
///
/// Falls back to `send_document()` if the photo exceeds 10 MB.
fn send_photo(
    chat_id: i64,
    filename: &str,
    mime_type: &str,
    data: &[u8],
    reply_to_message_id: Option<i64>,
    message_thread_id: Option<i64>,
) -> Result<(), String> {
    if data.len() > MAX_PHOTO_SIZE {
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Photo {} exceeds 10MB ({}), sending as document",
                filename,
                data.len()
            ),
        );
        return send_document(
            chat_id,
            filename,
            mime_type,
            data,
            reply_to_message_id,
            message_thread_id,
        );
    }
    send_multipart_upload(
        "sendPhoto",
        "photo",
        chat_id,
        filename,
        mime_type,
        data,
        reply_to_message_id,
        message_thread_id,
    )
}

/// Send a document via the Telegram Bot API (multipart upload).
fn send_document(
    chat_id: i64,
    filename: &str,
    mime_type: &str,
    data: &[u8],
    reply_to_message_id: Option<i64>,
    message_thread_id: Option<i64>,
) -> Result<(), String> {
    send_multipart_upload(
        "sendDocument",
        "document",
        chat_id,
        filename,
        mime_type,
        data,
        reply_to_message_id,
        message_thread_id,
    )
}

/// Send a voice note via the Telegram Bot API (multipart upload).
///
/// Telegram's `sendVoice` requires ogg/opus audio and displays it as an
/// in-chat voice note with waveform and playback controls.
/// Falls back to `send_document()` if the voice note exceeds 50 MB.
fn send_voice(
    chat_id: i64,
    filename: &str,
    mime_type: &str,
    data: &[u8],
    reply_to_message_id: Option<i64>,
    message_thread_id: Option<i64>,
) -> Result<(), String> {
    if data.len() > MAX_VOICE_SIZE {
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Voice note {} exceeds 50MB ({}), sending as document",
                filename,
                data.len()
            ),
        );
        return send_document(
            chat_id,
            filename,
            mime_type,
            data,
            reply_to_message_id,
            message_thread_id,
        );
    }
    send_multipart_upload(
        "sendVoice",
        "voice",
        chat_id,
        filename,
        mime_type,
        data,
        reply_to_message_id,
        message_thread_id,
    )
}

/// Send a full agent response (attachments + text) to a chat.
///
/// Shared implementation for both `on_respond` and `on_broadcast`.
fn send_response(
    chat_id: i64,
    response: &AgentResponse,
    reply_to_message_id: Option<i64>,
    message_thread_id: Option<i64>,
) -> Result<(), String> {
    // Send attachments first (photos/documents)
    for attachment in &response.attachments {
        send_attachment(chat_id, attachment, reply_to_message_id, message_thread_id)?;
    }

    // Skip text if empty and we already sent attachments
    if response.content.is_empty() && !response.attachments.is_empty() {
        return Ok(());
    }

    // Split large messages into chunks that fit Telegram's limit.
    let chunks = split_message(&response.content);
    let total = chunks.len();

    // The first chunk replies to the original message; subsequent chunks
    // reply to the previously sent chunk so they form a visual thread.
    let mut reply_to = reply_to_message_id;

    for (i, chunk) in chunks.into_iter().enumerate() {
        // Try Markdown, fall back to plain text on parse errors
        let result = send_message(
            chat_id,
            &chunk,
            reply_to,
            Some("Markdown"),
            message_thread_id,
        );

        let msg_id = match result {
            Ok(id) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Sent message chunk {}/{} to chat {}: message_id={}",
                        i + 1,
                        total,
                        chat_id,
                        id,
                    ),
                );
                id
            }
            Err(SendError::ParseEntities(detail)) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "Markdown parse failed on chunk {}/{} ({}), retrying as plain text",
                        i + 1,
                        total,
                        detail
                    ),
                );
                let id = send_message(chat_id, &chunk, reply_to, None, message_thread_id)
                    .map_err(|e| format!("Plain-text retry also failed: {}", e))?;
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Sent plain-text chunk {}/{} to chat {}: message_id={}",
                        i + 1,
                        total,
                        chat_id,
                        id,
                    ),
                );
                id
            }
            Err(e) => return Err(e.to_string()),
        };

        // Each subsequent chunk threads off the previous sent message.
        reply_to = Some(msg_id);
    }

    Ok(())
}

/// Extract the base MIME type, stripping any parameters after `;`.
///
/// e.g. `"audio/ogg; codecs=opus"` → `"audio/ogg"`
fn base_mime_type(mime: &str) -> &str {
    mime.split(';').next().unwrap_or(mime).trim()
}

/// Attachment routing category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachmentKind {
    Photo,
    Voice,
    Document,
}

/// Classify an attachment's send method based on its MIME type.
fn classify_attachment(mime_type: &str) -> AttachmentKind {
    let base = base_mime_type(mime_type);
    if PHOTO_MIME_TYPES.contains(&base) {
        AttachmentKind::Photo
    } else if VOICE_MIME_TYPES.contains(&base) {
        AttachmentKind::Voice
    } else {
        AttachmentKind::Document
    }
}

/// Send a single attachment, choosing sendPhoto, sendVoice, or sendDocument based on MIME type.
fn send_attachment(
    chat_id: i64,
    attachment: &Attachment,
    reply_to_message_id: Option<i64>,
    message_thread_id: Option<i64>,
) -> Result<(), String> {
    match classify_attachment(&attachment.mime_type) {
        AttachmentKind::Photo => send_photo(
            chat_id,
            &attachment.filename,
            &attachment.mime_type,
            &attachment.data,
            reply_to_message_id,
            message_thread_id,
        ),
        AttachmentKind::Voice => send_voice(
            chat_id,
            &attachment.filename,
            &attachment.mime_type,
            &attachment.data,
            reply_to_message_id,
            message_thread_id,
        ),
        AttachmentKind::Document => send_document(
            chat_id,
            &attachment.filename,
            &attachment.mime_type,
            &attachment.data,
            reply_to_message_id,
            message_thread_id,
        ),
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

    let mut response = match result {
        Ok(response) => response,
        Err(e) => return Err(format!("HTTP request failed: {}", e)),
    };

    let mut retried = false;
    if response.status == 409 {
        channel_host::log(
            channel_host::LogLevel::Warn,
            "409 Conflict -- deleting existing webhook and retrying",
        );
        let _ = delete_webhook();
        retried = true;

        response = match channel_host::http_request(
            "POST",
            "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/setWebhook",
            &headers.to_string(),
            Some(&body_bytes),
            None,
        ) {
            Ok(resp) => resp,
            Err(e) => return Err(format!("HTTP request failed (after 409 retry): {}", e)),
        };
    }

    if response.status != 200 {
        let body_str = String::from_utf8_lossy(&response.body);
        let context = if retried { " (after 409 retry)" } else { "" };
        return Err(format!("HTTP {}{}: {}", response.status, context, body_str));
    }

    // Parse Telegram API response
    let api_response: TelegramApiResponse<serde_json::Value> =
        serde_json::from_slice(&response.body)
            .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !api_response.ok {
        let context = if retried { " (after 409 retry)" } else { "" };
        return Err(format!(
            "Telegram API error{}: {}",
            context,
            api_response
                .description
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }

    let context = if retried { " (after retry)" } else { "" };
    channel_host::log(
        channel_host::LogLevel::Info,
        &format!(
            "Webhook registered successfully{}: {}",
            context, webhook_url
        ),
    );

    Ok(())
}

// ============================================================================
// Pairing Reply
// ============================================================================

/// Send a pairing code message to a chat. Used when an unknown user DMs the bot.
fn send_pairing_reply(chat_id: i64, code: &str) -> Result<(), String> {
    send_message(
        chat_id,
        &format!(
            "Enter this code in IronClaw to pair your telegram account: `{}`. CLI fallback: `ironclaw pairing approve telegram {}`",
            code, code
        ),
        None,
        Some("Markdown"),
        None,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
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

/// Build extras-json with optional duration.
fn extras_json(duration_secs: Option<u32>) -> String {
    match duration_secs {
        Some(d) => format!(r#"{{"duration_secs":{}}}"#, d),
        None => String::new(),
    }
}

/// Build an inbound attachment with the standard fields.
fn make_inbound_attachment(
    id: String,
    mime_type: String,
    filename: Option<String>,
    size_bytes: Option<u64>,
    source_url: Option<String>,
    extracted_text: Option<String>,
    duration_secs: Option<u32>,
) -> InboundAttachment {
    InboundAttachment {
        id,
        mime_type,
        filename,
        size_bytes,
        source_url,
        storage_key: None,
        extracted_text,
        extras_json: extras_json(duration_secs),
    }
}

/// Extract attachments from a Telegram message.
fn extract_attachments(message: &TelegramMessage) -> Vec<InboundAttachment> {
    let mut attachments = Vec::new();
    let get_file_url = |file_id: &str| {
        format!(
            "https://api.telegram.org/bot{{TELEGRAM_BOT_TOKEN}}/getFile?file_id={}",
            percent_encode(file_id)
        )
    };

    // Photo: Telegram sends multiple sizes; use the largest (last).
    if let Some(ref photos) = message.photo {
        if let Some(largest) = photos.last() {
            attachments.push(make_inbound_attachment(
                largest.file_id.clone(),
                "image/jpeg".to_string(),
                None,
                largest.file_size.map(|s| s as u64),
                Some(get_file_url(&largest.file_id)),
                None,
                None,
            ));
        }
    }

    // Document
    if let Some(ref doc) = message.document {
        attachments.push(make_inbound_attachment(
            doc.file_id.clone(),
            doc.mime_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            doc.file_name.clone(),
            doc.file_size.map(|s| s as u64),
            Some(get_file_url(&doc.file_id)),
            None,
            None,
        ));
    }

    // Audio
    if let Some(ref audio) = message.audio {
        attachments.push(make_inbound_attachment(
            audio.file_id.clone(),
            audio
                .mime_type
                .clone()
                .unwrap_or_else(|| "audio/mpeg".to_string()),
            audio.file_name.clone(),
            audio.file_size.map(|s| s as u64),
            Some(get_file_url(&audio.file_id)),
            None,
            audio.duration,
        ));
    }

    // Video
    if let Some(ref video) = message.video {
        attachments.push(make_inbound_attachment(
            video.file_id.clone(),
            video
                .mime_type
                .clone()
                .unwrap_or_else(|| "video/mp4".to_string()),
            video.file_name.clone(),
            video.file_size.map(|s| s as u64),
            Some(get_file_url(&video.file_id)),
            None,
            video.duration,
        ));
    }

    // Voice
    if let Some(ref voice) = message.voice {
        let mime_type = voice
            .mime_type
            .clone()
            .unwrap_or_else(|| "audio/ogg".to_string());

        attachments.push(make_inbound_attachment(
            voice.file_id.clone(),
            mime_type,
            Some(format!("voice_{}.ogg", voice.file_id)),
            voice.file_size.map(|s| s as u64),
            Some(get_file_url(&voice.file_id)),
            None,
            Some(voice.duration),
        ));
    }

    // Sticker
    if let Some(ref sticker) = message.sticker {
        attachments.push(make_inbound_attachment(
            sticker.file_id.clone(),
            "image/webp".to_string(),
            None,
            sticker.file_size.map(|s| s as u64),
            Some(get_file_url(&sticker.file_id)),
            None,
            None,
        ));
    }

    attachments
}

/// Download voice file bytes and store them via the host for transcription.
///
/// Separated from `extract_attachments` so that function stays pure (no host
/// calls) and remains testable in native unit tests.
fn download_and_store_voice(attachments: &[InboundAttachment]) {
    for att in attachments {
        // Voice attachments have a generated filename like "voice_<id>.ogg"
        let is_voice = att
            .filename
            .as_ref()
            .is_some_and(|f| f.starts_with("voice_"));
        if !is_voice {
            continue;
        }

        match download_telegram_file(&att.id) {
            Ok(bytes) => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Downloaded voice file: {} bytes", bytes.len()),
                );
                if let Err(e) = channel_host::store_attachment_data(&att.id, &bytes) {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to store voice data: {}", e),
                    );
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to download voice file: {}", e),
                );
            }
        }
    }
}

/// Download image file bytes and store them via the host for the vision pipeline.
///
/// Separated from `extract_attachments` so that function stays pure (no host
/// calls) and remains testable in native unit tests.
fn download_and_store_images(attachments: &[InboundAttachment]) {
    for att in attachments {
        if !att.mime_type.starts_with("image/") {
            continue;
        }

        match download_telegram_file(&att.id) {
            Ok(bytes) => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Downloaded image file: {} bytes", bytes.len()),
                );
                if let Err(e) = channel_host::store_attachment_data(&att.id, &bytes) {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to store image data: {}", e),
                    );
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to download image file: {}", e),
                );
            }
        }
    }
}

/// Returns true if the attachment should be downloaded for document text extraction.
///
/// Excludes voice (handled by transcription), image (vision pipeline),
/// audio (transcription), and video attachments.
fn is_downloadable_document(att: &InboundAttachment) -> bool {
    let is_voice = att
        .filename
        .as_ref()
        .is_some_and(|f| f.starts_with("voice_"));
    if is_voice {
        return false;
    }
    if att.mime_type.starts_with("image/")
        || att.mime_type.starts_with("audio/")
        || att.mime_type.starts_with("video/")
    {
        return false;
    }
    true
}

/// Download document file bytes and store them via the host for text extraction.
///
/// Downloads any attachment that isn't voice or image so the host-side
/// `DocumentExtractionMiddleware` can extract text from PDFs, Office docs, etc.
///
/// On failure, sets `extracted_text` to an error message so the user gets feedback.
fn download_and_store_documents(attachments: &mut [InboundAttachment]) {
    for att in attachments.iter_mut() {
        if !is_downloadable_document(att) {
            continue;
        }

        match download_telegram_file(&att.id) {
            Ok(bytes) => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!(
                        "Downloaded document file: {} bytes, mime={}",
                        bytes.len(),
                        att.mime_type
                    ),
                );
                if let Err(e) = channel_host::store_attachment_data(&att.id, &bytes) {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to store document data: {}", e),
                    );
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to download document file: {}", e),
                );
                let name = att.filename.as_deref().unwrap_or("document");
                att.extracted_text = Some(format!(
                    "[Failed to download '{name}': {e}. \
                     The file may be too large or unavailable. Please try a smaller file.]"
                ));
            }
        }
    }
}

/// Process a single message.
fn handle_message(message: TelegramMessage) {
    // Extract attachments from media fields (pure data mapping, no host calls)
    let mut attachments = extract_attachments(&message);

    // Download and store voice attachments for host-side transcription
    download_and_store_voice(&attachments);

    // Download and store image attachments for host-side vision pipeline
    download_and_store_images(&attachments);

    // Download and store document attachments for host-side text extraction
    download_and_store_documents(&mut attachments);

    // Use text or caption (for media messages)
    let has_voice = message.voice.is_some();
    let content = message
        .text
        .filter(|t| !t.is_empty())
        .or_else(|| message.caption.filter(|c| !c.is_empty()))
        .unwrap_or_else(|| {
            if has_voice {
                "[Voice note]".to_string()
            } else {
                String::new()
            }
        });

    // Allow messages with attachments even if text content is empty
    if content.is_empty() && attachments.is_empty() {
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

    let owner_id = channel_host::workspace_read(OWNER_ID_PATH)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<i64>().ok());
    let is_owner = owner_id == Some(from.id);

    if !is_owner {
        // Non-owner senders remain guests. Apply authorization based on
        // dm_policy / allow_from before letting them chat in their own scope.
        let dm_policy =
            channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

        // For private chats with non-open policy, check allowlist
        // For group chats with non-open policy, also check allowlist
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
                || username_opt.is_some_and(|u| allowed.contains(&u.to_string()));

            if !is_allowed {
                if is_private && dm_policy == "pairing" {
                    // Upsert pairing request and send reply (only for private chats)
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
                } else if !is_private {
                    // For group chats with non-open dm_policy, just log and drop
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!(
                            "Dropping message from unauthorized user {} in group chat",
                            from.id
                        ),
                    );
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
            let bot_username = channel_host::workspace_read(BOT_USERNAME_PATH).unwrap_or_default();
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
        message_thread_id: message.message_thread_id,
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    let bot_username = channel_host::workspace_read(BOT_USERNAME_PATH).unwrap_or_default();
    let content_to_emit = match content_to_emit_for_agent(
        &content,
        if bot_username.is_empty() {
            None
        } else {
            Some(bot_username.as_str())
        },
    ) {
        Some(value) => value,
        // Allow attachment-only messages even without text
        None if !attachments.is_empty() => String::new(),
        None => return,
    };

    // Emit the message to the agent
    channel_host::emit_message(&EmittedMessage {
        user_id: from.id.to_string(),
        user_name: Some(user_name),
        content: content_to_emit,
        thread_id: Some(message.chat.id.to_string()),
        metadata_json,
        attachments,
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

/// Decide which user content should be emitted to the agent loop.
///
/// - `/start` emits a placeholder so the agent can greet the user
/// - bare slash commands are passed through for Submission parsing
/// - empty/mention-only messages are ignored
/// - otherwise cleaned text is emitted
fn content_to_emit_for_agent(content: &str, bot_username: Option<&str>) -> Option<String> {
    let cleaned_text = clean_message_text(content, bot_username);
    let trimmed_content = content.trim();

    if trimmed_content.eq_ignore_ascii_case("/start") {
        return Some("[User started the bot]".to_string());
    }

    if cleaned_text.is_empty() && trimmed_content.starts_with('/') {
        return Some(trimmed_content.to_string());
    }

    if cleaned_text.is_empty() {
        return None;
    }

    Some(cleaned_text)
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
    fn test_split_message_short() {
        let text = "Hello, world!";
        let chunks = split_message(text);
        assert_eq!(chunks, vec![text]);
    }

    #[test]
    fn test_split_message_paragraph_boundary() {
        let para_a = "A".repeat(3000);
        let para_b = "B".repeat(3000);
        let text = format!("{}\n\n{}", para_a, para_b);
        let chunks = split_message(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], para_a);
        assert_eq!(chunks[1], para_b);
    }

    #[test]
    fn test_split_message_word_boundary() {
        // Build a string well over the limit with no newlines.
        let words: Vec<String> = (0..1000).map(|i| format!("word{:04}", i)).collect();
        let text = words.join(" ");
        assert!(text.len() > TELEGRAM_MAX_MESSAGE_LEN);
        let chunks = split_message(&text);
        assert!(chunks.len() > 1, "expected multiple chunks");
        for chunk in &chunks {
            assert!(chunk.chars().count() <= TELEGRAM_MAX_MESSAGE_LEN);
        }
        // Rejoined chunks must equal the original text exactly.
        let rejoined = chunks.join(" ");
        assert_eq!(rejoined, text);
    }

    #[test]
    fn test_split_message_each_chunk_fits() {
        // Stress-test: 20 000 chars of mixed text.
        let text: String = (0..500)
            .map(|i| format!("Sentence number {}. ", i))
            .collect();
        assert!(text.len() > TELEGRAM_MAX_MESSAGE_LEN);
        let chunks = split_message(&text);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= TELEGRAM_MAX_MESSAGE_LEN);
        }
    }

    #[test]
    fn test_split_message_sentence_boundary() {
        // Build text that exceeds the limit, with sentence boundaries inside.
        let sentence = "This is a test sentence. ";
        let repeat_count = TELEGRAM_MAX_MESSAGE_LEN / sentence.len() + 5;
        let text: String = sentence.repeat(repeat_count);
        assert!(text.chars().count() > TELEGRAM_MAX_MESSAGE_LEN);

        let chunks = split_message(&text);
        assert!(chunks.len() > 1);
        // First chunk should end at a sentence boundary (trimmed)
        let first = &chunks[0];
        assert!(
            first.ends_with('.'),
            "First chunk should end at a sentence boundary, got: ...{}",
            &first[first.len().saturating_sub(20)..]
        );
    }

    #[test]
    fn test_split_message_hard_cut_no_spaces() {
        // Pathological input: a single huge "word" with no spaces or newlines.
        let text = "x".repeat(TELEGRAM_MAX_MESSAGE_LEN * 2 + 100);
        let chunks = split_message(&text);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= TELEGRAM_MAX_MESSAGE_LEN);
        }
        // Rejoined must preserve all characters
        let rejoined: String = chunks.concat();
        assert_eq!(rejoined, text);
    }

    #[test]
    fn test_split_message_multibyte_chars() {
        // Emoji are 4 bytes each. Ensure we don't panic or split mid-character.
        let emoji = "\u{1F600}"; // 😀
        let text: String = emoji.repeat(TELEGRAM_MAX_MESSAGE_LEN + 100);
        assert!(text.chars().count() > TELEGRAM_MAX_MESSAGE_LEN);

        let chunks = split_message(&text);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= TELEGRAM_MAX_MESSAGE_LEN);
            // Every char should be a complete emoji
            assert!(chunk.chars().all(|c| c == '\u{1F600}'));
        }
    }

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
    fn test_clean_message_text_bare_commands() {
        // Bare commands return empty (the caller decides what to emit)
        assert_eq!(clean_message_text("/start", None), "");
        assert_eq!(clean_message_text("/interrupt", None), "");
        assert_eq!(clean_message_text("/stop", None), "");
        assert_eq!(clean_message_text("/help", None), "");
        assert_eq!(clean_message_text("/undo", None), "");
        assert_eq!(clean_message_text("/ping", None), "");

        // Commands with args: command prefix stripped, args returned
        assert_eq!(clean_message_text("/start hello", None), "hello");
        assert_eq!(clean_message_text("/help me please", None), "me please");
        assert_eq!(
            clean_message_text("/model claude-opus-4-6", None),
            "claude-opus-4-6"
        );
    }

    /// Tests for the content_to_emit logic in handle_message.
    /// Since handle_message uses WASM host calls, test the extracted decision function.
    #[test]
    fn test_content_to_emit_logic() {
        // /start → welcome placeholder
        assert_eq!(
            content_to_emit_for_agent("/start", None),
            Some("[User started the bot]".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/Start", None),
            Some("[User started the bot]".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("  /start  ", None),
            Some("[User started the bot]".to_string())
        );

        // /start with args → pass args through
        assert_eq!(
            content_to_emit_for_agent("/start hello", None),
            Some("hello".to_string())
        );

        // Control commands → pass through raw so Submission::parse() can match
        assert_eq!(
            content_to_emit_for_agent("/interrupt", None),
            Some("/interrupt".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/stop", None),
            Some("/stop".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/help", None),
            Some("/help".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/undo", None),
            Some("/undo".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/redo", None),
            Some("/redo".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/ping", None),
            Some("/ping".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/tools", None),
            Some("/tools".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/compact", None),
            Some("/compact".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/clear", None),
            Some("/clear".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/version", None),
            Some("/version".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/approve", None),
            Some("/approve".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/always", None),
            Some("/always".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/deny", None),
            Some("/deny".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/yes", None),
            Some("/yes".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/no", None),
            Some("/no".to_string())
        );

        // Commands with args → cleaned text (command stripped)
        assert_eq!(
            content_to_emit_for_agent("/help me please", None),
            Some("me please".to_string())
        );

        // Plain text → pass through
        assert_eq!(
            content_to_emit_for_agent("hello world", None),
            Some("hello world".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("just text", None),
            Some("just text".to_string())
        );

        // Empty / whitespace → skip (None)
        assert_eq!(content_to_emit_for_agent("", None), None);
        assert_eq!(content_to_emit_for_agent("   ", None), None);

        // Bare @mention without bot → skip
        assert_eq!(content_to_emit_for_agent("@botname", None), None);

        // With bot username configured: other mentions are preserved.
        assert_eq!(
            content_to_emit_for_agent("@alice hello", Some("MyBot")),
            Some("@alice hello".to_string())
        );
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

    #[test]
    fn test_get_updates_url_includes_offset_and_timeout() {
        let url = get_updates_url(444_809_884, 30);
        assert!(url.contains("offset=444809884"));
        assert!(url.contains("timeout=30"));
        assert!(url.contains("allowed_updates=[\"message\",\"edited_message\"]"));
    }

    #[test]
    fn test_classify_status_update_thinking() {
        let update = StatusUpdate {
            status: StatusType::Thinking,
            message: "Thinking...".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Typing)
        );
    }

    #[test]
    fn test_classify_status_update_approval_needed() {
        let update = StatusUpdate {
            status: StatusType::ApprovalNeeded,
            message: "Approval needed for tool 'http_request'".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Approval needed for tool 'http_request'".to_string()
            ))
        );
    }

    #[test]
    fn test_classify_status_update_done_ignored() {
        let update = StatusUpdate {
            status: StatusType::Done,
            message: "Done".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_auth_required() {
        let update = StatusUpdate {
            status: StatusType::AuthRequired,
            message: "Authentication required for weather.".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Authentication required for weather.".to_string()
            ))
        );
    }

    #[test]
    fn test_classify_status_update_tool_started_ignored() {
        let update = StatusUpdate {
            status: StatusType::ToolStarted,
            message: "Tool started: http_request".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_tool_completed_ignored() {
        let update = StatusUpdate {
            status: StatusType::ToolCompleted,
            message: "Tool completed: http_request (ok)".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_job_started_notify() {
        let update = StatusUpdate {
            status: StatusType::JobStarted,
            message: "Job started: Daily sync".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Job started: Daily sync".to_string()
            ))
        );
    }

    #[test]
    fn test_classify_status_update_auth_completed_notify() {
        let update = StatusUpdate {
            status: StatusType::AuthCompleted,
            message: "Authentication completed for weather.".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Authentication completed for weather.".to_string()
            ))
        );
    }

    #[test]
    fn test_classify_status_update_tool_result_ignored() {
        let update = StatusUpdate {
            status: StatusType::ToolResult,
            message: "Tool result: http_request ...".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_awaiting_approval_ignored() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "Awaiting approval".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_interrupted_ignored() {
        let update = StatusUpdate {
            status: StatusType::Interrupted,
            message: "Interrupted".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_status_done_ignored_case_insensitive() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "done".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_status_interrupted_ignored() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "interrupted".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_status_rejected_ignored() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "Rejected".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_status_notify() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "Context compaction started".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Context compaction started".to_string()
            ))
        );
    }

    #[test]
    fn test_status_message_for_user_ignores_blank() {
        let update = StatusUpdate {
            status: StatusType::AuthRequired,
            message: "   ".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(status_message_for_user(&update), None);
    }

    #[test]
    fn test_truncate_status_message_appends_ellipsis() {
        let input = "abcdefghijklmnopqrstuvwxyz";
        let output = truncate_status_message(input, 10);
        assert_eq!(output, "abcdefghij...");
    }

    #[test]
    fn test_status_message_for_user_truncates_long_input() {
        let update = StatusUpdate {
            status: StatusType::AuthRequired,
            message: "x".repeat(700),
            metadata_json: "{}".to_string(),
        };

        let msg = status_message_for_user(&update).expect("expected message");
        assert!(msg.len() <= TELEGRAM_STATUS_MAX_CHARS + 3);
        assert!(msg.ends_with("..."));
    }

    // === Attachment extraction fixture tests ===

    #[test]
    fn test_extract_attachments_photo() {
        let json = r#"{
            "message_id": 1,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "caption": "What is this?",
            "photo": [
                {"file_id": "small_id", "file_unique_id": "s1", "width": 90, "height": 90, "file_size": 1234},
                {"file_id": "large_id", "file_unique_id": "l1", "width": 800, "height": 600, "file_size": 54321}
            ]
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        let attachments = extract_attachments(&msg);

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "large_id"); // Largest photo
        assert_eq!(attachments[0].mime_type, "image/jpeg");
        assert_eq!(attachments[0].size_bytes, Some(54321));
        assert!(
            attachments[0]
                .source_url
                .as_ref()
                .unwrap()
                .contains("large_id")
        );
    }

    #[test]
    fn test_extract_attachments_document() {
        let json = r#"{
            "message_id": 2,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "document": {
                "file_id": "doc_abc",
                "file_unique_id": "d1",
                "file_name": "report.pdf",
                "mime_type": "application/pdf",
                "file_size": 102400
            },
            "caption": "Here is the report"
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        let attachments = extract_attachments(&msg);

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "doc_abc");
        assert_eq!(attachments[0].mime_type, "application/pdf");
        assert_eq!(attachments[0].filename, Some("report.pdf".to_string()));
        assert_eq!(attachments[0].size_bytes, Some(102400));
    }

    #[test]
    fn test_extract_attachments_voice() {
        let json = r#"{
            "message_id": 3,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "voice": {
                "file_id": "voice_xyz",
                "file_unique_id": "v1",
                "duration": 5,
                "mime_type": "audio/ogg",
                "file_size": 9000
            }
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        let attachments = extract_attachments(&msg);

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "voice_xyz");
        assert_eq!(attachments[0].mime_type, "audio/ogg");
        assert_eq!(
            attachments[0].filename.as_deref(),
            Some("voice_voice_xyz.ogg")
        );
        assert!(attachments[0].extras_json.contains("\"duration_secs\":5"));
    }

    #[test]
    fn test_extract_attachments_video() {
        let json = r#"{
            "message_id": 4,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "video": {
                "file_id": "vid_1",
                "file_unique_id": "vv1",
                "file_name": "clip.mp4",
                "mime_type": "video/mp4",
                "file_size": 5000000
            },
            "caption": "Check this out"
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        let attachments = extract_attachments(&msg);

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "vid_1");
        assert_eq!(attachments[0].mime_type, "video/mp4");
        assert_eq!(attachments[0].filename, Some("clip.mp4".to_string()));
    }

    #[test]
    fn test_extract_attachments_audio() {
        let json = r#"{
            "message_id": 5,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "audio": {
                "file_id": "audio_1",
                "file_unique_id": "a1",
                "file_name": "song.mp3",
                "mime_type": "audio/mpeg",
                "file_size": 3000000
            }
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        let attachments = extract_attachments(&msg);

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "audio_1");
        assert_eq!(attachments[0].mime_type, "audio/mpeg");
        assert_eq!(attachments[0].filename, Some("song.mp3".to_string()));
    }

    #[test]
    fn test_extract_attachments_sticker() {
        let json = r#"{
            "message_id": 6,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "sticker": {
                "file_id": "sticker_1",
                "file_unique_id": "st1",
                "type": "regular",
                "file_size": 20000
            }
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        let attachments = extract_attachments(&msg);

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "sticker_1");
        assert_eq!(attachments[0].mime_type, "image/webp");
    }

    #[test]
    fn test_extract_attachments_text_only_empty() {
        let json = r#"{
            "message_id": 7,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "text": "Hello"
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        let attachments = extract_attachments(&msg);

        assert!(attachments.is_empty());
    }

    #[test]
    fn test_extract_attachments_multiple_types() {
        let json = r#"{
            "message_id": 8,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "photo": [
                {"file_id": "photo_1", "file_unique_id": "p1", "width": 100, "height": 100}
            ],
            "document": {
                "file_id": "doc_1",
                "file_unique_id": "d1",
                "file_name": "file.txt",
                "mime_type": "text/plain"
            }
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        let attachments = extract_attachments(&msg);

        // Both photo and document should be extracted
        assert_eq!(attachments.len(), 2);
    }

    #[test]
    fn test_parse_update_with_photo_fallback_content() {
        // A photo-only message (no text, no caption) should have empty content
        // but still produce attachments
        let json = r#"{
            "message_id": 9,
            "from": {"id": 42, "is_bot": false, "first_name": "Test"},
            "chat": {"id": 42, "type": "private"},
            "photo": [
                {"file_id": "ph1", "file_unique_id": "u1", "width": 320, "height": 240}
            ]
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();

        // Content is empty (no text, no caption)
        assert!(msg.text.is_none());
        assert!(msg.caption.is_none());

        // But attachments exist
        let attachments = extract_attachments(&msg);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "ph1");
    }

    #[test]
    fn test_is_downloadable_document() {
        let make = |mime: &str, filename: Option<&str>| InboundAttachment {
            id: "test".to_string(),
            mime_type: mime.to_string(),
            filename: filename.map(|s| s.to_string()),
            size_bytes: Some(1024),
            source_url: None,
            storage_key: None,
            extracted_text: None,
            extras_json: String::new(),
        };

        // PDFs and Office docs should be downloaded
        assert!(is_downloadable_document(&make(
            "application/pdf",
            Some("report.pdf")
        )));
        assert!(is_downloadable_document(&make(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Some("doc.docx"),
        )));
        assert!(is_downloadable_document(&make(
            "text/plain",
            Some("notes.txt")
        )));

        // Voice, image, audio, video should NOT be downloaded
        assert!(!is_downloadable_document(&make(
            "audio/ogg",
            Some("voice_123.ogg")
        )));
        assert!(!is_downloadable_document(&make("image/jpeg", None)));
        assert!(!is_downloadable_document(&make(
            "audio/mpeg",
            Some("song.mp3")
        )));
        assert!(!is_downloadable_document(&make(
            "video/mp4",
            Some("clip.mp4")
        )));
    }

    #[test]
    fn test_max_download_size_constant() {
        // Verify the constant is 20 MB, matching the Slack channel limit
        assert_eq!(MAX_DOWNLOAD_SIZE_BYTES, 20 * 1024 * 1024);
    }

    #[test]
    fn test_base_mime_type() {
        assert_eq!(base_mime_type("audio/ogg"), "audio/ogg");
        assert_eq!(base_mime_type("audio/ogg; codecs=opus"), "audio/ogg");
        assert_eq!(base_mime_type("image/jpeg"), "image/jpeg");
        assert_eq!(base_mime_type("text/plain; charset=utf-8"), "text/plain");
        assert_eq!(base_mime_type(""), "");
    }

    #[test]
    fn test_classify_attachment_routing() {
        // Photos
        assert_eq!(classify_attachment("image/jpeg"), AttachmentKind::Photo);
        assert_eq!(classify_attachment("image/png"), AttachmentKind::Photo);
        assert_eq!(classify_attachment("image/gif"), AttachmentKind::Photo);
        assert_eq!(classify_attachment("image/webp"), AttachmentKind::Photo);

        // Voice notes — exact and parameterized
        assert_eq!(classify_attachment("audio/ogg"), AttachmentKind::Voice);
        assert_eq!(classify_attachment("audio/opus"), AttachmentKind::Voice);
        assert_eq!(
            classify_attachment("audio/ogg; codecs=opus"),
            AttachmentKind::Voice
        );

        // Everything else falls through to document
        assert_eq!(
            classify_attachment("application/pdf"),
            AttachmentKind::Document
        );
        assert_eq!(classify_attachment("audio/mpeg"), AttachmentKind::Document);
        assert_eq!(classify_attachment("video/mp4"), AttachmentKind::Document);
    }
}
