//! Shared primitives for parsing provider tool-call responses.
//!
//! Each provider file (Layer 1) owns its wire-dialect mapping. This module
//! provides shared sub-step primitives every provider can call:
//! argument-JSON parsing with explicit error policy, and ordered
//! reasoning-field probe.
//!
//! Design influenced by shared JSON-repair utilities and ordered
//! reasoning-field probe patterns common in OpenAI-compatible provider
//! implementations.
//!
//! Layer 3 (NormalizingProvider) handles shape invariants that operate on
//! the decoded ToolCompletionResponse. This module handles sub-steps that
//! need raw wire bytes (provider-internal).
//!
//! ## Migration recipe (per follow-up plan, one provider per PR)
//!
//! 1. Replace `unwrap_or(Value::Object(default))` with
//!    `parse_tool_call_args_lossy(raw)`. Populates
//!    `ToolCall.arguments_parse_error`.
//! 2. If the provider extracts reasoning from a JSON wire body, switch to
//!    `probe_reasoning_field(json, &CANDIDATE_FIELDS)` with the provider's
//!    candidate list as a const slice (pattern parallels
//!    `reasoning_models.rs`'s flat-slice convention).
//! 3. Add provider-specific `_finish_reason_*` tests if the provider
//!    decodes finish_reason from a string (Class B coverage). Class A
//!    shape invariants are owned by `NormalizingProvider`.

use serde_json::Value;

/// Parse error returned by [`parse_tool_call_args`].
#[derive(Debug, thiserror::Error)]
#[error("{reason}")]
pub(crate) struct ArgsParseError {
    pub reason: String,
}

/// Fail-loud variant. Returns `Err` on malformed JSON.
///
/// Policy: any valid JSON value (object, array, number, string, null) is
/// accepted — `Ok` is returned. Callers decide whether a non-object shape
/// should be rejected. Only a parse failure produces `Err`.
pub(crate) fn parse_tool_call_args(raw: &str) -> Result<Value, ArgsParseError> {
    serde_json::from_str(raw).map_err(|e| ArgsParseError {
        reason: format!("failed to parse tool-call arguments JSON: {e}"),
    })
}

/// Migration helper: silent-`{}` fallback with error capture.
///
/// Returns `(parsed, None)` on success and `(empty object, Some(reason))` on
/// failure. An empty string is treated as a parse failure.
///
/// **Doc-only deprecation notice:** This function is a migration bridge.
/// It preserves the current silent-`{}` behavior at existing call sites
/// while populating `ToolCall.arguments_parse_error` for future gateway
/// surfacing. Remove once every provider migrates to [`parse_tool_call_args`]
/// AND the gateway reads the `arguments_parse_error` field.
/// Do not rely on this function in new code.
pub(crate) fn parse_tool_call_args_lossy(raw: &str) -> (Value, Option<String>) {
    if raw.is_empty() {
        return (
            Value::Object(serde_json::Map::new()),
            Some("empty arguments string".to_owned()),
        );
    }
    match parse_tool_call_args(raw) {
        Ok(v) => (v, None),
        Err(ArgsParseError { reason }) => (Value::Object(serde_json::Map::new()), Some(reason)),
    }
}

/// Ordered probe over candidate reasoning-field names.
///
/// Iterates `fields` in order; returns a borrowed reference to the first value
/// that is a non-empty string. Skips missing keys, non-string values, and empty
/// strings. Returns `None` for non-object inputs or when no candidate matches.
pub(crate) fn probe_reasoning_field<'a>(json: &'a Value, fields: &[&str]) -> Option<&'a str> {
    let obj = json.as_object()?;
    for &field in fields {
        if let Some(Value::String(s)) = obj.get(field)
            && !s.trim().is_empty()
        {
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_args_valid_object() {
        let result = parse_tool_call_args(r#"{"x": 1}"#).expect("should parse");
        assert_eq!(result["x"], 1);
    }

    #[test]
    fn parse_args_valid_array_accepted() {
        let result = parse_tool_call_args(r#"[1, 2, 3]"#).expect("arrays are valid JSON");
        assert!(result.is_array());
    }

    #[test]
    fn parse_args_invalid_json_returns_err() {
        let err = parse_tool_call_args(r#"{not valid"#).expect_err("should fail");
        assert!(
            err.reason
                .starts_with("failed to parse tool-call arguments JSON: ")
        );
    }

    #[test]
    fn parse_args_lossy_invalid_json_returns_empty_object_and_error() {
        let (val, err) = parse_tool_call_args_lossy(r#"{not valid"#);
        assert!(val.as_object().expect("should be object").is_empty());
        let reason = err.expect("should have error");
        assert!(reason.starts_with("failed to parse tool-call arguments JSON: "));
    }

    #[test]
    fn parse_args_lossy_empty_string() {
        let (val, err) = parse_tool_call_args_lossy("");
        assert!(val.as_object().expect("should be object").is_empty());
        assert_eq!(
            err.expect("error should be present"),
            "empty arguments string"
        );
    }

    #[test]
    fn parse_args_lossy_valid_returns_no_error() {
        let (val, err) = parse_tool_call_args_lossy(r#"{"a": 2}"#);
        assert_eq!(val["a"], 2);
        assert!(err.is_none());
    }

    #[test]
    fn probe_reasoning_field_first_present_wins() {
        let json = json!({"reasoning": "first", "reasoning_content": "second"});

        let result = probe_reasoning_field(&json, &["reasoning", "reasoning_content"]);
        assert_eq!(result, Some("first"));

        let result = probe_reasoning_field(&json, &["reasoning_content", "reasoning"]);
        assert_eq!(result, Some("second"));
    }

    #[test]
    fn probe_reasoning_field_skips_empty_strings_and_misses() {
        // Empty string is skipped; falls through to next candidate.
        let json = json!({"reasoning": "", "reasoning_content": "x"});
        let result = probe_reasoning_field(&json, &["reasoning", "reasoning_content"]);
        assert_eq!(result, Some("x"));

        // Empty object → no candidates match → None.
        let empty_json = json!({});
        let result = probe_reasoning_field(&empty_json, &["reasoning", "reasoning_content"]);
        assert!(result.is_none());

        // Non-object input → None.
        let null_json = json!(null);
        let result = probe_reasoning_field(&null_json, &["reasoning"]);
        assert!(result.is_none());
    }

    #[test]
    fn parse_args_empty_string_returns_err() {
        let err = parse_tool_call_args("").expect_err("empty string is not valid JSON");
        assert!(
            err.reason
                .starts_with("failed to parse tool-call arguments JSON: ")
        );
    }

    #[test]
    fn parse_args_valid_primitives_accepted() {
        let result = parse_tool_call_args("null").expect("null is valid JSON");
        assert!(result.is_null());

        let result = parse_tool_call_args("42").expect("number is valid JSON");
        assert_eq!(result, 42);

        let result = parse_tool_call_args(r#""hello""#).expect("string is valid JSON");
        assert_eq!(result, "hello");
    }

    #[test]
    fn probe_reasoning_field_skips_non_string_values() {
        // Number at first candidate → skip, fall through.
        let json = json!({"reasoning": 42, "reasoning_content": "actual"});
        let result = probe_reasoning_field(&json, &["reasoning", "reasoning_content"]);
        assert_eq!(result, Some("actual"));

        // Object / bool / array at first candidate also skipped.
        let json = json!({"reasoning": {"x": 1}, "reasoning_content": "actual"});
        let result = probe_reasoning_field(&json, &["reasoning", "reasoning_content"]);
        assert_eq!(result, Some("actual"));
    }

    #[test]
    fn probe_reasoning_field_empty_fields_returns_none() {
        let json = json!({"reasoning": "x"});
        let result = probe_reasoning_field(&json, &[]);
        assert!(result.is_none());
    }

    #[test]
    fn probe_reasoning_field_skips_whitespace_only_values() {
        let json = json!({"reasoning": "   \n\t", "reasoning_content": "actual"});
        let result = probe_reasoning_field(&json, &["reasoning", "reasoning_content"]);
        assert_eq!(result, Some("actual"));

        let json = json!({"reasoning": "   "});
        let result = probe_reasoning_field(&json, &["reasoning"]);
        assert_eq!(result, None);
    }

    #[test]
    fn parse_args_lossy_non_object_passthrough() {
        let result = parse_tool_call_args_lossy("null");
        assert_eq!(result.0, Value::Null);
        assert!(result.1.is_none());

        let result = parse_tool_call_args_lossy("42");
        assert_eq!(result.0, 42);
        assert!(result.1.is_none());

        let result = parse_tool_call_args_lossy("[1,2,3]");
        assert_eq!(result.0, json!([1, 2, 3]));
        assert!(result.1.is_none());
    }
}
