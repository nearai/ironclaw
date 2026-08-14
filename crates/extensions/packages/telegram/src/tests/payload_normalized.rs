use super::*;
use proptest::prelude::*;

fn install_id() -> AdapterInstallationId {
    AdapterInstallationId::new("install_alpha").expect("valid installation")
}

fn policy() -> GroupTriggerPolicy {
    GroupTriggerPolicy {
        bot_username: "ironclaw_bot".to_string(),
        bot_user_id: 9000,
        recognized_commands: vec!["start".to_string(), "help".to_string()],
    }
}

fn normalize(payload: &[u8]) -> Result<TelegramInboundEvent, PayloadParseError> {
    normalize_telegram_update(payload, &install_id(), &policy())
}

fn message(payload: &[u8]) -> Box<ParsedTelegramInboundMessage> {
    match normalize(payload).expect("normalizes") {
        TelegramInboundEvent::Message(message) => message,
        other => panic!("expected message, got {other:?}"),
    }
}

#[test]
fn provider_command_syntax_is_normalized_before_the_host_boundary() {
    let payload = br#"{
        "update_id": 501,
        "message": {
            "message_id": 71,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": -42, "type": "supergroup"},
            "text": "/help@ironclaw_bot verbose now",
            "entities": [{"type": "bot_command", "offset": 0, "length": 18}]
        }
    }"#;

    let message = message(payload);
    assert_eq!(message.text, "/help verbose now");
    assert_eq!(message.trigger, ProductTriggerReason::BotCommand);
}

#[test]
fn private_commands_for_another_bot_remain_ordinary_text() {
    let payload = br#"{
        "update_id": 502,
        "message": {
            "message_id": 72,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"},
            "text": "/model@other_bot openai/gpt-5",
            "entities": [{"type": "bot_command", "offset": 0, "length": 16}]
        }
    }"#;

    let message = message(payload);
    assert_eq!(message.text, "/model@other_bot openai/gpt-5");
    assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
}

#[test]
fn ambient_group_updates_and_senderless_updates_are_ignored() {
    for payload in [
        br#"{
            "update_id": 503,
            "message": {
                "message_id": 73,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "ambient"
            }
        }"#
        .as_slice(),
        br#"{
            "update_id": 504,
            "message": {
                "message_id": 74,
                "date": 1700000000,
                "chat": {"id": -42, "type": "supergroup"},
                "text": "anonymous"
            }
        }"#
        .as_slice(),
    ] {
        assert!(matches!(
            normalize(payload),
            Ok(TelegramInboundEvent::Ignore)
        ));
    }
}

#[test]
fn complete_protocol_metadata_is_normalized_without_file_bytes() {
    let payload = br#"{
        "update_id": 505,
        "message": {
            "message_id": 75,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"},
            "caption": "read this",
            "document": {
                "file_id": "file-alpha",
                "file_name": "alpha.txt",
                "mime_type": "text/plain",
                "file_size": 5
            }
        }
    }"#;

    let message = message(payload);
    assert_eq!(message.text, "read this");
    assert!(message.attachments.is_empty());
    assert_eq!(message.pending_attachments.len(), 1);
    assert_eq!(message.pending_attachments[0].vendor_ref, "file-alpha");
    assert_eq!(
        message.pending_attachments[0]
            .descriptor
            .filename
            .as_deref(),
        Some("alpha.txt")
    );
}

#[test]
fn sticker_updates_normalize_with_truthful_mime_types() {
    // Regression: stickers hardcoded `image/webp` + kind `Sticker`, a pair the
    // descriptor validator rejected — the whole update failed parse and
    // Telegram's redelivery queue wedged behind it. Static stickers are WEBP,
    // animated stickers are gzipped Lottie (.tgs), video stickers are WEBM.
    let cases = [
        (
            r#"{"file_id": "st-static", "file_size": 4096}"#,
            "image/webp",
        ),
        (
            r#"{"file_id": "st-animated", "file_size": 4096, "is_animated": true}"#,
            "application/x-tgsticker",
        ),
        (
            r#"{"file_id": "st-video", "file_size": 4096, "is_video": true}"#,
            "video/webm",
        ),
    ];
    for (sticker_json, expected_mime) in cases {
        let payload = format!(
            r#"{{
                "update_id": 601,
                "message": {{
                    "message_id": 91,
                    "date": 1700000000,
                    "from": {{"id": 777, "is_bot": false, "first_name": "Alice"}},
                    "chat": {{"id": 777, "type": "private"}},
                    "sticker": {sticker_json}
                }}
            }}"#
        );
        let message = message(payload.as_bytes());
        assert!(message.text.is_empty());
        assert_eq!(message.pending_attachments.len(), 1, "{sticker_json}");
        let descriptor = &message.pending_attachments[0].descriptor;
        assert_eq!(descriptor.mime_type, expected_mime, "{sticker_json}");
        assert_eq!(descriptor.kind, ProductAttachmentKind::Sticker);
    }
}

#[test]
fn voice_updates_normalize_as_voice_attachments() {
    // Regression twin of the sticker case: `audio/ogg` + kind `Voice` was
    // unconstructible for the same reason.
    let payload = br#"{
        "update_id": 602,
        "message": {
            "message_id": 92,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"},
            "voice": {"file_id": "voice-1", "mime_type": "audio/ogg", "file_size": 2048}
        }
    }"#;
    let message = message(payload);
    assert!(message.text.is_empty());
    assert_eq!(message.pending_attachments.len(), 1);
    let descriptor = &message.pending_attachments[0].descriptor;
    assert_eq!(descriptor.mime_type, "audio/ogg");
    assert_eq!(descriptor.kind, ProductAttachmentKind::Voice);
}

#[test]
fn media_group_fragments_share_one_event_but_keep_distinct_fragment_ids() {
    let payload = |update_id: i64, message_id: i64, file_id: &str| {
        serde_json::to_vec(&serde_json::json!({
            "update_id": update_id,
            "message": {
                "message_id": message_id,
                "media_group_id": "album-1",
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "document": {
                    "file_id": file_id,
                    "file_name": "part.txt",
                    "mime_type": "text/plain",
                    "file_size": 5
                }
            }
        }))
        .expect("payload")
    };
    let fragment = |payload: Vec<u8>| match normalize(&payload).expect("normalizes") {
        TelegramInboundEvent::BatchFragment(fragment) => fragment,
        other => panic!("expected fragment, got {other:?}"),
    };

    let first = fragment(payload(601, 1, "file-a"));
    let second = fragment(payload(602, 2, "file-b"));
    assert_eq!(first.message.event_id, second.message.event_id);
    assert_ne!(first.fragment_id, second.fragment_id);
    assert_eq!(first.order, 1);
    assert_eq!(second.order, 2);
}

#[test]
fn invalid_update_identity_and_batch_order_fail_closed() {
    assert!(matches!(
        normalize(br#"{"message":null}"#),
        Err(PayloadParseError::MissingUpdateId)
    ));

    let payload = br#"{
        "update_id": 603,
        "message": {
            "message_id": -1,
            "media_group_id": "album-negative",
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"},
            "document": {"file_id": "file-a", "mime_type": "text/plain"}
        }
    }"#;
    assert!(matches!(
        normalize(payload),
        Err(PayloadParseError::InvalidExternalRef {
            kind: "telegram_media_group_order",
            ..
        })
    ));
}

proptest! {
    #[test]
    fn arbitrary_untrusted_bytes_never_panic(raw in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = normalize(&raw);
    }
}
