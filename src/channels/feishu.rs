//! Feishu / Lark channel via WebSocket long connection.
//!
//! Maintains a persistent WSS connection to Feishu servers using the
//! official long-connection protocol.  No public IP or webhook endpoint is
//! required — the server pushes events over the WebSocket.
//!
//! # Protocol overview
//!
//! 1. `POST /callback/ws/endpoint` with `AppID` / `AppSecret` → WSS URL + ticket
//! 2. Connect via WSS, receive binary protobuf frames
//! 3. Ping/pong heartbeat to keep the connection alive
//! 4. Event payloads arrive as data frames; replies go via standard HTTP API
//!
//! # Why native instead of WASM
//!
//! IronClaw's current WASM channel contract is callback-oriented: the host owns
//! webhook/polling infrastructure and invokes sandboxed `on-http-request` /
//! `on-poll` handlers with a fresh instance per callback. That model works well
//! for webhook or polling integrations, and the repository already ships a
//! smaller Feishu webhook MVP through the WASM path.
//!
//! Feishu long-connection mode is a different shape entirely. The official
//! protocol requires a persistent WSS session, binary protobuf frames, ongoing
//! heartbeat handling, reconnect loops, reply-context tracking, and process-
//! local dedup state. Those lifecycle and state requirements do not map cleanly
//! onto the current stateless WASM callback boundary, so the long-connection
//! implementation lives here as a native channel while webhook hardening remains
//! relevant for the older WASM/webhook route.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

use super::feishu_proto::{
    Frame, FRAME_DATA, HDR_MESSAGE_ID, HDR_TYPE, MSG_TYPE_EVENT, MSG_TYPE_PONG,
};
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::FeishuConfig;
use crate::error::ChannelError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WS_PING_INTERVAL: Duration = Duration::from_secs(120);
const MAX_REPLY_LEN: usize = 4000;

/// Message-level deduplication window.
const DEDUP_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const DEDUP_MAX_SIZE: usize = 20_000;
const DEDUP_CLEANUP_INTERVAL: Duration = Duration::from_secs(5 * 60);
const DEDUP_PERSIST_INTERVAL: Duration = Duration::from_secs(15);
const DEDUP_STATE_FILE: &str = ".ironclaw/state/feishu_dedup.json";

/// Feishu reaction emoji for typing indicator.
const TYPING_REACTION_EMOJI: &str = "Typing";
const FEISHU_POST_TITLE: &str = "IronClaw";

// ── Feishu HTTP API response shapes ─────────────────────────

#[derive(Debug, Deserialize)]
struct WsEndpointResp {
    code: i64,
    msg: String,
    data: Option<WsEndpointData>,
}

#[derive(Debug, Deserialize)]
struct WsEndpointData {
    #[serde(rename = "URL")]
    url: String,
    #[serde(rename = "ClientConfig")]
    client_config: Option<ClientConfig>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ClientConfig {
    #[serde(rename = "ReconnectCount")]
    reconnect_count: Option<i64>,
    #[serde(rename = "ReconnectInterval")]
    reconnect_interval: Option<i64>,
    #[serde(rename = "ReconnectNonce")]
    reconnect_nonce: Option<i64>,
    #[serde(rename = "PingInterval")]
    ping_interval: Option<i64>,
}

/// Feishu event callback envelope (JSON inside protobuf payload).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct EventEnvelope {
    schema: Option<String>,
    token: Option<String>,
    header: Option<EventHeader>,
    event: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct EventHeader {
    event_id: Option<String>,
    event_type: Option<String>,
    create_time: Option<String>,
    token: Option<String>,
    tenant_key: Option<String>,
}

/// `im.message.receive_v1` event body.
#[derive(Debug, Deserialize)]
struct ImMessageEvent {
    sender: Option<ImSender>,
    message: Option<ImMessage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ImSender {
    sender_id: Option<ImSenderId>,
    sender_type: Option<String>,
    tenant_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ImSenderId {
    open_id: Option<String>,
    user_id: Option<String>,
    union_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImMessage {
    message_id: Option<String>,
    chat_id: Option<String>,
    chat_type: Option<String>,
    content: Option<String>,
    message_type: Option<String>,
    #[serde(default)]
    mentions: Option<Vec<ImMention>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ImMention {
    key: Option<String>,
    id: Option<ImSenderId>,
    name: Option<String>,
    tenant_key: Option<String>,
}

/// Deserialized `content` field when `message_type == "text"`.
#[derive(Debug, Deserialize)]
struct TextContent {
    text: String,
}

/// Deserialized `content` field when `message_type == "post"`.
#[derive(Debug, Deserialize)]
struct PostContent {
    #[allow(dead_code)]
    title: Option<String>,
    content: Option<Vec<Vec<PostElement>>>,
}

/// A single element inside a `post` content block.
#[derive(Debug, Deserialize)]
struct PostElement {
    tag: Option<String>,
    text: Option<String>,
    #[allow(dead_code)]
    href: Option<String>,
}

/// Reply message request body for Feishu HTTP API.
#[derive(Debug, serde::Serialize)]
struct ReplyBody {
    content: String,
    msg_type: String,
}

/// Generic Feishu API response wrapper.
#[derive(Debug, Deserialize)]
struct ApiResp {
    code: i64,
    msg: String,
}

/// Reply/send API response — includes the newly created message_id.
#[derive(Debug, Deserialize)]
struct SendResp {
    code: i64,
    msg: String,
    data: Option<SendRespData>,
}

#[derive(Debug, Deserialize)]
struct SendRespData {
    message_id: Option<String>,
}

/// Reaction API response.
#[derive(Debug, Deserialize)]
struct ReactionResp {
    code: i64,
    msg: String,
    data: Option<ReactionRespData>,
}

#[derive(Debug, Deserialize)]
struct ReactionRespData {
    reaction_id: Option<String>,
}

/// Bot info API response.
#[derive(Debug, Deserialize)]
struct BotInfoResp {
    code: i64,
    msg: String,
    bot: Option<BotInfoData>,
}

#[derive(Debug, Deserialize)]
struct BotInfoData {
    open_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct DedupPersistState {
    seen: HashMap<String, u64>,
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn truncate_for_feishu(content: &str) -> String {
    let char_count = content.chars().count();
    if char_count <= MAX_REPLY_LEN {
        return content.to_string();
    }

    let truncated: String = content.chars().take(MAX_REPLY_LEN).collect();
    format!("{truncated}…")
}

fn build_post_content_from_markdown(content: &str) -> String {
    serde_json::json!({
        "zh_cn": {
            "title": FEISHU_POST_TITLE,
            "content": [[{
                "tag": "md",
                "text": content
            }]]
        }
    })
    .to_string()
}

fn build_interactive_content_from_markdown(content: &str) -> String {
    serde_json::json!({
        "schema": "2.0",
        "config": {
            "wide_screen_mode": true
        },
        "body": {
            "elements": [
                {
                    "tag": "markdown",
                    "content": content
                }
            ]
        }
    })
    .to_string()
}

fn build_text_content_payload(content: &str) -> String {
    serde_json::json!({ "text": content }).to_string()
}

fn has_valid_verification_token(envelope: &EventEnvelope, expected: Option<&str>) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|token| !token.is_empty()) else {
        return true;
    };

    let provided = envelope
        .token
        .as_deref()
        .or_else(|| envelope.header.as_ref().and_then(|h| h.token.as_deref()))
        .map(str::trim)
        .unwrap_or("");

    provided == expected
}

fn should_fallback_to_text(err: &ChannelError) -> bool {
    match err {
        ChannelError::SendFailed { reason, .. } => {
            let reason_lower = reason.to_ascii_lowercase();
            reason_lower.contains("api error 230001")
                || reason_lower.contains("invalid request parameter")
                || reason_lower.contains("invalid param")
                || reason_lower.contains("msg_type")
                || reason_lower.contains("content")
        }
        _ => false,
    }
}

// ── Channel implementation ──────────────────────────────────

pub struct FeishuChannel {
    config: FeishuConfig,
    client: Client,
    /// Shared reference for `respond()` to look up the original Feishu message_id.
    reply_map: Arc<tokio::sync::RwLock<lru::LruCache<Uuid, ReplyContext>>>,
    /// Maps feishu_message_id → reaction_id of the typing indicator.
    typing_map: Arc<tokio::sync::RwLock<HashMap<String, String>>>,
}

struct ReplyContext {
    message_id: String,
    chat_id: Option<String>,
}

impl FeishuChannel {
    pub fn new(config: FeishuConfig) -> Result<Self, ChannelError> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        let cap = std::num::NonZeroUsize::new(10_000).expect("reply map capacity is non-zero");
        let reply_map = Arc::new(tokio::sync::RwLock::new(lru::LruCache::new(cap)));
        let typing_map = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

        Ok(Self {
            config,
            client,
            reply_map,
            typing_map,
        })
    }

    /// Acquire a WSS URL from the Feishu open-platform endpoint.
    async fn acquire_ws_url(
        client: &Client,
        config: &FeishuConfig,
    ) -> Result<(String, Option<ClientConfig>), ChannelError> {
        let url = format!("{}/callback/ws/endpoint", config.api_base);
        let body = serde_json::json!({
            "AppID": config.app_id,
            "AppSecret": config.app_secret,
        });

        let resp: WsEndpointResp = client
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: "feishu".into(),
                reason: format!("ws endpoint request failed: {e}"),
            })?
            .json()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: "feishu".into(),
                reason: format!("ws endpoint response parse error: {e}"),
            })?;

        if resp.code != 0 {
            return Err(ChannelError::StartupFailed {
                name: "feishu".into(),
                reason: format!("ws endpoint error {}: {}", resp.code, resp.msg),
            });
        }

        let data = resp.data.ok_or_else(|| ChannelError::StartupFailed {
            name: "feishu".into(),
            reason: "ws endpoint returned no data".into(),
        })?;

        Ok((data.url, data.client_config))
    }

    /// Obtain a tenant_access_token for sending replies.
    async fn get_tenant_token(
        client: &Client,
        config: &FeishuConfig,
    ) -> Result<String, ChannelError> {
        let url = format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal",
            config.api_base
        );
        let body = serde_json::json!({
            "app_id": config.app_id,
            "app_secret": config.app_secret,
        });

        #[derive(Deserialize)]
        struct TokenResp {
            code: i64,
            msg: String,
            tenant_access_token: Option<String>,
        }

        let resp: TokenResp = client
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::Http(e.to_string()))?
            .json()
            .await
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        if resp.code != 0 {
            return Err(ChannelError::AuthFailed {
                name: "feishu".into(),
                reason: format!("tenant_access_token error {}: {}", resp.code, resp.msg),
            });
        }

        resp.tenant_access_token
            .ok_or_else(|| ChannelError::AuthFailed {
                name: "feishu".into(),
                reason: "empty tenant_access_token".into(),
            })
    }

    /// Fetch the bot's own open_id via the Feishu Bot Info API.
    async fn get_bot_open_id(
        client: &Client,
        config: &FeishuConfig,
    ) -> Result<String, ChannelError> {
        let token = Self::get_tenant_token(client, config).await?;
        let url = format!("{}/open-apis/bot/v3/info", config.api_base);

        let resp: BotInfoResp = client
            .get(&url)
            .bearer_auth(&token)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::Http(e.to_string()))?
            .json()
            .await
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        if resp.code != 0 {
            return Err(ChannelError::StartupFailed {
                name: "feishu".into(),
                reason: format!("bot info error {}: {}", resp.code, resp.msg),
            });
        }

        resp.bot
            .and_then(|b| b.open_id)
            .ok_or_else(|| ChannelError::StartupFailed {
                name: "feishu".into(),
                reason: "bot info returned no open_id".into(),
            })
    }

    async fn reply_message_with_body(
        client: &Client,
        config: &FeishuConfig,
        message_id: &str,
        body: &ReplyBody,
    ) -> Result<Option<String>, ChannelError> {
        let token = Self::get_tenant_token(client, config).await?;
        let url = format!(
            "{}/open-apis/im/v1/messages/{}/reply",
            config.api_base, message_id
        );

        let resp: SendResp = client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "feishu".into(),
                reason: format!("reply HTTP error: {e}"),
            })?
            .json()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "feishu".into(),
                reason: format!("reply response parse error: {e}"),
            })?;

        if resp.code != 0 {
            return Err(ChannelError::SendFailed {
                name: "feishu".into(),
                reason: format!("reply API error {}: {}", resp.code, resp.msg),
            });
        }
        Ok(resp.data.and_then(|d| d.message_id))
    }

    /// Reply to a Feishu message via the HTTP API.
    async fn reply_message(
        client: &Client,
        config: &FeishuConfig,
        message_id: &str,
        content: &str,
    ) -> Result<Option<String>, ChannelError> {
        let text = truncate_for_feishu(content);
        let interactive_body = ReplyBody {
            content: build_interactive_content_from_markdown(&text),
            msg_type: "interactive".into(),
        };

        match Self::reply_message_with_body(client, config, message_id, &interactive_body).await {
            Ok(reply_message_id) => {
                tracing::debug!(
                    send_mode = "interactive",
                    message_id,
                    "Feishu: reply sent"
                );
                Ok(reply_message_id)
            }
            Err(e) if should_fallback_to_text(&e) => {
                tracing::warn!(
                    error = %e,
                    send_mode = "interactive",
                    "Feishu: interactive reply failed, falling back to post"
                );
                let post_body = ReplyBody {
                    content: build_post_content_from_markdown(&text),
                    msg_type: "post".into(),
                };
                match Self::reply_message_with_body(client, config, message_id, &post_body).await {
                    Ok(reply_message_id) => {
                        tracing::debug!(
                            send_mode = "post_fallback",
                            message_id,
                            "Feishu: reply sent"
                        );
                        Ok(reply_message_id)
                    }
                    Err(e2) if should_fallback_to_text(&e2) => {
                        tracing::warn!(
                            error = %e2,
                            send_mode = "post_fallback",
                            "Feishu: post fallback failed, falling back to text"
                        );
                        let text_body = ReplyBody {
                            content: build_text_content_payload(&text),
                            msg_type: "text".into(),
                        };
                        let reply_message_id = Self::reply_message_with_body(
                            client, config, message_id, &text_body,
                        )
                        .await?;
                        tracing::debug!(
                            send_mode = "text_fallback",
                            message_id,
                            "Feishu: reply sent"
                        );
                        Ok(reply_message_id)
                    }
                    Err(e2) => Err(e2),
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn send_message_with_body(
        client: &Client,
        config: &FeishuConfig,
        _receive_id: &str,
        body: serde_json::Value,
    ) -> Result<(), ChannelError> {
        let token = Self::get_tenant_token(client, config).await?;
        let url = format!(
            "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
            config.api_base
        );

        let resp: ApiResp = client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "feishu".into(),
                reason: format!("send HTTP error: {e}"),
            })?
            .json()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "feishu".into(),
                reason: format!("send response parse error: {e}"),
            })?;

        if resp.code != 0 {
            return Err(ChannelError::SendFailed {
                name: "feishu".into(),
                reason: format!("send API error {}: {}", resp.code, resp.msg),
            });
        }
        Ok(())
    }

    /// Send a proactive message to a specific chat via the HTTP API.
    async fn send_message(
        client: &Client,
        config: &FeishuConfig,
        receive_id: &str,
        content: &str,
    ) -> Result<(), ChannelError> {
        let text = truncate_for_feishu(content);
        let interactive_body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": "interactive",
            "content": build_interactive_content_from_markdown(&text),
        });

        match Self::send_message_with_body(client, config, receive_id, interactive_body).await {
            Ok(()) => {
                tracing::debug!(
                    send_mode = "interactive",
                    receive_id,
                    "Feishu: proactive message sent"
                );
                Ok(())
            }
            Err(e) if should_fallback_to_text(&e) => {
                tracing::warn!(
                    error = %e,
                    send_mode = "interactive",
                    "Feishu: interactive send failed, falling back to post"
                );
                let post_body = serde_json::json!({
                    "receive_id": receive_id,
                    "msg_type": "post",
                    "content": build_post_content_from_markdown(&text),
                });
                match Self::send_message_with_body(client, config, receive_id, post_body).await {
                    Ok(()) => {
                        tracing::debug!(
                            send_mode = "post_fallback",
                            receive_id,
                            "Feishu: proactive message sent"
                        );
                        Ok(())
                    }
                    Err(e2) if should_fallback_to_text(&e2) => {
                        tracing::warn!(
                            error = %e2,
                            send_mode = "post_fallback",
                            "Feishu: post fallback failed, falling back to text"
                        );
                        let text_body = serde_json::json!({
                            "receive_id": receive_id,
                            "msg_type": "text",
                            "content": build_text_content_payload(&text),
                        });
                        Self::send_message_with_body(client, config, receive_id, text_body).await?;
                        tracing::debug!(
                            send_mode = "text_fallback",
                            receive_id,
                            "Feishu: proactive message sent"
                        );
                        Ok(())
                    }
                    Err(e2) => Err(e2),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Add a "Typing" reaction to a user's message as a typing indicator.
    async fn add_typing_reaction(
        client: &Client,
        config: &FeishuConfig,
        message_id: &str,
    ) -> Result<Option<String>, ChannelError> {
        let token = Self::get_tenant_token(client, config).await?;
        let url = format!(
            "{}/open-apis/im/v1/messages/{}/reactions",
            config.api_base, message_id
        );

        let body = serde_json::json!({
            "reaction_type": { "emoji_type": TYPING_REACTION_EMOJI }
        });

        let resp: ReactionResp = client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::Http(e.to_string()))?
            .json()
            .await
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        if resp.code != 0 {
            return Err(ChannelError::SendFailed {
                name: "feishu".into(),
                reason: format!("add reaction error {}: {}", resp.code, resp.msg),
            });
        }

        Ok(resp.data.and_then(|d| d.reaction_id))
    }

    /// Remove a typing reaction from a message.
    async fn remove_typing_reaction(
        client: &Client,
        config: &FeishuConfig,
        message_id: &str,
        reaction_id: &str,
    ) {
        let token = match Self::get_tenant_token(client, config).await {
            Ok(t) => t,
            Err(_) => return,
        };
        let url = format!(
            "{}/open-apis/im/v1/messages/{}/reactions/{}",
            config.api_base, message_id, reaction_id
        );

        let _ = client
            .delete(&url)
            .bearer_auth(&token)
            .timeout(Duration::from_secs(10))
            .send()
            .await;
    }
}

#[async_trait]
impl Channel for FeishuChannel {
    fn name(&self) -> &str {
        "feishu"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel::<IncomingMessage>(256);

        let config = self.config.clone();
        let client = self.client.clone();
        let reply_map = Arc::clone(&self.reply_map);

        let bot_open_id = match Self::get_bot_open_id(&self.client, &self.config).await {
            Ok(id) => {
                tracing::info!(bot_open_id = %id, "Feishu: resolved bot identity");
                Some(id)
            }
            Err(e) => {
                tracing::warn!("Feishu: failed to get bot open_id ({e}), @mention filtering disabled");
                None
            }
        };

        tokio::spawn(async move {
            ws_loop(config, client, tx, reply_map, bot_open_id).await;
        });

        tracing::info!("Feishu channel started (WebSocket long connection)");
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let feishu_msg_id = msg
            .metadata
            .get("feishu_message_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Remove typing reaction if we added one for this message.
        if let Some(ref fid) = feishu_msg_id {
            if let Some(reaction_id) = self.typing_map.write().await.remove(fid) {
                Self::remove_typing_reaction(&self.client, &self.config, fid, &reaction_id).await;
            }
        }

        // Send the actual reply.
        let ctx = {
            let map = self.reply_map.read().await;
            map.peek(&msg.id).map(|c| ReplyContext {
                message_id: c.message_id.clone(),
                chat_id: c.chat_id.clone(),
            })
        };

        if let Some(ctx) = ctx {
            Self::reply_message(&self.client, &self.config, &ctx.message_id, &response.content)
                .await?;
        } else if let Some(chat_id) = msg.metadata.get("chat_id").and_then(|v| v.as_str()) {
            Self::send_message(&self.client, &self.config, chat_id, &response.content).await?;
        } else {
            tracing::warn!(msg_id = %msg.id, "Feishu: no reply context, dropping response");
        }

        self.reply_map.write().await.pop(&msg.id);
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        if !matches!(status, StatusUpdate::Thinking(_)) {
            return Ok(());
        }

        let feishu_msg_id = match metadata.get("feishu_message_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return Ok(()),
        };

        if self.typing_map.read().await.contains_key(&feishu_msg_id) {
            return Ok(());
        }

        match Self::add_typing_reaction(&self.client, &self.config, &feishu_msg_id).await {
            Ok(Some(reaction_id)) => {
                self.typing_map
                    .write()
                    .await
                    .insert(feishu_msg_id, reaction_id);
            }
            Ok(None) => {
                tracing::debug!("Feishu: typing reaction added but no reaction_id returned");
            }
            Err(e) => {
                tracing::debug!("Feishu: typing reaction failed: {e}");
            }
        }

        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Self::send_message(&self.client, &self.config, user_id, &response.content).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    fn conversation_context(
        &self,
        metadata: &serde_json::Value,
    ) -> std::collections::HashMap<String, String> {
        let mut ctx = std::collections::HashMap::new();
        if let Some(v) = metadata.get("sender_open_id").and_then(|v| v.as_str()) {
            ctx.insert("sender".into(), v.into());
        }
        if let Some(v) = metadata.get("chat_id").and_then(|v| v.as_str()) {
            ctx.insert("group".into(), v.into());
        }
        if let Some(v) = metadata.get("chat_type").and_then(|v| v.as_str()) {
            ctx.insert("chat_type".into(), v.into());
        }
        ctx
    }
}

// ── Message deduplication ───────────────────────────────────

struct DedupCache {
    seen: HashMap<String, u64>,
    last_cleanup: Instant,
    last_persist: Instant,
    state_path: Option<PathBuf>,
    dirty: bool,
}

impl DedupCache {
    fn new() -> Self {
        let mut cache = Self {
            seen: HashMap::new(),
            last_cleanup: Instant::now(),
            last_persist: Instant::now(),
            state_path: std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(DEDUP_STATE_FILE)),
            dirty: false,
        };
        cache.load_from_disk();
        cache.cleanup_expired_and_oversize(now_unix_secs());
        cache
    }

    fn load_from_disk(&mut self) {
        let Some(path) = &self.state_path else {
            return;
        };

        let raw = match fs::read_to_string(path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "Feishu: failed to read dedup state");
                return;
            }
        };

        let state: DedupPersistState = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "Feishu: failed to parse dedup state");
                return;
            }
        };

        let loaded = state.seen.len();
        self.seen = state.seen;
        tracing::info!(
            dedup_persist_load_count = loaded,
            dedup_loaded_count = self.seen.len(),
            path = %path.display(),
            "Feishu: dedup cache loaded from disk"
        );
    }

    fn cleanup_expired_and_oversize(&mut self, now_secs: u64) {
        let ttl_secs = DEDUP_TTL.as_secs();
        self.seen
            .retain(|_, ts| now_secs.saturating_sub(*ts) < ttl_secs);

        if self.seen.len() > DEDUP_MAX_SIZE {
            let mut entries: Vec<(String, u64)> =
                self.seen.iter().map(|(k, v)| (k.clone(), *v)).collect();
            entries.sort_by_key(|(_, ts)| *ts);
            let remove_n = entries.len().saturating_sub(DEDUP_MAX_SIZE);
            for (k, _) in entries.into_iter().take(remove_n) {
                self.seen.remove(&k);
            }
        }
    }

    fn maybe_cleanup(&mut self, now: Instant, now_secs: u64) {
        if now.duration_since(self.last_cleanup) > DEDUP_CLEANUP_INTERVAL {
            self.cleanup_expired_and_oversize(now_secs);
            self.last_cleanup = now;
            self.maybe_persist(false);
        }
    }

    fn maybe_persist(&mut self, force: bool) {
        if !self.dirty {
            return;
        }
        if !force && self.last_persist.elapsed() < DEDUP_PERSIST_INTERVAL {
            return;
        }

        let Some(path) = &self.state_path else {
            return;
        };
        if let Some(parent) = path.parent()
            && let Err(e) = fs::create_dir_all(parent)
        {
            tracing::warn!(
                path = %parent.display(),
                error = %e,
                "Feishu: failed to create dedup state directory"
            );
            return;
        }

        let state = DedupPersistState {
            seen: self.seen.clone(),
        };
        let bytes = match serde_json::to_vec(&state) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "Feishu: failed to serialize dedup state");
                return;
            }
        };

        if let Err(e) = fs::write(path, bytes) {
            tracing::warn!(path = %path.display(), error = %e, "Feishu: failed to write dedup state");
            return;
        }

        self.last_persist = Instant::now();
        self.dirty = false;
        tracing::debug!(
            dedup_persist_save_count = self.seen.len(),
            path = %path.display(),
            "Feishu: dedup cache persisted"
        );
    }

    fn flush(&mut self) {
        self.maybe_persist(true);
    }

    fn key(kind: &str, raw_id: &str) -> String {
        format!("{kind}:{raw_id}")
    }

    /// Returns `true` if this dedup key has NOT been seen before (i.e. is new).
    fn try_record(&mut self, kind: &str, id: &str) -> bool {
        if id.trim().is_empty() {
            return true;
        }

        let now = Instant::now();
        let now_secs = now_unix_secs();
        self.maybe_cleanup(now, now_secs);
        let key = Self::key(kind, id);

        if self.seen.contains_key(&key) {
            return false;
        }

        self.seen.insert(key, now_secs);
        self.dirty = true;
        self.maybe_persist(false);
        true
    }
}

// ── WebSocket event loop ────────────────────────────────────

async fn ws_loop(
    config: FeishuConfig,
    client: Client,
    tx: mpsc::Sender<IncomingMessage>,
    reply_map: Arc<tokio::sync::RwLock<lru::LruCache<Uuid, ReplyContext>>>,
    bot_open_id: Option<String>,
) {
    let mut dedup = DedupCache::new();
    let boot_time = now_unix_secs();

    loop {
        match ws_session(&config, &client, &tx, &reply_map, &bot_open_id, &mut dedup, boot_time).await {
            Ok(()) => {
                tracing::info!("Feishu WebSocket session ended normally, reconnecting…");
            }
            Err(e) => {
                tracing::error!("Feishu WebSocket error: {e}, reconnecting…");
            }
        }
        dedup.flush();
        tokio::time::sleep(Duration::from_secs(config.reconnect_delay_secs)).await;
    }
}

async fn ws_session(
    config: &FeishuConfig,
    client: &Client,
    tx: &mpsc::Sender<IncomingMessage>,
    reply_map: &Arc<tokio::sync::RwLock<lru::LruCache<Uuid, ReplyContext>>>,
    bot_open_id: &Option<String>,
    dedup: &mut DedupCache,
    boot_time: u64,
) -> Result<(), ChannelError> {
    let (ws_url, client_config) = FeishuChannel::acquire_ws_url(client, config).await?;
    tracing::info!("Feishu: acquired WebSocket URL, connecting…");

    let ping_interval = client_config
        .as_ref()
        .and_then(|c| c.ping_interval)
        .and_then(|v| u64::try_from(v).ok())
        .map(Duration::from_secs)
        .unwrap_or(WS_PING_INTERVAL);

    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| ChannelError::StartupFailed {
            name: "feishu".into(),
            reason: format!("WebSocket connect failed: {e}"),
        })?;

    tracing::info!("Feishu: WebSocket connected");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();
    let mut ping_ticker = tokio::time::interval(ping_interval);
    ping_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = ping_ticker.tick() => {
                let ping_frame = Frame::new_ping(0);
                let bytes = ping_frame.encode_to_vec();
                if let Err(e) = ws_tx.send(WsMessage::Binary(bytes.into())).await {
                    tracing::error!("Feishu: ping send error: {e}");
                    return Ok(());
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(WsMessage::Binary(data))) => {
                        handle_binary_frame(
                            &data,
                            tx,
                            reply_map,
                            bot_open_id,
                            dedup,
                            boot_time,
                            config.verification_token.as_deref(),
                        )
                        .await;
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        let _ = ws_tx.send(WsMessage::Pong(data)).await;
                    }
                    Some(Ok(WsMessage::Close(_))) | None => {
                        tracing::info!("Feishu: WebSocket closed by server");
                        return Ok(());
                    }
                    Some(Err(e)) => {
                        tracing::error!("Feishu: WebSocket read error: {e}");
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn handle_binary_frame(
    data: &[u8],
    tx: &mpsc::Sender<IncomingMessage>,
    reply_map: &Arc<tokio::sync::RwLock<lru::LruCache<Uuid, ReplyContext>>>,
    bot_open_id: &Option<String>,
    dedup: &mut DedupCache,
    boot_time: u64,
    verification_token: Option<&str>,
) {
    let frame = match Frame::decode(data) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Feishu: protobuf decode error: {e}");
            return;
        }
    };

    let msg_type = frame.header_str(HDR_TYPE);

    if msg_type == MSG_TYPE_PONG {
        tracing::trace!("Feishu: pong received");
        return;
    }

    if frame.method != FRAME_DATA || msg_type != MSG_TYPE_EVENT {
        tracing::trace!(method = frame.method, msg_type, "Feishu: non-event frame, skipping");
        return;
    }

    let payload_str = match std::str::from_utf8(&frame.payload) {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!("Feishu: event payload is not valid UTF-8");
            return;
        }
    };

    let feishu_msg_id = frame.header_str(HDR_MESSAGE_ID).to_string();
    tracing::debug!(feishu_msg_id, "Feishu: received event frame");

    let envelope: EventEnvelope = match serde_json::from_str(payload_str) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Feishu: event JSON parse error: {e}");
            return;
        }
    };

    if !has_valid_verification_token(&envelope, verification_token) {
        tracing::warn!("Feishu: verification token mismatch, dropping event");
        return;
    }

    let event_type = envelope
        .header
        .as_ref()
        .and_then(|h| h.event_type.as_deref())
        .unwrap_or("");

    if event_type != "im.message.receive_v1" {
        tracing::debug!(event_type, "Feishu: unhandled event type, skipping");
        return;
    }

    let skip_pre_boot = std::env::var("FEISHU_SKIP_PRE_BOOT")
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true);
    if skip_pre_boot {
        if let Some(ct) = envelope.header.as_ref().and_then(|h| h.create_time.as_deref()) {
            if let Ok(event_ms) = ct.parse::<u64>() {
                let event_secs = event_ms / 1000;
                if event_secs < boot_time {
                    tracing::info!(
                        create_time = ct,
                        boot_time,
                        "Feishu: skipping pre-boot event"
                    );
                    return;
                }
            }
        }
    }

    let event_id = envelope
        .header
        .as_ref()
        .and_then(|h| h.event_id.as_deref())
        .unwrap_or("");
    if !event_id.is_empty() && !dedup.try_record("event_id", event_id) {
        tracing::debug!(
            event_id,
            dedup_hit_key = "event_id",
            "Feishu: duplicate event, skipping"
        );
        return;
    }

    let Some(event_val) = envelope.event else {
        tracing::debug!("Feishu: envelope.event is None, skipping");
        return;
    };
    let im_event: ImMessageEvent = match serde_json::from_value(event_val) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Feishu: im event parse error: {e}");
            return;
        }
    };

    // ── Fix 5: Bot self-echo filter ──
    let sender_type = im_event
        .sender
        .as_ref()
        .and_then(|s| s.sender_type.as_deref())
        .unwrap_or("user");

    if sender_type != "user" {
        tracing::debug!(sender_type, "Feishu: non-user message, skipping");
        return;
    }

    let Some(im_msg) = im_event.message else {
        return;
    };

    // ── Fix 1: Message-level dedup ──
    let msg_id_feishu = im_msg.message_id.clone().unwrap_or_default();
    if !msg_id_feishu.is_empty() && !dedup.try_record("message_id", &msg_id_feishu) {
        tracing::debug!(
            msg_id_feishu,
            dedup_hit_key = "message_id",
            "Feishu: duplicate message, skipping"
        );
        return;
    }

    // ── Fix 2: Group @mention filter ──
    let is_group = im_msg.chat_type.as_deref() == Some("group");
    if is_group {
        if let Some(bot_oid) = bot_open_id {
            let bot_mentioned = im_msg
                .mentions
                .as_ref()
                .map(|ms| {
                    ms.iter().any(|m| {
                        m.id.as_ref()
                            .and_then(|id| id.open_id.as_deref())
                            == Some(bot_oid.as_str())
                    })
                })
                .unwrap_or(false);

            if !bot_mentioned {
                tracing::debug!(msg_id_feishu, "Feishu: group message without @bot, skipping");
                return;
            }
        }
    }

    // ── Fix 6: Support text + post message types ──
    let msg_type_str = im_msg.message_type.as_deref().unwrap_or("");
    let raw_content = im_msg.content.as_deref().unwrap_or("");

    let raw_text = match msg_type_str {
        "text" => extract_text_content(raw_content),
        "post" => extract_post_text(raw_content),
        _ => {
            tracing::debug!(msg_type = msg_type_str, "Feishu: unsupported message type, skipping");
            return;
        }
    };

    let Some(raw_text) = raw_text else {
        tracing::debug!(msg_type = msg_type_str, raw_content, "Feishu: text extraction returned None, skipping");
        return;
    };

    // ── Fix 3: Strip @mention placeholders ──
    let text = strip_mention_tags(&raw_text, im_msg.mentions.as_deref().unwrap_or(&[]), bot_open_id);

    if text.trim().is_empty() {
        tracing::debug!(raw_text = %raw_text, "Feishu: text empty after stripping mentions, skipping");
        return;
    }

    let sender_open_id = im_event
        .sender
        .as_ref()
        .and_then(|s| s.sender_id.as_ref())
        .and_then(|id| id.open_id.as_deref())
        .unwrap_or("unknown")
        .to_string();

    let chat_id = im_msg.chat_id.clone();
    let chat_type = im_msg.chat_type.clone();

    let thread_id = chat_id
        .as_deref()
        .map(|cid| Uuid::new_v5(&Uuid::NAMESPACE_URL, cid.as_bytes()).to_string());

    let incoming_id = Uuid::new_v4();

    let metadata = serde_json::json!({
        "sender_open_id": sender_open_id,
        "feishu_message_id": msg_id_feishu,
        "chat_id": chat_id,
        "chat_type": chat_type,
    });

    let mut incoming = IncomingMessage::new("feishu", &sender_open_id, text)
        .with_metadata(metadata);
    incoming.id = incoming_id;
    if let Some(tid) = thread_id {
        incoming = incoming.with_thread(tid);
    }

    {
        let mut map = reply_map.write().await;
        map.put(
            incoming_id,
            ReplyContext {
                message_id: msg_id_feishu.clone(),
                chat_id,
            },
        );
    }

    if tx.send(incoming).await.is_err() {
        tracing::warn!("Feishu: message channel closed, event dropped");
    } else {
        tracing::debug!(msg_id = %msg_id_feishu, %sender_open_id, "Feishu: message dispatched to orchestrator");
    }
}

// ── Text extraction helpers ─────────────────────────────────

/// Extract text from a `{"text": "..."}` JSON content field.
fn extract_text_content(raw: &str) -> Option<String> {
    match serde_json::from_str::<TextContent>(raw) {
        Ok(tc) if !tc.text.trim().is_empty() => Some(tc.text),
        _ => None,
    }
}

/// Extract plain text from a `post` rich-text content field.
/// Walks the content array and concatenates text/link elements.
fn extract_post_text(raw: &str) -> Option<String> {
    // Post content may be locale-keyed: {"zh_cn": {title, content}} or flat {title, content}.
    let post: PostContent = serde_json::from_str::<PostContent>(raw)
        .ok()
        .filter(|post| post.content.is_some())
        .or_else(|| {
            let map: HashMap<String, PostContent> = serde_json::from_str(raw).ok()?;
            map.into_values().next()
        })?;

    let content = post.content?;
    let mut parts: Vec<String> = Vec::new();
    for line in &content {
        let mut line_parts: Vec<&str> = Vec::new();
        for elem in line {
            match elem.tag.as_deref() {
                Some("text") => {
                    if let Some(t) = &elem.text {
                        line_parts.push(t.as_str());
                    }
                }
                Some("a") => {
                    if let Some(t) = &elem.text {
                        line_parts.push(t.as_str());
                    }
                }
                _ => {}
            }
        }
        let joined = line_parts.join("");
        if !joined.is_empty() {
            parts.push(joined);
        }
    }

    let text = parts.join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Remove @mention placeholder keys (e.g. `@_user_1`) from message text,
/// and specifically strip the bot's own @mention.
fn strip_mention_tags(text: &str, mentions: &[ImMention], bot_open_id: &Option<String>) -> String {
    let mut result = text.to_string();

    for mention in mentions {
        let is_bot = bot_open_id.as_ref().is_some_and(|bot_oid| {
            mention
                .id
                .as_ref()
                .and_then(|id| id.open_id.as_deref())
                == Some(bot_oid.as_str())
        });

        // Always strip the bot's @mention; keep other users' @names.
        if is_bot {
            if let Some(key) = &mention.key {
                result = result.replace(key.as_str(), "");
            }
            if let Some(name) = &mention.name {
                result = result.replace(&format!("@{name}"), "");
            }
        }
    }

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mention(key: &str, name: &str, open_id: &str) -> ImMention {
        ImMention {
            key: Some(key.to_string()),
            id: Some(ImSenderId {
                open_id: Some(open_id.to_string()),
                user_id: None,
                union_id: None,
            }),
            name: Some(name.to_string()),
            tenant_key: None,
        }
    }

    fn make_dedup_cache(state_path: Option<PathBuf>) -> DedupCache {
        DedupCache {
            seen: HashMap::new(),
            last_cleanup: Instant::now(),
            last_persist: Instant::now(),
            state_path,
            dirty: false,
        }
    }

    #[test]
    fn test_truncate_for_feishu_preserves_unicode_boundary() {
        let text = "你".repeat(MAX_REPLY_LEN + 1);
        let truncated = truncate_for_feishu(&text);

        assert!(truncated.ends_with('…'));
        assert_eq!(truncated.chars().count(), MAX_REPLY_LEN + 1);
        assert!(truncated.trim_end_matches('…').chars().all(|ch| ch == '你'));
    }

    #[test]
    fn test_extract_post_text_flat_content() {
        let raw = serde_json::json!({
            "title": "ignored",
            "content": [
                [
                    {"tag": "text", "text": "hello"},
                    {"tag": "a", "text": " world"},
                    {"tag": "img", "text": "ignored"}
                ],
                [
                    {"tag": "text", "text": "next"}
                ]
            ]
        })
        .to_string();

        assert_eq!(extract_post_text(&raw), Some("hello world\nnext".to_string()));
    }

    #[test]
    fn test_extract_post_text_locale_keyed_content() {
        let raw = serde_json::json!({
            "zh_cn": {
                "title": "ignored",
                "content": [
                    [
                        {"tag": "text", "text": "你好"},
                        {"tag": "a", "text": "世界"}
                    ]
                ]
            }
        })
        .to_string();

        assert_eq!(extract_post_text(&raw), Some("你好世界".to_string()));
    }

    #[test]
    fn test_strip_mention_tags_only_removes_bot_mentions() {
        let text = "@IronClaw @_bot hello @_user_2";
        let mentions = vec![
            mention("@_bot", "IronClaw", "bot-open-id"),
            mention("@_user_2", "Alice", "user-open-id"),
        ];

        let stripped = strip_mention_tags(text, &mentions, &Some("bot-open-id".to_string()));

        assert_eq!(stripped, "hello @_user_2");
    }

    #[test]
    fn test_should_fallback_to_text_for_invalid_param_errors() {
        let err = ChannelError::SendFailed {
            name: "feishu".to_string(),
            reason: "reply API error 230001: invalid request parameter msg_type".to_string(),
        };

        assert!(should_fallback_to_text(&err));
    }

    #[test]
    fn test_should_not_fallback_to_text_for_unrelated_errors() {
        let err = ChannelError::SendFailed {
            name: "feishu".to_string(),
            reason: "temporary upstream outage".to_string(),
        };

        assert!(!should_fallback_to_text(&err));
    }

    #[test]
    fn test_has_valid_verification_token_accepts_root_or_header_token() {
        let root_token = EventEnvelope {
            schema: None,
            token: Some("expected".to_string()),
            header: None,
            event: None,
        };
        let header_token = EventEnvelope {
            schema: None,
            token: None,
            header: Some(EventHeader {
                event_id: None,
                event_type: None,
                create_time: None,
                token: Some("expected".to_string()),
                tenant_key: None,
            }),
            event: None,
        };

        assert!(has_valid_verification_token(&root_token, Some("expected")));
        assert!(has_valid_verification_token(&header_token, Some("expected")));
        assert!(!has_valid_verification_token(&header_token, Some("other")));
        assert!(has_valid_verification_token(&header_token, None));
    }

    #[test]
    fn test_dedup_try_record_rejects_duplicates() {
        let mut cache = make_dedup_cache(None);

        assert!(cache.try_record("message_id", "abc"));
        assert!(!cache.try_record("message_id", "abc"));
        assert!(cache.try_record("message_id", "def"));
    }

    #[test]
    fn test_dedup_cleanup_removes_expired_and_oversize_entries() {
        let mut cache = make_dedup_cache(None);
        let now = DEDUP_TTL.as_secs() + 10;

        cache
            .seen
            .insert(DedupCache::key("message_id", "expired"), 0);
        for idx in 0..(DEDUP_MAX_SIZE + 2) {
            cache
                .seen
                .insert(DedupCache::key("message_id", &format!("fresh-{idx}")), now + idx as u64);
        }

        cache.cleanup_expired_and_oversize(now);

        assert_eq!(cache.seen.len(), DEDUP_MAX_SIZE);
        assert!(!cache.seen.contains_key("message_id:expired"));
        assert!(!cache.seen.contains_key("message_id:fresh-0"));
        assert!(!cache.seen.contains_key("message_id:fresh-1"));
        assert!(cache.seen.contains_key("message_id:fresh-2"));
    }

    #[test]
    fn test_dedup_persist_round_trip() {
        let path =
            std::env::temp_dir().join(format!("ironclaw-feishu-dedup-{}.json", Uuid::new_v4()));
        let mut cache = make_dedup_cache(Some(path.clone()));
        cache
            .seen
            .insert(DedupCache::key("message_id", "persisted"), 123);
        cache.dirty = true;
        cache.maybe_persist(true);

        let mut loaded = make_dedup_cache(Some(path.clone()));
        loaded.load_from_disk();

        assert_eq!(
            loaded.seen.get("message_id:persisted"),
            Some(&123),
        );

        let _ = std::fs::remove_file(path);
    }
}
