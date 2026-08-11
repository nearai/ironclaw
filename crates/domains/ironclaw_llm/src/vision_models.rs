//! Vision model detection utilities.

/// Known vision-capable model families.
///
/// Matching is substring-based (see [`is_vision_model`]), so the Anthropic
/// entries must cover both naming schemes that ship in production: the older
/// version-first form (`claude-3-5-sonnet`, `claude-4-...`) and the current
/// tier-first form (`claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5`,
/// `claude-fable-5`), including Bedrock-prefixed ids
/// (`anthropic.claude-opus-4-6-v1`). The substring `claude-4` does *not* appear
/// in `claude-opus-4-8`, so the tier-first patterns are load-bearing — without
/// them every current-generation Claude model is mis-classified as text-only
/// and its image attachments are silently dropped. Every Claude 3+ model is
/// vision-capable.
const VISION_PATTERNS: &[&str] = &[
    "claude-3",
    "claude-4",
    "claude-opus-",
    "claude-sonnet-",
    "claude-haiku-",
    "claude-fable-",
    "gpt-4o",
    "gpt-4-turbo",
    "gpt-4-vision",
    "gemini-pro-vision",
    "gemini-1.5",
    "gemini-2",
    "llava",
    "cogvlm",
    "internvl",
    "qwen-vl",
    "qwen2-vl",
    "pixtral",
];

/// Check if a model name indicates vision capabilities.
pub fn is_vision_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    VISION_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Suggest the best vision model from a list of available models.
///
/// Priority: Claude > GPT-4 > Gemini > others.
pub fn suggest_vision_model(models: &[String]) -> Option<&str> {
    let priorities: &[&str] = &[
        "claude-3",
        "claude-4",
        "gpt-4o",
        "gpt-4-turbo",
        "gpt-4-vision",
        "gemini",
        "llava",
        "pixtral",
    ];
    for priority in priorities {
        if let Some(model) = models.iter().find(|m| m.to_lowercase().contains(priority)) {
            return Some(model);
        }
    }
    models.iter().find_map(|m| {
        if is_vision_model(m) {
            Some(m.as_str())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_claude_vision() {
        assert!(is_vision_model("claude-3-5-sonnet-20241022"));
        assert!(is_vision_model("claude-3-opus"));
        assert!(is_vision_model("claude-4-sonnet"));
    }

    /// Regression guard: the tier-first ids that current Claude models actually
    /// ship under (and their Bedrock-prefixed forms) must classify as vision.
    /// The substring `claude-4` does not appear in `claude-opus-4-8`, so before
    /// the tier-first patterns existed these all fell through to text-only and
    /// silently dropped image attachments.
    #[test]
    fn detects_current_generation_claude_ids() {
        for model in [
            "claude-opus-4-8",
            "claude-sonnet-4-6",
            "claude-haiku-4-5-20251001",
            "claude-fable-5",
            "anthropic.claude-opus-4-6-v1",
            "us.anthropic.claude-sonnet-4-6-v1:0",
        ] {
            assert!(is_vision_model(model), "{model} should be vision-capable");
        }
    }

    #[test]
    fn detects_gpt4_vision() {
        assert!(is_vision_model("gpt-4o"));
        assert!(is_vision_model("gpt-4-turbo"));
        assert!(is_vision_model("gpt-4-vision-preview"));
    }

    #[test]
    fn detects_other_vision_models() {
        assert!(is_vision_model("gemini-1.5-pro"));
        assert!(is_vision_model("llava-v1.6"));
        assert!(is_vision_model("pixtral-12b"));
    }

    #[test]
    fn rejects_non_vision_models() {
        assert!(!is_vision_model("gpt-3.5-turbo"));
        assert!(!is_vision_model("llama-3.1-70b"));
        assert!(!is_vision_model("mistral-7b"));
    }

    #[test]
    fn suggests_claude_first() {
        let models = vec![
            "gpt-4o".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
        ];
        assert_eq!(
            suggest_vision_model(&models),
            Some("claude-3-5-sonnet-20241022")
        );
    }

    #[test]
    fn returns_none_when_no_vision_models() {
        let models = vec!["gpt-3.5-turbo".to_string(), "llama-3.1-70b".to_string()];
        assert_eq!(suggest_vision_model(&models), None);
    }
}
