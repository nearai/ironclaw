use crate::config::helpers::optional_env;
use crate::error::ConfigError;
use crate::workspace::layer::MemoryLayer;

/// Workspace-level configuration (memory layers, read scopes).
///
/// Parsed from environment variables. Lives outside of `GatewayConfig`
/// so that non-gateway channels can eventually use the same settings.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceConfig {
    /// Memory layer definitions (JSON in `MEMORY_LAYERS` env var, or defaults).
    pub memory_layers: Vec<MemoryLayer>,
    /// Additional user scopes for workspace reads.
    ///
    /// When set, the workspace can read (search, read, list) from these
    /// additional user scopes while writes remain isolated to the primary
    /// `user_id`. Parsed from `WORKSPACE_READ_SCOPES` (comma-separated).
    pub read_scopes: Vec<String>,
}

impl WorkspaceConfig {
    /// Resolve workspace config from environment variables.
    ///
    /// `user_id` is used to derive default memory layers when `MEMORY_LAYERS`
    /// is not set.
    pub fn resolve(user_id: &str) -> Result<Self, ConfigError> {
        // --- Memory layers ---
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
                    message: format!("layer name '{}' exceeds 64 characters", layer.name),
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
            if !layer
                .scope
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(ConfigError::InvalidValue {
                    key: "MEMORY_LAYERS".to_string(),
                    message: format!(
                        "layer '{}' scope '{}' contains invalid characters \
                         (allowed: a-z, A-Z, 0-9, _, -)",
                        layer.name, layer.scope
                    ),
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

        // --- Read scopes ---
        let read_scopes: Vec<String> = optional_env("WORKSPACE_READ_SCOPES")?
            .map(|s| {
                s.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        for scope in &read_scopes {
            if scope.len() > 128 {
                let prefix: String = scope.chars().take(32).collect();
                return Err(ConfigError::InvalidValue {
                    key: "WORKSPACE_READ_SCOPES".to_string(),
                    message: format!("scope '{prefix}...' exceeds 128 characters"),
                });
            }
            if !scope
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(ConfigError::InvalidValue {
                    key: "WORKSPACE_READ_SCOPES".to_string(),
                    message: format!(
                        "scope '{}' contains invalid characters \
                         (allowed: a-z, A-Z, 0-9, _, -)",
                        scope
                    ),
                });
            }
        }

        Ok(Self {
            memory_layers,
            read_scopes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    fn with_env(key: &str, val: Option<&str>, f: impl FnOnce()) {
        let _guard = lock_env();
        let prev = std::env::var(key).ok();
        match val {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
        f();
        match prev {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    #[test]
    fn valid_json_parses_correctly() {
        let json = r#"[{"name":"private","scope":"alice","writable":true,"sensitivity":"private"},{"name":"shared","scope":"shared","writable":true,"sensitivity":"shared"}]"#;
        with_env("MEMORY_LAYERS", Some(json), || {
            let config = WorkspaceConfig::resolve("alice").expect("should parse");
            assert_eq!(config.memory_layers.len(), 2);
            assert_eq!(config.memory_layers[0].name, "private");
            assert_eq!(config.memory_layers[1].name, "shared");
        });
    }

    #[test]
    fn invalid_json_returns_error() {
        with_env("MEMORY_LAYERS", Some("not json"), || {
            let result = WorkspaceConfig::resolve("alice");
            assert!(result.is_err(), "invalid JSON should fail");
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("valid JSON"),
                "error should mention JSON: {err}"
            );
        });
    }

    #[test]
    fn empty_layer_name_returns_error() {
        let json = r#"[{"name":"","scope":"alice"}]"#;
        with_env("MEMORY_LAYERS", Some(json), || {
            let result = WorkspaceConfig::resolve("alice");
            assert!(result.is_err(), "empty layer name should fail");
            let err = result.unwrap_err().to_string();
            assert!(err.contains("empty"), "error should mention empty: {err}");
        });
    }

    #[test]
    fn layer_name_exceeding_64_chars_returns_error() {
        let long_name = "a".repeat(65);
        let json = format!(r#"[{{"name":"{long_name}","scope":"alice"}}]"#);
        with_env("MEMORY_LAYERS", Some(&json), || {
            let result = WorkspaceConfig::resolve("alice");
            assert!(result.is_err(), "long layer name should fail");
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("exceeds 64"),
                "error should mention 64 chars: {err}"
            );
        });
    }

    #[test]
    fn layer_name_with_invalid_chars_returns_error() {
        for bad_name in ["has space", "has@at", "has.dot", "has/slash"] {
            let json = format!(r#"[{{"name":"{bad_name}","scope":"alice"}}]"#);
            with_env("MEMORY_LAYERS", Some(&json), || {
                let result = WorkspaceConfig::resolve("alice");
                assert!(
                    result.is_err(),
                    "layer name '{bad_name}' should fail validation"
                );
                let err = result.unwrap_err().to_string();
                assert!(
                    err.contains("invalid characters"),
                    "error for '{bad_name}' should mention invalid characters: {err}"
                );
            });
        }
    }

    #[test]
    fn empty_scope_returns_error() {
        let json = r#"[{"name":"private","scope":""}]"#;
        with_env("MEMORY_LAYERS", Some(json), || {
            let result = WorkspaceConfig::resolve("alice");
            assert!(result.is_err(), "empty scope should fail");
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("empty scope"),
                "error should mention empty scope: {err}"
            );
        });
    }

    #[test]
    fn duplicate_layer_names_returns_error() {
        let json = r#"[{"name":"private","scope":"alice"},{"name":"private","scope":"bob"}]"#;
        with_env("MEMORY_LAYERS", Some(json), || {
            let result = WorkspaceConfig::resolve("alice");
            assert!(result.is_err(), "duplicate names should fail");
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("duplicate"),
                "error should mention duplicate: {err}"
            );
        });
    }

    #[test]
    fn missing_env_defaults_to_single_private_layer() {
        with_env("MEMORY_LAYERS", None, || {
            let config = WorkspaceConfig::resolve("alice").expect("should default");
            assert_eq!(config.memory_layers.len(), 1);
            assert_eq!(config.memory_layers[0].name, "private");
            assert_eq!(config.memory_layers[0].scope, "alice");
            assert!(config.memory_layers[0].writable);
        });
    }
}
