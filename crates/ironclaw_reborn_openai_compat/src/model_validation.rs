//! Shared validation for the client-supplied `model` field.
//!
//! Reborn carries the requested `model` string through to the projection reader
//! as a composition/policy hint and echoes it back in the response. Bounding it
//! at the request-parse boundary keeps an unbounded, whitespace-padded, or
//! control-character-laden value out of projection requests, response bodies,
//! and logs. Mirrors the v1 OpenAI-compatible proxy bound and the
//! bounded-resources rule (#2673: 256-byte cap on model strings).

use crate::OpenAiCompatHttpError;

/// Maximum accepted `model` string length, in bytes.
pub(crate) const MAX_MODEL_NAME_BYTES: usize = 256;

/// Validate the client-supplied `model` string before it is carried as a
/// projection/policy hint.
///
/// Returns a sanitized `400` naming the `model` param on any violation: empty,
/// leading/trailing whitespace, over the byte cap, or containing control
/// characters. Length is measured in bytes to match the cap's intent (an upper
/// bound on stored/forwarded size, not a user-facing character count).
pub(crate) fn validate_model_name(model: &str) -> Result<(), OpenAiCompatHttpError> {
    if model.is_empty()
        || model.trim() != model
        || model.len() > MAX_MODEL_NAME_BYTES
        || model.chars().any(char::is_control)
    {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "model".to_string(),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_rejected(model: &str) {
        let err = validate_model_name(model).unwrap_err();
        assert_eq!(err.status_code(), 400, "model {model:?} must be rejected");
        assert_eq!(
            err.body().error.param(),
            Some("model"),
            "rejection must name the model param"
        );
    }

    #[test]
    fn accepts_normal_model_name() {
        assert!(validate_model_name("gpt-4").is_ok());
        assert!(validate_model_name("reborn").is_ok());
        assert!(validate_model_name("anthropic/claude-opus-4").is_ok());
    }

    #[test]
    fn rejects_empty_model() {
        assert_rejected("");
    }

    #[test]
    fn rejects_surrounding_whitespace() {
        assert_rejected(" gpt-4");
        assert_rejected("gpt-4 ");
        assert_rejected("\tgpt-4\n");
    }

    #[test]
    fn rejects_control_characters() {
        assert_rejected("gpt\u{0000}4");
        assert_rejected("gpt\u{0007}4");
    }

    #[test]
    fn rejects_model_over_byte_cap() {
        let too_long = "m".repeat(MAX_MODEL_NAME_BYTES + 1);
        assert_rejected(&too_long);
    }

    #[test]
    fn accepts_model_at_byte_cap() {
        let at_cap = "m".repeat(MAX_MODEL_NAME_BYTES);
        assert!(validate_model_name(&at_cap).is_ok());
    }
}
