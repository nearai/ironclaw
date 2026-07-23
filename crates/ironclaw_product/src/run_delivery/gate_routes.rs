//! Delivered-gate-route recording: after a gate/auth prompt lands in a
//! channel conversation, remember which conversation fingerprints can carry
//! a bare `approve`/`deny` reply back to the run's (possibly foreign) scope.

use crate::ExternalConversationRef;
use chrono::Utc;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_outbound::{DeliveredGateRouteRecord, DeliveredGateRouteStore};
use ironclaw_turns::{TurnRunId, TurnScope};

use super::DeliveredChannelMessage;

/// Record the delivered conversations for a gate/auth prompt so a later
/// reply in one of them resolves the gate. Every fingerprint variant an
/// inbound reply could produce is recorded: with and without the space id
/// (vendors omit it on some events), and with and without the message ref
/// as the topic (threaded replies to the prompt vs bare replies in the
/// conversation root).
// arch-exempt: too_many_args, gate-route identity is a wide tuple; a context bundle is tracked with the P6 extraction.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_gate_route_if_needed(
    route_store: &dyn DeliveredGateRouteStore,
    run_id: TurnRunId,
    tenant_id: &TenantId,
    user_id: &UserId,
    gate_ref: &str,
    scope: &TurnScope,
    delivered: &[DeliveredChannelMessage],
    source_conversation: Option<&ExternalConversationRef>,
) {
    let mut conversation_fingerprints = std::collections::BTreeSet::new();

    for message in delivered {
        let space = message.conversation.space_id();
        let conversation_id = message.conversation.conversation_id();
        let vendor_ref = message.vendor_message_ref.as_str();
        // Space-qualified refs (matches inbound events that carry the
        // vendor space id), threaded on the prompt and at conversation root.
        if let Some(space) = space {
            if let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
                Some(space),
                conversation_id,
                Some(vendor_ref),
                None,
            ) {
                conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
            }
            if let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
                Some(space),
                conversation_id,
                None,
                None,
            ) {
                conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
            }
        }
        // No-space fallbacks for events that omit the space id; the set
        // deduplicates when the two forms coincide.
        if let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
            None,
            conversation_id,
            Some(vendor_ref),
            None,
        ) {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
        if let Ok(conv_ref) =
            ironclaw_conversations::ExternalConversationRef::new(None, conversation_id, None, None)
        {
            conversation_fingerprints.insert(conv_ref.conversation_fingerprint());
        }
    }

    // The originating conversation (without its message ref) also routes:
    // a bare reply next to the prompt resolves the same gate.
    if let Some(source) = source_conversation
        && let Ok(conv_ref) = ironclaw_conversations::ExternalConversationRef::new(
            source.space_id(),
            source.conversation_id(),
            source.topic_id(),
            source.reply_target_message_id(),
        )
    {
        conversation_fingerprints.insert(conv_ref.without_message_id().conversation_fingerprint());
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
        // silent-ok: route recording is best-effort; resolution falls back
        // to explicit gate refs and the hint path, so a write failure never
        // aborts delivery.
        tracing::debug!(
            target = "ironclaw::reborn::run_delivery",
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
            target = "ironclaw::reborn::run_delivery",
            %run_id,
            error = %sweep_err,
            "delivered gate route sweep failed"
        );
    }
}
