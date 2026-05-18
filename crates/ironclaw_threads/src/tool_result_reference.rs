use serde::{Deserialize, Serialize};

// Mirrors `ironclaw_turns::LoopResultRef` without adding a threads -> turns
// dependency: `result:` plus a non-empty 256-byte opaque id made from
// ASCII letters, digits, `_`, `-`, or `.`.
const MAX_TOOL_RESULT_REF_BYTES: usize = 256;
const MAX_TOOL_RESULT_SUMMARY_BYTES: usize = 512;
const RAW_PAYLOAD_OR_PATH_DELIMITERS: [char; 9] = ['{', '}', '[', ']', '`', '<', '>', '/', '\\'];
const SENSITIVE_SUMMARY_MARKERS: [&str; 18] = [
    "access token",
    "api key",
    "api_key",
    "apikey",
    "authorization:",
    "bearer ",
    "host path",
    "invalid api key",
    "invalid_api_key",
    "password",
    "passwd",
    "provider error",
    "raw runtime",
    "secret",
    "stack trace",
    "tool input",
    "tool_input",
    "traceback",
];

/// Safe summary text for tool-result transcript references.
///
/// Thread records can be replayed into model-visible context through transcript
/// adapters, so this boundary rejects summaries that look like raw payloads,
/// paths, stack traces, or credentials. The validator below is the canonical
/// stored-content schema for this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ToolResultSafeSummary(String);

impl ToolResultSafeSummary {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        validate_tool_result_safe_summary(value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl<'de> Deserialize<'de> for ToolResultSafeSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultReferenceEnvelope {
    pub version: u32,
    pub result_ref: String,
    pub safe_summary: ToolResultSafeSummary,
}

impl ToolResultReferenceEnvelope {
    pub fn new(
        result_ref: impl Into<String>,
        safe_summary: ToolResultSafeSummary,
    ) -> Result<Self, String> {
        let result_ref = result_ref.into();
        validate_tool_result_ref(&result_ref)?;
        Ok(Self {
            version: 1,
            result_ref,
            safe_summary,
        })
    }
}

fn validate_tool_result_ref(value: &str) -> Result<(), String> {
    let Some(suffix) = value.strip_prefix("result:") else {
        return Err("tool result ref must start with result:".to_string());
    };
    if suffix.is_empty() {
        return Err("tool result ref must include an opaque id after result:".to_string());
    }
    if value.len() > MAX_TOOL_RESULT_REF_BYTES {
        return Err(format!(
            "tool result ref exceeds {MAX_TOOL_RESULT_REF_BYTES} bytes"
        ));
    }
    if !suffix
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(
            "tool result ref opaque id must contain only ASCII letters, digits, _, -, or ."
                .to_string(),
        );
    }
    Ok(())
}

fn validate_tool_result_safe_summary(value: String) -> Result<String, String> {
    if value.is_empty() {
        return Err("tool result summary must not be empty".to_string());
    }
    if value.len() > MAX_TOOL_RESULT_SUMMARY_BYTES {
        return Err(format!(
            "tool result summary exceeds {MAX_TOOL_RESULT_SUMMARY_BYTES} bytes"
        ));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err("tool result summary must not contain NUL/control characters".to_string());
    }
    if value
        .chars()
        .any(|character| RAW_PAYLOAD_OR_PATH_DELIMITERS.contains(&character))
    {
        return Err(
            "tool result summary must not contain raw payload or path delimiters".to_string(),
        );
    }

    let lower = value.to_ascii_lowercase();
    for forbidden in SENSITIVE_SUMMARY_MARKERS {
        if lower.contains(forbidden) {
            return Err(format!(
                "tool result summary must not contain sensitive marker `{forbidden}`"
            ));
        }
    }
    // Intentionally over-reject short `sk-...` tokens: opaque tool summaries
    // are cheap to rephrase, while credential-shaped text is costly to persist.
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(|token| token.starts_with("sk-"))
    {
        return Err("tool result summary must not contain API-key-like tokens".to_string());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{ToolResultReferenceEnvelope, ToolResultSafeSummary};

    #[test]
    fn safe_summary_rejects_control_characters() {
        assert!(ToolResultSafeSummary::new("line\u{0}break").is_err());
        assert!(ToolResultSafeSummary::new("line\nbreak").is_err());
    }

    #[test]
    fn safe_summary_api_key_check_is_token_based() {
        assert!(ToolResultSafeSummary::new("sky-high confidence").is_ok());
        assert!(ToolResultSafeSummary::new("completed with sk-live-token").is_err());
    }

    #[test]
    fn tool_result_ref_uses_loop_result_ref_shape() {
        let summary = ToolResultSafeSummary::new("tool completed").expect("summary");
        assert!(
            ToolResultReferenceEnvelope::new("result:tool-output_1.2", summary.clone()).is_ok()
        );

        for invalid_ref in [
            "result:",
            "result:foo:bar",
            "result:contains/slash",
            "result:contains space",
            "result:contains\ncontrol",
        ] {
            assert!(
                ToolResultReferenceEnvelope::new(invalid_ref, summary.clone()).is_err(),
                "accepted invalid result ref {invalid_ref:?}"
            );
        }
    }

    #[test]
    fn tool_result_ref_rejects_over_256_bytes() {
        let summary = ToolResultSafeSummary::new("tool completed").expect("summary");
        let too_long = format!("result:{}", "a".repeat(250));

        assert!(ToolResultReferenceEnvelope::new(too_long, summary).is_err());
    }
}
