// Feishu API types have fields reserved for future use.
#![allow(dead_code)]

//! Feishu/Lark Bot channel for IronClaw.
//!
//! This WASM component implements the channel interface for handling Feishu
//! Event Subscription v2.0 deliveries and sending messages back via the
//! Feishu/Lark Bot API. Inbound delivery supports Feishu's long-connection
//! websocket mode through IronClaw's host-managed websocket runtime, with
//! webhook delivery retained as a fallback.
//!
//! # Features
//!
//! - Long-connection websocket message receiving
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
//! - Webhook requests must be authenticated by the host or by a matching
//!   Feishu verification token in the request body

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, Attachment, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage, InboundAttachment};

// ============================================================================
// Workspace paths for cross-callback state
// ============================================================================

const OWNER_ID_PATH: &str = "owner_id";
const DM_POLICY_PATH: &str = "dm_policy";
const ALLOW_FROM_PATH: &str = "allow_from";
const API_BASE_PATH: &str = "api_base";
const APP_ID_PATH: &str = "app_id";
const APP_SECRET_PATH: &str = "app_secret";
const VERIFICATION_TOKEN_PATH: &str = "verification_token";
const TOKEN_PATH: &str = "tenant_access_token";
const TOKEN_EXPIRY_PATH: &str = "token_expiry";
const CONNECTION_MODE_PATH: &str = "connection_mode";
const WEBSOCKET_EVENT_QUEUE_PATH: &str = "state/gateway_event_queue_processing";
const WEBHOOK_EVENT_QUEUE_PATH: &str = "state/webhook_event_queue";
const RECENT_EVENT_IDS_PATH: &str = "state/recent_event_ids";
const WEBHOOK_POLL_INTERVAL_MS: u32 = 30_000;
const INBOUND_IMAGE_DOWNLOAD_TIMEOUT_MS: u32 = 10_000;
const MAX_INBOUND_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_OUTBOUND_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_RECENT_EVENT_IDS: usize = 512;
const MAX_RECENT_EVENT_ID_AGE_MS: u64 = 24 * 60 * 60 * 1_000;

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

    /// Verification token for v2 event payloads.
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RecentEventIdEntry {
    id: String,
    seen_at_ms: u64,
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

/// Image message content (when message_type == "image").
#[derive(Debug, Deserialize)]
struct ImageContent {
    image_key: String,
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
    data: Option<T>,
}

/// Upload image API response data.
#[derive(Debug, Deserialize)]
struct UploadImageData {
    image_key: String,
}

/// Tenant access token response (flat format).
///
/// Unlike most Feishu APIs that nest results under `data`, the
/// `/auth/v3/tenant_access_token/internal` endpoint returns `code`, `msg`,
/// `tenant_access_token`, and `expire` at the top level.
#[derive(Debug, Deserialize)]
struct TenantAccessTokenResponse {
    #[serde(default)]
    code: i32,
    #[serde(default)]
    msg: String,
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

    /// Feishu Event Subscription verification token.
    verification_token: Option<String>,

    /// Inbound delivery mode: "websocket" (default) or "webhook".
    #[serde(default = "default_connection_mode")]
    connection_mode: String,

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

fn default_connection_mode() -> String {
    "websocket".to_string()
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct FeishuChannel;

export!(FeishuChannel);

impl Guest for FeishuChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: FeishuConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Feishu channel starting");

        // Persist config for cross-callback access.
        let api_base = config.api_base.trim_end_matches('/').to_string();
        let _ = channel_host::workspace_write(API_BASE_PATH, &api_base);
        let connection_mode = config.connection_mode.trim().to_ascii_lowercase();
        let connection_mode = if connection_mode == "webhook" {
            "webhook"
        } else {
            "websocket"
        };
        let _ = channel_host::workspace_write(CONNECTION_MODE_PATH, connection_mode);

        // Persist app credentials for token exchange in later callbacks.
        // These are injected by the host from the secrets store into the
        // config JSON (see setup.rs inject_channel_secrets_into_config).
        if let Some(ref app_id) = config.app_id {
            let _ = channel_host::workspace_write(APP_ID_PATH, app_id);
        }
        if let Some(ref app_secret) = config.app_secret {
            let _ = channel_host::workspace_write(APP_SECRET_PATH, app_secret);
        }
        if let Some(ref verification_token) = config.verification_token {
            let _ = channel_host::workspace_write(VERIFICATION_TOKEN_PATH, verification_token);
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
            poll: poll_config_for_connection_mode(connection_mode),
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

        let configured_token =
            channel_host::workspace_read(VERIFICATION_TOKEN_PATH).filter(|token| !token.is_empty());
        if !is_authenticated_webhook(
            req.secret_validated,
            configured_token.as_deref(),
            request_verification_token(&event),
        ) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Rejecting unauthenticated Feishu webhook request",
            );
            return json_response(
                401,
                serde_json::json!({"error": "Webhook authentication failed"}),
            );
        }

        // Handle URL verification challenge (initial webhook setup).
        if event.event_type.as_deref() == Some("url_verification") {
            if let Some(challenge) = &event.challenge {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    "Handling URL verification challenge",
                );
                return json_response(200, serde_json::json!({ "challenge": challenge }));
            }
        }

        if let Err(error) = enqueue_webhook_event(body_str) {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to enqueue Feishu webhook event: {error}"),
            );
            return json_response(500, serde_json::json!({"error": "Failed to enqueue event"}));
        }

        // Feishu expects a fast acknowledgement. Non-challenge events are
        // processed by on_poll so attachment downloads cannot block this reply.
        json_response(200, serde_json::json!({}))
    }

    fn on_poll() {
        process_websocket_event_queue();
        process_webhook_event_queue();
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: FeishuMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        send_response_to_metadata(
            &metadata,
            &response,
            send_reply_response,
            send_message_response,
            channel_host::log,
        )
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        send_message_response(&user_id, "open_id", &response)
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

fn poll_config_for_connection_mode(connection_mode: &str) -> Option<PollConfig> {
    (connection_mode == "webhook").then_some(PollConfig {
        interval_ms: WEBHOOK_POLL_INTERVAL_MS,
        enabled: true,
    })
}

/// Process events queued by the host-managed Feishu websocket runtime.
fn process_websocket_event_queue() {
    process_websocket_event_queue_with(
        || channel_host::workspace_read(WEBSOCKET_EVENT_QUEUE_PATH).unwrap_or_default(),
        |queue_json| {
            let _ = channel_host::workspace_write(WEBSOCKET_EVENT_QUEUE_PATH, queue_json);
        },
        |frame| process_feishu_event_payload(frame, false),
        None,
        channel_host::log,
    );
}

fn enqueue_webhook_event(body_str: &str) -> Result<(), String> {
    enqueue_event_payload_with(
        || channel_host::workspace_read(WEBHOOK_EVENT_QUEUE_PATH).unwrap_or_default(),
        |queue_json| channel_host::workspace_write(WEBHOOK_EVENT_QUEUE_PATH, queue_json),
        body_str,
        channel_host::log,
    )
}

fn process_webhook_event_queue() {
    process_websocket_event_queue_with(
        || channel_host::workspace_read(WEBHOOK_EVENT_QUEUE_PATH).unwrap_or_default(),
        |queue_json| {
            let _ = channel_host::workspace_write(WEBHOOK_EVENT_QUEUE_PATH, queue_json);
        },
        process_verified_feishu_event_payload,
        Some(1),
        channel_host::log,
    );
}

fn enqueue_event_payload_with(
    mut read_queue: impl FnMut() -> String,
    mut write_queue: impl FnMut(&str) -> Result<(), String>,
    payload: &str,
    mut log: impl FnMut(channel_host::LogLevel, &str),
) -> Result<(), String> {
    let queue_json = read_queue();
    let mut frames: Vec<String> = if queue_json.trim().is_empty() {
        Vec::new()
    } else {
        match serde_json::from_str(&queue_json) {
            Ok(frames) => frames,
            Err(error) => {
                log(
                    channel_host::LogLevel::Warn,
                    &format!("Replacing malformed Feishu webhook queue: {error}"),
                );
                Vec::new()
            }
        }
    };

    frames.push(payload.to_string());
    let queue_json =
        serde_json::to_string(&frames).map_err(|e| format!("Failed to serialize queue: {e}"))?;
    write_queue(&queue_json)
}

fn process_websocket_event_queue_with(
    mut read_queue: impl FnMut() -> String,
    mut write_queue: impl FnMut(&str),
    mut process_payload: impl FnMut(&str),
    max_frames: Option<usize>,
    mut log: impl FnMut(channel_host::LogLevel, &str),
) {
    let queue_json = read_queue();
    if queue_json.trim().is_empty() {
        return;
    }

    let mut frames: Vec<String> = match serde_json::from_str(&queue_json) {
        Ok(frames) => frames,
        Err(error) => {
            log(
                channel_host::LogLevel::Warn,
                &format!("Failed to deserialize Feishu websocket queue: {error}"),
            );
            write_queue("[]");
            return;
        }
    };

    if frames.is_empty() {
        return;
    }

    let remaining = max_frames
        .and_then(|max| (frames.len() > max).then(|| frames.split_off(max)))
        .unwrap_or_default();
    let remaining_json =
        serde_json::to_string(&remaining).unwrap_or_else(|_| "[]".to_string());
    write_queue(&remaining_json);
    for frame in frames {
        process_payload(&frame);
    }
}

fn process_feishu_event_payload(body_str: &str, require_webhook_auth: bool) {
    process_feishu_event_payload_with_workspace(
        body_str,
        require_webhook_auth,
        channel_host::workspace_read,
        channel_host::workspace_write,
        channel_host::now_millis,
        handle_message_event,
        channel_host::log,
    );
}

fn process_verified_feishu_event_payload(body_str: &str) {
    process_feishu_event_payload_with_workspace_auth(
        body_str,
        FeishuEventAuthMode::AlreadyVerified,
        channel_host::workspace_read,
        channel_host::workspace_write,
        channel_host::now_millis,
        handle_message_event,
        channel_host::log,
    );
}

#[derive(Clone, Copy)]
enum FeishuEventAuthMode {
    Websocket,
    Webhook,
    AlreadyVerified,
}

fn process_feishu_event_payload_with_workspace(
    body_str: &str,
    require_webhook_auth: bool,
    mut workspace_read: impl FnMut(&str) -> Option<String>,
    mut workspace_write: impl FnMut(&str, &str) -> Result<(), String>,
    mut now_ms: impl FnMut() -> u64,
    mut handle_message: impl FnMut(&serde_json::Value),
    mut log: impl FnMut(channel_host::LogLevel, &str),
) -> bool {
    let auth_mode = if require_webhook_auth {
        FeishuEventAuthMode::Webhook
    } else {
        FeishuEventAuthMode::Websocket
    };
    process_feishu_event_payload_with_workspace_auth(
        body_str,
        auth_mode,
        &mut workspace_read,
        &mut workspace_write,
        &mut now_ms,
        &mut handle_message,
        &mut log,
    )
}

fn process_feishu_event_payload_with_workspace_auth(
    body_str: &str,
    auth_mode: FeishuEventAuthMode,
    mut workspace_read: impl FnMut(&str) -> Option<String>,
    mut workspace_write: impl FnMut(&str, &str) -> Result<(), String>,
    mut now_ms: impl FnMut() -> u64,
    mut handle_message: impl FnMut(&serde_json::Value),
    mut log: impl FnMut(channel_host::LogLevel, &str),
) -> bool {
    let event: FeishuEvent = match serde_json::from_str(body_str) {
        Ok(e) => e,
        Err(e) => {
            log(
                channel_host::LogLevel::Error,
                &format!("Failed to parse Feishu event: {}", e),
            );
            return false;
        }
    };

    let configured_token =
        workspace_read(VERIFICATION_TOKEN_PATH).filter(|token| !token.is_empty());
    match auth_mode {
        FeishuEventAuthMode::Webhook
            if !is_authenticated_webhook(
                false,
                configured_token.as_deref(),
                request_verification_token(&event),
            ) =>
        {
            log(
                channel_host::LogLevel::Warn,
                "Rejecting unauthenticated Feishu webhook request",
            );
            return false;
        }
        FeishuEventAuthMode::Websocket
            if !is_authenticated_websocket_event(
                configured_token.as_deref(),
                request_verification_token(&event),
            ) =>
        {
            log(
                channel_host::LogLevel::Warn,
                "Ignoring Feishu websocket event with missing or mismatched verification token",
            );
            return false;
        }
        FeishuEventAuthMode::Webhook
        | FeishuEventAuthMode::Websocket
        | FeishuEventAuthMode::AlreadyVerified => {}
    }

    // Handle URL verification challenge (initial webhook setup).
    if event.event_type.as_deref() == Some("url_verification") {
        return false;
    }

    // Handle v2.0 events.
    if let Some(header) = &event.header {
        match header.event_type.as_str() {
            "im.message.receive_v1" => {
                if !should_process_event_id(
                    &header.event_id,
                    &mut workspace_read,
                    &mut workspace_write,
                    &mut now_ms,
                    &mut log,
                ) {
                    return false;
                }
                if let Some(event_data) = &event.event {
                    handle_message(event_data);
                    return true;
                }
            }
            other => {
                log(
                    channel_host::LogLevel::Debug,
                    &format!("Ignoring event type: {}", other),
                );
            }
        }
    }
    false
}

fn update_recent_event_ids(
    existing_json: Option<&str>,
    event_id: &str,
    max_ids: usize,
    now_ms: u64,
    ttl_ms: u64,
) -> Result<(bool, String), String> {
    let mut ids: Vec<RecentEventIdEntry> = match existing_json.filter(|s| !s.trim().is_empty()) {
        Some(raw) => serde_json::from_str::<Vec<RecentEventIdEntry>>(raw).or_else(|_| {
            serde_json::from_str::<Vec<String>>(raw).map(|legacy| {
                legacy
                    .into_iter()
                    .map(|id| RecentEventIdEntry {
                        id,
                        seen_at_ms: now_ms,
                    })
                    .collect()
            })
        })
        .map_err(|e| format!("Failed to parse recent Feishu event ids: {e}"))?,
        None => Vec::new(),
    };

    ids.retain(|entry| now_ms.saturating_sub(entry.seen_at_ms) <= ttl_ms);

    if ids.iter().any(|existing| existing.id == event_id) {
        let json = serde_json::to_string(&ids)
            .map_err(|e| format!("Failed to serialize recent Feishu event ids: {e}"))?;
        return Ok((false, json));
    }

    ids.push(RecentEventIdEntry {
        id: event_id.to_string(),
        seen_at_ms: now_ms,
    });
    if ids.len() > max_ids {
        let to_drop = ids.len() - max_ids;
        ids.drain(0..to_drop);
    }

    let json = serde_json::to_string(&ids)
        .map_err(|e| format!("Failed to serialize recent Feishu event ids: {e}"))?;
    Ok((true, json))
}

fn should_process_event_id(
    event_id: &str,
    mut workspace_read: impl FnMut(&str) -> Option<String>,
    mut workspace_write: impl FnMut(&str, &str) -> Result<(), String>,
    mut now_ms: impl FnMut() -> u64,
    mut log: impl FnMut(channel_host::LogLevel, &str),
) -> bool {
    match update_recent_event_ids(
        workspace_read(RECENT_EVENT_IDS_PATH).as_deref(),
        event_id,
        MAX_RECENT_EVENT_IDS,
        now_ms(),
        MAX_RECENT_EVENT_ID_AGE_MS,
    ) {
        Ok((true, json)) => {
            if let Err(error) = workspace_write(RECENT_EVENT_IDS_PATH, &json) {
                log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to persist Feishu event dedupe state: {error}"),
                );
            }
            true
        }
        Ok((false, _)) => false,
        Err(error) => {
            log(
                channel_host::LogLevel::Warn,
                &format!("Failed to update Feishu event dedupe state: {error}"),
            );
            true
        }
    }
}

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
                    &format!(
                        "Ignoring message from user not in allow_from: {}",
                        sender_id
                    ),
                );
                return;
            }
        }
    }

    // DM pairing check for p2p chats.
    let chat_type = msg_event.message.chat_type.as_deref().unwrap_or("unknown");

    // Resolved user_id for the emitted message. Defaults to sender_id but
    // is overwritten with the owner_id when the sender is paired, ensuring
    // the message is scoped to the correct owner/tenant.
    let mut user_id = sender_id.to_string();

    if chat_type == "p2p" {
        let dm_policy =
            channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

        if dm_policy == "pairing" {
            match channel_host::pairing_resolve_identity("feishu", sender_id) {
                Ok(Some(owner_id)) => {
                    // Sender is paired; scope message to owner.
                    user_id = owner_id;
                }
                Ok(None) => {
                    // Unknown sender — upsert a pairing request.
                    let meta = serde_json::json!({
                        "sender_id": sender_id,
                        "chat_id": msg_event.message.chat_id,
                        "chat_type": chat_type,
                    });
                    match channel_host::pairing_upsert_request(
                        "feishu",
                        sender_id,
                        &meta.to_string(),
                    ) {
                        Ok(result) => {
                            channel_host::log(
                                channel_host::LogLevel::Info,
                                &format!(
                                    "Pairing request created for {}: {}",
                                    sender_id, result.code
                                ),
                            );
                            let _ = send_message(
                                sender_id,
                                "open_id",
                                &format!(
                                    "Enter this code in IronClaw to pair your feishu account: `{}`. CLI fallback: `ironclaw pairing approve feishu {}`",
                                    result.code, result.code
                                ),
                            );
                        }
                        Err(e) => {
                            channel_host::log(
                                channel_host::LogLevel::Error,
                                &format!("Pairing upsert failed: {}", e),
                            );
                        }
                    }
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

    // Extract text and media content.
    let (text, attachments) = extract_message_content(&msg_event.message);
    if text.is_empty() && attachments.is_empty() {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Ignoring unsupported message type: {}",
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

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Determine thread ID from reply chain.
    let thread_id = msg_event
        .message
        .root_id
        .as_deref()
        .or(msg_event.message.parent_id.as_deref())
        .map(|s| s.to_string());

    // Emit message to the agent.
    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: None,
        content: text,
        thread_id,
        metadata_json,
        attachments,
    });
}

/// Extract text and supported media content from a Feishu message.
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

fn extract_message_content(message: &FeishuMessage) -> (String, Vec<InboundAttachment>) {
    match message.message_type.as_str() {
        "text" => (extract_text_content(message), Vec::new()),
        "image" => match feishu_image_attachment(message) {
            Ok(attachment) => (String::new(), vec![attachment]),
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to prepare Feishu image attachment: {error}"),
                );
                (String::new(), Vec::new())
            }
        },
        _ => (String::new(), Vec::new()),
    }
}

fn base_mime_type(mime_type: &str) -> &str {
    mime_type.split(';').next().unwrap_or("").trim()
}

fn header_value_case_insensitive<'a>(
    headers: &'a serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_str())
}

fn filename_extension_for_mime(mime_type: &str) -> &'static str {
    match base_mime_type(mime_type).to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "image/tiff" => "tiff",
        "image/heic" | "image/heif" => "heic",
        _ => "jpg",
    }
}

fn parse_image_content(content: &str) -> Result<ImageContent, String> {
    serde_json::from_str::<ImageContent>(content)
        .map_err(|e| format!("Failed to parse Feishu image content: {e}"))
}

fn feishu_image_resource_url(api_base: &str, message_id: &str, image_key: &str) -> String {
    format!(
        "{}/open-apis/im/v1/messages/{}/resources/{}?type=image",
        api_base, message_id, image_key
    )
}

fn feishu_image_attachment(message: &FeishuMessage) -> Result<InboundAttachment, String> {
    let image = parse_image_content(&message.content)?;
    let api_base = channel_host::workspace_read(API_BASE_PATH)
        .unwrap_or_else(|| "https://open.feishu.cn".to_string());
    let source_url =
        feishu_image_resource_url(&api_base, &message.message_id, image.image_key.as_str());
    let attachment_id = format!("{}:{}", message.message_id, image.image_key);
    let attachment = InboundAttachment {
        id: attachment_id,
        mime_type: "image/jpeg".to_string(),
        filename: Some(format!("feishu-{}.jpg", message.message_id)),
        size_bytes: None,
        source_url: Some(source_url.clone()),
        storage_key: None,
        extracted_text: None,
        extras_json: serde_json::json!({
            "feishu_image_key": image.image_key,
            "feishu_message_id": message.message_id,
        })
        .to_string(),
    };

    #[cfg(not(test))]
    {
        let mut attachment = attachment;
        hydrate_feishu_image_attachment(&mut attachment, &source_url)?;
        Ok(attachment)
    }

    #[cfg(test)]
    {
        Ok(attachment)
    }
}

#[cfg(not(test))]
fn hydrate_feishu_image_attachment(
    attachment: &mut InboundAttachment,
    source_url: &str,
) -> Result<(), String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH)
        .unwrap_or_else(|| "https://open.feishu.cn".to_string());
    let token = get_valid_token(&api_base)?;
    let headers = serde_json::json!({
        "Authorization": format!("Bearer {}", token),
        "Content-Type": "application/json; charset=utf-8",
    });

    let response = channel_host::http_request(
        "GET",
        source_url,
        &headers.to_string(),
        None,
        Some(INBOUND_IMAGE_DOWNLOAD_TIMEOUT_MS),
    )
    .map_err(|e| format!("Failed to download Feishu image: {e}"))?;
    if response.status != 200 {
        return Err(format!(
            "Feishu image download returned {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    if response.body.len() > MAX_INBOUND_IMAGE_BYTES {
        return Err(format!(
            "Feishu image exceeds {} bytes",
            MAX_INBOUND_IMAGE_BYTES
        ));
    }

    let headers: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&response.headers_json).unwrap_or_default();
    let mime_type = header_value_case_insensitive(&headers, "content-type")
        .map(base_mime_type)
        .filter(|value| value.starts_with("image/"))
        .unwrap_or("image/jpeg")
        .to_string();
    let extension = filename_extension_for_mime(&mime_type);
    attachment.mime_type = mime_type;
    attachment.filename = Some(format!("{}.{}", attachment.id, extension));
    attachment.size_bytes = Some(response.body.len() as u64);

    channel_host::store_attachment_data(&attachment.id, &response.body)
        .map_err(|e| format!("Failed to store Feishu image attachment data: {e}"))?;

    Ok(())
}

// ============================================================================
// Outbound Messaging
// ============================================================================

fn validate_feishu_api_response<T: for<'de> Deserialize<'de>>(
    body: &[u8],
) -> Result<FeishuApiResponse<T>, String> {
    let api_resp: FeishuApiResponse<T> = serde_json::from_slice(body)
        .map_err(|e| format!("Failed to parse Feishu API response: {e}"))?;
    if api_resp.code != 0 {
        return Err(format!(
            "Feishu API error {}: {}",
            api_resp.code, api_resp.msg
        ));
    }
    Ok(api_resp)
}

fn checked_feishu_request(
    method: &str,
    url: &str,
    headers_json: &str,
    body: Option<&[u8]>,
    timeout_ms: u32,
) -> Result<Vec<u8>, String> {
    let response = channel_host::http_request(method, url, headers_json, body, Some(timeout_ms))
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    if response.status != 200 {
        return Err(format!(
            "Feishu API returned {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    Ok(response.body)
}

/// Reply to a specific message.
fn send_reply(message_id: &str, content: &str) -> Result<(), String> {
    if content.trim().is_empty() {
        return Ok(());
    }
    send_reply_payload(
        message_id,
        "text",
        serde_json::json!({"text": content}).to_string(),
    )
}

fn send_reply_payload(message_id: &str, msg_type: &str, content: String) -> Result<(), String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH)
        .unwrap_or_else(|| "https://open.feishu.cn".to_string());

    let token = get_valid_token(&api_base)?;

    let url = format!("{}/open-apis/im/v1/messages/{}/reply", api_base, message_id);

    let body = ReplyMessageBody {
        msg_type: msg_type.to_string(),
        content,
    };

    let body_json =
        serde_json::to_string(&body).map_err(|e| format!("Failed to serialize body: {}", e))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json; charset=utf-8",
        "Authorization": format!("Bearer {}", token),
    });

    let body = checked_feishu_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(body_json.as_bytes()),
        10_000,
    )?;

    validate_feishu_api_response::<serde_json::Value>(&body)?;
    Ok(())
}

/// Send a new message to a user/chat (for broadcast).
fn send_message(receive_id: &str, receive_id_type: &str, content: &str) -> Result<(), String> {
    if content.trim().is_empty() {
        return Ok(());
    }
    send_message_payload(
        receive_id,
        receive_id_type,
        "text",
        serde_json::json!({"text": content}).to_string(),
    )
}

fn send_message_payload(
    receive_id: &str,
    receive_id_type: &str,
    msg_type: &str,
    content: String,
) -> Result<(), String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH)
        .unwrap_or_else(|| "https://open.feishu.cn".to_string());

    let token = get_valid_token(&api_base)?;

    let url = format!(
        "{}/open-apis/im/v1/messages?receive_id_type={}",
        api_base, receive_id_type
    );

    let body = SendMessageBody {
        receive_id: receive_id.to_string(),
        msg_type: msg_type.to_string(),
        content,
    };

    let body_json =
        serde_json::to_string(&body).map_err(|e| format!("Failed to serialize body: {}", e))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json; charset=utf-8",
        "Authorization": format!("Bearer {}", token),
    });

    let body = checked_feishu_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(body_json.as_bytes()),
        10_000,
    )?;

    validate_feishu_api_response::<serde_json::Value>(&body)?;
    Ok(())
}

fn sanitize_multipart_filename(filename: &str) -> String {
    let sanitized: String = filename
        .chars()
        .map(|ch| match ch {
            '"' | '\\' | '\r' | '\n' => '_',
            other => other,
        })
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "image".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_multipart_image_body(
    boundary: &str,
    filename: &str,
    mime_type: &str,
    data: &[u8],
) -> Vec<u8> {
    let filename = sanitize_multipart_filename(filename);
    let mime_type = base_mime_type(mime_type);
    let mime_type = if mime_type.starts_with("image/") {
        mime_type
    } else {
        "application/octet-stream"
    };

    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"image\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {mime_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"image_type\"\r\n\r\n");
    body.extend_from_slice(b"message\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

fn is_supported_outbound_image(attachment: &Attachment) -> bool {
    base_mime_type(&attachment.mime_type)
        .to_ascii_lowercase()
        .starts_with("image/")
}

fn upload_feishu_image(attachment: &Attachment) -> Result<String, String> {
    if !is_supported_outbound_image(attachment) {
        return Err(format!(
            "Feishu only supports image attachments here; '{}' has MIME type '{}'",
            attachment.filename, attachment.mime_type
        ));
    }
    if attachment.data.is_empty() {
        return Err(format!(
            "Feishu image attachment '{}' is empty",
            attachment.filename
        ));
    }
    if attachment.data.len() > MAX_OUTBOUND_IMAGE_BYTES {
        return Err(format!(
            "Feishu image attachment '{}' exceeds {} bytes",
            attachment.filename, MAX_OUTBOUND_IMAGE_BYTES
        ));
    }

    let api_base = channel_host::workspace_read(API_BASE_PATH)
        .unwrap_or_else(|| "https://open.feishu.cn".to_string());
    let token = get_valid_token(&api_base)?;
    let url = format!("{}/open-apis/im/v1/images", api_base);
    let boundary = format!("ironclaw-feishu-image-{}", channel_host::now_millis());
    let body = build_multipart_image_body(
        &boundary,
        &attachment.filename,
        &attachment.mime_type,
        &attachment.data,
    );
    let headers = serde_json::json!({
        "Authorization": format!("Bearer {}", token),
        "Content-Type": format!("multipart/form-data; boundary={}", boundary),
    });

    let response_body =
        checked_feishu_request("POST", &url, &headers.to_string(), Some(&body), 30_000)?;
    let api_resp = validate_feishu_api_response::<UploadImageData>(&response_body)?;
    let data = api_resp
        .data
        .ok_or_else(|| "Feishu image upload response missing data".to_string())?;
    if data.image_key.trim().is_empty() {
        return Err("Feishu image upload response missing image_key".to_string());
    }
    Ok(data.image_key)
}

fn image_message_content(image_key: &str) -> String {
    serde_json::json!({ "image_key": image_key }).to_string()
}

enum FeishuResponsePart {
    Image(String),
    Text(String),
}

fn send_reply_image(message_id: &str, image_key: &str) -> Result<(), String> {
    send_reply_payload(message_id, "image", image_message_content(image_key))
}

fn send_message_image(
    receive_id: &str,
    receive_id_type: &str,
    image_key: &str,
) -> Result<(), String> {
    send_message_payload(
        receive_id,
        receive_id_type,
        "image",
        image_message_content(image_key),
    )
}

fn append_attachment_errors_to_text(content: &str, errors: &[String]) -> String {
    let first = errors
        .first()
        .map(String::as_str)
        .unwrap_or("unknown error");
    let trimmed = content.trim();
    if trimmed.is_empty() {
        format!("Image attachment delivery failed: {first}")
    } else {
        format!("{trimmed}\n\nImage attachment delivery failed: {first}")
    }
}

fn send_reply_response(message_id: &str, response: &AgentResponse) -> Result<(), String> {
    send_reply_response_with_upload(
        message_id,
        response,
        upload_feishu_image,
        send_reply_payload,
    )
}

fn send_message_response(
    receive_id: &str,
    receive_id_type: &str,
    response: &AgentResponse,
) -> Result<(), String> {
    send_message_response_with_upload(
        receive_id,
        receive_id_type,
        response,
        upload_feishu_image,
        |receive_id, receive_id_type, msg_type, content| {
            send_message_payload(receive_id, receive_id_type, msg_type, content)
        },
    )
}

fn send_response_to_metadata(
    metadata: &FeishuMessageMetadata,
    response: &AgentResponse,
    mut send_reply: impl FnMut(&str, &AgentResponse) -> Result<(), String>,
    mut send_message: impl FnMut(&str, &str, &AgentResponse) -> Result<(), String>,
    mut log: impl FnMut(channel_host::LogLevel, &str),
) -> Result<(), String> {
    match send_reply(&metadata.message_id, response) {
        Ok(()) => Ok(()),
        Err(reply_error) => {
            if metadata.chat_id.trim().is_empty() {
                return Err(reply_error);
            }

            log(
                channel_host::LogLevel::Warn,
                &format!(
                    "Failed to reply to Feishu message; falling back to chat send: {reply_error}"
                ),
            );
            send_message(&metadata.chat_id, "chat_id", response).map_err(|fallback_error| {
                format!(
                    "Failed to reply to Feishu message ({reply_error}); fallback chat send also failed ({fallback_error})"
                )
            })
        }
    }
}

fn send_reply_response_with_upload(
    message_id: &str,
    response: &AgentResponse,
    upload_image: impl FnMut(&Attachment) -> Result<String, String>,
    mut send_payload: impl FnMut(&str, &str, String) -> Result<(), String>,
) -> Result<(), String> {
    send_response_images_then_text_with_upload(response, upload_image, |part| match part {
        FeishuResponsePart::Image(image_key) => {
            send_payload(message_id, "image", image_message_content(&image_key))
        }
        FeishuResponsePart::Text(content) => send_payload(
            message_id,
            "text",
            serde_json::json!({"text": content}).to_string(),
        ),
    })
}

fn send_message_response_with_upload(
    receive_id: &str,
    receive_id_type: &str,
    response: &AgentResponse,
    upload_image: impl FnMut(&Attachment) -> Result<String, String>,
    mut send_payload: impl FnMut(&str, &str, &str, String) -> Result<(), String>,
) -> Result<(), String> {
    send_response_images_then_text_with_upload(response, upload_image, |part| match part {
        FeishuResponsePart::Image(image_key) => send_payload(
            receive_id,
            receive_id_type,
            "image",
            image_message_content(&image_key),
        ),
        FeishuResponsePart::Text(content) => send_payload(
            receive_id,
            receive_id_type,
            "text",
            serde_json::json!({"text": content}).to_string(),
        ),
    })
}

fn send_response_images_then_text_with_upload(
    response: &AgentResponse,
    mut upload_image: impl FnMut(&Attachment) -> Result<String, String>,
    mut send_part: impl FnMut(FeishuResponsePart) -> Result<(), String>,
) -> Result<(), String> {
    let mut sent_any = false;
    let mut errors = Vec::new();

    for attachment in &response.attachments {
        match upload_image(attachment)
            .and_then(|image_key| send_part(FeishuResponsePart::Image(image_key)))
        {
            Ok(()) => sent_any = true,
            Err(error) => errors.push(error),
        }
    }

    let content = if errors.is_empty() {
        response.content.trim().to_string()
    } else {
        append_attachment_errors_to_text(&response.content, &errors)
    };
    if !content.is_empty() {
        match send_part(FeishuResponsePart::Text(content)) {
            Ok(()) => sent_any = true,
            Err(error) if sent_any => {
                errors.push(format!(
                    "Feishu text delivery failed after attachment delivery: {error}"
                ));
            }
            Err(error) => return Err(error),
        }
    }

    if !errors.is_empty() {
        let joined = errors.join("; ");
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!("Feishu response delivery had errors: {joined}"),
        );
        if !sent_any {
            return Err(joined);
        }
    }

    Ok(())
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

    let body_bytes = body.to_string();
    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(body_bytes.as_bytes()),
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

            let token_resp: TenantAccessTokenResponse = serde_json::from_slice(&response.body)
                .map_err(|e| format!("Failed to parse token response: {}", e))?;

            if token_resp.code != 0 {
                return Err(format!(
                    "Token exchange error {}: {}",
                    token_resp.code, token_resp.msg
                ));
            }

            if token_resp.tenant_access_token.is_empty() {
                return Err("Token response missing tenant_access_token".to_string());
            }

            if token_resp.expire <= 0 {
                return Err(format!(
                    "Token response has invalid expire value: {}",
                    token_resp.expire
                ));
            }

            // Cache the token with expiry.
            let now = channel_host::now_millis();
            let expiry = now.saturating_add((token_resp.expire as u64).saturating_mul(1000));

            let _ = channel_host::workspace_write(TOKEN_PATH, &token_resp.tenant_access_token);
            let _ = channel_host::workspace_write(TOKEN_EXPIRY_PATH, &expiry.to_string());

            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!(
                    "Tenant access token refreshed, expires in {}s",
                    token_resp.expire
                ),
            );

            Ok(token_resp.tenant_access_token)
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

fn is_authenticated_webhook(
    secret_validated: bool,
    configured_token: Option<&str>,
    request_token: Option<&str>,
) -> bool {
    if secret_validated {
        return true;
    }

    match (configured_token, request_token) {
        (Some(expected), Some(provided)) => {
            bool::from(expected.as_bytes().ct_eq(provided.as_bytes()))
        }
        _ => false,
    }
}

fn is_authenticated_websocket_event(
    configured_token: Option<&str>,
    request_token: Option<&str>,
) -> bool {
    match configured_token {
        None => true,
        Some(expected) => request_token
            .map(|provided| bool::from(expected.as_bytes().ct_eq(provided.as_bytes())))
            .unwrap_or(false),
    }
}

fn request_verification_token(event: &FeishuEvent) -> Option<&str> {
    event
        .header
        .as_ref()
        .and_then(|header| header.token.as_deref())
        .or(event.token.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_flat_token_response() {
        let json = r#"{
            "code": 0,
            "msg": "ok",
            "tenant_access_token": "t-abc123",
            "expire": 7200
        }"#;
        let resp: TenantAccessTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert_eq!(resp.msg, "ok");
        assert_eq!(resp.tenant_access_token, "t-abc123");
        assert_eq!(resp.expire, 7200);
    }

    #[test]
    fn parse_token_response_rejects_missing_token() {
        let json = r#"{"code": 0, "msg": "ok", "expire": 7200}"#;
        let result: Result<TenantAccessTokenResponse, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "should fail when tenant_access_token is missing"
        );
    }

    #[test]
    fn parse_token_response_rejects_missing_expire() {
        let json = r#"{"code": 0, "msg": "ok", "tenant_access_token": "t-abc"}"#;
        let result: Result<TenantAccessTokenResponse, _> = serde_json::from_str(json);
        assert!(result.is_err(), "should fail when expire is missing");
    }

    #[test]
    fn parse_token_response_defaults_code_and_msg() {
        let json = r#"{"tenant_access_token": "t-abc", "expire": 3600}"#;
        let resp: TenantAccessTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert_eq!(resp.msg, "");
        assert_eq!(resp.tenant_access_token, "t-abc");
        assert_eq!(resp.expire, 3600);
    }

    #[test]
    fn parse_token_error_response() {
        let json = r#"{
            "code": 10003,
            "msg": "invalid app_id",
            "tenant_access_token": "",
            "expire": 0
        }"#;
        let resp: TenantAccessTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 10003);
        assert!(resp.tenant_access_token.is_empty());
    }

    #[test]
    fn webhook_mode_enables_polling_for_deferred_events() {
        let poll = poll_config_for_connection_mode("webhook").unwrap();
        assert!(poll.enabled);
        assert_eq!(poll.interval_ms, WEBHOOK_POLL_INTERVAL_MS);
        assert!(poll_config_for_connection_mode("websocket").is_none());
    }

    #[test]
    fn webhook_auth_requires_host_auth_or_matching_verification_token() {
        assert!(
            !is_authenticated_webhook(false, None, Some("token")),
            "requests without any configured verification mechanism must be rejected"
        );
        assert!(
            !is_authenticated_webhook(false, Some("expected"), None),
            "requests missing the Feishu token must be rejected when host auth did not pass"
        );
        assert!(
            !is_authenticated_webhook(false, Some("expected"), Some("wrong")),
            "requests with the wrong Feishu token must be rejected"
        );
        assert!(
            is_authenticated_webhook(false, Some("expected"), Some("expected")),
            "matching Feishu verification token should authenticate the request"
        );
        assert!(
            is_authenticated_webhook(true, None, None),
            "host-authenticated requests should still be accepted"
        );
        assert!(
            is_authenticated_webhook(true, Some("expected"), Some("wrong")),
            "host authentication should take precedence over body token checks"
        );
    }

    #[test]
    fn websocket_auth_requires_matching_token_when_configured() {
        assert!(
            is_authenticated_websocket_event(None, None),
            "websocket events should be accepted without a configured verification token"
        );
        assert!(
            is_authenticated_websocket_event(None, Some("token")),
            "provided event tokens are ignored when no token is configured"
        );
        assert!(
            !is_authenticated_websocket_event(Some("expected"), None),
            "configured verification tokens must reject websocket events missing the token"
        );
        assert!(
            !is_authenticated_websocket_event(Some("expected"), Some("wrong")),
            "configured verification tokens must reject mismatched websocket events"
        );
        assert!(
            is_authenticated_websocket_event(Some("expected"), Some("expected")),
            "configured verification tokens should accept matching websocket events"
        );
    }

    #[test]
    fn process_feishu_event_payload_accepts_websocket_event_without_configured_token() {
        let handled = std::cell::RefCell::new(false);
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt_123",
                "event_type": "im.message.receive_v1"
            },
            "event": {}
        })
        .to_string();

        let processed = process_feishu_event_payload_with_workspace(
            &body,
            false,
            |_path| None,
            |_path, _content| Ok(()),
            || 1_000,
            |_event| {
                *handled.borrow_mut() = true;
            },
            |_level, _message| {},
        );

        assert!(processed);
        assert!(*handled.borrow());
    }

    #[test]
    fn parse_image_content_extracts_image_key() {
        let content = parse_image_content(r#"{"image_key":"img_v2_abc"}"#).unwrap();
        assert_eq!(content.image_key, "img_v2_abc");
    }

    #[test]
    fn feishu_image_resource_url_uses_message_resource_endpoint() {
        assert_eq!(
            feishu_image_resource_url("https://open.feishu.cn", "om_123", "img_v2_abc"),
            "https://open.feishu.cn/open-apis/im/v1/messages/om_123/resources/img_v2_abc?type=image"
        );
    }

    #[test]
    fn build_multipart_image_body_includes_image_and_message_type() {
        let body =
            build_multipart_image_body("boundary-1", "bad\r\nname.png", "image/png", b"png-bytes");
        let body_text = String::from_utf8_lossy(&body);

        assert!(body_text.contains("filename=\"bad__name.png\""));
        assert!(body_text.contains("Content-Type: image/png"));
        assert!(body_text.contains("name=\"image_type\""));
        assert!(body_text.contains("message"));
        assert!(body.ends_with(b"--boundary-1--\r\n"));
    }

    #[test]
    fn image_message_content_uses_image_key_shape() {
        let value: serde_json::Value =
            serde_json::from_str(&image_message_content("img_v2_abc")).unwrap();
        assert_eq!(value["image_key"], "img_v2_abc");
    }

    #[test]
    fn send_reply_response_uploads_image_reply_before_text_reply() {
        let response = AgentResponse {
            message_id: "agent-msg-1".to_string(),
            content: "generated".to_string(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: vec![Attachment {
                filename: "result.png".to_string(),
                mime_type: "image/png".to_string(),
                data: b"png-bytes".to_vec(),
            }],
        };
        let uploaded = std::cell::RefCell::new(Vec::<String>::new());
        let sent = std::cell::RefCell::new(Vec::<(String, String, String)>::new());

        send_reply_response_with_upload(
            "om_123",
            &response,
            |attachment| {
                uploaded.borrow_mut().push(format!(
                    "{}:{}:{}",
                    attachment.filename,
                    attachment.mime_type,
                    attachment.data.len()
                ));
                Ok("img_v2_uploaded".to_string())
            },
            |message_id, msg_type, content| {
                sent.borrow_mut()
                    .push((message_id.to_string(), msg_type.to_string(), content));
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            uploaded.borrow().as_slice(),
            &["result.png:image/png:9".to_string()]
        );
        let sent = sent.borrow();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].0, "om_123");
        assert_eq!(sent[0].1, "image");
        let image_content: serde_json::Value = serde_json::from_str(&sent[0].2).unwrap();
        assert_eq!(image_content["image_key"], "img_v2_uploaded");
        assert_eq!(sent[1].0, "om_123");
        assert_eq!(sent[1].1, "text");
        let text_content: serde_json::Value = serde_json::from_str(&sent[1].2).unwrap();
        assert_eq!(text_content["text"], "generated");
    }

    #[test]
    fn send_response_to_metadata_uses_reply_when_available() {
        let metadata = FeishuMessageMetadata {
            chat_id: "oc_chat".to_string(),
            message_id: "om_msg".to_string(),
            chat_type: "group".to_string(),
        };
        let response = AgentResponse {
            message_id: "agent-msg-1".to_string(),
            content: "generated".to_string(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: Vec::new(),
        };
        let replied = std::cell::RefCell::new(Vec::<String>::new());
        let fallback_sent = std::cell::Cell::new(false);

        send_response_to_metadata(
            &metadata,
            &response,
            |message_id, _response| {
                replied.borrow_mut().push(message_id.to_string());
                Ok(())
            },
            |_receive_id, _receive_id_type, _response| {
                fallback_sent.set(true);
                Ok(())
            },
            |_level, _message| {},
        )
        .unwrap();

        assert_eq!(replied.borrow().as_slice(), &["om_msg".to_string()]);
        assert!(!fallback_sent.get());
    }

    #[test]
    fn send_response_to_metadata_falls_back_to_chat_send_when_reply_fails() {
        let metadata = FeishuMessageMetadata {
            chat_id: "oc_chat".to_string(),
            message_id: "om_msg".to_string(),
            chat_type: "group".to_string(),
        };
        let response = AgentResponse {
            message_id: "agent-msg-1".to_string(),
            content: "generated".to_string(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: Vec::new(),
        };
        let sent = std::cell::RefCell::new(Vec::<(String, String)>::new());
        let warnings = std::cell::RefCell::new(Vec::<String>::new());

        send_response_to_metadata(
            &metadata,
            &response,
            |_message_id, _response| Err("reply expired".to_string()),
            |receive_id, receive_id_type, _response| {
                sent.borrow_mut()
                    .push((receive_id.to_string(), receive_id_type.to_string()));
                Ok(())
            },
            |level, message| {
                if matches!(level, channel_host::LogLevel::Warn) {
                    warnings.borrow_mut().push(message.to_string());
                }
            },
        )
        .unwrap();

        assert_eq!(
            sent.borrow().as_slice(),
            &[("oc_chat".to_string(), "chat_id".to_string())]
        );
        assert_eq!(warnings.borrow().len(), 1);
        assert!(warnings.borrow()[0].contains("reply expired"));
    }

    #[test]
    fn send_response_to_metadata_returns_combined_error_when_fallback_fails() {
        let metadata = FeishuMessageMetadata {
            chat_id: "oc_chat".to_string(),
            message_id: "om_msg".to_string(),
            chat_type: "group".to_string(),
        };
        let response = AgentResponse {
            message_id: "agent-msg-1".to_string(),
            content: "generated".to_string(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: Vec::new(),
        };

        let error = send_response_to_metadata(
            &metadata,
            &response,
            |_message_id, _response| Err("reply expired".to_string()),
            |_receive_id, _receive_id_type, _response| Err("missing permission".to_string()),
            |_level, _message| {},
        )
        .unwrap_err();

        assert!(error.contains("reply expired"));
        assert!(error.contains("missing permission"));
    }

    #[test]
    fn send_reply_response_does_not_error_after_image_part_is_sent() {
        let response = AgentResponse {
            message_id: "agent-msg-1".to_string(),
            content: "generated".to_string(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: vec![Attachment {
                filename: "result.png".to_string(),
                mime_type: "image/png".to_string(),
                data: b"png-bytes".to_vec(),
            }],
        };
        let sent = std::cell::RefCell::new(Vec::<String>::new());

        send_reply_response_with_upload(
            "om_123",
            &response,
            |_attachment| Ok("img_v2_uploaded".to_string()),
            |_message_id, msg_type, _content| {
                sent.borrow_mut().push(msg_type.to_string());
                if msg_type == "text" {
                    Err("text send failed".to_string())
                } else {
                    Ok(())
                }
            },
        )
        .expect("partial image delivery should not trigger fallback resend");

        assert_eq!(
            sent.borrow().as_slice(),
            &["image".to_string(), "text".to_string()]
        );
    }

    #[test]
    fn append_attachment_errors_to_text_preserves_existing_text() {
        assert_eq!(
            append_attachment_errors_to_text("done", &["upload failed".to_string()]),
            "done\n\nImage attachment delivery failed: upload failed"
        );
        assert_eq!(
            append_attachment_errors_to_text("", &["upload failed".to_string()]),
            "Image attachment delivery failed: upload failed"
        );
    }

    #[test]
    fn process_websocket_event_queue_clears_queue_on_malformed_json() {
        let queue = std::cell::RefCell::new(r#"{"not":"an array"}"#.to_string());
        let processed = std::cell::RefCell::new(Vec::<String>::new());

        process_websocket_event_queue_with(
            || queue.borrow().clone(),
            |queue_json| {
                *queue.borrow_mut() = queue_json.to_string();
            },
            |frame| processed.borrow_mut().push(frame.to_string()),
            None,
            |_level, _message| {},
        );

        assert_eq!(queue.borrow().as_str(), "[]");
        assert!(processed.borrow().is_empty());
    }

    #[test]
    fn process_websocket_event_queue_processes_each_snapshot_frame_once() {
        let queue = std::cell::RefCell::new(serde_json::json!(["first", "second"]).to_string());
        let processed = std::cell::RefCell::new(Vec::<String>::new());

        process_websocket_event_queue_with(
            || queue.borrow().clone(),
            |queue_json| {
                *queue.borrow_mut() = queue_json.to_string();
            },
            |frame| processed.borrow_mut().push(frame.to_string()),
            None,
            |_level, _message| {},
        );

        assert_eq!(
            processed.borrow().as_slice(),
            &["first".to_string(), "second".to_string()]
        );
        assert_eq!(queue.borrow().as_str(), "[]");
    }

    #[test]
    fn process_websocket_event_queue_does_not_depend_on_reading_its_own_writes() {
        let original_queue = serde_json::json!(["first"]).to_string();
        let write_count = std::cell::Cell::new(0usize);
        let processed = std::cell::RefCell::new(Vec::<String>::new());

        process_websocket_event_queue_with(
            || original_queue.clone(),
            |_queue_json| write_count.set(write_count.get() + 1),
            |frame| processed.borrow_mut().push(frame.to_string()),
            None,
            |_level, _message| {},
        );

        assert_eq!(processed.borrow().as_slice(), &["first".to_string()]);
        assert_eq!(write_count.get(), 1);
    }

    #[test]
    fn enqueue_event_payload_appends_to_existing_queue() {
        let queue = std::cell::RefCell::new(serde_json::json!(["first"]).to_string());

        enqueue_event_payload_with(
            || queue.borrow().clone(),
            |queue_json| {
                *queue.borrow_mut() = queue_json.to_string();
                Ok(())
            },
            "second",
            |_level, _message| {},
        )
        .unwrap();

        let frames: Vec<String> = serde_json::from_str(&queue.borrow()).unwrap();
        assert_eq!(frames, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn process_websocket_event_queue_can_leave_unprocessed_frames() {
        let queue =
            std::cell::RefCell::new(serde_json::json!(["first", "second", "third"]).to_string());
        let processed = std::cell::RefCell::new(Vec::<String>::new());

        process_websocket_event_queue_with(
            || queue.borrow().clone(),
            |queue_json| {
                *queue.borrow_mut() = queue_json.to_string();
            },
            |frame| processed.borrow_mut().push(frame.to_string()),
            Some(1),
            |_level, _message| {},
        );

        assert_eq!(processed.borrow().as_slice(), &["first".to_string()]);
        let remaining: Vec<String> = serde_json::from_str(&queue.borrow()).unwrap();
        assert_eq!(remaining, vec!["second".to_string(), "third".to_string()]);
    }

    #[test]
    fn update_recent_event_ids_rejects_duplicate_event_id() {
        let (is_new, json) =
            update_recent_event_ids(None, "evt_1", MAX_RECENT_EVENT_IDS, 1_000, 10_000)
                .expect("first dedupe update");
        assert!(is_new);

        let (is_new, _json) = update_recent_event_ids(
            Some(&json),
            "evt_1",
            MAX_RECENT_EVENT_IDS,
            1_001,
            10_000,
        )
        .expect("duplicate dedupe update");
        assert!(!is_new);
    }

    #[test]
    fn process_feishu_event_payload_drops_duplicate_event_id() {
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt_duplicate",
                "event_type": "im.message.receive_v1"
            },
            "event": {}
        })
        .to_string();
        let dedupe_state = std::cell::RefCell::new(None::<String>);
        let handled = std::cell::Cell::new(0usize);

        let first = process_feishu_event_payload_with_workspace_auth(
            &body,
            FeishuEventAuthMode::AlreadyVerified,
            |path| {
                if path == RECENT_EVENT_IDS_PATH {
                    dedupe_state.borrow().clone()
                } else {
                    None
                }
            },
            |path, content| {
                if path == RECENT_EVENT_IDS_PATH {
                    *dedupe_state.borrow_mut() = Some(content.to_string());
                }
                Ok(())
            },
            || 1_000,
            |_event| handled.set(handled.get() + 1),
            |_level, _message| {},
        );
        let second = process_feishu_event_payload_with_workspace_auth(
            &body,
            FeishuEventAuthMode::AlreadyVerified,
            |path| {
                if path == RECENT_EVENT_IDS_PATH {
                    dedupe_state.borrow().clone()
                } else {
                    None
                }
            },
            |path, content| {
                if path == RECENT_EVENT_IDS_PATH {
                    *dedupe_state.borrow_mut() = Some(content.to_string());
                }
                Ok(())
            },
            || 1_001,
            |_event| handled.set(handled.get() + 1),
            |_level, _message| {},
        );

        assert!(first);
        assert!(!second);
        assert_eq!(handled.get(), 1);
    }

    #[test]
    fn queued_webhook_event_is_processed_as_already_verified() {
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt_123",
                "event_type": "im.message.receive_v1"
            },
            "event": {}
        })
        .to_string();
        let queue = std::cell::RefCell::new(serde_json::json!([body]).to_string());
        let handled = std::cell::RefCell::new(false);

        process_websocket_event_queue_with(
            || queue.borrow().clone(),
            |queue_json| {
                *queue.borrow_mut() = queue_json.to_string();
            },
            |frame| {
                process_feishu_event_payload_with_workspace_auth(
                    frame,
                    FeishuEventAuthMode::AlreadyVerified,
                    |path| (path == VERIFICATION_TOKEN_PATH).then(|| "expected".to_string()),
                    |_path, _content| Ok(()),
                    || 1_000,
                    |_event| {
                        *handled.borrow_mut() = true;
                    },
                    |_level, _message| {},
                );
            },
            None,
            |_level, _message| {},
        );

        assert_eq!(queue.borrow().as_str(), "[]");
        assert!(*handled.borrow());
    }

    #[test]
    fn process_feishu_event_payload_drops_websocket_event_with_mismatched_token() {
        let handled = std::cell::RefCell::new(false);
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt_123",
                "event_type": "im.message.receive_v1",
                "token": "wrong"
            },
            "event": {}
        })
        .to_string();

        let processed = process_feishu_event_payload_with_workspace(
            &body,
            false,
            |path| (path == VERIFICATION_TOKEN_PATH).then(|| "expected".to_string()),
            |_path, _content| Ok(()),
            || 1_000,
            |_event| {
                *handled.borrow_mut() = true;
            },
            |_level, _message| {},
        );

        assert!(!processed);
        assert!(!*handled.borrow());
    }

    #[test]
    fn request_verification_token_prefers_v2_header_token() {
        let event: FeishuEvent = serde_json::from_str(
            r#"{
                "schema": "2.0",
                "header": {
                    "event_id": "evt_123",
                    "event_type": "im.message.receive_v1",
                    "token": "header-token"
                },
                "event": {}
            }"#,
        )
        .unwrap();

        assert_eq!(request_verification_token(&event), Some("header-token"));
    }

    #[test]
    fn request_verification_token_falls_back_to_top_level_token() {
        let event: FeishuEvent = serde_json::from_str(
            r#"{
                "type": "url_verification",
                "challenge": "abc",
                "token": "top-level-token"
            }"#,
        )
        .unwrap();

        assert_eq!(request_verification_token(&event), Some("top-level-token"));
    }
}
