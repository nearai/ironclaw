// Feishu API types have fields reserved for future use
#![allow(dead_code)]

//! Feishu (Lark) Bot channel for IronClaw.
//!
//! This WASM component implements the channel interface for handling Feishu
//! event subscriptions (webhooks) and sending messages back via the Feishu API.
//!
//! # Features
//!
//! - Webhook-based message receiving (event subscription)
//! - URL verification challenge handling
//! - Private chat (P2P) support
//! - Group chat support with @mention triggering
//! - Reply threading support
//! - User name extraction
//!
//! # Security
//!
//! - App credentials are managed by host (tenant_access_token auto-refresh)
//! - WASM never sees raw credentials
//! - Verification token validated by host

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, Attachment, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage, InboundAttachment};

// ============================================================================
// Feishu API Types
// ============================================================================

/// Feishu event callback payload (schema 2.0).
#[derive(Debug, Deserialize)]
struct FeishuEventCallback {
    /// Schema version (should be "2.0").
    #[serde(default)]
    schema: Option<String>,

    /// Event header with metadata.
    header: Option<FeishuEventHeader>,

    /// The actual event payload.
    event: Option<serde_json::Value>,

    // URL verification fields (only present for challenge requests)
    /// Challenge string for URL verification.
    challenge: Option<String>,

    /// Verification token (present in both events and challenges).
    token: Option<String>,

    /// Request type: "url_verification" for challenge requests.
    #[serde(rename = "type")]
    request_type: Option<String>,
}

/// Feishu event header.
#[derive(Debug, Deserialize)]
struct FeishuEventHeader {
    /// Unique event ID.
    event_id: String,

    /// Event type (e.g., "im.message.receive_v1").
    event_type: String,

    /// Event creation timestamp.
    create_time: Option<String>,

    /// Verification token.
    token: Option<String>,

    /// App ID.
    app_id: Option<String>,

    /// Tenant key.
    tenant_key: Option<String>,
}

/// Feishu message receive event payload.
#[derive(Debug, Deserialize)]
struct FeishuMessageEvent {
    /// Sender information.
    sender: FeishuSender,

    /// Message information.
    message: FeishuMessage,
}

/// Feishu sender information.
#[derive(Debug, Deserialize)]
struct FeishuSender {
    /// Sender ID information.
    sender_id: FeishuSenderId,

    /// Sender type: "user" or "app".
    sender_type: Option<String>,

    /// Tenant key.
    tenant_key: Option<String>,
}

/// Feishu sender ID.
#[derive(Debug, Deserialize)]
struct FeishuSenderId {
    /// Open ID (unique per app).
    open_id: Option<String>,

    /// User ID (enterprise-scoped).
    user_id: Option<String>,

    /// Union ID (cross-app).
    union_id: Option<String>,
}

/// Feishu message information.
#[derive(Debug, Deserialize)]
struct FeishuMessage {
    /// Unique message ID.
    message_id: String,

    /// Thread root message ID (for threaded conversations).
    root_id: Option<String>,

    /// Parent message ID (direct reply target).
    parent_id: Option<String>,

    /// Chat ID (group or P2P conversation).
    chat_id: String,

    /// Chat type: "p2p" or "group".
    chat_type: String,

    /// Message type: "text", "image", "file", etc.
    message_type: String,

    /// Message content as JSON string.
    content: Option<String>,

    /// Mentions in the message.
    #[serde(default)]
    mentions: Option<Vec<FeishuMention>>,
}

/// Feishu mention in a message.
#[derive(Debug, Deserialize)]
struct FeishuMention {
    /// Mention key (e.g., "@_user_1").
    key: String,

    /// Mentioned entity ID.
    id: Option<FeishuMentionId>,

    /// Display name of the mentioned entity.
    name: Option<String>,

    /// Tenant key.
    tenant_key: Option<String>,
}

/// Feishu mention ID.
#[derive(Debug, Deserialize)]
struct FeishuMentionId {
    /// Open ID of the mentioned user.
    open_id: Option<String>,

    /// User ID of the mentioned user.
    user_id: Option<String>,
}

/// Feishu text content (parsed from content JSON string).
#[derive(Debug, Deserialize)]
struct FeishuTextContent {
    /// Text content.
    text: String,
}

/// Feishu image content (parsed from content JSON string).
#[derive(Debug, Deserialize)]
struct FeishuImageContent {
    /// Image key for downloading.
    image_key: String,
}

/// Feishu file content (parsed from content JSON string).
#[derive(Debug, Deserialize)]
struct FeishuFileContent {
    /// File key for downloading.
    file_key: String,

    /// File name.
    file_name: Option<String>,
}

/// Feishu API response wrapper for sendMessage.
#[derive(Debug, Deserialize)]
struct FeishuApiResponse {
    /// Status code (0 = success).
    code: i32,

    /// Status message.
    msg: String,

    /// Response data.
    data: Option<serde_json::Value>,
}

/// Feishu user info response.
#[derive(Debug, Deserialize)]
struct FeishuUserInfo {
    /// User's display name.
    name: Option<String>,
}

// ============================================================================
// Workspace State Paths
// ============================================================================

/// Workspace path for persisting owner_open_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_open_id";

/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";

/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";

/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "feishu";

/// Workspace path for persisting bot_name for mention detection in groups.
const BOT_NAME_PATH: &str = "state/bot_name";

/// Workspace path for persisting respond_to_all_group_messages flag.
const RESPOND_TO_ALL_GROUP_PATH: &str = "state/respond_to_all_group_messages";

/// Workspace path for deduplicating events.
const LAST_EVENT_ID_PATH: &str = "state/last_event_id";

/// Workspace path for persisting the app's own open_id (bot ID) for self-message filtering.
const BOT_OPEN_ID_PATH: &str = "state/bot_open_id";

// ============================================================================
// Feishu API Base URL
// ============================================================================

const FEISHU_API_BASE: &str = "https://open.feishu.cn/open-apis";

// ============================================================================
// Channel Metadata
// ============================================================================

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
struct FeishuMessageMetadata {
    /// Chat ID where the message was received.
    chat_id: String,

    /// Original message ID (for reply threading).
    message_id: String,

    /// Sender's open_id.
    open_id: String,

    /// Whether this is a P2P (DM) chat.
    is_p2p: bool,

    /// Root message ID for threading (if present).
    root_id: Option<String>,
}

/// Channel configuration injected by host.
#[derive(Debug, Deserialize)]
struct FeishuConfig {
    /// Bot display name for mention detection in groups.
    #[serde(default)]
    bot_name: Option<String>,

    /// Open ID of the bot owner. When set, only messages from this
    /// user are processed. All others are silently dropped.
    #[serde(default)]
    owner_open_id: Option<String>,

    /// DM policy: "pairing" (default), "allowlist", or "open".
    #[serde(default)]
    dm_policy: Option<String>,

    /// Allowed sender IDs from config (merged with pairing-approved store).
    #[serde(default)]
    allow_from: Option<Vec<String>>,

    /// Whether to respond to all group messages (not just mentions).
    #[serde(default)]
    respond_to_all_group_messages: bool,

    /// Public tunnel URL for webhook mode (injected by host from global settings).
    #[serde(default)]
    tunnel_url: Option<String>,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct FeishuChannel;

const FEISHU_STATUS_MAX_CHARS: usize = 600;

fn truncate_status_message(input: &str, max_chars: usize) -> String {
    let mut iter = input.chars();
    let truncated: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

fn status_message_for_user(update: &StatusUpdate) -> Option<String> {
    let message = update.message.trim();
    if message.is_empty() {
        None
    } else {
        Some(truncate_status_message(message, FEISHU_STATUS_MAX_CHARS))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FeishuStatusAction {
    /// Feishu has no native typing indicator, so we skip typing.
    Notify(String),
}

fn classify_status_update(update: &StatusUpdate) -> Option<FeishuStatusAction> {
    match update.status {
        // Feishu has no typing indicator API, so thinking is a no-op.
        StatusType::Thinking => None,
        StatusType::Done | StatusType::Interrupted => None,
        // Tool telemetry is too noisy for chat.
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
                status_message_for_user(update).map(FeishuStatusAction::Notify)
            }
        }
        StatusType::ApprovalNeeded
        | StatusType::JobStarted
        | StatusType::AuthRequired
        | StatusType::AuthCompleted => {
            status_message_for_user(update).map(FeishuStatusAction::Notify)
        }
    }
}

impl Guest for FeishuChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Feishu channel config: {}", config_json),
        );

        let config: FeishuConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Feishu channel starting");

        if let Some(ref name) = config.bot_name {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Bot name: {}", name),
            );
        }

        // Persist owner_open_id so subsequent callbacks can read it
        if let Some(ref owner_id) = config.owner_open_id {
            if let Err(e) = channel_host::workspace_write(OWNER_ID_PATH, owner_id) {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to persist owner_open_id: {}", e),
                );
            }
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: {}", owner_id),
            );
        } else {
            // Clear any stale owner_id from a previous config
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
            channel_host::log(
                channel_host::LogLevel::Warn,
                "No owner_open_id configured, bot is open to all users",
            );
        }

        // Persist dm_policy and allow_from for DM pairing in handle_message
        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing").to_string();
        let _ = channel_host::workspace_write(DM_POLICY_PATH, &dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // Persist bot_name and respond_to_all_group_messages for group handling
        let _ = channel_host::workspace_write(
            BOT_NAME_PATH,
            &config.bot_name.unwrap_or_default(),
        );
        let _ = channel_host::workspace_write(
            RESPOND_TO_ALL_GROUP_PATH,
            &config.respond_to_all_group_messages.to_string(),
        );

        // Feishu is webhook-only (event subscription), no polling mode
        channel_host::log(
            channel_host::LogLevel::Info,
            "Feishu channel uses webhook mode (event subscription)",
        );

        Ok(ChannelConfig {
            display_name: "Feishu".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/feishu".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false, // Feishu uses token in body, not header
            }],
            poll: None, // Feishu is webhook-only
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

        // Parse the event callback
        let callback: FeishuEventCallback = match serde_json::from_str(body_str) {
            Ok(c) => c,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse Feishu event: {}", e),
                );
                return json_response(200, serde_json::json!({"ok": true}));
            }
        };

        // Handle URL verification challenge
        if callback.request_type.as_deref() == Some("url_verification") {
            if let Some(challenge) = callback.challenge {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    "Handling Feishu URL verification challenge",
                );
                return json_response(200, serde_json::json!({"challenge": challenge}));
            }
        }

        // Handle event callback
        if let Some(ref header) = callback.header {
            // Deduplicate events using event_id
            let event_id = &header.event_id;
            let last_event_id = channel_host::workspace_read(LAST_EVENT_ID_PATH)
                .unwrap_or_default();

            if !last_event_id.is_empty() && last_event_id == *event_id {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Duplicate event {}, skipping", event_id),
                );
                return json_response(200, serde_json::json!({"ok": true}));
            }

            // Store this event_id for deduplication
            let _ = channel_host::workspace_write(LAST_EVENT_ID_PATH, event_id);

            // Route by event type
            match header.event_type.as_str() {
                "im.message.receive_v1" => {
                    if let Some(event) = callback.event {
                        handle_message_event(event);
                    }
                }
                other => {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("Unhandled event type: {}", other),
                    );
                }
            }
        }

        // Always respond 200 quickly (Feishu expects fast responses)
        json_response(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        // Feishu is webhook-only, polling is not used.
        channel_host::log(
            channel_host::LogLevel::Debug,
            "on_poll called but Feishu uses webhooks only",
        );
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: FeishuMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Reply in thread if there's a root_id, otherwise reply to the original message
        let reply_to = metadata.root_id.as_deref()
            .unwrap_or(&metadata.message_id);

        send_response(&metadata.chat_id, &response, Some(reply_to))
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        // user_id is the chat_id for Feishu
        send_response(&user_id, &response, None)
    }

    fn on_status(update: StatusUpdate) {
        let action = match classify_status_update(&update) {
            Some(action) => action,
            None => return,
        };

        // Parse chat_id from metadata
        let metadata: FeishuMessageMetadata = match serde_json::from_str(&update.metadata_json) {
            Ok(m) => m,
            Err(_) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    "on_status: no valid Feishu metadata, skipping status update",
                );
                return;
            }
        };

        match action {
            FeishuStatusAction::Notify(text) => {
                // Feishu has no typing indicator, send a text message for status updates
                if let Err(e) = send_message(&metadata.chat_id, &text, None) {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("Failed to send status message: {}", e),
                    );
                }
            }
        }
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Feishu channel shutting down",
        );
    }
}

// ============================================================================
// Send Message Helper
// ============================================================================

/// Send a text message to a Feishu chat.
///
/// Uses `POST /im/v1/messages?receive_id_type=chat_id` for new messages
/// or `POST /im/v1/messages/{message_id}/reply` for replies.
fn send_message(
    chat_id: &str,
    text: &str,
    reply_to_message_id: Option<&str>,
) -> Result<String, String> {
    let content = serde_json::json!({"text": text}).to_string();

    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    let (url, payload) = if let Some(msg_id) = reply_to_message_id {
        // Reply to a specific message (thread reply)
        let url = format!("{}/im/v1/messages/{}/reply", FEISHU_API_BASE, msg_id);
        let payload = serde_json::json!({
            "msg_type": "text",
            "content": content,
        });
        (url, payload)
    } else {
        // Send a new message to the chat
        let url = format!(
            "{}/im/v1/messages?receive_id_type=chat_id",
            FEISHU_API_BASE
        );
        let payload = serde_json::json!({
            "receive_id": chat_id,
            "msg_type": "text",
            "content": content,
        });
        (url, payload)
    };

    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| format!("Failed to serialize payload: {}", e))?;

    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(http_response) => {
            if http_response.status != 200 {
                let body_str = String::from_utf8_lossy(&http_response.body);
                return Err(format!(
                    "Feishu API returned status {}: {}",
                    http_response.status, body_str
                ));
            }

            let api_response: FeishuApiResponse = serde_json::from_slice(&http_response.body)
                .map_err(|e| format!("Failed to parse response: {}", e))?;

            if api_response.code != 0 {
                return Err(format!(
                    "Feishu API error (code {}): {}",
                    api_response.code, api_response.msg
                ));
            }

            // Extract message_id from response
            let message_id = api_response
                .data
                .and_then(|d| d.get("message_id").and_then(|v| v.as_str()).map(String::from))
                .unwrap_or_default();

            Ok(message_id)
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

// ============================================================================
// Attachment Sending (Image / File)
// ============================================================================

/// Maximum file size for Feishu image upload (10 MB).
const MAX_IMAGE_SIZE: usize = 10 * 1024 * 1024;

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

/// Upload an image to Feishu and return the image_key.
fn upload_image(data: &[u8], filename: &str, mime_type: &str) -> Result<String, String> {
    let boundary = format!("ironclaw-{}", channel_host::now_millis());
    let mut body = Vec::new();

    write_multipart_field(&mut body, &boundary, "image_type", "message");
    write_multipart_file(&mut body, &boundary, "image", filename, mime_type, data);
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

    let headers = serde_json::json!({
        "Content-Type": format!("multipart/form-data; boundary={}", boundary)
    });

    let result = channel_host::http_request(
        "POST",
        &format!("{}/im/v1/images", FEISHU_API_BASE),
        &headers.to_string(),
        Some(&body),
        Some(60_000),
    );

    match result {
        Ok(resp) if resp.status == 200 => {
            let api_response: FeishuApiResponse = serde_json::from_slice(&resp.body)
                .map_err(|e| format!("Failed to parse upload response: {}", e))?;

            if api_response.code != 0 {
                return Err(format!(
                    "Image upload failed (code {}): {}",
                    api_response.code, api_response.msg
                ));
            }

            api_response
                .data
                .and_then(|d| d.get("image_key").and_then(|v| v.as_str()).map(String::from))
                .ok_or_else(|| "No image_key in upload response".to_string())
        }
        Ok(resp) => {
            let body_str = String::from_utf8_lossy(&resp.body);
            Err(format!(
                "Image upload failed (HTTP {}): {}",
                resp.status, body_str
            ))
        }
        Err(e) => Err(format!("Image upload HTTP request failed: {}", e)),
    }
}

/// Upload a file to Feishu and return the file_key.
fn upload_file(data: &[u8], filename: &str, mime_type: &str) -> Result<String, String> {
    let boundary = format!("ironclaw-{}", channel_host::now_millis());
    let mut body = Vec::new();

    write_multipart_field(&mut body, &boundary, "file_type", "stream");
    write_multipart_field(&mut body, &boundary, "file_name", filename);
    write_multipart_file(&mut body, &boundary, "file", filename, mime_type, data);
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

    let headers = serde_json::json!({
        "Content-Type": format!("multipart/form-data; boundary={}", boundary)
    });

    let result = channel_host::http_request(
        "POST",
        &format!("{}/im/v1/files", FEISHU_API_BASE),
        &headers.to_string(),
        Some(&body),
        Some(60_000),
    );

    match result {
        Ok(resp) if resp.status == 200 => {
            let api_response: FeishuApiResponse = serde_json::from_slice(&resp.body)
                .map_err(|e| format!("Failed to parse upload response: {}", e))?;

            if api_response.code != 0 {
                return Err(format!(
                    "File upload failed (code {}): {}",
                    api_response.code, api_response.msg
                ));
            }

            api_response
                .data
                .and_then(|d| d.get("file_key").and_then(|v| v.as_str()).map(String::from))
                .ok_or_else(|| "No file_key in upload response".to_string())
        }
        Ok(resp) => {
            let body_str = String::from_utf8_lossy(&resp.body);
            Err(format!(
                "File upload failed (HTTP {}): {}",
                resp.status, body_str
            ))
        }
        Err(e) => Err(format!("File upload HTTP request failed: {}", e)),
    }
}

/// Image MIME types that can be sent via Feishu's image message type.
const IMAGE_MIME_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/bmp",
];

/// Send a single attachment to a Feishu chat.
fn send_attachment(
    chat_id: &str,
    attachment: &Attachment,
    reply_to_message_id: Option<&str>,
) -> Result<(), String> {
    if IMAGE_MIME_TYPES.contains(&attachment.mime_type.as_str())
        && attachment.data.len() <= MAX_IMAGE_SIZE
    {
        // Upload as image and send image message
        let image_key = upload_image(&attachment.data, &attachment.filename, &attachment.mime_type)?;

        let content = serde_json::json!({"image_key": image_key}).to_string();
        let headers = serde_json::json!({"Content-Type": "application/json"});

        let (url, payload) = if let Some(msg_id) = reply_to_message_id {
            let url = format!("{}/im/v1/messages/{}/reply", FEISHU_API_BASE, msg_id);
            let payload = serde_json::json!({
                "msg_type": "image",
                "content": content,
            });
            (url, payload)
        } else {
            let url = format!(
                "{}/im/v1/messages?receive_id_type=chat_id",
                FEISHU_API_BASE
            );
            let payload = serde_json::json!({
                "receive_id": chat_id,
                "msg_type": "image",
                "content": content,
            });
            (url, payload)
        };

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| format!("Failed to serialize payload: {}", e))?;

        let result = channel_host::http_request(
            "POST",
            &url,
            &headers.to_string(),
            Some(&payload_bytes),
            None,
        );

        match result {
            Ok(resp) if resp.status == 200 => Ok(()),
            Ok(resp) => {
                let body_str = String::from_utf8_lossy(&resp.body);
                Err(format!(
                    "Send image failed (HTTP {}): {}",
                    resp.status, body_str
                ))
            }
            Err(e) => Err(format!("Send image HTTP request failed: {}", e)),
        }
    } else {
        // Upload as file and send file message
        let file_key = upload_file(&attachment.data, &attachment.filename, &attachment.mime_type)?;

        let content = serde_json::json!({"file_key": file_key}).to_string();
        let headers = serde_json::json!({"Content-Type": "application/json"});

        let (url, payload) = if let Some(msg_id) = reply_to_message_id {
            let url = format!("{}/im/v1/messages/{}/reply", FEISHU_API_BASE, msg_id);
            let payload = serde_json::json!({
                "msg_type": "file",
                "content": content,
            });
            (url, payload)
        } else {
            let url = format!(
                "{}/im/v1/messages?receive_id_type=chat_id",
                FEISHU_API_BASE
            );
            let payload = serde_json::json!({
                "receive_id": chat_id,
                "msg_type": "file",
                "content": content,
            });
            (url, payload)
        };

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| format!("Failed to serialize payload: {}", e))?;

        let result = channel_host::http_request(
            "POST",
            &url,
            &headers.to_string(),
            Some(&payload_bytes),
            None,
        );

        match result {
            Ok(resp) if resp.status == 200 => Ok(()),
            Ok(resp) => {
                let body_str = String::from_utf8_lossy(&resp.body);
                Err(format!(
                    "Send file failed (HTTP {}): {}",
                    resp.status, body_str
                ))
            }
            Err(e) => Err(format!("Send file HTTP request failed: {}", e)),
        }
    }
}

/// Send a full agent response (attachments + text) to a chat.
///
/// Shared implementation for both `on_respond` and `on_broadcast`.
fn send_response(
    chat_id: &str,
    response: &AgentResponse,
    reply_to_message_id: Option<&str>,
) -> Result<(), String> {
    // Send attachments first
    for attachment in &response.attachments {
        send_attachment(chat_id, attachment, reply_to_message_id)?;
    }

    // Skip text if empty and we already sent attachments
    if response.content.is_empty() && !response.attachments.is_empty() {
        return Ok(());
    }

    send_message(chat_id, &response.content, reply_to_message_id)
        .map(|_| ())
}

// ============================================================================
// Pairing Reply
// ============================================================================

/// Send a pairing code message to a chat. Used when an unknown user DMs the bot.
fn send_pairing_reply(chat_id: &str, code: &str) -> Result<(), String> {
    send_message(
        chat_id,
        &format!(
            "To pair with this bot, run: ironclaw pairing approve feishu {}",
            code
        ),
        None,
    )
    .map(|_| ())
}

// ============================================================================
// Event Handling
// ============================================================================

/// Process a Feishu im.message.receive_v1 event.
fn handle_message_event(event: serde_json::Value) {
    // Parse the event payload
    let msg_event: FeishuMessageEvent = match serde_json::from_value(event) {
        Ok(e) => e,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to parse message event: {}", e),
            );
            return;
        }
    };

    let sender = &msg_event.sender;
    let message = &msg_event.message;

    // Skip messages from bots/apps to avoid loops
    if sender.sender_type.as_deref() == Some("app") {
        channel_host::log(
            channel_host::LogLevel::Debug,
            "Skipping message from app/bot",
        );
        return;
    }

    // Get sender's open_id
    let open_id = match sender.sender_id.open_id.as_deref() {
        Some(id) => id.to_string(),
        None => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Message sender has no open_id, skipping",
            );
            return;
        }
    };

    // Skip messages from the bot itself
    let bot_open_id = channel_host::workspace_read(BOT_OPEN_ID_PATH).unwrap_or_default();
    if !bot_open_id.is_empty() && bot_open_id == open_id {
        channel_host::log(
            channel_host::LogLevel::Debug,
            "Skipping message from self (bot)",
        );
        return;
    }

    let is_p2p = message.chat_type == "p2p";

    // Owner validation: when owner_open_id is set, only that user can message
    let owner_id_str = channel_host::workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());

    if let Some(ref owner_id) = owner_id_str {
        if open_id != *owner_id {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!(
                    "Dropping message from non-owner user {} (owner: {})",
                    open_id, owner_id
                ),
            );
            return;
        }
    } else {
        // No owner_id: apply authorization based on dm_policy and allow_from
        let dm_policy =
            channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

        if dm_policy != "open" {
            // Build effective allow list: config allow_from + pairing store
            let mut allowed: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            if let Ok(store_allowed) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
                allowed.extend(store_allowed);
            }

            let is_allowed = allowed.contains(&"*".to_string())
                || allowed.contains(&open_id);

            if !is_allowed {
                if is_p2p && dm_policy == "pairing" {
                    // Upsert pairing request and send reply (only for P2P chats)
                    let meta = serde_json::json!({
                        "chat_id": message.chat_id,
                        "open_id": open_id,
                    })
                    .to_string();

                    match channel_host::pairing_upsert_request(CHANNEL_NAME, &open_id, &meta) {
                        Ok(result) => {
                            channel_host::log(
                                channel_host::LogLevel::Info,
                                &format!(
                                    "Pairing request for user {} (chat {}): code {}",
                                    open_id, message.chat_id, result.code
                                ),
                            );
                            if result.created {
                                let _ = send_pairing_reply(&message.chat_id, &result.code);
                            }
                        }
                        Err(e) => {
                            channel_host::log(
                                channel_host::LogLevel::Error,
                                &format!("Pairing upsert failed: {}", e),
                            );
                        }
                    }
                } else if !is_p2p {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!(
                            "Dropping message from unauthorized user {} in group chat",
                            open_id
                        ),
                    );
                }
                return;
            }
        }
    }

    // Parse message content based on message_type
    let (content, attachments) = parse_message_content(message);

    // Allow messages with attachments even if text content is empty
    if content.is_empty() && attachments.is_empty() {
        return;
    }

    // For group chats, only respond if bot was mentioned or respond_to_all is enabled
    if !is_p2p {
        let respond_to_all = channel_host::workspace_read(RESPOND_TO_ALL_GROUP_PATH)
            .as_deref()
            .unwrap_or("false")
            == "true";

        if !respond_to_all {
            let has_bot_mention = check_bot_mention(message);

            if !has_bot_mention {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Ignoring group message without mention: {}", content),
                );
                return;
            }
        }
    }

    // Build user display name from open_id (Feishu events don't include user name directly)
    let user_name = sender
        .sender_id
        .user_id
        .clone()
        .unwrap_or_else(|| open_id.clone());

    // Build metadata for response routing
    let metadata = FeishuMessageMetadata {
        chat_id: message.chat_id.clone(),
        message_id: message.message_id.clone(),
        open_id: open_id.clone(),
        is_p2p,
        root_id: message.root_id.clone(),
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Clean mention tags from content before emitting
    let content_to_emit = clean_message_text(&content);

    // Allow attachment-only messages even without text
    if content_to_emit.is_empty() && attachments.is_empty() {
        return;
    }

    // Emit the message to the agent
    channel_host::emit_message(&EmittedMessage {
        user_id: open_id.clone(),
        user_name: Some(user_name),
        content: content_to_emit,
        thread_id: message.root_id.clone(),
        metadata_json,
        attachments,
    });

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "Emitted message from user {} in chat {}",
            open_id, message.chat_id
        ),
    );
}

/// Check if the bot was mentioned in a message.
fn check_bot_mention(message: &FeishuMessage) -> bool {
    if let Some(ref mentions) = message.mentions {
        let bot_name = channel_host::workspace_read(BOT_NAME_PATH).unwrap_or_default();
        let bot_open_id = channel_host::workspace_read(BOT_OPEN_ID_PATH).unwrap_or_default();

        for mention in mentions {
            // Check if the mention name matches the bot name
            if !bot_name.is_empty() {
                if let Some(ref name) = mention.name {
                    if name.eq_ignore_ascii_case(&bot_name) {
                        return true;
                    }
                }
            }

            // Check if the mentioned open_id matches the bot's open_id
            if !bot_open_id.is_empty() {
                if let Some(ref id) = mention.id {
                    if let Some(ref mention_open_id) = id.open_id {
                        if mention_open_id == &bot_open_id {
                            return true;
                        }
                    }
                }
            }

            // If we don't know bot identity, any mention triggers (conservative)
            if bot_name.is_empty() && bot_open_id.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Parse message content based on message_type.
///
/// Returns (text_content, attachments).
fn parse_message_content(message: &FeishuMessage) -> (String, Vec<InboundAttachment>) {
    let mut attachments = Vec::new();

    let content_str = match message.content.as_deref() {
        Some(s) => s,
        None => return (String::new(), attachments),
    };

    match message.message_type.as_str() {
        "text" => {
            let text = match serde_json::from_str::<FeishuTextContent>(content_str) {
                Ok(tc) => tc.text,
                Err(_) => content_str.to_string(),
            };
            (text, attachments)
        }
        "image" => {
            if let Ok(img) = serde_json::from_str::<FeishuImageContent>(content_str) {
                let download_url = format!(
                    "{}/im/v1/images/{}",
                    FEISHU_API_BASE, img.image_key
                );

                attachments.push(InboundAttachment {
                    id: img.image_key.clone(),
                    mime_type: "image/jpeg".to_string(), // Feishu doesn't specify, assume JPEG
                    filename: Some(format!("{}.jpg", img.image_key)),
                    size_bytes: None,
                    source_url: Some(download_url),
                    storage_key: None,
                    extracted_text: None,
                    extras_json: String::new(),
                });

                // Download and store the image
                download_and_store_feishu_image(&img.image_key);
            }
            ("[Image]".to_string(), attachments)
        }
        "file" => {
            if let Ok(file) = serde_json::from_str::<FeishuFileContent>(content_str) {
                let download_url = format!(
                    "{}/im/v1/messages/{}/resources/{}?type=file",
                    FEISHU_API_BASE, message.message_id, file.file_key
                );
                let filename = file.file_name.clone().unwrap_or_else(|| format!("{}", file.file_key));

                attachments.push(InboundAttachment {
                    id: file.file_key.clone(),
                    mime_type: "application/octet-stream".to_string(),
                    filename: Some(filename.clone()),
                    size_bytes: None,
                    source_url: Some(download_url),
                    storage_key: None,
                    extracted_text: None,
                    extras_json: String::new(),
                });

                // Download and store the file
                download_and_store_feishu_file(&message.message_id, &file.file_key);
            }
            ("[File]".to_string(), attachments)
        }
        other => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Unsupported message type: {}", other),
            );
            (format!("[Unsupported message type: {}]", other), attachments)
        }
    }
}

/// Download a Feishu image and store it via the host.
fn download_and_store_feishu_image(image_key: &str) {
    let url = format!("{}/im/v1/images/{}", FEISHU_API_BASE, image_key);
    let headers = serde_json::json!({});

    match channel_host::http_request("GET", &url, &headers.to_string(), None, None) {
        Ok(response) if response.status == 200 => {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Downloaded image: {} bytes", response.body.len()),
            );
            if let Err(e) = channel_host::store_attachment_data(image_key, &response.body) {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to store image data: {}", e),
                );
            }
        }
        Ok(response) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Image download returned status {}", response.status),
            );
        }
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Image download failed: {}", e),
            );
        }
    }
}

/// Download a Feishu file and store it via the host.
fn download_and_store_feishu_file(message_id: &str, file_key: &str) {
    let url = format!(
        "{}/im/v1/messages/{}/resources/{}?type=file",
        FEISHU_API_BASE, message_id, file_key
    );
    let headers = serde_json::json!({});

    match channel_host::http_request("GET", &url, &headers.to_string(), None, None) {
        Ok(response) if response.status == 200 => {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Downloaded file: {} bytes", response.body.len()),
            );
            if let Err(e) = channel_host::store_attachment_data(file_key, &response.body) {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to store file data: {}", e),
                );
            }
        }
        Ok(response) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("File download returned status {}", response.status),
            );
        }
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("File download failed: {}", e),
            );
        }
    }
}

/// Clean message text by removing Feishu @mention tags.
///
/// Feishu represents mentions as `@_user_N` in the text body.
/// We strip these mention placeholders to get clean text for the agent.
fn clean_message_text(text: &str) -> String {
    let mut result = text.trim().to_string();

    // Remove @_user_N mention placeholders (Feishu format)
    // These look like "@_user_1" in the text
    while let Some(start) = result.find("@_user_") {
        let rest = &result[start + 7..]; // skip "@_user_"
        // Find the end of the mention key (digits only)
        let end_offset = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let end = start + 7 + end_offset;
        // Remove the mention and any trailing space
        let after = if end < result.len() && result.as_bytes()[end] == b' ' {
            end + 1
        } else {
            end
        };
        result = format!("{}{}", &result[..start], &result[after..]);
    }

    result.trim().to_string()
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
export!(FeishuChannel);

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_message_text() {
        assert_eq!(clean_message_text("@_user_1 hello world"), "hello world");
        assert_eq!(clean_message_text("hello @_user_1 world"), "hello world");
        assert_eq!(clean_message_text("@_user_1"), "");
        assert_eq!(clean_message_text("just text"), "just text");
        assert_eq!(clean_message_text("  spaced  "), "spaced");
        assert_eq!(
            clean_message_text("@_user_1 @_user_2 hello"),
            "hello"
        );
    }

    #[test]
    fn test_clean_message_text_no_mention() {
        assert_eq!(clean_message_text("normal message"), "normal message");
        assert_eq!(clean_message_text(""), "");
    }

    #[test]
    fn test_parse_text_content() {
        let content_json = r#"{"text":"hello world"}"#;
        let tc: FeishuTextContent = serde_json::from_str(content_json).unwrap();
        assert_eq!(tc.text, "hello world");
    }

    #[test]
    fn test_parse_image_content() {
        let content_json = r#"{"image_key":"img_v2_xxx"}"#;
        let ic: FeishuImageContent = serde_json::from_str(content_json).unwrap();
        assert_eq!(ic.image_key, "img_v2_xxx");
    }

    #[test]
    fn test_parse_file_content() {
        let content_json = r#"{"file_key":"file_v2_xxx","file_name":"doc.pdf"}"#;
        let fc: FeishuFileContent = serde_json::from_str(content_json).unwrap();
        assert_eq!(fc.file_key, "file_v2_xxx");
        assert_eq!(fc.file_name, Some("doc.pdf".to_string()));
    }

    #[test]
    fn test_parse_event_callback_challenge() {
        let json = r#"{"challenge":"abc123","token":"verify_token","type":"url_verification"}"#;
        let cb: FeishuEventCallback = serde_json::from_str(json).unwrap();
        assert_eq!(cb.request_type.as_deref(), Some("url_verification"));
        assert_eq!(cb.challenge.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_parse_event_callback_message() {
        let json = r#"{
            "schema": "2.0",
            "header": {
                "event_id": "evt_001",
                "event_type": "im.message.receive_v1",
                "create_time": "1234567890",
                "token": "verify_token",
                "app_id": "cli_xxx",
                "tenant_key": "tenant_xxx"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_xxx",
                        "user_id": "user_xxx"
                    },
                    "sender_type": "user",
                    "tenant_key": "tenant_xxx"
                },
                "message": {
                    "message_id": "om_xxx",
                    "chat_id": "oc_xxx",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"hello\"}"
                }
            }
        }"#;

        let cb: FeishuEventCallback = serde_json::from_str(json).unwrap();
        assert_eq!(cb.header.as_ref().unwrap().event_type, "im.message.receive_v1");

        let event: FeishuMessageEvent =
            serde_json::from_value(cb.event.unwrap()).unwrap();
        assert_eq!(event.sender.sender_id.open_id.as_deref(), Some("ou_xxx"));
        assert_eq!(event.message.chat_type, "p2p");
        assert_eq!(event.message.message_type, "text");

        let text: FeishuTextContent =
            serde_json::from_str(event.message.content.as_deref().unwrap()).unwrap();
        assert_eq!(text.text, "hello");
    }

    #[test]
    fn test_truncate_status_message() {
        let short = "hello";
        assert_eq!(truncate_status_message(short, 600), "hello");

        let long = "a".repeat(700);
        let truncated = truncate_status_message(&long, 600);
        assert!(truncated.ends_with("..."));
        // 600 chars + "..."
        assert_eq!(truncated.len(), 603);
    }

    #[test]
    fn test_feishu_metadata_roundtrip() {
        let meta = FeishuMessageMetadata {
            chat_id: "oc_xxx".to_string(),
            message_id: "om_xxx".to_string(),
            open_id: "ou_xxx".to_string(),
            is_p2p: true,
            root_id: Some("om_root".to_string()),
        };

        let json = serde_json::to_string(&meta).unwrap();
        let parsed: FeishuMessageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.chat_id, "oc_xxx");
        assert_eq!(parsed.message_id, "om_xxx");
        assert_eq!(parsed.open_id, "ou_xxx");
        assert!(parsed.is_p2p);
        assert_eq!(parsed.root_id.as_deref(), Some("om_root"));
    }
}
