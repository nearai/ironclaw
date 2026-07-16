//! Channel-agnostic incoming attachment types.
//!
//! `IncomingAttachment` carries a single file/media item attached to a
//! message received from any channel (Telegram, web, REPL, WASM, …). It is
//! pure data with no transport-trait coupling; the channel layer wraps it
//! into an `IncomingMessage`, while `ironclaw_llm::transcription` operates
//! directly on `&mut [IncomingAttachment]` to fill `extracted_text` for
//! audio inputs.

/// Normalize a MIME type to its canonical comparison form: drop any
/// `; parameter` suffix, trim surrounding whitespace, and lowercase.
///
/// MIME types are case-insensitive (RFC 2045 §5.1), so this is the single
/// normalizer every MIME comparison in the workspace routes through — the
/// attachment-format registry, kind inference, and audio transcription all call
/// it instead of re-deriving `split(';').next().trim()` (and disagreeing on
/// case) locally.
pub fn normalize_mime_type(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or(mime)
        .trim()
        .to_ascii_lowercase()
}

/// Kind of attachment carried on an incoming message.
///
/// Serializes as a wire-stable snake_case string (`"audio"`, `"image"`,
/// `"document"`) so it can be persisted in transcript attachment references
/// and other durable contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    /// Audio content (voice notes, audio files).
    Audio,
    /// Image content (photos, screenshots).
    Image,
    /// Document content (PDFs, files).
    Document,
}

impl AttachmentKind {
    /// Infer attachment kind from a MIME type string.
    pub fn from_mime_type(mime: &str) -> Self {
        let base = normalize_mime_type(mime);
        if base.starts_with("audio/") {
            Self::Audio
        } else if base.starts_with("image/") {
            Self::Image
        } else {
            Self::Document
        }
    }
}

/// A file or media attachment on an incoming message.
///
/// See [`AttachmentRef`] for the durable, byte-free projection persisted on the
/// transcript once the bytes have been landed in host-side storage.
#[derive(Debug, Clone)]
pub struct IncomingAttachment {
    /// Unique identifier within the channel (e.g., Telegram file_id).
    pub id: String,
    /// What kind of content this is.
    pub kind: AttachmentKind,
    /// MIME type (e.g., "image/jpeg", "audio/ogg", "application/pdf").
    pub mime_type: String,
    /// Original filename, if known.
    pub filename: Option<String>,
    /// File size in bytes, if known.
    pub size_bytes: Option<u64>,
    /// URL to download the file from the channel's API.
    pub source_url: Option<String>,
    /// Opaque key for host-side storage (e.g., after download/caching).
    pub storage_key: Option<String>,
    /// Relative path to a project-local copy saved on disk, if persisted.
    pub local_path: Option<String>,
    /// Extracted text content (e.g., OCR result, PDF text, audio transcript).
    pub extracted_text: Option<String>,
    /// Raw file bytes (for small files downloaded by the channel).
    pub data: Vec<u8>,
    /// Duration in seconds (for audio/video).
    pub duration_secs: Option<u32>,
}

/// A reference to a single attachment carried alongside a durable transcript
/// message.
///
/// This is the **durable, byte-free projection** of an [`IncomingAttachment`]:
/// it is a *reference*, never the bytes. A durable transcript must not hold raw
/// runtime payloads, host paths, or secrets, so an `AttachmentRef` deliberately
/// drops `source_url` / `local_path` / `data` and carries only metadata plus an
/// opaque `storage_key` (a rendered scoped path into host-side storage, not a
/// raw host path) and the extracted/transcribed text once an extractor has run.
/// The bytes live behind the filesystem authority that owns `storage_key`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentRef {
    /// Stable identifier for this attachment within its message. An opaque,
    /// channel-provided token with no format contract (unique only within its
    /// message, which `validate_attachment_refs` enforces), so it stays a
    /// boundary `String` rather than a validated newtype.
    pub id: String,
    /// Image / Audio / Document.
    pub kind: AttachmentKind,
    /// MIME type as received at the ingress boundary, stored verbatim. `kind`
    /// and the fallback extension are derived from it through the attachment
    /// format registry, which is a *recognizer, not an allowlist* — an unknown
    /// but well-formed MIME type is accepted, not rejected — so this is
    /// intentionally not gated to a registered set.
    pub mime_type: String,
    /// Original filename, when the source provided one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// File size in bytes, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// Opaque storage reference for the bytes (a rendered scoped path into
    /// host-side storage, never a raw host path). `None` until the attachment
    /// has been landed in storage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_key: Option<String>,
    /// Extracted document text or audio transcript, once an extractor has run.
    /// Sanitized external data; never raw bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_mime_type_strips_params_trims_and_lowercases() {
        // No-semicolon passthrough.
        assert_eq!(normalize_mime_type("text/plain"), "text/plain");
        // Parameter strip + case fold.
        assert_eq!(
            normalize_mime_type("TEXT/PLAIN; charset=UTF-8"),
            "text/plain"
        );
        // Surrounding whitespace and embedded space before the `;` (RFC-legal).
        assert_eq!(normalize_mime_type("  Image/PNG  "), "image/png");
        assert_eq!(
            normalize_mime_type("text/plain ; charset=utf-8"),
            "text/plain"
        );
        // Multiple parameters.
        assert_eq!(normalize_mime_type("a/b; x=1; y=2"), "a/b");
        // Degenerate inputs collapse predictably.
        assert_eq!(normalize_mime_type(""), "");
        assert_eq!(normalize_mime_type("; charset=utf-8"), "");
    }

    #[test]
    fn from_mime_type_normalizes_case_and_params() {
        // Mixed/upper case must still classify (regression: before
        // normalization, `"Image/JPEG".starts_with("image/")` was false).
        assert_eq!(
            AttachmentKind::from_mime_type("IMAGE/PNG"),
            AttachmentKind::Image
        );
        assert_eq!(
            AttachmentKind::from_mime_type("Audio/Ogg; codecs=opus"),
            AttachmentKind::Audio
        );
        assert_eq!(
            AttachmentKind::from_mime_type("APPLICATION/PDF"),
            AttachmentKind::Document
        );
    }
}
