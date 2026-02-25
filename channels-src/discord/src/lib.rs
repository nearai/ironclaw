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
//! - Automatic message truncation (> 2000 chars)
//!
//! # Security
//!
//! - Signature validation is handled in-channel using Discord's Ed25519 headers
//! - Bot token is injected by host during HTTP requests
//! - WASM never sees raw credentials

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use std::{cmp::Ordering, collections::HashMap};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusUpdate,
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

#[derive(Debug, Deserialize)]
struct DiscordChannelMessage {
    id: String,
    content: String,
    channel_id: String,
    author: DiscordChannelAuthor,
    #[serde(default)]
    mentions: Vec<DiscordUser>,
    #[serde(default)]
    webhook_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordChannelAuthor {
    id: String,
    username: String,
    global_name: Option<String>,
    #[serde(default)]
    bot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiscordRuntimeConfig {
    #[serde(default = "default_require_signature_verification")]
    require_signature_verification: bool,
    #[serde(default)]
    webhook_secret: Option<String>,
    #[serde(default)]
    polling_enabled: bool,
    #[serde(default = "default_poll_interval_ms")]
    poll_interval_ms: u32,
    #[serde(default)]
    mention_channel_ids: Vec<String>,
    #[serde(default)]
    owner_id: Option<String>,
    #[serde(default = "default_dm_policy")]
    dm_policy: String,
    #[serde(default)]
    allow_from: Vec<String>,
}

fn default_poll_interval_ms() -> u32 {
    30_000
}

fn default_require_signature_verification() -> bool {
    true
}

fn default_dm_policy() -> String {
    "pairing".to_string()
}

fn default_runtime_config() -> DiscordRuntimeConfig {
    DiscordRuntimeConfig {
        require_signature_verification: default_require_signature_verification(),
        webhook_secret: None,
        polling_enabled: false,
        poll_interval_ms: default_poll_interval_ms(),
        mention_channel_ids: Vec::new(),
        owner_id: None,
        dm_policy: default_dm_policy(),
        allow_from: Vec::new(),
    }
}

/// Workspace path for persisting owner_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_id";
/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";
/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";
/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "discord";

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
struct DiscordMessageMetadata {
    /// Discord channel ID
    channel_id: String,

    /// Interaction ID for followups
    #[serde(default)]
    interaction_id: Option<String>,

    /// Interaction token for responding
    #[serde(default)]
    token: Option<String>,

    /// Application ID
    #[serde(default)]
    application_id: Option<String>,

    /// Source message ID when handling mention-poll events.
    #[serde(default)]
    source_message_id: Option<String>,

    /// Thread ID (for forum threads)
    thread_id: Option<String>,
}

struct DiscordChannel;

impl Guest for DiscordChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(channel_host::LogLevel::Info, "Discord channel starting");

        let config =
            serde_json::from_str::<DiscordRuntimeConfig>(&config_json).unwrap_or_else(|e| {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Invalid config JSON, using defaults: {}", e),
                );
                default_runtime_config()
            });

        if let Ok(serialized) = serde_json::to_string(&config) {
            let _ = channel_host::workspace_write("config.json", &serialized);
        }

        if config.require_signature_verification
            && config
                .webhook_secret
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_none()
        {
            channel_host::log(
                channel_host::LogLevel::Error,
                "Discord channel misconfigured: require_signature_verification=true but webhook_secret is empty",
            );
        } else if !config.require_signature_verification {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Discord signature verification is disabled; webhook endpoint is unprotected",
            );
        }

        // Persist owner_id so subsequent callbacks can read it.
        if let Some(ref owner_id) = config.owner_id {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, owner_id);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: user {}", owner_id),
            );
        } else {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
        }

        // Persist dm_policy and allow_from for DM pairing.
        let _ = channel_host::workspace_write(DM_POLICY_PATH, &config.dm_policy);
        let allow_from_json =
            serde_json::to_string(&config.allow_from).unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        Ok(ChannelConfig {
            display_name: "Discord".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/discord".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false,
            }],
            poll: if config.polling_enabled {
                Some(PollConfig {
                    interval_ms: config.poll_interval_ms.max(30_000),
                    enabled: true,
                })
            } else {
                None
            },
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        let config = load_runtime_config();
        let headers: HashMap<String, String> =
            serde_json::from_str(&req.headers_json).unwrap_or_default();
        if config.require_signature_verification {
            if config
                .webhook_secret
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_none()
            {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    "Discord channel misconfigured: webhook_secret not set while verification is required",
                );
                return json_response(
                    500,
                    serde_json::json!({"error": "Channel misconfigured: webhook_secret not set"}),
                );
            }

            if !verify_discord_request_signature(
                headers,
                &req.body,
                config.webhook_secret.as_deref(),
            ) {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    "Discord signature verification failed",
                );
                return json_response(401, serde_json::json!({"error": "Invalid signature"}));
            }
        } else {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Discord signature verification is disabled; accepting unverified webhook request",
            );
        }

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
                    json_response(
                        200,
                        serde_json::json!({
                            "type": 5,
                            "data": {
                                "content": "🤔 Thinking..."
                            }
                        }),
                    )
                } else {
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
        poll_for_mentions();
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: DiscordMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Truncate content to 2000 characters to comply with Discord limits
        let content = truncate_message(&response.content);

        let mut payload = serde_json::json!({ "content": content });

        // Check for embeds in metadata
        if let Ok(meta_json) = serde_json::from_str::<serde_json::Value>(&response.metadata_json) {
            if let Some(embeds) = meta_json.get("embeds") {
                payload["embeds"] = embeds.clone();
            }
        }

        let payload_bytes =
            serde_json::to_vec(&payload).map_err(|e| format!("Failed to serialize: {}", e))?;

        let headers = serde_json::json!({
            "Content-Type": "application/json"
        });

        let (method, url) = if let (Some(application_id), Some(token)) =
            (metadata.application_id.as_ref(), metadata.token.as_ref())
        {
            (
                "PATCH",
                format!(
                    "https://discord.com/api/v10/webhooks/{}/{}/messages/@original",
                    application_id, token
                ),
            )
        } else if let Some(source_message_id) = metadata.source_message_id.as_ref() {
            payload["message_reference"] = serde_json::json!({
                "message_id": source_message_id
            });
            payload["allowed_mentions"] = serde_json::json!({
                "replied_user": true
            });
            let mention_payload = serde_json::to_vec(&payload)
                .map_err(|e| format!("Failed to serialize mention payload: {}", e))?;
            let mention_url = format!(
                "https://discord.com/api/v10/channels/{}/messages",
                metadata.channel_id
            );
            let result = channel_host::http_request(
                "POST",
                &mention_url,
                &discord_auth_headers_json(true),
                Some(&mention_payload),
                None,
            );
            return map_discord_response(result);
        } else {
            return Err("Unsupported Discord response metadata".to_string());
        };

        let result = channel_host::http_request(
            method,
            &url,
            &headers.to_string(),
            Some(&payload_bytes),
            None,
        );

        map_discord_response(result)
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Discord channel shutting down",
        );
    }
}

fn map_discord_response(
    result: Result<near::agent::channel_host::HttpResponse, String>,
) -> Result<(), String> {
    match result {
        Ok(http_response) => {
            if http_response.status >= 200 && http_response.status < 300 {
                channel_host::log(channel_host::LogLevel::Debug, "Posted response to Discord");
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

fn load_runtime_config() -> DiscordRuntimeConfig {
    channel_host::workspace_read("config.json")
        .and_then(|raw| serde_json::from_str::<DiscordRuntimeConfig>(&raw).ok())
        .unwrap_or_else(default_runtime_config)
}

fn poll_for_mentions() {
    let config = load_runtime_config();
    if !config.polling_enabled || config.mention_channel_ids.is_empty() {
        return;
    }

    let bot_id = match get_or_fetch_bot_id() {
        Some(id) => id,
        None => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Skipping mention polling: failed to resolve bot user id",
            );
            return;
        }
    };

    for channel_id in &config.mention_channel_ids {
        poll_channel_mentions(channel_id, &bot_id);
    }
}

fn get_or_fetch_bot_id() -> Option<String> {
    if let Some(id) = channel_host::workspace_read("bot_user_id.txt") {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let response = channel_host::http_request(
        "GET",
        "https://discord.com/api/v10/users/@me",
        &discord_auth_headers_json(false),
        None,
        Some(10_000),
    )
    .ok()?;

    if !(200..300).contains(&response.status) {
        return None;
    }

    let value: serde_json::Value = serde_json::from_slice(&response.body).ok()?;
    let id = value.get("id")?.as_str()?.to_string();
    let _ = channel_host::workspace_write("bot_user_id.txt", &id);
    Some(id)
}

fn poll_channel_mentions(channel_id: &str, bot_id: &str) {
    let cursor_path = format!("cursor_{}.txt", channel_id);
    let last_seen = channel_host::workspace_read(&cursor_path).map(|s| s.trim().to_string());

    // On first run for a channel, initialize the cursor to "latest seen" and
    // skip back-processing historical messages.
    if last_seen.is_none() {
        if let Some(latest) = fetch_latest_message_id(channel_id) {
            let _ = channel_host::workspace_write(&cursor_path, &latest);
        }
        return;
    }

    let Some(mut messages) =
        fetch_messages_after_cursor(channel_id, last_seen.as_deref().unwrap_or(""))
    else {
        return;
    };
    if messages.is_empty() {
        return;
    }

    messages.sort_by(|a, b| compare_message_ids(&a.id, &b.id));
    let mut max_seen = last_seen.clone();
    let mut recent_ids = load_recent_processed_ids(channel_id);
    let mut dedup_updated = false;

    for msg in messages {
        if is_new_message(max_seen.as_deref(), &msg.id) {
            max_seen = Some(msg.id.clone());
        }

        if msg.webhook_id.is_some() || msg.author.bot || msg.author.id == bot_id {
            continue;
        }

        if !message_mentions_bot(&msg, bot_id) {
            continue;
        }

        if recent_ids.iter().any(|id| id == &msg.id) {
            continue;
        }

        let user_name = msg
            .author
            .global_name
            .as_ref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&msg.author.username)
            .clone();
        if !check_sender_permission(&msg.author.id, Some(&user_name), false, None) {
            continue;
        }

        let content = strip_bot_mention(&msg.content, bot_id);
        let metadata = DiscordMessageMetadata {
            channel_id: msg.channel_id.clone(),
            interaction_id: None,
            token: None,
            application_id: None,
            source_message_id: Some(msg.id.clone()),
            thread_id: None,
        };

        let metadata_json = match serde_json::to_string(&metadata) {
            Ok(v) => v,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to serialize mention metadata: {}", e),
                );
                continue;
            }
        };

        channel_host::emit_message(&EmittedMessage {
            user_id: msg.author.id.clone(),
            user_name: Some(user_name.clone()),
            content: if content.is_empty() {
                "mention".to_string()
            } else {
                content
            },
            thread_id: None,
            metadata_json,
        });

        remember_processed_id(&mut recent_ids, &msg.id);
        dedup_updated = true;
    }

    if let Some(cursor) = max_seen {
        let _ = channel_host::workspace_write(&cursor_path, &cursor);
    }
    if dedup_updated {
        let _ = save_recent_processed_ids(channel_id, &recent_ids);
    }
}

fn fetch_latest_message_id(channel_id: &str) -> Option<String> {
    let url = format!(
        "https://discord.com/api/v10/channels/{}/messages?limit=1",
        channel_id
    );
    let response = channel_host::http_request(
        "GET",
        &url,
        &discord_auth_headers_json(false),
        None,
        Some(10_000),
    )
    .ok()?;
    if !(200..300).contains(&response.status) {
        let body = String::from_utf8_lossy(&response.body);
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!(
                "Discord initial poll failed for channel {}: status={} body={}",
                channel_id, response.status, body
            ),
        );
        return None;
    }
    let messages: Vec<DiscordChannelMessage> = serde_json::from_slice(&response.body).ok()?;
    messages.first().map(|m| m.id.clone())
}

fn fetch_messages_after_cursor(
    channel_id: &str,
    last_seen: &str,
) -> Option<Vec<DiscordChannelMessage>> {
    const PAGE_LIMIT: usize = 100;
    const MAX_PAGES: usize = 50;

    let mut all_messages = Vec::new();
    let mut after = last_seen.to_string();

    for page in 0..MAX_PAGES {
        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages?limit={}&after={}",
            channel_id, PAGE_LIMIT, after
        );
        let response = match channel_host::http_request(
            "GET",
            &url,
            &discord_auth_headers_json(false),
            None,
            Some(10_000),
        ) {
            Ok(r) => r,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "Discord poll request failed for channel {}: {}",
                        channel_id, e
                    ),
                );
                return None;
            }
        };

        if !(200..300).contains(&response.status) {
            let body = String::from_utf8_lossy(&response.body);
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!(
                    "Discord poll failed for channel {}: status={} body={}",
                    channel_id, response.status, body
                ),
            );
            return None;
        }

        let messages: Vec<DiscordChannelMessage> = match serde_json::from_slice(&response.body) {
            Ok(v) => v,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to parse polled Discord messages: {}", e),
                );
                return None;
            }
        };
        let page_len = messages.len();
        if messages.is_empty() {
            break;
        }

        let page_max_id = messages
            .iter()
            .map(|m| m.id.as_str())
            .max_by(|a, b| compare_message_ids(a, b))
            .map(str::to_string);

        all_messages.extend(messages.into_iter());

        if page_len < PAGE_LIMIT {
            break;
        }

        if let Some(max_id) = page_max_id {
            if max_id == after {
                break;
            }
            after = max_id;
        } else {
            break;
        }

        if page + 1 == MAX_PAGES {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!(
                    "Discord poll pagination limit reached for channel {}; processing partial batch",
                    channel_id
                ),
            );
        }
    }

    Some(all_messages)
}

fn compare_message_ids(a: &str, b: &str) -> Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => a.cmp(b),
    }
}

fn dedup_ids_path(channel_id: &str) -> String {
    format!("dedup_{}.json", channel_id)
}

fn load_recent_processed_ids(channel_id: &str) -> Vec<String> {
    let path = dedup_ids_path(channel_id);
    channel_host::workspace_read(&path)
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

fn save_recent_processed_ids(channel_id: &str, ids: &[String]) -> Result<(), String> {
    let path = dedup_ids_path(channel_id);
    let raw =
        serde_json::to_string(ids).map_err(|e| format!("Failed to serialize dedup ids: {}", e))?;
    channel_host::workspace_write(&path, &raw)
}

fn remember_processed_id(ids: &mut Vec<String>, message_id: &str) {
    const MAX_RECENT_IDS: usize = 200;
    if ids.iter().any(|id| id == message_id) {
        return;
    }
    ids.push(message_id.to_string());
    if ids.len() > MAX_RECENT_IDS {
        let drop_count = ids.len() - MAX_RECENT_IDS;
        ids.drain(0..drop_count);
    }
}

fn is_new_message(last_seen: Option<&str>, current: &str) -> bool {
    match last_seen {
        None => true,
        Some(prev) => {
            let prev_num = prev.parse::<u64>().ok();
            let cur_num = current.parse::<u64>().ok();
            match (prev_num, cur_num) {
                (Some(p), Some(c)) => c > p,
                _ => current > prev,
            }
        }
    }
}

fn message_mentions_bot(msg: &DiscordChannelMessage, bot_id: &str) -> bool {
    msg.mentions.iter().any(|u| u.id == bot_id)
        || msg.content.contains(&format!("<@{}>", bot_id))
        || msg.content.contains(&format!("<@!{}>", bot_id))
}

fn strip_bot_mention(content: &str, bot_id: &str) -> String {
    content
        .replace(&format!("<@{}>", bot_id), "")
        .replace(&format!("<@!{}>", bot_id), "")
        .trim()
        .to_string()
}

fn discord_auth_headers_json(include_content_type: bool) -> String {
    if include_content_type {
        serde_json::json!({
            "Content-Type": "application/json",
            "Authorization": "Bot {DISCORD_BOT_TOKEN}"
        })
        .to_string()
    } else {
        serde_json::json!({
            "Authorization": "Bot {DISCORD_BOT_TOKEN}"
        })
        .to_string()
    }
}

fn verify_discord_request_signature(
    headers: HashMap<String, String>,
    body: &[u8],
    public_key_hex: Option<&str>,
) -> bool {
    let Some(public_key_hex) = public_key_hex.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let Some(signature_hex) = header_case_insensitive(&headers, "x-signature-ed25519") else {
        return false;
    };
    let Some(timestamp) = header_case_insensitive(&headers, "x-signature-timestamp") else {
        return false;
    };

    let public_key_bytes = match hex::decode(public_key_hex) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let public_key_arr: [u8; 32] = match public_key_bytes.try_into() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let verifying_key = match VerifyingKey::from_bytes(&public_key_arr) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let sig_bytes = match hex::decode(signature_hex.trim()) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let sig_arr: [u8; 64] = match sig_bytes.try_into() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let signature = Signature::from_bytes(&sig_arr);

    let mut signed_message = Vec::with_capacity(timestamp.len() + body.len());
    signed_message.extend_from_slice(timestamp.as_bytes());
    signed_message.extend_from_slice(body);

    verifying_key.verify(&signed_message, &signature).is_ok()
}

fn header_case_insensitive<'a>(
    headers: &'a HashMap<String, String>,
    name: &str,
) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

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

    // DM if no guild member context (only direct user field set).
    let is_dm = interaction.member.is_none();
    if !check_sender_permission(
        &user_id,
        Some(&user_name),
        is_dm,
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
        interaction_id: Some(interaction.id.clone()),
        token: Some(interaction.token.clone()),
        application_id: Some(interaction.application_id.clone()),
        source_message_id: None,
        thread_id: None,
    };

    let metadata_json = match serde_json::to_string(&metadata) {
        Ok(json) => json,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to serialize metadata: {}", e),
            );
            // Attempt to notify user of internal error
            let url = format!(
                "https://discord.com/api/v10/webhooks/{}/{}",
                interaction.application_id, interaction.token
            );
            let payload = serde_json::json!({
                "content": "❌ Internal Error: Failed to process command metadata.",
                "flags": 64 // Ephemeral
            });
            let _ = channel_host::http_request(
                "POST",
                &url,
                &serde_json::json!({"Content-Type": "application/json"}).to_string(),
                Some(&serde_json::to_vec(&payload).unwrap_or_default()),
                None,
            );
            return true;
        }
    };

    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: Some(user_name),
        content,
        thread_id: None,
        metadata_json,
    });
    true
}

fn handle_message_component(interaction: &DiscordInteraction, message: &DiscordMessage) {
    // Check member first (for server contexts), then user (for DMs)
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
    if !check_sender_permission(&user_id, Some(&user_name), is_dm, None) {
        return;
    }

    let channel_id = message.channel_id.clone();

    let metadata = DiscordMessageMetadata {
        channel_id: channel_id.clone(),
        interaction_id: Some(interaction.id.clone()),
        token: Some(interaction.token.clone()),
        application_id: Some(interaction.application_id.clone()),
        source_message_id: None,
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
    });
}

/// Context needed to send a pairing reply via Discord webhook followup.
struct PairingReplyCtx {
    application_id: String,
    token: String,
}

/// Check if a sender is permitted to interact with the bot.
/// Returns true if allowed, false if denied (pairing reply sent if applicable).
fn check_sender_permission(
    user_id: &str,
    username: Option<&str>,
    is_dm: bool,
    reply_ctx: Option<&PairingReplyCtx>,
) -> bool {
    // 1. Owner check (highest priority, applies to all contexts).
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

    // 2. DM policy (only for DMs when no owner_id).
    if !is_dm {
        return true;
    }

    let dm_policy =
        channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| default_dm_policy());
    if dm_policy == "open" {
        return true;
    }

    // 3. Build merged allow list: config allow_from + pairing store.
    let mut allowed: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if let Ok(store_allowed) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        allowed.extend(store_allowed);
    }

    // 4. Check sender against allow list.
    let is_allowed = allowed.contains(&"*".to_string())
        || allowed.contains(&user_id.to_string())
        || username.is_some_and(|u| allowed.contains(&u.to_string()));

    if is_allowed {
        return true;
    }

    // 5. Not allowed - handle by policy.
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
        "flags": 64
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
    if content.len() <= 2000 {
        content.to_string()
    } else {
        let max_bytes = 1990;
        let cutoff = content
            .char_indices()
            .map(|(i, c)| i + c.len_utf8())
            .take_while(|&end| end <= max_bytes)
            .last()
            .unwrap_or(0);
        let mut truncated = content[..cutoff].to_string();
        truncated.push_str("\n... (truncated)");
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn test_truncate_message() {
        let short = "Hello world";
        assert_eq!(truncate_message(short), short);

        let long = "a".repeat(2005);
        let truncated = truncate_message(&long);
        assert_eq!(truncated.len(), 2006); // 1990 + 16 chars suffix
        assert!(truncated.ends_with("\n... (truncated)"));

        // Test with multibyte characters (Euro sign is 3 bytes)
        // 1000 chars * 3 bytes = 3000 bytes
        let multi = "€".repeat(1000);
        let truncated_multi = truncate_message(&multi);

        // 1990 bytes limit. 1990 / 3 = 663 with remainder 1.
        // Should truncate at 663 chars (1989 bytes).
        // Suffix is 16 bytes. Total: 1989 + 16 = 2005 bytes.
        assert!(truncated_multi.len() <= 2006);
        assert!(truncated_multi.len() >= 2006 - 4); // Allow for max utf8 char width variance
        assert!(truncated_multi.ends_with("\n... (truncated)"));

        let content_part = &truncated_multi[..truncated_multi.len() - 16];
        assert!(content_part.chars().all(|c| c == '€'));
    }

    #[test]
    fn test_metadata_serialization() {
        let metadata = DiscordMessageMetadata {
            channel_id: "123".into(),
            interaction_id: Some("456".into()),
            token: Some("abc".into()),
            application_id: Some("789".into()),
            source_message_id: None,
            thread_id: None,
        };
        let json = serde_json::to_string(&metadata).unwrap();
        let parsed: DiscordMessageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.channel_id, "123");
        assert_eq!(parsed.interaction_id.as_deref(), Some("456"));
    }

    #[test]
    fn test_is_new_message() {
        assert!(is_new_message(None, "100"));
        assert!(is_new_message(Some("100"), "200"));
        assert!(!is_new_message(Some("200"), "100"));
        assert!(!is_new_message(Some("100"), "100"));
    }

    #[test]
    fn test_strip_bot_mention() {
        assert_eq!(strip_bot_mention("<@123> hello", "123"), "hello");
        assert_eq!(strip_bot_mention("<@!123> hello", "123"), "hello");
        assert_eq!(strip_bot_mention("<@123>", "123"), "");
        assert_eq!(
            strip_bot_mention("hello <@123> world <@!123>", "123"),
            "hello  world"
        );
    }

    #[test]
    fn test_message_mentions_bot() {
        let msg = DiscordChannelMessage {
            id: "1".to_string(),
            content: "hello <@123>".to_string(),
            channel_id: "10".to_string(),
            author: DiscordChannelAuthor {
                id: "u1".to_string(),
                username: "alice".to_string(),
                global_name: None,
                bot: false,
            },
            mentions: vec![],
            webhook_id: None,
        };
        assert!(message_mentions_bot(&msg, "123"));
        assert!(!message_mentions_bot(&msg, "999"));
    }

    #[test]
    fn test_verify_discord_request_signature_valid() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let timestamp = "1234567890";
        let body = br#"{"type":1}"#;

        let mut signed = Vec::new();
        signed.extend_from_slice(timestamp.as_bytes());
        signed.extend_from_slice(body);
        let signature = signing_key.sign(&signed);

        let mut headers = HashMap::new();
        headers.insert(
            "x-signature-ed25519".to_string(),
            hex::encode(signature.to_bytes()),
        );
        headers.insert("x-signature-timestamp".to_string(), timestamp.to_string());

        assert!(verify_discord_request_signature(
            headers,
            body,
            Some(&public_key_hex)
        ));
    }

    #[test]
    fn test_verify_discord_request_signature_tampered_body() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let timestamp = "1234567890";
        let body = b"hello";

        let mut signed = Vec::new();
        signed.extend_from_slice(timestamp.as_bytes());
        signed.extend_from_slice(body);
        let signature = signing_key.sign(&signed);

        let mut headers = HashMap::new();
        headers.insert(
            "x-signature-ed25519".to_string(),
            hex::encode(signature.to_bytes()),
        );
        headers.insert("x-signature-timestamp".to_string(), timestamp.to_string());

        assert!(!verify_discord_request_signature(
            headers,
            b"hello-modified",
            Some(&public_key_hex)
        ));
    }

    #[test]
    fn test_verify_discord_request_signature_wrong_public_key() {
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let wrong_key = SigningKey::from_bytes(&[12u8; 32]);
        let timestamp = "1234567890";
        let body = b"payload";

        let mut signed = Vec::new();
        signed.extend_from_slice(timestamp.as_bytes());
        signed.extend_from_slice(body);
        let signature = signing_key.sign(&signed);

        let mut headers = HashMap::new();
        headers.insert(
            "x-signature-ed25519".to_string(),
            hex::encode(signature.to_bytes()),
        );
        headers.insert("x-signature-timestamp".to_string(), timestamp.to_string());

        assert!(!verify_discord_request_signature(
            headers,
            body,
            Some(&hex::encode(wrong_key.verifying_key().to_bytes()))
        ));
    }

    #[test]
    fn test_verify_discord_request_signature_missing_headers() {
        let headers = HashMap::new();
        assert!(!verify_discord_request_signature(
            headers,
            b"abc",
            Some("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
        ));
    }

    #[test]
    fn test_verify_discord_request_signature_invalid_signature_hex() {
        let mut headers = HashMap::new();
        headers.insert("x-signature-ed25519".to_string(), "not-hex".to_string());
        headers.insert(
            "x-signature-timestamp".to_string(),
            "1234567890".to_string(),
        );
        assert!(!verify_discord_request_signature(
            headers,
            b"abc",
            Some("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
        ));
    }

    #[test]
    fn test_verify_discord_request_signature_empty_public_key() {
        let mut headers = HashMap::new();
        headers.insert("x-signature-ed25519".to_string(), "00".repeat(64));
        headers.insert(
            "x-signature-timestamp".to_string(),
            "1234567890".to_string(),
        );
        assert!(!verify_discord_request_signature(headers, b"abc", Some("")));
    }
}
