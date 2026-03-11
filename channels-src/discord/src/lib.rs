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
//! - Markdown attachment fallback for oversized replies
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
    OutgoingHttpResponse, StatusType, StatusUpdate,
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
    #[allow(dead_code)] // Part of API payload, currently unused
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

#[derive(Debug, Deserialize, Clone)]
struct DiscordMember {
    user: DiscordUser,
    #[allow(dead_code)] // Part of API payload, currently unused
    nick: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct DiscordUser {
    id: String,
    username: String,
    global_name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct DiscordCommandData {
    #[allow(dead_code)] // Part of API payload, currently unused
    id: String,
    name: String,
    options: Option<Vec<DiscordCommandOption>>,
}

#[derive(Debug, Deserialize, Clone)]
struct DiscordCommandOption {
    name: String,
    value: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone)]
struct DiscordMessage {
    #[allow(dead_code)] // Part of API payload, currently unused
    id: String,
    content: String,
    channel_id: String,
    #[allow(dead_code)] // Part of API payload, currently unused
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

#[derive(Debug, PartialEq, Eq)]
enum DiscordResponseRoute {
    InteractionWebhook(String),
    ChannelMessage(String),
}

fn response_route_for_metadata(metadata: &DiscordMessageMetadata) -> DiscordResponseRoute {
    if !metadata.application_id.is_empty() && !metadata.token.is_empty() {
        DiscordResponseRoute::InteractionWebhook(format!(
            "https://discord.com/api/v10/webhooks/{}/{}",
            metadata.application_id, metadata.token
        ))
    } else {
        DiscordResponseRoute::ChannelMessage(format!(
            "https://discord.com/api/v10/channels/{}/messages",
            metadata.channel_id
        ))
    }
}

fn typing_request_url_for_update(update: &StatusUpdate) -> Option<String> {
    if update.status != StatusType::Thinking {
        return None;
    }

    let metadata: DiscordMessageMetadata = serde_json::from_str(&update.metadata_json).ok()?;
    if metadata.channel_id.is_empty() {
        return None;
    }

    Some(format!(
        "https://discord.com/api/v10/channels/{}/typing",
        metadata.channel_id
    ))
}

const DISCORD_MESSAGE_CHAR_LIMIT: usize = 2000;
const DISCORD_MULTIPART_BOUNDARY: &str = "ironclaw-discord-response-boundary";
const DISCORD_ATTACHMENT_FILENAME: &str = "response.md";
const DISCORD_ATTACHMENT_NOTICE: &str = "Response too long for Discord; attached as response.md.";

#[derive(Debug, PartialEq, Eq)]
struct DiscordHttpRequest {
    headers_json: String,
    body: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
enum DiscordReplyPlan {
    Inline(DiscordHttpRequest),
    Attachment {
        upload: DiscordHttpRequest,
        fallback: DiscordHttpRequest,
    },
}

fn embeds_from_metadata_json(metadata_json: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(metadata_json)
        .ok()?
        .get("embeds")
        .cloned()
}

fn build_discord_json_request(
    content: &str,
    embeds: Option<&serde_json::Value>,
) -> Result<DiscordHttpRequest, String> {
    let mut payload = serde_json::json!({
        "content": content,
    });

    if let Some(embeds) = embeds {
        payload["embeds"] = embeds.clone();
    }

    Ok(DiscordHttpRequest {
        headers_json: serde_json::json!({
            "Content-Type": "application/json"
        })
        .to_string(),
        body: serde_json::to_vec(&payload).map_err(|e| format!("Failed to serialize: {}", e))?,
    })
}

fn build_discord_attachment_request(
    content: &str,
    embeds: Option<&serde_json::Value>,
) -> Result<DiscordHttpRequest, String> {
    let mut payload = serde_json::json!({
        "content": DISCORD_ATTACHMENT_NOTICE,
    });

    if let Some(embeds) = embeds {
        payload["embeds"] = embeds.clone();
    }

    let payload_json =
        serde_json::to_string(&payload).map_err(|e| format!("Failed to serialize: {}", e))?;

    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{payload_json}\r\n",
            boundary = DISCORD_MULTIPART_BOUNDARY,
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"{filename}\"\r\nContent-Type: text/markdown\r\n\r\n",
            boundary = DISCORD_MULTIPART_BOUNDARY,
            filename = DISCORD_ATTACHMENT_FILENAME,
        )
        .as_bytes(),
    );
    body.extend_from_slice(content.as_bytes());
    body.extend_from_slice(format!("\r\n--{}--\r\n", DISCORD_MULTIPART_BOUNDARY).as_bytes());

    Ok(DiscordHttpRequest {
        headers_json: serde_json::json!({
            "Content-Type": format!(
                "multipart/form-data; boundary={}",
                DISCORD_MULTIPART_BOUNDARY
            )
        })
        .to_string(),
        body,
    })
}

fn build_discord_reply_plan(response: &AgentResponse) -> Result<DiscordReplyPlan, String> {
    let embeds = embeds_from_metadata_json(&response.metadata_json);

    if response.content.chars().count() <= DISCORD_MESSAGE_CHAR_LIMIT {
        return build_discord_json_request(&response.content, embeds.as_ref())
            .map(DiscordReplyPlan::Inline);
    }

    Ok(DiscordReplyPlan::Attachment {
        upload: build_discord_attachment_request(&response.content, embeds.as_ref())?,
        fallback: build_discord_json_request(
            &truncate_message(&response.content),
            embeds.as_ref(),
        )?,
    })
}

fn send_discord_request(url: &str, request: &DiscordHttpRequest) -> Result<(), String> {
    match channel_host::http_request(
        "POST",
        url,
        &request.headers_json,
        Some(&request.body),
        None,
    ) {
        Ok(http_response) => {
            if http_response.status >= 200 && http_response.status < 300 {
                channel_host::log(channel_host::LogLevel::Debug, "Posted followup to Discord");
                Ok(())
            } else {
                let body_str = String::from_utf8_lossy(&http_response.body);
                Err(format!(
                    "Discord API error: {} - {}",
                    http_response.status, body_str
                ))
            }
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

/// Workspace path for persisting owner_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_id";
/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";
/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";
/// Workspace path for queued gateway text frames persisted by the host runtime.
const GATEWAY_EVENT_QUEUE_PATH: &str = "state/gateway_event_queue";
/// Workspace path for persisting the bot user id learned from READY dispatches.
const BOT_USER_ID_PATH: &str = "state/bot_user_id";
/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "discord";

#[derive(Debug, Deserialize)]
struct DiscordGatewayEvent {
    op: u64,
    #[serde(default)]
    t: Option<String>,
    #[serde(default)]
    d: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct DiscordGatewayReady {
    user: DiscordGatewayAuthor,
}

#[derive(Debug, Deserialize, Clone)]
struct DiscordGatewayAuthor {
    id: String,
    username: String,
    global_name: Option<String>,
    #[serde(default)]
    bot: bool,
}

#[derive(Debug, Deserialize)]
struct DiscordGatewayMessageCreate {
    channel_id: String,
    #[serde(default)]
    guild_id: Option<String>,
    content: String,
    author: DiscordGatewayAuthor,
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedGatewayMessage {
    user_id: String,
    user_name: String,
    channel_id: String,
    content: String,
    is_dm: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct GatewayPollResult {
    bot_user_id: Option<String>,
    messages: Vec<ParsedGatewayMessage>,
}

fn parse_gateway_event_queue(
    queue_json: &str,
    known_bot_user_id: Option<&str>,
) -> GatewayPollResult {
    let frames: Vec<String> = serde_json::from_str(queue_json).unwrap_or_default();
    let mut result = GatewayPollResult::default();
    let mut bot_user_id = known_bot_user_id.map(ToOwned::to_owned);

    for frame in frames {
        let event: DiscordGatewayEvent = match serde_json::from_str(&frame) {
            Ok(value) => value,
            Err(_) => continue,
        };

        if event.op != 0 {
            continue;
        }

        match event.t.as_deref() {
            Some("READY") => {
                if let Ok(ready) = serde_json::from_value::<DiscordGatewayReady>(event.d) {
                    if !ready.user.id.is_empty() {
                        bot_user_id = Some(ready.user.id);
                    }
                }
            }
            Some("MESSAGE_CREATE") => {
                let message = match serde_json::from_value::<DiscordGatewayMessageCreate>(event.d) {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                let active_bot_user_id = bot_user_id.as_deref().or(known_bot_user_id);
                if message.author.bot
                    || active_bot_user_id.is_some_and(|bot_id| message.author.id == bot_id)
                {
                    continue;
                }

                let is_dm = message.guild_id.is_none();
                let content =
                    match gateway_content_for_agent(&message.content, active_bot_user_id, is_dm) {
                        Some(value) => value,
                        None => continue,
                    };

                result.messages.push(ParsedGatewayMessage {
                    user_id: message.author.id,
                    user_name: message
                        .author
                        .global_name
                        .unwrap_or(message.author.username),
                    channel_id: message.channel_id,
                    content,
                    is_dm,
                });
            }
            _ => {}
        }
    }

    result.bot_user_id = bot_user_id;
    result
}

fn gateway_content_for_agent(
    content: &str,
    bot_user_id: Option<&str>,
    is_dm: bool,
) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    if is_dm {
        return Some(trimmed.to_string());
    }

    let bot_user_id = bot_user_id?;
    for mention in [
        format!("<@{}>", bot_user_id),
        format!("<@!{}>", bot_user_id),
    ] {
        if let Some(stripped) = trimmed.strip_prefix(&mention) {
            let cleaned = stripped.trim();
            return if cleaned.is_empty() {
                None
            } else {
                Some(cleaned.to_string())
            };
        }
    }

    None
}

/// Channel configuration from capabilities file.
#[derive(Debug, Deserialize)]
struct DiscordConfig {
    #[serde(default)]
    #[allow(dead_code)]
    require_signature_verification: bool,
    #[serde(default)]
    owner_id: Option<String>,
    #[serde(default)]
    dm_policy: Option<String>,
    #[serde(default)]
    allow_from: Option<Vec<String>>,
}

struct DiscordChannel;

impl Guest for DiscordChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: DiscordConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Discord channel starting");

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
                channel_host::log(channel_host::LogLevel::Info, "Responding to Discord ping");
                json_response(200, serde_json::json!({"type": 1}))
            }

            // Application Command (slash command)
            2 => {
                if handle_slash_command(&interaction) {
                    json_response(200, serde_json::json!({"type": 5}))
                } else {
                    // Permission denied — ephemeral response
                    json_response(
                        200,
                        serde_json::json!({
                            "type": 4,
                            "data": {
                                "content": "You are not authorized to use this bot.",
                                "flags": 64
                            }
                        }),
                    )
                }
            }

            // Message Component (buttons, selects)
            3 => {
                if let Some(ref message) = interaction.message {
                    handle_message_component(&interaction, message);
                }
                json_response(200, serde_json::json!({"type": 6}))
            }

            _ => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "Unknown Discord interaction type: {}",
                        interaction.interaction_type
                    ),
                );
                json_response(200, serde_json::json!({"type": 6}))
            }
        }
    }

    fn on_poll() {
        let queue_json = channel_host::workspace_read(GATEWAY_EVENT_QUEUE_PATH).unwrap_or_default();
        if queue_json.trim().is_empty() || queue_json.trim() == "[]" {
            return;
        }

        let known_bot_user_id = channel_host::workspace_read(BOT_USER_ID_PATH);
        let parsed = parse_gateway_event_queue(&queue_json, known_bot_user_id.as_deref());

        if let Err(error) = channel_host::workspace_write(GATEWAY_EVENT_QUEUE_PATH, "[]") {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to clear Discord gateway queue: {}", error),
            );
        }

        if let Some(bot_user_id) = parsed.bot_user_id.as_deref() {
            if let Err(error) = channel_host::workspace_write(BOT_USER_ID_PATH, bot_user_id) {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to persist Discord bot user id: {}", error),
                );
            }
        }

        for message in parsed.messages {
            if !check_sender_permission(
                &message.user_id,
                Some(&message.user_name),
                message.is_dm,
                PermissionSource::Gateway,
                None,
            ) {
                continue;
            }

            let metadata = DiscordMessageMetadata {
                channel_id: message.channel_id,
                interaction_id: String::new(),
                token: String::new(),
                application_id: String::new(),
                thread_id: None,
            };

            let metadata_json = match serde_json::to_string(&metadata) {
                Ok(json) => json,
                Err(error) => {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to serialize gateway metadata: {}", error),
                    );
                    continue;
                }
            };

            channel_host::emit_message(&EmittedMessage {
                user_id: message.user_id,
                user_name: Some(message.user_name),
                content: message.content,
                thread_id: None,
                metadata_json,
                attachments: vec![],
            });
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: DiscordMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        let route = response_route_for_metadata(&metadata);
        let plan = build_discord_reply_plan(&response)?;

        let url = match &route {
            DiscordResponseRoute::InteractionWebhook(url) => url,
            DiscordResponseRoute::ChannelMessage(url) => url,
        };

        match plan {
            DiscordReplyPlan::Inline(request) => send_discord_request(url, &request),
            DiscordReplyPlan::Attachment { upload, fallback } => {
                match send_discord_request(url, &upload) {
                    Ok(()) => Ok(()),
                    Err(upload_error) => {
                        channel_host::log(
                            channel_host::LogLevel::Warn,
                            &format!(
                                "Discord attachment upload failed, falling back to truncated text: {}",
                                upload_error
                            ),
                        );
                        send_discord_request(url, &fallback).map_err(|fallback_error| {
                            format!(
                                "Discord attachment upload failed: {}; fallback also failed: {}",
                                upload_error, fallback_error
                            )
                        })
                    }
                }
            }
        }
    }

    fn on_status(update: StatusUpdate) {
        let Some(url) = typing_request_url_for_update(&update) else {
            return;
        };

        let headers = serde_json::json!({
            "Content-Type": "application/json"
        });

        match channel_host::http_request("POST", &url, &headers.to_string(), None, None) {
            Ok(response) if (200..300).contains(&response.status) => {}
            Ok(response) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "Discord typing indicator failed with status {}",
                        response.status
                    ),
                );
            }
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Discord typing indicator request failed: {}", error),
                );
            }
        }
    }

    fn on_broadcast(_user_id: String, _response: AgentResponse) -> Result<(), String> {
        Err("broadcast not yet implemented for Discord channel".to_string())
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Discord channel shutting down",
        );
    }
}

/// Returns true if the message was emitted, false if permission denied.
fn handle_slash_command(interaction: &DiscordInteraction) -> bool {
    let user = interaction
        .member
        .as_ref()
        .map(|m| &m.user)
        .or(interaction.user.as_ref());
    let user_id = user.map(|u| u.id.clone()).unwrap_or_default();
    let user_name = user
        .map(|u| {
            u.global_name
                .as_ref()
                .filter(|s| !s.is_empty())
                .unwrap_or(&u.username)
                .clone()
        })
        .unwrap_or_default();

    // DM if no guild member context (only direct user field set)
    let is_dm = interaction.member.is_none();

    // Permission check
    if !check_sender_permission(
        &user_id,
        Some(&user_name),
        is_dm,
        PermissionSource::Webhook,
        Some(&PairingReplyCtx {
            application_id: interaction.application_id.clone(),
            token: interaction.token.clone(),
        }),
    ) {
        return false;
    }

    let channel_id = interaction.channel_id.clone().unwrap_or_default();

    let command_name = interaction
        .data
        .as_ref()
        .map(|d| d.name.clone())
        .unwrap_or_default();
    let options = interaction.data.as_ref().and_then(|d| d.options.clone());

    let content = if let Some(opts) = options {
        let opt_str = opts
            .iter()
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

    let metadata_json = match serde_json::to_string(&metadata) {
        Ok(json) => json,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to serialize metadata: {}", e),
            );
            let url = format!(
                "https://discord.com/api/v10/webhooks/{}/{}",
                interaction.application_id, interaction.token
            );
            let payload = serde_json::json!({
                "content": "❌ Internal Error: Failed to process command metadata.",
                "flags": 64
            });
            let _ = channel_host::http_request(
                "POST",
                &url,
                &serde_json::json!({"Content-Type": "application/json"}).to_string(),
                Some(&serde_json::to_vec(&payload).unwrap_or_default()),
                None,
            );
            return true; // Error, but not a permission denial
        }
    };

    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: Some(user_name),
        content,
        thread_id: None,
        metadata_json,
        attachments: vec![],
    });
    true
}

fn handle_message_component(interaction: &DiscordInteraction, message: &DiscordMessage) {
    let user = interaction
        .member
        .as_ref()
        .map(|m| &m.user)
        .or(interaction.user.as_ref());
    let user_id = user.map(|u| u.id.clone()).unwrap_or_default();
    let user_name = user
        .map(|u| {
            u.global_name
                .as_ref()
                .filter(|s| !s.is_empty())
                .unwrap_or(&u.username)
                .clone()
        })
        .unwrap_or_default();

    let is_dm = interaction.member.is_none();
    if !check_sender_permission(
        &user_id,
        Some(&user_name),
        is_dm,
        PermissionSource::Webhook,
        None,
    ) {
        return;
    }

    let channel_id = message.channel_id.clone();

    let metadata = DiscordMessageMetadata {
        channel_id: channel_id.clone(),
        interaction_id: interaction.id.clone(),
        token: interaction.token.clone(),
        application_id: interaction.application_id.clone(),
        thread_id: None,
    };

    let metadata_json = match serde_json::to_string(&metadata) {
        Ok(json) => json,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to serialize metadata: {}", e),
            );
            return; // Don't emit message if metadata can't be serialized
        }
    };

    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: Some(user_name),
        content: format!("[Button clicked] {}", message.content),
        thread_id: None,
        metadata_json,
        attachments: vec![],
    });
}

// ============================================================================
// Permission & Pairing
// ============================================================================

/// Context needed to send a pairing reply via Discord webhook followup.
struct PairingReplyCtx {
    application_id: String,
    token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionSource {
    Webhook,
    Gateway,
}

fn should_apply_dm_pairing(source: PermissionSource, is_dm: bool) -> bool {
    matches!(source, PermissionSource::Webhook) && is_dm
}

/// Check if a sender is permitted to interact with the bot.
/// Returns true if allowed, false if denied (pairing reply sent if applicable).
fn check_sender_permission(
    user_id: &str,
    username: Option<&str>,
    is_dm: bool,
    source: PermissionSource,
    reply_ctx: Option<&PairingReplyCtx>,
) -> bool {
    // 1. Owner check (highest priority, applies to all contexts)
    let owner_id = channel_host::workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());
    if let Some(ref owner) = owner_id {
        if user_id != owner {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!(
                    "Dropping interaction from non-owner user {} (owner: {})",
                    user_id, owner
                ),
            );
            return false;
        }
        return true;
    }

    // 2. DM policy (only for DMs when no owner_id)
    if !should_apply_dm_pairing(source, is_dm) {
        return true;
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

    // 4. Check sender against allow list
    let is_allowed = allowed.contains(&"*".to_string())
        || allowed.contains(&user_id.to_string())
        || username.is_some_and(|u| allowed.contains(&u.to_string()));

    if is_allowed {
        return true;
    }

    // 5. Not allowed — handle by policy
    if dm_policy == "pairing" {
        let meta = serde_json::json!({
            "user_id": user_id,
            "username": username,
        })
        .to_string();

        match channel_host::pairing_upsert_request(CHANNEL_NAME, user_id, &meta) {
            Ok(result) => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Pairing request for user {}: code {}", user_id, result.code),
                );
                if result.created {
                    if let Some(ctx) = reply_ctx {
                        let _ = send_pairing_reply(ctx, &result.code);
                    }
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

/// Send a pairing code as an ephemeral Discord followup message.
fn send_pairing_reply(ctx: &PairingReplyCtx, code: &str) -> Result<(), String> {
    let url = format!(
        "https://discord.com/api/v10/webhooks/{}/{}",
        ctx.application_id, ctx.token
    );

    let payload = serde_json::json!({
        "content": format!(
            "To pair with this bot, run: `ironclaw pairing approve discord {}`",
            code
        ),
        "flags": 64 // Ephemeral — only visible to the sender
    });

    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("Failed to serialize: {}", e))?;

    let headers = serde_json::json!({"Content-Type": "application/json"});

    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(response) if response.status >= 200 && response.status < 300 => Ok(()),
        Ok(response) => {
            let body_str = String::from_utf8_lossy(&response.body);
            Err(format!(
                "Discord API error: {} - {}",
                response.status, body_str
            ))
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
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

fn truncate_message(content: &str) -> String {
    if content.chars().count() <= DISCORD_MESSAGE_CHAR_LIMIT {
        content.to_string()
    } else {
        let suffix = "\n... (truncated)";
        let allowed_chars = DISCORD_MESSAGE_CHAR_LIMIT.saturating_sub(suffix.chars().count());
        let mut truncated = content.chars().take(allowed_chars).collect::<String>();
        truncated.push_str("\n... (truncated)");
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISCORD_CAPABILITIES_JSON: &str = include_str!("../discord.capabilities.json");

    #[test]
    fn test_truncate_message() {
        let short = "Hello world";
        assert_eq!(truncate_message(short), short);

        let long = "a".repeat(2005);
        let truncated = truncate_message(&long);
        assert_eq!(truncated.chars().count(), 2000);
        assert!(truncated.ends_with("\n... (truncated)"));

        // Test with multibyte characters (Euro sign is 3 bytes)
        let multi = "€".repeat(2005);
        let truncated_multi = truncate_message(&multi);

        assert_eq!(truncated_multi.chars().count(), 2000);
        assert!(truncated_multi.ends_with("\n... (truncated)"));

        let content_part = &truncated_multi[..truncated_multi.len() - 16];
        assert!(content_part.chars().all(|c| c == '€'));
    }

    #[test]
    fn test_reply_plan_uses_character_count_for_attachment_threshold() {
        let inline =
            build_discord_reply_plan(&test_response(test_metadata_json(), "€".repeat(2000)))
                .unwrap();

        assert!(matches!(inline, DiscordReplyPlan::Inline(_)));
    }

    fn test_response(metadata_json: String, content: String) -> AgentResponse {
        AgentResponse {
            message_id: "msg-1".to_string(),
            content,
            thread_id: None,
            metadata_json,
            attachments: vec![],
        }
    }

    fn test_metadata_json() -> String {
        serde_json::json!({
            "channel_id": "chan-1",
            "interaction_id": "int-1",
            "token": "tok-1",
            "application_id": "app-1",
            "thread_id": null,
            "embeds": [{"title": "embed title"}]
        })
        .to_string()
    }

    #[test]
    fn test_reply_plan_threshold_uses_attachment_only_above_2000_chars() {
        let inline =
            build_discord_reply_plan(&test_response(test_metadata_json(), "a".repeat(2000)))
                .unwrap();
        assert!(matches!(inline, DiscordReplyPlan::Inline(_)));

        let attachment =
            build_discord_reply_plan(&test_response(test_metadata_json(), "a".repeat(2001)))
                .unwrap();
        assert!(matches!(attachment, DiscordReplyPlan::Attachment { .. }));
    }

    #[test]
    fn test_reply_plan_preserves_short_message_content_and_embeds() {
        let plan = build_discord_reply_plan(&test_response(
            test_metadata_json(),
            "short reply".to_string(),
        ))
        .unwrap();

        let DiscordReplyPlan::Inline(request) = plan else {
            panic!("expected inline plan");
        };

        assert_eq!(
            request.headers_json,
            r#"{"Content-Type":"application/json"}"#
        );

        let payload: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(payload["content"], "short reply");
        assert_eq!(payload["embeds"][0]["title"], "embed title");
    }

    #[test]
    fn test_reply_plan_builds_markdown_attachment_multipart_payload() {
        let content = "# Heading\n\nA long markdown reply".repeat(80);
        let plan = build_discord_reply_plan(&test_response(test_metadata_json(), content.clone()))
            .unwrap();

        let DiscordReplyPlan::Attachment { upload, .. } = plan else {
            panic!("expected attachment plan");
        };

        assert!(upload
            .headers_json
            .contains("multipart/form-data; boundary="));

        let body = String::from_utf8(upload.body).unwrap();
        assert!(body.contains("name=\"payload_json\""));
        assert!(body.contains("filename=\"response.md\""));
        assert!(body.contains("Content-Type: text/markdown"));
        assert!(body.contains(DISCORD_ATTACHMENT_NOTICE));
        assert!(body.contains("embed title"));
        assert!(body.contains(&content));
    }

    #[test]
    fn test_reply_plan_includes_truncated_text_fallback_for_attachment_failures() {
        let content = "a".repeat(2400);
        let plan = build_discord_reply_plan(&test_response(test_metadata_json(), content.clone()))
            .unwrap();

        let DiscordReplyPlan::Attachment { fallback, .. } = plan else {
            panic!("expected attachment plan");
        };

        let payload: serde_json::Value = serde_json::from_slice(&fallback.body).unwrap();
        assert_eq!(payload["content"], truncate_message(&content));
        assert_eq!(payload["embeds"][0]["title"], "embed title");
    }

    #[test]
    fn test_metadata_serialization() {
        let metadata = DiscordMessageMetadata {
            channel_id: "123".into(),
            interaction_id: "456".into(),
            token: "abc".into(),
            application_id: "789".into(),
            thread_id: None,
        };
        let json = serde_json::to_string(&metadata).unwrap();
        let parsed: DiscordMessageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.channel_id, "123");
        assert_eq!(parsed.interaction_id, "456");
    }

    #[test]
    fn test_response_route_uses_webhook_for_interactions() {
        let metadata = DiscordMessageMetadata {
            channel_id: "123".into(),
            interaction_id: "456".into(),
            token: "tok".into(),
            application_id: "app".into(),
            thread_id: None,
        };

        assert_eq!(
            response_route_for_metadata(&metadata),
            DiscordResponseRoute::InteractionWebhook(
                "https://discord.com/api/v10/webhooks/app/tok".to_string()
            )
        );
    }

    #[test]
    fn test_response_route_uses_channel_messages_for_gateway_metadata() {
        let metadata = DiscordMessageMetadata {
            channel_id: "chan-1".into(),
            interaction_id: String::new(),
            token: String::new(),
            application_id: String::new(),
            thread_id: None,
        };

        assert_eq!(
            response_route_for_metadata(&metadata),
            DiscordResponseRoute::ChannelMessage(
                "https://discord.com/api/v10/channels/chan-1/messages".to_string()
            )
        );
    }

    #[test]
    fn test_typing_request_url_uses_channel_id_for_thinking_status() {
        let update = StatusUpdate {
            status: StatusType::Thinking,
            message: "Thinking...".to_string(),
            metadata_json: serde_json::json!({
                "channel_id": "chan-42",
                "interaction_id": "",
                "token": "",
                "application_id": "",
                "thread_id": null
            })
            .to_string(),
        };

        assert_eq!(
            typing_request_url_for_update(&update),
            Some("https://discord.com/api/v10/channels/chan-42/typing".to_string())
        );
    }

    #[test]
    fn test_typing_request_url_ignores_non_thinking_status() {
        let update = StatusUpdate {
            status: StatusType::Done,
            message: "Done".to_string(),
            metadata_json: serde_json::json!({
                "channel_id": "chan-42",
                "interaction_id": "",
                "token": "",
                "application_id": "",
                "thread_id": null
            })
            .to_string(),
        };

        assert_eq!(typing_request_url_for_update(&update), None);
    }

    #[test]
    fn test_typing_request_url_ignores_invalid_metadata() {
        let update = StatusUpdate {
            status: StatusType::Thinking,
            message: "Thinking...".to_string(),
            metadata_json: "not-json".to_string(),
        };

        assert_eq!(typing_request_url_for_update(&update), None);
    }

    #[test]
    fn test_parse_slash_command_interaction() {
        // Verify that a slash command interaction deserializes correctly.
        let json = r#"{
            "type": 2,
            "id": "int_1",
            "application_id": "app_1",
            "channel_id": "ch_1",
            "member": {
                "user": {
                    "id": "user_1",
                    "username": "testuser",
                    "global_name": "Test User"
                }
            },
            "data": {
                "id": "cmd_1",
                "name": "ask",
                "options": [
                    {"name": "question", "value": "What is rust?"}
                ]
            },
            "token": "token_abc"
        }"#;

        let interaction: DiscordInteraction = serde_json::from_str(json).unwrap();
        assert_eq!(interaction.interaction_type, 2);
        assert!(interaction.data.is_some());
    }

    #[test]
    fn test_capabilities_default_to_gateway_mode() {
        let caps: serde_json::Value =
            serde_json::from_str(DISCORD_CAPABILITIES_JSON).expect("capabilities parse");
        let allowlist = caps["capabilities"]["http"]["allowlist"]
            .as_array()
            .expect("http allowlist array");

        assert_eq!(
            caps["capabilities"]["channel"]["allow_polling"],
            serde_json::Value::Bool(false)
        );
        assert!(allowlist.iter().any(|entry| {
            entry["host"] == serde_json::Value::String("gateway.discord.gg".to_string())
                && entry["methods"] == serde_json::json!(["GET"])
        }));
        assert_eq!(
            caps["capabilities"]["websocket"]["url"],
            serde_json::Value::String("wss://gateway.discord.gg/?v=10&encoding=json".to_string())
        );
        assert_eq!(
            caps["capabilities"]["websocket"]["connect_on_start"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            caps["capabilities"]["websocket"]["identify_secret_name"],
            serde_json::Value::String("discord_bot_token".to_string())
        );
    }

    #[test]
    fn test_parse_gateway_event_queue_emits_message_create_after_ready() {
        let queue_json = serde_json::json!([
            serde_json::json!({
                "op": 0,
                "t": "READY",
                "d": {
                    "user": {
                        "id": "bot-1",
                        "username": "ironclaw",
                        "global_name": "IronClaw",
                        "bot": true
                    }
                }
            })
            .to_string(),
            serde_json::json!({
                "op": 0,
                "t": "MESSAGE_CREATE",
                "d": {
                    "channel_id": "chan-1",
                    "guild_id": "guild-1",
                    "content": "<@bot-1> hello from discord",
                    "author": {
                        "id": "user-1",
                        "username": "alice",
                        "global_name": "Alice",
                        "bot": false
                    }
                }
            })
            .to_string()
        ])
        .to_string();

        let result = parse_gateway_event_queue(&queue_json, None);

        assert_eq!(result.bot_user_id.as_deref(), Some("bot-1"));
        assert_eq!(
            result.messages,
            vec![ParsedGatewayMessage {
                user_id: "user-1".to_string(),
                user_name: "Alice".to_string(),
                channel_id: "chan-1".to_string(),
                content: "hello from discord".to_string(),
                is_dm: false,
            }]
        );
    }

    #[test]
    fn test_parse_gateway_event_queue_ignores_bot_and_unmentioned_guild_messages() {
        let queue_json = serde_json::json!([
            serde_json::json!({
                "op": 0,
                "t": "MESSAGE_CREATE",
                "d": {
                    "channel_id": "chan-1",
                    "guild_id": "guild-1",
                    "content": "this should not trigger",
                    "author": {
                        "id": "user-1",
                        "username": "alice",
                        "global_name": "Alice",
                        "bot": false
                    }
                }
            })
            .to_string(),
            serde_json::json!({
                "op": 0,
                "t": "MESSAGE_CREATE",
                "d": {
                    "channel_id": "dm-1",
                    "content": "bot echo",
                    "author": {
                        "id": "bot-1",
                        "username": "ironclaw",
                        "global_name": "IronClaw",
                        "bot": true
                    }
                }
            })
            .to_string(),
            serde_json::json!({
                "op": 0,
                "t": "MESSAGE_CREATE",
                "d": {
                    "channel_id": "dm-2",
                    "content": "direct message",
                    "author": {
                        "id": "user-2",
                        "username": "bob",
                        "global_name": null,
                        "bot": false
                    }
                }
            })
            .to_string()
        ])
        .to_string();

        let result = parse_gateway_event_queue(&queue_json, Some("bot-1"));

        assert_eq!(result.bot_user_id.as_deref(), Some("bot-1"));
        assert_eq!(
            result.messages,
            vec![ParsedGatewayMessage {
                user_id: "user-2".to_string(),
                user_name: "bob".to_string(),
                channel_id: "dm-2".to_string(),
                content: "direct message".to_string(),
                is_dm: true,
            }]
        );
    }

    #[test]
    fn test_gateway_messages_bypass_dm_pairing_policy() {
        assert!(!should_apply_dm_pairing(PermissionSource::Gateway, true));
    }

    #[test]
    fn test_non_gateway_dm_pairing_behavior_is_unchanged() {
        assert!(should_apply_dm_pairing(PermissionSource::Webhook, true));
        assert!(!should_apply_dm_pairing(PermissionSource::Webhook, false));
    }
}
