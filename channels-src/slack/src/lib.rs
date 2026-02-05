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
    OutgoingHttpResponse, PollConfig,
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

/// Channel configuration from capabilities file.
#[derive(Debug, Deserialize)]
struct SlackConfig {
    /// Name of secret containing signing secret (for verification by host).
    #[serde(default = "default_signing_secret_name")]
    signing_secret_name: String,
}

fn default_signing_secret_name() -> String {
    "slack_signing_secret".to_string()
}

struct SlackChannel;

impl Guest for SlackChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        // Parse configuration
        let _config: SlackConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Slack channel starting");

        Ok(ChannelConfig {
            display_name: "Slack".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/slack".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: true,
            }],
            poll: None, // Slack uses push via webhooks, no polling needed
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
                    handle_slack_event(
                        event,
                        event_wrapper.team_id,
                        event_wrapper.event_id,
                    );
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
                    serde_json::from_slice(&http_response.body).map_err(|e| {
                        format!("Failed to parse Slack response: {}", e)
                    })?;

                if !slack_response.ok {
                    return Err(format!(
                        "Slack API error: {}",
                        slack_response.error.unwrap_or_else(|| "unknown".to_string())
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

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Slack channel shutting down");
    }
}

/// Handle a Slack event and emit message if applicable.
fn handle_slack_event(
    event: SlackEvent,
    team_id: Option<String>,
    _event_id: Option<String>,
) {
    match event.event_type.as_str() {
        // Direct mention of the bot
        "app_mention" => {
            if let (Some(user), Some(channel), Some(text), Some(ts)) =
                (event.user, event.channel.clone(), event.text, event.ts.clone())
            {
                emit_message(user, text, channel, event.thread_ts.or(Some(ts)), team_id);
            }
        }

        // Direct message to the bot
        "message" => {
            // Skip messages from bots (including ourselves)
            if event.bot_id.is_some() || event.subtype.is_some() {
                return;
            }

            if let (Some(user), Some(channel), Some(text), Some(ts)) =
                (event.user, event.channel.clone(), event.text, event.ts.clone())
            {
                // Only process DMs (channel IDs starting with D)
                if channel.starts_with('D') {
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

    let metadata_json =
        serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Strip @ mentions of the bot from the text for cleaner messages
    let cleaned_text = strip_bot_mention(&text);

    channel_host::emit_message(EmittedMessage {
        user_id,
        user_name: None, // Could fetch from Slack API if needed
        content: cleaned_text,
        thread_id: thread_ts,
        metadata_json,
    });
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
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let headers = serde_json::json!({"Content-Type": "application/json"});

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

// Export the component
export!(SlackChannel);
