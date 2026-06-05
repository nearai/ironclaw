//! Outbound rendering for Slack v2.
//!
//! Renders projection-derived final replies into Slack Web API
//! `chat.postMessage` requests. All requests use the host-mediated egress
//! path and carry only a credential handle; the adapter never sees raw bot
//! tokens.

use ironclaw_product_adapters::{
    AuthPromptView, DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod,
    EgressPath, EgressRequest, FinalReplyView, GatePromptView, ProductOutboundTarget,
};
use serde::Serialize;
use thiserror::Error;

use crate::payload::SLACK_API_HOST;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlackRenderError {
    #[error("reply target did not contain a valid Slack channel/thread: {reason}")]
    InvalidReplyTarget { reason: String },
    #[error("failed to serialize Slack chat.postMessage body: {reason}")]
    Serialization { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackReplyTarget {
    pub(crate) channel: String,
    pub(crate) thread_ts: Option<String>,
}

pub(crate) fn slack_reply_target(
    target: &ProductOutboundTarget,
) -> Result<SlackReplyTarget, SlackRenderError> {
    let channel = target.external_conversation_ref.conversation_id();
    if !looks_like_slack_id(channel) {
        return Err(SlackRenderError::InvalidReplyTarget {
            reason: "external conversation id is not a Slack channel/DM id".into(),
        });
    }
    Ok(SlackReplyTarget {
        channel: channel.to_string(),
        thread_ts: target
            .external_conversation_ref
            .topic_id()
            .map(str::to_string),
    })
}

pub fn render_final_reply(
    target: &ProductOutboundTarget,
    view: &FinalReplyView,
    credential_handle: EgressCredentialHandle,
) -> Result<EgressRequest, SlackRenderError> {
    render_text_message(
        target,
        render_slack_mrkdwn(&view.text),
        true,
        credential_handle,
    )
}

pub fn render_gate_prompt(
    target: &ProductOutboundTarget,
    view: &GatePromptView,
    credential_handle: EgressCredentialHandle,
) -> Result<EgressRequest, SlackRenderError> {
    render_text_message(
        target,
        format!(
            "{}\n\n{}\n\n{}",
            view.headline,
            view.body,
            gate_prompt_reply_instruction(target, &view.gate_ref)
        ),
        false,
        credential_handle,
    )
}

pub fn render_auth_prompt(
    target: &ProductOutboundTarget,
    view: &AuthPromptView,
    credential_handle: EgressCredentialHandle,
) -> Result<EgressRequest, SlackRenderError> {
    let mut text = format!(
        "{}\n\n{}\n\n{}",
        view.headline,
        view.body,
        auth_prompt_reply_instruction(target, &view.auth_request_ref)
    );
    if let Some(url) = &view.authorization_url {
        text.push_str("\n\nSetup link: ");
        text.push_str(url);
    }
    render_text_message(target, text, false, credential_handle)
}

fn gate_prompt_reply_instruction(target: &ProductOutboundTarget, gate_ref: &str) -> String {
    if requires_app_mention(target) {
        return format!(
            "Mention this app in this Slack thread with `approve` or `deny`. If the thread has multiple pending approvals, use `approve {gate_ref}` or `deny {gate_ref}`."
        );
    }
    format!(
        "Reply `approve` or `deny` in this Slack thread. If the thread has multiple pending approvals, use `approve {gate_ref}` or `deny {gate_ref}`."
    )
}

fn auth_prompt_reply_instruction(target: &ProductOutboundTarget, auth_request_ref: &str) -> String {
    if requires_app_mention(target) {
        return format!(
            "Mention this app in this Slack thread with `auth deny {auth_request_ref}` to cancel this blocked run."
        );
    }
    format!("Reply `auth deny {auth_request_ref}` to cancel this blocked run.")
}

fn requires_app_mention(target: &ProductOutboundTarget) -> bool {
    !target
        .external_conversation_ref
        .conversation_id()
        .starts_with('D')
}

fn render_text_message(
    target: &ProductOutboundTarget,
    text: String,
    mrkdwn: bool,
    credential_handle: EgressCredentialHandle,
) -> Result<EgressRequest, SlackRenderError> {
    let reply = slack_reply_target(target)?;
    let body = ChatPostMessageRequest {
        channel: reply.channel,
        text,
        mrkdwn,
        thread_ts: reply.thread_ts,
    };
    let body_bytes = serde_json::to_vec(&body).map_err(|err| SlackRenderError::Serialization {
        reason: err.to_string(),
    })?;

    Ok(build_egress_request(
        "/api/chat.postMessage",
        body_bytes,
        credential_handle,
    ))
}

fn render_slack_mrkdwn(markdown: &str) -> String {
    let mut rendered = String::with_capacity(markdown.len());
    let lines = markdown.lines().collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        if is_markdown_table_separator(lines[index]) {
            index += 1;
            continue;
        }
        let line = lines[index];
        let converted = if is_markdown_table_row(line) {
            render_table_row(line)
        } else {
            render_slack_mrkdwn_line(line)
        };
        rendered.push_str(&converted);
        if index + 1 < lines.len() {
            rendered.push('\n');
        }
        index += 1;
    }
    rendered
}

fn render_slack_mrkdwn_line(line: &str) -> String {
    let line = strip_heading_marker(line);
    let line = convert_markdown_links(line);
    convert_markdown_bold(&line)
}

fn strip_heading_marker(line: &str) -> &str {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return line;
    }
    let hash_count = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hash_count) {
        return line;
    }
    let rest = &trimmed[hash_count..];
    let Some(rest) = rest.strip_prefix(' ') else {
        return line;
    };
    rest
}

fn convert_markdown_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < line.len() {
        if bytes[index] == b'['
            && let Some(label_end_rel) = line[index + 1..].find(']')
        {
            let label_end = index + 1 + label_end_rel;
            if line[label_end..].starts_with("](")
                && let Some(url_end_rel) = line[label_end + 2..].find(')')
            {
                let label = &line[index + 1..label_end];
                let url_end = label_end + 2 + url_end_rel;
                let url = &line[label_end + 2..url_end];
                if is_safe_slack_link_url(url) {
                    out.push('<');
                    out.push_str(url);
                    out.push('|');
                    out.push_str(label);
                    out.push('>');
                    index = url_end + 1;
                    continue;
                }
            }
        }
        let Some(ch) = line[index..].chars().next() else {
            break;
        };
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn is_safe_slack_link_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn convert_markdown_bold(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for (index, part) in line.split("**").enumerate() {
        if index > 0 {
            out.push('*');
        }
        out.push_str(part);
    }
    out
}

fn is_markdown_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 2
}

fn is_markdown_table_separator(line: &str) -> bool {
    if !is_markdown_table_row(line) {
        return false;
    }
    line.trim().trim_matches('|').split('|').all(|cell| {
        let cell = cell.trim();
        !cell.is_empty() && cell.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
    })
}

fn render_table_row(line: &str) -> String {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| render_slack_mrkdwn_line(cell.trim()))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[derive(Debug, Serialize)]
struct ChatPostMessageRequest {
    channel: String,
    text: String,
    mrkdwn: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<String>,
}

fn build_egress_request(
    path: &'static str,
    body: Vec<u8>,
    credential_handle: EgressCredentialHandle,
) -> EgressRequest {
    let host = DeclaredEgressHost::new(SLACK_API_HOST).expect("static Slack host valid"); // safety: compile-time const "slack.com" satisfies DeclaredEgressHost validation
    let method = EgressMethod::post();
    let egress_path = EgressPath::new(path).expect("static Slack API path valid"); // safety: only static origin-form Slack Web API paths are passed here
    let content_type = EgressHeader::new("content-type", "application/json")
        .expect("static content-type header valid"); // safety: static name/value satisfies EgressHeader validation
    EgressRequest::new(host, method, egress_path)
        .with_header(content_type)
        .with_body(body)
        .with_credential_handle(Some(credential_handle))
}

fn looks_like_slack_id(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some('C' | 'D' | 'G' | 'U' | 'W') => {
            chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_product_adapters::{ExternalConversationRef, ProductOutboundTarget};
    use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};

    fn handle() -> EgressCredentialHandle {
        EgressCredentialHandle::new("slack_bot_token").expect("valid")
    }

    fn target(channel: &str, thread_ts: Option<&str>) -> ProductOutboundTarget {
        ProductOutboundTarget::new(
            ReplyTargetBindingRef::new("reply:slack-test").expect("valid"),
            ExternalConversationRef::new(Some("T123"), channel, thread_ts, Some("171.1"))
                .expect("valid"),
            None,
        )
    }

    #[test]
    fn final_reply_renders_chat_post_message_with_thread() {
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hello Slack".to_string(),
            generated_at: Utc::now(),
        };

        let request =
            render_final_reply(&target("C123", Some("1710000000.000001")), &view, handle())
                .expect("render");

        assert_eq!(request.host().as_str(), SLACK_API_HOST);
        assert_eq!(request.method().as_str(), "POST");
        assert_eq!(request.path().as_str(), "/api/chat.postMessage");
        assert_eq!(
            request
                .credential_handle()
                .map(EgressCredentialHandle::as_str),
            Some("slack_bot_token")
        );
        let body: serde_json::Value = serde_json::from_slice(request.body()).expect("body json");
        assert_eq!(body["channel"], "C123");
        assert_eq!(body["text"], "hello Slack");
        assert_eq!(body["mrkdwn"], true);
        assert_eq!(body["thread_ts"], "1710000000.000001");
    }

    #[test]
    fn final_reply_renders_common_markdown_as_slack_mrkdwn() {
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "Here are your top Notion docs:\n\n### Top Priority Docs\n\n1. **NEAR AI Engineering Weekly Updates** ([link](https://www.notion.com/p/abc))\n   - \"Multi tenancy for migrating from Railway => top priority\"\n\n| Doc | Highlight |\n|---|---|\n| **Priority Agents** | Priority Agents |"
                .to_string(),
            generated_at: Utc::now(),
        };

        let request = render_final_reply(&target("C123", None), &view, handle()).expect("render");
        let body: serde_json::Value = serde_json::from_slice(request.body()).expect("body json");

        assert_eq!(body["mrkdwn"], true);
        let text = body["text"].as_str().expect("text");
        assert!(text.contains("Top Priority Docs"));
        assert!(!text.contains("###"));
        assert!(text.contains("*NEAR AI Engineering Weekly Updates*"));
        assert!(text.contains("<https://www.notion.com/p/abc|link>"));
        assert!(!text.contains("|---|---|"));
        assert!(text.contains("Doc | Highlight"));
        assert!(text.contains("*Priority Agents* | Priority Agents"));
    }

    #[test]
    fn dm_final_reply_omits_thread_when_absent() {
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hello DM".to_string(),
            generated_at: Utc::now(),
        };

        let request = render_final_reply(&target("D123", None), &view, handle()).expect("render");
        let body: serde_json::Value = serde_json::from_slice(request.body()).expect("body json");
        assert_eq!(body["channel"], "D123");
        assert!(body.get("thread_ts").is_none());
    }

    #[test]
    fn invalid_slack_channel_is_rejected() {
        let err = slack_reply_target(&target("not-a-channel", None)).expect_err("invalid target");
        assert!(matches!(err, SlackRenderError::InvalidReplyTarget { .. }));
    }
}
