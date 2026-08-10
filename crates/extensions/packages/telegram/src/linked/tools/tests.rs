//! Binding-map tests: which capability ids this adapter answers, and — just as
//! load-bearing — which it does not.

use super::*;

#[test]
fn every_bound_capability_id_routes_to_its_standard_operation() {
    let expected: &[(&str, StandardMessagingOp)] = &[
        ("telegram.send_message", StandardMessagingOp::SendMessage),
        ("telegram.edit_message", StandardMessagingOp::EditMessage),
        (
            "telegram.delete_message",
            StandardMessagingOp::DeleteMessage,
        ),
        ("telegram.add_reaction", StandardMessagingOp::AddReaction),
        (
            "telegram.remove_reaction",
            StandardMessagingOp::RemoveReaction,
        ),
        ("telegram.open_dm", StandardMessagingOp::OpenDm),
        (
            "telegram.list_conversations",
            StandardMessagingOp::ListConversations,
        ),
        (
            "telegram.get_conversation_info",
            StandardMessagingOp::GetConversationInfo,
        ),
        (
            "telegram.get_conversation_history",
            StandardMessagingOp::GetConversationHistory,
        ),
        ("telegram.get_message", StandardMessagingOp::GetMessage),
        (
            "telegram.search_messages",
            StandardMessagingOp::SearchMessages,
        ),
        ("telegram.whoami", StandardMessagingOp::Whoami),
        ("telegram.get_user_info", StandardMessagingOp::GetUserInfo),
        ("telegram.resolve_user", StandardMessagingOp::ResolveUser),
        ("telegram.list_members", StandardMessagingOp::ListMembers),
    ];
    for (capability_id, op) in expected {
        assert_eq!(bound_op(capability_id), Some(*op), "{capability_id}");
    }
    assert_eq!(
        expected.len(),
        15,
        "fifteen of the sixteen core ops bind; get_thread_replies does not"
    );
}

/// Telegram keeps thread replies inside the chat history, so binding
/// `get_thread_replies` would ship a second, duplicate history read. The
/// manifest omits it; this pins the execution side to agree.
#[test]
fn get_thread_replies_is_not_bound() {
    assert_eq!(bound_op("telegram.get_thread_replies"), None);
}

#[test]
fn a_foreign_or_reserved_capability_id_is_not_answered() {
    for capability_id in [
        // Another extension's binding of the same op.
        "slack.send_message",
        // A reserved standard op that has not graduated a contract.
        "telegram.forward_message",
        // The channel surface's own identity, not a tool.
        "telegram",
        // Namespace squatting: `telegramx.` must not match `telegram.`.
        "telegramx.whoami",
        "",
    ] {
        assert_eq!(bound_op(capability_id), None, "{capability_id}");
    }
}

/// The capability ids this adapter answers are exactly the ids the binding
/// rules force: `telegram.<op_name>`, using the canonical op name verbatim.
#[test]
fn bound_ids_are_the_extension_namespaced_canonical_op_names() {
    for op in StandardMessagingOp::ALL {
        let capability_id = format!("telegram.{}", op.op_name());
        if let Some(bound) = bound_op(&capability_id) {
            assert_eq!(bound, *op);
            assert!(
                op.contract().is_some(),
                "{} is reserved and must not bind",
                op.op_name()
            );
        }
    }
}

/// A write op that fails must not look like a write op that succeeded, and an
/// unroutable id must not look like either.
#[test]
fn an_unroutable_capability_reports_an_undeclared_capability() {
    let error = undeclared_capability("telegram.get_thread_replies");
    let ToolError::Failed { kind, .. } = error else {
        panic!("an unknown capability is not an auth problem");
    };
    assert_eq!(
        kind,
        ironclaw_host_api::dispatch::RuntimeDispatchErrorKind::UndeclaredCapability
    );
}
