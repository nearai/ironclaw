//! Slack conversation-context fetch over the generic restricted-egress
//! boundary (channel-context hydration for shared-channel pings).
//!
//! A ping inside a thread fetches that thread's replies
//! (`conversations.replies`); a top-level channel ping fetches recent channel
//! history (`conversations.history`). Both use the workspace BOT token the
//! host injects by declared handle, and both degrade to `Ok(None)` on any
//! vendor refusal (`missing_scope`, `not_in_channel`, …) or transport
//! failure — context is advisory and must never fail admission. The
//! formatted text is UNTRUSTED third-party content; the host frames it as
//! quoted context before any model sees it.

use ironclaw_extension_contracts::channel_adapter::{
    ChannelConversationContext, ChannelError, MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES,
};
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::tool_adapter::{RestrictedEgress, RestrictedEgressRequest};
use ironclaw_host_api::{action::NetworkMethod, ids::SecretHandle};
use serde::Deserialize;
use url::Url;

use crate::payload::SLACK_API_HOST;

/// The administrator-configuration handle carrying the bot token (manifest
/// data; the host injects the secret at egress time).
const SLACK_BOT_TOKEN_HANDLE: &str = "slack_bot_token";

/// Thread pings fetch the full thread (Slack returns the root first); capped
/// so one giant thread cannot dominate the fetch.
const THREAD_REPLIES_FETCH_LIMIT: u32 = 100;
/// Top-level pings fetch a modest slice of recent channel history.
const CHANNEL_HISTORY_FETCH_LIMIT: u32 = 30;
/// Per-message character cap; longer messages keep their head plus an
/// ellipsis.
const MAX_MESSAGE_CHARS: usize = 1000;
/// First line of the formatted context when OLDEST messages were dropped to
/// fit the contract byte bound.
const TRUNCATION_MARKER: &str = "(earlier messages truncated)";

#[derive(Debug, Deserialize)]
struct SlackConversationMessagesResponse {
    ok: bool,
    error: Option<String>,
    #[serde(default)]
    messages: Vec<SlackConversationMessage>,
}

#[derive(Debug, Deserialize)]
struct SlackConversationMessage {
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    bot_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

/// Fetch and format recent conversation context for one inbound shared
/// message. `Ok(None)` on every degradable miss; `Err` is never returned —
/// the advisory contract maps all failures to no-context.
pub(super) async fn fetch_conversation_context(
    conversation: &ExternalConversationRef,
    egress: &dyn RestrictedEgress,
) -> Result<Option<ChannelConversationContext>, ChannelError> {
    let request = match conversation_context_request(conversation) {
        Ok(request) => request,
        Err(reason) => {
            tracing::debug!(reason, "slack conversation context request was invalid");
            return Ok(None);
        }
    };
    let in_thread = matches!(
        conversation_context_kind(conversation),
        ConversationContextKind::ThreadReplies { .. }
    );
    let response = match egress.send(request).await {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(
                error = %error,
                "slack conversation context egress failed; continuing without context"
            );
            return Ok(None);
        }
    };
    if !(200..300).contains(&response.status) {
        tracing::debug!(
            status = response.status,
            "slack conversation context fetch returned a non-success status"
        );
        return Ok(None);
    }
    let parsed: SlackConversationMessagesResponse = match serde_json::from_slice(&response.body) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::debug!(
                error = %error,
                "slack conversation context response was not valid JSON"
            );
            return Ok(None);
        }
    };
    if !parsed.ok {
        // e.g. `missing_scope` (channels:history not granted operator-side)
        // or `not_in_channel` — advisory context degrades to none.
        tracing::debug!(
            error = parsed.error.as_deref().unwrap_or("unknown_error"),
            "slack rejected the conversation context fetch"
        );
        return Ok(None);
    }

    let mut messages = parsed.messages;
    // `conversations.history` returns NEWEST-first; `conversations.replies`
    // returns the thread oldest-first (root message first). Normalize to
    // oldest-first.
    if !in_thread {
        messages.reverse();
    }
    match format_conversation_context(&messages) {
        Some(text) => match ChannelConversationContext::new(text) {
            Ok(context) => Ok(Some(context)),
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "formatted slack conversation context failed contract bounds"
                );
                Ok(None)
            }
        },
        None => Ok(None),
    }
}

/// Which vendor context a conversation ref should hydrate.
///
/// The slack adapter roots a TOP-LEVEL mention as its own thread before
/// binding (topic = the ping's own message id), so "inside a thread" is
/// `topic ≠ this message` — when the topic IS this message, the thread was
/// just rooted by this ping and holds nothing, and the useful context is
/// recent CHANNEL history. An existing thread hydrates its replies.
enum ConversationContextKind<'a> {
    ChannelHistory,
    ThreadReplies { root_ts: &'a str },
}

fn conversation_context_kind(
    conversation: &ExternalConversationRef,
) -> ConversationContextKind<'_> {
    match conversation.topic_id() {
        None => ConversationContextKind::ChannelHistory,
        Some(topic) => {
            if conversation.reply_target_message_id() == Some(topic) {
                ConversationContextKind::ChannelHistory
            } else {
                ConversationContextKind::ThreadReplies { root_ts: topic }
            }
        }
    }
}

fn conversation_context_request(
    conversation: &ExternalConversationRef,
) -> Result<RestrictedEgressRequest, String> {
    let credential = SecretHandle::new(SLACK_BOT_TOKEN_HANDLE)
        .map_err(|error| format!("invalid bot token handle: {error}"))?;
    let mut url = match conversation_context_kind(conversation) {
        ConversationContextKind::ThreadReplies { .. } => Url::parse(&format!(
            "https://{SLACK_API_HOST}/api/conversations.replies"
        )),
        ConversationContextKind::ChannelHistory => Url::parse(&format!(
            "https://{SLACK_API_HOST}/api/conversations.history"
        )),
    }
    .map_err(|error| format!("conversation context URL is invalid: {error}"))?;
    match conversation_context_kind(conversation) {
        ConversationContextKind::ThreadReplies { root_ts } => {
            url.query_pairs_mut()
                .append_pair("channel", conversation.conversation_id())
                .append_pair("ts", root_ts)
                .append_pair("limit", &THREAD_REPLIES_FETCH_LIMIT.to_string());
        }
        ConversationContextKind::ChannelHistory => {
            url.query_pairs_mut()
                .append_pair("channel", conversation.conversation_id())
                .append_pair("limit", &CHANNEL_HISTORY_FETCH_LIMIT.to_string());
        }
    }
    Ok(RestrictedEgressRequest {
        method: NetworkMethod::Get,
        url: url.into(),
        headers: Vec::new(),
        body: None,
        credential: Some(credential),
        body_credentials: Vec::new(),
    })
}

/// Format OLDEST-FIRST messages as `<@author>: text` entries joined by
/// newlines, dropping OLDEST entries (behind a marker line) until the result
/// fits the contract byte bound. Empty texts and author-less messages are
/// skipped; bot messages are included (`user` first, `bot_id` fallback).
fn format_conversation_context(oldest_first: &[SlackConversationMessage]) -> Option<String> {
    let entries: Vec<String> = oldest_first
        .iter()
        .filter_map(format_message_entry)
        .collect();
    if entries.is_empty() {
        return None;
    }

    // Conservative running total: every entry counts one joining newline.
    let mut total: usize = entries.iter().map(|entry| entry.len() + 1).sum();
    let budget_with_marker =
        MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES.saturating_sub(TRUNCATION_MARKER.len() + 1);
    let mut start = 0usize;
    while start < entries.len() {
        let budget = if start == 0 {
            MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES
        } else {
            budget_with_marker
        };
        if total <= budget {
            break;
        }
        total -= entries[start].len() + 1;
        start += 1;
    }
    if start >= entries.len() {
        return None;
    }

    let mut lines: Vec<&str> = Vec::with_capacity(entries.len() - start + 1);
    if start > 0 {
        lines.push(TRUNCATION_MARKER);
    }
    lines.extend(entries[start..].iter().map(String::as_str));
    Some(lines.join("\n"))
}

fn format_message_entry(message: &SlackConversationMessage) -> Option<String> {
    let text = message.text.as_deref().map(str::trim).unwrap_or("");
    if text.is_empty() {
        return None;
    }
    let author = message
        .user
        .as_deref()
        .filter(|value| !value.is_empty())
        .or(message.bot_id.as_deref().filter(|value| !value.is_empty()))?;
    Some(format!("<@{author}>: {}", truncate_message_text(text)))
}

/// Cap one message at [`MAX_MESSAGE_CHARS`] characters (ellipsis when cut)
/// and drop control characters other than `\n`/`\t` (`\r\n` folds to `\n`).
fn truncate_message_text(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len().min(MAX_MESSAGE_CHARS + 4));
    let mut characters = text.chars().peekable();
    let mut kept = 0usize;
    let mut cut = false;
    while let Some(character) = characters.next() {
        if kept >= MAX_MESSAGE_CHARS {
            cut = true;
            break;
        }
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                sanitized.push('\n');
                kept += 1;
            }
            '\n' | '\t' => {
                sanitized.push(character);
                kept += 1;
            }
            character if character.is_control() => {}
            character => {
                sanitized.push(character);
                kept += 1;
            }
        }
    }
    if cut {
        sanitized.push('…');
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use ironclaw_extension_contracts::tool_adapter::{
        RestrictedEgressError, RestrictedEgressResponse,
    };
    use std::sync::Mutex;

    use super::*;

    struct ScriptedEgress {
        requests: Mutex<Vec<RestrictedEgressRequest>>,
        response: Result<RestrictedEgressResponse, RestrictedEgressError>,
    }

    impl ScriptedEgress {
        fn json(body: &str) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                response: Ok(RestrictedEgressResponse {
                    status: 200,
                    body: body.as_bytes().to_vec(),
                }),
            }
        }

        fn failing() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                response: Err(RestrictedEgressError::PolicyDenied),
            }
        }

        fn recorded_urls(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .map(|request| request.url.clone())
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl RestrictedEgress for ScriptedEgress {
        async fn send(
            &self,
            request: RestrictedEgressRequest,
        ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request);
            match &self.response {
                Ok(response) => Ok(RestrictedEgressResponse {
                    status: response.status,
                    body: response.body.clone(),
                }),
                Err(error) => Err(error.clone()),
            }
        }
    }

    fn channel_conversation() -> ExternalConversationRef {
        ExternalConversationRef::new(Some("T1"), "C123", None, None).expect("conversation")
    }

    fn thread_conversation() -> ExternalConversationRef {
        ExternalConversationRef::new(Some("T1"), "C123", Some("1712.001"), None)
            .expect("conversation")
    }

    #[tokio::test]
    async fn top_level_ping_fetches_channel_history_and_formats_oldest_first() {
        // conversations.history returns NEWEST-first.
        let egress = ScriptedEgress::json(
            r#"{"ok":true,"messages":[
                {"user":"U3","text":"newest"},
                {"bot_id":"B9","text":"from the bot"},
                {"user":"U1","text":"oldest"},
                {"user":"U4","text":"   "}
            ]}"#,
        );

        let context = fetch_conversation_context(&channel_conversation(), &egress)
            .await
            .expect("advisory fetch never errors")
            .expect("context present");

        assert_eq!(
            context.text,
            "<@U1>: oldest\n<@B9>: from the bot\n<@U3>: newest"
        );
        let urls = egress.recorded_urls();
        assert_eq!(urls.len(), 1);
        assert!(
            urls[0].starts_with("https://slack.com/api/conversations.history?"),
            "unexpected URL: {}",
            urls[0]
        );
        assert!(urls[0].contains("channel=C123"));
        assert!(urls[0].contains("limit=30"));

        let request = &egress
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())[0];
        assert_eq!(request.method, NetworkMethod::Get);
        assert_eq!(
            request.credential.as_ref().map(|handle| handle.as_str()),
            Some("slack_bot_token")
        );
    }

    /// The slack adapter roots a TOP-LEVEL mention as its own thread before
    /// binding (topic == the ping's own message id). That thread holds
    /// nothing yet, so hydration must fetch recent CHANNEL history — the
    /// replies branch would return only the ping itself.
    #[tokio::test]
    async fn mention_that_rooted_its_own_thread_fetches_channel_history() {
        let egress = ScriptedEgress::json(
            r#"{"ok":true,"messages":[
                {"user":"U2","text":"deploy went out at noon"},
                {"user":"U1","text":"morning standup notes"}
            ]}"#,
        );
        let rooted =
            ExternalConversationRef::new(Some("T1"), "C123", Some("1719.100"), Some("1719.100"))
                .expect("conversation");

        let context = fetch_conversation_context(&rooted, &egress)
            .await
            .expect("advisory fetch never errors")
            .expect("context present");

        assert_eq!(
            context.text,
            "<@U1>: morning standup notes\n<@U2>: deploy went out at noon"
        );
        let urls = egress.recorded_urls();
        assert!(
            urls[0].starts_with("https://slack.com/api/conversations.history?"),
            "a rooted-by-this-ping topic must hydrate channel history: {}",
            urls[0]
        );
        assert!(urls[0].contains("limit=30"));
    }

    #[tokio::test]
    async fn thread_ping_fetches_replies_keeping_root_first_order() {
        // conversations.replies returns the thread OLDEST-first (root first).
        let egress = ScriptedEgress::json(
            r#"{"ok":true,"messages":[
                {"user":"U1","text":"thread root"},
                {"user":"U2","text":"first reply"},
                {"user":"U3","text":"second reply"}
            ]}"#,
        );

        let context = fetch_conversation_context(&thread_conversation(), &egress)
            .await
            .expect("advisory fetch never errors")
            .expect("context present");

        assert_eq!(
            context.text,
            "<@U1>: thread root\n<@U2>: first reply\n<@U3>: second reply"
        );
        let urls = egress.recorded_urls();
        assert!(
            urls[0].starts_with("https://slack.com/api/conversations.replies?"),
            "unexpected URL: {}",
            urls[0]
        );
        assert!(urls[0].contains("channel=C123"));
        assert!(urls[0].contains("ts=1712.001"));
        assert!(urls[0].contains("limit=100"));
    }

    #[tokio::test]
    async fn vendor_refusal_degrades_to_no_context() {
        for error in ["missing_scope", "not_in_channel"] {
            let egress = ScriptedEgress::json(&format!(r#"{{"ok":false,"error":"{error}"}}"#));
            let context = fetch_conversation_context(&channel_conversation(), &egress)
                .await
                .expect("advisory fetch never errors");
            assert!(context.is_none(), "`{error}` must degrade to no context");
        }
    }

    #[tokio::test]
    async fn transport_failure_and_invalid_json_degrade_to_no_context() {
        let failing = ScriptedEgress::failing();
        assert!(
            fetch_conversation_context(&channel_conversation(), &failing)
                .await
                .expect("advisory fetch never errors")
                .is_none()
        );

        let invalid = ScriptedEgress::json("not json");
        assert!(
            fetch_conversation_context(&channel_conversation(), &invalid)
                .await
                .expect("advisory fetch never errors")
                .is_none()
        );

        let empty = ScriptedEgress::json(r#"{"ok":true,"messages":[]}"#);
        assert!(
            fetch_conversation_context(&channel_conversation(), &empty)
                .await
                .expect("advisory fetch never errors")
                .is_none()
        );
    }

    #[test]
    fn per_message_truncation_caps_at_one_thousand_chars_with_ellipsis() {
        let long = "a".repeat(MAX_MESSAGE_CHARS + 50);
        let truncated = truncate_message_text(&long);
        assert_eq!(truncated.chars().count(), MAX_MESSAGE_CHARS + 1);
        assert!(truncated.ends_with('…'));

        assert_eq!(truncate_message_text("short"), "short");
        assert_eq!(truncate_message_text("a\r\nb\u{0007}c"), "a\nbc");
    }

    #[test]
    fn oversized_context_drops_oldest_entries_behind_a_marker() {
        let filler = "f".repeat(MAX_MESSAGE_CHARS);
        let mut messages: Vec<SlackConversationMessage> = (0..40)
            .map(|index| SlackConversationMessage {
                user: Some(format!("U{index}")),
                bot_id: None,
                text: Some(filler.clone()),
            })
            .collect();
        messages.push(SlackConversationMessage {
            user: Some("ULAST".to_string()),
            bot_id: None,
            text: Some("the newest message".to_string()),
        });

        let formatted = format_conversation_context(&messages).expect("formatted context");
        assert!(formatted.len() <= MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES);
        assert!(formatted.starts_with(TRUNCATION_MARKER));
        assert!(formatted.ends_with("<@ULAST>: the newest message"));
        assert!(
            !formatted.contains("<@U0>:"),
            "oldest entries must be dropped first"
        );
    }

    #[test]
    fn authorless_messages_are_skipped_and_empty_sets_yield_none() {
        let messages = vec![SlackConversationMessage {
            user: None,
            bot_id: None,
            text: Some("subtype-only event".to_string()),
        }];
        assert!(format_conversation_context(&messages).is_none());
        assert!(format_conversation_context(&[]).is_none());
    }
}
