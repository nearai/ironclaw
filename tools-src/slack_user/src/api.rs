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

    host::log(
        host::LogLevel::Debug,
        &format!("Slack API: {} {}", method, endpoint),
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

/// List conversations the user belongs to (channels, DMs, group DMs).
pub fn list_conversations(types: &str, limit: u32) -> Result<ListConversationsResult, String> {
    let url = format!(
        "conversations.list?types={}&limit={}&exclude_archived=true",
        url_encode(types),
        limit
    );

    let parsed = slack_api_call("GET", &url, None)?;

    let conversations = parsed["channels"]
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
                })
                .collect()
        })
        .unwrap_or_default();

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
    let messages = parsed["messages"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|m| HistoryMessage {
                    ts: m["ts"].as_str().unwrap_or("").to_string(),
                    text: m["text"].as_str().unwrap_or("").to_string(),
                    user: m["user"].as_str().map(|s| s.to_string()),
                    msg_type: m["type"].as_str().unwrap_or("message").to_string(),
                    thread_ts: m["thread_ts"].as_str().map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ConversationHistoryResult {
        ok: true,
        messages,
        has_more,
    })
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
