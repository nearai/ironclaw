//! Channel trait implementation for channel-relay webhook callbacks.
//!
//! `RelayChannel` receives events from channel-relay via HTTP POST callbacks
//! (pushed through an mpsc channel by the webhook handler), converts them
//! to `IncomingMessage`s, and sends responses via the relay's provider-specific
//! proxy API (Slack).

use std::collections::HashMap;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::channels::relay::client::{ChannelEvent, RelayClient};
use crate::channels::{
    Channel, ChatApprovalPrompt, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use crate::error::ChannelError;

/// Default channel name for the Slack relay integration.
pub const DEFAULT_RELAY_NAME: &str = "slack-relay";

/// The messaging provider backing a relay channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayProvider {
    Slack,
}

impl RelayProvider {
    /// Provider string used in proxy API routes and metadata.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Slack => "slack",
        }
    }

    /// The default channel name for this provider.
    pub fn channel_name(&self) -> &'static str {
        match self {
            Self::Slack => DEFAULT_RELAY_NAME,
        }
    }
}

/// Channel implementation that receives events from channel-relay via webhook callbacks.
pub struct RelayChannel {
    client: RelayClient,
    provider: RelayProvider,
    team_id: String,
    instance_id: String,
    /// Sender side of the event channel — shared with the webhook handler.
    event_tx: mpsc::Sender<ChannelEvent>,
    /// Receiver side — taken once by `start()`.
    event_rx: tokio::sync::Mutex<Option<mpsc::Receiver<ChannelEvent>>>,
}

impl RelayChannel {
    /// Create a new relay channel for Slack (default provider).
    pub fn new(
        client: RelayClient,
        team_id: String,
        instance_id: String,
        event_tx: mpsc::Sender<ChannelEvent>,
        event_rx: mpsc::Receiver<ChannelEvent>,
    ) -> Self {
        Self::new_with_provider(
            client,
            RelayProvider::Slack,
            team_id,
            instance_id,
            event_tx,
            event_rx,
        )
    }

    /// Create a new relay channel with a specific provider.
    pub fn new_with_provider(
        client: RelayClient,
        provider: RelayProvider,
        team_id: String,
        instance_id: String,
        event_tx: mpsc::Sender<ChannelEvent>,
        event_rx: mpsc::Receiver<ChannelEvent>,
    ) -> Self {
        Self {
            client,
            provider,
            team_id,
            instance_id,
            event_tx,
            event_rx: tokio::sync::Mutex::new(Some(event_rx)),
        }
    }

    /// Get a clone of the event sender for wiring into the webhook endpoint.
    pub fn event_sender(&self) -> mpsc::Sender<ChannelEvent> {
        self.event_tx.clone()
    }

    /// Build a provider-appropriate proxy body for sending a message.
    fn build_send_body(
        &self,
        channel_id: &str,
        text: &str,
        thread_id: Option<&str>,
    ) -> (String, serde_json::Value) {
        match self.provider {
            RelayProvider::Slack => {
                let mut body = serde_json::json!({
                    "channel": channel_id,
                    "text": text,
                });
                if let Some(tid) = thread_id {
                    body["thread_ts"] = serde_json::Value::String(tid.to_string());
                }
                ("chat.postMessage".to_string(), body)
            }
        }
    }

    /// Send a message via the provider proxy.
    async fn proxy_send(
        &self,
        team_id: &str,
        method: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, crate::channels::relay::client::RelayError> {
        self.client
            .proxy_provider(self.provider.as_str(), team_id, method, body)
            .await
    }

    fn build_approval_body(
        &self,
        channel_id: &str,
        thread_id: Option<&str>,
        prompt: &ChatApprovalPrompt,
        approval_token: &str,
    ) -> serde_json::Value {
        let value_payload = serde_json::json!({
            "approval_token": approval_token,
        });
        let value_str = value_payload.to_string();

        let blocks = serde_json::json!([
            {
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": prompt.markdown_message(),
                }
            },
            {
                "type": "actions",
                "elements": [
                    {
                        "type": "button",
                        "text": { "type": "plain_text", "text": "Approve" },
                        "style": "primary",
                        "action_id": "approve_tool",
                        "value": value_str,
                    },
                    {
                        "type": "button",
                        "text": { "type": "plain_text", "text": "Deny" },
                        "style": "danger",
                        "action_id": "deny_tool",
                        "value": value_str,
                    }
                ]
            }
        ]);

        let mut body = serde_json::json!({
            "channel": channel_id,
            "text": prompt.plain_text_message(),
            "blocks": blocks,
        });
        if let Some(tid) = thread_id {
            body["thread_ts"] = serde_json::Value::String(tid.to_string());
        }
        body
    }
}

#[async_trait]
impl Channel for RelayChannel {
    fn name(&self) -> &str {
        self.provider.channel_name()
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let channel_name = self.name().to_string();

        // Take the receiver (can only start once)
        let mut event_rx =
            self.event_rx
                .lock()
                .await
                .take()
                .ok_or_else(|| ChannelError::StartupFailed {
                    name: channel_name.clone(),
                    reason: "RelayChannel already started".to_string(),
                })?;

        let (tx, rx) = mpsc::channel(64);
        let provider_str = self.provider.as_str().to_string();
        let relay_name = channel_name.clone();

        // Spawn a task that reads events from the webhook handler and converts to IncomingMessage
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                // Validate required fields
                if event.sender_id.is_empty()
                    || event.channel_id.is_empty()
                    || event.provider_scope.is_empty()
                {
                    tracing::debug!(
                        event_type = %event.event_type,
                        sender_id = %event.sender_id,
                        channel_id = %event.channel_id,
                        "Relay: skipping event with missing required fields"
                    );
                    continue;
                }

                // Skip non-message events
                if !event.is_message() {
                    tracing::debug!(
                        event_type = %event.event_type,
                        "Relay: skipping non-message event"
                    );
                    continue;
                }

                tracing::info!(
                    event_type = %event.event_type,
                    sender = %event.sender_id,
                    channel = %event.channel_id,
                    provider = %provider_str,
                    "Relay: received message from {}", provider_str
                );

                let msg = IncomingMessage::new(&relay_name, &event.sender_id, event.text())
                    .with_user_name(event.display_name())
                    .with_metadata(serde_json::json!({
                        "team_id": event.team_id(),
                        "channel_id": event.channel_id,
                        "sender_id": event.sender_id,
                        "sender_name": event.display_name(),
                        "event_type": event.event_type,
                        "thread_id": event.thread_id,
                        "provider": event.provider,
                    }));

                let msg = if let Some(ref thread_id) = event.thread_id {
                    msg.with_thread(thread_id)
                } else {
                    msg.with_thread(&event.channel_id)
                };

                if tx.send(msg).await.is_err() {
                    tracing::info!("Relay channel receiver dropped, stopping");
                    return;
                }
            }

            tracing::info!("Relay event channel closed");
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let channel_name = self.name().to_string();
        let metadata = &msg.metadata;
        let team_id = metadata
            .get("team_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.team_id);
        let channel_id = metadata
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::SendFailed {
                name: channel_name.clone(),
                reason: "Missing channel_id in message metadata".to_string(),
            })?;

        // Determine thread_id from response or metadata
        let thread_id = response
            .thread_id
            .as_deref()
            .or_else(|| metadata.get("thread_id").and_then(|v| v.as_str()));

        let (method, body) = self.build_send_body(channel_id, &response.content, thread_id);

        self.proxy_send(team_id, &method, body)
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: channel_name,
                reason: e.to_string(),
            })?;

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Only handle ApprovalNeeded — all other variants are no-ops
        let Some(prompt) = ChatApprovalPrompt::from_status(&status) else {
            return Ok(());
        };

        // Only send buttons in DMs (dispatcher gates upstream, but guard here too)
        let event_type = metadata
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if event_type != "direct_message" {
            tracing::warn!(
                tool = %prompt.tool_name,
                event_type,
                "Approval requested in non-DM, skipping buttons"
            );
            return Ok(());
        }

        // Extract required metadata — error if missing
        let channel_id = metadata
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::SendFailed {
                name: self.name().to_string(),
                reason: "Missing channel_id for approval buttons".into(),
            })?;
        let thread_id = metadata.get("thread_id").and_then(|v| v.as_str());
        let team_id = metadata
            .get("team_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.team_id);

        // Register server-side approval record and get opaque token.
        // The button value contains ONLY the token — no routing fields.
        let approval_token = self
            .client
            .create_approval(team_id, channel_id, thread_id, &prompt.request_id)
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: self.name().to_string(),
                reason: format!("Failed to register approval: {e}"),
            })?;
        let body = self.build_approval_body(channel_id, thread_id, &prompt, &approval_token);

        self.proxy_send(team_id, "chat.postMessage", body)
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: self.name().to_string(),
                reason: e.to_string(),
            })?;

        Ok(())
    }

    async fn broadcast(
        &self,
        target: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let channel_name = self.name().to_string();

        // Determine thread_id from response or metadata
        let thread_id = response
            .thread_id
            .as_deref()
            .or_else(|| response.metadata.get("thread_ts").and_then(|v| v.as_str()));

        let (method, body) = self.build_send_body(target, &response.content, thread_id);

        self.proxy_send(&self.team_id, &method, body)
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: channel_name,
                reason: e.to_string(),
            })?;

        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        self.client
            .list_connections(&self.instance_id)
            .await
            .map_err(|_| ChannelError::HealthCheckFailed {
                name: self.name().to_string(),
            })?;
        Ok(())
    }

    fn conversation_context(&self, metadata: &serde_json::Value) -> HashMap<String, String> {
        let mut ctx = HashMap::new();

        if let Some(sender) = metadata.get("sender_name").and_then(|v| v.as_str()) {
            ctx.insert("sender".to_string(), sender.to_string());
        }
        if let Some(sender_id) = metadata.get("sender_id").and_then(|v| v.as_str()) {
            ctx.insert("sender_uuid".to_string(), sender_id.to_string());
        }
        if let Some(channel_id) = metadata.get("channel_id").and_then(|v| v.as_str()) {
            ctx.insert("group".to_string(), channel_id.to_string());
        }
        ctx.insert("platform".to_string(), self.provider.as_str().to_string());

        ctx
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        // Relay cleanup is driven by the extension manager dropping the shared
        // sender and removing the channel from the channel manager.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> RelayClient {
        RelayClient::new(
            "http://localhost:3001".into(),
            secrecy::SecretString::from("key".to_string()),
            30,
        )
        .expect("client")
    }

    fn make_channel() -> RelayChannel {
        let (tx, rx) = mpsc::channel(64);
        RelayChannel::new(test_client(), "T123".into(), "inst1".into(), tx, rx)
    }

    #[test]
    fn relay_channel_name() {
        let channel = make_channel();
        assert_eq!(channel.name(), DEFAULT_RELAY_NAME);
    }

    #[test]
    fn conversation_context_extracts_metadata() {
        let channel = make_channel();

        let metadata = serde_json::json!({
            "sender_name": "bob",
            "sender_id": "U123",
            "channel_id": "C456",
        });
        let ctx = channel.conversation_context(&metadata);
        assert_eq!(ctx.get("sender"), Some(&"bob".to_string()));
        assert_eq!(ctx.get("sender_uuid"), Some(&"U123".to_string()));
        assert_eq!(ctx.get("platform"), Some(&"slack".to_string()));
    }

    #[test]
    fn metadata_shape_includes_event_type_and_sender_name() {
        let metadata = serde_json::json!({
            "team_id": "T123",
            "channel_id": "C456",
            "sender_id": "U789",
            "sender_name": "alice",
            "event_type": "direct_message",
            "thread_id": null,
            "provider": "slack",
        });
        assert_eq!(
            metadata.get("event_type").and_then(|v| v.as_str()),
            Some("direct_message")
        );
        assert_eq!(
            metadata.get("sender_name").and_then(|v| v.as_str()),
            Some("alice")
        );
    }

    #[test]
    fn build_send_body_slack() {
        let channel = make_channel();
        let (method, body) = channel.build_send_body("C456", "hello", Some("1234567.890"));
        assert_eq!(method, "chat.postMessage");
        assert_eq!(body["channel"], "C456");
        assert_eq!(body["text"], "hello");
        assert_eq!(body["thread_ts"], "1234567.890");
    }

    #[test]
    fn build_approval_body_includes_chat_reply_instructions() {
        let channel = make_channel();
        let prompt = ChatApprovalPrompt {
            request_id: "req-1".into(),
            tool_name: "http".into(),
            description: "HTTP requests to external APIs".into(),
            parameters: serde_json::json!({"method": "POST", "url": "https://example.com"}),
            allow_always: true,
        };

        let body = channel.build_approval_body("C456", Some("1234567.890"), &prompt, "token-123");
        let text = body["text"].as_str().expect("plain text");
        let block_text = body["blocks"][0]["text"]["text"].as_str().expect("mrkdwn");

        assert!(text.contains("approve, or /approve"));
        assert!(text.contains("always, a, or /always"));
        assert!(block_text.contains("`/approve`"));
        assert!(block_text.contains("`/always`"));
        assert_eq!(body["thread_ts"], "1234567.890");
    }

    #[tokio::test]
    async fn start_processes_events() {
        let (tx, rx) = mpsc::channel(64);
        let channel =
            RelayChannel::new(test_client(), "T123".into(), "inst1".into(), tx.clone(), rx);

        let mut stream = channel.start().await.unwrap();

        // Send an event
        tx.send(ChannelEvent {
            id: "1".into(),
            event_type: "message".into(),
            provider: "slack".into(),
            provider_scope: "T123".into(),
            channel_id: "C456".into(),
            sender_id: "U789".into(),
            sender_name: Some("alice".into()),
            content: Some("hello".into()),
            thread_id: None,
            raw: serde_json::Value::Null,
            timestamp: None,
        })
        .await
        .unwrap();

        use futures::StreamExt;
        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(msg.content, "hello");
        assert_eq!(msg.user_id, "U789");
    }

    #[tokio::test]
    async fn start_threaded_message_preserves_thread_scope() {
        let (tx, rx) = mpsc::channel(64);
        let channel =
            RelayChannel::new(test_client(), "T123".into(), "inst1".into(), tx.clone(), rx);

        let mut stream = channel.start().await.unwrap();

        tx.send(ChannelEvent {
            id: "threaded-1".into(),
            event_type: "direct_message".into(),
            provider: "slack".into(),
            provider_scope: "T123".into(),
            channel_id: "D456".into(),
            sender_id: "U789".into(),
            sender_name: Some("alice".into()),
            content: Some("approve".into()),
            thread_id: Some("1712345678.123".into()),
            raw: serde_json::Value::Null,
            timestamp: None,
        })
        .await
        .unwrap();

        use futures::StreamExt;
        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(msg.content, "approve");
        assert_eq!(msg.thread_id.as_deref(), Some("1712345678.123"));
        assert_eq!(msg.conversation_scope(), Some("1712345678.123"));
    }

    #[tokio::test]
    async fn start_skips_non_message_events() {
        let (tx, rx) = mpsc::channel(64);
        let channel =
            RelayChannel::new(test_client(), "T123".into(), "inst1".into(), tx.clone(), rx);

        let mut stream = channel.start().await.unwrap();

        // Send a non-message event (should be skipped)
        tx.send(ChannelEvent {
            id: "1".into(),
            event_type: "reaction".into(),
            provider: "slack".into(),
            provider_scope: "T123".into(),
            channel_id: "C456".into(),
            sender_id: "U789".into(),
            sender_name: None,
            content: None,
            thread_id: None,
            raw: serde_json::Value::Null,
            timestamp: None,
        })
        .await
        .unwrap();

        // Send a real message
        tx.send(ChannelEvent {
            id: "2".into(),
            event_type: "message".into(),
            provider: "slack".into(),
            provider_scope: "T123".into(),
            channel_id: "C456".into(),
            sender_id: "U789".into(),
            sender_name: None,
            content: Some("real message".into()),
            thread_id: None,
            raw: serde_json::Value::Null,
            timestamp: None,
        })
        .await
        .unwrap();

        use futures::StreamExt;
        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(msg.content, "real message");
    }

    #[tokio::test]
    async fn test_send_status_non_approval_is_noop() {
        let channel = make_channel();
        let metadata = serde_json::json!({});
        let result = channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "echo".into(),
                },
                &metadata,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_status_approval_non_dm_skips() {
        let channel = make_channel();
        let metadata = serde_json::json!({
            "event_type": "message",
            "channel_id": "C456",
            "sender_id": "U789",
        });
        let result = channel
            .send_status(
                StatusUpdate::ApprovalNeeded {
                    request_id: "req1".into(),
                    tool_name: "shell".into(),
                    description: "run command".into(),
                    parameters: serde_json::json!({}),
                    allow_always: true,
                },
                &metadata,
            )
            .await;
        // Non-DM approval requests are silently skipped (no HTTP call)
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_status_approval_dm_missing_channel_id_errors() {
        let channel = make_channel();
        let metadata = serde_json::json!({
            "event_type": "direct_message",
            "sender_id": "U789",
        });
        let result = channel
            .send_status(
                StatusUpdate::ApprovalNeeded {
                    request_id: "req1".into(),
                    tool_name: "shell".into(),
                    description: "run command".into(),
                    parameters: serde_json::json!({}),
                    allow_always: true,
                },
                &metadata,
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("channel_id"),
            "expected channel_id error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_send_status_approval_dm_without_sender_id_is_ok() {
        let channel = make_channel();
        let metadata = serde_json::json!({
            "event_type": "direct_message",
            "channel_id": "C456",
        });
        let result = channel
            .send_status(
                StatusUpdate::ApprovalNeeded {
                    request_id: "req1".into(),
                    tool_name: "shell".into(),
                    description: "run command".into(),
                    parameters: serde_json::json!({}),
                    allow_always: true,
                },
                &metadata,
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("sender_id"),
            "sender_id should not be required anymore, got: {err}"
        );
    }
}
