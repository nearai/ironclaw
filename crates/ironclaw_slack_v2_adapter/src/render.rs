//! Outbound rendering for Slack v2.
//!
//! Renders projection-derived final replies into Slack Web API
//! `chat.postMessage` requests. All requests use the host-mediated egress
//! path and carry only a credential handle; the adapter never sees raw bot
//! tokens.

use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath,
    EgressRequest, FinalReplyView, ProductOutboundTarget,
};
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
pub struct SlackReplyTarget {
    pub channel: String,
    pub thread_ts: Option<String>,
}

pub fn slack_reply_target(
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
    let reply = slack_reply_target(target)?;
    let mut body = serde_json::Map::new();
    body.insert("channel".into(), serde_json::Value::String(reply.channel));
    body.insert("text".into(), serde_json::Value::String(view.text.clone()));
    if let Some(thread_ts) = reply.thread_ts {
        body.insert("thread_ts".into(), serde_json::Value::String(thread_ts));
    }
    let body_bytes = serde_json::to_vec(&serde_json::Value::Object(body)).map_err(|err| {
        SlackRenderError::Serialization {
            reason: err.to_string(),
        }
    })?;

    Ok(build_egress_request(
        "/api/chat.postMessage",
        body_bytes,
        credential_handle,
    ))
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
        assert_eq!(body["thread_ts"], "1710000000.000001");
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
