//! Composition helpers for wiring transport adapters into Reborn runtime paths.

use std::sync::Arc;

use ironclaw_common::ExternalThreadId;
use ironclaw_transport::{
    AttachmentKind as TransportAttachmentKind, TransportAdapter, TransportAdapterId,
    TransportAttachment, TransportDeliveryAck, TransportEgress, TransportError, TransportErrorKind,
    TransportHealth, TransportIngress, TransportIngressSink, TransportMetadata, TransportRegistry,
    TransportSubmission,
};
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::channels::{
    AttachmentKind, ChannelManager, IncomingAttachment, IncomingMessage, MessageStream,
};
use crate::error::ChannelError;

/// Runtime-owned transport composition surface for Reborn startup wiring.
///
/// This type owns the transport registry used by the kernel-facing runtime path.
/// It deliberately does not implement turn orchestration, authorization, approval
/// resolution, transcript durability, or projection reduction.
#[derive(Clone)]
pub struct RebornTransportRuntime {
    registry: Arc<TransportRegistry>,
}

impl RebornTransportRuntime {
    /// Create an empty transport runtime.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(TransportRegistry::new()),
        }
    }

    /// Wrap an existing transport registry.
    pub fn with_registry(registry: Arc<TransportRegistry>) -> Self {
        Self { registry }
    }

    /// Build a transport runtime from the currently registered v1 channels.
    pub async fn from_channel_manager(channels: &ChannelManager) -> Result<Self, TransportError> {
        let runtime = Self::new();
        register_channel_transport_adapters(runtime.registry.as_ref(), channels).await?;
        Ok(runtime)
    }

    /// Return the underlying registry handle for kernel composition.
    pub fn registry(&self) -> Arc<TransportRegistry> {
        Arc::clone(&self.registry)
    }

    /// List currently registered adapter ids.
    pub fn adapter_ids(&self) -> Result<Vec<TransportAdapterId>, TransportError> {
        self.registry.adapter_ids()
    }

    /// Start every registered adapter against the provided kernel ingress sink.
    pub async fn start(&self, sink: Arc<dyn TransportIngressSink>) -> Result<(), TransportError> {
        self.registry.start_all(sink).await
    }

    /// Deliver egress through the adapter named by its typed route.
    pub async fn deliver(
        &self,
        egress: TransportEgress,
    ) -> Result<TransportDeliveryAck, TransportError> {
        let adapter_id = egress
            .route()
            .map(|route| route.adapter_id.clone())
            .ok_or_else(|| {
                TransportError::new(
                    TransportErrorKind::InvalidRequest,
                    "transport egress cannot be routed without a route",
                )
            })?;
        self.registry.deliver(&adapter_id, egress).await
    }

    /// Health-check all registered adapters.
    pub async fn health_check_all(
        &self,
    ) -> Result<std::collections::BTreeMap<TransportAdapterId, TransportHealth>, TransportError>
    {
        self.registry.health_check_all().await
    }

    /// Shut down all registered adapters.
    pub async fn shutdown(&self) -> Result<(), TransportError> {
        self.registry.shutdown_all().await
    }
}

impl Default for RebornTransportRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Register v1 channel adapters into a Reborn transport registry.
pub async fn register_channel_transport_adapters(
    registry: &TransportRegistry,
    channels: &ChannelManager,
) -> Result<Vec<TransportAdapterId>, TransportError> {
    let adapters = channels.transport_adapters().await?;
    let mut adapter_ids = Vec::with_capacity(adapters.len());
    for adapter in adapters {
        let adapter_id = adapter.adapter_id().clone();
        registry.register(Arc::new(adapter) as Arc<dyn TransportAdapter>)?;
        adapter_ids.push(adapter_id);
    }
    adapter_ids.sort();
    Ok(adapter_ids)
}

/// Agent-facing message source started through Reborn transport composition.
pub struct LegacyAgentTransportSource {
    pub runtime: RebornTransportRuntime,
    pub messages: MessageStream,
}

/// Start registered channels through Reborn transport and return the v1 agent
/// message stream that receives normalized transport ingress.
pub async fn start_legacy_agent_transport_source(
    channels: &ChannelManager,
) -> Result<LegacyAgentTransportSource, TransportError> {
    let runtime = RebornTransportRuntime::from_channel_manager(channels).await?;
    let messages = channels
        .take_injected_message_stream()
        .await
        .map_err(channel_error)?;
    let sink = Arc::new(LegacyAgentTransportSink::from_channel_manager(channels));
    runtime.start(sink.clone()).await?;
    channels
        .set_transport_runtime(runtime.registry(), sink)
        .await;
    Ok(LegacyAgentTransportSource { runtime, messages })
}

/// Kernel-side ingress sink that hands normalized Reborn transport ingress to
/// the existing v1 agent message stream.
///
/// This is a compatibility bridge for the transition period: transport startup
/// can be owned by [`RebornTransportRuntime`] while the current agent loop still
/// consumes [`IncomingMessage`]. Authorization, approvals, session/thread
/// resolution, transcript writes, and model/tool execution remain in the agent
/// path after injection.
#[derive(Clone)]
pub struct LegacyAgentTransportSink {
    inject_tx: mpsc::Sender<IncomingMessage>,
}

impl LegacyAgentTransportSink {
    pub fn new(inject_tx: mpsc::Sender<IncomingMessage>) -> Self {
        Self { inject_tx }
    }

    pub fn from_channel_manager(channels: &ChannelManager) -> Self {
        Self::new(channels.inject_sender())
    }
}

#[async_trait::async_trait]
impl TransportIngressSink for LegacyAgentTransportSink {
    async fn submit_ingress(
        &self,
        ingress: TransportIngress,
    ) -> Result<TransportSubmission, TransportError> {
        let message = transport_ingress_to_incoming_message(ingress)?;
        self.inject_tx.send(message).await.map_err(|_| {
            TransportError::new(
                TransportErrorKind::Unavailable,
                "legacy agent ingress channel closed",
            )
        })?;
        Ok(TransportSubmission {
            accepted_at: chrono::Utc::now(),
            correlation_id: None,
        })
    }
}

/// Convert normalized Reborn ingress into the current v1 [`IncomingMessage`]
/// shape.
pub fn transport_ingress_to_incoming_message(
    ingress: TransportIngress,
) -> Result<IncomingMessage, TransportError> {
    let mut metadata = ingress_metadata_value(&ingress.metadata, &ingress.route.metadata);
    let transport_message_id = ingress.message_id.as_str().to_string();
    metadata_insert(
        &mut metadata,
        "transport_message_id",
        transport_message_id.clone(),
    );
    metadata_insert(
        &mut metadata,
        "transport_adapter_id",
        ingress.route.adapter_id.as_str().to_string(),
    );
    metadata_insert(
        &mut metadata,
        "transport_user_id",
        ingress.route.scope.user_id.as_str().to_string(),
    );
    if let Some(recipient) = &ingress.route.recipient {
        metadata_insert(&mut metadata, "transport_recipient", recipient.clone());
        if metadata.get("target").is_none() {
            metadata_insert(&mut metadata, "target", recipient.clone());
        }
    }
    if let Some(conversation_id) = &ingress.route.conversation_id {
        metadata_insert(
            &mut metadata,
            "transport_conversation_id",
            conversation_id.clone(),
        );
    }
    if let Some(thread_id) = &ingress.route.thread_id {
        metadata_insert(
            &mut metadata,
            "transport_thread_id",
            thread_id.as_str().to_string(),
        );
    }

    let message_id = Uuid::parse_str(&transport_message_id).unwrap_or_else(|_| Uuid::new_v4());
    let sender_id = metadata
        .get("sender_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| ingress.route.recipient.clone())
        .unwrap_or_else(|| ingress.route.scope.user_id.as_str().to_string());

    let mut message = IncomingMessage {
        id: message_id,
        channel: ingress.route.adapter_id.as_str().to_string(),
        user_id: ingress.route.scope.user_id.as_str().to_string(),
        sender_id,
        user_name: ingress.sender_display_name,
        content: ingress.message.text,
        structured_submission: None,
        thread_id: None,
        conversation_scope_id: None,
        received_at: ingress.received_at,
        metadata,
        timezone: ingress.timezone,
        attachments: ingress
            .message
            .attachments
            .iter()
            .map(incoming_attachment_from_transport)
            .collect(),
        is_internal: false,
        is_agent_broadcast: false,
        triggering_mission_id: None,
    };

    if let Some(thread_id) = &ingress.route.thread_id {
        message = message.with_external_thread(ExternalThreadId::from_trusted(
            thread_id.as_str().to_string(),
        ));
    }
    if let Some(conversation_id) = &ingress.route.conversation_id {
        message = message.with_conversation_scope(conversation_id.clone());
    }

    Ok(message)
}

fn incoming_attachment_from_transport(attachment: &TransportAttachment) -> IncomingAttachment {
    IncomingAttachment {
        id: attachment.id.clone(),
        kind: match attachment.kind {
            TransportAttachmentKind::Audio => AttachmentKind::Audio,
            TransportAttachmentKind::Image => AttachmentKind::Image,
            TransportAttachmentKind::Document
            | TransportAttachmentKind::Video
            | TransportAttachmentKind::Other => AttachmentKind::Document,
        },
        mime_type: attachment
            .mime_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string()),
        filename: attachment.filename.clone(),
        size_bytes: attachment.size_bytes,
        source_url: attachment.source_url.clone(),
        storage_key: attachment.storage_ref.clone(),
        local_path: attachment
            .metadata
            .get("local_path")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        extracted_text: attachment
            .metadata
            .get("extracted_text")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        data: Vec::new(),
        duration_secs: attachment
            .metadata
            .get("duration_secs")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
    }
}

fn ingress_metadata_value(ingress: &TransportMetadata, route: &TransportMetadata) -> Value {
    let mut merged = ingress.clone();
    merged.extend(
        route
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    Value::Object(merged.into_iter().collect::<Map<_, _>>())
}

fn metadata_insert(metadata: &mut Value, key: &'static str, value: String) {
    if let Value::Object(map) = metadata {
        map.insert(key.to_string(), Value::String(value));
    }
}

fn channel_error(error: ChannelError) -> TransportError {
    let kind = match error {
        ChannelError::StartupFailed { .. } => TransportErrorKind::StartupFailed,
        ChannelError::Disconnected { .. }
        | ChannelError::HealthCheckFailed { .. }
        | ChannelError::RateLimited { .. } => TransportErrorKind::Unavailable,
        ChannelError::SendFailed { .. } | ChannelError::MissingRoutingTarget { .. } => {
            TransportErrorKind::DeliveryFailed
        }
        ChannelError::InvalidMessage(_) => TransportErrorKind::InvalidRequest,
        ChannelError::AuthFailed { .. } => TransportErrorKind::Unauthorized,
        ChannelError::Http(_) => TransportErrorKind::DeliveryFailed,
    };
    TransportError::new(kind, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
    use ironclaw_transport::{
        TransportAdapterId, TransportEgress, TransportError, TransportIngress,
        TransportIngressSink, TransportMessageId, TransportReply, TransportRoute,
        TransportSubmission,
    };
    use serde_json::json;
    use tokio::sync::mpsc;

    use crate::channels::{ChannelManager, IncomingMessage, OutgoingResponse};
    use crate::reborn::transport::{RebornTransportRuntime, start_legacy_agent_transport_source};
    use crate::testing::StubChannel;

    struct CaptureSink {
        tx: mpsc::Sender<TransportIngress>,
    }

    #[async_trait]
    impl TransportIngressSink for CaptureSink {
        async fn submit_ingress(
            &self,
            ingress: TransportIngress,
        ) -> Result<TransportSubmission, TransportError> {
            self.tx.send(ingress).await.map_err(|_| {
                TransportError::new(
                    ironclaw_transport::TransportErrorKind::Unavailable,
                    "capture sink closed",
                )
            })?;
            Ok(TransportSubmission {
                accepted_at: Utc::now(),
                correlation_id: None,
            })
        }
    }

    fn route(adapter: &str, user: &str) -> TransportRoute {
        TransportRoute {
            adapter_id: TransportAdapterId::new(adapter).expect("valid adapter id"),
            scope: ResourceScope::local_default(
                UserId::new(user).expect("valid user"),
                InvocationId::new(),
            )
            .expect("valid scope"),
            recipient: Some("chat-1".to_string()),
            conversation_id: Some("chat-1".to_string()),
            thread_id: None,
            metadata: [("channel_id".to_string(), json!("chat-1"))]
                .into_iter()
                .collect(),
        }
    }

    #[tokio::test]
    async fn reborn_transport_runtime_registers_channel_adapters_and_routes_reply() {
        let manager = ChannelManager::new();
        let (channel, _) = StubChannel::new("gateway");
        let responses = channel.captured_responses_handle();
        manager.add(Box::new(channel)).await;

        let runtime = RebornTransportRuntime::from_channel_manager(&manager)
            .await
            .expect("runtime");

        let adapter_ids = runtime.adapter_ids().expect("adapter ids");
        assert_eq!(adapter_ids.len(), 1);
        assert_eq!(adapter_ids[0].as_str(), "gateway");

        let ack = runtime
            .deliver(TransportEgress::Reply(TransportReply {
                route: route("gateway", "alice"),
                content: "hello from reborn".to_string(),
                attachments: Vec::new(),
                metadata: Default::default(),
            }))
            .await
            .expect("reply delivered");

        assert_eq!(ack.adapter_id.as_str(), "gateway");
        let responses = responses.lock().expect("poisoned");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].0.channel, "gateway");
        assert_eq!(responses[0].0.user_id, "alice");
        assert_eq!(responses[0].1.content, "hello from reborn");
    }

    #[tokio::test]
    async fn reborn_transport_runtime_start_forwards_channel_ingress_to_sink() {
        let manager = ChannelManager::new();
        let (channel, sender) = StubChannel::new("gateway");
        manager.add(Box::new(channel)).await;
        let runtime = RebornTransportRuntime::from_channel_manager(&manager)
            .await
            .expect("runtime");

        let (tx, mut rx) = mpsc::channel(4);
        runtime
            .start(Arc::new(CaptureSink { tx }))
            .await
            .expect("runtime started");

        let message = IncomingMessage::new("gateway", "alice", "hello kernel")
            .with_metadata(json!({"channel_id": "chat-1"}));
        let message_id = message.id.to_string();
        sender.send(message).await.expect("send message");

        let ingress = rx.recv().await.expect("ingress forwarded through runtime");
        assert_eq!(ingress.route.adapter_id.as_str(), "gateway");
        assert_eq!(ingress.message_id.as_str(), message_id);
        assert_eq!(ingress.message.text, "hello kernel");
        assert_eq!(ingress.route.recipient.as_deref(), Some("chat-1"));
    }

    #[tokio::test]
    async fn legacy_agent_transport_source_starts_channels_through_reborn_runtime() {
        use futures::StreamExt;

        let manager = ChannelManager::new();
        let (channel, sender) = StubChannel::new("gateway");
        let responses = channel.captured_responses_handle();
        manager.add(Box::new(channel)).await;

        let mut source = start_legacy_agent_transport_source(&manager)
            .await
            .expect("transport source");

        let adapter_ids = source.runtime.adapter_ids().expect("adapter ids");
        assert_eq!(adapter_ids.len(), 1);
        assert_eq!(adapter_ids[0].as_str(), "gateway");

        let message = IncomingMessage::new("gateway", "alice", "hello over reborn")
            .with_metadata(json!({"channel_id": "chat-1"}));
        sender.send(message).await.expect("send message");

        let injected = source
            .messages
            .next()
            .await
            .expect("message injected through source");
        assert_eq!(injected.channel, "gateway");
        assert_eq!(injected.user_id, "alice");
        assert_eq!(injected.content, "hello over reborn");
        assert_eq!(injected.conversation_scope(), None);
        assert_eq!(injected.routing_target().as_deref(), Some("chat-1"));
        assert_eq!(
            injected.metadata.get("transport_adapter_id"),
            Some(&json!("gateway"))
        );
        assert_eq!(
            injected.metadata.get("transport_user_id"),
            Some(&json!("alice"))
        );

        manager
            .respond(&injected, OutgoingResponse::text("reply over reborn"))
            .await
            .expect("response delivered through transport registry");
        let responses = responses.lock().expect("poisoned");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].1.content, "reply over reborn");
    }

    #[tokio::test]
    async fn reborn_transport_runtime_rejects_unroutable_egress() {
        let runtime = RebornTransportRuntime::new();

        let err = runtime
            .deliver(TransportEgress::Heartbeat(
                ironclaw_transport::TransportHeartbeat {
                    route: None,
                    cursor: None,
                },
            ))
            .await
            .expect_err("unroutable egress should fail");

        assert_eq!(
            err.kind(),
            ironclaw_transport::TransportErrorKind::InvalidRequest
        );
    }

    #[tokio::test]
    async fn legacy_agent_transport_sink_injects_message_with_typed_route_fields() {
        let (tx, mut rx) = mpsc::channel(4);
        let sink = crate::reborn::transport::LegacyAgentTransportSink::new(tx);
        let ingress = TransportIngress {
            message_id: TransportMessageId::new("external-message-1").expect("message id"),
            route: route("gateway", "alice"),
            message: ironclaw_transport::TransportMessage {
                text: "hello legacy agent".to_string(),
                attachments: Vec::new(),
            },
            sender_display_name: Some("Alice".to_string()),
            timezone: Some("America/Los_Angeles".to_string()),
            received_at: Utc::now(),
            metadata: [
                ("channel".to_string(), json!("evil")),
                ("user_id".to_string(), json!("mallory")),
                ("thread_id".to_string(), json!("wrong-thread")),
                ("sender_id".to_string(), json!("alice-transport")),
            ]
            .into_iter()
            .collect(),
        };

        sink.submit_ingress(ingress)
            .await
            .expect("ingress submitted");

        let message = rx.recv().await.expect("message injected");
        assert_eq!(message.channel, "gateway");
        assert_eq!(message.user_id, "alice");
        assert_eq!(message.sender_id, "alice-transport");
        assert_eq!(message.user_name.as_deref(), Some("Alice"));
        assert_eq!(message.content, "hello legacy agent");
        assert_eq!(message.timezone.as_deref(), Some("America/Los_Angeles"));
        assert_eq!(message.conversation_scope(), Some("chat-1"));
        assert_eq!(
            message.metadata.get("transport_message_id"),
            Some(&json!("external-message-1"))
        );
        assert_eq!(message.metadata.get("channel"), Some(&json!("evil")));
        assert_eq!(message.metadata.get("user_id"), Some(&json!("mallory")));
    }

    #[test]
    fn transport_ingress_to_incoming_message_maps_attachment_descriptors() {
        let ingress = TransportIngress {
            message_id: TransportMessageId::new(uuid::Uuid::new_v4().to_string())
                .expect("message id"),
            route: route("gateway", "alice"),
            message: ironclaw_transport::TransportMessage {
                text: "see file".to_string(),
                attachments: vec![ironclaw_transport::TransportAttachment {
                    id: "att-1".to_string(),
                    kind: ironclaw_transport::AttachmentKind::Image,
                    mime_type: Some("image/png".to_string()),
                    filename: Some("diagram.png".to_string()),
                    size_bytes: Some(123),
                    storage_ref: Some("attachments/att-1".to_string()),
                    source_url: Some("https://example.invalid/file.png".to_string()),
                    metadata: [("local_path".to_string(), json!("tmp/diagram.png"))]
                        .into_iter()
                        .collect(),
                }],
            },
            sender_display_name: None,
            timezone: None,
            received_at: Utc::now(),
            metadata: Default::default(),
        };

        let message = crate::reborn::transport::transport_ingress_to_incoming_message(ingress)
            .expect("converted");

        assert_eq!(message.attachments.len(), 1);
        let attachment = &message.attachments[0];
        assert_eq!(attachment.id, "att-1");
        assert_eq!(attachment.kind, crate::channels::AttachmentKind::Image);
        assert_eq!(attachment.mime_type, "image/png");
        assert_eq!(attachment.filename.as_deref(), Some("diagram.png"));
        assert_eq!(attachment.size_bytes, Some(123));
        assert_eq!(attachment.storage_key.as_deref(), Some("attachments/att-1"));
        assert_eq!(attachment.local_path.as_deref(), Some("tmp/diagram.png"));
        assert!(attachment.data.is_empty());
    }
}
