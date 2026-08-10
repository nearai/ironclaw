//! Mapping tests. The rules exercised here are the ones a well-meaning change
//! is most likely to break quietly: an accepted send reported as a failure, a
//! fabricated ref, a silently restarted cursor, a filtered sticker.

use grammers_client::{InvocationError, sender::RpcError, session::types::PeerAuth};
use ironclaw_host_api::messaging::StandardMessagingErrorCode;

use super::*;

fn user_ref_for(id: i64, hash: i64) -> PeerRef {
    PeerRef {
        id: PeerId::user(id).expect("user id in range"),
        auth: PeerAuth::from_hash(hash),
    }
}

fn rpc(name: &str, value: Option<u32>) -> InvocationError {
    let message = match value {
        Some(value) => format!("{name}_{value}"),
        None => name.to_string(),
    };
    InvocationError::Rpc(RpcError::from(grammers_client::tl::types::RpcError {
        error_code: 400,
        error_message: message,
    }))
}

fn code_of(error: &ToolError) -> String {
    match error {
        ToolError::Failed { safe_summary, .. } => {
            safe_summary.clone().unwrap_or_else(|| "<none>".to_string())
        }
        ToolError::AuthRequired { .. } => "auth_required".to_string(),
    }
}

#[test]
fn conversation_refs_round_trip_for_every_peer_kind() {
    for id in [PeerId::user(7), PeerId::chat(7), PeerId::channel(7)] {
        let peer_ref = PeerRef {
            id: id.expect("id in range"),
            auth: PeerAuth::from_hash(-99),
        };
        let encoded = ConversationRef::from_peer_ref(peer_ref).encode();
        let decoded = ConversationRef::decode(&encoded).expect("round trip");
        assert_eq!(decoded.peer_ref(), peer_ref);
    }
}

/// A Telegram DM's peer *is* its counterpart, so the two nouns would collapse
/// under one encoding. They must not: a conversation ref spent as a user ref
/// is exactly the confusion the canonical contract forbids.
#[test]
fn a_conversation_ref_is_not_spendable_as_a_user_ref() {
    let peer_ref = user_ref_for(42, 1234);
    let conversation = ConversationRef::from_peer_ref(peer_ref).encode();
    let user = UserRef::from_peer_ref(peer_ref).encode();
    assert_ne!(conversation, user);
    assert!(UserRef::decode(&conversation).is_none());
    assert!(ConversationRef::decode(&user).is_none());
}

#[test]
fn an_authorless_ref_decodes_as_authorless_and_never_as_a_peer() {
    let conversation = ConversationRef::from_peer_ref(PeerRef {
        id: PeerId::channel(500).expect("channel id"),
        auth: PeerAuth::from_hash(7),
    });
    let encoded = UserRef::authorless(&conversation);
    assert_eq!(UserRef::decode(&encoded), Some(RefKind::Authorless));
}

#[test]
fn a_garbled_ref_decodes_to_nothing_rather_than_a_plausible_peer() {
    for raw in ["", "not-base64!!", "aGVsbG8", "12345"] {
        assert!(ConversationRef::decode(raw).is_none(), "{raw}");
        assert!(UserRef::decode(raw).is_none(), "{raw}");
    }
}

#[test]
fn cursors_round_trip_and_refuse_to_decode_garbage() {
    let cursor = Cursor {
        offset_id: 900,
        offset_date: 1_700_000_000,
    };
    assert_eq!(Cursor::decode(&cursor.encode()), Some(cursor));
    // A cursor that does not decode must reach the caller as `None` so it can
    // answer `messaging.unsupported_content`; anything that quietly produced a
    // default here would restart the caller at page one.
    for raw in [
        "",
        "zzzz",
        &ConversationRef::from_peer_ref(user_ref_for(1, 1)).encode(),
    ] {
        assert!(Cursor::decode(raw).is_none(), "{raw}");
    }
}

#[test]
fn message_refs_round_trip() {
    let conversation = ConversationRef::from_peer_ref(user_ref_for(11, 22));
    let value = message_ref(&conversation, 314);
    let (back, id) = parse_message_ref(&value).expect("round trip");
    assert_eq!(back, conversation);
    assert_eq!(id, 314);
    assert!(parse_message_ref(&json!({ "conversation": "x" })).is_none());
}

#[test]
fn sanitizer_strips_bidi_and_zero_width_but_keeps_joiners() {
    assert_eq!(
        sanitize_untrusted_text("a\u{200B}b\u{202E}c\u{2066}d\u{FEFF}e\u{200F}f"),
        "abcdef"
    );
    assert_eq!(sanitize_untrusted_text("با\u{200C}هم"), "با\u{200C}هم");
    assert_eq!(sanitize_untrusted_text("a\u{200D}b"), "a\u{200D}b");
    assert_eq!(sanitize_untrusted_text("a\r\nb\u{0007}c\td"), "a\nbc\td");
}

/// An empty `text` is legal on every read output (no `minLength`); an ABSENT
/// one is not. The sanitizer must therefore return `""`, never `None`-shaped
/// behaviour that a caller might turn into an omitted field.
#[test]
fn sanitizer_returns_an_empty_string_rather_than_dropping_the_field() {
    assert_eq!(sanitize_untrusted_text(""), "");
    assert_eq!(sanitize_untrusted_text("\u{200B}\u{202E}"), "");
}

#[test]
fn sanitizer_truncates_on_a_character_boundary() {
    let long = "é".repeat(MAX_MESSAGE_TEXT_BYTES);
    let clamped = sanitize_untrusted_text(&long);
    assert!(clamped.len() <= MAX_MESSAGE_TEXT_BYTES);
    assert!(long.starts_with(&clamped));
}

#[test]
fn result_bounding_drops_oldest_rows_until_the_budget_holds() {
    let row = json!({ "text": "x".repeat(1024) });
    let mut items = vec![row; 512];
    assert!(bound_result_bytes(&mut items));
    assert!(!items.is_empty());
    assert!(serde_json::to_vec(&items).expect("serializes").len() <= MAX_RESULT_BYTES);
}

/// The single most consequential rule in this module: Telegram took the
/// message, so the model must not be told to try again.
#[test]
fn an_uncorrelated_send_is_completed_and_unverified_not_a_failure() {
    let conversation = ConversationRef::from_peer_ref(user_ref_for(5, 6));
    let output = send_result(&conversation, 0, None, None);
    assert_eq!(output["sent_unverified"], json!(true));
    assert!(
        output.get("message_ref").is_none(),
        "a fabricated ref would poison every later edit/delete: {output}"
    );
}

#[test]
fn a_correlated_send_carries_provider_issued_evidence_and_echoes_its_routing() {
    let conversation = ConversationRef::from_peer_ref(user_ref_for(5, 6));
    let reply_to = message_ref(&conversation, 41);
    let output = send_result(&conversation, 42, Some("41"), Some(&reply_to));
    assert_eq!(output["message_ref"]["message_id"], json!("42"));
    assert!(output.get("sent_unverified").is_none());
    assert_eq!(output["thread"], json!("41"));
    assert_eq!(output["reply_to"], reply_to);
}

/// One row per canonical code, acme-style, plus the two rows that leave the
/// messaging vocabulary entirely.
#[test]
fn the_vendor_error_table_covers_every_canonical_code() {
    use StandardMessagingErrorCode::*;
    let cases: &[(&str, OpFamily, StandardMessagingErrorCode)] = &[
        ("CHANNEL_PRIVATE", OpFamily::Read, UnknownConversation),
        ("MESSAGE_ID_INVALID", OpFamily::Read, UnknownMessage),
        ("USER_ID_INVALID", OpFamily::People, UnknownUser),
        ("CHAT_WRITE_FORBIDDEN", OpFamily::Write, PermissionDenied),
        ("USER_IS_BLOCKED", OpFamily::Write, CannotMessageUser),
        ("MESSAGE_TOO_LONG", OpFamily::Write, MessageTooLong),
        ("MEDIA_INVALID", OpFamily::Write, UnsupportedContent),
        ("SLOWMODE_WAIT", OpFamily::Write, RateLimited),
        ("MESSAGE_EDIT_TIME_EXPIRED", OpFamily::Write, EditNotAllowed),
        ("USER_DEACTIVATED", OpFamily::Write, VendorError),
        ("SOMETHING_BRAND_NEW", OpFamily::Write, VendorError),
    ];
    for (name, family, expected) in cases {
        let mapped = map_vendor_error(*family, &rpc(name, None));
        assert!(
            code_of(&mapped).contains(expected.as_str()),
            "{name} mapped to {}",
            code_of(&mapped)
        );
    }
    // `not_a_member` is deliberately absent from the table: Telegram does not
    // distinguish "gone" from "you cannot see it", so claiming membership
    // knowledge would over-report. Every code except that one is covered above.
    assert_eq!(
        StandardMessagingErrorCode::ALL.len(),
        12,
        "a new canonical code needs a row in map_vendor_error and here"
    );
}

#[test]
fn a_bad_peer_resolves_by_op_family_rather_than_by_guess() {
    let peer = rpc("PEER_ID_INVALID", None);
    assert!(
        code_of(&map_vendor_error(OpFamily::People, &peer))
            .contains(StandardMessagingErrorCode::UnknownUser.as_str())
    );
    assert!(
        code_of(&map_vendor_error(OpFamily::Read, &peer))
            .contains(StandardMessagingErrorCode::UnknownConversation.as_str())
    );
}

#[test]
fn credential_failures_leave_the_messaging_vocabulary() {
    for name in [
        "AUTH_KEY_UNREGISTERED",
        "SESSION_REVOKED",
        "SESSION_EXPIRED",
    ] {
        let mapped = map_vendor_error(OpFamily::Read, &rpc(name, None));
        let ToolError::AuthRequired {
            required_secrets,
            credential_requirements,
        } = mapped
        else {
            panic!("{name} must park the run on the re-auth gate, not answer messaging.*");
        };
        assert_eq!(required_secrets.len(), 1);
        assert_eq!(
            credential_requirements
                .first()
                .map(|requirement| requirement.setup.clone()),
            Some(RuntimeCredentialAccountSetup::DeviceLink)
        );
    }
}

/// `Dropped`/`Io` on a write is an UNKNOWN outcome. Reporting it as
/// `sent_unverified` would assert a delivery that may never have happened —
/// the same false report as the failure mapping, inverted.
#[test]
fn an_unknown_write_outcome_is_a_vendor_error_and_never_sent_unverified() {
    for error in [
        InvocationError::Dropped,
        InvocationError::Io(std::io::Error::other("reset")),
    ] {
        let mapped = map_vendor_error(OpFamily::Write, &error);
        let summary = code_of(&mapped);
        assert!(
            summary.contains(StandardMessagingErrorCode::VendorError.as_str()),
            "{summary}"
        );
    }
}

#[test]
fn a_flood_wait_carries_its_retry_after_as_prose_only() {
    let mapped = map_vendor_error(OpFamily::Write, &rpc("FLOOD_WAIT", Some(31)));
    let ToolError::Failed {
        safe_summary,
        model_visible_cause,
        ..
    } = mapped
    else {
        panic!("flood wait is a failure");
    };
    // The fixed half names only the canonical code; the vendor's number rides
    // the free-form half, because no structured retry-after slot exists.
    let safe_summary = safe_summary.expect("summary");
    assert!(safe_summary.contains(StandardMessagingErrorCode::RateLimited.as_str()));
    assert!(!safe_summary.contains("31"));
    assert!(model_visible_cause.expect("cause").contains("31"));
}
