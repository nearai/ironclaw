//! Input-reader and refusal tests.
//!
//! Everything here is reachable without a live Telegram connection: argument
//! decoding, the two "this listing cannot be resumed" refusals, and the
//! account-resolution error projection. The vendor-call paths are exercised
//! against a scripted fake at the integration tier (PROPOSAL §10).

use ironclaw_host_api::messaging::StandardMessagingErrorCode;
use serde_json::json;

use super::*;
use crate::linked::mapping::{ConversationRef, UserRef};

fn summary(error: &ToolError) -> String {
    match error {
        ToolError::Failed { safe_summary, .. } => {
            safe_summary.clone().unwrap_or_else(|| "<none>".to_string())
        }
        ToolError::Rejected { kind, .. } => kind.human_summary().to_string(),
        ToolError::AuthRequired { .. } => "auth_required".to_string(),
    }
}

fn conversation() -> ConversationRef {
    ConversationRef::from_peer_ref(grammers_client::session::types::PeerRef {
        id: grammers_client::session::types::PeerId::user(17).expect("user id"),
        auth: grammers_client::session::types::PeerAuth::from_hash(99),
    })
}

#[test]
fn an_undecodable_conversation_ref_is_unknown_conversation() {
    let input = json!({ "conversation": "not-a-ref" });
    let error = conversation_arg(&input, "conversation").expect_err("must refuse");
    assert!(
        summary(&error).contains(StandardMessagingErrorCode::UnknownConversation.as_str()),
        "{}",
        summary(&error)
    );
}

/// A user ref must not be spendable where a conversation ref is expected: on
/// Telegram both can name the same peer, and letting one stand in for the other
/// is the noun confusion the canonical contract exists to prevent.
#[test]
fn a_user_ref_is_not_accepted_as_a_conversation() {
    let user = UserRef::from_peer_ref(conversation().peer_ref()).encode();
    let input = json!({ "conversation": user });
    assert!(conversation_arg(&input, "conversation").is_err());
}

#[test]
fn a_message_ref_missing_its_conversation_half_is_refused() {
    let input = json!({ "message_ref": { "conversation": "junk", "message_id": "7" } });
    let error = message_ref_arg(&input).expect_err("must refuse");
    assert!(summary(&error).contains(StandardMessagingErrorCode::UnknownMessage.as_str()));
}

/// The per-page item cap is a content bound (PROPOSAL §6.4/§7.2), so what
/// matters is that nothing a *caller* supplies can raise it — not the absent
/// case, not a large number, not a number larger than `usize` arithmetic
/// expects. Every list-shaped read multiplies this by `MAX_PAGES_PER_CALL` to
/// get its walk ceiling, so an unclamped value would also uncap the vendor
/// walk.
#[test]
fn limits_clamp_to_the_adapter_page_ceiling() {
    assert_eq!(limit_arg(&json!({})), MAX_ITEMS_PER_PAGE);
    assert_eq!(limit_arg(&json!({ "limit": 5 })), 5);
    assert_eq!(limit_arg(&json!({ "limit": 1000 })), MAX_ITEMS_PER_PAGE);
    assert_eq!(limit_arg(&json!({ "limit": u64::MAX })), MAX_ITEMS_PER_PAGE);
    assert_eq!(limit_arg(&json!({ "limit": 0 })), 1);
    // A non-numeric or negative `limit` cannot reach here past the canonical
    // schema, but falling back to the ceiling rather than to "unbounded" is
    // what makes that a defence rather than a coincidence.
    assert_eq!(limit_arg(&json!({ "limit": -1 })), MAX_ITEMS_PER_PAGE);
    assert_eq!(limit_arg(&json!({ "limit": "many" })), MAX_ITEMS_PER_PAGE);
}

/// The rule this protects: a cursor the adapter cannot honour must be an
/// error. Ignoring it returns page one under the name of page two, which reads
/// as success and quietly loses everything after it.
#[test]
fn an_undecodable_cursor_is_a_model_visible_error_not_a_restart() {
    let error = cursor_arg(&json!({ "cursor": "garbage" })).expect_err("must refuse");
    assert!(summary(&error).contains(StandardMessagingErrorCode::UnsupportedContent.as_str()));
    assert!(
        cursor_arg(&json!({}))
            .expect("absent cursor is fine")
            .is_none()
    );
}

#[test]
fn an_unpaginated_listing_refuses_a_cursor_rather_than_ignoring_it() {
    assert!(reject_cursor(&json!({})).is_ok());
    let error = reject_cursor(&json!({ "cursor": "anything" })).expect_err("must refuse");
    assert!(summary(&error).contains(StandardMessagingErrorCode::UnsupportedContent.as_str()));
}

/// "Not linked" and "resolution failed" are different facts and must not
/// collapse: telling a user to re-link because a store blipped is a bad
/// instruction, and answering a genuinely unlinked account with a vendor error
/// hides the connect affordance.
#[test]
fn account_resolution_failures_project_to_distinct_outcomes() {
    use ironclaw_extension_contracts::linked_session::LinkedAccountResolutionError;

    let not_linked = resolution_error(LinkedAccountResolutionError::NotLinked);
    assert!(matches!(not_linked, ToolError::AuthRequired { .. }));

    let unavailable = resolution_error(LinkedAccountResolutionError::Unavailable);
    assert!(summary(&unavailable).contains(StandardMessagingErrorCode::VendorError.as_str()));
}

#[test]
fn a_revoked_session_parks_on_the_auth_gate() {
    let revoked = ToolError::from(SessionPoolError::Revoked);
    assert!(matches!(revoked, ToolError::AuthRequired { .. }));
    let not_linked = ToolError::from(SessionPoolError::NotLinked);
    assert!(matches!(not_linked, ToolError::AuthRequired { .. }));
    let at_capacity = ToolError::from(SessionPoolError::AtCapacity);
    let poisoned = ToolError::from(SessionPoolError::Poisoned);
    let not_configured = ToolError::from(SessionPoolError::NotConfigured);
    assert!(
        matches!(&poisoned, ToolError::Failed { .. }),
        "a poisoned pool is a failure, never an auth prompt: {poisoned:?}"
    );
    match &not_configured {
        ToolError::Failed {
            model_visible_cause: Some(cause),
            ..
        } => assert!(
            cause.contains("MTProto application identity"),
            "the not-configured cause must name the missing config: {cause}"
        ),
        other => panic!("expected a Failed with a cause, got {other:?}"),
    }
    assert!(summary(&at_capacity).contains(StandardMessagingErrorCode::VendorError.as_str()));
}

/// `open_dm` is a pure re-encode, so it is fully testable here — and the thing
/// worth pinning is what it refuses: a channel-post author ref names nobody,
/// and turning it into a DM target would be a fabricated identity.
#[test]
fn open_dm_re_encodes_a_user_and_refuses_everything_else() {
    let peer_ref = conversation().peer_ref();
    let user_ref = UserRef::from_peer_ref(peer_ref).encode();
    let output = writes::open_dm(&json!({ "user_ref": user_ref })).expect("re-encode");
    let encoded = output["conversation"].as_str().expect("conversation");
    assert_eq!(
        ConversationRef::decode(encoded).map(|reference| reference.peer_ref()),
        Some(peer_ref)
    );

    let authorless = UserRef::authorless(&conversation());
    let error = writes::open_dm(&json!({ "user_ref": authorless })).expect_err("must refuse");
    assert!(summary(&error).contains(StandardMessagingErrorCode::UnknownUser.as_str()));

    let group = ConversationRef::from_peer_ref(grammers_client::session::types::PeerRef {
        id: grammers_client::session::types::PeerId::channel(4).expect("channel id"),
        auth: grammers_client::session::types::PeerAuth::from_hash(1),
    });
    let group_as_user = UserRef::from_peer_ref(group.peer_ref()).encode();
    let error = writes::open_dm(&json!({ "user_ref": group_as_user })).expect_err("must refuse");
    assert!(summary(&error).contains(StandardMessagingErrorCode::UnknownUser.as_str()));
}
