use serde_json::Value as JsonValue;

use crate::provider::{ToolCall, ToolDefinition};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlaceholderStrippingMode {
    NullOnly,
    NullAndEmptyStrings,
}

impl PlaceholderStrippingMode {
    fn strips_empty_strings(self) -> bool {
        matches!(self, Self::NullAndEmptyStrings)
    }
}

/// Strip "unset optional" placeholder values that the strict-mode tool-schema
/// transform induces, from each tool call's arguments.
///
/// `shape_tool_schema(StrictOpenAi, ..)` forces every property to be `required`
/// and makes the originally-optional ones nullable, because OpenAI strict
/// function-calling has no notion of an absent property. Models therefore fill
/// the optionals they aren't using with a placeholder: `null` for most, `""`
/// for some (e.g. gpt-5.2-codex). Validating those against the tool's original
/// schema would reject them, so this removes them before loop-side validation.
pub(crate) fn strip_unset_optional_fields(
    tool_calls: &mut [ToolCall],
    tools: &[ToolDefinition],
    mode: PlaceholderStrippingMode,
) {
    for call in tool_calls.iter_mut() {
        if let Some(tool) = tools.iter().find(|tool| tool.name == call.name) {
            strip_unset_optionals_in_value(&mut call.arguments, &tool.parameters, mode);
        }
    }
}

fn strip_unset_optionals_in_value(
    value: &mut JsonValue,
    schema: &JsonValue,
    mode: PlaceholderStrippingMode,
) {
    match value {
        JsonValue::Object(map) => {
            let required = placeholder_required_fields(schema);
            let properties = placeholder_property_schemas(schema);
            let to_remove = map
                .iter()
                .filter(|(key, val)| {
                    properties.contains_key(key.as_str())
                        && !required.contains(key.as_str())
                        && is_unset_placeholder(val, mode)
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            for key in &to_remove {
                map.remove(key);
            }
            for (key, val) in map.iter_mut() {
                if let Some(prop_schema) = properties.get(key.as_str()) {
                    strip_unset_optionals_in_value(val, prop_schema, mode);
                }
            }
        }
        JsonValue::Array(items) => {
            if let Some(item_schema) = schema.get("items") {
                for item in items.iter_mut() {
                    strip_unset_optionals_in_value(item, item_schema, mode);
                }
            }
        }
        _ => {}
    }
}

fn placeholder_required_fields(schema: &JsonValue) -> std::collections::HashSet<&str> {
    let mut required = schema
        .get("required")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .collect::<std::collections::HashSet<_>>();

    for keyword in ["oneOf", "anyOf"] {
        let Some(variants) = schema.get(keyword).and_then(JsonValue::as_array) else {
            continue;
        };
        let mut common = variants.iter().map(placeholder_required_fields);
        let Some(mut intersection) = common.next() else {
            continue;
        };
        for required in common {
            intersection.retain(|field| required.contains(field));
        }
        required.extend(intersection);
    }

    if let Some(variants) = schema.get("allOf").and_then(JsonValue::as_array) {
        for variant in variants {
            required.extend(placeholder_required_fields(variant));
        }
    }

    required
}

fn placeholder_property_schemas(schema: &JsonValue) -> std::collections::HashMap<&str, &JsonValue> {
    let mut properties = std::collections::HashMap::new();
    collect_placeholder_property_schemas(schema, &mut properties);
    properties
}

fn collect_placeholder_property_schemas<'a>(
    schema: &'a JsonValue,
    properties: &mut std::collections::HashMap<&'a str, &'a JsonValue>,
) {
    if let Some(schema_properties) = schema.get("properties").and_then(JsonValue::as_object) {
        for (key, property_schema) in schema_properties {
            properties.entry(key.as_str()).or_insert(property_schema);
        }
    }
    for keyword in ["oneOf", "anyOf", "allOf"] {
        let Some(variants) = schema.get(keyword).and_then(JsonValue::as_array) else {
            continue;
        };
        for variant in variants {
            collect_placeholder_property_schemas(variant, properties);
        }
    }
}

fn is_unset_placeholder(value: &JsonValue, mode: PlaceholderStrippingMode) -> bool {
    match value {
        JsonValue::Null => true,
        JsonValue::String(string) => mode.strips_empty_strings() && string.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_optional_null_and_empty() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string" },
                "format": { "type": "string" },
                "timezone": { "type": "string" }
            }
        });
        let mut value = serde_json::json!({ "operation": "now", "format": null, "timezone": "" });
        strip_unset_optionals_in_value(
            &mut value,
            &schema,
            PlaceholderStrippingMode::NullAndEmptyStrings,
        );
        assert_eq!(value, serde_json::json!({ "operation": "now" }));
    }

    #[test]
    fn keeps_required_placeholders() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "id": { "type": "string" }, "note": { "type": "string" } },
            "required": ["id"]
        });
        let mut value = serde_json::json!({ "id": null, "note": null });
        strip_unset_optionals_in_value(
            &mut value,
            &schema,
            PlaceholderStrippingMode::NullAndEmptyStrings,
        );
        assert_eq!(value, serde_json::json!({ "id": null }));
    }

    #[test]
    fn collapses_top_level_oneof_to_one_variant() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "content": { "type": "string" },
                "url": { "type": "string" }
            },
            "oneOf": [ { "required": ["content"] }, { "required": ["url"] } ],
            "additionalProperties": false
        });
        let mut value = serde_json::json!({ "url": "https://example.com/SKILL.md", "content": "" });
        strip_unset_optionals_in_value(
            &mut value,
            &schema,
            PlaceholderStrippingMode::NullAndEmptyStrings,
        );
        assert_eq!(
            value,
            serde_json::json!({ "url": "https://example.com/SKILL.md" })
        );
    }

    #[test]
    fn drops_nested_oneof_branch_placeholders() {
        let schema = trigger_create_schema();

        let mut once_call = serde_json::json!({
            "name": "Dog walking reminder",
            "prompt": "Walk the dog",
            "schedule": {
                "kind": "once",
                "at": "2026-06-23T14:00:00",
                "timezone": "America/Los_Angeles",
                "expression": null
            }
        });
        strip_unset_optionals_in_value(&mut once_call, &schema, PlaceholderStrippingMode::NullOnly);
        assert_eq!(
            once_call,
            serde_json::json!({
                "name": "Dog walking reminder",
                "prompt": "Walk the dog",
                "schedule": {
                    "kind": "once",
                    "at": "2026-06-23T14:00:00",
                    "timezone": "America/Los_Angeles"
                }
            })
        );

        let mut cron_call = serde_json::json!({
            "name": "Tuesday reminder",
            "prompt": "Send the Tuesday reminder",
            "schedule": {
                "kind": "cron",
                "expression": "0 14 * * 2",
                "timezone": "America/Los_Angeles",
                "at": null
            }
        });
        strip_unset_optionals_in_value(&mut cron_call, &schema, PlaceholderStrippingMode::NullOnly);
        assert_eq!(
            cron_call,
            serde_json::json!({
                "name": "Tuesday reminder",
                "prompt": "Send the Tuesday reminder",
                "schedule": {
                    "kind": "cron",
                    "expression": "0 14 * * 2",
                    "timezone": "America/Los_Angeles"
                }
            })
        );
    }

    #[test]
    fn keeps_nested_oneof_shared_required_placeholders() {
        let schema = trigger_create_schema();
        let mut missing_shared_field = serde_json::json!({
            "name": "Bad reminder",
            "prompt": "Walk the dog",
            "schedule": {
                "kind": "once",
                "at": "2026-06-23T14:00:00",
                "timezone": null,
                "expression": null
            }
        });

        strip_unset_optionals_in_value(
            &mut missing_shared_field,
            &schema,
            PlaceholderStrippingMode::NullOnly,
        );

        assert_eq!(
            missing_shared_field,
            serde_json::json!({
                "name": "Bad reminder",
                "prompt": "Walk the dog",
                "schedule": {
                    "kind": "once",
                    "at": "2026-06-23T14:00:00",
                    "timezone": null
                }
            })
        );
    }

    #[test]
    fn recurses_objects_and_arrays() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "a": { "type": "string" }, "b": { "type": "string" } },
                        "required": ["a"]
                    }
                }
            }
        });
        let mut value =
            serde_json::json!({ "items": [ { "a": "x", "b": null }, { "a": "y", "b": "" } ] });
        strip_unset_optionals_in_value(
            &mut value,
            &schema,
            PlaceholderStrippingMode::NullAndEmptyStrings,
        );
        assert_eq!(
            value,
            serde_json::json!({ "items": [ { "a": "x" }, { "a": "y" } ] })
        );
    }

    #[test]
    fn preserves_empty_string_when_disabled() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "prefix": { "type": "string" } }
        });
        let mut keep = serde_json::json!({ "prefix": "" });
        strip_unset_optionals_in_value(&mut keep, &schema, PlaceholderStrippingMode::NullOnly);
        assert_eq!(keep, serde_json::json!({ "prefix": "" }));

        let mut drop_null = serde_json::json!({ "prefix": null });
        strip_unset_optionals_in_value(&mut drop_null, &schema, PlaceholderStrippingMode::NullOnly);
        assert_eq!(drop_null, serde_json::json!({}));
    }

    #[test]
    fn preserves_untyped_free_form_object_members() {
        let schema = serde_json::json!({ "type": "object" });
        let mut value = serde_json::json!({
            "free_form_null": null,
            "free_form_empty": ""
        });

        strip_unset_optionals_in_value(
            &mut value,
            &schema,
            PlaceholderStrippingMode::NullAndEmptyStrings,
        );

        assert_eq!(
            value,
            serde_json::json!({
                "free_form_null": null,
                "free_form_empty": ""
            })
        );
    }

    #[test]
    fn matches_calls_to_their_tool_schema() {
        let tools = vec![ToolDefinition {
            name: "time".to_string(),
            description: String::new(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "operation": { "type": "string" }, "format": { "type": "string" } }
            }),
        }];
        let mut calls = vec![ToolCall {
            id: "1".to_string(),
            name: "time".to_string(),
            arguments: serde_json::json!({ "operation": "now", "format": null }),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        }];
        strip_unset_optional_fields(
            &mut calls,
            &tools,
            PlaceholderStrippingMode::NullAndEmptyStrings,
        );
        assert_eq!(
            calls[0].arguments,
            serde_json::json!({ "operation": "now" })
        );
    }

    #[test]
    fn strips_placeholder_when_oneof_variant_has_no_required_fields() {
        let tools = vec![ToolDefinition {
            name: "lookup".to_string(),
            description: String::new(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "oneOf": [
                    { "required": ["query"] },
                    {}
                ]
            }),
        }];
        let mut calls = vec![ToolCall {
            id: "1".to_string(),
            name: "lookup".to_string(),
            arguments: serde_json::json!({ "query": null }),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        }];

        strip_unset_optional_fields(&mut calls, &tools, PlaceholderStrippingMode::NullOnly);

        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    fn trigger_create_schema() -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "prompt": { "type": "string" },
                "schedule": {
                    "oneOf": [
                        {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "kind": { "const": "cron" },
                                "expression": { "type": "string" },
                                "timezone": { "type": "string" }
                            },
                            "required": ["kind", "expression", "timezone"]
                        },
                        {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "kind": { "const": "once" },
                                "at": { "type": "string" },
                                "timezone": { "type": "string" }
                            },
                            "required": ["kind", "at", "timezone"]
                        }
                    ]
                }
            },
            "required": ["name", "prompt", "schedule"],
            "additionalProperties": false
        })
    }
}
