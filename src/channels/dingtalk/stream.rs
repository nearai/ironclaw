//! DingTalk Stream mode: WebSocket-based message delivery.
//!
//! Flow:
//! 1. POST /v1.0/gateway/connections/open → get WebSocket endpoint + ticket
//! 2. Connect to WebSocket endpoint with ticket
//! 3. Receive CALLBACK frames with bot messages
//! 4. ACK each message to confirm receipt
//! 5. On disconnect, re-register and reconnect (via ConnectionManager)

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use lru::LruCache;
use reqwest::Client;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::config::{DingTalkConfig, GroupSessionScope};
use crate::error::ChannelError;

use super::connection::ConnectionManager;
use super::filters::{DedupFilter, check_access, should_process};
use super::types::{BotCallbackPayload, DingTalkMetadata, StreamFrame};

const STREAM_GATEWAY_URL: &str = "https://api.dingtalk.com/v1.0/gateway/connections/open";
const BOT_MESSAGE_TOPIC: &str = "/v1.0/im/bot/messages/get";
const CARD_CALLBACK_TOPIC: &str = "/v1.0/card/instances/callback";
const PING_INTERVAL: Duration = Duration::from_secs(30);
const STOP_ACTION_TTL: Duration = Duration::from_secs(300);
const HEARTBEAT_MISS_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, Copy)]
enum StopAction {
    Stop,
    Interrupt,
}

impl std::fmt::Display for StopAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stop => write!(f, "stop"),
            Self::Interrupt => write!(f, "interrupt"),
        }
    }
}

impl FromStr for StopAction {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "stop" | "/stop" | "停止" => Ok(Self::Stop),
            "interrupt" | "/interrupt" | "esc" | "中断" => Ok(Self::Interrupt),
            _ => Err(()),
        }
    }
}

fn make_conversation_key<'a>(conversation_id: &'a str, fallback_sender_id: &'a str) -> &'a str {
    if !conversation_id.is_empty() {
        conversation_id
    } else {
        fallback_sender_id
    }
}

fn conversation_scope_id(
    config: &DingTalkConfig,
    is_group: bool,
    conversation_id: &str,
    sender_id: &str,
) -> Option<String> {
    if is_group {
        match config.group_session_scope {
            GroupSessionScope::Group => {
                (!conversation_id.is_empty()).then(|| conversation_id.to_string())
            }
            GroupSessionScope::User => {
                if !conversation_id.is_empty() && !sender_id.is_empty() {
                    Some(format!("{conversation_id}:{sender_id}"))
                } else if !sender_id.is_empty() {
                    Some(sender_id.to_string())
                } else {
                    None
                }
            }
        }
    } else if !sender_id.is_empty() {
        Some(format!("dm:{sender_id}"))
    } else {
        None
    }
}

fn parse_stop_action(raw: &str) -> Option<StopAction> {
    StopAction::from_str(raw).ok()
}

pub(super) async fn is_conversation_stopped(
    stopped_conversations: &Arc<RwLock<HashMap<String, Instant>>>,
    conversation_id: &str,
) -> bool {
    if conversation_id.is_empty() {
        return false;
    }

    let now = Instant::now();
    let mut stopped = stopped_conversations.write().await;

    match stopped.get(conversation_id).copied() {
        Some(stopped_at) if now.duration_since(stopped_at) <= STOP_ACTION_TTL => true,
        Some(_) => {
            stopped.remove(conversation_id);
            false
        }
        None => false,
    }
}

async fn set_conversation_stopped(
    stopped_conversations: &Arc<RwLock<HashMap<String, Instant>>>,
    conversation_id: &str,
) {
    if conversation_id.is_empty() {
        return;
    }

    stopped_conversations
        .write()
        .await
        .insert(conversation_id.to_string(), Instant::now());
}

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

    let gateway_value = super::send::parse_business_response(resp, "stream register")
        .await?
        .ok_or_else(|| ChannelError::Http("stream register returned empty body".to_string()))?;

    let endpoint = gateway_value
        .get("endpoint")
        .or_else(|| gateway_value.get("result").and_then(|v| v.get("endpoint")))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ChannelError::Http("no endpoint in gateway response".to_string()))?
        .to_string();
    let ticket = gateway_value
        .get("ticket")
        .or_else(|| gateway_value.get("result").and_then(|v| v.get("ticket")))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ChannelError::Http("no ticket in gateway response".to_string()))?
        .to_string();

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
async fn process_callback(
    frame: &StreamFrame,
    tx: &mpsc::Sender<IncomingMessage>,
    reply_targets: &Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
    dedup: &Arc<Mutex<DedupFilter>>,
    config: &DingTalkConfig,
    stopped_conversations: &Arc<RwLock<HashMap<String, Instant>>>,
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

    let sender_id = payload
        .sender_staff_id
        .as_deref()
        .or(payload.sender_id.as_deref())
        .unwrap_or("unknown");
    let sender_nick = payload.sender_nick.as_deref().unwrap_or("User");
    let conversation_id = payload.conversation_id.as_deref().unwrap_or("");
    let conversation_type = payload.conversation_type.as_deref().unwrap_or("1");
    let is_group = conversation_type == "2";
    let conversation_scope = conversation_scope_id(config, is_group, conversation_id, sender_id);
    let conversation_key = conversation_scope
        .as_deref()
        .unwrap_or_else(|| make_conversation_key(conversation_id, sender_id));
    let dingtalk_msg_id = payload.msg_id.as_deref().unwrap_or("");

    if dingtalk_msg_id.is_empty() {
        tracing::debug!(
            sender_id,
            conversation_id,
            "DingTalk: dropping message without dingTalk msg_id"
        );
        return msg_id_for_ack;
    }

    // ── Filter 1: deduplication ────────────────────────────────────────────
    // We use try_lock so a poisoned/contended mutex never blocks the WebSocket
    // receive loop. If we can't acquire the lock, we optimistically let it through.
    {
        if let Ok(mut guard) = dedup.try_lock() {
            let dedup_key = format!("{conversation_key}:{dingtalk_msg_id}");
            if guard.is_duplicate(&dedup_key) {
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

    // ── Stop-keyword interception ──────────────────────────────────────────
    let stop_action = parse_stop_action(&content);
    if let Some(action) = stop_action {
        set_conversation_stopped(stopped_conversations, conversation_key).await;
        tracing::info!(
            action = %action,
            conversation = %conversation_key,
            "DingTalk: stop action received, conversation marked stopped"
        );
        content = "/stop".to_string();
    } else if is_conversation_stopped(stopped_conversations, conversation_key).await {
        tracing::debug!(
            conversation_id,
            dingtalk_msg_id,
            "DingTalk: conversation is in stopped state, dropping message"
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

    // Store metadata for reply routing before forwarding the message so the
    // responder never races the metadata insert.
    {
        let mut guard = reply_targets.write().await;
        guard.put(msg_id, metadata.clone());
        tracing::info!(
            msg_id = %msg_id,
            conversation_id = %conversation_id,
            conversation_type = %conversation_type,
            sender = %sender_nick,
            reply_targets_len = guard.len(),
            "DingTalk: stored reply metadata"
        );
    }

    let incoming = IncomingMessage::new("dingtalk", sender_id, content)
        .with_sender_id(sender_id)
        .with_user_name(sender_nick);

    let incoming = if let Some(scope_id) = conversation_scope {
        incoming.with_thread(scope_id)
    } else {
        incoming
    };

    // Override the generated UUID with our tracked one
    let mut incoming = incoming;
    incoming.id = msg_id;
    // Serialize DingTalk metadata, then inject internal UUID as "message_id"
    // so that send_status() can map status updates back to the correct message.
    let mut meta_value = serde_json::to_value(&metadata).unwrap_or_default();
    if let Some(obj) = meta_value.as_object_mut() {
        obj.insert(
            "message_id".to_string(),
            serde_json::Value::String(msg_id.to_string()),
        );
    }
    incoming.metadata = meta_value;

    tracing::info!(
        sender = %sender_nick,
        mode = if is_group { "group" } else { "dm" },
        "DingTalk message received"
    );

    if tx.try_send(incoming).is_err() {
        reply_targets.write().await.pop(&msg_id);
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
///
/// The `reconnect_notify` handle allows external callers (e.g. the reconfigure
/// handler) to trigger an immediate reconnect so the stream picks up fresh
/// credentials from the runtime environment.
pub async fn run_stream_listener(
    config: DingTalkConfig,
    client: Client,
    tx: mpsc::Sender<IncomingMessage>,
    reply_targets: Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
    reconnect_notify: Arc<tokio::sync::Notify>,
    stopped_conversations: Arc<RwLock<HashMap<String, Instant>>>,
) -> Result<(), ChannelError> {
    let dedup = Arc::new(Mutex::new(DedupFilter::new()));
    let mut config = config;
    let mut conn =
        ConnectionManager::new(config.max_reconnect_cycles, config.reconnect_deadline_ms);
    let mut first_connection = true;

    loop {
        // Check whether we are still allowed to (re)connect.
        if !conn.should_reconnect() {
            return Err(ChannelError::Http(
                "DingTalk Stream: reconnect limit reached, giving up".to_string(),
            ));
        }

        // Reload config from env on each reconnect cycle so reconfigure
        // changes (applied via apply_channel_env) take effect.
        if !first_connection || conn.reconnect_cycles > 1 {
            config = config.reload_from_env();
            tracing::debug!("DingTalk: reloaded config from env for reconnect");
        }

        tracing::info!(
            client_id = %config.client_id,
            reconnect_cycle = conn.reconnect_cycles,
            "DingTalk Stream: registering connection"
        );

        let (endpoint, ticket) = match register_stream(&client, &config).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    client_id = %config.client_id,
                    error = %e,
                    reconnect_cycle = conn.reconnect_cycles,
                    "DingTalk Stream registration failed"
                );
                conn.on_reconnect_failed();
                if conn.state == super::connection::ConnectionState::Failed {
                    return Err(ChannelError::Http(format!(
                        "DingTalk Stream: registration failed and limits exhausted: {e}"
                    )));
                }
                let delay = conn.next_backoff();
                tracing::info!(
                    client_id = %config.client_id,
                    delay_ms = delay.as_millis() as u64,
                    "DingTalk Stream: backing off before retry"
                );
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        let ws_url = build_ws_url(&endpoint, &ticket);
        tracing::info!(
            client_id = %config.client_id,
            endpoint = %endpoint,
            "DingTalk Stream: connecting WebSocket"
        );

        let ws_stream = match tokio_tungstenite::connect_async(&ws_url).await {
            Ok((stream, _)) => {
                tracing::info!(
                    client_id = %config.client_id,
                    reconnect_cycle = conn.reconnect_cycles,
                    "DingTalk Stream: WebSocket connected"
                );
                conn.on_connected();
                first_connection = false;
                stream
            }
            Err(e) => {
                tracing::warn!(
                    client_id = %config.client_id,
                    error = %e,
                    "DingTalk Stream: WebSocket connect failed"
                );
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
        // Keep track of heartbeat liveness and periodic keepalive pings.
        let mut heartbeat = tokio::time::interval(PING_INTERVAL);
        heartbeat.reset();

        // Process incoming WebSocket messages.
        // We use tokio::select! to also listen for reconfigure-triggered reconnects.
        let notified = reconnect_notify.notified();
        tokio::pin!(notified);
        let mut reconnect_immediately = false;

        loop {
            tokio::select! {
                msg = ws_stream_rx.next() => match msg {
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
                                        let data = frame.data.as_deref().unwrap_or("{}");
                                        let mid = frame.message_id().unwrap_or("");
                                        let ack = build_ack(mid, data);
                                        if let Err(e) = ws_sink
                                            .lock()
                                            .await
                                            .send(WsMessage::Text(ack.into()))
                                            .await
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
                            Some("CALLBACK") => match frame.topic() {
                                Some(BOT_MESSAGE_TOPIC) => {
                                    if let Some(msg_id) = process_callback(
                                        &frame,
                                        &tx,
                                        &reply_targets,
                                        &dedup,
                                        &config,
                                        &stopped_conversations,
                                    )
                                    .await
                                    {
                                        let ack = build_ack(&msg_id, CALLBACK_ACK_DATA);
                                        if let Err(e) = ws_sink
                                            .lock()
                                            .await
                                            .send(WsMessage::Text(ack.into()))
                                            .await
                                        {
                                            tracing::warn!(error = %e, "Failed to send ACK");
                                            break;
                                        }
                                    }
                                }
                                Some(CARD_CALLBACK_TOPIC) => {
                                    if let Some(msg_id) = frame.message_id() {
                                        let is_duplicate = if let Ok(mut guard) = dedup.try_lock() {
                                            let dedup_key = format!("card-callback:{msg_id}");
                                            guard.is_duplicate(&dedup_key)
                                        } else {
                                            false
                                        };

                                        if is_duplicate {
                                            tracing::debug!(
                                                msg_id,
                                                "DingTalk: dropping duplicate card callback"
                                            );
                                            let ack = build_ack(msg_id, CALLBACK_ACK_DATA);
                                            if let Err(e) = ws_sink
                                                .lock()
                                                .await
                                                .send(WsMessage::Text(ack.into()))
                                                .await
                                            {
                                                tracing::warn!(
                                                    error = %e,
                                                    "Failed to send duplicate card callback ACK"
                                                );
                                                break;
                                            }
                                            continue;
                                        }

                                        if let Some(ref data_str) = frame.data {
                                            if let Ok(data) =
                                                serde_json::from_str::<serde_json::Value>(data_str)
                                            {
                                                let action = data
                                                    .get("action")
                                                    .or_else(|| data.get("callbackType"))
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("");

                                                tracing::debug!(
                                                    action = %action,
                                                    "DingTalk: card callback received"
                                                );

                                                if let Some(stop_action) = parse_stop_action(action) {
                                                    let thread_key = data
                                                        .get("conversationId")
                                                        .or_else(|| data.get("openConversationId"))
                                                        .or_else(|| data.get("senderStaffId"))
                                                        .or_else(|| data.get("staffId"))
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("");

                                                    set_conversation_stopped(
                                                        &stopped_conversations,
                                                        thread_key,
                                                    )
                                                    .await;

                                                    let incoming = if thread_key.is_empty() {
                                                        IncomingMessage::new("dingtalk", "", "/stop")
                                                    } else {
                                                        IncomingMessage::new("dingtalk", "", "/stop")
                                                            .with_thread(thread_key)
                                                    };

                                                    if tx.try_send(incoming).is_err() {
                                                        tracing::warn!(
                                                            "DingTalk card stop: channel full, dropping"
                                                        );
                                                    }

                                                    tracing::info!(
                                                        action = %stop_action,
                                                        conversation = %thread_key,
                                                        "DingTalk: card callback stop action"
                                                    );
                                                } else {
                                                    tracing::debug!(
                                                        action = %action,
                                                        "DingTalk: card callback non-stop action"
                                                    );
                                                }
                                            }
                                        }

                                        let ack = build_ack(msg_id, CALLBACK_ACK_DATA);
                                        if let Err(e) = ws_sink
                                            .lock()
                                            .await
                                            .send(WsMessage::Text(ack.into()))
                                            .await
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
                            },
                            Some("EVENT") => {
                                if let Some(msg_id) = frame.message_id() {
                                    let ack = build_ack(msg_id, EVENT_ACK_DATA);
                                    if let Err(e) = ws_sink
                                        .lock()
                                        .await
                                        .send(WsMessage::Text(ack.into()))
                                        .await
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
                    Some(Ok(WsMessage::Pong(_))) => {
                        conn.on_message_received();
                    }
                    Some(Ok(WsMessage::Close(frame))) => {
                        tracing::warn!(
                            client_id = %config.client_id,
                            reason = ?frame,
                            "DingTalk Stream: WebSocket closed by server — will reconnect"
                        );
                        break;
                    }
                    Some(Ok(WsMessage::Binary(_))) => {
                        conn.on_message_received();
                    }
                    Some(Err(e)) => {
                        tracing::warn!(
                            client_id = %config.client_id,
                            error = %e,
                            "DingTalk Stream: WebSocket error — will reconnect"
                        );
                        break;
                    }
                    None => {
                        tracing::warn!(
                            client_id = %config.client_id,
                            "DingTalk Stream: WebSocket stream ended — will reconnect"
                        );
                        break;
                    }
                    _ => {}
                },
                _ = heartbeat.tick() => {
                    if conn.on_heartbeat_miss() {
                        tracing::warn!(
                            client_id = %config.client_id,
                            threshold = HEARTBEAT_MISS_THRESHOLD,
                            misses = conn.consecutive_heartbeat_misses,
                            "DingTalk: heartbeat miss threshold reached, reconnecting"
                        );
                        conn.on_reconnect_failed();
                        break;
                    }

                    let mut guard = ws_sink.lock().await;
                    if let Err(e) = guard.send(WsMessage::Ping(vec![].into())).await {
                        tracing::warn!(
                            client_id = %config.client_id,
                            error = %e,
                            "DingTalk: failed to send ping — will reconnect"
                        );
                        conn.on_reconnect_failed();
                        break;
                    }
                }
                _ = &mut notified => {
                    tracing::info!(
                        client_id = %config.client_id,
                        "DingTalk: reconfigure triggered reconnect"
                    );
                    reconnect_immediately = true;
                    break;
                }
            }
        }

        if reconnect_immediately {
            conn.on_connected();
            tracing::info!(
                client_id = %config.client_id,
                "DingTalk: reconnecting immediately after reconfigure"
            );
            continue;
        }

        let delay = conn.next_backoff();
        tracing::info!(
            client_id = %config.client_id,
            delay_ms = delay.as_millis() as u64,
            reconnect_cycle = conn.reconnect_cycles,
            "DingTalk Stream disconnected, backing off before reconnect"
        );
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::super::types::{BotCallbackPayload, TextContent};
    use super::*;
    use secrecy::SecretString;

    use crate::config::{
        CardStreamMode, DingTalkConfig, DisplayNameResolution, DmPolicy, GroupPolicy,
    };

    fn test_config(group_session_scope: GroupSessionScope) -> DingTalkConfig {
        DingTalkConfig {
            enabled: true,
            client_id: "client".to_string(),
            client_secret: SecretString::from("secret".to_string()),
            robot_code: Some("robot".to_string()),
            message_type: Default::default(),
            card_template_id: None,
            card_template_key: "content".to_string(),
            card_stream_mode: CardStreamMode::Off,
            card_stream_interval_ms: 1000,
            ack_reaction: None,
            require_mention: false,
            dm_policy: DmPolicy::Open,
            group_policy: GroupPolicy::Open,
            allow_from: vec![],
            group_allow_from: vec![],
            group_session_scope,
            display_name_resolution: DisplayNameResolution::Disabled,
            max_reconnect_cycles: 3,
            reconnect_deadline_ms: 10_000,
            additional_accounts: vec![],
            status_tick_ms: 2000,
            slow_threshold_secs: (15, 60),
            reasoning_summary_enabled: false,
            max_active_cards: 1000,
        }
    }

    fn callback_frame(payload: serde_json::Value) -> StreamFrame {
        StreamFrame {
            frame_type: Some("CALLBACK".to_string()),
            data: Some(payload.to_string()),
            headers: Some(serde_json::json!({
                "topic": BOT_MESSAGE_TOPIC,
                "messageId": "ack-1"
            })),
        }
    }

    async fn process_and_recv(
        config: DingTalkConfig,
        payload: serde_json::Value,
    ) -> IncomingMessage {
        let (tx, mut rx) = mpsc::channel(1);
        let reply_targets = Arc::new(RwLock::new(LruCache::new(
            NonZeroUsize::new(8).expect("non-zero cache cap"),
        )));
        let dedup = Arc::new(Mutex::new(DedupFilter::new()));
        let stopped = Arc::new(RwLock::new(HashMap::new()));

        let ack = process_callback(
            &callback_frame(payload),
            &tx,
            &reply_targets,
            &dedup,
            &config,
            &stopped,
        )
        .await;

        assert_eq!(ack.as_deref(), Some("ack-1"));
        rx.recv()
            .await
            .expect("expected forwarded incoming message")
    }

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

    #[tokio::test]
    async fn dm_messages_get_stable_scope_from_sender() {
        let msg = process_and_recv(
            test_config(GroupSessionScope::Group),
            serde_json::json!({
                "conversationId": "cid-1",
                "conversationType": "1",
                "text": { "content": "hello" },
                "senderStaffId": "staff-1",
                "senderNick": "Alice",
                "msgId": "dt-1"
            }),
        )
        .await;

        assert_eq!(msg.thread_id.as_deref(), Some("dm:staff-1"));
        assert_eq!(msg.conversation_scope(), Some("dm:staff-1"));
    }

    #[tokio::test]
    async fn group_messages_use_group_scope_by_default() {
        let msg = process_and_recv(
            test_config(GroupSessionScope::Group),
            serde_json::json!({
                "conversationId": "cid-group",
                "conversationType": "2",
                "text": { "content": "hello" },
                "senderStaffId": "staff-1",
                "senderNick": "Alice",
                "msgId": "dt-2"
            }),
        )
        .await;

        assert_eq!(msg.thread_id.as_deref(), Some("cid-group"));
        assert_eq!(msg.conversation_scope(), Some("cid-group"));
    }

    #[tokio::test]
    async fn group_messages_can_scope_per_user() {
        let msg = process_and_recv(
            test_config(GroupSessionScope::User),
            serde_json::json!({
                "conversationId": "cid-group",
                "conversationType": "2",
                "text": { "content": "hello" },
                "senderStaffId": "staff-1",
                "senderNick": "Alice",
                "msgId": "dt-3"
            }),
        )
        .await;

        assert_eq!(msg.thread_id.as_deref(), Some("cid-group:staff-1"));
        assert_eq!(msg.conversation_scope(), Some("cid-group:staff-1"));
    }
}
