use serde_json::Value as JsonValue;

use crate::provider::{ToolCall, ToolDefinition};

/// Policy for shaping tool schemas at the provider boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolSchemaPolicy {
    /// Apply top-level flattening plus strict-mode object rewriting.
    StrictOpenAi,
    /// Apply top-level flattening plus provider-safe cleanup, but do not
    /// force optional fields into required-nullable strict mode.
    FlattenOnly,
}

/// Shape a tool schema for the target provider contract.
///
/// `description` is mutable because flattening can append an advisory hint
/// containing the original top-level schema when the request path cannot send
/// that shape directly.
pub(crate) fn shape_tool_schema(
    policy: ToolSchemaPolicy,
    schema: &JsonValue,
    description: &mut String,
) -> JsonValue {
    match policy {
        ToolSchemaPolicy::StrictOpenAi => normalize_schema_strict(schema, description),
        ToolSchemaPolicy::FlattenOnly => normalize_schema_flatten_only(schema, description),
    }
}

/// Normalize a JSON Schema for OpenAI strict tool-calling compatibility.
pub(crate) fn normalize_schema_strict(schema: &JsonValue, description: &mut String) -> JsonValue {
    normalize_schema(schema, description, true)
}

fn normalize_schema_flatten_only(schema: &JsonValue, description: &mut String) -> JsonValue {
    normalize_schema(schema, description, false)
}

fn normalize_schema(
    schema: &JsonValue,
    description: &mut String,
    strict_objects: bool,
) -> JsonValue {
    let mut schema = schema.clone();

    if needs_top_level_flatten(&schema) {
        flatten_top_level(&mut schema, description);
        if let Some(props) = schema.get_mut("properties").and_then(|v| v.as_object_mut()) {
            for (_key, prop_schema) in props.iter_mut() {
                normalize_schema_recursive(prop_schema, strict_objects);
            }
        }
        if strict_objects {
            return schema;
        }
    } else {
        normalize_schema_recursive(&mut schema, strict_objects);
    }

    // Diagnostic strict-mode validation lives in the main crate's
    // `tools::schema_validator` and is exercised in CI tests there. The
    // run-time `normalize_schema_strict` itself does not need it — invalid
    // schemas remain usable (the LLM provider may reject them at request
    // time) so the previous debug-log call has been intentionally dropped
    // during the LLM-crate extraction.
    schema
}

/// JSON Schema keywords that OpenAI's tool API rejects at the top level of a
/// tool's `parameters`.
const FORBIDDEN_TOP_LEVEL: &[&str] = &["oneOf", "anyOf", "allOf", "enum", "not"];

fn detect_forbidden_top_level(schema: &JsonValue) -> Option<&'static str> {
    let map = schema.as_object()?;
    FORBIDDEN_TOP_LEVEL
        .iter()
        .find(|keyword| map.contains_key(**keyword))
        .copied()
}

fn needs_top_level_flatten(schema: &JsonValue) -> bool {
    match schema {
        JsonValue::Object(map) => {
            let has_forbidden = FORBIDDEN_TOP_LEVEL.iter().any(|k| map.contains_key(*k));
            has_forbidden || !top_level_accepts_object_properties(map)
        }
        _ => true,
    }
}

fn schema_flatten_hint_intro(detected: Option<&'static str>) -> &'static str {
    match detected {
        Some("oneOf") | Some("anyOf") => {
            "\n\nUpstream JSON schema (advisory; the actual top-level union has been \
             flattened so the OpenAI tool API will accept the tool — pick ONE variant \
             and pass its fields as a flat object):\n"
        }
        Some("allOf") => {
            "\n\nUpstream JSON schema (advisory; the actual top-level intersection has \
             been flattened so the OpenAI tool API will accept the tool — pass fields \
             from ALL variants combined as a flat object):\n"
        }
        Some("enum") => {
            "\n\nUpstream JSON schema (advisory; the actual top-level was an enum, \
             which OpenAI's tool API doesn't allow at the top level — pass one of \
             the listed values as the parameters object):\n"
        }
        Some("not") => {
            "\n\nUpstream JSON schema (advisory; the actual top-level was a `not` \
             constraint, which OpenAI's tool API doesn't allow at the top level — \
             pass any object that does NOT match the constraint):\n"
        }
        _ => {
            "\n\nUpstream JSON schema (advisory; the original was not a top-level \
             object schema, so we flattened to a free-form object — see below for \
             the actual constraints the upstream server will enforce):\n"
        }
    }
}

fn merge_top_level_variant_properties(schema: &JsonValue) -> serde_json::Map<String, JsonValue> {
    let mut merged = serde_json::Map::new();
    let Some(obj) = schema.as_object() else {
        return merged;
    };
    if top_level_accepts_object_properties(obj)
        && let Some(props) = obj.get("properties").and_then(|v| v.as_object())
    {
        for (key, value) in props {
            merged.insert(key.clone(), value.clone());
        }
    }
    for keyword in &["oneOf", "anyOf", "allOf"] {
        let Some(JsonValue::Array(variants)) = obj.get(*keyword) else {
            continue;
        };
        for variant in variants {
            let Some(props) = variant.get("properties").and_then(|v| v.as_object()) else {
                continue;
            };
            for (key, value) in props {
                if !merged.contains_key(key) {
                    merged.insert(key.clone(), value.clone());
                }
            }
        }
    }
    merged
}

fn top_level_accepts_object_properties(map: &serde_json::Map<String, JsonValue>) -> bool {
    match map.get("type") {
        Some(JsonValue::String(s)) => s == "object",
        Some(JsonValue::Array(arr)) => arr
            .iter()
            .any(|v| matches!(v, JsonValue::String(s) if s == "object")),
        None => map.contains_key("properties"),
        _ => false,
    }
}

pub(crate) struct CappedJson {
    pub(crate) text: String,
    pub(crate) was_truncated: bool,
}

pub(crate) fn serialize_json_capped(value: &JsonValue, max_bytes: usize) -> Result<CappedJson, ()> {
    use std::io::{Error, Write};

    const CAP_REACHED: &str = "json cap reached";

    struct CappedWriter {
        buf: Vec<u8>,
        max: usize,
        was_truncated: bool,
    }

    impl Write for CappedWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            let remaining = self.max.saturating_sub(self.buf.len());
            if remaining == 0 {
                self.was_truncated = true;
                return Err(Error::other(CAP_REACHED));
            }
            let to_write = data.len().min(remaining);
            self.buf.extend_from_slice(&data[..to_write]);
            if to_write < data.len() {
                self.was_truncated = true;
                return Err(Error::other(CAP_REACHED));
            }
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let writer = CappedWriter {
        buf: Vec::with_capacity(max_bytes.min(8192)),
        max: max_bytes,
        was_truncated: false,
    };
    let mut ser = serde_json::Serializer::new(writer);
    let serialize_result = serde::Serialize::serialize(value, &mut ser);
    let writer = ser.into_inner();
    let was_truncated = writer.was_truncated;
    let buf = writer.buf;

    match serialize_result {
        Ok(()) => {}
        Err(e) if was_truncated && e.to_string().contains(CAP_REACHED) => {}
        Err(_) => return Err(()),
    }

    let text = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => {
            let valid_len = e.utf8_error().valid_up_to();
            let mut buf = e.into_bytes();
            buf.truncate(valid_len);
            String::from_utf8(buf).map_err(|_| ())?
        }
    };

    Ok(CappedJson {
        text,
        was_truncated,
    })
}

fn flatten_top_level(parameters: &mut JsonValue, description: &mut String) {
    const SCHEMA_HINT_MAX_BYTES: usize = 1500;

    let detected = detect_forbidden_top_level(parameters);
    let merged_properties = merge_top_level_variant_properties(parameters);

    if let Ok(capped) = serialize_json_capped(parameters, SCHEMA_HINT_MAX_BYTES)
        && !capped.text.is_empty()
    {
        let hint = if capped.was_truncated {
            format!("{} ... (truncated)", capped.text)
        } else {
            capped.text
        };
        description.push_str(schema_flatten_hint_intro(detected));
        description.push_str(&hint);
    }

    *parameters = serde_json::json!({
        "type": "object",
        "properties": JsonValue::Object(merged_properties),
        "additionalProperties": true,
        "required": []
    });
}

fn normalize_schema_recursive(schema: &mut JsonValue, strict_objects: bool) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    for key in &["anyOf", "oneOf", "allOf"] {
        if let Some(JsonValue::Array(variants)) = obj.get_mut(*key) {
            for variant in variants.iter_mut() {
                normalize_schema_recursive(variant, strict_objects);
            }
        }
    }

    let is_array = obj
        .get("type")
        .map(|t| {
            t.as_str() == Some("array")
                || t.as_array()
                    .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("array")))
        })
        .unwrap_or(false);
    if is_array {
        let needs_fix = match obj.get("items") {
            None => true,
            Some(JsonValue::Object(_)) => false,
            _ => true,
        };
        if needs_fix {
            obj.insert("items".to_string(), serde_json::json!({}));
        }
    }
    if let Some(items) = obj.get_mut("items") {
        normalize_schema_recursive(items, strict_objects);
    }

    for key in &["not", "if", "then", "else"] {
        if let Some(sub) = obj.get_mut(*key) {
            normalize_schema_recursive(sub, strict_objects);
        }
    }

    if !strict_objects {
        if obj.contains_key("properties") && !obj.contains_key("type") {
            obj.insert("type".to_string(), JsonValue::String("object".to_string()));
        }
        if let Some(JsonValue::Object(props)) = obj.get_mut("properties") {
            for prop_schema in props.values_mut() {
                normalize_schema_recursive(prop_schema, false);
            }
        }
        return;
    }

    let is_object = obj
        .get("type")
        .and_then(|t| t.as_str())
        .map(|t| t == "object")
        .unwrap_or(false);
    let has_properties = obj.contains_key("properties");

    if !is_object && !has_properties {
        return;
    }

    if !obj.contains_key("type") && has_properties {
        obj.insert("type".to_string(), JsonValue::String("object".to_string()));
    }

    obj.insert("additionalProperties".to_string(), JsonValue::Bool(false));

    if !obj.contains_key("properties") {
        obj.insert(
            "properties".to_string(),
            JsonValue::Object(serde_json::Map::new()),
        );
    }

    let current_required: std::collections::HashSet<String> = obj
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let all_keys: Vec<String> = obj
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|props| {
            let mut keys: Vec<String> = props.keys().cloned().collect();
            keys.sort();
            keys
        })
        .unwrap_or_default();

    if let Some(JsonValue::Object(props)) = obj.get_mut("properties") {
        for key in &all_keys {
            if let Some(prop_schema) = props.get_mut(key) {
                normalize_schema_recursive(prop_schema, true);
                if !current_required.contains(key) {
                    make_nullable(prop_schema);
                }
            }
        }
    }

    let required_value: Vec<JsonValue> = all_keys.into_iter().map(JsonValue::String).collect();
    obj.insert("required".to_string(), JsonValue::Array(required_value));
}

fn make_nullable(schema: &mut JsonValue) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    if let Some(type_val) = obj.get("type").cloned() {
        match type_val {
            JsonValue::String(ref t) if t != "null" => {
                obj.insert("type".to_string(), serde_json::json!([t, "null"]));
            }
            JsonValue::Array(ref arr) => {
                let has_null = arr.iter().any(|v| v.as_str() == Some("null"));
                if !has_null {
                    let mut new_arr = arr.clone();
                    new_arr.push(JsonValue::String("null".to_string()));
                    obj.insert("type".to_string(), JsonValue::Array(new_arr));
                }
            }
            _ => {}
        }
    } else {
        let existing = JsonValue::Object(obj.clone());
        obj.clear();
        obj.insert(
            "anyOf".to_string(),
            serde_json::json!([existing, {"type": "null"}]),
        );
    }
}

/// Strip "unset optional" placeholder values that the strict-mode tool-schema
/// transform induces, from each tool call's arguments.
///
/// `shape_tool_schema(StrictOpenAi, ..)` forces every property to be `required`
/// and makes the originally-optional ones nullable, because OpenAI strict
/// function-calling has no notion of an absent property. Models therefore fill
/// the optionals they aren't using with a placeholder — `null` for most, `""`
/// for some (e.g. gpt-5.2-codex). Validating those against the tool's *original*
/// schema (where the fields are optional and non-nullable) would reject them, so
/// this removes them: the inverse of that transform, keyed on the effective
/// `required` set (the original schema's, unioned across `oneOf`/`anyOf`/`allOf`
/// variants — so tagged-enum tools keep their variant-required fields), leaving
/// required fields and genuinely provided values intact.
///
/// `strip_empty_strings` should be `true` only for the codex family, which fills
/// unset optionals with `""`; other strict providers send `null`, so passing
/// `false` preserves a deliberately-empty string they may have sent.
///
/// Only the strict providers (Codex / OpenAI / `RigAdapter`) call this; NEAR
/// AI's `FlattenOnly` path never induces the placeholders and must not be
/// touched. (When the RC3/M9 `NormalizingProvider` decorator lands it should
/// adopt this as a Class-A shape invariant.)
pub(crate) fn strip_unset_optional_fields(
    tool_calls: &mut [ToolCall],
    tools: &[ToolDefinition],
    strip_empty_strings: bool,
) {
    for call in tool_calls.iter_mut() {
        if let Some(tool) = tools.iter().find(|tool| tool.name == call.name) {
            strip_unset_optionals_in_value(
                &mut call.arguments,
                &tool.parameters,
                strip_empty_strings,
            );
        }
    }
}

/// Recursive worker for [`strip_unset_optional_fields`]: walks `value` alongside
/// its original `schema`, dropping unset-placeholder values on the fields the
/// schema's *top-level* `required` marks optional, and recursing into nested
/// objects and array items.
///
/// Top-level on purpose: a tagged-enum tool is a top-level `oneOf`/`anyOf`, which
/// the forward transform (`flatten_top_level`) rewrites to `required: []` +
/// `additionalProperties: true`, so dropping every placeholder is exactly what
/// collapses the model's reply to the single variant it populated. Unioning the
/// variants' `required` sets would keep a non-selected variant's placeholder and
/// make the args match more than one `oneOf` branch (regressing e.g.
/// `skill_install`/`routine`), so keep it top-level.
fn strip_unset_optionals_in_value(
    value: &mut JsonValue,
    schema: &JsonValue,
    strip_empty_strings: bool,
) {
    match value {
        JsonValue::Object(map) => {
            let required: Vec<&str> = schema
                .get("required")
                .and_then(JsonValue::as_array)
                .map(|names| names.iter().filter_map(JsonValue::as_str).collect())
                .unwrap_or_default();
            let properties = schema.get("properties").and_then(JsonValue::as_object);
            let to_remove: Vec<String> = map
                .iter()
                .filter(|(key, val)| {
                    !required.contains(&key.as_str())
                        && is_unset_placeholder(val, strip_empty_strings)
                })
                .map(|(key, _)| key.clone())
                .collect();
            for key in &to_remove {
                map.remove(key);
            }
            for (key, val) in map.iter_mut() {
                if let Some(prop_schema) = properties.and_then(|props| props.get(key.as_str())) {
                    strip_unset_optionals_in_value(val, prop_schema, strip_empty_strings);
                }
            }
        }
        JsonValue::Array(items) => {
            if let Some(item_schema) = schema.get("items") {
                for item in items.iter_mut() {
                    strip_unset_optionals_in_value(item, item_schema, strip_empty_strings);
                }
            }
        }
        _ => {}
    }
}

/// A value the model emitted only as a strict-mode placeholder for an unset
/// field: `null` always, or an empty string when `strip_empty_strings` is set.
/// The codex family fills unset optionals with `""`; other strict providers send
/// `null`, so their callers pass `false` to preserve a deliberately-empty string.
fn is_unset_placeholder(value: &JsonValue, strip_empty_strings: bool) -> bool {
    match value {
        JsonValue::Null => true,
        JsonValue::String(string) => strip_empty_strings && string.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_unset_optional_fields_drops_optional_null_and_empty() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string" },
                "format": { "type": "string" },
                "timezone": { "type": "string" }
            }
        });
        let mut value = serde_json::json!({ "operation": "now", "format": null, "timezone": "" });
        strip_unset_optionals_in_value(&mut value, &schema, true);
        assert_eq!(value, serde_json::json!({ "operation": "now" }));
    }

    #[test]
    fn test_strip_unset_optional_fields_keeps_required_placeholders() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "id": { "type": "string" }, "note": { "type": "string" } },
            "required": ["id"]
        });
        let mut value = serde_json::json!({ "id": null, "note": null });
        strip_unset_optionals_in_value(&mut value, &schema, true);
        // The required `id` is kept (it surfaces as a validation error
        // downstream); only the optional `note` placeholder is dropped.
        assert_eq!(value, serde_json::json!({ "id": null }));
    }

    #[test]
    fn test_strip_unset_optional_fields_collapses_top_level_oneof_to_one_variant() {
        // A tagged-enum tool (top-level `oneOf`, mirroring `skill_install`) has no
        // top-level `required`, and the forward transform flattens it to a
        // permissive object. The model populates the variant it chose (`url`) and
        // leaves the other variant's field as a placeholder; stripping every
        // placeholder collapses the args to the single matching variant. Verified
        // end-to-end on a live build by driving `skill_install` through the agent
        // (the call passed validation and reached the approval gate). Unioning the
        // variants' `required` would instead keep `content` and match both.
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
        strip_unset_optionals_in_value(&mut value, &schema, true);
        assert_eq!(
            value,
            serde_json::json!({ "url": "https://example.com/SKILL.md" })
        );
    }

    #[test]
    fn test_strip_unset_optional_fields_recurses_objects_and_arrays() {
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
        strip_unset_optionals_in_value(&mut value, &schema, true);
        assert_eq!(
            value,
            serde_json::json!({ "items": [ { "a": "x" }, { "a": "y" } ] })
        );
    }

    #[test]
    fn test_strip_unset_optional_fields_preserves_empty_string_when_disabled() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "prefix": { "type": "string" } }
        });
        // With empty-string stripping off (non-codex providers), a deliberately
        // empty optional string is preserved...
        let mut keep = serde_json::json!({ "prefix": "" });
        strip_unset_optionals_in_value(&mut keep, &schema, false);
        assert_eq!(keep, serde_json::json!({ "prefix": "" }));
        // ...but `null` is always a placeholder and is still dropped.
        let mut drop_null = serde_json::json!({ "prefix": null });
        strip_unset_optionals_in_value(&mut drop_null, &schema, false);
        assert_eq!(drop_null, serde_json::json!({}));
    }

    #[test]
    fn test_strip_unset_optional_fields_matches_calls_to_their_tool_schema() {
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
        strip_unset_optional_fields(&mut calls, &tools, true);
        assert_eq!(
            calls[0].arguments,
            serde_json::json!({ "operation": "now" })
        );
    }

    #[test]
    fn test_flatten_only_preserves_optional_object_fields() {
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "content": { "type": "string" },
                "channel": { "type": "string" },
                "attachments": { "type": "array" }
            },
            "required": ["content"]
        });
        let mut description = "Message tool".to_string();

        let result = shape_tool_schema(ToolSchemaPolicy::FlattenOnly, &input, &mut description);

        assert_eq!(
            result,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string" },
                    "channel": { "type": "string" },
                    "attachments": { "type": "array", "items": {} }
                },
                "required": ["content"]
            })
        );
        assert_eq!(description, "Message tool");
    }

    #[test]
    fn test_flatten_only_flattens_top_level_oneof_without_strict_nullability() {
        let input = serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "properties": {
                        "mode": { "const": "by_name" },
                        "name": { "type": "string" }
                    },
                    "required": ["mode", "name"]
                },
                {
                    "properties": {
                        "mode": { "const": "by_id" },
                        "id": { "type": "string" }
                    },
                    "required": ["mode", "id"]
                }
            ]
        });
        let mut description = "Lookup tool".to_string();

        let result = shape_tool_schema(ToolSchemaPolicy::FlattenOnly, &input, &mut description);

        assert_eq!(result["type"], "object");
        assert!(result.get("oneOf").is_none());
        assert_eq!(result["additionalProperties"], true);
        assert_eq!(result["required"], serde_json::json!([]));
        assert_eq!(result["properties"]["mode"]["const"], "by_name");
        assert_eq!(result["properties"]["name"]["type"], "string");
        assert_eq!(result["properties"]["id"]["type"], "string");
        assert!(description.contains("Upstream JSON schema"));
    }

    #[test]
    fn test_flatten_top_level_anyof_preserves_parent_properties() {
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "capability_id": { "type": "string" },
                "detail": { "type": "string", "enum": ["names", "summary", "schema"] }
            },
            "anyOf": [
                { "required": ["name"] },
                { "required": ["capability_id"] }
            ]
        });
        let mut description = "Capability info".to_string();

        let result = shape_tool_schema(ToolSchemaPolicy::FlattenOnly, &input, &mut description);

        assert_eq!(result["type"], "object");
        assert!(result.get("anyOf").is_none());
        assert_eq!(result["additionalProperties"], true);
        assert_eq!(result["required"], serde_json::json!([]));
        assert_eq!(result["properties"]["name"]["type"], "string");
        assert_eq!(result["properties"]["capability_id"]["type"], "string");
        assert_eq!(
            result["properties"]["detail"]["enum"],
            serde_json::json!(["names", "summary", "schema"])
        );
        assert!(description.contains("Upstream JSON schema"));
    }

    #[test]
    fn test_flatten_only_preserves_properties_without_explicit_object_type() {
        let input = serde_json::json!({
            "properties": {
                "content": { "type": "string" },
                "channel": { "type": "string" }
            },
            "required": ["content"]
        });
        let mut description = "Message tool".to_string();

        let result = shape_tool_schema(ToolSchemaPolicy::FlattenOnly, &input, &mut description);

        assert_eq!(result["type"], "object");
        assert_eq!(result["properties"]["content"]["type"], "string");
        assert_eq!(result["properties"]["channel"]["type"], "string");
        assert_eq!(result["required"], serde_json::json!(["content"]));
        assert_eq!(description, "Message tool");
    }

    #[test]
    fn test_strict_openai_preserves_properties_without_explicit_object_type() {
        let input = serde_json::json!({
            "properties": {
                "content": { "type": "string" },
                "channel": { "type": "string" }
            },
            "required": ["content"]
        });
        let mut description = "Message tool".to_string();

        let result = shape_tool_schema(ToolSchemaPolicy::StrictOpenAi, &input, &mut description);

        assert_eq!(result["type"], "object");
        assert_eq!(result["properties"]["content"]["type"], "string");
        assert_eq!(
            result["properties"]["channel"]["type"],
            serde_json::json!(["string", "null"])
        );
        assert_eq!(
            result["required"],
            serde_json::json!(["channel", "content"])
        );
        assert_eq!(result["additionalProperties"], false);
        assert_eq!(description, "Message tool");
    }

    #[test]
    fn test_non_object_top_level_type_with_properties_still_flattens() {
        for policy in [
            ToolSchemaPolicy::FlattenOnly,
            ToolSchemaPolicy::StrictOpenAi,
        ] {
            let input = serde_json::json!({
                "type": "string",
                "properties": {
                    "content": { "type": "string" }
                },
                "required": ["content"]
            });
            let mut description = "Malformed tool".to_string();

            let result = shape_tool_schema(policy, &input, &mut description);

            assert_eq!(result["type"], "object");
            assert!(
                result["properties"]
                    .as_object()
                    .expect("properties")
                    .is_empty()
            );
            assert_eq!(result["additionalProperties"], true);
            assert_eq!(result["required"], serde_json::json!([]));
            assert!(description.contains("Upstream JSON schema"));
        }
    }
}
