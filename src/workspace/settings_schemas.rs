//! Compile-time JSON Schema registry for known settings keys.
//!
//! When a setting is written to `.system/settings/{key}.json`, the schema
//! for that key (if known) is stored in the document's metadata. The
//! workspace write path validates content against it automatically.
//!
//! Unknown keys get no schema — they accept any valid JSON (extensible).

use serde_json::{Value, json};

use crate::error::WorkspaceError;
use crate::workspace::document::system_paths::SETTINGS_PREFIX;

/// Maximum allowed length for a settings key.
const MAX_SETTINGS_KEY_LEN: usize = 128;

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
        // Schema mirrors `CustomLlmProviderSettings` in `src/settings.rs`.
        "llm_custom_providers" => Some(json!({
            "type": "array",
            "description": "User-defined LLM provider configurations",
            "items": {
                "type": "object",
                "properties": {
                    "id":            { "type": "string" },
                    "name":          { "type": "string" },
                    "adapter":       { "type": "string" },
                    "base_url":      { "type": ["string", "null"] },
                    "default_model": { "type": ["string", "null"] },
                    "api_key":       { "type": ["string", "null"] },
                    "builtin":       { "type": "boolean" }
                },
                "required": ["id", "name", "adapter"],
                "additionalProperties": false
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

/// Validate a settings key against path-traversal and structural rules.
///
/// Settings keys are concatenated into workspace paths under
/// `.system/settings/{key}.json`, so they must not contain path separators
/// or relative-path segments that could escape the prefix. Allowed: ASCII
/// alphanumerics, `_`, `-`, and `.` (but not `..`).
pub fn validate_settings_key(key: &str) -> Result<(), WorkspaceError> {
    let reject = |reason: &str| WorkspaceError::InvalidPath {
        path: format!("{SETTINGS_PREFIX}{key}"),
        reason: reason.to_string(),
    };

    if key.is_empty() {
        return Err(reject("key is empty"));
    }
    if key.len() > MAX_SETTINGS_KEY_LEN {
        return Err(reject(&format!(
            "key longer than {MAX_SETTINGS_KEY_LEN} chars"
        )));
    }
    if key.contains('/') || key.contains('\\') {
        return Err(reject("path separators are not allowed"));
    }
    if key.contains("..") {
        return Err(reject("'..' is not allowed"));
    }
    if key.starts_with('.') {
        return Err(reject("leading '.' is not allowed"));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(reject(
            "only ASCII alphanumerics, '_', '-', and '.' are allowed",
        ));
    }
    Ok(())
}

/// Build the path for a settings document in the workspace.
///
/// Caller is responsible for validating the key with [`validate_settings_key`]
/// first; this function does not re-validate.
pub fn settings_path(key: &str) -> String {
    format!("{SETTINGS_PREFIX}{key}.json")
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
            ".system/settings/llm_backend.json"
        );
    }

    #[test]
    fn validate_settings_key_accepts_normal_keys() {
        assert!(validate_settings_key("llm_backend").is_ok());
        assert!(validate_settings_key("selected_model").is_ok());
        assert!(validate_settings_key("tool_permissions.shell").is_ok());
        assert!(validate_settings_key("a-b-c").is_ok());
    }

    #[test]
    fn validate_settings_key_rejects_path_traversal() {
        assert!(validate_settings_key("../etc/passwd").is_err());
        assert!(validate_settings_key("foo/../bar").is_err());
        assert!(validate_settings_key("foo/bar").is_err());
        assert!(validate_settings_key("foo\\bar").is_err());
        assert!(validate_settings_key(".hidden").is_err());
    }

    #[test]
    fn validate_settings_key_rejects_empty_and_too_long() {
        assert!(validate_settings_key("").is_err());
        let long = "a".repeat(MAX_SETTINGS_KEY_LEN + 1);
        assert!(validate_settings_key(&long).is_err());
    }

    #[test]
    fn validate_settings_key_returns_invalid_path_variant() {
        // Regression: key/path validation must surface as `InvalidPath`,
        // not `SchemaValidation` — callers and downstream UIs need to
        // distinguish "your settings key has bad characters" from "your
        // settings *value* failed JSON-Schema validation".
        let err = validate_settings_key("foo/bar").unwrap_err();
        assert!(
            matches!(err, WorkspaceError::InvalidPath { .. }),
            "expected InvalidPath, got {err:?}"
        );
    }

    #[test]
    fn validate_settings_key_rejects_disallowed_chars() {
        assert!(validate_settings_key("foo bar").is_err());
        assert!(validate_settings_key("foo:bar").is_err());
        assert!(validate_settings_key("foo*bar").is_err());
        assert!(validate_settings_key("foo$bar").is_err());
    }
}
