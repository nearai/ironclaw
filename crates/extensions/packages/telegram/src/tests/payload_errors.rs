//! Fail-closed rejection paths of the Telegram payload normalizer.
//!
//! `payload.rs` is an untrusted-input boundary: everything below the
//! webhook signature check is attacker-authored. The sibling
//! `tests/payload.rs` pins the forwarding behaviour; this file pins the
//! other half — what a malformed, hostile, or simply unrecognized update
//! must do. Two outcomes are in scope and they are deliberately
//! different:
//!
//! * **Ignore / NoOp** — the update is well-formed but not for us
//!   (ambient chatter, a channel post, a mention of another bot). The
//!   webhook still acks 200 so Telegram does not retry forever.
//! * **`PayloadParseError`** — the update carries a value that cannot be
//!   normalized without violating a shared contract bound (oversize ids,
//!   control characters, a negative batch order). These must surface as
//!   errors, never as a silently degraded message.

use super::*;
use ironclaw_host_api::product_adapter::auth::AuthRequirement;

fn evidence() -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".to_string(),
        },
        "telegram_install_alpha",
    )
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

fn parse(payload: &[u8]) -> Result<ParsedProductInbound, PayloadParseError> {
    parse_telegram_update(payload, &evidence(), &install_id(), &policy())
}

fn normalize(payload: &[u8]) -> Result<TelegramInboundEvent, PayloadParseError> {
    normalize_telegram_update(payload, &install_id(), &policy())
}

/// `{"update_id": …, "message": …}` around a message body.
fn update_with(update_id: i64, message: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "update_id": update_id,
        "message": message,
    }))
    .expect("payload serializes")
}

fn expect_invalid_ref(error: PayloadParseError, expected_kind: &str) -> String {
    match error {
        PayloadParseError::InvalidExternalRef { kind, reason } => {
            assert_eq!(kind, expected_kind, "unexpected rejection kind ({reason})");
            reason
        }
        other => panic!("expected InvalidExternalRef{{kind:{expected_kind}}}, got {other:?}"),
    }
}

/// Both entry points must agree that an update is authenticated-but-inert:
/// `parse_telegram_update` yields the explicit `NoOp` payload variant (never
/// an out-of-band `None`) and `normalize_telegram_update` yields `Ignore`.
fn assert_ignored(payload: &[u8], context: &str) {
    let parsed = parse(payload).expect("an authenticated update must not hard-error");
    assert!(
        matches!(parsed.payload, ProductInboundPayload::NoOp),
        "{context}: expected NoOp, got {:?}",
        parsed.payload
    );
    assert!(
        matches!(
            normalize(payload).expect("normalizes"),
            TelegramInboundEvent::Ignore
        ),
        "{context}: normalize must agree with parse on the ignore decision",
    );
}

/// The single `UserMessage` payload an update normalized to, or a panic.
fn expect_user_message(payload: &[u8]) -> UserMessagePayload {
    match parse(payload).expect("parses").payload {
        ProductInboundPayload::UserMessage(user) => user,
        other => panic!("expected UserMessage, got {other:?}"),
    }
}

#[test]
fn update_without_an_update_id_is_rejected_by_both_entry_points() {
    // `update_id` is `#[serde(default)]`, so a body that omits it
    // deserializes to `0` instead of failing — and `0` is not a Telegram
    // update id. Accepting it would mint `tg-<install>-0` as the durable
    // event identity for *every* such body, collapsing them all onto one
    // event. Both entry points reject instead.
    let payload = br#"{
            "message": {
                "message_id": 11,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "hello"
            }
        }"#;
    assert!(matches!(
        parse(payload),
        Err(PayloadParseError::MissingUpdateId)
    ));
    assert!(matches!(
        normalize(payload),
        Err(PayloadParseError::MissingUpdateId)
    ));
}

#[test]
fn chat_kind_classification_is_fail_closed_for_unknown_vendor_kinds() {
    // `chat.type` is an untrusted vendor string that Telegram extends over
    // time (`sender`, business-account kinds, …). Anything unrecognized
    // must land in `Other`, and every kind except `private` must demand an
    // explicit trigger — a new Telegram chat kind must never start
    // forwarding ambient traffic by default.
    assert_eq!(
        TelegramChatKind::from_str("private"),
        TelegramChatKind::Private
    );
    assert_eq!(TelegramChatKind::from_str("group"), TelegramChatKind::Group);
    assert_eq!(
        TelegramChatKind::from_str("supergroup"),
        TelegramChatKind::Supergroup
    );
    assert_eq!(
        TelegramChatKind::from_str("channel"),
        TelegramChatKind::Channel
    );
    for unknown in ["sender", "PRIVATE", "", "business_account"] {
        assert_eq!(
            TelegramChatKind::from_str(unknown),
            TelegramChatKind::Other,
            "unrecognized chat kind {unknown:?} must fall into Other"
        );
    }

    assert!(!TelegramChatKind::Private.requires_explicit_trigger());
    for kind in [
        TelegramChatKind::Group,
        TelegramChatKind::Supergroup,
        TelegramChatKind::Channel,
        TelegramChatKind::Other,
    ] {
        assert!(
            kind.requires_explicit_trigger(),
            "{kind:?} must require an explicit trigger"
        );
    }
}

#[test]
fn channel_typed_message_is_ignored_even_when_it_names_the_bot() {
    // A `channel` chat reached through the `message` slot (rather than
    // `channel_post`) is still a broadcast surface: unsigned,
    // non-interactive. It is dropped *before* the mention/command checks,
    // so even an explicit `@ironclaw_bot` in a channel post stays inert.
    let payload = br#"{
            "update_id": 910,
            "message": {
                "message_id": 91,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -1001, "type": "channel"},
                "text": "@ironclaw_bot broadcast",
                "entities": [{"type": "mention", "offset": 0, "length": 13}]
            }
        }"#;
    assert_ignored(payload, "channel-typed message");
}

#[test]
fn unrecognized_chat_kind_message_is_ignored_without_an_explicit_trigger() {
    // The `Other` arm of the chat-kind table, driven end to end: an
    // unknown `chat.type` behaves like a group — ambient text is dropped.
    let payload = br#"{
            "update_id": 911,
            "message": {
                "message_id": 92,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -4242, "type": "sender"},
                "text": "ambient chatter in a chat kind we do not know"
            }
        }"#;
    assert_ignored(payload, "unrecognized chat kind");
}

#[test]
fn mention_entities_that_do_not_resolve_to_the_bot_are_not_triggers() {
    // Two shapes that must both leave a group message ambient. The first
    // is the untrusted-offset case: entity offsets are UTF-16 indices the
    // sender controls, so a window that runs past the text must make the
    // entity be *skipped*, never guessed at. The second is the ordinary
    // "someone mentioned a different bot" case.
    let out_of_range = br#"{
            "update_id": 912,
            "message": {
                "message_id": 93,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "hi there",
                "entities": [{"type": "mention", "offset": 50, "length": 5}]
            }
        }"#;
    assert_ignored(out_of_range, "out-of-range mention entity");

    let other_bot = br#"{
            "update_id": 913,
            "message": {
                "message_id": 94,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "@other_bot are you there",
                "entities": [{"type": "mention", "offset": 0, "length": 10}]
            }
        }"#;
    assert_ignored(other_bot, "mention of a different bot");
}

#[test]
fn reply_to_a_message_without_a_sender_is_not_a_bot_reply() {
    // Telegram omits `from` on anonymous-admin and forwarded-channel
    // messages. Treating a reply to one as a reply to the bot would
    // forward arbitrary group chatter, so the sender-less reply target
    // must fail the check rather than default to "probably us".
    let payload = br#"{
            "update_id": 914,
            "message": {
                "message_id": 95,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "thanks",
                "reply_to_message": {
                    "message_id": 7,
                    "date": 1699999999,
                    "chat": {"id": -42, "type": "supergroup"},
                    "text": "posted by an anonymous admin"
                }
            }
        }"#;
    assert_ignored(payload, "reply to a message without `from`");
}

#[test]
fn media_group_fragment_with_a_negative_message_id_is_rejected() {
    // `message_id` is the batch fragment's ordering key and the contract
    // types it `u64`. Telegram never sends a negative one; a body that
    // does must fail the update closed rather than be coerced into an
    // ordering that silently reshuffles an album.
    let payload = update_with(
        915,
        serde_json::json!({
            "message_id": -5,
            "media_group_id": "album-negative",
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"},
            "document": {
                "file_id": "file-alpha",
                "file_name": "alpha.txt",
                "mime_type": "text/plain",
                "file_size": 5
            }
        }),
    );
    let error = normalize(&payload).expect_err("a negative batch order must be rejected");
    expect_invalid_ref(error, "telegram_media_group_order");
}

#[test]
fn media_group_id_that_breaks_the_event_id_bound_is_rejected() {
    // `media_group_id` is attacker-authored and flows straight into the
    // durable event identity (`tg-<install>-media-chat-…-group-<id>`).
    // Both shared-contract bounds on an `ExternalEventId` — the 512-byte
    // ceiling and the control-character ban — must fail the update closed
    // instead of minting a malformed identity that later storage layers
    // would have to cope with.
    for media_group_id in ["a".repeat(600), "album\u{1}injected".to_string()] {
        let payload = update_with(
            916,
            serde_json::json!({
                "message_id": 96,
                "media_group_id": media_group_id,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "document": {
                    "file_id": "file-alpha",
                    "file_name": "alpha.txt",
                    "mime_type": "text/plain",
                    "file_size": 5
                }
            }),
        );
        let error = normalize(&payload).expect_err("a hostile media_group_id must be rejected");
        expect_invalid_ref(error, "external_event_id");
    }
}

#[test]
fn hostile_sender_display_names_are_rejected_as_actor_refs() {
    // `username` / `first_name` are attacker-authored and become the actor
    // ref's display name, which crosses into the trusted inbound envelope.
    // An oversize name and a control-character name must both be rejected
    // by the shared validator rather than truncated or sanitized here —
    // and both entry points must reject identically.
    let cases = [
        ("oversize username", serde_json::json!("u".repeat(600))),
        ("control-char username", serde_json::json!("Al\u{1}ice")),
    ];
    for (label, username) in cases {
        let payload = update_with(
            917,
            serde_json::json!({
                "message_id": 97,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "username": username},
                "chat": {"id": 777, "type": "private"},
                "text": "hello"
            }),
        );
        let Err(error) = parse(&payload) else {
            panic!("{label}: parse must reject a hostile display name");
        };
        expect_invalid_ref(error, "external_actor_ref");
        let Err(error) = normalize(&payload) else {
            panic!("{label}: normalize must reject a hostile display name");
        };
        expect_invalid_ref(error, "external_actor_ref");
    }
}

#[test]
fn empty_gate_reference_in_an_interaction_command_is_rejected() {
    // The shared busy/prompt copy advertises `approve gate:<ref>` in this
    // chat. A reply that keeps the authority-bearing prefix but drops the
    // reference is malformed authority input, not conversation: the shared
    // grammar rejects it and the adapter must surface that rejection
    // instead of quietly demoting it to a user turn (which is how the
    // phantom-affordance loop started).
    let payload = br#"{
            "update_id": 918,
            "message": {
                "message_id": 98,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "approve gate:"
            }
        }"#;
    let error = parse(payload).expect_err("an empty gate reference must be rejected");
    let reason = expect_invalid_ref(error, "interaction_resolution_payload");
    assert!(
        reason.contains("<redacted>"),
        "the rejection must stay redacted; got {reason}"
    );
}

#[test]
fn user_message_text_that_fails_shared_validation_is_rejected() {
    // The user-message arm routes through `UserMessagePayload::new` so the
    // shared `ironclaw_product` validation fires on untrusted Telegram
    // text. A struct literal here would let control characters and
    // oversize bodies cross into the trusted envelope. Both bounds are
    // pinned: a control character (newline and tab stay legal) and a body
    // past the 64 KiB text ceiling.
    let cases = [
        ("control character", "hello\u{1}world".to_string()),
        ("oversize body", "a".repeat(70_000)),
    ];
    for (label, text) in cases {
        let payload = update_with(
            919,
            serde_json::json!({
                "message_id": 99,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": text
            }),
        );
        let Err(error) = parse(&payload) else {
            panic!("{label}: parse must reject text that fails shared validation");
        };
        let reason = expect_invalid_ref(error, "user_message_payload");
        assert!(
            reason.contains("<redacted>"),
            "{label}: rejection must stay redacted, got {reason}"
        );
    }

    // A newline-bearing body is the control case: it must still parse.
    let multiline = update_with(
        920,
        serde_json::json!({
            "message_id": 100,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"},
            "text": "line one\nline two\tindented"
        }),
    );
    assert_eq!(
        expect_user_message(&multiline).text,
        "line one\nline two\tindented"
    );
}

#[test]
fn bot_command_entity_out_of_range_falls_back_to_plain_text() {
    // A `bot_command` entity whose UTF-16 window overruns the text cannot
    // be sliced. The command must be skipped — not reconstructed from the
    // raw text — and the message must route as ordinary text, because
    // promoting an unverifiable entity into a `Command` payload would let
    // a sender forge a command token.
    let payload = br#"{
            "update_id": 921,
            "message": {
                "message_id": 101,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/help",
                "entities": [{"type": "bot_command", "offset": 0, "length": 99}]
            }
        }"#;
    let user = expect_user_message(payload);
    assert_eq!(user.text, "/help");
    assert_eq!(user.trigger, ProductTriggerReason::DirectChat);

    let TelegramInboundEvent::Message(message) = normalize(payload).expect("normalizes") else {
        panic!("a private-chat message must be forwarded");
    };
    assert_eq!(
        message.text, "/help",
        "an unsliceable command entity must not be canonicalized"
    );
}

#[test]
fn a_command_is_only_leading_when_a_real_bot_mention_precedes_it() {
    // `bot_command_is_leading` is the gate that stops a mid-sentence
    // `/command` from being promoted to a `Command` payload. A command at
    // a non-zero offset is only leading when a *verified* mention of this
    // bot sits at offset 0 and only whitespace separates them. Three
    // malformed prefixes must all fail it.

    // 1. The leading mention's window overruns the text, so the mention
    //    cannot be verified — neither the mention trigger nor the command
    //    promotion may fire.
    let unverifiable_mention = br#"{
            "update_id": 922,
            "message": {
                "message_id": 102,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "@ironclaw_bot /help",
                "entities": [
                    {"type": "mention", "offset": 0, "length": 99},
                    {"type": "bot_command", "offset": 14, "length": 5}
                ]
            }
        }"#;
    assert_ignored(unverifiable_mention, "unverifiable leading mention");

    // 2. The leading mention names a different bot. `/help` belongs to
    //    that bot's conversation, not ours.
    let other_bots_mention = br#"{
            "update_id": 923,
            "message": {
                "message_id": 103,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "@other_bot /help",
                "entities": [
                    {"type": "mention", "offset": 0, "length": 10},
                    {"type": "bot_command", "offset": 11, "length": 5}
                ]
            }
        }"#;
    assert_ignored(other_bots_mention, "leading mention of a different bot");

    // 3. The command entity *overlaps* the mention instead of following
    //    it (offset 5 sits inside the 13-unit mention). There is no
    //    whitespace-only gap to check, so the promotion must fail and the
    //    message stays an ordinary mention-triggered user message.
    let overlapping_command = br#"{
            "update_id": 924,
            "message": {
                "message_id": 104,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "@ironclaw_bot help me",
                "entities": [
                    {"type": "mention", "offset": 0, "length": 13},
                    {"type": "bot_command", "offset": 5, "length": 5}
                ]
            }
        }"#;
    let user = expect_user_message(overlapping_command);
    assert_eq!(user.trigger, ProductTriggerReason::BotMention);
    assert_eq!(
        user.text, "help me",
        "an overlapping command entity must not become a Command payload"
    );
}

#[test]
fn every_attachment_slot_fails_the_update_closed_on_an_invalid_descriptor() {
    // `collect_attachments` walks photo -> document -> voice -> audio ->
    // video -> sticker, and every slot's `file_id` is attacker-authored.
    // A slot that cannot produce a valid descriptor must abort the whole
    // update: dropping the attachment and forwarding the message would
    // hand the agent a message that silently lost its media.
    let slots: [(&str, serde_json::Value); 6] = [
        (
            "photo",
            serde_json::json!([{"file_id": "", "file_size": 10}]),
        ),
        (
            "document",
            serde_json::json!({"file_id": "", "file_name": "a.txt", "mime_type": "text/plain"}),
        ),
        (
            "voice",
            serde_json::json!({"file_id": "", "mime_type": "audio/ogg"}),
        ),
        (
            "audio",
            serde_json::json!({"file_id": "", "mime_type": "audio/mpeg"}),
        ),
        (
            "video",
            serde_json::json!({"file_id": "", "mime_type": "video/mp4"}),
        ),
        ("sticker", serde_json::json!({"file_id": ""})),
    ];
    for (slot, value) in slots {
        let mut message = serde_json::json!({
            "message_id": 105,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"}
        });
        message[slot] = value;
        let payload = update_with(925, message);

        let error = parse(&payload).expect_err("an empty file id must be rejected");
        let reason = expect_invalid_ref(error, "attachment_descriptor");
        assert!(
            reason.contains("must not be empty"),
            "{slot}: expected the empty-file-id rejection, got {reason}"
        );
        let error = normalize(&payload).expect_err("an empty file id must be rejected");
        expect_invalid_ref(error, "attachment_descriptor");
    }
}

#[test]
fn voice_and_sticker_slots_are_rejected_by_the_shared_kind_or_mime_rule() {
    // Contract pin, not an endorsement: the shared descriptor validator
    // requires `kind` to match the MIME base type (`audio/*` -> Audio,
    // `image/*` -> Image) unless the kind is `Other`. The voice slot pairs
    // `audio/ogg` with `Voice` and the sticker slot pairs `image/webp`
    // with `Sticker`, so both are rejected today even for a perfectly
    // well-formed Telegram body. This test records that behaviour so a
    // change to either side is a deliberate, visible one.
    let voice = update_with(
        926,
        serde_json::json!({
            "message_id": 106,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"},
            "voice": {"file_id": "voice-1", "mime_type": "audio/ogg", "file_size": 2048}
        }),
    );
    let reason = expect_invalid_ref(
        parse(&voice).expect_err("voice descriptor is rejected"),
        "attachment_descriptor",
    );
    assert!(
        reason.contains("MIME"),
        "expected the kind/MIME mismatch rejection, got {reason}"
    );

    let sticker = update_with(
        927,
        serde_json::json!({
            "message_id": 107,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"},
            "sticker": {"file_id": "sticker-1", "file_size": 512}
        }),
    );
    let reason = expect_invalid_ref(
        parse(&sticker).expect_err("sticker descriptor is rejected"),
        "attachment_descriptor",
    );
    assert!(
        reason.contains("MIME"),
        "expected the kind/MIME mismatch rejection, got {reason}"
    );
}

#[test]
fn audio_and_video_slots_normalize_into_descriptors() {
    // The happy path for the two slots that do satisfy the shared
    // kind/MIME rule, collected from one message so the ordering
    // (audio before video) is pinned alongside the per-slot mapping of
    // `file_name` / `file_size` / default MIME.
    let payload = update_with(
        928,
        serde_json::json!({
            "message_id": 108,
            "date": 1700000000,
            "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
            "chat": {"id": 777, "type": "private"},
            "caption": "two files",
            "audio": {"file_id": "audio-1", "file_name": "song.mp3", "file_size": 4096},
            "video": {"file_id": "video-1", "file_size": 8192}
        }),
    );
    let user = expect_user_message(&payload);
    assert_eq!(user.text, "two files");
    assert_eq!(user.attachments.len(), 2);
    assert_eq!(user.attachments[0].external_file_id, "audio-1");
    assert_eq!(user.attachments[0].kind, ProductAttachmentKind::Audio);
    assert_eq!(
        user.attachments[0].mime_type, "audio/mpeg",
        "a missing audio MIME falls back to the Telegram default"
    );
    assert_eq!(user.attachments[0].filename.as_deref(), Some("song.mp3"));
    assert_eq!(user.attachments[1].external_file_id, "video-1");
    assert_eq!(user.attachments[1].kind, ProductAttachmentKind::Video);
    assert_eq!(user.attachments[1].mime_type, "video/mp4");
    assert_eq!(user.attachments[1].size_bytes, Some(8192));
}

#[test]
fn an_empty_photo_array_yields_a_message_with_no_attachment() {
    // Telegram always sends at least one `PhotoSize`, so an empty `photo`
    // array is malformed. It must degrade to "no attachment" rather than
    // panicking on an empty-vec pick or failing the whole update — the
    // caption is still a real user message.
    let payload = br#"{
            "update_id": 929,
            "message": {
                "message_id": 109,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "caption": "look at this",
                "photo": []
            }
        }"#;
    let user = expect_user_message(payload);
    assert_eq!(user.text, "look at this");
    assert!(
        user.attachments.is_empty(),
        "an empty photo array must yield no descriptor, got {:?}",
        user.attachments
    );
}

#[test]
fn adapter_errors_map_to_invalid_json_preserving_the_host_redacted_text() {
    // `adapter_error_to_payload_error` is the single funnel from
    // `ProductAdapterError` into `PayloadParseError`. Every call site that
    // guards a `ParsedProductInbound::new` is infallible today, so this
    // pins the mapping directly: it must keep the already-host-redacted
    // Display text and must not swap in a different variant (which would
    // change how ingress classifies the failure).
    let mapped = adapter_error_to_payload_error(ProductAdapterError::InvalidIdentifier {
        kind: "external_actor_id",
        reason: "must not be empty".into(),
    });
    match mapped {
        PayloadParseError::InvalidJson { reason } => {
            assert!(
                reason.contains("external_actor_id") && reason.contains("must not be empty"),
                "the renderable adapter message must survive the mapping, got {reason}"
            );
        }
        other => panic!("expected InvalidJson, got {other:?}"),
    }
}
