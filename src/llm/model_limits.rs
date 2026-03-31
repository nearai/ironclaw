//! Best-effort context window inference for common model families.
//!
//! These values are used only for UI and local context-pressure heuristics
//! when the provider does not expose metadata directly. They are intentionally
//! conservative in scope and were last verified against official provider docs
//! on 2026-03-30.

/// Infer the total context window for a known model ID.
pub fn infer_context_length(model_id: &str) -> Option<u32> {
    let normalized = normalize_model_id(model_id);

    infer_openai_context_length(&normalized)
        .or_else(|| infer_anthropic_context_length(&normalized))
        .or_else(|| infer_gemini_context_length(&normalized))
}

fn normalize_model_id(model_id: &str) -> String {
    model_id
        .trim()
        .to_ascii_lowercase()
        .rsplit('/')
        .next()
        .unwrap_or(model_id)
        .split(':')
        .next()
        .unwrap_or(model_id)
        .to_string()
}

fn infer_openai_context_length(model_id: &str) -> Option<u32> {
    if model_id.starts_with("gpt-5") {
        if model_id.contains("-chat") {
            return Some(128_000);
        }
        return Some(400_000);
    }

    if model_id.starts_with("gpt-4.1") {
        return Some(1_047_576);
    }

    if model_id.starts_with("gpt-4o") || model_id.starts_with("chatgpt-4o") {
        return Some(128_000);
    }

    if model_id.starts_with("o1") || model_id.starts_with("o3") || model_id.starts_with("o4") {
        return Some(200_000);
    }

    None
}

fn infer_anthropic_context_length(model_id: &str) -> Option<u32> {
    if model_id.contains("claude-opus-4-6") || model_id.contains("claude-sonnet-4-6") {
        return Some(1_000_000);
    }

    if model_id.contains("claude") {
        return Some(200_000);
    }

    None
}

fn infer_gemini_context_length(model_id: &str) -> Option<u32> {
    match model_id {
        "gemini-2.5-pro"
        | "gemini-3-pro-preview"
        | "gemini-3.1-pro-preview"
        | "gemini-3.1-pro-preview-customtools" => Some(1_048_576),
        "gemini-2.5-flash"
        | "gemini-2.5-flash-lite"
        | "gemini-3-flash-preview"
        | "gemini-3.1-flash-lite-preview"
        | "gemini-1.5-pro"
        | "gemini-1.5-flash"
        | "gemini-2.0-flash" => Some(1_000_000),
        id if id.starts_with("gemini-") => Some(1_000_000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::infer_context_length;

    #[test]
    fn infers_openai_gpt5_family() {
        assert_eq!(infer_context_length("gpt-5-mini"), Some(400_000));
        assert_eq!(infer_context_length("gpt-5.1-codex-mini"), Some(400_000));
        assert_eq!(infer_context_length("gpt-5-chat-latest"), Some(128_000));
    }

    #[test]
    fn infers_openai_legacy_families() {
        assert_eq!(infer_context_length("gpt-4.1-mini"), Some(1_047_576));
        assert_eq!(infer_context_length("openai/gpt-4o"), Some(128_000));
        assert_eq!(infer_context_length("o4-mini"), Some(200_000));
    }

    #[test]
    fn infers_anthropic_families() {
        assert_eq!(infer_context_length("claude-sonnet-4-6"), Some(1_000_000));
        assert_eq!(
            infer_context_length("anthropic.claude-sonnet-4-6-v1:0"),
            Some(1_000_000)
        );
        assert_eq!(infer_context_length("claude-haiku-4-5"), Some(200_000));
    }

    #[test]
    fn infers_gemini_families() {
        assert_eq!(infer_context_length("gemini-2.5-pro"), Some(1_048_576));
        assert_eq!(
            infer_context_length("gemini-3-pro-preview"),
            Some(1_048_576)
        );
        assert_eq!(infer_context_length("gemini-2.5-flash"), Some(1_000_000));
    }

    #[test]
    fn returns_none_for_unknown_models() {
        assert_eq!(infer_context_length("my-local-model"), None);
    }
}
