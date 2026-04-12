//! DingTalk Stream mode: WebSocket-based message delivery.
//!
//! Flow:
//! 1. POST /v1.0/gateway/connections/open → get WebSocket endpoint + ticket
//! 2. Connect to WebSocket endpoint with ticket
//! 3. Receive CALLBACK frames with bot messages
//! 4. ACK each message to confirm receipt
//! 5. On disconnect, re-register and reconnect (via ConnectionManager)

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use lru::LruCache;
use reqwest::Client;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::config::DingTalkConfig;
use crate::error::ChannelError;

use super::connection::ConnectionManager;
use super::filters::{DedupFilter, check_access, should_process};
use super::types::{BotCallbackPayload, DingTalkMetadata, StreamEndpointResponse, StreamFrame};

const STREAM_GATEWAY_URL: &str = "https://api.dingtalk.com/v1.0/gateway/connections/open";
const BOT_MESSAGE_TOPIC: &str = "/v1.0/im/bot/messages/get";
const CARD_CALLBACK_TOPIC: &str = "/v1.0/card/instances/callback";
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
            { "type": "CALLBACK", "topic": BOT_MESSAGE_TOPIC },
            { "type": "CALLBACK", "topic": CARD_CALLBACK_TOPIC }
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
    let msgtype = payload.msgtype.as_deref().unwrap_or("unknown");

    match msgtype {
        "text" => {
            if let Some(ref text) = payload.text {
                if let Some(ref content) = text.content {
                    return content.trim().to_string();
                }
            }
        }

        "richText" => {
            if let Some(ref rich) = payload.rich_text {
                if let Some(arr) = rich.as_array() {
                    let texts: Vec<String> = arr
                        .iter()
                        .filter_map(|item| {
                            item.get("text").and_then(|t| t.as_str()).map(String::from)
                        })
                        .collect();
                    if !texts.is_empty() {
                        return texts.join("\n");
                    }
                }
            }
        }

        "audio" | "voice" => {
            // Try to use speech-to-text recognition if present.
            let recognition = payload
                .content
                .as_ref()
                .and_then(|c| c.get("recognition"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            return if let Some(text) = recognition {
                format!("[语音转文字] {text}")
            } else {
                "[语音消息]".to_string()
            };
        }

        "picture" | "image" => {
            return "[图片消息]".to_string();
        }

        "video" => {
            return "[视频消息]".to_string();
        }

        "file" => {
            let file_name = payload
                .content
                .as_ref()
                .and_then(|c| c.get("fileName"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            return if let Some(name) = file_name {
                format!("[文件] {name}")
            } else {
                "[文件消息]".to_string()
            };
        }

        _ => {}
    }

    // Try text content as a final fallback for any unhandled type.
    if let Some(ref text) = payload.text {
        if let Some(ref content) = text.content {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    format!("[{msgtype}]")
}

/// Extract a short text summary from a `repliedMsg` JSON blob.
fn extract_quoted_text(replied_msg: &serde_json::Value) -> Option<String> {
    // Try common fields: text.content, content, body, or message.
    if let Some(text) = replied_msg
        .get("text")
        .and_then(|t| t.get("content"))
        .and_then(|v| v.as_str())
    {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(text) = replied_msg.get("content").and_then(|v| v.as_str()) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(text) = replied_msg.get("body").and_then(|v| v.as_str()) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

/// Process a single CALLBACK frame from the Stream.
///
/// Returns the DingTalk `message_id` that must be ACK-ed regardless of whether
/// the message was forwarded (we always ACK to prevent re-delivery).
fn process_callback(
    frame: &StreamFrame,
    tx: &mpsc::Sender<IncomingMessage>,
    reply_targets: &Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
    dedup: &Arc<Mutex<DedupFilter>>,
    config: &DingTalkConfig,
) -> Option<String> {
    let msg_id_for_ack = frame.message_id().map(String::from);
    let data_str = frame.data.as_ref()?;
    let payload: BotCallbackPayload = match serde_json::from_str(data_str) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to parse DingTalk callback: {e}");
            return msg_id_for_ack;
        }
    };

    let mut content = extract_content(&payload);
    if content.is_empty() {
        return msg_id_for_ack;
    }

    // ── Quoted / reply message prefix ─────────────────────────────────────────
    if payload.is_reply_msg.unwrap_or(false) {
        if let Some(ref replied) = payload.replied_msg {
            if let Some(quoted_text) = extract_quoted_text(replied) {
                tracing::debug!(quoted = %quoted_text, "DingTalk: message is a reply, prepending quote");
                content = format!("[引用: {quoted_text}]\n{content}");
            }
        }
    }

    // ── Stop-keyword interception ──────────────────────────────────────────
    // Rewrite natural-language stop commands to "/stop" so SubmissionParser
    // maps them to Submission::Interrupt. Must be an exact full-string match
    // after trimming, so "stop please" does NOT trigger this.
    {
        let lower = content.trim().to_ascii_lowercase();
        if matches!(lower.as_str(), "停止" | "stop" | "/stop" | "esc") {
            tracing::debug!(original = %content, "DingTalk: rewriting stop keyword to /stop");
            content = "/stop".to_string();
        }
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
    let dingtalk_msg_id = payload.msg_id.as_deref().unwrap_or("");

    // ── Filter 1: deduplication ────────────────────────────────────────────
    // We use try_lock so a poisoned/contended mutex never blocks the WebSocket
    // receive loop. If we can't acquire the lock, we optimistically let it through.
    if !dingtalk_msg_id.is_empty() {
        if let Ok(mut guard) = dedup.try_lock() {
            if guard.is_duplicate(dingtalk_msg_id) {
                tracing::debug!(msg_id = dingtalk_msg_id, "DingTalk: dropping duplicate");
                return msg_id_for_ack.clone();
            }
        }
    }

    // ── Filter 2: access control ───────────────────────────────────────────
    if !check_access(is_group, conversation_id, sender_id, config) {
        tracing::debug!(
            sender_id,
            conversation_id,
            is_group,
            "DingTalk: message blocked by access control"
        );
        return msg_id_for_ack;
    }

    // ── Filter 3: @mention gate ────────────────────────────────────────────
    if !should_process(payload.is_in_at_list, is_group, config.require_mention) {
        tracing::debug!(
            conversation_id,
            "DingTalk: group message dropped (bot not mentioned)"
        );
        return msg_id_for_ack;
    }

    let msg_id = Uuid::new_v4();

    let metadata = DingTalkMetadata {
        conversation_id: conversation_id.to_string(),
        conversation_type: conversation_type.to_string(),
        sender_staff_id: sender_id.to_string(),
        sender_nick: sender_nick.to_string(),
        msg_id: dingtalk_msg_id.to_string(),
        robot_code: payload.robot_code.clone(),
        session_webhook: payload.session_webhook.clone(),
        session_webhook_expired_time: payload.session_webhook_expired_time,
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

    msg_id_for_ack
}

/// Build an ACK response per the DingTalk Stream protocol.
///
/// The `messageId` must be echoed back inside `headers` for frame correlation,
/// and `data` carries the response payload as a JSON string.
fn build_ack(message_id: &str, data: &str) -> String {
    serde_json::json!({
        "code": 200,
        "headers": {
            "contentType": "application/json",
            "messageId": message_id,
        },
        "message": "OK",
        "data": data,
    })
    .to_string()
}

/// ACK data for bot message callbacks (server ignores the content).
const CALLBACK_ACK_DATA: &str = r#"{"response": null}"#;
/// ACK data for event subscriptions.
const EVENT_ACK_DATA: &str = r#"{"status": "SUCCESS", "message": "ok"}"#;

/// Main Stream listener loop with automatic, bounded reconnection.
///
/// Uses [`ConnectionManager`] to enforce cycle limits and a wall-clock deadline.
/// Returns an error when reconnect limits are exhausted.
pub async fn run_stream_listener(
    config: DingTalkConfig,
    client: Client,
    tx: mpsc::Sender<IncomingMessage>,
    reply_targets: Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
) -> Result<(), ChannelError> {
    let dedup = Arc::new(Mutex::new(DedupFilter::new()));
    let mut conn =
        ConnectionManager::new(config.max_reconnect_cycles, config.reconnect_deadline_ms);

    loop {
        // Check whether we are still allowed to (re)connect.
        if conn.reconnect_cycles > 0 && !conn.should_reconnect() {
            return Err(ChannelError::Http(
                "DingTalk Stream: reconnect limit reached, giving up".to_string(),
            ));
        }

        tracing::debug!("Registering DingTalk Stream connection...");

        let (endpoint, ticket) = match register_stream(&client, &config).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(error = %e, "Failed to register DingTalk Stream");
                conn.on_reconnect_failed();
                if conn.state == super::connection::ConnectionState::Failed {
                    return Err(ChannelError::Http(format!(
                        "DingTalk Stream: registration failed and limits exhausted: {e}"
                    )));
                }
                let delay = conn.next_backoff();
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        let ws_url = build_ws_url(&endpoint, &ticket);
        tracing::debug!("Connecting to DingTalk Stream WebSocket...");

        let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
            Ok((stream, _)) => {
                tracing::debug!("DingTalk Stream connected");
                conn.on_connected();
                stream
            }
            Err(e) => {
                tracing::debug!(error = %e, "WebSocket connect failed");
                conn.on_reconnect_failed();
                if conn.state == super::connection::ConnectionState::Failed {
                    return Err(ChannelError::Http(format!(
                        "DingTalk Stream: WS connect failed and limits exhausted: {e}"
                    )));
                }
                let delay = conn.next_backoff();
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        let (ws_sink, mut ws_stream_rx) = ws_stream.split();
        let ws_sink = Arc::new(tokio::sync::Mutex::new(ws_sink));

        // Spawn a ping-interval task that sends WebSocket pings to keep the
        // DingTalk Stream connection alive. Without active pings the server
        // considers the connection idle and drops it.
        let ping_handle = tokio::spawn({
            let mut interval = tokio::time::interval(PING_INTERVAL);
            let sink = Arc::clone(&ws_sink);
            async move {
                loop {
                    interval.tick().await;
                    let mut guard = sink.lock().await;
                    if let Err(e) = guard.send(WsMessage::Ping(vec![].into())).await {
                        tracing::debug!(error = %e, "DingTalk: failed to send ping");
                        break;
                    }
                }
            }
        });

        // Whether the inner loop asked for a reconnect (vs. a hard exit).
        let should_reconnect_after = true;

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

                    // Any valid frame counts as a liveness signal.
                    conn.on_message_received();

                    match frame.frame_type.as_deref() {
                        Some("SYSTEM") => {
                            let topic = frame.topic().unwrap_or("");
                            match topic {
                                "ping" => {
                                    // Echo back the opaque value per protocol spec.
                                    let data = frame.data.as_deref().unwrap_or("{}");
                                    let mid = frame.message_id().unwrap_or("");
                                    let ack = build_ack(mid, data);
                                    if let Err(e) =
                                        ws_sink.lock().await.send(WsMessage::Text(ack.into())).await
                                    {
                                        tracing::warn!(error = %e, "Failed to send ping ACK");
                                        break;
                                    }
                                    tracing::debug!("DingTalk: responded to system ping");
                                }
                                "disconnect" => {
                                    tracing::info!(
                                        "DingTalk: server requested disconnect, will reconnect"
                                    );
                                    break;
                                }
                                _ => {
                                    tracing::debug!(topic, "DingTalk: unknown system topic");
                                }
                            }
                        }
                        Some("CALLBACK") => {
                            match frame.topic() {
                                Some(BOT_MESSAGE_TOPIC) => {
                                    if let Some(msg_id) = process_callback(
                                        &frame,
                                        &tx,
                                        &reply_targets,
                                        &dedup,
                                        &config,
                                    ) {
                                        let ack = build_ack(&msg_id, CALLBACK_ACK_DATA);
                                        if let Err(e) =
                                            ws_sink.lock().await.send(WsMessage::Text(ack.into())).await
                                        {
                                            tracing::warn!(error = %e, "Failed to send ACK");
                                            break;
                                        }
                                    }
                                }
                                Some(CARD_CALLBACK_TOPIC) => {
                                    if let Some(msg_id) = frame.message_id() {
                                        if let Some(ref data_str) = frame.data {
                                            if let Ok(data) =
                                                serde_json::from_str::<serde_json::Value>(data_str)
                                            {
                                                let action = data
                                                    .get("action")
                                                    .or_else(|| data.get("callbackType"))
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_ascii_lowercase();

                                                tracing::debug!(
                                                    action = %action,
                                                    "DingTalk: card callback received"
                                                );

                                                if action == "stop" || action == "interrupt" {
                                                    let thread_id = data
                                                        .get("conversationId")
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("")
                                                        .to_string();

                                                    let incoming = IncomingMessage::new(
                                                        "dingtalk", "", "/stop",
                                                    )
                                                    .with_thread(&thread_id);

                                                    if tx.try_send(incoming).is_err() {
                                                        tracing::warn!(
                                                            "DingTalk card stop: channel full, dropping"
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        let ack = build_ack(msg_id, CALLBACK_ACK_DATA);
                                        if let Err(e) =
                                            ws_sink.lock().await.send(WsMessage::Text(ack.into())).await
                                        {
                                            tracing::warn!(
                                                error = %e,
                                                "Failed to send card callback ACK"
                                            );
                                            break;
                                        }
                                    }
                                }
                                _ => {
                                    tracing::debug!(
                                        topic = ?frame.topic(),
                                        "Ignoring callback for unknown topic"
                                    );
                                }
                            }
                        }
                        Some("EVENT") => {
                            // Event subscriptions — ACK to prevent re-delivery.
                            if let Some(msg_id) = frame.message_id() {
                                let ack = build_ack(msg_id, EVENT_ACK_DATA);
                                if let Err(e) =
                                    ws_sink.lock().await.send(WsMessage::Text(ack.into())).await
                                {
                                    tracing::warn!(error = %e, "Failed to send event ACK");
                                    break;
                                }
                            }
                            tracing::debug!(topic = ?frame.topic(), "DingTalk: event received (ACK sent)");
                        }
                        _ => {
                            tracing::debug!(frame_type = ?frame.frame_type, "Unknown frame type");
                        }
                    }
                }
                Some(Ok(WsMessage::Ping(data))) => {
                    conn.on_message_received();
                    if let Err(e) = ws_sink.lock().await.send(WsMessage::Pong(data)).await {
                        tracing::debug!(error = %e, "Failed to send pong");
                        break;
                    }
                }
                Some(Ok(WsMessage::Close(_))) => {
                    tracing::debug!("DingTalk Stream WebSocket closed by server");
                    break;
                }
                Some(Err(e)) => {
                    tracing::debug!(error = %e, "DingTalk Stream WebSocket error");
                    break;
                }
                None => {
                    tracing::debug!("DingTalk Stream WebSocket ended");
                    break;
                }
                _ => {} // Binary, Pong, Frame — ignore
            }
        }

        ping_handle.abort();

        if !should_reconnect_after {
            return Ok(());
        }

        // Enforce reconnect limits before sleeping.
        if !conn.should_reconnect() {
            return Err(ChannelError::Http(
                "DingTalk Stream: reconnect limit reached after disconnect".to_string(),
            ));
        }

        let delay = conn.next_backoff();
        tracing::debug!(
            delay_ms = delay.as_millis(),
            "DingTalk Stream disconnected, reconnecting..."
        );
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{BotCallbackPayload, TextContent};
    use super::*;

    fn make_payload(msgtype: &str) -> BotCallbackPayload {
        BotCallbackPayload {
            conversation_id: None,
            conversation_type: None,
            text: None,
            rich_text: None,
            sender_id: None,
            sender_nick: None,
            sender_staff_id: None,
            msg_id: None,
            msgtype: Some(msgtype.to_string()),
            robot_code: None,
            is_in_at_list: None,
            session_webhook: None,
            session_webhook_expired_time: None,
            content: None,
            is_reply_msg: None,
            replied_msg: None,
        }
    }

    #[test]
    fn extract_content_text_message() {
        let mut p = make_payload("text");
        p.text = Some(TextContent {
            content: Some("  hello world  ".to_string()),
        });
        assert_eq!(extract_content(&p), "hello world");
    }

    #[test]
    fn extract_content_rich_text() {
        let mut p = make_payload("richText");
        p.rich_text = Some(serde_json::json!([
            {"type": "text", "text": "Hello"},
            {"type": "text", "text": "world"}
        ]));
        assert_eq!(extract_content(&p), "Hello\nworld");
    }

    #[test]
    fn extract_content_audio_with_recognition() {
        let mut p = make_payload("audio");
        p.content = Some(serde_json::json!({ "recognition": "请帮我发邮件" }));
        assert_eq!(extract_content(&p), "[语音转文字] 请帮我发邮件");
    }

    #[test]
    fn extract_content_audio_without_recognition() {
        let p = make_payload("audio");
        assert_eq!(extract_content(&p), "[语音消息]");
    }

    #[test]
    fn extract_content_picture() {
        let p = make_payload("picture");
        assert_eq!(extract_content(&p), "[图片消息]");
    }

    #[test]
    fn extract_content_image() {
        let p = make_payload("image");
        assert_eq!(extract_content(&p), "[图片消息]");
    }

    #[test]
    fn extract_content_video() {
        let p = make_payload("video");
        assert_eq!(extract_content(&p), "[视频消息]");
    }

    #[test]
    fn extract_content_file_with_name() {
        let mut p = make_payload("file");
        p.content = Some(serde_json::json!({ "fileName": "report.pdf" }));
        assert_eq!(extract_content(&p), "[文件] report.pdf");
    }

    #[test]
    fn extract_content_file_without_name() {
        let p = make_payload("file");
        assert_eq!(extract_content(&p), "[文件消息]");
    }

    #[test]
    fn extract_content_unknown_type() {
        let p = make_payload("dingdoc");
        assert_eq!(extract_content(&p), "[dingdoc]");
    }

    #[test]
    fn extract_quoted_text_from_text_content() {
        let replied = serde_json::json!({
            "text": { "content": "What is the weather?" }
        });
        assert_eq!(
            extract_quoted_text(&replied),
            Some("What is the weather?".to_string())
        );
    }

    #[test]
    fn extract_quoted_text_from_content_field() {
        let replied = serde_json::json!({ "content": "Some quoted text" });
        assert_eq!(
            extract_quoted_text(&replied),
            Some("Some quoted text".to_string())
        );
    }

    #[test]
    fn extract_quoted_text_from_body_field() {
        let replied = serde_json::json!({ "body": "Body text" });
        assert_eq!(extract_quoted_text(&replied), Some("Body text".to_string()));
    }

    #[test]
    fn extract_quoted_text_empty_returns_none() {
        let replied = serde_json::json!({ "other": "field" });
        assert_eq!(extract_quoted_text(&replied), None);
    }
}
