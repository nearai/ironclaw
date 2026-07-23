//! Shared validators, secret/marker-detection heuristics, and model-visible
//! text sanitization for host-owned loop-ref newtypes and DTOs.

use ironclaw_host_api::INPUT_ENCODE_HUMAN_SUMMARY;

use crate::run_profile::prompt_text::{PromptTextSurface, validate_prompt_text};

const FORBIDDEN_MODEL_ROUTE_MARKERS: &[&str] = &[
    "access_token",
    "api_key",
    "apikey",
    "authorization",
    "password",
    "passwd",
    "secret",
];

const FORBIDDEN_EXACT_MODEL_ROUTE_MARKERS: &[&str] = &["bearer"];

pub(crate) fn validate_bounded_loop_string(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{label} must be at most {max_bytes} bytes"));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(format!("{label} must not contain NUL/control characters"));
    }
    Ok(value)
}

pub(crate) fn validate_prefixed_loop_ref(
    label: &'static str,
    prefix: &'static str,
    max_bytes: usize,
    value: String,
) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, label, max_bytes)?;
    if !value.starts_with(prefix) {
        return Err(format!("{label} must start with `{prefix}`"));
    }
    Ok(value)
}

pub(crate) fn validate_prefixed_path_safe_loop_ref(
    label: &'static str,
    prefix: &'static str,
    max_bytes: usize,
    value: String,
) -> Result<String, String> {
    let value = validate_prefixed_loop_ref(label, prefix, max_bytes, value)?;
    if value.contains('/') || value.contains('\\') || value.contains("..") {
        return Err(format!(
            "{label} must not contain path separators or parent-directory markers"
        ));
    }
    Ok(value)
}

pub(crate) fn validate_loop_opaque_token(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, label, max_bytes)?;
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(format!(
            "{label} must contain only ASCII letters, digits, _, -, or ."
        ));
    }
    Ok(value)
}

pub(crate) fn validate_loop_safe_identifier(
    value: String,
    label: &'static str,
    max_bytes: usize,
) -> Result<String, String> {
    let value = validate_bounded_loop_string(value, label, max_bytes)?;
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
    }) {
        return Err(format!(
            "{label} must contain only ASCII letters, digits, _, -, ., or :"
        ));
    }

    let lower = value.to_ascii_lowercase();
    for forbidden in [
        "access_token",
        "access-token",
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "password",
        "passwd",
        "secret",
    ] {
        if lower.contains(forbidden) {
            return Err(format!(
                "{label} must not contain sensitive marker `{forbidden}`"
            ));
        }
    }
    if contains_secret_like_token(&lower) {
        return Err(format!("{label} must not contain API-key-like tokens"));
    }
    Ok(value)
}

pub(crate) fn validate_loop_safe_summary(value: String) -> Result<String, String> {
    // Loop-input-encoding sentinel bypass: a host-authored fixed literal that is
    // not a redaction concern. Everything else delegates to the single canonical
    // redaction rule owned by `ironclaw_host_api::SafeSummary` (see that type's
    // module docs) — this validator must never diverge from it.
    if value == INPUT_ENCODE_HUMAN_SUMMARY {
        return Ok(value);
    }
    ironclaw_host_api::SafeSummary::new(value)
        .map(|summary| summary.into_inner())
        .map_err(|error| error.to_string())
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

pub(crate) fn validate_loop_inline_message_body(value: String) -> Result<String, String> {
    validate_prompt_text(
        value,
        "loop inline message body",
        PromptTextSurface::GenericModelContent,
    )
    .map_err(|error| error.safe_summary)
}

/// Validate a persisted provider/model route component with the same redaction
/// marker policy used by host-owned loop snapshots and IronClaw route keys.
pub fn validate_model_route_component_value(
    label: &'static str,
    value: &str,
    max_bytes: usize,
    allowed: impl Fn(char) -> bool,
) -> Result<(), String> {
    validate_bounded_loop_string(value.to_string(), label, max_bytes)?;
    if value.trim() != value {
        return Err(format!("{label} must not contain surrounding whitespace"));
    }
    if !value.chars().all(allowed) {
        return Err(format!("{label} contains unsupported characters"));
    }
    reject_sensitive_model_route_markers(label, value)?;
    Ok(())
}

fn reject_sensitive_model_route_markers(label: &'static str, value: &str) -> Result<(), String> {
    let lower = value.to_ascii_lowercase();
    for token in model_route_marker_tokens(&lower) {
        if FORBIDDEN_EXACT_MODEL_ROUTE_MARKERS.contains(&token)
            || FORBIDDEN_MODEL_ROUTE_MARKERS
                .iter()
                .any(|forbidden| token_contains_sensitive_marker(token, forbidden))
            || token.starts_with("sk-")
        {
            return Err(format!("{label} contains a forbidden marker"));
        }
    }
    Ok(())
}

fn model_route_marker_tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '-' && character != '_'
        })
        .filter(|token| !token.is_empty())
}

fn token_contains_sensitive_marker(token: &str, marker: &str) -> bool {
    let normalized = token.replace('-', "_");
    normalized == marker
        || normalized.starts_with(&format!("{marker}_"))
        || normalized.ends_with(&format!("_{marker}"))
        || normalized.contains(&format!("_{marker}_"))
}

/// Redact credential-looking tokens before model deltas cross public/loggable
/// loop surfaces.
pub fn sanitize_model_visible_text(value: impl Into<String>) -> String {
    let value = value.into();
    let mut sanitized = String::with_capacity(value.len());
    let mut token = String::new();

    for character in value.chars() {
        if character.is_whitespace() {
            flush_sanitized_model_token(&mut sanitized, &mut token);
            sanitized.push(character);
        } else {
            token.push(character);
        }
    }
    flush_sanitized_model_token(&mut sanitized, &mut token);

    sanitized
}

fn flush_sanitized_model_token(sanitized: &mut String, token: &mut String) {
    if token.is_empty() {
        return;
    }
    if model_token_needs_redaction(token) {
        sanitized.push_str("[redacted]");
    } else {
        sanitized.push_str(token);
    }
    token.clear();
}

fn model_token_needs_redaction(token: &str) -> bool {
    let normalized = token
        .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .to_ascii_lowercase();
    normalized.contains("api_key")
        || normalized.contains("access_token")
        || normalized.contains("raw_credential_sentinel")
        || normalized.contains("raw_provider_secret")
        // Reuse the shared prefix heuristic so every provider-token format this
        // file already recognizes (`sk-`, GitHub `ghp_`/`gho_`/…, GitLab, GCP,
        // Google, AWS `AKIA`/`ASIA`) is redacted at the model-visible boundary,
        // not just the handful of substrings enumerated above.
        || is_secret_like_token(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_summary_accepts_ordinary_error_vocabulary() {
        // Words that used to be banned outright are ordinary error vocabulary,
        // not secrets, and must now be accepted.
        for accepted in [
            "provider error occurred during the call",
            "stack trace was captured for diagnosis",
            "the tool input was malformed",
            "a traceback is available for review",
            "host path resolution did not complete",
            "raw runtime returned an unexpected status",
        ] {
            validate_loop_safe_summary(accepted.to_string())
                .unwrap_or_else(|error| panic!("`{accepted}` should be accepted: {error}"));
        }
    }

    #[test]
    fn safe_summary_still_rejects_secret_markers_and_delimiters() {
        // Credential markers must still be rejected.
        for rejected in [
            "leaked sk-LIVEsecretvalue token",
            "authorization header bearer abc123",
            "the api key was exposed",
            "user password was logged",
            "a secret slipped into the message",
        ] {
            validate_loop_safe_summary(rejected.to_string())
                .expect_err(&format!("`{rejected}` must still be rejected"));
        }

        // Path / payload delimiters must still be rejected.
        validate_loop_safe_summary("missing schema at /system/extensions".to_string())
            .expect_err("path delimiter `/` must still be rejected");
    }

    #[test]
    fn sanitize_model_visible_text_redacts_provider_token_formats() {
        // The model-visible boundary must redact every provider-token format the
        // file already recognizes via `is_secret_like_token`, not only `sk-`,
        // `api_key`, and `access_token`. Regression for GitHub/AWS/GCP/Google
        // formats leaking through `sanitize_model_visible_text`.
        for secret in [
            "ghp_0123456789abcdefABCDEF0123456789abcd",
            "AKIAIOSFODNN7EXAMPLE",
            "ya29.a0ARrdaM-exampletoken",
            "AIzaSyExampleToken1234567890",
        ] {
            let sanitized = sanitize_model_visible_text(format!("token {secret} here"));
            assert!(
                sanitized.contains("[redacted]"),
                "`{secret}` should have been redacted, got: {sanitized}"
            );
            assert!(
                !sanitized.contains(secret),
                "`{secret}` leaked through sanitization: {sanitized}"
            );
        }
    }
}
