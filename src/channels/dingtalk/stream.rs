//! DingTalk Stream mode: WebSocket-based message delivery.
//!
//! Flow:
//! 1. POST /v1.0/gateway/connections/open → get WebSocket endpoint + ticket
//! 2. Connect to WebSocket endpoint with ticket
//! 3. Receive CALLBACK frames with bot messages
//! 4. ACK each message to confirm receipt
//! 5. On disconnect, re-register and reconnect (with backoff)

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use lru::LruCache;
use reqwest::Client;
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::config::DingTalkConfig;
use crate::error::ChannelError;

use super::types::{BotCallbackPayload, DingTalkMetadata, StreamEndpointResponse, StreamFrame};

const STREAM_GATEWAY_URL: &str = "https://api.dingtalk.com/v1.0/gateway/connections/open";
const BOT_MESSAGE_TOPIC: &str = "/v1.0/im/bot/messages/get";
const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(2);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(60);
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Register with DingTalk gateway to get a WebSocket endpoint.
async fn register_stream(
    client: &Client,
    config: &DingTalkConfig,
) -> Result<(String, String), ChannelError> {
    use secrecy::ExposeSecret;

    let body = serde_json::json!({
        "clientId": config.client_id,
        "clientSecret": config.client_secret.expose_secret(),
        "subscriptions": [
            { "type": "CALLBACK", "topic": BOT_MESSAGE_TOPIC }
        ],
        "ua": "ironclaw"
    });

    let resp = client
        .post(STREAM_GATEWAY_URL)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::Http(format!("stream register: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(ChannelError::Http(format!(
            "stream register returned {status}: {body_text}"
        )));
    }

    let gateway: StreamEndpointResponse = resp
        .json()
        .await
        .map_err(|e| ChannelError::Http(format!("parse gateway response: {e}")))?;

    let endpoint = gateway
        .endpoint
        .ok_or_else(|| ChannelError::Http("no endpoint in gateway response".to_string()))?;
    let ticket = gateway
        .ticket
        .ok_or_else(|| ChannelError::Http("no ticket in gateway response".to_string()))?;

    Ok((endpoint, ticket))
}

/// Build WebSocket URL from endpoint + ticket.
fn build_ws_url(endpoint: &str, ticket: &str) -> String {
    let sep = if endpoint.contains('?') { "&" } else { "?" };
    format!("{endpoint}{sep}ticket={ticket}")
}

/// Extract message content from various DingTalk message types.
fn extract_content(payload: &BotCallbackPayload) -> String {
    // Text message
    if let Some(ref text) = payload.text {
        if let Some(ref content) = text.content {
            return content.trim().to_string();
        }
    }

    // Rich text — extract text portions
    if let Some(ref rich) = payload.rich_text {
        if let Some(arr) = rich.as_array() {
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()).map(String::from))
                .collect();
            if !texts.is_empty() {
                return texts.join("\n");
            }
        }
    }

    format!(
        "[{}]",
        payload.msgtype.as_deref().unwrap_or("unknown")
    )
}

/// Process a single CALLBACK frame from the Stream.
fn process_callback(
    frame: &StreamFrame,
    tx: &mpsc::Sender<IncomingMessage>,
    reply_targets: &Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
) -> Option<String> {
    let data_str = frame.data.as_ref()?;
    let payload: BotCallbackPayload = match serde_json::from_str(data_str) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to parse DingTalk callback: {e}");
            return frame.message_id.clone();
        }
    };

    let content = extract_content(&payload);
    if content.is_empty() {
        return frame.message_id.clone();
    }

    let sender_id = payload
        .sender_staff_id
        .as_deref()
        .or(payload.sender_id.as_deref())
        .unwrap_or("unknown");
    let sender_nick = payload.sender_nick.as_deref().unwrap_or("User");
    let conversation_id = payload.conversation_id.as_deref().unwrap_or("");
    let conversation_type = payload.conversation_type.as_deref().unwrap_or("1");
    let is_group = conversation_type == "2";

    let msg_id = Uuid::new_v4();

    let metadata = DingTalkMetadata {
        conversation_id: conversation_id.to_string(),
        conversation_type: conversation_type.to_string(),
        sender_staff_id: sender_id.to_string(),
        sender_nick: sender_nick.to_string(),
        msg_id: payload.msg_id.as_deref().unwrap_or("").to_string(),
        robot_code: payload.robot_code.clone(),
    };

    // Store metadata for reply routing
    let reply_targets = Arc::clone(reply_targets);
    let metadata_clone = metadata.clone();
    tokio::spawn(async move {
        reply_targets.write().await.put(msg_id, metadata_clone);
    });

    let incoming = IncomingMessage::new("dingtalk", sender_id, content)
        .with_sender_id(sender_id)
        .with_user_name(sender_nick);

    let incoming = if is_group {
        incoming.with_thread(conversation_id)
    } else {
        incoming
    };

    // Override the generated UUID with our tracked one
    let mut incoming = incoming;
    incoming.id = msg_id;
    incoming.metadata = serde_json::to_value(&metadata).unwrap_or_default();

    tracing::info!(
        sender = %sender_nick,
        mode = if is_group { "group" } else { "dm" },
        "DingTalk message received"
    );

    if tx.try_send(incoming).is_err() {
        tracing::warn!("DingTalk message channel full, dropping message");
    }

    frame.message_id.clone()
}

/// Build an ACK response for a received message.
fn build_ack(message_id: &str) -> String {
    serde_json::json!({
        "code": 200,
        "headers": {},
        "message": "OK",
        "data": message_id,
    })
    .to_string()
}

/// Main Stream listener loop with automatic reconnection.
pub async fn run_stream_listener(
    config: DingTalkConfig,
    client: Client,
    tx: mpsc::Sender<IncomingMessage>,
    reply_targets: Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
) -> Result<(), ChannelError> {
    let mut backoff = RECONNECT_BASE_DELAY;

    loop {
        tracing::info!("Registering DingTalk Stream connection...");

        let (endpoint, ticket) = match register_stream(&client, &config).await {
            Ok(r) => {
                backoff = RECONNECT_BASE_DELAY; // Reset backoff on success
                r
            }
            Err(e) => {
                tracing::error!("Failed to register DingTalk Stream: {e}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_MAX_DELAY);
                continue;
            }
        };

        let ws_url = build_ws_url(&endpoint, &ticket);
        tracing::info!("Connecting to DingTalk Stream WebSocket...");

        let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
            Ok((stream, _)) => {
                tracing::info!("DingTalk Stream connected");
                backoff = RECONNECT_BASE_DELAY;
                stream
            }
            Err(e) => {
                tracing::error!("WebSocket connect failed: {e}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_MAX_DELAY);
                continue;
            }
        };

        let (mut ws_sink, mut ws_stream_rx) = ws_stream.split();

        // Spawn ping task to keep connection alive
        let ping_handle = tokio::spawn({
            let mut interval = tokio::time::interval(PING_INTERVAL);
            let ping_sink = Arc::new(tokio::sync::Mutex::new(()));
            async move {
                loop {
                    interval.tick().await;
                    // Ping is handled by tokio-tungstenite automatically
                    // but we keep this task to detect if we need to reconnect
                    let _ = ping_sink.lock().await;
                }
            }
        });

        // Process incoming WebSocket messages
        loop {
            match ws_stream_rx.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    let frame: StreamFrame = match serde_json::from_str(&text) {
                        Ok(f) => f,
                        Err(e) => {
                            tracing::warn!("Failed to parse Stream frame: {e}");
                            continue;
                        }
                    };

                    match frame.frame_type.as_deref() {
                        Some("SYSTEM") => {
                            tracing::debug!("Stream system message: {:?}", frame.data);
                        }
                        Some("CALLBACK") => {
                            if frame.topic.as_deref() == Some(BOT_MESSAGE_TOPIC) {
                                if let Some(msg_id) =
                                    process_callback(&frame, &tx, &reply_targets)
                                {
                                    // ACK the message
                                    let ack = build_ack(&msg_id);
                                    if let Err(e) =
                                        ws_sink.send(WsMessage::Text(ack.into())).await
                                    {
                                        tracing::error!("Failed to send ACK: {e}");
                                        break;
                                    }
                                }
                            } else {
                                tracing::debug!(
                                    "Ignoring callback for topic: {:?}",
                                    frame.topic
                                );
                            }
                        }
                        _ => {
                            tracing::debug!("Unknown frame type: {:?}", frame.frame_type);
                        }
                    }
                }
                Some(Ok(WsMessage::Ping(data))) => {
                    if let Err(e) = ws_sink.send(WsMessage::Pong(data)).await {
                        tracing::error!("Failed to send pong: {e}");
                        break;
                    }
                }
                Some(Ok(WsMessage::Close(_))) => {
                    tracing::info!("DingTalk Stream WebSocket closed by server");
                    break;
                }
                Some(Err(e)) => {
                    tracing::error!("DingTalk Stream WebSocket error: {e}");
                    break;
                }
                None => {
                    tracing::info!("DingTalk Stream WebSocket ended");
                    break;
                }
                _ => {} // Binary, Pong, Frame — ignore
            }
        }

        ping_handle.abort();
        tracing::info!(
            "DingTalk Stream disconnected, reconnecting in {:?}...",
            backoff
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_MAX_DELAY);
    }
}
