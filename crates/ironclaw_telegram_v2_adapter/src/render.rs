//! Outbound rendering for Telegram v2.
//!
//! Renders projection-derived payloads into Telegram Bot API egress requests.
//! All requests target the declared `api.telegram.org` host and use the
//! adapter's egress credential handle (the host resolves it to the bot
//! token at request time).

use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath,
    EgressRequest, FinalReplyView, ProgressKind, ProgressUpdateView,
};
use ironclaw_turns::ReplyTargetBindingRef;
use thiserror::Error;

use crate::payload::TELEGRAM_API_HOST;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelegramRenderError {
    #[error("reply target {target} did not parse as Telegram chat#message: {reason}")]
    InvalidReplyTarget { target: String, reason: String },
}

/// Reply-target encoding used by Telegram outbound. The workflow stores the
/// canonical reply target binding ref using the convention
/// `tg:<chat_id>:<topic_id>:<reply_message_id>`. The `topic_id` segment is
/// optional; absence is encoded as `_`.
pub fn parse_reply_target(
    target: &ReplyTargetBindingRef,
) -> Result<TelegramReplyTarget, TelegramRenderError> {
    let raw = target.as_str();
    let stripped = raw
        .strip_prefix("tg:")
        .ok_or(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "missing tg: prefix".into(),
        })?;
    let mut segments = stripped.split(':');
    let chat_id = segments
        .next()
        .ok_or(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "missing chat_id segment".into(),
        })?;
    let topic_segment = segments
        .next()
        .ok_or(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "missing topic segment".into(),
        })?;
    let reply_msg = segments
        .next()
        .ok_or(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "missing reply_message_id segment".into(),
        })?;
    let chat_id_num: i64 = chat_id.parse().map_err(|err: std::num::ParseIntError| {
        TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: format!("chat_id parse: {err}"),
        }
    })?;
    let topic_id = if topic_segment == "_" {
        None
    } else {
        Some(topic_segment.parse::<i64>().map_err(|err| {
            TelegramRenderError::InvalidReplyTarget {
                target: raw.to_string(),
                reason: format!("topic_id parse: {err}"),
            }
        })?)
    };
    let reply_msg_id: Option<i64> = if reply_msg == "_" {
        None
    } else {
        Some(reply_msg.parse().map_err(|err: std::num::ParseIntError| {
            TelegramRenderError::InvalidReplyTarget {
                target: raw.to_string(),
                reason: format!("reply_message_id parse: {err}"),
            }
        })?)
    };
    // Copilot's review: reject any reply target with more than three
    // colon-separated segments after the `tg:` prefix. The encoding is
    // exactly `tg:<chat_id>:<topic_id>:<reply_message_id>`; silently
    // ignoring trailing segments (`tg:1:_:2:extra`) would let corrupted
    // data pass parse and make the encoding ambiguous.
    if segments.next().is_some() {
        return Err(TelegramRenderError::InvalidReplyTarget {
            target: raw.to_string(),
            reason: "extra segments after reply_message_id".into(),
        });
    }
    Ok(TelegramReplyTarget {
        chat_id: chat_id_num,
        topic_id,
        reply_message_id: reply_msg_id,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelegramReplyTarget {
    pub chat_id: i64,
    pub topic_id: Option<i64>,
    pub reply_message_id: Option<i64>,
}

#[allow(dead_code)] // reserved for Telegram inbound reply-target construction once v2 adapter wiring stores outbound targets
pub fn build_reply_target_binding(
    chat_id: i64,
    topic_id: Option<i64>,
    reply_message_id: Option<i64>,
) -> ReplyTargetBindingRef {
    let topic = topic_id
        .map(|t| t.to_string())
        .unwrap_or_else(|| "_".to_string());
    let reply = reply_message_id
        .map(|r| r.to_string())
        .unwrap_or_else(|| "_".to_string());
    let formatted = format!("tg:{chat_id}:{topic}:{reply}");
    ReplyTargetBindingRef::new(formatted).expect("constructed reply target is well-formed") // safety: format produces ASCII digits/':'/'-'/'_' within bounded-ref length
}

/// Render a `FinalReplyView` into a `sendMessage` egress request.
pub fn render_final_reply(
    target: &ReplyTargetBindingRef,
    view: &FinalReplyView,
    credential_handle: EgressCredentialHandle,
) -> Result<EgressRequest, TelegramRenderError> {
    let reply = parse_reply_target(target)?;
    let mut body = serde_json::Map::new();
    body.insert(
        "chat_id".into(),
        serde_json::Value::Number(reply.chat_id.into()),
    );
    body.insert("text".into(), serde_json::Value::String(view.text.clone()));
    if let Some(topic_id) = reply.topic_id {
        body.insert(
            "message_thread_id".into(),
            serde_json::Value::Number(topic_id.into()),
        );
    }
    if let Some(reply_to) = reply.reply_message_id {
        body.insert(
            "reply_to_message_id".into(),
            serde_json::Value::Number(reply_to.into()),
        );
    }
    let body_bytes =
        serde_json::to_vec(&serde_json::Value::Object(body)).expect("body serializes to JSON"); // safety: body is a serde_json::Value::Object built from owned Strings/Numbers; serialization cannot fail

    Ok(build_egress_request(
        "/sendMessage",
        body_bytes,
        credential_handle,
    ))
}

/// Render a `ProgressUpdateView` (typing indicator) into a
/// `sendChatAction` egress request.
pub fn render_progress_typing(
    target: &ReplyTargetBindingRef,
    view: &ProgressUpdateView,
    credential_handle: EgressCredentialHandle,
) -> Result<Option<EgressRequest>, TelegramRenderError> {
    let reply = parse_reply_target(target)?;
    let action = match view.kind {
        ProgressKind::Typing | ProgressKind::Reflecting | ProgressKind::ToolRunning => "typing",
    };
    let mut body = serde_json::Map::new();
    body.insert(
        "chat_id".into(),
        serde_json::Value::Number(reply.chat_id.into()),
    );
    body.insert("action".into(), serde_json::Value::String(action.into()));
    if let Some(topic_id) = reply.topic_id {
        body.insert(
            "message_thread_id".into(),
            serde_json::Value::Number(topic_id.into()),
        );
    }
    let body_bytes =
        serde_json::to_vec(&serde_json::Value::Object(body)).expect("progress body serializes"); // safety: progress body is a serde_json::Value::Object built from owned scalars; serialization cannot fail

    Ok(Some(build_egress_request(
        "/sendChatAction",
        body_bytes,
        credential_handle,
    )))
}

/// Build a Telegram Bot API egress request via the
/// `ironclaw_product_adapters::EgressRequest` builder. All Telegram
/// outbound requests target `api.telegram.org`, are POST, and carry an
/// `application/json` body.
fn build_egress_request(
    path: &'static str,
    body: Vec<u8>,
    credential_handle: EgressCredentialHandle,
) -> EgressRequest {
    let host = DeclaredEgressHost::new(TELEGRAM_API_HOST).expect("static host valid"); // safety: TELEGRAM_API_HOST is a compile-time const that satisfies the host validator
    let method = EgressMethod::post();
    let egress_path = EgressPath::new(path).expect("static path valid"); // safety: only `/sendMessage` / `/sendChatAction` are passed here, both static
    let content_type =
        EgressHeader::new("content-type", "application/json").expect("static header valid"); // safety: static name/value satisfies the header validator
    EgressRequest::new(host, method, egress_path)
        .with_header(content_type)
        .with_body(body)
        .with_credential_handle(Some(credential_handle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_turns::TurnRunId;

    fn handle() -> EgressCredentialHandle {
        EgressCredentialHandle::new("telegram_bot_token").expect("valid")
    }

    #[test]
    fn parse_reply_target_round_trips() {
        let target = build_reply_target_binding(-100, Some(7), Some(42));
        let parsed = parse_reply_target(&target).expect("parse");
        assert_eq!(
            parsed,
            TelegramReplyTarget {
                chat_id: -100,
                topic_id: Some(7),
                reply_message_id: Some(42),
            }
        );
    }

    #[test]
    fn final_reply_renders_with_topic_and_reply_target() {
        let target = build_reply_target_binding(-100, Some(7), Some(42));
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hello!".into(),
            generated_at: Utc::now(),
        };
        let request = render_final_reply(&target, &view, handle()).expect("render");
        assert_eq!(request.host().as_str(), TELEGRAM_API_HOST);
        assert_eq!(request.method().as_str(), "POST");
        assert_eq!(request.path().as_str(), "/sendMessage");
        let body: serde_json::Value = serde_json::from_slice(request.body()).expect("body json");
        assert_eq!(body["chat_id"], -100);
        assert_eq!(body["text"], "hello!");
        assert_eq!(body["message_thread_id"], 7);
        assert_eq!(body["reply_to_message_id"], 42);
        assert_eq!(
            request
                .credential_handle()
                .expect("handle present")
                .as_str(),
            "telegram_bot_token"
        );
    }

    #[test]
    fn progress_typing_renders_send_chat_action() {
        let target = build_reply_target_binding(-100, None, None);
        let view = ProgressUpdateView {
            turn_run_id: TurnRunId::new(),
            kind: ProgressKind::Typing,
            generated_at: Utc::now(),
        };
        let request = render_progress_typing(&target, &view, handle())
            .expect("render")
            .expect("typing produces request");
        assert_eq!(request.path().as_str(), "/sendChatAction");
        let body: serde_json::Value = serde_json::from_slice(request.body()).expect("body json");
        assert_eq!(body["chat_id"], -100);
        assert_eq!(body["action"], "typing");
    }

    #[test]
    fn malformed_reply_target_fails_with_typed_error() {
        let bogus = ReplyTargetBindingRef::new("not-tg-format").expect("valid");
        let err = parse_reply_target(&bogus).expect_err("must fail");
        assert!(matches!(
            err,
            TelegramRenderError::InvalidReplyTarget { .. }
        ));
    }
}
