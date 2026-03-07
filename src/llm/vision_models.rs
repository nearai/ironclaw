//! Detection of vision-capable models across inference providers.

/// Check if a model name indicates vision capability.
///
/// Detects models like:
/// - Claude (Anthropic): `claude-opus`, `claude-sonnet`, etc.
/// - GPT (OpenAI): `gpt-4-vision`, `gpt-4-turbo`, `gpt-4o`, etc.
/// - Gemini (Google): `gemini-pro-vision`, `gemini-2.0-flash`, etc.
/// - Llama (Meta): `llama-2-vision`, etc.
/// - Other vision-capable models
pub fn is_vision_model(model: &str) -> bool {
    let model_lower = model.to_lowercase();

    // Claude models (Anthropic)
    if model_lower.contains("claude") {
        return true;
    }

    // GPT-4 models with vision support
    if (model_lower.contains("gpt-4")
        || model_lower.contains("gpt-4o")
        || model_lower.contains("gpt-4-turbo")
        || model_lower.contains("gpt-4-vision"))
        && !model_lower.contains("gpt-4-mini")
    {
        return true;
    }

    // Gemini models
    if model_lower.contains("gemini") {
        return true;
    }

    // Llava and other vision models
    if model_lower.contains("llava")
        || model_lower.contains("vision")
        || model_lower.contains("multimodal")
    {
        return true;
    }

    false
}

/// Check if any model in a list is a vision-capable model.
pub fn has_vision_model(models: &[String]) -> bool {
    models.iter().any(|m| is_vision_model(m))
}

/// Suggest the best vision model from available models.
///
/// Priority: Claude > GPT-4 > Gemini > others
pub fn suggest_vision_model(models: &[String]) -> Option<String> {
    // Prefer Claude
    if let Some(claude) = models.iter().find(|m| m.to_lowercase().contains("claude")) {
        return Some(claude.clone());
    }

    // Then GPT-4
    if let Some(gpt4) = models
        .iter()
        .find(|m| m.to_lowercase().contains("gpt-4") && !m.to_lowercase().contains("gpt-4-mini"))
    {
        return Some(gpt4.clone());
    }

    // Then Gemini
    if let Some(gemini) = models.iter().find(|m| m.to_lowercase().contains("gemini")) {
        return Some(gemini.clone());
    }

    // Then any other vision model
    models.iter().find(|m| is_vision_model(m)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_detection() {
        assert!(is_vision_model("claude-opus-4-20250514"));
        assert!(is_vision_model("claude-sonnet-4-20250514"));
        assert!(is_vision_model("claude-haiku-3-5-sonnet"));
    }

    #[test]
    fn test_gpt4_detection() {
        assert!(is_vision_model("gpt-4-turbo"));
        assert!(is_vision_model("gpt-4o"));
        assert!(is_vision_model("gpt-4-vision"));
        assert!(is_vision_model("gpt-4-32k"));
    }

    #[test]
    fn test_gpt4_mini_not_vision() {
        assert!(!is_vision_model("gpt-4-mini"));
    }

    #[test]
    fn test_gemini_detection() {
        assert!(is_vision_model("gemini-pro-vision"));
        assert!(is_vision_model("gemini-2.0-flash"));
        assert!(is_vision_model("gemini-1.5-pro"));
    }

    #[test]
    fn test_llava_detection() {
        assert!(is_vision_model("llava-1.6"));
        assert!(is_vision_model("llava-v1-7b"));
    }

    #[test]
    fn test_multimodal_detection() {
        assert!(is_vision_model("my-multimodal-model"));
        assert!(is_vision_model("custom-vision-model"));
    }

    #[test]
    fn test_non_vision_models() {
        assert!(!is_vision_model("text-davinci-3"));
        assert!(!is_vision_model("llama-2-7b"));
        assert!(!is_vision_model("mistral-7b"));
    }

    #[test]
    fn test_suggest_vision_model() {
        let models = vec![
            "gpt-4-turbo".to_string(),
            "claude-opus-4-20250514".to_string(),
            "gemini-2.0-flash".to_string(),
        ];

        // Should prefer Claude
        assert_eq!(
            suggest_vision_model(&models),
            Some("claude-opus-4-20250514".to_string())
        );
    }

    #[test]
    fn test_suggest_gpt4_when_no_claude() {
        let models = vec!["gpt-4-turbo".to_string(), "gemini-2.0-flash".to_string()];

        assert_eq!(
            suggest_vision_model(&models),
            Some("gpt-4-turbo".to_string())
        );
    }

    #[test]
    fn test_suggest_gemini_when_no_claude_or_gpt4() {
        let models = vec!["gemini-2.0-flash".to_string(), "text-davinci-3".to_string()];

        assert_eq!(
            suggest_vision_model(&models),
            Some("gemini-2.0-flash".to_string())
        );
    }
}
