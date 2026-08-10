//! The four people operations.
//!
//! One security rule dominates this file and is stated once here.
//! **A display-name or title match is never treated as an identity.** Telegram
//! group titles are settable by any admin and display names by the user
//! themselves, so anyone who shares a chat with the user can rename themselves
//! "Alice". If `resolve_user` auto-resolved a name to a single peer and a write
//! followed, "message Alice" would deliver the user's private message to that
//! attacker, behind an approval card that reads "send to Alice". So matches
//! come back as *candidates* carrying stable identity (the `@username`, the
//! opaque ref, whether the contact is mutual), never as a decision.

use ironclaw_host_api::messaging::StandardMessagingErrorCode;
use serde_json::{Value, json};

use super::{
    OpFamily, PooledSession, ToolError, conversation_arg, input_str, limit_arg, mapping,
    reject_cursor, vendor_call,
};
use crate::linked::mapping::{RefKind, UserRef};

/// Members returned by one `list_members` call.
///
/// Telegram truncates participant listings on large chats and grammers'
/// participant iterator exposes no offset to resume from, so the result is
/// capped here and says so rather than presenting a partial list as complete.
const MAX_MEMBERS_PER_CALL: usize = 200;

/// `whoami` — the linked account itself, the identity every write acts as.
pub(crate) async fn whoami(session: &PooledSession) -> Result<Value, ToolError> {
    let client = session.connection().client();
    let me = vendor_call(session, OpFamily::People, || client.get_me()).await?;
    let peer_ref = me
        .to_ref()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| me.id().to_ambient_ref());
    let value = mapping::user_value(&me, peer_ref);
    // `whoami`'s output allows only `user_ref`, `display_name`, and `vendor`.
    let mut output = json!({ "user_ref": value["user_ref"] });
    if let Some(display_name) = value.get("display_name") {
        output["display_name"] = display_name.clone();
    }
    if let Some(vendor) = value.get("vendor") {
        output["vendor"] = vendor.clone();
    }
    Ok(output)
}

/// `get_user_info` — one person, resolved live.
///
/// `presence` is never emitted: Telegram's last-seen visibility is a per-user
/// privacy setting, so any value would be a guess, and the canonical `unknown`
/// is for vendors that have no presence concept at all.
pub(crate) async fn get_user_info(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    let peer_ref = user_ref_arg(input, "user_ref")?;
    let client = session.connection().client();
    let peer = vendor_call(session, OpFamily::People, || client.resolve_peer(peer_ref)).await?;
    let user = super::reads::user_peer(&peer)
        .ok_or_else(|| mapping::failed(StandardMessagingErrorCode::UnknownUser))?;
    let peer_ref = user.to_ref().await.ok().flatten().unwrap_or(peer_ref);
    Ok(mapping::user_value(user, peer_ref))
}

/// `resolve_user` — free text to candidate people.
///
/// Two sources, deliberately in this order: an exact `@username` lookup (the
/// only match that carries vendor-verified identity), then a scan of the
/// account's own chat list by display name. Name matches are appended, never
/// substituted for the exact one, and the result is a list even when exactly
/// one candidate is found — the caller has to choose.
pub(crate) async fn resolve_user(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    reject_cursor(input)?;
    let query = input_str(input, "query")?;
    let limit = limit_arg(input);
    let needle = query.trim().trim_start_matches('@').to_lowercase();
    if needle.is_empty() {
        return Ok(json!({ "matches": [] }));
    }

    let client = session.connection().client();
    let mut matches = Vec::new();
    let mut candidates = Vec::new();
    let mut seen = Vec::new();

    // 1. Exact @username. `resolve_username` is the flood-prone call in this
    //    package, so it runs once, only for the literal query.
    let resolved = vendor_call(session, OpFamily::People, || {
        client.resolve_username(&needle)
    })
    .await;
    match resolved {
        Ok(Some(peer)) => {
            if let Some(user) = super::reads::user_peer(&peer)
                && let Some(peer_ref) = user.to_ref().await.ok().flatten()
            {
                push_candidate(
                    &mut matches,
                    &mut candidates,
                    &mut seen,
                    mapping::user_value(user, peer_ref),
                    true,
                );
            }
        }
        Ok(None) => {}
        // A username that does not exist is not an error for a free-text
        // query — the dialog scan below may still find the person.
        Err(ToolError::AuthRequired { .. }) => return Err(mapping::auth_required()),
        Err(_) => {}
    }

    // 2. Dialog-title scan. This is the half that must never auto-resolve.
    let ceiling = limit.saturating_mul(crate::linked::MAX_PAGES_PER_CALL);
    let mut iterator = client.iter_dialogs().limit(ceiling);
    let mut walked = 0usize;
    while matches.len() < limit && walked < ceiling {
        session.check_fence()?;
        session.touch();
        let dialog = match iterator.next().await {
            Ok(Some(dialog)) => dialog,
            Ok(None) => break,
            Err(error) => return Err(mapping::map_vendor_error(OpFamily::People, &error)),
        };
        walked += 1;
        let Some(user) = super::reads::user_peer(dialog.peer()) else {
            continue;
        };
        let haystack = format!(
            "{} {}",
            user.full_name(),
            user.username().unwrap_or_default()
        )
        .to_lowercase();
        if !haystack.contains(&needle) {
            continue;
        }
        let Some(peer_ref) = user.to_ref().await.ok().flatten() else {
            continue;
        };
        push_candidate(
            &mut matches,
            &mut candidates,
            &mut seen,
            mapping::user_value(user, peer_ref),
            false,
        );
    }

    mapping::bound_result_bytes(&mut matches);
    Ok(json!({
        "matches": matches,
        // The item schema is closed to `user_ref` + `display_name`, so the
        // stable identity a caller needs to disambiguate safely rides the
        // top-level vendor block instead of being dropped.
        "vendor": {
            "candidates": candidates,
            "disambiguation_required": true,
        },
    }))
}

/// Appends one candidate, de-duplicated by ref.
///
/// `display_name` carries the `@username` inline when there is one, because the
/// approval card renders the match item and a bare attacker-settable name is
/// exactly what must not stand alone there.
fn push_candidate(
    matches: &mut Vec<Value>,
    candidates: &mut Vec<Value>,
    seen: &mut Vec<String>,
    user: Value,
    exact_username_match: bool,
) {
    let Some(user_ref) = user.get("user_ref").and_then(Value::as_str) else {
        return;
    };
    if seen.iter().any(|existing| existing == user_ref) {
        return;
    }
    seen.push(user_ref.to_string());

    let username = user
        .get("vendor")
        .and_then(|vendor| vendor.get("username"))
        .and_then(Value::as_str);
    let display_name = match (user.get("display_name").and_then(Value::as_str), username) {
        (Some(name), Some(username)) => format!("{name} (@{username})"),
        (Some(name), None) => name.to_string(),
        (None, Some(username)) => format!("@{username}"),
        (None, None) => String::new(),
    };

    let mut item = json!({ "user_ref": user_ref });
    if !display_name.is_empty() {
        item["display_name"] = json!(display_name);
    }
    matches.push(item);

    let mut candidate = json!({
        "user_ref": user_ref,
        "matched_by": if exact_username_match { "username" } else { "display_name" },
    });
    if let Some(username) = username {
        candidate["username"] = json!(username);
    }
    if let Some(vendor) = user.get("vendor")
        && let Some(mutual) = vendor.get("mutual_contact")
    {
        candidate["mutual_contact"] = mutual.clone();
    }
    candidates.push(candidate);
}

/// `list_members` — participants of a group or channel.
pub(crate) async fn list_members(
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    reject_cursor(input)?;
    let conversation = conversation_arg(input, "conversation")?;
    let peer_ref = conversation.peer_ref();
    if peer_ref.id.kind() == grammers_client::session::types::PeerKind::User {
        return Err(mapping::failed_because(
            StandardMessagingErrorCode::UnsupportedContent,
            "a telegram one-to-one chat has no member list".to_string(),
        ));
    }
    let limit = limit_arg(input).min(MAX_MEMBERS_PER_CALL);
    let client = session.connection().client();
    let mut iterator = client.iter_participants(peer_ref);
    let mut members = Vec::new();
    let mut more_available = false;

    loop {
        session.check_fence()?;
        session.touch();
        let participant = match iterator.next().await {
            Ok(Some(participant)) => participant,
            Ok(None) => break,
            Err(error) => return Err(mapping::map_vendor_error(OpFamily::People, &error)),
        };
        if members.len() >= limit {
            more_available = true;
            break;
        }
        let Some(peer_ref) = participant.user.to_ref().await.ok().flatten() else {
            continue;
        };
        let value = mapping::user_value(&participant.user, peer_ref);
        let mut item = json!({ "user_ref": value["user_ref"] });
        if let Some(display_name) = value.get("display_name") {
            item["display_name"] = display_name.clone();
        }
        members.push(item);
    }

    let clamped = mapping::bound_result_bytes(&mut members);
    let mut output = json!({ "members": members });
    if more_available || clamped {
        // No `next_cursor`: the participant iterator exposes no offset to
        // resume from, so promising continuation would be a lie. Saying the
        // list is capped is the honest half.
        output["vendor"] = json!({
            "truncated": true,
            "cap": limit,
        });
    }
    Ok(output)
}

/// Reads a `user_ref` argument and refuses the two refs that name nobody: the
/// authorless sentinel a channel post carries, and any ref that is not a user.
fn user_ref_arg(
    input: &Value,
    field: &str,
) -> Result<grammers_client::session::types::PeerRef, ToolError> {
    use grammers_client::session::types::PeerKind;
    let raw = input_str(input, field)?;
    match UserRef::decode(raw) {
        Some(RefKind::Peer(peer_ref)) if peer_ref.id.kind() == PeerKind::User => Ok(peer_ref),
        _ => Err(mapping::failed(StandardMessagingErrorCode::UnknownUser)),
    }
}
