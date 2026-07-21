//! The model-visible tool-result CONTENT preview — the correct vehicle for the
//! #5838 first-look inline preview across the capability-result collapse.
//!
//! [`crate::SafeSummary`] is a *caption* type: short (512 bytes), single-line,
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

use serde::{Deserialize, Serialize};

use crate::HostApiError;

/// Maximum size of a model-visible result preview, in bytes. Mirrors
/// `ironclaw_threads::contract::TOOL_RESULT_RECORD_READ_MAX_BYTES` (24 KiB) — the
/// largest raw first-look chunk a `result_read` returns — so the inline preview
/// and a follow-up read share one cap.
pub const MODEL_RESULT_PREVIEW_MAX_BYTES: usize = 24 * 1024;

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

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for ModelResultPreview {
    type Error = HostApiError;

    /// Wire revalidation matches construction (types.md canonical template): a
    /// persisted/relayed preview is re-checked against the current redaction rule
    /// on deserialize, never trusted transparently.
    fn try_from(value: String) -> Result<Self, HostApiError> {
        Self::new(value)
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
