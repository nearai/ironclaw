//! Telegram ⇄ canonical standard-messaging mapping.
//!
//! Everything vendor-shaped that has to become — or come back from — the
//! host's canonical messaging vocabulary lives here: the opaque conversation /
//! user / cursor codecs, the untrusted-text sanitizer, the canonical value
//! builders, and the single vendor-error table.
//!
//! Four rules in this module are load-bearing enough that changing one is a
//! design change, not a refactor
//! (`docs/internal/design/telegram-linked-device/PROPOSAL.md` §6.2, §6.3, §6.6):
//!
//! 1. **A send that Telegram accepted is never reported as a failure.** A
//!    failure is what a model retries, and a retried send double-messages a
//!    human. See [`send_result`].
//! 2. **An unknown outcome is never reported as a send.** `Dropped`/`Io` on a
//!    write means the request may or may not have executed, which is not the
//!    same fact as "delivered but uncorrelated" — they get different answers.
//! 3. **`is_self` comes from `Message::outgoing()`**, never from a guess.
//! 4. **Credential failures leave this vocabulary entirely** and ride
//!    [`ToolError::AuthRequired`], so the run parks on the auth gate instead
//!    of teaching the model that Telegram rejected its arguments.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use grammers_client::{
    InvocationError,
    message::Message,
    peer::{Peer, User},
    session::types::{PeerAuth, PeerId, PeerKind, PeerRef},
};
use ironclaw_extension_contracts::tool_adapter::ToolError;
use ironclaw_host_api::{
    capability::RuntimeCredentialAccountSetup,
    decision::RuntimeCredentialAuthRequirement,
    dispatch::RuntimeDispatchErrorKind,
    ids::{ExtensionId, SecretHandle, VendorId},
    messaging::StandardMessagingErrorCode,
};
use serde_json::{Value, json};

use super::{MAX_MESSAGE_TEXT_BYTES, MAX_RESULT_BYTES};

/// The extension id and the credential handle the linked tools gate on. Both
/// are manifest facts (`manifest.toml`), restated here because
/// [`auth_required`] has to name them to park a run on the connect gate.
pub(crate) const TELEGRAM_EXTENSION_ID: &str = "telegram";
pub(crate) const TELEGRAM_VENDOR_ID: &str = "telegram";
pub(crate) const TELEGRAM_LINKED_SESSION_HANDLE: &str = "telegram_linked_session";

/// Prefix of the non-addressable author ref used for messages with no personal
/// author — broadcast-channel posts and anonymous-admin posts. Both are real
/// messages a history read must be able to return, and every read item
/// requires `author.user_ref` with `minLength: 1`, so filtering them would
/// silently hide content while fabricating a user identity would hand the
/// model something it could feed to `open_dm`.
///
/// The chosen answer is a ref that is honest about being neither: it carries
/// no user identity, it is tagged distinctly from a real user ref, and every
/// people operation rejects it with `messaging.unknown_user`
/// ([`UserRef::decode`] returns [`RefKind::Authorless`]).
const AUTHORLESS_TAG: &str = "x1";

const CONVERSATION_TAG: &str = "c1";
const USER_TAG: &str = "u1";
const CURSOR_TAG: &str = "p1";

// ---------------------------------------------------------------------------
// Failure construction
// ---------------------------------------------------------------------------

/// A canonical messaging failure. `safe_summary` carries only the fixed
/// canonical code string (never vendor payload); `model_visible_cause` carries
/// the one piece of vendor detail worth surfacing — today, a flood wait's
/// retry-after — as prose, because no structured retry-after slot is plumbed
/// through tool dispatch (§6.6).
pub(crate) fn failed(code: StandardMessagingErrorCode) -> ToolError {
    ToolError::Failed {
        kind: RuntimeDispatchErrorKind::OperationFailed,
        safe_summary: Some(format!("telegram rejected the request: {}", code.as_str())),
        model_visible_cause: None,
    }
}

/// Same, plus a model-visible cause. The cause is vendor-derived and is NOT
/// display-safe; downstream scrubbing owns that.
pub(crate) fn failed_because(code: StandardMessagingErrorCode, cause: String) -> ToolError {
    ToolError::Failed {
        kind: RuntimeDispatchErrorKind::OperationFailed,
        safe_summary: Some(format!("telegram rejected the request: {}", code.as_str())),
        model_visible_cause: Some(cause),
    }
}

/// The linked session is missing, revoked, or rejected by Telegram. This is
/// deliberately NOT a `messaging.*` code: the host's re-auth gate parks the
/// run and offers the connect affordance, which is a thing the user can fix,
/// whereas a messaging code would tell the model to rephrase its arguments.
pub(crate) fn auth_required() -> ToolError {
    ToolError::AuthRequired {
        required_secrets: SecretHandle::new(TELEGRAM_LINKED_SESSION_HANDLE)
            .map(|handle| vec![handle])
            .unwrap_or_default(),
        credential_requirements: match (
            VendorId::new(TELEGRAM_VENDOR_ID),
            ExtensionId::new(TELEGRAM_EXTENSION_ID),
        ) {
            (Ok(provider), Ok(requester_extension)) => vec![RuntimeCredentialAuthRequirement {
                provider,
                setup: RuntimeCredentialAccountSetup::DeviceLink,
                requester_extension,
                provider_scopes: Vec::new(),
            }],
            // Both ids are compile-time literals that satisfy their
            // validators; an empty requirement list still parks the run on the
            // generic re-auth gate, so this arm degrades rather than panics.
            _ => Vec::new(),
        },
    }
}

// ---------------------------------------------------------------------------
// Opaque refs
// ---------------------------------------------------------------------------

/// What a decoded ref turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefKind {
    /// A real peer, rehydratable to a [`PeerRef`] without a session cache hit.
    Peer(PeerRef),
    /// The non-addressable author of a channel post or anonymous-admin post.
    Authorless,
}

/// An opaque conversation ref: `(peer kind, bare id, access hash)`, encoded so
/// it rehydrates to a usable [`PeerRef`] with no cache lookup. Refs are not
/// human-authorable and are not valid on another extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConversationRef(PeerRef);

impl ConversationRef {
    pub(crate) fn from_peer_ref(peer: PeerRef) -> Self {
        Self(peer)
    }

    pub(crate) fn peer_ref(&self) -> PeerRef {
        self.0
    }

    pub(crate) fn encode(&self) -> String {
        encode_peer(CONVERSATION_TAG, self.0)
    }

    /// Decodes a model-supplied conversation ref. A ref that does not decode
    /// is `messaging.unknown_conversation` at the call site: Telegram
    /// deliberately does not distinguish "gone" from "private", so anything
    /// stronger would over-claim knowledge.
    pub(crate) fn decode(raw: &str) -> Option<Self> {
        decode_peer(CONVERSATION_TAG, raw).map(Self)
    }
}

/// An opaque user ref. Same encoding shape as a conversation ref under a
/// different tag, so a conversation ref can never be spent as a user ref (or
/// the reverse) even though a Telegram DM's peer *is* its counterpart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UserRef(PeerRef);

impl UserRef {
    pub(crate) fn from_peer_ref(peer: PeerRef) -> Self {
        Self(peer)
    }

    pub(crate) fn encode(&self) -> String {
        encode_peer(USER_TAG, self.0)
    }

    /// The ref carried by a message with no personal author. Distinct per
    /// conversation only so two channels' posts do not collide; it identifies
    /// nobody and cannot be spent on a people operation.
    pub(crate) fn authorless(conversation: &ConversationRef) -> String {
        let id = conversation.peer_ref().id.bare_id().unwrap_or_default();
        URL_SAFE_NO_PAD.encode(format!("{AUTHORLESS_TAG}:{id}"))
    }

    /// Decodes a model-supplied user ref, distinguishing a real peer from the
    /// authorless sentinel so the caller can answer `messaging.unknown_user`
    /// rather than dialling a channel.
    pub(crate) fn decode(raw: &str) -> Option<RefKind> {
        let decoded = URL_SAFE_NO_PAD.decode(raw).ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        if decoded.starts_with(&format!("{AUTHORLESS_TAG}:")) {
            return Some(RefKind::Authorless);
        }
        decode_peer_str(USER_TAG, &decoded).map(RefKind::Peer)
    }
}

fn encode_peer(tag: &str, peer: PeerRef) -> String {
    let kind = match peer.id.kind() {
        PeerKind::User => 'u',
        PeerKind::Chat => 'g',
        PeerKind::Channel => 'c',
    };
    let id = peer.id.bare_id().unwrap_or_default();
    URL_SAFE_NO_PAD.encode(format!("{tag}:{kind}:{id}:{}", peer.auth.hash()))
}

fn decode_peer(tag: &str, raw: &str) -> Option<PeerRef> {
    let decoded = URL_SAFE_NO_PAD.decode(raw).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    decode_peer_str(tag, &decoded)
}

fn decode_peer_str(tag: &str, decoded: &str) -> Option<PeerRef> {
    let mut parts = decoded.split(':');
    if parts.next()? != tag {
        return None;
    }
    let kind = parts.next()?;
    let id: i64 = parts.next()?.parse().ok()?;
    let hash: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let id = match kind {
        "u" => PeerId::user(id),
        "g" => PeerId::chat(id),
        "c" => PeerId::channel(id),
        _ => None,
    }?;
    Some(PeerRef {
        id,
        auth: PeerAuth::from_hash(hash),
    })
}

/// An opaque pagination cursor over Telegram's `offset_id` / `offset_date`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cursor {
    pub(crate) offset_id: i32,
    pub(crate) offset_date: i32,
}

impl Cursor {
    pub(crate) fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(format!(
            "{CURSOR_TAG}:{}:{}",
            self.offset_id, self.offset_date
        ))
    }

    /// A cursor that does not decode is a model-visible error at the call site
    /// (`messaging.unsupported_content`), never a silent restart at page one —
    /// a restart looks like success and quietly re-reads content the caller
    /// already has.
    pub(crate) fn decode(raw: &str) -> Option<Self> {
        let decoded = URL_SAFE_NO_PAD.decode(raw).ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        let mut parts = decoded.split(':');
        if parts.next()? != CURSOR_TAG {
            return None;
        }
        let offset_id: i32 = parts.next()?.parse().ok()?;
        let offset_date: i32 = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            offset_id,
            offset_date,
        })
    }
}

/// One canonical `message_ref` object. A Telegram message id is only
/// meaningful inside its own conversation, so the pair always travels
/// together.
pub(crate) fn message_ref(conversation: &ConversationRef, message_id: i32) -> Value {
    json!({
        "conversation": conversation.encode(),
        "message_id": message_id.to_string(),
    })
}

/// Reads a canonical `message_ref` back. Returns `None` when either half is
/// missing or unparseable — the caller answers `unknown_conversation` or
/// `unknown_message` depending on which half failed.
pub(crate) fn parse_message_ref(value: &Value) -> Option<(ConversationRef, i32)> {
    let conversation = value.get("conversation")?.as_str()?;
    let message_id = value.get("message_id")?.as_str()?;
    Some((
        ConversationRef::decode(conversation)?,
        message_id.parse().ok()?,
    ))
}

// ---------------------------------------------------------------------------
// Untrusted content
// ---------------------------------------------------------------------------

/// Strips control and text-direction characters from vendor text before it
/// becomes model-visible, then clamps it to [`MAX_MESSAGE_TEXT_BYTES`].
///
/// This mirrors the host's channel-context sanitizer (#7397,
/// `ironclaw_extension_host::extension_ingress`): `Cc` controls except
/// `\n`/`\t`, plus the zero-width space, the bidi controls/isolates and the
/// BOM — the exact set that lets an untrusted message reorder or hide text as
/// the model and the approval card see it. `U+200C`/`U+200D` survive
/// deliberately: they are required orthography in Persian and Hindi and glue
/// inside emoji sequences, not injection vectors.
///
/// An **empty result is legal and is returned as such**. All four canonical
/// read outputs type `text` as a bare string with no `minLength`; a media-only
/// message emits `""` plus a vendor content marker. Only an *absent* `text`
/// fails validation.
pub(crate) fn sanitize_untrusted_text(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len().min(MAX_MESSAGE_TEXT_BYTES));
    let mut characters = raw.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                text.push('\n');
            }
            '\n' | '\t' => text.push(character),
            character
                if character.is_control()
                    || matches!(
                        character,
                        '\u{200B}'
                            | '\u{200E}'
                            | '\u{200F}'
                            | '\u{202A}'..='\u{202E}'
                            | '\u{2066}'..='\u{2069}'
                            | '\u{FEFF}'
                    ) => {}
            character => text.push(character),
        }
    }
    truncate_on_char_boundary(text, MAX_MESSAGE_TEXT_BYTES)
}

fn truncate_on_char_boundary(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// Drops trailing items until the serialized result fits [`MAX_RESULT_BYTES`].
/// Trailing rather than leading because every list-shaped read here is
/// newest-first, so the oldest rows are the ones to lose.
pub(crate) fn bound_result_bytes(items: &mut Vec<Value>) -> bool {
    let mut clamped = false;
    while !items.is_empty() && serialized_len(items) > MAX_RESULT_BYTES {
        items.pop();
        clamped = true;
    }
    clamped
}

fn serialized_len(items: &[Value]) -> usize {
    serde_json::to_vec(items)
        .map(|bytes| bytes.len())
        .unwrap_or(
            // A value built from `serde_json::json!` cannot fail to serialize;
            // treating an impossible failure as "over budget" fails closed.
            MAX_RESULT_BYTES + 1,
        )
}

// ---------------------------------------------------------------------------
// Canonical value builders
// ---------------------------------------------------------------------------

/// The canonical conversation kind for a Telegram peer (§6.3, stated once):
/// a private chat with a user or bot — Saved Messages included — is a `dm`; a
/// basic group is a `group_dm`; a supergroup or a broadcast channel is a
/// `channel`.
pub(crate) fn conversation_kind(peer: &Peer) -> &'static str {
    match peer {
        Peer::User(_) => "dm",
        Peer::Group(group) => {
            if group.is_megagroup() {
                "channel"
            } else {
                "group_dm"
            }
        }
        Peer::Channel(_) => "channel",
    }
}

/// One canonical conversation object.
///
/// `counterpart` is REQUIRED whenever `kind == "dm"` — the contract's one
/// conditional, and it is declared only on `list_conversations` items, so it
/// is enforced here for both ops rather than left to the schema. The
/// counterpart ref is built from the *user peer* the dialog resolves to, not
/// by re-tagging the conversation string: for a Telegram DM the peer is the
/// user, and the two nouns keep separate encodings.
pub(crate) fn conversation_value(peer: &Peer, peer_ref: PeerRef) -> Value {
    let conversation = ConversationRef::from_peer_ref(peer_ref);
    let kind = conversation_kind(peer);
    let mut value = json!({
        "conversation": conversation.encode(),
        "kind": kind,
    });
    if let Some(name) = display_name(peer) {
        value["display_name"] = json!(name);
    }
    if kind == "dm" {
        let mut counterpart = json!({ "user_ref": UserRef::from_peer_ref(peer_ref).encode() });
        if let Some(name) = display_name(peer) {
            counterpart["display_name"] = json!(name);
        }
        value["counterpart"] = counterpart;
    }
    value
}

/// A Telegram `@username`, sanitized, or `None` when nothing usable survives.
///
/// The handle is the *stable* identity the approval card, `resolve_user`'s
/// disambiguation, and every "identify people by @username, not by display
/// name" instruction lean on. That is exactly why it goes through the same
/// sanitizer as free-form text rather than being trusted for being
/// handle-shaped: a handle carrying a bidi override or a zero-width space
/// renders as a *different* handle, which turns the one field the model was
/// told to trust into the spoof (§6.3). Telegram's own charset would forbid
/// those characters — but this adapter enforces the property rather than
/// assuming the vendor did.
fn handle(username: &str) -> Option<String> {
    let sanitized = sanitize_untrusted_text(username);
    let sanitized = sanitized.trim();
    (!sanitized.is_empty()).then(|| format!("@{sanitized}"))
}

/// A peer's rendered name, **sanitized**.
///
/// A group title is settable by any admin and a display name by the user
/// themselves, so both are attacker-controlled strings that end up on the
/// approval card. Stripping the bidi and zero-width set here is what stops
/// "send to Alice" being rendered over a name that is not Alice's.
fn display_name(peer: &Peer) -> Option<String> {
    let raw = match peer {
        Peer::User(user) => {
            let full = user.full_name();
            let full = full.trim().to_string();
            if full.is_empty() {
                return user.username().and_then(handle);
            }
            full
        }
        other => other.name().map(str::to_string)?,
    };
    let sanitized = sanitize_untrusted_text(&raw);
    (!sanitized.trim().is_empty()).then_some(sanitized)
}

/// One canonical user object (`get_user_info` / `whoami` / member and match
/// items share this shape; callers project the subset their schema allows).
///
/// `presence` is deliberately never emitted: Telegram's last-seen visibility
/// is a per-user privacy setting, so any value would be a guess, and the
/// canonical field's `unknown` is for vendors with no presence concept at all.
pub(crate) fn user_value(user: &User, peer_ref: PeerRef) -> Value {
    let mut value = json!({ "user_ref": UserRef::from_peer_ref(peer_ref).encode() });
    let full = user.full_name();
    let full = full.trim();
    let handle = user.username().and_then(handle);
    if !full.is_empty() {
        value["display_name"] = json!(sanitize_untrusted_text(full));
        value["real_name"] = json!(sanitize_untrusted_text(full));
    } else if let Some(handle) = &handle {
        value["display_name"] = json!(handle);
    }
    if user.is_bot() {
        value["is_bot"] = json!(true);
    }
    let mut vendor = json!({});
    // The `@` prefix belongs to the rendered form, not to the identity, so the
    // vendor block carries the bare handle — sanitized all the same. Exactly
    // one prefix is stripped: `handle` adds exactly one, and stripping greedily
    // would silently rewrite a name rather than un-render it.
    if let Some(handle) = handle
        .as_deref()
        .and_then(|handle| handle.strip_prefix('@'))
    {
        vendor["username"] = json!(handle);
    }
    if user.mutual_contact() {
        vendor["mutual_contact"] = json!(true);
    }
    if user.deleted() {
        vendor["deleted"] = json!(true);
    }
    if vendor.as_object().is_some_and(|map| !map.is_empty()) {
        value["vendor"] = vendor;
    }
    value
}

/// The vendor content marker for a message whose payload is not text.
fn content_kind(message: &Message) -> Option<&'static str> {
    use grammers_client::media::Media;
    message.media().map(|media| match media {
        Media::Photo(_) => "photo",
        Media::Sticker(_) => "sticker",
        Media::Document(_) => "document",
        Media::Contact(_) => "contact",
        Media::Poll(_) => "poll",
        Media::Geo(_) | Media::GeoLive(_) => "location",
        Media::Dice(_) => "dice",
        Media::Venue(_) => "venue",
        Media::WebPage(_) => "web_page",
        // `Media` is `#[non_exhaustive]`: a media kind added by a future
        // grammers release still has to say that *something* was attached,
        // because the alternative is a message that reads as empty.
        _ => "media",
    })
}

/// One canonical message object, or `None` for a message that must not appear
/// in a read result.
///
/// Filtered: pure service messages (joins, leaves, title changes, pins). They
/// carry no author-authored content and Telegram emits them constantly, so
/// returning them would spend the result budget on noise.
///
/// Kept with `text: ""`: media-only messages, stickers, and voice notes — each
/// with a `vendor.content_kind` marker naming what was actually sent, so the
/// model can tell "empty message" from "a photo".
pub(crate) async fn message_value(
    conversation: &ConversationRef,
    message: &Message,
) -> Option<Value> {
    if message.action().is_some() {
        return None;
    }
    let text = sanitize_untrusted_text(message.text());
    let content_kind = content_kind(message);
    let (author_ref, author_name) = author_identity(conversation, message).await;

    let mut author = json!({ "user_ref": author_ref });
    if let Some(name) = author_name {
        author["display_name"] = json!(sanitize_untrusted_text(&name));
    }

    let mut value = json!({
        "message_ref": message_ref(conversation, message.id()),
        "author": author,
        "text": text,
        // Authorship of a read message comes from the vendor's own outgoing
        // flag. Never fabricated true, never omitted.
        "is_self": message.outgoing(),
        "timestamp": message.date().to_rfc3339(),
    });
    if message.edit_date().is_some() {
        value["edited"] = json!(true);
    }
    if let Some(thread) = thread_anchor(message) {
        let mut thread_value = json!({ "thread": thread.to_string() });
        if let Some(count) = message.reply_count() {
            thread_value["reply_count"] = json!(count.max(0));
        }
        value["thread"] = thread_value;
    }
    let mut vendor = json!({});
    if let Some(kind) = content_kind {
        vendor["content_kind"] = json!(kind);
    }
    if message.post() {
        vendor["channel_post"] = json!(true);
    }
    if vendor.as_object().is_some_and(|map| !map.is_empty()) {
        value["vendor"] = vendor;
    }
    Some(value)
}

/// The author ref and display name for a message.
///
/// A broadcast-channel post has no sender at all, and an anonymous admin posts
/// as the chat itself, so neither yields a user. Both fall back to the
/// non-addressable authorless ref rather than being dropped or given a
/// fabricated identity.
async fn author_identity(
    conversation: &ConversationRef,
    message: &Message,
) -> (String, Option<String>) {
    let sender_ref = message.sender_ref().await.ok().flatten();
    match sender_ref {
        Some(peer_ref) if peer_ref.id.kind() == PeerKind::User => (
            UserRef::from_peer_ref(peer_ref).encode(),
            message.sender().and_then(display_name),
        ),
        _ => (
            UserRef::authorless(conversation),
            message.post_author().map(str::to_string),
        ),
    }
}

fn thread_anchor(message: &Message) -> Option<i32> {
    use grammers_client::tl::enums::MessageReplyHeader;
    match message.reply_header() {
        Some(MessageReplyHeader::Header(header)) => {
            header.reply_to_top_id.or(header.reply_to_msg_id)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Vendor error table (§6.6)
// ---------------------------------------------------------------------------

/// Which family of operation raised an error. The canonical answer to a few
/// Telegram error names depends on what the caller was addressing — a bad peer
/// is an unknown conversation for a chat op and an unknown user for a people
/// op — so the family travels into the table rather than the table guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpFamily {
    /// A vendor-mutating operation. `Dropped`/`Io` here is an unknown outcome.
    Write,
    /// A read addressed at a conversation or a message.
    Read,
    /// A read addressed at a person.
    People,
}

/// The complete §6.6 vendor error table, in one function.
///
/// Two rows are not messaging codes at all and that is the point:
/// `AUTH_KEY_UNREGISTERED` / `SESSION_REVOKED` leave for
/// [`ToolError::AuthRequired`], and `USER_DEACTIVATED` stays a vendor error
/// here while the *account* moves to a terminal state elsewhere (§4.5) — it is
/// not re-linkable, so prompting for re-auth forever would be wrong.
pub(crate) fn map_vendor_error(family: OpFamily, error: &InvocationError) -> ToolError {
    use StandardMessagingErrorCode::*;
    let rpc = match error {
        InvocationError::Rpc(rpc) => rpc,
        // A request already written to the wire can surface as `Dropped`, and
        // `Io` can tear down a connection after the server processed it. On a
        // WRITE that is an unknown outcome: not "not executed", and — the
        // distinction §6.2 exists to protect — not `sent_unverified` either,
        // which asserts delivery.
        InvocationError::Dropped | InvocationError::Io(_) => {
            return failed_because(
                VendorError,
                "the telegram connection dropped before the outcome was known".to_string(),
            );
        }
        _ => return failed(VendorError),
    };
    match rpc.name.as_str() {
        "FLOOD_WAIT" | "FLOOD_PREMIUM_WAIT" | "SLOWMODE_WAIT" | "FLOOD_TEST_PHONE_WAIT" => {
            match rpc.value {
                // Prose is the only carrier available: `safe_summary` is fixed
                // host-authored text and may not interpolate a vendor value,
                // and nothing plumbs a tool-dispatch retry-after into the
                // structured `ToolRecoveryObservation` slot today. Do not read
                // this as a machine-readable back-off.
                Some(seconds) => failed_because(
                    RateLimited,
                    format!("telegram asked us to wait {seconds} seconds before retrying"),
                ),
                None => failed(RateLimited),
            }
        }
        "CHAT_WRITE_FORBIDDEN"
        | "CHAT_SEND_PLAIN_FORBIDDEN"
        | "CHAT_ADMIN_REQUIRED"
        | "MESSAGE_DELETE_FORBIDDEN"
        | "REACTION_INVALID"
        | "CHAT_RESTRICTED" => failed(PermissionDenied),
        "USER_IS_BLOCKED"
        | "YOU_BLOCKED_USER"
        | "USER_PRIVACY_RESTRICTED"
        | "USER_NOT_MUTUAL_CONTACT"
        | "PEER_FLOOD" => failed(CannotMessageUser),
        "MESSAGE_TOO_LONG" => failed(MessageTooLong),
        "MESSAGE_ID_INVALID" | "MESSAGE_NOT_MODIFIED" | "MESSAGE_EMPTY" => failed(UnknownMessage),
        "MESSAGE_EDIT_TIME_EXPIRED" | "MESSAGE_AUTHOR_REQUIRED" => failed(EditNotAllowed),
        "CHANNEL_PRIVATE" | "CHANNEL_INVALID" | "CHAT_ID_INVALID" => failed(UnknownConversation),
        // A migrated basic group's old ref, a peer the session never cached,
        // and a stale user ref all arrive as the same error; the family
        // decides which noun the caller was actually holding.
        "PEER_ID_INVALID" | "PEER_ID_NOT_SUPPORTED" => match family {
            OpFamily::People => failed(UnknownUser),
            OpFamily::Read | OpFamily::Write => failed(UnknownConversation),
        },
        "USER_ID_INVALID" | "USERNAME_NOT_OCCUPIED" | "USERNAME_INVALID" => failed(UnknownUser),
        "AUTH_KEY_UNREGISTERED"
        | "AUTH_KEY_INVALID"
        | "SESSION_REVOKED"
        | "SESSION_EXPIRED"
        | "USER_DEACTIVATED_BAN" => auth_required(),
        // Terminal at the account level, but still a plain vendor failure for
        // this call: re-linking cannot succeed, so it must not park on the
        // re-auth gate.
        "USER_DEACTIVATED" => failed(VendorError),
        "MEDIA_INVALID" | "MEDIA_EMPTY" | "ENTITY_BOUNDS_INVALID" => failed(UnsupportedContent),
        _ => failed(VendorError),
    }
}

/// Builds the `send_message` output for a completed send.
///
/// **`id == 0` means Telegram accepted the send but grammers could not
/// correlate it to a message identity.** The message reached the human. It is
/// therefore `Completed` with `sent_unverified: true` — never a failure (a
/// failure is what a model retries, and the retry double-sends), and never a
/// fabricated `message_ref` (a fake ref poisons every later edit/delete and
/// makes the evidence gate unfalsifiable).
pub(crate) fn send_result(
    conversation: &ConversationRef,
    message_id: i32,
    thread: Option<&str>,
    reply_to: Option<&Value>,
) -> Value {
    let mut value = if message_id == 0 {
        json!({ "sent_unverified": true })
    } else {
        json!({ "message_ref": message_ref(conversation, message_id) })
    };
    if let Some(thread) = thread.filter(|thread| !thread.is_empty()) {
        value["thread"] = json!(thread);
    }
    if let Some(reply_to) = reply_to {
        value["reply_to"] = reply_to.clone();
    }
    value
}

/// Builds the output for a write whose only evidence is the ref the caller
/// already held — `edit_message` and `remove_reaction`.
///
/// Telegram's edit and reaction-clear return no new identity, so re-stating the
/// caller's ref is the honest maximum: it says *which* message the call acted
/// on without claiming a fact the vendor did not supply.
pub(crate) fn ref_only_result(conversation: &ConversationRef, message_id: i32) -> Value {
    json!({ "message_ref": message_ref(conversation, message_id) })
}

/// Builds the `delete_message` output. `deleted` is `const: true` in the
/// canonical schema — a delete that did not happen is an error at the call
/// site, never `deleted: false`.
pub(crate) fn delete_result(conversation: &ConversationRef, message_id: i32) -> Value {
    json!({
        "deleted": true,
        "message_ref": message_ref(conversation, message_id),
    })
}

/// Builds the `add_reaction` output. The echoed `emoji` is the model's own
/// input, not vendor text, and `add_reaction` is the only reaction op that may
/// echo one: `remove_reaction` clears every reaction this account left, so
/// naming one would report a precision the call does not have (§6.3).
pub(crate) fn add_reaction_result(
    conversation: &ConversationRef,
    message_id: i32,
    emoji: &str,
) -> Value {
    json!({
        "message_ref": message_ref(conversation, message_id),
        "emoji": emoji,
    })
}

/// Builds the `open_dm` output: a peer-ref re-encode, with no vendor call and
/// therefore no evidence beyond the ref itself.
pub(crate) fn open_dm_result(peer_ref: PeerRef) -> Value {
    json!({ "conversation": ConversationRef::from_peer_ref(peer_ref).encode() })
}

#[cfg(test)]
mod tests;
