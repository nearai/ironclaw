//! The six write operations, every one of which acts **as the user**.
//!
//! A recipient cannot tell these apart from the person typing on their phone,
//! which is why the manifest gates them at `default_permission = "ask"` with
//! `product` and `automation` forbidden. The code-level obligation that goes
//! with it is narrower and lives here: **nothing in this file may repeat a
//! vendor call, and nothing may report an outcome it does not have.**

use grammers_client::{
    message::{InputMessage, InputReactions},
    session::types::PeerKind,
};
use ironclaw_host_api::messaging::StandardMessagingErrorCode;
use serde_json::Value;

use super::{
    OpFamily, PooledSession, ToolError, conversation_arg, input_str, mapping, message_ref_arg,
    optional_str, vendor_call,
};
use crate::linked::mapping::{RefKind, UserRef};

/// `send_message` — post as the linked account.
///
/// Telegram has one reply mechanism, so the canonical `thread` (post into a
/// container) and `reply_to` (quote one message) both resolve to it;
/// `reply_to` wins when both are supplied, because it names a specific message
/// and is therefore the more precise instruction. Both are echoed back when
/// honored, so a silent drop is checkable.
pub(crate) async fn send_message(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let conversation = conversation_arg(input, "conversation")?;
    let text = input_str(input, "text")?;
    let thread = optional_str(input, "thread");
    let reply_to = input.get("reply_to").filter(|value| value.is_object());

    let reply_target = match reply_to {
        Some(value) => Some(
            mapping::parse_message_ref(value)
                .ok_or_else(|| mapping::failed(StandardMessagingErrorCode::UnknownMessage))?
                .1,
        ),
        None => thread.and_then(|thread| thread.parse::<i32>().ok()),
    };

    let message = InputMessage::new()
        .text(text)
        .reply_to(reply_target)
        // Deliberately off: a link preview turns a message the user asked for
        // into a fetch of whatever URL the text happens to contain.
        .link_preview(false);
    let peer = conversation.peer_ref();
    let client = session.connection().client();
    let sent = vendor_call(session, OpFamily::Write, || {
        client.send_message(peer, message.clone())
    })
    .await?;

    // `id == 0` is grammers saying "Telegram took it, but I could not correlate
    // the response to a message identity". The human already has the message.
    // `send_result` is where that becomes `sent_unverified` rather than a
    // failure the model would retry into a second delivery.
    Ok(mapping::send_result(
        &conversation,
        sent.id(),
        thread,
        reply_to,
    ))
}

/// `edit_message` — the evidence is the ref the caller already holds, because
/// Telegram's edit returns no new identity.
pub(crate) async fn edit_message(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let (conversation, message_id) = message_ref_arg(input)?;
    let text = input_str(input, "text")?;
    let message = InputMessage::new().text(text).link_preview(false);
    let peer = conversation.peer_ref();
    let client = session.connection().client();
    vendor_call(session, OpFamily::Write, || {
        client.edit_message(peer, message_id, message.clone())
    })
    .await?;
    Ok(mapping::ref_only_result(&conversation, message_id))
}

/// `delete_message` — a delete that did not happen is an error, never
/// `deleted: false`.
///
/// The canonical input names exactly one message, so the count Telegram
/// returns is unambiguous: zero means there was nothing to delete, and that is
/// `messaging.unknown_message` rather than a success claim about a message
/// this account never removed.
pub(crate) async fn delete_message(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let (conversation, message_id) = message_ref_arg(input)?;
    let peer = conversation.peer_ref();
    let client = session.connection().client();
    let ids = [message_id];
    let ids = &ids;
    let deleted = vendor_call(session, OpFamily::Write, || {
        client.delete_messages(peer, ids)
    })
    .await?;
    if deleted == 0 {
        return Err(mapping::failed(StandardMessagingErrorCode::UnknownMessage));
    }
    Ok(mapping::delete_result(&conversation, message_id))
}

/// `add_reaction` — `emoji` is the unicode emoji itself; Telegram has no
/// `:shortname:` dialect.
pub(crate) async fn add_reaction(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let (conversation, message_id) = message_ref_arg(input)?;
    let emoji = input_str(input, "emoji")?;
    let reactions = InputReactions::emoticon(emoji);
    let peer = conversation.peer_ref();
    let client = session.connection().client();
    vendor_call(session, OpFamily::Write, || {
        client.send_reactions(peer, message_id, reactions.clone())
    })
    .await?;
    Ok(mapping::add_reaction_result(
        &conversation,
        message_id,
        emoji,
    ))
}

/// `remove_reaction` — clears **all** of the linked account's reactions on the
/// message, and says so by omitting `emoji`.
///
/// Telegram's removal primitive is "set this account's reactions to the empty
/// set"; it cannot remove one named emoji while leaving others. Read-modify-
/// write could narrow that, but it would cost an extra flood-prone read and
/// still race a concurrent reaction. The honest alternative is the one taken
/// here: clear everything and do not echo an `emoji` the operation did not
/// selectively remove. Echoing the requested emoji back would report a
/// precision this call does not have.
pub(crate) async fn remove_reaction(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let (conversation, message_id) = message_ref_arg(input)?;
    let peer = conversation.peer_ref();
    let client = session.connection().client();
    vendor_call(session, OpFamily::Write, || {
        client.send_reactions(peer, message_id, InputReactions::remove())
    })
    .await?;
    Ok(mapping::ref_only_result(&conversation, message_id))
}

/// `open_dm` — re-encodes a user ref as the conversation ref for the
/// one-to-one chat with that user.
///
/// **This performs no vendor call and has no side effect**, which is why it
/// returns no evidence beyond the ref: on Telegram a private conversation is
/// not a resource that gets created, it is the peer itself, and the chat
/// becomes visible to the other person when a message is sent. Resolving a
/// name to a person is `resolve_user`'s job and is the flood-prone half; doing
/// it here would make an innocuous re-encode a rate-limited network call.
pub(crate) fn open_dm(input: &Value) -> Result<Value, ToolError> {
    let raw = input_str(input, "user_ref")?;
    let peer_ref = match UserRef::decode(raw) {
        Some(RefKind::Peer(peer_ref)) if peer_ref.id.kind() == PeerKind::User => peer_ref,
        // A channel post's author ref, a group ref, or anything unparseable.
        // None of them names a person, so none of them can open a DM.
        _ => return Err(mapping::failed(StandardMessagingErrorCode::UnknownUser)),
    };
    Ok(mapping::open_dm_result(peer_ref))
}
