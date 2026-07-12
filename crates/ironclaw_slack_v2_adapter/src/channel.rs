//! The Slack [`ChannelAdapter`] (generic ingress cutover, extension-runtime P4).
//!
//! `inbound` is the extension's entire contribution to the inbound pipeline:
//! parse one HOST-VERIFIED Slack Events API request into a normalized
//! outcome. Signature verification moved to the host's generic recipe
//! verifier (the manifest's `[channel.ingress.verification]`); this adapter
//! never sees signing secrets or verification headers. Outbound delivery
//! stays on the pre-coordinator path until the P5 cutover, so `deliver`
//! reports `Unsupported` here.

use async_trait::async_trait;
use ironclaw_product_adapters::{
    AdapterInstallationId, AttachmentRef, ChannelAdapter, ChannelError, DeliveryReport,
    ImmediateResponse, InboundOutcome, NormalizedInboundMessage, OutboundEnvelope, VerifiedInbound,
};

use crate::payload::{SlackInboundEvent, SlackPayloadParseError, normalize_slack_event};

/// Stateless Slack channel adapter: pure protocol parsing for the generic
/// ingress router.
#[derive(Debug, Default, Clone, Copy)]
pub struct SlackChannelAdapter;

#[async_trait]
impl ChannelAdapter for SlackChannelAdapter {
    fn inbound(&self, request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        let installation_id =
            AdapterInstallationId::new(request.installation_id).map_err(|error| {
                ChannelError::Parse {
                    reason: format!("invalid installation id: {error}"),
                }
            })?;
        match normalize_slack_event(request.body, &installation_id).map_err(parse_error)? {
            SlackInboundEvent::UrlVerification { challenge } => {
                Ok(InboundOutcome::Respond(ImmediateResponse {
                    status: 200,
                    content_type: Some("text/plain".to_string()),
                    body: challenge.into_bytes(),
                }))
            }
            SlackInboundEvent::Ignore => Ok(InboundOutcome::Ignore),
            SlackInboundEvent::Message(message) => {
                let attachments = message
                    .attachments
                    .into_iter()
                    .map(|descriptor| AttachmentRef {
                        vendor_ref: descriptor.external_file_id.clone(),
                        mime_hint: Some(descriptor.mime_type.clone()),
                        descriptor,
                    })
                    .collect();
                Ok(InboundOutcome::Messages(vec![NormalizedInboundMessage {
                    actor: message.actor,
                    conversation: message.conversation,
                    event_id: message.event_id,
                    text: message.text,
                    trigger: message.trigger,
                    attachments,
                    // Reply routing rides the conversation ref's thread
                    // anchors (pre-coordinator delivery path); adopted when
                    // the P5 delivery coordinator consumes stored contexts.
                    reply_context: None,
                }]))
            }
        }
    }

    async fn deliver(
        &self,
        _envelope: OutboundEnvelope,
        _egress: &dyn ironclaw_host_api::RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        // Outbound cutover is extension-runtime P5 (delivery coordinator).
        Err(ChannelError::Unsupported)
    }
}

fn parse_error(error: SlackPayloadParseError) -> ChannelError {
    ChannelError::Parse {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_product_adapters::ProductTriggerReason;

    use super::*;

    fn inbound(body: &[u8]) -> Result<InboundOutcome, ChannelError> {
        SlackChannelAdapter.inbound(VerifiedInbound {
            extension_id: "slack",
            installation_id: "install_alpha",
            body,
            headers: &[],
        })
    }

    #[test]
    fn url_verification_challenge_becomes_an_immediate_response() {
        let outcome = inbound(br#"{"type":"url_verification","challenge":"challenge-token"}"#)
            .expect("challenge parses");
        let InboundOutcome::Respond(response) = outcome else {
            panic!("expected Respond");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"challenge-token");
    }

    #[test]
    fn dm_message_normalizes_with_text_trigger_and_event_identity() {
        let outcome = inbound(
            br#"{
                "type": "event_callback",
                "event_id": "Ev123",
                "team_id": "T-A",
                "event": {
                    "type": "message",
                    "user": "U123",
                    "channel": "D123",
                    "channel_type": "im",
                    "text": "hello there",
                    "ts": "1710000000.000100"
                }
            }"#,
        )
        .expect("dm parses");
        let InboundOutcome::Messages(messages) = outcome else {
            panic!("expected Messages");
        };
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.text, "hello there");
        assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
        assert_eq!(message.event_id.as_str(), "slack-install_alpha-Ev123");
        assert_eq!(message.actor.id(), "U123");
        assert_eq!(message.conversation.conversation_id(), "D123");
        assert!(message.reply_context.is_none());
    }

    #[test]
    fn app_mention_strips_the_leading_mention_and_keeps_thread_anchor() {
        let outcome = inbound(
            br#"{
                "type": "event_callback",
                "event_id": "Ev124",
                "team_id": "T-A",
                "event": {
                    "type": "app_mention",
                    "user": "U123",
                    "channel": "C123",
                    "text": "<@UBOT> summarize this",
                    "ts": "1710000000.000200"
                }
            }"#,
        )
        .expect("mention parses");
        let InboundOutcome::Messages(messages) = outcome else {
            panic!("expected Messages");
        };
        assert_eq!(messages[0].text, "summarize this");
        assert_eq!(messages[0].trigger, ProductTriggerReason::BotMention);
        assert_eq!(
            messages[0].conversation.topic_id(),
            Some("1710000000.000200"),
            "mention without thread anchors on its own ts"
        );
    }

    #[test]
    fn gate_resolution_text_stays_a_plain_message_for_host_reclassification() {
        // The adapter must NOT classify gate resolutions — the host sink
        // does, via `classify_interaction_resolution`.
        let outcome = inbound(
            br#"{
                "type": "event_callback",
                "event_id": "Ev125",
                "team_id": "T-A",
                "event": {
                    "type": "message",
                    "user": "U123",
                    "channel": "D123",
                    "channel_type": "im",
                    "text": "approve gate:approval-00000000-0000-0000-0000-000000000001",
                    "ts": "1710000000.000300"
                }
            }"#,
        )
        .expect("resolution text parses");
        let InboundOutcome::Messages(messages) = outcome else {
            panic!("expected Messages");
        };
        assert!(messages[0].text.starts_with("approve gate:"));
    }

    #[test]
    fn ignored_events_and_bot_echoes_are_authenticated_noops() {
        for body in [
            // Non event_callback wrapper.
            br#"{"type":"team_join","event_id":"Ev1"}"#.as_slice(),
            // Bot echo of its own message.
            br#"{
                "type": "event_callback",
                "event_id": "Ev2",
                "event": {
                    "type": "message",
                    "bot_id": "B123",
                    "channel": "D123",
                    "channel_type": "im",
                    "text": "echo",
                    "ts": "1710000000.000400"
                }
            }"#
            .as_slice(),
            // Channel message without a thread anchor (no mention).
            br#"{
                "type": "event_callback",
                "event_id": "Ev3",
                "event": {
                    "type": "message",
                    "user": "U123",
                    "channel": "C123",
                    "text": "ambient chatter",
                    "ts": "1710000000.000500"
                }
            }"#
            .as_slice(),
        ] {
            assert!(
                matches!(inbound(body), Ok(InboundOutcome::Ignore)),
                "expected Ignore for {}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn malformed_payloads_are_typed_parse_errors() {
        assert!(matches!(
            inbound(br#"{"type":"event_callback""#),
            Err(ChannelError::Parse { .. })
        ));
        // event_callback without event_id would collide dedupe keys.
        assert!(matches!(
            inbound(br#"{"type":"event_callback","event":{"type":"message"}}"#),
            Err(ChannelError::Parse { .. })
        ));
    }

    #[tokio::test]
    async fn deliver_is_unsupported_until_the_delivery_coordinator_cutover() {
        let outcome = SlackChannelAdapter
            .deliver(
                OutboundEnvelope {
                    extension_id: "slack".to_string(),
                    installation_id: "install_alpha".to_string(),
                    delivery_attempt_id: "attempt-1".to_string(),
                    target: ironclaw_product_adapters::OutboundTarget {
                        conversation: ironclaw_product_adapters::ExternalConversationRef::new(
                            None, "D123", None, None,
                        )
                        .expect("conversation"),
                        thread_anchor: None,
                    },
                    parts: Vec::new(),
                    reply_context: None,
                },
                &DenyAllEgress,
            )
            .await;
        assert!(matches!(outcome, Err(ChannelError::Unsupported)));
    }

    struct DenyAllEgress;

    #[async_trait]
    impl ironclaw_host_api::RestrictedEgress for DenyAllEgress {
        async fn send(
            &self,
            _request: ironclaw_host_api::RestrictedEgressRequest,
        ) -> Result<
            ironclaw_host_api::RestrictedEgressResponse,
            ironclaw_host_api::RestrictedEgressError,
        > {
            Err(ironclaw_host_api::RestrictedEgressError::PolicyDenied)
        }
    }
}
