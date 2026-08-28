//! Slack Events API payload normalization.
// arch-exempt: large_file, split into Events-API vs slash-form parsing vs shared normalization modules, plan #6894
//!
//! Inputs are raw, host-verified Slack webhook bytes. Outputs are the
//! provider-neutral channel contract: a normalized complete-message precursor,
//! an immediate verification response, or an authenticated ignore decision.

use ironclaw_extension_contracts::channel_adapter::{
    ChannelAttachmentRef, NormalizedInboundMessage, ProductTriggerReason,
};
use ironclaw_extension_contracts::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
    ProductAttachmentKind,
};
use ironclaw_host_api::product_adapter::AdapterInstallationId;
use serde::Deserialize;
use thiserror::Error;

pub const SLACK_API_HOST: &str = "slack.com";
pub const SLACK_USER_ACTOR_KIND: &str = "slack_user";
const SLACK_FILE_SHARE_SUBTYPE: &str = "file_share";
/// Slack's marker for a post authored by an integration. Authorship, not
/// rendering — see the guard in [`normalize_user_message`].
const SLACK_BOT_MESSAGE_SUBTYPE: &str = "bot_message";
/// Slack's marker for a canvas-body mention: a person mentions the bot
/// inside a canvas document, and Slack fires `app_mention` for it — but with
/// this `subtype` set, `text` holding a Slack-written caption ("was
/// mentioned in a canvas") rather than what the person typed, and no
/// `bot_id`. The person's actual words live only in `blocks`, a field this
/// contract never reads. Documented at
/// <https://docs.slack.dev/reference/events/message/document_mention/>,
/// whose example payload is exactly this shape; the same docs note that a
/// mention inside a threaded *comment* still arrives as an ordinary
/// `app_mention` with no such subtype. That makes `document_mention` the
/// one documented case where `app_mention` is not a person addressing the
/// bot in conversation — checked in [`normalize_user_message`] ahead of the
/// `AppMention` exemption, the same way `SLACK_BOT_MESSAGE_SUBTYPE` is
/// checked ahead of it, so it is rejected on every path rather than
/// exempted by it.
const SLACK_DOCUMENT_MENTION_SUBTYPE: &str = "document_mention";
/// Manifest handle for the bot's own member id (see `manifest.toml`).
const SLACK_BOT_USER_ID_HANDLE: &str = "slack_bot_user_id";

/// Message `subtype` values that carry one person's own message, rendered
/// specially.
///
/// Slack stamps `subtype` on two unrelated families: these, and its own
/// channel announcements (`channel_join`, `channel_topic`, `bot_add`, …).
/// No field distinguishes the two, so the human family is named explicitly
/// and everything else is treated as not-a-person-speaking.
///
/// This list governs `message` events only. An `app_mention` is exempt —
/// see [`normalize_user_message`] — so a human subtype missing from here
/// can never silence a channel mention.
const HUMAN_MESSAGE_SUBTYPES: &[&str] = &[
    SLACK_FILE_SHARE_SUBTYPE,
    // A threaded reply sent with "Also send to channel". Slack stamps it on
    // both events it emits for that one message.
    "thread_broadcast",
    // `thread_broadcast`'s deprecated predecessor, still emitted by older
    // clients.
    "reply_broadcast",
    // `/me`-style italicized message.
    "me_message",
];

/// Maximum accepted byte length for any Slack inbound webhook payload.
const MAX_SLACK_PAYLOAD_BYTES: usize = 1024 * 1024; // 1 MB

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlackPayloadParseError {
    #[error("invalid Slack event JSON: {reason}")]
    InvalidJson { reason: String },
    #[error("invalid Slack slash-command form: {reason}")]
    InvalidForm { reason: String },
    #[error("invalid external reference: {kind}: {reason}")]
    InvalidExternalRef { kind: &'static str, reason: String },
}

// ── Channel-normalized parsing (generic ingress router, extension-runtime P4) ──

/// One host-verified Slack inbound request, normalized for the generic
/// channel-adapter contract: a URL-verification challenge, an ignored event,
/// or one plain user message. Gate-resolution classification (`approve` /
/// `deny gate:<ref>` / `auth deny <ref>`) is deliberately NOT applied here —
/// the shared host sink applies the channel-neutral interaction grammar.
#[derive(Debug)]
pub enum SlackInboundEvent {
    UrlVerification {
        challenge: String,
    },
    /// Slack's one-time endpoint-verification probe for a native slash
    /// command (distinct from the Events API's `UrlVerification` challenge).
    /// Any 200 response satisfies it; the body is ignored.
    SslCheck,
    Ignore {
        reason: SlackIgnoreReason,
    },
    Message(Box<ParsedSlackInboundMessage>),
}

/// Why a verified Slack event produced no message.
///
/// Carried rather than discarded so the adapter can log one line per drop.
/// Every rejection on this path ends in [`SlackInboundEvent::Ignore`], the
/// router maps that to a bare `200`, and Slack's retry machinery is
/// satisfied — so a human message dropped by mistake leaves no trace
/// anywhere unless the reason travels with it. That is exactly how a
/// `thread_broadcast` mention stayed invisible in production.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlackIgnoreReason {
    /// Envelope is not `event_callback` (`app_rate_limited`, …).
    NotAnEventCallback,
    /// `event_callback` with no `event` object.
    MissingEventPayload,
    /// An event type this channel does not act on.
    UnsupportedEventType,
    /// A channel message that neither mentions the bot nor replies in a
    /// thread — bystander chatter.
    AmbientChannelMessage,
    /// Authored by an integration, including this bot's own echo.
    BotAuthored,
    /// An `app_mention` whose `subtype` is `document_mention`: a person did
    /// author the mention, but Slack — not that person — wrote the event's
    /// `text`, and their actual words live only in a `blocks` field this
    /// contract does not read. See [`SLACK_DOCUMENT_MENTION_SUBTYPE`].
    SyntheticMentionText,
    /// A `message` whose `subtype` is not in [`HUMAN_MESSAGE_SUBTYPES`].
    /// Never reached from an `app_mention`.
    NonUserMessageSubtype(String),
    /// A field the normalized contract cannot be built without.
    MissingField(&'static str),
}

/// Pure payload-normalization result retained inside the Slack package until
/// [`crate::channel::SlackChannelAdapter`] finishes vendor reads.
#[derive(Debug, PartialEq, Eq)]
pub struct ParsedSlackInboundMessage {
    pub message: NormalizedInboundMessage,
    pub pending_attachments: Vec<ChannelAttachmentRef>,
}

impl std::ops::Deref for ParsedSlackInboundMessage {
    type Target = NormalizedInboundMessage;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

/// Parse one host-verified Slack Events API request into its normalized
/// channel form. Pure protocol work — no I/O, no secrets; the host executed
/// the signature recipe before calling this.
pub fn normalize_slack_event(
    raw_payload: &[u8],
    installation_id: &AdapterInstallationId,
    bot_user_id: Option<&str>,
) -> Result<SlackInboundEvent, SlackPayloadParseError> {
    if raw_payload.len() > MAX_SLACK_PAYLOAD_BYTES {
        return Err(SlackPayloadParseError::InvalidJson {
            reason: "payload exceeds size limit".into(),
        });
    }
    let url_wrapper: SlackUrlVerificationWrapper =
        serde_json::from_slice(raw_payload).map_err(|err| SlackPayloadParseError::InvalidJson {
            reason: err.to_string(),
        })?;
    if url_wrapper.event_type == "url_verification" {
        let challenge =
            url_wrapper
                .challenge
                .ok_or_else(|| SlackPayloadParseError::InvalidExternalRef {
                    kind: "slack_url_verification_challenge",
                    reason: "missing challenge".to_string(),
                })?;
        return Ok(SlackInboundEvent::UrlVerification { challenge });
    }

    let wrapper: SlackEventWrapper =
        serde_json::from_slice(raw_payload).map_err(|err| SlackPayloadParseError::InvalidJson {
            reason: err.to_string(),
        })?;
    require_well_formed_envelope(wrapper.event_id.as_deref(), &wrapper.event_type)?;
    if wrapper.event_type != "event_callback" {
        return Ok(SlackInboundEvent::Ignore {
            reason: SlackIgnoreReason::NotAnEventCallback,
        });
    }
    let Some(event) = wrapper.event.as_ref() else {
        return Ok(SlackInboundEvent::Ignore {
            reason: SlackIgnoreReason::MissingEventPayload,
        });
    };
    let team_id = wrapper.team_id.as_deref();
    let kind = match event.event_type.as_str() {
        "app_mention" => SlackMessageKind::AppMention,
        "message" => {
            if is_dm_channel(
                event.channel.as_deref().unwrap_or_default(),
                event.channel_type.as_deref(),
            ) {
                SlackMessageKind::Dm
            } else if mentions_bot(event.text.as_deref().unwrap_or_default(), bot_user_id) {
                // A person put the bot's member id in the text, so this
                // message is addressed to it whether or not Slack also sent
                // `app_mention` — and Slack does not always send one. It is
                // reported unreliable for a mention made INSIDE an existing
                // thread (notably a thread predating the bot joining the
                // channel), and it does not fire for a mention that lives in
                // Block Kit or a legacy attachment rather than in `text`.
                // Depending on `app_mention` alone is what left a broadcast
                // reply unanswered, so the text is read as its own signal.
                //
                // This is checked BEFORE the thread/ambient split so it also
                // rescues a TOP-LEVEL message naming the bot, which the
                // ambient rule would otherwise drop for want of a thread
                // anchor.
                //
                // Note this is a distinct kind from `AppMention`: it decides
                // the TRIGGER, never admission. The subtype list still gates
                // it in `normalize_user_message`, which is what keeps Slack's
                // own announcements out — `bot_add` reads "added an
                // integration to this channel: <@BOT>" and names the bot by
                // construction, and a channel topic can be set to anything.
                SlackMessageKind::TextMention
            } else if event.thread_ts.is_some() {
                SlackMessageKind::ThreadReply
            } else {
                return Ok(SlackInboundEvent::Ignore {
                    reason: SlackIgnoreReason::AmbientChannelMessage,
                });
            }
        }
        _ => {
            return Ok(SlackInboundEvent::Ignore {
                reason: SlackIgnoreReason::UnsupportedEventType,
            });
        }
    };
    normalize_user_message(installation_id, team_id, event, kind, bot_user_id)
}

/// Parse one host-verified Slack inbound request that may be EITHER the
/// Events API's JSON envelope or a native slash-command form POST — Slack
/// registers both against the identical Request URL (one ingress route per
/// extension), distinguished only by the (host-forwarded, verification-
/// exempt) Content-Type header. The JSON branch delegates verbatim to
/// [`normalize_slack_event`] so the two entry points share exactly one JSON
/// parsing implementation; this function adds no new behavior to that path.
pub(crate) fn normalize_slack_inbound(
    raw_payload: &[u8],
    headers: &[(String, String)],
    installation_id: &AdapterInstallationId,
    bot_user_id: Option<&str>,
) -> Result<SlackInboundEvent, SlackPayloadParseError> {
    if is_form_urlencoded_content_type(headers) {
        return normalize_slack_slash_command(raw_payload, installation_id);
    }
    normalize_slack_event(raw_payload, installation_id, bot_user_id)
}

/// Case-insensitive Content-Type match for Slack's slash-command / ssl_check
/// form encoding. Absent or non-matching headers fall through to the JSON
/// path — the pre-existing default behavior.
fn is_form_urlencoded_content_type(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && value
                .to_ascii_lowercase()
                .contains("application/x-www-form-urlencoded")
    })
}

/// Parse one native Slack slash-command form POST (`ssl_check` handshake or
/// a real `/ironclaw ...` invocation) into its normalized channel form.
fn normalize_slack_slash_command(
    raw_payload: &[u8],
    installation_id: &AdapterInstallationId,
) -> Result<SlackInboundEvent, SlackPayloadParseError> {
    if raw_payload.len() > MAX_SLACK_PAYLOAD_BYTES {
        return Err(SlackPayloadParseError::InvalidForm {
            reason: "payload exceeds size limit".into(),
        });
    }

    // Slack's ssl_check verification probe carries ONLY `ssl_check` +
    // `token` — never the slash command's mandatory fields. Check for it
    // via a minimal, all-Option probe BEFORE parsing the full form (which
    // requires channel_id/user_id/command/trigger_id), or the probe would
    // always fail mandatory-field validation.
    let probe: SlackSlashCommandProbe =
        serde_urlencoded::from_bytes(raw_payload).map_err(|err| {
            SlackPayloadParseError::InvalidForm {
                reason: err.to_string(),
            }
        })?;
    if probe.ssl_check.is_some() {
        return Ok(SlackInboundEvent::SslCheck);
    }

    let form: SlackSlashCommandForm = serde_urlencoded::from_bytes(raw_payload).map_err(|err| {
        SlackPayloadParseError::InvalidForm {
            reason: err.to_string(),
        }
    })?;

    let event_id = build_slash_event_id(installation_id, &form.trigger_id)?;
    let actor = build_actor_ref(&form.user_id)?;
    let conversation =
        build_conversation_ref(form.team_id.as_deref(), &form.channel_id, None, None)?;
    let is_dm = form.channel_name.as_deref() == Some("directmessage")
        || is_dm_channel(&form.channel_id, None);
    let trigger = if is_dm {
        ProductTriggerReason::DirectChat
    } else {
        ProductTriggerReason::BotCommand
    };
    let text = slash_command_dispatch_text(&form.command, form.text.as_deref());

    Ok(SlackInboundEvent::Message(Box::new(
        ParsedSlackInboundMessage {
            message: NormalizedInboundMessage {
                actor,
                conversation,
                event_id,
                text,
                trigger,
                attachments: Vec::new(),
                conversation_context: None,
                reply_context: None,
            },
            pending_attachments: Vec::new(),
        },
    )))
}

/// Map a slash command's `command` + `text` fields to the dispatcher's
/// invocation text. `/ironclaw` is this extension's own registered command:
/// empty or `help` text becomes `/help`; otherwise the text becomes the
/// dispatched command, defensively stripped of a leading `/` first so a
/// user typing `/ironclaw /status` does not double it to `//status`. A
/// DIFFERENT registered command name (an app-config mistake pointing a
/// second slash command at this same URL) is passed through verbatim as
/// `"{command} {text}"` — the generic classifier/admission layer rejects it
/// as undeclared, with help, rather than this adapter guessing intent.
fn slash_command_dispatch_text(command: &str, text: Option<&str>) -> String {
    let text = text.unwrap_or_default().trim();
    if command != "/ironclaw" {
        return format!("{command} {text}").trim().to_string();
    }
    if text.is_empty() || text.eq_ignore_ascii_case("help") {
        return "/help".to_string();
    }
    let stripped = text.strip_prefix('/').unwrap_or(text);
    format!("/{stripped}")
}

fn build_slash_event_id(
    installation_id: &AdapterInstallationId,
    trigger_id: &str,
) -> Result<ExternalEventId, SlackPayloadParseError> {
    // Namespaced separately from the Events API's `event_callback` id space
    // (same defensive rationale as build_event_id's own `-noop-` namespace):
    // a slash invocation and an Events API callback must never collide on
    // dedup key even if some future id happened to coincide.
    ExternalEventId::new(format!(
        "slack-{}-slash-{trigger_id}",
        installation_id.as_str()
    ))
    .map_err(|err| SlackPayloadParseError::InvalidExternalRef {
        kind: "external_event_id",
        reason: err.to_string(),
    })
}

/// Fixed user-message routing strategies.
/// `AppMention`: Slack's own `app_mention` event — public channel, strip
/// leading `@mention`, thread fallback to `ts`.
/// `TextMention`: a `message` event whose TEXT names the bot. Same routing as
/// `AppMention`, but deliberately a distinct kind because it is NOT exempt
/// from the subtype list — see [`normalize_user_message`].
/// `Dm`: direct-message channel required, keep text verbatim, no thread fallback.
/// `ThreadReply`: channel thread reply, strip an optional leading `@mention`,
/// require `thread_ts`.
#[derive(Debug, Clone, Copy)]
enum SlackMessageKind {
    AppMention,
    TextMention,
    Dm,
    ThreadReply,
}

/// Slack renders a mention as `<@U…>` (or `<@U…|handle>`), so a person naming
/// the bot is detectable from the message text alone, without Slack's
/// `app_mention` event having to arrive.
fn mentions_bot(text: &str, bot_user_id: Option<&str>) -> bool {
    let Some(bot) = bot_user_id.filter(|id| !id.is_empty()) else {
        return false;
    };
    text.contains(&format!("<@{bot}>")) || text.contains(&format!("<@{bot}|"))
}

/// The bot's own member id, from host-resolved, manifest-declared non-secret
/// configuration (`slack_bot_user_id`). Absent only on an installation
/// configured before the handle existed; mention detection then degrades to
/// `app_mention` alone, which is the behavior that shipped before this.
pub fn slack_bot_user_id(config: &[(String, String)]) -> Option<&str> {
    config
        .iter()
        .find(|(handle, _)| handle == SLACK_BOT_USER_ID_HANDLE)
        .map(|(_, value)| value.as_str())
}

fn normalize_user_message(
    installation_id: &AdapterInstallationId,
    team_id: Option<&str>,
    event: &SlackEvent,
    kind: SlackMessageKind,
    bot_user_id: Option<&str>,
) -> Result<SlackInboundEvent, SlackPayloadParseError> {
    // Authorship. Universal, and the reason it is universal is loop
    // prevention: a mention exchange between two apps does not terminate.
    //
    // `bot_id` is the field Slack sets on an integration's own post;
    // `bot_message` is the same claim made in the subtype field. It is
    // checked HERE, with authorship, rather than below with the rendering
    // shapes — the `app_mention` exemption exempts "how does this render",
    // never "who wrote this", so loop prevention holds on every path.
    if event.bot_id.is_some() || event.subtype.as_deref() == Some(SLACK_BOT_MESSAGE_SUBTYPE) {
        return Ok(SlackInboundEvent::Ignore {
            reason: SlackIgnoreReason::BotAuthored,
        });
    }
    // Canvas-body mention. Checked HERE, ahead of the `AppMention`
    // exemption below, for the same reason `bot_message` is checked ahead
    // of it: the exemption below is a statement about *rendering*
    // ("`subtype` doesn't gate an `app_mention`"), and this is not a
    // rendering question. `document_mention` is the one documented case
    // where Slack firing `app_mention` does NOT mean a person addressed
    // the bot in conversation — a person did trigger it, but Slack itself
    // wrote `text` (a caption such as "was mentioned in a canvas"), and
    // the person's real words are only in `blocks`, which this contract
    // never reads. Admitting it would start a full agent turn on
    // Slack-generated text. See [`SLACK_DOCUMENT_MENTION_SUBTYPE`] for the
    // docs citation; a mention in a threaded *comment* carries no such
    // subtype and is unaffected.
    if event.subtype.as_deref() == Some(SLACK_DOCUMENT_MENTION_SUBTYPE) {
        return Ok(SlackInboundEvent::Ignore {
            reason: SlackIgnoreReason::SyntheticMentionText,
        });
    }
    // Content shape — deliberately NOT asked of an `app_mention`.
    //
    // `subtype` answers "how does this message render"; admission needs
    // "did a human address this bot". Those are different questions, and
    // Slack has already answered the second one by emitting `app_mention`
    // at all — it does that when a person types the bot's member id, and
    // never for its own channel announcements. Asking the shape question
    // anyway is what dropped a `thread_broadcast` mention in production:
    // the rendering of a message the user definitely addressed to us
    // decided whether we were allowed to read it.
    //
    // `AppMention` is reachable only from the `"app_mention"` arm of
    // `normalize_slack_event`, so matching on `kind` here is exact rather
    // than a proxy. Every other kind still consults the list, which is
    // what keeps Slack's announcements out of a DM.
    if !matches!(kind, SlackMessageKind::AppMention)
        && let Some(subtype) = event.subtype.as_deref()
        && !is_user_generated_message_subtype(Some(subtype))
    {
        return Ok(SlackInboundEvent::Ignore {
            reason: SlackIgnoreReason::NonUserMessageSubtype(subtype.to_string()),
        });
    }
    // A message mutation (`message_changed`, `message_deleted`) keeps its
    // author and text under a nested `message` object, so a missing
    // top-level author is the structural half of the subtype rule above —
    // and the half that still binds on the exempt `app_mention` path.
    let Some(user) = event.user.as_deref() else {
        return Ok(SlackInboundEvent::Ignore {
            reason: SlackIgnoreReason::MissingField("user"),
        });
    };
    let Some(channel) = event.channel.as_deref() else {
        return Ok(SlackInboundEvent::Ignore {
            reason: SlackIgnoreReason::MissingField("channel"),
        });
    };
    // `kind` was derived from `channel_type` and `thread_ts` in
    // `normalize_slack_event`, this function's only caller, so re-checking
    // either here could only ever agree. The checks are gone rather than
    // restated, which is what lets `MissingField` only ever name a field
    // that is genuinely absent.
    let Some(ts) = event.ts.as_deref() else {
        return Ok(SlackInboundEvent::Ignore {
            reason: SlackIgnoreReason::MissingField("ts"),
        });
    };

    let event_id = build_message_event_id(installation_id, team_id, channel, ts)?;

    let raw_text = event.text.as_deref().unwrap_or_default();
    let (text, thread_ts, trigger) = match kind {
        SlackMessageKind::AppMention | SlackMessageKind::TextMention => (
            strip_leading_bot_mention(raw_text, bot_user_id),
            event.thread_ts.as_deref().or(Some(ts)),
            ProductTriggerReason::BotMention,
        ),
        SlackMessageKind::Dm => (
            raw_text.to_string(),
            event.thread_ts.as_deref(),
            ProductTriggerReason::DirectChat,
        ),
        SlackMessageKind::ThreadReply => (
            strip_leading_bot_mention(raw_text, bot_user_id),
            event.thread_ts.as_deref(),
            ProductTriggerReason::ReplyToBot,
        ),
    };

    let actor = build_actor_ref(user)?;
    let conversation = build_conversation_ref(team_id, channel, thread_ts, Some(ts))?;
    let pending_attachments = collect_attachments(&event.files)?
        .into_iter()
        .map(|descriptor| ChannelAttachmentRef {
            vendor_ref: descriptor.external_file_id.clone(),
            descriptor,
        })
        .collect();
    Ok(SlackInboundEvent::Message(Box::new(
        ParsedSlackInboundMessage {
            message: NormalizedInboundMessage {
                actor,
                conversation,
                event_id,
                text,
                trigger,
                attachments: Vec::new(),
                conversation_context: None,
                reply_context: None,
            },
            pending_attachments,
        },
    )))
}

/// An `event_callback` must carry `event_id`. It is no longer the dedup key
/// (see [`build_message_event_id`]) but a callback without one is malformed,
/// and accepting it would mean accepting an envelope Slack did not produce.
fn require_well_formed_envelope(
    event_id: Option<&str>,
    wrapper_event_type: &str,
) -> Result<(), SlackPayloadParseError> {
    if wrapper_event_type == "event_callback" && event_id.is_none() {
        return Err(SlackPayloadParseError::InvalidExternalRef {
            kind: "external_event_id",
            reason: "event_callback must carry event_id".to_string(),
        });
    }
    Ok(())
}

/// The dedup key for one Slack MESSAGE, not one Slack EVENT.
///
/// Slack announces a single post as up to two events — `app_mention` and
/// `message` — with DISTINCT `event_id`s, and both can now start a run. Keyed
/// on `event_id` the durable admission gate cannot see they are the same
/// message and would admit two runs, so one @mention would be answered twice.
/// `(team, channel, ts)` is the identity Slack itself gives the message, so
/// the twins collapse to exactly one run whichever of them arrives first.
/// (`openclaw` and `suna` key the same collapse on the same triple.)
fn build_message_event_id(
    installation_id: &AdapterInstallationId,
    team_id: Option<&str>,
    channel: &str,
    ts: &str,
) -> Result<ExternalEventId, SlackPayloadParseError> {
    ExternalEventId::new(format!(
        "slack-{}-msg-{}-{channel}-{ts}",
        installation_id.as_str(),
        team_id.unwrap_or("noteam"),
    ))
    .map_err(|err| SlackPayloadParseError::InvalidExternalRef {
        kind: "external_event_id",
        reason: err.to_string(),
    })
}

fn build_actor_ref(user: &str) -> Result<ExternalActorRef, SlackPayloadParseError> {
    ExternalActorRef::new(SLACK_USER_ACTOR_KIND, user, None::<&str>).map_err(|err| {
        SlackPayloadParseError::InvalidExternalRef {
            kind: "external_actor_ref",
            reason: err.to_string(),
        }
    })
}

fn build_conversation_ref(
    team_id: Option<&str>,
    channel: &str,
    thread_ts: Option<&str>,
    message_ts: Option<&str>,
) -> Result<ExternalConversationRef, SlackPayloadParseError> {
    ExternalConversationRef::new(team_id, channel, thread_ts, message_ts).map_err(|err| {
        SlackPayloadParseError::InvalidExternalRef {
            kind: "external_conversation_ref",
            reason: err.to_string(),
        }
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

/// Strips the leading `<@U…>` / `<@U…|handle>` token ONLY when it names the
/// configured bot. A leading mention of anyone else stays: dropping it would
/// silently remove a third party from the prompt while leaving the bot's own
/// tag behind. This matters more now that [`SlackMessageKind::TextMention`]
/// admits messages naming the bot anywhere in the text, not just in front.
/// Non-leading mentions are never touched — the model sees them verbatim.
///
/// Without a configured `bot_user_id` this degrades to the pre-existing
/// behavior of stripping whatever mention leads, the same tradeoff
/// [`mentions_bot`] makes for detection.
fn strip_leading_bot_mention(text: &str, bot_user_id: Option<&str>) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("<@")
        && let Some(end) = trimmed.find('>')
    {
        // The id is the segment before an optional `|handle>` suffix.
        let inside = &trimmed[2..end];
        let mention_id = inside.split_once('|').map_or(inside, |(id, _)| id);
        let names_bot = match bot_user_id.filter(|id| !id.is_empty()) {
            Some(bot) => mention_id == bot,
            None => true,
        };
        if names_bot {
            return trimmed[end + 1..].trim_start().to_string();
        }
    }
    trimmed.to_string()
}

fn is_user_generated_message_subtype(subtype: Option<&str>) -> bool {
    subtype.is_none_or(|value| HUMAN_MESSAGE_SUBTYPES.contains(&value))
}

fn is_dm_channel(channel: &str, channel_type: Option<&str>) -> bool {
    match channel_type {
        Some("im") => true,
        Some(_) => false,
        None => channel.starts_with('D'),
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

/// Minimal probe for Slack's `ssl_check` endpoint-verification POST, which
/// carries only `ssl_check` + `token` — never a slash command's mandatory
/// fields. Parsed before [`SlackSlashCommandForm`] so the probe never trips
/// that struct's required-field validation.
#[derive(Debug, Clone, Deserialize)]
struct SlackSlashCommandProbe {
    ssl_check: Option<String>,
}

/// One native Slack slash-command form POST
/// (`application/x-www-form-urlencoded`). Liberal on purpose — Slack adds
/// fields across API versions and this is a public untrusted-ingress
/// boundary — so only the fields the dispatcher mapping cannot proceed
/// without are mandatory; everything else is `Option`. There is no
/// `deny_unknown_fields`, so fields this crate does not yet consume
/// (`response_url` — future out-of-DM delivery; `ssl_check` — already
/// resolved by [`SlackSlashCommandProbe`] before this struct is parsed;
/// `token` — Slack's legacy verification field, superseded here by HMAC
/// signing) arrive and are silently dropped rather than declared dead.
#[derive(Debug, Clone, Deserialize)]
struct SlackSlashCommandForm {
    channel_id: String,
    user_id: String,
    command: String,
    trigger_id: String,
    text: Option<String>,
    channel_name: Option<String>,
    team_id: Option<String>,
}

#[cfg(test)]
#[path = "tests/payload_normalized.rs"]
mod tests;
