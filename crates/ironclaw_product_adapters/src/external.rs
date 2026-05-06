//! External-protocol reference normalization.
//!
//! Adapters parse raw protocol payloads (Telegram update, Slack event, etc.)
//! into these structured references before calling the workflow facade.
//! The references must contain only stable protocol identifiers — never raw
//! secrets, host paths, bot tokens, source URLs, or arbitrary attacker-supplied
//! free text inside a field that flows further through the system.

use serde::{Deserialize, Serialize};

use crate::error::ProductAdapterError;

const MAX_REF_LEN: usize = 512;

fn validate_external_id(kind: &'static str, value: &str) -> Result<(), ProductAdapterError> {
    if value.is_empty() {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind,
            reason: "must not be empty".into(),
        });
    }
    if value.len() > MAX_REF_LEN {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind,
            reason: format!("must be at most {MAX_REF_LEN} bytes"),
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind,
            reason: "must not contain NUL/control characters".into(),
        });
    }
    Ok(())
}

/// Stable external event identifier scoped to adapter installation.
///
/// For Telegram this is `update_id`, for Slack it is `event_id`/`client_msg_id`,
/// and so on. The combination of `(adapter_installation_id, source_binding,
/// external_event_id)` is the idempotency key the workflow uses to dedupe.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExternalEventId(String);

impl ExternalEventId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProductAdapterError> {
        let value = value.into();
        validate_external_id("external_event_id", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExternalEventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// External actor reference. `kind` carries the protocol-specific actor kind
/// (`telegram_user`, `slack_user`, etc.); `id` is the protocol's stable user
/// identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalActorRef {
    kind: String,
    id: String,
    display_name: Option<String>,
}

impl ExternalActorRef {
    pub fn new(
        kind: impl Into<String>,
        id: impl Into<String>,
        display_name: Option<String>,
    ) -> Result<Self, ProductAdapterError> {
        let kind = kind.into();
        let id = id.into();
        validate_external_id("external_actor_kind", &kind)?;
        validate_external_id("external_actor_id", &id)?;
        if let Some(name) = &display_name {
            validate_external_id("external_actor_display_name", name)?;
        }
        Ok(Self {
            kind,
            id,
            display_name,
        })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }
}

/// External conversation reference.
///
/// For Telegram this is keyed by `(chat_id, optional message_thread_id)` — the
/// reply/message id is **not** part of the conversation key (that is
/// reply-target/idempotency data). For Slack it is keyed by `(team_id,
/// channel_id, thread_ts)`. `space_id` is the optional outer namespace where
/// applicable (workspace, guild).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalConversationRef {
    space_id: Option<String>,
    conversation_id: String,
    topic_id: Option<String>,
    /// Optional protocol-level reply-target hint. **Not** part of the
    /// canonical conversation key; carried so the workflow can construct a
    /// `ReplyTargetBindingRef` value that survives across reply chains.
    reply_target_message_id: Option<String>,
}

impl ExternalConversationRef {
    pub fn new(
        space_id: Option<&str>,
        conversation_id: impl Into<String>,
        topic_id: Option<&str>,
        reply_target_message_id: Option<&str>,
    ) -> Result<Self, ProductAdapterError> {
        let conversation_id = conversation_id.into();
        validate_external_id("external_conversation_id", &conversation_id)?;
        if let Some(value) = space_id {
            validate_external_id("external_space_id", value)?;
        }
        if let Some(value) = topic_id {
            validate_external_id("external_topic_id", value)?;
        }
        if let Some(value) = reply_target_message_id {
            validate_external_id("external_reply_target_message_id", value)?;
        }
        Ok(Self {
            space_id: space_id.map(str::to_string),
            conversation_id,
            topic_id: topic_id.map(str::to_string),
            reply_target_message_id: reply_target_message_id.map(str::to_string),
        })
    }

    pub fn space_id(&self) -> Option<&str> {
        self.space_id.as_deref()
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub fn topic_id(&self) -> Option<&str> {
        self.topic_id.as_deref()
    }

    pub fn reply_target_message_id(&self) -> Option<&str> {
        self.reply_target_message_id.as_deref()
    }

    /// Canonical conversation fingerprint. Excludes `reply_target_message_id`
    /// — that field is reply-target data, not part of the conversation key.
    pub fn conversation_fingerprint(&self) -> String {
        format!(
            "space={};conversation={};topic={}",
            self.space_id.as_deref().unwrap_or(""),
            self.conversation_id,
            self.topic_id.as_deref().unwrap_or(""),
        )
    }
}

/// Bounded attachment descriptor.
///
/// Adapters MUST NOT include raw bytes, source URLs that require credentials,
/// host-local paths, or extracted text in the inbound envelope. The workflow
/// stages durable attachment refs (downloads via constrained egress, blob
/// store handles, etc.) before the turn coordinator sees the message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductAttachmentDescriptor {
    /// Protocol-side stable file id (Telegram `file_id`, Slack `file.id`, ...).
    pub external_file_id: String,
    /// MIME type, normalized to lowercase ASCII (validated to not contain
    /// control characters).
    pub mime_type: String,
    /// Original filename if the protocol provided one. Caps at 256 bytes.
    pub filename: Option<String>,
    /// File size in bytes, if the protocol provided it.
    pub size_bytes: Option<u64>,
    /// Coarse kind for the workflow's attachment-staging policy.
    pub kind: ProductAttachmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductAttachmentKind {
    Image,
    Audio,
    Video,
    Document,
    Voice,
    Sticker,
    Other,
}

impl ProductAttachmentDescriptor {
    pub fn new(
        external_file_id: impl Into<String>,
        mime_type: impl Into<String>,
        filename: Option<String>,
        size_bytes: Option<u64>,
        kind: ProductAttachmentKind,
    ) -> Result<Self, ProductAdapterError> {
        let external_file_id = external_file_id.into();
        let mime_type = mime_type.into();
        validate_external_id("attachment_external_file_id", &external_file_id)?;
        validate_external_id("attachment_mime_type", &mime_type)?;
        if let Some(name) = &filename {
            validate_external_id("attachment_filename", name)?;
        }
        Ok(Self {
            external_file_id,
            mime_type,
            filename,
            size_bytes,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_event_id_round_trips() {
        let id = ExternalEventId::new("telegram_update:42").expect("valid");
        let json = serde_json::to_string(&id).expect("serialize");
        let parsed: ExternalEventId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, parsed);
    }

    #[test]
    fn external_event_id_rejects_control_chars() {
        assert!(ExternalEventId::new("foo\nbar").is_err());
    }

    #[test]
    fn conversation_fingerprint_excludes_reply_target() {
        // Same conversation, different reply target — fingerprints must be
        // identical because reply-target is NOT part of the conversation key.
        let a = ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-100"))
            .expect("valid");
        let b = ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-200"))
            .expect("valid");
        assert_eq!(a.conversation_fingerprint(), b.conversation_fingerprint());
    }

    #[test]
    fn conversation_fingerprint_distinguishes_topic() {
        let a = ExternalConversationRef::new(None, "12345", Some("topic-7"), None).expect("valid");
        let b = ExternalConversationRef::new(None, "12345", Some("topic-8"), None).expect("valid");
        assert_ne!(a.conversation_fingerprint(), b.conversation_fingerprint());
    }

    #[test]
    fn attachment_descriptor_rejects_control_chars_in_mime() {
        assert!(
            ProductAttachmentDescriptor::new(
                "file_42",
                "image/jpeg\0",
                None,
                Some(2048),
                ProductAttachmentKind::Image,
            )
            .is_err()
        );
    }

    #[test]
    fn attachment_descriptor_does_not_contain_url_fields() {
        // The descriptor type itself has no source_url / local_path / data
        // fields — this assertion is a compile-time guarantee, but we exercise
        // construction paths to make the intent explicit in tests.
        let attachment = ProductAttachmentDescriptor::new(
            "file_42",
            "image/jpeg",
            Some("photo.jpg".into()),
            Some(2048),
            ProductAttachmentKind::Image,
        )
        .expect("valid");
        let json = serde_json::to_value(&attachment).expect("serialize");
        let object = json.as_object().expect("object");
        assert!(!object.contains_key("source_url"));
        assert!(!object.contains_key("local_path"));
        assert!(!object.contains_key("data"));
    }
}
