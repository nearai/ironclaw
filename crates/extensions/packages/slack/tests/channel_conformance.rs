//! TEST-1: the Slack channel adapter runs the exported channel-adapter
//! conformance suite against a scripted Slack Web API — including the
//! `stream` reply cadence (Opened → repeat → Terminal → repeat) against the
//! native Agent endpoints.

use std::sync::Arc;

use ironclaw_extension_contracts::channel::ReplyTransport;
use ironclaw_extension_contracts::channel_adapter::ChannelSurfaces;
use ironclaw_extension_contracts::channel_adapter::{
    OutboundEnvelope, OutboundPart, OutboundTarget, OutboundVisibility,
};
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::test_support::conformance::{
    ChannelAdapterConformance, ConformanceInbound, run_channel_adapter_conformance,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_slack_extension::SlackChannelAdapter;

fn scripted_slack_api(request: &RestrictedEgressRequest) -> RestrictedEgressResponse {
    let body = if request.url.ends_with("/api/chat.postMessage") {
        br#"{"ok":true,"channel":"D123","ts":"1710000001.000001"}"#.to_vec()
    } else if request.url.ends_with("/api/conversations.open") {
        br#"{"ok":true,"channel":{"id":"D123"}}"#.to_vec()
    } else if request.url.ends_with("/api/agents.sessions.setStatus") {
        br#"{"ok":true,"status":"processing","agent_status":"processing"}"#.to_vec()
    } else if request.url.ends_with("/api/chat.startStream")
        || request.url.ends_with("/api/chat.appendStream")
        || request.url.ends_with("/api/chat.stopStream")
    {
        br#"{"ok":true,"channel":"D123","ts":"1710000002.000002"}"#.to_vec()
    } else {
        br#"{"ok":true}"#.to_vec()
    };
    RestrictedEgressResponse {
        retry_after: None,
        status: 200,
        body,
    }
}

#[tokio::test]
async fn slack_channel_adapter_satisfies_the_conformance_contract() {
    run_channel_adapter_conformance(ChannelAdapterConformance {
        // Slack implements every half: a webhook ingress, a stream reply
        // sink on the native Agent surface, and a message delivery. The
        // suite drives exactly what is bound, at the declared cadence.
        surfaces: {
            let adapter = Arc::new(SlackChannelAdapter);
            ChannelSurfaces::default()
                .with_ingress(adapter.clone())
                .with_reply(adapter.clone())
                .with_delivery(adapter)
        },
        reply_transport: Some(ReplyTransport::Stream),
        extension_id: "slack".to_string(),
        installation_id: "install_alpha".to_string(),
        message_inbound: Some(ConformanceInbound {
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
        }),
        challenge_inbound: Some(ConformanceInbound {
            body: br#"{"type":"url_verification","challenge":"conformance-token"}"#.to_vec(),
            headers: Vec::new(),
        }),
        outbound_envelope: OutboundEnvelope {
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
            registrations: Vec::new(),
            visibility: OutboundVisibility::Public,
        },
        vendor_responses: Arc::new(scripted_slack_api),
        config: Vec::new(),
    })
    .await;
}
