pub(crate) fn prepare_tool_params(
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

        let mut coerced = obj.clone();
        let properties = schema.get("properties").and_then(|p| p.as_object());

        if let Some(props) = properties {
            for (key, prop_schema) in props {
                if let Some(current) = coerced.get(key).cloned() {
                    coerced.insert(key.clone(), coerce_value(&current, prop_schema));
                }
            }
        }

        if let Some(additional_schema) =
            schema.get("additionalProperties").filter(|v| v.is_object())
        {
            for (key, current) in obj {
                if properties.is_some_and(|props| props.contains_key(key)) {
                    continue;
                }
                coerced.insert(key.clone(), coerce_value(current, additional_schema));
            }
        }

        return serde_json::Value::Object(coerced);
    }

    value.clone()
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
            "object" => schema
                .get("properties")
                .and_then(|p| p.as_object())
                .is_some(),
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
