//! Channel manager for coordinating multiple input channels.

use std::collections::HashMap;
use std::sync::Arc;

use futures::stream;
use ironclaw_transport::{
    TransportAdapter, TransportAdapterId, TransportEgress, TransportError, TransportErrorKind,
    TransportIngressSink, TransportRegistry,
};
use tokio::sync::{RwLock, mpsc};

use crate::channels::transport_adapter::{
    ChannelTransportAdapter, transport_reply_from_channel_response,
    transport_status_from_channel_status,
};
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Bound a transport registry and the ingress sink it dispatches into. The
/// pair is updated atomically so a hot-add never observes one half of the
/// composition without the other.
#[derive(Clone)]
struct TransportComposition {
    registry: Arc<TransportRegistry>,
    sink: Option<Arc<dyn TransportIngressSink>>,
}

/// Manages multiple input channels and merges their message streams.
///
/// Includes an injection channel so background tasks (e.g., job monitors) can
/// push messages into the agent loop without being a full `Channel` impl.
pub struct ChannelManager {
    channels: Arc<RwLock<HashMap<String, Arc<dyn Channel>>>>,
    transport_composition: RwLock<Option<TransportComposition>>,
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
            transport_composition: RwLock::new(None),
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

        let channel: Arc<dyn Channel> = Arc::from(channel);

        // Snapshot the composition once: registry and sink are stored together,
        // so a concurrent update cannot leave us with a registry but no sink.
        if let Some(composition) = self.transport_composition_snapshot().await {
            let adapter = Arc::new(
                ChannelTransportAdapter::new(Arc::clone(&channel))
                    .map_err(|error| transport_error_to_channel_error(&name, error))?,
            );
            composition
                .registry
                .replace(adapter.clone() as Arc<dyn TransportAdapter>)
                .map_err(|error| transport_error_to_channel_error(&name, error))?;

            if let Some(sink) = composition.sink {
                if let Err(error) = adapter.start(sink).await {
                    let _ = composition.registry.unregister(adapter.adapter_id());
                    return Err(transport_error_to_channel_error(&name, error));
                }

                // Register for respond/broadcast/send_status
                self.channels.write().await.insert(name.clone(), channel);
                return Ok(());
            }
            // Composition exists but has no sink yet (transport runtime not
            // fully wired). Fall through to the legacy injection path so the
            // channel still receives messages instead of being silently
            // dropped on hot-add.
        }

        let stream = channel.start().await?;

        // Register for respond/broadcast/send_status
        self.channels.write().await.insert(name.clone(), channel);

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

    /// Take the injection stream without starting registered channels.
    ///
    /// Reborn transport composition uses this during cutover: transport
    /// adapters own channel startup, then inject normalized messages into this
    /// stream for the legacy agent loop to consume.
    pub async fn take_injected_message_stream(&self) -> Result<MessageStream, ChannelError> {
        let inject_rx =
            self.inject_rx
                .lock()
                .await
                .take()
                .ok_or_else(|| ChannelError::StartupFailed {
                    name: "injection".to_string(),
                    reason: "injection stream already taken".to_string(),
                })?;
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            inject_rx,
        )))
    }

    /// Route future response/status egress through a Reborn transport registry.
    /// The composition starts with no sink; pair it with [`Self::set_transport_runtime`]
    /// to start hot-added channels.
    pub async fn set_transport_egress_registry(&self, registry: Arc<TransportRegistry>) {
        *self.transport_composition.write().await = Some(TransportComposition {
            registry,
            sink: None,
        });
    }

    /// Route future egress through a Reborn registry and start hot-added
    /// channels against the same ingress sink. Atomic with respect to readers.
    pub async fn set_transport_runtime(
        &self,
        registry: Arc<TransportRegistry>,
        sink: Arc<dyn TransportIngressSink>,
    ) {
        *self.transport_composition.write().await = Some(TransportComposition {
            registry,
            sink: Some(sink),
        });
    }

    /// Clear Reborn transport egress routing and return to direct channel calls.
    pub async fn clear_transport_egress_registry(&self) {
        *self.transport_composition.write().await = None;
    }

    async fn transport_composition_snapshot(&self) -> Option<TransportComposition> {
        self.transport_composition.read().await.clone()
    }

    async fn transport_egress_registry(&self) -> Option<Arc<TransportRegistry>> {
        self.transport_composition
            .read()
            .await
            .as_ref()
            .map(|c| Arc::clone(&c.registry))
    }

    /// Send a response to a specific channel.
    pub async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if let Some(registry) = self.transport_egress_registry().await {
            let adapter_id = TransportAdapterId::new(&msg.channel)
                .map_err(|error| transport_error_to_channel_error(&msg.channel, error))?;
            let reply = transport_reply_from_channel_response(msg, response)
                .map_err(|error| transport_error_to_channel_error(&msg.channel, error))?;
            registry
                .deliver(&adapter_id, TransportEgress::Reply(reply))
                .await
                .map(|_| ())
                .map_err(|error| transport_error_to_channel_error(&msg.channel, error))?;
            return Ok(());
        }

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
        if let Some(registry) = self.transport_egress_registry().await {
            let egress = transport_status_from_channel_status(channel_name, status, metadata)
                .map_err(|error| transport_error_to_channel_error(channel_name, error))?;
            let adapter_id = egress
                .route()
                .map(|route| route.adapter_id.clone())
                .ok_or_else(|| ChannelError::SendFailed {
                    name: channel_name.to_string(),
                    reason: "transport status egress missing route".to_string(),
                })?;
            return match registry.deliver(&adapter_id, egress).await {
                Ok(_) => Ok(()),
                Err(error) if error.kind() == TransportErrorKind::AdapterNotFound => Ok(()),
                Err(error) => Err(transport_error_to_channel_error(channel_name, error)),
            };
        }

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
    /// Used for proactive notifications like heartbeat alerts. Routes through
    /// the Reborn transport registry when active so that broadcasts honor the
    /// same egress policy as `respond`/`send_status` and do not silently
    /// bypass the registry.
    pub async fn broadcast(
        &self,
        channel_name: &str,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if let Some(registry) = self.transport_egress_registry().await {
            return self
                .broadcast_via_registry(&registry, channel_name, user_id, response)
                .await;
        }
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
    /// Sends to the specified user on every registered channel. Routes through
    /// the Reborn transport registry when active.
    pub async fn broadcast_all(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Vec<(String, Result<(), ChannelError>)> {
        let registry = self.transport_egress_registry().await;
        let channel_names: Vec<String> = {
            let channels = self.channels.read().await;
            channels.keys().cloned().collect()
        };
        let mut results = Vec::with_capacity(channel_names.len());
        if let Some(registry) = registry {
            for name in channel_names {
                let result = self
                    .broadcast_via_registry(&registry, &name, user_id, response.clone())
                    .await;
                results.push((name, result));
            }
            return results;
        }
        let channels = self.channels.read().await;
        for (name, channel) in channels.iter() {
            let result = channel.broadcast(user_id, response.clone()).await;
            results.push((name.clone(), result));
        }

        results
    }

    async fn broadcast_via_registry(
        &self,
        registry: &TransportRegistry,
        channel_name: &str,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Synthesize a minimal IncomingMessage so we can reuse the
        // outgoing-response → transport-reply mapping. The synthetic message
        // carries the broadcast user as both user_id and sender_id; downstream
        // adapters route by `route.scope.user_id` and `route.recipient`.
        let synthetic = IncomingMessage::new(channel_name, user_id, String::new())
            .with_sender_id(user_id.to_string());
        let reply = transport_reply_from_channel_response(&synthetic, response)
            .map_err(|error| transport_error_to_channel_error(channel_name, error))?;
        let adapter_id = TransportAdapterId::new(channel_name)
            .map_err(|error| transport_error_to_channel_error(channel_name, error))?;
        registry
            .deliver(&adapter_id, TransportEgress::Reply(reply))
            .await
            .map(|_| ())
            .map_err(|error| transport_error_to_channel_error(channel_name, error))
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

    /// Build Reborn transport adapters for the currently registered channels.
    pub async fn transport_adapters(&self) -> Result<Vec<ChannelTransportAdapter>, TransportError> {
        self.channels
            .read()
            .await
            .values()
            .cloned()
            .map(ChannelTransportAdapter::new)
            .collect()
    }

    /// Remove a channel from the manager.
    pub async fn remove(&self, name: &str) -> Option<Arc<dyn Channel>> {
        if let Some(registry) = self.transport_egress_registry().await
            && let Ok(adapter_id) = TransportAdapterId::new(name)
        {
            let _ = registry.unregister(&adapter_id);
        }
        self.channels.write().await.remove(name)
    }
}

fn transport_error_to_channel_error(channel_name: &str, error: TransportError) -> ChannelError {
    ChannelError::SendFailed {
        name: channel_name.to_string(),
        reason: format!("{}: {}", error.kind(), error.safe_reason()),
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
    use crate::reborn::transport::RebornTransportRuntime;
    use crate::testing::StubChannel;
    use futures::StreamExt;
    use ironclaw_transport::{
        TransportAdapter, TransportIngress, TransportIngressSink, TransportRegistry,
        TransportSubmission,
    };
    use serde_json::json;

    struct CapturingTransportSink {
        tx: mpsc::Sender<TransportIngress>,
    }

    #[async_trait::async_trait]
    impl TransportIngressSink for CapturingTransportSink {
        async fn submit_ingress(
            &self,
            ingress: TransportIngress,
        ) -> Result<TransportSubmission, TransportError> {
            self.tx.send(ingress).await.map_err(|_| {
                TransportError::new(
                    TransportErrorKind::Unavailable,
                    "capturing transport sink closed",
                )
            })?;
            Ok(TransportSubmission {
                accepted_at: chrono::Utc::now(),
                correlation_id: None,
            })
        }
    }

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
    async fn test_respond_routes_through_transport_registry_when_configured() {
        let manager = ChannelManager::new();
        let (stub, _sender) = StubChannel::new("gateway");
        let responses = stub.captured_responses_handle();
        manager.add(Box::new(stub)).await;
        let runtime = RebornTransportRuntime::from_channel_manager(&manager)
            .await
            .expect("runtime");
        manager
            .set_transport_egress_registry(runtime.registry())
            .await;

        let msg = IncomingMessage::new("gateway", "alice", "request")
            .with_sender_id("chat-1")
            .with_thread("thread-1")
            .with_metadata(json!({"channel_id": "chat-1"}));
        manager
            .respond(
                &msg,
                OutgoingResponse::text("reply over transport").in_thread("thread-2"),
            )
            .await
            .expect("respond failed");

        let captured = responses.lock().expect("poisoned");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0.channel, "gateway");
        assert_eq!(captured[0].0.user_id, "alice");
        assert_eq!(captured[0].0.sender_id, "chat-1");
        assert_eq!(captured[0].1.content, "reply over transport");
        assert_eq!(
            captured[0].1.thread_id.as_ref().map(|id| id.as_str()),
            Some("thread-2")
        );
    }

    #[tokio::test]
    async fn test_respond_unknown_channel_errors() {
        let manager = ChannelManager::new();
        let msg = IncomingMessage::new("nonexistent", "user1", "test");
        let result = manager.respond(&msg, OutgoingResponse::text("hi")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_status_routes_through_transport_registry_when_configured() {
        let manager = ChannelManager::new();
        let (stub, _sender) = StubChannel::new("gateway");
        let statuses = stub.captured_statuses_handle();
        manager.add(Box::new(stub)).await;
        let runtime = RebornTransportRuntime::from_channel_manager(&manager)
            .await
            .expect("runtime");
        manager
            .set_transport_egress_registry(runtime.registry())
            .await;

        manager
            .send_status(
                "gateway",
                StatusUpdate::ToolCompleted {
                    name: "search".to_string(),
                    success: false,
                    error: Some("failed".to_string()),
                    parameters: Some("{\"q\":\"near\"}".to_string()),
                    call_id: Some("call-1".to_string()),
                    duration_ms: Some(42),
                },
                &json!({
                    "transport_user_id": "alice",
                    "channel_id": "chat-1",
                    "transport_thread_id": "thread-1"
                }),
            )
            .await
            .expect("status failed");

        let captured = statuses.lock().expect("poisoned");
        assert_eq!(captured.len(), 1);
        assert!(matches!(
            &captured[0],
            StatusUpdate::ToolCompleted {
                name,
                success,
                error,
                parameters,
                call_id,
                duration_ms,
            } if name == "search"
                && !success
                && error.as_deref() == Some("failed")
                && parameters.as_deref() == Some("{\"q\":\"near\"}")
                && call_id.as_deref() == Some("call-1")
                && *duration_ms == Some(42)
        ));
    }

    #[tokio::test]
    async fn test_send_status_with_transport_registry_keeps_missing_channels_best_effort() {
        let manager = ChannelManager::new();
        manager
            .set_transport_egress_registry(Arc::new(TransportRegistry::new()))
            .await;

        manager
            .send_status(
                "missing",
                StatusUpdate::Thinking("still best effort".to_string()),
                &json!({"transport_user_id": "alice"}),
            )
            .await
            .expect("missing status should be ignored");
    }

    #[tokio::test]
    async fn test_send_status_with_transport_registry_fails_closed_without_user_id() {
        // Without an identifiable user, status egress must fail closed instead
        // of routing to a synthetic "default" user that bridges tenants.
        let manager = ChannelManager::new();
        manager
            .set_transport_egress_registry(Arc::new(TransportRegistry::new()))
            .await;

        let result = manager
            .send_status(
                "missing",
                StatusUpdate::Thinking("no user metadata".to_string()),
                &serde_json::Value::Null,
            )
            .await;
        match result {
            Err(ChannelError::SendFailed { reason, .. }) => {
                assert!(
                    reason.contains("missing user identity"),
                    "expected fail-closed reason, got: {reason}"
                );
            }
            other => panic!("expected SendFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_send_status_transport_registry_keeps_routing_target_out_of_user_scope() {
        let manager = ChannelManager::new();
        let (stub, _sender) = StubChannel::new("gateway");
        let statuses = stub.captured_statuses_handle();
        manager.add(Box::new(stub)).await;
        let runtime = RebornTransportRuntime::from_channel_manager(&manager)
            .await
            .expect("runtime");
        manager
            .set_transport_egress_registry(runtime.registry())
            .await;

        manager
            .send_status(
                "gateway",
                StatusUpdate::Thinking("url target is not a user id".to_string()),
                &json!({
                    "target": "https://example.com/hook",
                    "transport_user_id": "alice",
                }),
            )
            .await
            .expect("status should route with explicit user id");

        let captured = statuses.lock().expect("poisoned");
        assert_eq!(captured.len(), 1);
        assert!(
            matches!(&captured[0], StatusUpdate::Thinking(message) if message == "url target is not a user id")
        );
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

    #[tokio::test]
    async fn test_hot_add_registers_reborn_transport_adapter_when_configured() {
        let manager = ChannelManager::new();
        manager
            .set_transport_egress_registry(Arc::new(TransportRegistry::new()))
            .await;

        let (stub, _sender) = StubChannel::new("gateway");
        let responses = stub.captured_responses_handle();
        manager.hot_add(Box::new(stub)).await.expect("hot_add");

        let msg = IncomingMessage::new("gateway", "alice", "request")
            .with_metadata(json!({"channel_id": "chat-1"}));
        manager
            .respond(&msg, OutgoingResponse::text("hot-added reply"))
            .await
            .expect("respond through hot-added adapter");

        let captured = responses.lock().expect("poisoned");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].1.content, "hot-added reply");
    }

    #[tokio::test]
    async fn test_hot_add_uses_reborn_transport_ingress_sink_when_configured() {
        let manager = ChannelManager::new();
        let registry = Arc::new(TransportRegistry::new());
        let (tx, mut rx) = mpsc::channel(1);
        manager
            .set_transport_runtime(registry, Arc::new(CapturingTransportSink { tx }))
            .await;

        let (stub, sender) = StubChannel::new("gateway");
        manager.hot_add(Box::new(stub)).await.expect("hot_add");

        sender
            .send(
                IncomingMessage::new("gateway", "alice", "from hot add")
                    .with_metadata(json!({"channel_id": "chat-1"})),
            )
            .await
            .expect("send");

        let ingress = rx.recv().await.expect("ingress");
        assert_eq!(ingress.route.adapter_id.as_str(), "gateway");
        assert_eq!(ingress.route.scope.user_id.as_str(), "alice");
        assert_eq!(ingress.route.recipient.as_deref(), Some("chat-1"));
        assert_eq!(ingress.message.text, "from hot add");
    }

    #[tokio::test]
    async fn test_transport_adapters_wrap_registered_channels() {
        let manager = ChannelManager::new();
        let (stub, _) = StubChannel::new("gateway");
        manager.add(Box::new(stub)).await;

        let adapters = manager
            .transport_adapters()
            .await
            .expect("transport adapters");

        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].adapter_id().as_str(), "gateway");
    }
}
