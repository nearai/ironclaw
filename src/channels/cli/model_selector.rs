//! Model selector overlay for switching LLM models.

/// Request to show the model selector.
#[derive(Debug, Clone)]
pub struct ModelSelectorRequest {
    /// Currently selected model.
    pub current_model: String,
    /// Available models to choose from.
    pub available_models: Vec<String>,
}

/// Model selector overlay state.
#[derive(Debug, Clone)]
pub struct ModelSelectorOverlay {
    /// The request that triggered this overlay.
    pub request: ModelSelectorRequest,
    /// Currently highlighted index.
    pub selection_index: usize,
}

impl ModelSelectorOverlay {
    /// Create a new model selector overlay.
    pub fn new(request: ModelSelectorRequest) -> Self {
        // Find the current model in the list, default to 0
        let selection_index = request
            .available_models
            .iter()
            .position(|m| m == &request.current_model)
            .unwrap_or(0);

        Self {
            request,
            selection_index,
        }
    }

    /// Get the list of available models.
    pub fn models(&self) -> &[String] {
        &self.request.available_models
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        let len = self.request.available_models.len();
        if len == 0 {
            return;
        }
        if self.selection_index > 0 {
            self.selection_index -= 1;
        } else {
            // Wrap to bottom
            self.selection_index = len - 1;
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        let len = self.request.available_models.len();
        if len == 0 {
            return;
        }
        if self.selection_index < len - 1 {
            self.selection_index += 1;
        } else {
            // Wrap to top
            self.selection_index = 0;
        }
    }

    /// Get the currently selected model name.
    pub fn selected_model(&self) -> Option<&str> {
        self.request
            .available_models
            .get(self.selection_index)
            .map(|s| s.as_str())
    }

    /// Check if the selection is the current model.
    pub fn is_current(&self) -> bool {
        self.selected_model() == Some(&self.request.current_model)
    }

    /// Format a model name for display (shorten long names).
    pub fn format_model_name(model: &str) -> String {
        // Shorten fireworks model names
        if let Some(rest) = model.strip_prefix("fireworks::accounts/fireworks/models/") {
            return format!("fireworks/{}", rest);
        }
        // Shorten other long prefixes
        if let Some(rest) = model.strip_prefix("accounts/") {
            return rest.to_string();
        }
        model.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_selector_navigation() {
        let request = ModelSelectorRequest {
            current_model: "gpt-4o".to_string(),
            available_models: vec![
                "claude-3-5-sonnet".to_string(),
                "gpt-4o".to_string(),
                "gpt-4o-mini".to_string(),
            ],
        };
        let mut overlay = ModelSelectorOverlay::new(request);

        // Should start at gpt-4o index (1)
        assert_eq!(overlay.selected_model(), Some("gpt-4o"));

        // Navigate down
        overlay.select_next();
        assert_eq!(overlay.selected_model(), Some("gpt-4o-mini"));

        // Navigate down (wrap)
        overlay.select_next();
        assert_eq!(overlay.selected_model(), Some("claude-3-5-sonnet"));

        // Navigate up
        overlay.select_prev();
        assert_eq!(overlay.selected_model(), Some("gpt-4o-mini"));
    }

    #[test]
    fn test_format_model_name() {
        assert_eq!(
            ModelSelectorOverlay::format_model_name("claude-3-5-sonnet-20241022"),
            "claude-3-5-sonnet-20241022"
        );
        assert_eq!(
            ModelSelectorOverlay::format_model_name(
                "fireworks::accounts/fireworks/models/llama-v3p1-405b-instruct"
            ),
            "fireworks/llama-v3p1-405b-instruct"
        );
    }

    #[test]
    fn test_empty_models() {
        let request = ModelSelectorRequest {
            current_model: "unknown".to_string(),
            available_models: vec![],
        };
        let mut overlay = ModelSelectorOverlay::new(request);
        assert_eq!(overlay.selected_model(), None);

        // Should not panic
        overlay.select_next();
        overlay.select_prev();
    }
}
