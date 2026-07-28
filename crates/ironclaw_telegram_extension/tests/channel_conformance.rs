//! TEST-1: the Telegram channel adapter runs the exported channel-adapter
//! conformance suite against a scripted Bot API.

use std::sync::Arc;

use ironclaw_host_api::product_adapter::test_support::conformance::{
    ChannelAdapterConformance, ConformanceInbound, run_channel_adapter_conformance,
};
use ironclaw_host_api::product_adapter::{
    ExternalConversationRef, OutboundEnvelope, OutboundPart, OutboundTarget,
};
use ironclaw_host_api::{RestrictedEgressRequest, RestrictedEgressResponse};
use ironclaw_telegram_extension::{TELEGRAM_WEBHOOK_URL_CONFIG, TelegramChannelAdapter};
use ironclaw_telegram_v2_adapter::GroupTriggerPolicy;

fn scripted_bot_api(request: &RestrictedEgressRequest) -> RestrictedEgressResponse {
    let body = if request.url.ends_with("/sendMessage") {
        br#"{"ok":true,"result":{"message_id":42}}"#.to_vec()
    } else if request.url.ends_with("/deleteMessage") {
        br#"{"ok":true,"result":true}"#.to_vec()
    } else {
        // setWebhook / deleteWebhook and friends.
        br#"{"ok":true,"result":true}"#.to_vec()
    };
    RestrictedEgressResponse { status: 200, body }
}

#[tokio::test]
async fn telegram_adapter_satisfies_the_conformance_contract() {
    run_channel_adapter_conformance(ChannelAdapterConformance {
        adapter: Arc::new(TelegramChannelAdapter::new(GroupTriggerPolicy::default())),
        extension_id: "telegram".to_string(),
        installation_id: "install_alpha".to_string(),
        message_inbound: ConformanceInbound {
            body: br#"{
                "update_id": 99,
                "message": {
                    "message_id": 7,
                    "date": 1710000000,
                    "text": "conformance hello",
                    "from": {"id": 1001, "is_bot": false, "first_name": "Ada"},
                    "chat": {"id": 8675309, "type": "private"}
                }
            }"#
            .to_vec(),
            headers: Vec::new(),
        },
        // Telegram has no URL-verification challenge; webhook auth rides the
        // shared secret header the host verifies.
        challenge_inbound: None,
        outbound_envelope: OutboundEnvelope {
            extension_id: "telegram".to_string(),
            installation_id: "install_alpha".to_string(),
            delivery_attempt_id: "attempt-conformance".to_string(),
            target: OutboundTarget {
                conversation: ExternalConversationRef::new(None, "8675309", None, None)
                    .expect("conversation"),
                thread_anchor: None,
            },
            parts: vec![
                OutboundPart::Text("conformance reply".to_string()),
                OutboundPart::Retract {
                    vendor_message_ref: "42".to_string(),
                },
            ],
            reply_context: None,
        },
        vendor_responses: Arc::new(scripted_bot_api),
        config: vec![(
            TELEGRAM_WEBHOOK_URL_CONFIG.to_string(),
            "https://example.test/webhooks/extensions/telegram/events".to_string(),
        )],
        expects_unsupported_free_target_listing: true,
    })
    .await;
}
