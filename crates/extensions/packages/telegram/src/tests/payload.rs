use super::*;
use ironclaw_host_api::product_adapter::ProductAdapterId;
use ironclaw_host_api::product_adapter::auth::AuthRequirement;

fn evidence() -> ProtocolAuthEvidence {
    // `test_verified` is the `test-support` seam standing in for the host:
    // an adapter crate holds no `VerifiedInboundGrant` and must not be able
    // to mint in production (PROPOSAL §12.1a). Value-identical to the
    // pre-WS1.5 `mark_shared_secret_header_verified` call this replaced.
    ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".to_string(),
        },
        "telegram_install_alpha",
    )
}

#[allow(dead_code)]
fn adapter_id() -> ProductAdapterId {
    ProductAdapterId::new("telegram_v2").expect("valid")
}

fn install_id() -> AdapterInstallationId {
    AdapterInstallationId::new("install_alpha").expect("valid")
}

fn policy() -> GroupTriggerPolicy {
    GroupTriggerPolicy {
        bot_username: "ironclaw_bot".into(),
        bot_user_id: 9000,
        recognized_commands: vec!["start".into(), "help".into()],
    }
}

#[test]
fn normalized_bot_qualified_command_uses_generic_command_text() {
    let payload = include_bytes!("../../tests/fixtures/group_command.json");
    let event = normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes");
    let TelegramInboundEvent::Message(message) = event else {
        panic!("recognized group command must be forwarded");
    };
    assert_eq!(message.text, "/help");
    assert_eq!(message.trigger, ProductTriggerReason::BotCommand);
}

#[test]
fn normalized_bot_command_preserves_arguments() {
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
    let event = normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes");
    let TelegramInboundEvent::Message(message) = event else {
        panic!("recognized group command must be forwarded");
    };
    assert_eq!(message.text, "/help verbose now");
}

#[test]
fn private_qualified_product_command_canonicalizes_without_group_whitelist() {
    let payload = br#"{
            "update_id": 503,
            "message": {
                "message_id": 73,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/model@ironclaw_bot openai/gpt-5",
                "entities": [{"type": "bot_command", "offset": 0, "length": 19}]
            }
        }"#;
    let mut policy = policy();
    policy.recognized_commands.clear();

    let event = normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes");
    let TelegramInboundEvent::Message(message) = event else {
        panic!("private chat command must be forwarded");
    };
    assert_eq!(message.text, "/model openai/gpt-5");
    assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
}

#[test]
fn private_command_for_another_bot_keeps_its_target() {
    let payload = br#"{
            "update_id": 504,
            "message": {
                "message_id": 74,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/model@other_bot openai/gpt-5",
                "entities": [{"type": "bot_command", "offset": 0, "length": 16}]
            }
        }"#;
    let mut policy = policy();
    policy.recognized_commands.clear();

    let event = normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes");
    let TelegramInboundEvent::Message(message) = event else {
        panic!("private chat command must be forwarded");
    };
    assert_eq!(message.text, "/model@other_bot openai/gpt-5");
    assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
}

#[test]
fn private_mid_sentence_bot_command_remains_ordinary_text() {
    let payload = br#"{
            "update_id": 506,
            "message": {
                "message_id": 76,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "please run /help for me",
                "entities": [{"type": "bot_command", "offset": 11, "length": 5}]
            }
        }"#;

    let event = normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes");
    let TelegramInboundEvent::Message(message) = event else {
        panic!("private chat text must be forwarded");
    };
    assert_eq!(message.text, "please run /help for me");
    assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
}

#[test]
fn command_after_non_mention_prefix_remains_ordinary_text() {
    let payload = r#"{
            "update_id": 505,
            "message": {
                "message_id": 75,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "🦀 /model@ironclaw_bot openai/gpt-5",
                "entities": [{"type": "bot_command", "offset": 3, "length": 19}]
            }
        }"#
    .as_bytes();
    let mut policy = policy();
    policy.recognized_commands.clear();

    let event = normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes");
    let TelegramInboundEvent::Message(message) = event else {
        panic!("private chat command must be forwarded");
    };
    assert_eq!(
        message.text, "🦀 /model@ironclaw_bot openai/gpt-5",
        "only commands at offset zero or after a leading bot mention are canonicalized"
    );
}

#[test]
fn command_for_another_bot_remains_ignored_in_groups() {
    let payload = br#"{
            "update_id": 502,
            "message": {
                "message_id": 72,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "/help@other_bot",
                "entities": [{"type": "bot_command", "offset": 0, "length": 15}]
            }
        }"#;
    assert!(matches!(
        normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes"),
        TelegramInboundEvent::Ignore
    ));
}

#[test]
fn addressed_product_command_without_group_whitelist_remains_ignored_in_groups() {
    let payload = br#"{
            "update_id": 506,
            "message": {
                "message_id": 76,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "/model@ironclaw_bot openai/gpt-5",
                "entities": [{"type": "bot_command", "offset": 0, "length": 19}]
            }
        }"#;
    let mut policy = policy();
    policy.recognized_commands.clear();

    assert!(matches!(
        normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes"),
        TelegramInboundEvent::Ignore
    ));
}

#[test]
fn unauthenticated_payload_fails_closed() {
    let payload = br#"{"update_id":1}"#;
    // `ProtocolAuthEvidence` is now a sealed struct, not an enum;
    // `failed(failure)` constructs an unverified evidence.
    let evidence = ProtocolAuthEvidence::failed(
        ironclaw_host_api::product_adapter::ProtocolAuthFailure::SharedSecretMismatch,
    );
    let err = parse_telegram_update(payload, &evidence, &install_id(), &policy())
        .expect_err("unauthenticated must error");
    assert!(matches!(err, PayloadParseError::UnauthenticatedPayload));
}

#[test]
fn private_chat_recognized_bot_command_emits_command_payload() {
    // Henry's review (PR #3354) + Copilot's payload.rs:469 finding:
    // `/help` in a DM was previously downgraded to `UserMessage`
    // because the old `build_payload` gated `Command` emission on
    // `trigger == BotCommand`, and private chats always returned
    // `DirectChat`. The fix decouples them: payload kind is decided
    // by whether a recognized `bot_command` entity exists; the
    // trigger keeps its forwarding-reason semantics (DirectChat for
    // DMs).
    let payload = br#"{
            "update_id": 110,
            "message": {
                "message_id": 11,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/help",
                "entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    match envelope.payload {
        ProductInboundPayload::Command(cmd) => {
            assert_eq!(cmd.command, "help");
            assert_eq!(cmd.arguments, "");
            // Trigger reflects WHY the message was forwarded
            // (it's a DM); command-ness is captured in the payload
            // variant, not the trigger.
            assert_eq!(cmd.trigger, ProductTriggerReason::DirectChat);
        }
        other => panic!("expected Command, got {other:?}"),
    }
}

#[test]
fn group_mention_with_bot_command_emits_command_payload() {
    // Copilot's payload.rs:469 finding: a `/command` inside a
    // mention-triggered group message previously emitted
    // `UserMessage` because `build_payload` only produced `Command`
    // when `trigger == BotCommand` — but in groups the mention
    // check fires first and sets `trigger = BotMention`. The
    // decoupled `build_payload` now produces `Command` whenever a
    // recognized command is present, and the trigger preserves
    // the BotMention forwarding reason.
    let payload = br#"{
            "update_id": 220,
            "message": {
                "message_id": 12,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "@ironclaw_bot /help",
                "entities": [
                    {"type": "mention", "offset": 0, "length": 13},
                    {"type": "bot_command", "offset": 14, "length": 5}
                ]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    match envelope.payload {
        ProductInboundPayload::Command(cmd) => {
            assert_eq!(cmd.command, "help");
            assert_eq!(cmd.trigger, ProductTriggerReason::BotMention);
        }
        other => panic!("expected Command, got {other:?}"),
    }
}

#[test]
fn private_chat_unknown_command_still_classifies_as_direct_chat() {
    // Defense-in-depth for the fix above: an UNRECOGNIZED command
    // (`/nope` is not in the policy's `recognized_commands`) must
    // still fall through to `DirectChat`, not silently become a
    // `Command` for a command the adapter doesn't know about.
    let payload = br#"{
            "update_id": 111,
            "message": {
                "message_id": 12,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/nope",
                "entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    match envelope.payload {
        ProductInboundPayload::UserMessage(user) => {
            assert_eq!(user.trigger, ProductTriggerReason::DirectChat);
        }
        other => panic!("expected UserMessage with DirectChat trigger, got {other:?}"),
    }
}

#[test]
fn command_arguments_with_control_char_rejected_via_shared_validation() {
    // Henry's review (PR #3354, 2026-05-12T18:59:39Z) — Critical:
    // `build_payload` previously constructed `InboundCommandPayload`
    // with a struct literal, bypassing `InboundCommandPayload::new`
    // and the shared `ironclaw_assistant` validation
    // (token shape, byte limits, control-char rejection). Untrusted
    // Telegram webhook text could carry control characters into
    // the trusted inbound envelope.
    //
    // Asserts the validation now fires: a `/help` with a U+0001
    // control character in the argument text must be rejected with
    // `InvalidExternalRef { kind: "inbound_command_payload" }`,
    // mirroring how the user-message arm reports its own
    // validation failures.
    //
    // The control char is embedded via JSON's `` escape so
    // the raw bytes the JSON parser produces include a literal
    // control character.
    let payload = br#"{
            "update_id": 250,
            "message": {
                "message_id": 16,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/help \u0001oops",
                "entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
    let err = parse_telegram_update(payload, &evidence(), &install_id(), &policy())
        .expect_err("control-character arguments must be rejected");
    match err {
        PayloadParseError::InvalidExternalRef { kind, reason } => {
            assert_eq!(kind, "inbound_command_payload");
            // `MalformedInboundPayload` carries a `RedactedString`,
            // so its Display surface is the redaction marker, not
            // the raw failure detail (security contract). Asserting
            // on `<redacted>` proves the shared validator was
            // reached AND its redaction is intact — a regression
            // that leaked the control-char-bearing content into
            // the error message would fail this assertion.
            assert!(
                reason.contains("<redacted>"),
                "rejection reason must be redacted (control-char content must not leak); got {reason}",
            );
        }
        other => {
            panic!("expected InvalidExternalRef{{kind:inbound_command_payload}}, got {other:?}")
        }
    }
}

#[test]
fn command_arguments_exceeding_byte_limit_rejected_via_shared_validation() {
    // Defense-in-depth for the same fix: synthesize a command with
    // arguments larger than `COMMAND_ARGUMENTS_MAX_BYTES` (64 KiB
    // per `ironclaw_product_contracts::inbound`) and assert the
    // shared validator rejects it through `InboundCommandPayload::new`.
    // 70_000 bytes is comfortably over the 64 * 1024 = 65_536 limit.
    let oversized = "a".repeat(70_000);
    let payload = format!(
        r#"{{
                "update_id": 251,
                "message": {{
                    "message_id": 17,
                    "date": 1700000000,
                    "from": {{"id": 777, "is_bot": false, "first_name": "Alice"}},
                    "chat": {{"id": 777, "type": "private"}},
                    "text": "/help {oversized}",
                    "entities": [{{"type": "bot_command", "offset": 0, "length": 5}}]
                }}
            }}"#
    );
    let err = parse_telegram_update(payload.as_bytes(), &evidence(), &install_id(), &policy())
        .expect_err("oversized arguments must be rejected");
    match err {
        PayloadParseError::InvalidExternalRef { kind, reason } => {
            assert_eq!(kind, "inbound_command_payload");
            // Same redaction contract as the control-char test
            // above. The 70_000-byte payload must not leak into
            // the error message.
            assert!(
                reason.contains("<redacted>"),
                "rejection reason must be redacted (oversized content must not leak); got {reason}",
            );
        }
        other => {
            panic!("expected InvalidExternalRef{{kind:inbound_command_payload}}, got {other:?}")
        }
    }
}

#[test]
fn group_media_caption_mention_is_recognized_as_bot_mention() {
    // Copilot's payload.rs:222 finding: trigger detection previously
    // consulted only `text + entities`. A photo with caption
    // `@ironclaw_bot help` carries its mention in
    // `caption_entities`, so `classify_trigger` returned None and
    // the update was silently NoOp'd. The fix consults both text-
    // and caption-anchored entity lists.
    let payload = br#"{
            "update_id": 230,
            "message": {
                "message_id": 13,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "photo": [
                    {"file_id": "AAAA", "file_unique_id": "u1", "width": 100, "height": 100, "file_size": 500}
                ],
                "caption": "@ironclaw_bot please look",
                "caption_entities": [{"type": "mention", "offset": 0, "length": 13}]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    match envelope.payload {
        ProductInboundPayload::UserMessage(user) => {
            assert_eq!(user.trigger, ProductTriggerReason::BotMention);
        }
        other => panic!("expected UserMessage with BotMention trigger, got {other:?}"),
    }
}

#[test]
fn group_media_caption_bot_command_emits_command_payload() {
    // Caption-anchored `/help` must reach the Command path too.
    let payload = br#"{
            "update_id": 231,
            "message": {
                "message_id": 14,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "photo": [
                    {"file_id": "BBBB", "file_unique_id": "u2", "width": 100, "height": 100, "file_size": 500}
                ],
                "caption": "/help on this photo",
                "caption_entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    match envelope.payload {
        ProductInboundPayload::Command(cmd) => {
            assert_eq!(cmd.command, "help");
            assert_eq!(cmd.trigger, ProductTriggerReason::BotCommand);
        }
        other => panic!("expected Command, got {other:?}"),
    }
}

#[test]
fn message_without_from_classifies_as_noop_not_error() {
    // Copilot's payload.rs:419 finding: `message.from` is optional
    // in the Telegram schema (anonymous group admins, channel
    // posts that slipped through the kind filter, etc.). Returning
    // a hard `PayloadParseError` would force the webhook to retry
    // an otherwise-parseable update. The fail-soft path is `NoOp`
    // — the webhook acks 200 OK and Telegram does not retry.
    let payload = br#"{
            "update_id": 240,
            "message": {
                "message_id": 15,
                "date": 1700000000,
                "chat": {"id": -42, "type": "supergroup"},
                "text": "anonymous admin message"
            }
        }"#;
    let parsed = parse_telegram_update(payload, &evidence(), &install_id(), &policy())
        .expect("parse must not hard-error on missing `from`");
    assert!(
        matches!(parsed.payload, ProductInboundPayload::NoOp),
        "missing `from` must fail-soft to NoOp, got {parsed:?}"
    );
}

#[test]
fn private_chat_message_creates_envelope() {
    let payload = br#"{
            "update_id": 100,
            "message": {
                "message_id": 11,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "hello"
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    assert_eq!(envelope.external_event_id.as_str(), "tg-install_alpha-100");
    assert_eq!(envelope.external_actor_ref.id(), "777");
    assert_eq!(envelope.external_conversation_ref.conversation_id(), "777");
    assert_eq!(
        envelope.external_conversation_ref.reply_target_message_id(),
        Some("11")
    );
    match envelope.payload {
        ProductInboundPayload::UserMessage(user) => {
            assert_eq!(user.text, "hello");
            assert_eq!(user.trigger, ProductTriggerReason::DirectChat);
        }
        other => panic!("expected UserMessage, got {other:?}"),
    }
}

#[test]
fn group_ambient_message_is_noop() {
    let payload = br#"{
            "update_id": 200,
            "message": {
                "message_id": 12,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "just chatting"
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    assert!(matches!(parsed.payload, ProductInboundPayload::NoOp));
}

#[test]
fn group_explicit_mention_creates_envelope() {
    let payload = br#"{
            "update_id": 201,
            "message": {
                "message_id": 12,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "@ironclaw_bot please help",
                "entities": [{"type": "mention", "offset": 0, "length": 13}]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    match envelope.payload {
        ProductInboundPayload::UserMessage(user) => {
            assert_eq!(user.trigger, ProductTriggerReason::BotMention);
            assert_eq!(user.text, "please help");
        }
        other => panic!("expected UserMessage, got {other:?}"),
    }
}

#[test]
fn group_reply_to_bot_creates_envelope() {
    let payload = br#"{
            "update_id": 202,
            "message": {
                "message_id": 13,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "thanks",
                "reply_to_message": {
                    "message_id": 7,
                    "date": 1699999999,
                    "from": {"id": 9000, "is_bot": true, "first_name": "IronClaw"},
                    "chat": {"id": -42, "type": "supergroup"},
                    "text": "hi there"
                }
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    match envelope.payload {
        ProductInboundPayload::UserMessage(user) => {
            assert_eq!(user.trigger, ProductTriggerReason::ReplyToBot);
        }
        other => panic!("expected UserMessage, got {other:?}"),
    }
}

#[test]
fn unset_bot_user_id_does_not_match_a_group_reply() {
    let payload = br#"{
            "update_id": 206,
            "message": {
                "message_id": 17,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "ambient reply",
                "reply_to_message": {
                    "message_id": 7,
                    "date": 1699999999,
                    "from": {"id": 0, "is_bot": true, "first_name": "Unknown"},
                    "chat": {"id": -42, "type": "supergroup"},
                    "text": "hi there"
                }
            }
        }"#;
    let mut policy = policy();
    policy.bot_user_id = 0;
    assert!(matches!(
        normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes"),
        TelegramInboundEvent::Ignore
    ));
}

#[test]
fn group_recognized_command_creates_command_envelope() {
    let payload = br#"{
            "update_id": 203,
            "message": {
                "message_id": 14,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "/help@ironclaw_bot args here",
                "entities": [{"type": "bot_command", "offset": 0, "length": 18}]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    match envelope.payload {
        ProductInboundPayload::Command(cmd) => {
            assert_eq!(cmd.command, "help");
            assert_eq!(cmd.arguments, "args here");
            assert_eq!(cmd.trigger, ProductTriggerReason::BotCommand);
        }
        other => panic!("expected Command, got {other:?}"),
    }
}

#[test]
fn unknown_command_in_group_is_noop() {
    // /yolo isn't in the recognized list and there's no mention/reply.
    let payload = br#"{
            "update_id": 204,
            "message": {
                "message_id": 15,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "/yolo",
                "entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    assert!(matches!(parsed.payload, ProductInboundPayload::NoOp));
}

#[test]
fn topic_message_keys_conversation_by_topic_not_message_id() {
    let payload = br#"{
            "update_id": 300,
            "message": {
                "message_id": 50,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "message_thread_id": 7,
                "text": "@ironclaw_bot hello",
                "entities": [{"type": "mention", "offset": 0, "length": 13}]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    assert_eq!(
        envelope.external_conversation_ref.topic_id(),
        Some("7"),
        "topic must be carried in conversation key"
    );
    assert_eq!(
        envelope.external_conversation_ref.reply_target_message_id(),
        Some("50"),
        "reply target must come from message_id"
    );
    // Same chat, different message_id, same topic -> identical fingerprint.
    let payload2 = br#"{
            "update_id": 301,
            "message": {
                "message_id": 51,
                "date": 1700000001,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "message_thread_id": 7,
                "text": "@ironclaw_bot more",
                "entities": [{"type": "mention", "offset": 0, "length": 13}]
            }
        }"#;
    let parsed2 =
        parse_telegram_update(payload2, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope2 = parsed2;
    assert_eq!(
        envelope
            .external_conversation_ref
            .conversation_fingerprint(),
        envelope2
            .external_conversation_ref
            .conversation_fingerprint(),
    );
    // Reply targets differ.
    assert_ne!(
        envelope.external_conversation_ref.reply_target_message_id(),
        envelope2
            .external_conversation_ref
            .reply_target_message_id(),
    );
}

#[test]
fn private_chat_with_photo_emits_attachment_descriptor_no_bytes() {
    let payload = br#"{
            "update_id": 400,
            "message": {
                "message_id": 22,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "caption": "look",
                "photo": [
                    {"file_id": "AAAA", "file_size": 1024},
                    {"file_id": "BBBB", "file_size": 8192}
                ]
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    let envelope = parsed;
    match envelope.payload {
        ProductInboundPayload::UserMessage(user) => {
            assert_eq!(user.attachments.len(), 1);
            assert_eq!(user.attachments[0].external_file_id, "BBBB");
            assert_eq!(user.attachments[0].kind, ProductAttachmentKind::Image);
            let json = serde_json::to_value(&user.attachments[0]).expect("serialize");
            assert!(json.get("data").is_none());
            assert!(json.get("source_url").is_none());
        }
        other => panic!("expected UserMessage, got {other:?}"),
    }
}

#[test]
fn media_group_fragments_share_one_durable_event_identity() {
    let first = br#"{
            "update_id": 701,
            "message": {
                "message_id": 81,
                "media_group_id": "album-6364",
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "document": {
                    "file_id": "file-alpha",
                    "file_name": "alpha.txt",
                    "mime_type": "text/plain",
                    "file_size": 5
                }
            }
        }"#;
    let second = br#"{
            "update_id": 702,
            "message": {
                "message_id": 82,
                "media_group_id": "album-6364",
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "caption": "read both",
                "document": {
                    "file_id": "file-beta",
                    "file_name": "beta.txt",
                    "mime_type": "text/plain",
                    "file_size": 4
                }
            }
        }"#;
    let TelegramInboundEvent::BatchFragment(first) =
        normalize_telegram_update(first, &install_id(), &policy()).expect("first normalizes")
    else {
        panic!("first media-group fragment must normalize");
    };
    let TelegramInboundEvent::BatchFragment(second) =
        normalize_telegram_update(second, &install_id(), &policy()).expect("second normalizes")
    else {
        panic!("second media-group fragment must normalize");
    };

    assert_eq!(
        first.message.event_id, second.message.event_id,
        "all media-group fragments must converge on one durable event"
    );
    assert_ne!(first.fragment_id, second.fragment_id);
}

#[test]
fn media_group_identity_is_scoped_to_chat_and_thread() {
    let payload = |update_id: i64, chat_id: i64, thread_id: Option<i64>| {
        let mut message = serde_json::json!({
            "message_id": update_id,
            "media_group_id": "provider-reused-id",
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": chat_id, "type": "private"},
            "document": {
                "file_id": format!("file-{update_id}"),
                "file_name": "report.txt",
                "mime_type": "text/plain",
                "file_size": 5
            }
        });
        if let Some(thread_id) = thread_id {
            message["message_thread_id"] = serde_json::json!(thread_id);
        }
        serde_json::to_vec(&serde_json::json!({
            "update_id": update_id,
            "message": message,
        }))
        .expect("payload serializes")
    };
    let normalize = |payload: Vec<u8>| {
        let TelegramInboundEvent::BatchFragment(fragment) =
            normalize_telegram_update(&payload, &install_id(), &policy())
                .expect("media group normalizes")
        else {
            panic!("expected media-group fragment");
        };
        fragment
    };

    let first = normalize(payload(801, 777, None));
    let other_chat = normalize(payload(802, 778, None));
    let other_thread = normalize(payload(803, 777, Some(42)));

    assert_ne!(first.batch_key, other_chat.batch_key);
    assert_ne!(first.message.event_id, other_chat.message.event_id);
    assert_ne!(first.batch_key, other_thread.batch_key);
    assert_ne!(first.message.event_id, other_thread.message.event_id);
}

#[test]
fn channel_post_is_noop() {
    let payload = br#"{
            "update_id": 500,
            "channel_post": {
                "message_id": 1,
                "date": 1700000000,
                "chat": {"id": -1001, "type": "channel"},
                "text": "broadcast"
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    assert!(matches!(parsed.payload, ProductInboundPayload::NoOp));
}

#[test]
fn edited_message_is_noop() {
    let payload = br#"{
            "update_id": 600,
            "edited_message": {
                "message_id": 1,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false},
                "chat": {"id": 777, "type": "private"},
                "text": "edited"
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
    assert!(matches!(parsed.payload, ProductInboundPayload::NoOp));
}

#[test]
fn malformed_json_is_invalid_json_error() {
    let payload = b"this is not json";
    let err = parse_telegram_update(payload, &evidence(), &install_id(), &policy())
        .expect_err("malformed");
    assert!(matches!(err, PayloadParseError::InvalidJson { .. }));
}

/// Ben's regression (2026-07-17): the shared busy-on-auth hint tells the
/// user to reply `auth deny gate:<ref>` in this chat, but Telegram's
/// parse treated that reply as a plain `UserMessage` — it bounced off
/// the busy thread with the same hint, forever. The advertised
/// interaction grammar (shared with Slack via
/// `ironclaw_product_contracts::interaction_commands`) must parse here.
#[test]
fn dm_auth_deny_command_parses_to_auth_resolution_not_user_message() {
    let payload = br#"{
            "update_id": 300,
            "message": {
                "message_id": 30,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "auth deny gate:auth-abc123"
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parses");
    match parsed.payload {
        ironclaw_product_contracts::inbound::ProductInboundPayload::AuthResolution(resolution) => {
            assert_eq!(resolution.auth_request_ref, "gate:auth-abc123");
        }
        other => panic!("expected AuthResolution, got {other:?}"),
    }
}

/// The hint renders the command in backticks; Telegram clients copy them.
#[test]
fn dm_backticked_approve_command_parses_to_approval_resolution() {
    let payload = br#"{
            "update_id": 301,
            "message": {
                "message_id": 31,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "`approve gate:approval-9`"
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parses");
    assert!(
        matches!(
            parsed.payload,
            ironclaw_product_contracts::inbound::ProductInboundPayload::ApprovalResolution(_)
        ),
        "got {:?}",
        parsed.payload
    );
}

/// Guard: ordinary conversation that merely starts with a verb-like word
/// still routes as a user message.
#[test]
fn dm_ordinary_text_still_routes_as_user_message() {
    let payload = br#"{
            "update_id": 302,
            "message": {
                "message_id": 32,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "hello there, what can you do?"
            }
        }"#;
    let parsed =
        parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parses");
    assert!(
        matches!(
            parsed.payload,
            ironclaw_product_contracts::inbound::ProductInboundPayload::UserMessage(_)
        ),
        "got {:?}",
        parsed.payload
    );
}
