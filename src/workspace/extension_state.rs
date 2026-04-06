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

use crate::workspace::document::system_paths;

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
pub fn extension_config_path(name: &str) -> String {
    format!(
        "{}{}/config.json",
        system_paths::EXTENSIONS_PREFIX,
        name.replace('/', "_")
    )
}

/// Build workspace path for an extension state document.
pub fn extension_state_path(name: &str) -> String {
    format!(
        "{}{}/state.json",
        system_paths::EXTENSIONS_PREFIX,
        name.replace('/', "_")
    )
}

/// Build workspace path for the installed extensions registry.
pub fn extensions_registry_path() -> String {
    format!("{}installed.json", system_paths::EXTENSIONS_PREFIX)
}

/// Build workspace path for a skill manifest.
pub fn skill_manifest_path(name: &str) -> String {
    format!(
        "{}{}.json",
        system_paths::SKILLS_PREFIX,
        name.replace('/', "_")
    )
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
            extension_config_path("telegram"),
            ".system/extensions/telegram/config.json"
        );
        assert_eq!(
            extensions_registry_path(),
            ".system/extensions/installed.json"
        );
    }

    #[test]
    fn skill_paths() {
        assert_eq!(
            skill_manifest_path("code-review"),
            ".system/skills/code-review.json"
        );
        assert_eq!(skills_registry_path(), ".system/skills/installed.json");
    }

    #[test]
    fn schemas_are_valid_json() {
        // Schemas must be valid JSON objects
        assert!(extension_config_schema().is_object());
        assert!(extensions_registry_schema().is_object());
        assert!(skill_manifest_schema().is_object());
    }
}
