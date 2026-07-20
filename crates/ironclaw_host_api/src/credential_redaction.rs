//! Shared credential-redaction primitives for the model-visible result
//! vocabulary — the single definition used by both [`crate::SafeSummary`] (the
//! bounded caption) and [`crate::ModelResultPreview`] (the bounded tool-result
//! CONTENT preview).
//!
//! Two independent scans:
//!
//! - **credential markers** — human-readable credential words (`secret`,
//!   `password`, `bearer `, …) matched at a **word boundary**, not as a
//!   substring. The substring form is the #6129 bug: `"Secretary of the
//!   Treasury".contains("secret")` is true, so every legitimate tool result
//!   mentioning "Secretary" got scrubbed to a stub and the model re-read it in
//!   an amnesia loop. Markers that already begin/end with a non-alphanumeric
//!   delimiter (`bearer `, `authorization:`) carry their own boundary and keep
//!   matching exactly as before.
//! - **secret-like tokens** — credential-shaped opaque tokens (`sk-…`, `ghp_…`,
//!   `AKIA…`, …). Already word-split by its own tokenizer.
//!
//! Both are defense-in-depth: the redactor at the construction site scrubs
//! first; these types refuse to hold anything that slipped through.

/// Human-readable credential markers. Matched at a word boundary (see
/// [`contains_credential_marker`]); the ones ending/starting in a non-alnum
/// delimiter carry their own boundary.
const CREDENTIAL_MARKERS: [&str; 9] = [
    "access token",
    "api key",
    "api_key",
    "apikey",
    "authorization:",
    "bearer ",
    "password",
    "passwd",
    "secret",
];

/// True when `lower` (already lowercased) contains any credential marker as a
/// standalone token rather than embedded in a larger alphanumeric word.
pub(crate) fn contains_credential_marker(lower: &str) -> bool {
    CREDENTIAL_MARKERS
        .iter()
        .any(|marker| contains_marker_at_word_boundary(lower, marker))
}

/// True if `marker` occurs in `haystack` (already lowercased) as a standalone
/// token rather than embedded inside a larger alphanumeric word. Prevents
/// false positives like the marker `secret` matching the ordinary word
/// `secretary` ("Secretary of the Treasury"), which would otherwise scrub
/// legitimate tool output. Markers that begin/end with a non-alphanumeric
/// delimiter (e.g. `bearer `, `authorization:`) already carry their own
/// boundary and keep matching exactly as before. Canonical copy of
/// `ironclaw_threads::tool_result_reference::contains_marker_at_word_boundary`
/// (verified there by `sensitive_markers_match_on_word_boundary_not_substring`).
fn contains_marker_at_word_boundary(haystack: &str, marker: &str) -> bool {
    if marker.is_empty() {
        return false;
    }
    let starts_alnum = marker.starts_with(|c: char| c.is_ascii_alphanumeric());
    let ends_alnum = marker.ends_with(|c: char| c.is_ascii_alphanumeric());
    for (start, _) in haystack.match_indices(marker) {
        let end = start + marker.len();
        let before_ok = !starts_alnum
            || start == 0
            || !haystack[..start].ends_with(|c: char| c.is_ascii_alphanumeric());
        let after_ok = !ends_alnum
            || end >= haystack.len()
            || !haystack[end..].starts_with(|c: char| c.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// True when any whitespace/punctuation-delimited token in `lower` (already
/// lowercased) begins with a credential-shaped prefix (`sk-`, `ghp_`, `AKIA…`).
pub(crate) fn contains_secret_like_token(lower: &str) -> bool {
    lower
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_' | '.')
        })
        .any(has_secret_like_prefix)
}

/// True when a credential-shaped prefix starts this token or any interior
/// segment after a `-`/`_`/`.` separator. The tokenizer keeps those separators
/// inside tokens so multi-part prefixes like `github_pat_` stay matchable — but
/// that alone would let `memo_sk-abc123` hide a key behind a leading word, so
/// every separator boundary is checked as a token start too. (Tokens are pure
/// ASCII by construction: the split removes every non-ASCII-alphanumeric
/// character except `-`/`_`/`.`, so byte indexing after a separator is
/// char-boundary-safe.)
fn has_secret_like_prefix(token: &str) -> bool {
    if is_secret_like_token(token) {
        return true;
    }
    token
        .char_indices()
        .filter(|(_, character)| matches!(character, '-' | '_' | '.'))
        .any(|(index, _)| is_secret_like_token(&token[index + 1..]))
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
    fn markers_match_on_word_boundary_not_substring() {
        // The #6129 regression: `secret` must NOT trip on `Secretary`.
        assert!(!contains_credential_marker("secretary of the treasury"));
        assert!(!contains_credential_marker(
            "the secretariat scheduled a meeting"
        ));
        // But a standalone `secret` (and delimiter-bounded markers) still trip.
        assert!(contains_credential_marker("the secret is out"));
        assert!(contains_credential_marker("client secret: xyz"));
        assert!(contains_credential_marker("authorization: bearer x"));
        assert!(contains_credential_marker("bearer abc"));
        assert!(contains_credential_marker("the password is hunter2"));
        // `passwordless` is a different word — not a standalone `password`.
        assert!(!contains_credential_marker("passwordless login enabled"));
    }

    #[test]
    fn secret_like_tokens_are_detected_even_behind_a_leading_word() {
        assert!(contains_secret_like_token("token sk-ant-abc123"));
        assert!(contains_secret_like_token("ghp_0123456789abcdef"));
        assert!(contains_secret_like_token("note memo_sk-abc123 saved"));
        assert!(contains_secret_like_token("akia0123456789abcdef"));
        // A hyphenated ordinary phrase must not false-positive.
        assert!(!contains_secret_like_token("risk-based task-list check"));
    }
}
