// Feishu API types have fields reserved for future use.
#![allow(dead_code)]

//! Feishu/Lark Bot channel for IronClaw.
//!
//! This WASM component implements the channel interface for handling Feishu
//! webhooks (Event Subscription v2.0) and sending messages back via the
//! Feishu/Lark Bot API.
//!
//! # Features
//!
//! - Webhook-based message receiving (Event Subscription v2.0)
//! - URL verification challenge handling
//! - Private chat (DM) support
//! - Group chat support with @mention triggering
//! - Tenant access token management (app_id + app_secret exchange)
//! - Supports both Feishu (open.feishu.cn) and Lark (open.larksuite.com)
//!
//! # Security
//!
//! - App credentials (app_id, app_secret) are injected by the host into
//!   the config JSON during startup for token exchange
//! - Bearer token for API calls is obtained via token exchange and cached
//! - Verification token validated by host for webhook requests

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, Attachment, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

// ============================================================================
// Workspace paths for cross-callback state
// ============================================================================

const OWNER_ID_PATH: &str = "owner_id";
const DM_POLICY_PATH: &str = "dm_policy";
const ALLOW_FROM_PATH: &str = "allow_from";
const API_BASE_PATH: &str = "api_base";
const APP_ID_PATH: &str = "app_id";
const APP_SECRET_PATH: &str = "app_secret";
const TOKEN_PATH: &str = "tenant_access_token";
const TOKEN_EXPIRY_PATH: &str = "token_expiry";

// ============================================================================
// Feishu API Types
// ============================================================================

/// Feishu Event Subscription v2.0 envelope.
/// https://open.feishu.cn/document/server-docs/event-subscription-guide/event-subscription-configure-/request-url-configuration-case
#[derive(Debug, Deserialize)]
struct FeishuEvent {
    /// Schema version (always "2.0" for v2 events).
    #[serde(default)]
    schema: Option<String>,

    /// Event header with metadata.
    header: Option<FeishuEventHeader>,

    /// Event payload (varies by event type).
    event: Option<serde_json::Value>,

    /// URL verification challenge (only for initial setup).
    challenge: Option<String>,

    /// Token for URL verification (only for initial setup).
    token: Option<String>,

    /// Type field for URL verification ("url_verification").
    #[serde(rename = "type")]
    event_type: Option<String>,
}

/// Event header containing metadata.
#[derive(Debug, Deserialize)]
struct FeishuEventHeader {
    /// Unique event ID.
    event_id: String,

    /// Event type (e.g., "im.message.receive_v1").
    event_type: String,

    /// Timestamp.
    #[serde(default)]
    create_time: Option<String>,

    /// App ID.
    #[serde(default)]
    app_id: Option<String>,

    /// Tenant key.
    #[serde(default)]
    tenant_key: Option<String>,
}

/// Message receive event payload (im.message.receive_v1).
#[derive(Debug, Deserialize)]
struct MessageReceiveEvent {
    sender: FeishuSender,
    message: FeishuMessage,
}

/// Sender information.
#[derive(Debug, Deserialize)]
struct FeishuSender {
    sender_id: FeishuSenderId,
    #[serde(default)]
    sender_type: Option<String>,
    #[serde(default)]
    tenant_key: Option<String>,
}

/// Sender ID with multiple ID types.
#[derive(Debug, Deserialize)]
struct FeishuSenderId {
    #[serde(default)]
    open_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    union_id: Option<String>,
}

/// Message content.
#[derive(Debug, Deserialize)]
struct FeishuMessage {
    /// Unique message ID.
    message_id: String,

    /// Parent message ID (for thread replies).
    #[serde(default)]
    parent_id: Option<String>,

    /// Root message ID (for thread root).
    #[serde(default)]
    root_id: Option<String>,

    /// Chat ID the message belongs to.
    chat_id: String,

    /// Chat type: "p2p" (DM) or "group".
    #[serde(default)]
    chat_type: Option<String>,

    /// Message type: "text", "image", "post", etc.
    message_type: String,

    /// JSON-encoded content.
    content: String,

    /// Mentions in the message.
    #[serde(default)]
    mentions: Option<Vec<FeishuMention>>,
}

/// Mention in a message.
#[derive(Debug, Deserialize)]
struct FeishuMention {
    key: String,
    id: FeishuMentionId,
    name: String,
    #[serde(default)]
    tenant_key: Option<String>,
}

/// Mention ID.
#[derive(Debug, Deserialize)]
struct FeishuMentionId {
    #[serde(default)]
    open_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    union_id: Option<String>,
}

/// Text message content (when message_type == "text").
#[derive(Debug, Deserialize)]
struct TextContent {
    text: String,
}

/// Metadata stored for responding to messages.
#[derive(Debug, Serialize, Deserialize)]
struct FeishuMessageMetadata {
    chat_id: String,
    message_id: String,
    chat_type: String,
}

/// Feishu API response wrapper.
#[derive(Debug, Deserialize)]
struct FeishuApiResponse<T> {
    code: i32,
    msg: String,
    #[serde(default)]
    data: Option<T>,
}

/// Tenant access token response.
#[derive(Debug, Deserialize)]
struct TenantAccessTokenData {
    tenant_access_token: String,
    expire: i64,
}

/// Send message request body.
#[derive(Debug, Serialize)]
struct SendMessageBody {
    receive_id: String,
    msg_type: String,
    content: String,
}

/// Reply message request body.
#[derive(Debug, Serialize)]
struct ReplyMessageBody {
    msg_type: String,
    content: String,
}

// ============================================================================
// Configuration
// ============================================================================

/// Channel configuration parsed from capabilities.json `config` section.
#[derive(Debug, Deserialize)]
struct FeishuConfig {
    /// Feishu App ID (for token exchange).
    app_id: Option<String>,

    /// Feishu App Secret (for token exchange).
    app_secret: Option<String>,

    /// API base URL. Defaults to "https://open.feishu.cn" (use
    /// "https://open.larksuite.com" for Lark international).
    #[serde(default = "default_api_base")]
    api_base: String,

    /// Restrict to a single owner (open_id). If set, messages from other
    /// users are silently ignored.
    owner_id: Option<String>,

    /// DM pairing policy: "open" or "pairing" (default).
    dm_policy: Option<String>,

    /// Allowed user IDs (open_id) for DM pairing.
    #[serde(default)]
    allow_from: Option<Vec<String>>,
}

fn default_api_base() -> String {
    "https://open.feishu.cn".to_string()
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct FeishuChannel;

export_sandboxed_channel!(FeishuChannel);

impl Guest for FeishuChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: FeishuConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Feishu channel starting");

        // Persist config for cross-callback access.
        let api_base = config.api_base.trim_end_matches('/').to_string();
        let _ = channel_host::workspace_write(API_BASE_PATH, &api_base);

        // Persist app credentials for token exchange in later callbacks.
        // These are injected by the host from the secrets store into the
        // config JSON (see setup.rs inject_channel_secrets_into_config).
        if let Some(ref app_id) = config.app_id {
            let _ = channel_host::workspace_write(APP_ID_PATH, app_id);
        }
        if let Some(ref app_secret) = config.app_secret {
            let _ = channel_host::workspace_write(APP_SECRET_PATH, app_secret);
        }

        if let Some(owner_id) = &config.owner_id {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, owner_id);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: user {}", owner_id),
            );
        } else {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
        }

        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing").to_string();
        let _ = channel_host::workspace_write(DM_POLICY_PATH, &dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // Obtain initial tenant access token if credentials are available.
        let has_credentials = config.app_id.is_some() && config.app_secret.is_some();
        if has_credentials {
            match obtain_tenant_token(&api_base) {
                Ok(_) => {
                    channel_host::log(
                        channel_host::LogLevel::Info,
                        "Tenant access token obtained successfully",
                    );
                }
                Err(e) => {
                    // Non-fatal: token will be obtained on first message send.
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Failed to obtain initial token (will retry): {}", e),
                    );
                }
            }
        } else {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "No app credentials in config; outbound messaging will fail \
                 unless feishu_app_id and feishu_app_secret are injected by the host",
            );
        }

        Ok(ChannelConfig {
            display_name: "Feishu".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/feishu".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false,
            }],
            poll: None,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Parse the request body as UTF-8.
        let body_str = match std::str::from_utf8(&req.body) {
            Ok(s) => s,
            Err(_) => {
                return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
            }
        };

        // Parse as Feishu event envelope.
        let event: FeishuEvent = match serde_json::from_str(body_str) {
            Ok(e) => e,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse Feishu event: {}", e),
                );
                return json_response(200, serde_json::json!({}));
            }
        };

        // Handle URL verification challenge (initial webhook setup).
        if event.event_type.as_deref() == Some("url_verification") {
            if let Some(challenge) = &event.challenge {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    "Handling URL verification challenge",
                );
                return json_response(
                    200,
                    serde_json::json!({ "challenge": challenge }),
                );
            }
        }

        // Handle v2.0 events.
        if let Some(header) = &event.header {
            match header.event_type.as_str() {
                "im.message.receive_v1" => {
                    if let Some(event_data) = &event.event {
                        handle_message_event(event_data);
                    }
                }
                other => {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("Ignoring event type: {}", other),
                    );
                }
            }
        }

        // Always respond 200 quickly (Feishu expects fast responses).
        json_response(200, serde_json::json!({}))
    }

    fn on_poll() {
        // Feishu uses webhooks, not polling.
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: FeishuMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        send_reply(&metadata.message_id, &response.content)
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        send_message(&user_id, "open_id", &response.content)
    }

    fn on_status(_update: StatusUpdate) {
        // Status updates (thinking, tool execution, etc.) are not forwarded
        // to Feishu in this initial implementation.
    }

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Feishu channel shutting down");
    }
}

// ============================================================================
// Message Handling
// ============================================================================

/// Handle an im.message.receive_v1 event.
fn handle_message_event(event_data: &serde_json::Value) {
    let msg_event: MessageReceiveEvent = match serde_json::from_value(event_data.clone()) {
        Ok(e) => e,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to parse message event: {}", e),
            );
            return;
        }
    };

    let sender_id = msg_event
        .sender
        .sender_id
        .open_id
        .as_deref()
        .unwrap_or("unknown");

    // Owner restriction check.
    if let Some(owner_id) = channel_host::workspace_read(OWNER_ID_PATH) {
        if !owner_id.is_empty() && sender_id != owner_id {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Ignoring message from non-owner: {}", sender_id),
            );
            return;
        }
    }

    // allow_from restriction: if configured, only listed user IDs may interact.
    if let Some(allow_from_json) = channel_host::workspace_read(ALLOW_FROM_PATH) {
        if let Ok(allow_list) = serde_json::from_str::<Vec<String>>(&allow_from_json) {
            if !allow_list.is_empty() && !allow_list.iter().any(|id| id == sender_id) {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Ignoring message from user not in allow_from: {}", sender_id),
                );
                return;
            }
        }
    }

    // DM pairing check for p2p chats.
    let chat_type = msg_event
        .message
        .chat_type
        .as_deref()
        .unwrap_or("unknown");

    if chat_type == "p2p" {
        let dm_policy = channel_host::workspace_read(DM_POLICY_PATH)
            .unwrap_or_else(|| "pairing".to_string());

        if dm_policy == "pairing" {
            let sender_name = sender_id.to_string();
            match channel_host::pairing_is_allowed("feishu", sender_id, &sender_name) {
                Ok(true) => {}
                Ok(false) => {
                    // Upsert a pairing request.
                    let meta = serde_json::json!({
                        "sender_id": sender_id,
                        "chat_id": msg_event.message.chat_id,
                        "chat_type": chat_type,
                    });
                    let _ = channel_host::pairing_upsert_request(
                        "feishu",
                        sender_id,
                        &meta.to_string(),
                    );
                    channel_host::log(
                        channel_host::LogLevel::Info,
                        &format!("Pairing request created for {}", sender_id),
                    );
                    return;
                }
                Err(e) => {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Pairing check failed: {}", e),
                    );
                    return;
                }
            }
        }
    }

    // Extract text content.
    let text = extract_text_content(&msg_event.message);
    if text.is_empty() {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Ignoring non-text message type: {}",
                msg_event.message.message_type
            ),
        );
        return;
    }

    // Build metadata for responding.
    let metadata = FeishuMessageMetadata {
        chat_id: msg_event.message.chat_id.clone(),
        message_id: msg_event.message.message_id.clone(),
        chat_type: chat_type.to_string(),
    };

    let metadata_json =
        serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Determine thread ID from reply chain.
    let thread_id = msg_event
        .message
        .root_id
        .as_deref()
        .or(msg_event.message.parent_id.as_deref())
        .map(|s| s.to_string());

    // Emit message to the agent.
    channel_host::emit_message(EmittedMessage {
        user_id: sender_id.to_string(),
        user_name: None,
        content: text,
        thread_id,
        metadata_json,
        attachments: vec![],
    });
}

/// Extract text content from a Feishu message.
///
/// Currently handles "text" message type. Other types (image, post, file,
/// etc.) are logged and skipped.
fn extract_text_content(message: &FeishuMessage) -> String {
    match message.message_type.as_str() {
        "text" => {
            // Content is JSON: {"text": "hello"}
            match serde_json::from_str::<TextContent>(&message.content) {
                Ok(tc) => {
                    let mut text = tc.text;
                    // Strip @mention placeholders like @_user_1.
                    if let Some(mentions) = &message.mentions {
                        for mention in mentions {
                            text = text.replace(&mention.key, &mention.name);
                        }
                    }
                    text.trim().to_string()
                }
                Err(_) => String::new(),
            }
        }
        _ => String::new(),
    }
}

// ============================================================================
// Outbound Messaging
// ============================================================================

/// Reply to a specific message.
fn send_reply(message_id: &str, content: &str) -> Result<(), String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH)
        .unwrap_or_else(|| "https://open.feishu.cn".to_string());

    let token = get_valid_token(&api_base)?;

    let url = format!(
        "{}/open-apis/im/v1/messages/{}/reply",
        api_base, message_id
    );

    let body = ReplyMessageBody {
        msg_type: "text".to_string(),
        content: serde_json::json!({"text": content}).to_string(),
    };

    let body_json =
        serde_json::to_string(&body).map_err(|e| format!("Failed to serialize body: {}", e))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json; charset=utf-8",
        "Authorization": format!("Bearer {}", token),
    });

    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&body_json),
        Some(10_000),
    );

    match result {
        Ok(response) => {
            if response.status != 200 {
                let body_str = String::from_utf8_lossy(&response.body);
                return Err(format!(
                    "Feishu API returned {}: {}",
                    response.status, body_str
                ));
            }
            // Check API-level error code.
            if let Ok(api_resp) =
                serde_json::from_slice::<FeishuApiResponse<serde_json::Value>>(&response.body)
            {
                if api_resp.code != 0 {
                    return Err(format!(
                        "Feishu API error {}: {}",
                        api_resp.code, api_resp.msg
                    ));
                }
            }
            Ok(())
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

/// Send a new message to a user/chat (for broadcast).
fn send_message(receive_id: &str, receive_id_type: &str, content: &str) -> Result<(), String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH)
        .unwrap_or_else(|| "https://open.feishu.cn".to_string());

    let token = get_valid_token(&api_base)?;

    let url = format!(
        "{}/open-apis/im/v1/messages?receive_id_type={}",
        api_base, receive_id_type
    );

    let body = SendMessageBody {
        receive_id: receive_id.to_string(),
        msg_type: "text".to_string(),
        content: serde_json::json!({"text": content}).to_string(),
    };

    let body_json =
        serde_json::to_string(&body).map_err(|e| format!("Failed to serialize body: {}", e))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json; charset=utf-8",
        "Authorization": format!("Bearer {}", token),
    });

    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&body_json),
        Some(10_000),
    );

    match result {
        Ok(response) => {
            if response.status != 200 {
                let body_str = String::from_utf8_lossy(&response.body);
                return Err(format!(
                    "Feishu API returned {}: {}",
                    response.status, body_str
                ));
            }
            if let Ok(api_resp) =
                serde_json::from_slice::<FeishuApiResponse<serde_json::Value>>(&response.body)
            {
                if api_resp.code != 0 {
                    return Err(format!(
                        "Feishu API error {}: {}",
                        api_resp.code, api_resp.msg
                    ));
                }
            }
            Ok(())
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

// ============================================================================
// Token Management
// ============================================================================

/// Get a valid tenant access token, refreshing if needed.
fn get_valid_token(api_base: &str) -> Result<String, String> {
    // Check cached token.
    if let Some(token) = channel_host::workspace_read(TOKEN_PATH) {
        if !token.is_empty() {
            if let Some(expiry_str) = channel_host::workspace_read(TOKEN_EXPIRY_PATH) {
                if let Ok(expiry) = expiry_str.parse::<u64>() {
                    let now = channel_host::now_millis();
                    // Refresh 5 minutes before expiry.
                    if now < expiry.saturating_sub(300_000) {
                        return Ok(token);
                    }
                }
            }
        }
    }

    // Token expired or missing — obtain new one.
    obtain_tenant_token(api_base)
}

/// Exchange app_id + app_secret for a tenant access token.
///
/// Reads credentials from workspace storage (persisted during `on_start`
/// from config JSON injected by the host).
fn obtain_tenant_token(api_base: &str) -> Result<String, String> {
    let app_id = channel_host::workspace_read(APP_ID_PATH)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "app_id not configured (missing from workspace)".to_string())?;
    let app_secret = channel_host::workspace_read(APP_SECRET_PATH)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "app_secret not configured (missing from workspace)".to_string())?;

    let url = format!(
        "{}/open-apis/auth/v3/tenant_access_token/internal",
        api_base
    );

    let body = serde_json::json!({
        "app_id": &app_id,
        "app_secret": &app_secret,
    });

    let headers = serde_json::json!({
        "Content-Type": "application/json; charset=utf-8",
    });

    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&body.to_string()),
        Some(10_000),
    );

    match result {
        Ok(response) => {
            if response.status != 200 {
                let body_str = String::from_utf8_lossy(&response.body);
                return Err(format!(
                    "Token exchange returned {}: {}",
                    response.status, body_str
                ));
            }

            let token_resp: FeishuApiResponse<TenantAccessTokenData> =
                serde_json::from_slice(&response.body)
                    .map_err(|e| format!("Failed to parse token response: {}", e))?;

            if token_resp.code != 0 {
                return Err(format!(
                    "Token exchange error {}: {}",
                    token_resp.code, token_resp.msg
                ));
            }

            let data = token_resp
                .data
                .ok_or_else(|| "Token response missing data".to_string())?;

            // Cache the token with expiry.
            let now = channel_host::now_millis();
            let expiry = now + (data.expire as u64) * 1000;

            let _ = channel_host::workspace_write(TOKEN_PATH, &data.tenant_access_token);
            let _ = channel_host::workspace_write(TOKEN_EXPIRY_PATH, &expiry.to_string());

            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!(
                    "Tenant access token refreshed, expires in {}s",
                    data.expire
                ),
            );

            Ok(data.tenant_access_token)
        }
        Err(e) => Err(format!("Token exchange request failed: {}", e)),
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Build a JSON HTTP response.
fn json_response(status: u16, body: serde_json::Value) -> OutgoingHttpResponse {
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    OutgoingHttpResponse {
        status,
        headers_json: serde_json::json!({
            "Content-Type": "application/json",
        })
        .to_string(),
        body: body_bytes,
    }
}
