use ironclaw_host_api::CapabilityId;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCallReferenceEnvelope {
    pub provider_id: String,
    pub provider_model_id: String,
    pub provider_turn_id: String,
    pub provider_call_id: String,
    pub provider_tool_name: String,
    pub capability_id: CapabilityId,
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl ProviderToolCallReferenceEnvelope {
    pub fn validate(&self) -> Result<(), String> {
        validate_provider_identity(&self.provider_id, "provider id", 512)?;
        validate_provider_identity(&self.provider_model_id, "provider model id", 512)?;
        validate_provider_token(&self.provider_turn_id, "provider turn id", 512)?;
        validate_provider_token(&self.provider_call_id, "provider call id", 512)?;
        validate_provider_tool_name(&self.provider_tool_name)?;
        validate_provider_arguments(&self.arguments)?;
        validate_optional_provider_text(
            &self.response_reasoning,
            "provider response reasoning",
            4096,
        )?;
        validate_optional_provider_text(&self.reasoning, "provider reasoning", 4096)?;
        validate_optional_provider_text(&self.signature, "provider signature", 4096)?;
        Ok(())
    }
}

fn validate_provider_tool_name(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("provider tool name must not be empty".to_string());
    }
    if value.len() > 64 {
        return Err("provider tool name exceeds 64 bytes".to_string());
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(
            "provider tool name must contain only ASCII letters, digits, _, or -".to_string(),
        );
    }
    Ok(())
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

fn validate_provider_token(value: &str, label: &str, max_len: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.len() > max_len {
        return Err(format!("{label} exceeds {max_len} bytes"));
    }
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
    }) {
        return Err(format!(
            "{label} must contain only ASCII letters, digits, _, -, ., or :"
        ));
    }
    Ok(())
}

fn validate_provider_identity(value: &str, label: &str, max_len: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.len() > max_len {
        return Err(format!("{label} exceeds {max_len} bytes"));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(format!("{label} must not contain NUL/control characters"));
    }
    Ok(())
}

fn validate_provider_arguments(arguments: &serde_json::Value) -> Result<(), String> {
    let arguments_len = serde_json::to_vec(arguments)
        .map_err(|error| format!("provider arguments are not serializable: {error}"))?
        .len();
    if arguments_len > 16 * 1024 {
        return Err("provider tool arguments exceed 16384 bytes".to_string());
    }
    validate_provider_json_value(arguments, "provider arguments", 0)
}

fn validate_provider_json_value(
    value: &serde_json::Value,
    label: &str,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err(format!("{label} exceed maximum nesting depth"));
    }
    match value {
        serde_json::Value::String(text) => validate_provider_text_content(text, label),
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

fn validate_provider_json_key(key: &str) -> Result<(), String> {
    if key
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err("provider argument key must not contain NUL/control characters".to_string());
    }
    Ok(())
}

fn validate_optional_provider_text(
    value: &Option<String>,
    label: &str,
    max_len: usize,
) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.len() > max_len {
        return Err(format!("{label} exceeds {max_len} bytes"));
    }
    validate_provider_text_content(value, label)
}

fn validate_provider_text_content(value: &str, label: &str) -> Result<(), String> {
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(format!("{label} must not contain NUL/control characters"));
    }
    let lower = value.to_ascii_lowercase();
    for forbidden in SENSITIVE_SUMMARY_MARKERS {
        if lower.contains(forbidden) {
            return Err(format!(
                "{label} must not contain sensitive marker `{forbidden}`"
            ));
        }
    }
    if lower
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(|token| token.starts_with("sk-"))
    {
        return Err(format!("{label} must not contain API-key-like tokens"));
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
    use ironclaw_host_api::CapabilityId;

    use super::{
        ProviderToolCallReferenceEnvelope, ToolResultReferenceEnvelope, ToolResultSafeSummary,
    };

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

    #[test]
    fn provider_reference_validation_rejects_sensitive_arguments_and_text() {
        let mut envelope = provider_reference();
        envelope.arguments = serde_json::json!({"api_key":"sk-live-secret"});
        assert!(envelope.validate().is_err());

        let mut envelope = provider_reference();
        envelope.response_reasoning = Some("raw provider error included a stack trace".to_string());
        assert!(envelope.validate().is_err());
    }

    #[test]
    fn provider_reference_validation_accepts_safe_zero_arg_metadata() {
        let mut envelope = provider_reference();
        envelope.arguments = serde_json::json!({});
        envelope.validate().expect("safe provider metadata");
    }

    fn provider_reference() -> ProviderToolCallReferenceEnvelope {
        ProviderToolCallReferenceEnvelope {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            provider_turn_id: "turn_1".to_string(),
            provider_call_id: "call_1".to_string(),
            provider_tool_name: "demo__echo".to_string(),
            capability_id: CapabilityId::new("demo.echo").expect("capability id"),
            arguments: serde_json::json!({"message":"hello"}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }
}
