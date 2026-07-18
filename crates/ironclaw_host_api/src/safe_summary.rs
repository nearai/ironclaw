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
//! This is the canonical home for the safe-summary rule. It is an **exact,
//! non-weakening mirror** of `ironclaw_turns`' `validate_loop_safe_summary`
//! (the loop-facing `safe_summary: String` fields on `CapabilityOutcome` and
//! friends). Per `tools.md`, result vocabulary belongs in `host_api`; the doc's
//! migration folds those ad-hoc `String` fields onto this type. Until that
//! wiring slice lands, the turns validator is a temporary duplicate that must be
//! reconciled to delegate here — it must never diverge from the rules below, and
//! must never become *weaker* than them. The bound (512 bytes), the payload/path
//! delimiter ban, the credential-marker denylist, and the secret-like-token
//! detector are all defense-in-depth: the redactor at the construction site
//! scrubs first, and this type refuses to hold anything that slipped through.

use serde::{Deserialize, Serialize};

use crate::HostApiError;

/// Maximum length of a safe summary, in bytes. Matches the loop contract.
const MAX_SAFE_SUMMARY_BYTES: usize = 512;

/// A bounded, redacted, model-visible summary of a capability result.
///
/// Construction enforces the full redaction contract (see the module docs); a
/// value that contains raw payload/path delimiters, a credential marker, or a
/// secret-like token is rejected rather than stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for SafeSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for SafeSummary {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SafeSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Revalidate on the wire: a persisted/relayed summary is re-checked
        // against the current redaction rule, never trusted transparently.
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// The canonical safe-summary redaction rule. Exact mirror of
/// `ironclaw_turns::run_profile::host::validate_loop_safe_summary` (minus the
/// turns-local `INPUT_ENCODE_HUMAN_SUMMARY` sentinel bypass, which is a
/// loop-input-encoding concern, not a general redaction rule).
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
    if value.chars().any(|c| {
        matches!(
            c,
            '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\'
        )
    }) {
        return Err(HostApiError::invalid_safe_summary(
            "must not contain raw payload or path delimiters",
        ));
    }

    let lower = value.to_ascii_lowercase();
    // Only credential markers are banned; descriptive error vocabulary
    // ("provider error", "stack trace", "tool input", …) is allowed because the
    // raw cause rides the dedicated model-visible detail channel.
    for forbidden in [
        "access token",
        "api key",
        "api_key",
        "apikey",
        "authorization:",
        "bearer ",
        "password",
        "passwd",
        "secret",
    ] {
        if lower.contains(forbidden) {
            return Err(HostApiError::invalid_safe_summary(format!(
                "must not contain sensitive marker `{forbidden}`"
            )));
        }
    }
    if contains_secret_like_token(&lower) {
        return Err(HostApiError::invalid_safe_summary(
            "must not contain API-key-like tokens",
        ));
    }
    Ok(())
}

fn contains_secret_like_token(lower: &str) -> bool {
    lower
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_' | '.')
        })
        .any(is_secret_like_token)
}

fn is_secret_like_token(token: &str) -> bool {
    [
        "sk-",
        "sk-ant-",
        "ghp_",
        "github_pat_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "glpat-",
        "gcp-",
        "ya29.",
        "aiza",
    ]
    .iter()
    .any(|prefix| token.starts_with(prefix))
        || (token.len() >= 16 && (token.starts_with("akia") || token.starts_with("asia")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_plain_redacted_summary() {
        let s = SafeSummary::new("read 3 files, no changes").unwrap();
        assert_eq!(s.as_str(), "read 3 files, no changes");
    }

    #[test]
    fn rejects_empty_and_overlong() {
        assert!(SafeSummary::new("").is_err());
        assert!(SafeSummary::new("x".repeat(MAX_SAFE_SUMMARY_BYTES + 1)).is_err());
        assert!(SafeSummary::new("x".repeat(MAX_SAFE_SUMMARY_BYTES)).is_ok());
    }

    #[test]
    fn rejects_payload_and_path_delimiters() {
        for bad in ["{\"k\":1}", "a[0]", "path/to/x", "c:\\x", "<tag>", "`code`"] {
            assert!(SafeSummary::new(bad).is_err(), "should reject {bad:?}");
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
        ] {
            assert!(SafeSummary::new(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn allows_descriptive_error_vocabulary() {
        // The raw cause rides a separate channel; descriptive words are allowed.
        for ok in ["provider error", "stack trace truncated", "tool input rejected"] {
            assert!(SafeSummary::new(ok).is_ok(), "should allow {ok:?}");
        }
    }

    #[test]
    fn serde_revalidates_on_the_wire() {
        let json = serde_json::to_string(&SafeSummary::new("ok summary").unwrap()).unwrap();
        assert_eq!(json, "\"ok summary\"");
        // A hostile wire value is rejected on deserialize, not trusted.
        assert!(serde_json::from_str::<SafeSummary>("\"api key: sk-ant-x\"").is_err());
    }
}
