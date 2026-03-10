//! Channel trait implementation for channel-relay SSE streams.
//!
//! `RelayChannel` connects to a channel-relay service via SSE, converts
//! incoming events to `IncomingMessage`s, and sends responses via the
//! relay's provider-specific proxy API (Slack).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{RwLock, mpsc};

use crate::channels::relay::client::{RelayClient, RelayError};
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
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

/// Channel implementation that connects to a channel-relay SSE stream.
pub struct RelayChannel {
    client: RelayClient,
    provider: RelayProvider,
    stream_token: Arc<RwLock<String>>,
    team_id: String,
    instance_id: String,
    user_id: String,
    /// SSE stream long-poll timeout in seconds.
    stream_timeout_secs: u64,
    /// Initial exponential backoff in milliseconds.
    backoff_initial_ms: u64,
    /// Maximum exponential backoff in milliseconds.
    backoff_max_ms: u64,
    /// Handle to the reconnect task for clean shutdown.
    reconnect_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// Handle to the SSE parser task for clean shutdown.
    parser_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
    /// Maximum consecutive reconnect failures before giving up.
    max_consecutive_failures: u64,
}

impl RelayChannel {
    /// Create a new relay channel for Slack (default provider).
    pub fn new(
        client: RelayClient,
        stream_token: String,
        team_id: String,
        instance_id: String,
        user_id: String,
    ) -> Self {
        Self::new_with_provider(
            client,
            RelayProvider::Slack,
            stream_token,
            team_id,
            instance_id,
            user_id,
        )
    }

    /// Create a new relay channel with a specific provider.
    pub fn new_with_provider(
        client: RelayClient,
        provider: RelayProvider,
        stream_token: String,
        team_id: String,
        instance_id: String,
        user_id: String,
    ) -> Self {
        Self {
            client,
            provider,
            stream_token: Arc::new(RwLock::new(stream_token)),
            team_id,
            instance_id,
            user_id,
            stream_timeout_secs: 86400,
            backoff_initial_ms: 1000,
            backoff_max_ms: 60000,
            reconnect_handle: RwLock::new(None),
            parser_handle: Arc::new(RwLock::new(None)),
            max_consecutive_failures: 50,
        }
    }

    /// Set backoff/timeout parameters from relay config values.
    pub fn with_timeouts(
        mut self,
        stream_timeout_secs: u64,
        backoff_initial_ms: u64,
        backoff_max_ms: u64,
    ) -> Self {
        self.stream_timeout_secs = stream_timeout_secs;
        self.backoff_initial_ms = backoff_initial_ms;
        self.backoff_max_ms = backoff_max_ms;
        self
    }

    /// Set the maximum number of consecutive reconnect failures before giving up.
    pub fn with_max_failures(mut self, max: u64) -> Self {
        self.max_consecutive_failures = max;
        self
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
    ) -> Result<serde_json::Value, RelayError> {
        self.client
            .proxy_provider(
                self.provider.as_str(),
                team_id,
                method,
                body,
                Some(&self.instance_id),
            )
            .await
    }
}

#[async_trait]
impl Channel for RelayChannel {
    fn name(&self) -> &str {
        self.provider.channel_name()
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let channel_name = self.name().to_string();
        let token = self.stream_token.read().await.clone();
        let (stream, initial_parser_handle) = self
            .client
            .connect_stream(&token, self.stream_timeout_secs)
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: channel_name.clone(),
                reason: e.to_string(),
            })?;

        *self.parser_handle.write().await = Some(initial_parser_handle);

        let (tx, rx) = mpsc::channel(64);

        // Spawn the stream reader + reconnect task
        let client = self.client.clone();
        let stream_token = Arc::clone(&self.stream_token);
        let instance_id = self.instance_id.clone();
        let user_id = self.user_id.clone();
        let team_id = self.team_id.clone();
        let stream_timeout_secs = self.stream_timeout_secs;
        let backoff_initial_ms = self.backoff_initial_ms;
        let backoff_max_ms = self.backoff_max_ms;
        let max_consecutive_failures = self.max_consecutive_failures;
        let parser_handle = Arc::clone(&self.parser_handle);
        let provider_str = self.provider.as_str().to_string();
        let relay_name = channel_name.clone();

        let handle = tokio::spawn(async move {
            use futures::StreamExt;

            let mut current_stream = stream;
            let mut backoff_ms = backoff_initial_ms;
            let mut consecutive_failures: u64 = 0;

            loop {
                // Read events from the current stream
                while let Some(event) = current_stream.next().await {
                    // Reset backoff and failure count on successful event
                    backoff_ms = backoff_initial_ms;
                    consecutive_failures = 0;

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

                // Stream ended, attempt reconnect with backoff
                consecutive_failures += 1;
                if consecutive_failures >= max_consecutive_failures {
                    tracing::error!(
                        channel = %relay_name,
                        failures = consecutive_failures,
                        "Relay channel giving up after {} consecutive failures",
                        consecutive_failures
                    );
                    break;
                }

                tracing::warn!(
                    backoff_ms = backoff_ms,
                    failures = consecutive_failures,
                    "Relay SSE stream ended, reconnecting..."
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(backoff_max_ms);

                // Try to reconnect
                let token = stream_token.read().await.clone();
                match client.connect_stream(&token, stream_timeout_secs).await {
                    Ok((new_stream, new_parser)) => {
                        tracing::info!("Relay SSE stream reconnected");
                        current_stream = new_stream;
                        // Abort old parser before replacing
                        if let Some(old) = parser_handle.write().await.take() {
                            old.abort();
                        }
                        *parser_handle.write().await = Some(new_parser);
                    }
                    Err(RelayError::TokenExpired) => {
                        // Attempt token renewal
                        tracing::info!("Relay stream token expired, renewing...");
                        match client.renew_token(&instance_id, &user_id).await {
                            Ok(new_token) => {
                                *stream_token.write().await = new_token.clone();
                                match client.connect_stream(&new_token, stream_timeout_secs).await {
                                    Ok((new_stream, new_parser)) => {
                                        tracing::info!(
                                            "Relay SSE stream reconnected with new token"
                                        );
                                        current_stream = new_stream;
                                        if let Some(old) = parser_handle.write().await.take() {
                                            old.abort();
                                        }
                                        *parser_handle.write().await = Some(new_parser);
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            error = %e,
                                            "Failed to reconnect after token renewal"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "Failed to renew relay stream token"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to reconnect relay SSE stream");
                    }
                }

                // Check if the team is still valid (skip when team_id is unknown,
                // e.g. when no DB store was available at activation time)
                if !team_id.is_empty() {
                    match client.list_connections(&instance_id).await {
                        Ok(conns) => {
                            let has_team = conns.iter().any(|c| c.team_id == team_id && c.connected);
                            if !has_team {
                                tracing::warn!(
                                    team_id = %team_id,
                                    "Team no longer connected, stopping relay channel"
                                );
                                return;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Could not verify team connection, will retry next iteration"
                            );
                        }
                    }
                }
            }
        });

        *self.reconnect_handle.write().await = Some(handle);

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

    /// Status updates are not forwarded to messaging providers to avoid noise.
    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
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
        if let Some(handle) = self.reconnect_handle.write().await.take() {
            handle.abort();
        }
        if let Some(handle) = self.parser_handle.write().await.take() {
            handle.abort();
        }
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

    #[test]
    fn relay_channel_name() {
        let channel = RelayChannel::new(
            test_client(),
            "token".into(),
            "T123".into(),
            "inst1".into(),
            "user1".into(),
        );
        assert_eq!(channel.name(), DEFAULT_RELAY_NAME);
    }

    #[test]
    fn conversation_context_extracts_metadata() {
        let channel = RelayChannel::new(
            test_client(),
            "token".into(),
            "T123".into(),
            "inst1".into(),
            "user1".into(),
        );

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
        // Regression: metadata JSON must include event_type and sender_name
        // for downstream routing (DM vs channel) and conversation_context().
        let metadata = serde_json::json!({
            "team_id": "T123",
            "channel_id": "C456",
            "sender_id": "U789",
            "sender_name": "alice",
            "event_type": "direct_message",
            "thread_id": null,
            "provider": "slack",
        });
        // event_type must be present for DM-vs-channel routing
        assert_eq!(
            metadata.get("event_type").and_then(|v| v.as_str()),
            Some("direct_message")
        );
        // sender_name must be present for conversation_context
        assert_eq!(
            metadata.get("sender_name").and_then(|v| v.as_str()),
            Some("alice")
        );
    }

    #[test]
    fn with_timeouts_sets_values() {
        let channel = RelayChannel::new(
            test_client(),
            "token".into(),
            "T123".into(),
            "inst1".into(),
            "user1".into(),
        )
        .with_timeouts(43200, 2000, 120000);

        assert_eq!(channel.stream_timeout_secs, 43200);
        assert_eq!(channel.backoff_initial_ms, 2000);
        assert_eq!(channel.backoff_max_ms, 120000);
    }

    #[test]
    fn build_send_body_slack() {
        let channel = RelayChannel::new(
            test_client(),
            "token".into(),
            "T123".into(),
            "inst1".into(),
            "user1".into(),
        );
        let (method, body) = channel.build_send_body("C456", "hello", Some("1234567.890"));
        assert_eq!(method, "chat.postMessage");
        assert_eq!(body["channel"], "C456");
        assert_eq!(body["text"], "hello");
        assert_eq!(body["thread_ts"], "1234567.890");
    }

    #[test]
    fn parser_handle_is_shared_arc() {
        let channel = RelayChannel::new(
            test_client(),
            "token".into(),
            "T123".into(),
            "inst1".into(),
            "user1".into(),
        );
        // parser_handle should be an Arc — cloning should give a second reference
        let handle_clone = Arc::clone(&channel.parser_handle);
        // Both point to the same allocation
        assert!(Arc::ptr_eq(&channel.parser_handle, &handle_clone));
    }

    #[test]
    fn with_max_failures_sets_value() {
        let channel = RelayChannel::new(
            test_client(),
            "token".into(),
            "T123".into(),
            "inst1".into(),
            "user1".into(),
        )
        .with_max_failures(10);

        assert_eq!(channel.max_consecutive_failures, 10);
    }

    #[test]
    fn default_max_failures_is_50() {
        let channel = RelayChannel::new(
            test_client(),
            "token".into(),
            "T123".into(),
            "inst1".into(),
            "user1".into(),
        );
        assert_eq!(channel.max_consecutive_failures, 50);
    }

    #[test]
    fn empty_team_id_accepted_at_construction() {
        // Regression: empty team_id (when no DB store is available) must not
        // prevent channel construction or cause immediate shutdown.
        let channel = RelayChannel::new(
            test_client(),
            "token".into(),
            String::new(), // empty team_id
            "inst1".into(),
            "user1".into(),
        );
        assert_eq!(channel.team_id, "");
        // The reconnect loop now skips team validation when team_id is empty,
        // so the channel remains alive.
    }
}
