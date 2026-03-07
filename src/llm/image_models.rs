//! Detection of image generation models across inference providers.

/// Check if a model name indicates image generation capability.
///
/// Detects models like:
/// - FLUX (Black Forest Labs): `flux`, `flux.2`, `flux-pro`, etc.
/// - DALL-E (OpenAI): `dall-e-2`, `dall-e-3`, etc.
/// - Stable Diffusion: `stable-diffusion`, `sdxl`, etc.
/// - Imagen (Google): `imagen`, `imagen-2`, etc.
/// - Other generation models
pub fn is_image_generation_model(model: &str) -> bool {
    let model_lower = model.to_lowercase();

    // FLUX models
    if model_lower.contains("flux") {
        return true;
    }

    // DALL-E models
    if model_lower.contains("dall-e") || model_lower.contains("dalle") {
        return true;
    }

    // Stable Diffusion models
    if model_lower.contains("stable-diffusion")
        || model_lower.contains("sdxl")
        || model_lower.contains("stability")
    {
        return true;
    }

    // Imagen models
    if model_lower.contains("imagen") {
        return true;
    }

    // Midjourney (if exposed via API)
    if model_lower.contains("midjourney") {
        return true;
    }

    // Replicate FLUX via API
    if model_lower.contains("black-forest-labs") || model_lower.contains("lucataco") {
        return true;
    }

    false
}

/// Check if any model in a list is an image generation model.
pub fn has_image_generation_model(models: &[String]) -> bool {
    models.iter().any(|m| is_image_generation_model(m))
}

/// Suggest the best image generation model from available models.
///
/// Priority: FLUX > DALL-E > others
pub fn suggest_image_model(models: &[String]) -> Option<String> {
    // Prefer FLUX
    if let Some(flux) = models.iter().find(|m| m.to_lowercase().contains("flux")) {
        return Some(flux.clone());
    }

    // Then DALL-E
    if let Some(dalle) = models
        .iter()
        .find(|m| m.to_lowercase().contains("dall-e") || m.to_lowercase().contains("dalle"))
    {
        return Some(dalle.clone());
    }

    // Then any other image model
    models
        .iter()
        .find(|m| is_image_generation_model(m))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flux_detection() {
        assert!(is_image_generation_model(
            "black-forest-labs/FLUX.2-klein-4B"
        ));
        assert!(is_image_generation_model("flux"));
        assert!(is_image_generation_model("flux-pro"));
    }

    #[test]
    fn test_dalle_detection() {
        assert!(is_image_generation_model("dall-e-3"));
        assert!(is_image_generation_model("dall-e-2"));
        assert!(is_image_generation_model("dalle-3"));
    }

    #[test]
    fn test_stable_diffusion_detection() {
        assert!(is_image_generation_model("stable-diffusion-3"));
        assert!(is_image_generation_model("sdxl"));
    }

    #[test]
    fn test_imagen_detection() {
        assert!(is_image_generation_model("imagen"));
        assert!(is_image_generation_model("imagen-3"));
    }

    #[test]
    fn test_non_image_models() {
        assert!(!is_image_generation_model("claude-3-5-sonnet"));
        assert!(!is_image_generation_model("gpt-4"));
        assert!(!is_image_generation_model("gemini-pro"));
    }

    #[test]
    fn test_suggest_image_model() {
        let models = vec![
            "gpt-4".to_string(),
            "black-forest-labs/FLUX.2-klein-4B".to_string(),
            "dall-e-3".to_string(),
        ];

        // Should prefer FLUX
        assert_eq!(
            suggest_image_model(&models),
            Some("black-forest-labs/FLUX.2-klein-4B".to_string())
        );
    }

    #[test]
    fn test_suggest_dalle_when_no_flux() {
        let models = vec!["gpt-4".to_string(), "dall-e-3".to_string()];

        assert_eq!(suggest_image_model(&models), Some("dall-e-3".to_string()));
    }
}
