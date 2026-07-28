//! Slack Web API implementation for the personal (user-token) tool.
//!
//! All API calls go through the host's HTTP capability, which injects the
//! `slack_user_token` secret as a bearer token and scans responses for
//! leaks. The WASM tool never sees the actual token.

use crate::near::agent::host;
use crate::types::*;

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Emit the host runtime's structured guest-error contract
/// (`StructuredWasmGuestError { code, kind }`, parsed in
/// `crates/ironclaw_host_runtime/src/services/wasm_execution.rs`) so a Slack
/// failure keeps its actionable error code instead of collapsing to a generic
/// "the tool operation failed". `kind` must be one of the host parser's enum
/// values: `auth_required` | `input` | `output_too_large` | `executor` |
/// `network_denied` | `client` | `operation_failed`.
fn structured_error(code: &str, kind: &'static str) -> String {
    // Slack error codes are snake_case ASCII identifiers; reduce to that shape
    // so a hostile response body cannot smuggle free text into the error
    // channel (the host sanitizes again before anything reaches the model).
    let code: String = code
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
        .take(64)
        .collect();
    let code = if code.is_empty() {
        "unknown_error".to_string()
    } else {
        code
    };
    serde_json::json!({ "code": code, "kind": kind }).to_string()
}

/// Map a Slack `ok:false` error code onto the host's structured error kinds.
///
/// The explicit auth list is matched before the `invalid_` prefix rule so
/// `invalid_auth` gates on re-authentication rather than reading as bad input.
fn slack_error_kind(code: &str) -> &'static str {
    match code {
        "missing_scope" | "not_authed" | "invalid_auth" | "account_inactive"
        | "token_revoked" => "auth_required",
        "ratelimited" | "rate_limited" => "client",
        "channel_not_found" | "user_not_found" => "input",
        _ if code.starts_with("invalid_") => "input",
        _ => "operation_failed",
    }
}

/// Percent-encode a string for use as a URL query parameter value.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

/// Make a Slack API call and return the parsed JSON value, surfacing
/// Slack's `ok: false` errors as `Err`.
fn slack_api_call(
    method: &str,
    endpoint: &str,
    body: Option<&str>,
) -> Result<serde_json::Value, String> {
    let url = format!("{}/{}", SLACK_API_BASE, endpoint);

    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json; charset=utf-8"}"#
    } else {
        "{}"
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    // Log only the static resource name; the query string carries private
    // lookup data (search terms, DM/channel IDs, pagination timestamps).
    let resource = endpoint.split('?').next().unwrap_or(endpoint);
    host::log(
        host::LogLevel::Debug,
        &format!("Slack API: {} {}", method, resource),
    );

    let response = host::http_request(method, &url, headers, body_bytes.as_deref(), None)?;

    if response.status < 200 || response.status >= 300 {
        // The raw body may carry private data; keep it in debug logs only.
        host::log(
            host::LogLevel::Debug,
            &format!(
                "Slack API {} returned status {}: {}",
                resource,
                response.status,
                String::from_utf8_lossy(&response.body)
            ),
        );
        // Slack signals rate limiting at the HTTP layer (429 + Retry-After);
        // the host's structured error shape carries only {code, kind}, so the
        // Retry-After value cannot ride along.
        if response.status == 429 {
            return Err(structured_error("ratelimited", "client"));
        }
        return Err(structured_error(
            &format!("http_status_{}", response.status),
            "operation_failed",
        ));
    }

    let parsed: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|_| structured_error("invalid_json_response", "operation_failed"))?;

    if !parsed["ok"].as_bool().unwrap_or(false) {
        let error = parsed["error"].as_str().unwrap_or("unknown_error");
        host::log(
            host::LogLevel::Debug,
            &format!("Slack API {} error: {}", resource, error),
        );
        return Err(structured_error(error, slack_error_kind(error)));
    }

    Ok(parsed)
}

/// Search all messages visible to the user token.
pub fn search_messages(
    query: &str,
    count: u32,
    sort: Option<&str>,
    page: Option<u32>,
) -> Result<SearchMessagesResult, String> {
    let count = count.clamp(1, 100);
    let mut url = format!(
        "search.messages?query={}&count={}",
        url_encode(query),
        count
    );
    if let Some(sort) = sort {
        url.push_str(&format!("&sort={}", url_encode(sort)));
    }
    if let Some(page) = page {
        url.push_str(&format!("&page={}", page));
    }

    let parsed = slack_api_call("GET", &url, None)?;

    let messages = &parsed["messages"];
    let total = messages["total"].as_u64().unwrap_or(0);
    let mut matches: Vec<SearchMatch> = messages["matches"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|m| SearchMatch {
                    ts: m["ts"].as_str().unwrap_or("").to_string(),
                    text: m["text"].as_str().unwrap_or("").to_string(),
                    user: m["user"].as_str().map(|s| s.to_string()),
                    user_display_name: None,
                    username: m["username"].as_str().map(|s| s.to_string()),
                    channel_id: m["channel"]["id"].as_str().map(|s| s.to_string()),
                    channel_name: m["channel"]["name"].as_str().map(|s| s.to_string()),
                    thread_ts: m["thread_ts"].as_str().map(|s| s.to_string()),
                    permalink: m["permalink"].as_str().map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();

    // Resolve match authors AND in-text mentions to display names, same
    // contract as history: best-effort, one users.info per distinct id,
    // budget-capped, raw tokens left as-is when unresolved.
    let mut referenced_ids = Vec::new();
    for search_match in &matches {
        if let Some(user_id) = &search_match.user {
            referenced_ids.push(user_id.clone());
        }
        referenced_ids.extend(mention_user_ids(&search_match.text));
    }
    let names = resolve_user_display_names(referenced_ids);
    for search_match in &mut matches {
        if let Some(user_id) = &search_match.user {
            search_match.user_display_name = names.get(user_id).cloned();
        }
        search_match.text = humanize_message_text(&search_match.text, &names);
    }

    Ok(SearchMessagesResult {
        ok: true,
        total,
        matches,
    })
}

/// The synchronous WASM tool resolves names with sequential `users.info`
/// round-trips, so one read is allowed at most this many lookups; first-seen
/// ids win the budget and the rest keep raw ids (the same degraded shape as a
/// failed lookup). The single `auth.test` identity call does NOT count
/// against this budget.
const MAX_USER_NAME_LOOKUPS: usize = 25;

/// Resolve the CONNECTED account via `auth.test`: `(user_id, team_id)`.
fn auth_test() -> Result<(String, Option<String>), String> {
    let parsed = slack_api_call("GET", "auth.test", None)?;
    let user_id = parsed["user_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "auth.test response missing user_id".to_string())?
        .to_string();
    let team_id = parsed["team_id"].as_str().map(|s| s.to_string());
    Ok((user_id, team_id))
}

/// Best-effort connected-account lookup for identity marking on reads: a
/// failing `auth.test` (missing scope, outage) must never break the read
/// itself — identity fields are simply absent.
fn current_user_id_best_effort() -> Option<String> {
    match auth_test() {
        Ok((user_id, _team_id)) => Some(user_id),
        Err(error) => {
            crate::near::agent::host::log(
                crate::near::agent::host::LogLevel::Debug,
                &format!("auth.test identity lookup skipped: {error}"),
            );
            None
        }
    }
}

/// Mark which messages the CONNECTED account authored. `is_current_user` is
/// set only when both the connected identity and the message author are
/// known — never fabricated.
fn mark_current_user_messages(messages: &mut [HistoryMessage], current_user_id: Option<&str>) {
    let Some(current_user_id) = current_user_id else {
        return;
    };
    for message in messages.iter_mut() {
        if let Some(user_id) = &message.user {
            message.is_current_user = Some(user_id == current_user_id);
        }
    }
}

/// Resolve Slack user IDs to human-readable names via `users.info`, one
/// lookup per distinct ID in first-seen order, capped at
/// [`MAX_USER_NAME_LOOKUPS`]. Best-effort by contract: a failing lookup
/// (missing scope, deactivated user, outage) is skipped so read operations
/// never fail because a name could not be resolved.
fn resolve_user_display_names(
    user_ids: impl IntoIterator<Item = String>,
) -> std::collections::HashMap<String, String> {
    let mut distinct = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut skipped_over_budget = 0usize;
    for user_id in user_ids {
        if !seen.insert(user_id.clone()) {
            continue;
        }
        if distinct.len() < MAX_USER_NAME_LOOKUPS {
            distinct.push(user_id);
        } else {
            skipped_over_budget += 1;
        }
    }
    if skipped_over_budget > 0 {
        crate::near::agent::host::log(
            crate::near::agent::host::LogLevel::Debug,
            &format!(
                "users.info budget reached: {skipped_over_budget} distinct users left unresolved"
            ),
        );
    }
    let mut names = std::collections::HashMap::new();
    for user_id in distinct {
        match get_user_info(&user_id) {
            Ok(info) => {
                let display = info
                    .user
                    .display_name
                    .filter(|name| !name.is_empty())
                    .or(info.user.real_name.filter(|name| !name.is_empty()))
                    .unwrap_or(info.user.name);
                if !display.is_empty() {
                    names.insert(user_id, display);
                }
            }
            Err(error) => {
                crate::near::agent::host::log(
                    crate::near::agent::host::LogLevel::Debug,
                    &format!("users.info lookup skipped: {error}"),
                );
            }
        }
    }
    names
}

/// Map Slack's conversation object into the extension's stable output shape.
/// Both list and exact lookup use this mapper so their DM identity semantics
/// cannot drift apart.
fn conversation_from_value(value: &serde_json::Value) -> Conversation {
    Conversation {
        id: value["id"].as_str().unwrap_or("").to_string(),
        name: value["name"].as_str().map(|name| name.to_string()),
        is_channel: value["is_channel"].as_bool().unwrap_or(false),
        is_private: value["is_private"].as_bool().unwrap_or(false),
        is_im: value["is_im"].as_bool().unwrap_or(false),
        is_mpim: value["is_mpim"].as_bool().unwrap_or(false),
        // Absent when Slack omits it (DMs have no membership axis) — never
        // fabricated.
        is_member: value["is_member"].as_bool(),
        user: value["user"].as_str().map(|user| user.to_string()),
        user_display_name: None,
    }
}

/// Resolve a DM counterpart's display name without making the exact
/// conversation lookup fail when `users.info` is unavailable.
fn enrich_conversation_counterpart(conversation: &mut Conversation) {
    if !conversation.is_im {
        return;
    }
    let Some(user_id) = conversation.user.clone() else {
        return;
    };
    let names = resolve_user_display_names([user_id.clone()]);
    conversation.user_display_name = names.get(&user_id).cloned();
}

/// List conversations visible to the user token (channels, DMs, group DMs);
/// `is_member` marks which channels the connected account belongs to.
pub fn list_conversations(
    types: &str,
    limit: u32,
    cursor: Option<&str>,
) -> Result<ListConversationsResult, String> {
    let mut url = format!(
        "conversations.list?types={}&limit={}&exclude_archived=true",
        url_encode(types),
        limit
    );
    if let Some(cursor) = cursor {
        url.push_str(&format!("&cursor={}", url_encode(cursor)));
    }

    let parsed = slack_api_call("GET", &url, None)?;

    let mut conversations: Vec<Conversation> = parsed["channels"]
        .as_array()
        .map(|arr| arr.iter().map(conversation_from_value).collect())
        .unwrap_or_default();

    // DMs carry no `name`; resolve the counterpart's display name so output
    // is human-readable without a follow-up lookup per conversation.
    let counterpart_ids: Vec<String> = conversations
        .iter()
        .filter(|conversation| conversation.is_im)
        .filter_map(|conversation| conversation.user.clone())
        .collect();
    let names = resolve_user_display_names(counterpart_ids);
    for conversation in &mut conversations {
        if let Some(user_id) = &conversation.user {
            conversation.user_display_name = names.get(user_id).cloned();
        }
    }

    // Slack signals "no more pages" with an empty next_cursor.
    let next_cursor = parsed["response_metadata"]["next_cursor"]
        .as_str()
        .filter(|cursor| !cursor.is_empty())
        .map(|cursor| cursor.to_string());

    Ok(ListConversationsResult {
        ok: true,
        conversations,
        next_cursor,
    })
}

/// Retrieve one exact Slack conversation by ID. Unlike
/// `list_conversations`, this response cannot be hidden beyond a model-output
/// preview boundary or confused with another same-name DM.
pub fn get_conversation_info(channel: &str) -> Result<GetConversationInfoResult, String> {
    let url = format!("conversations.info?channel={}", url_encode(channel));
    let parsed = slack_api_call("GET", &url, None)?;
    let mut conversation = conversation_from_value(&parsed["channel"]);
    if conversation.id != channel {
        return Err("Slack API response did not contain the requested conversation".to_string());
    }
    if conversation.is_im
        && !matches!(conversation.user.as_deref(), Some(user_id) if !user_id.is_empty())
    {
        return Err("Slack API DM response did not contain a counterpart user".to_string());
    }
    enrich_conversation_counterpart(&mut conversation);
    Ok(GetConversationInfoResult {
        ok: true,
        conversation,
    })
}

/// Read message history from any conversation (channel, DM, or group DM).
pub fn get_conversation_history(
    channel: &str,
    limit: u32,
    latest: Option<&str>,
    oldest: Option<&str>,
) -> Result<ConversationHistoryResult, String> {
    // Slack rejects limit=1000; 999 is the real maximum.
    let limit = limit.clamp(1, 999);
    let mut url = format!(
        "conversations.history?channel={}&limit={}",
        url_encode(channel),
        limit
    );
    if let Some(latest) = latest {
        url.push_str(&format!("&latest={}", url_encode(latest)));
    }
    if let Some(oldest) = oldest {
        url.push_str(&format!("&oldest={}", url_encode(oldest)));
    }

    let parsed = slack_api_call("GET", &url, None)?;

    Ok(enriched_history_result(&parsed))
}

/// Read the replies of one thread (`conversations.replies`), with the same
/// enrichment contract as history: resolved display names and
/// connected-account marking.
pub fn get_thread_replies(
    channel: &str,
    thread_ts: &str,
    limit: u32,
) -> Result<ConversationHistoryResult, String> {
    // Slack rejects limit=1000; 999 is the real maximum.
    let limit = limit.clamp(1, 999);
    let url = format!(
        "conversations.replies?channel={}&ts={}&limit={}",
        url_encode(channel),
        url_encode(thread_ts),
        limit
    );

    let parsed = slack_api_call("GET", &url, None)?;

    Ok(enriched_history_result(&parsed))
}

/// Shared post-processing for `conversations.history` / `conversations.replies`
/// responses: map messages, resolve authors AND in-text `<@U…>` mentions to
/// display names (one users.info per distinct id, shared budget) so
/// user-facing output never has to echo raw `U…` ids, rewrite Slack control
/// tokens/entities in text, and mark the CONNECTED account's own messages so
/// the model attributes the requester's words to the requester. All
/// enrichments are best-effort: identity/name fields are absent (and tokens
/// stay raw) on failure, and the read still succeeds.
fn enriched_history_result(parsed: &serde_json::Value) -> ConversationHistoryResult {
    let has_more = parsed["has_more"].as_bool().unwrap_or(false);
    let mut messages = history_messages_from_response(parsed);

    // One combined id pool per read: message author first, then the mention
    // tokens inside its text, in reading order — all against the same
    // MAX_USER_NAME_LOOKUPS budget and cache.
    let mut referenced_ids = Vec::new();
    for message in &messages {
        if let Some(user_id) = &message.user {
            referenced_ids.push(user_id.clone());
        }
        referenced_ids.extend(mention_user_ids(&message.text));
    }
    let names = resolve_user_display_names(referenced_ids);
    for message in &mut messages {
        if let Some(user_id) = &message.user {
            message.user_display_name = names.get(user_id).cloned();
        }
        message.text = humanize_message_text(&message.text, &names);
    }

    let current_user_id = current_user_id_best_effort();
    mark_current_user_messages(&mut messages, current_user_id.as_deref());
    ConversationHistoryResult {
        ok: true,
        messages,
        has_more,
        current_user_id,
    }
}

/// Extract the user ids referenced by `<@U…>` / `<@U…|label>` mention tokens
/// inside message text, in order of appearance.
fn mention_user_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('>') else {
            break;
        };
        let id = after[..end].split('|').next().unwrap_or("");
        if is_slack_user_id(id) {
            ids.push(id.to_string());
        }
        rest = &after[end + 1..];
    }
    ids
}

/// Slack user ids are uppercase alphanumeric and start with `U` (or `W` for
/// Enterprise Grid users).
fn is_slack_user_id(id: &str) -> bool {
    (id.starts_with('U') || id.starts_with('W'))
        && id.len() > 1
        && id
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

/// Rewrite Slack control tokens in message text for human consumption
/// (inbound entity hygiene): resolved `<@U…>` / `<@U…|label>` mentions become
/// `@Display Name`; when `users.info` is unavailable but the token carries an
/// inline `|label`, that label is rendered (`@label`) — Slack's own text, not a
/// fabrication — so a labeled mention never leaks its raw `U…` id. A bare
/// unresolved `<@U…>` is left as-is (inventing a name would be fabrication).
/// `<#C…|name>` channel refs become `#name`, other tokens (links, `<!here>`)
/// pass through untouched, and Slack's HTML entities (&lt; &gt; &amp;) are
/// decoded AFTER token rewriting so literal `&lt;@U…&gt;` text never turns
/// into a live token.
fn humanize_message_text(
    text: &str,
    names: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            // Unterminated token: emit the remainder verbatim.
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let token = &after[..end];
        let original = &rest[start..start + end + 2];
        if let Some(mention) = token.strip_prefix('@') {
            // Slack encodes user mentions as `<@U…>` or `<@U…|label>`, where the
            // label is Slack's own inline rendering of that user's name.
            let (id, inline_label) = match mention.split_once('|') {
                Some((id, label)) => (id, Some(label)),
                None => (mention, None),
            };
            match names.get(id) {
                // Prefer the freshly resolved users.info display name.
                Some(name) if is_slack_user_id(id) => {
                    out.push('@');
                    out.push_str(name);
                }
                // users.info was unavailable (missing scope, over budget,
                // outage) but Slack embedded a label in the token: render
                // Slack's own `@label` rather than leak the raw `<@U…|label>`
                // token (which carries the raw `U…` id). The label is provided
                // by Slack, not fabricated — the same inline-label fallback the
                // `<#C…|name>` channel-ref arm below already uses. A bare
                // unresolved `<@U…>` still stays as-is: inventing a name would
                // be fabrication.
                _ => match inline_label.filter(|label| !label.is_empty()) {
                    Some(label) if is_slack_user_id(id) => {
                        out.push('@');
                        out.push_str(label);
                    }
                    _ => out.push_str(original),
                },
            }
        } else if let Some(channel_ref) = token.strip_prefix('#') {
            match channel_ref
                .split_once('|')
                .map(|(_id, label)| label)
                .filter(|label| !label.is_empty())
            {
                Some(label) => {
                    out.push('#');
                    out.push_str(label);
                }
                None => out.push_str(original),
            }
        } else {
            out.push_str(original);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    // &amp; must decode LAST so a literal "&amp;lt;" ends as "&lt;".
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Map a Slack `messages` array (conversations.history / conversations.replies)
/// into [`HistoryMessage`] entries.
fn history_messages_from_response(parsed: &serde_json::Value) -> Vec<HistoryMessage> {
    parsed["messages"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|m| HistoryMessage {
                    ts: m["ts"].as_str().unwrap_or("").to_string(),
                    text: m["text"].as_str().unwrap_or("").to_string(),
                    user: m["user"].as_str().map(|s| s.to_string()),
                    user_display_name: None,
                    is_current_user: None,
                    reply_count: m["reply_count"].as_u64(),
                    msg_type: m["type"].as_str().unwrap_or("message").to_string(),
                    thread_ts: m["thread_ts"].as_str().map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Get information about a user.
pub fn get_user_info(user_id: &str) -> Result<GetUserInfoResult, String> {
    let url = format!("users.info?user={}", url_encode(user_id));

    let parsed = slack_api_call("GET", &url, None)?;

    let user = &parsed["user"];
    let profile = &user["profile"];
    let non_empty = |value: &serde_json::Value| {
        value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(|text| text.to_string())
    };

    Ok(GetUserInfoResult {
        ok: true,
        user: UserInfo {
            id: user["id"].as_str().unwrap_or("").to_string(),
            name: user["name"].as_str().unwrap_or("").to_string(),
            real_name: profile["real_name"].as_str().map(|s| s.to_string()),
            display_name: profile["display_name"].as_str().map(|s| s.to_string()),
            // No email field: the slack_personal OAuth grant has no
            // users:read.email scope, so Slack never returns one.
            is_bot: user["is_bot"].as_bool().unwrap_or(false),
            tz: non_empty(&user["tz"]),
            tz_label: non_empty(&user["tz_label"]),
            title: non_empty(&profile["title"]),
            status_text: non_empty(&profile["status_text"]),
            status_emoji: non_empty(&profile["status_emoji"]),
            // Slack reports 0 for "no expiration"; only a real timestamp is
            // presence-relevant.
            status_expiration: profile["status_expiration"]
                .as_i64()
                .filter(|expiration| *expiration != 0),
        },
    })
}

/// Resolve who the CONNECTED account is: `auth.test` for the user id (the
/// operation itself, so its failure fails the call) plus a best-effort
/// `users.info` for the human-readable name.
pub fn whoami() -> Result<WhoamiResult, String> {
    let (user_id, team_id) = auth_test()?;
    let user_display_name = match get_user_info(&user_id) {
        Ok(info) => info
            .user
            .display_name
            .filter(|name| !name.is_empty())
            .or(info.user.real_name.filter(|name| !name.is_empty()))
            .or(Some(info.user.name))
            .filter(|name| !name.is_empty()),
        Err(error) => {
            crate::near::agent::host::log(
                crate::near::agent::host::LogLevel::Debug,
                &format!("whoami users.info lookup skipped: {error}"),
            );
            None
        }
    };
    Ok(WhoamiResult {
        ok: true,
        user_id,
        user_display_name,
        team_id,
    })
}

/// Send a message as the user.
///
/// Pins `as_user=true`: with a CLASSIC Slack app's user token,
/// `chat.postMessage` defaults to as_user=false and attributes the post to
/// the APP (bot_id/bot_profile) instead of the connected user — the exact
/// wrong-identity failure this tool exists to avoid ("posts as you").
/// Granular (new) apps reject the legacy flag with `as_user_not_supported`;
/// their user-token posts are always authored by the user, so retry exactly
/// once without it.
pub fn send_message(
    channel: &str,
    text: &str,
    thread_ts: Option<&str>,
) -> Result<SendMessageResult, String> {
    let payload = send_message_payload(channel, text, thread_ts, true)?;
    let parsed = match slack_api_call("POST", "chat.postMessage", Some(&payload)) {
        Ok(parsed) => parsed,
        Err(error) if error.contains("as_user_not_supported") => {
            let payload = send_message_payload(channel, text, thread_ts, false)?;
            slack_api_call("POST", "chat.postMessage", Some(&payload))?
        }
        Err(error) => return Err(error),
    };

    Ok(SendMessageResult {
        ok: true,
        channel: parsed["channel"].as_str().unwrap_or(channel).to_string(),
        ts: parsed["ts"].as_str().unwrap_or("").to_string(),
    })
}

fn send_message_payload(
    channel: &str,
    text: &str,
    thread_ts: Option<&str>,
    as_user: bool,
) -> Result<String, String> {
    let mut payload = serde_json::json!({
        "channel": channel,
        "text": text,
    });
    if as_user {
        payload["as_user"] = serde_json::Value::Bool(true);
    }
    if let Some(ts) = thread_ts {
        payload["thread_ts"] = serde_json::Value::String(ts.to_string());
    }
    serde_json::to_string(&payload).map_err(|e| e.to_string())
}
