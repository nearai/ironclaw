pub fn prepare_tool_params(
    tool: &dyn crate::tools::tool::Tool,
    params: &serde_json::Value,
) -> serde_json::Value {
    prepare_params_for_schema(params, &tool.discovery_schema())
}

pub(crate) fn prepare_params_for_schema(
    params: &serde_json::Value,
    schema: &serde_json::Value,
) -> serde_json::Value {
    coerce_value(params, schema)
}

fn coerce_value(value: &serde_json::Value, schema: &serde_json::Value) -> serde_json::Value {
    // This coercer handles concrete schema shapes including discriminated unions
    // (oneOf/anyOf with const or single-element enum discriminators) and allOf
    // merges. It does not resolve $ref references; those schemas pass through
    // unchanged unless they also advertise a directly coercible type/property shape.
    if value.is_null() {
        return value.clone();
    }

    if let Some(s) = value.as_str() {
        return coerce_string_value(s, schema).unwrap_or_else(|| value.clone());
    }

    if let Some(items) = value.as_array() {
        if !schema_allows_type(schema, "array") {
            return value.clone();
        }

        let Some(item_schema) = schema.get("items") else {
            return value.clone();
        };

        return serde_json::Value::Array(
            items
                .iter()
                .map(|item| coerce_value(item, item_schema))
                .collect(),
        );
    }

    if let Some(obj) = value.as_object() {
        if !schema_allows_type(schema, "object") {
            return value.clone();
        }

        let resolved = resolve_effective_properties(schema, obj);
        let properties = resolved
            .as_ref()
            .or_else(|| schema.get("properties").and_then(|p| p.as_object()));
        let additional_schema = schema.get("additionalProperties").filter(|v| v.is_object());
        let mut coerced = obj.clone();

        for (key, current) in &mut coerced {
            if let Some(prop_schema) = properties.and_then(|props| props.get(key)) {
                *current = coerce_value(current, prop_schema);
                continue;
            }

            if let Some(additional_schema) = additional_schema {
                *current = coerce_value(current, additional_schema);
            }
        }

        return serde_json::Value::Object(coerced);
    }

    value.clone()
}

/// When the schema uses `oneOf`, `anyOf`, or `allOf` combinators, build a
/// merged property map that can be used for coercion.
///
/// - Top-level `properties` are included first (base properties).
/// - `allOf`: merge ALL variants' properties (last-wins on conflicts).
/// - `oneOf`/`anyOf`: find the discriminated match and merge its properties.
///
/// Returns `None` if no combinators are present or no match is found, so the
/// caller falls back to the existing top-level `properties` lookup.
fn resolve_effective_properties(
    schema: &serde_json::Value,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let has_combinators = schema.get("allOf").is_some()
        || schema.get("oneOf").is_some()
        || schema.get("anyOf").is_some();

    if !has_combinators {
        return None;
    }

    let mut merged = serde_json::Map::new();

    // Start with top-level properties
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        merged.extend(props.iter().map(|(k, v)| (k.clone(), v.clone())));
    }

    // allOf: merge ALL variants' properties
    if let Some(all_of) = schema.get("allOf").and_then(|a| a.as_array()) {
        for variant in all_of {
            if let Some(props) = variant.get("properties").and_then(|p| p.as_object()) {
                merged.extend(props.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
    }

    // oneOf/anyOf: find discriminated match
    for key in ["oneOf", "anyOf"] {
        if let Some(variants) = schema.get(key).and_then(|v| v.as_array())
            && let Some(variant) = find_discriminated_variant(variants, obj)
            && let Some(props) = variant.get("properties").and_then(|p| p.as_object())
        {
            merged.extend(props.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }

    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

/// Find a `oneOf`/`anyOf` variant that matches the given object by checking
/// `const`-valued and single-element `enum`-valued properties (discriminators).
///
/// A variant matches when ALL its discriminator properties match the object's
/// values and at least one such discriminator exists. Returns `None` if no
/// variant matches (safe fallback — no coercion).
fn find_discriminated_variant<'a>(
    variants: &'a [serde_json::Value],
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    variants.iter().find(|variant| {
        let Some(props) = variant.get("properties").and_then(|p| p.as_object()) else {
            return false;
        };

        let mut discriminator_count = 0;

        for (key, prop_schema) in props {
            // Check for const discriminator
            if let Some(const_val) = prop_schema.get("const") {
                discriminator_count += 1;
                match obj.get(key) {
                    Some(v) if v == const_val => {}
                    _ => return false,
                }
                continue;
            }

            // Check for single-element enum discriminator
            if let Some(enum_vals) = prop_schema.get("enum").and_then(|e| e.as_array())
                && enum_vals.len() == 1
            {
                discriminator_count += 1;
                match obj.get(key) {
                    Some(v) if v == &enum_vals[0] => {}
                    _ => return false,
                }
            }
        }

        discriminator_count > 0
    })
}

fn coerce_string_value(s: &str, schema: &serde_json::Value) -> Option<serde_json::Value> {
    if schema_allows_type(schema, "string") {
        return None;
    }

    if schema_allows_type(schema, "integer")
        && let Ok(v) = s.parse::<i64>()
    {
        return Some(serde_json::Value::from(v));
    }

    if schema_allows_type(schema, "number")
        && let Ok(v) = s.parse::<f64>()
    {
        return Some(serde_json::Value::from(v));
    }

    if schema_allows_type(schema, "boolean") {
        match s.to_lowercase().as_str() {
            "true" => return Some(serde_json::json!(true)),
            "false" => return Some(serde_json::json!(false)),
            _ => {}
        }
    }

    if schema_allows_type(schema, "array") || schema_allows_type(schema, "object") {
        let parsed = serde_json::from_str::<serde_json::Value>(s).ok()?;
        let matches_schema = match &parsed {
            serde_json::Value::Array(_) => schema_allows_type(schema, "array"),
            serde_json::Value::Object(_) => schema_allows_type(schema, "object"),
            _ => false,
        };

        if matches_schema {
            return Some(coerce_value(&parsed, schema));
        }
    }

    None
}

fn schema_allows_type(schema: &serde_json::Value, expected: &str) -> bool {
    match schema.get("type") {
        Some(serde_json::Value::String(t)) => t == expected,
        Some(serde_json::Value::Array(types)) => types.iter().any(|t| t.as_str() == Some(expected)),
        _ => match expected {
            "object" => {
                schema
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .is_some()
                    || schema.get("oneOf").is_some()
                    || schema.get("anyOf").is_some()
                    || schema.get("allOf").is_some()
            }
            "array" => schema.get("items").is_some(),
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use async_trait::async_trait;

    use super::*;
    use crate::context::JobContext;
    use crate::tools::tool::{Tool, ToolError, ToolOutput};

    struct StubTool {
        schema: serde_json::Value,
    }

    #[async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str {
            "stub"
        }

        fn description(&self) -> &str {
            "stub"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            self.schema.clone()
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(params, Duration::from_millis(1)))
        }
    }

    #[test]
    fn coerces_scalar_strings() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "count": { "type": "number" },
                "limit": { "type": "integer" },
                "enabled": { "type": "boolean" }
            }
        });
        let params = serde_json::json!({
            "count": "5",
            "limit": "10",
            "enabled": "TRUE"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["count"], serde_json::json!(5.0)); // safety: test-only assertion
        assert_eq!(result["limit"], serde_json::json!(10)); // safety: test-only assertion
        assert_eq!(result["enabled"], serde_json::json!(true)); // safety: test-only assertion
    }

    #[test]
    fn coerces_stringified_array_and_recurses_into_items() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "values": {
                    "type": "array",
                    "items": {
                        "type": "array",
                        "items": { "type": "integer" }
                    }
                }
            }
        });
        let params = serde_json::json!({
            "values": "[[\"1\", \"2\"], [\"3\", 4]]"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["values"], serde_json::json!([[1, 2], [3, 4]])); // safety: test-only assertion
    }

    #[test]
    fn coerces_stringified_object_and_recurses_into_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "request": {
                    "type": "object",
                    "properties": {
                        "start_index": { "type": "integer" },
                        "enabled": { "type": ["boolean", "null"] }
                    }
                }
            }
        });
        let params = serde_json::json!({
            "request": "{\"start_index\":\"12\",\"enabled\":\"false\"}"
        });

        let result = prepare_params_for_schema(&params, &schema);

        #[rustfmt::skip]
        assert_eq!( // safety: test-only assertion
            result["request"],
            serde_json::json!({"start_index": 12, "enabled": false})
        );
    }

    #[test]
    fn coerces_nullable_stringified_arrays() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "requests": {
                    "type": ["array", "null"],
                    "items": {
                        "type": "object",
                        "properties": {
                            "enabled": { "type": "boolean" }
                        }
                    }
                }
            }
        });
        let params = serde_json::json!({
            "requests": "[{\"enabled\":\"true\"}]"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["requests"], serde_json::json!([{ "enabled": true }])); // safety: test-only assertion
    }

    #[test]
    fn coerces_typed_additional_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": {
                "type": "object",
                "properties": {
                    "count": { "type": "integer" },
                    "enabled": { "type": "boolean" }
                }
            }
        });
        let params = serde_json::json!({
            "alpha": "{\"count\":\"5\",\"enabled\":\"false\"}",
            "beta": { "count": "7", "enabled": "true" }
        });

        let result = prepare_params_for_schema(&params, &schema);

        #[rustfmt::skip]
        assert_eq!( // safety: test-only assertion
            result,
            serde_json::json!({
                "alpha": { "count": 5, "enabled": false },
                "beta": { "count": 7, "enabled": true }
            })
        );
    }

    #[test]
    fn leaves_invalid_json_strings_unchanged() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "requests": {
                    "type": "array",
                    "items": { "type": "object" }
                }
            }
        });
        let params = serde_json::json!({
            "requests": "[{\"oops\":]"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["requests"], serde_json::json!("[{\"oops\":]")); // safety: test-only assertion
    }

    #[test]
    fn leaves_string_when_schema_allows_string() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": ["string", "object"] }
            }
        });
        let params = serde_json::json!({
            "value": "{\"mode\":\"raw\"}"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["value"], serde_json::json!("{\"mode\":\"raw\"}")); // safety: test-only assertion
    }

    #[test]
    fn permissive_schema_is_noop() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": true
        });
        let params = serde_json::json!({"count": "10"});

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["count"], serde_json::json!("10")); // safety: test-only assertion
    }

    #[test]
    fn coerces_oneof_discriminated_variant() {
        let schema = serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "action": { "const": "list_repos" },
                        "limit": { "type": "integer" },
                        "sort": { "type": "string" }
                    }
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "const": "get_repo" },
                        "repo": { "type": "string" }
                    }
                }
            ]
        });
        let params = serde_json::json!({
            "action": "list_repos",
            "limit": "100",
            "sort": "stars"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["action"], serde_json::json!("list_repos"));
        assert_eq!(result["limit"], serde_json::json!(100));
        assert_eq!(result["sort"], serde_json::json!("stars"));
    }

    #[test]
    fn coerces_oneof_with_enum_discriminator() {
        let schema = serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "mode": { "enum": ["fetch"] },
                        "count": { "type": "integer" }
                    }
                },
                {
                    "type": "object",
                    "properties": {
                        "mode": { "enum": ["push"] },
                        "force": { "type": "boolean" }
                    }
                }
            ]
        });
        let params = serde_json::json!({
            "mode": "push",
            "force": "true"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["mode"], serde_json::json!("push"));
        assert_eq!(result["force"], serde_json::json!(true));
    }

    #[test]
    fn coerces_allof_merged_properties() {
        let schema = serde_json::json!({
            "allOf": [
                {
                    "type": "object",
                    "properties": {
                        "page": { "type": "integer" }
                    }
                },
                {
                    "type": "object",
                    "properties": {
                        "per_page": { "type": "integer" },
                        "verbose": { "type": "boolean" }
                    }
                }
            ]
        });
        let params = serde_json::json!({
            "page": "2",
            "per_page": "50",
            "verbose": "false"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["page"], serde_json::json!(2));
        assert_eq!(result["per_page"], serde_json::json!(50));
        assert_eq!(result["verbose"], serde_json::json!(false));
    }

    #[test]
    fn oneof_no_discriminator_match_is_noop() {
        let schema = serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "action": { "const": "list_repos" },
                        "limit": { "type": "integer" }
                    }
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "const": "get_repo" },
                        "repo": { "type": "string" }
                    }
                }
            ]
        });
        let params = serde_json::json!({
            "action": "unknown_action",
            "limit": "100"
        });

        let result = prepare_params_for_schema(&params, &schema);

        // No variant matched, so no coercion happens
        assert_eq!(result["limit"], serde_json::json!("100"));
    }

    #[test]
    fn anyof_without_discriminator_is_noop() {
        let schema = serde_json::json!({
            "anyOf": [
                {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    },
                    "required": ["name"]
                },
                {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer" }
                    },
                    "required": ["id"]
                }
            ]
        });
        let params = serde_json::json!({
            "id": "42"
        });

        let result = prepare_params_for_schema(&params, &schema);

        // No const/enum discriminators, so no variant matches, no coercion
        assert_eq!(result["id"], serde_json::json!("42"));
    }

    #[test]
    fn prepare_tool_params_uses_discovery_schema() {
        let tool = StubTool {
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "requests": {
                        "type": "array",
                        "items": { "type": "object" }
                    }
                }
            }),
        };
        let params = serde_json::json!({
            "requests": "[{\"insertText\":{\"text\":\"hello\"}}]"
        });

        let result = prepare_tool_params(&tool, &params);

        #[rustfmt::skip]
        assert_eq!( // safety: test-only assertion
            result["requests"],
            serde_json::json!([{ "insertText": { "text": "hello" } }])
        );
    }
}
