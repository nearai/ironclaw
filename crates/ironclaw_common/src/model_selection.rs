//! Caller-requested model-selection helpers shared across API surfaces
//! (OpenAI-compatible API, WebChat v2).

/// The alias every client may send to mean "use the server's active/default
/// model" rather than naming a concrete one. The models listing advertises it,
/// so it is not a routable model id.
const DEFAULT_MODEL_ALIAS: &str = "default";

/// Map a client `model` string to an optional caller-requested model *hint* for
/// turn routing.
///
/// Returns `None` for the [`DEFAULT_MODEL_ALIAS`] sentinel (and, defensively,
/// for empty/whitespace), so a client asking for the server default does not
/// pin an advisory route to the non-routable `"default"` id — which the model
/// gateway rejects as non-concrete and which would fail route resolution on
/// routed hosts. A concrete model name is forwarded as `Some`.
pub fn requested_model_hint(model: &str) -> Option<String> {
    let trimmed = model.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(DEFAULT_MODEL_ALIAS) {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_default_sentinel_and_empty() {
        assert_eq!(requested_model_hint("default"), None);
        assert_eq!(requested_model_hint("DEFAULT"), None);
        assert_eq!(requested_model_hint("Default"), None);
        assert_eq!(requested_model_hint(""), None);
        assert_eq!(requested_model_hint("   "), None);
    }

    #[test]
    fn forwards_concrete_model() {
        assert_eq!(requested_model_hint("gpt-4o"), Some("gpt-4o".to_string()));
        assert_eq!(
            requested_model_hint("anthropic/claude-opus-4"),
            Some("anthropic/claude-opus-4".to_string())
        );
        // Surrounding whitespace is trimmed.
        assert_eq!(
            requested_model_hint("  claude-opus-4-6  "),
            Some("claude-opus-4-6".to_string())
        );
    }
}
