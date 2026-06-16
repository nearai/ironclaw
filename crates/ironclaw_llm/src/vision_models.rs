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

/// Choose the model to back the vision (image-analysis) tool, given the
/// configured chat model and the set of models the provider reports as
/// image-capable via its authoritative modality metadata (see
/// [`crate::fetch_image_capable_models`]).
///
/// Unlike [`suggest_vision_model`], this never relies on name heuristics to
/// decide *capability* — `image_capable` is already the verified vision set.
/// Name matching is used only to rank *quality* among that set. Policy:
///
/// 1. Prefer the configured model if it is itself image-capable — keeps the
///    vision tool on the same model as the chat loop with no surprise routing.
/// 2. Otherwise pick the highest-quality available model by family preference
///    (Claude > GPT > Gemini > Qwen > anything else), matched case-insensitively
///    as a substring so provider-prefixed ids (`anthropic/claude-sonnet-4-6`)
///    and bare ids both work.
/// 3. Otherwise fall back to the first image-capable model offered.
///
/// Returns `None` only when `image_capable` is empty, leaving the caller to use
/// its own name-heuristic fallback.
pub fn choose_vision_model<'a>(
    configured: &'a str,
    image_capable: &'a [String],
) -> Option<&'a str> {
    if image_capable.is_empty() {
        return None;
    }
    let cfg = configured.to_lowercase();
    if image_capable.iter().any(|m| m.to_lowercase() == cfg) {
        return Some(configured);
    }
    const FAMILY_PREFERENCE: &[&str] = &["claude", "gpt-5", "gpt-4", "gemini", "qwen"];
    for family in FAMILY_PREFERENCE {
        if let Some(model) = image_capable
            .iter()
            .find(|m| m.to_lowercase().contains(family))
        {
            return Some(model.as_str());
        }
    }
    image_capable.first().map(String::as_str)
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

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn choose_prefers_configured_model_when_image_capable() {
        let capable = strings(&["anthropic/claude-sonnet-4-6", "openai/gpt-4.1"]);
        assert_eq!(
            choose_vision_model("openai/gpt-4.1", &capable),
            Some("openai/gpt-4.1")
        );
    }

    #[test]
    fn choose_falls_back_to_best_family_when_configured_is_text_only() {
        // Configured model (deepseek) is not in the image-capable set; Claude
        // outranks the GPT/Gemini alternatives.
        let capable = strings(&[
            "openai/gpt-4.1",
            "anthropic/claude-sonnet-4-6",
            "google/gemini-2.5-pro",
        ]);
        assert_eq!(
            choose_vision_model("deepseek/deepseek-v3.2", &capable),
            Some("anthropic/claude-sonnet-4-6")
        );
    }

    #[test]
    fn choose_returns_first_capable_when_no_family_matches() {
        // A vision model the family-preference list does not name (e.g. a
        // VL model) is still returned rather than dropped.
        let capable = strings(&["Qwen/Qwen3-VL-30B-A3B-Instruct"]);
        assert_eq!(
            choose_vision_model("deepseek/deepseek-v3.2", &capable),
            Some("Qwen/Qwen3-VL-30B-A3B-Instruct")
        );
    }

    #[test]
    fn choose_returns_none_for_empty_capable_set() {
        assert_eq!(choose_vision_model("anything", &[]), None);
    }
}
