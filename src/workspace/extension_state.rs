//! Workspace-backed extension and skill state persistence.
//!
//! Extension configs and skill manifests are stored as workspace documents
//! under `.system/extensions/` and `.system/skills/` with schema validation.
//! This module provides schemas and helpers for reading/writing extension
//! and skill state through workspace.
//!
//! The `ExtensionManager` and `SkillRegistry` continue to own runtime state
//! (active connections, in-memory caches). This module handles the durable
//! persistence layer.

use serde_json::{Value, json};

use crate::extensions::naming::canonicalize_extension_name;
use crate::workspace::document::system_paths;
use ironclaw_skills::validation::validate_skill_name;

/// Error returned when a name cannot be used to construct a workspace path.
#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("invalid extension name '{name}': {reason}")]
    InvalidExtensionName { name: String, reason: String },
    #[error("invalid skill name '{name}'")]
    InvalidSkillName { name: String },
}

/// JSON Schema for an installed extension's config.
pub fn extension_config_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "kind": {
                "type": "string",
                "enum": ["wasm_tool", "wasm_channel", "mcp_server", "channel_relay", "acp_agent"]
            },
            "version": { "type": "string" },
            "enabled": { "type": "boolean" },
            "source_url": { "type": "string" },
            "installed_at": { "type": "string" }
        },
        "required": ["name", "kind"]
    })
}

/// JSON Schema for the installed extensions registry.
pub fn extensions_registry_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "extensions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "kind": { "type": "string" }
                    },
                    "required": ["name"]
                }
            }
        },
        "required": ["extensions"]
    })
}

/// JSON Schema for an installed skill manifest.
pub fn skill_manifest_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "description": { "type": "string" },
            "version": { "type": "string" },
            "trust": {
                "type": "string",
                "enum": ["trusted", "installed"]
            },
            "source": { "type": "string" },
            "keywords": {
                "type": "array",
                "items": { "type": "string" }
            },
            "installed_at": { "type": "string" }
        },
        "required": ["name"]
    })
}

/// Build workspace path for an extension config.
///
/// The name is canonicalized via `canonicalize_extension_name`, which rejects
/// path separators, traversal sequences, NULs, and any non-snake-case
/// characters. This is the same validator used elsewhere in the extension
/// pipeline, so a name that fails here would also fail to install.
pub fn extension_config_path(name: &str) -> Result<String, PathError> {
    let canonical =
        canonicalize_extension_name(name).map_err(|e| PathError::InvalidExtensionName {
            name: name.to_string(),
            reason: e.to_string(),
        })?;
    Ok(format!(
        "{}{}/config.json",
        system_paths::EXTENSIONS_PREFIX,
        canonical
    ))
}

/// Build workspace path for an extension state document.
///
/// See `extension_config_path` for the name validation rules.
pub fn extension_state_path(name: &str) -> Result<String, PathError> {
    let canonical =
        canonicalize_extension_name(name).map_err(|e| PathError::InvalidExtensionName {
            name: name.to_string(),
            reason: e.to_string(),
        })?;
    Ok(format!(
        "{}{}/state.json",
        system_paths::EXTENSIONS_PREFIX,
        canonical
    ))
}

/// Build workspace path for the installed extensions registry.
pub fn extensions_registry_path() -> String {
    format!("{}installed.json", system_paths::EXTENSIONS_PREFIX)
}

/// Build workspace path for a skill manifest.
///
/// The name is checked against `validate_skill_name` (alphanumeric, hyphens,
/// underscores, dots; must start alphanumeric; max 64 chars). Names that would
/// otherwise escape `.system/skills/` are rejected.
pub fn skill_manifest_path(name: &str) -> Result<String, PathError> {
    if !validate_skill_name(name) {
        return Err(PathError::InvalidSkillName {
            name: name.to_string(),
        });
    }
    Ok(format!("{}{}.json", system_paths::SKILLS_PREFIX, name))
}

/// Build workspace path for the installed skills registry.
pub fn skills_registry_path() -> String {
    format!("{}installed.json", system_paths::SKILLS_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_paths() {
        assert_eq!(
            extension_config_path("telegram").unwrap(),
            ".system/extensions/telegram/config.json"
        );
        assert_eq!(
            extension_state_path("google_drive").unwrap(),
            ".system/extensions/google_drive/state.json"
        );
        assert_eq!(
            extensions_registry_path(),
            ".system/extensions/installed.json"
        );
    }

    #[test]
    fn extension_paths_canonicalize_hyphens() {
        // Hyphens become underscores; this matches the rest of the extension
        // pipeline so a name installed as `web-search` writes to the same doc
        // regardless of which alias the caller used.
        assert_eq!(
            extension_config_path("web-search").unwrap(),
            ".system/extensions/web_search/config.json"
        );
    }

    #[test]
    fn extension_paths_reject_traversal() {
        // Regression: path helpers must reject names that could escape
        // `.system/extensions/`. Previously these were silently rewritten via
        // `replace('/', "_")` which would not catch `..` or backslashes.
        assert!(extension_config_path("../escape").is_err());
        assert!(extension_config_path("foo/bar").is_err());
        assert!(extension_config_path("foo\\bar").is_err());
        assert!(extension_config_path("foo\0bar").is_err());
        assert!(extension_config_path("").is_err());
        assert!(extension_state_path("../escape").is_err());
    }

    #[test]
    fn skill_paths() {
        assert_eq!(
            skill_manifest_path("code-review").unwrap(),
            ".system/skills/code-review.json"
        );
        assert_eq!(skills_registry_path(), ".system/skills/installed.json");
    }

    #[test]
    fn skill_paths_reject_invalid_names() {
        // Regression: previously `name.replace('/', "_")` allowed `..` and
        // other escape characters through.
        assert!(skill_manifest_path("../escape").is_err());
        assert!(skill_manifest_path("foo/bar").is_err());
        assert!(skill_manifest_path(".hidden").is_err());
        assert!(skill_manifest_path("").is_err());
    }

    #[test]
    fn schemas_are_valid_json() {
        // Schemas must be valid JSON objects
        assert!(extension_config_schema().is_object());
        assert!(extensions_registry_schema().is_object());
        assert!(skill_manifest_schema().is_object());
    }
}
