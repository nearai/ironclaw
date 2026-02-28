//! Slack Events API channel for IronClaw.
//!
//! This WASM component implements the channel interface for handling Slack
//! webhooks and sending messages back to Slack.
//!
//! # Features
//!
//! - URL verification for Slack Events API
//! - Message event parsing (@mentions, DMs)
//! - Thread support for conversations
//! - Response posting via Slack Web API
//!
//! # Security
//!
//! - Signature validation is handled by the host (webhook secrets)
//! - Bot token is injected by host during HTTP requests
//! - WASM never sees raw credentials

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
use near::agent::channel_host::{self, EmittedMessage};

/// Slack event wrapper.
#[derive(Debug, Deserialize)]
struct SlackEventWrapper {
    /// Event type (url_verification, event_callback, etc.)
    #[serde(rename = "type")]
    event_type: String,

    /// Challenge token for URL verification.
    challenge: Option<String>,

    /// The actual event payload (for event_callback).
    event: Option<SlackEvent>,

    /// Team ID that sent this event.
    team_id: Option<String>,

    /// Event ID for deduplication.
    event_id: Option<String>,
}

/// Slack event payload.
#[derive(Debug, Deserialize)]
struct SlackEvent {
    /// Event type (message, app_mention, etc.)
    #[serde(rename = "type")]
    event_type: String,

    /// User who triggered the event.
    user: Option<String>,

    /// Channel where the event occurred.
    channel: Option<String>,

    /// Message text.
    text: Option<String>,

    /// Thread timestamp (for threaded messages).
    thread_ts: Option<String>,

    /// Message timestamp.
    ts: Option<String>,

    /// Bot ID (if message is from a bot).
    bot_id: Option<String>,

    /// Subtype (bot_message, etc.)
    subtype: Option<String>,
}

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
struct SlackMessageMetadata {
    /// Slack channel ID.
    channel: String,

    /// Thread timestamp for threaded replies.
    thread_ts: Option<String>,

    /// Original message timestamp.
    message_ts: String,

    /// Team ID.
    team_id: Option<String>,
}

/// Slack API response for chat.postMessage.
#[derive(Debug, Deserialize)]
struct SlackPostMessageResponse {
    ok: bool,
    error: Option<String>,
    ts: Option<String>,
}

/// Workspace path for persisting owner_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_id";
/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";
/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";
/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "slack";

/// Channel configuration from capabilities file.
#[derive(Debug, Deserialize)]
struct SlackConfig {
    /// Name of secret containing signing secret (for verification by host).
    #[serde(default = "default_signing_secret_name")]
    #[allow(dead_code)]
    signing_secret_name: String,

    #[serde(default)]
    owner_id: Option<String>,

    #[serde(default)]
    dm_policy: Option<String>,

    #[serde(default)]
    allow_from: Option<Vec<String>>,
}

fn default_signing_secret_name() -> String {
    "slack_signing_secret".to_string()
}

struct SlackChannel;

impl Guest for SlackChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: SlackConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Slack channel starting");

        // Persist owner_id so subsequent callbacks can read it
        if let Some(ref owner_id) = config.owner_id {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, owner_id);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: user {}", owner_id),
            );
        } else {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
        }

        // Persist dm_policy and allow_from for DM pairing
        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing");
        let _ = channel_host::workspace_write(DM_POLICY_PATH, dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        Ok(ChannelConfig {
            display_name: "Slack".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/slack".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: true,
            }],
            poll: None,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Parse the request body
        let body_str = match std::str::from_utf8(&req.body) {
            Ok(s) => s,
            Err(_) => {
                return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
            }
        };

        // Parse as Slack event
        let event_wrapper: SlackEventWrapper = match serde_json::from_str(body_str) {
            Ok(e) => e,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse Slack event: {}", e),
                );
                return json_response(400, serde_json::json!({"error": "Invalid event payload"}));
            }
        };

        match event_wrapper.event_type.as_str() {
            // URL verification challenge (Slack setup)
            "url_verification" => {
                if let Some(challenge) = event_wrapper.challenge {
                    channel_host::log(
                        channel_host::LogLevel::Info,
                        "Responding to Slack URL verification",
                    );
                    json_response(200, serde_json::json!({"challenge": challenge}))
                } else {
                    json_response(400, serde_json::json!({"error": "Missing challenge"}))
                }
            }

            // Actual event callback
            "event_callback" => {
                if let Some(event) = event_wrapper.event {
                    handle_slack_event(event, event_wrapper.team_id, event_wrapper.event_id);
                }
                // Always respond 200 quickly to Slack (they have a 3s timeout)
                json_response(200, serde_json::json!({"ok": true}))
            }

            // Unknown event type
            _ => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Unknown Slack event type: {}", event_wrapper.event_type),
                );
                json_response(200, serde_json::json!({"ok": true}))
            }
        }
    }

    fn on_poll() {
        // Slack uses webhooks, no polling needed
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        // Parse metadata to get channel info
        let metadata: SlackMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Build Slack API request
        let mut payload = serde_json::json!({
            "channel": metadata.channel,
            "text": response.content,
        });

        // Add thread_ts for threaded replies
        if let Some(thread_ts) = response.thread_id.or(metadata.thread_ts) {
            payload["thread_ts"] = serde_json::Value::String(thread_ts);
        }

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| format!("Failed to serialize payload: {}", e))?;

        // Make HTTP request to Slack API
        // The bot token is injected by the host based on credential configuration
        let headers = serde_json::json!({
            "Content-Type": "application/json"
        });

        let result = channel_host::http_request(
            "POST",
            "https://slack.com/api/chat.postMessage",
            &headers.to_string(),
            Some(&payload_bytes),
            None,
        );

        match result {
            Ok(http_response) => {
                if http_response.status != 200 {
                    return Err(format!(
                        "Slack API returned status {}",
                        http_response.status
                    ));
                }

                // Parse Slack response
                let slack_response: SlackPostMessageResponse =
                    serde_json::from_slice(&http_response.body)
                        .map_err(|e| format!("Failed to parse Slack response: {}", e))?;

                if !slack_response.ok {
                    return Err(format!(
                        "Slack API error: {}",
                        slack_response
                            .error
                            .unwrap_or_else(|| "unknown".to_string())
                    ));
                }

                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Posted message to Slack channel {}: ts={}",
                        metadata.channel,
                        slack_response.ts.unwrap_or_default()
                    ),
                );

                Ok(())
            }
            Err(e) => Err(format!("HTTP request failed: {}", e)),
        }
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_broadcast(_user_id: String, _response: AgentResponse) -> Result<(), String> {
        Ok(()) // Not yet implemented
    }

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Slack channel shutting down");
    }
}

/// Handle a Slack event and emit message if applicable.
fn handle_slack_event(event: SlackEvent, team_id: Option<String>, _event_id: Option<String>) {
    match event.event_type.as_str() {
        // Direct mention of the bot (always in a channel, not a DM)
        "app_mention" => {
            if let (Some(user), Some(channel), Some(text), Some(ts)) = (
                event.user,
                event.channel.clone(),
                event.text,
                event.ts.clone(),
            ) {
                // app_mention is always in a channel (not DM)
                if !check_sender_permission(&user, &channel, false) {
                    return;
                }
                emit_message(user, text, channel, event.thread_ts.or(Some(ts)), team_id);
            }
        }

        // Direct message to the bot
        "message" => {
            // Skip messages from bots (including ourselves)
            if event.bot_id.is_some() || event.subtype.is_some() {
                return;
            }

            if let (Some(user), Some(channel), Some(text), Some(ts)) = (
                event.user,
                event.channel.clone(),
                event.text,
                event.ts.clone(),
            ) {
                // Only process DMs (channel IDs starting with D)
                if channel.starts_with('D') {
                    if !check_sender_permission(&user, &channel, true) {
                        return;
                    }
                    emit_message(user, text, channel, event.thread_ts.or(Some(ts)), team_id);
                }
            }
        }

        _ => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Ignoring Slack event type: {}", event.event_type),
            );
        }
    }
}

/// Emit a message to the agent.
fn emit_message(
    user_id: String,
    text: String,
    channel: String,
    thread_ts: Option<String>,
    team_id: Option<String>,
) {
    let message_ts = thread_ts.clone().unwrap_or_default();

    let metadata = SlackMessageMetadata {
        channel: channel.clone(),
        thread_ts: thread_ts.clone(),
        message_ts: message_ts.clone(),
        team_id,
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|e| {
        channel_host::log(
            channel_host::LogLevel::Error,
            &format!("Failed to serialize Slack metadata: {}", e),
        );
        "{}".to_string()
    });

    // Strip @ mentions of the bot from the text for cleaner messages
    let cleaned_text = strip_bot_mention(&text);

    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: None, // Could fetch from Slack API if needed
        content: cleaned_text,
        thread_id: thread_ts,
        metadata_json,
    });
}

// ============================================================================
// Permission & Pairing
// ============================================================================

/// Check if a sender is permitted. Returns true if allowed.
/// For pairing mode, sends a pairing code DM if denied.
fn check_sender_permission(user_id: &str, channel_id: &str, is_dm: bool) -> bool {
    // 1. Owner check (highest priority, applies to all contexts)
    let owner_id = channel_host::workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());
    if let Some(ref owner) = owner_id {
        if user_id != owner {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!(
                    "Dropping message from non-owner user {} (owner: {})",
                    user_id, owner
                ),
            );
            return false;
        }
        return true;
    }

    // 2. DM policy (only for DMs when no owner_id)
    if !is_dm {
        return true; // Channel messages bypass DM policy
    }

    let dm_policy =
        channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

    if dm_policy == "open" {
        return true;
    }

    // 3. Build merged allow list: config allow_from + pairing store
    let mut allowed: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    if let Ok(store_allowed) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        allowed.extend(store_allowed);
    }

    // 4. Check sender (Slack events only have user ID, not username)
    let is_allowed =
        allowed.contains(&"*".to_string()) || allowed.contains(&user_id.to_string());

    if is_allowed {
        return true;
    }

    // 5. Not allowed — handle by policy
    if dm_policy == "pairing" {
        let meta = serde_json::json!({
            "user_id": user_id,
            "channel_id": channel_id,
        })
        .to_string();

        match channel_host::pairing_upsert_request(CHANNEL_NAME, user_id, &meta) {
            Ok(result) => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!(
                        "Pairing request for user {}: code {}",
                        user_id, result.code
                    ),
                );
                if result.created {
                    let _ = send_pairing_reply(channel_id, &result.code);
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
    false
}

/// Send a pairing code message via Slack chat.postMessage.
fn send_pairing_reply(channel_id: &str, code: &str) -> Result<(), String> {
    let payload = serde_json::json!({
        "channel": channel_id,
        "text": format!(
            "To pair with this bot, run: `ironclaw pairing approve slack {}`",
            code
        ),
    });

    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("Failed to serialize: {}", e))?;

    let headers = serde_json::json!({"Content-Type": "application/json"});

    let result = channel_host::http_request(
        "POST",
        "https://slack.com/api/chat.postMessage",
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(response) if response.status == 200 => Ok(()),
        Ok(response) => {
            let body_str = String::from_utf8_lossy(&response.body);
            Err(format!(
                "Slack API error: {} - {}",
                response.status, body_str
            ))
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

/// Strip leading bot mention from text.
fn strip_bot_mention(text: &str) -> String {
    // Slack mentions look like <@U12345678>
    let trimmed = text.trim();
    if trimmed.starts_with("<@") {
        if let Some(end) = trimmed.find('>') {
            return trimmed[end + 1..].trim_start().to_string();
        }
    }
    trimmed.to_string()
}

/// Create a JSON HTTP response.
fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_else(|e| {
        channel_host::log(
            channel_host::LogLevel::Error,
            &format!("Failed to serialize JSON response: {}", e),
        );
        Vec::new()
    });
    let headers = serde_json::json!({"Content-Type": "application/json"});

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

// Export the component
export!(SlackChannel);
