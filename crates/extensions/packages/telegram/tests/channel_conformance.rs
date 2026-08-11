//! TEST-1: the Telegram channel adapter runs the exported channel-adapter
//! conformance suite against a scripted Bot API.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelAdapter, ChannelContext, ChannelError, InboundOutcome, OutboundEnvelope, OutboundPart,
    OutboundTarget, VerifiedInbound,
};
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::test_support::conformance::{
    ChannelAdapterConformance, ConformanceInbound, run_channel_adapter_conformance,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_telegram_extension::GroupTriggerPolicy;
use ironclaw_telegram_extension::{
    TELEGRAM_BOT_USERNAME_CONFIG, TELEGRAM_WEBHOOK_URL_CONFIG, TelegramChannelAdapter,
};

#[derive(Default)]
struct RecordingEgress {
    requests: Mutex<Vec<RestrictedEgressRequest>>,
}

#[async_trait]
impl RestrictedEgress for RecordingEgress {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        self.requests.lock().expect("requests lock").push(request);
        Ok(RestrictedEgressResponse {
            status: 200,
            body: br#"{"ok":true,"result":true}"#.to_vec(),
        })
    }
}

fn private_message_body() -> &'static [u8] {
    br#"{
        "update_id": 42,
        "message": {
            "message_id": 7,
            "date": 1710000000,
            "text": "hello bot",
            "from": {"id": 1001, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 555, "type": "private"}
        }
    }"#
}

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
        config: vec![
            (
                TELEGRAM_WEBHOOK_URL_CONFIG.to_string(),
                "https://example.test/webhooks/extensions/telegram/events".to_string(),
            ),
            (
                TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
                "conformance_bot".to_string(),
            ),
        ],
        expects_unsupported_free_target_listing: true,
    })
    .await;
}

#[tokio::test]
async fn activation_fails_closed_without_a_valid_bot_username() {
    for username in [None, Some(""), Some(" configured_bot")] {
        let egress = RecordingEgress::default();
        let mut config = vec![(
            TELEGRAM_WEBHOOK_URL_CONFIG.to_string(),
            "https://host.example/hooks".to_string(),
        )];
        if let Some(username) = username {
            config.push((
                TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
                username.to_string(),
            ));
        }
        let context = ChannelContext {
            extension_id: "telegram",
            installation_id: "install_alpha",
            config: &config,
        };

        let error = TelegramChannelAdapter::default()
            .activate(&context, &egress)
            .await
            .expect_err("missing or invalid bot identity must fail activation");
        assert!(matches!(error, ChannelError::VendorWiring { .. }));
        assert!(
            egress.requests.lock().expect("requests lock").is_empty(),
            "invalid identity must fail before setWebhook"
        );
    }
}

#[test]
fn inbound_identity_is_required_with_constructor_compatibility() {
    for config in [
        Vec::new(),
        vec![(
            TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            " configured_bot".to_string(),
        )],
    ] {
        let result = TelegramChannelAdapter::default().inbound(VerifiedInbound {
            extension_id: "telegram",
            installation_id: "install_alpha",
            config: &config,
            body: private_message_body(),
            headers: &[],
            can_reply_in_threads: false,
        });
        let Err(error) = result else {
            panic!("missing or invalid bot identity must fail inbound normalization");
        };
        assert!(matches!(error, ChannelError::Configuration { .. }));
    }

    let adapter = TelegramChannelAdapter::new(GroupTriggerPolicy {
        bot_username: "fixture_bot".to_string(),
        ..GroupTriggerPolicy::default()
    });
    let outcome = adapter
        .inbound(VerifiedInbound {
            extension_id: "telegram",
            installation_id: "install_alpha",
            config: &[],
            body: private_message_body(),
            headers: &[],
            can_reply_in_threads: false,
        })
        .expect("an explicitly configured constructor policy remains valid");
    assert!(matches!(outcome, InboundOutcome::Messages(_)));
}

#[test]
fn username_enforces_vendor_grammar_for_inbound() {
    let invalid_usernames = [
        "fixture_name".to_string(),
        "bot".to_string(),
        format!("{}bot", "a".repeat(30)),
        "valid-bot".to_string(),
    ];
    for username in invalid_usernames {
        let config = vec![(TELEGRAM_BOT_USERNAME_CONFIG.to_string(), username.clone())];
        assert!(
            TelegramChannelAdapter::default()
                .inbound(VerifiedInbound {
                    extension_id: "telegram",
                    installation_id: "install_alpha",
                    config: &config,
                    body: private_message_body(),
                    headers: &[],
                    can_reply_in_threads: false,
                })
                .is_err(),
            "{username:?} must fail Telegram's public bot-username grammar"
        );
    }

    for username in ["a_bot".to_string(), format!("{}BOT", "a".repeat(29))] {
        assert!((5..=32).contains(&username.len()));
        let config = vec![(TELEGRAM_BOT_USERNAME_CONFIG.to_string(), username.clone())];
        assert!(
            matches!(
                TelegramChannelAdapter::default().inbound(VerifiedInbound {
                    extension_id: "telegram",
                    installation_id: "install_alpha",
                    config: &config,
                    body: private_message_body(),
                    headers: &[],
                    can_reply_in_threads: false,
                }),
                Ok(InboundOutcome::Messages(_))
            ),
            "{username:?} is a valid boundary example"
        );
    }

    let constructor_compatible = TelegramChannelAdapter::new(GroupTriggerPolicy {
        bot_username: "c_bot".to_string(),
        ..GroupTriggerPolicy::default()
    });
    assert!(matches!(
        constructor_compatible.inbound(VerifiedInbound {
            extension_id: "telegram",
            installation_id: "install_alpha",
            config: &[],
            body: private_message_body(),
            headers: &[],
            can_reply_in_threads: false,
        }),
        Ok(InboundOutcome::Messages(_))
    ));
}
