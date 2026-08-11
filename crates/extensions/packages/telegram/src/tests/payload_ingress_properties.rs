//! Property tests for the Telegram ingress boundary (#6524 workstream 9:
//! "focused fuzzing for untrusted ingress").
//!
//! The sibling of the Slack ingress properties. Both entry points sit on a
//! public webhook and see whatever the internet sends before any secret has
//! been trusted, so covering one and not the other would leave half the
//! surface unexamined while the box read as closed.

use super::*;
use ironclaw_host_api::product_adapter::auth::AuthRequirement;
use proptest::prelude::*;

fn verified_evidence() -> ProtocolAuthEvidence {
    // `test_verified` is the `test-support` seam standing in for the host:
    // an adapter crate holds no `VerifiedInboundGrant` and must not be able
    // to mint in production (PROPOSAL §12.1a). Value-identical to the
    // pre-WS1.5 `mark_shared_secret_header_verified` call this replaced.
    ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Telegram-Bot-Api-Secret-Token".to_string(),
        },
        "telegram_install_property",
    )
}

fn unverified_evidence() -> ProtocolAuthEvidence {
    ProtocolAuthEvidence::failed(ironclaw_host_api::product_adapter::ProtocolAuthFailure::Missing)
}

fn install() -> AdapterInstallationId {
    AdapterInstallationId::new("install_property").expect("valid")
}

fn trigger_policy() -> GroupTriggerPolicy {
    GroupTriggerPolicy {
        bot_username: "ironclaw_bot".into(),
        bot_user_id: 9000,
        recognized_commands: vec!["start".into(), "help".into()],
    }
}

/// Update-shaped payloads plus noise.
///
/// Biased for the same reason as the Slack generator: uniform random bytes
/// die at the JSON parser and exercise only the outermost guard, never the
/// branches that read chat, entities or commands.
fn update_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..256),
        "\\PC{0,200}".prop_map(|s| s.into_bytes()),
        (any::<i64>(), "\\PC{0,40}", any::<i64>(), "[a-z_]{0,12}").prop_map(
            |(update_id, text, chat_id, chat_type)| {
                serde_json::json!({
                    "update_id": update_id,
                    "message": {
                        "message_id": 1,
                        "date": 0,
                        "text": text,
                        "chat": {"id": chat_id, "type": chat_type},
                    }
                })
                .to_string()
                .into_bytes()
            }
        ),
        // Command-shaped text, which drives the entity/command branches.
        ("[a-z]{1,10}", any::<i64>()).prop_map(|(command, chat_id)| {
            serde_json::json!({
                "update_id": 7,
                "message": {
                    "message_id": 1,
                    "date": 0,
                    "text": format!("/{command}@ironclaw_bot payload"),
                    "entities": [{"type": "bot_command", "offset": 0, "length": command.len() + 1}],
                    "chat": {"id": chat_id, "type": "group"},
                }
            })
            .to_string()
            .into_bytes()
        }),
    ]
}

proptest! {
    /// No update is parsed without verified evidence.
    ///
    /// Telegram's webhook secret is the only thing standing between the
    /// public endpoint and an injected turn, exactly as the Slack
    /// signature is on that side.
    #[test]
    fn unverified_evidence_rejects_every_update(raw in update_bytes()) {
        let outcome =
            parse_telegram_update(&raw, &unverified_evidence(), &install(), &trigger_policy());
        prop_assert!(
            matches!(outcome, Err(PayloadParseError::UnauthenticatedPayload)),
            "unverified update was not rejected: {outcome:?}"
        );
    }

    /// Verified evidence plus arbitrary bytes parses or errors, never panics.
    #[test]
    fn verified_evidence_never_panics(raw in update_bytes()) {
        let _ =
            parse_telegram_update(&raw, &verified_evidence(), &install(), &trigger_policy());
        let _ = normalize_telegram_update(&raw, &install(), &trigger_policy());
    }
}

/// Telegram accepts an unbounded body where Slack refuses over 1 MiB.
///
/// Recorded rather than asserted as a limit, because there is none here to
/// assert. Whether that matters depends on a cap in the HTTP layer above,
/// which this function cannot see — so the honest thing is to pin the
/// current behaviour (a large body is parsed on its merits, not refused on
/// length) and leave the question visible instead of implying parity that
/// does not exist.
#[test]
fn large_bodies_are_judged_on_content_not_length() {
    let padding = "a".repeat(2 * 1024 * 1024);
    let payload = serde_json::json!({
        "update_id": 11,
        "message": {
            "message_id": 1,
            "date": 0,
            "text": padding,
            "chat": {"id": 42, "type": "private"},
        }
    })
    .to_string()
    .into_bytes();
    assert!(payload.len() > 1024 * 1024);

    let outcome = parse_telegram_update(
        &payload,
        &verified_evidence(),
        &install(),
        &trigger_policy(),
    );
    // Asserting only "not UnauthenticatedPayload" would be satisfied by
    // BodyTooLarge, or by any other parse error -- including the size
    // rejection this test exists to rule out. Pin the success instead.
    assert!(
        outcome.is_ok(),
        "a large body must still be judged on content, not rejected on \
             size or authentication; got {outcome:?}"
    );
}
