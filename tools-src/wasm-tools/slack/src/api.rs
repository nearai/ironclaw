//! Slack Web API implementation.
//!
//! All API calls go through the host's HTTP capability, which handles
//! credential injection and rate limiting. The WASM tool never sees
//! the actual bot token.

use crate::near::agent::host;
use crate::types::*;

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Make a Slack API call.
fn slack_api_call(method: &str, endpoint: &str, body: Option<&str>) -> Result<String, String> {
    let url = format!("{}/{}", SLACK_API_BASE, endpoint);

    // Content-Type header for POST requests
    let headers = if body.is_some() {
        r#"{"Content-Type": "application/json; charset=utf-8"}"#
    } else {
        "{}"
    };

    let body_bytes = body.map(|b| b.as_bytes().to_vec());

    host::log(host::LogLevel::Debug, &format!("Slack API: {} {}", method, endpoint));

    let response = host::http_request(method, &url, headers, body_bytes.as_deref())?;

    if response.status < 200 || response.status >= 300 {
        return Err(format!(
            "Slack API returned status {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }

    String::from_utf8(response.body).map_err(|e| format!("Invalid UTF-8 in response: {}", e))
}

/// Send a message to a Slack channel.
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
    let response = slack_api_call("POST", "chat.postMessage", Some(&body))?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    if !parsed["ok"].as_bool().unwrap_or(false) {
        let error = parsed["error"].as_str().unwrap_or("unknown_error");
        return Err(format!("Slack API error: {}", error));
    }

    Ok(SendMessageResult {
        ok: true,
        channel: parsed["channel"].as_str().unwrap_or(channel).to_string(),
        ts: parsed["ts"].as_str().unwrap_or("").to_string(),
        message: parsed.get("message").map(|m| MessageInfo {
            text: m["text"].as_str().unwrap_or("").to_string(),
            user: m["user"].as_str().map(|s| s.to_string()),
            ts: m["ts"].as_str().unwrap_or("").to_string(),
        }),
    })
}

/// List channels the bot has access to.
pub fn list_channels(limit: u32) -> Result<ListChannelsResult, String> {
    let url = format!(
        "conversations.list?types=public_channel,private_channel&limit={}",
        limit
    );

    let response = slack_api_call("GET", &url, None)?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    if !parsed["ok"].as_bool().unwrap_or(false) {
        let error = parsed["error"].as_str().unwrap_or("unknown_error");
        return Err(format!("Slack API error: {}", error));
    }

    let channels = parsed["channels"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| Channel {
                    id: c["id"].as_str().unwrap_or("").to_string(),
                    name: c["name"].as_str().unwrap_or("").to_string(),
                    is_private: c["is_private"].as_bool().unwrap_or(false),
                    is_member: c["is_member"].as_bool().unwrap_or(false),
                    topic: c["topic"]["value"].as_str().map(|s| s.to_string()),
                    purpose: c["purpose"]["value"].as_str().map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ListChannelsResult { ok: true, channels })
}

/// Get message history from a channel.
pub fn get_channel_history(channel: &str, limit: u32) -> Result<ChannelHistoryResult, String> {
    let url = format!("conversations.history?channel={}&limit={}", channel, limit);

    let response = slack_api_call("GET", &url, None)?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    if !parsed["ok"].as_bool().unwrap_or(false) {
        let error = parsed["error"].as_str().unwrap_or("unknown_error");
        return Err(format!("Slack API error: {}", error));
    }

    let messages = parsed["messages"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|m| HistoryMessage {
                    ts: m["ts"].as_str().unwrap_or("").to_string(),
                    text: m["text"].as_str().unwrap_or("").to_string(),
                    user: m["user"].as_str().map(|s| s.to_string()),
                    msg_type: m["type"].as_str().unwrap_or("message").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ChannelHistoryResult { ok: true, messages })
}

/// Add a reaction to a message.
pub fn post_reaction(channel: &str, timestamp: &str, emoji: &str) -> Result<PostReactionResult, String> {
    let payload = serde_json::json!({
        "channel": channel,
        "timestamp": timestamp,
        "name": emoji,
    });

    let body = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    let response = slack_api_call("POST", "reactions.add", Some(&body))?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    if !parsed["ok"].as_bool().unwrap_or(false) {
        let error = parsed["error"].as_str().unwrap_or("unknown_error");
        // "already_reacted" is not really an error
        if error != "already_reacted" {
            return Err(format!("Slack API error: {}", error));
        }
    }

    Ok(PostReactionResult { ok: true })
}

/// Get information about a user.
pub fn get_user_info(user_id: &str) -> Result<GetUserInfoResult, String> {
    let url = format!("users.info?user={}", user_id);

    let response = slack_api_call("GET", &url, None)?;

    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Failed to parse response: {}", e))?;

    if !parsed["ok"].as_bool().unwrap_or(false) {
        let error = parsed["error"].as_str().unwrap_or("unknown_error");
        return Err(format!("Slack API error: {}", error));
    }

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
