//! Slack Events API payload normalization.
//!
//! Inputs are raw Slack webhook event bytes. Outputs are
//! [`ParsedProductInbound`] values; the host stamps trusted context outside
//! this crate after verifying Slack request signatures.

use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    ParsedProductInbound, ProductAdapterError, ProductAttachmentDescriptor, ProductAttachmentKind,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence, UserMessagePayload,
};
use serde::Deserialize;
use thiserror::Error;

pub const SLACK_API_HOST: &str = "slack.com";
pub const SLACK_USER_ACTOR_KIND: &str = "slack_user";
const SLACK_SYSTEM_ACTOR_KIND: &str = "slack_system";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlackPayloadParseError {
    #[error("invalid Slack event JSON: {reason}")]
    InvalidJson { reason: String },
    #[error("invalid external reference: {kind}: {reason}")]
    InvalidExternalRef { kind: &'static str, reason: String },
    #[error(
        "auth evidence is not Verified — host MUST verify the Slack request before calling parse_slack_event"
    )]
    UnauthenticatedPayload,
}

pub fn parse_slack_event(
    raw_payload: &[u8],
    auth_evidence: &ProtocolAuthEvidence,
    installation_id: &AdapterInstallationId,
) -> Result<ParsedProductInbound, SlackPayloadParseError> {
    if !auth_evidence.is_verified() {
        return Err(SlackPayloadParseError::UnauthenticatedPayload);
    }

    let wrapper: SlackEventWrapper =
        serde_json::from_slice(raw_payload).map_err(|err| SlackPayloadParseError::InvalidJson {
            reason: err.to_string(),
        })?;
    let event_id = build_event_id(installation_id, wrapper.event_id.as_deref(), &wrapper.event)?;

    if wrapper.event_type != "event_callback" {
        return noop_parsed_inbound(event_id, wrapper.team_id.as_deref(), wrapper.event.as_ref());
    }

    let Some(event) = wrapper.event.as_ref() else {
        return noop_parsed_inbound(event_id, wrapper.team_id.as_deref(), None);
    };

    match event.event_type.as_str() {
        "app_mention" => parse_app_mention(event_id, wrapper.team_id.as_deref(), event),
        "message" => parse_message_event(event_id, wrapper.team_id.as_deref(), event),
        _ => noop_parsed_inbound(event_id, wrapper.team_id.as_deref(), Some(event)),
    }
}

fn parse_app_mention(
    event_id: ExternalEventId,
    team_id: Option<&str>,
    event: &SlackEvent,
) -> Result<ParsedProductInbound, SlackPayloadParseError> {
    if event.bot_id.is_some() || event.subtype.is_some() {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    }
    let Some(user) = event.user.as_deref() else {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    };
    let Some(channel) = event.channel.as_deref() else {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    };
    let Some(ts) = event.ts.as_deref() else {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    };
    let text = event
        .text
        .as_deref()
        .map(strip_leading_bot_mention)
        .unwrap_or_default();
    build_user_message(
        event_id,
        SlackUserMessageParts {
            team_id,
            user,
            channel,
            thread_ts: event.thread_ts.as_deref().or(Some(ts)),
            message_ts: Some(ts),
            text,
            attachments: collect_attachments(&event.files)?,
            trigger: ProductTriggerReason::BotMention,
        },
    )
}

fn parse_message_event(
    event_id: ExternalEventId,
    team_id: Option<&str>,
    event: &SlackEvent,
) -> Result<ParsedProductInbound, SlackPayloadParseError> {
    if event.bot_id.is_some() || event.subtype.is_some() {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    }
    let Some(user) = event.user.as_deref() else {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    };
    let Some(channel) = event.channel.as_deref() else {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    };
    if !is_dm_channel(channel, event.channel_type.as_deref()) {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    }
    let Some(ts) = event.ts.as_deref() else {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    };
    let text = event.text.as_deref().unwrap_or_default().to_string();
    build_user_message(
        event_id,
        SlackUserMessageParts {
            team_id,
            user,
            channel,
            thread_ts: event.thread_ts.as_deref(),
            message_ts: Some(ts),
            text,
            attachments: collect_attachments(&event.files)?,
            trigger: ProductTriggerReason::DirectChat,
        },
    )
}

struct SlackUserMessageParts<'a> {
    team_id: Option<&'a str>,
    user: &'a str,
    channel: &'a str,
    thread_ts: Option<&'a str>,
    message_ts: Option<&'a str>,
    text: String,
    attachments: Vec<ProductAttachmentDescriptor>,
    trigger: ProductTriggerReason,
}

fn build_user_message(
    event_id: ExternalEventId,
    parts: SlackUserMessageParts<'_>,
) -> Result<ParsedProductInbound, SlackPayloadParseError> {
    let actor_ref = build_actor_ref(Some(parts.user))?;
    let conversation_ref = build_conversation_ref(
        parts.team_id,
        Some(parts.channel),
        parts.thread_ts,
        parts.message_ts,
    )?;
    let user_message = UserMessagePayload::new(parts.text, parts.attachments, parts.trigger)
        .map_err(|err| SlackPayloadParseError::InvalidExternalRef {
            kind: "user_message_payload",
            reason: err.to_string(),
        })?;
    ParsedProductInbound::new(
        event_id,
        actor_ref,
        conversation_ref,
        ProductInboundPayload::UserMessage(user_message),
    )
    .map_err(adapter_error_to_payload_error)
}

fn noop_parsed_inbound(
    event_id: ExternalEventId,
    team_id: Option<&str>,
    event: Option<&SlackEvent>,
) -> Result<ParsedProductInbound, SlackPayloadParseError> {
    let actor = build_actor_ref(event.and_then(|e| e.user.as_deref()))?;
    let conversation = build_conversation_ref(
        team_id,
        event.and_then(|e| e.channel.as_deref()),
        event.and_then(noop_thread_hint),
        event.and_then(|e| e.ts.as_deref()),
    )?;
    ParsedProductInbound::new(event_id, actor, conversation, ProductInboundPayload::NoOp)
        .map_err(adapter_error_to_payload_error)
}

fn noop_thread_hint(event: &SlackEvent) -> Option<&str> {
    if is_dm_channel(
        event.channel.as_deref().unwrap_or_default(),
        event.channel_type.as_deref(),
    ) {
        event.thread_ts.as_deref()
    } else {
        event.thread_ts.as_deref().or(event.ts.as_deref())
    }
}

fn build_event_id(
    installation_id: &AdapterInstallationId,
    event_id: Option<&str>,
    event: &Option<SlackEvent>,
) -> Result<ExternalEventId, SlackPayloadParseError> {
    let suffix = event_id
        .or_else(|| event.as_ref().and_then(|e| e.ts.as_deref()))
        .or_else(|| event.as_ref().map(|e| e.event_type.as_str()))
        .unwrap_or("noop");
    ExternalEventId::new(format!("slack-{}-{suffix}", installation_id.as_str())).map_err(|err| {
        SlackPayloadParseError::InvalidExternalRef {
            kind: "external_event_id",
            reason: err.to_string(),
        }
    })
}

fn build_actor_ref(user: Option<&str>) -> Result<ExternalActorRef, SlackPayloadParseError> {
    match user {
        Some(user) => ExternalActorRef::new(SLACK_USER_ACTOR_KIND, user, None::<&str>),
        None => ExternalActorRef::new(SLACK_SYSTEM_ACTOR_KIND, "noop", None::<&str>),
    }
    .map_err(|err| SlackPayloadParseError::InvalidExternalRef {
        kind: "external_actor_ref",
        reason: err.to_string(),
    })
}

fn build_conversation_ref(
    team_id: Option<&str>,
    channel: Option<&str>,
    thread_ts: Option<&str>,
    message_ts: Option<&str>,
) -> Result<ExternalConversationRef, SlackPayloadParseError> {
    ExternalConversationRef::new(team_id, channel.unwrap_or("noop"), thread_ts, message_ts).map_err(
        |err| SlackPayloadParseError::InvalidExternalRef {
            kind: "external_conversation_ref",
            reason: err.to_string(),
        },
    )
}

fn collect_attachments(
    files: &Option<Vec<SlackFile>>,
) -> Result<Vec<ProductAttachmentDescriptor>, SlackPayloadParseError> {
    files
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|file| {
            let mime_type = file
                .mimetype
                .as_deref()
                .unwrap_or("application/octet-stream")
                .to_ascii_lowercase();
            ProductAttachmentDescriptor::new(
                file.id.clone(),
                mime_type.clone(),
                file.name.clone(),
                file.size,
                attachment_kind_for_mime(&mime_type),
            )
            .map_err(|err| SlackPayloadParseError::InvalidExternalRef {
                kind: "attachment_descriptor",
                reason: err.to_string(),
            })
        })
        .collect()
}

fn attachment_kind_for_mime(mime_type: &str) -> ProductAttachmentKind {
    match mime_type.split('/').next().unwrap_or_default() {
        "image" => ProductAttachmentKind::Image,
        "audio" => ProductAttachmentKind::Audio,
        "video" => ProductAttachmentKind::Video,
        _ => ProductAttachmentKind::Document,
    }
}

fn strip_leading_bot_mention(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("<@")
        && let Some(end) = trimmed.find('>')
    {
        return trimmed[end + 1..].trim_start().to_string();
    }
    trimmed.to_string()
}

fn is_dm_channel(channel: &str, channel_type: Option<&str>) -> bool {
    channel_type == Some("im") || channel.starts_with('D')
}

fn adapter_error_to_payload_error(err: ProductAdapterError) -> SlackPayloadParseError {
    SlackPayloadParseError::InvalidJson {
        reason: err.to_string(),
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEventWrapper {
    #[serde(rename = "type")]
    event_type: String,
    event: Option<SlackEvent>,
    team_id: Option<String>,
    event_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEvent {
    #[serde(rename = "type")]
    event_type: String,
    user: Option<String>,
    channel: Option<String>,
    text: Option<String>,
    thread_ts: Option<String>,
    ts: Option<String>,
    bot_id: Option<String>,
    subtype: Option<String>,
    channel_type: Option<String>,
    #[serde(default)]
    files: Option<Vec<SlackFile>>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackFile {
    id: String,
    mimetype: Option<String>,
    name: Option<String>,
    size: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_adapters::ProductInboundPayload;
    use ironclaw_product_adapters::auth::mark_request_signature_verified;

    fn installation_id() -> AdapterInstallationId {
        AdapterInstallationId::new("slack_install_beta").expect("valid")
    }

    fn verified() -> ProtocolAuthEvidence {
        mark_request_signature_verified(
            "X-Slack-Signature",
            Some("X-Slack-Request-Timestamp".to_string()),
            "T123",
        )
    }

    fn parse(value: serde_json::Value) -> ParsedProductInbound {
        parse_slack_event(
            serde_json::to_string(&value).expect("serialize").as_bytes(),
            &verified(),
            &installation_id(),
        )
        .expect("parse")
    }

    #[test]
    fn dm_message_becomes_user_message() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "Ev123",
            "event": {
                "type": "message",
                "channel_type": "im",
                "user": "U123",
                "channel": "D123",
                "text": "hello from dm",
                "ts": "1710000000.000001"
            }
        }));

        assert_eq!(inbound.external_actor_ref.kind(), SLACK_USER_ACTOR_KIND);
        assert_eq!(inbound.external_actor_ref.id(), "U123");
        assert_eq!(inbound.external_conversation_ref.space_id(), Some("T123"));
        assert_eq!(inbound.external_conversation_ref.conversation_id(), "D123");
        assert_eq!(inbound.external_conversation_ref.topic_id(), None);
        assert_eq!(
            inbound.external_conversation_ref.reply_target_message_id(),
            Some("1710000000.000001")
        );
        match inbound.payload {
            ProductInboundPayload::UserMessage(payload) => {
                assert_eq!(payload.text, "hello from dm");
                assert_eq!(payload.trigger, ProductTriggerReason::DirectChat);
            }
            other => panic!("expected user message, got {other:?}"),
        }
    }

    #[test]
    fn app_mention_becomes_threaded_user_message() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "Ev456",
            "event": {
                "type": "app_mention",
                "user": "U456",
                "channel": "C123",
                "text": "<@UBOT> please help",
                "ts": "1710000000.000002"
            }
        }));

        assert_eq!(inbound.external_conversation_ref.conversation_id(), "C123");
        assert_eq!(
            inbound.external_conversation_ref.topic_id(),
            Some("1710000000.000002")
        );
        match inbound.payload {
            ProductInboundPayload::UserMessage(payload) => {
                assert_eq!(payload.text, "please help");
                assert_eq!(payload.trigger, ProductTriggerReason::BotMention);
            }
            other => panic!("expected user message, got {other:?}"),
        }
    }

    #[test]
    fn bot_or_subtyped_app_mentions_are_noop() {
        let bot = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvBotMention",
            "event": {
                "type": "app_mention",
                "user": "U123",
                "channel": "C123",
                "text": "<@UBOT> loop",
                "ts": "1710000000.000007",
                "bot_id": "B123"
            }
        }));
        assert!(matches!(bot.payload, ProductInboundPayload::NoOp));

        let subtype = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvSubtypeMention",
            "event": {
                "type": "app_mention",
                "user": "U123",
                "channel": "C123",
                "text": "<@UBOT> changed",
                "ts": "1710000000.000008",
                "subtype": "message_changed"
            }
        }));
        assert!(matches!(subtype.payload, ProductInboundPayload::NoOp));
    }

    #[test]
    fn bot_or_subtyped_messages_are_noop() {
        let bot = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvBot",
            "event": {
                "type": "message",
                "user": "U123",
                "channel": "D123",
                "text": "loop",
                "ts": "1710000000.000003",
                "bot_id": "B123"
            }
        }));
        assert!(matches!(bot.payload, ProductInboundPayload::NoOp));

        let subtype = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvSubtype",
            "event": {
                "type": "message",
                "user": "U123",
                "channel": "D123",
                "text": "changed",
                "ts": "1710000000.000004",
                "subtype": "message_changed"
            }
        }));
        assert!(matches!(subtype.payload, ProductInboundPayload::NoOp));
    }

    #[test]
    fn non_dm_channel_message_is_noop_in_first_slice() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvAmbient",
            "event": {
                "type": "message",
                "user": "U123",
                "channel": "C123",
                "text": "ambient channel chatter",
                "ts": "1710000000.000005"
            }
        }));

        assert!(matches!(inbound.payload, ProductInboundPayload::NoOp));
    }

    #[test]
    fn unauthenticated_payload_is_rejected() {
        let err = parse_slack_event(
            br#"{"type":"event_callback","event_id":"EvNoAuth"}"#,
            &ProtocolAuthEvidence::failed(ironclaw_product_adapters::ProtocolAuthFailure::Missing),
            &installation_id(),
        )
        .expect_err("missing verified evidence must fail");

        assert!(matches!(
            err,
            SlackPayloadParseError::UnauthenticatedPayload
        ));
    }

    #[test]
    fn attachments_are_descriptors_without_private_urls() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvFile",
            "event": {
                "type": "message",
                "channel_type": "im",
                "user": "U123",
                "channel": "D123",
                "text": "see attached",
                "ts": "1710000000.000006",
                "files": [{
                    "id": "F123",
                    "mimetype": "image/png",
                    "name": "screenshot.png",
                    "size": 1234,
                    "url_private": "https://files.slack.com/secret"
                }]
            }
        }));

        match inbound.payload {
            ProductInboundPayload::UserMessage(payload) => {
                assert_eq!(payload.attachments.len(), 1);
                let json = serde_json::to_string(&payload.attachments[0]).expect("serialize");
                assert!(json.contains("F123"));
                assert!(!json.contains("files.slack.com"));
                assert!(!json.contains("secret"));
            }
            other => panic!("expected user message, got {other:?}"),
        }
    }
}
