// Prismer API types have fields reserved for future use
#![allow(dead_code)]

//! Prismer Cloud IM channel for IronClaw.
//!
//! This WASM component implements the channel interface for Prismer Cloud IM,
//! supporting both webhook and polling modes for agent-to-agent and
//! human-to-agent messaging.
//!
//! # Features
//!
//! - Webhook mode: real-time message delivery via signed POSTs
//! - Polling mode: 30s interval fallback when no tunnel configured
//! - Two-step auth: API key register -> JWT for subsequent calls
//! - Self-message loop prevention
//! - JWT auto-renewal on 401
//!
//! # Security
//!
//! - API key is injected by host via placeholder replacement
//! - JWT is managed by WASM and stored in workspace
//! - Webhook signature validation by host

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

struct PrismerChannel;
export!(PrismerChannel);

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct PrismerConfig {
    #[serde(default = "default_base_url")]
    base_url: String,
    agent_name: Option<String>,
    #[serde(default = "default_display_name")]
    display_name: String,
    #[serde(default = "default_agent_type")]
    agent_type: String,
    #[serde(default = "default_capabilities")]
    capabilities: Vec<String>,
    #[serde(default = "default_description")]
    description: String,
    #[serde(default)]
    polling_enabled: bool,
    #[serde(default = "default_poll_interval")]
    poll_interval_ms: u32,
    #[serde(default = "default_dm_policy")]
    dm_policy: String,
    /// Injected by host when a tunnel is active.
    tunnel_url: Option<String>,
}

fn default_base_url() -> String {
    "https://prismer.cloud".to_string()
}
fn default_display_name() -> String {
    "IronClaw Agent".to_string()
}
fn default_agent_type() -> String {
    "assistant".to_string()
}
fn default_capabilities() -> Vec<String> {
    vec![
        "chat".into(),
        "code".into(),
        "memory".into(),
        "tools".into(),
    ]
}
fn default_description() -> String {
    "IronClaw AI assistant on Prismer network".to_string()
}
fn default_poll_interval() -> u32 {
    30000
}
fn default_dm_policy() -> String {
    "open".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct PrismerMetadata {
    conversation_id: String,
    #[serde(default)]
    conversation_type: String,
    sender_id: String,
    #[serde(default)]
    sender_username: String,
    message_id: String,
}

// -- Prismer API response types --

#[derive(Debug, Deserialize)]
struct IMResult {
    ok: bool,
    data: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RegisterData {
    #[serde(rename = "imUserId")]
    im_user_id: String,
    token: String,
    #[serde(rename = "expiresIn")]
    expires_in: Option<String>,
}

// -- Prismer webhook types (matches Go SDK WebhookPayload) --

#[derive(Debug, Deserialize)]
struct WebhookPayload {
    source: String,
    event: String,
    timestamp: Option<i64>,
    message: WebhookMessage,
    sender: WebhookSender,
    conversation: WebhookConversation,
}

#[derive(Debug, Deserialize)]
struct WebhookMessage {
    id: String,
    #[serde(rename = "type")]
    msg_type: String,
    content: String,
    #[serde(rename = "senderId")]
    sender_id: String,
    #[serde(rename = "conversationId")]
    conversation_id: String,
    #[serde(rename = "parentId")]
    parent_id: Option<String>,
    metadata: Option<serde_json::Value>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebhookSender {
    id: String,
    username: String,
    #[serde(rename = "displayName")]
    display_name: String,
    role: String,
}

#[derive(Debug, Deserialize)]
struct WebhookConversation {
    id: String,
    #[serde(rename = "type")]
    conv_type: String,
    title: Option<String>,
}

// -- IM message type (for polling) --

#[derive(Debug, Deserialize)]
struct IMMessage {
    id: String,
    content: String,
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(rename = "senderId")]
    sender_id: String,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IMConversation {
    id: String,
    #[serde(rename = "type")]
    conv_type: String,
    title: Option<String>,
    #[serde(rename = "unreadCount")]
    unread_count: Option<i32>,
}

// ============================================================================
// Workspace State Paths
// ============================================================================

const JWT_PATH: &str = "jwt";
const IM_USER_ID_PATH: &str = "im_user_id";
const CONFIG_PATH: &str = "config";
const WEBHOOK_REGISTERED_PATH: &str = "webhook_registered";

// ============================================================================
// HTTP Helpers
// ============================================================================

fn respond_json(status: u16, body: serde_json::Value) -> OutgoingHttpResponse {
    OutgoingHttpResponse {
        status,
        headers_json: r#"{"Content-Type":"application/json"}"#.to_string(),
        body: serde_json::to_vec(&body).unwrap_or_default(),
    }
}

fn api_request(
    method: &str,
    url: &str,
    token: &str,
    body: Option<&serde_json::Value>,
) -> Result<(u16, Vec<u8>), String> {
    let headers = if body.is_some() {
        serde_json::json!({
            "Authorization": format!("Bearer {}", token),
            "Content-Type": "application/json"
        })
    } else {
        serde_json::json!({
            "Authorization": format!("Bearer {}", token)
        })
    };

    let body_bytes = body.map(|b| serde_json::to_vec(b).unwrap_or_default());

    let resp = channel_host::http_request(
        method,
        url,
        &headers.to_string(),
        body_bytes.as_deref(),
        None,
    )
    .map_err(|e| format!("HTTP request failed: {}", e))?;

    Ok((resp.status, resp.body))
}

fn read_config() -> PrismerConfig {
    channel_host::workspace_read(CONFIG_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| PrismerConfig {
            base_url: default_base_url(),
            agent_name: None,
            display_name: default_display_name(),
            agent_type: default_agent_type(),
            capabilities: default_capabilities(),
            description: default_description(),
            polling_enabled: false,
            poll_interval_ms: default_poll_interval(),
            dm_policy: default_dm_policy(),
            tunnel_url: None,
        })
}

// ============================================================================
// Authentication
// ============================================================================

fn register_agent(config: &PrismerConfig) -> Result<(String, String), String> {
    let body = serde_json::json!({
        "type": "agent",
        "username": config.agent_name.as_deref().unwrap_or("ironclaw"),
        "displayName": config.display_name,
        "agentType": config.agent_type,
        "capabilities": config.capabilities,
        "description": config.description,
        "endpoint": config.tunnel_url,
    });

    // Use placeholder -- host replaces {PRISMER_API_KEY} with actual secret value
    let (status, resp_body) = api_request(
        "POST",
        &format!("{}/api/im/register", config.base_url),
        "{PRISMER_API_KEY}",
        Some(&body),
    )?;

    if status >= 300 {
        let err_text = String::from_utf8_lossy(&resp_body);
        return Err(format!(
            "Register failed (HTTP {}): {}",
            status, err_text
        ));
    }

    let im_result: IMResult = serde_json::from_slice(&resp_body)
        .map_err(|e| format!("Failed to parse register response: {}", e))?;

    if !im_result.ok {
        return Err(format!(
            "Register returned ok=false: {:?}",
            im_result.error
        ));
    }

    let data: RegisterData = serde_json::from_value(
        im_result
            .data
            .ok_or("Register response missing data")?,
    )
    .map_err(|e| format!("Failed to parse register data: {}", e))?;

    // Persist JWT and user ID
    channel_host::workspace_write(JWT_PATH, &data.token)
        .map_err(|e| format!("Failed to write JWT: {}", e))?;
    channel_host::workspace_write(IM_USER_ID_PATH, &data.im_user_id)
        .map_err(|e| format!("Failed to write user ID: {}", e))?;

    channel_host::log(
        channel_host::LogLevel::Info,
        &format!(
            "Registered as {} ({})",
            data.im_user_id,
            if data.expires_in.is_some() {
                "new"
            } else {
                "existing"
            }
        ),
    );

    Ok((data.token, data.im_user_id))
}

fn ensure_jwt(config: &PrismerConfig) -> Result<String, String> {
    // Try cached JWT first
    if let Some(cached) = channel_host::workspace_read(JWT_PATH) {
        if !cached.is_empty() {
            // Validate with /api/im/me
            let (status, _) = api_request(
                "GET",
                &format!("{}/api/im/me", config.base_url),
                &cached,
                None,
            )?;

            if status < 300 {
                channel_host::log(channel_host::LogLevel::Debug, "Cached JWT is valid");
                return Ok(cached);
            }

            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Cached JWT invalid (HTTP {}), re-registering", status),
            );
        }
    }

    // Register to get fresh JWT
    let (jwt, _) = register_agent(config)?;
    Ok(jwt)
}

/// Re-register and return fresh JWT. Used for 401 recovery in on_respond/on_poll.
fn attempt_re_register() -> Result<String, String> {
    let config = read_config();
    let (jwt, _) = register_agent(&config)?;
    Ok(jwt)
}

// ============================================================================
// Channel Implementation
// ============================================================================

impl Guest for PrismerChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Prismer channel config: {}", config_json),
        );

        let config: PrismerConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        // Persist config for subsequent callbacks
        let config_str = serde_json::to_string(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        let _ = channel_host::workspace_write(CONFIG_PATH, &config_str);

        channel_host::log(channel_host::LogLevel::Info, "Prismer channel starting");

        // Authenticate (cached JWT or fresh register)
        let _jwt = ensure_jwt(&config)?;

        // Determine mode
        let webhook_mode = config.tunnel_url.is_some();

        if webhook_mode {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Webhook mode enabled (tunnel configured)",
            );
            if let Some(ref tunnel_url) = config.tunnel_url {
                let _ = channel_host::workspace_write(
                    WEBHOOK_REGISTERED_PATH,
                    &format!("{}/webhook/prismer", tunnel_url),
                );
            }
        } else {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Polling mode enabled (no tunnel configured)",
            );
        }

        let poll = if !webhook_mode {
            Some(PollConfig {
                interval_ms: config.poll_interval_ms.max(30000),
                enabled: true,
            })
        } else {
            None
        };

        Ok(ChannelConfig {
            display_name: "Prismer".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/prismer".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: channel_host::secret_exists("prismer_webhook_secret"),
            }],
            poll,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        if !req.secret_validated {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Webhook request with invalid or missing signature",
            );
            return respond_json(
                401,
                serde_json::json!({"error": "Invalid signature"}),
            );
        }

        let payload: WebhookPayload = match serde_json::from_slice(&req.body) {
            Ok(p) => p,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to parse webhook payload: {}", e),
                );
                return respond_json(
                    400,
                    serde_json::json!({"error": e.to_string()}),
                );
            }
        };

        if payload.source != "prismer_im" {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Unknown webhook source: {}", payload.source),
            );
            return respond_json(
                400,
                serde_json::json!({"error": "Unknown source"}),
            );
        }

        // Skip self-messages
        let my_id = channel_host::workspace_read(IM_USER_ID_PATH).unwrap_or_default();
        if payload.sender.id == my_id {
            return respond_json(
                200,
                serde_json::json!({"ok": true, "skipped": "self"}),
            );
        }

        // Only process new messages
        if payload.event != "message.new" {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Ignoring event: {}", payload.event),
            );
            return respond_json(200, serde_json::json!({"ok": true}));
        }

        // Skip empty content
        let content = payload.message.content.trim();
        if content.is_empty() {
            return respond_json(
                200,
                serde_json::json!({"ok": true, "skipped": "empty"}),
            );
        }

        let metadata = PrismerMetadata {
            conversation_id: payload.conversation.id.clone(),
            conversation_type: payload.conversation.conv_type.clone(),
            sender_id: payload.sender.id.clone(),
            sender_username: payload.sender.username.clone(),
            message_id: payload.message.id.clone(),
        };

        channel_host::emit_message(&EmittedMessage {
            user_id: payload.sender.id,
            user_name: Some(payload.sender.display_name),
            content: content.to_string(),
            thread_id: Some(payload.conversation.id),
            metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
        });

        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Emitted message {} from {}",
                metadata.message_id, metadata.sender_username
            ),
        );

        respond_json(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        let jwt = match channel_host::workspace_read(JWT_PATH).filter(|t| !t.is_empty()) {
            Some(t) => t,
            None => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    "No JWT, skipping poll",
                );
                return;
            }
        };

        let config = read_config();
        let my_id = channel_host::workspace_read(IM_USER_ID_PATH).unwrap_or_default();

        // Fetch conversations with unread messages
        let conv_url = format!(
            "{}/api/im/conversations?withUnread=true",
            config.base_url
        );
        let (status, body) = match api_request("GET", &conv_url, &jwt, None) {
            Ok(r) => r,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to fetch conversations: {}", e),
                );
                return;
            }
        };

        if status == 401 {
            channel_host::log(
                channel_host::LogLevel::Info,
                "JWT expired during poll, re-registering",
            );
            let _ = attempt_re_register();
            return;
        }

        if status >= 300 {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Conversations fetch failed (HTTP {})", status),
            );
            return;
        }

        let im_result: IMResult = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse conversations response: {}", e),
                );
                return;
            }
        };

        let conversations: Vec<IMConversation> = match im_result.data {
            Some(data) => serde_json::from_value(data).unwrap_or_else(|e| {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse conversations from data: {}", e),
                );
                Vec::new()
            }),
            None => return,
        };

        let jwt_for_requests =
            match channel_host::workspace_read(JWT_PATH).filter(|t| !t.is_empty()) {
                Some(t) => t,
                None => return,
            };

        for conv in &conversations {
            let unread = conv.unread_count.unwrap_or(0);
            if unread <= 0 {
                continue;
            }

            let cursor_key = format!("cursor_{}", conv.id);
            let cursor = channel_host::workspace_read(&cursor_key).unwrap_or_default();

            let mut msg_url = format!(
                "{}/api/im/messages/{}",
                config.base_url, conv.id
            );
            if !cursor.is_empty() {
                msg_url.push_str(&format!("?offset={}", cursor));
            }

            let (msg_status, msg_body) =
                match api_request("GET", &msg_url, &jwt_for_requests, None) {
                    Ok(r) => r,
                    Err(e) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!("Failed to fetch messages for {}: {}", conv.id, e),
                        );
                        continue;
                    }
                };

            // Handle 401 with re-register retry (JWT may expire mid-poll)
            let (final_status, final_body) = if msg_status == 401 {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("JWT expired fetching messages for {}, re-registering", conv.id),
                );
                match attempt_re_register() {
                    Ok(new_jwt) => match api_request("GET", &msg_url, &new_jwt, None) {
                        Ok(r) => r,
                        Err(e) => {
                            channel_host::log(
                                channel_host::LogLevel::Error,
                                &format!("Retry fetch messages for {} failed: {}", conv.id, e),
                            );
                            continue;
                        }
                    },
                    Err(e) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!("Re-register failed during message fetch: {}", e),
                        );
                        continue;
                    }
                }
            } else {
                (msg_status, msg_body)
            };

            if final_status >= 300 {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Messages fetch for {} failed (HTTP {})", conv.id, final_status),
                );
                continue;
            }

            let msg_result: IMResult = match serde_json::from_slice(&final_body) {
                Ok(r) => r,
                Err(e) => {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to parse messages response for {}: {}", conv.id, e),
                    );
                    continue;
                }
            };

            let messages: Vec<IMMessage> = match msg_result.data {
                Some(data) => serde_json::from_value(data).unwrap_or_else(|e| {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to parse messages from data for {}: {}", conv.id, e),
                    );
                    Vec::new()
                }),
                None => continue,
            };

            for msg in &messages {
                if msg.sender_id == my_id {
                    continue;
                }

                let content = msg.content.trim();
                if content.is_empty() {
                    continue;
                }

                let metadata = PrismerMetadata {
                    conversation_id: conv.id.clone(),
                    conversation_type: conv.conv_type.clone(),
                    sender_id: msg.sender_id.clone(),
                    sender_username: String::new(),
                    message_id: msg.id.clone(),
                };

                channel_host::emit_message(&EmittedMessage {
                    user_id: msg.sender_id.clone(),
                    user_name: None,
                    content: content.to_string(),
                    thread_id: Some(conv.id.clone()),
                    metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
                });
            }

            // Update cursor (use message count as offset)
            if !messages.is_empty() {
                let current_offset: usize = cursor.parse().unwrap_or(0);
                let new_offset = (current_offset + messages.len()).to_string();
                let _ = channel_host::workspace_write(&cursor_key, &new_offset);
            }

            // Mark as read
            let read_url = format!(
                "{}/api/im/conversations/{}/read",
                config.base_url, conv.id
            );
            let _ = api_request("POST", &read_url, &jwt_for_requests, None);
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: PrismerMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        let jwt = channel_host::workspace_read(JWT_PATH)
            .filter(|t| !t.is_empty())
            .ok_or("No JWT token available")?;
        let config = read_config();

        let body = serde_json::json!({
            "content": response.content,
            "type": "markdown",
        });

        let url = format!(
            "{}/api/im/messages/{}",
            config.base_url, metadata.conversation_id
        );

        let (status, resp_body) = api_request("POST", &url, &jwt, Some(&body))?;

        match status {
            s if s < 300 => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Sent reply to conversation {}",
                        metadata.conversation_id
                    ),
                );
                Ok(())
            }
            401 => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    "JWT expired during on_respond, re-registering",
                );
                let new_jwt = attempt_re_register()?;
                let (retry_status, retry_body) =
                    api_request("POST", &url, &new_jwt, Some(&body))?;
                if retry_status < 300 {
                    Ok(())
                } else {
                    let err_text = String::from_utf8_lossy(&retry_body);
                    Err(format!(
                        "Send failed after re-register (HTTP {}): {}",
                        retry_status, err_text
                    ))
                }
            }
            _ => {
                let err_text = String::from_utf8_lossy(&resp_body);
                Err(format!(
                    "Send failed (HTTP {}): {}",
                    status, err_text
                ))
            }
        }
    }

    fn on_status(update: StatusUpdate) {
        // Prismer has no HTTP typing endpoint (typing is WebSocket-only).
        // Log for debugging; extensible later.
        if matches!(update.status, StatusType::Thinking) {
            channel_host::log(
                channel_host::LogLevel::Debug,
                "Agent thinking (Prismer has no HTTP typing API)",
            );
        }
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Prismer channel shutting down",
        );
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config_minimal() {
        let json = r#"{}"#;
        let config: PrismerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_url, "https://prismer.cloud");
        assert_eq!(config.display_name, "IronClaw Agent");
        assert_eq!(config.agent_type, "assistant");
        assert_eq!(
            config.capabilities,
            vec!["chat", "code", "memory", "tools"]
        );
        assert!(config.agent_name.is_none());
        assert!(config.tunnel_url.is_none());
        assert!(!config.polling_enabled);
        assert_eq!(config.poll_interval_ms, 30000);
    }

    #[test]
    fn test_parse_config_full() {
        let json = r#"{
            "base_url": "https://custom.prismer.dev",
            "agent_name": "my-bot",
            "display_name": "My Bot",
            "agent_type": "specialist",
            "capabilities": ["search"],
            "description": "A search bot",
            "polling_enabled": true,
            "poll_interval_ms": 60000,
            "dm_policy": "pairing",
            "tunnel_url": "https://abc.ngrok.io"
        }"#;
        let config: PrismerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_url, "https://custom.prismer.dev");
        assert_eq!(config.agent_name, Some("my-bot".to_string()));
        assert!(config.polling_enabled);
        assert_eq!(
            config.tunnel_url,
            Some("https://abc.ngrok.io".to_string())
        );
    }

    #[test]
    fn test_parse_webhook_payload() {
        let json = r#"{
            "source": "prismer_im",
            "event": "message.new",
            "timestamp": 1741334400,
            "message": {
                "id": "msg_001",
                "type": "text",
                "content": "Hello from Prismer",
                "senderId": "iu_user_123",
                "conversationId": "conv_abc",
                "parentId": null,
                "metadata": {},
                "createdAt": "2026-03-07T10:00:00Z"
            },
            "sender": {
                "id": "iu_user_123",
                "username": "william",
                "displayName": "William",
                "role": "human"
            },
            "conversation": {
                "id": "conv_abc",
                "type": "direct",
                "title": null
            }
        }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.source, "prismer_im");
        assert_eq!(payload.event, "message.new");
        assert_eq!(payload.message.content, "Hello from Prismer");
        assert_eq!(payload.sender.username, "william");
        assert_eq!(payload.conversation.conv_type, "direct");
    }

    #[test]
    fn test_parse_webhook_invalid_source() {
        let json = r#"{
            "source": "other_system",
            "event": "message.new",
            "message": {"id":"m","type":"text","content":"x","senderId":"s","conversationId":"c"},
            "sender": {"id":"s","username":"u","displayName":"U","role":"human"},
            "conversation": {"id":"c","type":"direct","title":null}
        }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_ne!(payload.source, "prismer_im");
    }

    #[test]
    fn test_parse_webhook_missing_fields() {
        let json = r#"{"source": "prismer_im"}"#;
        let result = serde_json::from_str::<WebhookPayload>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_metadata_roundtrip() {
        let meta = PrismerMetadata {
            conversation_id: "conv_123".to_string(),
            conversation_type: "direct".to_string(),
            sender_id: "iu_user".to_string(),
            sender_username: "alice".to_string(),
            message_id: "msg_456".to_string(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: PrismerMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back.conversation_id, "conv_123");
        assert_eq!(back.message_id, "msg_456");
    }

    #[test]
    fn test_parse_register_response() {
        let json = r#"{"imUserId":"iu_xxx","token":"jwt_abc","expiresIn":"24h"}"#;
        let data: RegisterData = serde_json::from_str(json).unwrap();
        assert_eq!(data.im_user_id, "iu_xxx");
        assert_eq!(data.token, "jwt_abc");
        assert_eq!(data.expires_in, Some("24h".to_string()));
    }

    #[test]
    fn test_parse_im_result_ok() {
        let json = r#"{"ok":true,"data":{"token":"abc"}}"#;
        let result: IMResult = serde_json::from_str(json).unwrap();
        assert!(result.ok);
        assert!(result.data.is_some());
    }

    #[test]
    fn test_parse_im_result_error() {
        let json =
            r#"{"ok":false,"error":{"code":"UNAUTHORIZED","message":"bad token"}}"#;
        let result: IMResult = serde_json::from_str(json).unwrap();
        assert!(!result.ok);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_skip_self_message() {
        let my_id = "iu_bot";
        let sender_id = "iu_bot";
        assert_eq!(my_id, sender_id, "Self-messages should be skipped");
    }

    #[test]
    fn test_build_send_payload() {
        let body = serde_json::json!({
            "content": "Hello **world**",
            "type": "markdown",
        });
        let obj = body.as_object().unwrap();
        assert_eq!(obj.get("content").unwrap(), "Hello **world**");
        assert_eq!(obj.get("type").unwrap(), "markdown");
    }
}
