//! Discord Gateway/Webhook channel for IronClaw.
//!
//! This WASM component implements the channel interface for handling Discord
//! interactions via webhooks and sending messages back to Discord.
//!
//! # Features
//!
//! - URL verification for Discord interactions
//! - Slash command handling
//! - Message event parsing (@mentions, DMs)
//! - Thread support for conversations
//! - Response posting via Discord Web API
//!
//! # Security
//!
//! - Signature validation is handled by the host (webhook secrets)
//! - Bot token is injected by host during HTTP requests
//! - WASM never sees raw credentials

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

/// Discord interaction wrapper.
#[derive(Debug, Deserialize)]
struct DiscordInteraction {
    /// Interaction type (1=Ping, 2=ApplicationCommand, 3=MessageComponent)
    #[serde(rename = "type")]
    interaction_type: u8,

    /// Interaction ID
    id: String,

    /// Application ID
    application_id: String,

    /// Guild ID (if in server)
    guild_id: Option<String>,

    /// Channel ID
    channel_id: Option<String>,

    /// Member info (if in server)
    member: Option<DiscordMember>,

    /// User info (if DM)
    user: Option<DiscordUser>,

    /// Command data (for slash commands)
    data: Option<DiscordCommandData>,

    /// Message (for component interactions)
    message: Option<DiscordMessage>,

    /// Token for responding
    token: String,
}

#[derive(Debug, Deserialize)]
struct DiscordMember {
    user: DiscordUser,
    nick: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordUser {
    id: String,
    username: String,
    global_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordCommandData {
    id: String,
    name: String,
    options: Option<Vec<DiscordCommandOption>>,
}

#[derive(Debug, Deserialize)]
struct DiscordCommandOption {
    name: String,
    value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct DiscordMessage {
    id: String,
    content: String,
    channel_id: String,
    author: DiscordUser,
}

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
struct DiscordMessageMetadata {
    /// Discord channel ID
    channel_id: String,

    /// Interaction ID for followups
    interaction_id: String,

    /// Interaction token for responding
    token: String,

    /// Application ID
    application_id: String,

    /// Thread ID (for forum threads)
    thread_id: Option<String>,
}

/// Discord API response.
#[derive(Debug, Deserialize)]
struct DiscordApiResponse {
    id: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<u32>,
}

struct DiscordChannel;

impl Guest for DiscordChannel {
    fn on_start(_config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(channel_host::LogLevel::Info, "Discord channel starting");

        Ok(ChannelConfig {
            display_name: "Discord".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/discord".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: true,
            }],
            poll: None,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        let body_str = match std::str::from_utf8(&req.body) {
            Ok(s) => s,
            Err(_) => {
                return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
            }
        };

        let interaction: DiscordInteraction = match serde_json::from_str(body_str) {
            Ok(i) => i,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse Discord interaction: {}", e),
                );
                return json_response(400, serde_json::json!({"error": "Invalid interaction"}));
            }
        };

        match interaction.interaction_type {
            // Ping - Discord verification
            1 => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    "Responding to Discord ping",
                );
                json_response(200, serde_json::json!({"type": 1}))
            }

            // Application Command (slash command)
            2 => {
                handle_slash_command(interaction);
                json_response(200, serde_json::json!({
                    "type": 5,
                    "data": {
                        "content": "🤔 Thinking..."
                    }
                }))
            }

            // Message Component (buttons, selects)
            3 => {
                if let Some(message) = interaction.message {
                    handle_message_component(interaction, message);
                }
                json_response(200, serde_json::json!({"type": 6}))
            }

            _ => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Unknown Discord interaction type: {}", interaction.interaction_type),
                );
                json_response(200, serde_json::json!({"type": 6}))
            }
        }
    }

    fn on_poll() {}

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: DiscordMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Use webhook endpoint for followup
        let url = format!(
            "https://discord.com/api/v10/webhooks/{}/{}",
            metadata.application_id, metadata.token
        );

        let payload = serde_json::json!({
            "content": response.content,
        });

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| format!("Failed to serialize: {}", e))?;

        let headers = serde_json::json!({
            "Content-Type": "application/json"
        });

        let result = channel_host::http_request(
            "POST",
            &url,
            &headers.to_string(),
            Some(&payload_bytes),
            None,
        );

        match result {
            Ok(http_response) => {
                if http_response.status >= 200 && http_response.status < 300 {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        "Posted followup to Discord",
                    );
                    Ok(())
                } else {
                    let body_str = String::from_utf8_lossy(&http_response.body);
                    Err(format!("Discord API error: {} - {}", http_response.status, body_str))
                }
            }
            Err(e) => Err(format!("HTTP request failed: {}", e)),
        }
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Discord channel shutting down");
    }
}

fn handle_slash_command(interaction: DiscordInteraction) {
    let user = interaction.member.as_ref().map(|m| &m.user).or(interaction.user.as_ref());
    let user_id = user.map(|u| u.id.clone()).unwrap_or_default();
    let user_name = user.map(|u| u.global_name.as_ref().unwrap_or(&u.username).clone()).unwrap_or_default();

    let channel_id = interaction.channel_id.clone().unwrap_or_default();

    let command_name = interaction.data.as_ref().map(|d| d.name.clone()).unwrap_or_default();
    let options = interaction.data.as_ref().and_then(|d| d.options.clone());

    let content = if let Some(opts) = options {
        let opt_str = opts.iter()
            .map(|o| format!("{}: {}", o.name, o.value))
            .collect::<Vec<_>>()
            .join(", ");
        format!("/{} {}", command_name, opt_str)
    } else {
        format!("/{}", command_name)
    };

    let metadata = DiscordMessageMetadata {
        channel_id: channel_id.clone(),
        interaction_id: interaction.id.clone(),
        token: interaction.token.clone(),
        application_id: interaction.application_id.clone(),
        thread_id: None,
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: Some(user_name),
        content,
        thread_id: None,
        metadata_json,
    });
}

fn handle_message_component(interaction: DiscordInteraction, message: DiscordMessage) {
    // Check member first (for server contexts), then user (for DMs)
    let user = interaction.member.as_ref().map(|m| &m.user).or(interaction.user.as_ref());
    let user_id = user.map(|u| u.id.clone()).unwrap_or_default();
    let user_name = user.map(|u| u.global_name.as_ref().unwrap_or(&u.username).clone()).unwrap_or_default();

    let channel_id = message.channel_id.clone();

    let metadata = DiscordMessageMetadata {
        channel_id: channel_id.clone(),
        interaction_id: interaction.id.clone(),
        token: interaction.token.clone(),
        application_id: interaction.application_id.clone(),
        thread_id: None,
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: Some(user_name),
        content: format!("[Button clicked] {}", message.content),
        thread_id: None,
        metadata_json,
    });
}

fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let headers = serde_json::json!({"Content-Type": "application/json"});

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

export!(DiscordChannel);
