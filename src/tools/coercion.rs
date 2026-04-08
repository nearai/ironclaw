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
    let resolved = resolve_refs(schema);
    coerce_value(params, &resolved)
}

// ── $ref resolution ──────────────────────────────────────────────────

/// Inline all `$ref` pointers in a JSON Schema so downstream coercion
/// operates on a flat, self-contained schema tree.
///
/// Supports `#/definitions/<name>` and `#/$defs/<name>` (JSON Schema
/// draft-07 and 2020-12 respectively). Unknown `$ref` formats are left
/// unchanged. A depth limit prevents infinite recursion from circular refs.
fn resolve_refs(schema: &serde_json::Value) -> serde_json::Value {
    let definitions = schema
        .get("definitions")
        .or_else(|| schema.get("$defs"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    resolve_refs_inner(schema, &definitions, 0)
}

const MAX_REF_DEPTH: usize = 16;

fn resolve_refs_inner(
    schema: &serde_json::Value,
    definitions: &serde_json::Value,
    depth: usize,
) -> serde_json::Value {
    if depth > MAX_REF_DEPTH {
        return schema.clone();
    }
    match schema {
        serde_json::Value::Object(obj) => {
            // If this node is a $ref, resolve it and recurse into the target.
            if let Some(ref_str) = obj.get("$ref").and_then(|v| v.as_str()) {
                if let Some(target) = resolve_ref_pointer(ref_str, definitions) {
                    return resolve_refs_inner(&target, definitions, depth + 1);
                }
                return schema.clone();
            }

            // Recursively resolve refs in all values (skip definitions maps).
            let resolved: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .map(|(k, v)| {
                    if k == "definitions" || k == "$defs" {
                        (k.clone(), v.clone())
                    } else {
                        (k.clone(), resolve_refs_inner(v, definitions, depth + 1))
                    }
                })
                .collect();
            serde_json::Value::Object(resolved)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| resolve_refs_inner(v, definitions, depth + 1))
                .collect(),
        ),
        _ => schema.clone(),
    }
}

fn resolve_ref_pointer(
    ref_str: &str,
    definitions: &serde_json::Value,
) -> Option<serde_json::Value> {
    let path = ref_str.strip_prefix("#/")?;
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() == 2 && (parts[0] == "definitions" || parts[0] == "$defs") {
        return definitions.get(parts[1]).cloned();
    }
    None
}

// ── Core coercion ────────────────────────────────────────────────────

fn coerce_value(value: &serde_json::Value, schema: &serde_json::Value) -> serde_json::Value {
    // This coercer handles concrete schema shapes including discriminated unions
    // (oneOf/anyOf with const or single-element enum discriminators), allOf
    // merges, and $ref references (resolved in a pre-pass).
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
        let additional_schema = schema
            .get("additionalProperties")
            .filter(|v| v.is_object())
            .or_else(|| resolve_additional_properties(schema, obj));
        let required: std::collections::HashSet<&str> = schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let mut coerced = obj.clone();

        for (key, current) in &mut coerced {
            if let Some(prop_schema) = properties.and_then(|props| props.get(key)) {
                // LLMs send "" for optional fields instead of omitting them.
                // Coerce to null only when the field is not required AND the schema
                // allows null or doesn't allow string — a `type: "string"` field
                // may legitimately accept "" as a meaningful value.
                if current.as_str() == Some("")
                    && !required.contains(key.as_str())
                    && (schema_allows_type(prop_schema, "null")
                        || !schema_allows_type(prop_schema, "string"))
                {
                    *current = serde_json::Value::Null;
                    continue;
                }
                // LLMs (especially local models via llama.cpp --jinja) send
                // the literal string "null" instead of JSON null for optional
                // fields. Unlike "", "null" is never intentional content —
                // convert it for any non-required field regardless of type.
                if current.as_str() == Some("null")
                    && !required.contains(key.as_str())
                {
                    *current = serde_json::Value::Null;
                    continue;
                }
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
    collect_properties(schema, obj, 0)
}

const MAX_COMBINATOR_DEPTH: usize = 4;

/// Recursively collect properties from a schema and its combinator variants.
fn collect_properties(
    schema: &serde_json::Value,
    obj: &serde_json::Map<String, serde_json::Value>,
    depth: usize,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    if depth > MAX_COMBINATOR_DEPTH {
        return None;
    }

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

    // allOf: merge ALL variants' properties, recursing into nested combinators
    if let Some(all_of) = schema.get("allOf").and_then(|a| a.as_array()) {
        for variant in all_of {
            if let Some(props) = variant.get("properties").and_then(|p| p.as_object()) {
                merged.extend(props.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            // Recurse into variant if it has its own combinators
            if let Some(nested) = collect_properties(variant, obj, depth + 1) {
                merged.extend(nested);
            }
        }
    }

    // oneOf/anyOf: find discriminated match and merge its properties
    for key in ["oneOf", "anyOf"] {
        if let Some(variants) = schema.get(key).and_then(|v| v.as_array())
            && let Some(variant) = find_discriminated_variant(variants, obj)
        {
            if let Some(props) = variant.get("properties").and_then(|p| p.as_object()) {
                merged.extend(props.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            // Recurse into matched variant if it has its own combinators
            if let Some(nested) = collect_properties(variant, obj, depth + 1) {
                merged.extend(nested);
            }
        }
    }

    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

/// Find `additionalProperties` from a matched combinator variant.
///
/// Checks `allOf` variants first (last-wins), then the matched `oneOf`/`anyOf`
/// variant. Returns `None` if no variant defines `additionalProperties`.
fn resolve_additional_properties<'a>(
    schema: &'a serde_json::Value,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    // allOf: last variant with additionalProperties wins
    if let Some(all_of) = schema.get("allOf").and_then(|a| a.as_array()) {
        for variant in all_of.iter().rev() {
            if let Some(ap) = variant.get("additionalProperties")
                && ap.is_object()
            {
                return Some(ap);
            }
        }
    }

    // oneOf/anyOf: check matched variant
    for key in ["oneOf", "anyOf"] {
        if let Some(variants) = schema.get(key).and_then(|v| v.as_array())
            && let Some(variant) = find_discriminated_variant(variants, obj)
            && let Some(ap) = variant.get("additionalProperties")
            && ap.is_object()
        {
            return Some(ap);
        }
    }

    None
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
    // LLMs often send "" instead of null for optional fields. Coerce empty
    // strings to null when the schema allows null but not string, or allows
    // both but the value is empty (a string field with content "" is kept).
    // LLMs send "" or the literal string "null" instead of JSON null.
    if (s.is_empty() || s == "null") && schema_allows_type(schema, "null") && !schema_allows_type(schema, "string") {
        return Some(serde_json::Value::Null);
    }

    if schema_allows_type(schema, "string") {
        // Local LLMs (e.g. llama.cpp with --jinja) sometimes double-quote
        // string values by sending a JSON string literal as the string
        // content, e.g. `"\"private\""` instead of `"private"`.
        // Only unquote when the value itself parses as a valid JSON string
        // literal so legitimate strings that intentionally start and end
        // with quote characters are preserved.
        if s.len() >= 2
            && s.starts_with('"')
            && s.ends_with('"')
            && let Ok(parsed) = serde_json::from_str::<String>(s)
        {
            return Some(serde_json::Value::String(parsed));
        }
        return None;
    }

    // Empty string with no type match — return unchanged since we can't
    // determine the intended type.
    if s.is_empty() {
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
    fn coerces_empty_string_to_null_for_nullable_non_required_field() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "timezone": { "type": ["string", "null"] },
                "schedule": { "type": "string" }
            },
            "required": ["schedule"]
        });
        let params = serde_json::json!({
            "timezone": "",
            "schedule": "0 9 * * *"
        });

        let result = prepare_params_for_schema(&params, &schema);

        // Non-required nullable "timezone" with empty string → null
        assert_eq!(result["timezone"], serde_json::Value::Null);
        // Required "schedule" keeps its value even if empty would be weird
        assert_eq!(result["schedule"], serde_json::json!("0 9 * * *"));
    }

    #[test]
    fn keeps_empty_string_for_non_required_string_only_field() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "timezone": { "type": "string" },
                "schedule": { "type": "string" }
            },
            "required": ["schedule"]
        });
        let params = serde_json::json!({
            "timezone": "",
            "schedule": "0 9 * * *"
        });

        let result = prepare_params_for_schema(&params, &schema);

        // Non-required string-only "timezone" keeps empty string (meaningful value)
        assert_eq!(result["timezone"], serde_json::json!(""));
        assert_eq!(result["schedule"], serde_json::json!("0 9 * * *"));
    }

    #[test]
    fn coerces_empty_string_to_null_for_explicit_nullable_type() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "from_timezone": { "type": ["string", "null"] },
                "operation": { "type": "string" }
            },
            "required": ["operation"]
        });
        let params = serde_json::json!({
            "from_timezone": "",
            "operation": "now"
        });

        let result = prepare_params_for_schema(&params, &schema);

        // Nullable type with empty string → null (even if it were required,
        // the per-value coercion in coerce_string_value handles this)
        assert_eq!(result["from_timezone"], serde_json::Value::Null);
        assert_eq!(result["operation"], serde_json::json!("now"));
    }

    #[test]
    fn keeps_empty_string_for_required_string_only_field() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        let params = serde_json::json!({ "name": "" });

        let result = prepare_params_for_schema(&params, &schema);

        // Required string-only field keeps empty string
        assert_eq!(result["name"], serde_json::json!(""));
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
    fn resolves_ref_and_coerces_referenced_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "definitions": {
                "Pagination": {
                    "type": "object",
                    "properties": {
                        "page": { "type": "integer" },
                        "per_page": { "type": "integer" }
                    }
                }
            },
            "allOf": [
                { "$ref": "#/definitions/Pagination" },
                {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    }
                }
            ]
        });
        let params = serde_json::json!({
            "page": "2",
            "per_page": "50",
            "query": "test"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["page"], serde_json::json!(2));
        assert_eq!(result["per_page"], serde_json::json!(50));
        assert_eq!(result["query"], serde_json::json!("test"));
    }

    #[test]
    fn resolves_nested_refs_in_oneof_variants() {
        let schema = serde_json::json!({
            "type": "object",
            "$defs": {
                "ListParams": {
                    "properties": {
                        "action": { "const": "list" },
                        "limit": { "type": "integer" }
                    }
                }
            },
            "oneOf": [
                { "$ref": "#/$defs/ListParams" },
                {
                    "properties": {
                        "action": { "const": "get" },
                        "id": { "type": "integer" }
                    }
                }
            ]
        });
        let params = serde_json::json!({
            "action": "list",
            "limit": "25"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["limit"], serde_json::json!(25));
    }

    #[test]
    fn coerces_nested_combinators_allof_containing_oneof() {
        // allOf where one variant is itself a oneOf (nested combinator)
        let schema = serde_json::json!({
            "type": "object",
            "allOf": [
                {
                    "properties": {
                        "version": { "type": "integer" }
                    }
                },
                {
                    "oneOf": [
                        {
                            "properties": {
                                "mode": { "const": "fast" },
                                "threads": { "type": "integer" }
                            }
                        },
                        {
                            "properties": {
                                "mode": { "const": "safe" },
                                "retries": { "type": "integer" }
                            }
                        }
                    ]
                }
            ]
        });
        let params = serde_json::json!({
            "version": "3",
            "mode": "fast",
            "threads": "8"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["version"], serde_json::json!(3));
        assert_eq!(result["threads"], serde_json::json!(8));
    }

    #[test]
    fn coerces_array_items_with_oneof_discriminator() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "actions": {
                    "type": "array",
                    "items": {
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "const": "move" },
                                    "distance": { "type": "integer" }
                                }
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "const": "wait" },
                                    "seconds": { "type": "number" }
                                }
                            }
                        ]
                    }
                }
            }
        });
        let params = serde_json::json!({
            "actions": [
                { "type": "move", "distance": "10" },
                { "type": "wait", "seconds": "2.5" }
            ]
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["actions"][0]["distance"], serde_json::json!(10));
        assert_eq!(result["actions"][1]["seconds"], serde_json::json!(2.5));
    }

    #[test]
    fn circular_ref_does_not_infinite_loop() {
        let schema = serde_json::json!({
            "type": "object",
            "definitions": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "integer" },
                        "child": { "$ref": "#/definitions/Node" }
                    }
                }
            },
            "properties": {
                "root": { "$ref": "#/definitions/Node" }
            }
        });
        let params = serde_json::json!({
            "root": { "value": "42" }
        });

        // Should not hang — depth limit stops the recursion
        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["root"]["value"], serde_json::json!(42));
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

    #[test]
    fn coerces_literal_null_string_to_json_null_for_nullable_non_string_field() {
        // When a field only allows null (not string), the literal "null"
        // should be coerced to JSON null.
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "ref_id": { "type": ["integer", "null"] },
                "content": { "type": "string" }
            },
            "required": ["content"]
        });
        let params = serde_json::json!({
            "ref_id": "null",
            "content": "hello"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["ref_id"], serde_json::Value::Null);
        assert_eq!(result["content"], serde_json::json!("hello"));
    }

    #[test]
    fn converts_null_string_to_null_for_nullable_string_field() {
        // "null" is never intentional content — convert to JSON null
        // for any non-required field, even if the schema allows string.
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "layer": { "type": ["string", "null"] }
            }
        });
        let params = serde_json::json!({
            "layer": "null"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["layer"], serde_json::Value::Null);
    }

    #[test]
    fn strips_double_quoted_string_values() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "layer": { "type": "string" }
            }
        });
        let params = serde_json::json!({
            "layer": "\"private\""
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["layer"], serde_json::json!("private"));
    }

    #[test]
    fn preserves_legitimate_quoted_strings() {
        // A string that starts and ends with quotes but isn't a valid JSON
        // string literal should be left alone.
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            }
        });
        let params = serde_json::json!({
            "text": "\"unmatched"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["text"], serde_json::json!("\"unmatched"));
    }

    #[test]
    fn converts_null_string_to_null_for_non_required_string_field() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "time_min": { "type": "string" },
                "action": { "type": "string" }
            },
            "required": ["action"]
        });
        let params = serde_json::json!({
            "time_min": "null",
            "action": "list_events"
        });

        let result = prepare_params_for_schema(&params, &schema);

        assert_eq!(result["time_min"], serde_json::Value::Null);
        assert_eq!(result["action"], serde_json::json!("list_events"));
    }

    #[test]
    fn keeps_null_string_for_required_string_field() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        });
        let params = serde_json::json!({
            "query": "null"
        });

        let result = prepare_params_for_schema(&params, &schema);

        // Required fields keep "null" as-is — don't silently drop required values
        assert_eq!(result["query"], serde_json::json!("null"));
    }
}
