//! TEST-1: the Slack channel adapter runs the exported channel-adapter
//! conformance suite against a scripted Slack Web API.

use std::sync::Arc;

use ironclaw_host_api::product_adapter::test_support::conformance::{
    ChannelAdapterConformance, ConformanceInbound, run_channel_adapter_conformance,
};
use ironclaw_host_api::product_adapter::{
    ExternalConversationRef, OutboundEnvelope, OutboundPart, OutboundTarget,
};
use ironclaw_host_api::{RestrictedEgressRequest, RestrictedEgressResponse};
use ironclaw_slack_extension::SlackChannelAdapter;

fn scripted_slack_api(request: &RestrictedEgressRequest) -> RestrictedEgressResponse {
    let body = if request.url.ends_with("/api/chat.postMessage") {
        br#"{"ok":true,"channel":"D123","ts":"1710000001.000001"}"#.to_vec()
    } else if request.url.ends_with("/api/conversations.open") {
        br#"{"ok":true,"channel":{"id":"D123"}}"#.to_vec()
    } else {
        br#"{"ok":true}"#.to_vec()
    };
    RestrictedEgressResponse { status: 200, body }
}

#[tokio::test]
async fn slack_channel_adapter_satisfies_the_conformance_contract() {
    run_channel_adapter_conformance(ChannelAdapterConformance {
        adapter: Arc::new(SlackChannelAdapter),
        extension_id: "slack".to_string(),
        installation_id: "install_alpha".to_string(),
        message_inbound: ConformanceInbound {
            body: br#"{
                "type": "event_callback",
                "event_id": "Ev-conformance",
                "team_id": "T-A",
                "event": {
                    "type": "message",
                    "user": "U123",
                    "channel": "D123",
                    "channel_type": "im",
                    "text": "conformance hello",
                    "ts": "1710000000.000100"
                }
            }"#
            .to_vec(),
            headers: Vec::new(),
        },
        challenge_inbound: Some(ConformanceInbound {
            body: br#"{"type":"url_verification","challenge":"conformance-token"}"#.to_vec(),
            headers: Vec::new(),
        }),
        outbound_envelope: OutboundEnvelope {
            extension_id: "slack".to_string(),
            installation_id: "install_alpha".to_string(),
            delivery_attempt_id: "attempt-conformance".to_string(),
            target: OutboundTarget {
                conversation: ExternalConversationRef::new(Some("T-A"), "D123", None, None)
                    .expect("conversation"),
                thread_anchor: None,
            },
            parts: vec![
                OutboundPart::Text("conformance reply".to_string()),
                OutboundPart::Retract {
                    vendor_message_ref: "1710000001.000001".to_string(),
                },
            ],
            reply_context: None,
        },
        vendor_responses: Arc::new(scripted_slack_api),
        config: Vec::new(),
        expects_unsupported_free_target_listing: true,
    })
    .await;
}
