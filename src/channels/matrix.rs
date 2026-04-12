//! Matrix channel via external matrix-bridge HTTP/SSE API.
//!
//! Connects to a running matrix-bridge daemon.
//! Listens for messages via SSE at `/api/v1/events` and sends via
//! JSON-RPC at `/api/v1/rpc`.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use futures::StreamExt;
use lru::LruCache;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::channels::{
    Channel, ChatApprovalPrompt, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use crate::config::MatrixConfig;
use crate::error::ChannelError;

const MATRIX_HEALTH_ENDPOINT: &str = "/api/v1/check";
const MATRIX_EVENTS_ENDPOINT: &str = "/api/v1/events";
const MATRIX_RPC_ENDPOINT: &str = "/api/v1/rpc";
const MAX_REPLY_TARGETS: usize = 10_000;
const REPLY_TARGETS_CAP: NonZeroUsize = NonZeroUsize::new(MAX_REPLY_TARGETS).unwrap();

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatrixRoute {
    account: String,
    room_id: String,
    thread_root: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MatrixEvent {
    account: String,
    #[allow(dead_code)]
    event_id: String,
    room_id: String,
    sender: String,
    sender_name: Option<String>,
    body: String,
    thread_root: Option<String>,
    timestamp: u64,
    #[allow(dead_code)]
    is_encrypted: bool,
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
    #[serde(default)]
    accounts: Vec<HealthAccount>,
}

#[derive(Debug, Deserialize)]
struct HealthAccount {
    #[allow(dead_code)]
    name: Option<String>,
    #[allow(dead_code)]
    user_id: Option<String>,
    connected: bool,
    #[allow(dead_code)]
    last_sync_ms: Option<u64>,
}

pub struct MatrixChannel {
    config: MatrixConfig,
    client: Client,
    reply_targets: Arc<RwLock<LruCache<Uuid, MatrixRoute>>>,
    listener_task: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl MatrixChannel {
    pub fn new(config: MatrixConfig) -> Result<Self, ChannelError> {
        let mut config = config;
        config.daemon_url = config.daemon_url.trim_end_matches('/').to_string();

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        Ok(Self::from_parts(
            config,
            client,
            Arc::new(RwLock::new(LruCache::new(REPLY_TARGETS_CAP))),
        ))
    }

    fn from_parts(
        config: MatrixConfig,
        client: Client,
        reply_targets: Arc<RwLock<LruCache<Uuid, MatrixRoute>>>,
    ) -> Self {
        Self {
            config,
            client,
            reply_targets,
            listener_task: std::sync::Mutex::new(None),
        }
    }

    fn route_from_metadata(metadata: &Value) -> Option<MatrixRoute> {
        let room_id = metadata
            .get("matrix_room")
            .and_then(|v| v.as_str())
            .or_else(|| metadata.get("target").and_then(|v| v.as_str()))?;

        Some(MatrixRoute {
            account: metadata
                .get("matrix_account")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            room_id: room_id.to_string(),
            thread_root: metadata
                .get("matrix_thread_root")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    }

    fn is_sender_allowed(&self, sender: &str) -> bool {
        self.config
            .allow_from
            .iter()
            .any(|entry| entry == "*" || entry == sender)
    }

    fn is_room_allowed(&self, room_id: &str) -> bool {
        self.config
            .allow_from_rooms
            .iter()
            .any(|entry| entry == "*" || entry == room_id)
    }

    fn should_accept_event(&self, event: &MatrixEvent) -> bool {
        // `dm_policy` and `room_policy` are config placeholders for the
        // follow-on routing audit. Current POC behavior only applies flat
        // sender and room allowlists, with Signal-style semantics:
        // empty allowlists deny all, `*` opens access explicitly.
        let _ = (
            &self.config.dm_policy,
            &self.config.room_policy,
            &self.config.room_allow_from,
        );
        self.is_sender_allowed(&event.sender) && self.is_room_allowed(&event.room_id)
    }

    fn event_metadata(event: &MatrixEvent) -> Value {
        json!({
            "matrix_account": event.account,
            "matrix_room": event.room_id,
            "matrix_sender": event.sender,
            "matrix_thread_root": event.thread_root,
            "target": event.room_id,
        })
    }

    fn conversation_scope(event: &MatrixEvent) -> String {
        match event.thread_root.as_deref() {
            Some(thread_root) => format!("{}:{}", event.room_id, thread_root),
            None => event.room_id.clone(),
        }
    }

    fn incoming_message_from_event(
        &self,
        event: MatrixEvent,
    ) -> Option<(IncomingMessage, MatrixRoute)> {
        if event.body.trim().is_empty() || !self.should_accept_event(&event) {
            return None;
        }

        let conversation_scope = Self::conversation_scope(&event);
        let thread_root = event.thread_root.clone();
        let route = MatrixRoute {
            account: event.account.clone(),
            room_id: event.room_id.clone(),
            thread_root: thread_root.clone(),
        };

        let metadata = Self::event_metadata(&event);
        let mut msg = IncomingMessage::new("matrix", event.sender.clone(), event.body)
            .with_sender_id(event.sender)
            .with_metadata(metadata)
            .with_conversation_scope(conversation_scope.clone());

        if let Some(name) = event.sender_name {
            msg = msg.with_user_name(name);
        }
        if let Some(thread_root) = thread_root {
            msg = msg.with_thread(thread_root);
            msg = msg.with_conversation_scope(conversation_scope);
        }

        if let Some(dt) = Utc.timestamp_millis_opt(event.timestamp as i64).single() {
            msg.received_at = dt;
        }

        Some((msg, route))
    }

    async fn rpc_request(&self, method: &str, params: Value) -> Result<Value, ChannelError> {
        let url = format!("{}{}", self.config.daemon_url, MATRIX_RPC_ENDPOINT);
        let resp = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(30))
            .json(&json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
                "id": Uuid::new_v4().to_string(),
            }))
            .send()
            .await
            .map_err(|e| ChannelError::Http(format!("matrix rpc request failed: {e}")))?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| ChannelError::Http(format!("matrix rpc response decode failed: {e}")))?;

        if !status.is_success() {
            return Err(ChannelError::Http(format!(
                "matrix rpc returned HTTP {}",
                status
            )));
        }

        if let Some(err) = body.get("error") {
            return Err(ChannelError::SendFailed {
                name: "matrix".to_string(),
                reason: err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown matrix rpc error")
                    .to_string(),
            });
        }

        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn send_message(&self, route: &MatrixRoute, body: &str) -> Result<(), ChannelError> {
        if let Some(thread_root) = route.thread_root.as_deref() {
            tracing::debug!(
                room = %route.room_id,
                thread_root = %thread_root,
                "Matrix thread_root present; threaded outbound replies are deferred to the routing audit",
            );
        }
        tracing::debug!(
            room = %route.room_id,
            account = %route.account,
            "Sending Matrix message"
        );

        let _ = self
            .rpc_request(
                "send",
                json!({
                    "room_id": route.room_id,
                    "body": body,
                }),
            )
            .await?;
        Ok(())
    }

    async fn send_typing(&self, route: &MatrixRoute, active: bool) -> Result<(), ChannelError> {
        let _ = self
            .rpc_request(
                "sendTyping",
                json!({
                    "room_id": route.room_id,
                    "active": active,
                }),
            )
            .await?;
        Ok(())
    }

    fn parse_broadcast_target(target: &str) -> Option<MatrixRoute> {
        if !target.starts_with('!') {
            return None;
        }
        Some(MatrixRoute {
            account: String::new(),
            room_id: target.to_string(),
            thread_root: None,
        })
    }

    fn parse_health(body: &Value) -> Result<(), String> {
        let health: HealthResponse = serde_json::from_value(body.clone())
            .map_err(|e| format!("invalid health payload: {e}"))?;

        if health.status != "ok" {
            return Err(format!("bridge status is {}", health.status));
        }

        if !health.accounts.iter().any(|account| account.connected) {
            return Err("bridge reports no connected accounts".to_string());
        }

        Ok(())
    }
}

#[async_trait]
impl Channel for MatrixChannel {
    fn name(&self) -> &str {
        "matrix"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        let config = self.config.clone();
        let client = self.client.clone();
        let reply_targets = Arc::clone(&self.reply_targets);

        let task = tokio::spawn(async move {
            if let Err(e) = sse_listener(config, client, tx, reply_targets).await {
                tracing::error!("Matrix SSE listener exited with error: {e}");
            }
        });
        let mut listener_task = self
            .listener_task
            .lock()
            .expect("listener task mutex poisoned");
        if let Some(previous) = listener_task.replace(task) {
            previous.abort();
        }

        tracing::info!(url = %self.config.daemon_url, "Matrix channel started");
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if !response.attachments.is_empty() {
            return Err(ChannelError::SendFailed {
                name: "matrix".to_string(),
                reason: "attachments are not yet supported for Matrix".to_string(),
            });
        }

        let route = {
            let targets = self.reply_targets.read().await;
            targets.peek(&msg.id).cloned()
        }
        .or_else(|| Self::route_from_metadata(&msg.metadata))
        .ok_or(ChannelError::MissingRoutingTarget {
            name: "matrix".to_string(),
            reason: "no Matrix room target found in reply cache or message metadata".to_string(),
        })?;

        let result = self.send_message(&route, &response.content).await;
        self.reply_targets.write().await.pop(&msg.id);
        result
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &Value,
    ) -> Result<(), ChannelError> {
        let route = match Self::route_from_metadata(metadata) {
            Some(route) => route,
            None => return Ok(()),
        };

        if matches!(status, StatusUpdate::Thinking(_)) {
            let _ = self.send_typing(&route, true).await;
        }

        if let Some(prompt) = ChatApprovalPrompt::from_status(&status) {
            let _ = self.send_message(&route, &prompt.markdown_message()).await;
        }

        if let StatusUpdate::Status(message) = &status {
            let normalized = message.trim();
            if !normalized.eq_ignore_ascii_case("done")
                && !normalized.eq_ignore_ascii_case("awaiting approval")
                && !normalized.eq_ignore_ascii_case("rejected")
            {
                let _ = self.send_message(&route, normalized).await;
            }
        }

        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if !response.attachments.is_empty() {
            return Err(ChannelError::SendFailed {
                name: "matrix".to_string(),
                reason: "attachments are not yet supported for Matrix".to_string(),
            });
        }

        let route =
            Self::parse_broadcast_target(user_id).ok_or(ChannelError::MissingRoutingTarget {
                name: "matrix".to_string(),
                reason: "Matrix broadcast target must be a room ID like !room:homeserver"
                    .to_string(),
            })?;

        self.send_message(&route, &response.content).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        let url = format!("{}{}", self.config.daemon_url, MATRIX_HEALTH_ENDPOINT);
        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ChannelError::HealthCheckFailed {
                name: format!("matrix ({}): {e}", self.config.daemon_url),
            })?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| ChannelError::HealthCheckFailed {
                name: format!("matrix: invalid health payload ({e})"),
            })?;

        if !status.is_success() {
            return Err(ChannelError::HealthCheckFailed {
                name: format!("matrix: HTTP {status}"),
            });
        }

        Self::parse_health(&body).map_err(|reason| ChannelError::HealthCheckFailed {
            name: format!("matrix: {reason}"),
        })
    }

    fn conversation_context(&self, metadata: &Value) -> HashMap<String, String> {
        let mut ctx = HashMap::new();

        if let Some(sender) = metadata.get("matrix_sender").and_then(|v| v.as_str()) {
            ctx.insert("sender".to_string(), sender.to_string());
        }
        if let Some(account) = metadata.get("matrix_account").and_then(|v| v.as_str()) {
            ctx.insert("account".to_string(), account.to_string());
        }
        if let Some(room) = metadata.get("matrix_room").and_then(|v| v.as_str()) {
            ctx.insert("room".to_string(), room.to_string());
        }
        if let Some(thread) = metadata.get("matrix_thread_root").and_then(|v| v.as_str()) {
            ctx.insert("thread".to_string(), thread.to_string());
        }

        ctx
    }
}

impl Drop for MatrixChannel {
    fn drop(&mut self) {
        if let Some(handle) = self
            .listener_task
            .lock()
            .expect("listener task mutex poisoned")
            .take()
        {
            handle.abort();
        }
    }
}

async fn sse_listener(
    config: MatrixConfig,
    client: Client,
    tx: tokio::sync::mpsc::Sender<IncomingMessage>,
    reply_targets: Arc<RwLock<LruCache<Uuid, MatrixRoute>>>,
) -> Result<(), ChannelError> {
    let channel = MatrixChannel::from_parts(config, client, Arc::clone(&reply_targets));
    let url = format!("{}{}", channel.config.daemon_url, MATRIX_EVENTS_ENDPOINT);

    let mut retry_delay = Duration::from_secs(2);
    let max_delay = Duration::from_secs(60);

    loop {
        let resp = channel
            .client
            .get(&url)
            .header("Accept", "text/event-stream")
            .send()
            .await;

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::warn!("Matrix SSE returned HTTP {}", r.status());
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(max_delay);
                continue;
            }
            Err(e) => {
                tracing::warn!("Matrix SSE connect error to {}: {}", url, e);
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(max_delay);
                continue;
            }
        };

        retry_delay = Duration::from_secs(2);
        tracing::info!("Matrix SSE connected");

        let mut bytes_stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut current_data = String::new();

        while let Some(chunk) = bytes_stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => {
                    tracing::debug!("Matrix SSE chunk error, reconnecting: {e}");
                    break;
                }
            };

            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(idx) = buffer.find('\n') {
                let mut line = buffer[..idx].to_string();
                buffer = buffer[idx + 1..].to_string();
                line = line.trim_end_matches('\r').to_string();

                if line.is_empty() {
                    if current_data.is_empty() {
                        continue;
                    }

                    let event = match serde_json::from_str::<MatrixEvent>(current_data.trim_end()) {
                        Ok(event) => event,
                        Err(e) => {
                            tracing::warn!("Failed to decode Matrix SSE event: {}", e);
                            current_data.clear();
                            continue;
                        }
                    };
                    current_data.clear();

                    if let Some((msg, route)) = channel.incoming_message_from_event(event) {
                        reply_targets.write().await.put(msg.id, route);
                        if tx.send(msg).await.is_err() {
                            return Ok(());
                        }
                    }
                    continue;
                }

                if let Some(data) = line.strip_prefix("data:") {
                    current_data.push_str(data.trim_start());
                    current_data.push('\n');
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> MatrixConfig {
        MatrixConfig {
            daemon_url: "http://127.0.0.1:8090".to_string(),
            accounts: vec!["@bot:example.com".to_string()],
            allow_from: vec!["*".to_string()],
            allow_from_rooms: vec!["*".to_string()],
            dm_policy: "open".to_string(),
            room_policy: "allowlist".to_string(),
            room_allow_from: Vec::new(),
        }
    }

    #[test]
    fn incoming_message_from_event_sets_metadata_and_scope() {
        let channel = MatrixChannel::new(make_config()).unwrap();
        let event = MatrixEvent {
            account: "@bot:example.com".to_string(),
            event_id: "$event".to_string(),
            room_id: "!room:example.com".to_string(),
            sender: "@alice:example.com".to_string(),
            sender_name: Some("Alice".to_string()),
            body: "hello".to_string(),
            thread_root: Some("$thread".to_string()),
            timestamp: 1_700_000_000_000,
            is_encrypted: true,
        };

        let (msg, route) = channel.incoming_message_from_event(event).expect("message");
        assert_eq!(msg.channel, "matrix");
        assert_eq!(msg.user_id, "@alice:example.com");
        assert_eq!(msg.sender_id, "@alice:example.com");
        assert_eq!(msg.thread_id.as_deref(), Some("$thread"));
        assert_eq!(msg.conversation_scope(), Some("!room:example.com:$thread"));
        assert_eq!(
            msg.metadata.get("target").and_then(|v| v.as_str()),
            Some("!room:example.com")
        );
        assert_eq!(route.room_id, "!room:example.com");
    }

    #[test]
    fn incoming_message_respects_basic_filters() {
        let mut config = make_config();
        config.allow_from = vec!["@alice:example.com".to_string()];
        config.allow_from_rooms = vec!["!room:example.com".to_string()];
        let channel = MatrixChannel::new(config).unwrap();
        let event = MatrixEvent {
            account: "@bot:example.com".to_string(),
            event_id: "$event".to_string(),
            room_id: "!room:example.com".to_string(),
            sender: "@mallory:example.com".to_string(),
            sender_name: None,
            body: "hello".to_string(),
            thread_root: None,
            timestamp: 1,
            is_encrypted: false,
        };

        assert!(channel.incoming_message_from_event(event).is_none());
    }

    #[test]
    fn incoming_message_denies_when_allowlists_are_empty() {
        let mut config = make_config();
        config.allow_from.clear();
        config.allow_from_rooms.clear();
        let channel = MatrixChannel::new(config).unwrap();
        let event = MatrixEvent {
            account: "@bot:example.com".to_string(),
            event_id: "$event".to_string(),
            room_id: "!room:example.com".to_string(),
            sender: "@alice:example.com".to_string(),
            sender_name: None,
            body: "hello".to_string(),
            thread_root: None,
            timestamp: 1,
            is_encrypted: false,
        };

        assert!(channel.incoming_message_from_event(event).is_none());
    }

    #[test]
    fn incoming_message_accepts_wildcard_allowlists() {
        let channel = MatrixChannel::new(make_config()).unwrap();
        let event = MatrixEvent {
            account: "@bot:example.com".to_string(),
            event_id: "$event".to_string(),
            room_id: "!room:example.com".to_string(),
            sender: "@alice:example.com".to_string(),
            sender_name: None,
            body: "hello".to_string(),
            thread_root: None,
            timestamp: 1,
            is_encrypted: false,
        };

        assert!(channel.incoming_message_from_event(event).is_some());
    }

    #[test]
    fn conversation_context_extracts_matrix_fields() {
        let channel = MatrixChannel::new(make_config()).unwrap();
        let metadata = json!({
            "matrix_sender": "@alice:example.com",
            "matrix_account": "@bot:example.com",
            "matrix_room": "!room:example.com",
            "matrix_thread_root": "$thread",
        });

        let ctx = channel.conversation_context(&metadata);
        assert_eq!(ctx.get("sender"), Some(&"@alice:example.com".to_string()));
        assert_eq!(ctx.get("account"), Some(&"@bot:example.com".to_string()));
        assert_eq!(ctx.get("room"), Some(&"!room:example.com".to_string()));
        assert_eq!(ctx.get("thread"), Some(&"$thread".to_string()));
    }

    #[test]
    fn parse_health_requires_connected_account() {
        let healthy = json!({
            "status": "ok",
            "accounts": [{"connected": true}]
        });
        assert!(MatrixChannel::parse_health(&healthy).is_ok());

        let unhealthy = json!({
            "status": "starting",
            "accounts": [{"connected": false}]
        });
        assert!(MatrixChannel::parse_health(&unhealthy).is_err());
    }

    #[test]
    fn route_from_metadata_uses_generic_target_fallback() {
        let route = MatrixChannel::route_from_metadata(&json!({
            "matrix_account": "@bot:example.com",
            "target": "!room:example.com"
        }))
        .expect("route");

        assert_eq!(route.account, "@bot:example.com");
        assert_eq!(route.room_id, "!room:example.com");
    }
}
