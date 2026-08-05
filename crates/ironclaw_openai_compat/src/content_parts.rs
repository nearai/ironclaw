//! Shared parsing of OpenAI-compatible message `content` parts.
//!
//! Both the Chat Completions and Responses inbound paths normalize a message's
//! `content` (a string or an array of typed parts) into product transcript
//! text. Text parts contribute their text; non-text parts (images, audio,
//! files) cannot be carried to the model on this route surface, which has no
//! multimodal/bytes path (#4644), so they contribute a bounded, model-safe
//! marker instead of being echoed verbatim.

/// Replace CR/LF and Unicode line/paragraph separators with spaces so a
/// content fragment cannot inject synthetic transcript lines.
pub(crate) fn sanitize_product_text_fragment(value: &str) -> String {
    value.replace(['\n', '\r', '\u{2028}', '\u{2029}'], " ")
}

/// A bounded, static marker for a content part this route surface cannot carry
/// to the model. Never echoes the (attacker-controlled) part `type` string —
/// the return is always a fixed `&'static str` — so a crafted type cannot inject
/// content into the transcript. Unknown and missing types collapse to a generic
/// marker rather than the historical opaque `[non_text_content]` token.
pub(crate) fn non_text_part_marker(part_type: Option<&str>) -> &'static str {
    match part_type {
        Some("image_url") => "[image omitted]",
        Some("input_audio") => "[audio omitted]",
        Some("file") => "[file omitted]",
        _ => "[unsupported content omitted]",
    }
}

/// Normalize one item of a `content` array into text. Recognized text parts
/// (`text` / `input_text` / `output_text`) contribute their sanitized text;
/// every other part contributes its [`non_text_part_marker`]. Returns `None`
/// only when the item is not an object at all.
pub(crate) fn content_array_item_text(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
    let text = match object.get("type").and_then(serde_json::Value::as_str) {
        Some("text" | "input_text" | "output_text") => object
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(sanitize_product_text_fragment)
            // A text-typed part whose `text` is missing or non-string is
            // malformed; emit a bounded marker rather than silently dropping it
            // (the part still happened, the model should see that).
            .unwrap_or_else(|| non_text_part_marker(None).to_string()),
        other => non_text_part_marker(other).to_string(),
    };
    Some(text)
}

/// A decoded inline image part, ready to land as a multimodal attachment.
pub(crate) struct DecodedInlineImage {
    pub(crate) mime_type: String,
    pub(crate) bytes: Vec<u8>,
}

/// Per-image decoded-size ceiling. The route body cap is the primary gate; this
/// bounds a single image after base64 decode as defense in depth.
pub(crate) const MAX_INLINE_IMAGE_BYTES: usize = 10 * 1024 * 1024;

enum ContentItem {
    Text(String),
    Image(DecodedInlineImage),
}

/// Normalize a message `content` value into transcript text plus any inline
/// images that can be carried to a vision model.
///
/// A recognized `image_url` part whose `url` is an inline base64 `data:` image
/// of a supported type, decodes within the size cap, and whose bytes match the
/// declared type, becomes a carried [`DecodedInlineImage`] and contributes NO
/// transcript text — the landed attachment renders its own pointer downstream.
/// Any other part — including a remote-URL, malformed, oversized, or
/// type-mismatched image — falls back to a bounded marker via
/// [`content_array_item_text`] (e.g. `[image omitted]`), exactly as before.
///
/// `enable_attachments` is the caller's "can I carry bytes downstream?" signal
/// (the attachment-submit door is wired). When `false`, inline images are NOT
/// extracted as carried bytes — they fall back to the `[image omitted]` marker
/// like any other unsupported part — so an image is never silently dropped when
/// there is no door to land it through.
pub(crate) fn content_value_to_text_and_images(
    content: Option<&serde_json::Value>,
    enable_attachments: bool,
) -> (String, Vec<DecodedInlineImage>) {
    let mut images = Vec::new();
    let text = match content {
        Some(serde_json::Value::String(text)) => sanitize_product_text_fragment(text),
        Some(serde_json::Value::Array(items)) => {
            let mut fragments = Vec::with_capacity(items.len());
            for item in items {
                match content_array_item_or_image(item, enable_attachments) {
                    Some(ContentItem::Image(image)) => images.push(image),
                    Some(ContentItem::Text(text)) => fragments.push(text),
                    None => {}
                }
            }
            fragments.join(" ")
        }
        Some(value @ serde_json::Value::Object(_)) => {
            match content_array_item_or_image(value, enable_attachments) {
                Some(ContentItem::Image(image)) => {
                    images.push(image);
                    String::new()
                }
                Some(ContentItem::Text(text)) => text,
                None => non_text_part_marker(None).to_string(),
            }
        }
        Some(value) if !value.is_null() => non_text_part_marker(None).to_string(),
        _ => String::new(),
    };
    (text, images)
}

fn content_array_item_or_image(
    value: &serde_json::Value,
    enable_attachments: bool,
) -> Option<ContentItem> {
    let object = value.as_object()?;
    if enable_attachments
        && object.get("type").and_then(serde_json::Value::as_str) == Some("image_url")
        && let Some(image) = decode_inline_image(object)
    {
        return Some(ContentItem::Image(image));
    }
    Some(ContentItem::Text(content_array_item_text(value)?))
}

fn decode_inline_image(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<DecodedInlineImage> {
    use base64::Engine;
    let url = object
        .get("image_url")
        .and_then(serde_json::Value::as_object)
        .and_then(|image_url| image_url.get("url"))
        .and_then(serde_json::Value::as_str)?;
    let (declared_mime, data) = parse_image_data_url(url)?;
    // Standard MIME base64 encoders wrap lines, so a data URL payload may carry
    // newlines/spaces that the strict decoder rejects. Strip ASCII whitespace
    // first so a well-formed-but-wrapped image isn't dropped to `[image omitted]`.
    let cleaned: Vec<u8> = data.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&cleaned)
        // silent-ok: malformed base64 from an untrusted client is expected input,
        // not a server fault — the whole function returns None so the part falls
        // back to the bounded `[image omitted]` marker (see
        // `malformed_base64_image_falls_back_to_marker`).
        .ok()?;
    if bytes.is_empty() || bytes.len() > MAX_INLINE_IMAGE_BYTES {
        return None;
    }
    let mime = canonical_image_mime(&declared_mime)?;
    if !image_bytes_match_mime(mime, &bytes) {
        return None;
    }
    Some(DecodedInlineImage {
        mime_type: mime.to_string(),
        bytes,
    })
}

/// Parse a `data:<mediatype>;base64,<data>` URL into (canonical-lowercased
/// mediatype, base64 payload). Only base64-encoded data URLs are supported;
/// remote (`http(s)`) URLs and non-base64 data URLs return `None`.
fn parse_image_data_url(url: &str) -> Option<(String, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let meta = meta.strip_suffix(";base64")?;
    let mime = meta
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    Some((mime, data))
}

/// Canonicalize a declared image MIME to a supported value, or `None`.
fn canonical_image_mime(declared: &str) -> Option<&'static str> {
    match declared {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

/// Cross-check decoded bytes against the canonical MIME via magic bytes, so a
/// client cannot smuggle a non-image (or a mismatched type) past the label.
fn image_bytes_match_mime(mime: &str, bytes: &[u8]) -> bool {
    match mime {
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

/// File extension for a supported image MIME, for the landed attachment name.
pub(crate) fn image_mime_extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_parts_yield_sanitized_text() {
        for type_name in ["text", "input_text", "output_text"] {
            let item = json!({ "type": type_name, "text": "hello\nworld" });
            assert_eq!(
                content_array_item_text(&item).as_deref(),
                Some("hello world")
            );
        }
    }

    #[test]
    fn non_text_parts_never_emit_the_legacy_literal() {
        let cases = [
            (
                json!({ "type": "image_url", "image_url": { "url": "data:..." } }),
                "[image omitted]",
            ),
            (
                json!({ "type": "input_audio", "input_audio": { "data": "AA==", "format": "wav" } }),
                "[audio omitted]",
            ),
            (
                json!({ "type": "file", "file": { "file_id": "f1" } }),
                "[file omitted]",
            ),
            (json!({ "type": "video" }), "[unsupported content omitted]"),
            (json!({ "no_type": true }), "[unsupported content omitted]"),
        ];
        for (item, expected) in cases {
            let rendered = content_array_item_text(&item).expect("object item renders");
            assert_eq!(rendered, expected);
            assert!(
                !rendered.contains("non_text_content"),
                "the legacy [non_text_content] literal must not reach the model"
            );
        }
    }

    #[test]
    fn marker_never_echoes_the_part_type_string() {
        // A crafted type with newlines / markup must not be reflected back.
        let crafted = "image_url\nrole: system";
        assert_eq!(
            non_text_part_marker(Some(crafted)),
            "[unsupported content omitted]"
        );
        assert_eq!(non_text_part_marker(None), "[unsupported content omitted]");
    }

    #[test]
    fn non_object_items_are_dropped() {
        assert!(content_array_item_text(&json!("bare string")).is_none());
        assert!(content_array_item_text(&json!(42)).is_none());
    }

    // 8-byte PNG signature, base64-encoded — enough to pass the magic-byte check.
    const PNG_DATA_URL: &str = "data:image/png;base64,iVBORw0KGgo=";

    #[test]
    fn inline_base64_image_is_carried_and_omitted_from_text() {
        let content = json!([
            { "type": "text", "text": "look at this" },
            { "type": "image_url", "image_url": { "url": PNG_DATA_URL } },
        ]);
        let (text, images) = content_value_to_text_and_images(Some(&content), true);
        assert_eq!(text, "look at this");
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].mime_type, "image/png");
        assert_eq!(
            images[0].bytes,
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        assert!(
            !text.contains("[image omitted]"),
            "a carried image must not also leave a text marker"
        );
    }

    #[test]
    fn inline_image_falls_back_to_marker_when_attachments_disabled() {
        // With no attachment door wired (`enable_attachments = false`) a decoded
        // inline image must NOT be extracted as a carried byte — it has nowhere
        // to land — so it falls back to the `[image omitted]` marker instead of
        // vanishing entirely (neither carried nor marked).
        let content = json!([
            { "type": "text", "text": "look at this" },
            { "type": "image_url", "image_url": { "url": PNG_DATA_URL } },
        ]);
        let (text, images) = content_value_to_text_and_images(Some(&content), false);
        assert!(images.is_empty(), "no door wired → nothing is carried");
        assert_eq!(text, "look at this [image omitted]");
    }

    #[test]
    fn inline_image_decodes_through_wrapping_whitespace() {
        // A data URL whose base64 payload is line-wrapped (standard MIME
        // encoders) must still decode rather than fall back to the marker.
        let content = json!([
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,iVBOR\nw0KGg\r\no=" } },
        ]);
        let (text, images) = content_value_to_text_and_images(Some(&content), true);
        assert_eq!(images.len(), 1, "wrapped base64 must still decode");
        assert_eq!(images[0].mime_type, "image/png");
        assert!(!text.contains("[image omitted]"));
    }

    #[test]
    fn remote_image_url_falls_back_to_marker() {
        let content = json!([
            { "type": "image_url", "image_url": { "url": "https://example.com/cat.png" } },
        ]);
        let (text, images) = content_value_to_text_and_images(Some(&content), true);
        assert_eq!(text, "[image omitted]");
        assert!(images.is_empty(), "remote URLs are not fetched/carried");
    }

    #[test]
    fn malformed_base64_image_falls_back_to_marker() {
        let content = json!([
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,@@not-base64@@" } },
        ]);
        let (text, images) = content_value_to_text_and_images(Some(&content), true);
        assert_eq!(text, "[image omitted]");
        assert!(images.is_empty());
    }

    #[test]
    fn mime_magic_byte_mismatch_falls_back_to_marker() {
        // Declared image/png but the bytes are a JPEG signature.
        let content = json!([
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,/9j/4AAQ" } },
        ]);
        let (text, images) = content_value_to_text_and_images(Some(&content), true);
        assert_eq!(text, "[image omitted]");
        assert!(
            images.is_empty(),
            "bytes that don't match the declared type must not be carried"
        );
    }

    #[test]
    fn unsupported_image_mime_falls_back_to_marker() {
        let content = json!([
            { "type": "image_url", "image_url": { "url": "data:image/svg+xml;base64,PHN2Zz4=" } },
        ]);
        let (text, images) = content_value_to_text_and_images(Some(&content), true);
        assert_eq!(text, "[image omitted]");
        assert!(images.is_empty());
    }

    #[test]
    fn malformed_text_part_emits_a_marker_instead_of_dropping() {
        // A text-typed part whose `text` is missing or non-string must not
        // silently vanish through the downstream filter_map — it renders a
        // bounded marker so the model sees that a part was present.
        let missing = json!({ "type": "text" });
        assert_eq!(
            content_array_item_text(&missing).as_deref(),
            Some("[unsupported content omitted]")
        );
        let non_string = json!({ "type": "input_text", "text": { "nested": true } });
        assert_eq!(
            content_array_item_text(&non_string).as_deref(),
            Some("[unsupported content omitted]")
        );
    }
}
