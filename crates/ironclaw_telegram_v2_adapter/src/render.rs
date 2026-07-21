//! Outbound rendering for Telegram v2.
//!
//! Renders projection-derived payloads into Telegram Bot API egress requests.
//! All requests target the declared `api.telegram.org` host and use the
//! adapter's egress credential handle (the host resolves it to the bot
//! token at request time).

use ironclaw_product_adapters::{
    AuthPromptView, DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod,
    EgressPath, EgressRequest, ExternalConversationRef, FinalReplyView, GatePromptView,
    ProductOutboundTarget, ProgressKind, ProgressUpdateView,
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

/// Build the canonical Telegram reply-target binding ref
/// (`tg:<chat_id>:<topic|_>:<reply|_>`). The host's outbound-target surface
/// constructs stored delivery targets with this builder so they round-trip
/// through [`parse_reply_target`] at render time.
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

/// Resolve the concrete Telegram reply target for an outbound envelope.
///
/// Prefers the structured `external_conversation_ref` (the conversation the
/// host resolved for this delivery — a real chat id on both the live reply
/// path, where the host looks the binding up by the workflow's opaque
/// `reply:<id>` token, and the proactive/triggered path). Falls back to
/// parsing the `tg:` reply-target binding ref only when no usable
/// conversation ref is present. This mirrors the Slack adapter, which renders
/// from `external_conversation_ref` and never parses the opaque binding token.
pub fn resolve_reply_target(
    target: &ProductOutboundTarget,
) -> Result<TelegramReplyTarget, TelegramRenderError> {
    if let Some(reply) =
        telegram_reply_target_from_conversation_ref(&target.external_conversation_ref)
    {
        return Ok(reply);
    }
    parse_reply_target(&target.reply_target_binding_ref)
}

/// Build a [`TelegramReplyTarget`] from a conversation ref whose
/// `conversation_id` is a Telegram chat id. Returns `None` when the
/// conversation id is not a Telegram-shaped integer chat id (e.g. an empty or
/// non-numeric placeholder), so the caller can fall back to the binding ref.
fn telegram_reply_target_from_conversation_ref(
    conv: &ExternalConversationRef,
) -> Option<TelegramReplyTarget> {
    let chat_id: i64 = conv.conversation_id().parse().ok()?;
    let topic_id = conv.topic_id().and_then(|raw| raw.parse::<i64>().ok());
    let reply_message_id = conv
        .reply_target_message_id()
        .and_then(|raw| raw.parse::<i64>().ok());
    Some(TelegramReplyTarget {
        chat_id,
        topic_id,
        reply_message_id,
    })
}

/// Render a `FinalReplyView` into a `sendMessage` egress request.
/// Telegram caps message text at 4096 UTF-16 code units (its length
/// semantics); longer final replies are split into ordered lossless chunks.
pub const TELEGRAM_MESSAGE_MAX_UTF16_UNITS: usize = 4096;

/// Split `text` into ordered chunks of at most `max_units` UTF-16 code units,
/// never inside a character (a surrogate pair's 2 units stay together).
/// Concatenating the chunks reproduces the input exactly. Empty input yields
/// one empty chunk so the caller still sends exactly one message.
fn chunk_text_utf16(text: &str, max_units: usize) -> Vec<&str> {
    if text.is_empty() {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut units = 0usize;
    for (offset, ch) in text.char_indices() {
        let ch_units = ch.len_utf16();
        if units + ch_units > max_units && units > 0 {
            chunks.push(&text[start..offset]);
            start = offset;
            units = 0;
        }
        units += ch_units;
    }
    chunks.push(&text[start..]);
    chunks
}

/// Render a final reply as one or more ordered `sendMessage` requests: one
/// request per ≤4096-UTF-16-unit chunk (qa-telegram:C3). The adapter sends
/// them sequentially and stops at the first failure so partial delivery is
/// reported honestly.
pub fn render_final_reply(
    reply: &TelegramReplyTarget,
    view: &FinalReplyView,
    credential_handle: EgressCredentialHandle,
) -> Result<Vec<EgressRequest>, TelegramRenderError> {
    Ok(render_text_message_chunks(
        reply,
        &view.text,
        credential_handle,
    ))
}

/// Render a `BlockedAuth` prompt as `sendMessage` requests. The authorization
/// URL is the actionable part: the user opens it in a browser, consents on
/// the provider's site, and the parked run resumes through the OAuth
/// callback — nothing secret ever enters the chat. The shared channel
/// delivery driver only routes link-shaped challenges here (credential-entry
/// challenges take its deny arm), but render defensively when the URL is
/// absent rather than going silent.
pub fn render_auth_prompt(
    reply: &TelegramReplyTarget,
    view: &AuthPromptView,
    credential_handle: EgressCredentialHandle,
) -> Result<Vec<EgressRequest>, TelegramRenderError> {
    let mut text = format!("{}\n\n{}", view.headline, view.body);
    match &view.authorization_url {
        Some(url) => {
            text.push_str(
                "\n\nOpen this link to authorize — I'll continue automatically once it's done:\n",
            );
            text.push_str(url);
        }
        None => {
            text.push_str(
                "\n\nFinish this in the IronClaw web app (Extensions), then ask me again here.",
            );
        }
    }
    Ok(render_text_message_chunks(reply, &text, credential_handle))
}

/// Render a `BlockedApproval` prompt as `sendMessage` requests. The copy
/// advertises the in-chat reply because inbound genuinely parses it — the
/// channel-neutral grammar in
/// `ironclaw_product_adapters::interaction_commands`, the same one the
/// shared busy hint advertises. Keep copy and grammar in lockstep.
pub fn render_gate_prompt(
    reply: &TelegramReplyTarget,
    view: &GatePromptView,
    credential_handle: EgressCredentialHandle,
) -> Result<Vec<EgressRequest>, TelegramRenderError> {
    let text = format!(
        "{headline}\n\n{body}\n\nReply approve or deny in this chat to respond. If several requests are pending here, use approve {gate_ref} or deny {gate_ref}. You can also decide from the IronClaw web app.",
        headline = view.headline,
        body = view.body,
        gate_ref = view.gate_ref
    );
    Ok(render_text_message_chunks(reply, &text, credential_handle))
}

/// Shared `sendMessage` builder: split `text` into ≤4096-UTF-16-unit chunks
/// and produce one ordered request per chunk, each carrying the full chat
/// addressing (chat id, topic, reply-to).
fn render_text_message_chunks(
    reply: &TelegramReplyTarget,
    text: &str,
    credential_handle: EgressCredentialHandle,
) -> Vec<EgressRequest> {
    chunk_text_utf16(text, TELEGRAM_MESSAGE_MAX_UTF16_UNITS)
        .into_iter()
        .map(|chunk| {
            let mut body = serde_json::Map::new();
            body.insert(
                "chat_id".into(),
                serde_json::Value::Number(reply.chat_id.into()),
            );
            body.insert("text".into(), serde_json::Value::String(chunk.to_string()));
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
            let body_bytes = serde_json::to_vec(&serde_json::Value::Object(body))
                .expect("body serializes to JSON"); // safety: body is a serde_json::Value::Object built from owned Strings/Numbers; serialization cannot fail
            build_egress_request("/sendMessage", body_bytes, credential_handle.clone())
        })
        .collect()
}

/// Render a `ProgressUpdateView` (typing indicator) into a
/// `sendChatAction` egress request.
pub fn render_progress_typing(
    reply: &TelegramReplyTarget,
    view: &ProgressUpdateView,
    credential_handle: EgressCredentialHandle,
) -> Result<Option<EgressRequest>, TelegramRenderError> {
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

    fn reply(
        chat_id: i64,
        topic_id: Option<i64>,
        reply_message_id: Option<i64>,
    ) -> TelegramReplyTarget {
        TelegramReplyTarget {
            chat_id,
            topic_id,
            reply_message_id,
        }
    }

    fn outbound_target(
        binding: ReplyTargetBindingRef,
        conversation_id: &str,
        topic_id: Option<&str>,
        reply_target_message_id: Option<&str>,
    ) -> ProductOutboundTarget {
        ProductOutboundTarget::new(
            binding,
            ExternalConversationRef::new(None, conversation_id, topic_id, reply_target_message_id)
                .expect("valid conversation ref"),
            None,
        )
    }

    #[test]
    fn final_reply_renders_with_topic_and_reply_target() {
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hello!".into(),
            generated_at: Utc::now(),
        };
        let requests =
            render_final_reply(&reply(-100, Some(7), Some(42)), &view, handle()).expect("render");
        assert_eq!(requests.len(), 1, "short replies stay a single message");
        let request = &requests[0];
        assert_eq!(request.host().as_str(), TELEGRAM_API_HOST);
        assert_eq!(request.method().as_str(), "POST");
        assert_eq!(request.path().as_str(), "/sendMessage");
        let body: serde_json::Value = serde_json::from_slice(request.body()).expect("body json");
        assert_eq!(body["chat_id"], -100);
        assert_eq!(body["text"], "hello!");
        assert_eq!(body["message_thread_id"], 7);
        assert_eq!(body["reply_to_message_id"], 42);
        assert!(
            body.get("parse_mode").is_none(),
            "final replies are deterministic plain text — no parse_mode (qa-telegram:C4)"
        );
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
        let view = ProgressUpdateView {
            turn_run_id: TurnRunId::new(),
            kind: ProgressKind::Typing,
            generated_at: Utc::now(),
        };
        let request = render_progress_typing(&reply(-100, None, None), &view, handle())
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

    #[test]
    fn resolve_prefers_conversation_ref_on_live_reply_shape() {
        // The live reactive-reply path: the outbound target carries the
        // workflow's opaque `reply:<token>` binding (which is NOT a `tg:` ref
        // and would fail `parse_reply_target`) plus the host-resolved
        // conversation ref for the real chat. Rendering must target that chat.
        let binding = ReplyTargetBindingRef::new("reply:opaque-run-token").expect("valid");
        let target = outbound_target(binding, "-100", Some("7"), Some("42"));

        let resolved = resolve_reply_target(&target).expect("resolves from conversation ref");
        assert_eq!(resolved, reply(-100, Some(7), Some(42)));

        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "reactive reply".into(),
            generated_at: Utc::now(),
        };
        let requests = render_final_reply(&resolved, &view, handle()).expect("render");
        let request = &requests[0];
        assert_eq!(request.path().as_str(), "/sendMessage");
        let body: serde_json::Value = serde_json::from_slice(request.body()).expect("body json");
        assert_eq!(body["chat_id"], -100);
        assert_eq!(body["reply_to_message_id"], 42);
    }

    /// qa-telegram:C3 — replies over 4096 UTF-16 units split into ordered
    /// lossless chunks, each within the limit and carrying the same chat
    /// addressing.
    #[test]
    fn final_reply_over_4096_units_splits_into_ordered_lossless_chunks() {
        let text = "x".repeat(9000);
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: text.clone(),
            generated_at: Utc::now(),
        };
        let requests =
            render_final_reply(&reply(-100, Some(7), None), &view, handle()).expect("render");
        assert_eq!(requests.len(), 3, "9000 ASCII units -> 4096 + 4096 + 808");
        let mut reassembled = String::new();
        for request in &requests {
            assert_eq!(request.path().as_str(), "/sendMessage");
            let body: serde_json::Value =
                serde_json::from_slice(request.body()).expect("body json");
            assert_eq!(body["chat_id"], -100, "every chunk addresses the chat");
            assert_eq!(body["message_thread_id"], 7, "topic rides every chunk");
            let chunk = body["text"].as_str().expect("text");
            assert!(
                chunk.encode_utf16().count() <= TELEGRAM_MESSAGE_MAX_UTF16_UNITS,
                "chunk within Telegram's UTF-16 limit"
            );
            reassembled.push_str(chunk);
        }
        assert_eq!(reassembled, text, "ordered chunks reassemble losslessly");
    }

    /// Surrogate pairs (2 UTF-16 units) are never split across a chunk
    /// boundary, and multi-byte text still reassembles exactly.
    #[test]
    fn chunk_boundaries_never_split_a_surrogate_pair() {
        // '🦀' is 2 UTF-16 units; 2049 of them = 4098 units > one chunk.
        let text = "🦀".repeat(2049);
        let view = FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: text.clone(),
            generated_at: Utc::now(),
        };
        let requests =
            render_final_reply(&reply(555, None, None), &view, handle()).expect("render");
        assert_eq!(
            requests.len(),
            2,
            "4098 units -> 4096-boundary forces 2 chunks"
        );
        let mut reassembled = String::new();
        for request in &requests {
            let body: serde_json::Value =
                serde_json::from_slice(request.body()).expect("body json");
            let chunk = body["text"].as_str().expect("text");
            let units = chunk.encode_utf16().count();
            assert!(units <= TELEGRAM_MESSAGE_MAX_UTF16_UNITS);
            assert_eq!(units % 2, 0, "no torn surrogate pair at a boundary");
            reassembled.push_str(chunk);
        }
        assert_eq!(reassembled, text);
    }

    #[test]
    fn resolve_falls_back_to_tg_binding_when_conversation_ref_not_telegram() {
        // Targets without a Telegram-shaped conversation id (e.g. a proactive
        // delivery target carrying only the canonical `tg:` binding ref) fall
        // back to parsing the binding ref.
        let binding = build_reply_target_binding(555, None, None);
        let target = outbound_target(binding, "not-a-chat-id", None, None);
        let resolved = resolve_reply_target(&target).expect("falls back to tg: binding");
        assert_eq!(resolved, reply(555, None, None));
    }

    #[test]
    fn resolve_errors_when_neither_source_is_usable() {
        let binding = ReplyTargetBindingRef::new("reply:opaque").expect("valid");
        let target = outbound_target(binding, "not-a-chat-id", None, None);
        let err = resolve_reply_target(&target).expect_err("no usable target");
        assert!(matches!(
            err,
            TelegramRenderError::InvalidReplyTarget { .. }
        ));
    }
}
