//! Single source of truth for attachment format support.
//!
//! Historically the "is this MIME allowed", "MIME → file extension",
//! "which attachment kind" and "which extractor handles it" questions were
//! answered by four independent hardcoded lists scattered across the channel
//! layer, the document-extraction layer, the transcription layer, and the web
//! frontend. They drifted: a format added to one list but not another is the
//! root cause of bugs like "CSV uploaded as text instead of a document" and
//! "image-only web attachments".
//!
//! This module replaces those lists with one table, [`FORMATS`], exposed
//! through the functions below. Adding support for a new format is a single
//! new [`AttachmentFormat`] entry, not four edits.
//!
//! Scope note: the registry only *names* which extractor a format maps to (via
//! [`ExtractorId`]); it does not run extraction. The extractors themselves live
//! in the document-extraction and transcription layers. Keeping the dispatch
//! table here lets those layers (and a future crate-level extractor) select an
//! extractor from one authority instead of re-deriving it from a private match.

use crate::{AttachmentKind, normalize_mime_type};

/// Names the strategy that turns an attachment's bytes into `extracted_text`.
///
/// The registry records which extractor a format maps to; it deliberately does
/// not run it. `None` means there is no text extractor for the format: images
/// go to the vision model as a multimodal part (distinguished by
/// [`AttachmentFormat::kind`] being [`AttachmentKind::Image`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractorId {
    /// No text extraction. Images are sent to the vision model as a multimodal
    /// image part rather than extracted to text.
    None,
    /// PDF text extraction.
    Pdf,
    /// OOXML wordprocessing document (`.docx`).
    Docx,
    /// OOXML presentation (`.pptx`).
    Pptx,
    /// OOXML spreadsheet (`.xlsx`).
    Xlsx,
    /// Legacy OLE2 office binaries (`.doc` / `.ppt` / `.xls`) — best-effort
    /// printable-string scrape.
    LegacyOffice,
    /// UTF-8 text passthrough (plain text, CSV, Markdown, JSON, XML, …).
    Utf8Text,
    /// Rich Text Format.
    Rtf,
    /// Provider-backed audio transcription.
    AudioTranscription,
}

/// One supported attachment format: the authoritative mapping from a MIME type
/// to its canonical extension, attachment kind, and extractor.
///
/// `mime` is the canonical, lowercase MIME type and acts as the primary key.
/// `mime_aliases` are additional lowercase MIME spellings that resolve to the
/// same format (e.g. `image/jpg` for `image/jpeg`, `audio/x-wav` for
/// `audio/wav`). Lookups normalize the input (strip parameters, trim, lowercase)
/// before matching against `mime` or any alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentFormat {
    /// Canonical, lowercase MIME type (the primary key).
    pub mime: &'static str,
    /// Additional lowercase MIME spellings that resolve to this format.
    pub mime_aliases: &'static [&'static str],
    /// Canonical file extension, without the leading dot.
    pub canonical_ext: &'static str,
    /// Attachment kind (Image / Audio / Document).
    pub kind: AttachmentKind,
    /// Which extractor produces `extracted_text` for this format.
    pub extractor: ExtractorId,
}

/// The authoritative table of supported attachment formats.
///
/// Adding support for a new format is one new entry here. Two MIME types that
/// are genuinely the same format share an entry via `mime_aliases`; two
/// formats with different canonical extensions or extractors get separate
/// entries even if a downstream layer happens to treat them alike.
///
/// Deliberate exclusions:
/// - `image/svg+xml` — SVG is an active-content vector and is rejected on the
///   existing upload paths; it is not a supported attachment format.
/// - `application/octet-stream` — a generic catch-all, not a recognized
///   format. Unknown binaries are not advertised in the picker and resolve to
///   `None`/`Document` via the prefix fallback rather than a registry entry.
const FORMATS: &[AttachmentFormat] = &[
    // ── Images (no text extractor — sent to the vision model) ──────────────
    AttachmentFormat {
        mime: "image/png",
        mime_aliases: &[],
        canonical_ext: "png",
        kind: AttachmentKind::Image,
        extractor: ExtractorId::None,
    },
    AttachmentFormat {
        mime: "image/jpeg",
        mime_aliases: &["image/jpg"],
        canonical_ext: "jpg",
        kind: AttachmentKind::Image,
        extractor: ExtractorId::None,
    },
    AttachmentFormat {
        mime: "image/gif",
        mime_aliases: &[],
        canonical_ext: "gif",
        kind: AttachmentKind::Image,
        extractor: ExtractorId::None,
    },
    AttachmentFormat {
        mime: "image/webp",
        mime_aliases: &[],
        canonical_ext: "webp",
        kind: AttachmentKind::Image,
        extractor: ExtractorId::None,
    },
    // ── Documents ──────────────────────────────────────────────────────────
    AttachmentFormat {
        mime: "application/pdf",
        mime_aliases: &[],
        canonical_ext: "pdf",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Pdf,
    },
    AttachmentFormat {
        mime: "text/plain",
        mime_aliases: &[],
        canonical_ext: "txt",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/markdown",
        mime_aliases: &[],
        canonical_ext: "md",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/csv",
        mime_aliases: &[],
        canonical_ext: "csv",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "application/json",
        mime_aliases: &[],
        canonical_ext: "json",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "application/xml",
        mime_aliases: &["text/xml"],
        canonical_ext: "xml",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "application/rtf",
        mime_aliases: &["text/rtf"],
        canonical_ext: "rtf",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Rtf,
    },
    // ── Plain-text & source-code family (UTF-8 passthrough) ───────────────────
    // These mirror the text/code arms of the document extractor: each is a
    // distinct file type with its own extension, so each gets its own entry.
    AttachmentFormat {
        mime: "text/tab-separated-values",
        mime_aliases: &[],
        canonical_ext: "tsv",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/html",
        mime_aliases: &[],
        canonical_ext: "html",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/javascript",
        mime_aliases: &[],
        canonical_ext: "js",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/css",
        mime_aliases: &[],
        canonical_ext: "css",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-python",
        mime_aliases: &[],
        canonical_ext: "py",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-java",
        mime_aliases: &[],
        canonical_ext: "java",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-c",
        mime_aliases: &[],
        canonical_ext: "c",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-c++",
        mime_aliases: &[],
        canonical_ext: "cpp",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-rust",
        mime_aliases: &[],
        canonical_ext: "rs",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-go",
        mime_aliases: &[],
        canonical_ext: "go",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-ruby",
        mime_aliases: &[],
        canonical_ext: "rb",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-shellscript",
        mime_aliases: &["application/x-sh"],
        canonical_ext: "sh",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-toml",
        mime_aliases: &["application/toml"],
        canonical_ext: "toml",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-yaml",
        mime_aliases: &["application/yaml", "application/x-yaml"],
        canonical_ext: "yaml",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "text/x-log",
        mime_aliases: &[],
        canonical_ext: "log",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Utf8Text,
    },
    AttachmentFormat {
        mime: "application/msword",
        mime_aliases: &[],
        canonical_ext: "doc",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::LegacyOffice,
    },
    AttachmentFormat {
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        mime_aliases: &[],
        canonical_ext: "docx",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Docx,
    },
    AttachmentFormat {
        mime: "application/vnd.ms-excel",
        mime_aliases: &[],
        canonical_ext: "xls",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::LegacyOffice,
    },
    AttachmentFormat {
        mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        mime_aliases: &[],
        canonical_ext: "xlsx",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Xlsx,
    },
    AttachmentFormat {
        mime: "application/vnd.ms-powerpoint",
        mime_aliases: &[],
        canonical_ext: "ppt",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::LegacyOffice,
    },
    AttachmentFormat {
        mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        mime_aliases: &[],
        canonical_ext: "pptx",
        kind: AttachmentKind::Document,
        extractor: ExtractorId::Pptx,
    },
    // ── Audio (transcribed to text) ──────────────────────────────────────────
    AttachmentFormat {
        mime: "audio/mpeg",
        mime_aliases: &["audio/mp3"],
        canonical_ext: "mp3",
        kind: AttachmentKind::Audio,
        extractor: ExtractorId::AudioTranscription,
    },
    AttachmentFormat {
        mime: "audio/ogg",
        mime_aliases: &["audio/opus"],
        canonical_ext: "ogg",
        kind: AttachmentKind::Audio,
        extractor: ExtractorId::AudioTranscription,
    },
    AttachmentFormat {
        mime: "audio/wav",
        mime_aliases: &["audio/x-wav", "audio/wave"],
        canonical_ext: "wav",
        kind: AttachmentKind::Audio,
        extractor: ExtractorId::AudioTranscription,
    },
    AttachmentFormat {
        mime: "audio/mp4",
        mime_aliases: &[],
        canonical_ext: "mp4",
        kind: AttachmentKind::Audio,
        extractor: ExtractorId::AudioTranscription,
    },
    AttachmentFormat {
        mime: "audio/x-m4a",
        mime_aliases: &["audio/m4a"],
        canonical_ext: "m4a",
        kind: AttachmentKind::Audio,
        extractor: ExtractorId::AudioTranscription,
    },
    AttachmentFormat {
        mime: "audio/aac",
        mime_aliases: &[],
        canonical_ext: "aac",
        kind: AttachmentKind::Audio,
        extractor: ExtractorId::AudioTranscription,
    },
    AttachmentFormat {
        mime: "audio/flac",
        mime_aliases: &["audio/x-flac"],
        canonical_ext: "flac",
        kind: AttachmentKind::Audio,
        extractor: ExtractorId::AudioTranscription,
    },
    AttachmentFormat {
        mime: "audio/webm",
        mime_aliases: &[],
        canonical_ext: "webm",
        kind: AttachmentKind::Audio,
        extractor: ExtractorId::AudioTranscription,
    },
];

/// Look up the format for a MIME type, matching the canonical spelling or any
/// alias. The input is normalized (parameters stripped, trimmed, lowercased)
/// via [`normalize_mime_type`] before matching. Returns `None` for unsupported
/// MIME types.
pub fn lookup(mime: &str) -> Option<&'static AttachmentFormat> {
    let normalized = normalize_mime_type(mime);
    FORMATS.iter().find(|format| {
        format.mime == normalized || format.mime_aliases.contains(&normalized.as_str())
    })
}

/// Whether the registry recognizes (and therefore allows) a MIME type.
pub fn is_supported_mime(mime: &str) -> bool {
    lookup(mime).is_some()
}

/// The canonical file extension (without a leading dot) for a MIME type, or
/// `None` if the format is not supported.
pub fn canonical_extension(mime: &str) -> Option<&'static str> {
    lookup(mime).map(|format| format.canonical_ext)
}

/// The attachment kind for a MIME type.
///
/// The registry is authoritative for supported formats. For unsupported MIME
/// types this falls back to the prefix-based [`AttachmentKind::from_mime_type`]
/// so callers always get a kind (e.g. an unknown `image/*` still classifies as
/// [`AttachmentKind::Image`]).
pub fn kind_for_mime(mime: &str) -> AttachmentKind {
    lookup(mime)
        .map(|format| format.kind.clone())
        .unwrap_or_else(|| AttachmentKind::from_mime_type(mime))
}

/// The extractor that handles a MIME type, or `None` if the format is not
/// supported. Note that a supported format may itself map to
/// [`ExtractorId::None`] (images have no text extractor); use [`is_supported_mime`]
/// to distinguish "unsupported" from "supported but no text extractor".
pub fn extractor_for_mime(mime: &str) -> Option<ExtractorId> {
    lookup(mime).map(|format| format.extractor)
}

/// All supported formats, in table order.
pub fn all_formats() -> &'static [AttachmentFormat] {
    FORMATS
}

/// Build the token list for an HTML file-input `accept` attribute from the
/// registry: one explicit `.ext` token per registered format, in table order.
/// This is the single source the frontend `accept=` list should be generated
/// from or asserted against.
///
/// Every kind — image, document, and audio — is advertised the same way, by its
/// canonical extension. We deliberately do *not* emit `image/*` / `audio/*`
/// wildcards: a wildcard tells the browser to accept *any* image/audio type,
/// including ones the registry rejects (`image/svg+xml`, `image/bmp`, …), so the
/// picker would offer files that then fail server-side validation — the exact
/// drift this registry exists to remove. Advertising exactly the registry's
/// extensions keeps the picker and [`is_supported_mime`] in lockstep.
pub fn accept_tokens() -> Vec<String> {
    FORMATS
        .iter()
        .map(|format| format!(".{}", format.canonical_ext))
        .collect()
}

/// The comma-joined `accept` attribute value for an HTML file input, generated
/// from the registry. See [`accept_tokens`].
pub fn accept_attribute() -> String {
    accept_tokens().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn lookup_matches_canonical_mime() {
        let pdf = lookup("application/pdf").expect("pdf is supported");
        assert_eq!(pdf.canonical_ext, "pdf");
        assert_eq!(pdf.kind, AttachmentKind::Document);
        assert_eq!(pdf.extractor, ExtractorId::Pdf);
    }

    #[test]
    fn lookup_matches_aliases() {
        assert_eq!(lookup("image/jpg").unwrap().mime, "image/jpeg");
        assert_eq!(lookup("text/xml").unwrap().mime, "application/xml");
        assert_eq!(lookup("text/rtf").unwrap().mime, "application/rtf");
        assert_eq!(lookup("audio/x-wav").unwrap().mime, "audio/wav");
        assert_eq!(lookup("audio/wave").unwrap().mime, "audio/wav");
        assert_eq!(lookup("audio/mp3").unwrap().mime, "audio/mpeg");
        assert_eq!(lookup("audio/m4a").unwrap().mime, "audio/x-m4a");
        assert_eq!(lookup("audio/opus").unwrap().mime, "audio/ogg");
        assert_eq!(lookup("audio/x-flac").unwrap().mime, "audio/flac");
    }

    #[test]
    fn lookup_normalizes_case_and_parameters() {
        let format = lookup("Application/PDF; charset=UTF-8").expect("normalized lookup");
        assert_eq!(format.mime, "application/pdf");
        assert!(is_supported_mime("  IMAGE/PNG  "));
        assert_eq!(lookup("AUDIO/MPEG").unwrap().canonical_ext, "mp3");
    }

    #[test]
    fn unsupported_mimes_resolve_to_none() {
        // SVG is deliberately rejected (active-content vector).
        assert!(lookup("image/svg+xml").is_none());
        assert!(!is_supported_mime("image/svg+xml"));
        // Generic binary is a catch-all, not a registry format.
        assert!(lookup("application/octet-stream").is_none());
        assert!(extractor_for_mime("application/octet-stream").is_none());
        // Genuinely unknown.
        assert!(lookup("application/x-made-up").is_none());
    }

    #[test]
    fn canonical_extension_resolves_via_alias() {
        assert_eq!(canonical_extension("image/jpg"), Some("jpg"));
        assert_eq!(canonical_extension("application/json"), Some("json"));
        assert_eq!(canonical_extension("application/x-made-up"), None);
    }

    #[test]
    fn kind_for_mime_is_authoritative_with_prefix_fallback() {
        // Registry-known.
        assert_eq!(kind_for_mime("application/pdf"), AttachmentKind::Document);
        assert_eq!(kind_for_mime("image/png"), AttachmentKind::Image);
        assert_eq!(kind_for_mime("audio/mpeg"), AttachmentKind::Audio);
        // Unknown but prefix-classifiable falls back.
        assert_eq!(kind_for_mime("image/svg+xml"), AttachmentKind::Image);
        assert_eq!(kind_for_mime("audio/x-exotic"), AttachmentKind::Audio);
        assert_eq!(
            kind_for_mime("application/octet-stream"),
            AttachmentKind::Document
        );
    }

    #[test]
    fn extractor_selection_covers_every_document_and_audio_format() {
        for format in all_formats() {
            match format.kind {
                AttachmentKind::Image => assert_eq!(
                    format.extractor,
                    ExtractorId::None,
                    "image {} should have no text extractor",
                    format.mime
                ),
                AttachmentKind::Audio => assert_eq!(
                    format.extractor,
                    ExtractorId::AudioTranscription,
                    "audio {} should transcribe",
                    format.mime
                ),
                AttachmentKind::Document => assert_ne!(
                    format.extractor,
                    ExtractorId::None,
                    "document {} must have a text extractor",
                    format.mime
                ),
            }
        }
    }

    #[test]
    fn table_has_no_duplicate_mimes_or_extensions() {
        let mut seen_mimes = HashSet::new();
        let mut seen_exts = HashSet::new();
        for format in all_formats() {
            assert!(
                seen_mimes.insert(format.mime),
                "duplicate canonical MIME {}",
                format.mime
            );
            for alias in format.mime_aliases {
                assert!(
                    seen_mimes.insert(*alias),
                    "MIME {alias} appears as canonical and/or alias more than once",
                );
                assert_ne!(*alias, format.mime, "alias equals canonical for {alias}");
            }
            assert!(
                seen_exts.insert(format.canonical_ext),
                "duplicate canonical extension {}",
                format.canonical_ext
            );
            assert!(!format.canonical_ext.is_empty());
            assert_eq!(
                format.mime,
                normalize_mime_type(format.mime),
                "canonical MIME {} is not already normalized",
                format.mime
            );
        }
    }

    #[test]
    fn every_format_is_round_trippable_by_lookup() {
        for format in all_formats() {
            assert_eq!(lookup(format.mime), Some(format));
            for alias in format.mime_aliases {
                assert_eq!(lookup(alias), Some(format), "alias {alias} round-trip");
            }
        }
    }

    #[test]
    fn accept_tokens_are_exactly_the_registry_extensions() {
        let tokens = accept_tokens();

        // Every token is an explicit extension — no `image/*` / `audio/*`
        // wildcards that would advertise formats the registry rejects.
        assert!(
            tokens.iter().all(|t| t.starts_with('.')),
            "accept tokens must be extensions, not wildcards: {tokens:?}"
        );

        // No duplicate tokens.
        let unique: HashSet<&String> = tokens.iter().collect();
        assert_eq!(unique.len(), tokens.len(), "accept tokens must be unique");

        // The advertised set is exactly the canonical extension of every
        // registered format — including images, so the picker and
        // `is_supported_mime` stay in lockstep.
        let token_set: HashSet<String> = tokens.iter().cloned().collect();
        let expected: HashSet<String> = all_formats()
            .iter()
            .map(|f| format!(".{}", f.canonical_ext))
            .collect();
        assert_eq!(token_set, expected);

        // Images are advertised by explicit extension, not a wildcard.
        assert!(tokens.contains(&".png".to_string()));
        assert!(!tokens.contains(&"image/*".to_string()));
    }

    #[test]
    fn accept_attribute_is_comma_joined_tokens() {
        assert_eq!(accept_attribute(), accept_tokens().join(","));
        // Table order: the first registered format is `image/png`.
        assert!(accept_attribute().starts_with(".png,"));
    }

    /// The registry is the source of truth the v1 `src/` call sites will migrate
    /// onto, so it must already recognize every MIME those lists handle today —
    /// otherwise migration silently drops support. This crate can't import
    /// `src/`, so the existing lists are mirrored here as fixtures: if a format
    /// is dropped from the table (or one of the lists grows a format the table
    /// lacks), this fails loudly. Keep these in sync with their sources.
    #[test]
    fn registry_is_a_superset_of_the_document_extractor() {
        // Mirror of the dispatch arms in
        // `src/document_extraction/extractors.rs::extract_text` (everything it
        // maps to a concrete extractor, excluding the filename fallback).
        const EXTRACTOR_MIMES: &[(&str, ExtractorId)] = &[
            ("application/pdf", ExtractorId::Pdf),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                ExtractorId::Docx,
            ),
            (
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                ExtractorId::Pptx,
            ),
            (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                ExtractorId::Xlsx,
            ),
            ("application/msword", ExtractorId::LegacyOffice),
            ("application/vnd.ms-powerpoint", ExtractorId::LegacyOffice),
            ("application/vnd.ms-excel", ExtractorId::LegacyOffice),
            ("text/plain", ExtractorId::Utf8Text),
            ("text/csv", ExtractorId::Utf8Text),
            ("text/tab-separated-values", ExtractorId::Utf8Text),
            ("text/markdown", ExtractorId::Utf8Text),
            ("text/html", ExtractorId::Utf8Text),
            ("text/xml", ExtractorId::Utf8Text),
            ("text/x-python", ExtractorId::Utf8Text),
            ("text/x-java", ExtractorId::Utf8Text),
            ("text/x-c", ExtractorId::Utf8Text),
            ("text/x-c++", ExtractorId::Utf8Text),
            ("text/x-rust", ExtractorId::Utf8Text),
            ("text/x-go", ExtractorId::Utf8Text),
            ("text/x-ruby", ExtractorId::Utf8Text),
            ("text/x-shellscript", ExtractorId::Utf8Text),
            ("text/javascript", ExtractorId::Utf8Text),
            ("text/css", ExtractorId::Utf8Text),
            ("text/x-toml", ExtractorId::Utf8Text),
            ("text/x-yaml", ExtractorId::Utf8Text),
            ("text/x-log", ExtractorId::Utf8Text),
            ("application/json", ExtractorId::Utf8Text),
            ("application/xml", ExtractorId::Utf8Text),
            ("application/x-yaml", ExtractorId::Utf8Text),
            ("application/yaml", ExtractorId::Utf8Text),
            ("application/toml", ExtractorId::Utf8Text),
            ("application/x-sh", ExtractorId::Utf8Text),
            ("application/rtf", ExtractorId::Rtf),
            ("text/rtf", ExtractorId::Rtf),
        ];
        for (mime, expected) in EXTRACTOR_MIMES {
            assert_eq!(
                extractor_for_mime(mime),
                Some(*expected),
                "registry missing or mismaps document MIME {mime}"
            );
        }
    }

    #[test]
    fn registry_is_a_superset_of_the_web_upload_allow_list() {
        // Mirror of `is_allowed_attachment_mime` in
        // `src/channels/web/util.rs` (excluding `application/octet-stream`,
        // which is the deliberate generic-binary exclusion).
        const ALLOWED_MIMES: &[&str] = &[
            "image/png",
            "image/jpeg",
            "image/jpg",
            "image/gif",
            "image/webp",
            "audio/mpeg",
            "audio/ogg",
            "audio/wav",
            "audio/wave",
            "audio/x-wav",
            "audio/mp4",
            "audio/x-m4a",
            "audio/aac",
            "audio/flac",
            "audio/webm",
            "text/plain",
            "text/csv",
            "text/markdown",
            "text/xml",
            "application/pdf",
            "application/json",
            "application/xml",
            "application/rtf",
            "text/rtf",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/msword",
            "application/vnd.ms-powerpoint",
            "application/vnd.ms-excel",
        ];
        for mime in ALLOWED_MIMES {
            assert!(
                is_supported_mime(mime),
                "registry does not recognize allow-listed upload MIME {mime}"
            );
        }
    }

    #[test]
    fn registry_canonical_ext_matches_web_extension_map() {
        // Mirror of `web_attachment_ext` in `src/channels/web/util.rs`, minus
        // `application/octet-stream` (the deliberate generic-binary `bin`
        // exclusion). `canonical_ext` is the registry field meant to replace
        // that map; this locks the two from drifting (e.g. registry `jpg` vs a
        // future map `jpeg`), the same guard the document-extractor and
        // upload-allow-list lists already have.
        const WEB_EXT: &[(&str, &str)] = &[
            ("image/png", "png"),
            ("image/jpeg", "jpg"),
            ("image/jpg", "jpg"),
            ("image/gif", "gif"),
            ("image/webp", "webp"),
            ("application/pdf", "pdf"),
            ("text/plain", "txt"),
            ("text/markdown", "md"),
            ("text/csv", "csv"),
            ("application/json", "json"),
            ("application/xml", "xml"),
            ("text/xml", "xml"),
            ("application/rtf", "rtf"),
            ("text/rtf", "rtf"),
            (
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                "pptx",
            ),
            ("application/vnd.ms-powerpoint", "ppt"),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "docx",
            ),
            (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "xlsx",
            ),
            ("application/msword", "doc"),
            ("application/vnd.ms-excel", "xls"),
            ("audio/mpeg", "mp3"),
            ("audio/ogg", "ogg"),
            ("audio/wav", "wav"),
            ("audio/wave", "wav"),
            ("audio/x-wav", "wav"),
            ("audio/mp4", "mp4"),
            ("audio/x-m4a", "m4a"),
            ("audio/aac", "aac"),
            ("audio/flac", "flac"),
            ("audio/webm", "webm"),
        ];
        for (mime, ext) in WEB_EXT {
            assert_eq!(
                canonical_extension(mime),
                Some(*ext),
                "registry canonical_ext for {mime} diverged from web_attachment_ext"
            );
        }
    }
}
