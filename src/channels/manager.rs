//! Channel manager for coordinating multiple input channels.

use std::collections::HashMap;
use std::sync::Arc;

use futures::stream;
use tokio::sync::{RwLock, mpsc};

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Manages multiple input channels and merges their message streams.
///
/// Includes an injection channel so background tasks (e.g., job monitors) can
/// push messages into the agent loop without being a full `Channel` impl.
pub struct ChannelManager {
    channels: Arc<RwLock<HashMap<String, Arc<dyn Channel>>>>,
    inject_tx: mpsc::Sender<IncomingMessage>,
    /// Taken once in `start_all()` and merged into the stream.
    inject_rx: tokio::sync::Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
}

impl ChannelManager {
    /// Create a new channel manager.
    pub fn new() -> Self {
        let (inject_tx, inject_rx) = mpsc::channel(64);
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            inject_tx,
            inject_rx: tokio::sync::Mutex::new(Some(inject_rx)),
        }
    }

    /// Get a clone of the injection sender.
    ///
    /// Background tasks (like job monitors) use this to push messages into the
    /// agent loop without being a full `Channel` implementation.
    pub fn inject_sender(&self) -> mpsc::Sender<IncomingMessage> {
        self.inject_tx.clone()
    }

    /// Add a channel to the manager.
    pub async fn add(&self, channel: Box<dyn Channel>) {
        let name = channel.name().to_string();
        self.channels
            .write()
            .await
            .insert(name.clone(), Arc::from(channel));
        tracing::debug!("Added channel: {}", name);
    }

    /// Hot-add a channel to a running agent.
    ///
    /// Starts the channel, registers it in the channels map for `respond()`/`broadcast()`,
    /// and spawns a task that forwards its stream messages through `inject_tx` into
    /// the agent loop.
    pub async fn hot_add(&self, channel: Box<dyn Channel>) -> Result<(), ChannelError> {
        let name = channel.name().to_string();

        // Shut down any existing channel with the same name to avoid parallel consumers.
        // The old forwarding task will stop when the channel's stream ends after shutdown.
        {
            let channels = self.channels.read().await;
            if let Some(existing) = channels.get(&name) {
                tracing::debug!(channel = %name, "Shutting down existing channel before hot-add replacement");
                let _ = existing.shutdown().await;
            }
        }

        let stream = channel.start().await?;

        // Register for respond/broadcast/send_status
        self.channels
            .write()
            .await
            .insert(name.clone(), Arc::from(channel));

        // Forward stream messages through inject_tx
        let tx = self.inject_tx.clone();
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(msg) = stream.next().await {
                if tx.send(msg).await.is_err() {
                    tracing::warn!(channel = %name, "Inject channel closed, stopping hot-added channel");
                    break;
                }
            }
            tracing::debug!(channel = %name, "Hot-added channel stream ended");
        });

        Ok(())
    }

    /// Start all channels and return a merged stream of messages.
    ///
    /// Also merges the injection channel so background tasks can push messages
    /// into the same stream.
    pub async fn start_all(&self) -> Result<MessageStream, ChannelError> {
        let channels = self.channels.read().await;
        let mut streams: Vec<MessageStream> = Vec::new();

        for (name, channel) in channels.iter() {
            match channel.start().await {
                Ok(stream) => {
                    tracing::debug!("Started channel: {}", name);
                    streams.push(stream);
                }
                Err(e) => {
                    tracing::error!("Failed to start channel {}: {}", name, e);
                    // Continue with other channels, don't fail completely
                }
            }
        }

        if streams.is_empty() {
            return Err(ChannelError::StartupFailed {
                name: "all".to_string(),
                reason: "No channels started successfully".to_string(),
            });
        }

        // Take the injection receiver (can only be taken once)
        if let Some(inject_rx) = self.inject_rx.lock().await.take() {
            let inject_stream = tokio_stream::wrappers::ReceiverStream::new(inject_rx);
            streams.push(Box::pin(inject_stream));
            tracing::debug!("Injection channel merged into message stream");
        }

        // Merge all streams into one
        let merged = stream::select_all(streams);
        Ok(Box::pin(merged))
    }

    /// Send a response to a specific channel.
    pub async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(&msg.channel) {
            channel.respond(msg, response).await
        } else {
            Err(ChannelError::SendFailed {
                name: msg.channel.clone(),
                reason: "Channel not found".to_string(),
            })
        }
    }

    /// Send a status update to a specific channel.
    ///
    /// The metadata contains channel-specific routing info (e.g., Telegram chat_id)
    /// needed to deliver the status to the correct destination.
    pub async fn send_status(
        &self,
        channel_name: &str,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(channel_name) {
            channel.send_status(status, metadata).await
        } else {
            // Silently ignore if channel not found (status is best-effort)
            Ok(())
        }
    }

    /// Broadcast a message to a specific user on a specific channel.
    ///
    /// Used for proactive notifications like heartbeat alerts.
    pub async fn broadcast(
        &self,
        channel_name: &str,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(channel_name) {
            channel.broadcast(user_id, response).await
        } else {
            Err(ChannelError::SendFailed {
                name: channel_name.to_string(),
                reason: "Channel not found".to_string(),
            })
        }
    }

    /// Broadcast a message to all channels.
    ///
    /// Sends to the specified user on every registered channel.
    pub async fn broadcast_all(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Vec<(String, Result<(), ChannelError>)> {
        let channels = self.channels.read().await;
        let mut results = Vec::new();

        for (name, channel) in channels.iter() {
            let result = channel.broadcast(user_id, response.clone()).await;
            results.push((name.clone(), result));
        }

        results
    }

    /// Check health of all channels.
    pub async fn health_check_all(&self) -> HashMap<String, Result<(), ChannelError>> {
        let channels = self.channels.read().await;
        let mut results = HashMap::new();

        for (name, channel) in channels.iter() {
            results.insert(name.clone(), channel.health_check().await);
        }

        results
    }

    /// Shutdown all channels.
    pub async fn shutdown_all(&self) -> Result<(), ChannelError> {
        let channels = self.channels.read().await;
        for (name, channel) in channels.iter() {
            if let Err(e) = channel.shutdown().await {
                tracing::error!("Error shutting down channel {}: {}", name, e);
            }
        }
        Ok(())
    }

    /// Get list of channel names.
    pub async fn channel_names(&self) -> Vec<String> {
        self.channels.read().await.keys().cloned().collect()
    }

    /// Get a channel by name.
    pub async fn get_channel(&self, name: &str) -> Option<Arc<dyn Channel>> {
        self.channels.read().await.get(name).cloned()
    }
}

impl Default for ChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::IncomingMessage;
    use crate::testing::StubChannel;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_add_and_start_all() {
        let manager = ChannelManager::new();
        let (stub, sender) = StubChannel::new("test");

        manager.add(Box::new(stub)).await;

        let mut stream = manager.start_all().await.expect("start_all failed");

        // Inject a message through the stub
        sender
            .send(IncomingMessage::new("test", "user1", "hello"))
            .await
            .expect("send failed");

        // Should appear in the merged stream
        let msg = stream.next().await.expect("stream ended");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.channel, "test");
    }

    #[tokio::test]
    async fn test_respond_routes_to_correct_channel() {
        let manager = ChannelManager::new();
        let (stub, _sender) = StubChannel::new("alpha");

        // Keep a reference for response inspection
        let responses = stub.captured_responses_handle();
        manager.add(Box::new(stub)).await;

        let msg = IncomingMessage::new("alpha", "user1", "request");
        manager
            .respond(&msg, OutgoingResponse::text("reply"))
            .await
            .expect("respond failed");

        // Verify the stub captured the response
        let captured = responses.lock().expect("poisoned");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].1.content, "reply");
    }

    #[tokio::test]
    async fn test_respond_unknown_channel_errors() {
        let manager = ChannelManager::new();
        let msg = IncomingMessage::new("nonexistent", "user1", "test");
        let result = manager.respond(&msg, OutgoingResponse::text("hi")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_health_check_all() {
        let manager = ChannelManager::new();
        let (stub1, _) = StubChannel::new("healthy");
        let (stub2, _) = StubChannel::new("sick");
        stub2.set_healthy(false);

        manager.add(Box::new(stub1)).await;
        manager.add(Box::new(stub2)).await;

        let results = manager.health_check_all().await;
        assert!(results["healthy"].is_ok());
        assert!(results["sick"].is_err());
    }

    #[tokio::test]
    async fn test_start_all_no_channels_errors() {
        let manager = ChannelManager::new();
        let result = manager.start_all().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_injection_channel_merges() {
        let manager = ChannelManager::new();
        let (stub, _sender) = StubChannel::new("real");
        manager.add(Box::new(stub)).await;

        let mut stream = manager.start_all().await.expect("start_all failed");

        // Use the injection channel (simulating background task)
        let inject_tx = manager.inject_sender();
        inject_tx
            .send(IncomingMessage::new(
                "injected",
                "system",
                "background alert",
            ))
            .await
            .expect("inject failed");

        let msg = stream.next().await.expect("stream ended");
        assert_eq!(msg.content, "background alert");
    }

    #[tokio::test]
    async fn test_hot_add_replaces_existing_channel() {
        // Regression: hot_add must shut down the existing channel before replacing it,
        // to prevent duplicate SSE consumers from running in parallel.
        let manager = ChannelManager::new();
        let (stub1, _tx1) = StubChannel::new("relay");
        manager.add(Box::new(stub1)).await;
        let mut stream = manager.start_all().await.expect("start_all");

        // Hot-add a replacement channel with the same name
        let (stub2, tx2) = StubChannel::new("relay");
        manager.hot_add(Box::new(stub2)).await.expect("hot_add");

        // Send through the new channel — should arrive in the merged stream
        tx2.send(IncomingMessage::new("relay", "u1", "from new"))
            .await
            .expect("send");
        let msg = stream.next().await.expect("stream");
        assert_eq!(msg.content, "from new");

        // Verify only one channel entry exists
        let channels = manager.channels.read().await;
        assert_eq!(channels.len(), 1);
        assert!(channels.contains_key("relay"));
    }
}
