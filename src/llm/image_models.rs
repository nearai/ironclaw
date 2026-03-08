//! Image generation model detection utilities.

/// Known image generation model families.
const IMAGE_GEN_PATTERNS: &[&str] = &[
    "flux",
    "dall-e",
    "dalle",
    "stable-diffusion",
    "sdxl",
    "imagen",
    "midjourney",
    "ideogram",
    "playground",
];

/// Check if a model name indicates an image generation model.
pub fn is_image_generation_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    IMAGE_GEN_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Suggest the best image generation model from a list of available models.
///
/// Priority: FLUX > DALL-E > Stable Diffusion > others.
pub fn suggest_image_model(models: &[String]) -> Option<&str> {
    let priorities: &[&str] = &["flux", "dall-e", "dalle", "stable-diffusion", "sdxl", "imagen"];
    for priority in priorities {
        if let Some(model) = models.iter().find(|m| m.to_lowercase().contains(priority)) {
            return Some(model);
        }
    }
    // Fall back to any image gen model
    models.iter().find_map(|m| {
        if is_image_generation_model(m) {
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
    fn detects_flux_models() {
        assert!(is_image_generation_model("black-forest-labs/FLUX.1-schnell"));
        assert!(is_image_generation_model("flux-pro"));
    }

    #[test]
    fn detects_dalle_models() {
        assert!(is_image_generation_model("dall-e-3"));
        assert!(is_image_generation_model("dalle-3"));
    }

    #[test]
    fn rejects_non_image_models() {
        assert!(!is_image_generation_model("gpt-4o"));
        assert!(!is_image_generation_model("claude-3-sonnet"));
        assert!(!is_image_generation_model("llama-3.1-70b"));
    }

    #[test]
    fn suggests_flux_first() {
        let models = vec![
            "gpt-4o".to_string(),
            "dall-e-3".to_string(),
            "flux-pro".to_string(),
        ];
        assert_eq!(suggest_image_model(&models), Some("flux-pro"));
    }

    #[test]
    fn suggests_dalle_without_flux() {
        let models = vec!["gpt-4o".to_string(), "dall-e-3".to_string()];
        assert_eq!(suggest_image_model(&models), Some("dall-e-3"));
    }

    #[test]
    fn returns_none_when_no_image_models() {
        let models = vec!["gpt-4o".to_string(), "claude-3-sonnet".to_string()];
        assert_eq!(suggest_image_model(&models), None);
    }
}
