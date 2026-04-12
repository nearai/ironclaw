// LINE message types have fields reserved for future use
#![allow(dead_code)]

//! LINE Messaging API channel for IronClaw.
//!
//! This WASM component implements the channel interface for handling LINE
//! webhooks and sending messages back via the Messaging API.
//!
//! # Features
//!
//! - Webhook-based message receiving
//! - DM (one-to-one) chat support
//! - Group/room chat support with @mention triggering
//! - Media attachment handling (image, video, audio, file)
//! - DM pairing for guest access control
//! - Message chunking for LINE's 5000-char limit
//!
//! # Security
//!
//! - Channel access token is injected by host during HTTP requests
//! - WASM never sees raw credentials
//! - Webhook signature (X-Line-Signature) validated by host

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage, InboundAttachment};

// ============================================================================
// LINE API Types
// ============================================================================

/// LINE webhook body envelope.
/// https://developers.line.biz/en/reference/messaging-api/#webhooks
#[derive(Debug, Deserialize)]
struct LineWebhookBody {
    /// User ID of the bot that received the events.
    #[serde(default)]
    destination: Option<String>,

    /// Array of webhook event objects.
    #[serde(default)]
    events: Vec<LineEvent>,
}

/// LINE webhook event.
/// https://developers.line.biz/en/reference/messaging-api/#webhook-event-objects
#[derive(Debug, Deserialize)]
struct LineEvent {
    /// Event type: message, follow, unfollow, join, leave, postback, etc.
    #[serde(rename = "type", default)]
    event_type: String,

    /// Token for replying to this event. Expires ~1 minute after receipt.
    #[serde(rename = "replyToken")]
    reply_token: Option<String>,

    /// Source of the event (user, group, or room).
    source: Option<LineSource>,

    /// Message object (only present for message events).
    message: Option<LineMessage>,

    /// Timestamp in milliseconds since epoch.
    timestamp: Option<i64>,
}

/// Source of a LINE event.
#[derive(Debug, Deserialize)]
struct LineSource {
    /// Source type: "user", "group", or "room".
    #[serde(rename = "type", default)]
    source_type: String,

    /// User ID of the sender.
    #[serde(rename = "userId")]
    user_id: Option<String>,

    /// Group ID (only for group sources).
    #[serde(rename = "groupId")]
    group_id: Option<String>,

    /// Room ID (only for room sources).
    #[serde(rename = "roomId")]
    room_id: Option<String>,
}

/// LINE message object.
/// https://developers.line.biz/en/reference/messaging-api/#message-event
#[derive(Debug, Deserialize)]
struct LineMessage {
    /// Message ID.
    id: Option<String>,

    /// Message type: text, image, video, audio, file, location, sticker.
    #[serde(rename = "type", default)]
    message_type: String,

    /// Text content (for text messages).
    text: Option<String>,

    /// File name (for file messages).
    #[serde(rename = "fileName")]
    file_name: Option<String>,

    /// File size in bytes (for file messages).
    #[serde(rename = "fileSize")]
    file_size: Option<u64>,

    /// Content provider info.
    #[serde(rename = "contentProvider")]
    content_provider: Option<ContentProvider>,

    /// Duration in milliseconds (for audio/video).
    duration: Option<u64>,
}

/// Content provider info for media messages.
#[derive(Debug, Deserialize)]
struct ContentProvider {
    /// "line" for LINE-hosted content, "external" for external URLs.
    #[serde(rename = "type", default)]
    provider_type: String,

    /// External URL (only for external providers).
    #[serde(rename = "originalContentUrl")]
    original_content_url: Option<String>,
}

// ============================================================================
// Channel Constants
// ============================================================================

/// Channel name for pairing store.
const CHANNEL_NAME: &str = "line";

/// LINE's hard limit for text message length.
const LINE_MAX_MESSAGE_LEN: usize = 5000;

/// Maximum number of messages per LINE reply/push API call.
const LINE_MAX_MESSAGES_PER_REQUEST: usize = 5;

/// Workspace paths for persisting config across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";
const ALLOW_FROM_PATH: &str = "state/allow_from";
const BOT_NAME_PATH: &str = "state/bot_name";
const RESPOND_TO_ALL_GROUP_PATH: &str = "state/respond_to_all_group_messages";
const OWNER_ID_PATH: &str = "state/owner_id";

// ============================================================================
// Channel Metadata
// ============================================================================

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
struct LineMessageMetadata {
    /// User ID who sent the message.
    user_id: String,

    /// Reply token for responding (expires ~1 min).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reply_token: Option<String>,

    /// Whether this is a DM (user source) vs group/room.
    is_dm: bool,

    /// Group or room ID if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group_id: Option<String>,
}

/// Channel configuration injected by host.
#[derive(Debug, Deserialize)]
struct LineConfig {
    /// Bot display name for mention detection in groups.
    #[serde(default)]
    bot_name: Option<String>,

    /// Owner user ID restriction.
    #[serde(default)]
    owner_id: Option<String>,

    /// DM policy: "pairing" (default), "allowlist", or "open".
    #[serde(default)]
    dm_policy: Option<String>,

    /// Allowed sender IDs from config.
    #[serde(default)]
    allow_from: Option<Vec<String>>,

    /// Whether to respond to all group messages (not just mentions).
    #[serde(default)]
    respond_to_all_group_messages: bool,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct LineChannel;

impl Guest for LineChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("LINE channel config: {}", config_json),
        );

        let config: LineConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "LINE channel starting");

        // Persist config values for use in subsequent callbacks
        if let Some(ref owner_id) = config.owner_id {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, owner_id);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: {}", owner_id),
            );
        } else {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
        }

        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing").to_string();
        let _ = channel_host::workspace_write(DM_POLICY_PATH, &dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.clone().unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        let _ = channel_host::workspace_write(
            BOT_NAME_PATH,
            &config.bot_name.clone().unwrap_or_default(),
        );
        let _ = channel_host::workspace_write(
            RESPOND_TO_ALL_GROUP_PATH,
            &config.respond_to_all_group_messages.to_string(),
        );

        if let Some(ref name) = config.bot_name {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Bot name: {}", name),
            );
        }

        // Validate that LINE credentials exist
        if !channel_host::secret_exists("line_channel_access_token") {
            return Err("LINE channel access token not configured".to_string());
        }
        if !channel_host::secret_exists("line_channel_secret") {
            return Err("LINE channel secret not configured".to_string());
        }

        Ok(ChannelConfig {
            display_name: "LINE".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/line".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: true,
            }],
            poll: None, // LINE is webhook-only
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Defense in depth: reject if host signature validation failed
        if !req.secret_validated {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Webhook request with invalid or missing X-Line-Signature",
            );
            return json_response(403, serde_json::json!({"error": "Invalid signature"}));
        }

        // Parse the request body as UTF-8
        let body_str = match std::str::from_utf8(&req.body) {
            Ok(s) => s,
            Err(_) => {
                return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
            }
        };

        // Parse as LINE webhook body
        let webhook: LineWebhookBody = match serde_json::from_str(body_str) {
            Ok(w) => w,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse LINE webhook: {}", e),
                );
                // Return 200 to prevent LINE from retrying
                return json_response(200, serde_json::json!({}));
            }
        };

        // Process each event
        for event in webhook.events {
            handle_event(event);
        }

        // LINE expects a 200 OK response
        json_response(200, serde_json::json!({}))
    }

    fn on_poll() {
        // LINE does not support polling — this should never be called.
        channel_host::log(
            channel_host::LogLevel::Debug,
            "on_poll called but LINE is webhook-only",
        );
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: LineMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        send_response(&metadata, &response)
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        // Broadcast uses push API (no reply token)
        send_push_messages(&user_id, &response)
    }

    fn on_status(update: StatusUpdate) {
        // LINE does not have a public typing indicator API for bots.
        // Log the status for debugging purposes only.
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Status update: {:?} - {}",
                update.status, update.message
            ),
        );
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "LINE channel shutting down",
        );
    }
}

// ============================================================================
// Event Handling
// ============================================================================

/// Process a single LINE webhook event.
fn handle_event(event: LineEvent) {
    match event.event_type.as_str() {
        "message" => handle_message_event(event),
        "follow" => {
            channel_host::log(
                channel_host::LogLevel::Info,
                "User followed the bot (follow event)",
            );
        }
        "unfollow" => {
            channel_host::log(
                channel_host::LogLevel::Info,
                "User unfollowed the bot (unfollow event)",
            );
        }
        "join" => {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Bot joined a group/room (join event)",
            );
        }
        "leave" => {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Bot left a group/room (leave event)",
            );
        }
        other => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Ignoring event type: {}", other),
            );
        }
    }
}

/// Handle a message event — extract content and emit to agent.
fn handle_message_event(event: LineEvent) {
    let source = match event.source {
        Some(s) => s,
        None => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Message event with no source, skipping",
            );
            return;
        }
    };

    let user_id = match source.user_id {
        Some(ref id) => id.clone(),
        None => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Message event with no user ID, skipping",
            );
            return;
        }
    };

    let is_dm = source.source_type == "user";
    let group_id = source.group_id.clone().or_else(|| source.room_id.clone());

    // Check owner restriction
    let owner_id = channel_host::workspace_read(OWNER_ID_PATH).unwrap_or_default();
    if !owner_id.is_empty() && user_id != owner_id {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Dropping message from non-owner user: {}", user_id),
        );
        return;
    }

    // DM pairing / access control
    if is_dm {
        let dm_policy = channel_host::workspace_read(DM_POLICY_PATH)
            .unwrap_or_else(|| "pairing".to_string());

        match dm_policy.as_str() {
            "pairing" => {
                // Check if user is paired
                match channel_host::pairing_resolve_identity(CHANNEL_NAME, &user_id) {
                    Ok(Some(_owner)) => {
                        // User is paired, proceed
                    }
                    Ok(None) => {
                        // Not paired — upsert a pairing request and reply with the code
                        let meta = serde_json::json!({
                            "user_id": user_id,
                            "channel": "line",
                        })
                        .to_string();

                        match channel_host::pairing_upsert_request(CHANNEL_NAME, &user_id, &meta) {
                            Ok(result) => {
                                // Send pairing code to user
                                if let Some(ref reply_token) = event.reply_token {
                                    let msg = format!(
                                        "Enter this code in IronClaw to pair your LINE account: {}\n\nCLI: ironclaw pairing approve line {}",
                                        result.code, result.code
                                    );
                                    let _ = send_reply_text(reply_token, &msg);
                                }
                            }
                            Err(e) => {
                                channel_host::log(
                                    channel_host::LogLevel::Error,
                                    &format!("Failed to upsert pairing: {}", e),
                                );
                            }
                        }
                        return;
                    }
                    Err(e) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!("Pairing resolve failed: {}", e),
                        );
                        return;
                    }
                }
            }
            "allowlist" => {
                // Check allow_from list
                let allow_from_json = channel_host::workspace_read(ALLOW_FROM_PATH)
                    .unwrap_or_else(|| "[]".to_string());
                let allow_from: Vec<String> =
                    serde_json::from_str(&allow_from_json).unwrap_or_default();

                // Also check pairing-approved IDs
                let paired_ids = channel_host::pairing_read_allow_from(CHANNEL_NAME)
                    .unwrap_or_default();

                if !allow_from.contains(&user_id) && !paired_ids.contains(&user_id) {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("Dropping DM from non-allowed user: {}", user_id),
                    );
                    return;
                }
            }
            "open" => {
                // Accept all DMs
            }
            _ => {
                // Unknown policy, default to pairing behavior
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Unknown dm_policy '{}', treating as pairing", dm_policy),
                );
            }
        }
    } else {
        // Group/room message — check mention policy
        let respond_to_all = channel_host::workspace_read(RESPOND_TO_ALL_GROUP_PATH)
            .map(|v| v == "true")
            .unwrap_or(false);

        if !respond_to_all {
            // Check if the bot was mentioned in the message text
            let bot_name = channel_host::workspace_read(BOT_NAME_PATH).unwrap_or_default();
            let message_text = event
                .message
                .as_ref()
                .and_then(|m| m.text.as_deref())
                .unwrap_or("");

            if bot_name.is_empty() || !message_text.contains(&bot_name) {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    "Group message without bot mention, skipping",
                );
                return;
            }
        }
    }

    // Extract message content and attachments
    let message = match event.message {
        Some(m) => m,
        None => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                "Message event with no message object, skipping",
            );
            return;
        }
    };

    let (content, attachments) = extract_message_content(&message);

    // Build metadata for response routing
    let metadata = LineMessageMetadata {
        user_id: user_id.clone(),
        reply_token: event.reply_token,
        is_dm,
        group_id,
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Determine thread_id for group conversations
    let thread_id = metadata.group_id.clone();

    // Emit message to the agent
    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: None,
        content,
        thread_id,
        metadata_json,
        attachments,
    });
}

/// Extract text content and attachments from a LINE message.
fn extract_message_content(message: &LineMessage) -> (String, Vec<InboundAttachment>) {
    let mut attachments = Vec::new();

    match message.message_type.as_str() {
        "text" => {
            let text = message.text.clone().unwrap_or_default();
            (text, attachments)
        }
        "image" => {
            if let Some(ref id) = message.id {
                let source_url = format!(
                    "https://api-data.line.me/v2/bot/message/{}/content",
                    id
                );
                attachments.push(InboundAttachment {
                    id: id.clone(),
                    mime_type: "image/jpeg".to_string(),
                    filename: None,
                    size_bytes: None,
                    source_url: Some(source_url),
                    storage_key: None,
                    extracted_text: None,
                    extras_json: String::new(),
                });
            }
            let text = message.text.clone().unwrap_or_else(|| "[Image]".to_string());
            (text, attachments)
        }
        "video" => {
            if let Some(ref id) = message.id {
                let source_url = format!(
                    "https://api-data.line.me/v2/bot/message/{}/content",
                    id
                );
                let extras = match message.duration {
                    Some(ms) => format!(r#"{{"duration_secs":{}}}"#, ms / 1000),
                    None => String::new(),
                };
                attachments.push(InboundAttachment {
                    id: id.clone(),
                    mime_type: "video/mp4".to_string(),
                    filename: None,
                    size_bytes: None,
                    source_url: Some(source_url),
                    storage_key: None,
                    extracted_text: None,
                    extras_json: extras,
                });
            }
            let text = message.text.clone().unwrap_or_else(|| "[Video]".to_string());
            (text, attachments)
        }
        "audio" => {
            if let Some(ref id) = message.id {
                let source_url = format!(
                    "https://api-data.line.me/v2/bot/message/{}/content",
                    id
                );
                let extras = match message.duration {
                    Some(ms) => format!(r#"{{"duration_secs":{}}}"#, ms / 1000),
                    None => String::new(),
                };
                attachments.push(InboundAttachment {
                    id: id.clone(),
                    mime_type: "audio/m4a".to_string(),
                    filename: None,
                    size_bytes: None,
                    source_url: Some(source_url),
                    storage_key: None,
                    extracted_text: None,
                    extras_json: extras,
                });
            }
            let text = message.text.clone().unwrap_or_else(|| "[Audio]".to_string());
            (text, attachments)
        }
        "file" => {
            if let Some(ref id) = message.id {
                let source_url = format!(
                    "https://api-data.line.me/v2/bot/message/{}/content",
                    id
                );
                let mime = infer_mime_from_filename(message.file_name.as_deref());
                attachments.push(InboundAttachment {
                    id: id.clone(),
                    mime_type: mime,
                    filename: message.file_name.clone(),
                    size_bytes: message.file_size,
                    source_url: Some(source_url),
                    storage_key: None,
                    extracted_text: None,
                    extras_json: String::new(),
                });
            }
            let name = message
                .file_name
                .as_deref()
                .unwrap_or("file");
            let text = format!("[File: {}]", name);
            (text, attachments)
        }
        "location" => {
            let text = message
                .text
                .clone()
                .unwrap_or_else(|| "[Location]".to_string());
            (text, attachments)
        }
        "sticker" => {
            ("[Sticker]".to_string(), attachments)
        }
        other => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Unknown message type: {}", other),
            );
            let text = format!("[{}]", other);
            (text, attachments)
        }
    }
}

/// Infer MIME type from a filename extension.
fn infer_mime_from_filename(filename: Option<&str>) -> String {
    let ext = filename
        .and_then(|f| f.rsplit('.').next())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        Some("pdf") => "application/pdf".to_string(),
        Some("doc") | Some("docx") => "application/msword".to_string(),
        Some("xls") | Some("xlsx") => "application/vnd.ms-excel".to_string(),
        Some("ppt") | Some("pptx") => "application/vnd.ms-powerpoint".to_string(),
        Some("zip") => "application/zip".to_string(),
        Some("txt") => "text/plain".to_string(),
        Some("csv") => "text/csv".to_string(),
        Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
        Some("png") => "image/png".to_string(),
        Some("gif") => "image/gif".to_string(),
        Some("mp4") => "video/mp4".to_string(),
        Some("mp3") => "audio/mpeg".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

// ============================================================================
// Response Sending
// ============================================================================

/// Send an agent response back to LINE, using reply API first with push fallback.
fn send_response(metadata: &LineMessageMetadata, response: &AgentResponse) -> Result<(), String> {
    // Build text messages, chunked at LINE's limit
    let mut line_messages = Vec::new();

    // Add text chunks
    if !response.content.is_empty() {
        let chunks = split_message(&response.content);
        for chunk in chunks {
            line_messages.push(serde_json::json!({
                "type": "text",
                "text": chunk,
            }));
        }
    }

    if line_messages.is_empty() {
        return Ok(());
    }

    // Try reply API first (if we have a reply token)
    if let Some(ref reply_token) = metadata.reply_token {
        // LINE allows max 5 messages per reply — send in batches
        for batch in line_messages.chunks(LINE_MAX_MESSAGES_PER_REQUEST) {
            let payload = serde_json::json!({
                "replyToken": reply_token,
                "messages": batch,
            });

            match send_line_api("reply", &payload) {
                Ok(_) => {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("Reply sent ({} messages)", batch.len()),
                    );
                    // Reply token is consumed after first use — remaining batches use push
                    if line_messages.len() > LINE_MAX_MESSAGES_PER_REQUEST {
                        // Send remaining via push
                        let remaining: Vec<_> = line_messages
                            .iter()
                            .skip(LINE_MAX_MESSAGES_PER_REQUEST)
                            .cloned()
                            .collect();
                        return send_push_message_batches(&metadata.user_id, &remaining);
                    }
                    return Ok(());
                }
                Err(e) => {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Reply failed ({}), falling back to push", e),
                    );
                    // Fall through to push API
                    break;
                }
            }
        }
    }

    // Fallback: send all via push API
    send_push_message_batches(&metadata.user_id, &line_messages)
}

/// Send a proactive message via push API (for on-broadcast).
fn send_push_messages(user_id: &str, response: &AgentResponse) -> Result<(), String> {
    let mut line_messages = Vec::new();

    if !response.content.is_empty() {
        let chunks = split_message(&response.content);
        for chunk in chunks {
            line_messages.push(serde_json::json!({
                "type": "text",
                "text": chunk,
            }));
        }
    }

    if line_messages.is_empty() {
        return Ok(());
    }

    send_push_message_batches(user_id, &line_messages)
}

/// Send messages via push API in batches of 5.
fn send_push_message_batches(
    user_id: &str,
    messages: &[serde_json::Value],
) -> Result<(), String> {
    for batch in messages.chunks(LINE_MAX_MESSAGES_PER_REQUEST) {
        let payload = serde_json::json!({
            "to": user_id,
            "messages": batch,
        });

        send_line_api("push", &payload)?;

        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Push sent ({} messages) to {}", batch.len(), user_id),
        );
    }
    Ok(())
}

/// Send a simple text reply using the reply token.
fn send_reply_text(reply_token: &str, text: &str) -> Result<(), String> {
    let payload = serde_json::json!({
        "replyToken": reply_token,
        "messages": [{
            "type": "text",
            "text": text,
        }],
    });

    send_line_api("reply", &payload)
}

/// Call the LINE Messaging API (reply or push endpoint).
fn send_line_api(method: &str, payload: &serde_json::Value) -> Result<(), String> {
    let url = format!("https://api.line.me/v2/bot/message/{}", method);

    let payload_bytes = serde_json::to_vec(payload)
        .map_err(|e| format!("Failed to serialize payload: {}", e))?;

    let headers = serde_json::json!({ "Content-Type": "application/json" });

    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(resp) => {
            if resp.status == 200 {
                Ok(())
            } else {
                let body_str = String::from_utf8_lossy(&resp.body);
                Err(format!("LINE API {} returned {}: {}", method, resp.status, body_str))
            }
        }
        Err(e) => Err(format!("HTTP request to LINE {} failed: {}", method, e)),
    }
}

// ============================================================================
// Message Splitting
// ============================================================================

/// Split a long message into chunks that fit within LINE's 5000-char limit.
///
/// Tries to split at the most natural boundary available:
/// 1. Double newline (paragraph break)
/// 2. Single newline
/// 3. Sentence end (`. `, `! `, `? `)
/// 4. Word boundary (space)
/// 5. Hard cut at the limit
fn split_message(text: &str) -> Vec<String> {
    if text.chars().count() <= LINE_MAX_MESSAGE_LEN {
        return vec![text.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        let window_bytes = remaining
            .char_indices()
            .take(LINE_MAX_MESSAGE_LEN)
            .last()
            .map(|(byte_idx, ch)| byte_idx + ch.len_utf8())
            .unwrap_or(remaining.len());

        if window_bytes >= remaining.len() {
            chunks.push(remaining.to_string());
            break;
        }

        let window = &remaining[..window_bytes];

        let split_at = window
            .rfind("\n\n")
            .or_else(|| window.rfind('\n'))
            .or_else(|| {
                let bytes = window.as_bytes();
                (1..bytes.len()).rev().find(|&i| {
                    matches!(bytes[i - 1], b'.' | b'!' | b'?') && bytes[i] == b' '
                })
            })
            .or_else(|| window.rfind(' '))
            .unwrap_or(window_bytes);

        let split_at = if split_at == 0 { window_bytes } else { split_at };

        chunks.push(remaining[..split_at].trim_end().to_string());
        remaining = remaining[split_at..].trim_start();
    }

    chunks
}

// ============================================================================
// HTTP Response Helper
// ============================================================================

/// Build a JSON HTTP response.
fn json_response(status: u16, body: serde_json::Value) -> OutgoingHttpResponse {
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    let headers = serde_json::json!({
        "Content-Type": "application/json",
    });

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body: body_bytes,
    }
}

// Register the WASM component export.
export!(LineChannel);
