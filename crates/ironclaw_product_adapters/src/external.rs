//! External-protocol reference normalization.

use std::hash::{Hash, Hasher};

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::ProductAdapterError;

const MAX_REF_LEN: usize = 512;
const MAX_FILENAME_LEN: usize = 256;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
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

impl<'de> Deserialize<'de> for ExternalEventId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for ExternalEventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// External actor reference. Equality/hash use only stable identity
/// (`kind`, `id`); `display_name` is presentation metadata.
#[derive(Debug, Clone, Serialize)]
pub struct ExternalActorRef {
    kind: String,
    id: String,
    display_name: Option<String>,
}

impl ExternalActorRef {
    pub fn new(
        kind: impl Into<String>,
        id: impl Into<String>,
        display_name: Option<impl Into<String>>,
    ) -> Result<Self, ProductAdapterError> {
        let kind = kind.into();
        let id = id.into();
        let display_name = display_name.map(Into::into);
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

#[derive(Deserialize)]
struct ExternalActorRefWire {
    kind: String,
    id: String,
    display_name: Option<String>,
}

impl<'de> Deserialize<'de> for ExternalActorRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ExternalActorRefWire::deserialize(deserializer)?;
        Self::new(wire.kind, wire.id, wire.display_name).map_err(serde::de::Error::custom)
    }
}

impl PartialEq for ExternalActorRef {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.id == other.id
    }
}

impl Eq for ExternalActorRef {}

impl Hash for ExternalActorRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.id.hash(state);
    }
}

/// External conversation reference. Equality/hash use only stable conversation
/// identity; `reply_target_message_id` is reply-target metadata.
#[derive(Debug, Clone, Serialize)]
pub struct ExternalConversationRef {
    space_id: Option<String>,
    conversation_id: String,
    topic_id: Option<String>,
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

    /// Canonical conversation fingerprint. Length-prefixed segments prevent
    /// delimiter collisions and the reply-target hint is deliberately excluded.
    pub fn conversation_fingerprint(&self) -> String {
        fn seg(name: &str, value: Option<&str>) -> String {
            let value = value.unwrap_or("");
            format!("{name}:{}:{value};", value.len())
        }
        format!(
            "{}{}{}",
            seg("space", self.space_id.as_deref()),
            seg("conversation", Some(&self.conversation_id)),
            seg("topic", self.topic_id.as_deref()),
        )
    }
}

#[derive(Deserialize)]
struct ExternalConversationRefWire {
    space_id: Option<String>,
    conversation_id: String,
    topic_id: Option<String>,
    reply_target_message_id: Option<String>,
}

impl<'de> Deserialize<'de> for ExternalConversationRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ExternalConversationRefWire::deserialize(deserializer)?;
        Self::new(
            wire.space_id.as_deref(),
            wire.conversation_id,
            wire.topic_id.as_deref(),
            wire.reply_target_message_id.as_deref(),
        )
        .map_err(serde::de::Error::custom)
    }
}

impl PartialEq for ExternalConversationRef {
    fn eq(&self, other: &Self) -> bool {
        self.space_id == other.space_id
            && self.conversation_id == other.conversation_id
            && self.topic_id == other.topic_id
    }
}

impl Eq for ExternalConversationRef {}

impl Hash for ExternalConversationRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.space_id.hash(state);
        self.conversation_id.hash(state);
        self.topic_id.hash(state);
    }
}

/// Bounded attachment descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProductAttachmentDescriptor {
    pub external_file_id: String,
    pub mime_type: String,
    pub filename: Option<String>,
    pub size_bytes: Option<u64>,
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
        validate_mime_type(&mime_type)?;
        if let Some(name) = &filename {
            validate_attachment_filename(name)?;
        }
        validate_attachment_kind(&mime_type, kind)?;
        Ok(Self {
            external_file_id,
            mime_type,
            filename,
            size_bytes,
            kind,
        })
    }
}

#[derive(Deserialize)]
struct ProductAttachmentDescriptorWire {
    external_file_id: String,
    mime_type: String,
    filename: Option<String>,
    size_bytes: Option<u64>,
    kind: ProductAttachmentKind,
}

impl<'de> Deserialize<'de> for ProductAttachmentDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProductAttachmentDescriptorWire::deserialize(deserializer)?;
        Self::new(
            wire.external_file_id,
            wire.mime_type,
            wire.filename,
            wire.size_bytes,
            wire.kind,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn validate_mime_type(value: &str) -> Result<(), ProductAdapterError> {
    validate_external_id("attachment_mime_type", value)?;
    if !value.is_ascii() || value.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "attachment_mime_type",
            reason: "must be normalized lowercase ASCII".into(),
        });
    }
    if !value.contains('/') || value.starts_with('/') || value.ends_with('/') {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "attachment_mime_type",
            reason: "must be a type/subtype MIME value".into(),
        });
    }
    Ok(())
}

fn validate_attachment_filename(value: &str) -> Result<(), ProductAdapterError> {
    validate_external_id("attachment_filename", value)?;
    if value.len() > MAX_FILENAME_LEN {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "attachment_filename",
            reason: format!("must be at most {MAX_FILENAME_LEN} bytes"),
        });
    }
    Ok(())
}

fn validate_attachment_kind(
    mime_type: &str,
    kind: ProductAttachmentKind,
) -> Result<(), ProductAdapterError> {
    let base = mime_type.split('/').next().unwrap_or_default();
    let expected = match base {
        "image" => Some(ProductAttachmentKind::Image),
        "audio" => Some(ProductAttachmentKind::Audio),
        "video" => Some(ProductAttachmentKind::Video),
        _ => None,
    };
    if let Some(expected) = expected
        && kind != expected
        && kind != ProductAttachmentKind::Other
    {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "attachment_kind",
            reason: "must match normalized MIME base type".into(),
        });
    }
    Ok(())
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
        assert!(serde_json::from_str::<ExternalEventId>("\"foo\\nbar\"").is_err());
    }

    #[test]
    fn actor_equality_ignores_display_name() {
        let a = ExternalActorRef::new("telegram_user", "777", Some("Alice")).expect("valid");
        let b = ExternalActorRef::new("telegram_user", "777", Some("Alice Cooper")).expect("valid");
        assert_eq!(a, b);
    }

    #[test]
    fn conversation_fingerprint_excludes_reply_target() {
        let a = ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-100"))
            .expect("valid");
        let b = ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-200"))
            .expect("valid");
        assert_eq!(a, b);
        assert_eq!(a.conversation_fingerprint(), b.conversation_fingerprint());
    }

    #[test]
    fn conversation_fingerprint_distinguishes_delimiter_ambiguous_parts() {
        let a = ExternalConversationRef::new(Some("a;conversation=b"), "c", Some("d"), None)
            .expect("valid");
        let b =
            ExternalConversationRef::new(Some("a"), "b;topic=c", Some("d"), None).expect("valid");
        assert_ne!(a.conversation_fingerprint(), b.conversation_fingerprint());
    }

    #[test]
    fn attachment_descriptor_rejects_invalid_metadata() {
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
        assert!(
            ProductAttachmentDescriptor::new(
                "file_42",
                "Image/JPEG",
                None,
                Some(2048),
                ProductAttachmentKind::Image,
            )
            .is_err()
        );
        assert!(
            ProductAttachmentDescriptor::new(
                "file_42",
                "image/jpeg",
                Some("a".repeat(MAX_FILENAME_LEN + 1)),
                Some(2048),
                ProductAttachmentKind::Image,
            )
            .is_err()
        );
    }

    #[test]
    fn attachment_descriptor_does_not_contain_url_fields() {
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
