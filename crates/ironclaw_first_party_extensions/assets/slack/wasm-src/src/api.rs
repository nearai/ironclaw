//! Slack Web API implementation for the personal (user-token) tool.
//!
//! All API calls go through the host's HTTP capability, which injects the
//! `slack_user_token` secret as a bearer token and scans responses for
//! leaks. The WASM tool never sees the actual token.

use crate::near::agent::host;
use crate::types::*;

const SLACK_API_BASE: &str = "https://slack.com/api";

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
        return Err(format!(
            "Slack API returned status {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }

    let parsed: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if !parsed["ok"].as_bool().unwrap_or(false) {
        let error = parsed["error"].as_str().unwrap_or("unknown_error");
        return Err(format!("Slack API error: {}", error));
    }

    Ok(parsed)
}

/// Search all messages visible to the user token.
pub fn search_messages(
    query: &str,
    count: u32,
    sort: Option<&str>,
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

    let parsed = slack_api_call("GET", &url, None)?;

    let messages = &parsed["messages"];
    let total = messages["total"].as_u64().unwrap_or(0);
    let matches = messages["matches"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|m| SearchMatch {
                    ts: m["ts"].as_str().unwrap_or("").to_string(),
                    text: m["text"].as_str().unwrap_or("").to_string(),
                    user: m["user"].as_str().map(|s| s.to_string()),
                    username: m["username"].as_str().map(|s| s.to_string()),
                    channel_id: m["channel"]["id"].as_str().map(|s| s.to_string()),
                    channel_name: m["channel"]["name"].as_str().map(|s| s.to_string()),
                    permalink: m["permalink"].as_str().map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();

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

/// List conversations the user belongs to (channels, DMs, group DMs).
pub fn list_conversations(types: &str, limit: u32) -> Result<ListConversationsResult, String> {
    let url = format!(
        "conversations.list?types={}&limit={}&exclude_archived=true",
        url_encode(types),
        limit
    );

    let parsed = slack_api_call("GET", &url, None)?;

    let mut conversations: Vec<Conversation> = parsed["channels"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| Conversation {
                    id: c["id"].as_str().unwrap_or("").to_string(),
                    name: c["name"].as_str().map(|s| s.to_string()),
                    is_channel: c["is_channel"].as_bool().unwrap_or(false),
                    is_private: c["is_private"].as_bool().unwrap_or(false),
                    is_im: c["is_im"].as_bool().unwrap_or(false),
                    is_mpim: c["is_mpim"].as_bool().unwrap_or(false),
                    user: c["user"].as_str().map(|s| s.to_string()),
                    user_display_name: None,
                })
                .collect()
        })
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

    Ok(ListConversationsResult {
        ok: true,
        conversations,
    })
}

/// Read message history from any conversation (channel, DM, or group DM).
pub fn get_conversation_history(
    channel: &str,
    limit: u32,
    latest: Option<&str>,
    oldest: Option<&str>,
) -> Result<ConversationHistoryResult, String> {
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

    let has_more = parsed["has_more"].as_bool().unwrap_or(false);
    let mut messages = history_messages_from_response(&parsed);

    // Resolve authors to display names (one users.info per distinct author)
    // so user-facing output never has to echo raw `U…` ids.
    resolve_message_display_names(&mut messages);

    // Mark the CONNECTED account's own messages so the model attributes the
    // requester's words to the requester. Best-effort: identity fields are
    // absent when auth.test fails, and the read still succeeds.
    let current_user_id = current_user_id_best_effort();
    mark_current_user_messages(&mut messages, current_user_id.as_deref());

    Ok(ConversationHistoryResult {
        ok: true,
        messages,
        has_more,
        current_user_id,
    })
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
                    msg_type: m["type"].as_str().unwrap_or("message").to_string(),
                    thread_ts: m["thread_ts"].as_str().map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve message authors to display names (one `users.info` per distinct
/// author, best-effort, budget-capped).
fn resolve_message_display_names(messages: &mut [HistoryMessage]) {
    let author_ids: Vec<String> = messages
        .iter()
        .filter_map(|message| message.user.clone())
        .collect();
    let names = resolve_user_display_names(author_ids);
    for message in messages.iter_mut() {
        if let Some(user_id) = &message.user {
            message.user_display_name = names.get(user_id).cloned();
        }
    }
}

/// Get information about a user.
pub fn get_user_info(user_id: &str) -> Result<GetUserInfoResult, String> {
    let url = format!("users.info?user={}", url_encode(user_id));

    let parsed = slack_api_call("GET", &url, None)?;

    let user = &parsed["user"];
    let profile = &user["profile"];

    Ok(GetUserInfoResult {
        ok: true,
        user: UserInfo {
            id: user["id"].as_str().unwrap_or("").to_string(),
            name: user["name"].as_str().unwrap_or("").to_string(),
            real_name: profile["real_name"].as_str().map(|s| s.to_string()),
            display_name: profile["display_name"].as_str().map(|s| s.to_string()),
            email: profile["email"].as_str().map(|s| s.to_string()),
            is_bot: user["is_bot"].as_bool().unwrap_or(false),
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
pub fn send_message(
    channel: &str,
    text: &str,
    thread_ts: Option<&str>,
) -> Result<SendMessageResult, String> {
    let mut payload = serde_json::json!({
        "channel": channel,
        "text": text,
    });
    if let Some(ts) = thread_ts {
        payload["thread_ts"] = serde_json::Value::String(ts.to_string());
    }

    let body = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    let parsed = slack_api_call("POST", "chat.postMessage", Some(&body))?;

    Ok(SendMessageResult {
        ok: true,
        channel: parsed["channel"].as_str().unwrap_or(channel).to_string(),
        ts: parsed["ts"].as_str().unwrap_or("").to_string(),
    })
}
