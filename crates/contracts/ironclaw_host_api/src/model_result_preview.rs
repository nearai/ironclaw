//! The model-visible tool-result CONTENT preview — the correct vehicle for the
//! #5838 first-look inline preview across the capability-result collapse.
//!
//! [`crate::safe_summary::SafeSummary`] is a *caption* type: short (512 bytes), single-line,
//! delimiter-free. It was never the right home for tool-result CONTENT — routing
//! a first-look preview through it drops every legitimate result that contains a
//! `{`/`[`/`/` (all structured/JSON output) or the ordinary word "Secretary"
//! (the #6129 substring-`secret` bug), forcing the model into a re-read amnesia
//! loop. `ModelResultPreview` is the content vehicle instead:
//!
//! - **24 KiB** bound — mirrors `ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES`
//!   (the largest raw first-look chunk the model reads at once).
//! - **tolerates delimiters and newlines** — it is the tool's own raw-ish output,
//!   so `{ } [ ] / < >` and multi-line structure are legitimate content, not a
//!   redaction signal. (Rejects only NUL and other disallowed control chars.)
//! - **redacts ONLY genuine credentials** — word-boundary credential markers and
//!   secret-like tokens (the shared [`crate::credential_redaction`] scans, the
//!   same #5902/#6129 contract). Legitimate content and structure survive; only a
//!   real credential is refused. Per the `host_api` charter this is the tool's own
//!   output redacted for credentials — not internal host paths/secrets.
//!
//! The durable full output stays host-owned behind the result reference; this is
//! the bounded, credential-safe *preview* the model sees inline.

use crate::error::HostApiError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Maximum size of a model-visible result preview, in bytes. Mirrors
/// `ironclaw_threads::contract::TOOL_RESULT_RECORD_READ_MAX_BYTES` (24 KiB) — the
/// largest raw first-look chunk a `result_read` returns — so the inline preview
/// and a follow-up read share one cap.
pub const MODEL_RESULT_PREVIEW_MAX_BYTES: usize = 24 * 1024;
pub const MODEL_RESULT_JSON_PAGE_VIEW: &str = "ironclaw.json_page.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelResultJsonPageView {
    #[serde(rename = "ironclaw.json_page.v1")]
    V1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelResultJsonNodeType {
    Null,
    Boolean,
    Number,
    String,
    Array,
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelResultJsonOffsetUnit {
    Bytes,
    Items,
    Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelResultJsonOmittedDescriptor {
    Object(ModelResultJsonOmittedObject),
    Array(ModelResultJsonOmittedArray),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResultJsonOmittedObject {
    pub key: String,
    pub json_pointer: String,
    pub node_type: ModelResultJsonNodeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResultJsonOmittedArray {
    pub index: u64,
    pub json_pointer: String,
    pub node_type: ModelResultJsonNodeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResultJsonNextRequest {
    pub result_ref: String,
    pub json_pointer: String,
    pub offset: u64,
    pub max_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// Closed host controls around provider-owned JSON selected by threads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResultJsonPage {
    pub view: ModelResultJsonPageView,
    pub result_ref: String,
    pub json_pointer: String,
    pub node_type: ModelResultJsonNodeType,
    pub offset: u64,
    pub offset_unit: ModelResultJsonOffsetUnit,
    pub content: Value,
    pub omitted: Vec<ModelResultJsonOmittedDescriptor>,
    pub total_bytes: u64,
    pub next_offset: Option<u64>,
    pub next: Option<ModelResultJsonNextRequest>,
}

impl ModelResultJsonPage {
    pub fn from_json_str(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }

    pub fn to_value(&self) -> Result<Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}
/// A bounded, credential-redacted, model-visible preview of a tool result's
/// content. Tolerates delimiters/newlines (it is the tool's own output); refuses
/// only genuine credential material and NUL/disallowed control characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ModelResultPreview(String);

impl ModelResultPreview {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_model_result_preview(&value)?;
        Ok(Self(value))
    }

    /// Construct a preview from content that may contain credential markers or
    /// credential-shaped tokens, masking those spans instead of refusing the
    /// whole payload.
    ///
    /// The refusing [`Self::new`] is right for a caller that can reject the
    /// operation. It is wrong for model-visible *content*: dropping the preview
    /// also drops the continuation metadata that travels with it, so the model
    /// receives an opaque reference it cannot read or page. Prefer this when the
    /// alternative is showing nothing.
    pub fn redacted(value: impl Into<String>) -> Result<Self, HostApiError> {
        let mut value = value.into();
        match validate_model_result_preview(&value) {
            Ok(()) => Ok(Self(value)),
            Err(_) => {
                if let Ok(mut structured) = serde_json::from_str(&value)
                    && redact_structured_credential_values(&mut structured, 0)
                {
                    value = serde_json::to_string(&structured)
                        .unwrap_or_else(|_| "[redacted]".to_string());
                }
                let redacted = redact_model_result_text(&value)?;
                validate_model_result_preview(&redacted)?;
                Ok(Self(redacted))
            }
        }
    }

    /// Construct a preview from a threads-owned JSON page after provider
    /// content has passed the canonical redactor. Opaque references and JSON
    /// pointers remain exact host controls.
    pub fn from_redacted_json_page(page: ModelResultJsonPage) -> Result<Self, HostApiError> {
        let mut checked_content = page.content.clone();
        redact_structured_credential_values(&mut checked_content, 0);
        if checked_content != page.content {
            return Err(HostApiError::invalid_safe_summary(
                "JSON result page content must be credential-redacted",
            ));
        }
        let encoded = serde_json::to_string(&page).map_err(|error| {
            HostApiError::invalid_safe_summary(format!(
                "JSON result page could not serialize: {error}"
            ))
        })?;
        validate_json_page_wire(&encoded)?;
        Ok(Self(encoded))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

fn text_contains_redaction_target(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    crate::credential_redaction::contains_credential_label(&lower)
        || crate::credential_redaction::contains_secret_like_token(&lower)
        || contains_private_key_marker(&lower)
}

/// Whether a provider-owned JSON field name is credential-labeled and its
/// selected value must not cross the model-visible boundary.
///
/// Kept beside the canonical structured redactor so durable JSON paging cannot
/// drift to a second credential-label vocabulary.
pub fn json_field_name_requires_redaction(field_name: &str) -> bool {
    text_contains_redaction_target(field_name)
}

fn contains_private_key_marker(lower: &str) -> bool {
    lower.contains("private key") || lower.contains("private_key") || lower.contains("private-key")
}

fn contains_pem_private_key(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("-----begin ")
        && lower.contains("private key-----")
        && lower.contains("-----end ")
}

fn redact_model_result_text(value: &str) -> Result<String, HostApiError> {
    if contains_pem_private_key(value)
        || crate::credential_redaction::contains_unredacted_credential_value(
            &value.to_ascii_lowercase(),
        )
    {
        return Ok("[redacted]".to_string());
    }
    let redacted = crate::credential_redaction::redact_credential_text(value);
    validate_model_result_preview(&redacted)?;
    Ok(redacted)
}

fn redact_structured_credential_values(value: &mut serde_json::Value, depth: usize) -> bool {
    if depth >= 16 {
        *value = serde_json::Value::String("[redacted]".to_string());
        return true;
    }
    match value {
        serde_json::Value::Object(fields) => {
            fields.iter_mut().fold(false, |changed, (key, value)| {
                if json_field_name_requires_redaction(key) {
                    *value = serde_json::Value::String("[redacted]".to_string());
                    true
                } else {
                    redact_structured_credential_values(value, depth + 1) || changed
                }
            })
        }
        serde_json::Value::Array(values) => {
            let mut changed = false;
            for value in values {
                changed |= redact_structured_credential_values(value, depth + 1);
            }
            changed
        }
        serde_json::Value::String(text) => redact_embedded_structured_text(text, depth + 1),
        _ => false,
    }
}

fn redact_embedded_structured_text(text: &mut String, depth: usize) -> bool {
    let range = if serde_json::from_str::<serde_json::Value>(text).is_ok() {
        0..text.len()
    } else {
        let Some(start) = text.find(['{', '[']) else {
            return redact_unparsed_credential_text(text);
        };
        let Some(end) = text.rfind(['}', ']']).map(|end| end + 1) else {
            return redact_unparsed_credential_text(text);
        };
        start..end
    };
    let Ok(mut nested) = serde_json::from_str(&text[range.clone()]) else {
        return redact_unparsed_credential_text(text);
    };
    if !redact_structured_credential_values(&mut nested, depth) {
        return false;
    }
    let replacement = serde_json::to_string(&nested).unwrap_or_else(|_| "[redacted]".to_string());
    text.replace_range(range, &replacement);
    true
}

fn redact_unparsed_credential_text(text: &mut String) -> bool {
    let lower = text.to_ascii_lowercase();
    if !crate::credential_redaction::contains_unredacted_credential_value(&lower)
        && !contains_pem_private_key(text)
    {
        return false;
    }
    *text = "[redacted]".to_string();
    true
}

impl TryFrom<String> for ModelResultPreview {
    type Error = HostApiError;

    /// Wire revalidation matches construction (types.md canonical template): a
    /// persisted/relayed preview is re-checked against the current redaction rule
    /// on deserialize, never trusted transparently.
    fn try_from(value: String) -> Result<Self, HostApiError> {
        match ModelResultJsonPage::from_json_str(&value) {
            Ok(page) => {
                let preview = Self::from_redacted_json_page(page)?;
                if preview.as_str() == value {
                    Ok(preview)
                } else {
                    Err(HostApiError::invalid_safe_summary(
                        "JSON result page must use its canonical wire form",
                    ))
                }
            }
            Err(_) => Self::new(value),
        }
    }
}

impl AsRef<str> for ModelResultPreview {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelResultPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn validate_model_result_preview(value: &str) -> Result<(), HostApiError> {
    if value.is_empty() {
        return Err(HostApiError::invalid_safe_summary(
            "model result preview must not be empty",
        ));
    }
    if value.len() > MODEL_RESULT_PREVIEW_MAX_BYTES {
        return Err(HostApiError::invalid_safe_summary(format!(
            "model result preview must be at most {MODEL_RESULT_PREVIEW_MAX_BYTES} bytes"
        )));
    }
    // Content — NOT a caption: newlines/tabs are legitimate structure; only NUL
    // and other disallowed control characters are refused. Payload/path
    // delimiters (`{ } [ ] / < >`) are DELIBERATELY allowed (the tool's own
    // output), unlike the strict `SafeSummary` caption.
    if value
        .chars()
        .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(HostApiError::invalid_safe_summary(
            "model result preview must not contain NUL/disallowed control characters",
        ));
    }
    let lower = value.to_ascii_lowercase();
    // Only genuine credentials are refused — word-boundary markers so "Secretary"
    // survives (#6129), plus secret-like tokens. Everything else, delimiters and
    // structure included, is preserved.
    if crate::credential_redaction::contains_credential_marker(&lower) {
        return Err(HostApiError::invalid_safe_summary(
            "model result preview must not contain a sensitive marker",
        ));
    }
    if crate::credential_redaction::contains_secret_like_token(&lower) {
        return Err(HostApiError::invalid_safe_summary(
            "model result preview must not contain API-key-like tokens",
        ));
    }
    if contains_pem_private_key(value)
        || crate::credential_redaction::contains_unredacted_credential_value(&lower)
    {
        return Err(HostApiError::invalid_safe_summary(
            "model result preview must not contain private key material",
        ));
    }
    Ok(())
}

fn validate_json_page_wire(value: &str) -> Result<(), HostApiError> {
    if value.is_empty() || value.len() > MODEL_RESULT_PREVIEW_MAX_BYTES {
        return Err(HostApiError::invalid_safe_summary(format!(
            "model result preview must be between 1 and {MODEL_RESULT_PREVIEW_MAX_BYTES} bytes"
        )));
    }
    if value
        .chars()
        .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(HostApiError::invalid_safe_summary(
            "model result preview must not contain NUL/disallowed control characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The production incident: one catalog entry's summary said "no API key",
    /// `ModelResultPreview::new` refused the WHOLE 13.7 KB payload, and the
    /// caller dropped the preview *and* its continuation metadata — the model
    /// got an opaque reference it could neither read nor page.
    #[test]
    fn redacted_masks_credential_vocabulary_instead_of_refusing_the_payload() {
        let catalog = concat!(
            r#"{"catalog_total":61,"entries":["#,
            r#"{"name":"attio","description":"Attio CRM. Authenticated with a workspace API key."},"#,
            r#"{"name":"bitcoin-reddit-sentiment","description":"Reads Bitcoin posts (no API key needed)."},"#,
            r#"{"name":"near-rpc","description":"NEAR Protocol JSON-RPC integration."}"#,
            r#"]}"#
        );

        // Precondition: today's strict constructor refuses all of it.
        assert!(
            ModelResultPreview::new(catalog).is_err(),
            "fixture must reproduce the refusal that caused the incident"
        );

        let preview = ModelResultPreview::redacted(catalog).expect("redacted preview is built");
        let text = preview.as_str();

        // The payload survives: entries the user needed are still readable.
        assert!(text.contains("attio"), "content must survive redaction");
        assert!(text.contains("near-rpc"));
        assert!(text.contains("bitcoin-reddit-sentiment"));
        assert!(
            text.contains(r#""catalog_total":61"#),
            "totals must survive"
        );
        // The offending vocabulary is masked rather than taking the payload with it.
        assert!(!text.to_ascii_lowercase().contains("api key"));
        assert!(text.contains("[redacted]"));
    }

    /// Redaction must not fire on ordinary words that merely contain a marker.
    #[test]
    fn redacted_leaves_non_credential_words_alone() {
        let text = "The Secretary reviewed the passwordless login and the bearers of the note.";
        let preview = ModelResultPreview::redacted(text).expect("clean text passes through");
        assert_eq!(
            preview.as_str(),
            text,
            "word-boundary markers must not over-match"
        );
    }

    /// A genuine credential-shaped token is masked, not preserved.
    #[test]
    fn redacted_masks_secret_like_tokens() {
        let text = "deploy finished; token sk-abc123def456 was rotated";
        let preview = ModelResultPreview::redacted(text).expect("redacted preview is built");
        assert!(
            !preview.as_str().contains("sk-abc123def456"),
            "secret-shaped token must be masked"
        );
        assert!(
            preview.as_str().contains("deploy finished"),
            "surrounding content survives"
        );
    }

    #[test]
    fn preserves_delimiter_and_multiline_content() {
        // Structured/JSON output with delimiters and newlines is legitimate
        // content and must be retained (the whole point of the content vehicle).
        for ok in [
            "{\"rows\": 3, \"items\": [1, 2, 3]}",
            "line one\nline two\tindented",
            "path/to/workspace-sentinel.txt",
            "hello from tool",
            "<result>ok</result>",
        ] {
            assert!(
                ModelResultPreview::new(ok).is_ok(),
                "delimiter/multiline content must be retained: {ok:?}"
            );
        }
    }

    #[test]
    fn secret_markers_match_on_word_boundary_not_substring() {
        // #6129 regression: `secret` must NOT scrub `Secretary`.
        assert!(ModelResultPreview::new("Secretary of the Treasury signed the memo").is_ok());
        assert!(ModelResultPreview::new("The secretariat published its agenda").is_ok());
        // But a genuine credential is still refused.
        for bad in [
            "the client secret is xyz",
            "authorization: bearer abc123",
            "token sk-ant-abc123def456",
            "AKIA0123456789ABCDEF",
        ] {
            let why = ModelResultPreview::new(bad).unwrap_err().to_string();
            assert!(
                why.contains("sensitive marker") || why.contains("API-key-like tokens"),
                "genuine credential must be refused: {bad:?} (got {why})"
            );
        }
    }

    #[test]
    fn bounds_at_24_kib() {
        assert!(ModelResultPreview::new("x".repeat(MODEL_RESULT_PREVIEW_MAX_BYTES)).is_ok());
        assert_eq!(MODEL_RESULT_PREVIEW_MAX_BYTES, 24 * 1024);
        let err = ModelResultPreview::new("x".repeat(MODEL_RESULT_PREVIEW_MAX_BYTES + 1))
            .unwrap_err()
            .to_string();
        assert!(err.contains("at most"), "overlong must be rejected: {err}");
    }

    #[test]
    fn serde_revalidates_on_the_wire() {
        let value = "{\"ok\": true}\nSecretary of State";
        let json = serde_json::to_string(&ModelResultPreview::new(value).unwrap()).unwrap();
        let back: ModelResultPreview = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), value);
        // A hostile wire value is rejected on deserialize.
        assert!(
            serde_json::from_str::<ModelResultPreview>("\"token sk-ant-leaked123456\"").is_err()
        );
    }
}
