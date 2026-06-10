use std::sync::OnceLock;

use crate::LeakDetector;

pub const PROVIDER_TOOL_NAME_MAX_BYTES: usize = 64;

const PROVIDER_ARGUMENTS_MAX_BYTES: usize = 16 * 1024;
const PROVIDER_ARGUMENTS_MAX_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ProviderValidationError {
    message: String,
}

impl ProviderValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub fn validate_provider_tool_name(value: &str) -> Result<(), ProviderValidationError> {
    if value.is_empty() {
        return Err(ProviderValidationError::new(
            "provider tool name must not be empty",
        ));
    }
    if value.len() > PROVIDER_TOOL_NAME_MAX_BYTES {
        return Err(ProviderValidationError::new(format!(
            "provider tool name exceeds {PROVIDER_TOOL_NAME_MAX_BYTES} bytes"
        )));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(ProviderValidationError::new(
            "provider tool name must contain only ASCII letters, digits, _, or -",
        ));
    }
    Ok(())
}

pub fn validate_provider_identity(
    value: &str,
    label: &str,
    max_len: usize,
) -> Result<(), ProviderValidationError> {
    if value.trim().is_empty() {
        return Err(ProviderValidationError::new(format!(
            "{label} must not be empty"
        )));
    }
    if value.len() > max_len {
        return Err(ProviderValidationError::new(format!(
            "{label} exceeds {max_len} bytes"
        )));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(ProviderValidationError::new(format!(
            "{label} must not contain NUL/control characters"
        )));
    }
    Ok(())
}

pub fn validate_provider_token(
    value: &str,
    label: &str,
    max_len: usize,
) -> Result<(), ProviderValidationError> {
    if value.is_empty() {
        return Err(ProviderValidationError::new(format!(
            "{label} must not be empty"
        )));
    }
    if value.len() > max_len {
        return Err(ProviderValidationError::new(format!(
            "{label} exceeds {max_len} bytes"
        )));
    }
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
    }) {
        return Err(ProviderValidationError::new(format!(
            "{label} must contain only ASCII letters, digits, _, -, ., or :"
        )));
    }
    Ok(())
}

pub fn validate_provider_arguments(
    arguments: &serde_json::Value,
) -> Result<(), ProviderValidationError> {
    let arguments_len = serde_json::to_vec(arguments)
        .map_err(|error| ProviderValidationError::new(error.to_string()))?
        .len();
    if arguments_len > PROVIDER_ARGUMENTS_MAX_BYTES {
        return Err(ProviderValidationError::new(format!(
            "provider tool arguments exceed {PROVIDER_ARGUMENTS_MAX_BYTES} bytes"
        )));
    }
    validate_provider_json_value(arguments, "provider arguments", 0)
}

pub fn validate_optional_provider_metadata_text(
    value: Option<&str>,
    label: &str,
    max_len: usize,
) -> Result<(), ProviderValidationError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_provider_metadata_text_bounds(value, label, max_len)?;
    validate_provider_metadata_text(value, label)
}

pub fn scrub_optional_provider_metadata_text(
    value: Option<&str>,
    label: &str,
    max_len: usize,
) -> Result<Option<String>, ProviderValidationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    validate_provider_metadata_text_bounds(value, label, max_len)?;
    scrub_provider_metadata_text(value, label).map(Some)
}

pub fn scrub_optional_provider_metadata_string(
    value: Option<String>,
    label: &str,
    max_len: usize,
) -> Result<Option<String>, ProviderValidationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    validate_provider_metadata_text_bounds(&value, label, max_len)?;
    scrub_provider_metadata_string(value, label).map(Some)
}

pub fn scrub_provider_metadata_text(
    value: &str,
    label: &str,
) -> Result<String, ProviderValidationError> {
    validate_provider_metadata_text_controls(value, label)?;
    Ok(redact_provider_secret_matches(value))
}

pub fn scrub_provider_metadata_string(
    value: String,
    label: &str,
) -> Result<String, ProviderValidationError> {
    validate_provider_metadata_text_controls(&value, label)?;
    Ok(redact_provider_secret_matches_owned(value))
}

fn validate_provider_json_value(
    value: &serde_json::Value,
    label: &str,
    depth: usize,
) -> Result<(), ProviderValidationError> {
    if depth > PROVIDER_ARGUMENTS_MAX_DEPTH {
        return Err(ProviderValidationError::new(format!(
            "{label} exceed maximum nesting depth"
        )));
    }
    match value {
        serde_json::Value::String(text) => validate_provider_argument_text(text, label),
        serde_json::Value::Array(items) => {
            for item in items {
                validate_provider_json_value(item, label, depth + 1)?;
            }
            Ok(())
        }
        serde_json::Value::Object(entries) => {
            for (key, item) in entries {
                validate_provider_json_key(key)?;
                validate_provider_json_value(item, label, depth + 1)?;
            }
            Ok(())
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Ok(())
        }
    }
}

fn validate_provider_json_key(key: &str) -> Result<(), ProviderValidationError> {
    if key
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(ProviderValidationError::new(
            "provider argument key must not contain NUL/control characters",
        ));
    }
    Ok(())
}

fn validate_provider_metadata_text(
    value: &str,
    label: &str,
) -> Result<(), ProviderValidationError> {
    validate_provider_metadata_text_controls(value, label)?;
    reject_provider_secret_leaks(value, label)
}

fn validate_provider_metadata_text_bounds(
    value: &str,
    label: &str,
    max_len: usize,
) -> Result<(), ProviderValidationError> {
    if value.len() > max_len {
        return Err(ProviderValidationError::new(format!(
            "{label} exceeds {max_len} bytes"
        )));
    }
    validate_provider_metadata_text_controls(value, label)
}

fn validate_provider_metadata_text_controls(
    value: &str,
    label: &str,
) -> Result<(), ProviderValidationError> {
    if value.chars().any(|character| {
        character == '\0' || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    }) {
        return Err(ProviderValidationError::new(format!(
            "{label} must not contain NUL/control characters"
        )));
    }
    Ok(())
}

fn validate_provider_argument_text(
    value: &str,
    label: &str,
) -> Result<(), ProviderValidationError> {
    if value.chars().any(|character| {
        character == '\0' || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    }) {
        return Err(ProviderValidationError::new(format!(
            "{label} must not contain NUL/control characters"
        )));
    }
    reject_provider_secret_leaks(value, label)
}

fn reject_provider_secret_leaks(value: &str, label: &str) -> Result<(), ProviderValidationError> {
    static DETECTOR: OnceLock<LeakDetector> = OnceLock::new();
    let result = DETECTOR.get_or_init(LeakDetector::new).scan(value);
    if result.should_block || result.redacted_content.is_some() {
        return Err(ProviderValidationError::new(format!(
            "{label} must not contain secret-like tokens"
        )));
    }
    Ok(())
}

fn redact_provider_secret_matches(value: &str) -> String {
    static DETECTOR: OnceLock<LeakDetector> = OnceLock::new();
    let result = DETECTOR.get_or_init(LeakDetector::new).scan(value);
    if result.matches.is_empty() {
        return value.to_string();
    }

    redact_provider_secret_match_ranges(value, result.matches)
}

fn redact_provider_secret_matches_owned(value: String) -> String {
    static DETECTOR: OnceLock<LeakDetector> = OnceLock::new();
    let result = DETECTOR.get_or_init(LeakDetector::new).scan(&value);
    if result.matches.is_empty() {
        return value;
    }

    redact_provider_secret_match_ranges(&value, result.matches)
}

fn redact_provider_secret_match_ranges(value: &str, matches: Vec<crate::LeakMatch>) -> String {
    let mut ranges = matches
        .into_iter()
        .map(|leak_match| leak_match.location)
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);

    let mut redacted = String::with_capacity(value.len());
    let mut cursor = 0;
    for range in ranges {
        if range.end <= cursor {
            continue;
        }
        if range.start > cursor {
            redacted.push_str(&value[cursor..range.start]);
        }
        redacted.push_str("[REDACTED]");
        cursor = range.end;
    }
    redacted.push_str(&value[cursor..]);
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_arguments_allow_multiline_text() {
        validate_provider_arguments(&serde_json::json!({
            "content": "---\r\nname: pasted-skill\n---\n\nUse multiline Markdown.\n\t- with tabs\n"
        }))
        .expect("multiline provider argument text is valid");
    }

    #[test]
    fn provider_arguments_reject_non_whitespace_controls() {
        let error = validate_provider_arguments(&serde_json::json!({
            "content": "line one\u{0001}line two"
        }))
        .expect_err("non-whitespace control character should fail");

        assert!(error.to_string().contains("NUL/control characters"));
    }

    #[test]
    fn provider_metadata_allows_benign_sensitive_marker_phrases() {
        validate_optional_provider_metadata_text(
            Some("provider error included traceback"),
            "provider reasoning",
            4096,
        )
        .expect("ordinary provider reasoning phrases should pass");
    }

    #[test]
    fn provider_metadata_scrubber_redacts_secret_like_tokens() {
        let api_key = format!("sk-proj-{}", "a".repeat(24));
        let redacted = scrub_optional_provider_metadata_text(
            Some(&format!("use {api_key} to finish")),
            "provider reasoning",
            4096,
        )
        .expect("metadata should be scrubbed")
        .expect("metadata present");

        assert_eq!(redacted, "use [REDACTED] to finish");
        assert!(!redacted.contains(&api_key));
    }

    #[test]
    fn provider_metadata_allows_multiline_text() {
        for value in [
            "line one\nline two",
            "line one\rline two",
            "line one\tline two",
        ] {
            validate_optional_provider_metadata_text(Some(value), "provider reasoning", 4096)
                .expect("metadata text whitespace control should pass");
        }
    }

    #[test]
    fn provider_metadata_rejects_non_whitespace_controls() {
        let error = validate_optional_provider_metadata_text(
            Some("line one\u{0001}line two"),
            "provider reasoning",
            4096,
        )
        .expect_err("non-whitespace control character should fail");

        assert!(error.to_string().contains("NUL/control characters"));
    }

    #[test]
    fn provider_text_rejects_secret_like_tokens() {
        let api_key = format!("sk-proj-{}", "a".repeat(24));
        let error = validate_provider_arguments(&serde_json::json!({"api_key": api_key}))
            .expect_err("secret-like token should fail");

        assert!(error.to_string().contains("secret-like tokens"));
    }
}
