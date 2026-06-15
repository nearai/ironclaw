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
