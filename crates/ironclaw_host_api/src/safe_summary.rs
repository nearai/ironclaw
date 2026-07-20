//! Slice-C kernel vocabulary — the bounded, redacted result summary.
//!
//! Part of the capability-path result collapse
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md` §3):
//! every result channel carries a `SafeSummary` — a short, model-visible string
//! that is guaranteed to hold no raw payload, path, or credential material. Full
//! output stays host-owned and is retrieved only through a result reference.
//!
//! ## Redaction contract
//!
//! This is the canonical home for the safe-summary rule — the **single
//! definition**. `ironclaw_turns`' `validate_loop_safe_summary` (the
//! loop-facing `safe_summary: String` fields on `CapabilityOutcome` and
//! friends) and `ironclaw_memory_native`'s memory-snippet validator both
//! DELEGATE to `SafeSummary::new`; they layer only their own domain-specific
//! concerns on top (a loop-input-encoding sentinel bypass, extra snippet
//! vocabulary bans). Change the rule here, never in a delegate. The bound
//! (512 bytes), the payload/path delimiter ban, the credential-marker
//! denylist, and the secret-like-token detector are all defense-in-depth: the
//! redactor at the construction site scrubs first, and this type refuses to
//! hold anything that slipped through.

use serde::{Deserialize, Serialize};

use crate::HostApiError;

/// Maximum length of a safe summary, in bytes. Matches the loop contract.
const MAX_SAFE_SUMMARY_BYTES: usize = 512;

/// A bounded, redacted, model-visible summary of a capability result.
///
/// Construction enforces the full redaction contract (see the module docs); a
/// value that contains raw payload/path delimiters, a credential marker, or a
/// secret-like token is rejected rather than stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SafeSummary(String);

impl SafeSummary {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_safe_summary(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }

    /// An infallible, redaction-safe fallback summary.
    ///
    /// A caller that must produce a `SafeSummary` from an untrusted upstream
    /// string (e.g. mapping a loop-facing `safe_summary: String` onto this type)
    /// needs a non-panicking last resort when the upstream value fails the
    /// redaction contract. Only this crate can construct a `SafeSummary` without
    /// validation, so the placeholder lives here: it is built from a fixed
    /// compile-time constant that satisfies [`validate_safe_summary`] (pinned by
    /// `placeholder_is_valid`), so no external caller needs `unwrap`/`expect` to
    /// obtain a value.
    pub fn placeholder() -> Self {
        Self(SAFE_SUMMARY_PLACEHOLDER.to_string())
    }
}

/// The fixed, redaction-safe text backing [`SafeSummary::placeholder`]. Contains
/// only lowercase ASCII letters and spaces, so it can never trip a delimiter,
/// control-character, credential-marker, or secret-token rule.
const SAFE_SUMMARY_PLACEHOLDER: &str = "capability summary unavailable";

impl TryFrom<String> for SafeSummary {
    type Error = HostApiError;

    /// Wire revalidation matches construction (types.md canonical template): a
    /// persisted/relayed summary is re-checked against the current redaction
    /// rule on deserialize, never trusted transparently.
    fn try_from(value: String) -> Result<Self, HostApiError> {
        Self::new(value)
    }
}

impl AsRef<str> for SafeSummary {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SafeSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The canonical safe-summary redaction rule — the single definition.
/// `ironclaw_turns::run_profile::host::validate_loop_safe_summary` and
/// `ironclaw_memory_native`'s memory-snippet validator DELEGATE here (the turns
/// copy keeps only its `INPUT_ENCODE_HUMAN_SUMMARY` sentinel bypass, a
/// loop-input-encoding concern; memory_native layers extra snippet-specific
/// bans on top). Change the rule here, never in a delegate.
fn validate_safe_summary(value: &str) -> Result<(), HostApiError> {
    if value.is_empty() {
        return Err(HostApiError::invalid_safe_summary("must not be empty"));
    }
    if value.len() > MAX_SAFE_SUMMARY_BYTES {
        return Err(HostApiError::invalid_safe_summary(format!(
            "must be at most {MAX_SAFE_SUMMARY_BYTES} bytes"
        )));
    }
    // Mirror the loop contract exactly: ALL control characters are rejected
    // (newlines/tabs included) — a summary is a single bounded line.
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(HostApiError::invalid_safe_summary(
            "must not contain NUL/control characters",
        ));
    }
    if value
        .chars()
        .any(|c| matches!(c, '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\'))
    {
        return Err(HostApiError::invalid_safe_summary(
            "must not contain raw payload or path delimiters",
        ));
    }

    let lower = value.to_ascii_lowercase();
    // Only credential markers are banned; descriptive error vocabulary
    // ("provider error", "stack trace", "tool input", …) is allowed because the
    // raw cause rides the dedicated model-visible detail channel. Markers match
    // at a WORD BOUNDARY, not as a substring (#6129): `secret` must not scrub
    // `Secretary`.
    if crate::credential_redaction::contains_credential_marker(&lower) {
        return Err(HostApiError::invalid_safe_summary(
            "must not contain a sensitive marker",
        ));
    }
    if crate::credential_redaction::contains_secret_like_token(&lower) {
        return Err(HostApiError::invalid_safe_summary(
            "must not contain API-key-like tokens",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_plain_redacted_summary() {
        let s = SafeSummary::new("read 3 files, no changes").unwrap();
        assert_eq!(s.as_str(), "read 3 files, no changes");
    }

    fn rejection(value: impl Into<String>) -> String {
        SafeSummary::new(value).unwrap_err().to_string()
    }

    #[test]
    fn placeholder_is_valid() {
        // The infallible fallback MUST satisfy the same redaction contract it
        // bypasses at construction; if the constant ever regresses, this fails.
        assert!(
            validate_safe_summary(SAFE_SUMMARY_PLACEHOLDER).is_ok(),
            "placeholder constant must satisfy the redaction rule"
        );
        assert_eq!(
            SafeSummary::placeholder().as_str(),
            SAFE_SUMMARY_PLACEHOLDER
        );
        // And it round-trips through the wire like any other summary.
        let wire = serde_json::to_string(&SafeSummary::placeholder()).unwrap();
        let back: SafeSummary = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, SafeSummary::placeholder());
    }

    #[test]
    fn rejects_empty_and_overlong() {
        // Assert the specific reason, not just is_err(), so an infrastructure
        // failure can't masquerade as a validation pass.
        assert!(rejection("").contains("must not be empty"));
        assert!(
            rejection("x".repeat(MAX_SAFE_SUMMARY_BYTES + 1)).contains("at most"),
            "overlong must be rejected for length"
        );
        assert!(SafeSummary::new("x".repeat(MAX_SAFE_SUMMARY_BYTES)).is_ok());
    }

    #[test]
    fn rejects_payload_and_path_delimiters() {
        for bad in ["{\"k\":1}", "a[0]", "path/to/x", "c:\\x", "<tag>", "`code`"] {
            assert!(
                rejection(bad).contains("raw payload or path delimiters"),
                "should reject {bad:?} for delimiters"
            );
        }
    }

    #[test]
    fn rejects_credential_markers_and_secret_tokens() {
        for bad in [
            "api key leaked",
            "Authorization: bearer x",
            "the password is hunter2",
            "token sk-ant-abc123",
            "ghp_0123456789abcdef",
            "AKIA0123456789ABCDEF",
            // A separator-joined leading word must not hide a key: the former
            // memory_native tokenizer split on `_`/`.` and caught these; the
            // canonical detector must be no weaker (regression for the
            // boundary-scan in `has_secret_like_prefix`).
            "note memo_sk-abc123 saved",
            "memo.ghp_0123456789abcdef",
            "backup_AKIA0123456789ABCDEF",
        ] {
            let why = rejection(bad);
            assert!(
                why.contains("sensitive marker") || why.contains("API-key-like tokens"),
                "should reject {bad:?} for a credential-shaped reason, got: {why}"
            );
        }
    }

    #[test]
    fn allows_descriptive_error_vocabulary() {
        // The raw cause rides a separate channel; descriptive words are allowed.
        for ok in [
            "provider error",
            "stack trace truncated",
            "tool input rejected",
            // Boundary scan must not false-positive on words that merely
            // contain a prefix mid-segment.
            "risk-based task-list check",
            // #6129 regression: `secret` must not scrub `Secretary`.
            "Secretary of the Treasury confirmed",
            "the secretariat meets tuesday",
        ] {
            assert!(SafeSummary::new(ok).is_ok(), "should allow {ok:?}");
        }
    }

    #[test]
    fn serde_revalidates_on_the_wire() {
        let json = serde_json::to_string(&SafeSummary::new("ok summary").unwrap()).unwrap();
        assert_eq!(json, "\"ok summary\"");
        // A hostile wire value is rejected on deserialize, not trusted.
        let err = serde_json::from_str::<SafeSummary>("\"api key: sk-ant-x\"")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("sensitive marker"),
            "wire rejection must carry the validation reason: {err}"
        );
    }
}
