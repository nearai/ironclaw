//! The five bound read operations.
//!
//! Everything these return is **attacker-controlled text**. A personal Telegram
//! account can be messaged by any Telegram user on earth, unsolicited and for
//! free, and these ops surface that text into a model context that also holds
//! `send_message` as the user (PROPOSAL §6.4). Three consequences show up in
//! this file: text is sanitized before it leaves ([`mapping::sanitize_untrusted_text`]),
//! volume is bounded on three axes (per message, per page, per result), and the
//! manifest gates every one of these at `automation = "forbidden"`.
//!
//! The residue is disclosed rather than closed: tool results are not yet
//! wrapped in an untrusted-content envelope, so the interactive path — stranger
//! DM → history read → model context holding a write tool — stays open,
//! mitigated only by the write approval card.

use grammers_client::peer::Peer;
use ironclaw_host_api::messaging::StandardMessagingErrorCode;
use serde_json::{Value, json};

use super::{
    ConversationRef, Cursor, OpFamily, PooledSession, ToolError, conversation_arg, cursor_arg,
    input_str, limit_arg, mapping, message_ref_arg, reject_cursor, vendor_call,
};
use crate::linked::MAX_PAGES_PER_CALL;

/// How many vendor rows one call may walk while filling a page of `limit`
/// items ([`MAX_PAGES_PER_CALL`], PROPOSAL §7.2).
fn walk_ceiling(limit: usize) -> usize {
    limit.saturating_mul(MAX_PAGES_PER_CALL)
}

/// `list_conversations` — the linked account's dialogs, most recent first.
///
/// **Not paginated.** grammers' dialog iterator exposes no offset this adapter
/// can carry across calls, so no `next_cursor` is ever emitted and a supplied
/// cursor is rejected. Silently ignoring one would return page one under the
/// name of page two.
pub(crate) async fn list_conversations(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    reject_cursor(input)?;
    let limit = limit_arg(input);
    let wanted_kinds: Option<Vec<String>> =
        input.get("kinds").and_then(Value::as_array).map(|kinds| {
            kinds
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        });

    let client = session.connection().client();
    let ceiling = walk_ceiling(limit);
    let mut iterator = client.iter_dialogs().limit(ceiling);
    let mut items = Vec::new();
    let mut walked = 0usize;

    while items.len() < limit && walked < ceiling {
        session.check_fence()?;
        session.touch();
        let dialog = match iterator.next().await {
            Ok(Some(dialog)) => dialog,
            Ok(None) => break,
            Err(error) => return Err(mapping::map_vendor_error(OpFamily::Read, &error)),
        };
        walked += 1;
        // `Dialog::peer_ref` unwraps an `Option` internally; `to_ref` is the
        // fallible sibling and falls back to the session's peer cache. A peer
        // this session cannot reference is skipped rather than panicked on.
        let Some(peer_ref) = dialog.peer().to_ref().await.ok().flatten() else {
            continue;
        };
        let value = mapping::conversation_value(dialog.peer(), peer_ref);
        if let Some(kinds) = &wanted_kinds
            && !kinds
                .iter()
                .any(|kind| value.get("kind").and_then(Value::as_str) == Some(kind.as_str()))
        {
            continue;
        }
        items.push(value);
    }

    let clamped = mapping::bound_result_bytes(&mut items);
    let mut output = json!({ "conversations": items });
    if clamped {
        output["vendor"] = json!({ "result_truncated": true });
    }
    Ok(output)
}

/// `get_conversation_info` — one conversation, resolved live.
///
/// `counterpart` is required whenever `kind == "dm"`. The canonical schema
/// declares that conditional only on `list_conversations` items, so it is
/// enforced in adapter code for both ops
/// ([`mapping::conversation_value`] is the single place that decides).
pub(crate) async fn get_conversation_info(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let conversation = conversation_arg(input, "conversation")?;
    let peer_ref = conversation.peer_ref();
    let client = session.connection().client();
    let peer = vendor_call(session, OpFamily::Read, || client.resolve_peer(peer_ref)).await?;
    // Prefer the freshly resolved reference: a migrated or re-hashed peer's
    // live auth is better evidence than the one the caller was holding.
    let peer_ref = peer.to_ref().await.ok().flatten().unwrap_or(peer_ref);
    Ok(mapping::conversation_value(&peer, peer_ref))
}

/// `get_conversation_history` — newest first, thread replies included.
///
/// The inclusion is a documented deviation: the canonical description says
/// history excludes thread replies "where available", and on Telegram they are
/// not separable — replies live in the same chat history. The vendor addendum
/// states it, and `get_thread_replies` is left unbound rather than shipped as a
/// duplicate of this op.
pub(crate) async fn get_conversation_history(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let conversation = conversation_arg(input, "conversation")?;
    let limit = limit_arg(input);
    let cursor = cursor_arg(input)?;
    let client = session.connection().client();
    let ceiling = walk_ceiling(limit);

    let mut iterator = client.iter_messages(conversation.peer_ref()).limit(ceiling);
    if let Some(cursor) = cursor {
        iterator = iterator.offset_id(cursor.offset_id);
        if cursor.offset_date != 0 {
            iterator = iterator.offset_date(cursor.offset_date);
        }
    }

    let mut items = Vec::new();
    let mut walked = 0usize;
    let mut oldest: Option<(i32, i32)> = None;

    while items.len() < limit && walked < ceiling {
        session.check_fence()?;
        session.touch();
        let message = match iterator.next().await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(error) => return Err(mapping::map_vendor_error(OpFamily::Read, &error)),
        };
        walked += 1;
        oldest = Some((message.id(), message.date().timestamp() as i32));
        // `None` is a pure service message (join, leave, pin, title change):
        // no author-authored content, and Telegram emits them constantly.
        if let Some(value) = mapping::message_value(&conversation, &message).await {
            items.push(value);
        }
    }

    let clamped = mapping::bound_result_bytes(&mut items);
    let mut output = json!({ "messages": items });
    // Only offer to continue when the page actually filled: a short page is the
    // end of the history, and a cursor there would invite a pointless round
    // trip that returns nothing.
    if !clamped
        && output
            .get("messages")
            .and_then(Value::as_array)
            .is_some_and(|messages| messages.len() == limit)
        && let Some((offset_id, offset_date)) = oldest
    {
        output["next_cursor"] = json!(
            Cursor {
                offset_id,
                offset_date,
            }
            .encode()
        );
    }
    if clamped {
        output["vendor"] = json!({ "result_truncated": true });
    }
    Ok(output)
}

/// `get_message` — one message by the exact ref an earlier call returned.
pub(crate) async fn get_message(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let (conversation, message_id) = message_ref_arg(input)?;
    let peer = conversation.peer_ref();
    let client = session.connection().client();
    let ids = [message_id];
    let ids = &ids;
    let fetched = vendor_call(session, OpFamily::Read, || {
        client.get_messages_by_id(peer, ids)
    })
    .await?;
    let message = fetched
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| mapping::failed(StandardMessagingErrorCode::UnknownMessage))?;
    // A service message exists but has no canonical representation — saying so
    // is more honest than reporting the message as missing.
    let value = mapping::message_value(&conversation, &message)
        .await
        .ok_or_else(|| {
            mapping::failed_because(
                StandardMessagingErrorCode::UnsupportedContent,
                "that telegram message is a service notice with no readable content".to_string(),
            )
        })?;
    Ok(json!({ "message": value }))
}

/// `search_messages` — global search across every chat the account can see.
///
/// The canonical input is closed (`query`/`sort`/`limit`/`cursor`,
/// `additionalProperties: false`), so it carries no conversation and no date
/// bound; per-chat and date-bounded search cannot ride this op and would have
/// to ship as bespoke tools with their own schemas. `sort` is accepted and
/// echoed honestly: Telegram's global search returns newest-first regardless,
/// so a `relevance` request does not silently produce a differently ordered
/// result under that name.
pub(crate) async fn search_messages(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let query = input_str(input, "query")?;
    let limit = limit_arg(input);
    let cursor = cursor_arg(input)?;
    let client = session.connection().client();
    let ceiling = walk_ceiling(limit);

    let mut iterator = client.search_all_messages().query(query).limit(ceiling);
    if let Some(cursor) = cursor {
        iterator = iterator.offset_id(cursor.offset_id);
    }

    let mut items = Vec::new();
    let mut walked = 0usize;
    let mut oldest: Option<(i32, i32)> = None;

    while items.len() < limit && walked < ceiling {
        session.check_fence()?;
        session.touch();
        let message = match iterator.next().await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(error) => return Err(mapping::map_vendor_error(OpFamily::Read, &error)),
        };
        walked += 1;
        oldest = Some((message.id(), message.date().timestamp() as i32));
        // A match whose own conversation cannot be referenced is dropped: a
        // message_ref without a usable conversation half is not something the
        // model could ever spend.
        let Some(peer_ref) = message.peer_ref().await.ok().flatten() else {
            continue;
        };
        let conversation = ConversationRef::from_peer_ref(peer_ref);
        if let Some(value) = mapping::message_value(&conversation, &message).await {
            items.push(value);
        }
    }

    let clamped = mapping::bound_result_bytes(&mut items);
    let filled = items.len() == limit;
    let mut output = json!({
        "matches": items,
        "vendor": { "scope": "global", "order": "newest_first" },
    });
    if !clamped
        && filled
        && let Some((offset_id, offset_date)) = oldest
    {
        output["next_cursor"] = json!(
            Cursor {
                offset_id,
                offset_date,
            }
            .encode()
        );
    }
    if clamped {
        output["vendor"]["result_truncated"] = json!(true);
    }
    Ok(output)
}

/// Whether a peer is a person (used by the people ops to refuse a group).
pub(crate) fn user_peer(peer: &Peer) -> Option<&grammers_client::peer::User> {
    match peer {
        Peer::User(user) => Some(user),
        _ => None,
    }
}
