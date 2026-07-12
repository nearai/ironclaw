//! Slack Events API payload normalization.
//!
//! Inputs are raw Slack webhook event bytes. Event callbacks become
//! [`ParsedProductInbound`] values; URL-verification payloads are exposed for
//! the host to echo before normal ProductWorkflow routing. The host stamps
//! trusted context outside this crate after verifying Slack request signatures.

use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthResolutionPayload,
    AuthResolutionResult, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    ParsedProductInbound, ProductAdapterError, ProductAttachmentDescriptor, ProductAttachmentKind,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence,
    ScopedApprovalResolutionPayload, UserMessagePayload,
};
use serde::Deserialize;
use thiserror::Error;

pub const SLACK_API_HOST: &str = "slack.com";
pub const SLACK_USER_ACTOR_KIND: &str = "slack_user";
const SLACK_SYSTEM_ACTOR_KIND: &str = "slack_system";
const SLACK_IGNORED_ACTOR_ID: &str = "slack_ignored_actor";
const SLACK_IGNORED_CONVERSATION_ID: &str = "slack_ignored_conversation";
const SLACK_FILE_SHARE_SUBTYPE: &str = "file_share";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackUrlVerificationChallenge {
    pub challenge: String,
}

/// Maximum accepted byte length for any Slack inbound webhook payload.
const MAX_SLACK_PAYLOAD_BYTES: usize = 1024 * 1024; // 1 MB

pub fn parse_slack_url_verification_challenge(
    raw_payload: &[u8],
    auth_evidence: &ProtocolAuthEvidence,
) -> Result<Option<SlackUrlVerificationChallenge>, SlackPayloadParseError> {
    if !auth_evidence.is_verified() {
        return Err(SlackPayloadParseError::UnauthenticatedPayload);
    }
    if raw_payload.len() > MAX_SLACK_PAYLOAD_BYTES {
        return Err(SlackPayloadParseError::InvalidJson {
            reason: "payload exceeds size limit".into(),
        });
    }
    let wrapper: SlackUrlVerificationWrapper =
        serde_json::from_slice(raw_payload).map_err(|err| SlackPayloadParseError::InvalidJson {
            reason: err.to_string(),
        })?;
    if wrapper.event_type != "url_verification" {
        return Ok(None);
    }
    let Some(challenge) = wrapper.challenge else {
        return Err(SlackPayloadParseError::InvalidExternalRef {
            kind: "slack_url_verification_challenge",
            reason: "missing challenge".to_string(),
        });
    };
    Ok(Some(SlackUrlVerificationChallenge { challenge }))
}

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
    if raw_payload.len() > MAX_SLACK_PAYLOAD_BYTES {
        return Err(SlackPayloadParseError::InvalidJson {
            reason: "payload exceeds size limit".into(),
        });
    }

    let wrapper: SlackEventWrapper =
        serde_json::from_slice(raw_payload).map_err(|err| SlackPayloadParseError::InvalidJson {
            reason: err.to_string(),
        })?;
    let event_id = build_event_id(
        installation_id,
        wrapper.event_id.as_deref(),
        &wrapper.event_type,
    )?;

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
    if let Some(parsed) = try_parse_resolution_message(
        event_id.clone(),
        team_id,
        event,
        ProductTriggerReason::BotMention,
    )? {
        return Ok(parsed);
    }
    try_parse_user_message(event_id, team_id, event, SlackMessageKind::AppMention)
}

fn parse_message_event(
    event_id: ExternalEventId,
    team_id: Option<&str>,
    event: &SlackEvent,
) -> Result<ParsedProductInbound, SlackPayloadParseError> {
    if is_dm_channel(
        event.channel.as_deref().unwrap_or_default(),
        event.channel_type.as_deref(),
    ) {
        if let Some(parsed) = try_parse_resolution_message(
            event_id.clone(),
            team_id,
            event,
            ProductTriggerReason::DirectChat,
        )? {
            return Ok(parsed);
        }
        return try_parse_user_message(event_id, team_id, event, SlackMessageKind::Dm);
    }
    if event.thread_ts.is_some() {
        return parse_thread_interaction(event_id, team_id, event);
    }
    noop_parsed_inbound(event_id, team_id, Some(event))
}

/// Fixed user-message routing strategies in this first slice.
/// `AppMention`: public channel, strip leading `@mention`, thread fallback to `ts`.
/// `Dm`: direct-message channel required, keep text verbatim, no thread fallback.
/// `ThreadReply`: channel thread reply, keep text verbatim, require `thread_ts`.
#[derive(Debug, Clone, Copy)]
enum SlackMessageKind {
    AppMention,
    Dm,
    ThreadReply,
}

fn try_parse_user_message(
    event_id: ExternalEventId,
    team_id: Option<&str>,
    event: &SlackEvent,
    kind: SlackMessageKind,
) -> Result<ParsedProductInbound, SlackPayloadParseError> {
    if event.bot_id.is_some() || !is_user_generated_message_subtype(event.subtype.as_deref()) {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    }
    let Some(user) = event.user.as_deref() else {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    };
    let Some(channel) = event.channel.as_deref() else {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    };
    if matches!(kind, SlackMessageKind::Dm)
        && !is_dm_channel(channel, event.channel_type.as_deref())
    {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    }
    if matches!(kind, SlackMessageKind::ThreadReply) && event.thread_ts.is_none() {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    }
    let Some(ts) = event.ts.as_deref() else {
        return noop_parsed_inbound(event_id, team_id, Some(event));
    };

    let raw_text = event.text.as_deref().unwrap_or_default();
    let (text, thread_ts, trigger) = match kind {
        SlackMessageKind::AppMention => (
            strip_leading_bot_mention(raw_text),
            event.thread_ts.as_deref().or(Some(ts)),
            ProductTriggerReason::BotMention,
        ),
        SlackMessageKind::Dm => (
            raw_text.to_string(),
            event.thread_ts.as_deref(),
            ProductTriggerReason::DirectChat,
        ),
        SlackMessageKind::ThreadReply => (
            raw_text.to_string(),
            event.thread_ts.as_deref(),
            ProductTriggerReason::ReplyToBot,
        ),
    };

    let attachments = collect_attachments(&event.files)?;
    let parts = SlackUserMessageParts {
        team_id,
        user,
        channel,
        thread_ts,
        message_ts: Some(ts),
        text,
        attachments,
        trigger,
    };
    build_user_message(event_id, parts)
}

fn parse_thread_interaction(
    event_id: ExternalEventId,
    team_id: Option<&str>,
    event: &SlackEvent,
) -> Result<ParsedProductInbound, SlackPayloadParseError> {
    let Some(parsed) = try_parse_resolution_message(
        event_id.clone(),
        team_id,
        event,
        ProductTriggerReason::ReplyToBot,
    )?
    else {
        return try_parse_user_message(event_id, team_id, event, SlackMessageKind::ThreadReply);
    };
    Ok(parsed)
}

fn try_parse_resolution_message(
    event_id: ExternalEventId,
    team_id: Option<&str>,
    event: &SlackEvent,
    source_trigger: ProductTriggerReason,
) -> Result<Option<ParsedProductInbound>, SlackPayloadParseError> {
    if event.bot_id.is_some() || !is_user_generated_message_subtype(event.subtype.as_deref()) {
        return Ok(None);
    }
    let Some(user) = event.user.as_deref() else {
        return Ok(None);
    };
    let Some(channel) = event.channel.as_deref() else {
        return Ok(None);
    };
    let Some(ts) = event.ts.as_deref() else {
        return Ok(None);
    };

    let raw_text = event.text.as_deref().unwrap_or_default();
    let text = match source_trigger {
        ProductTriggerReason::BotMention => strip_leading_bot_mention(raw_text),
        ProductTriggerReason::DirectChat
        | ProductTriggerReason::ReplyToBot
        | ProductTriggerReason::BotCommand
        | ProductTriggerReason::LinkedThreadAction => raw_text.to_string(),
    };
    let Some(payload) = parse_interaction_resolution(&text, source_trigger)? else {
        return Ok(None);
    };
    let thread_ts = match source_trigger {
        ProductTriggerReason::BotMention => event.thread_ts.as_deref().or(Some(ts)),
        ProductTriggerReason::DirectChat
        | ProductTriggerReason::ReplyToBot
        | ProductTriggerReason::BotCommand
        | ProductTriggerReason::LinkedThreadAction => event.thread_ts.as_deref(),
    };
    let parts = SlackUserMessageParts {
        team_id,
        user,
        channel,
        thread_ts,
        message_ts: Some(ts),
        text,
        attachments: Vec::new(),
        trigger: source_trigger,
    };
    build_payload_message(event_id, &parts, payload).map(Some)
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

fn build_payload_message(
    event_id: ExternalEventId,
    parts: &SlackUserMessageParts<'_>,
    payload: ProductInboundPayload,
) -> Result<ParsedProductInbound, SlackPayloadParseError> {
    let actor_ref = build_actor_ref(Some(parts.user))?;
    let conversation_ref = build_conversation_ref(
        parts.team_id,
        Some(parts.channel),
        parts.thread_ts,
        parts.message_ts,
    )?;
    ParsedProductInbound::new(event_id, actor_ref, conversation_ref, payload)
        .map_err(adapter_error_to_payload_error)
}

fn parse_interaction_resolution(
    text: &str,
    source_trigger: ProductTriggerReason,
) -> Result<Option<ProductInboundPayload>, SlackPayloadParseError> {
    let text = strip_leading_slack_mentions(text);
    let text = strip_wrapping_inline_code(text);
    let text = strip_leading_slack_mentions(text);
    let mut parts = text.split_whitespace();
    let Some(first) = parts.next() else {
        return Ok(None);
    };
    match first.to_ascii_lowercase().as_str() {
        "approve" => {
            parse_approval_resolution(parts.next(), ApprovalDecision::ApproveOnce, source_trigger)
        }
        "deny" => parse_approval_resolution(parts.next(), ApprovalDecision::Deny, source_trigger),
        "auth" => {
            let Some(action) = parts.next() else {
                return malformed_interaction_noop("auth");
            };
            if action.eq_ignore_ascii_case("deny") {
                let Some(auth_request_ref) = parts.next() else {
                    return malformed_interaction_noop("auth deny");
                };
                if parts.next().is_some() {
                    return malformed_interaction_noop("auth deny");
                }
                AuthResolutionPayload::new(auth_request_ref, AuthResolutionResult::Denied)
                    .map(|payload| payload.with_source_trigger(source_trigger))
                    .map(ProductInboundPayload::AuthResolution)
                    .map(Some)
                    .map_err(adapter_error_to_payload_error)
            } else {
                malformed_interaction_noop("auth")
            }
        }
        _ => Ok(None),
    }
}

fn strip_wrapping_inline_code(text: &str) -> &str {
    let mut rest = text.trim();
    while rest.len() >= 2 && rest.starts_with('`') && rest.ends_with('`') {
        rest = rest[1..rest.len() - 1].trim();
    }
    rest
}

fn strip_leading_slack_mentions(text: &str) -> &str {
    let mut rest = text.trim_start();
    loop {
        let Some(after_open) = rest.strip_prefix("<@") else {
            return rest;
        };
        let Some((_mention, after_close)) = after_open.split_once('>') else {
            return rest;
        };
        rest = after_close.trim_start();
    }
}

fn parse_approval_resolution(
    gate_ref: Option<&str>,
    decision: ApprovalDecision,
    source_trigger: ProductTriggerReason,
) -> Result<Option<ProductInboundPayload>, SlackPayloadParseError> {
    match gate_ref {
        Some(gate_ref) => {
            // A well-formed `gate:<ref>` wins even when the user pasted the whole
            // instruction line (e.g. "approve gate:X or deny gate:X") — the
            // leading verb + first gate ref are the intent; trailing tokens are
            // ignored. Any token that is not a `gate:<ref>` is not a targeted
            // resolution (a genuine typo like "approve this"), so fall through to
            // a no-op regardless of whether trailing text follows — keeping
            // single- and multi-word non-gate replies consistent.
            if !gate_ref.starts_with("gate:") {
                return malformed_interaction_noop("approval");
            }
            ApprovalResolutionPayload::new(gate_ref, decision)
                .map(|payload| payload.with_source_trigger(source_trigger))
                .map(ProductInboundPayload::ApprovalResolution)
                .map(Some)
                .map_err(adapter_error_to_payload_error)
        }
        None => ScopedApprovalResolutionPayload::new(decision)
            .map(|payload| payload.with_source_trigger(source_trigger))
            .map(ProductInboundPayload::ScopedApprovalResolution)
            .map(Some)
            .map_err(adapter_error_to_payload_error),
    }
}

fn malformed_interaction_noop(
    _command: &'static str,
) -> Result<Option<ProductInboundPayload>, SlackPayloadParseError> {
    Ok(Some(ProductInboundPayload::NoOp))
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
    wrapper_event_type: &str,
) -> Result<ExternalEventId, SlackPayloadParseError> {
    if wrapper_event_type == "event_callback" {
        // event_callback must carry event_id to avoid dedup key collisions.
        // Two signed events of the same type without event_id would share an
        // identical ExternalEventId, silently dropping the second.
        let id = event_id.ok_or_else(|| SlackPayloadParseError::InvalidExternalRef {
            kind: "external_event_id",
            reason: "event_callback must carry event_id".to_string(),
        })?;
        ExternalEventId::new(format!("slack-{}-{id}", installation_id.as_str()))
    } else {
        // Non-event_callback types (team_join, url_verification, etc.) always
        // route to noop. Use a noop-namespaced key so they never collide with
        // real event_callback IDs.
        ExternalEventId::new(format!(
            "slack-{}-noop-{wrapper_event_type}",
            installation_id.as_str()
        ))
    }
    .map_err(|err| SlackPayloadParseError::InvalidExternalRef {
        kind: "external_event_id",
        reason: err.to_string(),
    })
}

fn build_actor_ref(user: Option<&str>) -> Result<ExternalActorRef, SlackPayloadParseError> {
    match user {
        Some(user) => ExternalActorRef::new(SLACK_USER_ACTOR_KIND, user, None::<&str>),
        None => ExternalActorRef::new(
            SLACK_SYSTEM_ACTOR_KIND,
            SLACK_IGNORED_ACTOR_ID,
            None::<&str>,
        ),
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
    ExternalConversationRef::new(
        team_id,
        channel.unwrap_or(SLACK_IGNORED_CONVERSATION_ID),
        thread_ts,
        message_ts,
    )
    .map_err(|err| SlackPayloadParseError::InvalidExternalRef {
        kind: "external_conversation_ref",
        reason: err.to_string(),
    })
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

fn is_user_generated_message_subtype(subtype: Option<&str>) -> bool {
    subtype.is_none_or(|value| value == SLACK_FILE_SHARE_SUBTYPE)
}

fn is_dm_channel(channel: &str, channel_type: Option<&str>) -> bool {
    match channel_type {
        Some("im") => true,
        Some(_) => false,
        None => channel.starts_with('D'),
    }
}

fn adapter_error_to_payload_error(err: ProductAdapterError) -> SlackPayloadParseError {
    match err {
        ProductAdapterError::InvalidIdentifier { kind, reason } => {
            SlackPayloadParseError::InvalidExternalRef { kind, reason }
        }
        ProductAdapterError::MalformedInboundPayload { reason } => {
            SlackPayloadParseError::InvalidJson {
                reason: reason.to_string(),
            }
        }
        other => SlackPayloadParseError::InvalidExternalRef {
            kind: "parsed_product_inbound",
            reason: other.to_string(),
        },
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SlackUrlVerificationWrapper {
    #[serde(rename = "type")]
    event_type: String,
    challenge: Option<String>,
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
    fn url_verification_challenge_is_extracted_for_host_response() {
        let challenge = parse_slack_url_verification_challenge(
            br#"{"type":"url_verification","challenge":"challenge-token"}"#,
            &verified(),
        )
        .expect("parse")
        .expect("url verification");

        assert_eq!(challenge.challenge, "challenge-token");
        assert!(
            parse_slack_url_verification_challenge(
                br#"{"type":"event_callback","event_id":"Ev123"}"#,
                &verified(),
            )
            .expect("parse")
            .is_none()
        );
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
    fn channel_thread_message_becomes_reply_to_bot_user_message() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvThreadReply",
            "event": {
                "type": "message",
                "user": "U123",
                "channel": "C123",
                "text": "continue without mentioning the bot",
                "ts": "1710000000.000011",
                "thread_ts": "1710000000.000010"
            }
        }));

        assert_eq!(inbound.external_conversation_ref.conversation_id(), "C123");
        assert_eq!(
            inbound.external_conversation_ref.topic_id(),
            Some("1710000000.000010")
        );
        assert_eq!(
            inbound.external_conversation_ref.reply_target_message_id(),
            Some("1710000000.000011")
        );
        match inbound.payload {
            ProductInboundPayload::UserMessage(payload) => {
                assert_eq!(payload.text, "continue without mentioning the bot");
                assert_eq!(payload.trigger, ProductTriggerReason::ReplyToBot);
            }
            other => panic!("expected user message, got {other:?}"),
        }
    }

    #[test]
    fn channel_thread_interaction_reply_becomes_approval_resolution() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvThreadApproval",
            "event": {
                "type": "message",
                "user": "U123",
                "channel": "C123",
                "text": "approve gate:abc123",
                "ts": "1710000000.000011",
                "thread_ts": "1710000000.000010"
            }
        }));

        assert_eq!(inbound.external_conversation_ref.conversation_id(), "C123");
        assert_eq!(
            inbound.external_conversation_ref.topic_id(),
            Some("1710000000.000010")
        );
        match inbound.payload {
            ProductInboundPayload::ApprovalResolution(payload) => {
                assert_eq!(payload.gate_ref, "gate:abc123");
                assert_eq!(
                    payload.source_trigger,
                    Some(ProductTriggerReason::ReplyToBot)
                );
            }
            other => panic!("expected approval resolution, got {other:?}"),
        }
    }

    #[test]
    fn channel_thread_auth_deny_reply_becomes_auth_resolution() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvThreadAuthDeny",
            "event": {
                "type": "message",
                "user": "U123",
                "channel": "C123",
                "text": "auth deny gate:auth-slack",
                "ts": "1710000000.000011",
                "thread_ts": "1710000000.000010"
            }
        }));

        match inbound.payload {
            ProductInboundPayload::AuthResolution(payload) => {
                assert_eq!(payload.auth_request_ref, "gate:auth-slack");
                assert_eq!(
                    payload.source_trigger,
                    Some(ProductTriggerReason::ReplyToBot)
                );
            }
            other => panic!("expected auth resolution, got {other:?}"),
        }
    }

    #[test]
    fn channel_thread_mentioned_auth_deny_reply_becomes_auth_resolution() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvThreadMentionAuthDeny",
            "event": {
                "type": "message",
                "user": "U123",
                "channel": "C123",
                "text": "<@UBOT> auth deny gate:auth-slack",
                "ts": "1710000000.000011",
                "thread_ts": "1710000000.000010"
            }
        }));

        match inbound.payload {
            ProductInboundPayload::AuthResolution(payload) => {
                assert_eq!(payload.auth_request_ref, "gate:auth-slack");
                assert_eq!(
                    payload.source_trigger,
                    Some(ProductTriggerReason::ReplyToBot)
                );
            }
            other => panic!("expected auth resolution, got {other:?}"),
        }
    }

    #[test]
    fn channel_thread_backticked_auth_deny_reply_becomes_auth_resolution() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvThreadBacktickedAuthDeny",
            "event": {
                "type": "message",
                "user": "U123",
                "channel": "C123",
                "text": "`auth deny gate:auth-slack`",
                "ts": "1710000000.000011",
                "thread_ts": "1710000000.000010"
            }
        }));

        match inbound.payload {
            ProductInboundPayload::AuthResolution(payload) => {
                assert_eq!(payload.auth_request_ref, "gate:auth-slack");
                assert_eq!(
                    payload.source_trigger,
                    Some(ProductTriggerReason::ReplyToBot)
                );
            }
            other => panic!("expected auth resolution, got {other:?}"),
        }
    }

    #[test]
    fn explicit_app_home_channel_type_is_noop_even_with_dm_prefixed_channel() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvAppHome",
            "event": {
                "type": "message",
                "channel_type": "app_home",
                "user": "U123",
                "channel": "D123",
                "text": "app home message",
                "ts": "1710000000.000009"
            }
        }));

        assert!(matches!(inbound.payload, ProductInboundPayload::NoOp));
    }

    #[test]
    fn dm_file_share_message_preserves_attachment_descriptors() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvFileShare",
            "event": {
                "type": "message",
                "channel_type": "im",
                "subtype": "file_share",
                "user": "U123",
                "channel": "D123",
                "text": "see attached",
                "ts": "1710000000.000010",
                "files": [{
                    "id": "F456",
                    "mimetype": "application/pdf",
                    "name": "brief.pdf",
                    "size": 4567,
                    "url_private": "https://files.slack.com/secret"
                }]
            }
        }));

        match inbound.payload {
            ProductInboundPayload::UserMessage(payload) => {
                assert_eq!(payload.text, "see attached");
                assert_eq!(payload.attachments.len(), 1);
                assert_eq!(payload.attachments[0].external_file_id, "F456");
            }
            other => panic!("expected user message, got {other:?}"),
        }
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

    // ── url_verification error paths ─────────────────────────────────────────

    #[test]
    fn url_verification_auth_guard_rejects_unauthenticated() {
        let err = parse_slack_url_verification_challenge(
            br#"{"type":"url_verification","challenge":"tok"}"#,
            &ProtocolAuthEvidence::failed(ironclaw_product_adapters::ProtocolAuthFailure::Missing),
        )
        .expect_err("unverified auth must fail");
        assert!(matches!(
            err,
            SlackPayloadParseError::UnauthenticatedPayload
        ));
    }

    #[test]
    fn url_verification_missing_challenge_returns_invalid_external_ref() {
        let err =
            parse_slack_url_verification_challenge(br#"{"type":"url_verification"}"#, &verified())
                .expect_err("missing challenge must fail");
        assert!(matches!(
            err,
            SlackPayloadParseError::InvalidExternalRef {
                kind: "slack_url_verification_challenge",
                ..
            }
        ));
    }

    #[test]
    fn url_verification_invalid_json_returns_parse_error() {
        let err = parse_slack_url_verification_challenge(
            br#"{"type":"url_verification","challenge":]"#,
            &verified(),
        )
        .expect_err("malformed JSON must fail");
        assert!(matches!(err, SlackPayloadParseError::InvalidJson { .. }));
    }

    // ── build_event_id uniqueness ─────────────────────────────────────────────

    #[test]
    fn event_callback_without_event_id_returns_error() {
        let err = parse_slack_event(
            br#"{"type":"event_callback","event":{"type":"message","channel":"D123","user":"U1","ts":"1.0"}}"#,
            &verified(),
            &installation_id(),
        )
        .expect_err("event_callback without event_id must fail");
        assert!(matches!(
            err,
            SlackPayloadParseError::InvalidExternalRef {
                kind: "external_event_id",
                ..
            }
        ));
    }

    // ── NoOp routing paths ────────────────────────────────────────────────────

    #[test]
    fn non_event_callback_type_is_noop() {
        let inbound =
            parse_slack_event(br#"{"type":"team_join"}"#, &verified(), &installation_id())
                .expect("non-event_callback must parse as noop");
        assert!(matches!(inbound.payload, ProductInboundPayload::NoOp));
    }

    #[test]
    fn unknown_inner_event_type_is_noop() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvUnknown",
            "event": {
                "type": "reaction_added",
                "user": "U123",
                "channel": "C123",
                "ts": "1710000000.000099"
            }
        }));
        assert!(matches!(inbound.payload, ProductInboundPayload::NoOp));
    }

    // ── app_mention thread_ts wins over ts ────────────────────────────────────

    #[test]
    fn app_mention_in_thread_uses_thread_ts_as_topic_id() {
        let inbound = parse(serde_json::json!({
            "type": "event_callback",
            "team_id": "T123",
            "event_id": "EvThread",
            "event": {
                "type": "app_mention",
                "user": "U456",
                "channel": "C123",
                "text": "<@UBOT> help",
                "thread_ts": "1710000000.000001",
                "ts": "1710000000.000002"
            }
        }));

        // thread_ts must win over ts as the topic_id
        assert_eq!(
            inbound.external_conversation_ref.topic_id(),
            Some("1710000000.000001")
        );
        match inbound.payload {
            ProductInboundPayload::UserMessage(payload) => {
                assert_eq!(payload.trigger, ProductTriggerReason::BotMention);
            }
            other => panic!("expected user message, got {other:?}"),
        }
    }
}
