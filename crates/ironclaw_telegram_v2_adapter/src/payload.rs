//! Telegram Bot API payload normalization.
//!
//! Inputs are raw webhook update bytes. Outputs are
//! [`ParsedProductInbound`] values — the host stamps trusted context
//! ([`TrustedInboundContext::from_verified_evidence`]) and wraps the
//! result in a [`ProductInboundEnvelope`] outside this crate.
//!
//! Ignored-but-authenticated updates (ambient group messages, channel
//! posts, edited-message kinds we don't act on, messages without a
//! `from` we can't actor-ref) return `ParsedProductInbound { payload:
//! ProductInboundPayload::NoOp, .. }` with synthetic external refs for
//! the slots we genuinely have no source for. This matches the
//! `ironclaw_product` contract that says NoOps must be a
//! parsed inbound with the explicit `NoOp` payload variant, NOT an
//! out-of-band `None` path.

use ironclaw_host_api::product_adapter::{
    AdapterInstallationId, ChannelAttachmentRef, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundBatchFragment, InboundCommandPayload, NormalizedInboundMessage,
    ParsedProductInbound, ProductAdapterError, ProductAttachmentDescriptor, ProductAttachmentKind,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence, UserMessagePayload,
};
use serde::Deserialize;
use thiserror::Error;

pub const TELEGRAM_API_HOST: &str = "api.telegram.org";
pub const TELEGRAM_FILE_API_HOST: &str = "api.telegram.org";
pub const TELEGRAM_USER_ACTOR_KIND: &str = "telegram_user";
const TELEGRAM_MEDIA_GROUP_SETTLE_MILLIS: u64 = 1_000;

/// What an adapter installation is configured to recognize as an explicit
/// trigger inside group/supergroup chats.
///
/// Telegram private/direct chats do not require any trigger — every message
/// is forwarded. In groups/supergroups the adapter forwards a message ONLY
/// when one of these triggers fires, per #3285's "explicit triggers" rule.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupTriggerPolicy {
    /// Configured bot username (without leading `@`). Must be ASCII
    /// alphanumeric or `_`. The adapter compares mention entities against
    /// this value case-insensitively.
    pub bot_username: String,
    /// Stable bot user id used to detect "reply to a message authored by the
    /// bot" triggers.
    pub bot_user_id: i64,
    /// Recognized bot commands (without leading `/`). When a message starts
    /// with `/foo` or `/foo@botusername`, it is an explicit trigger.
    pub recognized_commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PayloadParseError {
    #[error("invalid Telegram update JSON: {reason}")]
    InvalidJson { reason: String },
    #[error("Telegram update missing update_id")]
    MissingUpdateId,
    #[error("invalid external reference: {kind}: {reason}")]
    InvalidExternalRef { kind: &'static str, reason: String },
    #[error(
        "auth evidence is not Verified — host MUST verify the webhook before \
         calling parse_telegram_update"
    )]
    UnauthenticatedPayload,
}

/// Parse a Telegram webhook payload into a [`ParsedProductInbound`]. The
/// host stamps trusted context outside this function and wraps the
/// result in a `ProductInboundEnvelope` — that is the kernel/adapter
/// trust boundary and must not be crossed inside the adapter itself.
///
/// Ignored-but-authenticated updates (no message, no `from`, ambient
/// group chatter, etc.) return a parsed inbound with
/// `payload = ProductInboundPayload::NoOp` and synthetic external refs
/// for the slots that genuinely have no source. The webhook still
/// acks 200 OK; the NoOp payload variant short-circuits inside the
/// workflow service.
pub fn parse_telegram_update(
    raw_payload: &[u8],
    auth_evidence: &ProtocolAuthEvidence,
    installation_id: &AdapterInstallationId,
    group_trigger_policy: &GroupTriggerPolicy,
) -> Result<ParsedProductInbound, PayloadParseError> {
    // `ProtocolAuthEvidence` is a sealed struct (formerly an enum). The
    // host mints verified evidence; components cannot fabricate one.
    // Reject anything else up front.
    if !auth_evidence.is_verified() {
        return Err(PayloadParseError::UnauthenticatedPayload);
    }

    let update: TelegramUpdate =
        serde_json::from_slice(raw_payload).map_err(|err| PayloadParseError::InvalidJson {
            reason: err.to_string(),
        })?;
    let update_id = update.update_id;
    if update_id == 0 {
        return Err(PayloadParseError::MissingUpdateId);
    }

    let event_id = build_event_id(installation_id, update_id)?;

    // Choose the message variant. We act on `message` and explicitly drop
    // `edited_message`, `channel_post`, and other update kinds — they
    // become `ProductInboundPayload::NoOp` parsed inbounds with
    // synthetic refs.
    let Some(message) = update.message else {
        return noop_parsed_inbound(event_id);
    };

    // `message.from` is optional in the Telegram schema (anonymous group
    // admins, channel-style updates that slipped through). Without it we
    // can't build a real `ExternalActorRef`; return a synthetic-ref NoOp
    // so the webhook acks 200 OK rather than triggering retries.
    if message.from.is_none() {
        return noop_parsed_inbound(event_id);
    }

    let chat_kind = TelegramChatKind::from_str(message.chat.kind.as_str());
    let trigger_outcome = classify_trigger(&message, chat_kind, group_trigger_policy);
    let Some(trigger) = trigger_outcome else {
        // Ambient group message / unsupported chat kind. We have the
        // message context so the NoOp gets real refs.
        let actor_ref = build_actor_ref(message.from.as_ref())?;
        let conversation_ref = build_conversation_ref(&message)?;
        return ParsedProductInbound::new(
            event_id,
            actor_ref,
            conversation_ref,
            ProductInboundPayload::NoOp,
        )
        .map_err(adapter_error_to_payload_error);
    };

    let actor_ref = build_actor_ref(message.from.as_ref())?;
    let conversation_ref = build_conversation_ref(&message)?;
    let payload = build_payload(message, trigger, group_trigger_policy)?;

    ParsedProductInbound::new(event_id, actor_ref, conversation_ref, payload)
        .map_err(adapter_error_to_payload_error)
}

/// Construct a `ParsedProductInbound { payload: NoOp, .. }` with
/// synthetic external refs for adapter-side "I can't extract real
/// context" cases (no `message` field, no `from` field). The synthetic
/// actor/conversation kinds are deliberate sentinels so a workflow
/// that mistakenly tries to route a NoOp produces a recognizable
/// signal in logs.
fn noop_parsed_inbound(
    event_id: ExternalEventId,
) -> Result<ParsedProductInbound, PayloadParseError> {
    let actor = ExternalActorRef::new("telegram_system", "noop", None::<&str>).map_err(|err| {
        PayloadParseError::InvalidExternalRef {
            kind: "external_actor_ref",
            reason: err.to_string(),
        }
    })?;
    let conversation = ExternalConversationRef::new(None, "noop", None::<&str>, None::<&str>)
        .map_err(|err| PayloadParseError::InvalidExternalRef {
            kind: "external_conversation_ref",
            reason: err.to_string(),
        })?;
    ParsedProductInbound::new(event_id, actor, conversation, ProductInboundPayload::NoOp)
        .map_err(adapter_error_to_payload_error)
}

fn adapter_error_to_payload_error(err: ProductAdapterError) -> PayloadParseError {
    // Surface the renderable message; the underlying error variants are
    // already host-redacted by `ProductAdapterError`.
    PayloadParseError::InvalidJson {
        reason: err.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelegramChatKind {
    Private,
    Group,
    Supergroup,
    Channel,
    Other,
}

impl TelegramChatKind {
    fn from_str(value: &str) -> Self {
        match value {
            "private" => Self::Private,
            "group" => Self::Group,
            "supergroup" => Self::Supergroup,
            "channel" => Self::Channel,
            _ => Self::Other,
        }
    }

    fn requires_explicit_trigger(self) -> bool {
        matches!(
            self,
            Self::Group | Self::Supergroup | Self::Channel | Self::Other
        )
    }
}

fn classify_trigger(
    message: &TelegramMessage,
    chat_kind: TelegramChatKind,
    policy: &GroupTriggerPolicy,
) -> Option<ProductTriggerReason> {
    if chat_kind == TelegramChatKind::Private {
        // The `trigger` reflects WHY a message was forwarded; for private
        // chats that's always `DirectChat`. Whether the message ALSO
        // contains a `/command` entity is a payload-shape decision made
        // by `build_payload` independently — see Copilot's review note
        // on the trigger/payload decoupling.
        return Some(ProductTriggerReason::DirectChat);
    }

    if !chat_kind.requires_explicit_trigger() {
        return Some(ProductTriggerReason::DirectChat);
    }

    // Channel posts are explicitly NoOp in the first slice. Telegram channel
    // posts are unsigned/broadcast-style and not interactive.
    if chat_kind == TelegramChatKind::Channel {
        return None;
    }

    // 1. Explicit @mention of the bot.
    if has_bot_mention(message, policy) {
        return Some(ProductTriggerReason::BotMention);
    }
    // 2. Reply-to a message authored by the bot.
    if reply_to_bot(message, policy.bot_user_id) {
        return Some(ProductTriggerReason::ReplyToBot);
    }
    // 3. Recognized bot command.
    if recognized_bot_command(message, policy) {
        return Some(ProductTriggerReason::BotCommand);
    }
    None
}

/// Iterate every `(text, entities)` window on a Telegram message in the
/// order Telegram delivers them: first the message `text+entities`, then
/// the `caption+caption_entities` for media messages. Yields nothing for
/// either side when its companion field is missing — the offsets in
/// `entities` are bound to `text` and similarly for `caption_entities`
/// against `caption`, so the pair is meaningless apart.
fn text_entity_windows(
    message: &TelegramMessage,
) -> impl Iterator<Item = (&str, &[MessageEntity])> {
    let text_window = message
        .text
        .as_deref()
        .zip(message.entities.as_deref().map(|e| e as &[_]));
    let caption_window = message
        .caption
        .as_deref()
        .zip(message.caption_entities.as_deref().map(|e| e as &[_]));
    text_window.into_iter().chain(caption_window)
}

fn has_bot_mention(message: &TelegramMessage, policy: &GroupTriggerPolicy) -> bool {
    let target_lower = policy.bot_username.to_ascii_lowercase();
    for (text, entities) in text_entity_windows(message) {
        for entity in entities {
            if entity.entity_type != "mention" {
                continue;
            }
            let Some(slice) = slice_text_by_offset(text, entity.offset, entity.length) else {
                continue;
            };
            // Mentions look like `@botname`. Strip the `@`.
            let trimmed = slice.strip_prefix('@').unwrap_or(slice);
            if trimmed.eq_ignore_ascii_case(&target_lower) {
                return true;
            }
        }
    }
    false
}

fn reply_to_bot(message: &TelegramMessage, bot_user_id: i64) -> bool {
    if bot_user_id <= 0 {
        return false;
    }
    let Some(reply) = message.reply_to_message.as_deref() else {
        return false;
    };
    let Some(from) = reply.from.as_ref() else {
        return false;
    };
    from.is_bot && from.id == bot_user_id
}

fn recognized_bot_command(message: &TelegramMessage, policy: &GroupTriggerPolicy) -> bool {
    extract_first_bot_command(message, policy).is_some()
}

/// Slice a UTF-16 offset+length window out of a string.
///
/// Telegram message entities are encoded against the UTF-16 representation of
/// the text (per the Bot API docs). A naive byte slice would corrupt
/// multi-byte mentions. This helper iterates UTF-16 code units to recover
/// the substring.
/// Slice from a UTF-16 offset to the end of the string.
fn slice_text_to_end(text: &str, offset: u32) -> Option<&str> {
    let start = offset as usize;
    // Empty string + offset 0 must yield an empty slice rather than None
    // — a zero-length entity at the start of an empty mention/command
    // payload is well-formed, even if degenerate.
    if start == 0 {
        return Some(text);
    }
    let mut units = 0usize;
    for (byte_idx, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units == start {
            // Offset reached: slice begins at the byte after this char.
            let next = byte_idx + ch.len_utf8();
            return text.get(next..);
        }
    }
    if units == start { Some("") } else { None }
}

fn slice_text_by_offset(text: &str, offset: u32, length: u32) -> Option<&str> {
    let start = offset as usize;
    let end = start.checked_add(length as usize)?;
    // Initialize byte_start to Some(0) when offset is 0 — without this,
    // the loop never sets byte_start for the start-of-string case (and an
    // empty string never enters the loop body at all). This made
    // slice_text_by_offset(_, 0, 0) return None instead of Some(""), which
    // is wrong for zero-length entities at the start of the text. Same
    // shape applies when start lies past the text and length is 0.
    let mut byte_start = if start == 0 { Some(0) } else { None };
    let mut byte_end = if end == 0 { Some(0) } else { None };
    let mut units = 0usize;
    for (byte_idx, ch) in text.char_indices() {
        if units == start && byte_start.is_none() {
            byte_start = Some(byte_idx);
        }
        if units == end && byte_end.is_none() {
            byte_end = Some(byte_idx);
            break;
        }
        units += ch.len_utf16();
    }
    if byte_end.is_none() && units == end {
        byte_end = Some(text.len());
    }
    if byte_start.is_none() && units == start {
        byte_start = Some(text.len());
    }
    let start = byte_start?;
    let end = byte_end?;
    text.get(start..end)
}

// ── Channel-normalized parsing (generic ingress router, extension-runtime P4) ──

/// One host-verified Telegram webhook update, normalized for the generic
/// channel-adapter contract: an ignored (but authenticated) update or one
/// plain user message. Recognized bot commands are rewritten into generic
/// command form so host classification does not need Telegram username syntax.
#[derive(Debug)]
pub enum TelegramInboundEvent {
    Ignore,
    Message(Box<NormalizedInboundMessage>),
    BatchFragment(Box<InboundBatchFragment>),
}

/// Parse one HOST-VERIFIED Telegram update into its normalized channel form.
/// Pure protocol work — no I/O, no secrets; the host executed the
/// `shared_secret_header` recipe before calling this. Applies the same
/// forwarding rules as [`parse_telegram_update`]: private chats always
/// forward; groups only on an explicit trigger; everything else is an
/// authenticated no-op.
pub fn normalize_telegram_update(
    raw_payload: &[u8],
    installation_id: &AdapterInstallationId,
    group_trigger_policy: &GroupTriggerPolicy,
) -> Result<TelegramInboundEvent, PayloadParseError> {
    let update: TelegramUpdate =
        serde_json::from_slice(raw_payload).map_err(|err| PayloadParseError::InvalidJson {
            reason: err.to_string(),
        })?;
    let update_id = update.update_id;
    if update_id == 0 {
        return Err(PayloadParseError::MissingUpdateId);
    }
    let Some(message) = update.message else {
        return Ok(TelegramInboundEvent::Ignore);
    };
    if message.from.is_none() {
        return Ok(TelegramInboundEvent::Ignore);
    }
    let chat_kind = TelegramChatKind::from_str(message.chat.kind.as_str());
    let trigger = classify_trigger(&message, chat_kind, group_trigger_policy);
    if trigger.is_none() && message.media_group_id.is_none() {
        return Ok(TelegramInboundEvent::Ignore);
    }
    let triggered = trigger.is_some();
    let trigger = trigger.unwrap_or(ProductTriggerReason::DirectChat);
    let scoped_media_group_key = message
        .media_group_id
        .as_deref()
        .map(|media_group_id| build_media_group_key(&message, media_group_id));
    let event_id = match scoped_media_group_key.as_deref() {
        Some(media_group_key) => build_media_group_event_id(installation_id, media_group_key)?,
        None => build_event_id(installation_id, update_id)?,
    };
    let actor = build_actor_ref(message.from.as_ref())?;
    let conversation = build_conversation_ref(&message)?;
    let attachments = collect_attachments(&message)?;
    let text = normalize_forwarded_text(&message, group_trigger_policy);
    let attachments = attachments
        .into_iter()
        .map(|descriptor| ChannelAttachmentRef {
            vendor_ref: descriptor.external_file_id.clone(),
            descriptor,
        })
        .collect();
    let normalized = NormalizedInboundMessage {
        actor,
        conversation,
        event_id,
        text,
        trigger,
        attachments,
        // Reply routing rides the conversation ref's thread anchors
        // (pre-coordinator delivery path); adopted when the P5 delivery
        // coordinator consumes stored contexts.
        reply_context: None,
    };
    let Some(media_group_key) = scoped_media_group_key else {
        return Ok(TelegramInboundEvent::Message(Box::new(normalized)));
    };
    let order = u64::try_from(message.message_id).map_err(|error| {
        PayloadParseError::InvalidExternalRef {
            kind: "telegram_media_group_order",
            reason: error.to_string(),
        }
    })?;
    let fragment = InboundBatchFragment {
        batch_key: media_group_key,
        fragment_id: format!("tg-update-{update_id}"),
        order,
        settle_millis: TELEGRAM_MEDIA_GROUP_SETTLE_MILLIS,
        triggered,
        message: normalized,
    };
    fragment
        .validate()
        .map_err(|error| PayloadParseError::InvalidExternalRef {
            kind: "telegram_media_group",
            reason: error.to_string(),
        })?;
    Ok(TelegramInboundEvent::BatchFragment(Box::new(fragment)))
}

fn normalize_forwarded_text(message: &TelegramMessage, policy: &GroupTriggerPolicy) -> String {
    if let Some((command, arguments)) =
        extract_first_addressed_bot_command(message, &policy.bot_username)
    {
        if arguments.is_empty() {
            return format!("/{command}");
        }
        return format!("/{command} {arguments}");
    }

    strip_leading_mention(
        message
            .text
            .clone()
            .or_else(|| message.caption.clone())
            .unwrap_or_default(),
        policy,
    )
}

#[cfg(test)]
mod slice_tests {
    use super::*;

    #[test]
    fn zero_length_slice_at_offset_zero_returns_empty() {
        assert_eq!(slice_text_by_offset("", 0, 0), Some(""));
        assert_eq!(slice_text_by_offset("hello", 0, 0), Some(""));
    }

    #[test]
    fn full_string_slice() {
        assert_eq!(slice_text_by_offset("hello", 0, 5), Some("hello"));
    }

    #[test]
    fn slice_at_end_zero_length() {
        assert_eq!(slice_text_by_offset("hello", 5, 0), Some(""));
    }

    #[test]
    fn slice_past_end_returns_none() {
        assert_eq!(slice_text_by_offset("hello", 6, 0), None);
        assert_eq!(slice_text_by_offset("hello", 5, 1), None);
    }

    #[test]
    fn multibyte_slice_respects_utf16_offsets() {
        // "🦀" is 1 char, 2 UTF-16 code units, 4 bytes in UTF-8.
        let text = "ab🦀cd";
        // Slice "🦀" => offset 2 (after "ab"), length 2 (one surrogate pair).
        assert_eq!(slice_text_by_offset(text, 2, 2), Some("🦀"));
        // Slice the whole string.
        assert_eq!(slice_text_by_offset(text, 0, 6), Some("ab🦀cd"));
    }

    #[test]
    fn slice_to_end_handles_empty_text() {
        assert_eq!(slice_text_to_end("", 0), Some(""));
    }

    #[test]
    fn slice_to_end_at_string_end() {
        assert_eq!(slice_text_to_end("hello", 5), Some(""));
    }

    #[test]
    fn slice_to_end_past_string_returns_none() {
        assert_eq!(slice_text_to_end("hello", 6), None);
    }

    #[test]
    fn slice_to_end_basic() {
        assert_eq!(slice_text_to_end("hello world", 6), Some("world"));
    }
}

fn build_event_id(
    installation_id: &AdapterInstallationId,
    update_id: i64,
) -> Result<ExternalEventId, PayloadParseError> {
    ExternalEventId::new(format!("tg-{}-{update_id}", installation_id.as_str())).map_err(|err| {
        PayloadParseError::InvalidExternalRef {
            kind: "external_event_id",
            reason: err.to_string(),
        }
    })
}

fn build_media_group_event_id(
    installation_id: &AdapterInstallationId,
    media_group_key: &str,
) -> Result<ExternalEventId, PayloadParseError> {
    ExternalEventId::new(format!(
        "tg-{}-media-{media_group_key}",
        installation_id.as_str()
    ))
    .map_err(|err| PayloadParseError::InvalidExternalRef {
        kind: "external_event_id",
        reason: err.to_string(),
    })
}

fn build_media_group_key(message: &TelegramMessage, media_group_id: &str) -> String {
    let thread = message
        .message_thread_id
        .map(|thread_id| thread_id.to_string())
        .unwrap_or_else(|| "none".to_string());
    format!(
        "chat-{}-thread-{thread}-group-{media_group_id}",
        message.chat.id
    )
}

fn build_actor_ref(sender: Option<&TelegramUser>) -> Result<ExternalActorRef, PayloadParseError> {
    let sender = sender.ok_or(PayloadParseError::InvalidExternalRef {
        kind: "external_actor_ref",
        reason: "telegram message has no `from` field".into(),
    })?;
    let display_name = sender
        .username
        .clone()
        .or_else(|| sender.first_name.clone())
        .filter(|s| !s.is_empty());
    ExternalActorRef::new(
        TELEGRAM_USER_ACTOR_KIND,
        sender.id.to_string(),
        display_name,
    )
    .map_err(|err| PayloadParseError::InvalidExternalRef {
        kind: "external_actor_ref",
        reason: err.to_string(),
    })
}

fn build_conversation_ref(
    message: &TelegramMessage,
) -> Result<ExternalConversationRef, PayloadParseError> {
    let chat_id = message.chat.id.to_string();
    let topic_id = message.message_thread_id.map(|t| t.to_string());
    let reply_target = message.message_id.to_string();
    ExternalConversationRef::new(
        None,
        chat_id,
        topic_id.as_deref(),
        Some(reply_target.as_str()),
    )
    .map_err(|err| PayloadParseError::InvalidExternalRef {
        kind: "external_conversation_ref",
        reason: err.to_string(),
    })
}

fn build_payload(
    message: TelegramMessage,
    trigger: ProductTriggerReason,
    policy: &GroupTriggerPolicy,
) -> Result<ProductInboundPayload, PayloadParseError> {
    // Emit `Command` whenever the message carries a recognized
    // `bot_command` entity, regardless of why the message was forwarded.
    // The `trigger` field still records the forwarding reason
    // (DirectChat for DMs, BotMention/ReplyToBot for groups, etc.), but
    // the payload kind is determined by whether a command entity is
    // actually present. Previously this branch gated on
    // `trigger == BotCommand` and silently downgraded `/help` to
    // `UserMessage` in private chats and in mention-triggered group
    // messages that also contained a `/command`.
    if let Some((command, arguments)) = extract_first_bot_command(&message, policy) {
        // Route through `InboundCommandPayload::new` so the shared
        // `ironclaw_product` validation fires on the
        // untrusted Telegram text: command-token shape and byte limit,
        // arguments byte limit, control-character rejection. A struct
        // literal here would bypass those checks and let oversized or
        // NUL/control-containing arguments cross into the trusted
        // inbound envelope. Mirrors the `UserMessagePayload::new` call
        // shape below for the user-message arm.
        let command_payload =
            InboundCommandPayload::new(command, arguments, trigger).map_err(|err| {
                PayloadParseError::InvalidExternalRef {
                    kind: "inbound_command_payload",
                    reason: err.to_string(),
                }
            })?;
        return Ok(ProductInboundPayload::Command(command_payload));
    }

    let mut text = message
        .text
        .clone()
        .or_else(|| message.caption.clone())
        .unwrap_or_default();
    text = strip_leading_mention(text, policy);
    // In-chat gate commands (`approve`/`deny`/`auth deny <gate_ref>`) — the
    // channel-neutral grammar shared with Slack. The busy/prompt copy the
    // shared delivery driver posts to this chat advertises these commands,
    // so they must resolve gates here instead of bouncing off a busy thread
    // as plain user messages (Ben's 2026-07-17 phantom-affordance loop).
    if let Some(resolution) = ironclaw_host_api::product_adapter::parse_interaction_resolution_text(
        ironclaw_host_api::product_adapter::strip_wrapping_inline_code(&text),
        trigger,
    )
    .map_err(|err| PayloadParseError::InvalidExternalRef {
        kind: "interaction_resolution_payload",
        reason: err.to_string(),
    })? {
        return Ok(resolution);
    }
    let attachments = collect_attachments(&message)?;
    let user_message = UserMessagePayload::new(text, attachments, trigger).map_err(|err| {
        PayloadParseError::InvalidExternalRef {
            kind: "user_message_payload",
            reason: err.to_string(),
        }
    })?;
    Ok(ProductInboundPayload::UserMessage(user_message))
}

fn extract_first_bot_command(
    message: &TelegramMessage,
    policy: &GroupTriggerPolicy,
) -> Option<(String, String)> {
    extract_first_matching_bot_command(message, &policy.bot_username, |command| {
        policy
            .recognized_commands
            .iter()
            .any(|recognized| recognized.eq_ignore_ascii_case(command))
    })
}

fn extract_first_addressed_bot_command(
    message: &TelegramMessage,
    bot_username: &str,
) -> Option<(String, String)> {
    extract_first_matching_bot_command(message, bot_username, |_| true)
}

fn extract_first_matching_bot_command(
    message: &TelegramMessage,
    bot_username: &str,
    mut accepts_command: impl FnMut(&str) -> bool,
) -> Option<(String, String)> {
    // Consult both `text+entities` and `caption+caption_entities` so a
    // matching `/command` in a media-message caption is extracted correctly.
    // The first matching command in `text` wins; otherwise the first matching
    // command in `caption` wins. Offsets in each entities list are bound to
    // their companion string.
    for (text, entities) in text_entity_windows(message) {
        for entity in entities {
            if entity.entity_type != "bot_command" {
                continue;
            }
            if !bot_command_is_leading(text, entities, entity, bot_username) {
                continue;
            }
            let Some(slice) = slice_text_by_offset(text, entity.offset, entity.length) else {
                continue;
            };
            let trimmed = slice.strip_prefix('/').unwrap_or(slice);
            let cmd_only = match trimmed.split_once('@') {
                Some((cmd, target)) => {
                    if !target.eq_ignore_ascii_case(bot_username) {
                        continue;
                    }
                    cmd
                }
                None => trimmed,
            };
            let cmd_lower = cmd_only.to_ascii_lowercase();
            if !accepts_command(&cmd_lower) {
                continue;
            }
            let Some(after_offset) = entity.offset.checked_add(entity.length) else {
                continue;
            };
            let arguments = slice_text_to_end(text, after_offset)
                .unwrap_or("")
                .trim_start()
                .to_string();
            return Some((cmd_lower, arguments));
        }
    }
    None
}

fn bot_command_is_leading(
    text: &str,
    entities: &[MessageEntity],
    command: &MessageEntity,
    bot_username: &str,
) -> bool {
    if command.offset == 0 {
        return true;
    }
    let Some(mention) = entities
        .iter()
        .find(|entity| entity.entity_type == "mention" && entity.offset == 0)
    else {
        return false;
    };
    let Some(mention_text) = slice_text_by_offset(text, mention.offset, mention.length) else {
        return false;
    };
    if !mention_text
        .strip_prefix('@')
        .is_some_and(|target| target.eq_ignore_ascii_case(bot_username))
    {
        return false;
    }
    let Some(mention_end) = mention.offset.checked_add(mention.length) else {
        return false;
    };
    let Some(gap_length) = command.offset.checked_sub(mention_end) else {
        return false;
    };
    slice_text_by_offset(text, mention_end, gap_length)
        .is_some_and(|gap| gap.chars().all(char::is_whitespace))
}

fn strip_leading_mention(text: String, policy: &GroupTriggerPolicy) -> String {
    let lower = format!("@{}", policy.bot_username.to_ascii_lowercase());
    if text.to_ascii_lowercase().starts_with(&lower) {
        text[lower.len()..].trim_start().to_string()
    } else {
        text
    }
}

fn collect_attachments(
    message: &TelegramMessage,
) -> Result<Vec<ProductAttachmentDescriptor>, PayloadParseError> {
    let mut out = Vec::new();
    if let Some(photos) = message.photo.as_ref() {
        // Telegram sends multiple sizes; keep the largest by file_size if
        // present, otherwise the last (Telegram convention).
        if let Some(largest) = photos
            .iter()
            .max_by_key(|p| p.file_size.unwrap_or(0))
            .or_else(|| photos.last())
        {
            out.push(make_attachment(
                &largest.file_id,
                "image/jpeg",
                None,
                largest.file_size,
                ProductAttachmentKind::Image,
            )?);
        }
    }
    if let Some(doc) = message.document.as_ref() {
        out.push(make_attachment(
            &doc.file_id,
            doc.mime_type
                .as_deref()
                .unwrap_or("application/octet-stream"),
            doc.file_name.clone(),
            doc.file_size,
            ProductAttachmentKind::Document,
        )?);
    }
    if let Some(voice) = message.voice.as_ref() {
        out.push(make_attachment(
            &voice.file_id,
            voice.mime_type.as_deref().unwrap_or("audio/ogg"),
            None,
            voice.file_size,
            ProductAttachmentKind::Voice,
        )?);
    }
    if let Some(audio) = message.audio.as_ref() {
        out.push(make_attachment(
            &audio.file_id,
            audio.mime_type.as_deref().unwrap_or("audio/mpeg"),
            audio.file_name.clone(),
            audio.file_size,
            ProductAttachmentKind::Audio,
        )?);
    }
    if let Some(video) = message.video.as_ref() {
        out.push(make_attachment(
            &video.file_id,
            video.mime_type.as_deref().unwrap_or("video/mp4"),
            video.file_name.clone(),
            video.file_size,
            ProductAttachmentKind::Video,
        )?);
    }
    if let Some(sticker) = message.sticker.as_ref() {
        out.push(make_attachment(
            &sticker.file_id,
            "image/webp",
            None,
            sticker.file_size,
            ProductAttachmentKind::Sticker,
        )?);
    }
    Ok(out)
}

fn make_attachment(
    file_id: &str,
    mime_type: &str,
    filename: Option<String>,
    size_bytes: Option<u64>,
    kind: ProductAttachmentKind,
) -> Result<ProductAttachmentDescriptor, PayloadParseError> {
    ProductAttachmentDescriptor::new(file_id, mime_type, filename, size_bytes, kind).map_err(
        |err| PayloadParseError::InvalidExternalRef {
            kind: "attachment_descriptor",
            reason: err.to_string(),
        },
    )
}

// ---------------------------------------------------------------------------
// Telegram payload deserialization shapes (only the fields we read).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct TelegramUpdate {
    #[serde(default)]
    update_id: i64,
    #[serde(default)]
    message: Option<TelegramMessage>,
    #[serde(default)]
    edited_message: Option<TelegramMessage>,
    #[serde(default)]
    channel_post: Option<TelegramMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramMessage {
    #[serde(default)]
    message_id: i64,
    #[serde(default)]
    media_group_id: Option<String>,
    #[serde(default)]
    from: Option<TelegramUser>,
    chat: TelegramChat,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    caption: Option<String>,
    #[serde(default)]
    entities: Option<Vec<MessageEntity>>,
    /// `caption_entities` mirrors `entities` for media messages whose
    /// human-readable text lives in `caption` rather than `text`
    /// (photos, documents, videos, ...). Mentions and `bot_command`
    /// entities can appear here too; trigger detection must consult
    /// both `(text, entities)` and `(caption, caption_entities)` or
    /// it silently NoOps media messages that should fire.
    #[serde(default)]
    caption_entities: Option<Vec<MessageEntity>>,
    #[serde(default)]
    reply_to_message: Option<Box<TelegramMessage>>,
    #[serde(default)]
    photo: Option<Vec<PhotoSize>>,
    #[serde(default)]
    document: Option<TelegramDocument>,
    #[serde(default)]
    voice: Option<TelegramVoice>,
    #[serde(default)]
    audio: Option<TelegramAudio>,
    #[serde(default)]
    video: Option<TelegramVideo>,
    #[serde(default)]
    sticker: Option<TelegramSticker>,
    #[serde(default)]
    message_thread_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramUser {
    id: i64,
    #[serde(default)]
    is_bot: bool,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MessageEntity {
    #[serde(rename = "type")]
    entity_type: String,
    offset: u32,
    length: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct PhotoSize {
    file_id: String,
    #[serde(default)]
    file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramDocument {
    file_id: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramVoice {
    file_id: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramAudio {
    file_id: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramVideo {
    file_id: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    file_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramSticker {
    file_id: String,
    #[serde(default)]
    file_size: Option<u64>,
}

// keep clippy happy about read-only fields on edited_message / channel_post.
#[allow(dead_code)]
fn _suppress_unused_field_warnings(update: &TelegramUpdate) {
    let _ = (&update.edited_message, &update.channel_post);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::product_adapter::ProductAdapterId;
    use ironclaw_host_api::product_adapter::auth::mark_shared_secret_header_verified;

    fn evidence() -> ProtocolAuthEvidence {
        mark_shared_secret_header_verified(
            "X-Telegram-Bot-Api-Secret-Token",
            "telegram_install_alpha",
        )
    }

    #[allow(dead_code)]
    fn adapter_id() -> ProductAdapterId {
        ProductAdapterId::new("telegram_v2").expect("valid")
    }

    fn install_id() -> AdapterInstallationId {
        AdapterInstallationId::new("install_alpha").expect("valid")
    }

    fn policy() -> GroupTriggerPolicy {
        GroupTriggerPolicy {
            bot_username: "ironclaw_bot".into(),
            bot_user_id: 9000,
            recognized_commands: vec!["start".into(), "help".into()],
        }
    }

    #[test]
    fn normalized_bot_qualified_command_uses_generic_command_text() {
        let payload = include_bytes!("../tests/fixtures/group_command.json");
        let event =
            normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes");
        let TelegramInboundEvent::Message(message) = event else {
            panic!("recognized group command must be forwarded");
        };
        assert_eq!(message.text, "/help");
        assert_eq!(message.trigger, ProductTriggerReason::BotCommand);
    }

    #[test]
    fn normalized_bot_command_preserves_arguments() {
        let payload = br#"{
            "update_id": 501,
            "message": {
                "message_id": 71,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "/help@ironclaw_bot verbose now",
                "entities": [{"type": "bot_command", "offset": 0, "length": 18}]
            }
        }"#;
        let event =
            normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes");
        let TelegramInboundEvent::Message(message) = event else {
            panic!("recognized group command must be forwarded");
        };
        assert_eq!(message.text, "/help verbose now");
    }

    #[test]
    fn private_qualified_product_command_canonicalizes_without_group_whitelist() {
        let payload = br#"{
            "update_id": 503,
            "message": {
                "message_id": 73,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/model@ironclaw_bot openai/gpt-5",
                "entities": [{"type": "bot_command", "offset": 0, "length": 19}]
            }
        }"#;
        let mut policy = policy();
        policy.recognized_commands.clear();

        let event = normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes");
        let TelegramInboundEvent::Message(message) = event else {
            panic!("private chat command must be forwarded");
        };
        assert_eq!(message.text, "/model openai/gpt-5");
        assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
    }

    #[test]
    fn private_command_for_another_bot_keeps_its_target() {
        let payload = br#"{
            "update_id": 504,
            "message": {
                "message_id": 74,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/model@other_bot openai/gpt-5",
                "entities": [{"type": "bot_command", "offset": 0, "length": 16}]
            }
        }"#;
        let mut policy = policy();
        policy.recognized_commands.clear();

        let event = normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes");
        let TelegramInboundEvent::Message(message) = event else {
            panic!("private chat command must be forwarded");
        };
        assert_eq!(message.text, "/model@other_bot openai/gpt-5");
        assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
    }

    #[test]
    fn private_mid_sentence_bot_command_remains_ordinary_text() {
        let payload = br#"{
            "update_id": 506,
            "message": {
                "message_id": 76,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "please run /help for me",
                "entities": [{"type": "bot_command", "offset": 11, "length": 5}]
            }
        }"#;

        let event =
            normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes");
        let TelegramInboundEvent::Message(message) = event else {
            panic!("private chat text must be forwarded");
        };
        assert_eq!(message.text, "please run /help for me");
        assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
    }

    #[test]
    fn command_after_non_mention_prefix_remains_ordinary_text() {
        let payload = r#"{
            "update_id": 505,
            "message": {
                "message_id": 75,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "🦀 /model@ironclaw_bot openai/gpt-5",
                "entities": [{"type": "bot_command", "offset": 3, "length": 19}]
            }
        }"#
        .as_bytes();
        let mut policy = policy();
        policy.recognized_commands.clear();

        let event = normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes");
        let TelegramInboundEvent::Message(message) = event else {
            panic!("private chat command must be forwarded");
        };
        assert_eq!(
            message.text, "🦀 /model@ironclaw_bot openai/gpt-5",
            "only commands at offset zero or after a leading bot mention are canonicalized"
        );
    }

    #[test]
    fn command_for_another_bot_remains_ignored_in_groups() {
        let payload = br#"{
            "update_id": 502,
            "message": {
                "message_id": 72,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "/help@other_bot",
                "entities": [{"type": "bot_command", "offset": 0, "length": 15}]
            }
        }"#;
        assert!(matches!(
            normalize_telegram_update(payload, &install_id(), &policy()).expect("normalizes"),
            TelegramInboundEvent::Ignore
        ));
    }

    #[test]
    fn addressed_product_command_without_group_whitelist_remains_ignored_in_groups() {
        let payload = br#"{
            "update_id": 506,
            "message": {
                "message_id": 76,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "/model@ironclaw_bot openai/gpt-5",
                "entities": [{"type": "bot_command", "offset": 0, "length": 19}]
            }
        }"#;
        let mut policy = policy();
        policy.recognized_commands.clear();

        assert!(matches!(
            normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes"),
            TelegramInboundEvent::Ignore
        ));
    }

    #[test]
    fn unauthenticated_payload_fails_closed() {
        let payload = br#"{"update_id":1}"#;
        // `ProtocolAuthEvidence` is now a sealed struct, not an enum;
        // `failed(failure)` constructs an unverified evidence.
        let evidence = ProtocolAuthEvidence::failed(
            ironclaw_host_api::product_adapter::ProtocolAuthFailure::SharedSecretMismatch,
        );
        let err = parse_telegram_update(payload, &evidence, &install_id(), &policy())
            .expect_err("unauthenticated must error");
        assert!(matches!(err, PayloadParseError::UnauthenticatedPayload));
    }

    #[test]
    fn private_chat_recognized_bot_command_emits_command_payload() {
        // Henry's review (PR #3354) + Copilot's payload.rs:469 finding:
        // `/help` in a DM was previously downgraded to `UserMessage`
        // because the old `build_payload` gated `Command` emission on
        // `trigger == BotCommand`, and private chats always returned
        // `DirectChat`. The fix decouples them: payload kind is decided
        // by whether a recognized `bot_command` entity exists; the
        // trigger keeps its forwarding-reason semantics (DirectChat for
        // DMs).
        let payload = br#"{
            "update_id": 110,
            "message": {
                "message_id": 11,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/help",
                "entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        match envelope.payload {
            ProductInboundPayload::Command(cmd) => {
                assert_eq!(cmd.command, "help");
                assert_eq!(cmd.arguments, "");
                // Trigger reflects WHY the message was forwarded
                // (it's a DM); command-ness is captured in the payload
                // variant, not the trigger.
                assert_eq!(cmd.trigger, ProductTriggerReason::DirectChat);
            }
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn group_mention_with_bot_command_emits_command_payload() {
        // Copilot's payload.rs:469 finding: a `/command` inside a
        // mention-triggered group message previously emitted
        // `UserMessage` because `build_payload` only produced `Command`
        // when `trigger == BotCommand` — but in groups the mention
        // check fires first and sets `trigger = BotMention`. The
        // decoupled `build_payload` now produces `Command` whenever a
        // recognized command is present, and the trigger preserves
        // the BotMention forwarding reason.
        let payload = br#"{
            "update_id": 220,
            "message": {
                "message_id": 12,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "@ironclaw_bot /help",
                "entities": [
                    {"type": "mention", "offset": 0, "length": 13},
                    {"type": "bot_command", "offset": 14, "length": 5}
                ]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        match envelope.payload {
            ProductInboundPayload::Command(cmd) => {
                assert_eq!(cmd.command, "help");
                assert_eq!(cmd.trigger, ProductTriggerReason::BotMention);
            }
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn private_chat_unknown_command_still_classifies_as_direct_chat() {
        // Defense-in-depth for the fix above: an UNRECOGNIZED command
        // (`/nope` is not in the policy's `recognized_commands`) must
        // still fall through to `DirectChat`, not silently become a
        // `Command` for a command the adapter doesn't know about.
        let payload = br#"{
            "update_id": 111,
            "message": {
                "message_id": 12,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/nope",
                "entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        match envelope.payload {
            ProductInboundPayload::UserMessage(user) => {
                assert_eq!(user.trigger, ProductTriggerReason::DirectChat);
            }
            other => panic!("expected UserMessage with DirectChat trigger, got {other:?}"),
        }
    }

    #[test]
    fn command_arguments_with_control_char_rejected_via_shared_validation() {
        // Henry's review (PR #3354, 2026-05-12T18:59:39Z) — Critical:
        // `build_payload` previously constructed `InboundCommandPayload`
        // with a struct literal, bypassing `InboundCommandPayload::new`
        // and the shared `ironclaw_product` validation
        // (token shape, byte limits, control-char rejection). Untrusted
        // Telegram webhook text could carry control characters into
        // the trusted inbound envelope.
        //
        // Asserts the validation now fires: a `/help` with a U+0001
        // control character in the argument text must be rejected with
        // `InvalidExternalRef { kind: "inbound_command_payload" }`,
        // mirroring how the user-message arm reports its own
        // validation failures.
        //
        // The control char is embedded via JSON's `` escape so
        // the raw bytes the JSON parser produces include a literal
        // control character.
        let payload = br#"{
            "update_id": 250,
            "message": {
                "message_id": 16,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "/help \u0001oops",
                "entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
        let err = parse_telegram_update(payload, &evidence(), &install_id(), &policy())
            .expect_err("control-character arguments must be rejected");
        match err {
            PayloadParseError::InvalidExternalRef { kind, reason } => {
                assert_eq!(kind, "inbound_command_payload");
                // `MalformedInboundPayload` carries a `RedactedString`,
                // so its Display surface is the redaction marker, not
                // the raw failure detail (security contract). Asserting
                // on `<redacted>` proves the shared validator was
                // reached AND its redaction is intact — a regression
                // that leaked the control-char-bearing content into
                // the error message would fail this assertion.
                assert!(
                    reason.contains("<redacted>"),
                    "rejection reason must be redacted (control-char content must not leak); got {reason}",
                );
            }
            other => {
                panic!("expected InvalidExternalRef{{kind:inbound_command_payload}}, got {other:?}")
            }
        }
    }

    #[test]
    fn command_arguments_exceeding_byte_limit_rejected_via_shared_validation() {
        // Defense-in-depth for the same fix: synthesize a command with
        // arguments larger than `COMMAND_ARGUMENTS_MAX_BYTES` (64 KiB
        // per `ironclaw_host_api::product_adapter::inbound`) and assert the
        // shared validator rejects it through `InboundCommandPayload::new`.
        // 70_000 bytes is comfortably over the 64 * 1024 = 65_536 limit.
        let oversized = "a".repeat(70_000);
        let payload = format!(
            r#"{{
                "update_id": 251,
                "message": {{
                    "message_id": 17,
                    "date": 1700000000,
                    "from": {{"id": 777, "is_bot": false, "first_name": "Alice"}},
                    "chat": {{"id": 777, "type": "private"}},
                    "text": "/help {oversized}",
                    "entities": [{{"type": "bot_command", "offset": 0, "length": 5}}]
                }}
            }}"#
        );
        let err = parse_telegram_update(payload.as_bytes(), &evidence(), &install_id(), &policy())
            .expect_err("oversized arguments must be rejected");
        match err {
            PayloadParseError::InvalidExternalRef { kind, reason } => {
                assert_eq!(kind, "inbound_command_payload");
                // Same redaction contract as the control-char test
                // above. The 70_000-byte payload must not leak into
                // the error message.
                assert!(
                    reason.contains("<redacted>"),
                    "rejection reason must be redacted (oversized content must not leak); got {reason}",
                );
            }
            other => {
                panic!("expected InvalidExternalRef{{kind:inbound_command_payload}}, got {other:?}")
            }
        }
    }

    #[test]
    fn group_media_caption_mention_is_recognized_as_bot_mention() {
        // Copilot's payload.rs:222 finding: trigger detection previously
        // consulted only `text + entities`. A photo with caption
        // `@ironclaw_bot help` carries its mention in
        // `caption_entities`, so `classify_trigger` returned None and
        // the update was silently NoOp'd. The fix consults both text-
        // and caption-anchored entity lists.
        let payload = br#"{
            "update_id": 230,
            "message": {
                "message_id": 13,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "photo": [
                    {"file_id": "AAAA", "file_unique_id": "u1", "width": 100, "height": 100, "file_size": 500}
                ],
                "caption": "@ironclaw_bot please look",
                "caption_entities": [{"type": "mention", "offset": 0, "length": 13}]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        match envelope.payload {
            ProductInboundPayload::UserMessage(user) => {
                assert_eq!(user.trigger, ProductTriggerReason::BotMention);
            }
            other => panic!("expected UserMessage with BotMention trigger, got {other:?}"),
        }
    }

    #[test]
    fn group_media_caption_bot_command_emits_command_payload() {
        // Caption-anchored `/help` must reach the Command path too.
        let payload = br#"{
            "update_id": 231,
            "message": {
                "message_id": 14,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "photo": [
                    {"file_id": "BBBB", "file_unique_id": "u2", "width": 100, "height": 100, "file_size": 500}
                ],
                "caption": "/help on this photo",
                "caption_entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        match envelope.payload {
            ProductInboundPayload::Command(cmd) => {
                assert_eq!(cmd.command, "help");
                assert_eq!(cmd.trigger, ProductTriggerReason::BotCommand);
            }
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn message_without_from_classifies_as_noop_not_error() {
        // Copilot's payload.rs:419 finding: `message.from` is optional
        // in the Telegram schema (anonymous group admins, channel
        // posts that slipped through the kind filter, etc.). Returning
        // a hard `PayloadParseError` would force the webhook to retry
        // an otherwise-parseable update. The fail-soft path is `NoOp`
        // — the webhook acks 200 OK and Telegram does not retry.
        let payload = br#"{
            "update_id": 240,
            "message": {
                "message_id": 15,
                "date": 1700000000,
                "chat": {"id": -42, "type": "supergroup"},
                "text": "anonymous admin message"
            }
        }"#;
        let parsed = parse_telegram_update(payload, &evidence(), &install_id(), &policy())
            .expect("parse must not hard-error on missing `from`");
        assert!(
            matches!(parsed.payload, ProductInboundPayload::NoOp),
            "missing `from` must fail-soft to NoOp, got {parsed:?}"
        );
    }

    #[test]
    fn private_chat_message_creates_envelope() {
        let payload = br#"{
            "update_id": 100,
            "message": {
                "message_id": 11,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "hello"
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        assert_eq!(envelope.external_event_id.as_str(), "tg-install_alpha-100");
        assert_eq!(envelope.external_actor_ref.id(), "777");
        assert_eq!(envelope.external_conversation_ref.conversation_id(), "777");
        assert_eq!(
            envelope.external_conversation_ref.reply_target_message_id(),
            Some("11")
        );
        match envelope.payload {
            ProductInboundPayload::UserMessage(user) => {
                assert_eq!(user.text, "hello");
                assert_eq!(user.trigger, ProductTriggerReason::DirectChat);
            }
            other => panic!("expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn group_ambient_message_is_noop() {
        let payload = br#"{
            "update_id": 200,
            "message": {
                "message_id": 12,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "just chatting"
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        assert!(matches!(parsed.payload, ProductInboundPayload::NoOp));
    }

    #[test]
    fn group_explicit_mention_creates_envelope() {
        let payload = br#"{
            "update_id": 201,
            "message": {
                "message_id": 12,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "@ironclaw_bot please help",
                "entities": [{"type": "mention", "offset": 0, "length": 13}]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        match envelope.payload {
            ProductInboundPayload::UserMessage(user) => {
                assert_eq!(user.trigger, ProductTriggerReason::BotMention);
                assert_eq!(user.text, "please help");
            }
            other => panic!("expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn group_reply_to_bot_creates_envelope() {
        let payload = br#"{
            "update_id": 202,
            "message": {
                "message_id": 13,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "thanks",
                "reply_to_message": {
                    "message_id": 7,
                    "date": 1699999999,
                    "from": {"id": 9000, "is_bot": true, "first_name": "IronClaw"},
                    "chat": {"id": -42, "type": "supergroup"},
                    "text": "hi there"
                }
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        match envelope.payload {
            ProductInboundPayload::UserMessage(user) => {
                assert_eq!(user.trigger, ProductTriggerReason::ReplyToBot);
            }
            other => panic!("expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn unset_bot_user_id_does_not_match_a_group_reply() {
        let payload = br#"{
            "update_id": 206,
            "message": {
                "message_id": 17,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "ambient reply",
                "reply_to_message": {
                    "message_id": 7,
                    "date": 1699999999,
                    "from": {"id": 0, "is_bot": true, "first_name": "Unknown"},
                    "chat": {"id": -42, "type": "supergroup"},
                    "text": "hi there"
                }
            }
        }"#;
        let mut policy = policy();
        policy.bot_user_id = 0;
        assert!(matches!(
            normalize_telegram_update(payload, &install_id(), &policy).expect("normalizes"),
            TelegramInboundEvent::Ignore
        ));
    }

    #[test]
    fn group_recognized_command_creates_command_envelope() {
        let payload = br#"{
            "update_id": 203,
            "message": {
                "message_id": 14,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "/help@ironclaw_bot args here",
                "entities": [{"type": "bot_command", "offset": 0, "length": 18}]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        match envelope.payload {
            ProductInboundPayload::Command(cmd) => {
                assert_eq!(cmd.command, "help");
                assert_eq!(cmd.arguments, "args here");
                assert_eq!(cmd.trigger, ProductTriggerReason::BotCommand);
            }
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command_in_group_is_noop() {
        // /yolo isn't in the recognized list and there's no mention/reply.
        let payload = br#"{
            "update_id": 204,
            "message": {
                "message_id": 15,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "text": "/yolo",
                "entities": [{"type": "bot_command", "offset": 0, "length": 5}]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        assert!(matches!(parsed.payload, ProductInboundPayload::NoOp));
    }

    #[test]
    fn topic_message_keys_conversation_by_topic_not_message_id() {
        let payload = br#"{
            "update_id": 300,
            "message": {
                "message_id": 50,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "message_thread_id": 7,
                "text": "@ironclaw_bot hello",
                "entities": [{"type": "mention", "offset": 0, "length": 13}]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        assert_eq!(
            envelope.external_conversation_ref.topic_id(),
            Some("7"),
            "topic must be carried in conversation key"
        );
        assert_eq!(
            envelope.external_conversation_ref.reply_target_message_id(),
            Some("50"),
            "reply target must come from message_id"
        );
        // Same chat, different message_id, same topic -> identical fingerprint.
        let payload2 = br#"{
            "update_id": 301,
            "message": {
                "message_id": 51,
                "date": 1700000001,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": -42, "type": "supergroup"},
                "message_thread_id": 7,
                "text": "@ironclaw_bot more",
                "entities": [{"type": "mention", "offset": 0, "length": 13}]
            }
        }"#;
        let parsed2 =
            parse_telegram_update(payload2, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope2 = parsed2;
        assert_eq!(
            envelope
                .external_conversation_ref
                .conversation_fingerprint(),
            envelope2
                .external_conversation_ref
                .conversation_fingerprint(),
        );
        // Reply targets differ.
        assert_ne!(
            envelope.external_conversation_ref.reply_target_message_id(),
            envelope2
                .external_conversation_ref
                .reply_target_message_id(),
        );
    }

    #[test]
    fn private_chat_with_photo_emits_attachment_descriptor_no_bytes() {
        let payload = br#"{
            "update_id": 400,
            "message": {
                "message_id": 22,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "caption": "look",
                "photo": [
                    {"file_id": "AAAA", "file_size": 1024},
                    {"file_id": "BBBB", "file_size": 8192}
                ]
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        let envelope = parsed;
        match envelope.payload {
            ProductInboundPayload::UserMessage(user) => {
                assert_eq!(user.attachments.len(), 1);
                assert_eq!(user.attachments[0].external_file_id, "BBBB");
                assert_eq!(user.attachments[0].kind, ProductAttachmentKind::Image);
                let json = serde_json::to_value(&user.attachments[0]).expect("serialize");
                assert!(json.get("data").is_none());
                assert!(json.get("source_url").is_none());
            }
            other => panic!("expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn media_group_fragments_share_one_durable_event_identity() {
        let first = br#"{
            "update_id": 701,
            "message": {
                "message_id": 81,
                "media_group_id": "album-6364",
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "document": {
                    "file_id": "file-alpha",
                    "file_name": "alpha.txt",
                    "mime_type": "text/plain",
                    "file_size": 5
                }
            }
        }"#;
        let second = br#"{
            "update_id": 702,
            "message": {
                "message_id": 82,
                "media_group_id": "album-6364",
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": 777, "type": "private"},
                "caption": "read both",
                "document": {
                    "file_id": "file-beta",
                    "file_name": "beta.txt",
                    "mime_type": "text/plain",
                    "file_size": 4
                }
            }
        }"#;
        let TelegramInboundEvent::BatchFragment(first) =
            normalize_telegram_update(first, &install_id(), &policy()).expect("first normalizes")
        else {
            panic!("first media-group fragment must normalize");
        };
        let TelegramInboundEvent::BatchFragment(second) =
            normalize_telegram_update(second, &install_id(), &policy()).expect("second normalizes")
        else {
            panic!("second media-group fragment must normalize");
        };

        assert_eq!(
            first.message.event_id, second.message.event_id,
            "all media-group fragments must converge on one durable event"
        );
        assert_ne!(first.fragment_id, second.fragment_id);
    }

    #[test]
    fn media_group_identity_is_scoped_to_chat_and_thread() {
        let payload = |update_id: i64, chat_id: i64, thread_id: Option<i64>| {
            let mut message = serde_json::json!({
                "message_id": update_id,
                "media_group_id": "provider-reused-id",
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice"},
                "chat": {"id": chat_id, "type": "private"},
                "document": {
                    "file_id": format!("file-{update_id}"),
                    "file_name": "report.txt",
                    "mime_type": "text/plain",
                    "file_size": 5
                }
            });
            if let Some(thread_id) = thread_id {
                message["message_thread_id"] = serde_json::json!(thread_id);
            }
            serde_json::to_vec(&serde_json::json!({
                "update_id": update_id,
                "message": message,
            }))
            .expect("payload serializes")
        };
        let normalize = |payload: Vec<u8>| {
            let TelegramInboundEvent::BatchFragment(fragment) =
                normalize_telegram_update(&payload, &install_id(), &policy())
                    .expect("media group normalizes")
            else {
                panic!("expected media-group fragment");
            };
            fragment
        };

        let first = normalize(payload(801, 777, None));
        let other_chat = normalize(payload(802, 778, None));
        let other_thread = normalize(payload(803, 777, Some(42)));

        assert_ne!(first.batch_key, other_chat.batch_key);
        assert_ne!(first.message.event_id, other_chat.message.event_id);
        assert_ne!(first.batch_key, other_thread.batch_key);
        assert_ne!(first.message.event_id, other_thread.message.event_id);
    }

    #[test]
    fn channel_post_is_noop() {
        let payload = br#"{
            "update_id": 500,
            "channel_post": {
                "message_id": 1,
                "date": 1700000000,
                "chat": {"id": -1001, "type": "channel"},
                "text": "broadcast"
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        assert!(matches!(parsed.payload, ProductInboundPayload::NoOp));
    }

    #[test]
    fn edited_message_is_noop() {
        let payload = br#"{
            "update_id": 600,
            "edited_message": {
                "message_id": 1,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false},
                "chat": {"id": 777, "type": "private"},
                "text": "edited"
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parse");
        assert!(matches!(parsed.payload, ProductInboundPayload::NoOp));
    }

    #[test]
    fn malformed_json_is_invalid_json_error() {
        let payload = b"this is not json";
        let err = parse_telegram_update(payload, &evidence(), &install_id(), &policy())
            .expect_err("malformed");
        assert!(matches!(err, PayloadParseError::InvalidJson { .. }));
    }

    /// Ben's regression (2026-07-17): the shared busy-on-auth hint tells the
    /// user to reply `auth deny gate:<ref>` in this chat, but Telegram's
    /// parse treated that reply as a plain `UserMessage` — it bounced off
    /// the busy thread with the same hint, forever. The advertised
    /// interaction grammar (shared with Slack via
    /// `ironclaw_host_api::product_adapter::interaction_commands`) must parse here.
    #[test]
    fn dm_auth_deny_command_parses_to_auth_resolution_not_user_message() {
        let payload = br#"{
            "update_id": 300,
            "message": {
                "message_id": 30,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "auth deny gate:auth-abc123"
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parses");
        match parsed.payload {
            ironclaw_host_api::product_adapter::ProductInboundPayload::AuthResolution(
                resolution,
            ) => {
                assert_eq!(resolution.auth_request_ref, "gate:auth-abc123");
            }
            other => panic!("expected AuthResolution, got {other:?}"),
        }
    }

    /// The hint renders the command in backticks; Telegram clients copy them.
    #[test]
    fn dm_backticked_approve_command_parses_to_approval_resolution() {
        let payload = br#"{
            "update_id": 301,
            "message": {
                "message_id": 31,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "`approve gate:approval-9`"
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parses");
        assert!(
            matches!(
                parsed.payload,
                ironclaw_host_api::product_adapter::ProductInboundPayload::ApprovalResolution(_)
            ),
            "got {:?}",
            parsed.payload
        );
    }

    /// Guard: ordinary conversation that merely starts with a verb-like word
    /// still routes as a user message.
    #[test]
    fn dm_ordinary_text_still_routes_as_user_message() {
        let payload = br#"{
            "update_id": 302,
            "message": {
                "message_id": 32,
                "date": 1700000000,
                "from": {"id": 777, "is_bot": false, "first_name": "Alice", "username": "alice"},
                "chat": {"id": 777, "type": "private"},
                "text": "hello there, what can you do?"
            }
        }"#;
        let parsed =
            parse_telegram_update(payload, &evidence(), &install_id(), &policy()).expect("parses");
        assert!(
            matches!(
                parsed.payload,
                ironclaw_host_api::product_adapter::ProductInboundPayload::UserMessage(_)
            ),
            "got {:?}",
            parsed.payload
        );
    }
}

/// Property tests for the Telegram ingress boundary (#6524 workstream 9:
/// "focused fuzzing for untrusted ingress").
///
/// The sibling of the Slack ingress properties. Both entry points sit on a
/// public webhook and see whatever the internet sends before any secret has
/// been trusted, so covering one and not the other would leave half the
/// surface unexamined while the box read as closed.
#[cfg(test)]
mod ingress_properties {
    use super::*;
    use ironclaw_host_api::product_adapter::auth::mark_shared_secret_header_verified;
    use proptest::prelude::*;

    fn verified_evidence() -> ProtocolAuthEvidence {
        mark_shared_secret_header_verified(
            "X-Telegram-Bot-Api-Secret-Token",
            "telegram_install_property",
        )
    }

    fn unverified_evidence() -> ProtocolAuthEvidence {
        ProtocolAuthEvidence::failed(
            ironclaw_host_api::product_adapter::ProtocolAuthFailure::Missing,
        )
    }

    fn install() -> AdapterInstallationId {
        AdapterInstallationId::new("install_property").expect("valid")
    }

    fn trigger_policy() -> GroupTriggerPolicy {
        GroupTriggerPolicy {
            bot_username: "ironclaw_bot".into(),
            bot_user_id: 9000,
            recognized_commands: vec!["start".into(), "help".into()],
        }
    }

    /// Update-shaped payloads plus noise.
    ///
    /// Biased for the same reason as the Slack generator: uniform random bytes
    /// die at the JSON parser and exercise only the outermost guard, never the
    /// branches that read chat, entities or commands.
    fn update_bytes() -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            proptest::collection::vec(any::<u8>(), 0..256),
            "\\PC{0,200}".prop_map(|s| s.into_bytes()),
            (any::<i64>(), "\\PC{0,40}", any::<i64>(), "[a-z_]{0,12}").prop_map(
                |(update_id, text, chat_id, chat_type)| {
                    serde_json::json!({
                        "update_id": update_id,
                        "message": {
                            "message_id": 1,
                            "date": 0,
                            "text": text,
                            "chat": {"id": chat_id, "type": chat_type},
                        }
                    })
                    .to_string()
                    .into_bytes()
                }
            ),
            // Command-shaped text, which drives the entity/command branches.
            ("[a-z]{1,10}", any::<i64>()).prop_map(|(command, chat_id)| {
                serde_json::json!({
                    "update_id": 7,
                    "message": {
                        "message_id": 1,
                        "date": 0,
                        "text": format!("/{command}@ironclaw_bot payload"),
                        "entities": [{"type": "bot_command", "offset": 0, "length": command.len() + 1}],
                        "chat": {"id": chat_id, "type": "group"},
                    }
                })
                .to_string()
                .into_bytes()
            }),
        ]
    }

    proptest! {
        /// No update is parsed without verified evidence.
        ///
        /// Telegram's webhook secret is the only thing standing between the
        /// public endpoint and an injected turn, exactly as the Slack
        /// signature is on that side.
        #[test]
        fn unverified_evidence_rejects_every_update(raw in update_bytes()) {
            let outcome =
                parse_telegram_update(&raw, &unverified_evidence(), &install(), &trigger_policy());
            prop_assert!(
                matches!(outcome, Err(PayloadParseError::UnauthenticatedPayload)),
                "unverified update was not rejected: {outcome:?}"
            );
        }

        /// Verified evidence plus arbitrary bytes parses or errors, never panics.
        #[test]
        fn verified_evidence_never_panics(raw in update_bytes()) {
            let _ =
                parse_telegram_update(&raw, &verified_evidence(), &install(), &trigger_policy());
            let _ = normalize_telegram_update(&raw, &install(), &trigger_policy());
        }
    }

    /// Telegram accepts an unbounded body where Slack refuses over 1 MiB.
    ///
    /// Recorded rather than asserted as a limit, because there is none here to
    /// assert. Whether that matters depends on a cap in the HTTP layer above,
    /// which this function cannot see — so the honest thing is to pin the
    /// current behaviour (a large body is parsed on its merits, not refused on
    /// length) and leave the question visible instead of implying parity that
    /// does not exist.
    #[test]
    fn large_bodies_are_judged_on_content_not_length() {
        let padding = "a".repeat(2 * 1024 * 1024);
        let payload = serde_json::json!({
            "update_id": 11,
            "message": {
                "message_id": 1,
                "date": 0,
                "text": padding,
                "chat": {"id": 42, "type": "private"},
            }
        })
        .to_string()
        .into_bytes();
        assert!(payload.len() > 1024 * 1024);

        let outcome = parse_telegram_update(
            &payload,
            &verified_evidence(),
            &install(),
            &trigger_policy(),
        );
        // Asserting only "not UnauthenticatedPayload" would be satisfied by
        // BodyTooLarge, or by any other parse error -- including the size
        // rejection this test exists to rule out. Pin the success instead.
        assert!(
            outcome.is_ok(),
            "a large body must still be judged on content, not rejected on \
             size or authentication; got {outcome:?}"
        );
    }
}
