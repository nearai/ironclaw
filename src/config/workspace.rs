use crate::config::helpers::optional_env;
use crate::error::ConfigError;
use crate::workspace::layer::MemoryLayer;

/// Workspace memory configuration.
///
/// Controls memory layer definitions for privacy-aware writes.
/// Layers are parsed from the `MEMORY_LAYERS` env var (JSON array)
/// or default to a single private layer scoped to the gateway user.
#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    pub memory_layers: Vec<MemoryLayer>,
}

impl WorkspaceConfig {
    pub(crate) fn resolve(user_id: &str) -> Result<Self, ConfigError> {
        let memory_layers: Vec<MemoryLayer> = match optional_env("MEMORY_LAYERS")? {
            Some(json_str) => {
                serde_json::from_str(&json_str).map_err(|e| ConfigError::InvalidValue {
                    key: "MEMORY_LAYERS".to_string(),
                    message: format!("must be valid JSON array of layer objects: {e}"),
                })?
            }
            None => MemoryLayer::default_for_user(user_id),
        };

        // Validate layer names and scopes
        for layer in &memory_layers {
            if layer.name.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "MEMORY_LAYERS".to_string(),
                    message: "layer name must not be empty".to_string(),
                });
            }
            if layer.name.len() > 64 {
                return Err(ConfigError::InvalidValue {
                    key: "MEMORY_LAYERS".to_string(),
                    message: format!(
                        "layer name '{}' exceeds 64 characters",
                        layer.name
                    ),
                });
            }
            if !layer
                .name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                return Err(ConfigError::InvalidValue {
                    key: "MEMORY_LAYERS".to_string(),
                    message: format!(
                        "layer name '{}' contains invalid characters (only alphanumeric, _, - allowed)",
                        layer.name
                    ),
                });
            }
            if layer.scope.trim().is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "MEMORY_LAYERS".to_string(),
                    message: format!("layer '{}' has an empty scope", layer.name),
                });
            }
        }

        // Check for duplicate layer names
        {
            let mut seen = std::collections::HashSet::new();
            for layer in &memory_layers {
                if !seen.insert(&layer.name) {
                    return Err(ConfigError::InvalidValue {
                        key: "MEMORY_LAYERS".to_string(),
                        message: format!("duplicate layer name '{}'", layer.name),
                    });
                }
            }
        }

        Ok(Self { memory_layers })
    }
}
