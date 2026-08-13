//! Telegram Bot API payload normalization.
//!
//! Inputs are raw, host-verified webhook update bytes. Outputs are the
//! provider-neutral channel contract: a complete message, a batch fragment,
//! or an authenticated ignore decision. Product envelopes are constructed
//! only after this package boundary.

use ironclaw_extension_contracts::channel_adapter::{
    ChannelAttachmentRef, InboundBatchFragment, NormalizedInboundMessage, ProductTriggerReason,
};
use ironclaw_extension_contracts::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
    ProductAttachmentKind,
};
use ironclaw_host_api::product_adapter::AdapterInstallationId;
use serde::Deserialize;
use thiserror::Error;

use crate::attachment_transfer::{ParsedTelegramBatchFragment, ParsedTelegramInboundMessage};

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
    Message(Box<ParsedTelegramInboundMessage>),
    BatchFragment(Box<ParsedTelegramBatchFragment>),
}

/// Parse one HOST-VERIFIED Telegram update into its normalized channel form.
/// Pure protocol work — no I/O, no secrets; the host executed the
/// `shared_secret_header` recipe before calling this. Applies the same
/// forwarding rules: private chats always forward; groups only on an explicit
/// trigger; everything else is an authenticated no-op.
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
    let pending_attachments = attachments
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
        attachments: Vec::new(),
        conversation_context: None,
        // Reply routing rides the conversation ref's thread anchors
        // (pre-coordinator delivery path); adopted when the P5 delivery
        // coordinator consumes stored contexts.
        reply_context: None,
    };
    let Some(media_group_key) = scoped_media_group_key else {
        return Ok(TelegramInboundEvent::Message(Box::new(
            ParsedTelegramInboundMessage {
                message: normalized,
                pending_attachments,
            },
        )));
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
    Ok(TelegramInboundEvent::BatchFragment(Box::new(
        ParsedTelegramBatchFragment {
            fragment,
            pending_attachments,
        },
    )))
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
#[path = "tests/payload_slice.rs"]
mod slice_tests;

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

fn extract_first_addressed_bot_command(
    message: &TelegramMessage,
    bot_username: &str,
) -> Option<(String, String)> {
    extract_first_matching_bot_command(message, bot_username, |_| true)
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
#[path = "tests/payload_normalized.rs"]
mod tests;
