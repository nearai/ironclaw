use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_outbound::{DeliveredGateRouteRecord, DeliveredGateRouteStore};
use ironclaw_product_adapters::{
    EgressRequest, EgressResponse, ExternalConversationRef, ProtocolHttpEgress,
    ProtocolHttpEgressError,
};
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::services::*;

// arch-exempt: too_many_args, the generic route recorder receives one cross-owner store plus typed route identity and evidence; bundling remains tracked by this architecture cleanup, plan #6159
#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_gate_route_if_needed(
    route_store: &dyn DeliveredGateRouteStore,
    run_id: TurnRunId,
    tenant_id: &ironclaw_host_api::TenantId,
    user_id: &ironclaw_host_api::UserId,
    gate_ref: &str,
    scope: &TurnScope,
    posted_messages: &[PostedChannelMessage],
    envelope_conv_ref: Option<&ExternalConversationRef>,
    // Slack team id to attach to each posted-message conversation ref so that
    // inbound replies (which carry team_id as space_id) match the fingerprint.
    // A no-space fallback variant is always recorded too, for events that omit
    // team_id; the fingerprint set deduplicates when the two are identical
    // (i.e. when `posted_space_id` is None).
    posted_space_id: Option<&str>,
) {
    let mut conversation_fingerprints = std::collections::BTreeSet::new();

    for msg in posted_messages {
        // Record a space-qualified ref (matches inbound Slack events that carry team_id).
        if let Some(space) = posted_space_id
            && let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
                Some(space),
                &msg.conversation_id,
                Some(&msg.message_ref),
                None,
            )
        {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
        // Also record the root conversation for bare replies sent directly in
        // the DM/channel instead of as a threaded reply to the prompt.
        if let Some(space) = posted_space_id
            && let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
                Some(space),
                &msg.conversation_id,
                None,
                None,
            )
        {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
        // Always record a no-space fallback ref for events that omit team_id.
        if let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
            None,
            &msg.conversation_id,
            Some(&msg.message_ref),
            None,
        ) {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
        if let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
            None,
            &msg.conversation_id,
            None,
            None,
        ) {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
    }

    if let Some(env_ref) = envelope_conv_ref
        && let Ok(env_conv_ref) = conversations_ref_from_product_ref(env_ref)
    {
        let env_no_msg = env_conv_ref.without_message_id();
        conversation_fingerprints.insert(env_no_msg.conversation_fingerprint());
    }

    if conversation_fingerprints.is_empty() {
        return;
    }

    let record = DeliveredGateRouteRecord {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        gate_ref: gate_ref.to_string(),
        run_id,
        scope: scope.clone(),
        recorded_at: Utc::now(),
        delivered_conversation_fingerprints: conversation_fingerprints.into_iter().collect(),
    };

    if let Err(error) = route_store.record_delivered_gate_route(record).await {
        // silent-ok: route recording is best-effort; resolution falls back to
        // explicit gate refs and the hint path, so a write failure never
        // aborts delivery.
        tracing::debug!(
            target = "ironclaw::reborn::channel_delivery",
            %run_id,
            error = %error,
            "failed to record delivered gate route"
        );
        return;
    }

    if let Err(sweep_err) = route_store
        .sweep_expired_delivered_gate_routes(Utc::now())
        .await
    {
        // silent-ok: sweep is opportunistic; expired routes are filtered at
        // lookup time, so a failed sweep never affects correctness.
        tracing::debug!(
            target = "ironclaw::reborn::channel_delivery",
            %run_id,
            error = %sweep_err,
            "delivered gate route sweep failed"
        );
    }
}

pub(crate) fn conversations_ref_from_product_ref(
    conv_ref: &ExternalConversationRef,
) -> Result<ironclaw_conversations::ExternalConversationRef, ironclaw_conversations::InboundTurnError>
{
    ironclaw_conversations::ExternalConversationRef::new(
        conv_ref.space_id(),
        conv_ref.conversation_id(),
        conv_ref.topic_id(),
        conv_ref.reply_target_message_id(),
    )
}

/// Egress decorator that records posted-message handles from adapter-rendered
/// responses (via the channel protocol's sniffing) so placeholder/working
/// messages can be deleted and gate routes can reference the posted message.
pub(crate) struct TrackingPostEgress {
    inner: Arc<dyn ProtocolHttpEgress>,
    protocol: Arc<dyn ChannelDeliveryProtocol>,
    posted_messages: Arc<Mutex<Vec<PostedChannelMessage>>>,
}

impl TrackingPostEgress {
    pub(crate) fn new(
        inner: Arc<dyn ProtocolHttpEgress>,
        protocol: Arc<dyn ChannelDeliveryProtocol>,
    ) -> Self {
        Self {
            inner,
            protocol,
            posted_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn take_posted_messages(&self) -> Vec<PostedChannelMessage> {
        std::mem::take(
            &mut *self
                .posted_messages
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

#[async_trait]
impl ProtocolHttpEgress for TrackingPostEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        let request_path = request.path().as_str().to_string();
        let request_body = request.body().to_vec();
        let response = self.inner.send(request).await?;
        if let Some(message) = self.protocol.posted_message_from_render_response(
            &request_path,
            &request_body,
            response.body(),
        ) {
            self.posted_messages
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(message);
        }
        Ok(response)
    }
}
