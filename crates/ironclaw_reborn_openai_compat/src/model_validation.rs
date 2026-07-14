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

/// The OpenAI-compatible alias every client may send to mean "use the server's
/// active/default model" rather than naming a concrete one. The models listing
/// advertises it, so it is not a routable model id.
const DEFAULT_MODEL_ALIAS: &str = "default";

/// Map a validated client `model` string to an optional caller-requested model
/// *hint* for turn routing.
///
/// Returns `None` for the [`DEFAULT_MODEL_ALIAS`] sentinel (and defensively for
/// empty), so a client asking for the server default does not pin an advisory
/// route to the non-routable `"default"` id — which the model gateway rejects as
/// non-concrete and which would fail route resolution on routed hosts. A
/// concrete model name is forwarded as `Some`.
pub(crate) fn requested_model_hint(model: &str) -> Option<String> {
    let trimmed = model.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(DEFAULT_MODEL_ALIAS) {
        return None;
    }
    Some(trimmed.to_string())
}

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

    #[test]
    fn requested_model_hint_drops_default_sentinel() {
        assert_eq!(requested_model_hint("default"), None);
        assert_eq!(requested_model_hint("DEFAULT"), None);
        assert_eq!(requested_model_hint("Default"), None);
        assert_eq!(requested_model_hint(""), None);
        assert_eq!(requested_model_hint("   "), None);
    }

    #[test]
    fn requested_model_hint_forwards_concrete_model() {
        assert_eq!(requested_model_hint("gpt-4o"), Some("gpt-4o".to_string()));
        assert_eq!(
            requested_model_hint("anthropic/claude-opus-4"),
            Some("anthropic/claude-opus-4".to_string())
        );
    }
}
