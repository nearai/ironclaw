//! TEST-1: the Telegram channel adapter runs the exported channel-adapter
//! conformance suite against a scripted Bot API.

use std::sync::Arc;

use ironclaw_extension_contracts::channel_adapter::ChannelSurfaces;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelError, ChannelIngress, InboundOutcome, OutboundEnvelope, OutboundPart, OutboundTarget,
    OutboundVisibility, VerifiedInbound,
};
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::test_support::conformance::{
    ChannelAdapterConformance, ConformanceInbound, ScriptedVendorServer,
    run_channel_adapter_conformance,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_telegram_extension::GroupTriggerPolicy;
use ironclaw_telegram_extension::{
    TELEGRAM_BOT_USERNAME_CONFIG, TELEGRAM_WEBHOOK_URL_CONFIG, TelegramChannelAdapter,
};

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
        // Telegram implements every half, for the same reason Slack does:
        // one vendor mechanism serves both output axes.
        surfaces: {
            let adapter = Arc::new(TelegramChannelAdapter::new(GroupTriggerPolicy::default()));
            ChannelSurfaces::default()
                .with_ingress(adapter.clone())
                .with_reply(adapter.clone())
                .with_delivery(adapter)
        },
        extension_id: "telegram".to_string(),
        installation_id: "install_alpha".to_string(),
        message_inbound: Some(ConformanceInbound {
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
        }),
        // Telegram has no URL-verification challenge; webhook auth rides the
        // shared secret header the host verifies.
        challenge_inbound: None,
        outbound_envelope: OutboundEnvelope {
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
            registrations: Vec::new(),
            visibility: OutboundVisibility::Public,
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
    })
    .await;
}

// `activation_fails_closed_without_a_valid_bot_username` stood here. There is
// no adapter activation hook any more, so the check it drove has one home
// rather than two: the inbound path below, which rejects the same missing and
// malformed identities. What genuinely changed is the *timing* — a
// syntactically bad `bot_username` now fails the first inbound message
// instead of activation. An absent value still fails activation, because the
// manifest declares the field `required = true`.

#[tokio::test]
async fn inbound_identity_is_required_with_constructor_compatibility() {
    let egress = ScriptedVendorServer::new(Arc::new(scripted_bot_api));
    for config in [
        Vec::new(),
        vec![(
            TELEGRAM_BOT_USERNAME_CONFIG.to_string(),
            " configured_bot".to_string(),
        )],
    ] {
        let result = TelegramChannelAdapter::default()
            .receive(
                VerifiedInbound {
                    extension_id: "telegram",
                    installation_id: "install_alpha",
                    config: &config,
                    body: private_message_body(),
                    headers: &[],
                    can_reply_in_threads: false,
                },
                &egress,
            )
            .await;
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
        .receive(
            VerifiedInbound {
                extension_id: "telegram",
                installation_id: "install_alpha",
                config: &[],
                body: private_message_body(),
                headers: &[],
                can_reply_in_threads: false,
            },
            &egress,
        )
        .await
        .expect("an explicitly configured constructor policy remains valid");
    assert!(matches!(outcome, InboundOutcome::Messages(_)));
}

#[tokio::test]
async fn username_enforces_vendor_grammar_for_inbound() {
    let egress = ScriptedVendorServer::new(Arc::new(scripted_bot_api));
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
                .receive(
                    VerifiedInbound {
                        extension_id: "telegram",
                        installation_id: "install_alpha",
                        config: &config,
                        body: private_message_body(),
                        headers: &[],
                        can_reply_in_threads: false,
                    },
                    &egress
                )
                .await
                .is_err(),
            "{username:?} must fail Telegram's public bot-username grammar"
        );
    }

    for username in ["a_bot".to_string(), format!("{}BOT", "a".repeat(29))] {
        assert!((5..=32).contains(&username.len()));
        let config = vec![(TELEGRAM_BOT_USERNAME_CONFIG.to_string(), username.clone())];
        assert!(
            matches!(
                TelegramChannelAdapter::default()
                    .receive(
                        VerifiedInbound {
                            extension_id: "telegram",
                            installation_id: "install_alpha",
                            config: &config,
                            body: private_message_body(),
                            headers: &[],
                            can_reply_in_threads: false,
                        },
                        &egress
                    )
                    .await,
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
        constructor_compatible
            .receive(
                VerifiedInbound {
                    extension_id: "telegram",
                    installation_id: "install_alpha",
                    config: &[],
                    body: private_message_body(),
                    headers: &[],
                    can_reply_in_threads: false,
                },
                &egress
            )
            .await,
        Ok(InboundOutcome::Messages(_))
    ));
}
