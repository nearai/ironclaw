//! Compile-time JSON Schema registry for known settings keys.
//!
//! When a setting is written to `_system/settings/{key}.json`, the schema
//! for that key (if known) is stored in the document's metadata. The
//! workspace write path validates content against it automatically.
//!
//! Unknown keys get no schema — they accept any valid JSON (extensible).

use serde_json::{Value, json};

/// Return the JSON Schema for a known settings key, or `None` for unknown keys.
pub fn schema_for_key(key: &str) -> Option<Value> {
    match key {
        "llm_backend" => Some(json!({
            "type": "string",
            "description": "Active LLM provider backend identifier"
        })),
        "selected_model" => Some(json!({
            "type": "string",
            "description": "Currently selected model name"
        })),
        "llm_custom_providers" => Some(json!({
            "type": "array",
            "description": "User-defined LLM provider configurations",
            "items": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "protocol": { "type": "string" },
                    "base_url": { "type": "string" },
                    "model": { "type": "string" }
                },
                "required": ["name"]
            }
        })),
        "llm_builtin_overrides" => Some(json!({
            "type": "object",
            "description": "API key overrides for built-in LLM providers",
            "additionalProperties": {
                "type": "object"
            }
        })),
        // Tool permission keys (tool_permissions.*)
        key if key.starts_with("tool_permissions.") => Some(json!({
            "type": "string",
            "enum": ["always_allow", "ask_each_time", "disabled"],
            "description": "Permission state for a tool"
        })),
        _ => None,
    }
}

/// Build the path for a settings document in the workspace.
pub fn settings_path(key: &str) -> String {
    format!("_system/settings/{key}.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_have_schemas() {
        assert!(schema_for_key("llm_backend").is_some());
        assert!(schema_for_key("selected_model").is_some());
        assert!(schema_for_key("llm_custom_providers").is_some());
        assert!(schema_for_key("tool_permissions.shell").is_some());
    }

    #[test]
    fn unknown_keys_return_none() {
        assert!(schema_for_key("unknown_key").is_none());
        assert!(schema_for_key("custom_setting").is_none());
    }

    #[test]
    fn settings_path_format() {
        assert_eq!(
            settings_path("llm_backend"),
            "_system/settings/llm_backend.json"
        );
    }
}
