use super::*;
use proptest::prelude::*;

fn installation_id() -> AdapterInstallationId {
    AdapterInstallationId::new("install-alpha").expect("installation")
}

/// Matches the `<@UBOT>` mentions the fixtures below already use.
const TEST_BOT_USER_ID: &str = "UBOT";

fn normalize(value: serde_json::Value) -> SlackInboundEvent {
    normalize_slack_event(
        &serde_json::to_vec(&value).expect("payload"),
        &installation_id(),
        Some(TEST_BOT_USER_ID),
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
    for (event, expected_reason) in [
        (
            serde_json::json!({
                "type": "message", "user": "U1", "channel": "D1", "text": "loop",
                "ts": "1.0", "bot_id": "B1"
            }),
            SlackIgnoreReason::BotAuthored,
        ),
        (
            serde_json::json!({
                "type": "message", "user": "U1", "channel": "D1", "text": "changed",
                "ts": "1.0", "subtype": "message_changed"
            }),
            SlackIgnoreReason::NonUserMessageSubtype("message_changed".to_string()),
        ),
        // No mention, no thread_ts: still bystander chatter even though
        // `mentions_bot` is now consulted for `message` events — the text
        // here names nobody, so `TextMention` is never reached and the
        // ambient rule holds exactly as before that check existed.
        (
            serde_json::json!({
                "type": "message", "user": "U1", "channel": "C1", "text": "ambient",
                "ts": "1.0"
            }),
            SlackIgnoreReason::AmbientChannelMessage,
        ),
    ] {
        let outcome = normalize(serde_json::json!({
            "type": "event_callback", "event_id": "EvIgnored", "event": event
        }));
        let SlackInboundEvent::Ignore { reason } = outcome else {
            panic!("expected Ignore for {expected_reason:?}, got {outcome:?}");
        };
        assert_eq!(reason, expected_reason);
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
        Some(TEST_BOT_USER_ID),
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
    assert!(normalize_slack_event(&oversized, &installation_id(), Some(TEST_BOT_USER_ID)).is_err());
    assert!(matches!(
        normalize_slack_event(
            br#"{"type":"event_callback","event":{"type":"message"}}"#,
            &installation_id(),
            Some(TEST_BOT_USER_ID)
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
        let _ = normalize_slack_event(&raw, &installation_id(), Some(TEST_BOT_USER_ID));
    }
}

/// The incident: a threaded reply sent with "Also send to channel", naming
/// the bot. Slack stamps `thread_broadcast` on the `app_mention` event, and
/// the subtype allowlist used to drop it — so a person's explicit mention
/// became a bare 200 and no turn.
#[test]
fn a_broadcast_mention_still_reaches_the_host_as_a_bot_mention() {
    let broadcast = message(serde_json::json!({
        "type": "event_callback",
        "team_id": "T123",
        "event_id": "EvBroadcast",
        "event": {
            "type": "app_mention",
            "user": "U1",
            "channel": "C1",
            "subtype": "thread_broadcast",
            "text": "<@UBOT> what do you think?",
            "thread_ts": "1710000000.000010",
            "ts": "1710000000.000011"
        }
    }));
    assert_eq!(broadcast.text, "what do you think?");
    assert_eq!(broadcast.trigger, ProductTriggerReason::BotMention);
    assert_eq!(
        broadcast.conversation.topic_id(),
        Some("1710000000.000010"),
        "a broadcast mention stays anchored to the thread it was written in"
    );
}

/// The property that makes the fix structural rather than a longer list: an
/// `app_mention` is Slack's own statement that a human named this bot, so its
/// `subtype` is never consulted. A subtype nobody has seen before therefore
/// cannot silence a mention — which is the failure mode the allowlist had.
///
/// `document_mention` is the one documented carve-out — see
/// [`a_canvas_document_mention_is_dropped_for_synthetic_text`] below — so it
/// is deliberately excluded from this loop rather than asserted here as
/// still-admitted.
#[test]
fn an_app_mention_is_admitted_whatever_subtype_slack_stamps_on_it() {
    for subtype in [
        "thread_broadcast",
        "reply_broadcast",
        "file_share",
        "me_message",
        // Deliberately not a real Slack value, and deliberately not in
        // HUMAN_MESSAGE_SUBTYPES: the point is that membership is irrelevant
        // on this path.
        "some_subtype_slack_has_not_invented_yet",
    ] {
        let mention = message(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvExempt",
            "event": {
                "type": "app_mention", "user": "U1", "channel": "C1",
                "subtype": subtype, "text": "<@UBOT> ping",
                "ts": "1710000000.000012"
            }
        }));
        assert_eq!(
            mention.trigger,
            ProductTriggerReason::BotMention,
            "app_mention with subtype {subtype} must still start a run"
        );
    }
}

/// The one documented case where `app_mention` firing does NOT mean a
/// person addressed the bot in conversation: a canvas-body mention. Slack
/// sets `subtype: "document_mention"` and writes `text` itself (a caption
/// such as "was mentioned in a canvas") — the person's actual words live
/// only in `blocks`, which this contract never reads. Fixture is the exact
/// example payload from
/// <https://docs.slack.dev/reference/events/message/document_mention/>, so
/// this test is traceable to the source rather than an invented shape.
/// Before the fix this fell through the `AppMention` exemption above and
/// started a full agent turn on Slack-generated text.
#[test]
fn a_canvas_document_mention_is_dropped_for_synthetic_text() {
    let outcome = normalize(serde_json::json!({
        "event": {
            "user": "UA1BCD3EF",
            "subtype": "document_mention",
            "document_mention": {
                "file_id": "F123ABCDEFG",
                "section_id": "temp:C:GQL...",
                "mentioning_user_ids": ["UA1BCD3EF"]
            },
            "type": "app_mention",
            "ts": "1716411280.657549",
            "text": "<@U123456ABC7> was mentioned in a canvas",
            "blocks": [{
                "type": "section",
                "block_id": "gcn3v",
                "text": {
                    "type": "mrkdwn",
                    "text": ">>>Hey <@U123456ABC7>",
                    "verbatim": false
                }
            }],
            "team": "T1ABC2DE3",
            "channel": "C012ABCDEFG",
            "event_ts": "1716411280.657549"
        },
        "type": "event_callback",
        "team_id": "T1ABC2DE3",
        "event_id": "EvDocumentMention"
    }));
    assert!(
        matches!(
            outcome,
            SlackInboundEvent::Ignore {
                reason: SlackIgnoreReason::SyntheticMentionText
            }
        ),
        "a canvas-body mention must be dropped, not started as a turn on \
         Slack-written caption text: normalized to {outcome:?}"
    );
}

/// The exemption covers "how does this render", never "who wrote this" — a
/// mention exchange between two apps does not terminate, so the author guard
/// has to hold on the exempt path too.
#[test]
fn the_exemption_never_admits_a_bot_authored_app_mention() {
    for (label, event) in [
        (
            "bot_id set",
            serde_json::json!({
                "type": "app_mention", "user": "U1", "channel": "C1",
                "bot_id": "B123", "text": "<@UBOT> loop",
                "ts": "1710000000.000013"
            }),
        ),
        (
            "bot_message subtype without bot_id",
            serde_json::json!({
                "type": "app_mention", "user": "U1", "channel": "C1",
                "subtype": "bot_message", "text": "<@UBOT> loop",
                "ts": "1710000000.000014"
            }),
        ),
    ] {
        let outcome = normalize(serde_json::json!({
            "type": "event_callback", "team_id": "T123",
            "event_id": "EvBotMention", "event": event
        }));
        assert!(
            matches!(
                outcome,
                SlackInboundEvent::Ignore {
                    reason: SlackIgnoreReason::BotAuthored
                }
            ),
            "{label} normalized to {outcome:?}"
        );
    }
}

/// A DM gets no exemption — there is no `app_mention` to vouch for it — so
/// the subtype list still governs that branch. This is what keeps Slack's own
/// announcements from being answered in a direct chat, and it is the half of
/// the contract an over-eager relaxation would break.
#[test]
fn a_dm_still_consults_the_subtype_list() {
    let broadcast = message(serde_json::json!({
        "type": "event_callback",
        "team_id": "T123",
        "event_id": "EvDmBroadcast",
        "event": {
            "type": "message", "user": "U1", "channel": "D1",
            "channel_type": "im", "subtype": "thread_broadcast",
            "text": "broadcast in a dm thread",
            "thread_ts": "1710000000.000015", "ts": "1710000000.000016"
        }
    }));
    assert_eq!(broadcast.trigger, ProductTriggerReason::DirectChat);

    let announcement = normalize(serde_json::json!({
        "type": "event_callback", "team_id": "T123",
        "event_id": "EvDmAnnouncement",
        "event": {
            "type": "message", "user": "U1", "channel": "D1",
            "channel_type": "im", "subtype": "channel_join",
            "text": "channel join message", "ts": "1710000000.000017"
        }
    }));
    assert!(
        matches!(
            announcement,
            SlackInboundEvent::Ignore {
                reason: SlackIgnoreReason::NonUserMessageSubtype(ref subtype)
            } if subtype == "channel_join"
        ),
        "a system announcement in a DM normalized to {announcement:?}"
    );
}

/// Slack's channel announcements are subtyped messages whose text can name
/// this bot — `bot_add` does so by construction. None of them may become a
/// mention: their text naming the bot now classifies them as `TextMention`
/// (see [`SlackMessageKind::TextMention`]), but that kind is NOT exempt from
/// the subtype allowlist — only `AppMention` is — so the allowlist still
/// drops them. This is the regression that sank an earlier attempt at text
/// mention detection: without the subtype gate still applying to
/// `TextMention`, every one of these becomes an admitted `BotMention`.
#[test]
fn slack_channel_announcements_are_never_admitted() {
    for (label, subtype, event) in [
        (
            "bot_add names the bot in its own text",
            "bot_add",
            serde_json::json!({
                "type": "message", "user": "U9", "channel": "C1",
                "subtype": "bot_add", "ts": "1710000000.000018",
                "text": "added an integration to this channel: <@UBOT>"
            }),
        ),
        (
            "a channel topic set to mention the bot",
            "channel_topic",
            serde_json::json!({
                "type": "message", "user": "U9", "channel": "C1",
                "subtype": "channel_topic", "ts": "1710000000.000019",
                "text": "<@U9> set the channel topic: ask <@UBOT> for help"
            }),
        ),
        (
            "the bot's own join announcement",
            "channel_join",
            serde_json::json!({
                "type": "message", "user": "UBOT", "channel": "C1",
                "subtype": "channel_join", "ts": "1710000000.000020",
                "text": "<@UBOT> has joined the channel"
            }),
        ),
    ] {
        let outcome = normalize(serde_json::json!({
            "type": "event_callback", "team_id": "T123",
            "event_id": "EvAnnouncement", "event": event
        }));
        assert!(
            matches!(
                &outcome,
                SlackInboundEvent::Ignore {
                    reason: SlackIgnoreReason::NonUserMessageSubtype(got)
                } if got == subtype
            ),
            "{label} normalized to {outcome:?}"
        );
    }
}

/// The point of `TextMention`: a `message` event whose TEXT names the bot
/// becomes a `BotMention` even with no `app_mention` twin at all — Slack does
/// not always send one, notably for a mention made inside an existing thread.
/// Three shapes that previously either silently answered the wrong trigger or
/// (for the top-level and `thread_broadcast` cases) were dropped outright as
/// bystander chatter / non-human subtype.
#[test]
fn a_message_event_naming_the_bot_becomes_a_bot_mention_with_no_app_mention_twin() {
    for (label, event, expected_topic) in [
        (
            "inside an existing thread, no subtype (Slack does not reliably \
             fire app_mention for this case)",
            serde_json::json!({
                "type": "message", "user": "U1", "channel": "C1",
                "text": "<@UBOT> handle this thread",
                "thread_ts": "1710000000.000030", "ts": "1710000000.000031"
            }),
            Some("1710000000.000030"),
        ),
        (
            "top level, no thread_ts, no subtype (previously dropped as \
             ambient chatter)",
            serde_json::json!({
                "type": "message", "user": "U1", "channel": "C1",
                "text": "<@UBOT> handle this at top level",
                "ts": "1710000000.000032"
            }),
            Some("1710000000.000032"),
        ),
        (
            "thread_broadcast subtype, the incident shape arriving only as a \
             message",
            serde_json::json!({
                "type": "message", "user": "U1", "channel": "C1",
                "subtype": "thread_broadcast",
                "text": "<@UBOT> handle this broadcast",
                "thread_ts": "1710000000.000033", "ts": "1710000000.000034"
            }),
            Some("1710000000.000033"),
        ),
    ] {
        let parsed = message(serde_json::json!({
            "type": "event_callback", "team_id": "T123",
            "event_id": "EvTextMention", "event": event
        }));
        assert_eq!(parsed.trigger, ProductTriggerReason::BotMention, "{label}");
        assert_eq!(parsed.conversation.topic_id(), expected_topic, "{label}");
    }
}

/// Slack announces one post as up to two events — `app_mention` and
/// `message` — with distinct `event_id`s in the envelope. The dedup key is
/// built from message identity (team, channel, ts), not the envelope
/// `event_id`, so the twins collapse to one durable admission key whichever
/// arrives first. Both twins must still resolve to `BotMention`, so whichever
/// one wins the collapse still starts a run — that is the entire point of
/// keying on the message rather than the event.
#[test]
fn app_mention_and_message_twins_of_one_post_collapse_to_the_same_event_id() {
    let shared_ts = "1710000000.000040";
    let app_mention_twin = message(serde_json::json!({
        "type": "event_callback", "team_id": "T123",
        "event_id": "EvTwinAppMention",
        "event": {
            "type": "app_mention", "user": "U1", "channel": "C1",
            "text": "<@UBOT> ping", "ts": shared_ts
        }
    }));
    let message_twin = message(serde_json::json!({
        "type": "event_callback", "team_id": "T123",
        "event_id": "EvTwinMessage",
        "event": {
            "type": "message", "user": "U1", "channel": "C1",
            "text": "<@UBOT> ping", "ts": shared_ts
        }
    }));

    assert_eq!(
        app_mention_twin.event_id.as_str(),
        message_twin.event_id.as_str(),
        "the app_mention and message twins of one post must share one dedup key"
    );
    assert_eq!(app_mention_twin.trigger, ProductTriggerReason::BotMention);
    assert_eq!(message_twin.trigger, ProductTriggerReason::BotMention);
}

/// `strip_leading_bot_mention` strips ONLY a leading token that names the
/// configured bot. A third party's mention in front is left alone — even
/// though their tag happens to be followed by the bot's own — because it is
/// not the bot's mention leading the text; and a non-leading mention of the
/// bot is never touched regardless of position.
#[test]
fn strip_leading_bot_mention_never_eats_a_third_partys_leading_mention() {
    assert_eq!(
        strip_leading_bot_mention("<@UOTHER> <@UBOT> deploy", Some(TEST_BOT_USER_ID)),
        "<@UOTHER> <@UBOT> deploy",
        "a third party's mention must not be dropped just because the bot is \
         also named later in the text"
    );
    assert_eq!(
        strip_leading_bot_mention("<@UBOT> deploy", Some(TEST_BOT_USER_ID)),
        "deploy",
        "the bot's own leading mention still strips"
    );
}

/// A bare mention with nothing else in the text still admits a message —
/// stripping to an empty string must not itself be treated as missing
/// content.
#[test]
fn a_bare_bot_mention_still_produces_a_message() {
    let mention = message(serde_json::json!({
        "type": "event_callback", "team_id": "T123",
        "event_id": "EvBareMention",
        "event": {
            "type": "app_mention", "user": "U1", "channel": "C1",
            "text": "<@UBOT>", "ts": "1710000000.000041"
        }
    }));
    assert_eq!(mention.text, "");
    assert_eq!(mention.trigger, ProductTriggerReason::BotMention);
}
