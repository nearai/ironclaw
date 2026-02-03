//! User settings persistence.
//!
//! Stores user preferences like selected model in ~/.near-agent/settings.json.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// User settings persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Currently selected model.
    #[serde(default)]
    pub selected_model: Option<String>,
}

impl Settings {
    /// Get the default settings file path (~/.near-agent/settings.json).
    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".near-agent")
            .join("settings.json")
    }

    /// Load settings from disk, returning default if not found.
    pub fn load() -> Self {
        Self::load_from(&Self::default_path())
    }

    /// Load settings from a specific path.
    pub fn load_from(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save settings to disk.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::default_path())
    }

    /// Save settings to a specific path.
    pub fn save_to(&self, path: &PathBuf) -> std::io::Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        std::fs::write(path, json)
    }

    /// Get the selected model, falling back to the provided default.
    pub fn model_or(&self, default: &str) -> String {
        self.selected_model
            .clone()
            .unwrap_or_else(|| default.to_string())
    }

    /// Set the selected model and save.
    pub fn set_model(&mut self, model: &str) -> std::io::Result<()> {
        self.selected_model = Some(model.to_string());
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_settings_save_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let settings = Settings {
            selected_model: Some("claude-3-5-sonnet-20241022".to_string()),
        };

        settings.save_to(&path).unwrap();

        let loaded = Settings::load_from(&path);
        assert_eq!(
            loaded.selected_model,
            Some("claude-3-5-sonnet-20241022".to_string())
        );
    }

    #[test]
    fn test_model_or_default() {
        let settings = Settings::default();
        assert_eq!(
            settings.model_or("default-model"),
            "default-model".to_string()
        );

        let settings = Settings {
            selected_model: Some("my-model".to_string()),
        };
        assert_eq!(settings.model_or("default-model"), "my-model".to_string());
    }
}
