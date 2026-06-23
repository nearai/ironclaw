//! Host-side document text extraction for capability outputs.
//!
//! A WASM extension that downloads a binary document (PDF, PPTX, DOCX, ...)
//! cannot turn the bytes into text itself: the type-aware extractor
//! ([`ironclaw_extractors`]) is host Rust and is not linkable into a WASM
//! guest. Instead the guest returns the raw bytes base64-encoded under
//! `content_base64` (alongside `mime_type` and an optional `name`), and this
//! module — invoked at the capability-result seam in [`crate::obligations`] —
//! decodes them, runs the extractor, and replaces `content_base64` with an
//! extracted-text `content` field. The base64 never reaches the model; only the
//! bounded extracted text does.
//!
//! The transform is **capability-gated**: it runs only for the capabilities in
//! [`DOCUMENT_OUTPUT_CAPABILITIES`], so it is opt-in per capability rather than
//! a global rewrite of every result that happens to carry `content_base64`.
//! This host-side seam is transitional — when large-file download moves to the
//! per-project sandbox (#4999) it is removed.
//!
//! Scope: this inspects the **top-level** result object only. Download
//! capabilities return a single flat file object (one `file_id` -> one result);
//! there is no batch/array download that returns an array of files or nests a
//! document payload, so the base64 is always at the top level and recursive
//! traversal would be solving a problem no producer creates. The transform is
//! still fail-safe for the top-level object: once `content_base64` is present it
//! is removed regardless of outcome, so raw base64 can never reach the model.
//! Running before the redaction/size obligations in `complete_dispatch` means
//! the extracted text is leak-scanned and the large base64 is gone before any
//! output-size ceiling is evaluated.

use base64::Engine as _;
use serde_json::Value;

/// Maximum characters of extracted text retained (~25K tokens). Mirrors the
/// inbound-attachment extraction cap in `ironclaw_attachments`.
const MAX_EXTRACTED_TEXT_CHARS: usize = 100_000;

/// Upper bound on the base64 payload we will decode. This is the *only*
/// pre-decode size guard: `content_base64` is stripped before
/// `dispatch_output_bytes()` runs, so the output-size obligation never sees it.
/// Tied to the producer contract — the Drive guest caps raw downloads at 1 MB
/// (`MAX_DOWNLOAD_TEXT_BYTES`), ~1.37 MB once base64-encoded — so 2 MiB leaves
/// headroom for a legitimate payload while denying a misbehaving guest the
/// chance to force a large host allocation.
const MAX_BASE64_INPUT_BYTES: usize = 2 * 1024 * 1024;

/// Capabilities whose output may carry a base64 document payload to extract.
/// Keep this explicit so the transform is opt-in per capability, not a global
/// rewrite of every result that happens to contain `content_base64`.
/// Transitional: when large-file download moves to the per-project sandbox
/// (#4999) this host-side seam is removed.
const DOCUMENT_OUTPUT_CAPABILITIES: &[&str] = &["google-drive.download_file"];

/// If the top-level `output` object carries base64 document bytes
/// (`content_base64`), decode and run the document text extractor, replacing
/// `content_base64` with an extracted-text `content` field. Any other output
/// passes through unchanged.
///
/// **Capability-gated:** the transform only runs for capabilities listed in
/// [`DOCUMENT_OUTPUT_CAPABILITIES`]. Results from any other capability pass
/// through untouched even if they happen to contain a `content_base64` field —
/// the rewrite is opt-in per capability, never a global protocol. This gate is
/// transitional and removed once large-file download moves to the per-project
/// sandbox (#4999).
///
/// Once `content_base64` is present (for a gated capability) it is **always**
/// removed — on a missing `mime_type`, a non-string payload, an oversize
/// payload, or an extraction failure the field is dropped and a short marker is
/// placed in `content` instead, so raw base64 can never reach the model.
pub(crate) fn extract_documents_in_output(capability_id: &str, mut output: Value) -> Value {
    // Opt-in per capability: a result from any non-listed capability passes
    // through unchanged, even if it contains a `content_base64` field.
    if !DOCUMENT_OUTPUT_CAPABILITIES.contains(&capability_id) {
        return output;
    }
    let Some(obj) = output.as_object_mut() else {
        return output;
    };
    // The only leak-free early exit: there is no base64 payload to handle.
    if !obj.contains_key("content_base64") {
        return output;
    }

    // From here we are committed to removing `content_base64` below, whatever
    // the outcome. Copy the inputs out as owned values first so the borrow ends
    // before the mutating remove/insert.
    let encoded = obj
        .get("content_base64")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mime = obj
        .get("mime_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let filename = obj.get("name").and_then(Value::as_str).map(str::to_string);

    let content = match (encoded, mime) {
        (Some(encoded), Some(mime)) => decode_and_extract(&encoded, &mime, filename.as_deref()),
        (Some(_), None) => {
            "[Downloaded file is missing its mime type; cannot extract text.]".to_string()
        }
        (None, _) => "[Downloaded file payload was not a string; cannot extract text.]".to_string(),
    };

    obj.remove("content_base64");
    obj.insert("content".to_string(), Value::String(content));
    output
}

/// Decode base64 bytes and run the type-aware extractor, returning the text or
/// a bracketed, model-readable failure marker (never the raw bytes).
fn decode_and_extract(encoded: &str, mime: &str, filename: Option<&str>) -> String {
    decode_and_extract_capped(encoded, mime, filename, MAX_BASE64_INPUT_BYTES)
}

/// Like [`decode_and_extract`] but with an explicit base64-size cap (for tests).
fn decode_and_extract_capped(
    encoded: &str,
    mime: &str,
    filename: Option<&str>,
    max_base64_bytes: usize,
) -> String {
    if encoded.len() > max_base64_bytes {
        tracing::debug!(
            encoded_len = encoded.len(),
            mime,
            "download output base64 exceeds decode cap"
        );
        return "[Downloaded file is too large to process.]".to_string();
    }
    let bytes = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::debug!(%error, "download output base64 decode failed");
            return "[Could not decode the downloaded file.]".to_string();
        }
    };
    match ironclaw_extractors::extract_document(&bytes, mime, filename) {
        ironclaw_extractors::DocumentExtraction::Text(text) => {
            ironclaw_extractors::truncate_to_chars(&text, MAX_EXTRACTED_TEXT_CHARS)
        }
        ironclaw_extractors::DocumentExtraction::Empty => {
            format!("[No extractable text found in {mime} document.]")
        }
        ironclaw_extractors::DocumentExtraction::Failed(error) => {
            tracing::debug!(mime, filename, %error, "document text extraction failed");
            format!("[Could not extract text from {mime} document.]")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A capability that opts into the document-extraction transform.
    const GATED: &str = "google-drive.download_file";

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn non_object_passes_through() {
        let out = extract_documents_in_output(GATED, json!("just a string"));
        assert_eq!(out, json!("just a string"));
    }

    #[test]
    fn non_listed_capability_leaves_base64_untouched() {
        // A capability that is NOT in DOCUMENT_OUTPUT_CAPABILITIES must pass
        // through unchanged — the transform is opt-in per capability, not a
        // global rewrite of every result carrying `content_base64`.
        let input = json!({
            "file_id": "f1",
            "name": "data.csv",
            "mime_type": "text/csv",
            "content_base64": b64(b"name,age\nAlice,30"),
        });
        let out = extract_documents_in_output("builtin.echo", input.clone());
        assert_eq!(
            out, input,
            "a non-listed capability must leave `content_base64` untouched"
        );
        assert!(
            out.get("content_base64").is_some(),
            "the payload must pass through unchanged for a non-listed capability"
        );
        assert!(out.get("content").is_none());
    }

    #[test]
    fn text_content_is_untouched() {
        // A normal text/exported download carries `content`, never
        // `content_base64`, so the transform must leave it exactly as-is.
        let input = json!({
            "file_id": "f1",
            "name": "notes.txt",
            "mime_type": "text/plain",
            "content": "hello world",
        });
        assert_eq!(extract_documents_in_output(GATED, input.clone()), input);
    }

    #[test]
    fn missing_mime_strips_base64_and_marks() {
        // content_base64 present but no mime_type must NOT leak base64: it is
        // stripped and replaced with a marker.
        let out = extract_documents_in_output(GATED, json!({ "content_base64": b64(b"abc") }));
        assert!(
            out.get("content_base64").is_none(),
            "base64 must be stripped even when mime_type is missing"
        );
        assert!(
            out["content"].as_str().unwrap_or("").starts_with('['),
            "expected a marker in content, got: {:?}",
            out["content"]
        );
    }

    #[test]
    fn non_string_content_base64_is_stripped() {
        // An unexpected non-string `content_base64` must still be removed, never
        // passed through.
        let out = extract_documents_in_output(
            GATED,
            json!({
                "mime_type": "application/pdf",
                "content_base64": { "nested": "object" },
            }),
        );
        assert!(out.get("content_base64").is_none());
        assert!(out["content"].as_str().unwrap_or("").starts_with('['));
    }

    #[test]
    fn extracts_csv_and_drops_base64() {
        // Exercises the decode -> extract_text -> replace path end-to-end
        // without a binary fixture; text/csv is handled by the extractor.
        let out = extract_documents_in_output(
            GATED,
            json!({
                "file_id": "f1",
                "name": "data.csv",
                "mime_type": "text/csv",
                "content_base64": b64(b"name,age\nAlice,30"),
            }),
        );
        assert_eq!(out["content"], json!("name,age\nAlice,30"));
        assert!(
            out.get("content_base64").is_none(),
            "base64 must be removed so it never reaches the model"
        );
    }

    #[test]
    fn extracts_pdf_binary() {
        // Real binary document: a minimal PDF whose text is "Hello World".
        let pdf = include_bytes!("../../../tests/fixtures/hello.pdf");
        let out = extract_documents_in_output(
            GATED,
            json!({
                "file_id": "f1",
                "name": "hello.pdf",
                "mime_type": "application/pdf",
                "content_base64": b64(pdf),
            }),
        );
        let content = out["content"].as_str().unwrap_or("");
        assert!(
            content.contains("Hello"),
            "PDF extraction should contain 'Hello', got: {content}"
        );
        assert!(out.get("content_base64").is_none());
    }

    #[test]
    fn unsupported_binary_yields_failure_marker_not_base64() {
        // An unsupported/opaque binary still must not leak base64; the model
        // gets a bracketed marker instead.
        let out = extract_documents_in_output(
            GATED,
            json!({
                "file_id": "f1",
                "name": "image.png",
                "mime_type": "image/png",
                "content_base64": b64(&[0x89, 0x50, 0x4e, 0x47, 0x00, 0x01, 0x02]),
            }),
        );
        let content = out["content"].as_str().unwrap_or("");
        assert!(content.starts_with('['), "expected marker, got: {content}");
        assert!(out.get("content_base64").is_none());
    }

    #[test]
    fn invalid_base64_yields_marker() {
        let out = extract_documents_in_output(
            GATED,
            json!({
                "mime_type": "application/pdf",
                "content_base64": "not valid base64 !!!",
            }),
        );
        let content = out["content"].as_str().unwrap_or("");
        assert!(content.contains("Could not decode"), "got: {content}");
        assert!(out.get("content_base64").is_none());
    }

    #[test]
    fn oversize_base64_is_marked_not_decoded() {
        // Over the cap -> rejected before decoding, with a marker. Uses a tiny
        // cap so the test allocates nothing large.
        let marker = decode_and_extract_capped("QUFBQUFBQUE=", "application/pdf", None, 4);
        assert!(marker.contains("too large"), "got: {marker}");
    }

    #[test]
    fn payload_far_above_drive_contract_is_rejected_before_decode() {
        // ~3 MB base64 is well above the Drive producer contract (1 MB raw,
        // ~1.37 MB base64) and must be rejected before any decode. Guards against
        // loosening the pre-decode cap back toward the former 16 MiB ceiling: at
        // 16 MiB this blob would be decoded instead of marked "too large".
        let blob = "A".repeat(3 * 1024 * 1024);
        let marker = decode_and_extract(&blob, "application/pdf", None);
        assert!(
            marker.contains("too large"),
            "3 MB payload must be rejected pre-decode; got: {}",
            &marker[..marker.len().min(80)]
        );
    }
}
