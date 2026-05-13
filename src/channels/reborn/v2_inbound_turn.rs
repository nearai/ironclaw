//! Custom [`InboundTurnService`] implementation that bridges Telegram v2
//! inbound into the existing v1 agent runtime via the synthetic
//! [`ProductChannel`] (see [`super::product_channel`]).
//!
//! Why we don't use `DefaultInboundTurnService`:
//!
//! - `DefaultInboundTurnService` submits to `TurnCoordinator`, which writes a
//!   turn record but has no executor wired into the binary today. The Reborn
//!   agent loop ships in a follow-up.
//! - For the tracer, we instead emit an `IncomingMessage` into the existing
//!   v1 `ChannelManager` stream. The v1 agent processes it exactly like any
//!   other channel, produces a reply, and calls `ProductChannel::respond` —
//!   which the synthetic channel maps back through the v2 outbound path.
//!
//! Bridge seam: when the Reborn agent loop lands, swap this impl for
//! `DefaultInboundTurnService` and add a worker that consumes turn-state
//! submissions instead. No other v2 piece needs to change.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{
    ProductInboundEnvelope, ProductInboundPayload, UserMessagePayload,
};
use ironclaw_product_workflow::{
    ConversationBindingService, InboundTurnOutcome, InboundTurnService, ProductWorkflowError,
    ResolveBindingRequest,
};
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};
use tokio::sync::mpsc;

use crate::channels::channel::IncomingMessage;

/// Bridges v2 inbound to the v1 agent via an mpsc bus shared with
/// [`super::product_channel::ProductChannel`].
pub struct V2InboundTurnService {
    binding: Arc<dyn ConversationBindingService>,
    inbound_tx: mpsc::Sender<IncomingMessage>,
    /// Channel name reported on the emitted `IncomingMessage`. Must match
    /// `ProductChannel::name()` so `ChannelManager` routes the response back
    /// to the synthetic channel.
    channel_name: String,
}

impl V2InboundTurnService {
    pub fn new(
        binding: Arc<dyn ConversationBindingService>,
        inbound_tx: mpsc::Sender<IncomingMessage>,
        channel_name: impl Into<String>,
    ) -> Self {
        Self {
            binding,
            inbound_tx,
            channel_name: channel_name.into(),
        }
    }
}

#[async_trait]
impl InboundTurnService for V2InboundTurnService {
    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        let user_message: &UserMessagePayload = match envelope.payload() {
            ProductInboundPayload::UserMessage(m) => m,
            other => {
                return Err(ProductWorkflowError::UnsupportedActionKind {
                    kind: format!("{other:?}"),
                });
            }
        };

        // Resolve binding via the shared service. This also creates the
        // (tenant, user, thread) row on first inbound.
        let resolve_request = ResolveBindingRequest {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            auth_claim: envelope.auth_claim().clone(),
        };
        let binding = self.binding.resolve_binding(resolve_request).await?;

        // Synthesize an IncomingMessage and push it into the v1 stream. The
        // run_id we return is a synthetic correlation id, not a Reborn
        // TurnRunId persisted via TurnCoordinator — the tracer bypasses the
        // turn store entirely.
        let synthetic_run_id = TurnRunId::new();

        let conversation_ref = envelope.external_conversation_ref();
        let metadata = serde_json::json!({
            "v2_conversation_id": conversation_ref.conversation_id(),
            "v2_topic_id": conversation_ref.topic_id(),
            "v2_reply_target_message_id": conversation_ref.reply_target_message_id(),
        });
        let incoming = IncomingMessage::new(
            self.channel_name.clone(),
            binding.user_id.as_str().to_string(),
            user_message.text.clone(),
        )
        .with_sender_id(envelope.external_actor_ref().id().to_string())
        .with_thread(binding.thread_id.as_str().to_string())
        .with_metadata(metadata);

        self.inbound_tx
            .send(incoming)
            .await
            .map_err(|e| ProductWorkflowError::Transient {
                reason: format!("v2 inbound bus closed: {e}"),
            })?;

        let accepted_ref = AcceptedMessageRef::new(format!("v2:{}", synthetic_run_id.as_uuid()))
            .map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
                reason: format!("synthesize accepted_message_ref: {e}"),
            })?;

        Ok(InboundTurnOutcome::Submitted {
            accepted_message_ref: accepted_ref,
            submitted_run_id: synthetic_run_id,
            binding,
        })
    }
}
