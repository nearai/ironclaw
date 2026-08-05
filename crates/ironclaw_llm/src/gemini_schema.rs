use std::collections::HashSet;

use serde_json::Value as JsonValue;

/// Shape a tool schema for Gemini function declarations.
///
/// Gemini accepts a smaller JSON Schema subset than the OpenAI-compatible
/// providers. This reducer deliberately starts from the original schema so
/// provider-neutral top-level flattening cannot discard tagged-union
/// discriminator values before Gemini-specific merging runs.
pub(crate) fn shape_tool_schema(schema: &JsonValue, _description: &mut String) -> JsonValue {
    let mut schema = schema.clone();
    normalize_schema_recursive(&mut schema);
    schema
}

fn normalize_schema_recursive(schema: &mut JsonValue) {
    let Some(object) = schema.as_object_mut() else {
        *schema = serde_json::json!({ "type": "string" });
        return;
    };

    let constant = object.remove("const");
    let mut all_of = take_schema_variants(object, "allOf");
    let mut union_variants = take_schema_variants(object, "oneOf");
    union_variants.extend(take_schema_variants(object, "anyOf"));

    for variant in all_of.iter_mut().chain(union_variants.iter_mut()) {
        normalize_schema_recursive(variant);
    }

    for keyword in [
        "if",
        "then",
        "else",
        "not",
        "$schema",
        "$ref",
        "$defs",
        "definitions",
        "dependentRequired",
        "dependentSchemas",
        "contains",
        "prefixItems",
        "unevaluatedItems",
        "unevaluatedProperties",
        "patternProperties",
        "propertyNames",
    ] {
        object.remove(keyword);
    }

    normalize_type(object, constant.as_ref(), &all_of, &union_variants);

    for variant in &all_of {
        merge_variant(object, variant, true);
    }
    merge_required_fields(object, &all_of);
    if let Some(selected) = union_variants
        .iter()
        .max_by_key(|variant| schema_rank(variant))
    {
        // Copy the selected variant's scalar/array constraints, but merge
        // properties below in source order so equal-ranked tagged variants do
        // not privilege the last branch's discriminator.
        merge_variant(object, selected, false);
    }
    for variant in &union_variants {
        merge_properties(object, variant);
        merge_matching_enum(object, variant);
    }

    if let Some(JsonValue::Object(properties)) = object.get_mut("properties") {
        for property in properties.values_mut() {
            normalize_schema_recursive(property);
        }
    }
    if let Some(items) = object.get_mut("items") {
        if items.as_object().is_some_and(serde_json::Map::is_empty) {
            // Schemars represents `Vec<serde_json::Value>` as an empty item
            // schema. Gemini still requires a concrete scalar `type`; object
            // is the least-narrow representation for provider request maps.
            *items = serde_json::json!({ "type": "object" });
        } else {
            normalize_schema_recursive(items);
        }
    }
    if let Some(additional) = object.get_mut("additionalProperties")
        && additional.is_object()
    {
        normalize_schema_recursive(additional);
    }

    if object.get("type").and_then(JsonValue::as_str) == Some("array") {
        object
            .entry("items".to_string())
            .or_insert_with(|| serde_json::json!({ "type": "object" }));
    } else {
        object.remove("items");
    }
}

fn take_schema_variants(
    object: &mut serde_json::Map<String, JsonValue>,
    keyword: &str,
) -> Vec<JsonValue> {
    object
        .remove(keyword)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

fn normalize_type(
    object: &mut serde_json::Map<String, JsonValue>,
    constant: Option<&JsonValue>,
    all_of: &[JsonValue],
    union_variants: &[JsonValue],
) {
    let explicit = match object.get("type") {
        Some(JsonValue::String(kind)) => Some(kind.clone()),
        Some(JsonValue::Array(kinds)) => kinds
            .iter()
            .filter_map(JsonValue::as_str)
            .filter(|kind| *kind != "null")
            .max_by_key(|kind| type_rank(kind))
            .map(str::to_string),
        _ => None,
    };
    let structural = if object.contains_key("properties") {
        Some("object".to_string())
    } else if object.contains_key("items") {
        Some("array".to_string())
    } else {
        None
    };
    let variant_type = all_of
        .iter()
        .chain(union_variants)
        .max_by_key(|variant| schema_rank(variant))
        .and_then(schema_type);
    object.insert(
        "type".to_string(),
        JsonValue::String(
            structural
                .or(explicit)
                .or(variant_type)
                .or_else(|| constant.and_then(value_type).map(str::to_string))
                .unwrap_or_else(|| "string".to_string()),
        ),
    );

    if let Some(JsonValue::String(value)) = constant
        && !object.contains_key("enum")
    {
        object.insert("enum".to_string(), serde_json::json!([value]));
    }
}

fn value_type(value: &JsonValue) -> Option<&'static str> {
    match value {
        JsonValue::String(_) => Some("string"),
        JsonValue::Bool(_) => Some("boolean"),
        JsonValue::Number(number) if number.is_i64() || number.is_u64() => Some("integer"),
        JsonValue::Number(_) => Some("number"),
        JsonValue::Array(_) => Some("array"),
        JsonValue::Object(_) => Some("object"),
        JsonValue::Null => None,
    }
}

fn type_rank(kind: &str) -> u8 {
    match kind {
        "object" => 6,
        "array" => 5,
        "integer" | "number" | "boolean" => 4,
        "string" => 3,
        "null" => 0,
        _ => 1,
    }
}

fn schema_type(schema: &JsonValue) -> Option<String> {
    let object = schema.as_object()?;
    if object.contains_key("properties") {
        return Some("object".to_string());
    }
    if object.contains_key("items") {
        return Some("array".to_string());
    }
    object
        .get("type")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
}

fn schema_rank(schema: &JsonValue) -> u8 {
    schema_type(schema).as_deref().map(type_rank).unwrap_or(0)
}

fn merge_variant(
    target: &mut serde_json::Map<String, JsonValue>,
    variant: &JsonValue,
    merge_variant_properties: bool,
) {
    let Some(source) = variant.as_object() else {
        return;
    };
    for key in [
        "items",
        "enum",
        "format",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
    ] {
        if !target.contains_key(key)
            && let Some(value) = source.get(key)
        {
            target.insert(key.to_string(), value.clone());
        }
    }
    if merge_variant_properties {
        merge_properties(target, variant);
    }
}

fn merge_required_fields(target: &mut serde_json::Map<String, JsonValue>, variants: &[JsonValue]) {
    let mut seen: HashSet<String> = target
        .get("required")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_string)
        .collect();
    let mut additions = Vec::new();
    for required in variants
        .iter()
        .filter_map(|variant| variant.get("required").and_then(JsonValue::as_array))
    {
        for field in required.iter().filter_map(JsonValue::as_str) {
            if seen.insert(field.to_string()) {
                additions.push(JsonValue::String(field.to_string()));
            }
        }
    }
    if additions.is_empty() {
        return;
    }
    let target_required = target
        .entry("required".to_string())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    if let JsonValue::Array(existing) = target_required {
        existing.extend(additions);
    }
}

fn merge_properties(target: &mut serde_json::Map<String, JsonValue>, variant: &JsonValue) {
    let Some(source_properties) = variant.get("properties").and_then(JsonValue::as_object) else {
        return;
    };
    let target_properties = target
        .entry("properties".to_string())
        .or_insert_with(|| JsonValue::Object(serde_json::Map::new()));
    let Some(target_properties) = target_properties.as_object_mut() else {
        return;
    };
    for (name, property) in source_properties {
        match target_properties.entry(name.clone()) {
            serde_json::map::Entry::Vacant(entry) => {
                entry.insert(property.clone());
            }
            serde_json::map::Entry::Occupied(mut entry) => {
                merge_property_schema(entry.get_mut(), property);
            }
        }
    }
}

fn merge_property_schema(target: &mut JsonValue, source: &JsonValue) {
    let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) else {
        return;
    };
    if target.get("type") != source.get("type") {
        return;
    }
    let Some(JsonValue::Array(source_values)) = source.get("enum") else {
        return;
    };
    let target_values = target
        .entry("enum".to_string())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    let Some(target_values) = target_values.as_array_mut() else {
        return;
    };
    for value in source_values {
        if !target_values.contains(value) {
            target_values.push(value.clone());
        }
    }
}

fn merge_matching_enum(target: &mut serde_json::Map<String, JsonValue>, variant: &JsonValue) {
    let Some(source) = variant.as_object() else {
        return;
    };
    if target.get("type") != source.get("type") {
        return;
    }
    let Some(JsonValue::Array(source_values)) = source.get("enum") else {
        return;
    };
    let target_values = target
        .entry("enum".to_string())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    let Some(target_values) = target_values.as_array_mut() else {
        return;
    };
    for value in source_values {
        if !target_values.contains(value) {
            target_values.push(value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_gemini_schema_subset(value: &JsonValue) {
        match value {
            JsonValue::Object(object) => {
                for keyword in [
                    "if", "then", "else", "not", "const", "oneOf", "anyOf", "allOf",
                ] {
                    assert!(
                        !object.contains_key(keyword),
                        "Gemini schema still contains unsupported keyword {keyword}: {value}"
                    );
                }
                if let Some(schema_type) = object.get("type") {
                    assert!(
                        schema_type.is_string(),
                        "Gemini requires a scalar type: {value}"
                    );
                }
                for nested in object.values() {
                    assert_gemini_schema_subset(nested);
                }
            }
            JsonValue::Array(values) => {
                for nested in values {
                    assert_gemini_schema_subset(nested);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn recursively_reduces_unsupported_schema_constructs() {
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "headers": {
                    "type": ["string", "object", "null"],
                    "additionalProperties": { "type": "string" }
                },
                "action": { "const": "replace" },
                "payload": {
                    "oneOf": [
                        { "type": "string" },
                        {
                            "type": "object",
                            "properties": { "value": { "type": "string" } }
                        }
                    ]
                }
            },
            "allOf": [{
                "if": { "properties": { "action": { "const": "replace" } } },
                "then": { "required": ["payload"] }
            }]
        });
        let mut description = "HTTP request".to_string();

        let result = shape_tool_schema(&input, &mut description);

        assert_gemini_schema_subset(&result);
        assert_eq!(result["properties"]["headers"]["type"], "object");
        assert_eq!(result["properties"]["payload"]["type"], "object");
        assert_eq!(
            result["properties"]["payload"]["properties"]["value"]["type"],
            "string"
        );
        assert_eq!(result["properties"]["action"]["type"], "string");
        assert_eq!(
            result["properties"]["action"]["enum"],
            serde_json::json!(["replace"])
        );
    }

    #[test]
    fn preserves_top_level_tagged_union_discriminators() {
        let input = serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "action": { "const": "create_document" },
                        "title": { "type": "string" }
                    }
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "const": "batch_update" },
                        "requests": { "type": "array" }
                    }
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "const": "read_content" },
                        "document_id": { "type": "string" }
                    }
                }
            ]
        });
        let mut description = "Google Docs".to_string();

        let result = shape_tool_schema(&input, &mut description);

        assert_eq!(
            result["properties"]["action"]["enum"],
            serde_json::json!(["create_document", "batch_update", "read_content"]),
            "every tagged-union action must remain available to Gemini"
        );
    }

    #[test]
    fn preserves_nested_object_union_discriminators() {
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "schedule": {
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "kind": { "const": "cron" },
                                "expression": { "type": "string" }
                            }
                        },
                        {
                            "type": "object",
                            "properties": {
                                "kind": { "const": "once" },
                                "at": { "type": "string" }
                            }
                        }
                    ]
                }
            }
        });
        let mut description = "Create a trigger".to_string();

        let result = shape_tool_schema(&input, &mut description);

        assert_eq!(
            result["properties"]["schedule"]["properties"]["kind"]["enum"],
            serde_json::json!(["cron", "once"]),
            "both schedule variants must remain expressible"
        );
    }

    #[test]
    fn preserves_untyped_array_items_as_objects() {
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "requests": { "type": "array" }
            }
        });
        let mut description = "Batch update".to_string();

        let result = shape_tool_schema(&input, &mut description);

        assert_eq!(
            result["properties"]["requests"]["items"]["type"], "object",
            "Vec<serde_json::Value> items must remain free-form objects"
        );
    }

    #[test]
    fn recursively_reduces_nested_any_of() {
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "payload": {
                    "anyOf": [
                        { "type": "string" },
                        {
                            "type": "object",
                            "properties": { "value": { "type": "string" } }
                        }
                    ]
                }
            }
        });
        let mut description = "Submit a payload".to_string();

        let result = shape_tool_schema(&input, &mut description);

        assert_gemini_schema_subset(&result);
        assert_eq!(result["properties"]["payload"]["type"], "object");
        assert_eq!(
            result["properties"]["payload"]["properties"]["value"]["type"],
            "string"
        );
    }
}
