//! Synthetic Channel that bridges Reborn v2 inbound (via
//! [`super::v2_inbound_turn::V2InboundTurnService`]) and outbound (via
//! [`ironclaw_product_adapters::ProductAdapter::render_outbound`]).
//!
//! BRIDGE: emits v2-parsed inbound through the v1 `ChannelManager`. Replace
//! with native Reborn execution when the Reborn agent loop ships.
//!
//! Inbound flow:
//!   webhook → workflow → V2InboundTurnService → mpsc::send(IncomingMessage)
//!   → ProductChannel::start()'s ReceiverStream → ChannelManager merged stream
//!   → v1 Agent
//!
//! Outbound flow:
//!   v1 Agent → ChannelManager → ProductChannel::respond
//!   → spawn task → adapter.render_outbound(envelope, &egress, &sink)
//!   → reqwest POST to api.telegram.org (via TelegramHttpEgress)
//!   → sink records OutboundDeliveryAttempt

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, TenantId, ThreadId};
use ironclaw_outbound::{OutboundStateStore, ProjectionUpdateRef};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalConversationRef, FinalReplyView, ProductAdapter,
    ProductAdapterId, ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget,
    ProjectionCursor, ProtocolHttpEgress,
};
use ironclaw_product_workflow_storage::OutboundStateStoreDeliverySink;
use ironclaw_telegram_v2_adapter::TelegramV2Adapter;
use ironclaw_turns::{TurnRunId, TurnScope};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::channels::channel::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
use crate::error::ChannelError;

/// Configured-once parameters for the synthetic Telegram v2 channel.
pub struct ProductChannelConfig {
    pub name: String,
    pub adapter: Arc<TelegramV2Adapter>,
    pub egress: Arc<dyn ProtocolHttpEgress>,
    pub outbound_store: Arc<dyn OutboundStateStore>,
    pub default_tenant_id: TenantId,
    pub default_agent_id: AgentId,
}

pub struct ProductChannel {
    config: ProductChannelConfig,
    /// Receiver is `take()`n on first `start()`. Subsequent starts would error.
    rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
}

impl ProductChannel {
    /// Construct the bus. The returned sender goes to
    /// `V2InboundTurnService`; the channel itself keeps the receiver for
    /// later `start()`.
    pub fn new(config: ProductChannelConfig) -> (Self, mpsc::Sender<IncomingMessage>) {
        let (tx, rx) = mpsc::channel(256);
        (
            Self {
                config,
                rx: Mutex::new(Some(rx)),
            },
            tx,
        )
    }
}

#[async_trait]
impl Channel for ProductChannel {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let rx = self
            .rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: self.config.name.clone(),
                reason: "ProductChannel can only be started once".into(),
            })?;
        tracing::debug!("ProductChannel '{}' ready", self.config.name);
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Carry over the v1 inbound metadata so we can rebuild a
        // ProductOutboundTarget. The metadata was set by V2InboundTurnService
        // when it pushed this IncomingMessage onto the bus.
        let metadata = msg.metadata.clone();
        let conversation_id = metadata
            .get("v2_conversation_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::SendFailed {
                name: self.config.name.clone(),
                reason: "missing v2_conversation_id metadata".into(),
            })?
            .to_string();
        let reply_target_msg_id = metadata
            .get("v2_reply_target_message_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let topic_id = metadata
            .get("v2_topic_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let chat_id_i64: i64 = conversation_id
            .parse()
            .map_err(|e| ChannelError::SendFailed {
                name: self.config.name.clone(),
                reason: format!("v2_conversation_id not numeric: {e}"),
            })?;
        let topic_id_i64: Option<i64> = topic_id.as_ref().and_then(|s| s.parse::<i64>().ok());
        let reply_msg_id_i64: Option<i64> = reply_target_msg_id
            .as_ref()
            .and_then(|s| s.parse::<i64>().ok());

        let thread_id_str = msg
            .thread_id
            .as_ref()
            .map(|t| t.as_str().to_string())
            .ok_or_else(|| ChannelError::SendFailed {
                name: self.config.name.clone(),
                reason: "ProductChannel inbound missing thread_id".into(),
            })?;

        // Adapter accessors are needed for envelope construction.
        let adapter_id = self.config.adapter.config().adapter_id.clone();
        let installation_id = self.config.adapter.config().installation_id.clone();

        // Spawn the render + send + record in a detached task. The agent
        // loop only needs respond() to return after the request is enqueued
        // — if we awaited HTTP here we'd block the agent's event pump.
        let adapter = Arc::clone(&self.config.adapter);
        let egress = Arc::clone(&self.config.egress);
        let outbound_store = Arc::clone(&self.config.outbound_store);
        let default_tenant_id = self.config.default_tenant_id.clone();
        let default_agent_id = self.config.default_agent_id.clone();
        let channel_name = self.config.name.clone();

        tokio::spawn(async move {
            if let Err(err) = render_and_dispatch(
                adapter,
                egress,
                outbound_store,
                adapter_id,
                installation_id,
                response,
                chat_id_i64,
                topic_id_i64,
                reply_msg_id_i64,
                conversation_id,
                thread_id_str,
                default_tenant_id,
                default_agent_id,
            )
            .await
            {
                tracing::warn!(
                    channel = %channel_name,
                    error = %err,
                    "ProductChannel respond: render/dispatch failed"
                );
            }
        });
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn render_and_dispatch(
    adapter: Arc<TelegramV2Adapter>,
    egress: Arc<dyn ProtocolHttpEgress>,
    outbound_store: Arc<dyn OutboundStateStore>,
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    response: OutgoingResponse,
    chat_id: i64,
    topic_id: Option<i64>,
    reply_message_id: Option<i64>,
    conversation_id: String,
    thread_id_str: String,
    default_tenant_id: TenantId,
    default_agent_id: AgentId,
) -> Result<(), String> {
    let reply_target_binding = ironclaw_telegram_v2_adapter::build_reply_target_binding(
        chat_id,
        topic_id,
        reply_message_id,
    );

    let conversation_ref = ExternalConversationRef::new(
        None,
        conversation_id,
        topic_id.map(|t| t.to_string()).as_deref(),
        reply_message_id.map(|r| r.to_string()).as_deref(),
    )
    .map_err(|e| format!("rebuild conversation_ref: {e}"))?;

    let target = ProductOutboundTarget::new(reply_target_binding, conversation_ref, None);

    let final_reply = FinalReplyView {
        turn_run_id: TurnRunId::new(),
        text: response.content,
        generated_at: Utc::now(),
    };

    let projection_cursor = ProjectionCursor::new("v2:telegram_tracer:origin")
        .map_err(|e| format!("projection cursor: {e}"))?;

    let envelope = ProductOutboundEnvelope::new(
        adapter_id,
        installation_id,
        target,
        projection_cursor,
        ProductOutboundPayload::FinalReply(final_reply),
    );

    let thread_id = ThreadId::new(thread_id_str).map_err(|e| format!("thread id: {e}"))?;
    let scope = TurnScope {
        tenant_id: default_tenant_id,
        agent_id: Some(default_agent_id),
        project_id: None,
        thread_id: thread_id.clone(),
    };
    let projection_ref =
        ProjectionUpdateRef::new(format!("v2:final_reply:{}", envelope.delivery_attempt_id))
            .map_err(|e| format!("projection_ref: {e}"))?;
    let sink =
        OutboundStateStoreDeliverySink::new(outbound_store, scope, thread_id, projection_ref);

    adapter
        .render_outbound(envelope, egress.as_ref(), &sink)
        .await
        .map_err(|e| format!("render_outbound: {e}"))?;
    Ok(())
}
