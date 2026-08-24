//! Canonical messaging conformance for the linked tools (PROPOSAL §10).
//!
//! Every op bound in [`super::tools`] promises its output matches the host's
//! canonical JSON Schema for that operation. The assertions here are made with
//! the host's own exported helpers
//! (`ironclaw_host_api::test_support::messaging_conformance`) so the schema
//! this package is judged against is the registry's, never a copy — and they
//! run against the package's real output builders in [`super::mapping`], never
//! against literals restated in the test.
//!
//! **`send_message` needs two tests, and that is the whole point of this file.**
//! Its output graduated to `.output.v2`, which states evidence as a `oneOf`:
//! either a provider-issued `message_ref`, or `sent_unverified: true` for the
//! case where Telegram accepted the send but grammers could not correlate it
//! (`id == 0`). `message_ref_from_output` — the helper the evidence loop is
//! built on — cannot extract evidence from the second branch, because there is
//! no evidence to extract. So the evidence loop runs off the ref branch, and
//! the unverified branch gets its own test below (§6.2).
//!
//! Three claims that read alike and are not:
//!
//! | Fact | Projection |
//! | --- | --- |
//! | Telegram issued an id | `Completed` + `message_ref` |
//! | Telegram accepted it, id uncorrelated | `Completed` + `sent_unverified` |
//! | Outcome genuinely unknown (`Dropped`/`Io`) | `messaging.vendor_error` |
//!
//! Collapsing row 2 into row 1 fabricates evidence; collapsing row 3 into row 2
//! asserts a delivery that may never have happened. Both are tested here.
//!
//! The vendor-call paths that *produce* these values need a live connection and
//! are exercised against a scripted fake at the integration tier; what this
//! file owns is the shape contract, which is reachable without one.

use std::panic::{self, AssertUnwindSafe};

use ironclaw_extension_contracts::tool_adapter::ToolError;
use ironclaw_host_api::{
    dispatch::RuntimeDispatchErrorKind,
    messaging::{StandardMessagingErrorCode, StandardMessagingOp},
    test_support::messaging_conformance::{
        assert_canonical_input_accepted, assert_canonical_output, message_ref_from_output,
    },
};
use serde_json::json;

use super::{
    mapping::{self, ConversationRef, OpFamily},
    ops::writes,
    tools,
};
use grammers_client::{
    InvocationError,
    session::types::{PeerAuth, PeerId, PeerRef},
};

/// The model-facing addendum for `send_message`, embedded at compile time so
/// the test cannot drift from the file the manifest ships.
const SEND_MESSAGE_ADDENDUM: &str = include_str!("../../prompts/telegram/send_message.md");

fn peer(id: i64) -> PeerRef {
    PeerRef {
        id: PeerId::user(id).expect("user id in range"), // safety: this cfg(test) helper is called only with small positive fixture ids
        auth: PeerAuth::from_hash(id * 7),
    }
}

fn conversation() -> ConversationRef {
    ConversationRef::from_peer_ref(peer(4_242))
}

fn failure_kind(error: &ToolError) -> Option<RuntimeDispatchErrorKind> {
    match error {
        ToolError::Rejected { kind, .. } => Some(*kind),
        ToolError::AuthRequired { .. } => None,
    }
}

fn failure_summary(error: &ToolError) -> String {
    match error {
        ToolError::Rejected {
            kind, diagnostic, ..
        } => diagnostic
            .as_ref()
            .and_then(|diagnostic| diagnostic.code.as_ref())
            .map(|code| code.as_str().to_string())
            .unwrap_or_else(|| kind.human_summary().to_string()),
        ToolError::AuthRequired { .. } => "auth_required".to_string(),
    }
}

/// The evidence loop: a send that Telegram correlated hands back a
/// `message_ref`, that exact object is what every follow-up write takes as
/// input, and each of those writes echoes it back as its own evidence.
///
/// It runs off the ref branch because that is the only branch carrying
/// evidence — see the module header. Each hop asserts against the canonical
/// schema rather than against this file's expectations, and the final hop
/// re-reads the ref through the adapter's own input reader, which is what
/// proves the loop closes rather than merely type-checking.
#[test]
fn the_evidence_loop_runs_off_the_message_ref_branch() {
    let conversation = conversation();
    let send = mapping::send_result(&conversation, 9_001, None, None);
    assert_canonical_output(StandardMessagingOp::SendMessage, &send);

    let evidence = message_ref_from_output(&send);

    // Every write that consumes evidence accepts this exact object.
    assert_canonical_input_accepted(
        StandardMessagingOp::EditMessage,
        &json!({ "message_ref": evidence, "text": "corrected" }),
    );
    assert_canonical_input_accepted(
        StandardMessagingOp::DeleteMessage,
        &json!({ "message_ref": evidence }),
    );
    assert_canonical_input_accepted(
        StandardMessagingOp::AddReaction,
        &json!({ "message_ref": evidence, "emoji": "🎯" }),
    );
    assert_canonical_input_accepted(
        StandardMessagingOp::RemoveReaction,
        &json!({ "message_ref": evidence }),
    );

    // …and every one of them returns evidence of its own, in canonical shape.
    let edit = mapping::ref_only_result(&conversation, 9_001);
    assert_canonical_output(StandardMessagingOp::EditMessage, &edit);
    assert_eq!(message_ref_from_output(&edit), evidence);

    let delete = mapping::delete_result(&conversation, 9_001);
    assert_canonical_output(StandardMessagingOp::DeleteMessage, &delete);
    assert_eq!(delete["deleted"], json!(true));
    assert_eq!(message_ref_from_output(&delete), evidence);

    let added = mapping::add_reaction_result(&conversation, 9_001, "🎯");
    assert_canonical_output(StandardMessagingOp::AddReaction, &added);
    assert_eq!(message_ref_from_output(&added), evidence);

    // `remove_reaction` clears every reaction this account left, so it omits
    // `emoji` rather than echoing a precision it does not have (§6.3).
    let removed = mapping::ref_only_result(&conversation, 9_001);
    assert_canonical_output(StandardMessagingOp::RemoveReaction, &removed);
    assert!(removed.get("emoji").is_none(), "{removed}");

    // The loop closes: the ref the model would spend decodes back through the
    // adapter's own reader to the conversation and id it was minted from.
    let (decoded_conversation, decoded_id) =
        mapping::parse_message_ref(&evidence).expect("the adapter accepts its own evidence");
    assert_eq!(decoded_conversation, conversation);
    assert_eq!(decoded_id, 9_001);
}

/// `open_dm` is the one bound op that produces canonical output with no vendor
/// call at all, so its whole contract is testable here — and its output is the
/// conversation ref every read and write above is addressed with.
#[test]
fn open_dm_returns_a_canonical_conversation_ref() {
    let user_ref = mapping::UserRef::from_peer_ref(peer(77)).encode();
    let output = writes::open_dm(&json!({ "user_ref": user_ref })).expect("re-encode");
    assert_canonical_output(StandardMessagingOp::OpenDm, &output);
    assert_canonical_input_accepted(
        StandardMessagingOp::GetConversationHistory,
        &json!({ "conversation": output["conversation"] }),
    );
}

/// The branch that needs its own test: `id == 0` is a **completed send**, and
/// the evidence loop cannot reach it because there is no evidence to extract.
///
/// Asserted here: the output is canonical (v2's `oneOf` admits it), it carries
/// `sent_unverified: true`, it fabricates no `message_ref`, and
/// `message_ref_from_output` genuinely refuses it — which is the reason this
/// test exists rather than being folded into the loop above.
#[test]
fn the_sent_unverified_branch_is_canonical_and_yields_no_extractable_evidence() {
    let conversation = conversation();
    let output = mapping::send_result(&conversation, 0, None, None);

    assert_canonical_output(StandardMessagingOp::SendMessage, &output);
    assert_eq!(output["sent_unverified"], json!(true));
    assert!(
        output.get("message_ref").is_none(),
        "a fabricated ref poisons every later edit/delete and makes the evidence \
         gate unfalsifiable: {output}"
    );

    let extracted = panic::catch_unwind(AssertUnwindSafe(|| message_ref_from_output(&output)));
    assert!(
        extracted.is_err(),
        "the evidence loop must not be able to run off this branch — if it can, \
         something fabricated a ref"
    );

    // The schema-side facts about the graduation — that the two branches are
    // mutually exclusive, that `sent_unverified` is `const: true`, and that
    // `.v2` is a strict superset of `.v1` — belong to the contract and are
    // pinned there (`ironclaw_host_api::messaging`'s
    // `send_message_v2_accepts_every_v1_valid_output`). What this package owes
    // is the assertion above: that its own builder emits one branch and only
    // one, for exactly the vendor condition that branch describes.
}

/// An uncorrelated send is `Completed` and is never retried.
///
/// "Not retryable" has two halves at this tier, and both are asserted: the
/// projection is a success — `ToolResult`, never a `ToolError`, so no failure
/// kind and no same-call retry constraint is ever attached — and the
/// model-facing addendum says in words not to re-send, because a
/// `sent_unverified: true` the model does not understand is worse than no
/// field at all.
#[test]
fn an_uncorrelated_send_is_completed_and_is_never_offered_for_retry() {
    let conversation = conversation();
    let output = mapping::send_result(&conversation, 0, None, None);

    // The adapter's success projection. Reaching a `ToolError` from here is
    // impossible by construction: `send_result` is infallible, and that is the
    // point — a failure is what a model retries, and a retried send delivers a
    // second message to a human.
    let result = tools::tool_result(output.clone());
    assert_eq!(result.output, output);
    assert!(result.output_bytes > 0);
    assert!(result.display_preview.is_none());

    // Every id is projected, and only zero takes the unverified branch — so no
    // ordinary send can be silently reported as unverified either.
    for message_id in [1, 42, i32::MAX] {
        let sent = mapping::send_result(&conversation, message_id, None, None);
        assert!(sent.get("sent_unverified").is_none(), "{sent}");
        assert_eq!(
            sent["message_ref"]["message_id"],
            json!(message_id.to_string())
        );
    }

    assert!(
        SEND_MESSAGE_ADDENDUM.contains("sent_unverified"),
        "the vendor addendum must name the field, since the shared messaging core \
         deliberately does not — Slack's and acme's models must not be told about a \
         field their adapters can never emit (§6.2)"
    );
    let addendum = SEND_MESSAGE_ADDENDUM.to_lowercase();
    assert!(
        addendum.contains("do not send it again") || addendum.contains("do not re-send"),
        "sent_unverified is inert unless the addendum forbids the re-send: {SEND_MESSAGE_ADDENDUM}"
    );
}

/// `Dropped`/`Io` on a write is an **unknown outcome**, not an unverified send.
///
/// They are different epistemic states: `sent_unverified` asserts the message
/// reached the human, while a torn-down connection means the request may or may
/// not have executed. Reporting the unknown case as a send is the same false
/// report as reporting the accepted case as a failure — inverted — and this is
/// the test that keeps them apart.
#[test]
fn an_unknown_write_outcome_is_a_vendor_error_never_an_unverified_send() {
    for error in [
        InvocationError::Dropped,
        InvocationError::Io(std::io::Error::other("connection reset")),
    ] {
        let mapped = mapping::map_vendor_error(OpFamily::Write, &error);

        // A failure, so the model is told something went wrong…
        assert_eq!(
            failure_kind(&mapped),
            Some(RuntimeDispatchErrorKind::OperationFailed),
            "{error:?}"
        );
        let summary = failure_summary(&mapped);
        assert!(
            summary.contains(StandardMessagingErrorCode::VendorError.as_str()),
            "{summary}"
        );
        assert!(
            !matches!(mapped, ToolError::AuthRequired { .. }),
            "a dropped connection is not a credential problem"
        );

        // …and it is told the outcome is UNKNOWN. The wording is the whole
        // difference: "the send failed" invites a re-send that double-messages
        // a human, and "the message was sent" suppresses a re-send that may be
        // the only delivery.
        let ToolError::Rejected { diagnostic, .. } = &mapped else {
            panic!("an unknown write outcome is a rejection");
        };
        let cause = diagnostic
            .as_ref()
            .and_then(|diagnostic| diagnostic.message.as_ref())
            .map(|message| message.as_str())
            .expect("an unknown outcome must say so");
        assert!(cause.contains("before the outcome was known"), "{cause}");

        // The type is the guarantee: this arm produces a `ToolError`, and
        // `sent_unverified` only ever exists inside the `Value` that
        // `send_result` builds when the vendor RETURNED a message. An error
        // path never reaches that builder, so the marker cannot appear here —
        // which is what keeps "unknown" from being reported as "delivered".
        assert!(matches!(mapped, ToolError::Rejected { .. }));
    }

    // The contrast that makes the rule visible: the *same* op family, given a
    // vendor response instead of a torn-down connection, does produce the
    // unverified marker. Two different facts, two different projections.
    let unverified = mapping::send_result(&conversation(), 0, None, None);
    assert_eq!(unverified["sent_unverified"], json!(true));
}
