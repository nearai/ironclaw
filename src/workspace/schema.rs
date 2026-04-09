//! JSON Schema validation for workspace documents.
//!
//! When a document (or its folder `.config`) carries a `schema` field in
//! metadata, all write operations validate the new content against that schema
//! before persisting. This enables typed, structured storage within the
//! workspace — settings, extension configs, and other system documents can
//! declare their expected shape and reject malformed writes at the boundary.

use crate::error::WorkspaceError;

/// Validate `content` as JSON against a JSON Schema.
///
/// Returns `Ok(())` if the content is valid JSON that conforms to the schema.
/// Returns `WorkspaceError::SchemaValidation` if:
/// - `content` is not valid JSON (schema implies JSON content)
/// - The parsed JSON does not conform to the schema
///
/// The `path` argument is used only for error messages.
pub fn validate_content_against_schema(
    path: &str,
    content: &str,
    schema: &serde_json::Value,
) -> Result<(), WorkspaceError> {
    // Treat an explicit JSON `null` schema as "no schema". Serde deserializes
    // `"schema": null` in metadata as `Some(Value::Null)` (not `None`), so the
    // upstream `if let Some(schema) = &metadata.schema` check passes through to
    // here. Without this guard, `validator_for(Value::Null)` errors out and
    // every subsequent write to that document is permanently blocked — a
    // latent DoS if a caller ever writes a null schema field by accident.
    if schema.is_null() {
        return Ok(());
    }

    let instance: serde_json::Value =
        serde_json::from_str(content).map_err(|e| WorkspaceError::SchemaValidation {
            path: path.to_string(),
            errors: vec![format!("content is not valid JSON: {e}")],
        })?;

    // NOTE: `validator_for` recompiles the schema on every call. This is
    // intentionally not cached today: schema-validated writes are limited to
    // settings/extension/skill state, which are rare user-initiated operations
    // (not a hot path). If schema validation moves into a frequent write path,
    // build a `Validator` once per distinct schema (e.g., via `OnceCell`/
    // `DashMap` keyed on the schema's canonical JSON) and call
    // `Validator::iter_errors` here instead.
    //
    // `validator_for` + `iter_errors` (instead of `jsonschema::validate`) so
    // that we return *all* validation errors in a single round-trip — users
    // fixing a misconfigured `llm_custom_providers` setting shouldn't have to
    // submit five iterative writes to discover all the things that are wrong
    // with their payload. It also separates "bad schema" errors (compile
    // failure) from "bad content" errors (instance failure).
    let validator =
        jsonschema::validator_for(schema).map_err(|e| WorkspaceError::SchemaValidation {
            path: path.to_string(),
            errors: vec![format!("invalid schema: {e}")],
        })?;

    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|e| e.to_string())
        .collect();

    if errors.is_empty() {
        Ok(())
    } else {
        Err(WorkspaceError::SchemaValidation {
            path: path.to_string(),
            errors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_json_passes_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        });
        let content = r#"{"name": "Alice", "age": 30}"#;
        assert!(validate_content_against_schema("test.json", content, &schema).is_ok());
    }

    #[test]
    fn invalid_json_fails() {
        let schema = json!({"type": "object"});
        let content = "not json at all";
        let err = validate_content_against_schema("test.json", content, &schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { path, errors } => {
                assert_eq!(path, "test.json");
                assert!(errors[0].contains("not valid JSON"));
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn missing_required_field_fails() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        let content = r#"{"age": 30}"#;
        let err = validate_content_against_schema("test.json", content, &schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { errors, .. } => {
                assert!(!errors.is_empty());
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn wrong_type_fails() {
        let schema = json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer" }
            }
        });
        let content = r#"{"count": "not a number"}"#;
        let err = validate_content_against_schema("test.json", content, &schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { errors, .. } => {
                assert!(!errors.is_empty());
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn scalar_schema_validates_scalar_content() {
        let schema = json!({"type": "string"});
        let content = r#""hello""#;
        assert!(validate_content_against_schema("test.json", content, &schema).is_ok());

        let content = "42";
        let err = validate_content_against_schema("test.json", content, &schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { .. } => {}
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn enum_validation() {
        let schema = json!({
            "type": "string",
            "enum": ["anthropic", "openai", "ollama"]
        });
        assert!(validate_content_against_schema("test.json", r#""anthropic""#, &schema).is_ok());
        assert!(validate_content_against_schema("test.json", r#""unknown""#, &schema).is_err());
    }

    #[test]
    fn empty_object_passes_permissive_schema() {
        let schema = json!({"type": "object"});
        assert!(validate_content_against_schema("test.json", "{}", &schema).is_ok());
    }

    #[test]
    fn multiple_errors_are_all_reported() {
        // Regression: `jsonschema::validate` only returns the first error,
        // so users iteratively fix-and-retry. Switching to `iter_errors`
        // collects every violation in one round.
        let schema = json!({
            "type": "object",
            "properties": {
                "name":  { "type": "string" },
                "age":   { "type": "integer" },
                "email": { "type": "string", "format": "email" }
            },
            "required": ["name", "age", "email"]
        });
        // Missing all three required fields AND has a wrong-typed extra key.
        let content = r#"{"extra": 123}"#;
        let err = validate_content_against_schema("test.json", content, &schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { errors, .. } => {
                assert!(
                    errors.len() >= 3,
                    "expected at least 3 errors for 3 missing required fields, got {}: {errors:?}",
                    errors.len()
                );
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn null_schema_is_treated_as_no_op() {
        // Regression: a metadata field of `"schema": null` deserializes as
        // `Some(Value::Null)`, not `None`. Without an explicit null guard,
        // `jsonschema::validator_for(Value::Null)` errors out and every
        // subsequent write to that document is blocked — latent DoS.
        let schema = serde_json::Value::Null;

        // Any content (including invalid JSON) should pass when the schema
        // is null, because there's effectively nothing to validate against.
        assert!(validate_content_against_schema("test.json", "{}", &schema).is_ok());
        assert!(
            validate_content_against_schema("test.json", "not even json", &schema).is_ok(),
            "null schema must skip validation entirely, including the JSON parse step"
        );
    }

    #[test]
    fn invalid_schema_is_distinguished_from_invalid_content() {
        // A broken schema (e.g. `type` set to a non-string) should produce
        // a clear "invalid schema" error rather than confusing the user
        // about their content.
        let broken_schema = json!({"type": 123});
        let err = validate_content_against_schema("test.json", "{}", &broken_schema).unwrap_err();
        match err {
            WorkspaceError::SchemaValidation { errors, .. } => {
                assert!(
                    errors[0].contains("invalid schema"),
                    "expected 'invalid schema' prefix, got: {:?}",
                    errors[0]
                );
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }
}
