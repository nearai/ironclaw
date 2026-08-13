use super::*;
use proptest::prelude::*;

fn installation_id() -> AdapterInstallationId {
    AdapterInstallationId::new("install-alpha").expect("installation")
}

fn normalize(value: serde_json::Value) -> SlackInboundEvent {
    normalize_slack_event(
        &serde_json::to_vec(&value).expect("payload"),
        &installation_id(),
    )
    .expect("normalizes")
}

fn message(value: serde_json::Value) -> Box<ParsedSlackInboundMessage> {
    match normalize(value) {
        SlackInboundEvent::Message(message) => message,
        other => panic!("expected message, got {other:?}"),
    }
}

#[test]
fn url_verification_is_an_immediate_channel_outcome() {
    assert!(matches!(
        normalize(serde_json::json!({
            "type": "url_verification",
            "challenge": "challenge-token"
        })),
        SlackInboundEvent::UrlVerification { challenge }
            if challenge == "challenge-token"
    ));
}

#[test]
fn dm_and_thread_messages_normalize_to_the_same_contract() {
    let dm = message(serde_json::json!({
        "type": "event_callback",
        "team_id": "T123",
        "event_id": "EvDm",
        "event": {
            "type": "message",
            "channel_type": "im",
            "user": "U123",
            "channel": "D123",
            "text": "hello from dm",
            "ts": "1710000000.000001"
        }
    }));
    assert_eq!(dm.actor.id(), "U123");
    assert_eq!(dm.conversation.conversation_id(), "D123");
    assert_eq!(dm.text, "hello from dm");
    assert_eq!(dm.trigger, ProductTriggerReason::DirectChat);

    let thread = message(serde_json::json!({
        "type": "event_callback",
        "team_id": "T123",
        "event_id": "EvThread",
        "event": {
            "type": "message",
            "user": "U456",
            "channel": "C123",
            "text": "continue",
            "thread_ts": "1710000000.000010",
            "ts": "1710000000.000011"
        }
    }));
    assert_eq!(thread.conversation.topic_id(), Some("1710000000.000010"));
    assert_eq!(thread.trigger, ProductTriggerReason::ReplyToBot);
}

#[test]
fn app_mention_strips_only_the_provider_mention_and_self_roots_a_thread() {
    let message = message(serde_json::json!({
        "type": "event_callback",
        "team_id": "T123",
        "event_id": "EvMention",
        "event": {
            "type": "app_mention",
            "user": "U123",
            "channel": "C123",
            "text": "<@UBOT> please help",
            "ts": "1710000000.000002"
        }
    }));
    assert_eq!(message.text, "please help");
    assert_eq!(message.trigger, ProductTriggerReason::BotMention);
    assert_eq!(message.conversation.topic_id(), Some("1710000000.000002"));
}

#[test]
fn bots_subtypes_and_ambient_channels_are_ignored() {
    for event in [
        serde_json::json!({
            "type": "message", "user": "U1", "channel": "D1", "text": "loop",
            "ts": "1.0", "bot_id": "B1"
        }),
        serde_json::json!({
            "type": "message", "user": "U1", "channel": "D1", "text": "changed",
            "ts": "1.0", "subtype": "message_changed"
        }),
        serde_json::json!({
            "type": "message", "user": "U1", "channel": "C1", "text": "ambient",
            "ts": "1.0"
        }),
    ] {
        assert!(matches!(
            normalize(serde_json::json!({
                "type": "event_callback", "event_id": "EvIgnored", "event": event
            })),
            SlackInboundEvent::Ignore
        ));
    }
}

#[test]
fn attachment_handles_remain_provider_local_until_the_adapter_fetches_them() {
    let message = message(serde_json::json!({
        "type": "event_callback",
        "team_id": "T123",
        "event_id": "EvFile",
        "event": {
            "type": "message",
            "channel_type": "im",
            "user": "U123",
            "channel": "D123",
            "text": "see file",
            "ts": "1710000000.000003",
            "files": [{
                "id": "F123", "mimetype": "text/plain", "name": "notes.txt", "size": 12
            }]
        }
    }));
    assert!(message.attachments.is_empty());
    assert_eq!(message.pending_attachments.len(), 1);
    assert_eq!(message.pending_attachments[0].vendor_ref, "F123");
    assert_eq!(
        message.pending_attachments[0]
            .descriptor
            .filename
            .as_deref(),
        Some("notes.txt")
    );
}

#[test]
fn slash_command_forms_normalize_without_a_second_product_parser() {
    let headers = vec![(
        "content-type".to_string(),
        "application/x-www-form-urlencoded".to_string(),
    )];
    let event = normalize_slack_inbound(
        b"channel_id=D123&channel_name=directmessage&user_id=U123&command=%2Fironclaw&text=hello&trigger_id=trigger-1&team_id=T123",
        &headers,
        &installation_id(),
    )
    .expect("slash form");
    let SlackInboundEvent::Message(message) = event else {
        panic!("slash command must become a message");
    };
    assert_eq!(message.text, "/hello");
    assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
}

#[test]
fn oversized_payload_and_missing_event_id_fail_closed() {
    let oversized = vec![b'x'; MAX_SLACK_PAYLOAD_BYTES + 1];
    assert!(normalize_slack_event(&oversized, &installation_id()).is_err());
    assert!(matches!(
        normalize_slack_event(
            br#"{"type":"event_callback","event":{"type":"message"}}"#,
            &installation_id()
        ),
        Err(SlackPayloadParseError::InvalidExternalRef {
            kind: "external_event_id",
            ..
        })
    ));
}

proptest! {
    #[test]
    fn arbitrary_untrusted_bytes_never_panic(raw in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = normalize_slack_event(&raw, &installation_id());
    }
}
